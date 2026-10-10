//! `NiDX8TexturePass::setCTPipelineState` (0x6B41B0): builds the multitexture pass of one draw
//! from the texturing property and the dynamic effect state. It decides which texture stages
//! the draw has, which sub-pass (device pass) each one goes to and which blend operations
//! each one gets.
//!
//! Other mods change the targets of calls in this function, so every direct call of the game
//! code is made through `site::call_target` with the address of its `call` instruction.
//!
//! The body is written once, over the trait `Env`. `Live` is the game: plain writes and the
//! calls in memory. `Model` is a dry run for the shadow comparison: it keeps a journal of
//! every write, runs its own copy of the callees, and is rolled back; the journal gives the
//! memory blocks the call will write.

use core::mem::{size_of, transmute};

use crate::gen::layouts::{
    NiAlphaProperty as AP, NiDX8ConfigurableTexturePipeline as CTP, NiDX8RenderState as RS, NiDX8TexturePass as TP,
    NiDX8TexturePass__1 as SUB, NiDX8TextureStage as ST, NiDynamicEffectState as DES, NiPropertyState as PS, NiTexture,
    NiTexturingProperty as NTP, NiTexturingProperty_BumpMap as BUMP, NiTexturingProperty_Map as MAP,
};
use crate::mem::{rd, x87_pass};
use crate::shadow::Regions;

// ---------------------------------------------------------------------------------------
// Addresses and offsets
// ---------------------------------------------------------------------------------------

/// `dword_7C3834`: the texture stages of one device pass.
const MAX_STAGES: usize = 0x7C3834;
/// `dword_7C3838`: the textures of one device pass.
const MAX_TEXTURES: usize = 0x7C3838;
/// `NiDX8TexturePass::resetPassFlags`, slot 0x18 of the pass.
#[cfg(not(test))]
const ADDR_RESET_PASS_FLAGS: usize = 0x6B59E0;

/// The two stores of the base stage that MWSE replaces with a call into MWSE.dll.
const BASE_TEXCOORD_PATCH: usize = 0x6B42AC;

/// `NiRefObject::refCount`
const REF_COUNT: usize = 0x4;
/// The size of a texture stage as the game code multiplies it out.
const STAGE: usize = ST::SIZE;

// `NiTextureEffect` (no layout in the database export); the model of `setTexturePipeline`
// reads these.
const EFFECT_ENABLED: usize = 0x90;
const EFFECT_TEXTURE: usize = 0x108;
const EFFECT_FILTER_MODE: usize = 0x10C;
const EFFECT_CLAMP_MODE: usize = 0x110;

/// The address of every `call rel32` of the function, in the order of the code. The port
/// calls the target that is in memory at each of them.
#[rustfmt::skip]
pub static SITES: &[usize] = &[
    0x6B41F4, // NiDX8TexturePass::setTexturePipeline
    0x6B427B, // NiPointer::set
    0x6B428C, // NiDX8ConfigurableTexturePipeline::setStageAddressingMode
    0x6B429D, // NiDX8ConfigurableTexturePipeline::setStageFilterMode
    0x6B42DB, // NiTexturingProperty::IsMultiTexture
    0x6B42EB, // NiDX8TexturePass::bindStageTexture
    0x6B4301, // NiDX8TexturePass::bindStageTexture
    0x6B43E4, // NiPointer::set
    0x6B43F5, // setStageAddressingMode
    0x6B4406, // setStageFilterMode
    0x6B448E, // NiPointer::set
    0x6B449F, // setStageAddressingMode
    0x6B44AC, // setStageFilterMode
    0x6B4537, // NiObject::release
    0x6B4552, // setStageAddressingMode
    0x6B455F, // setStageFilterMode
    0x6B45BA, // NiDX8TexturePass::acquireSubpassWithCapacity
    0x6B45D7, // NiDX8TexturePass::$1::appendStagePair
    0x6B4610, // NiPointer::set
    0x6B4618, // NiPointer::set
    0x6B4634, // NiDX8TexturePass::applyStageBlendOps
    0x6B4642, // applyStageBlendOps
    0x6B4659, // acquireSubpassWithCapacity
    0x6B466B, // NiDX8TexturePass::$1::appendStage
    0x6B46C4, // acquireSubpassWithCapacity
    0x6B46D2, // appendStage
    0x6B4707, // NiPointer::set
    0x6B470F, // NiPointer::set
    0x6B4719, // applyStageBlendOps
    0x6B477D, // NiDX8TexturePass::acquireSubpassForPixelShader
    0x6B4788, // acquireSubpassWithCapacity
    0x6B47A3, // appendStagePair
    0x6B47D8, // NiPointer::set
    0x6B47E0, // NiPointer::set
    0x6B47F8, // applyStageBlendOps
    0x6B4802, // applyStageBlendOps
    0x6B482A, // acquireSubpassWithCapacity
    0x6B4838, // appendStage
    0x6B486D, // NiPointer::set
    0x6B4875, // NiPointer::set
    0x6B487F, // applyStageBlendOps
    0x6B48B4, // acquireSubpassWithCapacity
    0x6B48C2, // appendStage
    0x6B48F7, // NiPointer::set
    0x6B48FF, // NiPointer::set
    0x6B4909, // applyStageBlendOps
    0x6B4954, // acquireSubpassWithCapacity
    0x6B4962, // appendStage
    0x6B4997, // NiPointer::set
    0x6B499F, // NiPointer::set
    0x6B49A9, // applyStageBlendOps
    0x6B49DB, // acquireSubpassWithCapacity
    0x6B49E9, // appendStage
    0x6B4A1E, // NiPointer::set
    0x6B4A26, // NiPointer::set
    0x6B4A30, // applyStageBlendOps
    0x6B4A49, // acquireSubpassForPixelShader
    0x6B4A54, // acquireSubpassWithCapacity
    0x6B4A6D, // appendStagePair
    0x6B4AA6, // NiPointer::set
    0x6B4AAE, // NiPointer::set
    0x6B4ACA, // applyStageBlendOps
    0x6B4AE8, // acquireSubpassWithCapacity
    0x6B4AF6, // appendStage
    0x6B4B2C, // NiPointer::set
    0x6B4B34, // NiPointer::set
    0x6B4B3E, // applyStageBlendOps
    0x6B4BBC, // NiPointer::set
    0x6B4BCD, // setStageAddressingMode
    0x6B4BDA, // setStageFilterMode
    0x6B4C17, // acquireSubpassForPixelShader
    0x6B4C22, // acquireSubpassWithCapacity
    0x6B4C30, // appendStage
    0x6B4C66, // NiPointer::set
    0x6B4C6E, // NiPointer::set
    0x6B4C78, // applyStageBlendOps
    0x6B4D76, // acquireSubpassForPixelShader
    0x6B4D99, // acquireSubpassWithCapacity
    0x6B4DA7, // appendStage
    0x6B4DE0, // NiPointer::set
    0x6B4DE8, // NiPointer::set
    0x6B4E1C, // applyStageBlendOps
    0x6B4E2F, // acquireSubpassWithCapacity
    0x6B4E46, // appendStagePair
    0x6B4E7F, // NiPointer::set
    0x6B4E87, // NiPointer::set
    0x6B4E91, // applyStageBlendOps
    0x6B4EA8, // applyStageBlendOps
    0x6B4EF8, // acquireSubpassWithCapacity
    0x6B4F06, // appendStage
    0x6B4F3B, // NiPointer::set
    0x6B4F43, // NiPointer::set
    0x6B4F4D, // applyStageBlendOps
    0x6B4F94, // NiDX8TexturePass::initOrCheckSubpass
    0x6B4F9F, // acquireSubpassForPixelShader
    0x6B4FC8, // acquireSubpassForPixelShader
    0x6B4FDF, // acquireSubpassWithCapacity
    0x6B4FF1, // appendStage
    0x6B502A, // NiPointer::set
    0x6B5032, // NiPointer::set
    0x6B506F, // applyStageBlendOps
    0x6B50D9, // acquireSubpassWithCapacity
    0x6B50E7, // appendStage
    0x6B511C, // NiPointer::set
    0x6B5124, // NiPointer::set
    0x6B512E, // applyStageBlendOps
    0x6B51D2, // NiPointer::set
    0x6B51E3, // setStageAddressingMode
    0x6B51F4, // setStageFilterMode
    0x6B525C, // initOrCheckSubpass
    0x6B526B, // acquireSubpassForPixelShader
    0x6B52DB, // acquireSubpassWithCapacity
    0x6B52F2, // appendStagePair
    0x6B532C, // NiPointer::set
    0x6B5338, // NiPointer::set
    0x6B5342, // applyStageBlendOps
    0x6B5352, // applyStageBlendOps
    0x6B53C0, // NiPointer::set
    0x6B53D1, // setStageAddressingMode
    0x6B53DE, // setStageFilterMode
    0x6B542F, // acquireSubpassWithCapacity
    0x6B5441, // appendStage
    0x6B547F, // NiPointer::set
    0x6B5487, // NiPointer::set
    0x6B5494, // acquireSubpassWithCapacity
    0x6B54AB, // appendStagePair
    0x6B54D6, // NiDX8TexturePass::$1::setNiObjectField48
    0x6B54E6, // applyStageBlendOps
    0x6B54F5, // applyStageBlendOps
    0x6B5512, // acquireSubpassWithCapacity
    0x6B5520, // appendStage
    0x6B554B, // setNiObjectField48
    0x6B5555, // applyStageBlendOps
    0x6B556F, // initOrCheckSubpass
    0x6B557A, // acquireSubpassForPixelShader
    0x6B5585, // acquireSubpassWithCapacity
    0x6B5593, // appendStage
    0x6B55BE, // setNiObjectField48
    0x6B55C8, // applyStageBlendOps
    0x6B55E0, // acquireSubpassForPixelShader
    0x6B55F2, // bindStageTexture
    0x6B5603, // applyStageBlendOps
];

// ---------------------------------------------------------------------------------------
// The environment of the body
// ---------------------------------------------------------------------------------------

/// What the body does to the world: its writes, the two limits of the device, and the
/// fourteen functions the game code calls. `site` is the address of the `call` instruction.
trait Env {
    unsafe fn wr<T: Copy>(&mut self, base: usize, off: usize, value: T);
    unsafe fn max_stages(&mut self) -> u32;
    unsafe fn max_textures(&mut self) -> u32;
    /// True when the instruction at 0x6B42AC is a call: MWSE put its fix of the texture
    /// coordinate set of the base map there.
    unsafe fn base_texcoord_patch(&mut self) -> bool;

    /// `NiDX8TexturePass::setTexturePipeline(effectState)`
    unsafe fn set_texture_pipeline(&mut self, site: usize, pass: usize, effect_state: usize);
    /// `NiPointer::set(object)`
    unsafe fn pointer_set(&mut self, site: usize, pointer: usize, object: usize);
    /// `NiObject::release()`
    unsafe fn release(&mut self, site: usize, object: usize);
    /// `NiDX8ConfigurableTexturePipeline::setStageAddressingMode(clampMode, stage)`
    unsafe fn set_addressing(&mut self, site: usize, pipeline: usize, clamp_mode: u32, stage: usize);
    /// `NiDX8ConfigurableTexturePipeline::setStageFilterMode(filterMode, stage)`
    unsafe fn set_filter(&mut self, site: usize, pipeline: usize, filter_mode: u32, stage: usize);
    /// `NiTexturingProperty::IsMultiTexture()`; the result is AL.
    unsafe fn is_multi_texture(&mut self, site: usize, property: usize) -> u8;
    /// `NiDX8TexturePass::bindStageTexture(stage, textures, stages)`; the result is AL.
    unsafe fn bind_stage_texture(&mut self, site: usize, pass: usize, stage: usize, textures: u32, stages: u32) -> u8;
    /// `NiDX8TexturePass::applyStageBlendOps(stage, flag)`
    unsafe fn apply_blend_ops(&mut self, site: usize, pass: usize, stage: usize, flag: u32);
    /// `NiDX8TexturePass::acquireSubpassWithCapacity(a2, a3)`; the result is the sub-pass.
    unsafe fn acquire(&mut self, site: usize, pass: usize, a2: u32, a3: u32) -> usize;
    /// `NiDX8TexturePass::acquireSubpassForPixelShader()`
    unsafe fn acquire_for_ps(&mut self, site: usize, pass: usize);
    /// `NiDX8TexturePass::initOrCheckSubpass(a2, a3, a4)`; the result is AL.
    unsafe fn init_or_check(&mut self, site: usize, pass: usize, a2: u32, a3: u32, a4: u32) -> u8;
    /// `NiDX8TexturePass::$1::appendStage(stage)`; the result is AL.
    unsafe fn append_stage(&mut self, site: usize, subpass: usize, stage: usize) -> u8;
    /// `NiDX8TexturePass::$1::appendStagePair(stage, otherStage, flag)`; the result is EAX.
    unsafe fn append_pair(&mut self, site: usize, subpass: usize, stage: usize, other_stage: usize, flag: u32) -> u32;
    /// `NiDX8TexturePass::$1::setNiObjectField48(object)`
    unsafe fn set_field_48(&mut self, site: usize, subpass: usize, object: usize);
}

/// The game: every call goes to the target that is in memory at its site, with the
/// convention of the game code (`__thiscall`, the callee removes the arguments).
#[cfg_attr(test, allow(dead_code))]
struct Live;

#[cfg_attr(test, allow(dead_code))]
#[inline(always)]
unsafe fn target(site: usize) -> usize {
    crate::site::call_target(site)
}

type This0 = unsafe extern "thiscall" fn(usize) -> u32;
type This1 = unsafe extern "thiscall" fn(usize, usize) -> u32;
type This2 = unsafe extern "thiscall" fn(usize, usize, usize) -> u32;
type This3 = unsafe extern "thiscall" fn(usize, usize, usize, usize) -> u32;

impl Env for Live {
    #[inline(always)]
    unsafe fn wr<T: Copy>(&mut self, base: usize, off: usize, value: T) {
        crate::mem::wr(base, off, value)
    }
    #[inline(always)]
    unsafe fn max_stages(&mut self) -> u32 {
        rd(MAX_STAGES, 0)
    }
    #[inline(always)]
    unsafe fn max_textures(&mut self) -> u32 {
        rd(MAX_TEXTURES, 0)
    }
    #[inline(always)]
    unsafe fn base_texcoord_patch(&mut self) -> bool {
        rd::<u8>(BASE_TEXCOORD_PATCH, 0) == 0xE8
    }
    #[inline(always)]
    unsafe fn set_texture_pipeline(&mut self, site: usize, pass: usize, effect_state: usize) {
        transmute::<usize, This1>(target(site))(pass, effect_state);
    }
    #[inline(always)]
    unsafe fn pointer_set(&mut self, site: usize, pointer: usize, object: usize) {
        transmute::<usize, This1>(target(site))(pointer, object);
    }
    #[inline(always)]
    unsafe fn release(&mut self, site: usize, object: usize) {
        transmute::<usize, This0>(target(site))(object);
    }
    #[inline(always)]
    unsafe fn set_addressing(&mut self, site: usize, pipeline: usize, clamp_mode: u32, stage: usize) {
        transmute::<usize, This2>(target(site))(pipeline, clamp_mode as usize, stage);
    }
    #[inline(always)]
    unsafe fn set_filter(&mut self, site: usize, pipeline: usize, filter_mode: u32, stage: usize) {
        transmute::<usize, This2>(target(site))(pipeline, filter_mode as usize, stage);
    }
    #[inline(always)]
    unsafe fn is_multi_texture(&mut self, site: usize, property: usize) -> u8 {
        transmute::<usize, This0>(target(site))(property) as u8
    }
    #[inline(always)]
    unsafe fn bind_stage_texture(&mut self, site: usize, pass: usize, stage: usize, textures: u32, stages: u32) -> u8 {
        transmute::<usize, This3>(target(site))(pass, stage, textures as usize, stages as usize) as u8
    }
    #[inline(always)]
    unsafe fn apply_blend_ops(&mut self, site: usize, pass: usize, stage: usize, flag: u32) {
        transmute::<usize, This2>(target(site))(pass, stage, flag as usize);
    }
    #[inline(always)]
    unsafe fn acquire(&mut self, site: usize, pass: usize, a2: u32, a3: u32) -> usize {
        transmute::<usize, This2>(target(site))(pass, a2 as usize, a3 as usize) as usize
    }
    #[inline(always)]
    unsafe fn acquire_for_ps(&mut self, site: usize, pass: usize) {
        transmute::<usize, This0>(target(site))(pass);
    }
    #[inline(always)]
    unsafe fn init_or_check(&mut self, site: usize, pass: usize, a2: u32, a3: u32, a4: u32) -> u8 {
        transmute::<usize, This3>(target(site))(pass, a2 as usize, a3 as usize, a4 as usize) as u8
    }
    #[inline(always)]
    unsafe fn append_stage(&mut self, site: usize, subpass: usize, stage: usize) -> u8 {
        transmute::<usize, This1>(target(site))(subpass, stage) as u8
    }
    #[inline(always)]
    unsafe fn append_pair(&mut self, site: usize, subpass: usize, stage: usize, other_stage: usize, flag: u32) -> u32 {
        transmute::<usize, This3>(target(site))(subpass, stage, other_stage, flag as usize)
    }
    #[inline(always)]
    unsafe fn set_field_48(&mut self, site: usize, subpass: usize, object: usize) {
        transmute::<usize, This1>(target(site))(subpass, object);
    }
}

// ---------------------------------------------------------------------------------------
// Pieces the compiler put inline many times
// ---------------------------------------------------------------------------------------

#[inline(always)]
unsafe fn pipeline_of(this: usize) -> usize {
    rd(this, TP::configurableTexturePipeline)
}

/// The texture coordinate set of a map: its own when the geometry has it, else the last one.
#[inline(always)]
fn coordinate_set(set: u32, count: u32, last: u32) -> u32 {
    if set < count {
        set
    } else {
        last
    }
}

/// The texture coordinate set MWSE's patch at 0x6B42AC gives the base stage: the set of the
/// base map, or the last set of the geometry when the map asks for one it does not have.
unsafe fn base_map_coordinate_set(property: usize, texture_sets: u32) -> u32 {
    let base_map: usize = rd(rd::<usize>(property, NTP::maps_storage), 0);
    if base_map == 0 {
        return 0;
    }
    let set: u32 = rd(base_map, MAP::texCoordSet);
    if texture_sets == 0 {
        0
    } else if set >= texture_sets {
        texture_sets - 1
    } else {
        set
    }
}

/// The next free stage of the first bank (`stage`, 0x40) and the count moved on.
#[inline(always)]
unsafe fn next_stage<E: Env>(e: &mut E, this: usize) -> usize {
    let count: u32 = rd(this, TP::activeStageCount);
    e.wr(this, TP::activeStageCount, count.wrapping_add(1));
    this.wrapping_add((count as usize).wrapping_mul(STAGE)).wrapping_add(TP::stage)
}

/// A new stage for a map: the texture, the addressing and the filtering. `sites` are the
/// calls of `NiPointer::set`, `setStageAddressingMode` and `setStageFilterMode`.
#[inline(always)]
unsafe fn map_stage<E: Env>(e: &mut E, this: usize, texture: usize, clamp_mode: u32, filter_mode: u32, sites: [usize; 3]) -> usize {
    let stage = next_stage(e, this);
    e.pointer_set(sites[0], stage + ST::ptrTexture, texture);
    e.set_addressing(sites[1], pipeline_of(this), clamp_mode, stage);
    e.set_filter(sites[2], pipeline_of(this), filter_mode, stage);
    stage
}

/// The six members every map stage gets after the calls.
#[inline(always)]
unsafe fn fill_stage<E: Env>(e: &mut E, stage: usize, blend_slot: u32, coordinate_set: u32, map: usize) {
    e.wr::<u32>(stage, ST::field_94, blend_slot);
    e.wr::<u32>(stage, ST::sourceTexcoordIndex, coordinate_set);
    e.wr::<u32>(stage, ST::textureTransformFlags, 0);
    e.wr::<u8>(stage, ST::field_90, 0);
    e.wr::<u8>(stage, ST::bumpEnvMapEnabled, 0);
    e.wr::<usize>(stage, ST::texturePropMap, map);
}

/// The alpha template of the blend slot of the first stage of a sub-pass, when the sub-pass
/// is of a later pass and has stages.
#[inline(always)]
unsafe fn template_for(this: usize, subpass: usize) -> Option<usize> {
    if rd::<u32>(this, TP::field_C) == 0 {
        return None;
    }
    if rd::<u32>(subpass, SUB::field_C) == u32::MAX {
        return None;
    }
    let first_stage: usize = rd(subpass, SUB::field_14);
    let slot: u32 = rd(first_stage, ST::field_94);
    Some(rd(pipeline_of(this).wrapping_add((slot as usize).wrapping_mul(4)), CTP::field_2E8))
}

/// Gives the sub-pass its alpha template with the inline copy of `setNiObjectField48`: two
/// calls of `NiPointer::set`, with null and with the template.
#[inline(always)]
unsafe fn link_template<E: Env>(e: &mut E, this: usize, subpass: usize, sites: [usize; 2]) {
    if let Some(template) = template_for(this, subpass) {
        let member = subpass + SUB::field_48;
        if rd::<usize>(member, 0) != template {
            e.pointer_set(sites[0], member, 0);
            e.pointer_set(sites[1], member, template);
        }
    }
}

/// The same with a call of `setNiObjectField48`.
#[inline(always)]
unsafe fn link_template_call<E: Env>(e: &mut E, this: usize, subpass: usize, site: usize) {
    if let Some(template) = template_for(this, subpass) {
        e.set_field_48(site, subpass, template);
    }
}

/// One stage into the sub-pass that has room for it. `sites` are the calls of
/// `acquireSubpassWithCapacity`, `appendStage` and the two of `NiPointer::set`. Returns what
/// `appendStage` returned, or `None` when there was no sub-pass.
#[inline(always)]
unsafe fn append_single<E: Env>(e: &mut E, this: usize, stage: usize, sites: [usize; 4]) -> Option<u8> {
    let subpass = e.acquire(sites[0], this, 1, 1);
    if subpass == 0 {
        return None;
    }
    let appended = e.append_stage(sites[1], subpass, stage);
    link_template(e, this, subpass, [sites[2], sites[3]]);
    Some(appended)
}

/// 6B5608-6B5637: the pass count, and the result.
#[inline(always)]
unsafe fn finish<E: Env>(e: &mut E, this: usize, one_draw: u8) -> u32 {
    let passes: u32 = rd(this, TP::field_C);
    e.wr(this, TP::field_34, passes);
    e.wr::<u32>(this, TP::field_3C, 0);
    e.wr::<u32>(this, TP::field_8, 0);
    if passes > 1 && one_draw != 0 {
        0x8000_0000
    } else {
        0
    }
}

// ---------------------------------------------------------------------------------------
// The function
// ---------------------------------------------------------------------------------------

/// The body of 0x6B41B0. The names of the stack slots of the game code are in the comments
/// where one slot holds several things.
unsafe fn build<E: Env>(e: &mut E, this: usize, property_state: usize, effect_state: usize, texture_sets: u32) -> u32 {
    // 6B41B0-6B41E0: every sub-pass of the array is emptied.
    if rd::<u32>(this, TP::count_10) > 0 {
        let mut index = 0u32;
        loop {
            let storage: usize = rd(this, TP::TArray_18_storage);
            let subpass: usize = rd(storage, index as usize * 4);
            if subpass != 0 {
                e.wr::<u32>(subpass, SUB::field_8, 0);
                e.wr::<u32>(subpass, SUB::field_C, u32::MAX);
                e.wr::<u32>(subpass, SUB::field_10, 0);
            }
            index = index.wrapping_add(1);
            if index >= rd::<u32>(this, TP::count_10) {
                break;
            }
        }
    }

    // 6B41E2-6B41F4: the stages of the texture effects. `var_35` is 1 until a projected
    // light or shadow with a clip plane is built.
    e.wr::<usize>(this, TP::substruct_14, 0);
    e.wr::<u32>(this, TP::field_C, 0);
    let mut one_draw = 1u8;
    e.set_texture_pipeline(0x6B41F4, this, effect_state);

    // 6B41F9-6B4262: the base map of the texturing property.
    e.wr::<u32>(this, TP::activeStageCount, 0);
    e.wr::<u32>(this, TP::field_3C, 0);
    e.wr::<u32>(this, TP::field_34, 0);
    let property: usize = rd(property_state, PS::propTexturing);
    let base_map: usize = rd(rd::<usize>(property, NTP::maps_storage), 0);
    let base_texture: usize = if base_map != 0 { rd(base_map, MAP::texture) } else { 0 };
    e.wr::<u32>(this, TP::currentTextureApplyMode, rd(property, NTP::applyMode));
    // The byte in `arg_4`: the apply mode is not "replace".
    let apply: u8 = (rd::<u32>(property, NTP::applyMode) != 0) as u8;
    let base_map: usize = rd(rd::<usize>(property, NTP::maps_storage), 0);
    let base_filter: u32 = if base_map != 0 { rd(base_map, MAP::filterMode) } else { 1 };
    let base_clamp: u32 = if base_map != 0 { rd(base_map, MAP::clampMode) } else { 3 };

    // 6B4262-6B42C0: the stage of the base map. `texturePropMap` gets the texture here, not
    // the map.
    let mut base_stage = 0usize;
    if base_texture != 0 {
        let stage = this + TP::stage;
        e.wr::<u32>(this, TP::activeStageCount, 1);
        e.pointer_set(0x6B427B, stage + ST::ptrTexture, base_texture);
        e.set_addressing(0x6B428C, pipeline_of(this), base_clamp, stage);
        e.set_filter(0x6B429D, pipeline_of(this), base_filter, stage);
        e.wr::<u32>(stage, ST::field_94, 2);
        // 6B42AC: `mov [edi+28h], ebp; mov [edi+48h], ebp` in the database. MWSE replaces
        // the two instructions with a call (PatchNiDX8TexturePass_BaseMapTexcoord in MWSE's
        // PatchUtil.cpp) that gives the base stage the texture coordinate set of the base
        // map. The hook reads the stack frame of the game code, so it is not called: when
        // the call is there, this is what it does.
        let base_set = if e.base_texcoord_patch() { base_map_coordinate_set(property, texture_sets) } else { 0 };
        e.wr::<u32>(stage, ST::sourceTexcoordIndex, base_set);
        e.wr::<u32>(stage, ST::textureTransformFlags, 0);
        e.wr::<u8>(stage, ST::field_90, 0);
        e.wr::<u8>(stage, ST::bumpEnvMapEnabled, 0);
        e.wr::<usize>(stage, ST::texturePropMap, base_texture);
        base_stage = stage;
    }

    // 6B42C6-6B431B: no texture effect and one map only: the short way out.
    let effect_stages: u32 = rd(this, TP::field_6E0);
    e.wr::<u32>(this, TP::field_38, 0);
    if effect_stages == 0 && e.is_multi_texture(0x6B42DB, property) == 0 {
        let last: usize;
        if e.bind_stage_texture(0x6B42EB, this, base_stage, 1, 1) == 0 {
            last = this + TP::field_624;
            e.bind_stage_texture(0x6B4301, this, last, 1, 1);
        } else {
            if base_stage != 0 {
                e.wr::<u8>(base_stage, ST::field_90, apply);
            }
            last = base_stage;
        }
        e.apply_blend_ops(0x6B5603, this, last, 0);
        return finish(e, this, one_draw);
    }

    // 6B4320-6B434B
    let last_set = if texture_sets != 0 { texture_sets.wrapping_sub(1) } else { 0 };
    let mut gloss_map = 0usize;
    let mut decal_count = 0u32;
    let mut dark_stage = 0usize; // var_34
    let mut detail_stage = 0usize; // otherStage
    let mut glow_stage = 0usize; // baseMapClampMode
    if texture_sets != 0 {
        // 6B4351-6B438F: the decal count, and the dark, detail, gloss and glow maps.
        decal_count = rd::<u32>(property, NTP::maps_endIndex).wrapping_sub(6);
        if decal_count == 1 && rd::<usize>(rd::<usize>(property, NTP::maps_storage), 0x18) == 0 {
            decal_count = 0;
        }
        let maps: usize = rd(property, NTP::maps_storage);
        let detail_map: usize = rd(maps, 0x8);
        let dark_map: usize = rd(maps, 0x4);
        gloss_map = rd(maps, 0xC);
        let glow_map: usize = rd(maps, 0x10);

        // 6B4393-6B443B: the detail map, blend slot 3.
        if detail_map != 0 {
            let set = coordinate_set(rd(detail_map, MAP::texCoordSet), texture_sets, last_set);
            let filter: u32 = rd(detail_map, MAP::filterMode);
            let clamp: u32 = rd(detail_map, MAP::clampMode);
            let texture: usize = rd(detail_map, MAP::texture);
            if texture != 0 {
                let stage = map_stage(e, this, texture, clamp, filter, [0x6B43E4, 0x6B43F5, 0x6B4406]);
                fill_stage(e, stage, 3, set, detail_map);
                detail_stage = stage;
            }
        }

        // 6B443F-6B44D9: the glow map, blend slot 0.
        if glow_map != 0 {
            let set = coordinate_set(rd(glow_map, MAP::texCoordSet), texture_sets, last_set);
            let clamp: u32 = rd(glow_map, MAP::clampMode);
            let filter: u32 = rd(glow_map, MAP::filterMode);
            let texture: usize = rd(glow_map, MAP::texture);
            if texture != 0 {
                let stage = map_stage(e, this, texture, clamp, filter, [0x6B448E, 0x6B449F, 0x6B44AC]);
                fill_stage(e, stage, 0, set, glow_map);
                glow_stage = stage;
            }
        }

        // 6B44DD-6B4594: the dark map, blend slot 2. The pointer assignment is inline here,
        // with a call of `NiObject::release`.
        if dark_map != 0 {
            let set = coordinate_set(rd(dark_map, MAP::texCoordSet), texture_sets, last_set);
            let texture: usize = rd(dark_map, MAP::texture);
            let clamp: u32 = rd(dark_map, MAP::clampMode);
            let filter: u32 = rd(dark_map, MAP::filterMode);
            if texture != 0 {
                let stage = next_stage(e, this);
                let current: usize = rd(stage, ST::ptrTexture);
                if current != texture {
                    if current != 0 {
                        e.release(0x6B4537, current);
                    }
                    e.wr(stage, ST::ptrTexture, texture);
                    e.wr(texture, REF_COUNT, rd::<u32>(texture, REF_COUNT).wrapping_add(1));
                }
                e.set_addressing(0x6B4552, pipeline_of(this), clamp, stage);
                e.set_filter(0x6B455F, pipeline_of(this), filter, stage);
                fill_stage(e, stage, 2, set, dark_map);
                dark_stage = stage;
            }
        }
    }

    // 6B4598-6B45AE
    let lights: u32 = rd(this, TP::field_CD8);
    if lights == 0 && rd::<u32>(this, TP::field_18A0) == 0 {
        // 6B45B4-6B4642: no projected light. The base and the detail stage as a pair.
        let subpass = e.acquire(0x6B45BA, this, 2, 2);
        let mut bound = 0u32;
        if subpass != 0 {
            bound = e.append_pair(0x6B45D7, subpass, base_stage, detail_stage, 1);
            link_template(e, this, subpass, [0x6B4610, 0x6B4618]);
        }
        if bound != 0 {
            e.wr::<u8>(base_stage, ST::field_90, apply);
            e.apply_blend_ops(0x6B4634, this, base_stage, 0);
            e.apply_blend_ops(0x6B4642, this, detail_stage, 0);
        } else if dark_stage != 0 {
            // 6B46A0-6B46B2: the dark stage takes the place of the base stage.
            e.wr::<u8>(dark_stage, ST::field_90, apply);
        } else {
            // 6B46B4-6B4722: nothing to draw with: the stage with no texture (0x624).
            let fallback = this + TP::field_624;
            if fallback != 0 {
                append_single(e, this, fallback, [0x6B46C4, 0x6B46D2, 0x6B4707, 0x6B470F]);
            }
            e.apply_blend_ops(0x6B4719, this, fallback, 0);
        }
        // 6B4647-6B469B, 6B4B1E-6B4B3E: the dark stage.
        if dark_stage != 0 {
            append_single(e, this, dark_stage, [0x6B4659, 0x6B466B, 0x6B4B2C, 0x6B4B34]);
        }
        e.apply_blend_ops(0x6B4B3E, this, dark_stage, 0);
    } else {
        // 6B4727-6B475F: projected lights. `detailMapFilterMode` counts the lights with no
        // clip plane that are left, `var_36` says the dark stage is not placed yet.
        e.wr::<u32>(this, TP::field_30, 7);
        let mut limit = e.max_textures();
        let stages = e.max_stages();
        if limit < stages {
            limit = stages;
        }
        let mut lights_left = lights;
        let mut clip_index = 0u32; // var_24
        let mut dark_pending = (dark_stage != 0) as u8;
        if rd::<u32>(this, TP::field_18A0) > 0 {
            // 6B4765-6B4777
            let per_pass = limit.wrapping_sub(2);
            one_draw = 0;
            let mut clip_stage = this + TP::field_12C0; // var_14
            loop {
                // 6B477B-6B4802: a pass of its own for the light with a clip plane (bank
                // 0xCE0) and its clip stage (bank 0x12C0).
                e.acquire_for_ps(0x6B477D, this);
                let subpass = e.acquire(0x6B4788, this, 2, 2);
                let light_stage = clip_stage.wrapping_sub(0x5E0);
                if subpass != 0 {
                    e.append_pair(0x6B47A3, subpass, light_stage, clip_stage, 1);
                    link_template(e, this, subpass, [0x6B47D8, 0x6B47E0]);
                }
                e.apply_blend_ops(0x6B47F8, this, light_stage, 0);
                let mut room = per_pass; // temp
                e.apply_blend_ops(0x6B4802, this, clip_stage, 0);
                if room != 0 {
                    // 6B480F-6B487F: the dark stage goes into the first of these passes.
                    if dark_pending != 0 {
                        dark_pending = 0;
                        if dark_stage != 0 {
                            append_single(e, this, dark_stage, [0x6B482A, 0x6B4838, 0x6B486D, 0x6B4875]);
                        }
                        e.apply_blend_ops(0x6B487F, this, dark_stage, 0);
                    }
                    // 6B4884-6B4917: the lights with no clip plane (bank 0x6F8), last first.
                    let mut stage = this.wrapping_add((lights_left as usize).wrapping_mul(STAGE)).wrapping_add(TP::field_6F8);
                    loop {
                        if lights_left == 0 {
                            break;
                        }
                        lights_left -= 1;
                        stage = stage.wrapping_sub(STAGE);
                        if stage != 0 {
                            append_single(e, this, stage, [0x6B48B4, 0x6B48C2, 0x6B48F7, 0x6B48FF]);
                        }
                        e.apply_blend_ops(0x6B4909, this, stage, 0);
                        room = room.wrapping_sub(1);
                        if room == 0 {
                            break;
                        }
                    }
                }
                // 6B4919-6B4938
                clip_index = clip_index.wrapping_add(1);
                clip_stage = clip_stage.wrapping_add(STAGE);
                if clip_index >= rd::<u32>(this, TP::field_18A0) {
                    break;
                }
            }
        }
        // 6B493E-6B49A9: the dark stage when no pass took it.
        if dark_pending != 0 {
            if dark_stage != 0 {
                append_single(e, this, dark_stage, [0x6B4954, 0x6B4962, 0x6B4997, 0x6B499F]);
            }
            e.apply_blend_ops(0x6B49A9, this, dark_stage, 0);
        }
        // 6B49AE-6B4A3E: the lights with no clip plane that are left.
        if lights_left != 0 {
            let mut count = lights_left; // temp
            let mut stage = this.wrapping_add((lights_left as usize).wrapping_mul(STAGE)).wrapping_add(TP::field_6F8);
            loop {
                stage = stage.wrapping_sub(STAGE);
                if stage != 0 {
                    append_single(e, this, stage, [0x6B49DB, 0x6B49E9, 0x6B4A1E, 0x6B4A26]);
                }
                e.apply_blend_ops(0x6B4A30, this, stage, 0);
                count = count.wrapping_sub(1);
                if count == 0 {
                    break;
                }
            }
        }
        // 6B4A40-6B4B3E: the base and the detail stage in a pass after the lights.
        e.wr::<u32>(this, TP::field_30, 8);
        e.acquire_for_ps(0x6B4A49, this);
        let subpass = e.acquire(0x6B4A54, this, 2, 2);
        let mut bound = 0u32;
        if subpass != 0 {
            bound = e.append_pair(0x6B4A6D, subpass, base_stage, detail_stage, 1);
            link_template(e, this, subpass, [0x6B4AA6, 0x6B4AAE]);
        }
        if bound != 0 {
            e.wr::<u8>(base_stage, ST::field_90, apply);
            e.apply_blend_ops(0x6B4ACA, this, base_stage, 0);
            e.apply_blend_ops(0x6B4B3E, this, detail_stage, 0);
        } else {
            // 6B4AD8-6B4B3E
            let fallback = this + TP::field_624;
            if fallback != 0 {
                append_single(e, this, fallback, [0x6B4AE8, 0x6B4AF6, 0x6B4B2C, 0x6B4B34]);
            }
            e.apply_blend_ops(0x6B4B3E, this, fallback, 0);
        }
    }

    // 6B4B43-6B4C91: the decal maps (maps 6 and up), blend slot 1, each in a pass of its own.
    if decal_count != 0 {
        let mut offset = 0x18usize; // var_14
        loop {
            let map: usize = rd(rd::<usize>(property, NTP::maps_storage), offset);
            if map != 0 {
                let set = coordinate_set(rd(map, MAP::texCoordSet), texture_sets, last_set);
                let texture: usize = rd(map, MAP::texture);
                let clamp: u32 = rd(map, MAP::clampMode);
                let filter: u32 = rd(map, MAP::filterMode);
                if texture != 0 {
                    let stage = map_stage(e, this, texture, clamp, filter, [0x6B4BBC, 0x6B4BCD, 0x6B4BDA]);
                    fill_stage(e, stage, 1, set, map);
                    e.wr::<u8>(stage, ST::field_90, apply);
                    e.acquire_for_ps(0x6B4C17, this);
                    append_single(e, this, stage, [0x6B4C22, 0x6B4C30, 0x6B4C66, 0x6B4C6E]);
                    e.apply_blend_ops(0x6B4C78, this, stage, 0);
                }
            }
            offset = offset.wrapping_add(4);
            decal_count = decal_count.wrapping_sub(1);
            if decal_count == 0 {
                break;
            }
        }
    }

    // 6B4C97-6B5148: projected shadows.
    if rd::<u32>(this, TP::field_2A48) != 0 || rd::<u32>(this, TP::field_2464) != 0 {
        // 6B4CAF-6B4D1C: the blend slots follow the alpha property: alpha test (the
        // reference value goes into alpha template 4), blending, or neither.
        let alpha: usize = rd(property_state, PS::propAlpha);
        let flags: u16 = rd(alpha, AP::flags);
        let mut template = 0usize; // var_14
        let mut base_slot = 0u32; // temp; the game code leaves it as it was in the last case
        let shadow_slot: u32; // detailMapFilterMode
        if (flags >> 9) & 1 != 0 {
            let reference: u8 = rd(alpha, AP::alphaTestRef);
            base_slot = 4;
            shadow_slot = 5;
            template = rd(pipeline_of(this), CTP::field_2E8 + 4 * 4);
            e.wr(template, AP::alphaTestRef, reference);
        } else if rd::<u8>(alpha, AP::flags) & 1 != 0 {
            base_slot = 6;
            shadow_slot = 7;
            template = rd(pipeline_of(this), CTP::field_2E8 + 6 * 4);
        } else {
            shadow_slot = 2;
        }

        // 6B4D24-6B4D55
        let mut shadows_left: u32 = rd(this, TP::field_2A48); // var_34
        let mut limit = e.max_textures(); // otherStage
        let stages = e.max_stages();
        if limit < stages {
            limit = stages;
        }
        let mut clip_index = 0u32; // var_24
        let mut clip_count: u32 = rd(this, TP::field_2464);
        if clip_count != 0 {
            // 6B4D5B-6B4D6A
            let mut clip_stage = this + TP::field_18A4; // decalMapCount
            one_draw = 0;
            loop {
                // 6B4D70-6B4E1C: with an alpha template, the base stage first.
                e.acquire_for_ps(0x6B4D76, this);
                let mut room = limit; // detailMapClampMode
                if template != 0 && base_stage != 0 {
                    let subpass = e.acquire(0x6B4D99, this, 1, 1);
                    if subpass != 0 {
                        let appended = e.append_stage(0x6B4DA7, subpass, base_stage);
                        link_template(e, this, subpass, [0x6B4DE0, 0x6B4DE8]);
                        if appended != 0 {
                            e.wr::<u8>(base_stage, ST::field_90, apply);
                            e.wr::<u32>(base_stage, ST::field_94, base_slot);
                            e.wr::<u32>(clip_stage, ST::field_94, shadow_slot);
                            e.apply_blend_ops(0x6B4E1C, this, base_stage, 0);
                        }
                    }
                }
                // 6B4E21-6B4EA8: the shadow with a clip plane (bank 0x18A4) and its clip
                // stage (bank 0x1E84).
                let mut bound = 0u32;
                let subpass = e.acquire(0x6B4E2F, this, 1, 1);
                if subpass != 0 {
                    bound = e.append_pair(0x6B4E46, subpass, clip_stage, clip_stage.wrapping_add(0x5E0), 1);
                    link_template(e, this, subpass, [0x6B4E7F, 0x6B4E87]);
                }
                e.apply_blend_ops(0x6B4E91, this, clip_stage, 0);
                if bound & 2 != 0 {
                    e.apply_blend_ops(0x6B4EA8, this, clip_stage.wrapping_add(0x5E0), 0);
                }
                // 6B4EAD-6B4F5B: the shadows with no clip plane (bank 0x2468), last first.
                if limit != 0 {
                    let mut stage = this.wrapping_add((shadows_left as usize).wrapping_mul(STAGE)).wrapping_add(TP::field_2468);
                    loop {
                        if shadows_left == 0 {
                            break;
                        }
                        // `[ebx-28h]`: the blend slot of the stage below.
                        e.wr::<u32>(stage.wrapping_sub(0x28), 0, 2);
                        shadows_left -= 1;
                        stage = stage.wrapping_sub(STAGE);
                        if stage != 0 {
                            append_single(e, this, stage, [0x6B4EF8, 0x6B4F06, 0x6B4F3B, 0x6B4F43]);
                        }
                        e.apply_blend_ops(0x6B4F4D, this, stage, 0);
                        room = room.wrapping_sub(1);
                        if room == 0 {
                            break;
                        }
                    }
                }
                // 6B4F61-6B4F80
                clip_index = clip_index.wrapping_add(1);
                clip_stage = clip_stage.wrapping_add(STAGE);
                clip_count = rd(this, TP::field_2464);
                if clip_index >= clip_count {
                    break;
                }
            }
        }
        // 6B4F86-6B4F9F: the call is made when the check says the sub-pass can stay.
        if clip_count == 0 && e.init_or_check(0x6B4F94, this, 2, 1, 1) == 0 {
            e.acquire_for_ps(0x6B4F9F, this);
        }
        // 6B4FA4-6B5148: the shadows with no clip plane that are left.
        if shadows_left != 0 {
            loop {
                // 6B4FB0-6B4FC0
                let mut room = 1u32; // detailMapClampMode
                let mut first = 0u8; // the byte in `propertyState`
                let mut run = true;
                if template != 0 {
                    // 6B4FC6-6B5080: a pass that starts with the base stage.
                    e.acquire_for_ps(0x6B4FC8, this);
                    if base_stage != 0 {
                        let subpass = e.acquire(0x6B4FDF, this, 1, 1);
                        if subpass != 0 {
                            let appended = e.append_stage(0x6B4FF1, subpass, base_stage);
                            link_template(e, this, subpass, [0x6B502A, 0x6B5032]);
                            if appended != 0 {
                                e.wr::<u8>(base_stage, ST::field_90, apply);
                                e.wr::<u32>(base_stage, ST::field_94, base_slot);
                                // `[esi+ecx*4+1938h]`: the blend slot of the stage of bank
                                // 0x18A4 with the number `var_24`.
                                let clip_stage = this.wrapping_add((clip_index as usize).wrapping_mul(STAGE));
                                e.wr::<u32>(clip_stage, TP::field_18A4 + ST::field_94, shadow_slot);
                                e.apply_blend_ops(0x6B506F, this, base_stage, 0);
                                first = 1;
                            }
                        }
                    }
                    room = limit.wrapping_sub(1);
                    run = room != 0;
                }
                if run {
                    // 6B5086-6B513C. The game code does not stop at the first stage of the
                    // bank here.
                    let mut stage = this.wrapping_add((shadows_left as usize).wrapping_mul(STAGE)).wrapping_add(TP::field_2468);
                    loop {
                        shadows_left = shadows_left.wrapping_sub(1);
                        stage = stage.wrapping_sub(STAGE);
                        if first != 0 {
                            first = 0;
                            e.wr::<u32>(stage, ST::field_94, shadow_slot);
                        } else {
                            e.wr::<u32>(stage, ST::field_94, 2);
                        }
                        if stage != 0 {
                            append_single(e, this, stage, [0x6B50D9, 0x6B50E7, 0x6B511C, 0x6B5124]);
                        }
                        e.apply_blend_ops(0x6B512E, this, stage, 0);
                        room = room.wrapping_sub(1);
                        if room == 0 {
                            break;
                        }
                    }
                }
                // 6B5142-6B5148
                if shadows_left == 0 {
                    break;
                }
            }
        }
    }

    // 6B514E-6B5555: the environment map of the effect state, with the bump map (map 5) and
    // the gloss map (map 3) of the property. Without the environment map both are dropped.
    if rd::<u8>(this, TP::field_2B08) != 0 {
        // 6B515C-6B524A: the bump map, blend slot 8. `propertyState` counts the stages the
        // pass must have room for.
        let bump_map: usize = rd(rd::<usize>(property, NTP::maps_storage), 0x14);
        let mut needed: u32 = (gloss_map != 0) as u32 + 1;
        let mut bump_stage = 0usize;
        if bump_map != 0 {
            let set = coordinate_set(rd(bump_map, MAP::texCoordSet), texture_sets, last_set);
            let texture: usize = rd(bump_map, MAP::texture);
            let filter: u32 = rd(bump_map, MAP::filterMode);
            let clamp: u32 = rd(bump_map, MAP::clampMode);
            if texture != 0 {
                let stage = map_stage(e, this, texture, clamp, filter, [0x6B51D2, 0x6B51E3, 0x6B51F4]);
                fill_stage(e, stage, 8, set, bump_map);
                bump_stage = stage;
                // 6B5227-6B524A: bump, environment and gloss need three stages and textures.
                if gloss_map != 0 && (e.max_stages() < 3 || e.max_textures() < 3) {
                    gloss_map = 0;
                } else {
                    needed = needed.wrapping_add(1);
                }
            }
        }
        // 6B524E-6B526B: with a gloss map always a new pass, else when the check asks for it.
        let new_pass = if gloss_map != 0 { true } else { e.init_or_check(0x6B525C, this, 0, needed, needed) != 0 };
        if new_pass {
            e.acquire_for_ps(0x6B526B, this);
        }

        // 6B5270-6B5363: the bump stage and the environment stage as a pair. The byte in
        // `propertyState` says the environment stage is placed.
        let env_stage = this + TP::field_2A4C;
        let mut env_placed = 0u8;
        if bump_stage != 0 && rd::<u32>(pipeline_of(this), CTP::field_104) != 0 {
            e.wr::<u8>(bump_stage, ST::bumpEnvMapEnabled, 1);
            for (to, from) in [
                (ST::bumpEnvMap00, BUMP::bumpMat),
                (ST::bumpEnvMap01, BUMP::bumpMat + 0x4),
                (ST::bumpEnvMap10, BUMP::bumpMat + 0x8),
                (ST::bumpEnvMap11, BUMP::bumpMat + 0xC),
                (ST::lumaScale, BUMP::lumaScale),
                (ST::lumaOffset, BUMP::lumaOffset),
            ] {
                // `fld` / `fstp`
                e.wr::<u32>(bump_stage, to, x87_pass(rd(bump_map, from)));
            }
            e.wr::<u8>(bump_stage, ST::field_90, 1);
            let subpass = e.acquire(0x6B52DB, this, 2, 2);
            if subpass != 0 {
                e.append_pair(0x6B52F2, subpass, bump_stage, env_stage, 1);
                link_template(e, this, subpass, [0x6B532C, 0x6B5338]);
            }
            e.apply_blend_ops(0x6B5342, this, bump_stage, 0);
            e.apply_blend_ops(0x6B5352, this, env_stage, 0);
            env_placed = 1;
            if rd::<u32>(this, TP::field_C) > 0 {
                e.wr::<u32>(env_stage, ST::colorOp, 2);
            }
        }

        // 6B536D-6B54F5: the gloss map, blend slot 2.
        if gloss_map != 0 {
            let set = coordinate_set(rd(gloss_map, MAP::texCoordSet), texture_sets, last_set);
            let texture: usize = rd(gloss_map, MAP::texture);
            let clamp: u32 = rd(gloss_map, MAP::clampMode);
            let filter: u32 = rd(gloss_map, MAP::filterMode);
            let mut gloss_stage = 0usize;
            if texture != 0 {
                let stage = map_stage(e, this, texture, clamp, filter, [0x6B53C0, 0x6B53D1, 0x6B53DE]);
                fill_stage(e, stage, 2, set, gloss_map);
                e.wr::<u8>(stage, ST::field_90, apply);
                gloss_stage = stage;
            }
            if env_placed != 0 {
                // 6B5421-6B548C
                if gloss_stage != 0 {
                    append_single(e, this, gloss_stage, [0x6B542F, 0x6B5441, 0x6B547F, 0x6B5487]);
                }
            } else {
                // 6B548E-6B54EB: the environment stage and the gloss stage as a pair.
                let subpass = e.acquire(0x6B5494, this, 2, 2);
                if subpass != 0 {
                    e.append_pair(0x6B54AB, subpass, env_stage, gloss_stage, 1);
                    link_template_call(e, this, subpass, 0x6B54D6);
                }
                e.apply_blend_ops(0x6B54E6, this, env_stage, 0);
                env_placed = 1;
            }
            e.apply_blend_ops(0x6B54F5, this, gloss_stage, 0);
        }

        // 6B54FA-6B5555: the environment stage alone.
        if env_placed == 0 {
            if env_stage != 0 {
                let subpass = e.acquire(0x6B5512, this, 1, 1);
                if subpass != 0 {
                    e.append_stage(0x6B5520, subpass, env_stage);
                    link_template_call(e, this, subpass, 0x6B554B);
                }
            }
            e.apply_blend_ops(0x6B5555, this, env_stage, 0);
        }
    }

    // 6B555A-6B55C8: the glow stage, in a pass that starts with a stage of its blend slot.
    if glow_stage != 0 {
        if e.init_or_check(0x6B556F, this, rd(glow_stage, ST::field_94), 1, 1) != 0 {
            e.acquire_for_ps(0x6B557A, this);
        }
        let subpass = e.acquire(0x6B5585, this, 1, 1);
        if subpass != 0 {
            e.append_stage(0x6B5593, subpass, glow_stage);
            link_template_call(e, this, subpass, 0x6B55BE);
        }
        e.apply_blend_ops(0x6B55C8, this, glow_stage, 0);
    }

    // 6B55CD-6B5603: the fog map of the effect state.
    if rd::<u8>(this, TP::field_2BC8) != 0 {
        if rd::<u32>(this, TP::field_8) != 0 {
            e.acquire_for_ps(0x6B55E0, this);
        }
        let fog_stage = this + TP::field_2B0C;
        e.bind_stage_texture(0x6B55F2, this, fog_stage, 1, 1);
        e.wr::<u8>(fog_stage, ST::field_90, 0);
        e.apply_blend_ops(0x6B5603, this, fog_stage, 0);
    }

    finish(e, this, one_draw)
}

/// 0x6B41B0 `NiDX8TexturePass::setCTPipelineState`, slot 8 of the pass. Returns 0x80000000
/// when the draw has more than two sub-passes and none of them is of a projected light or
/// shadow with a clip plane, else 0. The last five arguments are not used.
#[cfg_attr(test, allow(dead_code))]
pub unsafe fn set_ct_pipeline_state(
    this: usize,
    property_state: usize,
    effect_state: usize,
    _has_normals: u32,
    texture_sets: u32,
    _geometry_data: usize,
    _skin_instance: usize,
    _world_transform: usize,
    _world_bound: usize,
) -> u32 {
    crate::shadow::touch(&set_ct_pipeline_state_port::STATE);
    build(&mut Live, this, property_state, effect_state, texture_sets)
}

// ---------------------------------------------------------------------------------------
// The dry run
// ---------------------------------------------------------------------------------------

/// One write of the dry run: where, how many bytes, and what was there.
#[derive(Clone, Copy)]
struct Entry {
    addr: usize,
    len: usize,
    old: [u8; 8],
}

/// The dry run of the body. Every write goes to memory and into the journal; the callees
/// are copies of the game functions (from the ports of `texture_pass_a` and `texture_pass_b`)
/// that write through the journal too. They call nothing that cannot be undone: an object
/// that would be deleted, or a device texture that would be made, sets `skip`. Sub-passes and
/// array storage the game would allocate are blocks of the model.
struct Model {
    max_stages: u32,
    max_textures: u32,
    base_texcoord_patch: bool,
    journal: Vec<Entry>,
    /// Blocks that stand for memory the game would allocate.
    scratch: Vec<Box<[u32]>>,
    allocates: bool,
    skip: bool,
}

impl Model {
    fn new(max_stages: u32, max_textures: u32, base_texcoord_patch: bool) -> Model {
        Model { max_stages, max_textures, base_texcoord_patch, journal: Vec::with_capacity(256), scratch: Vec::new(), allocates: false, skip: false }
    }

    /// Puts back every byte the run wrote, last write first.
    unsafe fn roll_back(&self) {
        for entry in self.journal.iter().rev() {
            core::ptr::copy_nonoverlapping(entry.old.as_ptr(), entry.addr as *mut u8, entry.len);
        }
    }

    fn in_scratch(&self, addr: usize) -> bool {
        self.scratch.iter().any(|block| {
            let start = block.as_ptr() as usize;
            addr >= start && addr < start + block.len() * 4
        })
    }

    fn block(&mut self, words: usize) -> usize {
        self.allocates = true;
        let mut block = vec![0u32; words].into_boxed_slice();
        let addr = block.as_mut_ptr() as usize;
        self.scratch.push(block);
        addr
    }

    /// `new NiDX8TexturePass::$1`: no stages, no objects.
    unsafe fn new_subpass(&mut self) -> usize {
        let subpass = self.block(SUB::SIZE / 4);
        crate::mem::wr::<u32>(subpass, SUB::field_C, u32::MAX);
        subpass
    }

    unsafe fn slot(this: usize, index: u32) -> usize {
        rd::<usize>(this, TP::TArray_18_storage) + index as usize * 4
    }

    /// The game code would read outside the array: the call is not compared, and the run
    /// goes on with a sub-pass of the model.
    unsafe fn lost(&mut self, this: usize) {
        self.skip = true;
        let subpass = self.new_subpass();
        self.wr(this, TP::substruct_14, subpass);
    }

    unsafe fn add_reference(&mut self, object: usize) {
        self.wr(object, REF_COUNT, rd::<u32>(object, REF_COUNT).wrapping_add(1));
    }

    /// One reference less. The game would delete the object with the last one.
    unsafe fn drop_reference(&mut self, object: usize) {
        let count = rd::<u32>(object, REF_COUNT).wrapping_sub(1);
        if count == 0 {
            self.skip = true;
        }
        self.wr(object, REF_COUNT, count);
    }

    unsafe fn assign(&mut self, pointer: usize, object: usize) {
        let current: usize = rd(pointer, 0);
        if current != object {
            if current != 0 {
                self.drop_reference(current);
            }
            self.wr(pointer, 0, object);
            if object != 0 {
                self.add_reference(object);
            }
        }
    }

    // The array of sub-passes, as in `texture_pass_a`.

    unsafe fn array_set_at(&mut self, this: usize, index: u32, value: usize) {
        if index >= rd::<u32>(this, TP::TArray_18_storageCount) {
            return;
        }
        let filled: u32 = rd(this, TP::TArray_18_filledCount);
        if index >= rd::<u32>(this, TP::TArray_18_endIndex) {
            self.wr(this, TP::TArray_18_endIndex, index + 1);
            if value != 0 {
                self.wr(this, TP::TArray_18_filledCount, filled.wrapping_add(1));
            }
        } else if value != 0 {
            if rd::<usize>(Model::slot(this, index), 0) == 0 {
                self.wr(this, TP::TArray_18_filledCount, filled.wrapping_add(1));
            }
        } else if rd::<usize>(Model::slot(this, index), 0) != 0 {
            self.wr(this, TP::TArray_18_filledCount, filled.wrapping_sub(1));
        }
        self.wr(Model::slot(this, index), 0, value);
    }

    unsafe fn array_clear_slot(&mut self, this: usize, index: u32) {
        if index < rd::<u32>(this, TP::TArray_18_storageCount) {
            if index >= rd::<u32>(this, TP::TArray_18_endIndex) {
                self.wr(this, TP::TArray_18_endIndex, index + 1);
            } else if rd::<usize>(Model::slot(this, index), 0) != 0 {
                self.wr(this, TP::TArray_18_filledCount, rd::<u32>(this, TP::TArray_18_filledCount).wrapping_sub(1));
            }
            self.wr::<usize>(Model::slot(this, index), 0, 0);
        }
    }

    /// A larger storage. The old one is freed in the game, so it needs no block.
    unsafe fn array_set_size(&mut self, this: usize, size: u32) {
        if size == rd::<u32>(this, TP::TArray_18_storageCount) {
            return;
        }
        let end: u32 = rd(this, TP::TArray_18_endIndex);
        if size < end || size > 0x1000 {
            // The path that makes the array smaller is not in the model.
            self.skip = true;
            return;
        }
        let old: usize = rd(this, TP::TArray_18_storage);
        let new = self.block(size as usize);
        for index in 0..end as usize {
            crate::mem::wr::<usize>(new, index * 4, rd(old, index * 4));
        }
        self.wr(this, TP::TArray_18_storageCount, size);
        self.wr(this, TP::TArray_18_storage, new);
    }

    unsafe fn ensure_current_subpass(&mut self, this: usize) -> bool {
        if rd::<usize>(this, TP::substruct_14) != 0 {
            return false;
        }
        let index: u32 = rd(this, TP::field_C);
        if index >= rd::<u32>(this, TP::TArray_18_storageCount) {
            self.lost(this);
            return true;
        }
        let existing: usize = rd(Model::slot(this, index), 0);
        self.wr(this, TP::substruct_14, existing);
        if existing != 0 {
            return false;
        }
        let subpass = self.new_subpass();
        self.wr(this, TP::substruct_14, subpass);
        self.array_set_at(this, index, subpass);
        true
    }

    unsafe fn advance_to_next_subpass(&mut self, this: usize) {
        let index = rd::<u32>(this, TP::field_C).wrapping_add(1);
        let count: u32 = rd(this, TP::count_10);
        self.wr(this, TP::field_C, index);
        if index >= count {
            let size = count.wrapping_add(2);
            self.wr(this, TP::count_10, size);
            self.array_set_size(this, size);
            let mut clear = index;
            while clear < rd::<u32>(this, TP::count_10) {
                self.array_clear_slot(this, clear);
                clear += 1;
            }
        }
        if index >= rd::<u32>(this, TP::TArray_18_storageCount) {
            self.lost(this);
            return;
        }
        let existing: usize = rd(Model::slot(this, index), 0);
        self.wr(this, TP::substruct_14, existing);
        if existing == 0 {
            let subpass = self.new_subpass();
            self.array_set_at(this, index, subpass);
            self.wr(this, TP::substruct_14, subpass);
        }
    }

    /// `getAvailableSlotCounts`: free stages and free textures.
    unsafe fn free_slots(&self, subpass: usize) -> (u32, u32) {
        let used: u32 = rd(subpass, SUB::field_C);
        let stages = if used == u32::MAX { self.max_stages } else { self.max_stages.wrapping_sub(used) };
        (stages, self.max_textures.wrapping_sub(rd(subpass, SUB::field_10)))
    }

    // The sub-pass, as in `texture_pass_b`.

    /// The stage goes into the list with the next index. Returns the stage count before and
    /// the next index, or `None` when the list is full (the game code has no such check).
    unsafe fn store_appended(&mut self, subpass: usize, stage: usize) -> Option<(u32, u32)> {
        let index: u32 = rd(subpass, SUB::field_8);
        if index >= 8 {
            self.skip = true;
            return None;
        }
        self.wr(stage, ST::stage, index);
        self.wr(subpass, SUB::field_14 + index as usize * 4, stage);
        let count: u32 = rd(subpass, SUB::field_C);
        self.wr(subpass, SUB::field_8, index + 1);
        Some((count, index + 1))
    }

    unsafe fn count_appended(&mut self, subpass: usize, stage: usize, count: u32, next: u32) {
        if count < next || count == u32::MAX {
            self.wr(subpass, SUB::field_C, next);
        }
        if rd::<usize>(stage, ST::ptrTexture) != 0 {
            self.wr(subpass, SUB::field_10, rd::<u32>(subpass, SUB::field_10).wrapping_add(1));
        }
    }

    unsafe fn append(&mut self, subpass: usize, stage: usize) -> bool {
        match self.store_appended(subpass, stage) {
            Some((count, next)) => {
                self.count_appended(subpass, stage, count, next);
                true
            }
            None => false,
        }
    }

    /// `NiDX8TexturePass::setTexturePipeline` for an effect state with no projected light or
    /// shadow. The texture matrix the coordinate callback computes is not written: it is
    /// inside the pass, and nothing in the body reads it.
    unsafe fn configure_effect_stage(&mut self, this: usize, effect: usize, stage: usize) -> u8 {
        if rd::<u32>(this, TP::field_6E0) >= 8 || rd::<u8>(effect, EFFECT_ENABLED) == 0 {
            return 0;
        }
        let texture: usize = rd(effect, EFFECT_TEXTURE);
        if texture == 0 {
            return 0;
        }
        if rd::<usize>(texture, NiTexture::rendererData) == 0 {
            // `GetFormatFromTexture` makes the device texture.
            self.skip = true;
            return 0;
        }
        if texture_format(rd(this, TP::textureManager), texture) == 14 {
            return 0;
        }
        self.wr::<u32>(stage, ST::sourceTexcoordIndex, 1);
        self.assign(stage + ST::ptrTexture, texture);
        self.wr::<u8>(stage, ST::bumpEnvMapEnabled, 0);
        self.wr(this, TP::field_6E0, rd::<u32>(this, TP::field_6E0).wrapping_add(1));
        self.wr(stage, ST::texturePropMap, effect);
        self.wr::<u8>(stage, ST::field_90, 0);
        let pipeline = pipeline_of(this);
        self.set_filter(0, pipeline, rd(effect, EFFECT_FILTER_MODE), stage);
        self.set_addressing(0, pipeline, rd(effect, EFFECT_CLAMP_MODE), stage);
        1
    }
}

/// `NiDX8TextureManager::GetFormatFromTexture`, slot 0x10 of the texture manager. It only
/// reads when the texture has its device data.
#[cfg(not(test))]
unsafe fn texture_format(texture_manager: usize, texture: usize) -> u32 {
    let vtable: usize = rd(texture_manager, 0);
    transmute::<usize, This1>(rd::<usize>(vtable, 0x10))(texture_manager, texture)
}

#[cfg(test)]
unsafe fn texture_format(_texture_manager: usize, _texture: usize) -> u32 {
    0
}

/// True when slot 0x18 of the pass is the `resetPassFlags` the model knows.
#[cfg(not(test))]
unsafe fn knows_reset_pass_flags(this: usize) -> bool {
    rd::<usize>(rd::<usize>(this, 0), 0x18) == ADDR_RESET_PASS_FLAGS
}

#[cfg(test)]
unsafe fn knows_reset_pass_flags(_this: usize) -> bool {
    true
}

impl Env for Model {
    unsafe fn wr<T: Copy>(&mut self, base: usize, off: usize, value: T) {
        let addr = base.wrapping_add(off);
        let len = size_of::<T>();
        debug_assert!(len <= 8);
        let mut old = [0u8; 8];
        core::ptr::copy_nonoverlapping(addr as *const u8, old.as_mut_ptr(), len);
        self.journal.push(Entry { addr, len, old });
        crate::mem::wr(addr, 0, value);
    }

    unsafe fn max_stages(&mut self) -> u32 {
        self.max_stages
    }

    unsafe fn max_textures(&mut self) -> u32 {
        self.max_textures
    }

    unsafe fn base_texcoord_patch(&mut self) -> bool {
        self.base_texcoord_patch
    }

    unsafe fn set_texture_pipeline(&mut self, _site: usize, this: usize, effect_state: usize) {
        let pipeline = pipeline_of(this);
        if effect_state == rd::<usize>(pipeline, CTP::field_B04) {
            return;
        }
        if !knows_reset_pass_flags(this) {
            self.skip = true;
            return;
        }
        // `resetPassFlags`
        self.wr::<u32>(this, TP::field_CD8, 0);
        self.wr::<u32>(this, TP::field_18A0, 0);
        self.wr::<u32>(this, TP::field_2A48, 0);
        self.wr::<u32>(this, TP::field_2464, 0);
        self.wr::<u8>(this, TP::field_2B08, 0);
        self.wr::<u8>(this, TP::field_2A4C + ST::bumpEnvMapEnabled, 0);
        self.wr::<u8>(this, TP::field_2BC8, 0);
        self.wr::<u32>(this, TP::field_2BD0, 0);
        self.wr::<u32>(this, TP::field_6E0, 0);
        self.wr(pipeline, CTP::field_B04, effect_state);
        self.wr(this, TP::field_2BD0, effect_state);
        if effect_state == 0 {
            return;
        }
        if rd::<usize>(effect_state, DES::projLights) != 0 || rd::<usize>(effect_state, DES::projShadows) != 0 {
            // Projected lights and shadows are not in the model.
            self.skip = true;
            return;
        }
        let environment_map: usize = rd(effect_state, DES::environmentMap);
        if environment_map != 0 {
            let valid = self.configure_effect_stage(this, environment_map, this + TP::field_2A4C);
            self.wr::<u8>(this, TP::field_2B08, valid);
        }
        let fog_map: usize = rd(effect_state, DES::fogMap);
        if fog_map != 0 {
            let valid = self.configure_effect_stage(this, fog_map, this + TP::field_2B0C);
            self.wr::<u8>(this, TP::field_2BC8, valid);
        }
    }

    unsafe fn pointer_set(&mut self, _site: usize, pointer: usize, object: usize) {
        self.assign(pointer, object);
    }

    unsafe fn release(&mut self, _site: usize, object: usize) {
        self.drop_reference(object);
    }

    /// The values come from tables of the game; here the stage gets the mode itself. The
    /// body does not read them back, and the run is rolled back.
    unsafe fn set_addressing(&mut self, _site: usize, _pipeline: usize, clamp_mode: u32, stage: usize) {
        self.wr(stage, ST::addressModeU, clamp_mode);
        self.wr(stage, ST::addressModeV, clamp_mode);
    }

    unsafe fn set_filter(&mut self, _site: usize, _pipeline: usize, filter_mode: u32, stage: usize) {
        self.wr(stage, ST::textureStageIndex, filter_mode);
        self.wr(stage, ST::filter_3C, filter_mode);
        self.wr(stage, ST::filter_40, filter_mode);
        self.wr(stage, ST::filter_44, filter_mode);
    }

    /// 0x6E1D40: a map after the base map.
    unsafe fn is_multi_texture(&mut self, _site: usize, property: usize) -> u8 {
        let end: u32 = rd(property, NTP::maps_endIndex);
        let maps: usize = rd(property, NTP::maps_storage);
        let mut index = 1u32;
        while index < end {
            if rd::<usize>(maps, index as usize * 4) != 0 {
                return 1;
            }
            index += 1;
        }
        0
    }

    unsafe fn bind_stage_texture(&mut self, _site: usize, this: usize, stage: usize, textures: u32, stages: u32) -> u8 {
        if stage == 0 {
            return 0;
        }
        let subpass = self.acquire(0, this, textures, stages);
        if subpass == 0 {
            return 0;
        }
        let result = self.append_stage(0, subpass, stage);
        if let Some(template) = template_for(this, subpass) {
            self.set_field_48(0, subpass, template);
        }
        result
    }

    unsafe fn apply_blend_ops(&mut self, _site: usize, this: usize, stage: usize, flag: u32) {
        if stage == 0 {
            return;
        }
        let pipeline = pipeline_of(this);
        let slot: u32 = rd(stage, ST::field_94);
        if rd::<u32>(stage, ST::stage) == 0 && flag as u8 == 0 {
            let entry = pipeline.wrapping_add((slot.wrapping_mul(3).wrapping_add(0x21) as usize).wrapping_mul(8));
            for index in 0..6 {
                self.wr::<u32>(stage, ST::colorOp + index * 4, rd(entry, index * 4));
            }
            if rd::<u8>(stage, ST::field_90) == 0 {
                self.wr::<u32>(stage, ST::colorOp, 2);
                self.wr::<u32>(stage, ST::colorArg1, 2);
                self.wr::<u32>(stage, ST::alphaOp, 2);
                self.wr::<u32>(stage, ST::alphaArg1, 2);
            } else if rd::<usize>(stage, ST::ptrTexture) == 0 {
                self.wr::<u32>(stage, ST::colorOp, 3);
                self.wr::<u32>(stage, ST::alphaOp, 3);
            }
        } else {
            let entry = pipeline.wrapping_add((slot.wrapping_mul(3).wrapping_add(0x3F) as usize).wrapping_mul(8));
            for index in 0..6 {
                self.wr::<u32>(stage, ST::colorOp + index * 4, rd(entry, index * 4));
            }
        }
    }

    unsafe fn acquire(&mut self, _site: usize, this: usize, a2: u32, a3: u32) -> usize {
        self.ensure_current_subpass(this);
        let (stages, textures) = self.free_slots(rd(this, TP::substruct_14));
        if stages < a3 || textures < a2 {
            self.advance_to_next_subpass(this);
        }
        rd(this, TP::substruct_14)
    }

    unsafe fn acquire_for_ps(&mut self, _site: usize, this: usize) {
        self.ensure_current_subpass(this);
        if rd::<u32>(rd::<usize>(this, TP::substruct_14), SUB::field_C) != u32::MAX {
            self.advance_to_next_subpass(this);
        }
    }

    unsafe fn init_or_check(&mut self, _site: usize, this: usize, a2: u32, a3: u32, a4: u32) -> u8 {
        if self.ensure_current_subpass(this) {
            return 0;
        }
        let subpass: usize = rd(this, TP::substruct_14);
        if rd::<u32>(this, TP::field_C) != 0 {
            let first_stage: usize = rd(subpass, SUB::field_14);
            if first_stage == 0 {
                // The game code would read through a null pointer.
                self.skip = true;
                return 1;
            }
            if rd::<u32>(first_stage, ST::field_94) != a2 {
                return 1;
            }
        }
        let (stages, textures) = self.free_slots(subpass);
        (stages < a3 || textures < a4) as u8
    }

    unsafe fn append_stage(&mut self, _site: usize, subpass: usize, stage: usize) -> u8 {
        if stage == 0 {
            return 0;
        }
        if rd::<u32>(subpass, SUB::field_8) != 0 {
            let texture: usize = rd(stage, ST::ptrTexture);
            self.wr::<u32>(stage, ST::colorArg2, 1);
            self.wr::<u32>(stage, ST::colorArg1, if texture != 0 { 2 } else { 0 });
        }
        self.append(subpass, stage) as u8
    }

    unsafe fn append_pair(&mut self, _site: usize, subpass: usize, stage: usize, other_stage: usize, flag: u32) -> u32 {
        let textures: u32 = rd(subpass, SUB::field_10);
        if textures >= self.max_textures {
            return 0;
        }
        if other_stage == 0 {
            if stage == 0 {
                return 0;
            }
            return self.append(subpass, stage) as u32;
        }
        if stage == 0 {
            if flag as u8 != 0 {
                return 0;
            }
            return if self.append(subpass, other_stage) { 2 } else { 0 };
        }
        let room_for_two = textures < self.max_textures.wrapping_sub(1);
        if !self.append(subpass, stage) {
            return 0;
        }
        if !room_for_two {
            return 1;
        }
        if self.append(subpass, other_stage) {
            3
        } else {
            1
        }
    }

    unsafe fn set_field_48(&mut self, _site: usize, subpass: usize, object: usize) {
        self.assign(subpass + SUB::field_48, object);
    }
}

// ---------------------------------------------------------------------------------------
// Regions for the shadow comparison
// ---------------------------------------------------------------------------------------

fn inside(addr: usize, start: usize, size: usize) -> bool {
    addr >= start && addr < start + size
}

/// The blocks the call writes outside the pass and the pipeline, from the journal of a dry
/// run. A write into a sub-pass of the array counts as the whole sub-pass. Returns `None`
/// when there are more than `out` holds.
fn blocks_of(model: &Model, this: usize, pipeline: usize, subpasses: &[usize], out: &mut [(usize, usize); 6]) -> Option<usize> {
    let mut len = 0;
    for entry in &model.journal {
        if inside(entry.addr, this, TP::SIZE) || inside(entry.addr, pipeline, CTP::SIZE) || model.in_scratch(entry.addr) {
            continue;
        }
        let (start, size) = match subpasses.iter().find(|&&subpass| inside(entry.addr, subpass, SUB::SIZE)) {
            Some(&subpass) => (subpass, SUB::SIZE),
            None => (entry.addr, entry.len),
        };
        if out[..len].iter().any(|&(s, n)| start >= s && start + size <= s + n) {
            continue;
        }
        if len == out.len() {
            return None;
        }
        out[len] = (start, size);
        len += 1;
    }
    Some(len)
}

/// Runs the body on the model, puts the memory back, and gives the blocks the call writes.
#[allow(clippy::too_many_arguments)]
unsafe fn plan(
    set: &mut Regions,
    max_stages: u32,
    max_textures: u32,
    base_texcoord_patch: bool,
    this: usize,
    property_state: usize,
    effect_state: usize,
    texture_sets: u32,
    render_state: usize,
) {
    let pipeline = pipeline_of(this);
    set.add(this, TP::SIZE);
    set.add(pipeline, CTP::SIZE);

    let count: u32 = rd(this, TP::count_10);
    if count > 64 || count > rd::<u32>(this, TP::TArray_18_storageCount) {
        // Not a state the game leaves the array in.
        set.skip();
        return;
    }
    let storage: usize = rd(this, TP::TArray_18_storage);
    let mut subpasses = [0usize; 64];
    let mut subpass_count = 0;
    for index in 0..count as usize {
        let subpass: usize = rd(storage, index * 4);
        if subpass != 0 {
            subpasses[subpass_count] = subpass;
            subpass_count += 1;
        }
    }

    let mut model = Model::new(max_stages, max_textures, base_texcoord_patch);
    build(&mut model, this, property_state, effect_state, texture_sets);
    model.roll_back();
    if model.skip {
        set.skip();
        return;
    }
    if model.allocates {
        set.allocates();
    }
    let mut blocks = [(0usize, 0usize); 6];
    match blocks_of(&model, this, pipeline, &subpasses[..subpass_count], &mut blocks) {
        Some(len) => {
            for &(start, size) in &blocks[..len] {
                set.add(start, size);
            }
        }
        None => {
            // More blocks than `Regions` holds.
            set.skip();
            return;
        }
    }
    // Nothing the function calls writes the render state. It is compared when a block is
    // free for it.
    set.add(render_state, RS::SIZE);
}

/// The memory a call writes: the pass, the pipeline (`field_B04`), every sub-pass of the
/// array (all are emptied at the start), the slot of the array a new sub-pass goes into,
/// the reference counts of the textures that enter or leave a stage and of the alpha
/// templates that enter or leave a sub-pass, and the alpha test value of alpha template 4.
/// Which of these, a dry run of the body tells (see `Model`).
///
/// The call is not compared when the dry run finds that an object would be deleted (a
/// texture or an alpha property that loses its last reference), that a device texture would
/// be made (an effect texture with no device data), an effect state with projected lights
/// or shadows that is not the one of the previous call, a sub-pass array that is not as the
/// game leaves it, or more than 8 blocks. A new sub-pass or a longer array is
/// `set.allocates()`.
#[cfg_attr(test, allow(dead_code))]
unsafe fn regions(set: &mut Regions, this: usize, property_state: usize, effect_state: usize, texture_sets: u32) {
    let patch = rd::<u8>(BASE_TEXCOORD_PATCH, 0) == 0xE8;
    plan(set, rd(MAX_STAGES, 0), rd(MAX_TEXTURES, 0), patch, this, property_state, effect_state, texture_sets, rd(this, TP::renderState));
}

port_thiscall!(set_ct_pipeline_state_port, 0x6B41B0, "NiDX8TexturePass::setCTPipelineState",
    fn(this, property_state: usize, effect_state: usize, has_normals: u32, texture_sets: u32,
       geometry_data: usize, skin_instance: usize, world_transform: usize, world_bound: usize) -> u32,
    compare_eax = true,
    regions = |set| regions(set, this, property_state, effect_state, texture_sets),
    rust = set_ct_pipeline_state);

pub static PORTS: &[&crate::shadow::PortState] = &[&set_ct_pipeline_state_port::STATE];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::wr;
    use crate::{census, hook};

    #[test]
    fn sites_are_the_calls_of_the_function() {
        let func = hook::find(0x6B41B0).unwrap();
        let code = census::expected(func);
        // The disassembly of the database has 142 `call rel32` instructions.
        assert_eq!(SITES.len(), 142);
        for pair in SITES.windows(2) {
            assert!(pair[0] + 5 <= pair[1], "{:#x} and {:#x}", pair[0], pair[1]);
        }
        for &site in SITES {
            assert!(site >= func.ea && site + 5 <= func.ea + func.size, "{site:#x} is outside the function");
            assert_eq!(code[site - func.ea], 0xE8, "{site:#x} is not a call");
        }
        assert!(!SITES.contains(&BASE_TEXCOORD_PATCH));
    }

    #[test]
    fn the_body_makes_every_call_of_the_list() {
        let source = include_str!("ct_pipeline_state.rs");
        let start = source.find("unsafe fn build<E: Env>").unwrap();
        let end = source.find("// The dry run").unwrap();
        let body = &source[start..end];
        for &site in SITES {
            assert!(body.contains(&format!("0x{site:X}")), "{site:#x} is not called");
        }
    }

    /// A pass with its pipeline, the ten alpha templates, a texturing property with seven
    /// map slots and an array of sub-passes.
    struct World {
        pass: Vec<u32>,
        pipeline: Vec<u32>,
        templates: Vec<Box<[u32; 8]>>,
        storage: Box<[usize]>,
        subpasses: Vec<Box<[u32; 21]>>,
        property: Vec<u32>,
        map_slots: Box<[usize; 8]>,
        maps: Vec<Box<[u32; 12]>>,
        textures: Vec<Box<[u32; 16]>>,
        state: Vec<u32>,
        alpha: Vec<u32>,
    }

    impl World {
        fn new(subpass_count: usize) -> World {
            let mut world = World {
                pass: vec![0; TP::SIZE / 4],
                pipeline: vec![0; CTP::SIZE / 4],
                templates: (0..10).map(|_| Box::new([0u32; 8])).collect(),
                storage: vec![0usize; subpass_count].into_boxed_slice(),
                subpasses: (0..subpass_count).map(|_| Box::new([0u32; 21])).collect(),
                property: vec![0; 16],
                map_slots: Box::new([0; 8]),
                maps: Vec::new(),
                textures: Vec::new(),
                state: vec![0; 16],
                alpha: vec![0; 8],
            };
            unsafe {
                let (this, pipeline) = (world.this(), world.pipeline());
                wr(this, TP::configurableTexturePipeline, pipeline);
                for index in 0..10 {
                    let template = world.template(index);
                    wr::<u32>(template, REF_COUNT, 1);
                    wr(pipeline, CTP::field_2E8 + index * 4, template);
                }
                for index in 0..subpass_count {
                    let subpass = world.subpasses[index].as_mut_ptr() as usize;
                    wr::<u32>(subpass, SUB::field_C, u32::MAX);
                    world.storage[index] = subpass;
                }
                wr(this, TP::TArray_18_storage, world.storage.as_mut_ptr() as usize);
                wr::<u32>(this, TP::TArray_18_storageCount, subpass_count as u32);
                wr::<u32>(this, TP::TArray_18_endIndex, subpass_count as u32);
                wr::<u32>(this, TP::TArray_18_filledCount, subpass_count as u32);
                wr::<u32>(this, TP::count_10, subpass_count as u32);
                let property = world.property.as_mut_ptr() as usize;
                wr(property, NTP::maps_storage, world.map_slots.as_mut_ptr() as usize);
                wr::<u32>(property, NTP::maps_endIndex, 7);
                wr::<u32>(property, NTP::applyMode, 2);
                let state = world.state.as_mut_ptr() as usize;
                wr(state, PS::propTexturing, property);
                wr(state, PS::propAlpha, world.alpha.as_mut_ptr() as usize);
            }
            world
        }
        fn this(&mut self) -> usize {
            self.pass.as_mut_ptr() as usize
        }
        fn pipeline(&mut self) -> usize {
            self.pipeline.as_mut_ptr() as usize
        }
        fn template(&mut self, index: usize) -> usize {
            self.templates[index].as_mut_ptr() as usize
        }
        fn subpass(&mut self, index: usize) -> usize {
            self.subpasses[index].as_mut_ptr() as usize
        }
        fn stage(&mut self, index: usize) -> usize {
            self.this() + TP::stage + index * STAGE
        }
        fn texture(&mut self) -> usize {
            let mut texture = Box::new([0u32; 16]);
            texture[REF_COUNT / 4] = 1;
            let addr = texture.as_mut_ptr() as usize;
            self.textures.push(texture);
            addr
        }
        /// Puts a map with a new texture into a slot of the property. Returns the texture.
        fn map(&mut self, slot: usize, coordinate_set: u32) -> usize {
            let texture = self.texture();
            let mut map = Box::new([0u32; 12]);
            let addr = map.as_mut_ptr() as usize;
            unsafe {
                wr(addr, MAP::texture, texture);
                wr::<u32>(addr, MAP::clampMode, 3);
                wr::<u32>(addr, MAP::filterMode, 2);
                wr::<u32>(addr, MAP::texCoordSet, coordinate_set);
            }
            self.maps.push(map);
            self.map_slots[slot] = addr;
            texture
        }
        fn run(&mut self, model: &mut Model, texture_sets: u32) -> u32 {
            let (this, state) = (self.this(), self.state.as_mut_ptr() as usize);
            unsafe { build(model, this, state, 0, texture_sets) }
        }
        /// Every byte of the world the game would know.
        fn snapshot(&self) -> Vec<u32> {
            let mut all = self.pass.clone();
            all.extend_from_slice(&self.pipeline);
            self.templates.iter().for_each(|block| all.extend_from_slice(&block[..]));
            all.extend(self.storage.iter().map(|&slot| slot as u32));
            self.subpasses.iter().for_each(|block| all.extend_from_slice(&block[..]));
            self.textures.iter().for_each(|block| all.extend_from_slice(&block[..]));
            all
        }
    }

    #[test]
    fn one_map_takes_the_short_way() {
        let mut world = World::new(1);
        let texture = world.map(0, 1);
        let (this, pipeline, stage, subpass) = (world.this(), world.pipeline(), world.stage(0), world.subpass(0));
        unsafe {
            // The colour operation of blend slot 2 for a first stage.
            wr::<u32>(pipeline, (2 * 3 + 0x21) * 8, 4);
            // The sub-pass is as an earlier draw left it.
            wr::<u32>(subpass, SUB::field_8, 3);
            wr::<u32>(subpass, SUB::field_C, 3);
            wr::<u32>(subpass, SUB::field_10, 3);
            let mut model = Model::new(8, 8, false);
            assert_eq!(world.run(&mut model, 2), 0);
            assert!(!model.skip && !model.allocates);
            assert_eq!(rd::<u32>(this, TP::activeStageCount), 1);
            assert_eq!(rd::<u32>(this, TP::field_C), 0);
            assert_eq!(rd::<u32>(this, TP::currentTextureApplyMode), 2);
            assert_eq!(rd::<usize>(stage, ST::ptrTexture), texture);
            assert_eq!(rd::<u32>(texture, REF_COUNT), 2);
            assert_eq!(rd::<u32>(stage, ST::field_94), 2);
            assert_eq!(rd::<u8>(stage, ST::field_90), 1);
            assert_eq!(rd::<usize>(stage, ST::texturePropMap), texture);
            // The database stores: set 0, whatever the map asks for.
            assert_eq!(rd::<u32>(stage, ST::sourceTexcoordIndex), 0);
            assert_eq!(rd::<u32>(stage, ST::colorOp), 4);
            assert_eq!(rd::<u32>(subpass, SUB::field_8), 1);
            assert_eq!(rd::<u32>(subpass, SUB::field_C), 1);
            assert_eq!(rd::<u32>(subpass, SUB::field_10), 1);
            assert_eq!(rd::<usize>(subpass, SUB::field_14), stage);
        }
    }

    #[test]
    fn no_base_texture_binds_the_stage_without_texture() {
        let mut world = World::new(1);
        let (this, subpass) = (world.this(), world.subpass(0));
        let mut model = Model::new(8, 8, false);
        assert_eq!(world.run(&mut model, 1), 0);
        unsafe {
            assert_eq!(rd::<u32>(this, TP::activeStageCount), 0);
            assert_eq!(rd::<usize>(subpass, SUB::field_14), this + TP::field_624);
            assert_eq!(rd::<u32>(subpass, SUB::field_10), 0);
        }
    }

    #[test]
    fn base_texcoord_patch_clamps_the_set_of_the_base_map() {
        let mut world = World::new(1);
        world.map(0, 3);
        let property = world.property.as_mut_ptr() as usize;
        unsafe {
            assert_eq!(base_map_coordinate_set(property, 0), 0);
            assert_eq!(base_map_coordinate_set(property, 2), 1);
            assert_eq!(base_map_coordinate_set(property, 3), 2);
            assert_eq!(base_map_coordinate_set(property, 4), 3);
            assert_eq!(base_map_coordinate_set(property, 9), 3);
        }
        // Through the body, with the call of MWSE at 0x6B42AC and without it.
        let stage = world.stage(0);
        for (patch, expected) in [(true, 1), (false, 0)] {
            let mut model = Model::new(8, 8, patch);
            world.run(&mut model, 2);
            unsafe {
                assert_eq!(rd::<u32>(stage, ST::sourceTexcoordIndex), expected);
                assert_eq!(rd::<u32>(stage, ST::textureTransformFlags), 0);
                model.roll_back();
            }
        }
        // No base map: set 0.
        world.map_slots[0] = 0;
        unsafe { assert_eq!(base_map_coordinate_set(property, 2), 0) };
    }

    #[test]
    fn a_glow_map_shares_the_pass_when_the_device_has_room() {
        let mut world = World::new(1);
        let base = world.map(0, 0);
        let glow = world.map(4, 5);
        let (this, subpass) = (world.this(), world.subpass(0));
        let (base_stage, glow_stage) = (world.stage(0), world.stage(1));
        let mut model = Model::new(8, 8, false);
        assert_eq!(world.run(&mut model, 2), 0);
        unsafe {
            assert!(!model.skip && !model.allocates);
            assert_eq!(rd::<u32>(this, TP::activeStageCount), 2);
            assert_eq!(rd::<u32>(this, TP::field_C), 0);
            assert_eq!(rd::<usize>(base_stage, ST::ptrTexture), base);
            assert_eq!(rd::<usize>(glow_stage, ST::ptrTexture), glow);
            assert_eq!(rd::<u32>(glow_stage, ST::field_94), 0);
            // The map asks for set 5 of 2: the last one.
            assert_eq!(rd::<u32>(glow_stage, ST::sourceTexcoordIndex), 1);
            assert_eq!(rd::<[usize; 2]>(subpass, SUB::field_14), [base_stage, glow_stage]);
            assert_eq!(rd::<u32>(subpass, SUB::field_8), 2);
            assert_eq!(rd::<usize>(subpass, SUB::field_48), 0);
        }
    }

    #[test]
    fn a_device_with_one_stage_gets_a_pass_for_each_map_and_the_dry_run_is_undone() {
        let mut world = World::new(1);
        let base = world.map(0, 0);
        let glow = world.map(4, 0);
        let (this, pipeline, subpass) = (world.this(), world.pipeline(), world.subpass(0));
        let (base_stage, glow_stage) = (world.stage(0), world.stage(1));
        let (template_0, template_2) = (world.template(0), world.template(2));
        let before = world.snapshot();
        let mut model = Model::new(1, 1, false);
        let result = world.run(&mut model, 1);
        unsafe {
            assert!(!model.skip && model.allocates);
            let passes: u32 = rd(this, TP::field_C);
            assert_eq!(rd::<u32>(this, TP::field_34), passes);
            assert_eq!(result, if passes > 1 { 0x8000_0000 } else { 0 });
            let storage: usize = rd(this, TP::TArray_18_storage);
            let base_pass: usize = rd(storage, (passes as usize - 1) * 4);
            let glow_pass: usize = rd(storage, passes as usize * 4);
            assert_eq!(rd::<usize>(base_pass, SUB::field_14), base_stage);
            assert_eq!(rd::<usize>(glow_pass, SUB::field_14), glow_stage);
            assert_eq!(rd::<usize>(glow_pass, SUB::field_48), template_0);
            assert_eq!(rd::<u32>(template_0, REF_COUNT), 2);
            let base_linked = rd::<usize>(base_pass, SUB::field_48) == template_2;
            assert_eq!(base_linked, passes > 1);

            model.roll_back();
            assert!(world.snapshot() == before, "the dry run left a change");

            // The blocks outside the pass and the pipeline: the sub-pass of the array and
            // the reference counts that change.
            let mut blocks = [(0usize, 0usize); 6];
            let len = blocks_of(&model, this, pipeline, &[subpass], &mut blocks).unwrap();
            let mut found = blocks[..len].to_vec();
            found.sort();
            let mut expected = vec![(subpass, SUB::SIZE), (base + REF_COUNT, 4), (glow + REF_COUNT, 4), (template_0 + REF_COUNT, 4)];
            if base_linked {
                expected.push((template_2 + REF_COUNT, 4));
            }
            expected.sort();
            assert_eq!(found, expected);

            let mut set = Regions::new();
            let state = world.state.as_mut_ptr() as usize;
            plan(&mut set, 1, 1, false, this, state, 0, 1, 0);
            assert!(world.snapshot() == before, "the plan left a change");
            assert_eq!(set.as_slice().len(), 2 + expected.len());
            assert_eq!(set.as_slice()[0], (this, TP::SIZE));
            assert_eq!(set.as_slice()[1], (pipeline, CTP::SIZE));
        }
    }

    #[test]
    fn the_dark_map_releases_the_texture_the_stage_had() {
        let mut world = World::new(1);
        world.map(0, 0);
        let dark = world.map(1, 0);
        let old = world.texture();
        let (subpass, dark_stage) = (world.subpass(0), world.stage(1));
        unsafe {
            wr(dark_stage, ST::ptrTexture, old);
            wr::<u32>(old, REF_COUNT, 2);
            let mut model = Model::new(8, 8, false);
            assert_eq!(world.run(&mut model, 1), 0);
            assert!(!model.skip);
            assert_eq!(rd::<usize>(dark_stage, ST::ptrTexture), dark);
            assert_eq!(rd::<u32>(dark, REF_COUNT), 2);
            assert_eq!(rd::<u32>(old, REF_COUNT), 1);
            assert_eq!(rd::<u32>(dark_stage, ST::field_94), 2);
            assert_eq!(rd::<usize>(subpass, SUB::field_14 + 4), dark_stage);
            model.roll_back();
            assert_eq!(rd::<usize>(dark_stage, ST::ptrTexture), old);

            // The last reference: the game would delete the texture, so the call is not compared.
            wr::<u32>(old, REF_COUNT, 1);
            let mut model = Model::new(8, 8, false);
            world.run(&mut model, 1);
            assert!(model.skip);
            model.roll_back();
            assert_eq!(rd::<u32>(old, REF_COUNT), 1);
        }
    }

    #[test]
    fn the_environment_stage_follows_the_base_stage() {
        let mut world = World::new(1);
        world.map(0, 0);
        let (this, subpass, base_stage) = (world.this(), world.subpass(0), world.stage(0));
        unsafe {
            // As `setTexturePipeline` leaves a pass with an environment map.
            wr::<u32>(this, TP::field_6E0, 1);
            wr::<u8>(this, TP::field_2B08, 1);
            let mut model = Model::new(8, 8, false);
            assert_eq!(world.run(&mut model, 1), 0);
            assert!(!model.skip && !model.allocates);
            assert_eq!(rd::<[usize; 2]>(subpass, SUB::field_14), [base_stage, this + TP::field_2A4C]);
            assert_eq!(rd::<u32>(this, TP::field_C), 0);
        }
    }
}
