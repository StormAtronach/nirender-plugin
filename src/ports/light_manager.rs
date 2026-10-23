//! `NiDX8LightManager`: one `D3DLIGHT8` for each `NiLight`, the list of the lights that are on in
//! the device, and the lighting render states of a draw.

use core::arch::asm;

use crate::d3d::{
    device_add_ref, device_light_enable, device_release, device_set_light, D3DRS_AMBIENT, D3DRS_AMBIENTMATERIALSOURCE,
    D3DRS_DIFFUSEMATERIALSOURCE, D3DRS_EMISSIVEMATERIALSOURCE, D3DRS_LIGHTING,
};
use crate::engine::{device_set_render_state, vcall0, VTBL_NI_REF_OBJECT};
use crate::gen::layouts::{
    NiDX8LightManager as M, NiDX8LightManager_LightEntry as E, NiDX8RenderState as RS, NiDynamicEffectState as DES,
    NiDynamicEffectState_InternalList as DL, NiLight as L, NiTexturingProperty as TP, NiTexturingProperty_Map as TM,
    NiVertexColorProperty as VC, THashMap as HM, THashMap_Node as HN,
};
use crate::mem::{rd, wr, x87_pass};
use crate::ports::hash_map;
use crate::ports::render_state::{set_if_changed, set_render_state, setting_set_value};
use crate::shadow::Regions;

// The members the subclasses of `NiLight` add. They are not in `gen::layouts`.
/// `NiDirectionalLight`: world direction, 3 floats (0x6BACCE).
const DIRECTIONAL_DIRECTION: usize = 0xD0;
/// `NiPointLight` and `NiSpotLight`: constant, linear and quadratic attenuation (0x6BABAA).
const POINT_ATTENUATION: usize = 0xD0;
/// `NiSpotLight`: world direction, 3 floats (0x6BAB6E).
const SPOT_DIRECTION: usize = 0xDC;
/// `NiSpotLight`: cone angle in degrees (0x6BAB89).
const SPOT_ANGLE: usize = 0xE8;
/// `NiSpotLight`: exponent (0x6BABA1).
const SPOT_EXPONENT: usize = 0xEC;
/// Slot of `NiLight::GetType` in the vtable of a light (0x6BAB3E, 0x6BB801).
const LIGHT_VTBL_GET_TYPE: usize = 0x94;

// `TListItem`: data, next, previous.
const ITEM_DATA: usize = 0x0;
const ITEM_NEXT: usize = 0x4;
const ITEM_PREVIOUS: usize = 0x8;
const ITEM_SIZE: usize = 0xC;

// `NiTListAllocator::Data`: the next block and its array of items.
const BLOCK_NEXT: usize = 0x0;
const BLOCK_ITEMS: usize = 0x4;

/// 0x74F2EC: the game does not use a light with a dimmer below this value.
pub(crate) const MIN_DIMMER: u32 = 0x3C23_D70A; // 0.01
/// 0x74C08C: 1/255.
const INV_255: u32 = 0x3B80_8081;
/// 0x74F6E0: 1/90. The game code multiplies the cone angle in degrees by pi and by this value.
/// The result is two times the angle in radians: 90 degrees give pi.
const INV_90: u32 = 0x3C36_0B61;

const VTBL_LIGHT_MANAGER: u32 = 0x74F6E4;
const VTBL_HASH_MAP_LIGHT: u32 = 0x74F6E8;

// The names the game code passes to `NiCriticalSection::Lock`. The function does not read them.
const NAME_ENTRY_CTOR: usize = 0x7C3DA4;
const NAME_ENTRY_UPDATE: usize = 0x7C3DD0;
const NAME_NEXT_FRAME: usize = 0x7C3DF8;
const NAME_SET_STATE: usize = 0x7C3E18;
const NAME_REMOVE_LIGHT: usize = 0x7C3E34;
const NAME_UPDATE_LIGHTS: usize = 0x7C3E54;
const NAME_RELEASE_LIGHTS: usize = 0x7C3E74;
/// The critical section of `LightEntry` (`stru_7DD9E4`).
const ENTRY_CRITICAL_SECTION: usize = 0x7DD9E4;

const _: () = assert!(RS::renderStateSettings + D3DRS_AMBIENT as usize * 8 == 0x650);
const _: () = assert!(E::SIZE == 0x70 && E::revisionId == 0x68 && E::lightIndex == 0x6C);

// ---- engine functions and data at fixed addresses

#[cfg(not(test))]
mod eng {
    use crate::engine::thiscall1;
    use crate::mem::rd;

    pub use crate::engine::{
        critical_section_ctor, critical_section_dtor, critical_section_lock, critical_section_unlock, list_allocator_block_ctor,
        operator_delete, operator_new, texture_manager_clear_hash_map,
    };
    pub use crate::ports::ni::ref_object_count_add;

    /// 0x6ECED0 `NiPackedColor::setRed`. `value` is the bits of the float argument.
    pub unsafe fn packed_color_set_red(color: usize, value: u32) {
        thiscall1(0x6ECED0, color, value);
    }
    /// 0x6ECF20 `NiPackedColor::setGreen`
    pub unsafe fn packed_color_set_green(color: usize, value: u32) {
        thiscall1(0x6ECF20, color, value);
    }
    /// 0x6ECF60 `NiPackedColor::setBlue`
    pub unsafe fn packed_color_set_blue(color: usize, value: u32) {
        thiscall1(0x6ECF60, color, value);
    }
    /// 0x6BBEA0 `NiTListBase::deallocItem`
    pub unsafe fn list_dealloc_item(list: usize, item: usize) {
        thiscall1(0x6BBEA0, list, item as u32);
    }
    /// 0x6BBEC0 `sub_6BBEC0`: the deleting destructor of `NiTListAllocator::Data`.
    pub unsafe fn allocator_block_deleting_dtor(block: usize, flags: u32) {
        thiscall1(0x6BBEC0, block, flags);
    }
    /// `f_pi_static` (0x7DEDA4). The game sets it when the game starts.
    pub unsafe fn pi() -> u32 {
        rd(0x7DEDA4, 0)
    }
    /// `dword_7DDA0C`: the range that each point light and each spot light gets.
    pub unsafe fn light_range() -> u32 {
        rd(0x7DDA0C, 0)
    }
}

/// Replacements for the unit tests. The unit tests run without the game. The functions that
/// `engine` and `ports::ni` have in the other `eng` have the same names here.
#[cfg(test)]
mod eng {
    use crate::mem::{rd, wr};

    pub unsafe fn critical_section_ctor(_cs: usize) {}
    pub unsafe fn critical_section_dtor(_cs: usize) {}
    pub unsafe fn critical_section_lock(_cs: usize, _name: usize) {}
    pub unsafe fn critical_section_unlock(_cs: usize) {}
    pub unsafe fn operator_new(size: usize) -> usize {
        Box::leak(vec![0xCDu8; size].into_boxed_slice()).as_mut_ptr() as usize
    }
    pub unsafe fn operator_delete(_block: usize) {}
    fn channel(value: u32) -> u32 {
        (f32::from_bits(value) * 255.0).round_ties_even() as i32 as u32
    }
    pub unsafe fn packed_color_set_red(color: usize, value: u32) {
        let now: u32 = rd(color, 0);
        wr(color, 0, ((((now >> 24) << 8) | channel(value)) << 16) | (now & 0xFFFF));
    }
    pub unsafe fn packed_color_set_green(color: usize, value: u32) {
        let now: u32 = rd(color, 0);
        wr(color, 0, (((now >> 16) << 8 | channel(value)) << 8) | (now & 0xFF));
    }
    pub unsafe fn packed_color_set_blue(color: usize, value: u32) {
        let now: u32 = rd(color, 0);
        wr(color, 0, ((now >> 8) << 8) | channel(value));
    }
    pub unsafe fn list_dealloc_item(list: usize, item: usize) {
        wr::<u32>(item, 0, 0);
        wr::<usize>(item, 4, rd(list, 0xC));
        wr(list, 0xC, item);
    }
    pub unsafe fn list_allocator_block_ctor(block: usize, count: u32) -> usize {
        let items = operator_new(count as usize * 12);
        wr::<u32>(block, 0, 0);
        wr(block, 4, items);
        for i in 0..count as usize {
            wr::<usize>(items, i * 12 + 4, if i + 1 < count as usize { items + (i + 1) * 12 } else { 0 });
        }
        block
    }
    pub unsafe fn allocator_block_deleting_dtor(_block: usize, _flags: u32) {}
    pub unsafe fn texture_manager_clear_hash_map(_map: usize) {}
    pub unsafe fn pi() -> u32 {
        core::f32::consts::PI.to_bits()
    }
    pub const TEST_RANGE: u32 = 0x4479_C000; // 999.0
    pub unsafe fn light_range() -> u32 {
        TEST_RANGE
    }
    pub unsafe fn ref_object_count_add(_delta: i32) {}
}

// ---- calls through a vtable

/// `NiLight::GetType`: 0 ambient, 1 directional, 2 point, 3 spot.
#[inline(always)]
pub(crate) unsafe fn light_get_type(light: usize) -> u32 {
    vcall0(light, LIGHT_VTBL_GET_TYPE)
}

// ---- x87 sequences

/// `fld a; fmul b; fstp`: the bits of the product.
fn x87_mul(a: u32, b: u32) -> u32 {
    let input = [a, b];
    let mut output = 0u32;
    unsafe {
        asm!(
            "fld dword ptr [{i}]",
            "fmul dword ptr [{i} + 4]",
            "fstp dword ptr [{o}]",
            i = in(reg) input.as_ptr(),
            o = in(reg) &mut output,
            clobber_abi("C"),
            options(nostack),
        );
    }
    output
}

/// 0x6BAB89: `fld angle; fmul pi; fmul 1/90; fstp`. The first product stays in the register.
fn x87_spot_angle(angle: u32, pi: u32) -> u32 {
    let input = [angle, pi, INV_90];
    let mut output = 0u32;
    unsafe {
        asm!(
            "fld dword ptr [{i}]",
            "fmul dword ptr [{i} + 4]",
            "fmul dword ptr [{i} + 8]",
            "fstp dword ptr [{o}]",
            i = in(reg) input.as_ptr(),
            o = in(reg) &mut output,
            clobber_abi("C"),
            options(nostack),
        );
    }
    output
}

/// 0x6BB82B: `fild channel; fmul 1/255; fld dimmer; fmul colour; faddp; fstp`.
fn x87_ambient_channel(channel: u32, dimmer: u32, colour: u32) -> u32 {
    let input = [channel, INV_255, dimmer, colour];
    let mut output = 0u32;
    unsafe {
        asm!(
            "fild dword ptr [{i}]",
            "fmul dword ptr [{i} + 4]",
            "fld dword ptr [{i} + 8]",
            "fmul dword ptr [{i} + 12]",
            "faddp st(1), st",
            "fstp dword ptr [{o}]",
            i = in(reg) input.as_ptr(),
            o = in(reg) &mut output,
            clobber_abi("C"),
            options(nostack),
        );
    }
    output
}

// ---- shared pieces the game code has inline

/// Disables each light of the list in the device and puts its list item on the free list.
#[inline(always)]
unsafe fn disable_device_lights(this: usize) {
    while rd::<u32>(this, M::deviceLights_count) != 0 {
        let item: usize = rd(this, M::deviceLights_head);
        let next: usize = rd(item, ITEM_NEXT);
        wr(this, M::deviceLights_head, next);
        if next != 0 {
            wr::<u32>(next, ITEM_PREVIOUS, 0);
        } else {
            wr::<u32>(this, M::deviceLights_tail, 0);
        }
        let light: usize = rd(item, ITEM_DATA);
        wr::<u32>(item, ITEM_DATA, 0);
        wr::<usize>(item, ITEM_NEXT, rd(this, M::deviceLights_allocator_freeList));
        wr(this, M::deviceLights_allocator_freeList, item);
        wr(this, M::deviceLights_count, rd::<u32>(this, M::deviceLights_count).wrapping_sub(1));
        device_light_enable(rd(this, M::device), rd(light, L::index), 0);
    }
}

// ---- the ports

/// 0x6BAB00 `NiDX8LightManager::LightEntry::Update`. Returns 1 when the port built the entry again.
pub unsafe fn light_entry_update(entry: usize, light: usize) -> u8 {
    crate::shadow::touch(&light_entry_update_port::STATE);
    eng::critical_section_lock(ENTRY_CRITICAL_SECTION, NAME_ENTRY_UPDATE);
    let revision: u32 = rd(light, L::revisionID);
    if rd::<u32>(entry, E::revisionId) == revision {
        eng::critical_section_unlock(ENTRY_CRITICAL_SECTION);
        return 0;
    }
    wr(entry, E::revisionId, revision);
    write_d3d_light(entry, light);
    eng::critical_section_unlock(ENTRY_CRITICAL_SECTION);
    1
}

/// The `D3DLIGHT8` of a light as the light manager gives it to the device: the 26 words.
/// For a light of the world that no draw has (`world_lights`).
pub unsafe fn d3d_light_of(light: usize) -> [u32; 26] {
    let mut entry = [0u32; E::SIZE / 4];
    write_d3d_light(entry.as_mut_ptr() as usize, light);
    rd(entry.as_ptr() as usize, E::d3dLight)
}

/// The `D3DLIGHT8` part of an entry, from the light.
unsafe fn write_d3d_light(entry: usize, light: usize) {
    core::ptr::write_bytes(entry as *mut u8, 0, E::revisionId);

    let copy3 = |to: usize, from: usize| {
        wr::<u32>(entry, to, rd(light, from));
        wr::<u32>(entry, to + 4, rd(light, from + 4));
        wr::<u32>(entry, to + 8, rd(light, from + 8));
    };
    let attenuation_and_range = || {
        wr::<u32>(entry, E::d3dLight_Attenuation0, x87_pass(rd(light, POINT_ATTENUATION)));
        wr::<u32>(entry, E::d3dLight_Attenuation1, x87_pass(rd(light, POINT_ATTENUATION + 4)));
        wr::<u32>(entry, E::d3dLight_Attenuation2, x87_pass(rd(light, POINT_ATTENUATION + 8)));
        wr::<u32>(entry, E::d3dLight_Range, eng::light_range());
    };
    match light_get_type(light) {
        1 => {
            wr::<u32>(entry, E::d3dLight_Type, 3); // D3DLIGHT_DIRECTIONAL
            copy3(E::d3dLight_Direction, DIRECTIONAL_DIRECTION);
        }
        2 => {
            wr::<u32>(entry, E::d3dLight_Type, 1); // D3DLIGHT_POINT
            copy3(E::d3dLight_Position, L::worldTransform_translation);
            attenuation_and_range();
        }
        3 => {
            wr::<u32>(entry, E::d3dLight_Type, 2); // D3DLIGHT_SPOT
            copy3(E::d3dLight_Position, L::worldTransform_translation);
            copy3(E::d3dLight_Direction, SPOT_DIRECTION);
            let angle = x87_spot_angle(rd(light, SPOT_ANGLE), eng::pi());
            wr::<u32>(entry, E::d3dLight_Theta, angle);
            wr::<u32>(entry, E::d3dLight_Phi, angle);
            wr::<u32>(entry, E::d3dLight_Falloff, x87_pass(rd(light, SPOT_EXPONENT)));
            attenuation_and_range();
        }
        _ => {}
    }

    // The port multiplies each colour by the dimmer. Alpha stays 0.
    let dimmer: u32 = rd(light, L::dimmer);
    let scaled = |from: usize| x87_mul(rd(light, from), dimmer);
    let ambient_r = scaled(L::ambientColor_r);
    let ambient_g = scaled(L::ambientColor_g);
    wr::<u32>(entry, E::d3dLight_Ambient + 8, scaled(L::ambientColor_b));
    wr::<u32>(entry, E::d3dLight_Diffuse_r, scaled(L::diffuseColor_r));
    wr::<u32>(entry, E::d3dLight_Diffuse_g, scaled(L::diffuseColor_g));
    wr::<u32>(entry, E::d3dLight_Diffuse_b, scaled(L::diffuseColor_b));
    wr::<u32>(entry, E::d3dLight_Specular, scaled(L::specularColor_r));
    wr::<u32>(entry, E::d3dLight_Specular + 4, scaled(L::specularColor_g));
    let specular_b = scaled(L::specularColor_b);
    wr::<u32>(entry, E::d3dLight_Ambient, ambient_r);
    wr::<u32>(entry, E::d3dLight_Specular + 8, specular_b);
    wr::<u32>(entry, E::d3dLight_Ambient + 4, ambient_g);
}

/// 0x6BAEC0 `NiDX8LightManager::resetRenderStates`
pub unsafe fn reset_render_states(this: usize) {
    crate::shadow::touch(&reset_render_states_port::STATE);
    wr::<u32>(this, M::ambientLightingState, 0);
    set_if_changed(rd(this, M::renderState), D3DRS_AMBIENT, 0);
    set_if_changed(rd(this, M::renderState), D3DRS_LIGHTING, 0);
    wr::<u32>(this, M::applyMode, 0);
    wr::<u8>(this, M::d3dLightingState, 0);
    wr::<u8>(this, M::useAmbientDiffuse, 0);
    wr::<u32>(this, M::sourceVertexMode, 0);
    set_if_changed(rd(this, M::renderState), D3DRS_EMISSIVEMATERIALSOURCE, 0);
    set_if_changed(rd(this, M::renderState), D3DRS_AMBIENTMATERIALSOURCE, 0);
    set_if_changed(rd(this, M::renderState), D3DRS_DIFFUSEMATERIALSOURCE, 0);
    wr::<u32>(this, M::lastState, 0);
    wr::<u8>(this, M::lastUseAmbientDiffuse, 0);
}

/// 0x6BAD10 `NiDX8LightManager::ctor`. Returns `this`.
pub unsafe fn ctor(this: usize, render_state: usize, device: usize) -> usize {
    crate::shadow::touch(&ctor_port::STATE);
    wr(this, M::vtbl, VTBL_NI_REF_OBJECT);
    wr::<u32>(this, M::refCount, 0);
    eng::ref_object_count_add(1);
    wr(this, M::lightEntries_vtbl, VTBL_HASH_MAP_LIGHT);
    wr::<u32>(this, M::lightEntries_bucketCount, 37);
    wr::<u32>(this, M::lightEntries_count, 0);
    let buckets = eng::operator_new(0x94);
    let bytes = rd::<u32>(this, M::lightEntries_bucketCount).wrapping_shl(2);
    wr(this, M::lightEntries_arrayBuckets, buckets);
    core::ptr::write_bytes(buckets as *mut u8, 0, bytes as usize);
    wr::<u32>(this, M::deviceLights_count, 0);
    wr::<u32>(this, M::deviceLights_head, 0);
    wr::<u32>(this, M::deviceLights_tail, 0);
    wr::<u32>(this, M::deviceLights_allocator_freeList, 0);
    wr::<u32>(this, M::deviceLights_allocator_allocator, 0);
    wr::<u32>(this, M::deviceLights_allocator_count, 2);
    eng::critical_section_ctor(this + M::criticalSection);
    wr(this, M::vtbl, VTBL_LIGHT_MANAGER);
    wr(this, M::renderState, render_state);
    wr(this, M::device, device);
    device_add_ref(device);
    // The remaining code is the body of `resetRenderStates`. The game code has it inline.
    reset_render_states(this);
    this
}

/// 0x6BAF90 `NiDX8LightManager::dtor`
pub unsafe fn dtor(this: usize) {
    crate::shadow::touch(&dtor_port::STATE);
    wr(this, M::vtbl, VTBL_LIGHT_MANAGER);
    let map = this + M::lightEntries;
    while rd::<u32>(map, HM::count) != 0 {
        let first = hash_map::first(map);
        let entry: usize = rd(first, HN::data);
        let key: u32 = rd(first, HN::key);
        if rd::<usize>(first, HN::nextNode) == 0 {
            // The iterator moves to the next bucket. No code reads the result.
            hash_map::next_bucket(map, key);
        }
        hash_map::remove_with(map, key, |node| eng::operator_delete(node));
        eng::operator_delete(entry);
    }

    while rd::<u32>(this, M::deviceLights_count) != 0 {
        let item: usize = rd(this, M::deviceLights_head);
        let next: usize = rd(item, ITEM_NEXT);
        wr(this, M::deviceLights_head, next);
        if next != 0 {
            wr::<u32>(next, ITEM_PREVIOUS, 0);
        } else {
            wr::<u32>(this, M::deviceLights_tail, 0);
        }
        wr::<u32>(item, ITEM_DATA, 0);
        wr::<usize>(item, ITEM_NEXT, rd(this, M::deviceLights_allocator_freeList));
        let count = rd::<u32>(this, M::deviceLights_count).wrapping_sub(1);
        wr(this, M::deviceLights_allocator_freeList, item);
        wr(this, M::deviceLights_count, count);
    }

    let device: usize = rd(this, M::device);
    if device != 0 {
        device_release(device);
        wr::<u32>(this, M::device, 0);
    }
    eng::critical_section_dtor(this + M::criticalSection);
    let mut block: usize = rd(this, M::deviceLights_allocator_allocator);
    while block != 0 {
        let next: usize = rd(block, BLOCK_NEXT);
        eng::allocator_block_deleting_dtor(block, 1);
        block = next;
    }
    wr::<u32>(this, M::deviceLights_count, 0);
    wr::<u32>(this, M::deviceLights_head, 0);
    wr::<u32>(this, M::deviceLights_tail, 0);
    wr::<u32>(this, M::deviceLights_allocator_freeList, 0);
    wr::<u32>(this, M::deviceLights_allocator_allocator, 0);
    wr(map, HM::vtbl, VTBL_HASH_MAP_LIGHT);
    eng::texture_manager_clear_hash_map(map);
    eng::operator_delete(rd(map, HM::arrayBuckets));
    wr(this, M::vtbl, VTBL_NI_REF_OBJECT);
    eng::ref_object_count_add(-1);
}

/// 0x6BB190 `NiDX8LightManager::nextFrame`
pub unsafe fn next_frame(this: usize) {
    crate::shadow::touch(&next_frame_port::STATE);
    let cs = this + M::criticalSection;
    eng::critical_section_lock(cs, NAME_NEXT_FRAME);
    disable_device_lights(this);
    wr::<u32>(this, M::lastState, 0);
    eng::critical_section_unlock(cs);
}

/// 0x6BB200 `NiDX8LightManager::setState`
pub unsafe fn set_state(this: usize, state: usize, texturing: usize, vertex_color: usize) {
    crate::shadow::touch(&set_state_port::STATE);
    let cs = this + M::criticalSection;
    eng::critical_section_lock(cs, NAME_SET_STATE);
    let render_state = || rd::<usize>(this, M::renderState);
    let setting = |state: u32| RS::renderStateSettings + state as usize * 8;

    let apply_mode: u32 = rd(texturing, TP::applyMode);
    wr(this, M::applyMode, apply_mode);
    let base_map: usize = rd(rd::<usize>(texturing, TP::maps_storage), 0);
    if base_map != 0 && rd::<usize>(base_map, TM::texture) != 0 && apply_mode == 0 {
        // A base texture in replace mode: the lights are not visible.
        if rd::<u8>(this, M::d3dLightingState) != 0 {
            set_if_changed(render_state(), D3DRS_LIGHTING, 0);
            wr::<u8>(this, M::d3dLightingState, 0);
        }
        eng::critical_section_unlock(cs);
        return;
    }

    let lighting_mode: u32 = rd(vertex_color, VC::lightingMode);
    let source_mode: u32 = rd(vertex_color, VC::sourceMode);
    if lighting_mode == 0 {
        if source_mode != 0 && source_mode != 2 {
            if rd::<u8>(this, M::d3dLightingState) != 0 {
                set_render_state(render_state(), D3DRS_LIGHTING, 0, 0);
                wr::<u8>(this, M::d3dLightingState, 0);
            }
            eng::critical_section_unlock(cs);
            return;
        }
        let current: u32 = rd(this, M::sourceVertexMode);
        wr::<u8>(this, M::useAmbientDiffuse, 0);
        if current == 1 {
            set_if_changed(render_state(), D3DRS_EMISSIVEMATERIALSOURCE, 0);
            wr::<u32>(this, M::sourceVertexMode, 0);
        }
    } else {
        if source_mode != rd::<u32>(this, M::sourceVertexMode) {
            match source_mode {
                0 => {
                    set_if_changed(render_state(), D3DRS_EMISSIVEMATERIALSOURCE, 0);
                    set_if_changed(render_state(), D3DRS_AMBIENTMATERIALSOURCE, 0);
                    set_if_changed(render_state(), D3DRS_DIFFUSEMATERIALSOURCE, 0);
                }
                1 => {
                    let rs = render_state();
                    if rd::<u32>(rs, setting(D3DRS_EMISSIVEMATERIALSOURCE)) != 1 {
                        setting_set_value(rs + setting(D3DRS_EMISSIVEMATERIALSOURCE), 1, 0);
                        device_set_render_state(rd(rs, RS::device), D3DRS_EMISSIVEMATERIALSOURCE, 1);
                    }
                    let rs = render_state();
                    if rd::<u32>(rs, setting(D3DRS_AMBIENTMATERIALSOURCE)) != 0 {
                        setting_set_value(rs + setting(D3DRS_AMBIENTMATERIALSOURCE), 0, 0);
                        device_set_render_state(rd(rs, RS::device), D3DRS_AMBIENTMATERIALSOURCE, 0);
                    }
                    set_render_state(render_state(), D3DRS_DIFFUSEMATERIALSOURCE, 0, 0);
                }
                2 => {
                    set_if_changed(render_state(), D3DRS_EMISSIVEMATERIALSOURCE, 0);
                    set_if_changed(render_state(), D3DRS_AMBIENTMATERIALSOURCE, 1);
                    let rs = render_state();
                    if rd::<u32>(rs, setting(D3DRS_DIFFUSEMATERIALSOURCE)) != 1 {
                        setting_set_value(rs + setting(D3DRS_DIFFUSEMATERIALSOURCE), 1, 0);
                        device_set_render_state(rd(rs, RS::device), D3DRS_DIFFUSEMATERIALSOURCE, 1);
                    }
                }
                _ => {}
            }
            wr(this, M::sourceVertexMode, source_mode);
        }
        wr::<u8>(this, M::useAmbientDiffuse, 1);
    }

    if rd::<u8>(this, M::d3dLightingState) == 0 {
        set_render_state(render_state(), D3DRS_LIGHTING, 1, 0);
        wr::<u8>(this, M::d3dLightingState, 1);
    }
    if state != rd::<usize>(this, M::lastState) || rd::<u8>(this, M::useAmbientDiffuse) != rd::<u8>(this, M::lastUseAmbientDiffuse) {
        update_lights(this, state);
        let use_ambient_diffuse: u8 = rd(this, M::useAmbientDiffuse);
        wr(this, M::lastState, state);
        wr(this, M::lastUseAmbientDiffuse, use_ambient_diffuse);
    }
    eng::critical_section_unlock(cs);
}

/// 0x6BB4E0 `NiDX8LightManager::RemoveLight`
pub unsafe fn remove_light(this: usize, light: usize) {
    crate::shadow::touch(&remove_light_port::STATE);
    let cs = this + M::criticalSection;
    eng::critical_section_lock(cs, NAME_REMOVE_LIGHT);
    let map = this + M::lightEntries;

    let entry = hash_map::find_value(map, rd(light, L::index));

    let key: u32 = rd(light, L::index);
    hash_map::remove_with(map, key, |node| eng::operator_delete(node));
    eng::operator_delete(entry);

    device_light_enable(rd(this, M::device), rd(light, L::index), 0);

    let list = this + M::deviceLights;
    let head: usize = rd(this, M::deviceLights_head);
    let mut next = head;
    while next != 0 {
        let item = next;
        let data: usize = rd(item, ITEM_DATA);
        next = rd(item, ITEM_NEXT);
        if data != light {
            continue;
        }
        if item == head {
            let after: usize = rd(head, ITEM_NEXT);
            wr(this, M::deviceLights_head, after);
            if after != 0 {
                wr::<u32>(after, ITEM_PREVIOUS, 0);
            } else {
                wr::<u32>(this, M::deviceLights_tail, 0);
            }
            wr::<u32>(head, ITEM_DATA, 0);
            wr::<usize>(head, ITEM_NEXT, rd(this, M::deviceLights_allocator_freeList));
            wr(this, M::deviceLights_allocator_freeList, head);
        } else {
            let tail: usize = rd(this, M::deviceLights_tail);
            if item == tail {
                let before: usize = rd(tail, ITEM_PREVIOUS);
                wr(this, M::deviceLights_tail, before);
                if before != 0 {
                    wr::<u32>(before, ITEM_NEXT, 0);
                } else {
                    wr::<u32>(this, M::deviceLights_head, 0);
                }
                eng::list_dealloc_item(list, tail);
            } else {
                let before: usize = rd(item, ITEM_PREVIOUS);
                let after: usize = rd(item, ITEM_NEXT);
                if before != 0 {
                    wr(before, ITEM_NEXT, after);
                }
                if after != 0 {
                    wr(after, ITEM_PREVIOUS, before);
                }
                eng::list_dealloc_item(list, item);
            }
        }
        wr(this, M::deviceLights_count, rd::<u32>(this, M::deviceLights_count).wrapping_sub(1));
        break;
    }
    eng::critical_section_unlock(cs);
    // On the cell worker the event goes through `worker_events`.
    if let Some(t) = crate::main_thread::MainThread::here() {
        crate::light_stream::forget(&t, key);
    }
}

/// 0x6BB6C0 `NiDX8LightManager::updateLights`. The port sends the lights of one effect state
/// to the device. The sum of the ambient lights becomes the ambient render state.
pub unsafe fn update_lights(this: usize, state: usize) {
    crate::shadow::touch(&update_lights_port::STATE);
    let cs = this + M::criticalSection;
    eng::critical_section_lock(cs, NAME_UPDATE_LIGHTS);
    // Alpha is 255 and there is no colour. The game code makes the value from four float
    // constants.
    let mut ambient: u32 = 0xFF00_0000;
    let ambient_ptr = core::ptr::addr_of_mut!(ambient) as usize;
    disable_device_lights(this);

    if rd::<u8>(this, M::useAmbientDiffuse) != 0 && state != 0 {
        let map = this + M::lightEntries;
        let mut iter: usize = rd(state, DES::lights);
        while iter != 0 {
            let light: usize = rd(iter, DL::effect);
            iter = rd(iter, DL::next);
            if rd::<u8>(light, L::enabled) == 0 {
                continue;
            }
            if rd::<f32>(light, L::dimmer) < f32::from_bits(MIN_DIMMER) {
                continue;
            }
            if light_get_type(light) == 0 {
                let dimmer: u32 = rd(light, L::dimmer);
                let now: u32 = rd(ambient_ptr, 0);
                eng::packed_color_set_red(ambient_ptr, x87_ambient_channel((now >> 16) & 0xFF, dimmer, rd(light, L::ambientColor_r)));
                let now: u32 = rd(ambient_ptr, 0);
                eng::packed_color_set_green(ambient_ptr, x87_ambient_channel((now >> 8) & 0xFF, dimmer, rd(light, L::ambientColor_g)));
                let now: u32 = rd(ambient_ptr, 0);
                eng::packed_color_set_blue(ambient_ptr, x87_ambient_channel(now & 0xFF, dimmer, rd(light, L::ambientColor_b)));
                continue;
            }

            let mut entry = hash_map::find_value(map, rd(light, L::index));
            let changed;
            if entry != 0 {
                changed = light_entry_update(entry, light) != 0;
            } else {
                entry = eng::operator_new(E::SIZE);
                if entry != 0 {
                    // The constructor of the entry. The game code has it inline.
                    eng::critical_section_lock(ENTRY_CRITICAL_SECTION, NAME_ENTRY_CTOR);
                    wr::<u32>(entry, E::revisionId, 0);
                    wr::<u32>(entry, E::lightIndex, rd(light, L::index));
                    light_entry_update(entry, light);
                    eng::critical_section_unlock(ENTRY_CRITICAL_SECTION);
                }
                let key: u32 = rd(light, L::index);
                hash_map::set_with(map, key, entry, |size| eng::operator_new(size));
                changed = true;
            }
            if changed {
                device_set_light(rd(this, M::device), rd(entry, E::lightIndex), entry);
            }
            device_light_enable(rd(this, M::device), rd(entry, E::lightIndex), 1);

            if rd::<usize>(this, M::deviceLights_allocator_freeList) == 0 {
                let mut block = eng::operator_new(8);
                if block != 0 {
                    block = eng::list_allocator_block_ctor(block, rd(this, M::deviceLights_allocator_count));
                }
                wr::<usize>(block, BLOCK_NEXT, rd(this, M::deviceLights_allocator_allocator));
                wr(this, M::deviceLights_allocator_allocator, block);
                wr::<usize>(this, M::deviceLights_allocator_freeList, rd(block, BLOCK_ITEMS));
            }
            let item: usize = rd(this, M::deviceLights_allocator_freeList);
            wr::<usize>(this, M::deviceLights_allocator_freeList, rd(item, ITEM_NEXT));
            wr(item, ITEM_DATA, light);
            wr::<u32>(item, ITEM_PREVIOUS, 0);
            wr::<usize>(item, ITEM_NEXT, rd(this, M::deviceLights_head));
            let head: usize = rd(this, M::deviceLights_head);
            if head != 0 {
                wr(head, ITEM_PREVIOUS, item);
            } else {
                wr(this, M::deviceLights_tail, item);
            }
            wr(this, M::deviceLights_head, item);
            wr(this, M::deviceLights_count, rd::<u32>(this, M::deviceLights_count).wrapping_add(1));
        }
    }

    let ambient: u32 = rd(ambient_ptr, 0);
    if rd::<u32>(this, M::ambientLightingState) != ambient {
        set_if_changed(rd(this, M::renderState), D3DRS_AMBIENT, ambient);
        wr(this, M::ambientLightingState, ambient);
    }
    eng::critical_section_unlock(cs);
}

/// The ambient light the light manager would set for a draw with this effect state
/// (`D3DRS_AMBIENT`): the ambient lights of the state that are on and bright enough,
/// each channel by the dimmer, as `update_lights` makes it. For a shape the engine does not
/// display (`instance_stream`).
pub(crate) unsafe fn ambient_of_state(state: usize) -> u32 {
    let mut ambient: u32 = 0xFF00_0000;
    let ambient_ptr = core::ptr::addr_of_mut!(ambient) as usize;
    if state == 0 {
        return ambient;
    }
    let mut iter: usize = rd(state, DES::lights);
    while iter != 0 {
        let light: usize = rd(iter, DL::effect);
        iter = rd(iter, DL::next);
        if light == 0 || rd::<u8>(light, L::enabled) == 0 || rd::<f32>(light, L::dimmer) < f32::from_bits(MIN_DIMMER) || light_get_type(light) != 0 {
            continue;
        }
        let dimmer: u32 = rd(light, L::dimmer);
        let now: u32 = rd(ambient_ptr, 0);
        eng::packed_color_set_red(ambient_ptr, x87_ambient_channel((now >> 16) & 0xFF, dimmer, rd(light, L::ambientColor_r)));
        let now: u32 = rd(ambient_ptr, 0);
        eng::packed_color_set_green(ambient_ptr, x87_ambient_channel((now >> 8) & 0xFF, dimmer, rd(light, L::ambientColor_g)));
        let now: u32 = rd(ambient_ptr, 0);
        eng::packed_color_set_blue(ambient_ptr, x87_ambient_channel(now & 0xFF, dimmer, rd(light, L::ambientColor_b)));
    }
    rd(ambient_ptr, 0)
}

/// The ambient light the light manager would set for a draw whose effect state holds the
/// lights of the effect lists of `nodes` (`NiNode::effectList`, +0xA8, a linked list of
/// effect and next): their ambient lights summed by the rule of `ambient_of_state`. For the
/// ambient of the world from the lights of its roots, without a draw (`world_walk`).
pub(crate) unsafe fn ambient_of_effect_lists(nodes: &[usize]) -> u32 {
    let mut ambient: u32 = 0xFF00_0000;
    let ambient_ptr = core::ptr::addr_of_mut!(ambient) as usize;
    for &node in nodes {
        if node == 0 {
            continue;
        }
        let mut item = node + crate::gen::layouts::NiNode::effectList;
        let mut seen = 0;
        while item != 0 && seen < 256 {
            let light: usize = rd(item, 0);
            item = rd(item, 4);
            seen += 1;
            if light == 0 || rd::<u8>(light, L::enabled) == 0 || rd::<f32>(light, L::dimmer) < f32::from_bits(MIN_DIMMER) || light_get_type(light) != 0 {
                continue;
            }
            let dimmer: u32 = rd(light, L::dimmer);
            let now: u32 = rd(ambient_ptr, 0);
            eng::packed_color_set_red(ambient_ptr, x87_ambient_channel((now >> 16) & 0xFF, dimmer, rd(light, L::ambientColor_r)));
            let now: u32 = rd(ambient_ptr, 0);
            eng::packed_color_set_green(ambient_ptr, x87_ambient_channel((now >> 8) & 0xFF, dimmer, rd(light, L::ambientColor_g)));
            let now: u32 = rd(ambient_ptr, 0);
            eng::packed_color_set_blue(ambient_ptr, x87_ambient_channel(now & 0xFF, dimmer, rd(light, L::ambientColor_b)));
        }
    }
    rd(ambient_ptr, 0)
}

/// 0x6BBB40 `NiDX8LightManager::ReleaseLights`. The port deletes each entry. The key of the
/// entry stays in the map with no entry.
pub unsafe fn release_lights(this: usize) {
    crate::shadow::touch(&release_lights_port::STATE);
    let cs = this + M::criticalSection;
    eng::critical_section_lock(cs, NAME_RELEASE_LIGHTS);
    let map = this + M::lightEntries;
    let mut node = hash_map::first(map);
    while node != 0 {
        let key: u32 = rd(node, HN::key);
        let entry: usize = rd(node, HN::data);
        let mut next: usize = rd(node, HN::nextNode);
        if next == 0 {
            next = hash_map::next_bucket(map, key);
        }
        if entry != 0 {
            device_light_enable(rd(this, M::device), rd(entry, E::lightIndex), 0);
            eng::operator_delete(entry);
            hash_map::set_with(map, key, 0, |size| eng::operator_new(size));
        }
        node = next;
    }
    eng::critical_section_unlock(cs);
}

// ---- the memory a call may write

/// Adds the items in the list of device lights. Returns the number of items.
unsafe fn add_device_light_items(set: &mut Regions, this: usize) -> usize {
    let mut item: usize = rd(this, M::deviceLights_head);
    let mut count = 0;
    while item != 0 {
        set.add(item, ITEM_SIZE);
        count += 1;
        item = rd(item, ITEM_NEXT);
    }
    count
}

/// Adds the memory that `updateLights` writes: the manager, the render state, the list items
/// that it frees and takes, and the entries of the lights that changed.
///
/// The call is a skip when it allocates and the allocator hooks are not in place. A call
/// allocates when it makes a new entry, a new map node or a new allocator block. The entry
/// and the node come from `operator new` (0x727692). The allocator block also comes from
/// `operator new`, and `NiTListAllocator::Data::ctor` (0x6BBEF0) gets the items of the block
/// from `operator new`. `updateLights` frees nothing. Thus such a call calls
/// `set.allocates()`, and the new blocks need no region. A new node becomes the head of its
/// bucket, thus the head of the bucket is a region. A node that has no entry gets the new
/// entry, thus that node is a region.
unsafe fn add_update_lights(set: &mut Regions, this: usize, state: usize, use_ambient_diffuse: u8) {
    set.add(this, M::SIZE);
    set.add(rd(this, M::renderState), RS::SIZE);
    let in_list = add_device_light_items(set, this);
    if use_ambient_diffuse == 0 || state == 0 {
        return;
    }
    let map = this + M::lightEntries;
    let mut needed = 0;
    let mut iter: usize = rd(state, DES::lights);
    while iter != 0 {
        let light: usize = rd(iter, DL::effect);
        iter = rd(iter, DL::next);
        if rd::<u8>(light, L::enabled) == 0 || rd::<f32>(light, L::dimmer) < f32::from_bits(MIN_DIMMER) || light_get_type(light) == 0 {
            continue;
        }
        needed += 1;
        let (bucket, node) = hash_map::find_in_bucket(map, rd(light, L::index));
        let entry: usize = if node != 0 { rd(node, HN::data) } else { 0 };
        if entry == 0 {
            set.allocates();
            if node != 0 {
                set.add(node, HN::SIZE);
            } else {
                set.add(rd::<usize>(map, HM::arrayBuckets) + bucket * 4, 4);
            }
            continue;
        }
        if rd::<u32>(entry, E::revisionId) != rd::<u32>(light, L::revisionID) {
            set.add(entry, E::SIZE);
        }
    }
    // The function uses the freed items first. Then it takes items from the free list. When
    // the free list is empty, it makes a new allocator block.
    let mut free: usize = rd(this, M::deviceLights_allocator_freeList);
    for _ in in_list..needed {
        if free == 0 {
            set.allocates();
            break;
        }
        set.add(free, ITEM_SIZE);
        free = rd(free, ITEM_NEXT);
    }
}

/// The call is a skip when it allocates and the allocator hooks are not in place.
/// `add_update_lights` gives the cases.
unsafe fn update_lights_regions(set: &mut Regions, this: usize, state: usize) {
    add_update_lights(set, this, state, rd(this, M::useAmbientDiffuse));
}

/// The call is a skip when it gets to `updateLights`, that call allocates and the allocator
/// hooks are not in place. `add_update_lights` gives the cases. A call that does not get to
/// `updateLights` writes only the manager and the render state.
unsafe fn set_state_regions(set: &mut Regions, this: usize, state: usize, texturing: usize, vertex_color: usize) {
    // The code makes the same decisions as the function, to find if the function gets to
    // `updateLights`.
    let base_map: usize = rd(rd::<usize>(texturing, TP::maps_storage), 0);
    let replace = base_map != 0 && rd::<usize>(base_map, TM::texture) != 0 && rd::<u32>(texturing, TP::applyMode) == 0;
    let source_mode: u32 = rd(vertex_color, VC::sourceMode);
    let use_ambient_diffuse = if rd::<u32>(vertex_color, VC::lightingMode) != 0 {
        Some(1u8)
    } else if source_mode == 0 || source_mode == 2 {
        Some(0u8)
    } else {
        None
    };
    if let (false, Some(use_ambient_diffuse)) = (replace, use_ambient_diffuse) {
        if state != rd::<usize>(this, M::lastState) || use_ambient_diffuse != rd::<u8>(this, M::lastUseAmbientDiffuse) {
            // This adds the manager and the render state also.
            add_update_lights(set, this, state, use_ambient_diffuse);
            return;
        }
    }
    set.add(this, M::SIZE);
    set.add(rd(this, M::renderState), RS::SIZE);
}

/// The call is not a skip. `nextFrame` writes the manager and the items of the list of device
/// lights. It allocates nothing and frees nothing.
unsafe fn next_frame_regions(set: &mut Regions, this: usize) {
    set.add(this, M::SIZE);
    add_device_light_items(set, this);
}

/// The call is a skip when the map has a node for the light. `RemoveLight` then frees the
/// node and the entry with `operator delete`, and takes the node out of its bucket. A
/// comparison of such a call needs `set.allocates()`, and the head of the bucket or the node
/// before as a region. This function does not give them.
unsafe fn remove_light_regions(set: &mut Regions, this: usize, light: usize) {
    if hash_map::find(this + M::lightEntries, rd(light, L::index)) != 0 {
        set.skip();
        return;
    }
    set.add(this, M::SIZE);
    let mut item: usize = rd(this, M::deviceLights_head);
    while item != 0 {
        if rd::<usize>(item, ITEM_DATA) == light {
            set.add(item, ITEM_SIZE);
            set.add(rd(item, ITEM_PREVIOUS), ITEM_SIZE);
            set.add(rd(item, ITEM_NEXT), ITEM_SIZE);
            break;
        }
        item = rd(item, ITEM_NEXT);
    }
}

/// The call is a skip when a node of the map has an entry. `ReleaseLights` then frees the
/// entry with `operator delete` and writes the node. A comparison of such a call needs
/// `set.allocates()` and each such node as a region. This function does not give them.
unsafe fn release_lights_regions(set: &mut Regions, this: usize) {
    let buckets: usize = rd(this, M::lightEntries_arrayBuckets);
    for i in 0..rd::<u32>(this, M::lightEntries_bucketCount) as usize {
        let mut node: usize = rd(buckets, i * 4);
        while node != 0 {
            if rd::<usize>(node, HN::data) != 0 {
                set.skip();
                return;
            }
            node = rd(node, HN::nextNode);
        }
    }
    set.add(this, M::SIZE);
}

/// The call is not a skip. `resetRenderStates` writes the manager and the render state.
unsafe fn reset_render_states_regions(set: &mut Regions, this: usize) {
    set.add(this, M::SIZE);
    set.add(rd(this, M::renderState), RS::SIZE);
}

port_thiscall!(light_entry_update_port, 0x6BAB00, "NiDX8LightManager::LightEntry::Update",
    fn(this, light: usize) -> u8,
    compare_eax = true,
    regions = |set| set.add(this, E::SIZE),
    rust = light_entry_update);

// The call is always a skip. The constructor makes a critical section and adds a reference to
// the device. The dry device passes `AddRef` on, thus two runs add two references.
port_thiscall!(ctor_port, 0x6BAD10, "NiDX8LightManager::ctor",
    fn(this, render_state: usize, device: usize) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = ctor);

port_thiscall!(reset_render_states_port, 0x6BAEC0, "NiDX8LightManager::resetRenderStates",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| reset_render_states_regions(set, this),
    rust = reset_render_states);

// The call is always a skip. The destructor releases the reference to the device and deletes
// the critical section. These cannot run twice.
port_thiscall!(dtor_port, 0x6BAF90, "NiDX8LightManager::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = dtor);

port_thiscall!(next_frame_port, 0x6BB190, "NiDX8LightManager::nextFrame",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| next_frame_regions(set, this),
    rust = next_frame);

port_thiscall!(set_state_port, 0x6BB200, "NiDX8LightManager::setState",
    fn(this, state: usize, texturing: usize, vertex_color: usize) -> (),
    compare_eax = false,
    regions = |set| set_state_regions(set, this, state, texturing, vertex_color),
    rust = set_state);

port_thiscall!(remove_light_port, 0x6BB4E0, "NiDX8LightManager::RemoveLight",
    fn(this, light: usize) -> (),
    compare_eax = false,
    regions = |set| remove_light_regions(set, this, light),
    rust = remove_light);

port_thiscall!(update_lights_port, 0x6BB6C0, "NiDX8LightManager::updateLights",
    fn(this, state: usize) -> (),
    compare_eax = false,
    regions = |set| update_lights_regions(set, this, state),
    rust = update_lights);

port_thiscall!(release_lights_port, 0x6BBB40, "NiDX8LightManager::ReleaseLights",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| release_lights_regions(set, this),
    rust = release_lights);

pub static PORTS: &[&crate::shadow::PortState] = &[
    &light_entry_update_port::STATE,
    &ctor_port::STATE,
    &reset_render_states_port::STATE,
    &dtor_port::STATE,
    &next_frame_port::STATE,
    &set_state_port::STATE,
    &remove_light_port::STATE,
    &update_lights_port::STATE,
    &release_lights_port::STATE,
];

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::d3d::{DEVICE_ADD_REF, DEVICE_LIGHT_ENABLE, DEVICE_SET_LIGHT};
    use crate::ports::hash_map::{
        SLOT_CMP_KEY as MAP_VTBL_CMP_KEY, SLOT_DTOR_KEY_VALUE_PAIR as MAP_VTBL_DTOR_KEY_VALUE_PAIR, SLOT_HASH_KEY as MAP_VTBL_HASH_KEY,
        SLOT_MAKE_KEY_VALUE_PAIR as MAP_VTBL_MAKE_KEY_VALUE_PAIR,
    };

    thread_local! {
        /// The calls the fake device got: (vtable slot, first argument, second argument).
        static CALLS: RefCell<Vec<(u32, u32, u32)>> = const { RefCell::new(Vec::new()) };
    }

    fn calls() -> Vec<(u32, u32, u32)> {
        CALLS.with(|c| core::mem::take(&mut *c.borrow_mut()))
    }

    extern "system" fn dev_add_ref(_device: usize) -> u32 {
        CALLS.with(|c| c.borrow_mut().push((1, 0, 0)));
        0
    }
    extern "system" fn dev_set_light(_device: usize, index: u32, light: usize) -> u32 {
        let kind = unsafe { rd::<u32>(light, 0) };
        CALLS.with(|c| c.borrow_mut().push((44, index, kind)));
        0
    }
    extern "system" fn dev_light_enable(_device: usize, index: u32, enable: u32) -> u32 {
        CALLS.with(|c| c.borrow_mut().push((46, index, enable)));
        0
    }
    extern "system" fn dev_set_render_state(_device: usize, state: u32, value: u32) -> u32 {
        CALLS.with(|c| c.borrow_mut().push((50, state, value)));
        0
    }

    extern "thiscall" fn map_hash(map: usize, key: u32) -> u32 {
        key % unsafe { rd::<u32>(map, HM::bucketCount) }
    }
    extern "thiscall" fn map_cmp(_map: usize, a: u32, b: u32) -> u32 {
        // Only AL is the result.
        0x1234_5600 | (a == b) as u32
    }
    extern "thiscall" fn map_make(_map: usize, node: usize, key: u32, value: usize) -> u32 {
        unsafe {
            wr(node, HN::key, key);
            wr(node, HN::data, value);
        }
        0
    }
    extern "thiscall" fn map_dtor(_map: usize, _node: usize) -> u32 {
        0
    }

    extern "thiscall" fn type_ambient(_light: usize) -> u32 {
        0
    }
    extern "thiscall" fn type_directional(_light: usize) -> u32 {
        1
    }
    extern "thiscall" fn type_point(_light: usize) -> u32 {
        2
    }
    extern "thiscall" fn type_spot(_light: usize) -> u32 {
        3
    }

    const BUCKETS: usize = 5;

    /// A manager with a render state, a device and an empty map, all in leaked memory.
    struct World {
        manager: usize,
        render_state: usize,
    }

    fn leak(size: usize) -> usize {
        Box::leak(vec![0u8; size].into_boxed_slice()).as_mut_ptr() as usize
    }

    unsafe fn world() -> World {
        let device_vtable = leak(97 * 4);
        wr(device_vtable, DEVICE_ADD_REF, dev_add_ref as *const () as usize);
        wr(device_vtable, DEVICE_SET_LIGHT, dev_set_light as *const () as usize);
        wr(device_vtable, DEVICE_LIGHT_ENABLE, dev_light_enable as *const () as usize);
        wr(device_vtable, 0xC8, dev_set_render_state as *const () as usize);
        let device = leak(4);
        wr(device, 0, device_vtable);

        let render_state = leak(RS::SIZE);
        wr(render_state, RS::device, device);

        let map_vtable = leak(0x14);
        wr(map_vtable, MAP_VTBL_HASH_KEY, map_hash as *const () as usize);
        wr(map_vtable, MAP_VTBL_CMP_KEY, map_cmp as *const () as usize);
        wr(map_vtable, MAP_VTBL_MAKE_KEY_VALUE_PAIR, map_make as *const () as usize);
        wr(map_vtable, MAP_VTBL_DTOR_KEY_VALUE_PAIR, map_dtor as *const () as usize);

        let manager = leak(M::SIZE);
        wr(manager, M::lightEntries_vtbl, map_vtable);
        wr::<u32>(manager, M::lightEntries_bucketCount, BUCKETS as u32);
        wr(manager, M::lightEntries_arrayBuckets, leak(BUCKETS * 4));
        wr::<u32>(manager, M::deviceLights_allocator_count, 2);
        wr(manager, M::device, device);
        wr(manager, M::renderState, render_state);
        calls();
        World { manager, render_state }
    }

    unsafe fn light(get_type: extern "thiscall" fn(usize) -> u32, index: u32) -> usize {
        let vtable = leak(0x98);
        wr(vtable, LIGHT_VTBL_GET_TYPE, get_type as *const () as usize);
        let light = leak(0xF0);
        wr(light, 0, vtable);
        wr::<u8>(light, L::enabled, 1);
        wr::<u32>(light, L::index, index);
        wr::<u32>(light, L::revisionID, 1);
        wr::<f32>(light, L::dimmer, 0.5);
        for (i, base) in [L::ambientColor, L::diffuseColor, L::specularColor].into_iter().enumerate() {
            for c in 0..3 {
                wr::<f32>(light, base + c * 4, (i * 4 + c + 1) as f32);
            }
        }
        light
    }

    /// An effect state with these lights, in this order.
    unsafe fn effect_state(lights: &[usize]) -> usize {
        let state = leak(DES::SIZE);
        let mut next = 0usize;
        for &light in lights.iter().rev() {
            let link = leak(DL::SIZE);
            wr(link, DL::next, next);
            wr(link, DL::effect, light);
            next = link;
        }
        wr(state, DES::lights, next);
        state
    }

    unsafe fn setting(w: &World, state: u32) -> u32 {
        rd(w.render_state, RS::renderStateSettings + state as usize * 8)
    }

    unsafe fn device_light_list(manager: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut item: usize = rd(manager, M::deviceLights_head);
        let mut previous = 0usize;
        while item != 0 {
            assert_eq!(rd::<usize>(item, ITEM_PREVIOUS), previous);
            out.push(rd(item, ITEM_DATA));
            previous = item;
            item = rd(item, ITEM_NEXT);
        }
        assert_eq!(rd::<usize>(manager, M::deviceLights_tail), previous);
        assert_eq!(rd::<u32>(manager, M::deviceLights_count) as usize, out.len());
        out
    }

    #[test]
    fn x87_sequences_on_exact_values() {
        assert_eq!(f32::from_bits(x87_mul(3.0f32.to_bits(), 0.5f32.to_bits())), 1.5);
        // 0 / 255 + 0.5 * 0.25
        assert_eq!(f32::from_bits(x87_ambient_channel(0, 0.5f32.to_bits(), 0.25f32.to_bits())), 0.125);
        assert_eq!(x87_spot_angle(0, core::f32::consts::PI.to_bits()), 0);
        let angle = f32::from_bits(x87_spot_angle(90.0f32.to_bits(), core::f32::consts::PI.to_bits()));
        assert!((angle - core::f32::consts::PI).abs() < 1e-5);
    }

    #[test]
    fn entry_update_point_light() {
        unsafe {
            let l = light(type_point, 7);
            for c in 0..3 {
                wr::<f32>(l, L::worldTransform_translation + c * 4, 100.0 + c as f32);
                wr::<f32>(l, POINT_ATTENUATION + c * 4, 0.25 * (c + 1) as f32);
            }
            let e = leak(E::SIZE);
            core::ptr::write_bytes(e as *mut u8, 0xEE, E::SIZE);
            wr::<u32>(e, E::revisionId, 0);
            wr::<u32>(e, E::lightIndex, 7);

            assert_eq!(light_entry_update(e, l), 1);
            assert_eq!(rd::<u32>(e, E::d3dLight_Type), 1);
            assert_eq!(rd::<[f32; 4]>(e, E::d3dLight_Diffuse), [2.5, 3.0, 3.5, 0.0]);
            assert_eq!(rd::<[f32; 4]>(e, E::d3dLight_Specular), [4.5, 5.0, 5.5, 0.0]);
            assert_eq!(rd::<[f32; 4]>(e, E::d3dLight_Ambient), [0.5, 1.0, 1.5, 0.0]);
            assert_eq!(rd::<[f32; 3]>(e, E::d3dLight_Position), [100.0, 101.0, 102.0]);
            assert_eq!(rd::<[f32; 3]>(e, E::d3dLight_Direction), [0.0; 3]);
            assert_eq!(rd::<u32>(e, E::d3dLight_Range), eng::TEST_RANGE);
            assert_eq!(rd::<[f32; 3]>(e, E::d3dLight_Attenuation0), [0.25, 0.5, 0.75]);
            assert_eq!(rd::<[f32; 2]>(e, E::d3dLight_Theta), [0.0; 2]);
            assert_eq!(rd::<u32>(e, E::revisionId), 1);
            assert_eq!(rd::<u32>(e, E::lightIndex), 7);

            // The revision is the same: the port changes nothing.
            wr::<f32>(l, L::dimmer, 1.0);
            assert_eq!(light_entry_update(e, l), 0);
            assert_eq!(rd::<f32>(e, E::d3dLight_Diffuse_r), 2.5);
        }
    }

    #[test]
    fn entry_update_directional_spot_and_ambient() {
        unsafe {
            let e = leak(E::SIZE);
            let l = light(type_directional, 1);
            wr::<[f32; 3]>(l, DIRECTIONAL_DIRECTION, [0.0, 0.0, -1.0]);
            assert_eq!(light_entry_update(e, l), 1);
            assert_eq!(rd::<u32>(e, E::d3dLight_Type), 3);
            assert_eq!(rd::<[f32; 3]>(e, E::d3dLight_Direction), [0.0, 0.0, -1.0]);
            assert_eq!(rd::<u32>(e, E::d3dLight_Range), 0);
            assert_eq!(rd::<[f32; 3]>(e, E::d3dLight_Attenuation0), [0.0; 3]);

            let l = light(type_spot, 1);
            wr::<u32>(l, L::revisionID, 2);
            wr::<[f32; 3]>(l, L::worldTransform_translation, [1.0, 2.0, 3.0]);
            wr::<[f32; 3]>(l, POINT_ATTENUATION, [1.0, 0.0, 0.5]);
            wr::<[f32; 3]>(l, SPOT_DIRECTION, [0.0, 1.0, 0.0]);
            wr::<f32>(l, SPOT_ANGLE, 0.0);
            wr::<f32>(l, SPOT_EXPONENT, 4.0);
            assert_eq!(light_entry_update(e, l), 1);
            assert_eq!(rd::<u32>(e, E::d3dLight_Type), 2);
            assert_eq!(rd::<[f32; 3]>(e, E::d3dLight_Position), [1.0, 2.0, 3.0]);
            assert_eq!(rd::<[f32; 3]>(e, E::d3dLight_Direction), [0.0, 1.0, 0.0]);
            assert_eq!(rd::<f32>(e, E::d3dLight_Falloff), 4.0);
            assert_eq!(rd::<[f32; 3]>(e, E::d3dLight_Attenuation0), [1.0, 0.0, 0.5]);
            assert_eq!(rd::<u32>(e, E::d3dLight_Range), eng::TEST_RANGE);

            // An ambient light gets no type. It gets only the colours.
            let l = light(type_ambient, 1);
            wr::<u32>(l, L::revisionID, 3);
            assert_eq!(light_entry_update(e, l), 1);
            assert_eq!(rd::<u32>(e, E::d3dLight_Type), 0);
            assert_eq!(rd::<f32>(e, E::d3dLight_Ambient), 0.5);
        }
    }

    #[test]
    fn ctor_and_reset_render_states() {
        unsafe {
            let w = world();
            let device: usize = rd(w.manager, M::device);
            let fresh = leak(M::SIZE);
            core::ptr::write_bytes(fresh as *mut u8, 0xEE, M::SIZE);
            let rs = w.render_state;
            wr::<u32>(rs, RS::renderStateSettings + D3DRS_LIGHTING as usize * 8, 1);
            wr::<u32>(rs, RS::renderStateSettings + D3DRS_AMBIENTMATERIALSOURCE as usize * 8, 1);

            assert_eq!(ctor(fresh, rs, device), fresh);
            assert_eq!(calls(), [(1, 0, 0), (50, D3DRS_LIGHTING, 0), (50, D3DRS_AMBIENTMATERIALSOURCE, 0)]);
            assert_eq!(rd::<u32>(fresh, M::vtbl), VTBL_LIGHT_MANAGER);
            assert_eq!(rd::<u32>(fresh, M::lightEntries_bucketCount), 37);
            assert_eq!(rd::<u32>(fresh, M::deviceLights_allocator_count), 2);
            assert_eq!(rd::<[u32; 37]>(rd(fresh, M::lightEntries_arrayBuckets), 0), [0; 37]);
            assert_eq!(rd::<u32>(fresh, M::lastState), 0);
            assert_eq!(rd::<u8>(fresh, M::lastUseAmbientDiffuse), 0);
            assert_eq!(rd::<u8>(fresh, M::d3dLightingState), 0);

            wr::<u8>(fresh, M::useAmbientDiffuse, 1);
            wr::<u32>(fresh, M::ambientLightingState, 5);
            wr::<u32>(rs, RS::renderStateSettings + D3DRS_AMBIENT as usize * 8, 5);
            reset_render_states(fresh);
            assert_eq!(calls(), [(50, D3DRS_AMBIENT, 0)]);
            assert_eq!(rd::<u8>(fresh, M::useAmbientDiffuse), 0);
            assert_eq!(rd::<u32>(fresh, M::ambientLightingState), 0);
        }
    }

    #[test]
    fn update_lights_makes_entries_and_sums_the_ambient() {
        unsafe {
            let w = world();
            let m = w.manager;
            wr::<u8>(m, M::useAmbientDiffuse, 1);
            let ambient = light(type_ambient, 1);
            wr::<[f32; 3]>(ambient, L::ambientColor, [1.0, 0.5, 0.0]);
            let point = light(type_point, 12);
            let off = light(type_point, 13);
            wr::<u8>(off, L::enabled, 0);
            let dim = light(type_point, 14);
            wr::<f32>(dim, L::dimmer, 0.005);
            let directional = light(type_directional, 17); // the same bucket as 12
            let state = effect_state(&[ambient, point, off, dim, directional]);

            update_lights(m, state);
            // 0.5 * (1, 0.5, 0) gives 128 (127.5 rounded to even), 64 (63.75) and 0.
            let ambient_total = 0xFF80_4000;
            assert_eq!(
                calls(),
                [(44, 12, 1), (46, 12, 1), (44, 17, 3), (46, 17, 1), (50, D3DRS_AMBIENT, ambient_total)]
            );
            assert_eq!(rd::<u32>(m, M::ambientLightingState), ambient_total);
            assert_eq!(setting(&w, D3DRS_AMBIENT), ambient_total);
            assert_eq!(rd::<u32>(m, M::lightEntries_count), 2);
            assert_eq!(device_light_list(m), [directional, point]);
            let node = hash_map::find(m + M::lightEntries, 12);
            assert_eq!(rd::<u32>(rd(node, HN::data), E::lightIndex), 12);

            // In the second call the entries exist and did not change. There is no SetLight
            // call, and the list items come from the free list. The regions are the manager,
            // the render state and the two list items.
            let mut set = Regions::new();
            add_update_lights(&mut set, m, state, 1);
            assert_eq!(set.as_slice().len(), 4);
            assert_eq!(set.as_slice()[..2], [(m, M::SIZE), (w.render_state, RS::SIZE)]);
            assert!(set.as_slice()[2..].iter().all(|&(_, size)| size == ITEM_SIZE));
            update_lights(m, state);
            assert_eq!(calls(), [(46, 17, 0), (46, 12, 0), (46, 12, 1), (46, 17, 1)]);
            assert_eq!(device_light_list(m), [directional, point]);

            // The port sends the entry of a changed light again.
            wr::<u32>(point, L::revisionID, 2);
            let mut set = Regions::new();
            add_update_lights(&mut set, m, state, 1);
            assert_eq!(set.as_slice().len(), 5);
            assert!(set.as_slice().contains(&(hash_map::find_value(m + M::lightEntries, 12), E::SIZE)));
            update_lights(m, state);
            assert_eq!(calls(), [(46, 17, 0), (46, 12, 0), (44, 12, 1), (46, 12, 1), (46, 17, 1)]);

            // Without ambient and diffuse lighting, only the ambient state remains, and it is
            // black.
            wr::<u8>(m, M::useAmbientDiffuse, 0);
            update_lights(m, state);
            assert_eq!(calls(), [(46, 17, 0), (46, 12, 0), (50, D3DRS_AMBIENT, 0xFF00_0000)]);
            assert_eq!(device_light_list(m), []);

            // A new light needs an entry and a map node. The head of its bucket is a region.
            // The list is empty, thus the one list item comes from the free list.
            let new_light = light(type_point, 30);
            let mut set = Regions::new();
            add_update_lights(&mut set, m, effect_state(&[new_light]), 1);
            let buckets: usize = rd(m, M::lightEntries_arrayBuckets);
            let free: usize = rd(m, M::deviceLights_allocator_freeList);
            assert_eq!(
                set.as_slice(),
                [(m, M::SIZE), (w.render_state, RS::SIZE), (buckets + (30 % BUCKETS) * 4, 4), (free, ITEM_SIZE)]
            );

            // A light whose node has no entry, as after ReleaseLights: the node is a region.
            let node = hash_map::find(m + M::lightEntries, 12);
            let entry: usize = rd(node, HN::data);
            wr::<usize>(node, HN::data, 0);
            let mut set = Regions::new();
            add_update_lights(&mut set, m, effect_state(&[point]), 1);
            assert_eq!(set.as_slice()[2], (node, HN::SIZE));
            wr(node, HN::data, entry);
        }
    }

    #[test]
    fn next_frame_and_remove_light_and_release() {
        unsafe {
            let w = world();
            let m = w.manager;
            wr::<u8>(m, M::useAmbientDiffuse, 1);
            let lights = [light(type_point, 1), light(type_point, 2), light(type_point, 3), light(type_point, 4)];
            let state = effect_state(&lights);
            update_lights(m, state);
            calls();
            assert_eq!(device_light_list(m), [lights[3], lights[2], lights[1], lights[0]]);

            // RemoveLight frees the entry. The shadow test skips the call while the light has
            // a node.
            let mut set = Regions::new();
            remove_light_regions(&mut set, m, lights[1]);
            assert!(set.as_slice().is_empty());

            remove_light(m, lights[2]); // in the middle
            assert_eq!(calls(), [(46, 3, 0)]);
            assert_eq!(device_light_list(m), [lights[3], lights[1], lights[0]]);
            assert_eq!(rd::<u32>(m, M::lightEntries_count), 3);
            remove_light(m, lights[0]); // the tail
            assert_eq!(device_light_list(m), [lights[3], lights[1]]);
            remove_light(m, lights[3]); // the head
            assert_eq!(device_light_list(m), [lights[1]]);
            assert_eq!(rd::<u32>(m, M::lightEntries_count), 1);
            calls();

            // A light the manager does not know: the port makes only the device call.
            remove_light(m, lights[3]);
            assert_eq!(calls(), [(46, 4, 0)]);
            assert_eq!(device_light_list(m), [lights[1]]);
            let mut set = Regions::new();
            remove_light_regions(&mut set, m, lights[3]);
            assert_eq!(set.as_slice(), [(m, M::SIZE)]);

            wr(m, M::lastState, state);
            next_frame(m);
            assert_eq!(calls(), [(46, 2, 0)]);
            assert_eq!(device_light_list(m), []);
            assert_eq!(rd::<usize>(m, M::lastState), 0);

            // ReleaseLights deletes the entry and keeps the key.
            release_lights(m);
            assert_eq!(calls(), [(46, 2, 0)]);
            assert_eq!(rd::<u32>(m, M::lightEntries_count), 1);
            let node = hash_map::find(m + M::lightEntries, 2);
            assert_ne!(node, 0);
            assert_eq!(rd::<usize>(node, HN::data), 0);
            release_lights(m);
            assert_eq!(calls(), []);
            let mut set = Regions::new();
            release_lights_regions(&mut set, m);
            assert_eq!(set.as_slice(), [(m, M::SIZE)]);
        }
    }

    #[test]
    fn set_state_paths() {
        unsafe {
            let w = world();
            let m = w.manager;
            let texturing = leak(TP::SIZE);
            let maps = leak(4);
            wr(texturing, TP::maps_storage, maps);
            wr::<u32>(texturing, TP::applyMode, 2);
            let vertex_color = leak(VC::SIZE);
            let point = light(type_point, 3);
            let state = effect_state(&[point]);

            // Lighting is on, with material colours.
            wr::<u32>(vertex_color, VC::lightingMode, 1);
            set_state(m, state, texturing, vertex_color);
            assert_eq!(
                calls(),
                [(50, D3DRS_LIGHTING, 1), (44, 3, 1), (46, 3, 1), (50, D3DRS_AMBIENT, 0xFF00_0000)]
            );
            assert_eq!(rd::<u32>(m, M::applyMode), 2);
            assert_eq!(rd::<u8>(m, M::d3dLightingState), 1);
            assert_eq!(rd::<u8>(m, M::useAmbientDiffuse), 1);
            assert_eq!(rd::<usize>(m, M::lastState), state);
            assert_eq!(rd::<u8>(m, M::lastUseAmbientDiffuse), 1);

            // The same call again: the port makes no device call.
            set_state(m, state, texturing, vertex_color);
            assert_eq!(calls(), []);

            // Vertex colours as ambient and diffuse.
            wr::<u32>(vertex_color, VC::sourceMode, 2);
            set_state(m, state, texturing, vertex_color);
            assert_eq!(calls(), [(50, D3DRS_AMBIENTMATERIALSOURCE, 1), (50, D3DRS_DIFFUSEMATERIALSOURCE, 1)]);
            assert_eq!(rd::<u32>(m, M::sourceVertexMode), 2);

            // Vertex colours as emissive.
            wr::<u32>(vertex_color, VC::sourceMode, 1);
            set_state(m, state, texturing, vertex_color);
            assert_eq!(
                calls(),
                [(50, D3DRS_EMISSIVEMATERIALSOURCE, 1), (50, D3DRS_AMBIENTMATERIALSOURCE, 0), (50, D3DRS_DIFFUSEMATERIALSOURCE, 0)]
            );
            assert_eq!(rd::<u32>(m, M::sourceVertexMode), 1);

            // Emissive lighting mode with ignored vertex colours: the port disables the lights
            // in the device and resets the emissive source.
            wr::<u32>(vertex_color, VC::lightingMode, 0);
            wr::<u32>(vertex_color, VC::sourceMode, 0);
            set_state(m, state, texturing, vertex_color);
            assert_eq!(calls(), [(50, D3DRS_EMISSIVEMATERIALSOURCE, 0), (46, 3, 0)]);
            assert_eq!(rd::<u8>(m, M::useAmbientDiffuse), 0);
            assert_eq!(rd::<u8>(m, M::lastUseAmbientDiffuse), 0);
            assert_eq!(rd::<u32>(m, M::sourceVertexMode), 0);

            // Emissive lighting mode with emissive vertex colours: the port sets lighting off.
            wr::<u32>(vertex_color, VC::sourceMode, 1);
            set_state(m, state, texturing, vertex_color);
            assert_eq!(calls(), [(50, D3DRS_LIGHTING, 0)]);
            assert_eq!(rd::<u8>(m, M::d3dLightingState), 0);
            set_state(m, state, texturing, vertex_color);
            assert_eq!(calls(), []);

            // A base texture in replace mode: the port sets lighting off for all vertex colour
            // settings.
            wr::<u32>(vertex_color, VC::lightingMode, 1);
            set_state(m, state, texturing, vertex_color);
            assert_eq!(rd::<u8>(m, M::d3dLightingState), 1);
            calls();
            let map = leak(TM::SIZE);
            wr::<u32>(map, TM::texture, 0x1000);
            wr(maps, 0, map);
            wr::<u32>(texturing, TP::applyMode, 0);
            set_state(m, state, texturing, vertex_color);
            assert_eq!(calls(), [(50, D3DRS_LIGHTING, 0)]);
            assert_eq!(rd::<u8>(m, M::d3dLightingState), 0);
            assert_eq!(rd::<u32>(m, M::applyMode), 0);

            // The regions of such a call are the manager and the render state.
            let mut set = Regions::new();
            set_state_regions(&mut set, m, 0, texturing, vertex_color);
            assert_eq!(set.as_slice(), [(m, M::SIZE), (w.render_state, RS::SIZE)]);
        }
    }
}
