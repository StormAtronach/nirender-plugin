//! `NiDX8TexturePass`, first half: construction, the reset for each frame, the array of
//! sub-passes, and the pixel shader and the vertex shader of the current sub-pass.
//!
//! A texture pass owns an array of sub-pass objects (`NiDX8TexturePass::$1`, 0x54 bytes). A
//! sub-pass holds a maximum of 8 texture stages and the shaders that it draws with. The
//! second half of the class is in `texture_pass_b`.

use core::mem::transmute;

use crate::d3d::{device_add_ref, device_release, device_set_pixel_shader, device_set_vertex_shader, D3DTSS_TCI_CAMERASPACEPOSITION};
use crate::engine::{
    self, eh_vector_constructor_iterator, eh_vector_destructor_iterator, ni_object_release, ni_pointer_set, operator_delete,
    FILTER_REMAP_MAGNIFY, FILTER_REMAP_MINIFY, FILTER_REMAP_MIP, MAX_SIMULTANEOUS_TEXTURES, MAX_TEXTURE_BLEND_STAGES, NI_REF_OBJECT_COUNT,
    VTBL_NI_DX8_TEXTURE_PASS, VTBL_NI_REF_OBJECT,
};
use crate::gen::layouts::{
    NiDX8RenderState as RS, NiDX8Renderer as R, NiDX8TexturePass as TP, NiDX8TexturePass__1 as SP, NiDX8TextureStage as TS, NiPixelData,
    NiPixelFormat,
};
use crate::mem::{rd, wr};
use crate::ports::ni::{self, release_and_clear, REF_COUNT};
use crate::shadow::Regions;

// ---- engine data at fixed addresses ----------------------------------------------------

/// The vtable of the array of sub-passes (`TArray_18`).
const VTBL_SUBPASS_ARRAY: u32 = 0x74F66C;
/// The texture coordinate callbacks, one for each `NiTextureEffect` coordinate mode. They are
/// in the second half of the class.
const TEX_GEN_CALLBACKS: [u32; 5] = [0x6B5CE0, 0x6B5EB0, 0x6B6090, 0x6B61B0, 0x6B6300];

/// Member of the shader object of a sub-pass (`$1::field_4C`, `$1::field_50`): the D3D handle.
const SHADER_HANDLE: usize = 0x20;
/// The sub-pass members with the pixel and the vertex shader object.
const SUBPASS_PIXEL_SHADER: usize = SP::field_4C;
const SUBPASS_VERTEX_SHADER: usize = SP::field_50;
/// `$1::field_C`: -1 until the sub-pass has a pixel shader.
const SUBPASS_SLOTS_USED_1: usize = SP::field_C;
const SUBPASS_SLOTS_USED_2: usize = SP::field_10;

const STAGES_PER_BANK: usize = 8;

// ---- engine functions at fixed addresses -----------------------------------------------
//
// `engine` has no function for these addresses.

const TEXTURE_STAGE_CTOR: usize = 0x6C1930;
const TEXTURE_STAGE_DTOR: usize = 0x6C1A70;

/// 0x6C1930 `NiDX8TextureStage::ctor`
unsafe fn texture_stage_ctor(stage: usize) -> usize {
    transmute::<usize, unsafe extern "thiscall" fn(usize) -> usize>(TEXTURE_STAGE_CTOR)(stage)
}

/// 0x6C1A70 `NiDX8TextureStage::dtor`
unsafe fn texture_stage_dtor(stage: usize) {
    transmute::<usize, unsafe extern "thiscall" fn(usize)>(TEXTURE_STAGE_DTOR)(stage)
}

/// 0x6B7990 `NiDX8TexturePass::_1::getAvailableSlotCounts`
unsafe fn subpass_get_available_slot_counts(subpass: usize) -> (u32, u32) {
    let mut out = [0u32; 2];
    let f: unsafe extern "thiscall" fn(usize, *mut u32, *mut u32) = transmute(0x6B7990usize);
    f(subpass, out.as_mut_ptr(), out.as_mut_ptr().add(1));
    (out[0], out[1])
}

/// 0x6DE940 `NiSourceTexture_static::CreateFromPixelData`
unsafe fn source_texture_create_from_pixel_data(pixel_data: usize, prefs: *const u32) -> usize {
    transmute::<usize, unsafe extern "C" fn(usize, *const u32) -> usize>(0x6DE940)(pixel_data, prefs)
}

/// Calls `NiObject::release` (0x404630) for the object that a member points to, then sets the
/// member to null. `ni::release_and_clear` is the inline form and does not call 0x404630.
unsafe fn release_call_and_clear(member: usize) {
    let object: usize = rd(member, 0);
    if object != 0 {
        ni_object_release(object);
        wr::<u32>(member, 0, 0);
    }
}

// ---- the array of sub-passes -----------------------------------------------------------
//
// The array code is a template that the compiler put inline in each function. The helpers
// below are the pieces that are the same in every copy.

#[inline(always)]
unsafe fn slot(this: usize, index: u32) -> usize {
    rd::<usize>(this, TP::TArray_18_storage) + index as usize * 4
}

/// Sets every slot below the end index to null. The port reads the storage pointer and the end
/// index again on each pass, as the game code does.
unsafe fn array_null_used_slots(this: usize) {
    if rd::<u32>(this, TP::TArray_18_endIndex) > 0 {
        let mut index = 0u32;
        loop {
            let storage: usize = rd(this, TP::TArray_18_storage);
            index += 1;
            wr::<u32>(storage, (index - 1) as usize * 4, 0);
            if index >= rd::<u32>(this, TP::TArray_18_endIndex) {
                break;
            }
        }
    }
}

/// Gives the array a storage of `size` slots, when that is not its size now.
unsafe fn array_set_size(this: usize, size: u32) {
    if size == rd::<u32>(this, TP::TArray_18_storageCount) {
        return;
    }
    if size < rd::<u32>(this, TP::TArray_18_endIndex) {
        let mut index = size;
        loop {
            let entry = slot(this, index);
            if rd::<u32>(entry, 0) != 0 {
                wr::<u32>(entry, 0, 0);
                wr(this, TP::TArray_18_filledCount, rd::<u32>(this, TP::TArray_18_filledCount).wrapping_sub(1));
            }
            index += 1;
            if index >= rd::<u32>(this, TP::TArray_18_endIndex) {
                break;
            }
        }
        wr(this, TP::TArray_18_endIndex, size);
    }
    let old: usize = rd(this, TP::TArray_18_storage);
    wr(this, TP::TArray_18_storageCount, size);
    let new = engine::operator_new(size.wrapping_mul(4) as usize);
    wr(this, TP::TArray_18_storage, new);
    let mut index = 0u32;
    while index < rd::<u32>(this, TP::TArray_18_endIndex) {
        let to: usize = rd(this, TP::TArray_18_storage);
        wr::<u32>(to, index as usize * 4, rd(old, index as usize * 4));
        index += 1;
    }
    operator_delete(old);
}

/// Sets slot `index` to null. This is the "set at" of the array with a null value.
unsafe fn array_clear_slot(this: usize, index: u32) {
    if index < rd::<u32>(this, TP::TArray_18_storageCount) {
        if index >= rd::<u32>(this, TP::TArray_18_endIndex) {
            wr(this, TP::TArray_18_endIndex, index + 1);
        } else if rd::<u32>(slot(this, index), 0) != 0 {
            wr(this, TP::TArray_18_filledCount, rd::<u32>(this, TP::TArray_18_filledCount).wrapping_sub(1));
        }
        wr::<u32>(slot(this, index), 0, 0);
    }
}

/// Sets the slots from `first` up to `count_10` to null.
unsafe fn array_clear_slots_from(this: usize, first: u32) {
    let mut index = first;
    while index < rd::<u32>(this, TP::count_10) {
        array_clear_slot(this, index);
        index += 1;
    }
}

/// The "set at" of the array for a value that can be null.
unsafe fn array_set_at(this: usize, index: u32, value: usize) {
    if index >= rd::<u32>(this, TP::TArray_18_storageCount) {
        return;
    }
    let filled: u32 = rd(this, TP::TArray_18_filledCount);
    if index >= rd::<u32>(this, TP::TArray_18_endIndex) {
        wr(this, TP::TArray_18_endIndex, index + 1);
        if value != 0 {
            wr(this, TP::TArray_18_filledCount, filled.wrapping_add(1));
        }
    } else if value != 0 {
        if rd::<u32>(slot(this, index), 0) == 0 {
            wr(this, TP::TArray_18_filledCount, filled.wrapping_add(1));
        }
    } else if rd::<u32>(slot(this, index), 0) != 0 {
        wr(this, TP::TArray_18_filledCount, filled.wrapping_sub(1));
    }
    wr(slot(this, index), 0, value);
}

/// `new NiDX8TexturePass::$1`
unsafe fn new_subpass() -> usize {
    let block = engine::operator_new(SP::SIZE);
    if block != 0 {
        engine::texture_pass_stage_group_ctor(block)
    } else {
        0
    }
}

/// The start of the three "acquire" functions. The current sub-pass becomes the one in the slot
/// `field_C`. The function makes the sub-pass when the slot is empty.
unsafe fn ensure_current_subpass(this: usize) -> bool {
    if rd::<usize>(this, TP::substruct_14) != 0 {
        return false;
    }
    let existing: usize = rd(slot(this, rd(this, TP::field_C)), 0);
    wr(this, TP::substruct_14, existing);
    if existing != 0 {
        return false;
    }
    let subpass = new_subpass();
    wr(this, TP::substruct_14, subpass);
    array_set_at(this, rd(this, TP::field_C), subpass);
    true
}

/// The end of the two functions that go to the next sub-pass. `field_C` increases by 1. The
/// array grows by 2 slots when it is at its end. The function makes the sub-pass of the slot
/// when the slot is empty.
unsafe fn advance_to_next_subpass(this: usize) {
    let index = rd::<u32>(this, TP::field_C).wrapping_add(1);
    let count: u32 = rd(this, TP::count_10);
    wr(this, TP::field_C, index);
    if index >= count {
        let size = count.wrapping_add(2);
        wr(this, TP::count_10, size);
        array_set_size(this, size);
        array_clear_slots_from(this, rd(this, TP::field_C));
    }
    let existing: usize = rd(slot(this, rd(this, TP::field_C)), 0);
    wr(this, TP::substruct_14, existing);
    if existing == 0 {
        let subpass = new_subpass();
        array_set_at(this, rd(this, TP::field_C), subpass);
        wr(this, TP::substruct_14, subpass);
    }
}

// ---- the ports -------------------------------------------------------------------------

/// The reset of one stage of the banks `field_2468` and `field_1E84`.
unsafe fn reset_generated_stage(stage: usize, texcoord_index: u32, transform_flags: u32, field_94: u32) {
    release_and_clear(stage + TS::ptrTexture);
    wr::<u32>(stage, TS::addressModeU, 3);
    wr::<u32>(stage, TS::addressModeV, 3);
    wr::<u32>(stage, TS::textureStageIndex, 0);
    wr::<u32>(stage, TS::sourceTexcoordIndex, texcoord_index);
    wr::<u32>(stage, TS::textureTransformFlags, transform_flags);
    wr::<u8>(stage, TS::field_90, 0);
    wr::<u32>(stage, TS::texturePropMap, 0);
    wr::<u32>(stage, TS::field_94, field_94);
    core::ptr::write_bytes((stage + TS::textureMatrix) as *mut u8, 0, 0x40);
}

/// 0x6B2FA0 `NiDX8TexturePass::resetForNewFrame`
pub unsafe fn reset_for_new_frame(this: usize) {
    crate::shadow::touch(&reset_for_new_frame_port::STATE);
    wr::<u32>(this, TP::field_8, 0);
    wr::<u32>(this, TP::field_C, 0);
    wr::<u32>(this, TP::count_10, 2);
    wr::<u32>(this, TP::substruct_14, 0);
    array_null_used_slots(this);
    wr::<u32>(this, TP::TArray_18_endIndex, 0);
    wr::<u32>(this, TP::TArray_18_filledCount, 0);
    // The end index is 0 here. Thus the part of the resize that drops slots past the new size
    // cannot run. That part is 0x6B2FE2..0x6B3006 in the game code.
    array_set_size(this, rd(this, TP::count_10));
    array_clear_slots_from(this, 0);

    wr::<u32>(this, TP::field_30, 0);
    wr::<u32>(this, TP::field_34, 0);
    wr::<u32>(this, TP::field_38, 0);
    wr::<u32>(this, TP::field_3C, 0);
    wr::<u32>(this, TP::activeStageCount, 0);
    wr::<u32>(this, TP::field_6E0, 0);
    for index in 0..5 {
        wr::<u32>(this, TP::callbacks_6E4 + index * 4, 0);
    }
    wr::<u32>(this, TP::field_CD8, 0);
    release_and_clear(this + TP::texture_CDC);
    wr::<u32>(this, TP::field_18A0, 0);
    wr::<u32>(this, TP::field_2464, 0);
    wr::<u32>(this, TP::field_2A48, 0);
    wr::<u8>(this, TP::field_2B08, 0);
    wr::<u8>(this, TP::field_2BC8, 0);
    wr::<u32>(this, TP::field_2BD0, 0);
    wr::<u32>(this, TP::renderState, 0);
    wr::<u32>(this, TP::configurableTexturePipeline, 0);
    wr::<u32>(this, TP::textureManager, 0);
    wr::<u32>(this, TP::deviceInterface, 0);
    wr::<u32>(this, TP::renderer, 0);

    let base = this + TP::field_624;
    wr::<u32>(base, TS::stage, 0);
    release_and_clear(base + TS::ptrTexture);
    wr::<u32>(base, TS::colorArg1, 2);
    wr::<u32>(base, TS::colorOp, 4);
    wr::<u32>(base, TS::colorArg2, 0);
    wr::<u32>(base, TS::alphaOp, 4);
    wr::<u32>(base, TS::alphaArg1, 2);
    wr::<u32>(base, TS::alphaArg2, 0);
    wr::<u32>(base, TS::sourceTexcoordIndex, 0);
    wr::<u32>(base, TS::addressModeU, 1);
    wr::<u32>(base, TS::addressModeV, 1);
    wr::<u32>(base, TS::field_34, 1);
    wr::<u32>(base, TS::textureStageIndex, 0);
    wr::<u32>(base, TS::filter_3C, rd(FILTER_REMAP_MAGNIFY, 0));
    wr::<u32>(base, TS::filter_40, rd(FILTER_REMAP_MINIFY, 0));
    wr::<u32>(base, TS::filter_44, rd(FILTER_REMAP_MIP, 0));
    wr::<u32>(base, TS::textureTransformFlags, 0);
    core::ptr::write_bytes((base + TS::textureMatrix) as *mut u8, 0, 0x40);
    wr::<u32>(base, TS::extraBlendColorArgument, 0);
    wr::<u8>(base, TS::field_90, 1);
    wr::<u32>(base, TS::field_94, 2);
    wr(base, TS::refCount, rd::<u32>(base, TS::refCount).wrapping_add(1));

    wr::<u32>(this, TP::field_2A4C_field_94, 0);
    wr::<u32>(this, TP::field_2B0C_field_94, 1);
    for index in 0..STAGES_PER_BANK {
        wr::<u32>(this + TP::field_6F8 + index * TS::SIZE, TS::field_94, 0);
    }
    for index in 0..STAGES_PER_BANK {
        reset_generated_stage(this + TP::field_2468 + index * TS::SIZE, 0, 0, 2);
    }
    for index in 0..STAGES_PER_BANK {
        let stage = this + TP::field_CE0 + index * TS::SIZE;
        release_and_clear(stage + TS::ptrTexture);
        wr::<u32>(stage, TS::field_94, 0);
    }
    for index in 0..STAGES_PER_BANK {
        let stage = this + TP::field_18A4 + index * TS::SIZE;
        release_and_clear(stage + TS::ptrTexture);
        wr::<u32>(stage, TS::field_94, 2);
    }
    for index in 0..STAGES_PER_BANK {
        reset_generated_stage(this + TP::field_1E84 + index * TS::SIZE, D3DTSS_TCI_CAMERASPACEPOSITION, 2, 0);
    }
    wr::<u32>(this, TP::field_2464, 0);
    for (index, callback) in TEX_GEN_CALLBACKS.into_iter().enumerate() {
        wr::<u32>(this, TP::callbacks_6E4 + index * 4, callback);
    }
}

/// 0x6B33A0 `NiDX8TexturePass::ctor`
pub unsafe fn ctor(this: usize, renderer: usize) -> usize {
    crate::shadow::touch(&ctor_port::STATE);
    wr::<usize>(this, TP::vtbl, VTBL_NI_REF_OBJECT);
    wr::<u32>(this, TP::refCount, 0);
    wr(NI_REF_OBJECT_COUNT, 0, rd::<u32>(NI_REF_OBJECT_COUNT, 0).wrapping_add(1));
    wr::<u32>(this, TP::TArray_18_vtbl, VTBL_SUBPASS_ARRAY);
    wr::<u32>(this, TP::TArray_18_storageCount, 1);
    wr::<u32>(this, TP::TArray_18_growByCount, 1);
    wr::<u32>(this, TP::TArray_18_endIndex, 0);
    wr::<u32>(this, TP::TArray_18_filledCount, 0);
    wr(this, TP::TArray_18_storage, engine::operator_new(4));

    let bank = |offset: usize| eh_vector_constructor_iterator(this + offset, TS::SIZE, 8, TEXTURE_STAGE_CTOR, TEXTURE_STAGE_DTOR);
    bank(TP::stage);
    texture_stage_ctor(this + TP::field_624);
    bank(TP::field_6F8);
    wr::<u32>(this, TP::texture_CDC, 0);
    bank(TP::field_CE0);
    bank(TP::field_12C0);
    bank(TP::field_18A4);
    bank(TP::field_1E84);
    bank(TP::field_2468);
    texture_stage_ctor(this + TP::field_2A4C);
    texture_stage_ctor(this + TP::field_2B0C);
    wr::<usize>(this, TP::vtbl, VTBL_NI_DX8_TEXTURE_PASS);

    reset_for_new_frame(this);
    init_renderer(this, renderer);
    wr::<u32>(this, TP::field_CD8, 0);
    wr::<u32>(this, TP::field_18A0, 0);
    wr::<u32>(this, TP::field_2A48, 0);
    wr::<u32>(this, TP::field_2464, 0);
    wr::<u8>(this, TP::field_2B08, 0);
    wr::<u8>(this, TP::field_2A4C_bumpEnvMapEnabled, 0);
    wr::<u8>(this, TP::field_2BC8, 0);
    wr::<u32>(this, TP::field_2BD0, 0);
    wr::<u32>(this, TP::field_6E0, 0);
    this
}

/// 0x6B3590 `NiDX8TexturePass::dtor`
pub unsafe fn dtor(this: usize) {
    crate::shadow::touch(&dtor_port::STATE);
    wr::<usize>(this, TP::vtbl, VTBL_NI_DX8_TEXTURE_PASS);
    let end: u32 = rd(this, TP::TArray_18_endIndex);
    let mut index = 0u32;
    while index < end {
        let subpass: usize = rd(slot(this, index), 0);
        if subpass != 0 {
            // The array owns the sub-passes. The port deletes them and does not read the
            // reference count.
            ni::delete_object(subpass);
            array_clear_slot(this, index);
        }
        index += 1;
    }
    wr::<u32>(this, TP::substruct_14, 0);
    array_null_used_slots(this);
    wr::<u32>(this, TP::TArray_18_endIndex, 0);
    wr::<u32>(this, TP::TArray_18_filledCount, 0);

    for index in 0..STAGES_PER_BANK {
        let at = index * TS::SIZE + TS::ptrTexture;
        release_and_clear(this + TP::stage + at);
        release_and_clear(this + TP::field_6F8 + at);
        release_and_clear(this + TP::field_CE0 + at);
        release_call_and_clear(this + TP::field_12C0 + at);
        release_call_and_clear(this + TP::field_18A4 + at);
        release_call_and_clear(this + TP::field_1E84 + at);
        release_call_and_clear(this + TP::field_2468 + at);
    }
    release_call_and_clear(this + TP::field_624_ptrTexture);
    release_call_and_clear(this + TP::field_2A4C_ptrTexture);
    ni_pointer_set(this + TP::field_2B0C_ptrTexture, 0);
    ni_pointer_set(this + TP::texture_CDC, 0);

    let device: usize = rd(this, TP::deviceInterface);
    wr::<u32>(this, TP::renderer, 0);
    wr::<u32>(this, TP::renderState, 0);
    wr::<u32>(this, TP::configurableTexturePipeline, 0);
    wr::<u32>(this, TP::textureManager, 0);
    if device != 0 {
        device_release(device);
        wr::<u32>(this, TP::deviceInterface, 0);
    }

    let bank = |offset: usize| eh_vector_destructor_iterator(this + offset, TS::SIZE, 8, TEXTURE_STAGE_DTOR);
    texture_stage_dtor(this + TP::field_2B0C);
    texture_stage_dtor(this + TP::field_2A4C);
    bank(TP::field_2468);
    bank(TP::field_1E84);
    bank(TP::field_18A4);
    bank(TP::field_12C0);
    bank(TP::field_CE0);
    // The destructor of the smart pointer member. `NiPointer::set` above left it null.
    let texture: usize = rd(this, TP::texture_CDC);
    if texture != 0 {
        ni_object_release(texture);
    }
    bank(TP::field_6F8);
    texture_stage_dtor(this + TP::field_624);
    bank(TP::stage);

    let storage: usize = rd(this, TP::TArray_18_storage);
    wr::<u32>(this, TP::TArray_18_vtbl, VTBL_SUBPASS_ARRAY);
    operator_delete(storage);
    wr::<usize>(this, TP::vtbl, VTBL_NI_REF_OBJECT);
    wr(NI_REF_OBJECT_COUNT, 0, rd::<u32>(NI_REF_OBJECT_COUNT, 0).wrapping_sub(1));
}

/// 0x6B38A0 `NiDX8TexturePass::initRenderer`. Takes the device and the managers of the
/// renderer and makes the 2x1 texture that the stages of `field_12C0` use.
pub unsafe fn init_renderer(this: usize, renderer: usize) -> u8 {
    crate::shadow::touch(&init_renderer_port::STATE);
    wr(this, TP::renderer, renderer);
    if renderer != 0 {
        let device: usize = rd(renderer, R::d3dDevice);
        wr(this, TP::deviceInterface, device);
        device_add_ref(device);
        wr::<u32>(this, TP::renderState, rd(renderer, R::renderState));
        wr::<u32>(this, TP::configurableTexturePipeline, rd(renderer, R::configurableTexturePipeline));
        wr::<u32>(this, TP::textureManager, rd(renderer, R::textureManager));
    }
    let format: usize = rd(rd::<usize>(this, TP::renderer), R::defaultTextureFormats);
    let block = engine::operator_new(NiPixelData::SIZE);
    let pixel_data = if block != 0 { engine::pixel_data_ctor_args(block, 2, 1, format, 1) } else { 0 };
    if pixel_data != 0 {
        wr(pixel_data, REF_COUNT, rd::<u32>(pixel_data, REF_COUNT).wrapping_add(1));
    }
    let offsets: usize = rd(pixel_data, NiPixelData::mipmapOffsets);
    let pixels = rd::<usize>(pixel_data, NiPixelData::pixelsAllMips) + rd::<u32>(offsets, 0) as usize;
    match rd::<u8>(format, NiPixelFormat::bitsPerPixel) {
        16 => {
            wr::<u8>(pixels, 3, 0xFF);
            wr::<u8>(pixels, 2, 0xFF);
            wr::<u8>(pixels, 1, 0);
            wr::<u8>(pixels, 0, 0);
        }
        32 => {
            wr::<u8>(pixels, 3, 0);
            wr::<u8>(pixels, 2, 0);
            wr::<u8>(pixels, 7, 0xFF);
            wr::<u8>(pixels, 6, 0xFF);
            wr::<u8>(pixels, 5, 0xFF);
            wr::<u8>(pixels, 4, 0xFF);
            wr::<u8>(pixels, 1, 0);
            wr::<u8>(pixels, 0, 0);
        }
        _ => {}
    }

    // `NiTexture::FormatPrefs`: the pixel layout is 5 and the other two members are 0.
    let prefs = [5u32, 0, 0];
    let texture = source_texture_create_from_pixel_data(pixel_data, prefs.as_ptr());
    let old: usize = rd(this, TP::texture_CDC);
    if old != texture {
        if old != 0 {
            ni::release(old);
        }
        wr(this, TP::texture_CDC, texture);
        if texture != 0 {
            wr(texture, REF_COUNT, rd::<u32>(texture, REF_COUNT).wrapping_add(1));
        }
    }

    // The port calls virtual function 4 of the texture manager with the new texture.
    let manager: usize = rd(this, TP::textureManager);
    let link: unsafe extern "thiscall" fn(usize, usize) -> u32 = transmute(rd::<usize>(rd::<usize>(manager, 0), 0x10));
    link(manager, rd(this, TP::texture_CDC));

    for index in 0..STAGES_PER_BANK {
        let stage = this + TP::field_12C0 + index * TS::SIZE;
        let now: usize = rd(stage, TS::ptrTexture);
        if now != rd::<usize>(this, TP::texture_CDC) {
            if now != 0 {
                ni::release(now);
            }
            let texture: usize = rd(this, TP::texture_CDC);
            wr(stage, TS::ptrTexture, texture);
            if texture != 0 {
                wr(texture, REF_COUNT, rd::<u32>(texture, REF_COUNT).wrapping_add(1));
            }
        }
        wr::<u32>(stage, TS::addressModeU, 3);
        wr::<u32>(stage, TS::addressModeV, 3);
        wr::<u32>(stage, TS::textureTransformFlags, 2);
        wr::<u32>(stage, TS::field_94, 2);
        wr::<u32>(stage, TS::textureStageIndex, 0);
        wr::<u32>(stage, TS::sourceTexcoordIndex, D3DTSS_TCI_CAMERASPACEPOSITION);
        wr::<u8>(stage, TS::field_90, 0);
        wr::<u32>(stage, TS::texturePropMap, 0);
        core::ptr::write_bytes((stage + TS::textureMatrix) as *mut u8, 0, 0x40);
    }
    wr::<u32>(this, TP::field_18A0, 0);
    ni::release(pixel_data);
    1
}

/// 0x6B3AB0 `NiDX8TexturePass::setStageObject`. Puts a sub-pass or null in a slot of the
/// array. `field_C` counts the slots that went from empty to used.
pub unsafe fn set_stage_object(this: usize, index: u32, subpass: usize) {
    crate::shadow::touch(&set_stage_object_port::STATE);
    // The port reads the slot before the check against the size of the storage, as the game
    // code does.
    let before: usize = rd(slot(this, index), 0);
    if subpass != 0 {
        if index < rd::<u32>(this, TP::TArray_18_storageCount) {
            let filled: u32 = rd(this, TP::TArray_18_filledCount);
            if index >= rd::<u32>(this, TP::TArray_18_endIndex) {
                wr(this, TP::TArray_18_endIndex, index + 1);
                wr(this, TP::TArray_18_filledCount, filled.wrapping_add(1));
            } else if rd::<u32>(slot(this, index), 0) == 0 {
                wr(this, TP::TArray_18_filledCount, filled.wrapping_add(1));
            }
            wr(slot(this, index), 0, subpass);
        }
        if before == 0 {
            wr(this, TP::field_C, rd::<u32>(this, TP::field_C).wrapping_add(1));
        }
    } else {
        array_clear_slot(this, index);
        if before != 0 {
            wr(this, TP::field_C, rd::<u32>(this, TP::field_C).wrapping_sub(1));
        }
    }
}

/// 0x6B3B30 `NiDX8TexturePass::acquireSubpassWithCapacity`. Returns the current sub-pass. It
/// returns the next one when the current one has fewer free slots than the caller needs.
pub unsafe fn acquire_subpass_with_capacity(this: usize, need_2: u32, need_1: u32) -> usize {
    crate::shadow::touch(&acquire_subpass_with_capacity_port::STATE);
    ensure_current_subpass(this);
    let (free_1, free_2) = subpass_get_available_slot_counts(rd(this, TP::substruct_14));
    if free_1 < need_1 || free_2 < need_2 {
        advance_to_next_subpass(this);
    }
    rd(this, TP::substruct_14)
}

/// 0x6B3D70 `NiDX8TexturePass::initOrCheckSubpass`. Returns 1 when the current sub-pass
/// cannot take what the caller wants to add.
pub unsafe fn init_or_check_subpass(this: usize, stage_kind: u32, need_1: u32, need_2: u32) -> u8 {
    crate::shadow::touch(&init_or_check_subpass_port::STATE);
    if ensure_current_subpass(this) {
        return 0;
    }
    let subpass: usize = rd(this, TP::substruct_14);
    if rd::<u32>(this, TP::field_C) != 0 {
        let first_stage: usize = rd(subpass, SP::field_14);
        if rd::<u32>(first_stage, TS::field_94) != stage_kind {
            return 1;
        }
    }
    let (free_1, free_2) = subpass_get_available_slot_counts(rd(this, TP::substruct_14));
    if free_1 < need_1 || free_2 < need_2 {
        return 1;
    }
    0
}

/// 0x6B3EE0 `NiDX8TexturePass::acquireSubpassForPixelShader`. Goes to the next sub-pass when
/// the current one has a pixel shader. No caller reads EAX.
pub unsafe fn acquire_subpass_for_pixel_shader(this: usize) {
    crate::shadow::touch(&acquire_subpass_for_pixel_shader_port::STATE);
    ensure_current_subpass(this);
    if rd::<u32>(rd::<usize>(this, TP::substruct_14), SUBPASS_SLOTS_USED_1) != u32::MAX {
        advance_to_next_subpass(this);
    }
}

/// The shader object of the sub-pass that the game draws now, or 0. `field_8` is the number of
/// that sub-pass plus 1.
#[inline(always)]
unsafe fn drawn_subpass_shader(this: usize, member: usize) -> usize {
    let index = rd::<u32>(this, TP::field_8).wrapping_sub(1);
    if index == u32::MAX {
        return 0;
    }
    let subpass: usize = rd(slot(this, index), 0);
    if subpass == 0 {
        return 0;
    }
    rd(subpass, member)
}

/// 0x6B40F0 `NiDX8TexturePass::applyPixelShader`. Returns the handle it selected.
pub unsafe fn apply_pixel_shader(this: usize) -> u32 {
    crate::shadow::touch(&apply_pixel_shader_port::STATE);
    let mut handle = 0u32;
    let shader = drawn_subpass_shader(this, SUBPASS_PIXEL_SHADER);
    if shader != 0 {
        handle = rd(shader, SHADER_HANDLE);
    }
    let render_state: usize = rd(this, TP::renderState);
    if rd::<u32>(render_state, RS::currentPixelShader) != handle {
        wr(render_state, RS::currentPixelShader, handle);
        device_set_pixel_shader(rd(render_state, RS::device), handle);
    }
    handle
}

/// 0x6B4140 `NiDX8TexturePass::applyVertexShader`. `handle` is the shader to use when the
/// sub-pass has none. Returns the handle it selected.
pub unsafe fn apply_vertex_shader(this: usize, handle: u32) -> u32 {
    crate::shadow::touch(&apply_vertex_shader_port::STATE);
    let mut handle = handle;
    let shader = drawn_subpass_shader(this, SUBPASS_VERTEX_SHADER);
    if shader != 0 {
        handle = rd(shader, SHADER_HANDLE);
    }
    if handle == 0 {
        return 0;
    }
    let render_state: usize = rd(this, TP::renderState);
    if rd::<u32>(render_state, RS::currentVertexShader) != handle {
        wr(render_state, RS::currentVertexShader, handle);
        device_set_vertex_shader(rd(render_state, RS::device), handle);
    }
    handle
}

// ---- regions ---------------------------------------------------------------------------

/// The objects that a call releases, with the number of references that each one loses.
struct Drops {
    list: Vec<(usize, u32)>,
}

impl Drops {
    fn new() -> Drops {
        Drops { list: Vec::new() }
    }

    fn add(&mut self, object: usize) {
        if object == 0 {
            return;
        }
        if let Some(entry) = self.list.iter_mut().find(|entry| entry.0 == object) {
            entry.1 += 1;
        } else {
            self.list.push((object, 1));
        }
    }

    /// Adds the reference counts to the regions. The call is a skip when an object loses its
    /// last reference, because the call deletes the object.
    unsafe fn into_regions(&self, set: &mut Regions) {
        for &(object, count) in &self.list {
            // The last reference: the call releases the object `count` times.
            if rd::<u32>(object, REF_COUNT) <= count {
                set.skip();
            }
            set.add(object + REF_COUNT, 4);
        }
    }
}

/// The storage of the array of sub-passes.
unsafe fn add_storage(set: &mut Regions, this: usize) {
    set.add(rd(this, TP::TArray_18_storage), rd::<u32>(this, TP::TArray_18_storageCount) as usize * 4);
}

/// The call is a skip when the storage of the array does not have 2 slots. The function then
/// makes the storage again. The call is a skip when the end index is more than 2. The array
/// is then not in a state that the function leaves it in. The call is a skip when a texture
/// loses its last reference, because the call deletes the texture.
unsafe fn reset_for_new_frame_regions(set: &mut Regions, this: usize) {
    set.add(this, TP::SIZE);
    if rd::<u32>(this, TP::TArray_18_storageCount) != 2 || rd::<u32>(this, TP::TArray_18_endIndex) > 2 {
        set.skip();
        return;
    }
    add_storage(set, this);
    let mut drops = Drops::new();
    drops.add(rd(this, TP::texture_CDC));
    drops.add(rd(this, TP::field_624_ptrTexture));
    for bank in [TP::field_2468, TP::field_CE0, TP::field_18A4, TP::field_1E84] {
        for index in 0..STAGES_PER_BANK {
            drops.add(rd(this, bank + index * TS::SIZE + TS::ptrTexture));
        }
    }
    drops.into_regions(set);
}

/// No call is a skip. The function writes the pass and one slot of the array.
unsafe fn set_stage_object_regions(set: &mut Regions, this: usize, index: u32) {
    set.add(this, TP::SIZE);
    if index < rd::<u32>(this, TP::TArray_18_storageCount) {
        set.add(slot(this, index), 4);
    }
}

/// The values that `getAvailableSlotCounts` returns, without the call.
unsafe fn free_slots(subpass: usize) -> (u32, u32) {
    let used: u32 = rd(subpass, SUBPASS_SLOTS_USED_1);
    let limit: u32 = rd(MAX_TEXTURE_BLEND_STAGES, 0);
    let free_1 = if used == u32::MAX { limit } else { limit.wrapping_sub(used) };
    (free_1, rd::<u32>(MAX_SIMULTANEOUS_TEXTURES, 0).wrapping_sub(rd(subpass, SUBPASS_SLOTS_USED_2)))
}

/// The current sub-pass as the "acquire" functions find it, or 0 when they must make one.
unsafe fn current_subpass_without_allocation(this: usize) -> usize {
    let current: usize = rd(this, TP::substruct_14);
    if current != 0 {
        return current;
    }
    rd(slot(this, rd(this, TP::field_C)), 0)
}

/// The regions of a call that goes to the next sub-pass. The call is a skip when the next
/// slot is outside the array or has no sub-pass, because the function then allocates.
unsafe fn advance_regions(set: &mut Regions, this: usize) {
    let next = rd::<u32>(this, TP::field_C).wrapping_add(1);
    if next >= rd::<u32>(this, TP::count_10) || next >= rd::<u32>(this, TP::TArray_18_storageCount) || rd::<u32>(slot(this, next), 0) == 0 {
        set.skip();
    }
}

/// The call is a skip when the function must make the current sub-pass. It is also a skip
/// when `advance_regions` finds that the step to the next sub-pass allocates.
unsafe fn acquire_subpass_with_capacity_regions(set: &mut Regions, this: usize, need_2: u32, need_1: u32) {
    set.add(this, TP::SIZE);
    add_storage(set, this);
    let subpass = current_subpass_without_allocation(this);
    if subpass == 0 {
        set.skip();
        return;
    }
    let (free_1, free_2) = free_slots(subpass);
    if free_1 < need_1 || free_2 < need_2 {
        advance_regions(set, this);
    }
}

/// The call is a skip when the function must make the current sub-pass.
unsafe fn init_or_check_subpass_regions(set: &mut Regions, this: usize) {
    set.add(this, TP::SIZE);
    add_storage(set, this);
    if current_subpass_without_allocation(this) == 0 {
        set.skip();
    }
}

/// The call is a skip when the function must make the current sub-pass. It is also a skip
/// when `advance_regions` finds that the step to the next sub-pass allocates.
unsafe fn acquire_subpass_for_pixel_shader_regions(set: &mut Regions, this: usize) {
    set.add(this, TP::SIZE);
    add_storage(set, this);
    let subpass = current_subpass_without_allocation(this);
    if subpass == 0 {
        set.skip();
        return;
    }
    if rd::<u32>(subpass, SUBPASS_SLOTS_USED_1) != u32::MAX {
        advance_regions(set, this);
    }
}

port_thiscall!(reset_for_new_frame_port, 0x6B2FA0, "NiDX8TexturePass::resetForNewFrame",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| reset_for_new_frame_regions(set, this),
    rust = reset_for_new_frame);

// The call is a skip always: the function allocates the storage of the array and the 2x1
// texture.
port_thiscall!(ctor_port, 0x6B33A0, "NiDX8TexturePass::ctor",
    fn(this, renderer: usize) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = ctor);

// The call is a skip always: the function frees the sub-passes and the storage and releases
// the device.
port_thiscall!(dtor_port, 0x6B3590, "NiDX8TexturePass::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = dtor);

// The call is a skip always: the function makes a `NiPixelData` and a `NiSourceTexture`.
port_thiscall!(init_renderer_port, 0x6B38A0, "NiDX8TexturePass::initRenderer",
    fn(this, renderer: usize) -> u8,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = init_renderer);

port_thiscall!(set_stage_object_port, 0x6B3AB0, "NiDX8TexturePass::setStageObject",
    fn(this, index: u32, subpass: usize) -> (),
    compare_eax = false,
    regions = |set| set_stage_object_regions(set, this, index),
    rust = set_stage_object);

port_thiscall!(acquire_subpass_with_capacity_port, 0x6B3B30, "NiDX8TexturePass::acquireSubpassWithCapacity",
    fn(this, need_2: u32, need_1: u32) -> usize,
    compare_eax = true,
    regions = |set| acquire_subpass_with_capacity_regions(set, this, need_2, need_1),
    rust = acquire_subpass_with_capacity);

port_thiscall!(init_or_check_subpass_port, 0x6B3D70, "NiDX8TexturePass::initOrCheckSubpass",
    fn(this, stage_kind: u32, need_1: u32, need_2: u32) -> u8,
    compare_eax = true,
    regions = |set| init_or_check_subpass_regions(set, this),
    rust = init_or_check_subpass);

port_thiscall!(acquire_subpass_for_pixel_shader_port, 0x6B3EE0, "NiDX8TexturePass::acquireSubpassForPixelShader",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| acquire_subpass_for_pixel_shader_regions(set, this),
    rust = acquire_subpass_for_pixel_shader);

port_thiscall!(apply_pixel_shader_port, 0x6B40F0, "NiDX8TexturePass::applyPixelShader",
    fn(this) -> u32,
    compare_eax = true,
    regions = |set| set.add(rd(this, TP::renderState), RS::SIZE),
    rust = apply_pixel_shader);

port_thiscall!(apply_vertex_shader_port, 0x6B4140, "NiDX8TexturePass::applyVertexShader",
    fn(this, handle: u32) -> u32,
    compare_eax = true,
    regions = |set| set.add(rd(this, TP::renderState), RS::SIZE),
    rust = apply_vertex_shader);

pub static PORTS: &[&crate::shadow::PortState] = &[
    &reset_for_new_frame_port::STATE,
    &ctor_port::STATE,
    &dtor_port::STATE,
    &init_renderer_port::STATE,
    &set_stage_object_port::STATE,
    &acquire_subpass_with_capacity_port::STATE,
    &init_or_check_subpass_port::STATE,
    &acquire_subpass_for_pixel_shader_port::STATE,
    &apply_pixel_shader_port::STATE,
    &apply_vertex_shader_port::STATE,
];

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicU32, Ordering};

    /// A texture pass with a storage of `slots` slots for the array.
    struct Pass {
        bytes: Vec<u8>,
        storage: Vec<u32>,
    }

    impl Pass {
        fn new(slots: usize) -> Pass {
            let mut pass = Pass { bytes: vec![0u8; TP::SIZE], storage: vec![0u32; slots] };
            unsafe {
                wr(pass.this(), TP::TArray_18_storage, pass.storage.as_mut_ptr() as usize);
                wr::<u32>(pass.this(), TP::TArray_18_storageCount, slots as u32);
            }
            pass
        }
        fn this(&mut self) -> usize {
            self.bytes.as_mut_ptr() as usize
        }
        fn get(&mut self, offset: usize) -> u32 {
            unsafe { rd(self.this(), offset) }
        }
    }

    #[test]
    fn set_stage_object_counts_slots_that_change() {
        let mut pass = Pass::new(4);
        let this = pass.this();
        unsafe {
            set_stage_object(this, 2, 0x1000);
            assert_eq!(pass.storage, [0, 0, 0x1000, 0]);
            assert_eq!(pass.get(TP::TArray_18_endIndex), 3);
            assert_eq!(pass.get(TP::TArray_18_filledCount), 1);
            assert_eq!(pass.get(TP::field_C), 1);

            // Another object goes into a used slot. No count changes.
            set_stage_object(this, 2, 0x2000);
            assert_eq!(pass.storage[2], 0x2000);
            assert_eq!(pass.get(TP::TArray_18_filledCount), 1);
            assert_eq!(pass.get(TP::field_C), 1);

            // An empty slot below the end index.
            set_stage_object(this, 0, 0x3000);
            assert_eq!(pass.get(TP::TArray_18_endIndex), 3);
            assert_eq!(pass.get(TP::TArray_18_filledCount), 2);
            assert_eq!(pass.get(TP::field_C), 2);

            set_stage_object(this, 2, 0);
            assert_eq!(pass.storage, [0x3000, 0, 0, 0]);
            assert_eq!(pass.get(TP::TArray_18_filledCount), 1);
            assert_eq!(pass.get(TP::field_C), 1);

            // Null in an empty slot past the end index moves the end index only.
            set_stage_object(this, 3, 0);
            assert_eq!(pass.get(TP::TArray_18_endIndex), 4);
            assert_eq!(pass.get(TP::TArray_18_filledCount), 1);
            assert_eq!(pass.get(TP::field_C), 1);
        }
    }

    #[test]
    fn array_helpers_keep_the_counts() {
        let mut pass = Pass::new(4);
        let this = pass.this();
        unsafe {
            array_set_at(this, 1, 0x1000);
            array_set_at(this, 3, 0x2000);
            assert_eq!((pass.get(TP::TArray_18_endIndex), pass.get(TP::TArray_18_filledCount)), (4, 2));
            array_set_at(this, 1, 0);
            assert_eq!((pass.get(TP::TArray_18_endIndex), pass.get(TP::TArray_18_filledCount)), (4, 1));
            array_set_at(this, 2, 0);
            assert_eq!(pass.get(TP::TArray_18_filledCount), 1);
            // The index is past the storage. Nothing changes.
            array_set_at(this, 4, 0x3000);
            assert_eq!(pass.storage, [0, 0, 0, 0x2000]);

            wr::<u32>(this, TP::count_10, 4);
            array_clear_slots_from(this, 2);
            assert_eq!(pass.storage, [0, 0, 0, 0]);
            assert_eq!(pass.get(TP::TArray_18_filledCount), 0);

            pass.storage.copy_from_slice(&[1, 2, 3, 4]);
            wr::<u32>(this, TP::TArray_18_endIndex, 3);
            array_null_used_slots(this);
            assert_eq!(pass.storage, [0, 0, 0, 4]);

            // The array has this size. The function makes no new storage.
            array_set_size(this, 4);
            assert_eq!(pass.get(TP::TArray_18_storage), pass.storage.as_ptr() as u32);
        }
    }

    #[test]
    fn ensure_current_subpass_takes_the_one_in_the_slot() {
        let mut pass = Pass::new(2);
        let this = pass.this();
        pass.storage[1] = 0x4000;
        unsafe {
            wr::<u32>(this, TP::field_C, 1);
            assert!(!ensure_current_subpass(this));
            assert_eq!(pass.get(TP::substruct_14), 0x4000);
        }
    }

    static LAST_PIXEL_SHADER: AtomicU32 = AtomicU32::new(!0);
    static LAST_VERTEX_SHADER: AtomicU32 = AtomicU32::new(!0);

    extern "system" fn fake_set_pixel_shader(_device: usize, handle: u32) -> u32 {
        LAST_PIXEL_SHADER.store(handle, Ordering::Relaxed);
        0
    }

    extern "system" fn fake_set_vertex_shader(_device: usize, handle: u32) -> u32 {
        LAST_VERTEX_SHADER.store(handle, Ordering::Relaxed);
        0
    }

    #[test]
    fn shaders_of_the_drawn_subpass_go_to_the_device_once() {
        let mut vtable = [0usize; 0x60];
        vtable[0x160 / 4] = fake_set_pixel_shader as *const () as usize;
        vtable[0x130 / 4] = fake_set_vertex_shader as *const () as usize;
        let device = [vtable.as_ptr() as usize];
        let mut render_state = vec![0u8; RS::SIZE];
        let rs = render_state.as_mut_ptr() as usize;
        let mut subpass = [0u8; SP::SIZE];
        let sp = subpass.as_mut_ptr() as usize;
        let mut pixel_shader = [0u8; 0x24];
        let mut vertex_shader = [0u8; 0x24];
        let mut pass = Pass::new(2);
        let this = pass.this();
        pass.storage[1] = sp as u32;
        unsafe {
            wr(rs, RS::device, device.as_ptr() as usize);
            wr(this, TP::renderState, rs);
            wr::<u32>(pixel_shader.as_mut_ptr() as usize, SHADER_HANDLE, 0x55);
            wr::<u32>(vertex_shader.as_mut_ptr() as usize, SHADER_HANDLE, 0x66);

            // The game draws no sub-pass. The device has pixel shader 0, thus there is no call.
            assert_eq!(apply_pixel_shader(this), 0);
            assert_eq!(LAST_PIXEL_SHADER.load(Ordering::Relaxed), !0);
            // The vertex shader becomes the argument. An argument of 0 does nothing.
            assert_eq!(apply_vertex_shader(this, 0), 0);
            assert_eq!(LAST_VERTEX_SHADER.load(Ordering::Relaxed), !0);
            assert_eq!(apply_vertex_shader(this, 0x142), 0x142);
            assert_eq!(LAST_VERTEX_SHADER.load(Ordering::Relaxed), 0x142);
            assert_eq!(rd::<u32>(rs, RS::currentVertexShader), 0x142);

            // The game draws sub-pass 1. `field_8` is the number plus 1. The sub-pass has no
            // shaders at this time.
            wr::<u32>(this, TP::field_8, 2);
            assert_eq!(apply_pixel_shader(this), 0);
            wr(sp, SUBPASS_PIXEL_SHADER, pixel_shader.as_ptr() as usize);
            wr(sp, SUBPASS_VERTEX_SHADER, vertex_shader.as_ptr() as usize);
            assert_eq!(apply_pixel_shader(this), 0x55);
            assert_eq!(LAST_PIXEL_SHADER.load(Ordering::Relaxed), 0x55);
            assert_eq!(rd::<u32>(rs, RS::currentPixelShader), 0x55);
            assert_eq!(apply_vertex_shader(this, 0x142), 0x66);
            assert_eq!(LAST_VERTEX_SHADER.load(Ordering::Relaxed), 0x66);

            // The same calls again. The cache stops the device calls.
            LAST_PIXEL_SHADER.store(!0, Ordering::Relaxed);
            LAST_VERTEX_SHADER.store(!0, Ordering::Relaxed);
            assert_eq!(apply_pixel_shader(this), 0x55);
            assert_eq!(apply_vertex_shader(this, 0), 0x66);
            assert_eq!(LAST_PIXEL_SHADER.load(Ordering::Relaxed), !0);
            assert_eq!(LAST_VERTEX_SHADER.load(Ordering::Relaxed), !0);
        }
    }

    #[test]
    fn drops_count_the_references_an_object_loses() {
        let mut texture_a = [0u32; 2];
        let mut texture_b = [0u32; 2];
        let (a, b) = (texture_a.as_mut_ptr() as usize, texture_b.as_mut_ptr() as usize);
        let mut drops = Drops::new();
        drops.add(0);
        drops.add(a);
        drops.add(b);
        drops.add(a);
        assert_eq!(drops.list, [(a, 2), (b, 1)]);
        // The list has no limit.
        for extra in 1..=16 {
            drops.add(extra * 8);
        }
        assert_eq!(drops.list.len(), 18);
    }
}
