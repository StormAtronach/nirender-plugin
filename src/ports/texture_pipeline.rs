//! `NiDX8TextureStage` and `NiDX8ConfigurableTexturePipeline`. The first is the texture stage
//! that sends its states to the device. The second is the object that owns the fixed-function
//! texture pass.

use core::arch::asm;
use core::mem::transmute;

use crate::d3d::{
    device_add_ref, device_release, device_set_texture, device_set_transform, D3DRS_TEXTUREFACTOR, D3DRS_WRAP0, D3DRS_WRAP1,
    D3DTSS_ADDRESSU, D3DTSS_ADDRESSV, D3DTSS_ALPHAARG1, D3DTSS_ALPHAARG2, D3DTSS_ALPHAOP, D3DTSS_BUMPENVLOFFSET,
    D3DTSS_BUMPENVLSCALE, D3DTSS_BUMPENVMAT00, D3DTSS_BUMPENVMAT01, D3DTSS_BUMPENVMAT10, D3DTSS_BUMPENVMAT11, D3DTSS_COLORARG1,
    D3DTSS_COLORARG2, D3DTSS_COLOROP, D3DTSS_MAGFILTER, D3DTSS_MINFILTER, D3DTSS_MIPFILTER, D3DTSS_TEXCOORDINDEX,
    D3DTSS_TEXTURETRANSFORMFLAGS, D3DTS_TEXTURE0,
};
use crate::engine::{
    self, eh_vector_constructor_iterator, eh_vector_destructor_iterator, ni_object_net_ctor, ni_pointer_assign, ni_pointer_set,
    virtual_at, NI_POINTER_CTOR, NI_POINTER_DTOR, NI_REF_OBJECT_COUNT, VTBL_NI_ALPHA_PROPERTY, VTBL_NI_DX8_TEXTURE_PASS,
    VTBL_NI_REF_OBJECT,
};
use crate::gen::layouts::{
    NiAlphaProperty as AP, NiDX8ConfigurableTexturePipeline as CTP, NiDX8RenderState as RS, NiDX8Renderer as R,
    NiDX8TexturePass as TP, NiDX8TextureStage as TS, NiPropertyState, NiRendererSpecificProperty, NiSourceTexture,
    NiTexture,
};
use crate::mem::{rd, wr, x87_pass};
use crate::ports::ni::{delete_object, pointer_assign, release, REF_COUNT};
use crate::ports::render_state;
use crate::shadow::Regions;

// ---------------------------------------------------------------------------------------------
// Engine data at fixed addresses
// ---------------------------------------------------------------------------------------------

const VTBL_TEXTURE_STAGE: usize = 0x74F828;
const VTBL_PIPELINE: usize = 0x74F6D0;

/// The filter remapper: 6 entries of 12 bytes (minify, magnify, mip), one for each filter mode.
const FILTER_REMAP: usize = engine::FILTER_REMAP_MINIFY;
const FILTER_REMAP_SIZE: usize = 0x48;
const FILTER_MINIFY: usize = engine::FILTER_REMAP_MINIFY;
const FILTER_MAGNIFY: usize = engine::FILTER_REMAP_MAGNIFY;
const FILTER_MIP: usize = engine::FILTER_REMAP_MIP;
/// The address mode remapper: 4 entries of 8 bytes (U, V), one for each clamp mode.
const ADDRESS_REMAP_U: usize = 0x7DDBC8;
const ADDRESS_REMAP_V: usize = 0x7DDBCC;

// The capabilities of the device that `initPipelineState` copies to globals.
const G_VERTEX_SHADER_VERSION: usize = 0x7DD950;
const G_MAX_VERTEX_SHADER_CONST: usize = 0x7DD954;
const G_MAX_STREAMS: usize = engine::MAX_STREAMS;
const G_MAX_STREAM_STRIDE: usize = 0x7DD95C;
const G_MAX_PIXEL_SHADER_VALUE: usize = 0x7DDBF4;
const G_PIXEL_SHADER_VERSION: usize = 0x7DDBF8;
const G_MAX_TEXTURE_BLEND_STAGES: usize = engine::MAX_TEXTURE_BLEND_STAGES;
const G_MAX_SIMULTANEOUS_TEXTURES: usize = engine::MAX_SIMULTANEOUS_TEXTURES;

/// The strings that `create` gives to `nullsub_36` (0x6E9A00), the debug print of the renderer.
/// The function is empty in the game.
const STR_CREATION_ATTEMPT: usize = 0x7C3D58;
const STR_CREATION_DONE: usize = 0x7C3D10;

/// The address of a game global. In the game it is the address with no change. The unit
/// tests map the globals into a block of their own.
#[cfg(not(test))]
#[inline(always)]
fn g(addr: usize) -> usize {
    addr
}

#[cfg(test)]
fn g(addr: usize) -> usize {
    tests::global(addr)
}

// ---------------------------------------------------------------------------------------------
// Members without a name in the layouts
// ---------------------------------------------------------------------------------------------

/// `NiDX8ConfigurableTexturePipeline`: 10 `NiPointer<NiAlphaProperty>`, the alpha templates.
const CTP_ALPHA_TEMPLATES: usize = CTP::alphaTemplates;
/// The capability flags. 1 is an independent V address mode. 0x40 is the flag of `create`.
/// 0x80 and 0x100 come from the texture caps.
const CTP_FLAGS: usize = CTP::field_FC;
/// The blend operation for the "add signed" template, from `TextureOpCaps`.
const CTP_SIGNED_OP: usize = CTP::field_104;
/// A block of 0x7E8 bytes that the game sets to 0, and one byte after it.
const CTP_BLOCK_314: usize = CTP::field_314;
const CTP_BLOCK_314_DWORDS: usize = (CTP_BYTE_AFC - CTP_BLOCK_314) / 4;
const CTP_BYTE_AFC: usize = CTP::field_AFC;
/// The three stages of the default pass.
const CTP_STAGES: [usize; 3] = [CTP::field_10, CTP::field_14, CTP::pipelineState_18];
/// The pass in use (`field_8`). `combiners` is the fixed-function pass.
const CTP_CURRENT_PASS: usize = CTP::currentPass;

const RENDER_DATA_TEXTURE_PASS: usize = crate::gen::layouts::NiRendererSpecificProperty_RenderData7::texturePass;
/// The renderer type id that `selectAndLinkPass` accepts.
const RENDERER_TYPE_DX8: u32 = 7;

/// The stage group of a pass.
const STAGE_GROUP_SIZE: usize = crate::gen::layouts::NiDX8TexturePass__1::SIZE;

const HALF: u32 = 0x3F00_0000;
const ONE: u32 = 0x3F80_0000;

// ---------------------------------------------------------------------------------------------
// Engine functions at fixed addresses
// ---------------------------------------------------------------------------------------------

// `engine` has no function for these addresses.

/// 0x405960 `NiProperty::Flags::SetBit`
unsafe fn property_set_bit(property: usize, set: u32, bit: u32) {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32, u32) -> u32>(0x405960)(property, set, bit);
}

/// 0x408A10 `NiFlags::SetField`
unsafe fn flags_set_field(object: usize, value: u32, mask: u32, shift: u32) {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32, u32, u32) -> u32>(0x408A10)(object, value, mask, shift);
}

/// 0x6FE4F0 `NiPropertyState::GetRendererSpecific`
#[cfg(not(test))]
unsafe fn property_state_renderer_specific(property_state: usize) -> usize {
    transmute::<usize, unsafe extern "thiscall" fn(usize) -> usize>(0x6FE4F0)(property_state)
}

/// The unit tests have no game code. This function does the same read as the function at
/// 0x6FE4F0.
#[cfg(test)]
unsafe fn property_state_renderer_specific(property_state: usize) -> usize {
    rd(property_state, NiPropertyState::propRendererSpecific)
}

/// 0x6B33A0 `NiDX8TexturePass::ctor`
unsafe fn texture_pass_ctor(pass: usize, renderer: usize) -> usize {
    transmute::<usize, unsafe extern "thiscall" fn(usize, usize) -> usize>(0x6B33A0)(pass, renderer)
}

/// 0x6B3AB0 `NiDX8TexturePass::setStageObject`
unsafe fn texture_pass_set_stage_object(pass: usize, index: u32, group: usize) {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32, usize) -> u32>(0x6B3AB0)(pass, index, group);
}

/// 0x6B6450 `sub_6B6450`: calls slot 0x18 of the pass.
unsafe fn texture_pass_next_frame(pass: usize) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize) -> u32>(0x6B6450)(pass)
}

/// 0x6B78D0 `NiDX8TexturePass::_1::setStageAtIndex`
unsafe fn stage_group_set_stage_at_index(group: usize, index: u32, stage: usize) {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32, usize) -> u32>(0x6B78D0)(group, index, stage);
}

/// 0x6C1FA0 `NiDX8TexturePass::setTextureFiltering`. `this` is a texture stage.
unsafe fn stage_set_texture_filtering(stage: usize, renderer: usize, mipmapped: u32, flag_b: u32) {
    transmute::<usize, unsafe extern "thiscall" fn(usize, usize, u32, u32) -> u32>(0x6C1FA0)(stage, renderer, mipmapped, flag_b);
}

/// 0x6C1EE0 `NiDX8TexturePass::setTextureWrapping`. `this` is a texture stage.
unsafe fn stage_set_texture_wrapping(stage: usize, renderer: usize) {
    transmute::<usize, unsafe extern "thiscall" fn(usize, usize) -> u32>(0x6C1EE0)(stage, renderer);
}

// ---------------------------------------------------------------------------------------------
// The cache of the render state
// ---------------------------------------------------------------------------------------------

/// The cache entry of a texture stage state. The game code shifts the stage left by 8.
#[inline(always)]
unsafe fn tss_setting(rs: usize, stage: u32, state: u32) -> usize {
    rs.wrapping_add(stage.wrapping_shl(8) as usize) + RS::maybe_textureStageStateSettings + state as usize * 8
}

/// The inline pattern that writes the value directly to the cache.
unsafe fn tss_write(rs: usize, stage: u32, state: u32, value: u32) {
    let setting = tss_setting(rs, stage, state);
    if rd::<u32>(setting, 0) != value {
        wr(setting, 0, value);
        engine::device_set_texture_stage_state(rd(rs, RS::device), stage, state, value);
    }
}

/// The inline pattern that calls `NiRenderStateSetting::SetValue` for the cache.
unsafe fn tss_set_value(rs: usize, stage: u32, state: u32, value: u32) {
    let setting = tss_setting(rs, stage, state);
    if rd::<u32>(setting, 0) != value {
        render_state::setting_set_value(setting, value, 0);
        engine::device_set_texture_stage_state(rd(rs, RS::device), stage, state, value);
    }
}

#[inline(always)]
unsafe fn texture_slot(rs: usize, stage: u32) -> usize {
    rs.wrapping_add(stage.wrapping_mul(4) as usize) + RS::texturePerStage
}

// ---------------------------------------------------------------------------------------------
// The call at 0x6C1BA4
// ---------------------------------------------------------------------------------------------

/// The target of the call at 0x6C1BA4 in `updateRenderer`. The game code calls
/// `setTextureStageStateWithSave` there. The Morrowind Code Patch changes the target to
/// `patchEnvmapCombiner` (0x736F80). That function reads ESI, EDI, EBP and EDX of the caller.
/// `SITE_LISTS` in `src/ports/mod.rs` declares this site.
#[cfg(not(test))]
unsafe fn texcoord_index_call_target() -> usize {
    crate::site::call_target(0x6C1BA4)
}

#[cfg(test)]
unsafe fn texcoord_index_call_target() -> usize {
    0x65EDE0
}

/// The code of the Morrowind Code Patch's `patchEnvmapCombiner` before its jump back to
/// `setTextureStageStateWithSave` (0x736F80 to 0x736FCE in the database). For a stage whose
/// coordinate index is the camera-space reflection vector of set 0 (an environment map) it
/// turns SELECTARG1(texture) into MODULATE(texture, diffuse), and ADD(texture, current) into
/// MULTIPLYADD(current + texture * diffuse) with COLORARG0 = CURRENT.
const ENVMAP_LIT_HOOK: [u8; 78] = [
    0x81, 0xFA, 0x00, 0x00, 0x03, 0x00, 0x75, 0x46, 0x83, 0x7E, 0x10, 0x02, 0x75, 0x10, 0xC7, 0x46, 0x10, 0x04, 0x00, 0x00,
    0x00, 0xC7, 0x46, 0x18, 0x00, 0x00, 0x00, 0x00, 0xEB, 0x30, 0x83, 0x7E, 0x10, 0x07, 0x75, 0x2A, 0xC7, 0x46, 0x10, 0x19,
    0x00, 0x00, 0x00, 0xC7, 0x46, 0x14, 0x02, 0x00, 0x00, 0x00, 0xC7, 0x46, 0x18, 0x00, 0x00, 0x00, 0x00, 0x8B, 0x47, 0x08,
    0x6A, 0x01, 0x6A, 0x1A, 0xFF, 0x76, 0x08, 0x8B, 0x08, 0x50, 0xFF, 0x91, 0xFC, 0x00, 0x00, 0x00, 0x8B, 0xCD,
];

/// The game's environment map stages take the lit colour: the call at 0x6C1BA4 goes to the
/// Code Patch's hook (`ENVMAP_LIT_HOOK`, wherever it lies). The ports call that target, so
/// the stream has it; the helper derives the stages of the retained table with this fact
/// (`capture::HEADER_ENVMAP_LIT`). Another target is a patch of unknown effect: the table
/// keeps the engine's combiner, and the stream has the patch's.
pub unsafe fn envmap_lit() -> bool {
    let target = texcoord_index_call_target();
    target != 0x65EDE0 && core::slice::from_raw_parts(target as *const u8, ENVMAP_LIT_HOOK.len()) == ENVMAP_LIT_HOOK
}

/// Makes the call at 0x6C1BA4 with the registers that the game code has there. ECX and EBP
/// are the render state of the renderer. EDI is the render state that the function read at
/// entry. ESI is the stage. EBX is the renderer. EDX is the value. EAX is the stage number.
unsafe fn texcoord_index_call(target: usize, rs: usize, rs_at_entry: usize, this: usize, renderer: usize, stage: u32, value: u32) {
    if target == 0x65EDE0 {
        render_state::set_texture_stage_state_with_save(rs, stage, D3DTSS_TEXCOORDINDEX, value, 0);
        return;
    }
    let frame: [usize; 7] = [this, rs_at_entry, rs, value as usize, stage as usize, target, renderer];
    asm!(
        "push ebp",
        "push esi",
        "push edi",
        "push ebx",
        "push dword ptr [eax + 20]",
        "mov esi, [eax]",
        "mov edi, [eax + 4]",
        "mov ebp, [eax + 8]",
        "mov edx, [eax + 12]",
        "mov ebx, [eax + 24]",
        "mov eax, [eax + 16]",
        "push 0",
        "push edx",
        "push 11",
        "push eax",
        "mov ecx, ebp",
        "call dword ptr [esp + 16]",
        "add esp, 4",
        "pop ebx",
        "pop edi",
        "pop esi",
        "pop ebp",
        inout("eax") frame.as_ptr() => _,
        clobber_abi("C"),
    );
}

// ---------------------------------------------------------------------------------------------
// NiDX8TextureStage
// ---------------------------------------------------------------------------------------------

/// 0x6C1930 `NiDX8TextureStage::ctor`
pub unsafe fn texture_stage_ctor(this: usize) -> usize {
    crate::shadow::touch(&texture_stage_ctor_port::STATE);
    wr(this, TS::vtbl, VTBL_NI_REF_OBJECT);
    wr::<u32>(this, TS::refCount, 0);
    let count = g(NI_REF_OBJECT_COUNT);
    wr::<u32>(count, 0, rd::<u32>(count, 0).wrapping_add(1));
    wr::<usize>(this, TS::ptrTexture, 0);
    wr(this, TS::vtbl, VTBL_TEXTURE_STAGE);
    wr::<u32>(this, TS::stage, 0);
    // The game code sets the texture pointer to 0 a second time, with a release.
    let texture: usize = rd(this, TS::ptrTexture);
    if texture != 0 {
        release(texture);
        wr::<usize>(this, TS::ptrTexture, 0);
    }
    wr::<u32>(this, TS::colorArg2, 0);
    wr::<u32>(this, TS::colorOp, 2);
    wr::<u32>(this, TS::colorArg1, 2);
    wr::<u32>(this, TS::alphaOp, 2);
    wr::<u32>(this, TS::alphaArg1, 2);
    wr::<u32>(this, TS::alphaArg2, 0);
    wr::<u32>(this, TS::sourceTexcoordIndex, 0);
    wr::<u32>(this, TS::addressModeU, 3);
    wr::<u32>(this, TS::addressModeV, 3);
    wr::<u32>(this, TS::field_34, 3);
    wr::<u32>(this, TS::textureStageIndex, 0);
    wr::<u32>(this, TS::filter_3C, rd(g(FILTER_MAGNIFY), 0));
    wr::<u32>(this, TS::filter_40, rd(g(FILTER_MINIFY), 0));
    wr::<u32>(this, TS::filter_44, rd(g(FILTER_MIP), 0));
    wr::<u32>(this, TS::textureTransformFlags, 0);
    for i in 0..16 {
        wr::<u32>(this, TS::textureMatrix + i * 4, 0);
    }
    wr::<u32>(this, TS::extraBlendColorArgument, 0xFFFF_FFFF);
    wr::<u8>(this, TS::field_90, 1);
    wr::<u32>(this, TS::field_94, 9);
    wr::<u8>(this, TS::bumpEnvMapEnabled, 0);
    wr::<u32>(this, TS::bumpEnvMap00, 0);
    wr::<u32>(this, TS::bumpEnvMap01, 0);
    wr::<u32>(this, TS::bumpEnvMap10, 0);
    wr::<u32>(this, TS::bumpEnvMap11, 0);
    wr::<u32>(this, TS::lumaScale, ONE);
    wr::<u32>(this, TS::lumaOffset, 0);
    wr::<u32>(this, TS::texturePropMap, 0);
    wr::<u32>(this, TS::field_B8, 0);
    this
}

/// 0x6C1A70 `NiDX8TextureStage::dtor`
pub unsafe fn texture_stage_dtor(this: usize) {
    crate::shadow::touch(&texture_stage_dtor_port::STATE);
    wr(this, TS::vtbl, VTBL_TEXTURE_STAGE);
    let texture: usize = rd(this, TS::ptrTexture);
    if texture != 0 {
        release(texture);
        wr::<usize>(this, TS::ptrTexture, 0);
    }
    // The destructor of the pointer member. The pointer is 0 at this point.
    let texture: usize = rd(this, TS::ptrTexture);
    if texture != 0 {
        release(texture);
    }
    wr(this, TS::vtbl, VTBL_NI_REF_OBJECT);
    let count = g(NI_REF_OBJECT_COUNT);
    wr::<u32>(count, 0, rd::<u32>(count, 0).wrapping_sub(1));
}

/// 0x6C1B00 `NiDX8TextureStage::updateRenderer`. The game code ends with `mov al, 1`.
pub unsafe fn texture_stage_update_renderer(this: usize, renderer: usize) -> u8 {
    crate::shadow::touch(&texture_stage_update_renderer_port::STATE);
    let rs: usize = rd(renderer, R::renderState);
    let texture: usize = rd(this, TS::ptrTexture);
    if texture != 0 {
        let manager: usize = rd(renderer, R::textureManager);
        // The texture manager writes 1 byte to each of the two. For the first, the game code
        // gives the stack slot of its `renderer` argument.
        let mut created: u32 = 0;
        let mut mipmapped: u32 = 0;
        let prepare: unsafe extern "thiscall" fn(usize, usize, *mut u32, *mut u32) -> usize = transmute(virtual_at(manager, 8));
        let d3d_texture = prepare(manager, texture, &mut created, &mut mipmapped);
        let stage: u32 = rd(this, TS::stage);
        let slot = texture_slot(rs, stage);
        if rd::<usize>(slot, 0) != d3d_texture {
            wr(slot, 0, d3d_texture);
            device_set_texture(rd(rs, RS::device), stage, d3d_texture);
        }
        if d3d_texture != 0 {
            let transform_flags: u32 = rd(this, TS::textureTransformFlags);
            let device: usize = rd(renderer, R::d3dDevice);
            let rs_now: usize = rd(renderer, R::renderState);
            if transform_flags != 0 {
                device_set_transform(device, rd::<u32>(this, TS::stage).wrapping_add(D3DTS_TEXTURE0), this + TS::textureMatrix);
            }
            render_state::set_texture_stage_state_with_save(
                rs_now,
                rd(this, TS::stage),
                D3DTSS_TEXTURETRANSFORMFLAGS,
                rd(this, TS::textureTransformFlags),
                0,
            );
            texcoord_index_call(
                texcoord_index_call_target(),
                rs_now,
                rs,
                this,
                renderer,
                rd(this, TS::stage),
                rd(this, TS::sourceTexcoordIndex),
            );
            stage_set_texture_filtering(this, renderer, mipmapped, 1);

            let address_u: u32 = rd(this, TS::addressModeU);
            let rs_now: usize = rd(renderer, R::renderState);
            let address_v: u32 = rd(this, TS::addressModeV);
            render_state::set_texture_stage_state_with_save(rs_now, rd(this, TS::stage), D3DTSS_ADDRESSU, address_u, 0);
            let pipeline: usize = rd(renderer, R::configurableTexturePipeline);
            if rd::<u8>(pipeline, CTP_FLAGS) & 1 != 0 {
                render_state::set_texture_stage_state_with_save(rs_now, rd(this, TS::stage), D3DTSS_ADDRESSV, address_v, 0);
            } else {
                render_state::set_texture_stage_state_with_save(rs_now, rd(this, TS::stage), D3DTSS_ADDRESSV, address_u, 0);
            }
        }
    } else {
        let stage: u32 = rd(this, TS::stage);
        let slot = texture_slot(rs, stage);
        if rd::<usize>(slot, 0) != 0 {
            wr::<usize>(slot, 0, 0);
            device_set_texture(rd(rs, RS::device), stage, 0);
        }
    }

    for (state, member) in [
        (D3DTSS_COLOROP, TS::colorOp),
        (D3DTSS_COLORARG1, TS::colorArg1),
        (D3DTSS_COLORARG2, TS::colorArg2),
        (D3DTSS_ALPHAOP, TS::alphaOp),
        (D3DTSS_ALPHAARG1, TS::alphaArg1),
        (D3DTSS_ALPHAARG2, TS::alphaArg2),
    ] {
        tss_set_value(rs, rd(this, TS::stage), state, rd(this, member));
    }
    render_state::set_render_state(rs, D3DRS_TEXTUREFACTOR, rd(this, TS::extraBlendColorArgument), 0);

    if rd::<u8>(this, TS::bumpEnvMapEnabled) != 0 {
        // The game code copies the floats with integer moves.
        for (state, member) in [
            (D3DTSS_BUMPENVMAT00, TS::bumpEnvMap00),
            (D3DTSS_BUMPENVMAT01, TS::bumpEnvMap01),
            (D3DTSS_BUMPENVMAT10, TS::bumpEnvMap10),
            (D3DTSS_BUMPENVMAT11, TS::bumpEnvMap11),
            (D3DTSS_BUMPENVLSCALE, TS::lumaScale),
            (D3DTSS_BUMPENVLOFFSET, TS::lumaOffset),
        ] {
            let value: u32 = rd(this, member);
            render_state::set_texture_stage_state_with_save(rs, rd(this, TS::stage), state, value, 0);
        }
    }
    1
}

/// True when `PrepareTextureForRendering` makes or fills a device texture for `texture`. This
/// function makes the same virtual calls as that function. Those calls only read.
unsafe fn texture_needs_device_work(texture: usize) -> bool {
    let data: usize = rd(texture, NiTexture::rendererData);
    if data == 0 {
        return true;
    }
    let getter = |slot: usize| transmute::<usize, unsafe extern "thiscall" fn(usize) -> usize>(virtual_at(data, slot))(data);
    if getter(0x20) != 0 {
        // A rendered texture. The function reads only its D3D texture.
        return false;
    }
    getter(0x18) == 0 || rd::<u8>(texture, NiSourceTexture::isStatic) == 0
}

/// No call is a skip. The constructor allocates nothing. It can run two times on the same
/// memory.
unsafe fn texture_stage_ctor_regions(set: &mut Regions, this: usize) {
    set.add(this, TS::SIZE);
    set.add(NI_REF_OBJECT_COUNT, 4);
}

/// The call is a skip when the texture of the stage loses its last reference, because the
/// release deletes the texture.
unsafe fn texture_stage_dtor_regions(set: &mut Regions, this: usize) {
    set.add(this, TS::SIZE);
    set.add(NI_REF_OBJECT_COUNT, 4);
    let texture: usize = rd(this, TS::ptrTexture);
    if texture != 0 {
        // The last reference: the count is 1 and the release deletes the texture.
        if rd::<u32>(texture, REF_COUNT) == 1 {
            return set.skip();
        }
        set.add(texture, 8);
    }
}

/// The call is a skip when the texture manager must make or fill the device texture of the
/// stage. That work cannot run two times.
unsafe fn texture_stage_update_renderer_regions(set: &mut Regions, this: usize, renderer: usize) {
    // The stage changes only when the hook of the Code Patch writes its combiner again.
    set.add(this, TS::SIZE);
    set.add(rd(renderer, R::renderState), RS::SIZE);
    let texture: usize = rd(this, TS::ptrTexture);
    if texture != 0 && texture_needs_device_work(texture) {
        set.skip();
    }
}

// ---------------------------------------------------------------------------------------------
// NiDX8ConfigurableTexturePipeline
// ---------------------------------------------------------------------------------------------

/// 0x6B8DA0 `NiDX8ConfigurableTexturePipeline::ctor`
pub unsafe fn ctor(this: usize, renderer: usize, caps: usize, rs: usize, texture_manager: usize, _unused: u8) -> usize {
    crate::shadow::touch(&ctor_port::STATE);
    wr(this, CTP::vtbl, VTBL_NI_REF_OBJECT);
    wr::<u32>(this, CTP::refCount, 0);
    let count = g(NI_REF_OBJECT_COUNT);
    wr::<u32>(count, 0, rd::<u32>(count, 0).wrapping_add(1));
    wr::<usize>(this, CTP_CURRENT_PASS, 0);
    wr::<usize>(this, CTP::combiners, 0);
    eh_vector_constructor_iterator(this + CTP_ALPHA_TEMPLATES, 4, 10, NI_POINTER_CTOR, NI_POINTER_DTOR);
    wr(this, CTP::vtbl, VTBL_PIPELINE);
    wr(this, CTP::renderer, renderer);
    wr(renderer, R::configurableTexturePipeline, this);
    for i in 0..0x35 {
        wr::<u32>(this, CTP::d3dCaps + i * 4, rd(caps, i * 4));
    }
    for stage in CTP_STAGES {
        wr::<usize>(this, stage, 0);
    }
    let device: usize = rd(renderer, R::d3dDevice);
    wr(this, CTP::device, device);
    device_add_ref(device);
    wr(this, CTP::textureManager, texture_manager);
    wr(this, CTP::renderState, rs);
    wr::<u32>(this, CTP_FLAGS, 0);
    wr::<u32>(this, CTP::field_100, 0);
    wr::<u32>(this, CTP::field_310, 0);
    wr::<u32>(this, CTP::field_B04, 0);
    init_alpha_state_templates(this);
    let memory = engine::operator_new(TP::SIZE);
    let pass = if memory != 0 { texture_pass_ctor(memory, rd(this, CTP::renderer)) } else { 0 };
    pointer_assign(this + CTP::combiners, pass);
    this
}

/// 0x6B8EE0 `NiDX8ConfigurableTexturePipeline::dtor`
pub unsafe fn dtor(this: usize) {
    crate::shadow::touch(&dtor_port::STATE);
    wr(this, CTP::vtbl, VTBL_PIPELINE);
    let rs: usize = rd(this, CTP::renderState);
    for stage in 0..8 {
        wr::<usize>(rs, RS::texturePerStage + stage * 4, 0);
    }
    wr::<usize>(this, CTP::textureManager, 0);
    for member in CTP_STAGES {
        let stage: usize = rd(this, member);
        if stage != 0 {
            delete_object(stage);
        }
    }
    let device: usize = rd(this, CTP::device);
    wr::<usize>(this, CTP::renderer, 0);
    if device != 0 {
        device_release(device);
    }
    eh_vector_destructor_iterator(this + CTP_ALPHA_TEMPLATES, 4, 10, NI_POINTER_DTOR);
    let combiners: usize = rd(this, CTP::combiners);
    if combiners != 0 {
        release(combiners);
    }
    let pass: usize = rd(this, CTP_CURRENT_PASS);
    if pass != 0 {
        release(pass);
    }
    wr(this, CTP::vtbl, VTBL_NI_REF_OBJECT);
    let count = g(NI_REF_OBJECT_COUNT);
    wr::<u32>(count, 0, rd::<u32>(count, 0).wrapping_sub(1));
}

/// 0x6B8FF0 `NiDX8ConfigurableTexturePipeline::create`
pub unsafe fn create(renderer: usize, caps: usize, rs: usize, texture_manager: usize, flag: u8) -> usize {
    crate::shadow::touch(&create_port::STATE);
    engine::debug_print0(STR_CREATION_ATTEMPT);
    let memory = engine::operator_new(CTP::SIZE);
    let this = if memory != 0 { ctor(memory, renderer, caps, rs, texture_manager, flag) } else { 0 };
    if flag != 0 {
        wr::<u32>(this, CTP_FLAGS, rd::<u32>(this, CTP_FLAGS) | 0x40);
    }
    init_filtering_and_address_caps(this);
    init_pipeline_state(this);
    let rs: usize = rd(this, CTP::renderState);
    for stage in 0..8 {
        wr::<usize>(rs, RS::texturePerStage + stage * 4, 0);
    }
    engine::debug_print0(STR_CREATION_DONE);
    this
}

/// A new `NiAlphaProperty`, as the game code builds it inline for the templates 0 to 8.
unsafe fn new_alpha_template() -> usize {
    let property = engine::operator_new(AP::SIZE);
    if property != 0 {
        ni_object_net_ctor(property);
        wr::<u16>(property, AP::flags, 0);
        wr(property, AP::vtbl, VTBL_NI_ALPHA_PROPERTY);
        property_set_bit(property, 0, 0);
        flags_set_field(property, 6, 0xF, 1);
        flags_set_field(property, 7, 0xF, 5);
        property_set_bit(property, 0, 9);
        flags_set_field(property, 0, 7, 0xA);
        wr::<u8>(property, AP::alphaTestRef, 0);
    }
    property
}

/// The game code builds the last template with other inline code.
unsafe fn new_last_alpha_template() -> usize {
    let property = engine::operator_new(AP::SIZE);
    if property != 0 {
        ni_object_net_ctor(property);
        wr::<u16>(property, AP::flags, 0);
        wr(property, AP::vtbl, VTBL_NI_ALPHA_PROPERTY);
        property_set_bit(property, 0, 0);
        flags_set_field(property, 6, 0xF, 1);
        let flags = rd::<u16>(property, AP::flags);
        wr::<u16>(property, AP::flags, (flags & 0xFEFF) | 0xE0);
        property_set_bit(property, 0, 9);
        wr::<u8>(property, AP::flags + 1, rd::<u8>(property, AP::flags + 1) & 0xE3);
        wr::<u8>(property, AP::alphaTestRef, 0);
    }
    property
}

/// The changes that `initAlphaStateTemplates` makes to template `index` after it stored the
/// template.
unsafe fn adjust_alpha_template(index: usize, property: usize) {
    let word = |f: fn(u16) -> u16| wr::<u16>(property, AP::flags, f(rd::<u16>(property, AP::flags)));
    let low = |f: fn(u8) -> u8| wr::<u8>(property, AP::flags, f(rd::<u8>(property, AP::flags)));
    match index {
        0 | 8 => {
            word(|w| w | 1);
            low(|b| b & 0xE1);
            word(|w| w & 0xFE1F);
        }
        1 | 6 => {
            word(|w| w | 1);
            word(|w| (w & 0xFFED) | 0xC);
            word(|w| (w & 0xFEFF) | 0xE0);
        }
        2 | 4 => {
            word(|w| w | 1);
            word(|w| (w & 0xFFE3) | 2);
            word(|w| (w & 0xFE5F) | 0x40);
            if index == 4 {
                wr::<u8>(property, AP::flags + 1, rd::<u8>(property, AP::flags + 1) | 2);
                word(|w| (w & 0xF3FF) | 0x1000);
                wr::<u8>(property, AP::alphaTestRef, 0x80);
            }
        }
        _ => low(|b| b & 0xFE),
    }
}

/// Marks the places in [`BLEND_TEMPLATES`] that take the value of `field_104`.
const SIGNED_OP: u32 = 0xFFFF_FFFF;

/// The writes at the end of `initAlphaStateTemplates` (0x6B99AD to 0x6B9C02), in the order of
/// the game code. Each entry is an offset in the pipeline and a value.
#[rustfmt::skip]
const BLEND_TEMPLATES: [(u16, u32); 96] = [
    (0x108, 2), (0x10C, 2), (0x110, 0), (0x114, 2), (0x118, 2), (0x11C, 0),
    (0x120, 4), (0x124, 2), (0x128, 0), (0x12C, 4), (0x130, 2), (0x134, 0),
    (0x138, 4), (0x144, 4), (0x13C, 2), (0x140, 0), (0x148, 2), (0x14C, 0),
    (0x168, 4), (0x174, 4), (0x16C, 2), (0x170, 0), (0x178, 2), (0x17C, 0),
    (0x198, 4), (0x19C, 2), (0x1A0, 0), (0x1A4, 2), (0x1A8, 2), (0x1AC, 0),
    (0x1C8, SIGNED_OP), (0x1CC, 2), (0x1D0, 1), (0x1D4, 1), (0x1D8, 2), (0x1DC, 1),
    (0x228, 4), (0x22C, 2), (0x230, 1), (0x234, 4), (0x238, 2), (0x23C, 1),
    (0x210, 0xD), (0x214, 2), (0x218, 1), (0x21C, 1), (0x220, 2), (0x224, 1),
    (0x1F8, 7), (0x1FC, 2), (0x200, 1), (0x204, 1), (0x208, 2), (0x20C, 1),
    (0x240, 5), (0x244, 2), (0x248, 1), (0x24C, 1), (0x250, 2), (0x254, 1),
    (0x258, 2), (0x25C, 2), (0x260, 1), (0x264, 2), (0x268, 2), (0x26C, 1),
    (0x270, 2), (0x274, 2), (0x278, 1), (0x27C, 3), (0x280, 2), (0x284, 1),
    (0x288, 4), (0x28C, 2), (0x290, 1), (0x294, 2), (0x298, 2), (0x29C, 1),
    (0x2A0, 4), (0x2A4, 2), (0x2A8, 1), (0x2AC, 3), (0x2B0, 2), (0x2B4, 1),
    (0x2B8, SIGNED_OP), (0x2BC, 2), (0x2C0, 1), (0x2C4, 2), (0x2C8, 2), (0x2CC, 1),
    (0x2D0, 4), (0x2D4, 2), (0x2D8, 1), (0x2DC, 4), (0x2E0, 2), (0x2E4, 1),
];

unsafe fn write_blend_templates(this: usize) {
    for (offset, value) in BLEND_TEMPLATES {
        let value = if value == SIGNED_OP { rd(this, CTP_SIGNED_OP) } else { value };
        wr::<u32>(this, offset as usize, value);
    }
}

/// 0x6B90E0 `NiDX8ConfigurableTexturePipeline::initAlphaStateTemplates`. Also fills the
/// filter remapper and the address remapper with default values. The game code leaves 4 in
/// EAX.
pub unsafe fn init_alpha_state_templates(this: usize) {
    crate::shadow::touch(&init_alpha_state_templates_port::STATE);
    for i in 0..CTP_BLOCK_314_DWORDS {
        wr::<u32>(this, CTP_BLOCK_314 + i * 4, 0);
    }
    wr::<u8>(this, CTP_BYTE_AFC, 0);
    for entry in 0..6 {
        wr::<u32>(g(FILTER_MINIFY), entry * 12, 1);
        wr::<u32>(g(FILTER_MAGNIFY), entry * 12, 1);
        wr::<u32>(g(FILTER_MIP), entry * 12, 0);
    }
    for (offset, value) in [(0x18, 1u32), (0x1C, 1), (0x10, 1), (0x14, 3), (0x08, 3), (0x0C, 1), (0x00, 3), (0x04, 3)] {
        wr::<u32>(g(ADDRESS_REMAP_U), offset, value);
    }

    for index in 0..9 {
        let property = new_alpha_template();
        pointer_assign(this + CTP_ALPHA_TEMPLATES + index * 4, property);
        adjust_alpha_template(index, rd(this, CTP_ALPHA_TEMPLATES + index * 4));
    }

    let texture_op_caps: u32 = rd(this, CTP::d3dCaps_TextureOpCaps);
    let signed_op: u32 = if texture_op_caps & 0x40_0000 != 0 {
        0x17
    } else if texture_op_caps & 0x20_0000 != 0 {
        0x16
    } else {
        0
    };
    wr(this, CTP_SIGNED_OP, signed_op);

    let property = new_last_alpha_template();
    pointer_assign(this + CTP_ALPHA_TEMPLATES + 9 * 4, property);
    adjust_alpha_template(9, rd(this, CTP_ALPHA_TEMPLATES + 9 * 4));

    write_blend_templates(this);
}

/// 0x6B9C20 `NiDX8ConfigurableTexturePipeline::buildPass`: a jump to slot 8 of the pass.
pub unsafe fn build_pass(
    this: usize,
    property_state: usize,
    effect_state: usize,
    has_normals: u8,
    texture_sets: u32,
    geometry_data: usize,
    skin_instance: usize,
    world_transform: usize,
    world_bound: usize,
) -> u32 {
    crate::shadow::touch(&build_pass_port::STATE);
    let pass: usize = rd(this, CTP_CURRENT_PASS);
    let f: unsafe extern "thiscall" fn(usize, usize, usize, u32, u32, usize, usize, usize, usize) -> u32 = transmute(virtual_at(pass, 8));
    f(pass, property_state, effect_state, has_normals as u32, texture_sets, geometry_data, skin_instance, world_transform, world_bound)
}

/// 0x6B9C30 `NiDX8ConfigurableTexturePipeline::loadBinary`: a jump to slot 0xC of the pass.
pub unsafe fn load_binary(this: usize, a: u32, b: u32, c: u32, d: u32) -> u32 {
    crate::shadow::touch(&load_binary_port::STATE);
    let pass: usize = rd(this, CTP_CURRENT_PASS);
    let f: unsafe extern "thiscall" fn(usize, u32, u32, u32, u32) -> u32 = transmute(virtual_at(pass, 0xC));
    f(pass, a, b, c, d)
}

/// 0x6B9C40 `NiDX8ConfigurableTexturePipeline::selectAndLinkPass`. Takes the pass of the
/// renderer-specific property, or the fixed-function pass. Then it calls slot 0x10 of that
/// pass.
pub unsafe fn select_and_link_pass(
    this: usize,
    property_state: usize,
    effect_state: usize,
    has_normals: u8,
    texture_sets: u32,
    geometry_data: usize,
    skin_instance: usize,
    transform: usize,
    world_bound: usize,
) -> u32 {
    crate::shadow::touch(&select_and_link_pass_port::STATE);
    let mut changed = false;
    let mut own_pass: usize = 0;
    let specific = property_state_renderer_specific(property_state);
    if specific != 0 {
        let data: usize = rd(specific, NiRendererSpecificProperty::renderData);
        if data != 0 {
            let type_id = transmute::<usize, unsafe extern "thiscall" fn(usize) -> u32>(virtual_at(data, 4))(data);
            if type_id == RENDERER_TYPE_DX8 {
                let data: usize = rd(specific, NiRendererSpecificProperty::renderData);
                if data != 0 {
                    own_pass = rd(data, RENDER_DATA_TEXTURE_PASS);
                    let now: usize = rd(this, CTP_CURRENT_PASS);
                    if now != own_pass {
                        if now != 0 {
                            release(now);
                        }
                        wr(this, CTP_CURRENT_PASS, own_pass);
                        if own_pass != 0 {
                            wr::<u32>(own_pass, 4, rd::<u32>(own_pass, 4).wrapping_add(1));
                        }
                        changed = true;
                    }
                }
            }
        }
    }
    if own_pass == 0 {
        let now: usize = rd(this, CTP_CURRENT_PASS);
        if now != rd::<usize>(this, CTP::combiners) {
            if now != 0 {
                release(now);
            }
            let combiners: usize = rd(this, CTP::combiners);
            wr(this, CTP_CURRENT_PASS, combiners);
            if combiners != 0 {
                wr::<u32>(combiners, 4, rd::<u32>(combiners, 4).wrapping_add(1));
            }
            changed = true;
        }
    }
    if changed {
        let pass: usize = rd(this, CTP_CURRENT_PASS);
        transmute::<usize, unsafe extern "thiscall" fn(usize) -> u32>(virtual_at(pass, 0x18))(pass);
        wr::<u32>(this, CTP::field_B04, 0);
    }
    let pass: usize = rd(this, CTP_CURRENT_PASS);
    let f: unsafe extern "thiscall" fn(usize, usize, usize, u32, u32, usize, usize, usize, usize) -> u32 = transmute(virtual_at(pass, 0x10));
    f(pass, property_state, effect_state, has_normals as u32, texture_sets, geometry_data, skin_instance, transform, world_bound)
}

/// 0x6B9D30 `NiDX8ConfigurableTexturePipeline::ensureAndRegisterStreamables`. Takes the
/// fixed-function pass when there is no pass. Then it jumps to slot 0x14 of the pass.
pub unsafe fn ensure_and_register_streamables(
    this: usize,
    property_state: usize,
    effect_state: usize,
    has_normals: u8,
    texture_sets: u32,
    geometry_data: usize,
    skin_instance: usize,
    transform: usize,
    world_bound: usize,
) -> u32 {
    crate::shadow::touch(&ensure_and_register_streamables_port::STATE);
    if rd::<usize>(this, CTP_CURRENT_PASS) == 0 && rd::<usize>(this, CTP::combiners) != 0 {
        let combiners: usize = rd(this, CTP::combiners);
        wr(this, CTP_CURRENT_PASS, combiners);
        if combiners != 0 {
            wr::<u32>(combiners, 4, rd::<u32>(combiners, 4).wrapping_add(1));
        }
    }
    let pass: usize = rd(this, CTP_CURRENT_PASS);
    let f: unsafe extern "thiscall" fn(usize, usize, usize, u32, u32, usize, usize, usize, usize) -> u32 = transmute(virtual_at(pass, 0x14));
    f(pass, property_state, effect_state, has_normals as u32, texture_sets, geometry_data, skin_instance, transform, world_bound)
}

/// 0x6B9D80 `NiDX8ConfigurableTexturePipeline::setCameraData`. The location is a `vec3` by
/// value. The game code copies it with integer moves. The axes go through the FPU.
pub unsafe fn set_camera_data(this: usize, direction: usize, up: usize, right: usize, x: u32, y: u32, z: u32) {
    crate::shadow::touch(&set_camera_data_port::STATE);
    for (column, axis) in [(CTP::worldRightX, right), (CTP::worldUpX, up), (CTP::worldDirectionX, direction)] {
        for row in 0..3 {
            wr::<u32>(this, column + row * 12, x87_pass(rd(axis, row * 4)));
        }
    }
    wr(this, CTP::worldLocation_x, x);
    wr(this, CTP::worldLocation_y, y);
    wr(this, CTP::worldLocation_z, z);
}

/// 0x6B9E00 `NiDX8ConfigurableTexturePipeline::nextFrame`. EAX is the value that the pass
/// leaves.
pub unsafe fn next_frame(this: usize) {
    crate::shadow::touch(&next_frame_port::STATE);
    texture_pass_next_frame(rd(this, CTP_CURRENT_PASS));
    wr::<u32>(this, CTP::field_B04, 0);
}

/// 0x6B9E20 `NiDX8ConfigurableTexturePipeline::setStageAddressingMode`
pub unsafe fn set_stage_addressing_mode(_this: usize, clamp_mode: u32, stage: usize) {
    crate::shadow::touch(&set_stage_addressing_mode_port::STATE);
    let entry = clamp_mode.wrapping_mul(8) as usize;
    wr::<u32>(stage, TS::addressModeU, rd(g(ADDRESS_REMAP_U).wrapping_add(entry), 0));
    wr::<u32>(stage, TS::addressModeV, rd(g(ADDRESS_REMAP_V).wrapping_add(entry), 0));
}

/// 0x6B9E40 `NiDX8ConfigurableTexturePipeline::setStageFilterMode`
pub unsafe fn set_stage_filter_mode(_this: usize, filter_mode: u32, stage: usize) {
    crate::shadow::touch(&set_stage_filter_mode_port::STATE);
    wr(stage, TS::textureStageIndex, filter_mode);
    let entry = filter_mode.wrapping_mul(3).wrapping_shl(2) as usize;
    wr::<u32>(stage, TS::filter_3C, rd(g(FILTER_MAGNIFY).wrapping_add(entry), 0));
    wr::<u32>(stage, TS::filter_40, rd(g(FILTER_MINIFY).wrapping_add(entry), 0));
    wr::<u32>(stage, TS::filter_44, rd(g(FILTER_MIP).wrapping_add(entry), 0));
}

/// 0x6B9E90 `NiDX8ConfigurableTexturePipeline::buildSkinPartitionPass`: a jump to slot 4 of
/// the pass.
pub unsafe fn build_skin_partition_pass(
    this: usize,
    property_state: usize,
    effect_state: usize,
    has_normals: u8,
    texture_sets: u32,
    geometry_data: usize,
    skin_instance: usize,
    partition: usize,
    transform: usize,
    world_bound: usize,
) -> u32 {
    crate::shadow::touch(&build_skin_partition_pass_port::STATE);
    let pass: usize = rd(this, CTP_CURRENT_PASS);
    let f: unsafe extern "thiscall" fn(usize, usize, usize, u32, u32, usize, usize, usize, usize, usize) -> u32 = transmute(virtual_at(pass, 4));
    f(pass, property_state, effect_state, has_normals as u32, texture_sets, geometry_data, skin_instance, partition, transform, world_bound)
}

/// The colour operation, the alpha operation and their arguments, for the three stages of the
/// default pass.
const DEFAULT_STAGES: [[u32; 6]; 3] = [[2, 2, 0, 2, 2, 0], [1, 2, 1, 1, 2, 1], [1, 1, 0, 1, 1, 0]];

/// 0x6B9EA0 `NiDX8ConfigurableTexturePipeline::initPipelineState`
pub unsafe fn init_pipeline_state(this: usize) {
    crate::shadow::touch(&init_pipeline_state_port::STATE);
    wr(rd::<usize>(this, CTP::combiners), TP::configurableTexturePipeline, this);
    wr::<usize>(rd::<usize>(this, CTP::combiners), TP::renderer, rd(this, CTP::renderer));

    let rs = || rd::<usize>(this, CTP::renderState);
    for (stage, values) in [(0u32, [2u32, 2, 0, 2, 2, 0]), (1, [2, 2, 1, 2, 2, 1])] {
        for (index, value) in values.into_iter().enumerate() {
            tss_write(rs(), stage, D3DTSS_COLOROP + index as u32, value);
        }
    }
    for stage in 2u32..8 {
        tss_write(rs(), stage, D3DTSS_COLOROP, 1);
        tss_write(rs(), stage, D3DTSS_COLORARG1, 2);
        tss_set_value(rs(), stage, D3DTSS_COLORARG2, 1);
        tss_set_value(rs(), stage, D3DTSS_ALPHAOP, 1);
        tss_set_value(rs(), stage, D3DTSS_ALPHAARG1, 2);
        tss_set_value(rs(), stage, D3DTSS_ALPHAARG2, 1);
    }
    for stage in 0u32..8 {
        let state = rs();
        let slot = state + RS::texturePerStage + stage as usize * 4;
        if rd::<usize>(slot, 0) != 0 {
            wr::<usize>(slot, 0, 0);
            device_set_texture(rd(state, RS::device), stage, 0);
        }
        for (tss, value) in [
            (D3DTSS_BUMPENVMAT00, HALF),
            (D3DTSS_BUMPENVMAT01, 0),
            (D3DTSS_BUMPENVMAT10, 0),
            (D3DTSS_BUMPENVMAT11, HALF),
            (D3DTSS_BUMPENVLSCALE, ONE),
            (D3DTSS_BUMPENVLOFFSET, 0),
            (D3DTSS_TEXCOORDINDEX, 0),
            // The game code sets ADDRESSU two times and does not set ADDRESSV.
            (D3DTSS_ADDRESSU, 1),
            (D3DTSS_ADDRESSU, 1),
            (D3DTSS_MAGFILTER, 1),
            (D3DTSS_MINFILTER, 1),
            (D3DTSS_MIPFILTER, 0),
        ] {
            tss_set_value(rs(), stage, tss, value);
        }
        render_state::set_texture_stage_state_with_save(rs(), stage, D3DTSS_TEXTURETRANSFORMFLAGS, 0, 0);
    }

    let groups: usize = rd(rd::<usize>(this, CTP::combiners), TP::TArray_18_storage);
    if rd::<usize>(groups, 0) == 0 {
        let memory = engine::operator_new(STAGE_GROUP_SIZE);
        let group = if memory != 0 { engine::texture_pass_stage_group_ctor(memory) } else { 0 };
        for (index, values) in DEFAULT_STAGES.into_iter().enumerate() {
            let member = CTP_STAGES[index];
            let stage = || rd::<usize>(this, member);
            let memory = engine::operator_new(TS::SIZE);
            let new_stage = if memory != 0 { texture_stage_ctor(memory) } else { 0 };
            wr(this, member, new_stage);
            wr::<u32>(new_stage, TS::stage, index as u32);
            wr::<u32>(stage(), TS::colorOp, values[0]);
            wr::<u32>(stage(), TS::colorArg1, values[1]);
            wr::<u32>(stage(), TS::colorArg2, values[2]);
            wr::<u32>(stage(), TS::alphaOp, values[3]);
            wr::<u32>(stage(), TS::alphaArg1, values[4]);
            wr::<u32>(stage(), TS::alphaArg2, values[5]);
            ni_pointer_set(stage() + TS::ptrTexture, 0);
            wr::<u32>(stage(), TS::sourceTexcoordIndex, 0);
            wr::<u32>(stage(), TS::addressModeU, 1);
            wr::<u32>(stage(), TS::addressModeV, 1);
            set_stage_filter_mode(this, 0, stage());
            wr::<u32>(stage(), TS::textureTransformFlags, 0);
            wr::<u8>(stage(), TS::bumpEnvMapEnabled, 0);
            stage_group_set_stage_at_index(group, index as u32, stage());
        }
        texture_pass_set_stage_object(rd(this, CTP::combiners), 0, group);
    }

    for member in CTP_STAGES {
        texture_stage_update_renderer(rd(this, member), rd(this, CTP::renderer));
        stage_set_texture_filtering(rd(this, member), rd(this, CTP::renderer), 1, 1);
        stage_set_texture_wrapping(rd(this, member), rd(this, CTP::renderer));
    }
    render_state::set_render_state(rs(), D3DRS_WRAP0, 0, 0);
    render_state::set_render_state(rs(), D3DRS_WRAP1, 0, 0);
    ni_pointer_assign(this + CTP_CURRENT_PASS, this + CTP::combiners);

    wr::<u32>(g(G_MAX_VERTEX_SHADER_CONST), 0, rd(this, CTP::d3dCaps_MaxVertexShaderConst));
    wr::<u32>(g(G_VERTEX_SHADER_VERSION), 0, rd(this, CTP::d3dCaps_VertexShaderVersion));
    wr::<u32>(g(G_MAX_SIMULTANEOUS_TEXTURES), 0, rd(this, CTP::d3dCaps_MaxSimultaneousTextures));
    let blend_stages: u32 = rd(this, CTP::d3dCaps_MaxTextureBlendStages);
    wr(g(G_MAX_TEXTURE_BLEND_STAGES), 0, blend_stages);
    if blend_stages > 8 {
        wr::<u32>(g(G_MAX_TEXTURE_BLEND_STAGES), 0, 8);
    }
    wr::<u32>(g(G_MAX_STREAMS), 0, rd(this, CTP::d3dCaps_MaxStreams));
    wr::<u32>(g(G_MAX_STREAM_STRIDE), 0, rd(this, CTP::d3dCaps_MaxStreamStride));
    wr::<u32>(g(G_PIXEL_SHADER_VERSION), 0, rd(this, CTP::d3dCaps_PixelShaderVersion));
    wr::<u32>(g(G_MAX_PIXEL_SHADER_VALUE), 0, rd(this, CTP::d3dCaps_MaxPixelShaderValue));
}

/// 0x6BA840 `NiDX8ConfigurableTexturePipeline::initFilteringAndAddressCaps`
pub unsafe fn init_filtering_and_address_caps(this: usize) {
    crate::shadow::touch(&init_filtering_and_address_caps_port::STATE);
    wr::<u32>(g(FILTER_MAGNIFY), 0, 1);
    wr::<u32>(g(FILTER_MINIFY), 0, 1);
    wr::<u32>(g(FILTER_MIP), 0, 0);
    // Entries 1 to 5 of the remapper, in the order of the game code.
    for (offset, value) in [
        (0x10, 2u32), (0x0C, 2), (0x14, 0),
        (0x1C, 2), (0x18, 2), (0x20, 2),
        (0x28, 1), (0x24, 1), (0x2C, 1),
        (0x34, 1), (0x30, 1), (0x38, 2),
        (0x40, 2), (0x3C, 2), (0x44, 1),
    ] {
        wr::<u32>(g(FILTER_REMAP), offset, value);
    }

    let address_caps: u32 = rd(this, CTP::d3dCaps_TextureAddressCaps);
    let flags: u32 = rd(this, CTP_FLAGS);
    wr(this, CTP_FLAGS, if address_caps & 0x10 != 0 { flags | 1 } else { flags & !1 });
    let flags: u32 = rd(this, CTP_FLAGS);
    wr(this, CTP_FLAGS, if address_caps & 4 != 0 { flags | 0x80 } else { flags & !0x80 });
    let texture_caps: u32 = rd(this, CTP::d3dCaps_TextureCaps);
    let flags: u32 = rd(this, CTP_FLAGS);
    if texture_caps & 0x400 != 0 {
        wr::<u32>(this, CTP::field_310, 0x103);
        wr(this, CTP_FLAGS, flags | 0x100);
    } else {
        wr::<u32>(this, CTP::field_310, 2);
        wr(this, CTP_FLAGS, flags & !0x100);
    }
}

// ---------------------------------------------------------------------------------------------
// Regions
// ---------------------------------------------------------------------------------------------

/// True when `pass` is of the fixed-function pass class. Slots 4, 0x10 and 0x14 of that class
/// return 0. Slot 0x18 of that class writes only the pass.
unsafe fn is_fixed_function_pass(pass: usize) -> bool {
    pass != 0 && rd::<usize>(pass, 0) == VTBL_NI_DX8_TEXTURE_PASS
}

/// The call is a skip when the property state has a pass of another class, because this file
/// does not know the memory that such a pass writes. The call is a skip when `combiners` is
/// not of the fixed-function pass class. The call is a skip when the pass in use loses its
/// last reference.
unsafe fn select_and_link_pass_regions(set: &mut Regions, this: usize, property_state: usize) {
    let specific: usize = rd(property_state, NiPropertyState::propRendererSpecific);
    if specific != 0 && rd::<usize>(specific, NiRendererSpecificProperty::renderData) != 0 {
        // A pass of another class. This file does not know the memory that it writes.
        return set.skip();
    }
    let now: usize = rd(this, CTP_CURRENT_PASS);
    let combiners: usize = rd(this, CTP::combiners);
    if !is_fixed_function_pass(combiners) {
        return set.skip();
    }
    if now != combiners && now != 0 {
        // The last reference: the count is 1 and the release deletes the pass.
        if rd::<u32>(now, REF_COUNT) == 1 {
            return set.skip();
        }
        set.add(now, 8);
    }
    set.add(this, CTP::SIZE);
    set.add(combiners, TP::SIZE);
}

/// The call is a skip when the pass that the function calls is not of the fixed-function
/// pass class.
unsafe fn ensure_and_register_streamables_regions(set: &mut Regions, this: usize) {
    let now: usize = rd(this, CTP_CURRENT_PASS);
    let pass = if now != 0 { now } else { rd(this, CTP::combiners) };
    if !is_fixed_function_pass(pass) {
        return set.skip();
    }
    set.add(this, CTP::SIZE);
    set.add(pass, TP::SIZE);
}

/// The call is a skip when the pass in use is not of the fixed-function pass class.
unsafe fn pass_call_regions(set: &mut Regions, this: usize) {
    let pass: usize = rd(this, CTP_CURRENT_PASS);
    if !is_fixed_function_pass(pass) {
        return set.skip();
    }
    set.add(this, CTP::SIZE);
    set.add(pass, TP::SIZE);
}

/// The call is a skip when the call makes the stage group and the three stages. The call is
/// a skip when the assignment at the end can delete the pass in use. The call is a skip when
/// the texture manager must make or fill the device texture of a stage.
unsafe fn init_pipeline_state_regions(set: &mut Regions, this: usize) {
    let combiners: usize = rd(this, CTP::combiners);
    if rd::<usize>(rd::<usize>(combiners, TP::TArray_18_storage), 0) == 0 {
        // This call makes the stage group and the three stages.
        return set.skip();
    }
    let now: usize = rd(this, CTP_CURRENT_PASS);
    if now != 0 && now != combiners {
        // The assignment at the end can delete the pass in use.
        return set.skip();
    }
    for member in CTP_STAGES {
        let stage: usize = rd(this, member);
        let texture: usize = rd(stage, TS::ptrTexture);
        if texture != 0 {
            if texture_needs_device_work(texture) {
                return set.skip();
            }
            // The hook of the Code Patch in `updateRenderer` can write the stage.
            set.add(stage, TS::SIZE);
        }
    }
    set.add(this, CTP::SIZE);
    set.add(rd(this, CTP::renderState), RS::SIZE);
    set.add(combiners, TP::SIZE);
    set.add(G_VERTEX_SHADER_VERSION, 0x10);
    set.add(G_MAX_PIXEL_SHADER_VALUE, 8);
    set.add(G_MAX_TEXTURE_BLEND_STAGES, 8);
}

/// No call is a skip. The function writes the pipeline and the filter remapper.
unsafe fn init_filtering_and_address_caps_regions(set: &mut Regions, this: usize) {
    set.add(this, CTP::SIZE);
    set.add(FILTER_REMAP, FILTER_REMAP_SIZE);
}

// ---------------------------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------------------------

port_thiscall!(texture_stage_ctor_port, 0x6C1930, "NiDX8TextureStage::ctor",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| texture_stage_ctor_regions(set, this),
    rust = texture_stage_ctor);

port_thiscall!(texture_stage_dtor_port, 0x6C1A70, "NiDX8TextureStage::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| texture_stage_dtor_regions(set, this),
    rust = texture_stage_dtor);

port_thiscall!(texture_stage_update_renderer_port, 0x6C1B00, "NiDX8TextureStage::updateRenderer",
    fn(this, renderer: usize) -> u8,
    compare_eax = true,
    regions = |set| texture_stage_update_renderer_regions(set, this, renderer),
    rust = texture_stage_update_renderer);

port_thiscall!(ctor_port, 0x6B8DA0, "NiDX8ConfigurableTexturePipeline::ctor",
    fn(this, renderer: usize, caps: usize, rs: usize, texture_manager: usize, unused: u8) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = ctor);

port_thiscall!(dtor_port, 0x6B8EE0, "NiDX8ConfigurableTexturePipeline::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = dtor);

port_cdecl!(create_port, 0x6B8FF0, "NiDX8ConfigurableTexturePipeline::create",
    fn(renderer: usize, caps: usize, rs: usize, texture_manager: usize, flag: u8) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = create);

port_thiscall!(init_alpha_state_templates_port, 0x6B90E0, "NiDX8ConfigurableTexturePipeline::initAlphaStateTemplates",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = init_alpha_state_templates);

port_thiscall!(build_pass_port, 0x6B9C20, "NiDX8ConfigurableTexturePipeline::buildPass",
    fn(this, property_state: usize, effect_state: usize, has_normals: u8, texture_sets: u32, geometry_data: usize,
       skin_instance: usize, world_transform: usize, world_bound: usize) -> u32,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = build_pass);

port_thiscall!(load_binary_port, 0x6B9C30, "NiDX8ConfigurableTexturePipeline::loadBinary",
    fn(this, a: u32, b: u32, c: u32, d: u32) -> u32,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = load_binary);

port_thiscall!(select_and_link_pass_port, 0x6B9C40, "NiDX8ConfigurableTexturePipeline::selectAndLinkPass",
    fn(this, property_state: usize, effect_state: usize, has_normals: u8, texture_sets: u32, geometry_data: usize,
       skin_instance: usize, transform: usize, world_bound: usize) -> u32,
    compare_eax = true,
    regions = |set| select_and_link_pass_regions(set, this, property_state),
    rust = select_and_link_pass);

port_thiscall!(ensure_and_register_streamables_port, 0x6B9D30, "NiDX8ConfigurableTexturePipeline::ensureAndRegisterStreamables",
    fn(this, property_state: usize, effect_state: usize, has_normals: u8, texture_sets: u32, geometry_data: usize,
       skin_instance: usize, transform: usize, world_bound: usize) -> u32,
    compare_eax = true,
    regions = |set| ensure_and_register_streamables_regions(set, this),
    rust = ensure_and_register_streamables);

port_thiscall!(set_camera_data_port, 0x6B9D80, "NiDX8ConfigurableTexturePipeline::setCameraData",
    fn(this, direction: usize, up: usize, right: usize, x: u32, y: u32, z: u32) -> (),
    compare_eax = false,
    regions = |set| set.add(this, CTP::SIZE),
    rust = set_camera_data);

port_thiscall!(next_frame_port, 0x6B9E00, "NiDX8ConfigurableTexturePipeline::nextFrame",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| pass_call_regions(set, this),
    rust = next_frame);

port_thiscall!(set_stage_addressing_mode_port, 0x6B9E20, "NiDX8ConfigurableTexturePipeline::setStageAddressingMode",
    fn(this, clamp_mode: u32, stage: usize) -> (),
    compare_eax = false,
    regions = |set| set.add(stage, TS::SIZE),
    rust = set_stage_addressing_mode);

port_thiscall!(set_stage_filter_mode_port, 0x6B9E40, "NiDX8ConfigurableTexturePipeline::setStageFilterMode",
    fn(this, filter_mode: u32, stage: usize) -> (),
    compare_eax = false,
    regions = |set| set.add(stage, TS::SIZE),
    rust = set_stage_filter_mode);

port_thiscall!(build_skin_partition_pass_port, 0x6B9E90, "NiDX8ConfigurableTexturePipeline::buildSkinPartitionPass",
    fn(this, property_state: usize, effect_state: usize, has_normals: u8, texture_sets: u32, geometry_data: usize,
       skin_instance: usize, partition: usize, transform: usize, world_bound: usize) -> u32,
    compare_eax = true,
    regions = |set| pass_call_regions(set, this),
    rust = build_skin_partition_pass);

port_thiscall!(init_pipeline_state_port, 0x6B9EA0, "NiDX8ConfigurableTexturePipeline::initPipelineState",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| init_pipeline_state_regions(set, this),
    rust = init_pipeline_state);

port_thiscall!(init_filtering_and_address_caps_port, 0x6BA840, "NiDX8ConfigurableTexturePipeline::initFilteringAndAddressCaps",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| init_filtering_and_address_caps_regions(set, this),
    rust = init_filtering_and_address_caps);

pub static PORTS: &[&crate::shadow::PortState] = &[
    &texture_stage_ctor_port::STATE,
    &texture_stage_dtor_port::STATE,
    &texture_stage_update_renderer_port::STATE,
    &ctor_port::STATE,
    &dtor_port::STATE,
    &create_port::STATE,
    &init_alpha_state_templates_port::STATE,
    &build_pass_port::STATE,
    &load_binary_port::STATE,
    &select_and_link_pass_port::STATE,
    &ensure_and_register_streamables_port::STATE,
    &set_camera_data_port::STATE,
    &next_frame_port::STATE,
    &set_stage_addressing_mode_port::STATE,
    &set_stage_filter_mode_port::STATE,
    &build_skin_partition_pass_port::STATE,
    &init_pipeline_state_port::STATE,
    &init_filtering_and_address_caps_port::STATE,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen::layouts::NiDX8ConfigurableTexturePipeline_BlendOps as BO;
    use std::cell::{RefCell, UnsafeCell};

    const ARENA: usize = 0x300;

    thread_local! {
        /// The globals of the game, one copy for each test thread.
        static GLOBALS: UnsafeCell<[u8; ARENA]> = const { UnsafeCell::new([0; ARENA]) };
        static DEVICE_LOG: RefCell<Vec<[u32; 4]>> = const { RefCell::new(Vec::new()) };
        static PASS_LOG: RefCell<Vec<(usize, u32)>> = const { RefCell::new(Vec::new()) };
    }

    pub fn global(addr: usize) -> usize {
        let offset = match addr {
            0x7DD950..=0x7DDBFF => addr - 0x7DD950,
            0x7C3834..=0x7C383B => 0x2B0 + addr - 0x7C3834,
            NI_REF_OBJECT_COUNT => 0x2C0,
            _ => panic!("global {addr:#x} is not mapped"),
        };
        GLOBALS.with(|block| block.get() as usize + offset)
    }

    // A device that records SetTransform (37), SetRenderState (50), SetTexture (61) and
    // SetTextureStageState (63).
    unsafe extern "system" fn dev_set3(_device: usize, a: u32, b: u32) -> u32 {
        DEVICE_LOG.with(|log| log.borrow_mut().push([3, a, b, 0]));
        0
    }
    unsafe extern "system" fn dev_set_tss(_device: usize, stage: u32, state: u32, value: u32) -> u32 {
        DEVICE_LOG.with(|log| log.borrow_mut().push([4, stage, state, value]));
        0
    }

    struct Device {
        _vtable: Box<[usize; 80]>,
        object: Box<usize>,
    }

    fn device() -> Device {
        let mut vtable = Box::new([0usize; 80]);
        vtable[0x94 / 4] = dev_set3 as *const () as usize;
        vtable[0xC8 / 4] = dev_set3 as *const () as usize;
        vtable[0xF4 / 4] = dev_set3 as *const () as usize;
        vtable[0xFC / 4] = dev_set_tss as *const () as usize;
        let object = Box::new(vtable.as_ptr() as usize);
        DEVICE_LOG.with(|log| log.borrow_mut().clear());
        Device { _vtable: vtable, object }
    }

    fn device_log() -> Vec<[u32; 4]> {
        DEVICE_LOG.with(|log| log.borrow().clone())
    }

    // A pass with a vtable of its own.
    unsafe extern "thiscall" fn pass_reset(pass: usize) -> u32 {
        PASS_LOG.with(|log| log.borrow_mut().push((pass, 0x18)));
        0
    }
    unsafe extern "thiscall" fn pass_link(pass: usize, _ps: usize, _es: usize, normals: u32, sets: u32, _g: usize, _s: usize, _t: usize, _b: usize) -> u32 {
        PASS_LOG.with(|log| log.borrow_mut().push((pass, 0x10)));
        normals * 100 + sets
    }
    unsafe extern "thiscall" fn pass_register(pass: usize, _ps: usize, _es: usize, _n: u32, _sets: u32, _g: usize, _s: usize, _t: usize, _b: usize) -> u32 {
        PASS_LOG.with(|log| log.borrow_mut().push((pass, 0x14)));
        77
    }
    unsafe extern "thiscall" fn pass_build(_pass: usize, _ps: usize, _es: usize, _n: u32, sets: u32, _g: usize, _s: usize, _t: usize, bound: usize) -> u32 {
        sets + bound as u32
    }
    unsafe extern "thiscall" fn pass_blend(_pass: usize, a: u32, b: u32, c: u32, d: u32) -> u32 {
        a + b * 10 + c * 100 + d * 1000
    }
    unsafe extern "thiscall" fn pass_skin(_pass: usize, _ps: usize, _es: usize, _n: u32, _sets: u32, _g: usize, _s: usize, partition: usize, _t: usize, bound: usize) -> u32 {
        (partition + bound) as u32
    }
    unsafe extern "thiscall" fn type_id_7(_data: usize) -> u32 {
        7
    }

    fn pass_vtable() -> Box<[usize; 8]> {
        Box::new([0, pass_skin as *const () as usize, pass_build as *const () as usize, pass_blend as *const () as usize, pass_link as *const () as usize, pass_register as *const () as usize, pass_reset as *const () as usize, 0])
    }

    // A replacement for the hook of the Code Patch. It stores the registers and the stack
    // arguments.
    #[no_mangle]
    static mut NIRENDER_TP_PROBE: [u32; 11] = [0; 11];

    core::arch::global_asm!(
        ".global _nirender_tp_probe",
        "_nirender_tp_probe:",
        "mov dword ptr [_NIRENDER_TP_PROBE], eax",
        "mov dword ptr [_NIRENDER_TP_PROBE + 4], ecx",
        "mov dword ptr [_NIRENDER_TP_PROBE + 8], edx",
        "mov dword ptr [_NIRENDER_TP_PROBE + 12], ebx",
        "mov dword ptr [_NIRENDER_TP_PROBE + 16], esi",
        "mov dword ptr [_NIRENDER_TP_PROBE + 20], edi",
        "mov dword ptr [_NIRENDER_TP_PROBE + 24], ebp",
        "mov eax, [esp + 4]",
        "mov dword ptr [_NIRENDER_TP_PROBE + 28], eax",
        "mov eax, [esp + 8]",
        "mov dword ptr [_NIRENDER_TP_PROBE + 32], eax",
        "mov eax, [esp + 12]",
        "mov dword ptr [_NIRENDER_TP_PROBE + 36], eax",
        "mov eax, [esp + 16]",
        "mov dword ptr [_NIRENDER_TP_PROBE + 40], eax",
        "ret 16",
    );

    extern "C" {
        fn nirender_tp_probe();
    }

    #[test]
    fn the_hooked_call_gets_the_registers_of_the_game_code() {
        unsafe {
            texcoord_index_call(nirender_tp_probe as *const () as usize, 0x1111, 0x2222, 0x3333, 0x4444, 5, 0x30000);
            let probe = core::ptr::addr_of!(NIRENDER_TP_PROBE).read();
            // The registers: EAX stage, ECX and EBP render state, EDX value, EBX renderer, ESI
            // stage object, EDI render state at entry. Then the stack arguments: stage,
            // TEXCOORDINDEX, value, no save.
            assert_eq!(probe, [5, 0x1111, 0x30000, 0x4444, 0x3333, 0x2222, 0x1111, 5, 11, 0x30000, 0]);
        }
    }

    #[test]
    fn stage_ctor_sets_the_defaults() {
        let mut stage = [0xAAu8; TS::SIZE];
        let s = stage.as_mut_ptr() as usize;
        unsafe {
            wr::<u32>(g(FILTER_MAGNIFY), 0, 11);
            wr::<u32>(g(FILTER_MINIFY), 0, 12);
            wr::<u32>(g(FILTER_MIP), 0, 13);
            wr::<u32>(g(NI_REF_OBJECT_COUNT), 0, 5);
            assert_eq!(texture_stage_ctor(s), s);
            assert_eq!(rd::<u32>(g(NI_REF_OBJECT_COUNT), 0), 6);
            assert_eq!(rd::<usize>(s, TS::vtbl), VTBL_TEXTURE_STAGE);
            assert_eq!(rd::<[u32; 2]>(s, TS::refCount), [0, 0]);
            assert_eq!(rd::<usize>(s, TS::ptrTexture), 0);
            assert_eq!(rd::<[u32; 6]>(s, TS::colorOp), [2, 2, 0, 2, 2, 0]);
            assert_eq!(rd::<[u32; 4]>(s, TS::sourceTexcoordIndex), [0, 3, 3, 3]);
            assert_eq!(rd::<[u32; 5]>(s, TS::textureStageIndex), [0, 11, 12, 13, 0]);
            assert_eq!(rd::<[u32; 16]>(s, TS::textureMatrix), [0; 16]);
            assert_eq!(rd::<u32>(s, TS::extraBlendColorArgument), 0xFFFF_FFFF);
            // The constructor writes one byte here: the instruction at 0x6C19F0 has 7 bytes.
            assert_eq!(rd::<u8>(s, TS::field_90), 1);
            assert_eq!(rd::<u32>(s, TS::field_94), 9);
            assert_eq!(rd::<u8>(s, TS::bumpEnvMapEnabled), 0);
            // The constructor does not write the padding after the bool.
            assert_eq!(rd::<u8>(s, TS::bumpEnvMapEnabled + 1), 0xAA);
            assert_eq!(rd::<[f32; 6]>(s, TS::bumpEnvMap00), [0.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
            assert_eq!(rd::<[u32; 2]>(s, TS::texturePropMap), [0, 0]);

            texture_stage_dtor(s);
            assert_eq!(rd::<usize>(s, TS::vtbl), VTBL_NI_REF_OBJECT);
            assert_eq!(rd::<u32>(g(NI_REF_OBJECT_COUNT), 0), 5);
        }
    }

    #[test]
    fn update_renderer_without_a_texture_sends_only_what_changed() {
        let device = device();
        let mut rs = vec![0u8; RS::SIZE];
        let mut renderer = vec![0u8; R::SIZE];
        let mut stage = [0u8; TS::SIZE];
        let (rs, r, s) = (rs.as_mut_ptr() as usize, renderer.as_mut_ptr() as usize, stage.as_mut_ptr() as usize);
        unsafe {
            wr(rs, RS::device, &*device.object as *const usize as usize);
            wr(r, R::renderState, rs);
            texture_stage_ctor(s);
            wr::<u32>(s, TS::stage, 2);
            wr::<u32>(s, TS::colorOp, 4);
            wr::<u32>(s, TS::extraBlendColorArgument, 0x1234);
            wr::<usize>(rs, RS::texturePerStage + 8, 0xBEEF);
            // The cache of stage 2 has COLORARG1 = 2 before the call.
            wr::<u32>(rs, RS::maybe_textureStageStateSettings + (2 * 32 + 2) * 8, 2);

            assert_eq!(texture_stage_update_renderer(s, r), 1);
            assert_eq!(
                device_log(),
                [[3, 2, 0, 0], [4, 2, 1, 4], [4, 2, 4, 2], [4, 2, 5, 2], [3, D3DRS_TEXTUREFACTOR, 0x1234, 0]]
            );
            assert_eq!(rd::<usize>(rs, RS::texturePerStage + 8), 0);
            assert_eq!(rd::<u32>(rs, RS::maybe_textureStageStateSettings + (2 * 32 + 1) * 8), 4);
            assert_eq!(rd::<u32>(rs, RS::renderStateSettings + 60 * 8), 0x1234);

            // The second call finds all the values in the cache. Bump mapping adds its 6
            // states.
            DEVICE_LOG.with(|log| log.borrow_mut().clear());
            wr::<u8>(s, TS::bumpEnvMapEnabled, 1);
            wr::<f32>(s, TS::bumpEnvMap00, 0.5);
            assert_eq!(texture_stage_update_renderer(s, r), 1);
            assert_eq!(device_log(), [[4, 2, 7, HALF], [4, 2, 22, ONE]]);
        }
    }

    #[test]
    fn stage_modes_come_from_the_remappers() {
        let mut stage = [0u8; TS::SIZE];
        let s = stage.as_mut_ptr() as usize;
        unsafe {
            for (i, base) in [FILTER_MINIFY, FILTER_MAGNIFY, FILTER_MIP].into_iter().enumerate() {
                wr::<u32>(g(base), 3 * 12, 100 + i as u32);
            }
            set_stage_filter_mode(0, 3, s);
            assert_eq!(rd::<[u32; 4]>(s, TS::textureStageIndex), [3, 101, 100, 102]);

            wr::<u32>(g(ADDRESS_REMAP_U), 2 * 8, 7);
            wr::<u32>(g(ADDRESS_REMAP_V), 2 * 8, 9);
            set_stage_addressing_mode(0, 2, s);
            assert_eq!(rd::<[u32; 2]>(s, TS::addressModeU), [7, 9]);
        }
    }

    #[test]
    fn filtering_caps_set_the_flags_and_the_remapper() {
        let mut pipeline = vec![0u8; CTP::SIZE];
        let p = pipeline.as_mut_ptr() as usize;
        unsafe {
            wr::<u32>(p, CTP_FLAGS, 0x40 | 0x100 | 0x80);
            wr::<u32>(p, CTP::d3dCaps_TextureAddressCaps, 0x10);
            init_filtering_and_address_caps(p);
            assert_eq!(rd::<u32>(p, CTP_FLAGS), 0x41);
            assert_eq!(rd::<u32>(p, CTP::field_310), 2);
            assert_eq!(
                rd::<[u32; 18]>(g(FILTER_REMAP), 0),
                [1, 1, 0, 2, 2, 0, 2, 2, 2, 1, 1, 1, 1, 1, 2, 2, 2, 1]
            );

            wr::<u32>(p, CTP::d3dCaps_TextureAddressCaps, 4);
            wr::<u32>(p, CTP::d3dCaps_TextureCaps, 0x400);
            init_filtering_and_address_caps(p);
            assert_eq!(rd::<u32>(p, CTP_FLAGS), 0x1C0);
            assert_eq!(rd::<u32>(p, CTP::field_310), 0x103);
        }
    }

    #[test]
    fn camera_data_is_stored_by_column() {
        let mut pipeline = vec![0u8; CTP::SIZE];
        let p = pipeline.as_mut_ptr() as usize;
        let (direction, up, right) = ([1.0f32, 2.0, 3.0], [4.0f32, 5.0, 6.0], [7.0f32, 8.0, 9.0]);
        unsafe {
            set_camera_data(p, direction.as_ptr() as usize, up.as_ptr() as usize, right.as_ptr() as usize, 10, 11, 12);
            assert_eq!(rd::<[f32; 9]>(p, CTP::worldRightX), [7.0, 4.0, 1.0, 8.0, 5.0, 2.0, 9.0, 6.0, 3.0]);
            assert_eq!(rd::<f32>(p, CTP::worldRightY), 8.0);
            assert_eq!(rd::<f32>(p, CTP::worldRightZ), 9.0);
            assert_eq!(rd::<[u32; 3]>(p, CTP::worldLocation), [10, 11, 12]);
        }
    }

    #[test]
    fn alpha_templates_get_their_flags() {
        // 0x00EC is the value that the inline constructor leaves: source alpha, inverse source
        // alpha.
        let expected = [0x0001u16, 0x00ED, 0x0043, 0x00EC, 0x1243, 0x00EC, 0x00ED, 0x00EC, 0x0001, 0x00EC];
        for (index, flags) in expected.into_iter().enumerate() {
            let mut property = [0u8; AP::SIZE];
            let p = property.as_mut_ptr() as usize;
            unsafe {
                wr::<u16>(p, AP::flags, 0x00EC);
                adjust_alpha_template(index, p);
                assert_eq!(rd::<u16>(p, AP::flags), flags, "template {index}");
                assert_eq!(rd::<u8>(p, AP::alphaTestRef), if index == 4 { 0x80 } else { 0 });
            }
        }
    }

    #[test]
    fn blend_templates_cover_their_block_once() {
        let mut pipeline = vec![0xEEu8; CTP::SIZE];
        let p = pipeline.as_mut_ptr() as usize;
        unsafe {
            wr::<u32>(p, CTP_SIGNED_OP, 0x17);
            write_blend_templates(p);
            let slot = |table: usize, index: usize, member: usize| table + index * BO::SIZE + member;
            assert_eq!(rd::<u32>(p, slot(CTP::firstStageBlendOps, 8, BO::colorOp)), 0x17);
            assert_eq!(rd::<u32>(p, slot(CTP::laterStageBlendOps, 8, BO::colorOp)), 0x17);
            assert_eq!(rd::<[u32; 3]>(p, slot(CTP::laterStageBlendOps, 1, BO::colorOp)), [0xD, 2, 1]);
            assert_eq!(rd::<[u32; 3]>(p, slot(CTP::laterStageBlendOps, 9, BO::alphaOp)), [4, 2, 1]);
            // The blocks that the game code does not write.
            for untouched in [0x150, 0x164, 0x180, 0x194, 0x1B0, 0x1C4, 0x1E0, 0x1F4] {
                assert_eq!(rd::<u32>(p, untouched), 0xEEEE_EEEE, "{untouched:#x}");
            }
        }
        let written: Vec<u16> = BLEND_TEMPLATES.iter().map(|e| e.0).collect();
        assert_eq!(written.len(), 96);
        let mut unique = written.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 96);
    }

    #[test]
    fn select_and_link_pass_falls_back_to_the_combiners() {
        let vtable = pass_vtable();
        let mut combiners = [vtable.as_ptr() as usize, 1usize];
        let mut other = [vtable.as_ptr() as usize, 2usize];
        let (c, o) = (combiners.as_mut_ptr() as usize, other.as_mut_ptr() as usize);
        let mut pipeline = vec![0u8; CTP::SIZE];
        let p = pipeline.as_mut_ptr() as usize;
        let property_state = [0u8; 0x40];
        let ps = property_state.as_ptr() as usize;
        unsafe {
            PASS_LOG.with(|log| log.borrow_mut().clear());
            wr(p, CTP::combiners, c);
            wr(p, CTP_CURRENT_PASS, o);
            wr::<u32>(p, CTP::field_B04, 9);
            assert_eq!(select_and_link_pass(p, ps, 0, 1, 3, 0, 0, 0, 0), 103);
            assert_eq!(rd::<usize>(p, CTP_CURRENT_PASS), c);
            assert_eq!((combiners[1], other[1]), (2, 1));
            assert_eq!(rd::<u32>(p, CTP::field_B04), 0);
            assert_eq!(PASS_LOG.with(|log| log.borrow().clone()), [(c, 0x18), (c, 0x10)]);

            // The pass does not change. Thus there is no reset.
            wr::<u32>(p, CTP::field_B04, 9);
            select_and_link_pass(p, ps, 0, 0, 0, 0, 0, 0, 0);
            assert_eq!(rd::<u32>(p, CTP::field_B04), 9);
            assert_eq!(PASS_LOG.with(|log| log.borrow().len()), 3);
        }
    }

    #[test]
    fn select_and_link_pass_takes_the_pass_of_the_property() {
        let vtable = pass_vtable();
        let mut combiners = [vtable.as_ptr() as usize, 1usize];
        let mut own = [vtable.as_ptr() as usize, 1usize];
        let (c, o) = (combiners.as_mut_ptr() as usize, own.as_mut_ptr() as usize);
        let data_vtable = [0usize, type_id_7 as *const () as usize];
        let data = [data_vtable.as_ptr() as usize, o];
        let mut specific = [0u8; NiRendererSpecificProperty::SIZE];
        let mut property_state = [0u8; 0x40];
        let mut pipeline = vec![0u8; CTP::SIZE];
        let p = pipeline.as_mut_ptr() as usize;
        unsafe {
            PASS_LOG.with(|log| log.borrow_mut().clear());
            wr(specific.as_mut_ptr() as usize, NiRendererSpecificProperty::renderData, data.as_ptr() as usize);
            wr(property_state.as_mut_ptr() as usize, NiPropertyState::propRendererSpecific, specific.as_ptr() as usize);
            wr(p, CTP::combiners, c);
            wr(p, CTP_CURRENT_PASS, c);
            combiners[1] = 2;
            select_and_link_pass(p, property_state.as_ptr() as usize, 0, 0, 5, 0, 0, 0, 0);
            assert_eq!(rd::<usize>(p, CTP_CURRENT_PASS), o);
            assert_eq!((combiners[1], own[1]), (1, 2));
            assert_eq!(PASS_LOG.with(|log| log.borrow().clone()), [(o, 0x18), (o, 0x10)]);
        }
    }

    #[test]
    fn the_forwarders_call_their_slot() {
        let vtable = pass_vtable();
        let mut combiners = [vtable.as_ptr() as usize, 1usize];
        let c = combiners.as_mut_ptr() as usize;
        let mut pipeline = vec![0u8; CTP::SIZE];
        let p = pipeline.as_mut_ptr() as usize;
        unsafe {
            PASS_LOG.with(|log| log.borrow_mut().clear());
            wr(p, CTP::combiners, c);
            assert_eq!(ensure_and_register_streamables(p, 0, 0, 0, 0, 0, 0, 0, 0), 77);
            assert_eq!(rd::<usize>(p, CTP_CURRENT_PASS), c);
            assert_eq!(combiners[1], 2);
            assert_eq!(ensure_and_register_streamables(p, 0, 0, 0, 0, 0, 0, 0, 0), 77);
            assert_eq!(combiners[1], 2);
            assert_eq!(build_pass(p, 0, 0, 1, 40, 0, 0, 0, 2), 42);
            assert_eq!(load_binary(p, 1, 2, 3, 4), 4321);
            assert_eq!(build_skin_partition_pass(p, 0, 0, 0, 0, 0, 0, 30, 0, 3), 33);
        }
    }
}
