//! The retained instances of the plugin (plan 0.5 entry T3): a shape of the safe class gets
//! an `INSTANCE` record once (its handles, its world transform with an exact translation,
//! the colours of its material), and then an `INSTANCE_DRAW` record at each draw (its id,
//! the ambient light, the fog of the draw and the keys of its lights) in the place of the
//! shape record.
//! The helper draws the instance from its table. A change of the instance (its transform,
//! its colours, its handles) sends the `INSTANCE` record again with the same id; the
//! destructor of the geometry sends `INSTANCE_DELETE`.
//!
//! The safe class (the census of entry T0): a triangle shape of the world scene under the
//! object root or the landscape root (entry T6), not skinned, not from the sorted queue, not blended, without a texture
//! effect, without projected lights or shadows, and under no sort adjust node (the other
//! node kinds that change what or how they draw need nothing of the table:
//! `REJECTED_KINDS`). A controller on a node above moves the shape, and the instance moves
//! in place. A controller on the shape itself or on one of its properties makes it a live
//! shape (a morpher, a UV controller, a flip, alpha or colour controller): its display
//! runs the controllers' `OnPreDisplay` as the engine's display does and notes the
//! geometry and the material every time, so a new revision of the data goes out at once
//! under the same handle (`NIRENDER_CONTROLLERS=0` rejects them, the rule before). The
//! class of a shape is decided once and kept until the shape is deleted.
//!
//! The draw of a retained shape skips the notes of the geometry and the material and the
//! object record (`link_client::send_draw`): their cost was the point. Every 64th frame of
//! an instance the notes run and their handles are compared with the retained ones, so a
//! changed material or geometry shows within a second. When a device texture goes away
//! (the destructor of a source texture's renderer data, the recreation of the texture
//! manager; a capture starts with such a purge) every instance takes the full path at its
//! next draw: the material handle has the device texture, and the game makes a new one.
//! The world transform and the colours are compared at each draw: they are words of the
//! shape, cheap to read.
//!
//! The modes: `off` (no instance), `dual` (every record as before, plus the instance
//! records with the shape record marked as the twin of an instance draw: the helper draws
//! one of the two, `MGE_XE_RETAINED`, for the test of the two paths on one capture), `on`
//! (the instance records in the place of the shape records), `gpu` (plan 0.5 entry T4:
//! the helper draws the instance from its table, with its bound for the cull of a view;
//! the plugin sends no draw, only the changes: the lights of the instance
//! (`INSTANCE_LIGHTS`) and its visibility (`INSTANCE_STATE`, from the draws of the engine:
//! an instance in the view of the camera of the world that the engine did not draw is
//! hidden, one that it drew is visible). `NIRENDER_RETAINED` sets the mode at the start
//! (default `gpu` since T5); `nirender.retained(mode)` sets it from Lua.
//!
//! In mode gpu the engine does not display a retained shape (plan 0.5 entry T5): the shape
//! carries the flag `RETAINED_BIT`, and the ports of its `Display` call `skipped_display`
//! instead, which does what the draw did (the check of the handles when due, of the
//! transform, the bound and the colours, the lights of the shape's effect state as the
//! light manager would turn them on, the visibility) and returns. The engine's traversal
//! still reaches the shape (its parents are displayed and its bound is tested), so the
//! draws of the engine stay the measure of its visibility. `NIRENDER_SKIP=0` keeps the
//! display of the engine (the draw of the stream then does the checks).
//!
//! The record of an instance carries two translations: the stored one of the shape's world
//! transform (the engine's 32-bit floats) and the exact one of the camera-relative matrices
//! (`camera_space::exact_translation`, 64-bit from the scene graph). The own host reports
//! the origin of each scene since 2026-10-16, so the draws of the game are relative to the
//! camera and the records say so (flag 1). The helper takes the exact translation for a
//! relative record and the stored one for the others. For the draws of the stream it
//! subtracts the origin of the scene, as the plugin does: the picture is the same to the
//! bit. For its own table it subtracts an origin of its own that follows the camera. A parent
//! that moves by less than the step of a 32-bit float at the position of the shape leaves
//! the stored translation as it was and is not seen: a difference under one step of a
//! float, not checked.

use std::collections::{HashMap, HashSet};

use crate::capture;
use crate::engine::{thiscall1, vcall0};
use crate::gen::layouts::{NiCamera as C, NiDX8LightManager_LightEntry as E, NiDynamicEffectState as DES, NiDynamicEffectState_InternalList as DL, NiLight as L, NiPropertyState as PS, NiTimeController as TC, NiZBufferProperty as ZP};
use crate::link::{CLASS_LANDSCAPE, CLASS_OBJECT, CLASS_UNKNOWN, SCENE_WORLD};
use crate::mem::{rd, wr};

/// `NiObjectNET::controllers`, `NiAVObject::parentNode`, `NiAVObject::worldTransform` and
/// the translation in a transform.
const CONTROLLERS: usize = 0x10;
const PARENT: usize = 0x18;
/// `NiTextureEffect::coordinateMode` (see `texture_pass_b`).
const EFFECT_COORDINATE_MODE: usize = 0x118;
const WORLD: usize = 0x40;
const WORLD_TRANSLATION: usize = WORLD + 0x24;
/// `NiAVObject::worldBound`: the centre and the radius.
const WORLD_BOUND: usize = 0x1c;
/// `NiBound::WhichSide(plane)`: 0 across the plane, 1 in front, 2 behind (out of the view).
const WHICH_SIDE: usize = 0x6F2660;
/// The flag of an instance record of the mode gpu: the helper draws it from its table.
const BLOCK: u32 = 0x100;
/// `NiAVObject::flags` bit 12: the plugin's mark on a shape the helper draws from its table.
pub const RETAINED_BIT: u16 = 0x1000;
const FLAGS: usize = 0x14;
/// `NiGeometry::spPropertyState`, `spEffectState`, `spModelData`.
const GEOMETRY_PROPERTY_STATE: usize = 0x90;
const GEOMETRY_EFFECT_STATE: usize = 0x94;
const GEOMETRY_MODEL_DATA: usize = 0x98;
/// The words of a `NiTransform`: the rotation, the translation (from word 9), the scale.
const TRANSFORM_WORDS: usize = 13;
const MOST_LIGHTS: usize = 64;
/// The property pointers of a `NiPropertyState`: alpha first, 12 of them.
const STATE_FIRST: usize = 0x8;
const STATE_COUNT: usize = 12;
/// `NiProperty::flags` (u16); bit 0 of the alpha property is the blend.
const PROPERTY_FLAGS: usize = 0x14;
/// The vtable slot of `NiTimeController::OnPreDisplay`, which the display of a shape calls
/// for each of its controllers before the draw (a morpher writes its vertices then).
const SLOT_ON_PRE_DISPLAY: usize = 0x40;
/// `NiRTTI`: the name, then the base.
const RTTI_NAME: usize = 0x0;
const RTTI_BASE: usize = 0x4;
/// The most nodes above a shape that the walk looks at.
const MAX_DEPTH: usize = 32;
/// The frames between two checks of the handles of an instance.
const CHECK_FRAMES: u32 = 64;

/// The node kinds that keep a shape under them on the engine's path. A sort adjust node
/// draws its subtree in an order of its own (a cluster drawn late, or in place without
/// the accumulator): a retained shape under one came too early and a translucent window
/// drew over it (the NIF test level, 2026-10-16). The other kinds that change what or how
/// they draw need nothing of the table (plan step 5, 2026-10-16), as long as the engine's
/// traversal reaches the shape: a billboard turns its node to the camera in its display,
/// and the display of the shape then sees the moved transform and moves the instance; a
/// switch, a level of detail and a flipbook display one child, so the shapes of the
/// others are never drawn and never enter the table, and one shown before is hidden at
/// the end of the scene like any shape the engine stops drawing (`scene_end`); a BSP node
/// orders its two children, which matters for blended shapes (not retained) and for the
/// ties of the depth test (the open class of the order); an animation node marks itself
/// displayed, and a particle node has no display of its own. `NIRENDER_NODE_KINDS=0`
/// keeps all of them out (the rule before, for an A/B).
const REJECTED_KINDS: [&[u8]; 1] = [b"NiSortAdjustNode"];
/// The rule before plan step 5: every kind that changes what or how it draws.
const REJECTED_KINDS_BEFORE: [&[u8]; 7] = [b"NiBillboardNode", b"NiSwitchNode", b"NiLODNode", b"NiBSPNode", b"NiSortAdjustNode", b"BSAnimationNode", b"BSParticleNode"];
/// The rule before, for a skinned shape (an actor): only the kinds that change which
/// shape is drawn; the bones it follows can be animated, and the palette of each draw
/// carries them.
const REJECTED_KINDS_SKINNED_BEFORE: [&[u8]; 5] = [b"NiBillboardNode", b"NiSwitchNode", b"NiLODNode", b"NiBSPNode", b"NiSortAdjustNode"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Off,
    Dual,
    On,
    Gpu,
}

impl Mode {
    pub fn parse(text: &str) -> Option<Mode> {
        match text {
            "off" => Some(Mode::Off),
            "dual" => Some(Mode::Dual),
            "on" => Some(Mode::On),
            "gpu" => Some(Mode::Gpu),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Dual => "dual",
            Mode::On => "on",
            Mode::Gpu => "gpu",
        }
    }
}

/// The reasons a shape is not retained, for the report.
const REASON_CONTROLLER: usize = 0;
const REASON_NODE_KIND: usize = 1;
const REASON_PROPERTY_CONTROLLER: usize = 2;
const REASON_BLEND: usize = 3;
const REASON_EFFECT: usize = 4;
const REASON_LATER: usize = 5;
/// The depth test of the shape is equal, greater, not equal or greater or equal: the draw
/// depends on the depth of a draw before it, so its place in the order of the game matters.
const REASON_DEPTH: usize = 6;
const REASONS: usize = 7;

struct Instance {
    id: u32,
    class: u32,
    geometry: u32,
    material: u32,
    /// The rows 0 to 2 of the world matrix of the draw (the rotation with the scale).
    rotation: [u32; 12],
    /// The stored translation of the world transform of the shape, as bits.
    stored: [u32; 3],
    /// The colours of the material, as the shape record has them.
    colours: [u32; 14],
    /// The world bound of the shape: the centre and the radius, and its world transform.
    bound: [f32; 4],
    transform: [u32; TRANSFORM_WORDS],
    /// The keys of the lights the helper has for the instance (mode gpu).
    lights: Vec<u32>,
    /// The frame of the last draw of the engine, and whether the helper shows the instance.
    drawn: u32,
    visible: bool,
    /// The `INSTANCE` record with these values went into the stream or the file.
    sent: bool,
    /// The values above were taken from a draw.
    filled: bool,
    /// The generation of the tables at the last check of the handles.
    generation: u32,
    /// A skinned shape (an actor): its draw carries the palette of its bones each frame, so
    /// it stays on the draw records (never in the helper's table, never skipped).
    skinned: bool,
    /// A controller on the shape or on one of its properties changes its geometry or its
    /// material (a morpher, a UV controller, a flip, alpha or colour controller): the
    /// display runs the controllers' `OnPreDisplay` as the engine's display does, and
    /// notes the geometry and the material every time.
    live: bool,
}

struct Tables {
    instances: HashMap<usize, Instance>,
    rejected: HashSet<usize>,
    next_id: u32,
    frame: u32,
    /// Grows when the handles of every instance need a check: the next draw of each takes
    /// the full path once.
    generation: u32,
    /// The draw in progress is of a retained shape: the shape, its id, and whether the
    /// notes of the draw ran (the handles of the draw are then those of `shape_stream`).
    pending: Option<(usize, u32)>,
    mode: Option<Mode>,
    /// The camera of the world scene in progress, for the check of the visibility at its
    /// end (mode gpu).
    world_camera: usize,
    states: u64,
    light_records: u64,
    /// The displays the engine skipped, and the device values of the lights of the frame
    /// (built once a frame for each light, as the light manager builds its entry).
    skipped: u64,
    skip: bool,
    /// The frame of the last `RETAINED_SCENE` record and its values; the skipped shapes
    /// whose own ambient light or fog differed from them.
    scene_frame: u32,
    scene_values: (u32, Option<[u32; 4]>),
    scene_mixed: u64,
    /// The landscape (class 11) is retained too (`NIRENDER_LANDSCAPE=0` keeps it out).
    landscape: bool,
    /// The actors (a skinned shape of class 0: the actors hang under no root the classes
    /// know) are retained too (`NIRENDER_ACTORS=0` keeps them out).
    actors: bool,
    /// A shape under an animated node, and a shape of class 0 that is not skinned, are
    /// retained too (`NIRENDER_ANIMATED=0` keeps them out: the state before, for an A/B).
    animated: bool,
    /// A shape under a billboard, a switch, a level of detail, a BSP, an animation or a
    /// particle node is retained too (`NIRENDER_NODE_KINDS=0` keeps them out: the rule
    /// before, for an A/B). See `REJECTED_KINDS`.
    node_kinds: bool,
    /// A shape with a controller on itself or on a property is retained as a live shape
    /// (`NIRENDER_CONTROLLERS=0` rejects it: the rule before, for an A/B).
    controllers: bool,
    /// The frame each light (by its index) was noted last: its values are made and its LIGHT
    /// record sent at most once a frame, whatever number of shapes it lights.
    light_noted: Vec<u32>,
    scratch_entry: [u32; E::SIZE / 4],
    scratch_lights: Vec<usize>,
    /// The keys of the lights of the shape in progress, kept between the shapes so that a
    /// display does not allocate.
    scratch_keys: Vec<u32>,
    fast_draws: u64,
    records: u64,
    /// The instance records sent again: for changed handles, and for changed values of
    /// the draw (the transform, the colours).
    handle_updates: u64,
    value_updates: u64,
    deletes: u64,
    rejected_by: [u64; REASONS],
    /// The draws that `consider` did not take, by the first cause: not a plain world draw
    /// of a retained class, no geometry handle, no material handle, the path off, the
    /// material not modelled.
    skipped_by: [u64; 5],
}

static mut TABLES: Option<Tables> = None;

unsafe fn tables() -> &'static mut Tables {
    (*core::ptr::addr_of_mut!(TABLES)).get_or_insert_with(|| Tables {
        instances: HashMap::new(),
        rejected: HashSet::new(),
        next_id: 1,
        frame: 0,
        generation: 1,
        pending: None,
        mode: None,
        world_camera: 0,
        states: 0,
        light_records: 0,
        skipped: 0,
        skip: std::env::var("NIRENDER_SKIP").map_or(true, |v| v != "0"),
        scene_frame: u32::MAX,
        scene_values: (0, None),
        scene_mixed: 0,
        landscape: std::env::var("NIRENDER_LANDSCAPE").map_or(true, |v| v != "0"),
        actors: std::env::var("NIRENDER_ACTORS").map_or(true, |v| v != "0"),
        animated: std::env::var("NIRENDER_ANIMATED").map_or(true, |v| v != "0"),
        node_kinds: std::env::var("NIRENDER_NODE_KINDS").map_or(true, |v| v != "0"),
        controllers: std::env::var("NIRENDER_CONTROLLERS").map_or(true, |v| v != "0"),
        light_noted: Vec::new(),
        scratch_entry: [0; E::SIZE / 4],
        scratch_lights: Vec::new(),
        scratch_keys: Vec::new(),
        fast_draws: 0,
        records: 0,
        handle_updates: 0,
        value_updates: 0,
        deletes: 0,
        rejected_by: [0; REASONS],
        skipped_by: [0; 5],
    })
}

/// The mode, from `NIRENDER_RETAINED` at the first call.
pub unsafe fn mode() -> Mode {
    let t = tables();
    *t.mode.get_or_insert_with(|| std::env::var("NIRENDER_RETAINED").ok().and_then(|v| Mode::parse(&v)).unwrap_or(Mode::Gpu))
}

/// Sets the mode and returns the one before. A change of the mode sends the `INSTANCE`
/// record of each shape again at its next draw: the record says which mode draws it.
pub unsafe fn set_mode(mode: Mode) -> Mode {
    let before = self::mode();
    let t = tables();
    t.mode = Some(mode);
    if mode != before {
        for (&shape, instance) in t.instances.iter_mut() {
            instance.sent = false;
            instance.lights.clear();
            if mode != Mode::Gpu {
                mark(shape, false);
            }
        }
    }
    before
}

/// True for a class of object the plugin retains: the objects of the references, the
/// landscape, and class 0 (plan 0.5 entry T6). Class 0 is a shape of the world scene under
/// no root the classes know: the actors and their parts, and objects hung elsewhere (the
/// arms of the first person have a class of their own, and the picture of the character is
/// another scene). A skinned one is an actor (its palette each frame); one that is not
/// skinned follows the rules of any shape (a part under an animated bone has a controller
/// above it and stays out; a still object is retained; one that moves without a controller
/// is moved in place). `NIRENDER_ACTORS=0` keeps class 0 out.
pub unsafe fn class_retained(class: u32, skinned: bool) -> bool {
    let t = tables();
    class == CLASS_OBJECT || (class == CLASS_LANDSCAPE && t.landscape) || (class == CLASS_UNKNOWN && t.actors && (skinned || t.animated))
}

/// Sets or clears the mark of a retained shape.
unsafe fn mark(shape: usize, on: bool) {
    let flags: u16 = rd(shape, FLAGS);
    wr::<u16>(shape, FLAGS, if on { flags | RETAINED_BIT } else { flags & !RETAINED_BIT });
}

/// True when the retained path is in use: a mode other than off, and the matrices of the
/// draws are made by the plugin (the exact translations exist then).
unsafe fn enabled() -> bool {
    mode() != Mode::Off && crate::camera_space::own()
}

/// Forgets which instances were sent: for the start of a capture file from the stream, so
/// that the next draw of each sends its `INSTANCE` record into the file. The capture purges
/// the source textures too, so the handles of every instance get a check.
pub unsafe fn reset() {
    let t = tables();
    t.generation = t.generation.wrapping_add(1);
    for instance in t.instances.values_mut() {
        instance.sent = false;
        instance.lights.clear();
    }
    // The LIGHT records go again (`light_stream::reset`): the lights are noted again too.
    t.light_noted.clear();
}

/// A scene of the game begins: the camera of the world scene is kept for the check at its
/// end.
pub unsafe fn scene_begin(kind: u32, camera: usize) {
    tables().world_camera = if kind == SCENE_WORLD { camera } else { 0 };
}

/// A scene of the game ends. At the end of the world scene in mode gpu: an instance the
/// helper shows that the engine did not draw although it is in the view of the camera is
/// hidden (`INSTANCE_STATE`); an instance the engine drew was made visible at its draw.
/// An instance whose record was not sent again since a reset or a change of the mode is
/// hidden too: the helper's table keeps it from before (a capture that holds resets the
/// records at each release; the far level of a level-of-detail node stayed in the table
/// at the stops of the NIF tour, 2026-10-16).
pub unsafe fn scene_end() {
    let t = tables();
    let camera = core::mem::replace(&mut t.world_camera, 0);
    if camera == 0 || mode() != Mode::Gpu || !capture::recording() {
        return;
    }
    let count = rd::<i32>(camera, C::countCullingPlanes).max(0) as usize;
    let storage: usize = rd(camera, C::cullingPlanes_storage);
    let planes: Vec<usize> = (0..count).map(|i| rd(storage, i * 4)).collect();
    let frame = t.frame;
    for instance in t.instances.values_mut() {
        if !instance.filled || !instance.visible || instance.drawn == frame {
            continue;
        }
        let bound = instance.bound.as_ptr() as usize;
        let in_view = planes.iter().all(|&plane| thiscall1(WHICH_SIDE, bound, plane as u32) != 2);
        if in_view {
            instance.visible = false;
            capture::instance_state(instance.id, false);
            t.states += 1;
        }
    }
}

/// A device texture went away: the material handle of an instance can be stale. The next
/// draw of each instance notes its material and geometry again.
pub unsafe fn textures_changed() {
    let t = tables();
    t.generation = t.generation.wrapping_add(1);
}

/// The start of a draw of the engine: a pending draw that no shape record took (a skinned
/// shape whose palette failed, or one skinned in software) must not reach the next record.
pub unsafe fn begin_draw() {
    tables().pending = None;
}

/// The end of a frame.
pub unsafe fn frame_tick() {
    let t = tables();
    t.frame = t.frame.wrapping_add(1);
}

/// The shape is deleted (the destructor of `NiGeometry`): its instance goes, with a record
/// for the helper when the helper had it.
pub unsafe fn forget(shape: usize) {
    let t = tables();
    t.rejected.remove(&shape);
    if let Some(instance) = t.instances.remove(&shape) {
        if instance.sent && capture::recording() {
            capture::instance_delete(instance.id);
        }
        t.deletes += 1;
    }
}

/// The fast path of a draw, before the notes of the draw: true when the shape is a
/// retained instance whose handles need no check this frame. The handles of the draw are
/// then those of the instance, and the draw in progress is pending for `draw`.
pub unsafe fn seen(shape: usize, kind: u32, class_out: &mut u32) -> Option<(u32, u32)> {
    if !matches!(mode(), Mode::On | Mode::Gpu) {
        return None;
    }
    let t = tables();
    let instance = t.instances.get(&shape)?;
    if !instance.filled || !instance.sent || instance.live || instance.generation != t.generation || instance.id.wrapping_add(t.frame) % CHECK_FRAMES == 0 {
        return None;
    }
    let _ = kind;
    *class_out = instance.class;
    t.pending = Some((shape, instance.id));
    t.fast_draws += 1;
    Some((instance.geometry, instance.material))
}

/// After the notes of a draw: decides if the shape is retained, and makes the draw in
/// progress pending for `draw` when it is. `property_state` and `effect_state` are those
/// of the renderer for the draw.
pub unsafe fn consider(shape: usize, eligible: bool, skinned: bool, class: u32, geometry: u32, material: u32, property_state: usize, effect_state: usize) {
    let cause = if !eligible {
        Some(0)
    } else if geometry == 0 {
        Some(1)
    } else if material == 0 {
        Some(2)
    } else if !enabled() {
        Some(3)
    } else if !crate::material_stream::last_modelled() {
        Some(4)
    } else {
        None
    };
    let t = tables();
    if let Some(cause) = cause {
        t.skipped_by[cause] += 1;
        return;
    }
    if let Some(instance) = t.instances.get_mut(&shape) {
        instance.generation = t.generation;
        if instance.geometry != geometry || instance.material != material || instance.class != class {
            instance.geometry = geometry;
            instance.material = material;
            instance.class = class;
            instance.sent = false;
            t.handle_updates += 1;
        }
        t.pending = Some((shape, instance.id));
        return;
    }
    if t.rejected.contains(&shape) {
        return;
    }
    let mut live = false;
    if let Some(reason) = reject_reason(shape, property_state, effect_state, skinned, &mut live) {
        t.rejected.insert(shape);
        t.rejected_by[reason] += 1;
        return;
    }
    let id = t.next_id;
    t.next_id = t.next_id.wrapping_add(1).max(1);
    let generation = t.generation;
    t.instances.insert(shape, Instance { id, class, geometry, material, rotation: [0; 12], stored: [0; 3], colours: [0; 14], bound: [0.0; 4], transform: [0; TRANSFORM_WORDS], lights: Vec::new(), drawn: t.frame, visible: true, sent: false, filled: false, generation, skinned, live });
    t.pending = Some((shape, id));
}

/// Why the shape is not retained, or `None` when it is of the safe class. `live` is set
/// when a controller on the shape or on a property changes it (plan step 5, the morphers).
unsafe fn reject_reason(shape: usize, property_state: usize, effect_state: usize, skinned: bool, live: &mut bool) -> Option<usize> {
    // A controller on the shape itself changes it: a morpher writes its vertices at the
    // display (`OnPreDisplay`), a UV controller its texture coordinates at the update, each
    // with a new revision of the data; the display of a live shape runs the controllers
    // and notes the geometry again. A controller on a node above only moves the shape: the
    // engine still updates the world transform of a shape it does not display, a skinned
    // shape follows its bones through the palette, and the display of a retained shape
    // (`skipped_display`) compares the transform each time and moves the instance in place.
    let t = tables();
    if rd::<usize>(shape, CONTROLLERS) != 0 {
        if !t.controllers {
            return Some(REASON_CONTROLLER);
        }
        *live = true;
    }
    let animated = t.animated;
    let mut node: usize = rd(shape, PARENT);
    for _ in 0..MAX_DEPTH {
        if node == 0 {
            break;
        }
        if !animated && !skinned && rd::<usize>(node, CONTROLLERS) != 0 {
            return Some(REASON_CONTROLLER);
        }
        if kind_rejected(node, skinned) {
            return Some(REASON_NODE_KIND);
        }
        node = rd(node, PARENT);
    }
    if property_state != 0 {
        for slot in 0..STATE_COUNT {
            let property: usize = rd(property_state, STATE_FIRST + slot * 4);
            if property != 0 && rd::<usize>(property, CONTROLLERS) != 0 {
                // A flip, alpha or colour controller changes the material record: live.
                if !t.controllers {
                    return Some(REASON_PROPERTY_CONTROLLER);
                }
                *live = true;
            }
        }
        let alpha: usize = rd(property_state, STATE_FIRST);
        if alpha != 0 && rd::<u16>(alpha, PROPERTY_FLAGS) & 1 != 0 {
            return Some(REASON_BLEND);
        }
        // The test function of NiZBufferProperty, the enum of NetImmerse (not D3DCMPFUNC):
        // 0 always, 1 less, 2 equal, 3 less or equal, 4 greater, 5 not equal, 6 greater or
        // equal, 7 never. The first wrong form of this rule took 3 for "equal" and rejected
        // nearly every shape (2026-10-16). With the test off (flags bit 0) the engine uses
        // "always" whatever the function says, so the function counts only with the test on.
        let z: usize = rd(property_state, PS::propZBuffer);
        if z != 0 && rd::<u16>(z, PROPERTY_FLAGS) & 1 != 0 && !matches!(rd::<u32>(z, ZP::testFunction), 0 | 1 | 3 | 7) {
            return Some(REASON_DEPTH);
        }
    }
    // A texture effect is retained when the table can make its stage from the material
    // record: a parallel, a perspective or a sphere map (modes 0 to 2; a cube map needs a
    // cube texture) whose world projection does not change (no controller on the effect or
    // on a node above it: the record has the projection, and a change of it would send the
    // record again).
    if effect_state != 0 {
        for effect in [rd::<usize>(effect_state, DES::environmentMap), rd::<usize>(effect_state, DES::fogMap)] {
            if effect != 0 && !effect_retained(effect) {
                return Some(REASON_EFFECT);
            }
        }
    }
    None
}

/// True when the table can draw the texture effect: see `reject_reason`.
unsafe fn effect_retained(effect: usize) -> bool {
    if rd::<u32>(effect, EFFECT_COORDINATE_MODE) > 2 {
        return false;
    }
    let mut node = effect;
    for _ in 0..MAX_DEPTH {
        if node == 0 {
            return true;
        }
        if rd::<usize>(node, CONTROLLERS) != 0 {
            return false;
        }
        node = rd(node, PARENT);
    }
    false
}

/// True when the run-time type of the node, or a base of it, is one of the rejected kinds
/// (with the rule before, the narrower list for a skinned shape).
unsafe fn kind_rejected(node: usize, skinned: bool) -> bool {
    let kinds: &[&[u8]] = if tables().node_kinds {
        &REJECTED_KINDS
    } else if skinned {
        &REJECTED_KINDS_SKINNED_BEFORE
    } else {
        &REJECTED_KINDS_BEFORE
    };
    let mut rtti = crate::ports::ni::get_rtti(node);
    for _ in 0..16 {
        if rtti == 0 {
            return false;
        }
        let name: usize = rd(rtti, RTTI_NAME);
        if name != 0 {
            let text = core::ffi::CStr::from_ptr(name as *const core::ffi::c_char).to_bytes();
            if kinds.iter().any(|kind| text.ends_with(kind)) {
                return true;
            }
        }
        rtti = rd(rtti, RTTI_BASE);
    }
    false
}

/// Takes the draw in progress when it is of a retained shape.
pub unsafe fn take_pending() -> Option<(usize, u32)> {
    tables().pending.take()
}

/// The causes of a rejection at the record, for the report: no geometry handle, no material
/// handle, a material the helper does not model, no exact translation.
static mut LATER_BY: [u32; 4] = [0; 4];
const LATER_GEOMETRY: usize = 0;
const LATER_MATERIAL: usize = 1;
const LATER_NOT_MODELLED: usize = 2;
const LATER_TRANSLATION: usize = 3;

/// `reject_pending` with the cause counted.
unsafe fn reject_at_record(shape: usize, cause: usize) {
    (*core::ptr::addr_of_mut!(LATER_BY))[cause] += 1;
    reject_pending(shape, true);
}

/// The shape of a pending draw turned out not retainable at the record (a kind of draw
/// other than a shape, a picture of the Code Patch, no handle): it goes to the rejected set.
pub unsafe fn reject_pending(shape: usize, reason_later: bool) {
    let t = tables();
    mark(shape, false);
    if let Some(instance) = t.instances.remove(&shape) {
        if instance.sent && capture::recording() {
            capture::instance_delete(instance.id);
        }
    }
    t.rejected.insert(shape);
    if reason_later {
        t.rejected_by[REASON_LATER] += 1;
    }
}

/// The records of a draw of a retained shape: the `INSTANCE` record when the values of the
/// draw differ from the retained ones or the record was not sent, then `INSTANCE_DRAW`.
/// `world` is the world matrix of the draw as the renderer has it (relative to the origin
/// of the scene when the plugin makes the matrices: `relative` is the flag of the shape
/// record), `fog` the fog of the draw.
/// Returns false when the shape has no exact translation (the draw then goes as a shape
/// record, and the shape is rejected).
pub unsafe fn draw(shape: usize, id: u32, world: &[f32; 16], relative: bool, colours: &[u32; 14], fog: Option<[u32; 4]>, ambient: u32, lights: &[u32], bones: &[u8], bone_count: u32) -> bool {
    let t = tables();
    let Some(instance) = t.instances.get_mut(&shape) else { return false };
    // A skinned shape stays on the draw records in every mode: its palette changes each
    // frame. Its world matrix is the identity (the palette has the transforms), so only its
    // colours and its first record send the instance again.
    let skinned = instance.skinned;
    let gpu = t.mode == Some(Mode::Gpu) && !skinned;
    let rotation: [u32; 12] = core::array::from_fn(|i| world[i].to_bits());
    let stored: [u32; 3] = if skinned { [0; 3] } else { rd(shape, WORLD_TRANSLATION) };
    let bound: [f32; 4] = if skinned { [0.0; 4] } else { rd(shape, WORLD_BOUND) };
    let changed = !instance.filled || instance.rotation != rotation || instance.stored != stored || instance.colours != *colours || instance.bound.map(f32::to_bits) != bound.map(f32::to_bits);
    if changed || !instance.sent {
        let exact = if skinned {
            [0.0; 3]
        } else {
            let Some(exact) = crate::camera_space::exact_translation(shape, 0) else {
                reject_at_record(shape, LATER_TRANSLATION);
                return false;
            };
            exact
        };
        if instance.filled && changed {
            t.value_updates += 1;
            if t.value_updates <= 4 {
                let what = if instance.rotation != rotation { "rotation" } else if instance.stored != stored { "translation" } else { "colours" };
                crate::log!("instances: the {what} of instance {id} changed (shape 0x{shape:x})");
            }
        }
        instance.rotation = rotation;
        instance.stored = stored;
        instance.colours = *colours;
        instance.bound = bound;
        instance.transform = rd(shape, WORLD);
        instance.filled = true;
        instance.sent = true;
        // The flag as the shape record of the draw has it: the helper takes the translation
        // of the two the same way. In mode gpu the helper draws the instance from its table;
        // a skinned instance has the flag `SKINNED` and an identity world.
        let flags = if relative { crate::shape_stream::RELATIVE } else { 0 } | if gpu { BLOCK } else { 0 } | if skinned { crate::shape_stream::SKINNED } else { 0 };
        let mut words = [0u32; 44];
        words[0] = id;
        words[1] = instance.class;
        words[2] = flags;
        words[3] = instance.geometry;
        words[4] = instance.material;
        words[5..17].copy_from_slice(&rotation);
        for (i, value) in exact.iter().enumerate() {
            let bits = value.to_bits();
            words[17 + i * 2] = bits as u32;
            words[18 + i * 2] = (bits >> 32) as u32;
        }
        words[23..26].copy_from_slice(&stored);
        words[26..40].copy_from_slice(colours);
        words[40..44].copy_from_slice(&bound.map(f32::to_bits));
        let name_at: usize = rd(shape, 8);
        let name: &[u8] = if name_at != 0 { core::ffi::CStr::from_ptr(name_at as *const core::ffi::c_char).to_bytes() } else { &[] };
        capture::instance(&words, &name[..name.len().min(96)]);
        t.records += 1;
    }
    if gpu {
        // The helper draws the instance: the changes of its lights and its visibility. From
        // now on the engine does not display the shape (`skipped_display`).
        if t.skip {
            mark(shape, true);
        }
        if instance.lights != lights {
            instance.lights.clear();
            instance.lights.extend_from_slice(lights);
            capture::instance_lights(id, lights);
            t.light_records += 1;
        }
        instance.drawn = t.frame;
        if !instance.visible {
            instance.visible = true;
            capture::instance_state(id, true);
            t.states += 1;
        }
        return true;
    }
    capture::instance_draw(id, ambient, fog, lights, bones, bone_count);
    true
}

/// Sends the words of the `INSTANCE` record of an instance of the mode gpu.
unsafe fn send_instance(shape: usize, instance: &Instance, exact: [f64; 3], relative: bool) {
    let flags = if relative { crate::shape_stream::RELATIVE } else { 0 } | BLOCK;
    let mut words = [0u32; 44];
    words[0] = instance.id;
    words[1] = instance.class;
    words[2] = flags;
    words[3] = instance.geometry;
    words[4] = instance.material;
    words[5..17].copy_from_slice(&instance.rotation);
    for (i, value) in exact.iter().enumerate() {
        let bits = value.to_bits();
        words[17 + i * 2] = bits as u32;
        words[18 + i * 2] = (bits >> 32) as u32;
    }
    words[23..26].copy_from_slice(&instance.stored);
    words[26..40].copy_from_slice(&instance.colours);
    words[40..44].copy_from_slice(&instance.bound.map(f32::to_bits));
    let name_at: usize = rd(shape, 8);
    let name: &[u8] = if name_at != 0 { core::ffi::CStr::from_ptr(name_at as *const core::ffi::c_char).to_bytes() } else { &[] };
    capture::instance(&words, &name[..name.len().min(96)]);
}

/// The lights of an effect state as the light manager turns them on for a draw: on, bright
/// enough, not an ambient light; in the order of the manager's list (the last of the state
/// first).
unsafe fn state_lights(effect_state: usize, out: &mut Vec<usize>) {
    out.clear();
    if effect_state == 0 {
        return;
    }
    let mut iter: usize = rd(effect_state, DES::lights);
    while iter != 0 && out.len() < MOST_LIGHTS {
        let light: usize = rd(iter, DL::effect);
        iter = rd(iter, DL::next);
        if light == 0 || rd::<u8>(light, L::enabled) == 0 || rd::<f32>(light, L::dimmer) < f32::from_bits(crate::ports::light_manager::MIN_DIMMER) {
            continue;
        }
        if crate::ports::light_manager::light_get_type(light) == 0 {
            continue;
        }
        out.push(light);
    }
    out.reverse();
}

/// The highest light index noted (the indexes of the lights of the game are small numbers).
const MOST_LIGHT_INDEX: usize = 1 << 20;

/// Notes a light of the game once a frame: its device light as the light manager builds its
/// entry, and its LIGHT record when the values changed (`light_stream::note_values`). Keyed
/// by the index of the light, an array and not a hash: a frame notes each light of each
/// retained shape.
unsafe fn note_light_once(t: &mut Tables, light: usize, key: u32) {
    let k = key as usize;
    if k >= MOST_LIGHT_INDEX {
        return;
    }
    if t.light_noted.len() <= k {
        t.light_noted.resize(k + 1, u32::MAX);
    }
    if t.light_noted[k] == t.frame {
        return;
    }
    t.light_noted[k] = t.frame;
    let entry = t.scratch_entry.as_mut_ptr() as usize;
    core::ptr::write_bytes(entry as *mut u8, 0, E::SIZE);
    wr::<u32>(entry, E::revisionId, !rd::<u32>(light, L::revisionID));
    crate::ports::light_manager::light_entry_update(entry, light);
    let values: [u32; 26] = rd(entry, E::d3dLight);
    crate::light_stream::note_values(key, &values);
}

/// The display of a retained shape by the engine (mode gpu): what the draw of the stream
/// did for the instance, without the draw. True when the engine must not display the
/// shape; false when the shape is not retained after all (the mark goes).
pub unsafe fn skipped_display(shape: usize, kind: u32) -> bool {
    let t = tables();
    if t.mode != Some(Mode::Gpu) || !t.skip || !capture::recording() {
        return false;
    }
    let renderer = crate::engine::renderer();
    let (frame, generation) = (t.frame, t.generation);
    let Some(instance) = t.instances.get_mut(&shape) else {
        mark(shape, false);
        return false;
    };
    if !instance.filled || renderer == 0 {
        return false;
    }
    let id = instance.id;
    let property_state: usize = rd(shape, GEOMETRY_PROPERTY_STATE);
    let effect_state: usize = rd(shape, GEOMETRY_EFFECT_STATE);
    // The controllers of a live shape run before its draw, as the engine's display runs
    // them (`NiTriShape::Display`): a morpher writes its vertices then.
    if instance.live {
        let mut controller: usize = rd(shape, CONTROLLERS);
        while controller != 0 {
            vcall0(controller, SLOT_ON_PRE_DISPLAY);
            controller = rd(controller, TC::nextController);
        }
    }
    // The handles, when due: every 64th frame, after a change of the device textures, for
    // a record that must go again (the start of a capture), and at every display of a
    // live shape (a new revision of the data sends the geometry again under its handle).
    let due = instance.live || !instance.sent || instance.generation != generation || id.wrapping_add(frame) % CHECK_FRAMES == 0;
    if due {
        let geometry = crate::geometry_stream::note(kind, rd(shape, GEOMETRY_MODEL_DATA), 0);
        let material = crate::material_stream::note(renderer, property_state, effect_state);
        if geometry == 0 || material == 0 || !crate::material_stream::last_modelled() {
            let cause = if geometry == 0 {
                LATER_GEOMETRY
            } else if material == 0 {
                LATER_MATERIAL
            } else {
                LATER_NOT_MODELLED
            };
            reject_at_record(shape, cause);
            return false;
        }
        instance.generation = generation;
        if instance.geometry != geometry || instance.material != material {
            instance.geometry = geometry;
            instance.material = material;
            instance.sent = false;
            t.handle_updates += 1;
        }
    }
    // The transform (the rows of the world matrix come from the engine's own conversion
    // when it moved), the bound, the colours.
    let transform: [u32; TRANSFORM_WORDS] = rd(shape, WORLD);
    let bound: [f32; 4] = rd(shape, WORLD_BOUND);
    let colours = crate::shape_stream::material_values(property_state);
    let moved = instance.transform != transform;
    let changed = moved || instance.bound.map(f32::to_bits) != bound.map(f32::to_bits) || instance.colours != colours;
    if changed || !instance.sent {
        if moved {
            crate::ports::renderer_frame::write_world_matrix(renderer, shape + WORLD);
            let world = crate::shape_stream::current_world(renderer);
            instance.rotation = core::array::from_fn(|i| world[i].to_bits());
            instance.transform = transform;
            instance.stored = [transform[9], transform[10], transform[11]];
        }
        instance.bound = bound;
        instance.colours = colours;
        let Some(exact) = crate::camera_space::exact_translation(shape, 0) else {
            reject_at_record(shape, LATER_TRANSLATION);
            return false;
        };
        if changed {
            t.value_updates += 1;
        }
        instance.sent = true;
        // The plugin makes the matrices of every draw relative when it owns the camera
        // space: the record says so whether or not a draw happened this frame.
        send_instance(shape, instance, exact, crate::camera_space::own());
        t.records += 1;
    }
    // The values of the frame for the retained draws: the ambient light of this shape's
    // effect state, and the fog of the cell's property (`TES3::Game::fogProperty`, the
    // weather's), as the light manager and the render state would set them. A shape with
    // a fog property of its own carries that fog in its material record. One record in a
    // frame; a shape whose values differ is counted.
    let ambient = crate::ports::light_manager::ambient_of_state(effect_state);
    let cell_fog = crate::engine::game_fog_property();
    let fog = if cell_fog != 0 {
        crate::shape_stream::fog_values_of(renderer, cell_fog)
    } else {
        crate::shape_stream::fog_values(renderer, property_state)
    };
    if t.scene_frame != frame {
        t.scene_frame = frame;
        t.scene_values = (ambient, fog);
        capture::retained_scene(ambient, fog);
    } else if t.scene_values != (ambient, fog) {
        t.scene_mixed += 1;
    }
    // The lights of the effect state of the shape, their device values when they changed,
    // and the keys when the set changed.
    let mut lights = core::mem::take(&mut t.scratch_lights);
    state_lights(effect_state, &mut lights);
    let mut keys = core::mem::take(&mut t.scratch_keys);
    keys.clear();
    for &light in &lights {
        let key: u32 = rd(light, L::index);
        note_light_once(t, light, key);
        keys.push(key);
    }
    t.scratch_lights = lights;
    let Some(instance) = t.instances.get_mut(&shape) else {
        t.scratch_keys = keys;
        return false;
    };
    if instance.lights != keys {
        capture::instance_lights(id, &keys);
        instance.lights.clear();
        instance.lights.extend_from_slice(&keys);
        t.light_records += 1;
    }
    t.scratch_keys = keys;
    instance.drawn = frame;
    if !instance.visible {
        instance.visible = true;
        capture::instance_state(id, true);
        t.states += 1;
    }
    t.skipped += 1;
    true
}

pub unsafe fn report() -> String {
    let t = tables();
    let reasons = ["controller", "node kind", "property controller", "blend", "effect", "at the record", "depth function"];
    let mut rejected: Vec<String> = reasons.iter().zip(t.rejected_by.iter()).filter(|(_, n)| **n != 0).map(|(r, n)| format!("{r} {n}")).collect();
    let later = *core::ptr::addr_of!(LATER_BY);
    if later.iter().any(|&n| n != 0) {
        rejected.push(format!("at the record by cause: geometry {}, material {}, not modelled {}, translation {}", later[0], later[1], later[2], later[3]));
    }
    let causes = ["not a plain world draw of a retained class", "no geometry", "no material", "path off", "not modelled"];
    let skipped: Vec<String> = causes.iter().zip(t.skipped_by.iter()).filter(|(_, n)| **n != 0).map(|(r, n)| format!("{r} {n}")).collect();
    format!(
        "instances: mode {}, {} retained ({} live), {} rejected ({}), draws not taken ({}), {} fast draws, {} displays skipped ({} with other scene values), {} instance records, {} updates of the handles, {} of the values, {} deletes, {} light records, {} state records",
        mode().name(),
        t.instances.len(),
        t.instances.values().filter(|i| i.live).count(),
        t.rejected.len(),
        rejected.join(", "),
        skipped.join(", "),
        t.fast_draws,
        t.skipped,
        t.scene_mixed,
        t.records,
        t.handle_updates,
        t.value_updates,
        t.deletes,
        t.light_records,
        t.states
    )
}
