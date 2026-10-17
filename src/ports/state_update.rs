//! The two functions through which a shape gets a new property state or effect state (plan
//! 8b, step 2; engine-rules R27): `NiGeometry::UpdatePropertiesDownward` (0x6F41B0) and
//! `NiGeometry::UpdateEffectsDownward` (0x6F4270). The engine draws a shape from these cached
//! states, so a property attached or removed (by a Lua mod, then `updateProperties`) reaches
//! the engine only through them, and so it reaches the plugin here: when the state pointer
//! changes, the shape's retained instance is checked again at its next display
//! (`instance_stream::recheck`). The engine function runs as it is. MWSE wraps one call site
//! of `UpdateProperties` (the map tiles) and patches neither function's body.

use crate::gen::layouts::NiGeometry as G;
use crate::mem::rd;

port_thiscall!(update_properties_downward_port, 0x6F41B0, "NiGeometry::UpdatePropertiesDownward",
    fn(this, state: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = update_properties_downward);

/// The engine's update of the shape's property state; a new state asks for a recheck.
pub unsafe fn update_properties_downward(this: usize, state: usize) {
    crate::shadow::touch(&update_properties_downward_port::STATE);
    let before: usize = rd(this, G::spPropertyState);
    update_properties_downward_port::engine(this, state);
    if rd::<usize>(this, G::spPropertyState) != before {
        crate::instance_stream::recheck(this, crate::instance_stream::Recheck::Properties);
    }
}

port_thiscall!(update_effects_downward_port, 0x6F4270, "NiGeometry::UpdateEffectsDownward",
    fn(this, state: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = update_effects_downward);

/// The engine's update of the shape's effect state; a new state asks for a recheck. The
/// engine shares an equal state between shapes (`IsEqual` with the last state), so a new
/// pointer can hold the same effects: the recheck then notes the same material again.
pub unsafe fn update_effects_downward(this: usize, state: usize) {
    crate::shadow::touch(&update_effects_downward_port::STATE);
    let before: usize = rd(this, G::spEffectState);
    update_effects_downward_port::engine(this, state);
    if rd::<usize>(this, G::spEffectState) != before {
        crate::instance_stream::recheck(this, crate::instance_stream::Recheck::Effects);
    }
}

pub static PORTS: &[&crate::shadow::PortState] = &[
    &update_properties_downward_port::STATE,
    &update_effects_downward_port::STATE,
];
