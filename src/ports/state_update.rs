//! The two functions through which a shape gets a new property state or effect state (plan
//! 8b, step 2; engine-rules R27): `NiGeometry::UpdatePropertiesDownward` (0x6F41B0) and
//! `NiGeometry::UpdateEffectsDownward` (0x6F4270). The engine draws a shape from these cached
//! states, so a property attached or removed (by a Lua mod, then `updateProperties`) reaches
//! the engine only through them, and so it reaches the plugin here: when the content of the
//! state changes, the shape's retained instance is checked again at its next display
//! (`instance_stream::recheck`). The engine function runs as it is. MWSE wraps one call site
//! of `UpdateProperties` (the map tiles) and patches neither function's body.

use crate::gen::layouts::{NiDynamicEffectState as DES, NiGeometry as G, NiPropertyState as PS};
use crate::mem::rd;

/// The prints describe what a state holds, not where it lives (2026-10-19). The engine makes
/// a new state for every shape below a node at each `UpdateEffects` or `UpdateProperties` of
/// it, most of them equal to the old ones: the visual effects manager updates the effects of
/// the whole scene each time an effect starts or ends (`VFXManager::update` 0x4697E0, the
/// parent of its root), about 6400 shapes. Nothing reads a state through its pointer later
/// (the display reads the shape's state again), so a new pointer with the same content needs
/// no recheck.
const PRINT_START: u64 = 0xcbf2_9ce4_8422_2325;

fn mix(print: &mut u64, value: u32) {
    *print = (*print ^ value as u64).wrapping_mul(0x100_0000_01b3);
}

/// The shape's property state: its slots (the engine can update a state in place, keeping
/// the pointer: a property attached by a mod after the first display, entry 13).
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

port_thiscall!(update_properties_downward_port, 0x6F41B0, "NiGeometry::UpdatePropertiesDownward",
    fn(this, state: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = update_properties_downward);

/// The engine's update of the shape's property state; a new content asks for a recheck.
pub unsafe fn update_properties_downward(this: usize, state: usize) {
    crate::shadow::touch(&update_properties_downward_port::STATE);
    let before = property_state_print(this);
    update_properties_downward_port::engine(this, state);
    if property_state_print(this) != before {
        crate::instance_stream::recheck(this, crate::instance_stream::Recheck::Properties);
    }
}

port_thiscall!(update_effects_downward_port, 0x6F4270, "NiGeometry::UpdateEffectsDownward",
    fn(this, state: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = update_effects_downward);

/// The engine's update of the shape's effect state; a new content asks for a recheck. The
/// engine shares an equal state between shapes (`IsEqual` with the last state), so a new
/// pointer can hold the same effects.
pub unsafe fn update_effects_downward(this: usize, state: usize) {
    crate::shadow::touch(&update_effects_downward_port::STATE);
    let before = effect_state_print(this);
    update_effects_downward_port::engine(this, state);
    if effect_state_print(this) != before {
        crate::instance_stream::recheck(this, crate::instance_stream::Recheck::Effects);
    }
}

pub static PORTS: &[&crate::shadow::PortState] = &[
    &update_properties_downward_port::STATE,
    &update_effects_downward_port::STATE,
];
