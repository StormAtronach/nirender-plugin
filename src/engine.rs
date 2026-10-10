//! Engine functions and data the ports use at their fixed addresses in Morrowind.exe.
//!
//! A function here is outside the port (the scene graph, the allocator) or not ported yet.

use core::mem::transmute;

use crate::mem::rd;

/// `TES3::WorldController*`
pub const WORLD_CONTROLLER: usize = 0x7C67DC;
/// `WorldController::renderer`
pub const WORLD_CONTROLLER_RENDERER: usize = 0x30;
/// Initial values of the 32 texture stage states.
pub const TEXTURE_STAGE_STATE_INITIAL: usize = 0x7C3C58;

unsafe fn thiscall0(addr: usize, this: usize) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize) -> u32>(addr)(this)
}

unsafe fn thiscall1(addr: usize, this: usize, a: u32) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32) -> u32>(addr)(this, a)
}

/// The renderer the game draws with, or 0 before it exists.
pub unsafe fn renderer() -> usize {
    let world_controller: usize = rd(WORLD_CONTROLLER, 0);
    if world_controller == 0 {
        0
    } else {
        rd(world_controller, WORLD_CONTROLLER_RENDERER)
    }
}

/// `operator new` of the game's C runtime.
pub unsafe fn operator_new(size: usize) -> usize {
    transmute::<usize, unsafe extern "C" fn(usize) -> usize>(0x727692)(size)
}

pub unsafe fn property_state_global_shade_property(property_state: usize) -> usize {
    thiscall0(0x6FE500, property_state) as usize
}

pub unsafe fn property_state_global_dither_property(property_state: usize) -> usize {
    thiscall0(0x6FE4E0, property_state) as usize
}

pub unsafe fn property_state_global_specular_property(property_state: usize) -> usize {
    thiscall0(0x6FE510, property_state) as usize
}

/// Returns the material of the property state, after it moved a pending change of the
/// material into its revision number.
pub unsafe fn property_state_update_current_material_revision(property_state: usize) -> usize {
    thiscall0(0x6FE4C0, property_state) as usize
}

/// `NiDX8MaterialManager::UpdateToDevice`. The Morrowind Code Patch changes its body, so it
/// stays engine code.
pub unsafe fn material_manager_update_to_device(manager: usize, device: usize) -> u32 {
    thiscall1(0x6C2680, manager, device as u32)
}

/// `IDirect3DDevice8::SetRenderState` through the vtable of the device, as the game code calls it.
#[inline(always)]
pub unsafe fn device_set_render_state(device: usize, state: u32, value: u32) -> u32 {
    let vtable: usize = rd(device, 0);
    let f: unsafe extern "system" fn(usize, u32, u32) -> u32 = transmute(rd::<usize>(vtable, 0xC8));
    f(device, state, value)
}

/// `IDirect3DDevice8::SetTextureStageState` through the vtable of the device.
#[inline(always)]
pub unsafe fn device_set_texture_stage_state(device: usize, stage: u32, state: u32, value: u32) -> u32 {
    let vtable: usize = rd(device, 0);
    let f: unsafe extern "system" fn(usize, u32, u32, u32) -> u32 = transmute(rd::<usize>(vtable, 0xFC));
    f(device, stage, state, value)
}
