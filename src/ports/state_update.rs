//! The two functions through which a shape gets a new property state or effect state (plan
//! 8b, step 2; engine-rules R27): `NiGeometry::UpdatePropertiesDownward` (0x6F41B0) and
//! `NiGeometry::UpdateEffectsDownward` (0x6F4270). The engine draws a shape from these cached
//! states, so a property attached or removed (by a Lua mod, then `updateProperties`) reaches
//! the engine only through them, and so it reaches the plugin here: when the state pointer
//! changes, the shape's retained instance is checked again at its next display
//! (`instance_stream::recheck`). The engine function runs as it is. MWSE wraps one call site
//! of `UpdateProperties` (the map tiles) and patches neither function's body.

use crate::gen::layouts::{NiDynamicEffectState as DES, NiGeometry as G, NiPropertyState as PS};
use crate::mem::rd;

/// The prints describe what a state holds, not where it lives (2026-10-19). The engine makes
/// a new state for every shape below a node at each `UpdateEffects` or `UpdateProperties` of
/// it, most of them equal to the old ones: the visual effects manager updates the effects of
/// the whole scene each time an effect starts or ends (`VFXManager::update` 0x4697E0, the
/// parent of its root), and each of those updates rechecked about 6400 shapes, overflowed the
/// moved marks and made the next walk a full pass of 4 to 5 ms. Nothing reads a state through
/// its pointer later (the instance's state is a key of the counts; the lights are counted by
/// the lights themselves), so a new pointer with the same content needs no recheck: the
/// shape's next work takes the new pointer. `NIRENDER_MARK_ONCE=0` restores the pointer in the
/// prints (the A/B).
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
    let mut print = if crate::instance_stream::mark_once_on() { PRINT_START } else { state as u64 };
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
    if !crate::instance_stream::mark_once_on() {
        let mut print = state as u64;
        for at in [DES::environmentMap, DES::fogMap] {
            mix(&mut print, rd::<u32>(state, at));
        }
        return print;
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

/// The new states with the same content as the old ones (no recheck): effects, properties.
pub static POINTER_ONLY: [core::sync::atomic::AtomicU64; 2] = [core::sync::atomic::AtomicU64::new(0), core::sync::atomic::AtomicU64::new(0)];

port_thiscall!(update_properties_downward_port, 0x6F41B0, "NiGeometry::UpdatePropertiesDownward",
    fn(this, state: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = update_properties_downward);

/// The engine's update of the shape's property state; a new state asks for a recheck.
pub unsafe fn update_properties_downward(this: usize, state: usize) {
    crate::shadow::touch(&update_properties_downward_port::STATE);
    let before = property_state_print(this);
    let pointer_before: usize = rd(this, G::spPropertyState);
    update_properties_downward_port::engine(this, state);
    if property_state_print(this) != before {
        crate::instance_stream::recheck(this, crate::instance_stream::Recheck::Properties);
    } else if rd::<usize>(this, G::spPropertyState) != pointer_before {
        POINTER_ONLY[1].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
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
    let before = effect_state_print(this);
    let pointer_before: usize = rd(this, G::spEffectState);
    update_effects_downward_port::engine(this, state);
    if effect_state_print(this) != before {
        crate::instance_stream::recheck(this, crate::instance_stream::Recheck::Effects);
    } else if rd::<usize>(this, G::spEffectState) != pointer_before {
        POINTER_ONLY[0].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }
}

port_thiscall!(update_world_data_port, 0x6F4140, "NiGeometry::UpdateWorldData",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = update_world_data);

/// The engine's update of a shape's world transform (entry 13): every update pass reaches a
/// shape through this function (the vtables of NiTriShape, NiTriStrips and the particles).
/// The shape is marked for the table walk.
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
/// `appCulled` and the engine call it through the vtables. A real change marks the object
/// for the table walk.
pub unsafe fn set_app_culled(this: usize, flag: u32) {
    crate::shadow::touch(&set_app_culled_port::STATE);
    let before = rd::<u16>(this, 0x14) & 1;
    set_app_culled_port::engine(this, flag);
    if rd::<u16>(this, 0x14) & 1 != before {
        crate::instance_stream::note_app_culled(this);
    }
}

port_thiscall!(attach_child_port, 0x6C8410, "NiNode::AttachChild",
    fn(this, child: usize, first_available: u32) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = attach_child);

/// The engine attaches a child (entry 13): the vtables of every node, MWSE's Lua and MWSE's
/// animation manager (its slot calls this function first). The node and the child are
/// marked for the table walk.
pub unsafe fn attach_child(this: usize, child: usize, first_available: u32) {
    crate::shadow::touch(&attach_child_port::STATE);
    attach_child_port::engine(this, child, first_available);
    crate::instance_stream::note_tree(this, child);
}

port_thiscall!(detach_child_port, 0x6C8680, "NiNode::DetachChild",
    fn(this, out: usize, child: usize) -> usize,
    compare_eax = false,
    regions = |set| set.skip(),
    rust = detach_child);

/// The engine detaches a child (entry 13): the node is marked for the table walk.
pub unsafe fn detach_child(this: usize, out: usize, child: usize) -> usize {
    crate::shadow::touch(&detach_child_port::STATE);
    let result = detach_child_port::engine(this, out, child) as usize;
    crate::instance_stream::note_tree(this, 0);
    result
}

port_thiscall!(detach_child_at_port, 0x6C8590, "NiNode::DetachChildAt",
    fn(this, out: usize, index: u32) -> usize,
    compare_eax = false,
    regions = |set| set.skip(),
    rust = detach_child_at);

/// The engine detaches the child at an index (entry 13): the node is marked.
pub unsafe fn detach_child_at(this: usize, out: usize, index: u32) -> usize {
    crate::shadow::touch(&detach_child_at_port::STATE);
    let result = detach_child_at_port::engine(this, out, index) as usize;
    crate::instance_stream::note_tree(this, 0);
    result
}

port_thiscall!(set_child_at_port, 0x6C8780, "NiNode::SetChildAt",
    fn(this, out: usize, index: u32, child: usize) -> usize,
    compare_eax = false,
    regions = |set| set.skip(),
    rust = set_child_at);

/// The engine puts a child at an index (entry 13; MWSE's animation manager calls this
/// function first): the node and the new child are marked.
pub unsafe fn set_child_at(this: usize, out: usize, index: u32, child: usize) -> usize {
    crate::shadow::touch(&set_child_at_port::STATE);
    let result = set_child_at_port::engine(this, out, index, child) as usize;
    crate::instance_stream::note_tree(this, child);
    result
}

port_thiscall!(set_opacity_port, 0x542030, "ActorAnimController::setOpacity",
    fn(this, opacity: u32) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = set_opacity);

/// The actor's controller (`ActorAnimController`): its actor, its material property.
const CONTROLLER_ACTOR: usize = 0x38;
const CONTROLLER_MATERIAL: usize = 0x44;
/// `MobileObject::reference`, `Reference::sceneNode`, `MACP::refr1stPerson`.
const MOBILE_REFERENCE: usize = 0x14;
const REFERENCE_NODE: usize = 0x10;
const PLAYER_FIRST_PERSON: usize = 0x660;
/// `NiMaterialProperty::alpha`.
const MATERIAL_ALPHA: usize = 0x50;

/// The engine sets an actor's opacity (entry 13): the chameleon and invisibility effects,
/// the character image of the inventory, MWSE (its own refresh, the body parts, Lua's
/// `opacity`). It writes the alpha, vertex colour and material properties in place, with no
/// new property state, so no other event sees it. When the alpha changed, the actor's scene
/// node (and the player's first-person node) is marked: the walk checks the shapes below
/// again. The hook reads the live objects of the call and nothing below them.
pub unsafe fn set_opacity(this: usize, opacity: u32) {
    crate::shadow::touch(&set_opacity_port::STATE);
    let material: usize = rd(this, CONTROLLER_MATERIAL);
    let before: u32 = if material != 0 { rd(material, MATERIAL_ALPHA) } else { 0 };
    set_opacity_port::engine(this, opacity);
    let after: u32 = if material != 0 { rd(material, MATERIAL_ALPHA) } else { 0 };
    if before == after {
        return;
    }
    let actor: usize = rd(this, CONTROLLER_ACTOR);
    let reference: usize = if actor != 0 { rd(actor, MOBILE_REFERENCE) } else { 0 };
    if reference != 0 {
        crate::instance_stream::note_material_subtree(rd(reference, REFERENCE_NODE));
    }
    if actor != 0 && actor == crate::engine::player_mobile() {
        let first: usize = rd(actor, PLAYER_FIRST_PERSON);
        if first != 0 {
            crate::instance_stream::note_material_subtree(rd(first, REFERENCE_NODE));
        }
    }
}

pub static PORTS: &[&crate::shadow::PortState] = &[
    &update_properties_downward_port::STATE,
    &update_effects_downward_port::STATE,
    &update_world_data_port::STATE,
    &set_app_culled_port::STATE,
    &attach_child_port::STATE,
    &detach_child_port::STATE,
    &detach_child_at_port::STATE,
    &set_child_at_port::STATE,
    &set_opacity_port::STATE,
];
