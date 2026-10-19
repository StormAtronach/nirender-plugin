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
//! object root or the landscape root (entry T6), not skinned, without a texture
//! effect, without projected lights or shadows, and under no sort adjust node (the other
//! node kinds that change what or how they draw need nothing of the table:
//! `REJECTED_KINDS`). A controller on a node above moves the shape, and the instance moves
//! in place. A controller on the shape itself or on one of its properties makes it a live
//! shape (a morpher, a UV controller, a flip, alpha or colour controller): its display
//! runs the controllers' `OnPreDisplay` as the engine's display does and notes the
//! geometry and the material every time, so a new revision of the data goes out at once
//! under the same handle (`NIRENDER_CONTROLLERS=0` rejects them, the rule before). A
//! blended shape is retained too (plan step 6, 2026-10-17): the helper sorts the blends of
//! its table back to front and draws them where the engine flushes its sorted queue; the
//! material record carries the no-sorter bit of the alpha property, and such a blend
//! draws with the opaque in the order of its slot. The first draw of a blended shape is
//! the queue's, so a draw from the sorted queue of a world-camera scene is considered
//! like a draw of the world scene (`link_client::send_draw`). The
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
//! one of the two, `MGE_XE_RETAINED`, for the test of the two paths on one capture), `gpu`
//! (plan 0.5 entry T4:
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

// The tables are keyed by engine pointers: the pointer hasher, not SipHash (`ptr_hash`).
use crate::ptr_hash::{HashMap, HashSet};

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
/// `NiGeometry::spPropertyState`, `spEffectState`, `spModelData`, `skinInstance`.
const GEOMETRY_PROPERTY_STATE: usize = 0x90;
const GEOMETRY_EFFECT_STATE: usize = 0x94;
const GEOMETRY_MODEL_DATA: usize = 0x98;
const GEOMETRY_SKIN: usize = 0x9C;
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
/// Engine-rules R5 (the skill wgpu-renderer-dev).
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
    Gpu,
}

impl Mode {
    pub fn parse(text: &str) -> Option<Mode> {
        match text {
            "off" => Some(Mode::Off),
            "dual" => Some(Mode::Dual),
            "gpu" => Some(Mode::Gpu),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Dual => "dual",
            Mode::Gpu => "gpu",
        }
    }
}

/// The reasons a shape is not retained, for the report.
const REASON_CONTROLLER: usize = 0;
const REASON_NODE_KIND: usize = 1;
const REASON_PROPERTY_CONTROLLER: usize = 2;
const REASON_EFFECT: usize = 3;
const REASON_LATER: usize = 4;
/// The depth test of the shape is equal, greater, not equal or greater or equal: the draw
/// depends on the depth of a draw before it, so its place in the order of the game matters.
const REASON_DEPTH: usize = 5;
const REASONS: usize = 6;

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
    /// The keys of the lights the helper has for the instance (mode gpu), and the ambient
    /// light of its own effect state at the last lights record.
    lights: Vec<u32>,
    ambient: u32,
    /// The frame of the last draw of the engine, and whether the helper shows the instance.
    drawn: u32,
    visible: bool,
    /// The `INSTANCE` record with these values went into the stream or the file.
    sent: bool,
    /// The values above were taken from a draw.
    filled: bool,
    /// The generation of the tables at the last check of the handles.
    generation: u32,
    /// A skinned shape (an actor). In mode gpu the helper draws it from its table, and the
    /// table walk sends its bones in each frame it works it (`send_bones`); with
    /// `NIRENDER_SKIN_TABLE=0` its draw carries the palette of its bones each frame, so it
    /// stays on the draw records (never in the helper's table, never skipped).
    skinned: bool,
    /// The frame of the last `INSTANCE_BONES` record of a skinned instance (for the audit).
    bones_frame: u32,
    /// A controller on the shape or on one of its properties changes its geometry or its
    /// material (a morpher, a UV controller, a flip, alpha or colour controller): the
    /// display runs the controllers' `OnPreDisplay` as the engine's display does, and
    /// notes the geometry and the material every time.
    live: bool,
    /// The shape got a new property or effect state since its classification (plan 8b):
    /// its next display classifies it again and notes it.
    recheck: bool,
    /// The kind of its draw (`link::DRAW_SHAPE` or `DRAW_STRIPS`), 0 until known.
    kind: u32,
    /// The table walk (entry 13): the ancestors from the shape's parent up, each with its
    /// kind (`CHAIN_*`).
    chain: Vec<(usize, u8)>,
    /// D6: the listed root last found in the chain (0 for none), and the generation of the
    /// lists it was found under: `under_listed_root` scans the chain again only when the
    /// lists changed since.
    mob_root: usize,
    mob_checked: u64,
    /// D6 step 3: the node of the shape's reference (`reference_node`, 0 for none), found
    /// again when the chain changes; and whether the shape waits out of the view under it
    /// (`WAITING_ROOTS`) instead of in the per-shape waiting set.
    ref_root: usize,
    root_waiting: bool,
    /// The walk interval (`MARK_SERIAL`) of the instance's last moved mark: one mark per
    /// shape per walk.
    marked: u32,
    /// The table walk: the frame (plus one) it last worked the instance, so that it works it
    /// once a frame; the instance is in its sweep bucket; the effect state counted for the
    /// lights of the frame (`TABLE_STATE_COUNTS`), and the lights of that state that the
    /// instance counted in the light set (`light_set::count`).
    walk_frame: u32,
    bucketed: bool,
    counted_state: usize,
    counted_lights: Vec<(usize, u32)>,
    /// The table walk: the property and effect states whose properties and texture effects
    /// list this shape in `TABLE_SOURCES`, and those keys; a source mark of one of them sets
    /// `renote`: the next work notes the material and the colours (`display_work`).
    source_states: (usize, usize),
    source_keys: Vec<usize>,
    renote: bool,
    /// The effect state its lights were read from (the table walk reads them again only
    /// when it changes).
    effect_state: usize,
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
    /// In mode gpu the helper draws the skinned instances from its table (the takeover's
    /// step 1, 2026-10-18); `NIRENDER_SKIN_TABLE=0` keeps them on the engine's display with
    /// a palette in each draw (the path before, for an A/B and as the fallback).
    skin_table: bool,
    /// The bones of the instance in progress (`send_bones`), kept between the instances; the
    /// `INSTANCE_BONES` records, and the skinned instances hidden because their bones could
    /// not be made.
    scratch_bones: Vec<u8>,
    bone_records: u64,
    bone_failures: u64,
    /// The works of skinned instances whose own ambient light or fog differed from the
    /// scene's values (the table draws them with the scene's).
    skin_scene_mixed: u64,
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
        instances: HashMap::default(),
        rejected: HashSet::default(),
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
        skin_table: std::env::var("NIRENDER_SKIN_TABLE").map_or(true, |v| v != "0"),
        scratch_bones: Vec::new(),
        bone_records: 0,
        bone_failures: 0,
        skin_scene_mixed: 0,
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
/// Engine-rules R18 (the skill wgpu-renderer-dev).
pub unsafe fn set_mode(mode: Mode) -> Mode {
    let before = self::mode();
    // The events of the cell worker first: a shape it destroyed since the last drain leaves
    // the table before the marks below write into its flags (Tier 1 finding 3).
    crate::worker_events::drain();
    let t = tables();
    t.mode = Some(mode);
    if mode != before {
        TABLE_FULL = true;
        crate::live_walk::rediscover();
        // Mode off is the reference: the engine draws every shape, so the helper must not
        // draw its table too. Without the hides the table's blends doubled the engine's in
        // every picture of mode off (the opaque doubles were invisible), 2026-10-17. The
        // next display of a shape in mode gpu shows its instance again.
        let hide = mode == Mode::Off && capture::recording();
        // Back in mode gpu the instances the helper draws are marked again at once (the
        // condition of `draw`): with the sky-only walk the engine displays no shape after the
        // discovery frame, and the table walk works only marked shapes. The full pass then
        // shows each reached one (the tour's stop 2-26 lost a whole object, 2026-10-18).
        let remark = mode == Mode::Gpu && t.skip;
        let mut hidden = 0;
        for (&shape, instance) in t.instances.iter_mut() {
            instance.sent = false;
            instance.lights.clear();
            if mode != Mode::Gpu {
                mark(shape, false);
            } else if remark && instance.filled && (!instance.skinned || t.skin_table) {
                mark(shape, true);
            }
            if hide && instance.filled && instance.visible {
                instance.visible = false;
                capture::instance_state(instance.id, false);
                hidden += 1;
            }
        }
        t.states += hidden;
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
    // The shapes destroyed on another thread leave the tables before any walk of the scene.
    crate::worker_events::drain();
    crate::live_walk::scene_begin();
    tables().world_camera = if kind == SCENE_WORLD { camera } else { 0 };
    if kind == SCENE_WORLD && camera != 0 {
        crate::animation::note_camera(camera);
    }
}

/// A scene of the game ends. At the end of the world scene in mode gpu: an instance the
/// helper shows that the engine did not draw although it is in the view of the camera is
/// hidden (`INSTANCE_STATE`); an instance the engine drew was made visible at its draw.
/// An instance whose record was not sent again since a reset or a change of the mode is
/// hidden too: the helper's table keeps it from before (a capture that holds resets the
/// records at each release; the far level of a level-of-detail node stayed in the table
/// at the stops of the NIF tour, 2026-10-16).
/// Engine-rules R14, R16 (the skill wgpu-renderer-dev).
pub unsafe fn scene_end() {
    let t = tables();
    let camera = core::mem::replace(&mut t.world_camera, 0);
    if camera == 0 || mode() != Mode::Gpu || !capture::recording() || table_walk_on() {
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

/// True when the engine serves the shape's display as a rule (plan F8, part 2): the shape
/// is not a retained instance (rejected, or never considered), or it is a live one (its
/// display runs its controllers and notes it), or a skinned one that the table does not
/// draws (`table_draws_skins`). An instance the table draws is not the engine's.
pub unsafe fn engine_serves(shape: usize) -> bool {
    let t = tables();
    t.instances.get(&shape).map_or(true, |instance| instance.live || (instance.skinned && !table_draws_skins(t)))
}

/// The helper draws the skinned instances from its table: mode gpu, and the switch on.
unsafe fn table_draws_skins(t: &Tables) -> bool {
    t.mode == Some(Mode::Gpu) && t.skin_table
}

/// Whether the table draws the skinned instances (`NIRENDER_SKIN_TABLE`, `nirender.skinTable`).
pub unsafe fn skin_table() -> bool {
    tables().skin_table
}

/// Sets whether the table draws the skinned instances (`nirender.skinTable`, for the owner's
/// comparison of the two paths in one session) and returns the state before. Each skinned
/// instance sends its record again: on, it is marked and the table's full pass works it; off,
/// it is unmarked, and the engine's display of it in the live walk's discovery frame sends the
/// record without the table's flag, which takes it out of the helper's table.
pub unsafe fn set_skin_table(on: bool) -> bool {
    let t = tables();
    let before = t.skin_table;
    if on == before {
        return before;
    }
    // The events of the cell worker first (as `set_mode`; Tier 1 finding 3).
    crate::worker_events::drain();
    let t = tables();
    t.skin_table = on;
    TABLE_FULL = true;
    crate::live_walk::rediscover();
    let remark = on && t.mode == Some(Mode::Gpu) && t.skip;
    for (&shape, instance) in t.instances.iter_mut() {
        if !instance.skinned {
            continue;
        }
        instance.sent = false;
        instance.lights.clear();
        if remark && instance.filled {
            mark(shape, true);
        } else if !on {
            mark(shape, false);
        }
    }
    before
}

/// Sends the bones of a skinned instance (`INSTANCE_BONES`) with the shape's world bound.
/// Returns false when the bones cannot be made: the instance is hidden then, and counted.
unsafe fn send_bones(shape: usize, id: u32) -> bool {
    let t = tables();
    let mut bones = core::mem::take(&mut t.scratch_bones);
    let result = crate::camera_space::skin_bones(shape, rd(shape, GEOMETRY_SKIN), &mut bones);
    let t = tables();
    let sent = match result {
        Ok(count) => {
            capture::instance_bones(id, count, &rd(shape, WORLD_BOUND), &bones);
            t.bone_records += 1;
            if let Some(instance) = t.instances.get_mut(&shape) {
                instance.bones_frame = t.frame;
            }
            true
        }
        Err(why) => {
            t.bone_failures += 1;
            if t.bone_failures <= 4 {
                crate::log!("instances: no bones for the skinned instance {id} (shape 0x{shape:x}): {why}; it is hidden");
            }
            if let Some(instance) = t.instances.get_mut(&shape) {
                if instance.visible {
                    instance.visible = false;
                    capture::instance_state(id, false);
                    t.states += 1;
                }
            }
            false
        }
    };
    t.scratch_bones = bones;
    sent
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
    crate::worker_events::drain();
    apply_changes();
    let t = tables();
    t.frame = t.frame.wrapping_add(1);
}

/// The shape is deleted (the destructor of `NiGeometry`): its instance goes, with a record
/// for the helper when the helper had it. True when the shape was an instance.
pub unsafe fn forget(shape: usize) -> bool {
    let t = tables();
    t.rejected.remove(&shape);
    if let Some(instance) = t.instances.remove(&shape) {
        table_forget(shape, &instance);
        if instance.sent && capture::recording() {
            capture::instance_delete(instance.id);
        }
        t.deletes += 1;
        return true;
    }
    false
}

/// The fast path of a draw, before the notes of the draw: true when the shape is a
/// retained instance whose handles need no check this frame. The handles of the draw are
/// then those of the instance, and the draw in progress is pending for `draw`.
pub unsafe fn seen(shape: usize, kind: u32, class_out: &mut u32) -> Option<(u32, u32)> {
    if mode() != Mode::Gpu {
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
    t.instances.insert(shape, Instance { id, class, geometry, material, rotation: [0; 12], stored: [0; 3], colours: [0; 14], bound: [0.0; 4], transform: [0; TRANSFORM_WORDS], lights: Vec::new(), ambient: 0xFF00_0000, drawn: t.frame, visible: true, sent: false, filled: false, generation, skinned, bones_frame: 0, live, recheck: false, kind: 0, chain: Vec::new(), mob_root: 0, mob_checked: 0, ref_root: 0, root_waiting: false, marked: 0, walk_frame: 0, bucketed: false, counted_state: 0, counted_lights: Vec::new(), source_states: (0, 0), source_keys: Vec::new(), renote: false, effect_state: 0 });
    t.pending = Some((shape, id));
}

/// Why the shape is not retained, or `None` when it is of the safe class. `live` is set
/// when a controller on the shape or on a property changes it (plan step 5, the morphers).
/// Engine-rules R1, R6, R7 (the skill wgpu-renderer-dev).
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
        // The test function of NiZBufferProperty, the enum of NetImmerse (not D3DCMPFUNC):
        // 0 always, 1 less, 2 equal, 3 less or equal, 4 greater, 5 not equal, 6 greater or
        // equal, 7 never. With the test off (flags bit 0) the engine uses "always" whatever
        // the function says, so the function counts only with the test on (the skill
        // wgpu-renderer-dev, engine-rules R1).
        let z: usize = rd(property_state, PS::propZBuffer);
        if z != 0 && rd::<u16>(z, PROPERTY_FLAGS) & 1 != 0 && !matches!(rd::<u32>(z, ZP::testFunction), 0 | 1 | 3 | 7) {
            return Some(REASON_DEPTH);
        }
    }
    // A texture effect is retained when the table can make its stage from the material
    // record, whatever its coordinate mode (the cube map modes 3 and 4 generate the
    // coordinate from the reflection or the normal on a flat texture: Morrowind has no
    // cube map a file could carry, engine-rules R24), whose world projection does not
    // change (no controller on the effect or
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
        table_forget(shape, &instance);
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
    // A skinned shape's world matrix is the identity (the palette has the transforms), so
    // only its colours and its first record send the instance again. In mode gpu the table
    // draws it with the bones of `send_bones`; with `NIRENDER_SKIN_TABLE=0` it stays on the
    // draw records, with its palette in each draw.
    let skinned = instance.skinned;
    let gpu = t.mode == Some(Mode::Gpu) && (!skinned || t.skin_table);
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
            note_new(shape);
        }
        if instance.lights != lights || instance.ambient != ambient {
            instance.lights.clear();
            instance.lights.extend_from_slice(lights);
            instance.ambient = ambient;
            capture::instance_lights(id, lights, ambient);
            t.light_records += 1;
        }
        instance.drawn = t.frame;
        if !instance.visible {
            instance.visible = true;
            capture::instance_state(id, true);
            t.states += 1;
        }
        if skinned {
            send_bones(shape, id);
        }
        return true;
    }
    capture::instance_draw(id, ambient, fog, lights, bones, bone_count);
    true
}

/// Sends the words of the `INSTANCE` record of an instance of the mode gpu.
unsafe fn send_instance(shape: usize, instance: &Instance, exact: [f64; 3], relative: bool) {
    let flags = if relative { crate::shape_stream::RELATIVE } else { 0 } | BLOCK | if instance.skinned { crate::shape_stream::SKINNED } else { 0 };
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
    (*core::ptr::addr_of_mut!(RECORD_COUNTS))[RC_NAME_BYTES] += name.len().min(96) as u64;
    capture::instance(&words, &name[..name.len().min(96)]);
}

/// The lights of an effect state that the light manager can turn on for a draw: not an
/// ambient light; in the order of the manager's list (the last of the state first). A light
/// that is off or too dim stays in the list: its record has zero colours
/// (`note_light_once`), so that it gives light again when it is turned on, with no new work
/// of the instance.
unsafe fn state_lights(effect_state: usize, out: &mut Vec<usize>) {
    out.clear();
    if effect_state == 0 {
        return;
    }
    let mut iter: usize = rd(effect_state, DES::lights);
    while iter != 0 && out.len() < MOST_LIGHTS {
        let light: usize = rd(iter, DL::effect);
        iter = rd(iter, DL::next);
        if light == 0 {
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
    let counts = &mut *core::ptr::addr_of_mut!(LIGHT_COUNTS);
    counts[LC_NOTES] += 1;
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
    counts[LC_BUILT] += 1;
    let entry = t.scratch_entry.as_mut_ptr() as usize;
    core::ptr::write_bytes(entry as *mut u8, 0, E::SIZE);
    wr::<u32>(entry, E::revisionId, !rd::<u32>(light, L::revisionID));
    crate::ports::light_manager::light_entry_update(entry, light);
    let mut values: [u32; 26] = rd(entry, E::d3dLight);
    // A light that is off or too dim gives no light, as the light manager does not turn it on
    // for a draw: the instances keep its key, and the record has zero colours.
    if rd::<u8>(light, L::enabled) == 0 || rd::<f32>(light, L::dimmer) < f32::from_bits(crate::ports::light_manager::MIN_DIMMER) {
        values[crate::light_stream::COLOUR_WORDS].fill(0);
        counts[LC_DARK] += 1;
    }
    if crate::light_stream::note_values(key, &values) {
        counts[LC_SENT] += 1;
    }
}

/// The measure of the instance records (plan 8f), by `RC_*`: why a record went (the first
/// send or a new handle; a move of the transform; the bound alone; the colours alone; a move
/// and the colours), and for a moved instance what its chain holds (a live shape, plain
/// nodes only, a switch, a level of detail, an flt node, a billboard); the bytes of the
/// names.
static mut RECORD_COUNTS: [u64; 12] = [0; 12];
const RC_FIRST: usize = 0;
const RC_MOVED: usize = 1;
const RC_BOUND: usize = 2;
const RC_COLOURS: usize = 3;
const RC_MOVED_COLOURS: usize = 4;
const RC_LIVE: usize = 5;
const RC_PLAIN: usize = 6;
const RC_NAME_BYTES: usize = 11;
/// The time from the decision to send a record to its end, on the main thread: the matrix
/// (the engine's conversion of the world transform), the exact translation, the record.
static mut RECORD_TICKS: [i64; 3] = [0; 3];

unsafe fn note_record_cause(instance: &Instance, moved: bool, bound: bool, colours: bool) {
    let c = &mut *core::ptr::addr_of_mut!(RECORD_COUNTS);
    let cause = match (instance.sent, moved, bound, colours) {
        (false, ..) => RC_FIRST,
        (true, true, _, false) => RC_MOVED,
        (true, true, _, true) => RC_MOVED_COLOURS,
        (true, false, true, false) => RC_BOUND,
        _ => RC_COLOURS,
    };
    c[cause] += 1;
    if instance.sent && moved {
        let kind = if instance.live {
            RC_LIVE
        } else {
            match instance.chain.iter().find(|&&(_, kind)| kind != CHAIN_PLAIN).map(|&(_, kind)| kind) {
                None => RC_PLAIN,
                Some(CHAIN_SWITCH) => RC_PLAIN + 1,
                Some(CHAIN_LOD) => RC_PLAIN + 2,
                Some(CHAIN_FLT) => RC_PLAIN + 3,
                Some(_) => RC_PLAIN + 4,
            }
        };
        c[kind] += 1;
    }
}

pub unsafe fn record_report() -> String {
    let c = *core::ptr::addr_of!(RECORD_COUNTS);
    let parts = (*core::ptr::addr_of!(RECORD_TICKS)).map(|t| t as f64 * 1000.0 / capture::ticks_per_second() as f64);
    let ms: f64 = parts.iter().sum();
    let total: u64 = c[RC_FIRST..=RC_MOVED_COLOURS].iter().sum();
    format!(
        "instance records (plan 8f): {} ({:.3} ms, {:.2} us each: the matrix {:.3} ms, the exact translation {:.3}, the record {:.3}); first or new handle {}, moved {}, bound alone {}, colours alone {}, moved and colours {}; the moved by their chain: live {}, plain {}, switch {}, level {}, flt {}, billboard {}; name bytes {}",
        total,
        ms,
        ms * 1000.0 / total.max(1) as f64,
        parts[0],
        parts[1],
        parts[2],
        c[RC_FIRST],
        c[RC_MOVED],
        c[RC_BOUND],
        c[RC_COLOURS],
        c[RC_MOVED_COLOURS],
        c[RC_LIVE],
        c[RC_PLAIN],
        c[RC_PLAIN + 1],
        c[RC_PLAIN + 2],
        c[RC_PLAIN + 3],
        c[RC_PLAIN + 4],
        c[RC_NAME_BYTES]
    )
}

/// The measure of the lights (plan 8e), by `LC_*`: the light loops of the displays and their
/// lights; the frame passes over the light set and their lights; the notes of a light, the
/// entries built (one a light a frame), those with zero colours (off or too dim), and the
/// records sent.
static mut LIGHT_COUNTS: [u64; 8] = [0; 8];
const LC_LOOPS: usize = 0;
const LC_LOOP_LIGHTS: usize = 1;
const LC_PASSES: usize = 2;
const LC_PASS_LIGHTS: usize = 3;
const LC_NOTES: usize = 4;
const LC_BUILT: usize = 5;
const LC_DARK: usize = 6;
const LC_SENT: usize = 7;
/// The time of the light loops of the displays, and of the frame passes.
static mut LIGHT_TICKS: [i64; 2] = [0; 2];

pub unsafe fn light_report() -> String {
    let c = *core::ptr::addr_of!(LIGHT_COUNTS);
    let ticks = *core::ptr::addr_of!(LIGHT_TICKS);
    let ms = |t: i64| t as f64 * 1000.0 / capture::ticks_per_second() as f64;
    let per = |a: u64, b: u64| a as f64 / b.max(1) as f64;
    format!(
        "lights (plan 8e): display loops {} ({:.2} lights each, {:.3} ms, {:.2} us each); frame passes {} ({:.1} lights each, {:.3} ms, {:.4} ms each); notes {}, entries built {} ({:.1} a pass; {} with zero colours), records sent {}; {}",
        c[LC_LOOPS],
        per(c[LC_LOOP_LIGHTS], c[LC_LOOPS]),
        ms(ticks[0]),
        ms(ticks[0]) * 1000.0 / c[LC_LOOPS].max(1) as f64,
        c[LC_PASSES],
        per(c[LC_PASS_LIGHTS], c[LC_PASSES]),
        ms(ticks[1]),
        ms(ticks[1]) / c[LC_PASSES].max(1) as f64,
        c[LC_NOTES],
        c[LC_BUILT],
        per(c[LC_BUILT], c[LC_PASSES]),
        c[LC_DARK],
        c[LC_SENT],
        crate::light_set::report()
    )
}

/// The display of a retained shape by the engine (mode gpu): what the draw of the stream
/// did for the instance, without the draw. True when the engine must not display the
/// shape; false when the shape is not retained after all (the mark goes).
/// Engine-rules R5 (the skill wgpu-renderer-dev).
/// Why an instance is checked again (plan 8b, step 2).
#[derive(Clone, Copy)]
pub enum Recheck {
    Properties = 0,
    Effects = 1,
    /// A Lua mod changed the shape or a property of it (`nirender.changed`, plan C17).
    Notified = 2,
}

/// The rechecks asked for (by cause: 0 a property state, 1 an effect state, 2 a Lua
/// change), the rechecks done (3), the instances they rejected (4), and the instances whose
/// live state changed (5).
static mut RECHECKS: [u64; 6] = [0; 6];

/// TEMPORARY, plan C17 (until MWSE notifies the changes of its Lua bindings): the objects
/// a Lua mod changed (`nirender.changed`, from `mod/MWSE/mods/nirender/notifier.lua`), and
/// the notes received and the scans of the table they caused.
static mut CHANGED: Option<HashSet<usize>> = None;
static mut CHANGE_NOTES: [u64; 3] = [0; 3];

/// A Lua mod changed the object at `address` (a shape, a property, a controller's target).
pub unsafe fn note_change(address: usize) {
    if address == 0 {
        return;
    }
    (*core::ptr::addr_of_mut!(CHANGED)).get_or_insert_with(HashSet::default).insert(address);
    (*core::ptr::addr_of_mut!(CHANGE_NOTES))[0] += 1;
    // A Lua mod can write `flags` (the app-cull bit) with no call the plugin sees: the
    // table walk checks the reach of the object's shapes again.
    if table_walk_on() {
        (*core::ptr::addr_of_mut!(CULL_MARKS)).push(address);
    }
}

/// Once a frame: each retained shape that is a changed object, or whose property state holds
/// one, is checked again at its next display. One pass over the table, only in a frame
/// with changes.
unsafe fn apply_changes() {
    let Some(changed) = (*core::ptr::addr_of_mut!(CHANGED)).as_mut() else { return };
    if changed.is_empty() {
        return;
    }
    let notes = &mut *core::ptr::addr_of_mut!(CHANGE_NOTES);
    notes[1] += 1;
    let t = tables();
    for (&shape, instance) in t.instances.iter_mut() {
        let mut hit = changed.contains(&shape);
        if !hit {
            let state: usize = rd(shape, GEOMETRY_PROPERTY_STATE);
            if state != 0 {
                hit = (PS::propAlpha..=PS::field_30).step_by(4).any(|at| {
                    let property: usize = rd(state, at);
                    property != 0 && changed.contains(&property)
                });
            }
        }
        if hit && !instance.recheck {
            instance.recheck = true;
            (*core::ptr::addr_of_mut!(RECHECKS))[Recheck::Notified as usize] += 1;
            notes[2] += 1;
            if table_walk_on() && first_move_mark(instance) {
                (*core::ptr::addr_of_mut!(MOVE_COUNTS))[MV_PUSHED] += 1;
                (*core::ptr::addr_of_mut!(SHAPE_MARKS)).push(shape);
            }
        }
    }
    changed.clear();
}

/// A shape got a new property state or effect state (`ports::state_update`): its retained
/// instance, if it has one, is classified again and noted at its next display.
pub unsafe fn recheck(shape: usize, cause: Recheck) {
    let t = tables();
    if let Some(instance) = t.instances.get_mut(&shape) {
        instance.recheck = true;
        (*core::ptr::addr_of_mut!(RECHECKS))[cause as usize] += 1;
        if table_walk_on() && first_move_mark(instance) {
            (*core::ptr::addr_of_mut!(MOVE_COUNTS))[MV_PUSHED] += 1;
            (*core::ptr::addr_of_mut!(SHAPE_MARKS)).push(shape);
        }
    }
}

/// Plan 8b, step 1: note the material and the geometry of every retained shape at every
/// display, not every 64th frame (`NIRENDER_NOTE_EVERY=1`, `nirender.noteEvery(on)`), and
/// the cost of the skipped displays (`display_cost`).
static NOTE_EVERY: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(255);
static COST_CALLS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static COST_NOTES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static COST_TICKS: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(0);

pub fn note_every() -> bool {
    use core::sync::atomic::Ordering::Relaxed;
    let value = NOTE_EVERY.load(Relaxed);
    if value != 255 {
        return value != 0;
    }
    let on = std::env::var("NIRENDER_NOTE_EVERY").map_or(false, |v| v != "0");
    NOTE_EVERY.store(on as u8, Relaxed);
    on
}

/// Sets the switch; returns the state before.
pub fn set_note_every(on: bool) -> bool {
    let before = note_every();
    NOTE_EVERY.store(on as u8, core::sync::atomic::Ordering::Relaxed);
    before
}

/// The skipped displays since the last call, the notes among them, and their time in
/// milliseconds; the counters reset.
pub fn display_cost() -> (u64, u64, f64) {
    use core::sync::atomic::Ordering::Relaxed;
    COST_ON.store(true, Relaxed);
    let ticks = COST_TICKS.swap(0, Relaxed);
    (COST_CALLS.swap(0, Relaxed), COST_NOTES.swap(0, Relaxed), ticks as f64 * 1000.0 / capture::ticks_per_second() as f64)
}

/// The cost of the display hook is timed only after the first `display_cost` call (the
/// measuring batch calls it once to reset): two counter reads at every display cost about
/// 0.06 ms a frame in the profile of the quicksave (2026-10-17).
static COST_ON: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

pub unsafe fn skipped_display(shape: usize, kind: u32) -> bool {
    use core::sync::atomic::Ordering::Relaxed;
    if !COST_ON.load(Relaxed) {
        return skipped_display_timed(shape, kind);
    }
    let start = capture::ticks();
    let result = skipped_display_timed(shape, kind);
    COST_TICKS.fetch_add(capture::ticks() - start, Relaxed);
    COST_CALLS.fetch_add(1, Relaxed);
    result
}

unsafe fn skipped_display_timed(shape: usize, kind: u32) -> bool {
    let t = tables();
    if t.mode != Some(Mode::Gpu) || !t.skip || !capture::recording() {
        return false;
    }
    if let Some(instance) = t.instances.get_mut(&shape) {
        if instance.kind == 0 {
            instance.kind = kind;
        }
        // The table walk keeps a filled instance current (entry 13): its display does nothing.
        if table_walk_on() && instance.filled {
            return true;
        }
    }
    display_work(shape, kind, false)
}

/// The work of a display of a retained shape: the recheck, the live controllers, the notes
/// when due, the transform, the scene values, the lights, the visible flag. From the
/// engine's display (`skipped_display`) or from the table walk.
/// `table`: from the table walk, which does the work of a still frame by events (entry 13):
/// the colours only when due, rechecked or live, the lights only when the effect state
/// changed, the scene values once a frame (`table_walk`).
unsafe fn display_work(shape: usize, kind: u32, table: bool) -> bool {
    let t = tables();
    let renderer = crate::engine::renderer();
    let (frame, generation) = (t.frame, t.generation);
    // A new property or effect state since the classification (plan 8b): classify again.
    if let Some((skinned, was_live)) = t.instances.get_mut(&shape).filter(|i| i.recheck).map(|i| {
        i.recheck = false;
        (i.skinned, i.live)
    }) {
        let rechecks = &mut *core::ptr::addr_of_mut!(RECHECKS);
        rechecks[3] += 1;
        let mut live = false;
        if reject_reason(shape, rd(shape, GEOMETRY_PROPERTY_STATE), rd(shape, GEOMETRY_EFFECT_STATE), skinned, &mut live).is_some() {
            rechecks[4] += 1;
            reject_pending(shape, true);
            return false;
        }
        let t = tables();
        if let Some(instance) = t.instances.get_mut(&shape) {
            if instance.live != live {
                rechecks[5] += 1;
            }
            instance.live = live || was_live;
            instance.generation = instance.generation.wrapping_sub(1);
        }
    }
    let t = tables();
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
        crate::link_client::note_world_shape(shape, crate::live_walk::Kind::of_draw(kind));
        let mut controller: usize = rd(shape, CONTROLLERS);
        while controller != 0 {
            vcall0(controller, SLOT_ON_PRE_DISPLAY);
            controller = rd(controller, TC::nextController);
        }
    }
    // The handles, when due: every 64th frame, after a change of the device textures, for
    // a record that must go again (the start of a capture), and at every display of a
    // live shape (a new revision of the data sends the geometry again under its handle).
    let due = note_every() || instance.live || instance.renote || !instance.sent || instance.generation != generation || id.wrapping_add(frame) % CHECK_FRAMES == 0;
    instance.renote = false;
    if due {
        COST_NOTES.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        // A skinned geometry is packed with the bone influences of its skin.
        let skin = if instance.skinned { rd(shape, GEOMETRY_SKIN) } else { 0 };
        let geometry = crate::geometry_stream::note(kind, rd(shape, GEOMETRY_MODEL_DATA), skin);
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
    // when it moved), the bound, the colours. A skinned instance keeps the identity and no
    // bound in its record: its bones and its bound go in `INSTANCE_BONES` below.
    let skinned = instance.skinned;
    let (transform, bound): ([u32; TRANSFORM_WORDS], [f32; 4]) = if skinned { (instance.transform, instance.bound) } else { (rd(shape, WORLD), rd(shape, WORLD_BOUND)) };
    let colours = if table && !due { instance.colours } else { crate::shape_stream::material_values(property_state) };
    let moved = instance.transform != transform;
    let bound_changed = instance.bound.map(f32::to_bits) != bound.map(f32::to_bits);
    let colours_changed = instance.colours != colours;
    let changed = moved || bound_changed || colours_changed;
    if changed || !instance.sent {
        let record_ticks = &mut *core::ptr::addr_of_mut!(RECORD_TICKS);
        let record_start = capture::ticks();
        note_record_cause(instance, moved, bound_changed, colours_changed);
        if moved {
            crate::ports::renderer_frame::write_world_matrix(renderer, shape + WORLD);
            let world = crate::shape_stream::current_world(renderer);
            instance.rotation = core::array::from_fn(|i| world[i].to_bits());
            instance.transform = transform;
            instance.stored = [transform[9], transform[10], transform[11]];
        }
        instance.bound = bound;
        instance.colours = colours;
        let matrix_end = capture::ticks();
        let exact = if skinned {
            [0.0; 3]
        } else {
            let Some(exact) = crate::camera_space::exact_translation(shape, 0) else {
                reject_at_record(shape, LATER_TRANSLATION);
                return false;
            };
            exact
        };
        if changed {
            t.value_updates += 1;
        }
        instance.sent = true;
        let exact_end = capture::ticks();
        // The plugin makes the matrices of every draw relative when it owns the camera
        // space: the record says so whether or not a draw happened this frame.
        send_instance(shape, instance, exact, crate::camera_space::own());
        t.records += 1;
        let send_end = capture::ticks();
        record_ticks[0] += matrix_end - record_start;
        record_ticks[1] += exact_end - matrix_end;
        record_ticks[2] += send_end - exact_end;
    }
    // The bones of a skinned instance, in each frame its work runs (the table walk works it
    // each frame it is in the view).
    if skinned && !send_bones(shape, id) {
        return false;
    }
    // The table draws a skinned part with the scene's ambient light and fog: count the parts
    // whose own values differ (the engine's light manager and render state would set those).
    if skinned && table && t.scene_frame == frame {
        let ambient = crate::ports::light_manager::ambient_of_state(effect_state);
        let cell_fog = crate::engine::game_fog_property();
        let fog = if cell_fog != 0 { crate::shape_stream::fog_values_of(renderer, cell_fog) } else { crate::shape_stream::fog_values(renderer, property_state) };
        if (ambient, fog) != t.scene_values {
            t.skin_scene_mixed += 1;
            if t.skin_scene_mixed <= 4 {
                crate::log!("instances: the skinned instance {id} (shape 0x{shape:x}) has other scene values: ambient {ambient:#x} fog {fog:?}, the scene's ambient {:#x} fog {:?}", t.scene_values.0, t.scene_values.1);
            }
        }
    }
    // The values of the frame for the retained draws: the ambient light of this shape's
    // effect state, and the fog of the cell's property (`TES3::Game::fogProperty`, the
    // weather's), as the light manager and the render state would set them. A shape with
    // a fog property of its own carries that fog in its material record. One record in a
    // frame; a shape whose values differ is counted.
    if table {
        let Some(instance) = t.instances.get_mut(&shape) else { return false };
        if instance.effect_state == effect_state && !due {
            instance.drawn = frame;
            if !instance.visible {
                instance.visible = true;
                capture::instance_state(id, true);
                t.states += 1;
            }
            t.skipped += 1;
            return true;
        }
        instance.effect_state = effect_state;
    }
    let ambient = crate::ports::light_manager::ambient_of_state(effect_state);
    let cell_fog = crate::engine::game_fog_property();
    let fog = if cell_fog != 0 {
        crate::shape_stream::fog_values_of(renderer, cell_fog)
    } else {
        crate::shape_stream::fog_values(renderer, property_state)
    };
    // The live walk's displays (plan F8, part 2) leave the record to the engine's walk
    // while that walk runs: the order of the list is not the scene's. Part 4 gives the
    // values a source of their own (the cell's ambient light and fog).
    if t.scene_frame != frame && !crate::live_walk::in_walk() && !table {
        t.scene_frame = frame;
        t.scene_values = (ambient, fog);
        let sun = scene_sun(t);
        capture::retained_scene(ambient, fog, sun.as_ref());
    } else if t.scene_frame == frame && t.scene_values != (ambient, fog) {
        t.scene_mixed += 1;
    }
    // The lights of the effect state of the shape, their device values when they changed,
    // and the keys when the set changed.
    let light_start = capture::ticks();
    let mut lights = core::mem::take(&mut t.scratch_lights);
    state_lights(effect_state, &mut lights);
    let mut keys = core::mem::take(&mut t.scratch_keys);
    keys.clear();
    for &light in &lights {
        let key: u32 = rd(light, L::index);
        note_light_once(t, light, key);
        keys.push(key);
    }
    let counts = &mut *core::ptr::addr_of_mut!(LIGHT_COUNTS);
    counts[LC_LOOPS] += 1;
    counts[LC_LOOP_LIGHTS] += lights.len() as u64;
    (*core::ptr::addr_of_mut!(LIGHT_TICKS))[0] += capture::ticks() - light_start;
    t.scratch_lights = lights;
    let Some(instance) = t.instances.get_mut(&shape) else {
        t.scratch_keys = keys;
        return false;
    };
    // The ambient light of the instance's own effect state goes with its lights: the helper
    // lights each instance with its own, as the engine's light manager sets it for each
    // draw (a spell's effect has a white ambient light of its own; the owner's mark 2 of
    // 2026-10-19 had the whole town lit by it while the scene took the effect as its
    // reference).
    let ambient = crate::ports::light_manager::ambient_of_state(effect_state);
    if instance.lights != keys || instance.ambient != ambient {
        capture::instance_lights(id, &keys, ambient);
        instance.lights.clear();
        instance.lights.extend_from_slice(&keys);
        instance.ambient = ambient;
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

/// The table walk's counters (`TW_*`) and its time.
static mut TABLE_COUNTS: [u64; 32] = [0; 32];
const TW_RUNS: usize = 0;
const TW_WORKED: usize = 1;
const TW_HIDDEN: usize = 2;
const TW_CHAINS: usize = 3;
const TW_FULL: usize = 4;
const TW_ITEMS: usize = 5;
const TW_SWEPT: usize = 6;
const TW_SWEEP_FINDS: usize = 7;
const TW_OFF_ROOT: usize = 8;
const TW_LOST: usize = 9;
const TW_CULL_SHAPES: usize = 10;
const TW_TREE_SHAPES: usize = 11;
const TW_MATERIAL_SHAPES: usize = 12;
const TW_REACH_ONLY: usize = 13;
const TW_SOURCE_SHAPES: usize = 14;
const TW_DEFERRED: usize = 15;
/// The deferred instances hidden because they moved, or are skinned.
const TW_DEFER_HIDDEN: usize = 16;
/// The instances off the root whose effect state counts were taken away unread.
const TW_OFF_ROOT_UNCOUNTED: usize = 17;
/// The node rules: the changes of a node's index, and the shapes below the changed nodes
/// taken into the frame's work.
const TW_NODE_CHANGES: usize = 18;
const TW_NODE_SHAPES: usize = 19;
/// Design D6 (the mob manager's lists, `mob_set`): the roots the lists held, summed over
/// the runs (21 and 22 were the step-1 shares, measured in entry 26 and removed).
const TW_MOB_ROOTS: usize = 20;
/// D6, step 2: the listed roots in the view a run, the shapes taken from their subtrees,
/// the shapes hidden at a root's transition out of the view, the marked and per-frame
/// items skipped because a listed root serves them.
const TW_MOB_IN_VIEW: usize = 23;
const TW_MOB_TAKEN: usize = 24;
const TW_MOB_HIDDEN: usize = 25;
const TW_MOB_SKIPPED: usize = 26;
/// The shapes given one work pass because the lists lost their root.
const TW_MOB_RELEASED: usize = 27;
/// D6 step 3, the hierarchical gate: the shapes put to wait under their reference's node by
/// the loop, and those moved there from the per-shape waiting set; the shapes taken from a
/// waiting root that came into the view (the bound tests of the nodes: `ROOT_TESTS`).
const TW_ROOT_WAITS: usize = 28;
const TW_ROOT_MOVED: usize = 29;
const TW_ROOT_TAKEN: usize = 30;
/// The shapes taken from the listed roots in the view that the loop's gate then found out of
/// the view: the most a bound test per node in the descent of a listed root could save.
const TW_MOB_DEFERRED: usize = 31;
/// The bound tests of reference nodes (`root_in_view`, one per node a run at most).
static ROOT_TESTS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// The marks not made, and the per-frame items not taken, because the shape waits under its
/// reference's node.
static ROOT_MARKS_SUPPRESSED: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static TABLE_TICKS: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(0);
/// The part of the lists' time spent on the waiting sets (the roots and the shapes).
static TABLE_WAIT_TICKS: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(0);
/// The part of `TABLE_TICKS` spent on the node rules.
static TABLE_NODE_TICKS: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(0);
/// The part of `TABLE_TICKS` spent on the lists (the marks, the per-frame set, the sweep).
static TABLE_LIST_TICKS: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(0);
/// The part of the lists' time spent on the movers' roots (D6, step 2).
static TABLE_ROOT_TICKS: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(0);
/// The listed roots in the view in the last run (a diagnostic of the audit's log line).
static mut MOB_IN_VIEW_THIS_FRAME: u64 = 0;
/// The chain scans of `under_listed_root` (one per instance per generation of the lists).
static MOB_RESCANS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// The sweep finds logged so far (the first 16 are).
static mut SWEEP_LOGGED: u32 = 0;
/// The long runs of the table walk logged so far (the first 32 are).
static mut LONG_RUNS_LOGGED: u32 = 0;
/// The kinds of a chain's node.
const CHAIN_PLAIN: u8 = 0;
const CHAIN_SWITCH: u8 = 1;
const CHAIN_LOD: u8 = 2;
const CHAIN_FLT: u8 = 3;
const CHAIN_BILLBOARD: u8 = 4;
/// The app-cull bit of `NiAVObject::flags` (`SetAppCulled`, the controller inventory).
const APP_CULLED: u16 = 0x1;
/// The most ancestors of a chain.
const CHAIN_DEPTH: usize = 64;

/// The table walk (F8 parts 3 and 4, option B, entry 13): on, but with the engine's walk
/// (`live_walk::engine_walk_on`, the permanent debug switch). Its own switch retired at the
/// owner's review of CP4 (2026-10-18).
pub fn table_walk_on() -> bool {
    !crate::live_walk::engine_walk_on()
}

/// The next table walk works every instance (`live_walk::set_engine_walk`).
pub fn table_full() {
    unsafe { TABLE_FULL = true };
}

/// The dirty marks of the table walk (entry 13; the owner, 2026-10-17: events over tracking
/// where the engine offers them). The engine's functions store a pointer here and read
/// nothing more: a probe that read a node's children inside the app-cull hook crashed the
/// game at the night sky's load and at a cell change. The walk reads at display time, and
/// it uses a marked node only as a key (the node can be gone by then). A fixed array, so a
/// hook never allocates; a full array sets `overflow`, and the next walk is a full pass.
const MARK_CAPACITY: usize = 4096;
/// The moved list's array (2026-10-19): with one mark per shape per walk the list holds at
/// most the shapes that moved, and the arrival's town moved 5334 distinct shapes in one
/// frame; the table there holds about 9000.
const MOVED_CAPACITY: usize = 16384;

struct Marks<const N: usize> {
    items: [usize; N],
    len: usize,
    /// The marks a walk takes at most (0: the whole array). The moved list keeps the old cap
    /// of 4096 when one mark per shape is off (`NIRENDER_MARK_ONCE=0`, the A/B).
    limit: usize,
    overflow: bool,
    count: u64,
    /// The walks that lost marks of this list.
    lost_runs: u64,
}

impl<const N: usize> Marks<N> {
    const fn new() -> Marks<N> {
        Marks { items: [0; N], len: 0, limit: 0, overflow: false, count: 0, lost_runs: 0 }
    }

    fn push(&mut self, at: usize) {
        self.count += 1;
        let limit = if self.limit == 0 { N } else { self.limit.min(N) };
        if self.len < limit {
            self.items[self.len] = at;
            self.len += 1;
        } else {
            self.overflow = true;
        }
    }

    /// Moves the marks into `out`; true when some were lost (the array was full).
    fn drain(&mut self, out: &mut Vec<usize>) -> bool {
        out.clear();
        out.extend_from_slice(&self.items[..self.len]);
        self.len = 0;
        let lost = core::mem::replace(&mut self.overflow, false);
        if lost {
            self.lost_runs += 1;
        }
        lost
    }
}

/// A retained shape's world data was updated (`NiGeometry::UpdateWorldData`), or its instance
/// must be checked again (`recheck`, a Lua change).
static mut SHAPE_MARKS: Marks<MOVED_CAPACITY> = Marks::new();
/// One moved mark per shape per walk (2026-10-19): the instance keeps the serial of the walk
/// interval it was marked in (`Instance::marked`), and a second mark in the same interval is
/// not pushed. The moved list filled its 4096 on the crossing with at most about 3100
/// distinct shapes a frame, and each overflow made the next walk a full pass of 4 to 5 ms.
/// `NIRENDER_MARK_ONCE=0` is the A/B switch (the old cap and the duplicates).
static mut MARK_SERIAL: u32 = 1;
pub fn mark_once_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("NIRENDER_MARK_ONCE").map_or(true, |v| v != "0"))
}
/// The moved marks: pushed, duplicates of a shape already marked in the walk interval
/// (pushed when the switch is off, not pushed when on), the most pushed and the most distinct
/// in one interval, and the marked shapes whose world transform at the walk equals the one
/// the helper holds (a mark with no move).
static mut MOVE_COUNTS: [u64; 5] = [0; 5];
const MV_PUSHED: usize = 0;
const MV_DUPLICATES: usize = 1;
const MV_MOST: usize = 2;
const MV_MOST_DISTINCT: usize = 3;
const MV_UNCHANGED: usize = 4;
static mut MOVES_THIS_INTERVAL: [u64; 2] = [0; 2];
/// The walks with more than `MASS_MOVES` moved marks logged so far (the first 16 are).
static mut MASS_LOGGED: u32 = 0;
const MASS_MOVES: usize = 2000;

/// The mass moves' callers logged so far (the first 12 are).
static mut MASS_CALLERS_LOGGED: u32 = 0;

/// The moved mark of a walk interval at which the probe reads the stack:
/// `NIRENDER_MASS_PROBE=1` means the 2000th (`MASS_MOVES`); another value is the count
/// itself (300 caught the burst of one cell's pick root every 18 frames at the hold,
/// 2026-10-19). 0 when the probe is off.
fn mass_probe_threshold() -> u64 {
    static AT: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *AT.get_or_init(|| match std::env::var("NIRENDER_MASS_PROBE") {
        Ok(v) if v == "1" => MASS_MOVES as u64,
        Ok(v) => v.parse().unwrap_or(0),
        Err(_) => 0,
    })
}

/// A diagnostic (2026-10-19): who updates the whole world in a frame. At the `MASS_MOVES`th
/// moved mark of a walk interval, the stack above this function is scanned for return
/// addresses (a value inside a module whose bytes before it are a call), and the first 48 go
/// to the log with their modules. The scan stops at the thread's stack base (the thread
/// block's word at fs:[4]).
#[cfg(target_arch = "x86")]
unsafe fn log_mass_caller() {
    // Only with `NIRENDER_MASS_PROBE=1`; after the save's load (its frames are about 3400 to
    // 3750 in the scenarios); the first 12. It named `VFXManager::update` (0x469C30) on
    // 2026-10-19.
    if mass_probe_threshold() == 0 {
        return;
    }
    let logged = &mut *core::ptr::addr_of_mut!(MASS_CALLERS_LOGGED);
    if *logged >= 12 || tables().frame < 4000 {
        return;
    }
    *logged += 1;
    let marker = 0usize;
    let mut at = core::ptr::addr_of!(marker) as usize;
    let base: usize;
    core::arch::asm!("mov {}, fs:[4]", out(reg) base, options(nostack, readonly, preserves_flags));
    let mut found: Vec<String> = Vec::new();
    while at + 4 <= base && found.len() < 48 {
        let value = *(at as *const usize);
        at += 4;
        if value < 0x10000 {
            continue;
        }
        // A return address lies at least a page above its module's base, so the bytes before
        // it are in the image (a value equal to Morrowind.exe's base, 0x400000, made the read
        // below fault: the crash of 16:03, 2026-10-19).
        let mut module_base = 0usize;
        if GetModuleHandleExA(4 | 2, value as *const core::ffi::c_void, &mut module_base) == 0 || module_base == 0 || value < module_base + 0x1000 {
            continue;
        }
        let module = crate::census::module_of(value);
        let byte = |back: usize| *((value - back) as *const u8);
        let is_call = byte(5) == 0xE8
            || (byte(6) == 0xFF && (byte(5) == 0x15 || byte(5) & 0xF8 == 0x90))
            || (byte(2) == 0xFF && byte(1) & 0xF8 == 0xD0)
            || (byte(3) == 0xFF && byte(2) & 0xF8 == 0x50);
        if is_call {
            found.push(format!("{module}:{value:#x}"));
        }
    }
    crate::log!("marks: the {MASS_MOVES}th moved mark of a walk at frame {}: return addresses on the stack: {}", tables().frame, found.join(" "));
}

#[cfg(not(target_arch = "x86"))]
unsafe fn log_mass_caller() {}

#[cfg(target_arch = "x86")]
#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleExA(flags: u32, address: *const core::ffi::c_void, module: *mut usize) -> i32;
}

/// One moved mark for a shape: false when the shape was marked in this walk interval already
/// and the duplicates are not pushed.
unsafe fn first_move_mark(instance: &mut Instance) -> bool {
    let counts = &mut *core::ptr::addr_of_mut!(MOVE_COUNTS);
    let interval = &mut *core::ptr::addr_of_mut!(MOVES_THIS_INTERVAL);
    interval[0] += 1;
    if interval[0] == mass_probe_threshold() {
        log_mass_caller();
    }
    if instance.marked == MARK_SERIAL {
        counts[MV_DUPLICATES] += 1;
        return !mark_once_on();
    }
    instance.marked = MARK_SERIAL;
    interval[1] += 1;
    true
}
/// An object's app-cull flag changed (`NiAVObject::SetAppCulled`), or a Lua mod changed the
/// object (the notifier; `flags` is written with no call).
static mut CULL_MARKS: Marks<MARK_CAPACITY> = Marks::new();
/// A node's children changed, or an object was attached (the four tree functions of NiNode;
/// the destructor of a node detaches its children through one of them).
static mut TREE_MARKS: Marks<MARK_CAPACITY> = Marks::new();
/// A shape joined the table (its first draw marked it for the helper).
static mut NEW_MARKS: Marks<MARK_CAPACITY> = Marks::new();
/// A property, a texture effect or a shape's data changed by a time-varying writer
/// (`ports::controller_events`): the shapes that use it note their material and colours.
static mut SOURCE_MARKS: Marks<MARK_CAPACITY> = Marks::new();
/// The properties of a subtree changed in place (`ActorAnimController::setOpacity`): the
/// shapes below are checked again (their material and colours noted).
static mut MATERIAL_MARKS: Marks<MARK_CAPACITY> = Marks::new();
/// The next walk works every instance: the first run, the switch turned on, a change of the
/// mode or of the marks switch, a lost mark.
static mut TABLE_FULL: bool = true;
/// The generation of the tables at the last walk: a new one (a reset, a change of the device
/// textures) asks every instance for a check of its handles, so the walk is a full pass.
static mut TABLE_GENERATION: u32 = 0;

/// The retained shapes below each node of their chains: a node's app-cull or tree mark
/// reaches its shapes through this map, without reading the node.
static mut TABLE_BELOW: Option<HashMap<usize, HashSet<usize>>> = None;
/// The shapes worked every frame: those under a billboard node (the turn follows the
/// camera), and the live ones (their controllers run at the display). The shapes under a
/// switch, a level or a time-switch node are not here: their node is in `TABLE_NODES`.
static mut TABLE_DYNAMIC: Option<HashSet<usize>> = None;
/// The node rules (design D3, option A, 2026-10-19): the switch, level and time-switch nodes
/// above the retained shapes, each with the index it selected at its last evaluation. The
/// walk evaluates each node once a frame (`table_node_rules`): a level node from the camera's
/// distance, a time-switch node from the game time, a plain switch by reading its index. When
/// a node's index changes, the shapes below it (`TABLE_BELOW`) are the frame's work, and the
/// reach then shows the selected ones and hides the others. The engine's pick, collisions,
/// downward update, bound and controllers read the index of a node, so the level rule writes
/// its index into the node as the engine's display did, and a changed index gives the
/// selected child its lazy update (`switch_prepare_child`).
static mut TABLE_NODES: Option<HashMap<usize, NodeRule>> = None;

struct NodeRule {
    /// `CHAIN_SWITCH`, `CHAIN_LOD` or `CHAIN_FLT`.
    kind: u8,
    /// The index at the last evaluation; `i32::MIN` before the first.
    index: i32,
}
/// The instances by `id % 64`: the slow sweep works one bucket a frame, the bucket whose
/// instances have their 64th-frame check of the handles in that frame. It is the net for a
/// change that no event marked; its finds are counted.
static mut TABLE_BUCKETS: Vec<Vec<usize>> = Vec::new();
/// The shapes that use each property and texture effect (from their property and effect
/// states): a source mark reaches its shapes through this map.
static mut TABLE_SOURCES: Option<HashMap<usize, HashSet<usize>>> = None;
/// The effect states of the instances, counted: the lights of each are noted once a frame.
static mut TABLE_STATE_COUNTS: Option<HashMap<usize, u32>> = None;
static mut TABLE_WORK: Vec<usize> = Vec::new();
static mut TABLE_DRAINED: Vec<usize> = Vec::new();
static mut TABLE_SCRATCH: Vec<usize> = Vec::new();
static mut TABLE_CHAIN: Vec<(usize, u8)> = Vec::new();
static mut TABLE_TOUCHED: Option<HashSet<usize>> = None;

/// D6: whether a listed root (`mob_set`) is in the instance's chain. The root found last
/// time is tried first (one look-up); the chain is scanned only when it is not listed now.
fn under_listed_root(instance: &mut Instance) -> bool {
    let r = crate::mob_set::roots();
    if r.roots.is_empty() {
        return false;
    }
    // The answer of this generation of the lists stands (a chain rebuilt since sets
    // `mob_checked` to 0): the arrival's 1700 skinned shapes of unlisted actors scanned
    // their whole chain every frame before, 1.0 ms a run (2026-10-19).
    if instance.mob_checked == r.generation {
        return instance.mob_root != 0;
    }
    instance.mob_checked = r.generation;
    MOB_RESCANS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    match instance.chain.iter().rev().find(|&&(node, _)| r.roots.contains(&node)) {
        Some(&(node, _)) => {
            instance.mob_root = node;
            true
        }
        None => {
            instance.mob_root = 0;
            false
        }
    }
}

unsafe fn table_below() -> &'static mut HashMap<usize, HashSet<usize>> {
    (*core::ptr::addr_of_mut!(TABLE_BELOW)).get_or_insert_with(HashMap::default)
}

unsafe fn table_sources() -> &'static mut HashMap<usize, HashSet<usize>> {
    (*core::ptr::addr_of_mut!(TABLE_SOURCES)).get_or_insert_with(HashMap::default)
}

/// Files the shape under the properties of its property state and the texture effects of its
/// effect state (`TABLE_SOURCES`), when the states changed or `again` (the sweep: a property
/// attached in place to the same state).
unsafe fn set_sources(shape: usize, again: bool) {
    let property_state: usize = rd(shape, GEOMETRY_PROPERTY_STATE);
    let effect_state: usize = rd(shape, GEOMETRY_EFFECT_STATE);
    let Some(instance) = tables().instances.get_mut(&shape) else { return };
    if instance.source_states == (property_state, effect_state) && !again {
        return;
    }
    let mut keys: Vec<usize> = Vec::new();
    if property_state != 0 {
        for at in (PS::propAlpha..=PS::field_30).step_by(4) {
            let property: usize = rd(property_state, at);
            if property != 0 && !keys.contains(&property) {
                keys.push(property);
            }
        }
    }
    if effect_state != 0 {
        for at in [crate::gen::layouts::NiDynamicEffectState::environmentMap, crate::gen::layouts::NiDynamicEffectState::fogMap] {
            let effect: usize = rd(effect_state, at);
            if effect != 0 && !keys.contains(&effect) {
                keys.push(effect);
            }
        }
    }
    instance.source_states = (property_state, effect_state);
    if keys == instance.source_keys {
        return;
    }
    let sources = table_sources();
    for key in instance.source_keys.iter() {
        if let Some(set) = sources.get_mut(key) {
            set.remove(&shape);
            if set.is_empty() {
                sources.remove(key);
            }
        }
    }
    for &key in keys.iter() {
        sources.entry(key).or_default().insert(shape);
    }
    instance.source_keys = keys;
}

unsafe fn table_dynamic() -> &'static mut HashSet<usize> {
    (*core::ptr::addr_of_mut!(TABLE_DYNAMIC)).get_or_insert_with(HashSet::default)
}

unsafe fn table_nodes() -> &'static mut HashMap<usize, NodeRule> {
    (*core::ptr::addr_of_mut!(TABLE_NODES)).get_or_insert_with(HashMap::default)
}

/// The index a switch, level or time-switch node selects now, as the engine's display reads
/// or computes it; a level node gets it written, as the engine's display writes it, because
/// the engine's pick, collisions and update read it.
unsafe fn node_index(node: usize, kind: u8, camera: usize) -> i32 {
    let switch = match kind {
        CHAIN_SWITCH => crate::live_walk::SwitchKind::Plain,
        CHAIN_LOD => crate::live_walk::SwitchKind::LevelOfDetail,
        _ => crate::live_walk::SwitchKind::FltAnimation,
    };
    let index = crate::live_walk::switch_index_of(node, switch, camera);
    if kind == CHAIN_LOD {
        crate::mem::wr(node, crate::ports::scene::SWITCH_INDEX, index);
    }
    index
}

/// The selected child of a node for an index, or 0.
unsafe fn selected_child(node: usize, index: i32) -> usize {
    if index < 0 || index >= rd::<i32>(node, crate::gen::layouts::NiNode::kChildren_endIndex) {
        return 0;
    }
    rd(rd::<usize>(node, crate::gen::layouts::NiNode::kChildren_storage), index as usize * 4)
}

/// The node rules of a frame: each node of `TABLE_NODES` evaluated once. A changed index
/// gives the selected child its lazy update, as the engine's display does when it selects a
/// child the update pass skipped, and puts the shapes below the node into the frame's work.
unsafe fn table_node_rules(camera: usize, work: &mut Vec<usize>, take: &dyn Fn(usize, &mut Vec<usize>)) {
    let start = capture::ticks();
    let counts = &mut *core::ptr::addr_of_mut!(TABLE_COUNTS);
    let mut changed: Vec<usize> = Vec::new();
    for (&node, rule) in table_nodes().iter_mut() {
        if !crate::pointer_check::node(node, "the node rules") {
            continue;
        }
        let index = node_index(node, rule.kind, camera);
        if index == rule.index {
            continue;
        }
        rule.index = index;
        counts[TW_NODE_CHANGES] += 1;
        let selected = selected_child(node, index);
        if selected != 0 {
            crate::ports::scene::switch_prepare_child(node, index, selected);
        }
        if let Some(set) = table_below().get(&node) {
            counts[TW_NODE_SHAPES] += set.len() as u64;
            changed.extend(set.iter().copied());
        }
    }
    for shape in changed {
        take(shape, work);
    }
    TABLE_NODE_TICKS.fetch_add(capture::ticks() - start, core::sync::atomic::Ordering::Relaxed);
}

unsafe fn table_state_counts() -> &'static mut HashMap<usize, u32> {
    (*core::ptr::addr_of_mut!(TABLE_STATE_COUNTS)).get_or_insert_with(HashMap::default)
}

/// One effect state more or less in the counts.
unsafe fn count_state(state: usize, up: bool) {
    if state == 0 {
        return;
    }
    let counts = table_state_counts();
    if up {
        *counts.entry(state).or_insert(0) += 1;
    } else if let Some(n) = counts.get_mut(&state) {
        *n -= 1;
        if *n == 0 {
            counts.remove(&state);
        }
    }
}

/// An instance goes (`forget`, `reject_pending`): the walk's tables forget it. The sweep's
/// bucket drops it at its next turn.
unsafe fn table_forget(shape: usize, instance: &Instance) {
    let below = table_below();
    for &(node, _) in instance.chain.iter() {
        if let Some(set) = below.get_mut(&node) {
            set.remove(&shape);
            if set.is_empty() {
                below.remove(&node);
                // A node with no retained shape below leaves the node rules, or the rules
                // read it after the engine frees it (the crash of the arrival glide,
                // 2026-10-19 07:16: an access violation in the plugin after a cell unload).
                table_nodes().remove(&node);
            }
        }
    }
    table_dynamic().remove(&shape);
    count_state(instance.counted_state, false);
    crate::light_set::uncount(&instance.counted_lights);
    let sources = table_sources();
    for key in instance.source_keys.iter() {
        if let Some(set) = sources.get_mut(key) {
            set.remove(&shape);
            if set.is_empty() {
                sources.remove(key);
            }
        }
    }
}

/// The fields of an instance a sweep find compares, as hashes: the transform, the bound, the
/// colours, the geometry and material handles, the visibility.
fn fields_of(instance: &Instance) -> [u64; 6] {
    let hash = |words: &[u32]| words.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &w| (h ^ w as u64).wrapping_mul(0x100_0000_01b3));
    [
        hash(&instance.transform),
        hash(&instance.bound.map(f32::to_bits)),
        hash(&instance.colours),
        instance.geometry as u64,
        instance.material as u64,
        instance.visible as u64,
    ]
}

/// The name of a node (`NiObjectNET::name`), or an empty string.
unsafe fn node_name(node: usize) -> String {
    if node == 0 {
        return String::new();
    }
    let at: usize = rd(node, 8);
    if at == 0 {
        return String::new();
    }
    core::ffi::CStr::from_ptr(at as *const core::ffi::c_char).to_string_lossy().into_owned()
}

/// The census's description of an instance: its name, flags, kinds, its reference's node and
/// the names along its chain (a node named only with a shape of the table below it).
unsafe fn describe_instance(shape: usize, instance: &Instance) -> String {
    let alive = |node: usize| table_below().contains_key(&node);
    let names: Vec<String> = instance.chain.iter().take(10).map(|&(node, _)| if alive(node) { node_name(node) } else { String::from("(gone)") }).collect();
    let top_parent: usize = instance.chain.last().map_or(0, |&(top, _)| if alive(top) { rd(top, PARENT) } else { usize::MAX });
    format!(
        "'{}' (id {}, flags {:#x}) live {} skinned {} visible {} under the reference's node '{}', chain {} (the top's parent 0x{top_parent:x}): {}",
        node_name(shape), instance.id, rd::<u16>(shape, FLAGS), instance.live, instance.skinned, instance.visible,
        if instance.ref_root != 0 && alive(instance.ref_root) { node_name(instance.ref_root) } else { String::from("(none)") },
        instance.chain.len(), names.join(" < ")
    )
}

/// A walk with more than `MASS_MOVES` moved marks, or one that lost moved marks: the first 16
/// go to the log with the marks pushed and distinct in the interval, the shapes whose
/// transform did not change, and the nodes most of the shapes are under (the reference's node
/// and the third node from the top of the chain, the cell's level). The nodes are read only
/// while a shape of the table is below them (`TABLE_BELOW`).
unsafe fn log_mass_moves(drained: &[usize], pushed: u64, distinct: u64, unchanged: u64, lost: bool) {
    let logged = &mut *core::ptr::addr_of_mut!(MASS_LOGGED);
    if *logged >= 16 {
        return;
    }
    *logged += 1;
    let mut by_root: HashMap<usize, u32> = HashMap::default();
    let mut by_level: HashMap<usize, u32> = HashMap::default();
    let mut no_root = 0u32;
    for &shape in drained {
        let Some(instance) = tables().instances.get(&shape) else { continue };
        if instance.ref_root != 0 {
            *by_root.entry(instance.ref_root).or_default() += 1;
        } else {
            no_root += 1;
        }
        if instance.chain.len() >= 3 {
            *by_level.entry(instance.chain[instance.chain.len() - 3].0).or_default() += 1;
        }
    }
    let top = |map: &HashMap<usize, u32>, n: usize| -> String {
        let mut rows: Vec<(usize, u32)> = map.iter().map(|(&k, &v)| (k, v)).collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1));
        rows.iter()
            .take(n)
            .map(|&(node, count)| {
                let name = if table_below().contains_key(&node) { node_name(node) } else { String::from("(gone)") };
                format!("'{name}' {count}")
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    crate::log!(
        "marks: a mass of moves at frame {}: drained {}, pushed {pushed} (distinct {distinct}), lost {lost}, transform unchanged {unchanged}; reference nodes {} (shapes with none {no_root}), the most: {}; the cell level: {}",
        tables().frame,
        drained.len(),
        by_root.len(),
        top(&by_root, 6),
        top(&by_level, 4)
    );
}

/// A change that no event marked, found by the slow sweep: the first 16 go to the log with
/// the shape's name and what changed (the net of entry 13; each such find is a missing event).
unsafe fn log_sweep_find(shape: usize, what: &str) {
    let logged = &mut *core::ptr::addr_of_mut!(SWEEP_LOGGED);
    if *logged >= 16 {
        return;
    }
    *logged += 1;
    let name_at: usize = rd(shape, 8);
    let name = if name_at != 0 { core::ffi::CStr::from_ptr(name_at as *const core::ffi::c_char).to_string_lossy().into_owned() } else { String::new() };
    let parent: usize = rd(shape, PARENT);
    let parent_at: usize = if parent != 0 { rd(parent, 8) } else { 0 };
    let parent_name = if parent_at != 0 { core::ffi::CStr::from_ptr(parent_at as *const core::ffi::c_char).to_string_lossy().into_owned() } else { String::new() };
    let id = tables().instances.get(&shape).map_or(0, |i| i.id);
    let effect_state: usize = rd(shape, GEOMETRY_EFFECT_STATE);
    let (environment, fog): (usize, usize) = if effect_state != 0 {
        (rd(effect_state, crate::gen::layouts::NiDynamicEffectState::environmentMap), rd(effect_state, crate::gen::layouts::NiDynamicEffectState::fogMap))
    } else {
        (0, 0)
    };
    let property_state: usize = rd(shape, GEOMETRY_PROPERTY_STATE);
    let texturing: usize = if property_state != 0 { rd(property_state, PS::propTexturing) } else { 0 };
    let controller: usize = if texturing != 0 { rd(texturing, crate::gen::layouts::NiObjectNET::controllers) } else { 0 };
    let filed = table_sources().get(&texturing).map_or(0, |s| s.len());
    let filed_here = table_sources().get(&texturing).map_or(false, |s| s.contains(&shape));
    let (live, renote, keys) = tables().instances.get(&shape).map_or((false, false, 0), |i| (i.live, i.renote, i.source_keys.len()));
    crate::log!("table walk: the sweep found {what} of instance {id} ('{name}' under '{parent_name}', shape 0x{shape:x}) changed with no event; effect state 0x{effect_state:x}, environment map 0x{environment:x}, fog map 0x{fog:x}; texturing 0x{texturing:x} (controller 0x{controller:x}), shapes filed under it {filed} (this one {filed_here}), its keys {keys}, live {live}, renote {renote}");
}

/// Builds the shape's chain again. When it differs from the kept one, the map of the shapes
/// below each node and the per-frame set follow. True when it differed.
unsafe fn set_chain(shape: usize) -> bool {
    // The instance first, the shape's memory after: a mark can name a shape that was
    // destroyed since (a projectile, an effect), and its destructor forgot the instance
    // (the audit of the cell worker, Tier 1 finding 2).
    if !tables().instances.contains_key(&shape) {
        return false;
    }
    let fresh = &mut *core::ptr::addr_of_mut!(TABLE_CHAIN);
    build_chain(shape, fresh);
    let Some(instance) = tables().instances.get_mut(&shape) else { return false };
    if instance.chain == *fresh {
        return false;
    }
    let below = table_below();
    for &(node, _) in instance.chain.iter() {
        if let Some(set) = below.get_mut(&node) {
            set.remove(&shape);
            if set.is_empty() {
                below.remove(&node);
                table_nodes().remove(&node);
            }
        }
    }
    for &(node, kind) in fresh.iter() {
        below.entry(node).or_default().insert(shape);
        if matches!(kind, CHAIN_SWITCH | CHAIN_LOD | CHAIN_FLT) {
            table_nodes().entry(node).or_insert(NodeRule { kind, index: i32::MIN });
        }
    }
    core::mem::swap(&mut instance.chain, fresh);
    // The listed-root answer (D6) was about the old chain.
    instance.mob_checked = 0;
    // The reference's node (D6 step 3) of the new chain. A shape that waited under the old
    // one is taken by the caller of this function (every caller takes the shape), and the
    // loop's gate decides again.
    instance.ref_root = reference_node(&instance.chain);
    instance.root_waiting = false;
    if instance.live || instance.chain.iter().any(|&(_, kind)| kind == CHAIN_BILLBOARD) {
        table_dynamic().insert(shape);
    } else {
        table_dynamic().remove(&shape);
    }
    true
}

/// The run-time type of the engine's object extra data (`TES3ObjectExtraData`, MWSE's
/// `NIRTTIDefines.Morrowind.h`): the reference's own scene node carries it (MWSE's
/// `ObjectNET::getTes3Reference`).
const RTTI_TES3_OBJECT_EXTRA_DATA: usize = 0x7CE280;
/// `NiExtraData::next`.
const EXTRA_DATA_NEXT: usize = 0x10;

/// D6 step 3: the node of the shape's reference, the nearest node of the chain that carries
/// the engine's object extra data. 0 when the chain has none (the land, the sky, an effect).
/// The node's world bound holds the bounds of every shape below it (the engine's update
/// writes the bounds up the tree): out of the view, every shape below it is out too.
unsafe fn reference_node(chain: &[(usize, u8)]) -> usize {
    for &(node, _) in chain {
        let mut extra: usize = rd(node, crate::gen::layouts::NiObjectNET::extraData);
        let mut seen = 0;
        while extra != 0 && seen < 16 {
            if crate::ports::ni::get_rtti(extra) == RTTI_TES3_OBJECT_EXTRA_DATA {
                return node;
            }
            extra = rd(extra, EXTRA_DATA_NEXT);
            seen += 1;
        }
    }
    0
}

/// The ancestors of a shape from its parent up to the node without a parent, each with its
/// kind.
unsafe fn build_chain(shape: usize, out: &mut Vec<(usize, u8)>) {
    out.clear();
    let mut node: usize = rd(shape, PARENT);
    while node != 0 && out.len() < CHAIN_DEPTH {
        let kind = match crate::live_walk::switch_kind_of(node) {
            Some(crate::live_walk::SwitchKind::Plain) => CHAIN_SWITCH,
            Some(crate::live_walk::SwitchKind::LevelOfDetail) => CHAIN_LOD,
            Some(crate::live_walk::SwitchKind::FltAnimation) => CHAIN_FLT,
            None if crate::live_walk::is_kind(node, b"NiBillboardNode") => CHAIN_BILLBOARD,
            None => CHAIN_PLAIN,
        };
        out.push((node, kind));
        node = rd(node, PARENT);
    }
}

/// True when the engine's walk would reach the shape: no node of its chain (nor the shape)
/// app-culled, and each switch, level and time-switch node selects the chain's child (the
/// index of the node rules, evaluated once a frame; a node that joined the chains after the
/// rules ran is evaluated here). A billboard turns, once a frame.
unsafe fn table_reachable(shape: usize, chain: &[(usize, u8)], camera: usize, touched: &mut HashSet<usize>) -> bool {
    if rd::<u16>(shape, FLAGS) & APP_CULLED != 0 {
        return false;
    }
    for (at, &(node, kind)) in chain.iter().enumerate().rev() {
        if !crate::pointer_check::node(node, "the table walk's reach") {
            return false;
        }
        if rd::<u16>(node, FLAGS) & APP_CULLED != 0 {
            return false;
        }
        let child = if at == 0 { shape } else { chain[at - 1].0 };
        match kind {
            CHAIN_SWITCH | CHAIN_LOD | CHAIN_FLT => {
                let index = match table_nodes().get_mut(&node) {
                    Some(rule) if rule.index != i32::MIN => rule.index,
                    rule => {
                        let index = node_index(node, kind, camera);
                        if let Some(rule) = rule {
                            rule.index = index;
                        }
                        let selected = selected_child(node, index);
                        if selected != 0 {
                            crate::ports::scene::switch_prepare_child(node, index, selected);
                        }
                        index
                    }
                };
                if selected_child(node, index) != child {
                    return false;
                }
            }
            CHAIN_BILLBOARD => {
                if touched.insert(node) {
                    crate::engine::thiscall1(crate::ports::scene::ROTATE_TO_CAMERA, node, camera as u32);
                }
            }
            _ => {}
        }
    }
    true
}

/// The table walk of a frame (entry 13): called at the world scene, after the sky root
/// (`root` is the world's root, the node the engine's walk starts from). It works the
/// instances the events marked, the per-frame ones, and one bucket of the slow sweep; a full
/// pass works every instance. A worked instance is reached (its chain ends at `root` and the
/// engine's reach rules let it through) and then gets the work of the engine path, or it is
/// hidden.
pub unsafe fn table_walk(root: usize, camera: usize) {
    if !table_walk_on() || mode() != Mode::Gpu || !capture::recording() {
        return;
    }
    crate::worker_events::drain();
    let start = capture::ticks();
    let counts = &mut *core::ptr::addr_of_mut!(TABLE_COUNTS);
    counts[TW_RUNS] += 1;
    let (frame, generation) = {
        let t = tables();
        (t.frame, t.generation)
    };
    let stamp = frame.wrapping_add(1);
    let work = &mut *core::ptr::addr_of_mut!(TABLE_WORK);
    let drained = &mut *core::ptr::addr_of_mut!(TABLE_DRAINED);
    let scratch = &mut *core::ptr::addr_of_mut!(TABLE_SCRATCH);
    let buckets = &mut *core::ptr::addr_of_mut!(TABLE_BUCKETS);
    if buckets.len() != CHECK_FRAMES as usize {
        buckets.clear();
        buckets.resize_with(CHECK_FRAMES as usize, Vec::new);
    }
    work.clear();
    // The instance once a frame into the work list.
    let take = |shape: usize, work: &mut Vec<usize>| {
        if let Some(instance) = tables().instances.get_mut(&shape) {
            if instance.walk_frame != stamp {
                instance.walk_frame = stamp;
                work.push(shape);
            }
        }
    };
    let bucket_of = |id: u32| (id % CHECK_FRAMES) as usize;

    let mut lost = false;
    let mut tree = Vec::new();
    lost |= (*core::ptr::addr_of_mut!(TREE_MARKS)).drain(&mut tree);
    let mut fresh = Vec::new();
    lost |= (*core::ptr::addr_of_mut!(NEW_MARKS)).drain(&mut fresh);
    let mut culls = Vec::new();
    lost |= (*core::ptr::addr_of_mut!(CULL_MARKS)).drain(&mut culls);
    let mut materials = Vec::new();
    lost |= (*core::ptr::addr_of_mut!(MATERIAL_MARKS)).drain(&mut materials);
    let mut sources = Vec::new();
    lost |= (*core::ptr::addr_of_mut!(SOURCE_MARKS)).drain(&mut sources);
    // The source marks set `renote` on the shapes of each key (a key that is a shape: the
    // shape), in a full pass too (it works every instance, but notes only the due ones).
    for &key in sources.iter() {
        if let Some(instance) = tables().instances.get_mut(&key) {
            instance.renote = true;
            counts[TW_SOURCE_SHAPES] += 1;
        }
        if let Some(set) = table_sources().get(&key) {
            counts[TW_SOURCE_SHAPES] += set.len() as u64;
            for shape in set.iter() {
                if let Some(instance) = tables().instances.get_mut(shape) {
                    instance.renote = true;
                }
            }
        }
    }
    let moves_lost = (*core::ptr::addr_of_mut!(SHAPE_MARKS)).drain(drained);
    lost |= moves_lost;
    {
        // A new walk interval for one mark per shape; the interval's counts.
        let marks = &mut *core::ptr::addr_of_mut!(SHAPE_MARKS);
        marks.limit = if mark_once_on() { 0 } else { MARK_CAPACITY };
        let serial = &mut *core::ptr::addr_of_mut!(MARK_SERIAL);
        *serial = serial.wrapping_add(1).max(1);
        let interval = &mut *core::ptr::addr_of_mut!(MOVES_THIS_INTERVAL);
        let moves = &mut *core::ptr::addr_of_mut!(MOVE_COUNTS);
        moves[MV_MOST] = moves[MV_MOST].max(interval[0]);
        moves[MV_MOST_DISTINCT] = moves[MV_MOST_DISTINCT].max(interval[1]);
        let (pushed_here, distinct_here) = (interval[0], interval[1]);
        *interval = [0; 2];
        // The moved shapes whose world transform equals the one the helper holds.
        let mut unchanged = 0u64;
        for &shape in drained.iter() {
            if let Some(instance) = tables().instances.get(&shape) {
                if instance.filled && rd::<[u32; TRANSFORM_WORDS]>(shape, WORLD) == instance.transform {
                    unchanged += 1;
                }
            }
        }
        moves[MV_UNCHANGED] += unchanged;
        if drained.len() > MASS_MOVES || moves_lost {
            log_mass_moves(drained, pushed_here, distinct_here, unchanged, moves_lost);
        }
    }
    // The live walk's pending shapes below a node of an app-cull or tree mark (F8 part 3).
    crate::live_walk::release_pending(&culls, root);
    crate::live_walk::release_pending(&tree, root);
    crate::live_walk::note_tree_marks(&tree);
    if lost {
        counts[TW_LOST] += 1;
    }
    let full = lost || note_every() || TABLE_FULL || TABLE_GENERATION != generation;
    TABLE_FULL = false;
    TABLE_GENERATION = generation;
    // The node rules first: the reach of every item below reads the indexes of this frame.
    table_node_rules(camera, work, &take);
    // Design D6, step 1: the engine's lists of what moves, read for this run; the step-1
    // counters say how much of the per-frame set lies under their roots.
    crate::mob_set::refresh();
    counts[TW_MOB_ROOTS] += crate::mob_set::roots().roots.len() as u64;
    // D7 step 1: the census's roots for this run (the engine's globals only; the per-frame
    // set is classified after the lists are built, when the chains are fresh).
    if crate::mark_census::on() {
        crate::mark_census::refresh();
    }
    // The gate of the view (its planes), read here because the movers' roots need them.
    let gate = view_gate_on() && read_view_planes(camera);
    // D6, step 2: the engine's lists of what moves as the per-frame set of the movers. The
    // rule (the owner, 2026-10-19): the renderer keeps only what the helper has been told,
    // and asks the engine for everything else at the frame it needs it. A listed root in
    // the view: its subtree in the engine's own tree is walked and its shapes taken, so they
    // get the frame's work; a root out of the view: nothing, except once at the transition,
    // when its visible shapes are hidden (what the helper was told). The roots' shapes are
    // taken before the marks and the per-frame set, so that the marks' entries of the same
    // shapes are the ones the stamp drops; the marks and the per-frame shapes under a
    // listed root that remain (out of the view) are skipped in the loop below. The sweep
    // keeps them as its safety net.
    let mob_on = crate::mob_set::on() && gate;
    let roots_start = capture::ticks();
    *core::ptr::addr_of_mut!(MOB_IN_VIEW_THIS_FRAME) = 0;
    // Where the listed roots' shapes start in the work list (D6 step 3's descent measure).
    let mut mob_from = usize::MAX;
    if mob_on {
        crate::mob_set::prune_states();
        // A root the lists lost: the shapes under it get one work pass now, since their
        // marks were suppressed up to the moment it left (the engine's update of this frame
        // ran against the lists of the run before).
        if !crate::mob_set::roots().removed.is_empty() {
            let removed = crate::mob_set::roots().removed.clone();
            scratch.clear();
            scratch.extend(tables().instances.iter().filter(|(_, i)| i.mob_root != 0 && removed.contains(&i.mob_root)).map(|(&shape, _)| shape));
            for &shape in scratch.iter() {
                if let Some(instance) = tables().instances.get_mut(&shape) {
                    instance.mob_checked = 0;
                }
                take(shape, work);
            }
            counts[TW_MOB_RELEASED] += scratch.len() as u64;
        }
        let planes = &*core::ptr::addr_of!(VIEW_PLANES);
        mob_from = work.len();
        let roots: Vec<usize> = crate::mob_set::roots().roots.iter().copied().collect();
        for root in roots {
            let bound = vcall0(root, SLOT_GET_WORLD_BOUND) as usize;
            let in_view = bound == 0 || bound_in_view(rd(bound, 0), planes);
            let state = crate::mob_set::state(root);
            let was = *state;
            state.seen = true;
            state.in_view = in_view;
            if in_view {
                counts[TW_MOB_IN_VIEW] += 1;
                *core::ptr::addr_of_mut!(MOB_IN_VIEW_THIS_FRAME) += 1;
                let mut taken = 0u64;
                crate::mob_set::subtree(root, &mut |shape| {
                    if tables().instances.contains_key(&shape) {
                        take(shape, work);
                        taken += 1;
                    }
                });
                counts[TW_MOB_TAKEN] += taken;
            } else if !was.seen || was.in_view {
                let mut hidden = 0u64;
                crate::mob_set::subtree(root, &mut |shape| {
                    let t = tables();
                    if let Some(instance) = t.instances.get_mut(&shape) {
                        if instance.visible {
                            instance.visible = false;
                            capture::instance_state(instance.id, false);
                            t.states += 1;
                            hidden += 1;
                        }
                    }
                });
                counts[TW_MOB_HIDDEN] += hidden;
            }
        }
    }
    let roots_end = work.len();
    TABLE_ROOT_TICKS.fetch_add(capture::ticks() - roots_start, core::sync::atomic::Ordering::Relaxed);
    // D6, step 2: a moved or per-frame shape under a listed root is the root's. In the view
    // its root took it already (the stamp says so); out of the view it is not taken at all,
    // so it never enters the work list (the loop's per-item costs made a skip there worth
    // 0.08 ms for 1870 items at the hold, 2026-10-19).
    let mut skipped_at_take = 0u64;
    let mut take_free = |shape: usize, work: &mut Vec<usize>| {
        if let Some(instance) = tables().instances.get_mut(&shape) {
            if mob_on && instance.walk_frame != stamp && under_listed_root(instance) {
                skipped_at_take += 1;
                return;
            }
            // D6 step 3: a shape waiting under its reference's node is the node's (the
            // waiting roots' pass below takes it when the node comes into the view).
            if instance.root_waiting {
                ROOT_MARKS_SUPPRESSED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                return;
            }
        }
        take(shape, work);
    };
    let mut sweep_from = usize::MAX;
    let mut dynamic_from = usize::MAX;
    if full {
        counts[TW_FULL] += 1;
        scratch.clear();
        scratch.extend(tables().instances.keys().copied());
        for &shape in scratch.iter() {
            if set_chain(shape) {
                counts[TW_CHAINS] += 1;
            }
            if let Some(instance) = tables().instances.get_mut(&shape) {
                if !instance.bucketed {
                    instance.bucketed = true;
                    buckets[bucket_of(instance.id)].push(shape);
                }
            }
            take(shape, work);
        }
    } else {
        // 1. The tree first: the chains below a marked node are built again before any reach
        //    reads them, so no walk reads a node that was detached and destroyed.
        for &at in tree.iter() {
            if tables().instances.contains_key(&at) {
                if set_chain(at) {
                    counts[TW_CHAINS] += 1;
                }
                take(at, work);
            }
            if let Some(set) = table_below().get(&at) {
                scratch.clear();
                scratch.extend(set.iter().copied());
                counts[TW_TREE_SHAPES] += scratch.len() as u64;
                for &shape in scratch.iter() {
                    if set_chain(shape) {
                        counts[TW_CHAINS] += 1;
                    }
                    take(shape, work);
                }
            }
        }
        // 2. The shapes that joined the table.
        for &shape in fresh.iter() {
            if set_chain(shape) {
                counts[TW_CHAINS] += 1;
            }
            if let Some(instance) = tables().instances.get_mut(&shape) {
                if !instance.bucketed {
                    instance.bucketed = true;
                    buckets[bucket_of(instance.id)].push(shape);
                }
            }
            take(shape, work);
        }
        // D6 step 3: an app-cull or a source mark of a shape that waits under its
        // reference's node is not taken: when the node comes into the view, the waiting
        // roots' pass takes the shape, and its work then reads the app-cull flag and notes
        // the sources (`renote` stays set). At the hold about 70 such items a frame went
        // through the loop only to wait again (2026-10-19).
        let take_unless_waiting = |shape: usize, work: &mut Vec<usize>| {
            if tables().instances.get(&shape).is_some_and(|i| i.root_waiting) {
                ROOT_MARKS_SUPPRESSED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                return;
            }
            take(shape, work);
        };
        // 3. App-cull: the object itself, and the shapes below it. A node with no retained
        //    shape below (the HUD, the water, the effect roots) costs one look-up.
        for &at in culls.iter() {
            take_unless_waiting(at, work);
            if let Some(set) = table_below().get(&at) {
                counts[TW_CULL_SHAPES] += set.len() as u64;
                scratch.clear();
                scratch.extend(set.iter().copied());
                for &shape in scratch.iter() {
                    take_unless_waiting(shape, work);
                }
            }
        }
        // 3b. Properties changed in place below a node: the shapes are checked again.
        for &at in materials.iter() {
            let recheck = |shape: usize, work: &mut Vec<usize>| {
                if let Some(instance) = tables().instances.get_mut(&shape) {
                    instance.recheck = true;
                }
                take(shape, work);
            };
            recheck(at, work);
            if let Some(set) = table_below().get(&at) {
                counts[TW_MATERIAL_SHAPES] += set.len() as u64;
                scratch.clear();
                scratch.extend(set.iter().copied());
                for &shape in scratch.iter() {
                    recheck(shape, work);
                }
            }
        }
        // 4. The shapes that moved or must be checked again.
        for &shape in drained.iter() {
            take_free(shape, work);
        }
        // 4b. The shapes of the source marks (their `renote` is set above).
        for &key in sources.iter() {
            take_unless_waiting(key, work);
            if let Some(set) = table_sources().get(&key) {
                scratch.clear();
                scratch.extend(set.iter().copied());
                for &shape in scratch.iter() {
                    take_unless_waiting(shape, work);
                }
            }
        }
        // 5. The per-frame shapes.
        dynamic_from = work.len();
        scratch.clear();
        scratch.extend(table_dynamic().iter().copied());
        for &shape in scratch.iter() {
            take_free(shape, work);
        }
        counts[TW_MOB_SKIPPED] += skipped_at_take;
        // 6. The slow sweep: the bucket whose instances have their 64th-frame check now.
        sweep_from = work.len();
        let bucket = &mut buckets[((CHECK_FRAMES - frame % CHECK_FRAMES) % CHECK_FRAMES) as usize];
        let want = ((CHECK_FRAMES - frame % CHECK_FRAMES) % CHECK_FRAMES) as usize;
        let mut i = 0;
        while i < bucket.len() {
            let shape = bucket[i];
            match tables().instances.get(&shape) {
                Some(instance) if bucket_of(instance.id) == want => {
                    if set_chain(shape) {
                        counts[TW_CHAINS] += 1;
                        counts[TW_SWEEP_FINDS] += 1;
                        log_sweep_find(shape, "the chain");
                    }
                    take(shape, work);
                    i += 1;
                }
                _ => {
                    bucket.swap_remove(i);
                }
            }
        }
        counts[TW_SWEPT] += (work.len() - sweep_from) as u64;
    }
    // The gate of the view: an instance out of the camera's view waits (`TABLE_DEFERRED`), and
    // one that waits is taken in the frame it comes into the view. The deferred ones come
    // after the sweep in the list, and are not sweep items.
    let sweep_end = work.len();
    // D6 step 3, the hierarchical gate (`NIRENDER_ROOT_GATE=0` turns it off; the A/B
    // switch): a shape out of the view waits under its reference's node, and the node's
    // bound is tested once a frame for all of them; when the node comes into the view, its
    // waiting shapes are taken, and the loop's gate tests each one's own bound. A shape
    // without a reference's node, or whose node is in the view, waits in the per-shape set
    // as before.
    let root_gate = gate && root_gate_on();
    (*core::ptr::addr_of_mut!(ROOT_VIEW)).get_or_insert_with(HashMap::default).clear();
    if gate {
        let wait_start = capture::ticks();
        let planes = &*core::ptr::addr_of!(VIEW_PLANES);
        let mut root_taken = 0u64;
        let mut root_moved = 0u64;
        if root_gate {
            // The waiting roots first. A root with no shape of the table below it any more
            // leaves unread: the engine can have freed it (the node rules' rule).
            waiting_roots().retain(|&root| {
                let Some(below) = table_below().get(&root) else { return false };
                if !root_in_view(root, planes) {
                    return true;
                }
                for &shape in below.iter() {
                    if let Some(instance) = tables().instances.get_mut(&shape) {
                        if instance.root_waiting {
                            instance.root_waiting = false;
                            take(shape, work);
                            root_taken += 1;
                        }
                    }
                }
                false
            });
        }
        table_deferred().retain(|&shape| {
            let Some(instance) = tables().instances.get_mut(&shape) else { return false };
            // A waiting shape under a listed root is the root's: its root in the view
            // takes it, its root out of the view hid it.
            if mob_on && under_listed_root(instance) {
                return false;
            }
            // A waiting shape whose reference's node is out of the view goes to wait under
            // the node.
            let root = instance.ref_root;
            if root_gate && root != 0 && table_below().contains_key(&root) && !root_in_view(root, planes) {
                instance.root_waiting = true;
                waiting_roots().insert(root);
                root_moved += 1;
                return false;
            }
            if bound_in_view(rd(shape, AV_WORLD_BOUND), planes) {
                take(shape, work);
            }
            true
        });
        counts[TW_ROOT_TAKEN] += root_taken;
        counts[TW_ROOT_MOVED] += root_moved;
        TABLE_WAIT_TICKS.fetch_add(capture::ticks() - wait_start, core::sync::atomic::Ordering::Relaxed);
    }
    // D7 step 1: the census of the per-frame set by its source, once a run, after the tree
    // marks and a full pass rebuilt the chains (a chain node is read only with a shape of
    // the table below it).
    if crate::mark_census::on() {
        let alive = |node: usize| table_below().contains_key(&node);
        scratch.clear();
        scratch.extend(table_dynamic().iter().copied());
        for &shape in scratch.iter() {
            if let Some(instance) = tables().instances.get(&shape) {
                let source = crate::mark_census::classify(&instance.chain, instance.ref_root, &alive);
                crate::mark_census::count_dynamic(source);
                if crate::mark_census::is_unnamed(source) {
                    // The shape's name and the names along its chain (a node named only
                    // with a shape of the table below it), its flags and its kind.
                    let names: Vec<String> = instance.chain.iter().take(12).map(|&(node, _)| if alive(node) { node_name(node) } else { String::from("(gone)") }).collect();
                    let top_parent: usize = instance.chain.last().map_or(0, |&(top, _)| if alive(top) { rd(top, PARENT) } else { usize::MAX });
                    let line = format!(
                        "'{}' (shape 0x{shape:x}, id {}, flags {:#x}) live {} skinned {} visible {} filled {} chain {} (the top's parent 0x{top_parent:x}): {}",
                        node_name(shape), instance.id, rd::<u16>(shape, FLAGS), instance.live, instance.skinned, instance.visible, instance.filled, instance.chain.len(), names.join(" < ")
                    );
                    crate::mark_census::name_once(shape, &|| line.clone());
                }
            }
        }
    }
    let in_sweep = |at: usize| at >= sweep_from && at < sweep_end;
    counts[TW_ITEMS] += work.len() as u64;
    TABLE_LIST_TICKS.fetch_add(capture::ticks() - start, core::sync::atomic::Ordering::Relaxed);

    let touched = (*core::ptr::addr_of_mut!(TABLE_TOUCHED)).get_or_insert_with(HashSet::default);
    touched.clear();
    let chain = &mut *core::ptr::addr_of_mut!(TABLE_CHAIN);
    for at in 0..work.len() {
        let shape = work[at];
        if !crate::pointer_check::shape(shape, "the table walk") {
            continue;
        }
        // The lights of the frame come from the counted effect states: the count follows the
        // shape's state first, marked for the helper or not, reached or not. A state no shape
        // holds can be freed, and the lights read every counted state (the fault of dm17: in
        // mode off the marks are cleared, and the full pass at the return skipped the unmarked
        // shapes before their counts followed). The sources follow too (again at the sweep
        // and at a recheck: a state updated in place keeps its pointer).
        //
        // A shape off the world root (a cell that the engine's cell worker detached and
        // unloads) is read no further: its effect state can list lights the worker destroyed
        // already, and the engine never reads it, because it never draws the subtree. Its
        // counts go without a read of the light (`light_set::uncount` reads the set only). The
        // crash of the manual review, 2026-10-18 17:41: the count read a freed light.
        if tables().instances.get(&shape).is_some_and(|i| i.chain.is_empty()) && set_chain(shape) {
            counts[TW_CHAINS] += 1;
        }
        let on_root = tables().instances.get(&shape).is_some_and(|i| i.chain.last().is_some_and(|&(top, _)| top == root) || i.chain.len() >= CHAIN_DEPTH);
        if on_root {
            let state: usize = rd(shape, GEOMETRY_EFFECT_STATE);
            {
                let Some(instance) = tables().instances.get_mut(&shape) else { continue };
                if instance.counted_state != state {
                    count_state(instance.counted_state, false);
                    count_state(state, true);
                    instance.counted_state = state;
                    crate::light_set::uncount(&instance.counted_lights);
                    crate::light_set::count(state, &mut instance.counted_lights);
                }
            }
            let again = in_sweep(at) || tables().instances.get(&shape).map_or(false, |i| i.recheck);
            set_sources(shape, again);
        } else if let Some(instance) = tables().instances.get_mut(&shape) {
            if instance.counted_state != 0 {
                count_state(instance.counted_state, false);
                instance.counted_state = 0;
                crate::light_set::uncount(&instance.counted_lights);
                instance.counted_lights.clear();
                counts[TW_OFF_ROOT_UNCOUNTED] += 1;
            }
        }
        // Only the shapes the engine path skips (marked for the helper): a skinned shape is
        // marked only when the table draws the skins (`table_draws_skins`).
        if rd::<u16>(shape, FLAGS) & RETAINED_BIT == 0 {
            // A live shape without the bit stayed in the per-frame set for good, taken each
            // run and dropped here (the census of 2026-10-19: 42 shapes a run on the crossing,
            // the detached creature and body-part models under a parentless "Root Bone" or
            // "Head"). Out of the set; a tree mark or the sweep brings it back.
            table_dynamic().remove(&shape);
            continue;
        }
        // D6, step 2: a marked or per-frame item under a listed root is the root's, before
        // the gate's test and its bookkeeping: its root in the view took it already (the
        // entry before `roots_end`, and the stamp dropped this one), its root out of the
        // view hid it. The sweep's items and a full pass are not skipped: the safety net.
        // The chain of a filled instance is known from an earlier frame.
        if mob_on && !full && at >= roots_end && at < sweep_from && tables().instances.get_mut(&shape).is_some_and(|i| under_listed_root(i)) {
            counts[TW_MOB_SKIPPED] += 1;
            continue;
        }
        // The gate of the view, early: an instance out of the view waits before the chain and
        // the reach (the reach's work on the switches, the levels and the billboards then
        // happens in the view only, as in the engine's walk).
        // Off the world root (a detached model: the engine's head templates, the fish and
        // netch bodies under a parentless "Root Bone"; the census of 2026-10-19, 45 shapes
        // a run on the crossing): out of the per-frame set and the waiting sets, hidden,
        // before the gate's test, which otherwise kept such a shape waiting for good (the
        // off-root step at the loop's end was never reached). A tree mark or the sweep
        // brings it back when it is attached.
        if !on_root {
            counts[TW_OFF_ROOT] += 1;
            table_dynamic().remove(&shape);
            if gate {
                table_deferred().remove(&shape);
            }
            let t = tables();
            if let Some(instance) = t.instances.get_mut(&shape) {
                instance.root_waiting = false;
                if instance.visible {
                    instance.visible = false;
                    capture::instance_state(instance.id, false);
                    t.states += 1;
                    counts[TW_HIDDEN] += 1;
                }
            }
            continue;
        }
        // D6 step 3: the reference's node first. Out of the view, every shape below it is
        // out too, and the record the helper holds decides, as in `out_of_view`; the shape
        // then waits under the node.
        let planes = &*core::ptr::addr_of!(VIEW_PLANES);
        let root_out = root_gate && tables().instances.get(&shape).is_some_and(|i| i.filled && i.ref_root != 0 && table_below().contains_key(&i.ref_root) && !root_in_view(i.ref_root, planes));
        let waits = if root_out {
            tables().instances.get(&shape).map_or(true, |i| !i.visible || !bound_in_view(i.bound, planes))
        } else {
            gate && tables().instances.get(&shape).is_some_and(|i| i.filled) && out_of_view(shape, planes)
        };
        if waits {
            if root_out {
                if let Some(instance) = tables().instances.get_mut(&shape) {
                    instance.root_waiting = true;
                    waiting_roots().insert(instance.ref_root);
                }
                table_deferred().remove(&shape);
                counts[TW_ROOT_WAITS] += 1;
            } else {
                table_deferred().insert(shape);
            }
            counts[TW_DEFERRED] += 1;
            if at >= mob_from && at < roots_end {
                counts[TW_MOB_DEFERRED] += 1;
            }
            // The helper draws a waiting instance where its last record put it. One that moved
            // since, or a skinned one (its bones change each frame), is hidden until it comes
            // back into the view: else a part that leaves the view at its edge stays there.
            let t = tables();
            if let Some(instance) = t.instances.get_mut(&shape) {
                if instance.visible && (instance.skinned || rd::<[u32; TRANSFORM_WORDS]>(shape, WORLD) != instance.transform) {
                    instance.visible = false;
                    capture::instance_state(instance.id, false);
                    t.states += 1;
                    counts[TW_DEFER_HIDDEN] += 1;
                }
            }
            continue;
        }
        if tables().instances.get(&shape).map_or(false, |i| i.chain.is_empty()) && set_chain(shape) {
            counts[TW_CHAINS] += 1;
        }
        let (filled, kind) = {
            let Some(instance) = tables().instances.get_mut(&shape) else { continue };
            // A live instance, and a skinned one (its bones), get their work each frame. A
            // skinned one under a listed root stays in the set: the take skips it while its
            // root is listed, and the set takes it again when the root is not (the player's
            // first-person arms change their chain every frame, 2026-10-19).
            if instance.live || instance.skinned {
                table_dynamic().insert(shape);
            }
            chain.clear();
            chain.extend_from_slice(&instance.chain);
            if instance.kind == 0 {
                instance.kind = if crate::live_walk::is_kind(shape, b"NiTriStrips") { crate::link::DRAW_STRIPS } else { crate::link::DRAW_SHAPE };
            }
            (instance.filled, instance.kind)
        };
        if !filled {
            continue;
        }
        let under_root = chain.last().map_or(false, |&(top, _)| top == root) || chain.len() >= CHAIN_DEPTH;
        if !under_root {
            // Off the world (a cell left behind, a detached object): hidden, and out of the
            // per-frame set; attached again, the tree mark of the attach brings it back.
            counts[TW_OFF_ROOT] += 1;
            table_dynamic().remove(&shape);
        }
        let before = {
            let t = tables();
            (t.records, t.states)
        };
        let fields_before = if in_sweep(at) { tables().instances.get(&shape).map(fields_of) } else { None };
        if under_root && table_reachable(shape, chain, camera, touched) {
            // Taken only for the per-frame set (no mark): the reach was the frame's work. The
            // full work only when the instance is hidden, due, live, rechecked, not sent, or
            // under a billboard (the turn in the reach moved it in this frame).
            let reach_only = at >= dynamic_from && at < sweep_from && tables().instances.get(&shape).map_or(false, |i| {
                i.visible && i.sent && !i.live && !i.skinned && !i.recheck && !i.renote && i.generation == generation && i.id.wrapping_add(frame) % CHECK_FRAMES != 0
            }) && !chain.iter().any(|&(_, kind)| kind == CHAIN_BILLBOARD)
                // A waiting instance can have moved while out of the view: it gets the work.
                && !(gate && (table_deferred().contains(&shape) || tables().instances.get(&shape).is_some_and(|i| i.root_waiting)));
            if reach_only {
                if let Some(instance) = tables().instances.get_mut(&shape) {
                    instance.drawn = frame;
                }
                counts[TW_REACH_ONLY] += 1;
            } else {
                if gate {
                    table_deferred().remove(&shape);
                    if let Some(instance) = tables().instances.get_mut(&shape) {
                        instance.root_waiting = false;
                    }
                }
                display_work(shape, kind, true);
                counts[TW_WORKED] += 1;
            }
        } else if let Some(instance) = tables().instances.get_mut(&shape) {
            if gate {
                table_deferred().remove(&shape);
                instance.root_waiting = false;
            }
            if instance.visible {
                instance.visible = false;
                capture::instance_state(instance.id, false);
                tables().states += 1;
                counts[TW_HIDDEN] += 1;
            }
        }
        if in_sweep(at) {
            let t = tables();
            if (t.records, t.states) != before {
                counts[TW_SWEEP_FINDS] += 1;
                let after = tables().instances.get(&shape).map(fields_of);
                let what = match (fields_before, after) {
                    (Some(a), Some(b)) => {
                        let names = ["the transform", "the bound", "the colours", "the geometry", "the material", "the visibility"];
                        let changed: Vec<&str> = (0..6).filter(|&i| a[i] != b[i]).map(|i| names[i]).collect();
                        if changed.is_empty() { "a record (no field)".to_string() } else { changed.join(", ") }
                    }
                    _ => "a record".to_string(),
                };
                let what = match (fields_before, after) {
                    (Some(a), Some(b)) if a[4] != b[4] => format!("{what} (words {})", crate::material_stream::words_changed(a[4] as u32, b[4] as u32)),
                    _ => what,
                };
                log_sweep_find(shape, &what);
            }
        }
    }
    table_frame_values();
    table_frame_lights();
    let run_ticks = capture::ticks() - start;
    TABLE_TICKS.fetch_add(run_ticks, core::sync::atomic::Ordering::Relaxed);
    // A long run, with its causes (the owner, 2026-10-18: a hitch at the crossings of exterior
    // cells again, after the worker events): the first 32 go to the log.
    let run_ms = run_ticks as f64 * 1000.0 / capture::ticks_per_second() as f64;
    let logged = &mut *core::ptr::addr_of_mut!(LONG_RUNS_LOGGED);
    if run_ms > 2.0 && *logged < 32 {
        *logged += 1;
        crate::log!(
            "table walk: a long run of {run_ms:.2} ms at frame {frame}: full pass {full} (marks lost {lost}), items {}, tree marks {}, new {}, app-cull {}, material {}, moved {}, sources {}",
            work.len(),
            tree.len(),
            fresh.len(),
            culls.len(),
            materials.len(),
            drained.len(),
            sources.len()
        );
    }
    if audit_on() {
        table_audit(root, camera);
    }
}

/// The audit of the table walk (the owner, 2026-10-18: a debug tool to hone the walk by
/// events, not a second walk): `NIRENDER_TABLE_AUDIT=1`. After the walk of each frame it
/// takes every retained instance in the view (or that the helper shows in the view), works
/// out from the scene graph alone what the helper must show, and counts each difference
/// from what the walk sent: the visibility, the transform, the bound, the colours, the
/// chain. It writes nothing to the scene and sends nothing. The first finds go to the log.
static AUDIT_ON: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(255);
/// By `AU_*`: the frames, the instances checked, and the differences.
static mut AUDIT_COUNTS: [u64; 8] = [0; 8];
const AU_FRAMES: usize = 0;
const AU_CHECKED: usize = 1;
const AU_VISIBILITY: usize = 2;
const AU_TRANSFORM: usize = 3;
const AU_BOUND: usize = 4;
const AU_COLOURS: usize = 5;
const AU_CHAIN: usize = 6;
const AU_HIDDEN_SHOWN: usize = 7;
static mut AUDIT_TICKS: i64 = 0;
static mut AUDIT_FOUND: Option<HashSet<usize>> = None;
static mut AUDIT_LOGGED: u32 = 0;
static mut AUDIT_CHAIN: Vec<(usize, u8)> = Vec::new();

/// The test of the audit: `NIRENDER_TABLE_AUDIT=drop` turns the audit on and drops the
/// marks of the moves, a fault of the walk that the audit must find (a null test that can
/// show the effect).
fn audit_drops_moves() -> bool {
    audit_on() && AUDIT_DROP.load(core::sync::atomic::Ordering::Relaxed)
}
static AUDIT_DROP: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn audit_on() -> bool {
    let value = AUDIT_ON.load(core::sync::atomic::Ordering::Relaxed);
    if value != 255 {
        return value != 0;
    }
    let value = std::env::var("NIRENDER_TABLE_AUDIT").unwrap_or_default();
    let on = value == "1" || value == "drop";
    AUDIT_DROP.store(value == "drop", core::sync::atomic::Ordering::Relaxed);
    AUDIT_ON.store(on as u8, core::sync::atomic::Ordering::Relaxed);
    on
}

/// `table_reachable` without its writes (the level's index, the switch child's update, the
/// billboard's turn): the walk did them in this frame.
unsafe fn audit_reachable(shape: usize, chain: &[(usize, u8)], camera: usize) -> bool {
    if rd::<u16>(shape, FLAGS) & APP_CULLED != 0 {
        return false;
    }
    for (at, &(node, kind)) in chain.iter().enumerate().rev() {
        if rd::<u16>(node, FLAGS) & APP_CULLED != 0 {
            return false;
        }
        let switch = match kind {
            CHAIN_SWITCH => crate::live_walk::SwitchKind::Plain,
            CHAIN_LOD => crate::live_walk::SwitchKind::LevelOfDetail,
            CHAIN_FLT => crate::live_walk::SwitchKind::FltAnimation,
            _ => continue,
        };
        let child = if at == 0 { shape } else { chain[at - 1].0 };
        let index = crate::live_walk::switch_index_of(node, switch, camera);
        if index < 0 || index >= rd::<i32>(node, crate::gen::layouts::NiNode::kChildren_endIndex) {
            return false;
        }
        let selected: usize = rd(rd::<usize>(node, crate::gen::layouts::NiNode::kChildren_storage), index as usize * 4);
        if selected != child {
            return false;
        }
    }
    true
}

unsafe fn table_audit(root: usize, camera: usize) {
    let start = capture::ticks();
    let counts = &mut *core::ptr::addr_of_mut!(AUDIT_COUNTS);
    counts[AU_FRAMES] += 1;
    // The planes of this frame's camera (the walk reads the same ones when its gate is on).
    if !read_view_planes(camera) {
        return;
    }
    let planes = &*core::ptr::addr_of!(VIEW_PLANES);
    let fresh = &mut *core::ptr::addr_of_mut!(AUDIT_CHAIN);
    let found = (*core::ptr::addr_of_mut!(AUDIT_FOUND)).get_or_insert_with(HashSet::default);
    let renderer = crate::engine::renderer();
    for (&shape, instance) in tables().instances.iter() {
        if !instance.filled || rd::<u16>(shape, FLAGS) & RETAINED_BIT == 0 {
            continue;
        }
        let in_view = bound_in_view(rd(shape, AV_WORLD_BOUND), planes) || (instance.visible && bound_in_view(instance.bound, planes));
        if !in_view {
            continue;
        }
        counts[AU_CHECKED] += 1;
        build_chain(shape, fresh);
        let mut what: Vec<&str> = Vec::new();
        if *fresh != instance.chain {
            counts[AU_CHAIN] += 1;
            what.push("the chain");
        }
        let under_root = fresh.last().map_or(false, |&(top, _)| top == root) || fresh.len() >= CHAIN_DEPTH;
        let shown = under_root && audit_reachable(shape, fresh, camera);
        if shown != instance.visible {
            counts[if shown { AU_HIDDEN_SHOWN } else { AU_VISIBILITY }] += 1;
            what.push(if shown { "hidden but reached" } else { "shown but not reached" });
        }
        if shown && instance.skinned {
            // A skinned instance's transform and bound go with its bones, each frame.
            if instance.bones_frame != tables().frame {
                counts[AU_TRANSFORM] += 1;
                what.push("the bones");
            }
        } else if shown {
            if rd::<[u32; TRANSFORM_WORDS]>(shape, WORLD) != instance.transform {
                counts[AU_TRANSFORM] += 1;
                what.push("the transform");
            }
            if rd::<[f32; 4]>(shape, WORLD_BOUND).map(f32::to_bits) != instance.bound.map(f32::to_bits) {
                counts[AU_BOUND] += 1;
                what.push("the bound");
            }
            if renderer != 0 && crate::shape_stream::material_values(rd(shape, GEOMETRY_PROPERTY_STATE)) != instance.colours {
                counts[AU_COLOURS] += 1;
                what.push("the colours");
            }
        }
        if what.is_empty() {
            continue;
        }
        found.insert(shape);
        let logged = &mut *core::ptr::addr_of_mut!(AUDIT_LOGGED);
        if *logged < 24 {
            *logged += 1;
            let name_of = |object: usize| -> String {
                let at: usize = if object != 0 { rd(object, 8) } else { 0 };
                if at != 0 { core::ffi::CStr::from_ptr(at as *const core::ffi::c_char).to_string_lossy().into_owned() } else { String::new() }
            };
            let kinds: String = fresh.iter().map(|&(_, kind)| (b"PSLFB"[kind.min(4) as usize]) as char).collect();
            // D6 diagnostic: the geometry flag (bit 3) of each chain node, 'g' set, '.' clear;
            // and whether a listed root is in the chain.
            let geometry: String = fresh.iter().map(|&(node, _)| if rd::<u16>(node, FLAGS) & 0x8 != 0 { 'g' } else { '.' }).collect();
            crate::log!(
                "table audit: frame {}: instance {} ('{}' under '{}', shape 0x{shape:x}): {}; live {}, visible {}, chain {} (P plain, S switch, L level, F flt, B billboard), worked at frame {}; geometry flags {}, under a listed root {}, roots in view this frame {}",
                tables().frame,
                instance.id,
                name_of(shape),
                name_of(rd(shape, PARENT)),
                what.join(", "),
                instance.live,
                instance.visible,
                kinds,
                instance.walk_frame.wrapping_sub(1),
                geometry,
                crate::mob_set::under_root(fresh),
                *core::ptr::addr_of!(MOB_IN_VIEW_THIS_FRAME)
            );
        }
    }
    *core::ptr::addr_of_mut!(AUDIT_TICKS) += capture::ticks() - start;
}

pub unsafe fn audit_report() -> String {
    if !audit_on() {
        return "table audit: off".to_string();
    }
    let c = *core::ptr::addr_of!(AUDIT_COUNTS);
    let ms = *core::ptr::addr_of!(AUDIT_TICKS) as f64 * 1000.0 / capture::ticks_per_second() as f64;
    format!(
        "table audit: {} frames, {:.1} instances checked a frame, {:.3} ms ({:.4} ms a frame); differences: shown but not reached {}, hidden but reached {}, the transform {}, the bound {}, the colours {}, the chain {}; distinct instances {}",
        c[AU_FRAMES],
        c[AU_CHECKED] as f64 / c[AU_FRAMES].max(1) as f64,
        ms,
        ms / c[AU_FRAMES].max(1) as f64,
        c[AU_VISIBILITY],
        c[AU_HIDDEN_SHOWN],
        c[AU_TRANSFORM],
        c[AU_BOUND],
        c[AU_COLOURS],
        c[AU_CHAIN],
        (*core::ptr::addr_of!(AUDIT_FOUND)).as_ref().map_or(0, |f| f.len())
    )
}

/// Once a frame, right after the walk's instances: the scene values for the helper's
/// facts, from where the engine takes them and from no shape (design D4, step 2,
/// 2026-10-19): the ambient light of the cell (`Cell::getAmbientColour`: an interior's own
/// colour, black outside, the underwater blend on both), the fog of the game's fog property
/// (the cell's), and the weather's sky light as the sun. Each retained draw lights with
/// the ambient and the fog of its own records; these values are the frame's. Before, the
/// values came from a reference shape, the first the walk worked in the frame, and a spell's
/// effect with a white ambient light of its own lit the town white (entry 22).
unsafe fn table_frame_values() {
    let t = tables();
    let renderer = crate::engine::renderer();
    if renderer == 0 || t.scene_frame == t.frame {
        return;
    }
    let ambient = cell_ambient();
    let fog = crate::shape_stream::fog_values_of(renderer, crate::engine::game_fog_property());
    t.scene_frame = t.frame;
    t.scene_values = (ambient, fog);
    let sun = scene_sun(t);
    capture::retained_scene(ambient, fog, sun.as_ref());
}

/// `DataHandler` (0x7C67E0) -> `currentCell` (+0xB540) (MWSE `TES3DataHandler.h`), and
/// `Cell::getAmbientColour` (0x4E2030): the colour of an interior cell's ambient light, black
/// for an exterior, blended for a camera under water.
const DATA_HANDLER: usize = 0x7C67E0;
const CURRENT_CELL: usize = 0xB540;
const CELL_GET_AMBIENT_COLOUR: usize = 0x4E2030;

/// The ambient render state of a shape of the current cell that has no ambient light of its
/// own: the cell's ambient colour as a packed colour, alpha 255.
unsafe fn cell_ambient() -> u32 {
    let handler: usize = rd(DATA_HANDLER, 0);
    let cell: usize = if handler != 0 { rd(handler, CURRENT_CELL) } else { 0 };
    if cell == 0 {
        return 0xFF00_0000;
    }
    let mut colour = [0f32; 3];
    crate::engine::thiscall1(CELL_GET_AMBIENT_COLOUR, cell, colour.as_mut_ptr() as u32);
    let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    0xFF00_0000 | (channel(colour[0]) << 16) | (channel(colour[1]) << 8) | channel(colour[2])
}

/// `WorldController` (0x7C67DC) -> `weatherController` (+0x58) -> `sgSkyLight` (+0xAC):
/// the weather's directional light, the sun of the hour (MWSE `TES3WeatherController.h`).
const WORLD_CONTROLLER: usize = 0x7C67DC;
const WEATHER_CONTROLLER: usize = 0x58;
const SKY_LIGHT: usize = 0xAC;

/// The sun of the frame for the helper's own draws (the distant land, the shadows): the
/// weather controller's sky light, converted as the engine converts a light for the device
/// (`light_entry_update`, the 26 words of a `D3DLIGHT8`). The engine sets this light on the
/// device at its world draws; with the cut it draws no world, so the helper took the sun
/// from a stale draw, then from a guess among the light records (a constant white light won
/// the guess, 2026-10-18). The owner pointed at the world controller. None when the light
/// is off or dim, as `note_light_once`.
unsafe fn scene_sun(t: &mut Tables) -> Option<[u32; 26]> {
    static LOGGED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    static mut LAST_DIFFUSE: [u32; 3] = [0; 3];
    let log_once = |what: String| {
        if LOGGED.fetch_add(1, core::sync::atomic::Ordering::Relaxed) < 24 {
            crate::log!("scene sun: {what}");
        }
    };
    let world: usize = rd(WORLD_CONTROLLER, 0);
    if world == 0 {
        log_once(String::from("no world controller"));
        return None;
    }
    let weather: usize = rd(world, WEATHER_CONTROLLER);
    if weather == 0 {
        log_once(String::from("no weather controller"));
        return None;
    }
    let light: usize = rd(weather, SKY_LIGHT);
    if light == 0 {
        log_once(String::from("no sky light"));
        return None;
    }
    let enabled: u8 = rd(light, L::enabled);
    let dimmer: f32 = rd(light, L::dimmer);
    if enabled == 0 || dimmer < f32::from_bits(crate::ports::light_manager::MIN_DIMMER) {
        log_once(format!("the sky light 0x{light:x} is off or dim: enabled {enabled}, dimmer {dimmer}"));
        return None;
    }
    let entry = t.scratch_entry.as_mut_ptr() as usize;
    core::ptr::write_bytes(entry as *mut u8, 0, E::SIZE);
    crate::ports::light_manager::light_entry_update(entry, light);
    let values: [u32; 26] = rd(entry, E::d3dLight);
    // The values go to the log when the diffuse colour changes (the hour moves it).
    let diffuse = [values[1], values[2], values[3]];
    if diffuse == *core::ptr::addr_of!(LAST_DIFFUSE) {
        return Some(values);
    }
    LAST_DIFFUSE = diffuse;
    log_once(format!(
        "frame {}: the sky light 0x{light:x} goes into the scene record: type {}, diffuse {:.3} {:.3} {:.3}, ambient {:.3} {:.3} {:.3}, direction {:.3} {:.3} {:.3}",
        t.frame,
        values[0],
        f32::from_bits(values[1]),
        f32::from_bits(values[2]),
        f32::from_bits(values[3]),
        f32::from_bits(values[9]),
        f32::from_bits(values[10]),
        f32::from_bits(values[11]),
        f32::from_bits(values[16]),
        f32::from_bits(values[17]),
        f32::from_bits(values[18])
    ));
    Some(values)
}

/// After the instances: the note of each light of the light set (the lights of the
/// instances' effect states), once a frame each (`note_light_once`).
unsafe fn table_frame_lights() {
    let start = capture::ticks();
    let t = tables();
    let counts = &mut *core::ptr::addr_of_mut!(LIGHT_COUNTS);
    counts[LC_PASSES] += 1;
    counts[LC_PASS_LIGHTS] += crate::light_set::len() as u64;
    for light in crate::light_set::lights() {
        if !crate::pointer_check::light(light, "the frame's lights") {
            continue;
        }
        let key: u32 = rd(light, L::index);
        note_light_once(t, light, key);
    }
    (*core::ptr::addr_of_mut!(LIGHT_TICKS))[1] += capture::ticks() - start;
}

/// The events of entry 13, counted (no change of behaviour yet): the calls of
/// `NiGeometry::UpdateWorldData` and the distinct retained shapes among them, the real
/// app-cull changes; per frame, summed. [calls, retained distinct, app-cull changes, frames,
/// largest retained distinct in a frame]
static mut EVENT_COUNTS: [u64; 5] = [0; 5];
static mut EVENT_FRAME: u32 = u32::MAX;
static mut EVENT_SEEN: Option<HashSet<usize>> = None;

unsafe fn event_frame() -> &'static mut HashSet<usize> {
    let seen = (*core::ptr::addr_of_mut!(EVENT_SEEN)).get_or_insert_with(HashSet::default);
    let counts = &mut *core::ptr::addr_of_mut!(EVENT_COUNTS);
    let frame = tables().frame;
    if *core::ptr::addr_of!(EVENT_FRAME) != frame {
        EVENT_FRAME = frame;
        counts[3] += 1;
        counts[4] = counts[4].max(seen.len() as u64);
        seen.clear();
    }
    seen
}

/// A shape's world transform was updated (`ports::state_update::update_world_data`): a
/// retained shape is marked for the table walk.
pub unsafe fn note_world_data(shape: usize) {
    let seen = event_frame();
    let counts = &mut *core::ptr::addr_of_mut!(EVENT_COUNTS);
    counts[0] += 1;
    if rd::<u16>(shape, FLAGS) & RETAINED_BIT != 0 {
        if seen.insert(shape) {
            counts[1] += 1;
            // D7 step 1: the census of the moved shapes by their source (once a frame a shape,
            // before D6's filter, so the listed roots' share shows too).
            if crate::mark_census::on() {
                let alive = |node: usize| table_below().contains_key(&node);
                if let Some(instance) = tables().instances.get(&shape) {
                    let source = crate::mark_census::classify(&instance.chain, instance.ref_root, &alive);
                    crate::mark_census::count_moved(source);
                    if !crate::mark_census::note_world_update(&instance.chain) {
                        crate::mark_census::name_uncovered(shape);
                    }
                    if crate::mark_census::is_unlisted(source) {
                        crate::mark_census::note_unlisted(shape);
                        // The probe on the unlisted world updates (the rechecks of a relight
                        // come first in a frame and took the probe's firings, 2026-10-19).
                        if crate::mark_census::unlisted_this_run() == mass_probe_threshold() {
                            log_mass_caller();
                        }
                    }
                }
            }
        }
        // D6, step 2: a shape under a listed root is the root's: no mark (one look-up of
        // the instance and the answer cached by the generation of the lists).
        if let Some(instance) = tables().instances.get_mut(&shape) {
            if crate::mob_set::on() && under_listed_root(instance) {
                crate::mob_set::MARKS_SUPPRESSED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                return;
            }
            // D6 step 3: a shape that waits under its reference's node is the node's: when
            // the node comes into the view, the waiting roots' pass takes the shape (at
            // the hold about 90 such marks a frame went through the loop only to wait again,
            // 2026-10-19).
            if instance.root_waiting {
                ROOT_MARKS_SUPPRESSED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                return;
            }
            if table_walk_on() && !first_move_mark(instance) {
                return;
            }
        }
        if table_walk_on() && !audit_drops_moves() {
            (*core::ptr::addr_of_mut!(MOVE_COUNTS))[MV_PUSHED] += 1;
            (*core::ptr::addr_of_mut!(SHAPE_MARKS)).push(shape);
        }
    } else if crate::live_walk::sky_only_on() && !tables().instances.contains_key(&shape) {
        // A shape the table does not know: with the sky-only walk, the live walk finds it
        // (F8 part 3: a shape made at run time registers through the update pass).
        crate::live_walk::note_update(shape);
    }
}

/// True when the shape is a retained instance of the table.
pub unsafe fn is_instance(shape: usize) -> bool {
    tables().instances.contains_key(&shape)
}

/// The shape is a skinned instance (displayed by the live walk with its bones each frame).
pub unsafe fn is_skinned_instance(shape: usize) -> bool {
    tables().instances.get(&shape).is_some_and(|i| i.skinned)
}

/// An object's app-cull flag changed (`ports::state_update::set_app_culled`): it is marked
/// for the table walk.
pub unsafe fn note_app_culled(object: usize) {
    event_frame();
    (*core::ptr::addr_of_mut!(EVENT_COUNTS))[2] += 1;
    if table_walk_on() {
        (*core::ptr::addr_of_mut!(CULL_MARKS)).push(object);
    }
}

/// A node's children changed (the tree ports of `ports::state_update`): the node, and the
/// attached child when there is one, are marked for the table walk.
pub unsafe fn note_tree(node: usize, child: usize) {
    if !table_walk_on() {
        return;
    }
    let marks = &mut *core::ptr::addr_of_mut!(TREE_MARKS);
    marks.push(node);
    if child != 0 {
        marks.push(child);
    }
}

/// The properties of the subtree below `node` changed in place
/// (`ports::state_update::set_opacity`): it is marked for the table walk.
pub unsafe fn note_material_subtree(node: usize) {
    if table_walk_on() && node != 0 {
        (*core::ptr::addr_of_mut!(MATERIAL_MARKS)).push(node);
    }
}

/// A property, a texture effect or a shape's data changed (`ports::controller_events`): it is
/// marked for the table walk.
pub unsafe fn note_source(key: usize) {
    if table_walk_on() && key != 0 {
        (*core::ptr::addr_of_mut!(SOURCE_MARKS)).push(key);
    }
}

/// A shape joined the table (its first draw marked it for the helper).
unsafe fn note_new(shape: usize) {
    if table_walk_on() {
        (*core::ptr::addr_of_mut!(NEW_MARKS)).push(shape);
    }
}

/// The events' line of the report.
pub unsafe fn event_report() -> String {
    let c = *core::ptr::addr_of!(EVENT_COUNTS);
    let frames = c[3].max(1) as f64;
    format!(
        "events (entry 13): frames {}; shape world updates {} ({:.1} a frame), retained shapes updated {:.1} a frame (largest {}), app-cull changes {}",
        c[3], c[0], c[0] as f64 / frames, c[1] as f64 / frames, c[4], c[2]
    )
}

/// `NiAVObject::worldBound`: the centre and the radius.
const AV_WORLD_BOUND: usize = crate::gen::layouts::NiAVObject::worldBound;
/// The vtable slot of `NiAVObject::GetWorldBound` (as `ports/scene.rs`).
const SLOT_GET_WORLD_BOUND: usize = 0x34;

/// The gate of the view (entry 13, 2026-10-18): the table walk works an instance only when the
/// helper can draw it, that is in the camera's view (the helper draws the retained instances
/// in the main view only: the reflection replays the sky's class, and the blend replay no
/// instance). An instance out of the view waits in `TABLE_DEFERRED` with its marks done, and
/// is worked in the frame the view reaches it. On by default; `NIRENDER_TABLE_VIEW=0` is off
/// (the A/B). At the quicksave hold 55 % of the worked instances were out of the view.
static VIEW_GATE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(255);
/// The culling planes of the world camera for the run: the normal and the constant
/// (`NiPlane`, as `NiBound::WhichSide` reads it).
static mut VIEW_PLANES: Vec<[f64; 4]> = Vec::new();
static mut TABLE_DEFERRED: Option<HashSet<usize>> = None;

/// The gate is before the chain and the reach (its later form after the reach retired at the
/// owner's review of CP4, 2026-10-18).
fn view_gate_on() -> bool {
    let value = VIEW_GATE.load(core::sync::atomic::Ordering::Relaxed);
    if value != 255 {
        return value != 0;
    }
    let on = std::env::var("NIRENDER_TABLE_VIEW").map_or(true, |v| v != "0");
    VIEW_GATE.store(on as u8, core::sync::atomic::Ordering::Relaxed);
    on
}

unsafe fn table_deferred() -> &'static mut HashSet<usize> {
    (*core::ptr::addr_of_mut!(TABLE_DEFERRED)).get_or_insert_with(HashSet::default)
}

/// D6 step 3: the reference nodes with shapes waiting below them (`Instance::root_waiting`).
/// A node stays while the table has a shape below it (`TABLE_BELOW`), the proof that the
/// engine has not freed it; the shapes are found below it in that map when it comes into
/// the view.
static mut WAITING_ROOTS: Option<HashSet<usize>> = None;
/// The answers of `root_in_view` in this run.
static mut ROOT_VIEW: Option<HashMap<usize, bool>> = None;

unsafe fn waiting_roots() -> &'static mut HashSet<usize> {
    (*core::ptr::addr_of_mut!(WAITING_ROOTS)).get_or_insert_with(HashSet::default)
}

/// The hierarchical gate (D6 step 3). On by default; `NIRENDER_ROOT_GATE=0` is off (the A/B).
fn root_gate_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("NIRENDER_ROOT_GATE").map_or(true, |v| v != "0"))
}

/// Whether a reference's node is in the view, tested once a run. The caller knows the node
/// is alive (a shape of the table below it). A node the pointer check refuses counts as in
/// the view: its shapes are then tested one by one.
unsafe fn root_in_view(root: usize, planes: &[[f64; 4]]) -> bool {
    let answers = (*core::ptr::addr_of_mut!(ROOT_VIEW)).get_or_insert_with(HashMap::default);
    if let Some(&answer) = answers.get(&root) {
        return answer;
    }
    let answer = !crate::pointer_check::node(root, "the hierarchical gate") || bound_in_view(rd(root, AV_WORLD_BOUND), planes);
    answers.insert(root, answer);
    ROOT_TESTS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    answer
}

/// Reads the culling planes of the camera into `VIEW_PLANES`. False without planes (no gate).
unsafe fn read_view_planes(camera: usize) -> bool {
    let planes = &mut *core::ptr::addr_of_mut!(VIEW_PLANES);
    planes.clear();
    if camera == 0 {
        return false;
    }
    let count = rd::<i32>(camera, C::countCullingPlanes).max(0) as usize;
    let storage: usize = rd(camera, C::cullingPlanes_storage);
    planes.extend((0..count).map(|i| {
        let plane: [f32; 4] = rd(rd::<usize>(storage, i * 4), 0);
        plane.map(f64::from)
    }));
    !planes.is_empty()
}

/// True when a bound (centre, radius) is in the view: on the inner side of each plane or
/// across it, as `NiBound::WhichSide` decides (outside when the signed distance is at most
/// minus the radius).
fn bound_in_view(bound: [f32; 4], planes: &[[f64; 4]]) -> bool {
    let [x, y, z, radius] = bound.map(f64::from);
    planes.iter().all(|p| p[0] * x + p[1] * y + p[2] * z - p[3] > -radius)
}

/// True when the helper need not see the instance's change now: the shape's bound is out of
/// the view, and the instance the helper holds is hidden or its bound is out of the view too
/// (a shape that left the view is worked once more, or the helper would draw it where it
/// was).
unsafe fn out_of_view(shape: usize, planes: &[[f64; 4]]) -> bool {
    if bound_in_view(rd(shape, AV_WORLD_BOUND), planes) {
        return false;
    }
    tables().instances.get(&shape).map_or(true, |i| !i.visible || !bound_in_view(i.bound, planes))
}

/// The table walk's line of the report.
pub unsafe fn table_walk_report() -> String {
    let c = *core::ptr::addr_of!(TABLE_COUNTS);
    let ms = TABLE_TICKS.load(core::sync::atomic::Ordering::Relaxed) as f64 * 1000.0 / capture::ticks_per_second() as f64;
    let runs = c[TW_RUNS].max(1) as f64;
    let list_ms = TABLE_LIST_TICKS.load(core::sync::atomic::Ordering::Relaxed) as f64 * 1000.0 / capture::ticks_per_second() as f64;
    // The per-frame set by kind: live, then the first non-plain kind of the chain.
    let mut kinds = [0u32; 5];
    for shape in table_dynamic().iter() {
        let Some(instance) = tables().instances.get(shape) else { continue };
        if instance.live {
            kinds[0] += 1;
        } else if let Some(&(_, kind)) = instance.chain.iter().find(|&&(_, kind)| kind != CHAIN_PLAIN) {
            kinds[match kind {
                CHAIN_SWITCH => 1,
                CHAIN_LOD => 2,
                CHAIN_FLT => 3,
                _ => 4,
            }] += 1;
        }
    }
    let marks = [
        (*core::ptr::addr_of!(SHAPE_MARKS)).count,
        (*core::ptr::addr_of!(CULL_MARKS)).count,
        (*core::ptr::addr_of!(TREE_MARKS)).count,
        (*core::ptr::addr_of!(NEW_MARKS)).count,
        (*core::ptr::addr_of!(MATERIAL_MARKS)).count,
        (*core::ptr::addr_of!(SOURCE_MARKS)).count,
    ];
    // The node rules by kind: switch, level, time-switch.
    let mut node_kinds = [0u32; 3];
    for rule in table_nodes().values() {
        node_kinds[match rule.kind {
            CHAIN_SWITCH => 0,
            CHAIN_LOD => 1,
            _ => 2,
        }] += 1;
    }
    let node_ms = TABLE_NODE_TICKS.load(core::sync::atomic::Ordering::Relaxed) as f64 * 1000.0 / capture::ticks_per_second() as f64;
    let mobs = crate::mob_set::roots();
    let root_ms = TABLE_ROOT_TICKS.load(core::sync::atomic::Ordering::Relaxed) as f64 * 1000.0 / capture::ticks_per_second() as f64;
    let wait_ms = TABLE_WAIT_TICKS.load(core::sync::atomic::Ordering::Relaxed) as f64 * 1000.0 / capture::ticks_per_second() as f64;
    let moves = *core::ptr::addr_of!(MOVE_COUNTS);
    format!(
        "table walk (entry 13): {}; runs {}, full passes {} (marks lost in {}); items {:.1} a run (swept {:.1}), worked {:.1}, reach only {:.1}, waiting out of the view {:.1} (gate {}, {} now, hidden as they waited {}), hidden {}, not under the root {} (counts taken away unread {}); chains changed {}, sweep finds {}; marks: shape {}, app-cull {} (shapes reached {}), tree {} (shapes reached {}), material {} (shapes reached {}), source {} (shapes reached {}), new {}; per-frame shapes {} (live {}, billboard {}), node rules {} (switch {}, level {}, time-switch {}; index changes {}, shapes taken {}), nodes with shapes below {}, sources {}, effect states {}; time {:.3} ms ({:.4} ms a run, the lists {:.4}, the node rules {:.4}, the roots {:.4}); mobiles (D6): roots {:.1} a run (actors {}, projectiles {}, props {} now); step 2 {}: roots in view {:.1} a run, shapes taken from their subtrees {:.1}, hidden at a transition {}, marked and per-frame items skipped {:.1} a run, marks not made under a listed root {:.1} a run, list generations {}, chain scans {:.1} a run, shapes released at a list change {}; the hierarchical gate (D6 step 3) {}: root tests {:.1} a run, waiting roots {} now (shapes below them {} now), shapes put to wait under a root {:.1} a run, moved there from the per-shape set {}, taken from a root that came into the view {:.1} a run, marks and per-frame items refused for a waiting root {:.1} a run; the waiting sets {:.4} ms a run; the listed roots' shapes the gate then found out of the view {:.1} a run; the moved marks (one a shape a walk {}): pushed {:.1} a run, duplicates {:.1} a run, the most in a walk {} (distinct {}), transform unchanged {:.1} a run, new states with the same content (no recheck): effects {}, properties {}; walks that lost marks: moved {}, tree {}, new {}, app-cull {}, source {}, material {}",
        if table_walk_on() { "on" } else { "off" },
        c[TW_RUNS], c[TW_FULL], c[TW_LOST],
        c[TW_ITEMS] as f64 / runs, c[TW_SWEPT] as f64 / runs, c[TW_WORKED] as f64 / runs, c[TW_REACH_ONLY] as f64 / runs, c[TW_DEFERRED] as f64 / runs,
        if view_gate_on() { "on" } else { "off" }, table_deferred().len(), c[TW_DEFER_HIDDEN], c[TW_HIDDEN], c[TW_OFF_ROOT], c[TW_OFF_ROOT_UNCOUNTED],
        c[TW_CHAINS], c[TW_SWEEP_FINDS],
        marks[0], marks[1], c[TW_CULL_SHAPES], marks[2], c[TW_TREE_SHAPES], marks[4], c[TW_MATERIAL_SHAPES], marks[5], c[TW_SOURCE_SHAPES], marks[3],
        table_dynamic().len(), kinds[0], kinds[4],
        table_nodes().len(), node_kinds[0], node_kinds[1], node_kinds[2], c[TW_NODE_CHANGES], c[TW_NODE_SHAPES],
        table_below().len(), table_sources().len(), table_state_counts().len(),
        ms, ms / runs, list_ms / runs, node_ms / runs, root_ms / runs,
        c[TW_MOB_ROOTS] as f64 / runs, mobs.actors, mobs.projectiles, mobs.props,
        if crate::mob_set::on() { "on" } else { "off" }, c[TW_MOB_IN_VIEW] as f64 / runs, c[TW_MOB_TAKEN] as f64 / runs, c[TW_MOB_HIDDEN], c[TW_MOB_SKIPPED] as f64 / runs,
        crate::mob_set::MARKS_SUPPRESSED.load(core::sync::atomic::Ordering::Relaxed) as f64 / runs,
        mobs.generation, MOB_RESCANS.load(core::sync::atomic::Ordering::Relaxed) as f64 / runs, c[TW_MOB_RELEASED],
        if root_gate_on() { "on" } else { "off" }, ROOT_TESTS.load(core::sync::atomic::Ordering::Relaxed) as f64 / runs, waiting_roots().len(), tables().instances.values().filter(|i| i.root_waiting).count(),
        c[TW_ROOT_WAITS] as f64 / runs, c[TW_ROOT_MOVED], c[TW_ROOT_TAKEN] as f64 / runs, ROOT_MARKS_SUPPRESSED.load(core::sync::atomic::Ordering::Relaxed) as f64 / runs, wait_ms / runs, c[TW_MOB_DEFERRED] as f64 / runs,
        if mark_once_on() { "on" } else { "off" }, moves[MV_PUSHED] as f64 / runs, moves[MV_DUPLICATES] as f64 / runs, moves[MV_MOST], moves[MV_MOST_DISTINCT], moves[MV_UNCHANGED] as f64 / runs,
        crate::ports::state_update::POINTER_ONLY[0].load(core::sync::atomic::Ordering::Relaxed), crate::ports::state_update::POINTER_ONLY[1].load(core::sync::atomic::Ordering::Relaxed),
        (*core::ptr::addr_of!(SHAPE_MARKS)).lost_runs, (*core::ptr::addr_of!(TREE_MARKS)).lost_runs, (*core::ptr::addr_of!(NEW_MARKS)).lost_runs, (*core::ptr::addr_of!(CULL_MARKS)).lost_runs, (*core::ptr::addr_of!(SOURCE_MARKS)).lost_runs, (*core::ptr::addr_of!(MATERIAL_MARKS)).lost_runs
    )
}

pub unsafe fn report() -> String {
    let t = tables();
    let reasons = ["controller", "node kind", "property controller", "effect", "at the record", "depth function"];
    let mut rejected: Vec<String> = reasons.iter().zip(t.rejected_by.iter()).filter(|(_, n)| **n != 0).map(|(r, n)| format!("{r} {n}")).collect();
    let later = *core::ptr::addr_of!(LATER_BY);
    if later.iter().any(|&n| n != 0) {
        rejected.push(format!("at the record by cause: geometry {}, material {}, not modelled {}, translation {}", later[0], later[1], later[2], later[3]));
    }
    let causes = ["not a plain world draw of a retained class", "no geometry", "no material", "path off", "not modelled"];
    let skipped: Vec<String> = causes.iter().zip(t.skipped_by.iter()).filter(|(_, n)| **n != 0).map(|(r, n)| format!("{r} {n}")).collect();
    format!(
        "instances: mode {}, {} retained ({} live, {} skinned: the table draws them {}, {} bone records, {} hidden with no bones, {} works with other scene values), {} rejected ({}), draws not taken ({}), {} fast draws, {} displays skipped ({} with other scene values), {} instance records, {} updates of the handles, {} of the values, {} deletes, {} light records, {} state records",
        mode().name(),
        t.instances.len(),
        t.instances.values().filter(|i| i.live).count(),
        t.instances.values().filter(|i| i.skinned).count(),
        if table_draws_skins(t) { "yes" } else { "no" },
        t.bone_records,
        t.bone_failures,
        t.skin_scene_mixed,
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
    ) + "\n"
        + &format!(
            "rechecks (plan 8b): asked by a new property state {}, by a new effect state {}, by a Lua change {}; done {}, rejected {}, live changed {}; Lua change notes (C17, temporary) {}, scans {}, shapes hit {}\n",
            RECHECKS[0], RECHECKS[1], RECHECKS[2], RECHECKS[3], RECHECKS[4], RECHECKS[5], CHANGE_NOTES[0], CHANGE_NOTES[1], CHANGE_NOTES[2]
        )
        + &crate::animation::report()
        + "\n"
        + &table_walk_report()
        + "\n"
        + &light_report()
        + "\n"
        + &record_report()
        + "\n"
        + &audit_report()
        + "\n"
        + &crate::ports::controller_events::report()
        + "\n" + &crate::script_names::report()
        + "\n" + &crate::click_time::report()
        + "\n" + &crate::worker_events::report()
        + "\n" + &crate::deferred_unload::report()
        + "\n" + &crate::deferred_destroy::report()
        + "\n" + &{
            crate::mark_census::log_unlisted(&|shape| tables().instances.get(&shape).map(|instance| describe_instance(shape, instance)));
            crate::mark_census::report()
        }
        + "\n" + &crate::freed_fill::report()
        + "\n" + &crate::pointer_check::report()
        + "\n" + &crate::live_walk::class_report()
        + "\n" + &crate::camera_space::skin_report()
        + "\n" + &event_report()
        + "\n"
        + &crate::live_walk::report()
}
