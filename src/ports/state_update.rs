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

port_thiscall!(update_world_data_port, 0x6F4140, "NiGeometry::UpdateWorldData",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = update_world_data);

/// The engine's update of a shape's world transform (entry 13): every update pass reaches a
/// shape through this function (the vtables of NiTriShape, NiTriStrips and the particles).
/// The shape is noted for the table walk.
pub unsafe fn update_world_data(this: usize) {
    crate::shadow::touch(&update_world_data_port::STATE);
    update_world_data_port::engine(this);
    crate::instance_stream::note_world_data(this);
}

port_thiscall!(set_app_culled_port, 0x442B10, "NiAVObject::SetAppCulled",
    fn(this, flag: u32) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = set_app_culled);

/// The engine's set of the app-cull flag (entry 13): the visibility controller, MWSE's
/// `appCulled` and the engine call it through the vtables. A real change is noted for the
/// table walk.
pub unsafe fn set_app_culled(this: usize, flag: u32) {
    crate::shadow::touch(&set_app_culled_port::STATE);
    let before = rd::<u16>(this, 0x14) & 1;
    set_app_culled_port::engine(this, flag);
    if rd::<u16>(this, 0x14) & 1 != before {
        crate::instance_stream::note_app_culled(this);
    }
}

pub static PORTS: &[&crate::shadow::PortState] = &[
    &update_properties_downward_port::STATE,
    &update_effects_downward_port::STATE,
    &update_world_data_port::STATE,
    &set_app_culled_port::STATE,
];
