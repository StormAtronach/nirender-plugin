//! `NiDX8TexturePass`, second half. It has the stages of the dynamic texture effects
//! (projected lights and shadows, environment map, fog map) and the functions that generate
//! their texture coordinates. It has the sub-pass object `NiDX8TexturePass::$1`, which holds
//! the stages of one device pass. It has the stage helpers that write the addressing modes
//! and the filters to the device.

use core::mem::transmute;

use crate::d3d::{
    device_set_texture, D3DTSS_ADDRESSU, D3DTSS_ADDRESSV, D3DTSS_ALPHAOP, D3DTSS_COLOROP, D3DTSS_MAGFILTER, D3DTSS_MINFILTER,
    D3DTSS_MIPFILTER, D3DTSS_TEXCOORDINDEX,
};
use crate::engine::{self, EMPTY_STRING, MAX_SIMULTANEOUS_TEXTURES, MAX_TEXTURE_BLEND_STAGES, NI_REF_OBJECT_COUNT, VTBL_NI_REF_OBJECT};
use crate::gen::layouts::{
    NiDX8ConfigurableTexturePipeline as CTP, NiDX8RenderState as RS, NiDX8Renderer as REN, NiDX8TexturePass as TP,
    NiDX8TexturePass__1 as SUB, NiDX8TextureStage as ST, NiDynamicEffectState as DES, NiDynamicEffectState_InternalList as LIST,
    NiTexture,
};
use crate::mem::{rd, wr};
use crate::ports::ni::{add_reference, release, REF_COUNT};
use crate::shadow::Regions;

// ---------------------------------------------------------------------------------------
// Addresses in Morrowind.exe
// ---------------------------------------------------------------------------------------

/// `vtbl_sg_NiDX8TexturePass::$1`. Slot 0 is the deleting destructor (0x6B77C0). Slot 1 is
/// `method_4`.
const VTBL_SUBPASS: usize = 0x74F6C4;
/// The filter table. It has 12 bytes for each filter mode: { minify, magnify, mip }. The
/// database names the three interleaved columns
/// `data_NiDX8TexturePass_{minify,magnify,mip}FilteringRemapper`.
const FILTER_TABLE: usize = engine::FILTER_REMAP_MINIFY;
const FILTER_MIN: usize = 0;
const FILTER_MAG: usize = 4;
const FILTER_MIP: usize = 8;

const ADDR_METHOD_4: usize = 0x6B7900;
const ADDR_RESET_PASS_FLAGS: usize = 0x6B59E0;

// ---------------------------------------------------------------------------------------
// Offsets that the layouts do not name
// ---------------------------------------------------------------------------------------

// `NiTextureEffect`. The export of the database has no layout for it.
/// `NiDynamicEffect::enabled`
const EFFECT_ENABLED: usize = 0x90;
/// The 3x3 matrix of the world projection.
const EFFECT_PROJECTION_MATRIX: usize = 0xD8;
/// The translation of the world projection.
const EFFECT_PROJECTION_TRANSLATION: usize = 0xFC;
const EFFECT_TEXTURE: usize = 0x108;
const EFFECT_FILTER_MODE: usize = 0x10C;
const EFFECT_CLAMP_MODE: usize = 0x110;
/// The index into `NiDX8TexturePass::callbacks_6E4`.
const EFFECT_COORDINATE_MODE: usize = 0x118;
/// The effect has a clipping plane.
const EFFECT_HAS_PLANE: usize = 0x11C;
/// The clipping plane: normal x, y, z and the constant.
const EFFECT_PLANE: usize = 0x130;

/// `NiSourceTexture::isStatic`
const SOURCE_TEXTURE_IS_STATIC: usize = 0x38;
/// `NiDX8TextureData` vtable: `GetD3DTexture` and `GetAsRenderedTexture`.
const TEXTURE_DATA_GET_D3D_TEXTURE: usize = 0x18;
const TEXTURE_DATA_GET_AS_RENDERED: usize = 0x20;

/// `NiDX8RenderState::maybe_textureStageStateSettings`: 0x100 bytes for each stage, 8 bytes
/// for each state.
const STAGE_SETTINGS: usize = 0x100;

// ---------------------------------------------------------------------------------------
// Engine functions at fixed addresses, virtual calls, device calls
// ---------------------------------------------------------------------------------------

/// The call in `setBlendingState` that sets the alpha property of a sub-pass.
pub const SET_ALPHA_SITE: usize = 0x6B5663;
/// `patchSetEffectState` of the Morrowind Code Patch: the target of the call with the option
/// "Enchant glow in fog fix" (feature 12).
const PATCH_SET_EFFECT_STATE: usize = 0x7364E0;

/// The target of the call at `SET_ALPHA_SITE` as it is in memory now.
#[cfg(not(test))]
unsafe fn set_alpha_target() -> usize {
    crate::site::call_target(SET_ALPHA_SITE)
}

#[cfg(test)]
unsafe fn set_alpha_target() -> usize {
    PATCH_SET_EFFECT_STATE
}

/// The call at `SET_ALPHA_SITE`. With the Code Patch option on, the target is
/// `patchSetEffectState`: it takes the property from EDX and drops the stack argument, so the
/// call gives it both, as the call site does. Without the option the target is
/// `NiDX8RenderState::updateAlphaStates` (0x6B8480, a port too), a `__thiscall` with the
/// property on the stack.
unsafe fn set_alpha_state(render_state: usize, alpha_property: usize) {
    let target = set_alpha_target();
    if target == PATCH_SET_EFFECT_STATE {
        let f: unsafe extern "fastcall" fn(usize, usize, usize) -> u32 = transmute(target);
        f(render_state, alpha_property, alpha_property);
    } else {
        let f: unsafe extern "thiscall" fn(usize, usize) -> u32 = transmute(target);
        f(render_state, alpha_property);
    }
}

/// 0x6B3B30 `NiDX8TexturePass::acquireSubpassWithCapacity` (first half of the class).
unsafe fn acquire_subpass_with_capacity(pass: usize, textures: u32, stages: u32) -> usize {
    let f: unsafe extern "thiscall" fn(usize, u32, u32) -> usize = transmute(0x6B3B30usize);
    f(pass, textures, stages)
}

/// 0x6C1B00 `NiDX8TextureStage::updateRenderer`
unsafe fn texture_stage_update_renderer(stage: usize, renderer: usize) {
    let f: unsafe extern "thiscall" fn(usize, usize) -> u32 = transmute(0x6C1B00usize);
    f(stage, renderer);
}

/// 0x6B9E40 `NiDX8ConfigurableTexturePipeline::setStageFilterMode`
unsafe fn pipeline_set_stage_filter_mode(pipeline: usize, filter_mode: u32, stage: usize) {
    let f: unsafe extern "thiscall" fn(usize, u32, usize) -> u32 = transmute(0x6B9E40usize);
    f(pipeline, filter_mode, stage);
}

/// 0x6B9E20 `NiDX8ConfigurableTexturePipeline::setStageAddressingMode`
unsafe fn pipeline_set_stage_addressing_mode(pipeline: usize, clamp_mode: u32, stage: usize) {
    let f: unsafe extern "thiscall" fn(usize, u32, usize) -> u32 = transmute(0x6B9E20usize);
    f(pipeline, clamp_mode, stage);
}

/// `NiDX8TextureManager::GetFormatFromTexture` (0x6BC6F0), slot 0x10 of the texture manager.
unsafe fn texture_manager_get_format(texture_manager: usize, texture: usize) -> u32 {
    engine::vcall1(texture_manager, 0x10, texture as u32)
}

/// The address of the setting of one texture stage state in the render state.
#[inline(always)]
fn stage_setting(render_state: usize, stage: u32, state: u32) -> usize {
    render_state
        .wrapping_add((stage as usize).wrapping_mul(STAGE_SETTINGS))
        .wrapping_add(RS::maybe_textureStageStateSettings + state as usize * 8)
}

/// The pattern that the stage functions have inline. It writes the cache and the device when
/// the value is new. It does not keep the previous value.
#[inline(always)]
unsafe fn set_stage_state_if_changed(render_state: usize, stage: u32, state: u32, value: u32) {
    let setting = stage_setting(render_state, stage, state);
    if rd::<u32>(setting, 0) != value {
        wr(setting, 0, value);
        engine::device_set_texture_stage_state(rd(render_state, RS::device), stage, state, value);
    }
}

/// The address of element `index` of an array of texture stages in the pass.
#[inline(always)]
fn stage_at(pass: usize, array: usize, index: u32) -> usize {
    pass.wrapping_add(array).wrapping_add((index as usize).wrapping_mul(ST::SIZE))
}

// ---------------------------------------------------------------------------------------
// x87 sequences
// ---------------------------------------------------------------------------------------

/// `fld a / fmul b / fld c / fmul d / faddp / fld e / fmul f / faddp`: the sum of three
/// products. The game code calculates every matrix element of this module in this order.
#[inline(always)]
fn dot3(a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> f32 {
    let first = a * b;
    let second = c * d;
    let sum = first + second;
    let third = e * f;
    sum + third
}

// ---------------------------------------------------------------------------------------
// NiDX8TexturePass
// ---------------------------------------------------------------------------------------

/// 0x6B5640 `NiDX8TexturePass::setBlendingState`. Applies the next sub-pass to the device.
/// The function does not use the second argument.
pub unsafe fn set_blending_state(this: usize, a2: u32, _a3: u32, a4: u32, a5: u32) -> u32 {
    crate::shadow::touch(&set_blending_state_port::STATE);
    let mut result = 0u32;
    let index: u32 = rd(this, TP::field_8);
    let storage: usize = rd(this, TP::TArray_18_storage);
    let subpass: usize = rd(storage, index as usize * 4);
    wr(this, TP::substruct_14, subpass);
    if subpass != 0 {
        if index != 0 {
            set_alpha_state(rd(this, TP::renderState), rd(subpass, SUB::field_48));
        }
        let index: u32 = rd(this, TP::field_8);
        let subpass: usize = rd(this, TP::substruct_14);
        let apply: unsafe extern "thiscall" fn(usize, usize, u32, u32, u32, u32) -> u32 = transmute(engine::virtual_at(subpass, 4));
        result = apply(subpass, rd(this, TP::renderer), a2, a4, a5, index);
        let index: u32 = rd(this, TP::field_8);
        let remaining = rd::<u32>(this, TP::field_C).wrapping_sub(index);
        if remaining != 0 {
            result |= remaining;
        }
        wr(this, TP::field_8, index.wrapping_add(1));
    }
    result
}

/// 0x6B56B0 `NiDX8TexturePass::applyStageBlendOps`. Copies the colour operations and the
/// alpha operations of blend slot `stage.field_94` from the pipeline into the stage.
pub unsafe fn apply_stage_blend_ops(this: usize, stage: usize, flag: u8) {
    crate::shadow::touch(&apply_stage_blend_ops_port::STATE);
    if stage == 0 {
        return;
    }
    let stage_index: u32 = rd(stage, ST::stage);
    let slot: u32 = rd(stage, ST::field_94);
    if stage_index == 0 && flag == 0 {
        // The table of the first stage, at pipeline + 0x108.
        let pipeline: usize = rd(this, TP::configurableTexturePipeline);
        let entry = pipeline.wrapping_add((slot.wrapping_mul(3).wrapping_add(0x21) as usize).wrapping_mul(8));
        wr::<u32>(stage, ST::colorOp, rd(entry, 0x00));
        wr::<u32>(stage, ST::colorArg1, rd(entry, 0x04));
        wr::<u32>(stage, ST::colorArg2, rd(entry, 0x08));
        wr::<u32>(stage, ST::alphaOp, rd(entry, 0x0C));
        wr::<u32>(stage, ST::alphaArg1, rd(entry, 0x10));
        wr::<u32>(stage, ST::alphaArg2, rd(entry, 0x14));
        if rd::<u8>(stage, ST::field_90) == 0 {
            wr::<u32>(stage, ST::colorOp, 2);
            wr::<u32>(stage, ST::colorArg1, 2);
            wr::<u32>(stage, ST::alphaOp, 2);
            wr::<u32>(stage, ST::alphaArg1, 2);
            return;
        }
        if rd::<usize>(stage, ST::ptrTexture) != 0 {
            return;
        }
        wr::<u32>(stage, ST::colorOp, 3);
        wr::<u32>(stage, ST::alphaOp, 3);
    } else {
        // The table of the later stages, at pipeline + 0x1F8.
        let pipeline = || rd::<usize>(this, TP::configurableTexturePipeline);
        let first = (slot.wrapping_mul(3).wrapping_add(0x3F) as usize).wrapping_mul(8);
        let entry = (slot.wrapping_mul(3) << 3) as usize;
        wr::<u32>(stage, ST::colorOp, rd(pipeline().wrapping_add(first), 0));
        wr::<u32>(stage, ST::colorArg1, rd(pipeline().wrapping_add(entry), CTP::field_1FC));
        wr::<u32>(stage, ST::colorArg2, rd(pipeline().wrapping_add(entry), CTP::field_1FC + 0x04));
        wr::<u32>(stage, ST::alphaOp, rd(pipeline().wrapping_add(entry), CTP::field_1FC + 0x08));
        wr::<u32>(stage, ST::alphaArg1, rd(pipeline().wrapping_add(entry), CTP::field_1FC + 0x0C));
        wr::<u32>(stage, ST::alphaArg2, rd(pipeline().wrapping_add(entry), CTP::field_1FC + 0x10));
    }
}

/// 0x6B57B0 `NiDX8TexturePass::bindStageTexture`. Appends the stage to the sub-pass that has
/// space for it. In a later pass the sub-pass also takes the object of the blend slot of its
/// first stage.
pub unsafe fn bind_stage_texture(this: usize, stage: usize, textures: u32, stages: u32) -> u8 {
    crate::shadow::touch(&bind_stage_texture_port::STATE);
    if stage == 0 {
        return 0;
    }
    let subpass = acquire_subpass_with_capacity(this, textures, stages);
    if subpass == 0 {
        return 0;
    }
    let result = subpass_append_stage(subpass, stage);
    if rd::<u32>(this, TP::field_C) == 0 {
        return result;
    }
    if rd::<u32>(subpass, SUB::field_C) == u32::MAX {
        return result;
    }
    let first_stage: usize = rd(subpass, SUB::field_14);
    let pipeline: usize = rd(this, TP::configurableTexturePipeline);
    let slot: u32 = rd(first_stage, ST::field_94);
    let object: usize = rd(pipeline.wrapping_add((slot as usize).wrapping_mul(4)), CTP::field_2E8);
    // The game has the same code as `_1::setNiObjectField48` inline here.
    subpass_set_ni_object_field_48(subpass, object);
    result
}

/// 0x6B5870 `NiDX8TexturePass::setTexturePipeline`. Builds the stages of the texture effects
/// of a `NiDynamicEffectState` when it is not the one of the previous call.
pub unsafe fn set_texture_pipeline(this: usize, effect_state: usize) {
    crate::shadow::touch(&set_texture_pipeline_port::STATE);
    let pipeline: usize = rd(this, TP::configurableTexturePipeline);
    if effect_state == rd::<usize>(pipeline, CTP::field_B04) {
        return;
    }
    // Slot 0x18 of the pass: `resetPassFlags`.
    engine::vcall0(this, 0x18);
    let pipeline: usize = rd(this, TP::configurableTexturePipeline);
    wr(pipeline, CTP::field_B04, effect_state);
    wr(this, TP::field_2BD0, effect_state);
    if effect_state == 0 {
        return;
    }

    let mut node: usize = rd(effect_state, DES::projLights);
    while node != 0 {
        let effect: usize = rd(node, LIST::effect);
        node = rd(node, LIST::next);
        if rd::<u8>(effect, EFFECT_HAS_PLANE) != 0 {
            let stage = stage_at(this, TP::field_CE0, rd(this, TP::field_18A0));
            if configure_stage_from_texture_prop(this, effect, stage) != 0 {
                setup_projected_texture_matrix(this, effect);
                wr(this, TP::field_18A0, rd::<u32>(this, TP::field_18A0).wrapping_add(1));
            }
        } else {
            let stage = stage_at(this, TP::field_6F8, rd(this, TP::field_CD8));
            if configure_stage_from_texture_prop(this, effect, stage) != 0 {
                wr(this, TP::field_CD8, rd::<u32>(this, TP::field_CD8).wrapping_add(1));
            }
        }
    }

    let mut node: usize = rd(effect_state, DES::projShadows);
    while node != 0 {
        let effect: usize = rd(node, LIST::effect);
        node = rd(node, LIST::next);
        if rd::<u8>(effect, EFFECT_HAS_PLANE) != 0 {
            // The game code takes the stage from `field_CE0` here also, not from `field_18A4`.
            let stage = stage_at(this, TP::field_CE0, rd(this, TP::field_2464));
            if configure_stage_from_texture_prop(this, effect, stage) != 0 {
                setup_projected_texture_matrix_neg(this, effect);
                wr(this, TP::field_2464, rd::<u32>(this, TP::field_2464).wrapping_add(1));
            }
        } else {
            let stage = stage_at(this, TP::field_2468, rd(this, TP::field_2A48));
            if configure_stage_from_texture_prop(this, effect, stage) != 0 {
                wr(this, TP::field_2A48, rd::<u32>(this, TP::field_2A48).wrapping_add(1));
            }
        }
    }

    let environment_map: usize = rd(effect_state, DES::environmentMap);
    if environment_map != 0 {
        let valid = configure_stage_from_texture_prop(this, environment_map, this + TP::field_2A4C);
        wr::<u8>(this, TP::field_2B08, valid);
    }
    let fog_map: usize = rd(effect_state, DES::fogMap);
    if fog_map != 0 {
        let valid = configure_stage_from_texture_prop(this, fog_map, this + TP::field_2B0C);
        wr::<u8>(this, TP::field_2BC8, valid);
    }
}

/// 0x6B59E0 `NiDX8TexturePass::resetPassFlags`
pub unsafe fn reset_pass_flags(this: usize) {
    crate::shadow::touch(&reset_pass_flags_port::STATE);
    wr::<u32>(this, TP::field_CD8, 0);
    wr::<u32>(this, TP::field_18A0, 0);
    wr::<u32>(this, TP::field_2A48, 0);
    wr::<u32>(this, TP::field_2464, 0);
    wr::<u8>(this, TP::field_2B08, 0);
    wr::<u8>(this, TP::field_2A4C_bumpEnvMapEnabled, 0);
    wr::<u8>(this, TP::field_2BC8, 0);
    wr::<u32>(this, TP::field_2BD0, 0);
    wr::<u32>(this, TP::field_6E0, 0);
}

/// 0x6B5A20 `NiDX8TexturePass::configureStageFromTextureProp`. Makes a stage from a
/// `NiTextureEffect`. Returns 0 when the effect gives no stage.
pub unsafe fn configure_stage_from_texture_prop(this: usize, effect: usize, stage: usize) -> u8 {
    crate::shadow::touch(&configure_stage_from_texture_prop_port::STATE);
    if rd::<u32>(this, TP::field_6E0) >= 8 {
        return 0;
    }
    if rd::<u8>(effect, EFFECT_ENABLED) == 0 {
        return 0;
    }
    let texture: usize = rd(effect, EFFECT_TEXTURE);
    if texture == 0 {
        return 0;
    }
    if texture_manager_get_format(rd(this, TP::textureManager), texture) == 14 {
        return 0;
    }
    wr::<u32>(stage, ST::sourceTexcoordIndex, 1);
    let previous: usize = rd(stage, ST::ptrTexture);
    if previous != texture {
        if previous != 0 {
            release(previous);
        }
        wr(stage, ST::ptrTexture, texture);
        add_reference(texture);
    }
    wr::<u8>(stage, ST::bumpEnvMapEnabled, 0);
    let mode: u32 = rd(effect, EFFECT_COORDINATE_MODE);
    let generate: unsafe extern "thiscall" fn(usize, usize, usize, usize) -> u32 =
        transmute(rd::<usize>(this.wrapping_add((mode as usize).wrapping_mul(4)), TP::callbacks_6E4));
    generate(this, effect + EFFECT_PROJECTION_MATRIX, effect + EFFECT_PROJECTION_TRANSLATION, stage);
    wr(this, TP::field_6E0, rd::<u32>(this, TP::field_6E0).wrapping_add(1));
    wr(stage, ST::texturePropMap, effect);
    wr::<u8>(stage, ST::field_90, 0);
    let filter_mode: u32 = rd(effect, EFFECT_FILTER_MODE);
    pipeline_set_stage_filter_mode(rd(this, TP::configurableTexturePipeline), filter_mode, stage);
    let clamp_mode: u32 = rd(effect, EFFECT_CLAMP_MODE);
    pipeline_set_stage_addressing_mode(rd(this, TP::configurableTexturePipeline), clamp_mode, stage);
    1
}

/// The body that the two plane functions share. `negate` is the `fchs` of the second function
/// on each of the four plane values.
#[inline(always)]
unsafe fn projected_texture_matrix(this: usize, effect: usize, stage: usize, negate: bool) -> u8 {
    let load = |offset: usize| {
        let value: f32 = rd(effect, EFFECT_PLANE + offset);
        if negate {
            -value
        } else {
            value
        }
    };
    let x = load(0x0);
    let y = load(0x4);
    let z = load(0x8);
    // The game code loads the constant with no change. The second function changes its sign
    // at the end.
    let constant: f32 = rd(effect, EFFECT_PLANE + 0xC);
    let pipeline: usize = rd(this, TP::configurableTexturePipeline);
    let p = |offset: usize| rd::<f32>(pipeline, offset);

    wr(stage, ST::textureMatrix__11, dot3(p(CTP::worldRightZ), z, p(CTP::worldRightY), y, p(CTP::worldRightX), x));
    wr(stage, ST::textureMatrix__21, dot3(p(CTP::worldUpZ), z, p(CTP::worldUpY), y, p(CTP::worldUpX), x));
    wr(stage, ST::textureMatrix__31, dot3(p(CTP::worldDirectionZ), z, p(CTP::worldDirectionY), y, p(CTP::worldDirectionX), x));

    let sum = dot3(z, p(CTP::worldLocation_z), y, p(CTP::worldLocation_y), x, p(CTP::worldLocation_x));
    // `fadd ds:f_half_0`: a single-precision 0.5 at 0x74B714.
    let sum = sum + 0.5f32;
    let last = if negate { sum - (-constant) } else { sum - constant };
    wr(stage, ST::textureMatrix__41, last);

    wr(this, TP::field_6E0, rd::<u32>(this, TP::field_6E0).wrapping_add(1));
    wr(stage, ST::texturePropMap, effect);
    1
}

/// 0x6B5B20 `NiDX8TexturePass::setupProjectedTextureMatrix`. Makes the clip stage of a
/// projected light from the plane of the effect in camera space.
pub unsafe fn setup_projected_texture_matrix(this: usize, effect: usize) -> u8 {
    crate::shadow::touch(&setup_projected_texture_matrix_port::STATE);
    let stage = stage_at(this, TP::field_12C0, rd(this, TP::field_18A0));
    projected_texture_matrix(this, effect, stage, false)
}

/// 0x6B5C00 `NiDX8TexturePass::setupProjectedTextureMatrixNeg`. Does the same for a projected
/// shadow, with the opposite sign on the plane.
pub unsafe fn setup_projected_texture_matrix_neg(this: usize, effect: usize) -> u8 {
    crate::shadow::touch(&setup_projected_texture_matrix_neg_port::STATE);
    let stage = stage_at(this, TP::field_1E84, rd(this, TP::field_2464));
    projected_texture_matrix(this, effect, stage, true)
}

// The callbacks that generate texture coordinates (`callbacks_6E4`). `w` is the 3x4 camera
// matrix of the pipeline: 0x30 bytes that start at `worldRightX`. `m` is the 3x3 matrix of
// the effect. The order of the operands is the order of the instructions.

/// Offsets into the camera matrix of the pipeline, from `worldRightX`.
#[inline(always)]
unsafe fn camera(pipeline: usize, offset: usize) -> f32 {
    rd(pipeline, CTP::worldRightX + offset)
}

/// 0x6B5CE0 `NiDX8TexturePass::texGen_WorldParallel_Claude_`. Leaves the pipeline in EAX.
pub unsafe fn tex_gen_world_parallel(this: usize, matrix: usize, translation: usize, stage: usize) -> usize {
    crate::shadow::touch(&tex_gen_world_parallel_port::STATE);
    let pipeline: usize = rd(this, TP::configurableTexturePipeline);
    world_parallel(pipeline, matrix, translation, stage);
    pipeline
}

/// The writes of `tex_gen_world_parallel`, with the pipeline given.
#[inline(always)]
unsafe fn world_parallel(pipeline: usize, matrix: usize, translation: usize, stage: usize) {
    wr::<u32>(stage, ST::sourceTexcoordIndex, 0x20000);
    wr::<u32>(stage, ST::textureTransformFlags, 2);
    let w = |offset: usize| camera(pipeline, offset);
    let m = |offset: usize| rd::<f32>(matrix, offset);
    let t = |offset: usize| rd::<f32>(translation, offset);

    wr(stage, 0x4C, dot3(w(0x00), m(0x00), w(0x18), m(0x08), w(0x0C), m(0x04)));
    wr(stage, 0x5C, dot3(w(0x1C), m(0x08), w(0x10), m(0x04), m(0x00), w(0x04)));
    wr(stage, 0x6C, dot3(m(0x04), w(0x14), m(0x00), w(0x08), m(0x08), w(0x20)));
    wr(stage, 0x50, dot3(w(0x18), m(0x14), w(0x0C), m(0x10), m(0x0C), w(0x00)));
    wr(stage, 0x60, dot3(w(0x1C), m(0x14), w(0x10), m(0x10), m(0x0C), w(0x04)));
    wr(stage, 0x70, dot3(m(0x0C), w(0x08), w(0x20), m(0x14), w(0x14), m(0x10)));
    wr(stage, 0x54, dot3(w(0x0C), m(0x1C), w(0x00), m(0x18), w(0x18), m(0x20)));
    wr(stage, 0x64, dot3(w(0x10), m(0x1C), w(0x04), m(0x18), w(0x1C), m(0x20)));
    wr(stage, 0x74, dot3(w(0x20), m(0x20), w(0x14), m(0x1C), w(0x08), m(0x18)));
    wr(stage, 0x7C, dot3(m(0x04), w(0x28), m(0x08), w(0x2C), m(0x00), w(0x24)) + t(0x0));
    wr(stage, 0x80, dot3(m(0x0C), w(0x24), m(0x10), w(0x28), m(0x14), w(0x2C)) + t(0x4));
    wr(stage, 0x84, dot3(m(0x18), w(0x24), m(0x1C), w(0x28), m(0x20), w(0x2C)) + t(0x8));
}

/// 0x6B5EB0 `NiDX8TexturePass::texGen_WorldPerspective_Claude_`. Leaves the pipeline in EAX.
pub unsafe fn tex_gen_world_perspective(this: usize, matrix: usize, translation: usize, stage: usize) -> usize {
    crate::shadow::touch(&tex_gen_world_perspective_port::STATE);
    let renderer: usize = rd(this, TP::renderer);
    let renderer_pipeline: usize = rd(renderer, REN::configurableTexturePipeline);
    let flags: u32 = rd(renderer_pipeline, CTP::field_310);
    let pipeline: usize = rd(this, TP::configurableTexturePipeline);
    world_perspective(pipeline, flags, matrix, translation, stage);
    pipeline
}

/// The writes of `tex_gen_world_perspective`, with the pipeline and the transform flags of
/// the pipeline of the renderer given.
#[inline(always)]
unsafe fn world_perspective(pipeline: usize, flags: u32, matrix: usize, translation: usize, stage: usize) {
    wr::<u32>(stage, ST::sourceTexcoordIndex, 0x20000);
    wr::<u32>(stage, ST::textureTransformFlags, flags);
    let w = |offset: usize| camera(pipeline, offset);
    let m = |offset: usize| rd::<f32>(matrix, offset);
    let t = |offset: usize| rd::<f32>(translation, offset);

    wr(stage, 0x4C, dot3(w(0x00), m(0x00), w(0x18), m(0x08), w(0x0C), m(0x04)));
    wr(stage, 0x5C, dot3(w(0x04), m(0x00), w(0x1C), m(0x08), w(0x10), m(0x04)));
    wr(stage, 0x6C, dot3(m(0x08), w(0x20), w(0x14), m(0x04), m(0x00), w(0x08)));
    wr(stage, 0x50, dot3(w(0x00), m(0x0C), m(0x10), w(0x0C), w(0x18), m(0x14)));
    wr(stage, 0x60, dot3(m(0x10), w(0x10), w(0x04), m(0x0C), w(0x1C), m(0x14)));
    wr(stage, 0x70, dot3(w(0x14), m(0x10), w(0x08), m(0x0C), w(0x20), m(0x14)));
    wr(stage, 0x54, dot3(w(0x18), m(0x20), m(0x1C), w(0x0C), w(0x00), m(0x18)));
    wr(stage, 0x64, dot3(w(0x1C), m(0x20), m(0x1C), w(0x10), w(0x04), m(0x18)));
    wr(stage, 0x74, dot3(w(0x08), m(0x18), w(0x20), m(0x20), w(0x14), m(0x1C)));
    wr(stage, 0x7C, dot3(m(0x04), w(0x28), m(0x08), w(0x2C), m(0x00), w(0x24)) + t(0x0));
    wr(stage, 0x80, dot3(m(0x14), w(0x2C), m(0x0C), w(0x24), m(0x10), w(0x28)) + t(0x4));
    wr(stage, 0x84, dot3(m(0x1C), w(0x28), m(0x20), w(0x2C), m(0x18), w(0x24)) + t(0x8));
}

/// The body of the three callbacks that work on a direction. It applies the rotation only and
/// copies the translation of the effect with no change. It leaves the last word of the
/// translation in EAX.
#[inline(always)]
unsafe fn tex_gen_direction(this: usize, matrix: usize, translation: usize, stage: usize, source: u32, flags: u32) -> u32 {
    let pipeline: usize = rd(this, TP::configurableTexturePipeline);
    direction(pipeline, matrix, translation, stage, source, flags)
}

/// The writes of `tex_gen_direction`, with the pipeline given.
#[inline(always)]
unsafe fn direction(pipeline: usize, matrix: usize, translation: usize, stage: usize, source: u32, flags: u32) -> u32 {
    wr::<u32>(stage, ST::sourceTexcoordIndex, source);
    wr::<u32>(stage, ST::textureTransformFlags, flags);
    let w = |offset: usize| camera(pipeline, offset);
    let m = |offset: usize| rd::<f32>(matrix, offset);

    wr(stage, 0x4C, dot3(w(0x18), m(0x08), w(0x00), m(0x00), w(0x0C), m(0x04)));
    wr(stage, 0x5C, dot3(w(0x1C), m(0x08), w(0x10), m(0x04), m(0x00), w(0x04)));
    wr(stage, 0x6C, dot3(w(0x08), m(0x00), m(0x08), w(0x20), m(0x04), w(0x14)));
    wr(stage, 0x50, dot3(w(0x00), m(0x0C), w(0x0C), m(0x10), m(0x14), w(0x18)));
    wr(stage, 0x60, dot3(w(0x10), m(0x10), m(0x0C), w(0x04), m(0x14), w(0x1C)));
    wr(stage, 0x70, dot3(m(0x14), w(0x20), w(0x08), m(0x0C), w(0x14), m(0x10)));
    wr(stage, 0x54, dot3(w(0x18), m(0x20), m(0x18), w(0x00), w(0x0C), m(0x1C)));
    wr(stage, 0x64, dot3(m(0x18), w(0x04), w(0x1C), m(0x20), w(0x10), m(0x1C)));
    wr(stage, 0x74, dot3(w(0x14), m(0x1C), w(0x08), m(0x18), w(0x20), m(0x20)));
    wr::<u32>(stage, 0x7C, rd(translation, 0x0));
    wr::<u32>(stage, 0x80, rd(translation, 0x4));
    let last: u32 = rd(translation, 0x8);
    wr(stage, 0x84, last);
    last
}

/// The number of words of `effect_stage_words`: the coordinate index, the transform flags
/// and the 16 values of the texture matrix.
pub const EFFECT_STAGE_WORDS: usize = 18;

/// The stage of a texture effect as its coordinate callback makes it, for the shape record
/// of the retained pipeline: the texture coordinate index with its generation, the transform
/// flags, then the texture matrix. The callback writes into a stage of zeros, as a stage is
/// after its constructor, with the pipeline of the renderer and its camera of the frame: the
/// same values the pass of the device would set for the draw. Returns zeros when the effect
/// gives no stage (off, or without a texture).
pub unsafe fn effect_stage_words(renderer: usize, effect: usize) -> [u32; EFFECT_STAGE_WORDS] {
    let mut words = [0u32; EFFECT_STAGE_WORDS];
    if effect == 0 || rd::<u8>(effect, EFFECT_ENABLED) == 0 || rd::<usize>(effect, EFFECT_TEXTURE) == 0 {
        return words;
    }
    let mut stage = [0u8; ST::SIZE];
    let at = stage.as_mut_ptr() as usize;
    let pipeline: usize = rd(renderer, REN::configurableTexturePipeline);
    let (matrix, translation) = (effect + EFFECT_PROJECTION_MATRIX, effect + EFFECT_PROJECTION_TRANSLATION);
    match rd::<u32>(effect, EFFECT_COORDINATE_MODE) {
        0 => world_parallel(pipeline, matrix, translation, at),
        1 => world_perspective(pipeline, rd(pipeline, CTP::field_310), matrix, translation, at),
        2 => {
            direction(pipeline, matrix, translation, at, 0x30000, 2);
        }
        3 => {
            direction(pipeline, matrix, translation, at, 0x30000, 3);
        }
        _ => {
            direction(pipeline, matrix, translation, at, 0x10000, 3);
        }
    }
    words[0] = rd(at, ST::sourceTexcoordIndex);
    words[1] = rd(at, ST::textureTransformFlags);
    words[2..].copy_from_slice(&rd::<[u32; 16]>(at, ST::textureMatrix__11));
    words
}

/// The number of words of `effect_static_words`: the 3x3 matrix and the translation of the
/// world projection, then the transform flags of the stage.
pub const EFFECT_STATIC_WORDS: usize = 13;

/// The values of a texture effect that do not change with the camera, for the material
/// record: the 3x3 matrix and the translation of its world projection, then the transform
/// flags its coordinate callback sets (2 for a parallel or a sphere map, those of the
/// pipeline for a perspective map, 3 for a cube map). The stage of the effect for any camera
/// is this matrix applied to a vector of the world (the position for the parallel and the
/// perspective map, the reflection for the sphere and the specular cube map, the normal for
/// the diffuse cube map) plus this translation: the camera matrix of the callbacks only turns
/// the vector of camera space into one of the world. The table of the helper makes the stage
/// of a retained draw from these words. Zeros when the effect gives no stage.
pub unsafe fn effect_static_words(renderer: usize, effect: usize) -> [u32; EFFECT_STATIC_WORDS] {
    let mut words = [0u32; EFFECT_STATIC_WORDS];
    if effect == 0 || rd::<u8>(effect, EFFECT_ENABLED) == 0 || rd::<usize>(effect, EFFECT_TEXTURE) == 0 {
        return words;
    }
    words[..9].copy_from_slice(&rd::<[u32; 9]>(effect, EFFECT_PROJECTION_MATRIX));
    words[9..12].copy_from_slice(&rd::<[u32; 3]>(effect, EFFECT_PROJECTION_TRANSLATION));
    words[12] = match rd::<u32>(effect, EFFECT_COORDINATE_MODE) {
        0 | 2 => 2,
        1 => rd::<u32>(rd::<usize>(renderer, REN::configurableTexturePipeline), CTP::field_310),
        _ => 3,
    };
    words
}

/// 0x6B6090 `NiDX8TexturePass::texGen_SphereMap_Claude_`
pub unsafe fn tex_gen_sphere_map(this: usize, matrix: usize, translation: usize, stage: usize) -> u32 {
    crate::shadow::touch(&tex_gen_sphere_map_port::STATE);
    tex_gen_direction(this, matrix, translation, stage, 0x30000, 2)
}

/// 0x6B61B0 `NiDX8TexturePass::texGen_SpecularCubeMap_Claude_`
pub unsafe fn tex_gen_specular_cube_map(this: usize, matrix: usize, translation: usize, stage: usize) -> u32 {
    crate::shadow::touch(&tex_gen_specular_cube_map_port::STATE);
    tex_gen_direction(this, matrix, translation, stage, 0x30000, 3)
}

/// 0x6B6300 `NiDX8TexturePass::texGen_DiffuseCubeMap_Claude_`
pub unsafe fn tex_gen_diffuse_cube_map(this: usize, matrix: usize, translation: usize, stage: usize) -> u32 {
    crate::shadow::touch(&tex_gen_diffuse_cube_map_port::STATE);
    tex_gen_direction(this, matrix, translation, stage, 0x10000, 3)
}

// ---------------------------------------------------------------------------------------
// NiDX8TexturePass::$1, the sub-pass
// ---------------------------------------------------------------------------------------

/// 0x6B6540 `NiDX8TexturePass::_1::setNiObjectField48`. The assignment of a `NiPointer`, as
/// the compiler made it: a reset to null, then the assignment.
pub unsafe fn subpass_set_ni_object_field_48(this: usize, object: usize) {
    crate::shadow::touch(&subpass_set_ni_object_field_48_port::STATE);
    let current: usize = rd(this, SUB::field_48);
    if current == object {
        return;
    }
    if current != 0 {
        release(current);
        wr::<usize>(this, SUB::field_48, 0);
    }
    let current: usize = rd(this, SUB::field_48);
    if current == object {
        return;
    }
    if current != 0 {
        release(current);
    }
    wr(this, SUB::field_48, object);
    if object != 0 {
        add_reference(object);
    }
}

/// 0x6B76E0 `NiDX8TexturePass::$1::ctor`
pub unsafe fn subpass_ctor(this: usize) -> usize {
    crate::shadow::touch(&subpass_ctor_port::STATE);
    wr(this, SUB::vtbl, VTBL_NI_REF_OBJECT);
    wr::<u32>(this, SUB::refCount, 0);
    wr(NI_REF_OBJECT_COUNT, 0, rd::<u32>(NI_REF_OBJECT_COUNT, 0).wrapping_add(1));
    wr::<usize>(this, SUB::field_48, 0);
    wr::<usize>(this, SUB::field_4C, 0);
    wr::<usize>(this, SUB::field_50, 0);
    wr(this, SUB::vtbl, VTBL_SUBPASS);
    wr::<u32>(this, SUB::field_8, 0);
    wr::<u32>(this, SUB::field_C, u32::MAX);
    wr::<u32>(this, SUB::field_10, 0);
    for index in 0..8 {
        wr::<usize>(this, SUB::field_14 + index * 4, 0);
    }
    wr::<u32>(this, SUB::field_34, 0);
    wr::<u32>(this, SUB::field_38, 0);
    // A string copy into the bytes at 0x3C. The copy includes the terminator.
    let mut at = 0;
    loop {
        let byte: u8 = rd(EMPTY_STRING, at);
        wr(this, SUB::field_3C + at, byte);
        at += 1;
        if byte == 0 {
            break;
        }
    }
    wr::<u32>(this, SUB::field_44, 0);
    for field in [SUB::field_48, SUB::field_4C, SUB::field_50] {
        let object: usize = rd(this, field);
        if object != 0 {
            release(object);
            wr::<usize>(this, field, 0);
        }
    }
    this
}

/// 0x6B77E0 `NiDX8TexturePass::$1::dtor`
pub unsafe fn subpass_dtor(this: usize) {
    crate::shadow::touch(&subpass_dtor_port::STATE);
    wr(this, SUB::vtbl, VTBL_SUBPASS);
    for field in [SUB::field_48, SUB::field_4C, SUB::field_50] {
        let object: usize = rd(this, field);
        if object != 0 {
            release(object);
            wr::<usize>(this, field, 0);
        }
    }
    // The destructors of the three members, in reverse order.
    for field in [SUB::field_50, SUB::field_4C, SUB::field_48] {
        let object: usize = rd(this, field);
        if object != 0 {
            release(object);
        }
    }
    wr(this, SUB::vtbl, VTBL_NI_REF_OBJECT);
    wr(NI_REF_OBJECT_COUNT, 0, rd::<u32>(NI_REF_OBJECT_COUNT, 0).wrapping_sub(1));
}

/// 0x6B78D0 `NiDX8TexturePass::_1::setStageAtIndex`. EAX is the index. When the stage count
/// changed, EAX is the new stage count.
pub unsafe fn subpass_set_stage_at_index(this: usize, index: u32, stage: usize) -> u32 {
    crate::shadow::touch(&subpass_set_stage_at_index_port::STATE);
    let slot = this.wrapping_add((index as usize).wrapping_mul(4));
    if stage != rd::<usize>(slot, SUB::field_14) {
        wr(slot, SUB::field_14, stage);
    }
    let count: u32 = rd(this, SUB::field_C);
    if count != u32::MAX && index <= count.wrapping_sub(1) {
        return index;
    }
    let count = index.wrapping_add(1);
    wr(this, SUB::field_C, count);
    count
}

/// 0x6B7900 `NiDX8TexturePass::method_4`, slot 1 of `$1`. Sends the stages of the sub-pass
/// to the device and disables the stages after them. The function uses only the renderer of
/// the five arguments.
pub unsafe fn subpass_apply(this: usize, renderer: usize, _a2: u32, _a3: u32, _a4: u32, _a5: u32) -> u32 {
    crate::shadow::touch(&subpass_apply_port::STATE);
    let count: u32 = rd(this, SUB::field_C);
    if count == u32::MAX {
        return 0x4000_0000;
    }
    let mut index = 0u32;
    if count != 0 {
        let mut slot = this + SUB::field_14;
        loop {
            texture_stage_update_renderer(rd(slot, 0), renderer);
            let count: u32 = rd(this, SUB::field_C);
            index = index.wrapping_add(1);
            slot += 4;
            if index >= count {
                break;
            }
        }
    }
    reset_stage(index, renderer);
    let render_state: usize = rd(renderer, REN::renderState);
    if index < rd::<u32>(MAX_TEXTURE_BLEND_STAGES, 0) {
        let mut setting = stage_setting(render_state, index, D3DTSS_TEXCOORDINDEX);
        loop {
            if rd::<u32>(setting, 0) != index {
                wr(setting, 0, index);
                engine::device_set_texture_stage_state(rd(render_state, RS::device), index, D3DTSS_TEXCOORDINDEX, index);
            }
            let limit: u32 = rd(MAX_TEXTURE_BLEND_STAGES, 0);
            index = index.wrapping_add(1);
            setting = setting.wrapping_add(STAGE_SETTINGS);
            if index >= limit {
                break;
            }
        }
    }
    0
}

/// `getAvailableSlotCounts` with the two limits of the device as arguments.
#[inline(always)]
unsafe fn available_slot_counts(this: usize, out_stages: usize, out_textures: usize, max_stages: impl Fn() -> u32, max_textures: impl Fn() -> u32) {
    let count: u32 = rd(this, SUB::field_C);
    if count != u32::MAX {
        wr(out_stages, 0, max_stages().wrapping_sub(count));
    } else {
        wr(out_stages, 0, max_stages());
    }
    let limit = max_textures();
    wr(out_textures, 0, limit.wrapping_sub(rd::<u32>(this, SUB::field_10)));
}

/// 0x6B7990 `NiDX8TexturePass::_1::getAvailableSlotCounts`
pub unsafe fn subpass_get_available_slot_counts(this: usize, out_stages: usize, out_textures: usize) {
    crate::shadow::touch(&subpass_get_available_slot_counts_port::STATE);
    available_slot_counts(this, out_stages, out_textures, || rd(MAX_TEXTURE_BLEND_STAGES, 0), || rd(MAX_SIMULTANEOUS_TEXTURES, 0));
}

/// The end of every branch of the two append functions. The stage count becomes the next
/// index when it is less. A stage with a texture adds 1 to the texture count.
#[inline(always)]
unsafe fn count_appended(this: usize, stage: usize, count: u32, next: u32) {
    if count < next || count == u32::MAX {
        wr(this, SUB::field_C, next);
    }
    if rd::<usize>(stage, ST::ptrTexture) != 0 {
        wr(this, SUB::field_10, rd::<u32>(this, SUB::field_10).wrapping_add(1));
    }
}

/// The start of every branch. The stage gets the next index and goes into the list. Returns
/// the stage count before the call and the next index after it.
#[inline(always)]
unsafe fn store_appended(this: usize, stage: usize) -> (u32, u32) {
    wr::<u32>(stage, ST::stage, rd(this, SUB::field_8));
    let index: u32 = rd(this, SUB::field_8);
    wr(this.wrapping_add((index as usize).wrapping_mul(4)), SUB::field_14, stage);
    let next = rd::<u32>(this, SUB::field_8).wrapping_add(1);
    let count: u32 = rd(this, SUB::field_C);
    wr(this, SUB::field_8, next);
    (count, next)
}

/// 0x6B79E0 `NiDX8TexturePass::_1::appendStage`
pub unsafe fn subpass_append_stage(this: usize, stage: usize) -> u8 {
    crate::shadow::touch(&subpass_append_stage_port::STATE);
    if stage == 0 {
        return 0;
    }
    wr::<u32>(stage, ST::stage, rd(this, SUB::field_8));
    let index: u32 = rd(this, SUB::field_8);
    wr(this.wrapping_add((index as usize).wrapping_mul(4)), SUB::field_14, stage);
    if rd::<u32>(this, SUB::field_8) != 0 {
        let texture: usize = rd(stage, ST::ptrTexture);
        wr::<u32>(stage, ST::colorArg2, 1);
        if texture != 0 {
            wr::<u32>(stage, ST::colorArg1, 2);
        } else {
            wr::<u32>(stage, ST::colorArg1, 0);
        }
    }
    let next = rd::<u32>(this, SUB::field_8).wrapping_add(1);
    let count: u32 = rd(this, SUB::field_C);
    wr(this, SUB::field_8, next);
    count_appended(this, stage, count, next);
    1
}

/// `appendStagePair` with the texture limit of the device as an argument.
#[inline(always)]
unsafe fn append_stage_pair(this: usize, stage: usize, other_stage: usize, flag: u8, max_textures: u32) -> u32 {
    let textures: u32 = rd(this, SUB::field_10);
    if textures >= max_textures {
        return 0;
    }
    if other_stage == 0 {
        if stage == 0 {
            return 0;
        }
        let (count, next) = store_appended(this, stage);
        count_appended(this, stage, count, next);
        return 1;
    }
    if stage == 0 {
        if flag != 0 {
            return 0;
        }
        let (count, next) = store_appended(this, other_stage);
        count_appended(this, other_stage, count, next);
        return 2;
    }
    let room_for_two = textures < max_textures.wrapping_sub(1);
    let (count, next) = store_appended(this, stage);
    count_appended(this, stage, count, next);
    if !room_for_two {
        return 1;
    }
    wr(other_stage, ST::stage, next);
    let index: u32 = rd(this, SUB::field_8);
    wr(this.wrapping_add((index as usize).wrapping_mul(4)), SUB::field_14, other_stage);
    let next = rd::<u32>(this, SUB::field_8).wrapping_add(1);
    wr(this, SUB::field_8, next);
    let count: u32 = rd(this, SUB::field_C);
    count_appended(this, other_stage, count, next);
    3
}

/// 0x6B7A50 `NiDX8TexturePass::_1::appendStagePair`. Returns bit 0 for the first stage and
/// bit 1 for the second.
pub unsafe fn subpass_append_stage_pair(this: usize, stage: usize, other_stage: usize, flag: u8) -> u32 {
    crate::shadow::touch(&subpass_append_stage_pair_port::STATE);
    append_stage_pair(this, stage, other_stage, flag, rd(MAX_SIMULTANEOUS_TEXTURES, 0))
}

// ---------------------------------------------------------------------------------------
// Stage state on the device
// ---------------------------------------------------------------------------------------

/// 0x6C1E50 `NiDX8TexturePass_static::resetStage`. Sets no texture and disables the two
/// operations.
pub unsafe fn reset_stage(stage_index: u32, renderer: usize) {
    crate::shadow::touch(&reset_stage_port::STATE);
    let render_state: usize = rd(renderer, REN::renderState);
    let texture = render_state.wrapping_add((stage_index as usize).wrapping_mul(4));
    if rd::<usize>(texture, RS::texturePerStage) != 0 {
        wr::<usize>(texture, RS::texturePerStage, 0);
        device_set_texture(rd(render_state, RS::device), stage_index, 0);
    }
    set_stage_state_if_changed(render_state, stage_index, D3DTSS_COLOROP, 1);
    set_stage_state_if_changed(render_state, stage_index, D3DTSS_ALPHAOP, 1);
}

/// 0x6C1EE0 `NiDX8TexturePass::setTextureWrapping`. `this` is a `NiDX8TextureStage`.
pub unsafe fn set_texture_wrapping(this: usize, renderer: usize) {
    crate::shadow::touch(&set_texture_wrapping_port::STATE);
    let render_state: usize = rd(renderer, REN::renderState);
    let stage: u32 = rd(this, ST::stage);
    let address_u: u32 = rd(this, ST::addressModeU);
    let address_v: u32 = rd(this, ST::addressModeV);
    set_stage_state_if_changed(render_state, stage, D3DTSS_ADDRESSU, address_u);
    let stage: u32 = rd(this, ST::stage);
    let pipeline: usize = rd(renderer, REN::configurableTexturePipeline);
    if rd::<u8>(pipeline, CTP::field_FC) & 1 != 0 {
        set_stage_state_if_changed(render_state, stage, D3DTSS_ADDRESSV, address_v);
    } else {
        set_stage_state_if_changed(render_state, stage, D3DTSS_ADDRESSV, address_u);
    }
}

/// `setTextureFiltering` with the address of the filter table as an argument.
#[inline(always)]
unsafe fn texture_filtering(this: usize, renderer: usize, mipmapped: u8, flag: u8, table: usize) {
    let render_state: usize = rd(renderer, REN::renderState);
    let mut mode: u32;
    if mipmapped != 0 {
        mode = rd(this, ST::textureStageIndex);
        if flag == 0 {
            if mode == 2 {
                mode = 5;
            } else if mode == 4 {
                mode = 3;
            }
        }
    } else {
        mode = rd(this, ST::textureStageIndex);
        if mode == 3 || mode == 4 {
            mode = 0;
        } else if mode == 5 || mode == 2 {
            mode = 1;
        }
    }
    let entry = table.wrapping_add((mode.wrapping_mul(3) << 2) as usize);
    let stage: u32 = rd(this, ST::stage);
    set_stage_state_if_changed(render_state, stage, D3DTSS_MAGFILTER, rd(entry, FILTER_MAG));
    let stage: u32 = rd(this, ST::stage);
    set_stage_state_if_changed(render_state, stage, D3DTSS_MINFILTER, rd(entry, FILTER_MIN));
    let stage: u32 = rd(this, ST::stage);
    set_stage_state_if_changed(render_state, stage, D3DTSS_MIPFILTER, rd(entry, FILTER_MIP));
}

/// 0x6C1FA0 `NiDX8TexturePass::setTextureFiltering`. `this` is a `NiDX8TextureStage`. A
/// texture with no mipmaps gets the filter modes that need none.
pub unsafe fn set_texture_filtering(this: usize, renderer: usize, mipmapped: u8, flag: u8) {
    crate::shadow::touch(&set_texture_filtering_port::STATE);
    texture_filtering(this, renderer, mipmapped, flag, FILTER_TABLE)
}

// ---------------------------------------------------------------------------------------
// Regions for the shadow comparison
// ---------------------------------------------------------------------------------------

/// The reference count of an object as a region of its own.
fn add_ref_count(set: &mut Regions, object: usize) {
    set.add(object + REF_COUNT, 4);
}

fn has_region(set: &Regions, addr: usize) -> bool {
    set.as_slice().iter().any(|&(start, _)| start == addr)
}

/// True when `NiDX8TextureManager::PrepareTextureForRendering` makes or fills the device
/// texture of this texture. That work cannot run two times.
unsafe fn texture_needs_device_work(texture: usize) -> bool {
    let data: usize = rd(texture, NiTexture::rendererData);
    if data == 0 {
        return true;
    }
    if engine::vcall0(data, TEXTURE_DATA_GET_AS_RENDERED) != 0 {
        return false;
    }
    if engine::vcall0(data, TEXTURE_DATA_GET_D3D_TEXTURE) == 0 {
        return true;
    }
    rd::<u8>(texture, SOURCE_TEXTURE_IS_STATIC) == 0
}

/// The memory that `method_4` writes: the render state and the stages. The Code Patch changes
/// the operations of an environment-map stage in `updateRenderer`. `inside` is a block that
/// the set has before the call.
///
/// The call is a skip when the stage count is more than 8, because the sub-pass has 8 stage
/// slots. The call is a skip when a stage slot below the count is null. The call is a skip
/// when the texture manager must make or fill the device texture of a stage.
unsafe fn subpass_apply_regions(set: &mut Regions, this: usize, renderer: usize, inside: (usize, usize)) {
    let count: u32 = rd(this, SUB::field_C);
    if count == u32::MAX {
        return;
    }
    let render_state: usize = rd(renderer, REN::renderState);
    if !has_region(set, render_state) {
        set.add(render_state, RS::SIZE);
    }
    if count > 8 {
        set.skip();
        return;
    }
    for index in 0..count as usize {
        let stage: usize = rd(this, SUB::field_14 + index * 4);
        if stage == 0 {
            set.skip();
            return;
        }
        let texture: usize = rd(stage, ST::ptrTexture);
        if texture == 0 {
            continue;
        }
        if texture_needs_device_work(texture) {
            set.skip();
            return;
        }
        if stage >= inside.0 && stage + ST::SIZE <= inside.0 + inside.1 {
            continue;
        }
        set.add(stage, ST::SIZE);
    }
}

/// The call is a skip when slot 4 of the sub-pass is not `method_4` of this file. It is also
/// a skip when `subpass_apply_regions` makes it a skip.
unsafe fn set_blending_state_regions(set: &mut Regions, this: usize) {
    set.add(this, TP::SIZE);
    let index: u32 = rd(this, TP::field_8);
    let storage: usize = rd(this, TP::TArray_18_storage);
    let subpass: usize = rd(storage, index as usize * 4);
    if subpass == 0 {
        return;
    }
    // `patchSetEffectState` writes the alpha states and the fog states.
    set.add(rd(this, TP::renderState), RS::SIZE);
    if engine::virtual_at(subpass, 4) != ADDR_METHOD_4 {
        // The object is not of the class that this file has.
        set.skip();
        return;
    }
    subpass_apply_regions(set, subpass, rd(this, TP::renderer), (this, TP::SIZE));
}

/// The two objects that an assignment to `field_48` changes. The call is a skip when the old
/// object loses its last reference, because the call deletes the object.
unsafe fn field_48_regions(set: &mut Regions, subpass: usize, object: usize) {
    let current: usize = rd(subpass, SUB::field_48);
    if current == object {
        return;
    }
    if current != 0 {
        // The last reference: the assignment releases the old object one time.
        if rd::<u32>(current, REF_COUNT) <= 1 {
            set.skip();
        }
        add_ref_count(set, current);
    }
    if object != 0 {
        add_ref_count(set, object);
    }
}

/// The call is a skip when `acquireSubpassWithCapacity` makes a sub-pass or makes the list of
/// sub-passes longer. It is also a skip when `field_48_regions` makes it a skip.
unsafe fn bind_stage_texture_regions(set: &mut Regions, this: usize, stage: usize, textures: u32, stages: u32) {
    set.add(this, TP::SIZE);
    if stage == 0 {
        return;
    }
    // The code below finds what `acquireSubpassWithCapacity` does in this call.
    let mut pass_index: u32 = rd(this, TP::field_C);
    let storage: usize = rd(this, TP::TArray_18_storage);
    let mut subpass: usize = rd(this, TP::substruct_14);
    if subpass == 0 {
        subpass = rd(storage, pass_index as usize * 4);
        if subpass == 0 {
            // The function makes the sub-pass.
            set.skip();
            return;
        }
    }
    let mut free_stages = 0u32;
    let mut free_textures = 0u32;
    subpass_get_available_slot_counts(subpass, &mut free_stages as *mut u32 as usize, &mut free_textures as *mut u32 as usize);
    if free_stages < stages || free_textures < textures {
        pass_index = pass_index.wrapping_add(1);
        if pass_index >= rd::<u32>(this, TP::count_10) {
            // The function makes the list of sub-passes longer.
            set.skip();
            return;
        }
        subpass = rd(storage, pass_index as usize * 4);
        if subpass == 0 {
            set.skip();
            return;
        }
    }
    set.add(subpass, SUB::SIZE);
    set.add(stage, ST::SIZE);
    if pass_index == 0 {
        return;
    }
    let first_stage: usize = if rd::<u32>(subpass, SUB::field_8) == 0 { stage } else { rd(subpass, SUB::field_14) };
    let pipeline: usize = rd(this, TP::configurableTexturePipeline);
    let slot: u32 = rd(first_stage, ST::field_94);
    let object: usize = rd(pipeline.wrapping_add((slot as usize).wrapping_mul(4)), CTP::field_2E8);
    field_48_regions(set, subpass, object);
}

/// The reference counts that `setTexturePipeline` changes. A walk over the effects finds
/// them, in the order in which the function walks the effects.
struct EffectWalk {
    /// `NiDX8TexturePass::field_6E0` as the function has it at this point of the walk.
    used: u32,
    /// A texture and the change of its reference count.
    counts: Vec<(usize, i32)>,
    /// A stage and the texture that the walk gave it.
    stages: Vec<(usize, usize)>,
    skip: bool,
}

impl EffectWalk {
    fn change(&mut self, texture: usize, by: i32) {
        for entry in &mut self.counts {
            if entry.0 == texture {
                entry.1 += by;
                return;
            }
        }
        self.counts.push((texture, by));
    }

    unsafe fn texture_of(&self, stage: usize) -> usize {
        for &(known, texture) in self.stages.iter().rev() {
            if known == stage {
                return texture;
            }
        }
        rd(stage, ST::ptrTexture)
    }

    /// `configureStageFromTextureProp` without the writes.
    unsafe fn configure(&mut self, pass: usize, effect: usize, stage: usize) -> bool {
        if self.skip || self.used >= 8 || rd::<u8>(effect, EFFECT_ENABLED) == 0 {
            return false;
        }
        let texture: usize = rd(effect, EFFECT_TEXTURE);
        if texture == 0 {
            return false;
        }
        if rd::<usize>(texture, NiTexture::rendererData) == 0 {
            // `GetFormatFromTexture` makes the device texture.
            self.skip = true;
            return false;
        }
        if texture_manager_get_format(rd(pass, TP::textureManager), texture) == 14 {
            return false;
        }
        let previous = self.texture_of(stage);
        if previous != texture {
            if previous != 0 {
                self.change(previous, -1);
            }
            self.change(texture, 1);
            self.stages.push((stage, texture));
        }
        // The last reference: a count that becomes 0 during the walk deletes the texture.
        for &(object, by) in &self.counts {
            if rd::<u32>(object, REF_COUNT) as i64 + by as i64 <= 0 {
                self.skip = true;
            }
        }
        self.used += 1;
        true
    }
}

/// The call is a skip when slot 0x18 of the pass is not `resetPassFlags` of this file. The
/// call is a skip when `GetFormatFromTexture` must make the device texture of an effect. The
/// call is a skip when the reference count of a texture becomes 0 during the walk.
unsafe fn set_texture_pipeline_regions(set: &mut Regions, this: usize, effect_state: usize) {
    set.add(this, TP::SIZE);
    let pipeline: usize = rd(this, TP::configurableTexturePipeline);
    if effect_state == rd::<usize>(pipeline, CTP::field_B04) {
        return;
    }
    set.add(pipeline + CTP::field_B04, 4);
    if engine::virtual_at(this, 0x18) != ADDR_RESET_PASS_FLAGS {
        // The object is not of the class that this file has.
        set.skip();
        return;
    }
    if effect_state == 0 {
        return;
    }
    let mut walk = EffectWalk { used: 0, counts: Vec::new(), stages: Vec::new(), skip: false };
    // The four counters are 0 after `resetPassFlags`.
    let (mut lights, mut clipped_lights, mut shadows, mut clipped_shadows) = (0u32, 0u32, 0u32, 0u32);

    let mut node: usize = rd(effect_state, DES::projLights);
    while node != 0 && !walk.skip {
        let effect: usize = rd(node, LIST::effect);
        node = rd(node, LIST::next);
        if rd::<u8>(effect, EFFECT_HAS_PLANE) != 0 {
            if walk.configure(this, effect, stage_at(this, TP::field_CE0, clipped_lights)) {
                walk.used += 1;
                clipped_lights += 1;
            }
        } else if walk.configure(this, effect, stage_at(this, TP::field_6F8, lights)) {
            lights += 1;
        }
    }
    let mut node: usize = rd(effect_state, DES::projShadows);
    while node != 0 && !walk.skip {
        let effect: usize = rd(node, LIST::effect);
        node = rd(node, LIST::next);
        if rd::<u8>(effect, EFFECT_HAS_PLANE) != 0 {
            if walk.configure(this, effect, stage_at(this, TP::field_CE0, clipped_shadows)) {
                walk.used += 1;
                clipped_shadows += 1;
            }
        } else if walk.configure(this, effect, stage_at(this, TP::field_2468, shadows)) {
            shadows += 1;
        }
    }
    let environment_map: usize = rd(effect_state, DES::environmentMap);
    if environment_map != 0 {
        walk.configure(this, environment_map, this + TP::field_2A4C);
    }
    let fog_map: usize = rd(effect_state, DES::fogMap);
    if fog_map != 0 {
        walk.configure(this, fog_map, this + TP::field_2B0C);
    }
    if walk.skip {
        set.skip();
        return;
    }
    for &(texture, _) in &walk.counts {
        add_ref_count(set, texture);
    }
}

/// The call is a skip when `GetFormatFromTexture` must make the device texture of the effect.
/// The call is a skip when the texture that the stage had loses its last reference.
unsafe fn configure_stage_regions(set: &mut Regions, this: usize, effect: usize, stage: usize) {
    set.add(this, TP::SIZE);
    if rd::<u32>(this, TP::field_6E0) >= 8 || rd::<u8>(effect, EFFECT_ENABLED) == 0 {
        return;
    }
    let texture: usize = rd(effect, EFFECT_TEXTURE);
    if texture == 0 {
        return;
    }
    if rd::<usize>(texture, NiTexture::rendererData) == 0 {
        // `GetFormatFromTexture` makes the device texture.
        set.skip();
        return;
    }
    set.add(stage, ST::SIZE);
    let previous: usize = rd(stage, ST::ptrTexture);
    if previous != texture {
        if previous != 0 {
            // The last reference: the assignment releases the old texture one time.
            if rd::<u32>(previous, REF_COUNT) <= 1 {
                set.skip();
            }
            add_ref_count(set, previous);
        }
        add_ref_count(set, texture);
    }
}

/// No call is a skip. The function writes the sub-pass and the two stages.
unsafe fn subpass_append_regions(set: &mut Regions, this: usize, stage: usize, other_stage: usize) {
    set.add(this, SUB::SIZE);
    set.add(stage, ST::SIZE);
    set.add(other_stage, ST::SIZE);
}

// ---------------------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------------------

port_thiscall!(set_blending_state_port, 0x6B5640, "NiDX8TexturePass::setBlendingState",
    fn(this, a2: u32, a3: u32, a4: u32, a5: u32) -> u32,
    compare_eax = true,
    regions = |set| set_blending_state_regions(set, this),
    rust = set_blending_state);

port_thiscall!(apply_stage_blend_ops_port, 0x6B56B0, "NiDX8TexturePass::applyStageBlendOps",
    fn(this, stage: usize, flag: u8) -> (),
    compare_eax = false,
    regions = |set| set.add(stage, ST::SIZE),
    rust = apply_stage_blend_ops);

port_thiscall!(bind_stage_texture_port, 0x6B57B0, "NiDX8TexturePass::bindStageTexture",
    fn(this, stage: usize, textures: u32, stages: u32) -> u8,
    compare_eax = true,
    regions = |set| bind_stage_texture_regions(set, this, stage, textures, stages),
    rust = bind_stage_texture);

port_thiscall!(set_texture_pipeline_port, 0x6B5870, "NiDX8TexturePass::setTexturePipeline",
    fn(this, effect_state: usize) -> (),
    compare_eax = false,
    regions = |set| set_texture_pipeline_regions(set, this, effect_state),
    rust = set_texture_pipeline);

port_thiscall!(reset_pass_flags_port, 0x6B59E0, "NiDX8TexturePass::resetPassFlags",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| set.add(this, TP::SIZE),
    rust = reset_pass_flags);

port_thiscall!(configure_stage_from_texture_prop_port, 0x6B5A20, "NiDX8TexturePass::configureStageFromTextureProp",
    fn(this, effect: usize, stage: usize) -> u8,
    compare_eax = true,
    regions = |set| configure_stage_regions(set, this, effect, stage),
    rust = configure_stage_from_texture_prop);

port_thiscall!(setup_projected_texture_matrix_port, 0x6B5B20, "NiDX8TexturePass::setupProjectedTextureMatrix",
    fn(this, effect: usize) -> u8,
    compare_eax = true,
    regions = |set| set.add(this, TP::SIZE),
    rust = setup_projected_texture_matrix);

port_thiscall!(setup_projected_texture_matrix_neg_port, 0x6B5C00, "NiDX8TexturePass::setupProjectedTextureMatrixNeg",
    fn(this, effect: usize) -> u8,
    compare_eax = true,
    regions = |set| set.add(this, TP::SIZE),
    rust = setup_projected_texture_matrix_neg);

port_thiscall!(tex_gen_world_parallel_port, 0x6B5CE0, "NiDX8TexturePass::texGen_WorldParallel_Claude_",
    fn(this, matrix: usize, translation: usize, stage: usize) -> usize,
    compare_eax = true,
    regions = |set| set.add(stage, ST::SIZE),
    rust = tex_gen_world_parallel);

port_thiscall!(tex_gen_world_perspective_port, 0x6B5EB0, "NiDX8TexturePass::texGen_WorldPerspective_Claude_",
    fn(this, matrix: usize, translation: usize, stage: usize) -> usize,
    compare_eax = true,
    regions = |set| set.add(stage, ST::SIZE),
    rust = tex_gen_world_perspective);

port_thiscall!(tex_gen_sphere_map_port, 0x6B6090, "NiDX8TexturePass::texGen_SphereMap_Claude_",
    fn(this, matrix: usize, translation: usize, stage: usize) -> u32,
    compare_eax = true,
    regions = |set| set.add(stage, ST::SIZE),
    rust = tex_gen_sphere_map);

port_thiscall!(tex_gen_specular_cube_map_port, 0x6B61B0, "NiDX8TexturePass::texGen_SpecularCubeMap_Claude_",
    fn(this, matrix: usize, translation: usize, stage: usize) -> u32,
    compare_eax = true,
    regions = |set| set.add(stage, ST::SIZE),
    rust = tex_gen_specular_cube_map);

port_thiscall!(tex_gen_diffuse_cube_map_port, 0x6B6300, "NiDX8TexturePass::texGen_DiffuseCubeMap_Claude_",
    fn(this, matrix: usize, translation: usize, stage: usize) -> u32,
    compare_eax = true,
    regions = |set| set.add(stage, ST::SIZE),
    rust = tex_gen_diffuse_cube_map);

port_thiscall!(subpass_set_ni_object_field_48_port, 0x6B6540, "NiDX8TexturePass::_1::setNiObjectField48",
    fn(this, object: usize) -> (),
    compare_eax = false,
    regions = |set| {
        set.add(this, SUB::SIZE);
        field_48_regions(set, this, object)
    },
    rust = subpass_set_ni_object_field_48);

port_thiscall!(subpass_ctor_port, 0x6B76E0, "NiDX8TexturePass::$1::ctor",
    fn(this) -> usize,
    compare_eax = true,
    // The call is a skip always: the object is new memory, and the count of all objects is
    // outside the object.
    regions = |set| set.skip(),
    rust = subpass_ctor);

port_thiscall!(subpass_dtor_port, 0x6B77E0, "NiDX8TexturePass::$1::dtor",
    fn(this) -> (),
    compare_eax = false,
    // The call is a skip always: the function can delete the three objects that the sub-pass
    // holds.
    regions = |set| set.skip(),
    rust = subpass_dtor);

port_thiscall!(subpass_set_stage_at_index_port, 0x6B78D0, "NiDX8TexturePass::_1::setStageAtIndex",
    fn(this, index: u32, stage: usize) -> u32,
    compare_eax = true,
    regions = |set| set.add(this, SUB::SIZE),
    rust = subpass_set_stage_at_index);

port_thiscall!(subpass_apply_port, 0x6B7900, "NiDX8TexturePass::method_4",
    fn(this, renderer: usize, a2: u32, a3: u32, a4: u32, a5: u32) -> u32,
    compare_eax = true,
    regions = |set| subpass_apply_regions(set, this, renderer, (0, 0)),
    rust = subpass_apply);

port_thiscall!(subpass_get_available_slot_counts_port, 0x6B7990, "NiDX8TexturePass::_1::getAvailableSlotCounts",
    fn(this, out_stages: usize, out_textures: usize) -> (),
    compare_eax = false,
    regions = |set| {
        set.add(out_stages, 4);
        set.add(out_textures, 4)
    },
    rust = subpass_get_available_slot_counts);

port_thiscall!(subpass_append_stage_port, 0x6B79E0, "NiDX8TexturePass::_1::appendStage",
    fn(this, stage: usize) -> u8,
    compare_eax = true,
    regions = |set| subpass_append_regions(set, this, stage, 0),
    rust = subpass_append_stage);

port_thiscall!(subpass_append_stage_pair_port, 0x6B7A50, "NiDX8TexturePass::_1::appendStagePair",
    fn(this, stage: usize, other_stage: usize, flag: u8) -> u32,
    compare_eax = true,
    regions = |set| subpass_append_regions(set, this, stage, other_stage),
    rust = subpass_append_stage_pair);

port_cdecl!(reset_stage_port, 0x6C1E50, "NiDX8TexturePass_static::resetStage",
    fn(stage_index: u32, renderer: usize) -> (),
    compare_eax = false,
    regions = |set| set.add(rd(renderer, REN::renderState), RS::SIZE),
    rust = reset_stage);

port_thiscall!(set_texture_wrapping_port, 0x6C1EE0, "NiDX8TexturePass::setTextureWrapping",
    fn(this, renderer: usize) -> (),
    compare_eax = false,
    regions = |set| set.add(rd(renderer, REN::renderState), RS::SIZE),
    rust = set_texture_wrapping);

port_thiscall!(set_texture_filtering_port, 0x6C1FA0, "NiDX8TexturePass::setTextureFiltering",
    fn(this, renderer: usize, mipmapped: u8, flag: u8) -> (),
    compare_eax = false,
    regions = |set| set.add(rd(renderer, REN::renderState), RS::SIZE),
    rust = set_texture_filtering);

pub static PORTS: &[&crate::shadow::PortState] = &[
    &set_blending_state_port::STATE,
    &apply_stage_blend_ops_port::STATE,
    &bind_stage_texture_port::STATE,
    &set_texture_pipeline_port::STATE,
    &reset_pass_flags_port::STATE,
    &configure_stage_from_texture_prop_port::STATE,
    &setup_projected_texture_matrix_port::STATE,
    &setup_projected_texture_matrix_neg_port::STATE,
    &tex_gen_world_parallel_port::STATE,
    &tex_gen_world_perspective_port::STATE,
    &tex_gen_sphere_map_port::STATE,
    &tex_gen_specular_cube_map_port::STATE,
    &tex_gen_diffuse_cube_map_port::STATE,
    &subpass_set_ni_object_field_48_port::STATE,
    &subpass_ctor_port::STATE,
    &subpass_dtor_port::STATE,
    &subpass_set_stage_at_index_port::STATE,
    &subpass_apply_port::STATE,
    &subpass_get_available_slot_counts_port::STATE,
    &subpass_append_stage_port::STATE,
    &subpass_append_stage_pair_port::STATE,
    &reset_stage_port::STATE,
    &set_texture_wrapping_port::STATE,
    &set_texture_filtering_port::STATE,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    // A device that records only `SetTexture` and `SetTextureStageState`.
    thread_local! {
        static CALLS: RefCell<Vec<[u32; 4]>> = const { RefCell::new(Vec::new()) };
    }

    unsafe extern "system" fn fake_set_texture(_device: usize, stage: u32, texture: usize) -> u32 {
        CALLS.with(|calls| calls.borrow_mut().push([0xF4, stage, texture as u32, 0]));
        0
    }

    unsafe extern "system" fn fake_set_texture_stage_state(_device: usize, stage: u32, state: u32, value: u32) -> u32 {
        CALLS.with(|calls| calls.borrow_mut().push([0xFC, stage, state, value]));
        0
    }

    fn take_calls() -> Vec<[u32; 4]> {
        CALLS.with(|calls| core::mem::take(&mut *calls.borrow_mut()))
    }

    /// A renderer with a render state, a pipeline and the device that records.
    struct World {
        vtable: Box<[usize; 64]>,
        device: Box<usize>,
        render_state: Vec<u8>,
        pipeline: Vec<u8>,
        renderer: Vec<u8>,
    }

    impl World {
        fn new() -> World {
            let mut vtable = Box::new([0usize; 64]);
            vtable[0xF4 / 4] = fake_set_texture as *const () as usize;
            vtable[0xFC / 4] = fake_set_texture_stage_state as *const () as usize;
            let device = Box::new(vtable.as_ptr() as usize);
            let mut world = World { vtable, device, render_state: vec![0; RS::SIZE], pipeline: vec![0; CTP::SIZE], renderer: vec![0; REN::SIZE] };
            unsafe {
                wr(world.rs(), RS::device, &*world.device as *const usize as usize);
                let (renderer, rs, pipeline) = (world.renderer(), world.rs(), world.pipeline());
                wr(renderer, REN::renderState, rs);
                wr(renderer, REN::configurableTexturePipeline, pipeline);
            }
            let _ = &world.vtable;
            take_calls();
            world
        }
        fn rs(&mut self) -> usize {
            self.render_state.as_mut_ptr() as usize
        }
        fn pipeline(&mut self) -> usize {
            self.pipeline.as_mut_ptr() as usize
        }
        fn renderer(&mut self) -> usize {
            self.renderer.as_mut_ptr() as usize
        }
    }

    fn stage_block() -> Vec<u8> {
        vec![0u8; ST::SIZE]
    }

    /// A pass with a pipeline. The camera matrix of the pipeline holds `camera`: 12 values that
    /// start at `worldRightX`.
    fn pass_with_camera(camera: [f32; 12]) -> (Vec<u8>, Vec<u8>) {
        let mut pass = vec![0u8; TP::SIZE];
        let mut pipeline = vec![0u8; CTP::SIZE];
        unsafe {
            for (index, value) in camera.into_iter().enumerate() {
                wr(pipeline.as_mut_ptr() as usize, CTP::worldRightX + index * 4, value);
            }
            wr(pass.as_mut_ptr() as usize, TP::configurableTexturePipeline, pipeline.as_mut_ptr() as usize);
        }
        (pass, pipeline)
    }

    const CAMERA: [f32; 12] = [2.0, 3.0, 5.0, 7.0, 11.0, 13.0, 17.0, 19.0, 23.0, 29.0, 31.0, 37.0];
    const MATRIX: [f32; 9] = [1.0, -2.0, 3.0, -4.0, 5.0, -6.0, 7.0, -8.0, 9.0];
    const TRANSLATION: [f32; 3] = [0.5, 0.25, -0.75];

    /// Element (row, column) of the texture matrix that the callbacks build: the sum over `j`
    /// of camera[j][row] * matrix[column][j]. All values are small integers. Thus the order of
    /// the operations does not change the result.
    fn rotation_element(row: usize, column: usize) -> f32 {
        (0..3).map(|j| CAMERA[j * 3 + row] * MATRIX[column * 3 + j]).sum()
    }

    unsafe fn check_rotation(stage: usize) {
        for row in 0..3 {
            for column in 0..3 {
                let got: f32 = rd(stage, ST::textureMatrix + row * 0x10 + column * 4);
                assert_eq!(got, rotation_element(row, column), "row {row} column {column}");
            }
        }
    }

    #[test]
    fn direction_callbacks_rotate_and_copy_the_translation() {
        let (mut pass, _pipeline) = pass_with_camera(CAMERA);
        let this = pass.as_mut_ptr() as usize;
        let translation = [0x1111_1111u32, 0x7F80_0001, 0x3333_3333];
        let cases: [(unsafe fn(usize, usize, usize, usize) -> u32, u32, u32); 3] =
            [(tex_gen_sphere_map, 0x30000, 2), (tex_gen_specular_cube_map, 0x30000, 3), (tex_gen_diffuse_cube_map, 0x10000, 3)];
        for (function, source, flags) in cases {
            let mut stage = stage_block();
            let s = stage.as_mut_ptr() as usize;
            unsafe {
                let eax = function(this, MATRIX.as_ptr() as usize, translation.as_ptr() as usize, s);
                assert_eq!(eax, 0x3333_3333);
                assert_eq!(rd::<u32>(s, ST::sourceTexcoordIndex), source);
                assert_eq!(rd::<u32>(s, ST::textureTransformFlags), flags);
                check_rotation(s);
                // The copies are integer copies. A signalling NaN does not change.
                assert_eq!(rd::<[u32; 3]>(s, ST::textureMatrix__41), translation);
                // The callbacks do not write the fourth column.
                assert_eq!(rd::<u32>(s, ST::textureMatrix__14), 0);
                assert_eq!(rd::<u32>(s, ST::textureMatrix__44), 0);
            }
        }
    }

    #[test]
    fn world_callbacks_add_the_camera_position() {
        let (mut pass, mut pipeline) = pass_with_camera(CAMERA);
        let this = pass.as_mut_ptr() as usize;
        let mut renderer = vec![0u8; REN::SIZE];
        unsafe {
            wr(this, TP::renderer, renderer.as_mut_ptr() as usize);
            wr(renderer.as_mut_ptr() as usize, REN::configurableTexturePipeline, pipeline.as_mut_ptr() as usize);
            wr::<u32>(pipeline.as_mut_ptr() as usize, CTP::field_310, 0x103);
        }
        let cases: [(unsafe fn(usize, usize, usize, usize) -> usize, u32); 2] = [(tex_gen_world_parallel, 2), (tex_gen_world_perspective, 0x103)];
        for (function, flags) in cases {
            let mut stage = stage_block();
            let s = stage.as_mut_ptr() as usize;
            unsafe {
                let eax = function(this, MATRIX.as_ptr() as usize, TRANSLATION.as_ptr() as usize, s);
                assert_eq!(eax, pipeline.as_ptr() as usize);
                assert_eq!(rd::<u32>(s, ST::sourceTexcoordIndex), 0x20000);
                assert_eq!(rd::<u32>(s, ST::textureTransformFlags), flags);
                check_rotation(s);
                for column in 0..3 {
                    let expected: f32 = (0..3).map(|j| MATRIX[column * 3 + j] * CAMERA[9 + j]).sum::<f32>() + TRANSLATION[column];
                    assert_eq!(rd::<f32>(s, ST::textureMatrix__41 + column * 4), expected, "column {column}");
                }
                assert_eq!(rd::<u32>(s, ST::textureMatrix__44), 0);
            }
        }
    }

    #[test]
    fn projected_matrix_takes_the_plane_into_camera_space() {
        let (mut pass, _pipeline) = pass_with_camera(CAMERA);
        let this = pass.as_mut_ptr() as usize;
        let mut effect = vec![0u8; 0x140];
        let e = effect.as_mut_ptr() as usize;
        let plane = [3.0f32, -5.0, 7.0, 4.0];
        unsafe {
            wr(e, EFFECT_PLANE, plane);
            wr::<u32>(this, TP::field_18A0, 2);
            wr::<u32>(this, TP::field_2464, 1);
            wr::<u32>(this, TP::field_6E0, 3);

            assert_eq!(setup_projected_texture_matrix(this, e), 1);
            let stage = this + TP::field_12C0 + 2 * ST::SIZE;
            for row in 0..3 {
                let expected: f32 = (0..3).map(|j| CAMERA[j * 3 + row] * plane[j]).sum();
                assert_eq!(rd::<f32>(stage, ST::textureMatrix + row * 0x10), expected, "row {row}");
            }
            let distance: f32 = (0..3).map(|j| plane[j] * CAMERA[9 + j]).sum();
            assert_eq!(rd::<f32>(stage, ST::textureMatrix__41), distance + 0.5 - 4.0);
            assert_eq!(rd::<usize>(stage, ST::texturePropMap), e);
            assert_eq!(rd::<u32>(this, TP::field_6E0), 4);

            assert_eq!(setup_projected_texture_matrix_neg(this, e), 1);
            let stage = this + TP::field_1E84 + ST::SIZE;
            for row in 0..3 {
                let expected: f32 = (0..3).map(|j| CAMERA[j * 3 + row] * -plane[j]).sum();
                assert_eq!(rd::<f32>(stage, ST::textureMatrix + row * 0x10), expected, "row {row}");
            }
            assert_eq!(rd::<f32>(stage, ST::textureMatrix__41), -distance + 0.5 + 4.0);
            assert_eq!(rd::<usize>(stage, ST::texturePropMap), e);
            assert_eq!(rd::<u32>(this, TP::field_6E0), 5);
        }
    }

    #[test]
    fn blend_ops_come_from_the_table_of_the_stage_position() {
        let (mut pass, mut pipeline) = pass_with_camera([0.0; 12]);
        let (this, p) = (pass.as_mut_ptr() as usize, pipeline.as_mut_ptr() as usize);
        let mut stage = stage_block();
        let s = stage.as_mut_ptr() as usize;
        let ops = |s: usize| unsafe { rd::<[u32; 6]>(s, ST::colorOp) };
        unsafe {
            // Slot 2 of the two tables.
            for index in 0..6u32 {
                wr(p, CTP::field_108 + 2 * 24 + index as usize * 4, 0x100 + index);
                wr(p, CTP::field_1F8 + 2 * 24 + index as usize * 4, 0x200 + index);
            }
            wr::<u32>(s, ST::field_94, 2);

            // The first stage with a texture gets the first table with no change.
            wr::<u8>(s, ST::field_90, 1);
            wr::<usize>(s, ST::ptrTexture, 0x1234);
            apply_stage_blend_ops(this, s, 0);
            assert_eq!(ops(s), [0x100, 0x101, 0x102, 0x103, 0x104, 0x105]);
            // With no texture, the two operations become SELECTARG2.
            wr::<usize>(s, ST::ptrTexture, 0);
            apply_stage_blend_ops(this, s, 0);
            assert_eq!(ops(s), [3, 0x101, 0x102, 3, 0x104, 0x105]);
            // With `field_90` clear, the operations become SELECTARG1 of the texture.
            wr::<u8>(s, ST::field_90, 0);
            apply_stage_blend_ops(this, s, 0);
            assert_eq!(ops(s), [2, 2, 0x102, 2, 2, 0x105]);
            // The flag selects the second table. A later stage also selects it.
            apply_stage_blend_ops(this, s, 1);
            assert_eq!(ops(s), [0x200, 0x201, 0x202, 0x203, 0x204, 0x205]);
            wr::<[u32; 6]>(s, ST::colorOp, [0; 6]);
            wr::<u32>(s, ST::stage, 1);
            apply_stage_blend_ops(this, s, 0);
            assert_eq!(ops(s), [0x200, 0x201, 0x202, 0x203, 0x204, 0x205]);
            // A null stage is not an error.
            apply_stage_blend_ops(this, 0, 0);
        }
    }

    #[test]
    fn reset_pass_flags_clears_the_counters() {
        let mut pass = vec![0xFFu8; TP::SIZE];
        let this = pass.as_mut_ptr() as usize;
        unsafe {
            reset_pass_flags(this);
            for field in [TP::field_CD8, TP::field_18A0, TP::field_2A48, TP::field_2464, TP::field_2BD0, TP::field_6E0] {
                assert_eq!(rd::<u32>(this, field), 0);
            }
            // Each of the three flags is 1 byte.
            for field in [TP::field_2B08, TP::field_2A4C_bumpEnvMapEnabled, TP::field_2BC8] {
                assert_eq!(rd::<u32>(this, field), 0xFFFF_FF00);
            }
        }
        assert_eq!(pass.iter().filter(|&&byte| byte == 0).count(), 6 * 4 + 3);
    }

    #[test]
    fn configure_refuses_before_it_reads_the_texture_manager() {
        let mut pass = vec![0u8; TP::SIZE];
        let this = pass.as_mut_ptr() as usize;
        let mut effect = vec![0u8; 0x140];
        let e = effect.as_mut_ptr() as usize;
        let mut stage = stage_block();
        let s = stage.as_mut_ptr() as usize;
        unsafe {
            // The effect is disabled, then enabled with no texture. Then the pass has no space.
            assert_eq!(configure_stage_from_texture_prop(this, e, s), 0);
            wr::<u8>(e, EFFECT_ENABLED, 1);
            assert_eq!(configure_stage_from_texture_prop(this, e, s), 0);
            wr::<usize>(e, EFFECT_TEXTURE, 0x1234);
            wr::<u32>(this, TP::field_6E0, 8);
            assert_eq!(configure_stage_from_texture_prop(this, e, s), 0);
        }
        assert!(stage.iter().all(|&byte| byte == 0));
    }

    #[test]
    fn field_48_assignment_counts_references() {
        let mut subpass = vec![0u8; SUB::SIZE];
        let this = subpass.as_mut_ptr() as usize;
        // Two objects. Each has a null vtable and a count. The test does not use the vtable,
        // because the counts stay above 0.
        let mut first = [0u32, 5];
        let mut second = [0u32, 1];
        let (a, b) = (first.as_mut_ptr() as usize, second.as_mut_ptr() as usize);
        unsafe {
            subpass_set_ni_object_field_48(this, a);
            assert_eq!(rd::<usize>(this, SUB::field_48), a);
            assert_eq!(first[1], 6);
            // The same object again. Nothing changes.
            subpass_set_ni_object_field_48(this, a);
            assert_eq!(first[1], 6);
            subpass_set_ni_object_field_48(this, b);
            assert_eq!(rd::<usize>(this, SUB::field_48), b);
            assert_eq!((first[1], second[1]), (5, 2));
            subpass_set_ni_object_field_48(this, 0);
            assert_eq!(rd::<usize>(this, SUB::field_48), 0);
            assert_eq!(second[1], 1);
        }
    }

    #[test]
    fn set_stage_at_index_grows_the_count() {
        let mut subpass = vec![0u8; SUB::SIZE];
        let this = subpass.as_mut_ptr() as usize;
        unsafe {
            wr::<u32>(this, SUB::field_C, u32::MAX);
            assert_eq!(subpass_set_stage_at_index(this, 0, 0x100), 1);
            assert_eq!(rd::<u32>(this, SUB::field_C), 1);
            assert_eq!(subpass_set_stage_at_index(this, 3, 0x300), 4);
            assert_eq!(rd::<u32>(this, SUB::field_C), 4);
            // The index is inside the count. The port returns the index and the count stays.
            assert_eq!(subpass_set_stage_at_index(this, 1, 0x200), 1);
            assert_eq!(rd::<u32>(this, SUB::field_C), 4);
            assert_eq!(rd::<[usize; 4]>(this, SUB::field_14), [0x100, 0x200, 0, 0x300]);
            // A count of 0. The game code does an unsigned comparison with count - 1. Thus
            // every index is "inside" and the count stays 0.
            wr::<u32>(this, SUB::field_C, 0);
            assert_eq!(subpass_set_stage_at_index(this, 5, 0x500), 5);
            assert_eq!(rd::<u32>(this, SUB::field_C), 0);
        }
    }

    #[test]
    fn available_slot_counts_subtract_what_is_used() {
        let mut subpass = vec![0u8; SUB::SIZE];
        let this = subpass.as_mut_ptr() as usize;
        let mut out = [0u32; 2];
        let (stages, textures) = (out.as_mut_ptr() as usize, out.as_mut_ptr() as usize + 4);
        unsafe {
            wr::<u32>(this, SUB::field_C, u32::MAX);
            wr::<u32>(this, SUB::field_10, 1);
            available_slot_counts(this, stages, textures, || 8, || 4);
            assert_eq!(out, [8, 3]);
            wr::<u32>(this, SUB::field_C, 3);
            available_slot_counts(this, stages, textures, || 8, || 4);
            assert_eq!(out, [5, 3]);
        }
    }

    #[test]
    fn subpass_apply_does_nothing_on_an_empty_subpass() {
        let mut subpass = vec![0u8; SUB::SIZE];
        let this = subpass.as_mut_ptr() as usize;
        unsafe {
            wr::<u32>(this, SUB::field_C, u32::MAX);
            assert_eq!(subpass_apply(this, 0, 0, 0, 0, 0), 0x4000_0000);
        }
    }

    #[test]
    fn append_stage_numbers_the_stages() {
        let mut subpass = vec![0u8; SUB::SIZE];
        let this = subpass.as_mut_ptr() as usize;
        let (mut first, mut second, mut third) = (stage_block(), stage_block(), stage_block());
        let (a, b, c) = (first.as_mut_ptr() as usize, second.as_mut_ptr() as usize, third.as_mut_ptr() as usize);
        unsafe {
            wr::<u32>(this, SUB::field_C, u32::MAX);
            assert_eq!(subpass_append_stage(this, 0), 0);
            wr::<usize>(a, ST::ptrTexture, 0x1000);
            wr::<u32>(a, ST::colorArg1, 77);
            assert_eq!(subpass_append_stage(this, a), 1);
            // The first stage keeps its arguments.
            assert_eq!((rd::<u32>(a, ST::stage), rd::<u32>(a, ST::colorArg1), rd::<u32>(a, ST::colorArg2)), (0, 77, 0));
            assert_eq!(rd::<[u32; 3]>(this, SUB::field_8), [1, 1, 1]);
            // A later stage blends with the stage before it. Argument 1 is the texture or the
            // diffuse colour. Argument 2 is the current colour.
            wr::<u32>(b, ST::colorArg1, 77);
            assert_eq!(subpass_append_stage(this, b), 1);
            assert_eq!((rd::<u32>(b, ST::stage), rd::<u32>(b, ST::colorArg1), rd::<u32>(b, ST::colorArg2)), (1, 0, 1));
            assert_eq!(rd::<[u32; 3]>(this, SUB::field_8), [2, 2, 1]);
            wr::<usize>(c, ST::ptrTexture, 0x3000);
            assert_eq!(subpass_append_stage(this, c), 1);
            assert_eq!((rd::<u32>(c, ST::stage), rd::<u32>(c, ST::colorArg1), rd::<u32>(c, ST::colorArg2)), (2, 2, 1));
            assert_eq!(rd::<[u32; 3]>(this, SUB::field_8), [3, 3, 2]);
            assert_eq!(rd::<[usize; 3]>(this, SUB::field_14), [a, b, c]);
            // A count that is more than the index stays.
            wr::<u32>(this, SUB::field_8, 0);
            assert_eq!(subpass_append_stage(this, b), 1);
            assert_eq!(rd::<[u32; 3]>(this, SUB::field_8), [1, 3, 2]);
        }
    }

    #[test]
    fn append_stage_pair_reports_which_stages_went_in() {
        let (mut first, mut second) = (stage_block(), stage_block());
        let (a, b) = (first.as_mut_ptr() as usize, second.as_mut_ptr() as usize);
        let fresh = || {
            let mut subpass = vec![0u8; SUB::SIZE];
            unsafe { wr::<u32>(subpass.as_mut_ptr() as usize, SUB::field_C, u32::MAX) };
            subpass
        };
        let state = |subpass: &Vec<u8>| unsafe { rd::<[u32; 3]>(subpass.as_ptr() as usize, SUB::field_8) };
        unsafe {
            wr::<usize>(a, ST::ptrTexture, 0x1000);
            wr::<usize>(b, ST::ptrTexture, 0x2000);

            // No space for a texture.
            let mut subpass = fresh();
            let this = subpass.as_mut_ptr() as usize;
            wr::<u32>(this, SUB::field_10, 2);
            assert_eq!(append_stage_pair(this, a, b, 0, 2), 0);
            assert_eq!(state(&subpass), [0, u32::MAX, 2]);

            // Nothing to append.
            let mut subpass = fresh();
            let this = subpass.as_mut_ptr() as usize;
            assert_eq!(append_stage_pair(this, 0, 0, 0, 2), 0);

            // The first stage alone.
            assert_eq!(append_stage_pair(this, a, 0, 0, 2), 1);
            assert_eq!(state(&subpass), [1, 1, 1]);
            assert_eq!(rd::<u32>(a, ST::stage), 0);

            // The second stage alone, when the flag is clear.
            assert_eq!(append_stage_pair(this, 0, b, 1, 2), 0);
            assert_eq!(state(&subpass), [1, 1, 1]);
            assert_eq!(append_stage_pair(this, 0, b, 0, 2), 2);
            assert_eq!(state(&subpass), [2, 2, 2]);
            assert_eq!(rd::<u32>(b, ST::stage), 1);
            assert_eq!(rd::<[usize; 2]>(this, SUB::field_14), [a, b]);

            // The two stages, when there is space for two textures.
            let mut subpass = fresh();
            let this = subpass.as_mut_ptr() as usize;
            assert_eq!(append_stage_pair(this, a, b, 0, 2), 3);
            assert_eq!(state(&subpass), [2, 2, 2]);
            assert_eq!((rd::<u32>(a, ST::stage), rd::<u32>(b, ST::stage)), (0, 1));
            assert_eq!(rd::<[usize; 2]>(this, SUB::field_14), [a, b]);

            // Only the first stage, when there is space for one texture.
            let mut subpass = fresh();
            let this = subpass.as_mut_ptr() as usize;
            wr::<u32>(this, SUB::field_10, 1);
            wr::<u32>(b, ST::stage, 99);
            assert_eq!(append_stage_pair(this, a, b, 0, 2), 1);
            assert_eq!(state(&subpass), [1, 1, 2]);
            assert_eq!(rd::<u32>(b, ST::stage), 99);
        }
    }

    #[test]
    fn reset_stage_closes_the_stage_once() {
        let mut world = World::new();
        let (renderer, rs) = (world.renderer(), world.rs());
        unsafe {
            wr::<usize>(rs, RS::texturePerStage + 2 * 4, 0xABCD);
            reset_stage(2, renderer);
            assert_eq!(take_calls(), vec![[0xF4, 2, 0, 0], [0xFC, 2, D3DTSS_COLOROP, 1], [0xFC, 2, D3DTSS_ALPHAOP, 1]]);
            assert_eq!(rd::<usize>(rs, RS::texturePerStage + 2 * 4), 0);
            assert_eq!(rd::<u32>(stage_setting(rs, 2, D3DTSS_COLOROP), 0), 1);
            assert_eq!(rd::<u32>(stage_setting(rs, 2, D3DTSS_ALPHAOP), 0), 1);
            // The port does not change the saved values.
            assert_eq!(rd::<u32>(stage_setting(rs, 2, D3DTSS_COLOROP), 4), 0);
            reset_stage(2, renderer);
            assert!(take_calls().is_empty());
        }
    }

    #[test]
    fn wrapping_uses_one_mode_for_both_axes_without_the_capability() {
        let mut world = World::new();
        let (renderer, pipeline) = (world.renderer(), world.pipeline());
        let mut stage = stage_block();
        let s = stage.as_mut_ptr() as usize;
        unsafe {
            wr::<u32>(s, ST::stage, 1);
            wr::<u32>(s, ST::addressModeU, 3);
            wr::<u32>(s, ST::addressModeV, 1);
            set_texture_wrapping(s, renderer);
            assert_eq!(take_calls(), vec![[0xFC, 1, D3DTSS_ADDRESSU, 3], [0xFC, 1, D3DTSS_ADDRESSV, 3]]);
            wr::<u8>(pipeline, CTP::field_FC, 1);
            set_texture_wrapping(s, renderer);
            assert_eq!(take_calls(), vec![[0xFC, 1, D3DTSS_ADDRESSV, 1]]);
            set_texture_wrapping(s, renderer);
            assert!(take_calls().is_empty());
        }
    }

    #[test]
    fn filtering_falls_back_without_mipmaps() {
        let mut world = World::new();
        let renderer = world.renderer();
        let mut stage = stage_block();
        let s = stage.as_mut_ptr() as usize;
        // Six filter modes of { minify, magnify, mip }.
        let mut table = [0u32; 18];
        for mode in 0..6 {
            table[mode * 3] = 0x10 + mode as u32;
            table[mode * 3 + 1] = 0x20 + mode as u32;
            table[mode * 3 + 2] = 0x30 + mode as u32;
        }
        let t = table.as_ptr() as usize;
        let filtered = |mode: u32, mipmapped: u8, flag: u8| unsafe {
            wr::<u32>(s, ST::textureStageIndex, mode);
            wr::<u32>(s, ST::stage, 3);
            // A cache that holds no value of the table.
            for state in [D3DTSS_MAGFILTER, D3DTSS_MINFILTER, D3DTSS_MIPFILTER] {
                wr::<u32>(stage_setting(rd(renderer, REN::renderState), 3, state), 0, 0);
            }
            texture_filtering(s, renderer, mipmapped, flag, t);
            let calls = take_calls();
            assert_eq!(calls.len(), 3);
            assert_eq!([calls[0][2], calls[1][2], calls[2][2]], [D3DTSS_MAGFILTER, D3DTSS_MINFILTER, D3DTSS_MIPFILTER]);
            assert!(calls.iter().all(|call| call[0] == 0xFC && call[1] == 3));
            // The mode that the three values came from.
            assert_eq!(calls[0][3] - 0x20, calls[1][3] - 0x10);
            assert_eq!(calls[0][3] - 0x20, calls[2][3] - 0x30);
            calls[0][3] - 0x20
        };
        // Mipmaps and the flag set: the mode does not change.
        assert_eq!([0, 1, 2, 3, 4, 5].map(|mode| filtered(mode, 1, 1)), [0, 1, 2, 3, 4, 5]);
        // Mipmaps and the flag clear: modes 2 and 4 change.
        assert_eq!([0, 1, 2, 3, 4, 5].map(|mode| filtered(mode, 1, 0)), [0, 1, 5, 3, 3, 5]);
        // No mipmaps: the modes that need none. The flag has no effect.
        assert_eq!([0, 1, 2, 3, 4, 5].map(|mode| filtered(mode, 0, 0)), [0, 1, 1, 0, 0, 1]);
        assert_eq!([0, 1, 2, 3, 4, 5].map(|mode| filtered(mode, 0, 1)), [0, 1, 1, 0, 0, 1]);
        unsafe {
            // The same values again: no device call.
            texture_filtering(s, renderer, 0, 1, t);
            assert!(take_calls().is_empty());
        }
    }

    #[test]
    fn texture_pipeline_returns_early_on_the_same_effect_state() {
        let (mut pass, mut pipeline) = pass_with_camera([0.0; 12]);
        let (this, p) = (pass.as_mut_ptr() as usize, pipeline.as_mut_ptr() as usize);
        unsafe {
            // The pass has no vtable. A call that continues after the comparison causes a fault.
            wr::<usize>(p, CTP::field_B04, 0x5555);
            set_texture_pipeline(this, 0x5555);
            assert_eq!(rd::<usize>(this, TP::field_2BD0), 0);
        }
    }
}
