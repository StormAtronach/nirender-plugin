//! The two functions through which a shape gets a new property state or effect state:
//! `NiGeometry::UpdatePropertiesDownward` (0x6F41B0) and `NiGeometry::UpdateEffectsDownward`
//! (0x6F4270). The engine draws a shape from these cached states, so a property change
//! reaches the plugin here. When the content of a state changes, the port asks for a recheck
//! of the shape's retained instance at its next display (`instance_stream::recheck`). The
//! engine function runs as it is.

use crate::gen::layouts::{NiDynamicEffectState as DES, NiGeometry as G, NiPropertyState as PS};
use crate::mem::rd;

/// The prints describe what a state holds, not where it lives. The engine often makes a new
/// state with the same content, and a new pointer with the same content needs no recheck.
const PRINT_START: u64 = 0xcbf2_9ce4_8422_2325;

fn mix(print: &mut u64, value: u32) {
    *print = (*print ^ value as u64).wrapping_mul(0x100_0000_01b3);
}

/// The shape's property state: its slots. The engine can update a state in place and keep the
/// pointer.
unsafe fn property_state_print(shape: usize) -> u64 {
    let state: usize = rd(shape, G::spPropertyState);
    if state == 0 {
        return 0;
    }
    let mut print = PRINT_START;
    for at in (PS::propAlpha..=PS::field_30).step_by(4) {
        mix(&mut print, rd::<u32>(state, at));
    }
    print
}

/// The shape's effect state: its lights, its projected lights and shadows, in their order,
/// and its texture effects (environment and fog maps).
unsafe fn effect_state_print(shape: usize) -> u64 {
    let state: usize = rd(shape, G::spEffectState);
    if state == 0 {
        return 0;
    }
    let mut print = PRINT_START;
    for list in [DES::lights, DES::projLights, DES::projShadows] {
        let mut iter: usize = rd(state, list);
        let mut seen = 0;
        while iter != 0 && seen < 256 {
            mix(&mut print, rd::<u32>(iter, crate::gen::layouts::NiDynamicEffectState_InternalList::effect));
            iter = rd(iter, crate::gen::layouts::NiDynamicEffectState_InternalList::next);
            seen += 1;
        }
        mix(&mut print, u32::MAX);
    }
    for at in [DES::environmentMap, DES::fogMap] {
        mix(&mut print, rd::<u32>(state, at));
    }
    print
}

use crate::main_thread::MainThread;

port_thiscall!(update_properties_downward_port, 0x6F41B0, "NiGeometry::UpdatePropertiesDownward",
    fn(this, state: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust_main = update_properties_downward);

/// The engine's update of the shape's property state; a new content asks for a recheck.
///
/// # Safety
///
/// `this` points at a live shape whose property state the function reads before and after the
/// engine's update; `state` is the engine's argument, passed on as it is. The port is installed
/// (the function calls its engine function).
pub unsafe fn update_properties_downward(t: &MainThread, this: usize, state: usize) {
    crate::shadow::touch(&update_properties_downward_port::STATE);
    let before = property_state_print(this);
    update_properties_downward_port::engine(this, state);
    if property_state_print(this) != before {
        crate::instance_stream::recheck(t, this, crate::instance_stream::Recheck::Properties);
    }
}

port_thiscall!(update_effects_downward_port, 0x6F4270, "NiGeometry::UpdateEffectsDownward",
    fn(this, state: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust_main = update_effects_downward);

/// The engine's update of the shape's effect state; a new content asks for a recheck. The
/// engine shares an equal state between shapes (`IsEqual` with the last state), so a new
/// pointer can hold the same effects.
///
/// # Safety
///
/// `this` points at a live shape whose effect state the function reads before and after the
/// engine's update; `state` is the engine's argument, passed on as it is. The port is installed
/// (the function calls its engine function).
pub unsafe fn update_effects_downward(t: &MainThread, this: usize, state: usize) {
    crate::shadow::touch(&update_effects_downward_port::STATE);
    let before = effect_state_print(this);
    update_effects_downward_port::engine(this, state);
    if effect_state_print(this) != before {
        crate::instance_stream::recheck(t, this, crate::instance_stream::Recheck::Effects);
    }
}

pub static PORTS: &[&crate::shadow::PortState] = &[
    &update_properties_downward_port::STATE,
    &update_effects_downward_port::STATE,
];
