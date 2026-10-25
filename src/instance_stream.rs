//! The retained instances of the plugin. A shape of the safe class gets an `INSTANCE`
//! record once: its handles, its world transform with an exact translation, and the
//! colours of its material. The helper then draws the instance from its table. A change of
//! the transform, the colours or the handles sends the `INSTANCE` record again with the
//! same id. The destructor of the geometry sends `INSTANCE_DELETE`.
//!
//! The safe class (`reject_reason`): a shape of the world scene of a retained class
//! (`class_retained`), with no depth test that depends on an earlier draw, no texture
//! effect that moves, and no rejected node kind above it (`REJECTED_KINDS`). A controller
//! on a node above moves the instance in place. A controller on the shape or on one of its
//! properties makes it a live shape: its display runs the controllers' `OnPreDisplay` and
//! notes the geometry and the material each time. A blended shape is retained: the helper
//! sorts the blends of its table back to front. The class of a shape is decided once and
//! kept until the shape is deleted.
//!
//! The modes ([`Mode`]):
//! - `off`: no instance.
//! - `dual`: every record as before, plus the instance records. The shape record is the
//!   twin of an instance draw, and the helper draws one of the two (`MGE_XE_RETAINED`).
//! - `gpu`: the helper draws the instance from its table and culls it with its bound. The
//!   plugin sends no draw, only the changes of the lights (`INSTANCE_LIGHTS`) and of the
//!   visibility (`INSTANCE_STATE`).
//!
//! `NIRENDER_RETAINED` sets the mode at the start (default `gpu`); `nirender.retained(mode)`
//! sets it from Lua.
//!
//! In mode gpu the engine does not display a retained shape. The shape carries
//! [`RETAINED_BIT`], and the ports of its `Display` call `skipped_display`, which does the
//! checks of a draw and returns. The engine's traversal still reaches the shape, so the
//! engine's draws stay the measure of its visibility. `NIRENDER_SKIP=0` keeps the engine's
//! display.
//!
//! The handles of an instance get a check every `CHECK_FRAMES` frames, and at its next draw
//! after a device texture goes away (`textures_changed`, `reset`). The transform and the
//! colours get a check at each draw.
//!
//! An `INSTANCE` record carries two translations: the stored one of the world transform
//! (32-bit floats) and the exact one (`camera_space::exact_translation`, 64-bit). The helper
//! takes the exact one for a record with the flag `RELATIVE`, else the stored one. A parent
//! move smaller than one float step at the shape's position is not seen.
//!
//! The tables are not synchronized: all functions run on the main thread, except
//! `queue_change`.

// The tables are keyed by engine pointers: the pointer hasher, not SipHash (`ptr_hash`).
use crate::ptr_hash::{HashMap, HashSet};

use crate::capture;
use crate::engine::{thiscall1, vcall0};
use crate::gen::layouts::{InterfaceNiTimeController, NiAVObject, NiGeometry, NiProperty, NiPropertyState, NiRTTI};
use crate::gen::layouts::{NiCamera as C, NiDX8LightManager_LightEntry as E, NiDynamicEffectState as DES, NiDynamicEffectState_InternalList as DL, NiLight as L, NiPropertyState as PS, NiTimeController as TC, NiZBufferProperty as ZP};
use crate::link::{CLASS_LANDSCAPE, CLASS_OBJECT, CLASS_SKY_DOME, CLASS_SKY_OTHER, CLASS_UNKNOWN, SCENE_CHARACTER_PREVIEW, SCENE_LOCAL_MAP, SCENE_RACE_MENU_HEAD, SCENE_SORTED_ALPHA, SCENE_WATER, SCENE_WATER_REFLECTION, SCENE_WORLD};
use crate::main_thread::{MainCell, MainThread};
use crate::mem::{rd, wr};

const CONTROLLERS: usize = NiAVObject::controllers;
const PARENT: usize = NiAVObject::parentNode;
const WORLD: usize = NiAVObject::worldTransform;
const WORLD_TRANSLATION: usize = NiAVObject::worldTransform_translation;
/// The centre and the radius.
const WORLD_BOUND: usize = NiAVObject::worldBound;
/// `NiBound::WhichSide(plane)`: 0 across the plane, 1 in front, 2 behind (out of the view).
const WHICH_SIDE: usize = 0x6F2660;
/// The flag of an instance record of the mode gpu: the helper draws it from its table.
const BLOCK: u32 = 0x100;
/// `NiAVObject::flags` bit 12: the plugin's mark on a shape the helper draws from its table.
pub const RETAINED_BIT: u16 = 0x1000;
const FLAGS: usize = NiAVObject::flags;
const GEOMETRY_PROPERTY_STATE: usize = NiGeometry::spPropertyState;
const GEOMETRY_EFFECT_STATE: usize = NiGeometry::spEffectState;
const GEOMETRY_MODEL_DATA: usize = NiGeometry::spModelData;
const GEOMETRY_SKIN: usize = NiGeometry::skinInstance;
/// The words of a `NiTransform`: the rotation, the translation (from word 9), the scale.
const TRANSFORM_WORDS: usize = 13;
const MOST_LIGHTS: usize = 64;
/// The property pointers of a `NiPropertyState`: alpha first, 12 of them.
const STATE_FIRST: usize = NiPropertyState::propAlpha;
const STATE_COUNT: usize = 12;
/// A u16; bit 0 of the alpha property is the blend.
const PROPERTY_FLAGS: usize = NiProperty::flags;
/// The display of a shape calls it for each of its controllers before the draw (a morpher
/// writes its vertices then).
const SLOT_ON_PRE_DISPLAY: usize = InterfaceNiTimeController::OnPreDisplay;
const RTTI_NAME: usize = NiRTTI::name;
const RTTI_BASE: usize = NiRTTI::baseRTTI;
/// The most nodes above a shape that the walk looks at.
const MAX_DEPTH: usize = 32;
/// The frames between two checks of the handles of an instance.
const CHECK_FRAMES: u32 = 64;

/// The node kinds that keep a shape under them on the engine's path. A sort adjust node
/// draws its subtree in an order of its own, which the table does not follow. The other
/// kinds that change what or how they draw need nothing of the table, as long as the
/// engine's traversal reaches the shape. Engine-rules R5 (the skill wgpu-renderer-dev).
const REJECTED_KINDS: [&[u8]; 1] = [b"NiSortAdjustNode"];
/// The list of `NIRENDER_NODE_KINDS=0`: every kind that changes what or how it draws.
const REJECTED_KINDS_BEFORE: [&[u8]; 7] = [b"NiBillboardNode", b"NiSwitchNode", b"NiLODNode", b"NiBSPNode", b"NiSortAdjustNode", b"BSAnimationNode", b"BSParticleNode"];
/// The list of `NIRENDER_NODE_KINDS=0` for a skinned shape: only the kinds that change
/// which shape draws. The palette of each draw carries the animated bones.
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
    /// Zero for a skinned instance: its bound changes with each pose.
    bound: [f32; 4],
    /// The posed world bound of a skinned instance at its last bones (`send_bones`): the
    /// bound the engine culls the shape by (0x6F4160, 0x6D1250). `scene_end` tests it.
    posed_bound: [f32; 4],
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
    /// The matrices of its last `INSTANCE` record were relative to the camera's origin.
    relative: bool,
    /// The values above were taken from a draw.
    filled: bool,
    /// The generation of the tables at the last check of the handles.
    generation: u32,
    /// A skinned shape (an actor). In mode gpu the helper draws it from its table, and its
    /// display sends its bones in each frame the engine displays it (`send_bones`); with
    /// `NIRENDER_SKIN_TABLE=0` its draw carries the palette of its bones each frame, so it
    /// stays on the draw records (never in the helper's table, never skipped).
    skinned: bool,
    /// A controller on the shape or on one of its properties changes its geometry or its
    /// material (a morpher, a UV controller, a flip, alpha or colour controller): the
    /// display runs the controllers' `OnPreDisplay` as the engine's display does, and
    /// notes the geometry and the material every time.
    live: bool,
    /// The shape got a new property or effect state since its classification: its next
    /// display classifies it again and notes it.
    recheck: bool,
    /// The kind of its draw (`link::DRAW_SHAPE` or `DRAW_STRIPS`), 0 until known.
    kind: u32,
    /// A copy of the shape's property pointers at its last display. `apply_changes` reads
    /// only this copy, never engine memory: the cell worker can free a shape at any time.
    properties: [usize; PROPERTY_SLOTS],
    /// The frame in which the engine's main sorted queue took the shape (`queue_blend`), or
    /// [`u32::MAX`].
    queued: u32,
    /// A particle system (a `RenderPoints` draw): its particles go as an `INSTANCE_PARTICLES`
    /// record before each `INSTANCE_DRAW`, at the place of the engine's draw, and the table
    /// never draws it by itself (particles plan, step 2).
    points: bool,
}

/// The property slots of `NiPropertyState` from `propAlpha` (+0x08) to `field_30` (+0x30).
const PROPERTY_SLOTS: usize = 11;

struct Tables {
    instances: HashMap<usize, Instance>,
    rejected: HashSet<usize>,
    next_id: u32,
    frame: u32,
    /// Grows when the handles of every instance need a check: the next draw of each takes
    /// the full path once.
    generation: u32,
    /// The draw in progress is of a retained shape: the shape and its id.
    pending: Option<(usize, u32)>,
    mode: Option<Mode>,
    /// The camera of the world scene in progress, for the check of the visibility at its
    /// end (mode gpu).
    world_camera: usize,
    /// The kind of the scene in progress (`SCENE_*` of `link.rs`), for `skipped_display`.
    scene_kind: u32,
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
    /// The shapes of class 0 (under no root the classes know: the actors and their parts, and
    /// objects hung elsewhere) are retained too (`NIRENDER_ACTORS=0` keeps them out). The class
    /// does not say what a shape is; the reference of its scene node would.
    actors: bool,
    /// A shape under an animated node, and a shape of class 0 that is not skinned, are
    /// retained too (`NIRENDER_ANIMATED=0` keeps them out).
    animated: bool,
    /// A shape under a billboard, a switch, a level of detail, a BSP, an animation or a
    /// particle node is retained too (`NIRENDER_NODE_KINDS=0` keeps them out). See
    /// [`REJECTED_KINDS`].
    node_kinds: bool,
    /// A shape with a controller on itself or on a property is retained as a live shape
    /// (`NIRENDER_CONTROLLERS=0` rejects it).
    controllers: bool,
    /// In mode gpu the helper draws the skinned instances from its table.
    /// `NIRENDER_SKIN_TABLE=0` keeps them on the engine's display with a palette in each draw.
    skin_table: bool,
    /// The bones of the instance in progress (`send_bones`), kept between the instances; the
    /// `INSTANCE_BONES` records, and the skinned instances hidden because their bones could
    /// not be made.
    scratch_bones: Vec<u8>,
    bone_records: u64,
    bone_failures: u64,
    /// The displays of skinned instances whose own ambient light or fog differed from the
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
    /// The draws of particle instances sent as `INSTANCE_DRAW` records (particles plan, step 2).
    particle_draws: u64,
    /// The particle instances made (the log names the first 100).
    particle_instances_made: u64,
    rejected_by: [u64; REASONS],
    /// The draws that `consider` did not take, by the first cause: not a plain world draw
    /// of a retained class, no geometry handle, no material handle, the path off, the
    /// material not modelled.
    skipped_by: [u64; 5],
}

static TABLES: MainCell<Option<Tables>> = MainCell::new(None);

/// The table, for a holder of the proof of the main thread; made at the first open.
fn tables(mt: &MainThread) -> &mut Tables {
    TABLES.get(mt).get_or_insert_with(|| Tables {
        instances: HashMap::default(),
        rejected: HashSet::default(),
        next_id: 1,
        frame: 0,
        generation: 1,
        pending: None,
        mode: None,
        world_camera: 0,
        scene_kind: crate::link::SCENE_OTHER,
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
        particle_draws: 0,
        particle_instances_made: 0,
        rejected_by: [0; REASONS],
        skipped_by: [0; 5],
    })
}

/// The mode, from `NIRENDER_RETAINED` at the first call.
pub unsafe fn mode(mt: &MainThread) -> Mode {
    let t = tables(mt);
    *t.mode.get_or_insert_with(|| std::env::var("NIRENDER_RETAINED").ok().and_then(|v| Mode::parse(&v)).unwrap_or(Mode::Gpu))
}

/// Sets the mode and returns the one before. A change of the mode sends the `INSTANCE`
/// record of each shape again at its next draw: the record says which mode draws it.
/// Engine-rules R18 (the skill wgpu-renderer-dev).
pub unsafe fn set_mode(mt: &MainThread, mode: Mode) -> Mode {
    let before = self::mode(mt);
    // Drain the cell worker's events first: a shape it destroyed leaves the table.
    crate::worker_events::drain();
    let t = tables(mt);
    t.mode = Some(mode);
    if mode != before {
        // In mode off the engine draws every shape, so the helper must hide its table. The
        // next display of a shape in mode gpu shows its instance again.
        let hide = mode == Mode::Off && capture::recording();
        // Write no mark here: the cell worker can free a kept shape at any time. In mode
        // gpu `draw` marks a shape again at its next draw; in another mode the display
        // clears a stale mark (`skipped_display_timed`).
        let mut hidden = 0;
        for instance in t.instances.values_mut() {
            instance.sent = false;
            instance.lights.clear();
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
/// landscape, and class 0. Class 0 is a shape of the world scene under no root the classes
/// know: the actors and their parts, and objects hung elsewhere. By default every shape of
/// class 0 is retained, skinned or not. `NIRENDER_ACTORS=0` keeps class 0 out;
/// `NIRENDER_ANIMATED=0` keeps out the class 0 shapes that are not skinned.
pub unsafe fn class_retained(mt: &MainThread, class: u32, skinned: bool) -> bool {
    let t = tables(mt);
    class == CLASS_OBJECT || (class == CLASS_LANDSCAPE && t.landscape) || (class == CLASS_UNKNOWN && t.actors && (skinned || t.animated)) || (sky_table_class(class) && sky_table_on())
}

/// Sets or clears the mark of a retained shape.
unsafe fn mark(shape: usize, on: bool) {
    let flags: u16 = rd(shape, FLAGS);
    wr::<u16>(shape, FLAGS, if on { flags | RETAINED_BIT } else { flags & !RETAINED_BIT });
}

/// True when the retained path is in use: a mode other than off, and the matrices of the
/// draws are made by the plugin (the exact translations exist then).
unsafe fn enabled(mt: &MainThread) -> bool {
    mode(mt) != Mode::Off && crate::camera_space::own()
}

/// Forgets which instances were sent: for the start of a capture file from the stream, so
/// that the next draw of each sends its `INSTANCE` record into the file. The capture purges
/// the source textures too, so the handles of every instance get a check.
pub unsafe fn reset(mt: &MainThread) {
    let t = tables(mt);
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
pub unsafe fn scene_begin(mt: &MainThread, kind: u32, camera: usize) {
    // The shapes destroyed on another thread leave the tables before any walk of the scene.
    crate::worker_events::drain();
    tables(mt).world_camera = if kind == SCENE_WORLD { camera } else { 0 };
    tables(mt).scene_kind = kind;
    if kind == SCENE_WORLD && camera != 0 {
        crate::animation::note_camera(mt, camera);
    }
    // The lights of the world go before the draws of the world.
    if kind == SCENE_WORLD {
        crate::world_lights::send(mt);
    }
}

/// A scene of the game ends. At the end of the world scene in mode gpu: an instance the
/// helper shows that the engine did not draw although it is in the view of the camera is
/// hidden (`INSTANCE_STATE`); an instance the engine drew was made visible at its draw.
/// An instance whose record did not go again since a reset or a change of the mode is
/// hidden too: the helper's table keeps it from before.
/// Engine-rules R14, R16 (the skill wgpu-renderer-dev).
pub unsafe fn scene_end(mt: &MainThread) {
    let t = tables(mt);
    let camera = core::mem::replace(&mut t.world_camera, 0);
    if camera == 0 || mode(mt) != Mode::Gpu || !capture::recording() {
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
        let bound = if instance.skinned { instance.posed_bound.as_ptr() } else { instance.bound.as_ptr() } as usize;
        let in_view = planes.iter().all(|&plane| thiscall1(WHICH_SIDE, bound, plane as u32) != 2);
        if in_view {
            instance.visible = false;
            capture::instance_state(instance.id, false);
            t.states += 1;
        }
    }
}


/// The helper draws the skinned instances from its table: mode gpu, and the switch on.
unsafe fn table_draws_skins(t: &Tables) -> bool {
    t.mode == Some(Mode::Gpu) && t.skin_table
}

/// Whether the table draws the skinned instances (`NIRENDER_SKIN_TABLE`, `nirender.skinTable`).
pub unsafe fn skin_table(mt: &MainThread) -> bool {
    tables(mt).skin_table
}

/// Sets whether the table draws the skinned instances (`nirender.skinTable`) and returns
/// the state before. Each skinned instance sends its record again at its next draw: on,
/// `draw` marks it; off, its display clears the mark, and its draw sends the record without
/// the table's flag, which takes it out of the helper's table.
pub unsafe fn set_skin_table(mt: &MainThread, on: bool) -> bool {
    let t = tables(mt);
    let before = t.skin_table;
    if on == before {
        return before;
    }
    // Drain the cell worker's events first, as `set_mode` does.
    crate::worker_events::drain();
    let t = tables(mt);
    t.skin_table = on;
    // Write no mark here, as in `set_mode`: `draw` marks a skinned shape again at its next
    // draw, and the display clears a stale mark when the table does not draw skins.
    for instance in t.instances.values_mut() {
        if !instance.skinned {
            continue;
        }
        instance.sent = false;
        instance.lights.clear();
    }
    before
}

/// Sends the bones of a skinned instance (`INSTANCE_BONES`) with the shape's world bound.
/// Returns false when the bones cannot be made: the instance is hidden then, and counted.
unsafe fn send_bones(mt: &MainThread, shape: usize, id: u32) -> bool {
    let t = tables(mt);
    let mut bones = core::mem::take(&mut t.scratch_bones);
    let result = crate::camera_space::skin_bones(mt, shape, rd(shape, GEOMETRY_SKIN), &mut bones);
    let t = tables(mt);
    let sent = match result {
        Ok(count) => {
            let posed: [f32; 4] = rd(shape, WORLD_BOUND);
            capture::instance_bones(id, count, &posed, &bones);
            t.bone_records += 1;
            if let Some(instance) = t.instances.get_mut(&shape) {
                instance.posed_bound = posed;
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
pub unsafe fn textures_changed(mt: &MainThread) {
    let t = tables(mt);
    t.generation = t.generation.wrapping_add(1);
}

/// The start of a draw of the engine: a pending draw that no shape record took (a skinned
/// shape whose palette failed, or one skinned in software) must not reach the next record.
pub unsafe fn begin_draw(mt: &MainThread) {
    tables(mt).pending = None;
}

/// The end of a frame.
pub unsafe fn frame_tick(mt: &MainThread) {
    crate::worker_events::drain();
    // The change notes that Lua made on another thread (MWSE fires some events on the cell
    // worker): the main thread applies them here.
    let queued = OFF_THREAD_CHANGES.lock().map(|mut q| core::mem::take(&mut *q)).unwrap_or_default();
    for address in queued {
        note_change(mt, address);
    }
    apply_changes(mt);
    let t = tables(mt);
    t.frame = t.frame.wrapping_add(1);
}

/// The shape is deleted (the destructor of `NiGeometry`): its instance goes, with a record
/// for the helper when the helper had it. True when the shape was an instance.
pub unsafe fn forget(mt: &MainThread, shape: usize) -> bool {
    let t = tables(mt);
    t.rejected.remove(&shape);
    if let Some(instance) = t.instances.remove(&shape) {
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
pub unsafe fn seen(mt: &MainThread, shape: usize, kind: u32, class_out: &mut u32) -> Option<(u32, u32)> {
    if mode(mt) != Mode::Gpu {
        return None;
    }
    let t = tables(mt);
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
pub unsafe fn consider(mt: &MainThread, shape: usize, eligible: bool, skinned: bool, points: bool, class: u32, geometry: u32, material: u32, property_state: usize, effect_state: usize) {
    let cause = if !eligible {
        Some(0)
    } else if geometry == 0 {
        Some(1)
    } else if material == 0 {
        Some(2)
    } else if !enabled(mt) {
        Some(3)
    } else if !crate::material_stream::last_modelled(mt) {
        Some(4)
    } else {
        None
    };
    let t = tables(mt);
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
    if let Some(reason) = reject_reason(mt, shape, property_state, effect_state, skinned, &mut live) {
        t.rejected.insert(shape);
        t.rejected_by[reason] += 1;
        return;
    }
    // A particle system changes its particles every frame: live from the start.
    if points {
        live = true;
        t.particle_instances_made += 1;
    }
    if points && t.particle_instances_made <= 100 {
        let text: usize = rd(shape, 8);
        let name = if text != 0 { core::ffi::CStr::from_ptr(text as *const core::ffi::c_char).to_string_lossy().into_owned() } else { String::new() };
        let parent: usize = rd(shape, crate::gen::layouts::NiAVObject::parentNode);
        let parent_text: usize = if parent != 0 { rd(parent, crate::gen::layouts::NiObjectNET::name) } else { 0 };
        let parent_name = if parent_text != 0 { core::ffi::CStr::from_ptr(parent_text as *const core::ffi::c_char).to_string_lossy().into_owned() } else { String::new() };
        crate::log!("particles: instance {} of class {class}, '{name}' under '{parent_name}'", t.next_id);
    }
    // The engine changes the sky by writes of its vertices with a new revision, not with
    // controllers: a sky instance is live from the start.
    if sky_table_class(class) {
        live = true;
        let text: usize = rd(shape, 8);
        let name = if text != 0 { core::ffi::CStr::from_ptr(text as *const core::ffi::c_char).to_string_lossy().into_owned() } else { String::new() };
        crate::log!("sky: instance {} of class {class}, '{name}'", t.next_id);
    }
    let id = t.next_id;
    t.next_id = t.next_id.wrapping_add(1).max(1);
    let generation = t.generation;
    t.instances.insert(shape, Instance { id, class, geometry, material, rotation: [0; 12], stored: [0; 3], colours: [0; 14], bound: [0.0; 4], posed_bound: [0.0; 4], transform: [0; TRANSFORM_WORDS], lights: Vec::new(), ambient: 0xFF00_0000, drawn: t.frame, visible: true, sent: false, relative: false, filled: false, generation, skinned, live, recheck: false, kind: 0, properties: [0; PROPERTY_SLOTS], queued: u32::MAX, points });
    t.pending = Some((shape, id));
}

/// Why the shape is not retained, or [`None`] when it is of the safe class. `live` is set
/// when a controller on the shape or on a property changes it.
/// Engine-rules R1, R6, R7 (the skill wgpu-renderer-dev).
unsafe fn reject_reason(mt: &MainThread, shape: usize, property_state: usize, effect_state: usize, skinned: bool, live: &mut bool) -> Option<usize> {
    // A controller on the shape changes its data (a morpher, a UV controller): the shape is
    // live. A controller on a node above only moves the shape, and `skipped_display` moves
    // the instance in place.
    let t = tables(mt);
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
        if kind_rejected(mt, node, skinned) {
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
        // The test function of NiZBufferProperty is the NetImmerse enum, not D3DCMPFUNC:
        // 0 always, 1 less, 2 equal, 3 less or equal, 4 greater, 5 not equal, 6 greater or
        // equal, 7 never. With the test off (flags bit 0) the engine uses "always", so the
        // function counts only with the test on (engine-rules R1).
        let z: usize = rd(property_state, PS::propZBuffer);
        if z != 0 && rd::<u16>(z, PROPERTY_FLAGS) & 1 != 0 && !matches!(rd::<u32>(z, ZP::testFunction), 0 | 1 | 3 | 7) {
            return Some(REASON_DEPTH);
        }
    }
    // A texture effect is retained in any coordinate mode (engine-rules R24) when its
    // projection does not move: no controller on the effect or on a node above it
    // (`effect_retained`). The material record holds the projection.
    if effect_state != 0 {
        for effect in [rd::<usize>(effect_state, DES::environmentMap), rd::<usize>(effect_state, DES::fogMap)] {
            if effect != 0 && !effect_retained(effect) {
                return Some(REASON_EFFECT);
            }
        }
    }
    None
}

/// The switch of the sky (`NIRENDER_SKY`): 0 not read, 1 off (`off`), 2 the census (`count`),
/// 3 the sky's instances in the table (the default; milestone 6).
static SKY_MODE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
/// The sky shapes the census logged: once each.
static SKY_SEEN: MainCell<Vec<usize>> = MainCell::new(Vec::new());

fn sky_mode() -> u8 {
    use core::sync::atomic::Ordering::Relaxed;
    match SKY_MODE.load(Relaxed) {
        0 => {
            let mode = match std::env::var("NIRENDER_SKY").as_deref() {
                Ok("off") => 1,
                Ok("count") => 2,
                _ => 3,
            };
            SKY_MODE.store(mode, Relaxed);
            mode
        }
        mode => mode,
    }
}

/// True when the census of the sky is on.
pub fn sky_census_on() -> bool {
    sky_mode() == 2
}

/// True when the table takes the sky's shapes (milestone 6): each draw of a sky shape goes as
/// an `INSTANCE_DRAW` record in the place of the engine's shape record (`draw`).
pub fn sky_table_on() -> bool {
    sky_mode() == 3
}

/// The draws of the sky instances go as `INSTANCE_DRAW` records (`nirender.skyDraws`; on by
/// default): off, they go as the engine's shape records, for an A/B in one session.
static SKY_DRAWS: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

/// True when the draws of the sky instances go as `INSTANCE_DRAW` records.
pub fn sky_draws() -> bool {
    SKY_DRAWS.load(core::sync::atomic::Ordering::Relaxed)
}

/// Sets whether the draws of the sky instances go as `INSTANCE_DRAW` records.
pub fn set_sky_draws(on: bool) {
    SKY_DRAWS.store(on, core::sync::atomic::Ordering::Relaxed);
}

/// The switch of the particle systems (`NIRENDER_PARTICLES`; milestone 5): 0 not read, 1 off
/// (`off`: the engine's shape records), 2 the table (the default since step 5: a particle system
/// in a world camera's scene is an instance whose draws go as `INSTANCE_PARTICLES` and
/// `INSTANCE_DRAW` records at the places of the engine's draws).
static PARTICLE_MODE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// True when the table takes the particle systems.
pub fn particle_table_on() -> bool {
    use core::sync::atomic::Ordering::Relaxed;
    let mode = match PARTICLE_MODE.load(Relaxed) {
        0 => {
            let mode = if std::env::var("NIRENDER_PARTICLES").as_deref() == Ok("off") { 1 } else { 2 };
            PARTICLE_MODE.store(mode, Relaxed);
            mode
        }
        mode => mode,
    };
    mode == 2
}

/// The draws of the particle instances go as `INSTANCE_DRAW` records (`nirender.particleDraws`;
/// on when the table takes them): off, they go as the engine's shape records, for an A/B in one
/// session.
static PARTICLE_DRAWS: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

/// True when the draws of the particle instances go as `INSTANCE_DRAW` records.
pub fn particle_draws() -> bool {
    PARTICLE_DRAWS.load(core::sync::atomic::Ordering::Relaxed)
}

/// Sets whether the draws of the particle instances go as `INSTANCE_DRAW` records.
pub fn set_particle_draws(on: bool) {
    PARTICLE_DRAWS.store(on, core::sync::atomic::Ordering::Relaxed);
}

/// What the table knows of a shape, for a log line: an instance (with its id and kind),
/// rejected, or not considered.
pub unsafe fn describe(mt: &MainThread, shape: usize) -> String {
    let t = tables(mt);
    if let Some(instance) = t.instances.get(&shape) {
        format!("instance {} (particles {}, class {}, live {})", instance.id, instance.points, instance.class, instance.live)
    } else if t.rejected.contains(&shape) {
        "rejected".to_string()
    } else {
        "not considered".to_string()
    }
}

/// True when the shape is a particle instance (`shape_stream::record` sends its particles
/// before its `INSTANCE_DRAW`, or keeps the engine's record when `particle_draws` is off).
pub unsafe fn is_points(mt: &MainThread, shape: usize) -> bool {
    tables(mt).instances.get(&shape).is_some_and(|instance| instance.points)
}

/// A class of the sky the table can take: the sky root's shapes in the world camera's walk.
/// The glare (class 8) draws in the first-person scene.
fn sky_table_class(class: u32) -> bool {
    (CLASS_SKY_DOME..=CLASS_SKY_OTHER).contains(&class)
}

/// The census of the sky (milestone 6, step 0): what the table would say about a shape of the
/// sky's classes (1 to 8), without a mark or a skip. Logs each shape once: the name, the class,
/// the handles, whether the material is modelled, the reject reason, live, the depth and alpha
/// flags, and the controllers on the shape, above it and on its properties.
pub unsafe fn sky_census(mt: &MainThread, shape: usize, class: u32, plain_world_shape: bool, geometry: u32, material: u32, property_state: usize, effect_state: usize) {
    let seen = &mut *SKY_SEEN.get(mt);
    if seen.contains(&shape) {
        return;
    }
    seen.push(shape);
    let text: usize = rd(shape, 8);
    let name = if text != 0 { core::ffi::CStr::from_ptr(text as *const core::ffi::c_char).to_string_lossy().into_owned() } else { String::new() };
    let mut live = false;
    let reason = reject_reason(mt, shape, property_state, effect_state, false, &mut live);
    let (z_flags, z_function) = match if property_state != 0 { rd::<usize>(property_state, PS::propZBuffer) } else { 0 } {
        0 => (None, None),
        z => (Some(rd::<u16>(z, PROPERTY_FLAGS)), Some(rd::<u32>(z, ZP::testFunction))),
    };
    let alpha_flags = match if property_state != 0 { rd::<usize>(property_state, PS::propAlpha) } else { 0 } {
        0 => None,
        alpha => Some(rd::<u16>(alpha, PROPERTY_FLAGS)),
    };
    let mut node_controllers = 0;
    let mut node: usize = rd(shape, PARENT);
    for _ in 0..MAX_DEPTH {
        if node == 0 {
            break;
        }
        if rd::<usize>(node, CONTROLLERS) != 0 {
            node_controllers += 1;
        }
        node = rd(node, PARENT);
    }
    let property_controllers = if property_state != 0 { (0..STATE_COUNT).filter(|slot| { let p: usize = rd(property_state, STATE_FIRST + slot * 4); p != 0 && rd::<usize>(p, CONTROLLERS) != 0 }).count() } else { 0 };
    crate::log!(
        "sky census: '{name}' class {class}, world shape {plain_world_shape}, geometry {geometry:#x}, material {material:#x}, modelled {}, reject {reason:?}, live {live}, zbuffer flags {z_flags:?} function {z_function:?}, alpha flags {alpha_flags:?}, controllers: shape {}, nodes above {node_controllers}, properties {property_controllers}",
        crate::material_stream::last_modelled(mt),
        rd::<usize>(shape, CONTROLLERS) != 0
    );
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
/// (with `NIRENDER_NODE_KINDS=0`, the `*_BEFORE` lists: the narrower one for a skinned shape).
unsafe fn kind_rejected(mt: &MainThread, node: usize, skinned: bool) -> bool {
    let kinds: &[&[u8]] = if tables(mt).node_kinds {
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
pub unsafe fn take_pending(mt: &MainThread) -> Option<(usize, u32)> {
    tables(mt).pending.take()
}

/// The causes of a rejection at the record, for the report: no geometry handle, no material
/// handle, a material the helper does not model, no exact translation.
static LATER_BY: MainCell<[u32; 4]> = MainCell::new([0; 4]);
const LATER_GEOMETRY: usize = 0;
const LATER_MATERIAL: usize = 1;
const LATER_NOT_MODELLED: usize = 2;
const LATER_TRANSLATION: usize = 3;

/// `reject_pending` with the cause counted.
unsafe fn reject_at_record(mt: &MainThread, shape: usize, cause: usize) {
    (*LATER_BY.get(mt))[cause] += 1;
    reject_pending(mt, shape, true);
}

/// The shape of a pending draw turned out not retainable at the record (a kind of draw
/// other than a shape, a picture of the Code Patch, no handle): it goes to the rejected set.
pub unsafe fn reject_pending(mt: &MainThread, shape: usize, reason_later: bool) {
    let t = tables(mt);
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
/// record), `effects` the stages of the texture effects of the draw and `fog` its fog, for
/// an `INSTANCE_DRAW` record.
/// Returns false when the shape has no exact translation (the draw then goes as a shape
/// record, and the shape is rejected).
pub unsafe fn draw(mt: &MainThread, shape: usize, id: u32, world: &[f32; 16], relative: bool, colours: &[u32; 14], effects: Option<&[u32]>, fog: Option<[u32; 4]>, ambient: u32, lights: &[u32], bones: &[u8], bone_count: u32, queued: bool) -> bool {
    let t = tables(mt);
    let Some(instance) = t.instances.get_mut(&shape) else { return false };
    if sky_table_class(instance.class) && !sky_draws() {
        return false;
    }
    if instance.points && !particle_draws() {
        return false;
    }
    // A skinned shape's world matrix is the identity (the palette has the transforms), so
    // only its colours and its first record send the instance again. In mode gpu the table
    // draws it with the bones of `send_bones`; with `NIRENDER_SKIN_TABLE=0` it stays on the
    // draw records, with its palette in each draw.
    let skinned = instance.skinned;
    // A sky instance goes as an `INSTANCE_DRAW` record at the place of the engine's draw (step
    // 2 of milestone 6): the helper draws it from its record there, in the stream's order. In
    // mode gpu the caller writes no shape record after it.
    // A particle instance goes the same way: its particles change every frame (step 2 of the
    // particles plan).
    let gpu = t.mode == Some(Mode::Gpu) && (!skinned || t.skin_table) && !sky_table_class(instance.class) && !instance.points;
    let rotation: [u32; 12] = core::array::from_fn(|i| world[i].to_bits());
    let stored: [u32; 3] = if skinned { [0; 3] } else { rd(shape, WORLD_TRANSLATION) };
    let bound: [f32; 4] = if skinned { [0.0; 4] } else { rd(shape, WORLD_BOUND) };
    let changed = !instance.filled || instance.rotation != rotation || instance.stored != stored || instance.colours != *colours || instance.bound.map(f32::to_bits) != bound.map(f32::to_bits);
    if changed || !instance.sent {
        let exact = if skinned {
            [0.0; 3]
        } else {
            let Some(exact) = crate::camera_space::exact_translation(mt, shape, 0) else {
                reject_at_record(mt, shape, LATER_TRANSLATION);
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
            send_bones(mt, shape, id);
        }
        return true;
    }
    capture::instance_draw(id, ambient, effects, fog, lights, bones, bone_count, queued);
    if instance.points {
        t.particle_draws += 1;
    }
    true
}

/// How a move of a sent instance goes (`NIRENDER_COMPACT_MOVES`): as `INSTANCE_MOVE` (the
/// default); as the whole `INSTANCE` record (`0`, as before, for the A/B); or both (`check`:
/// the helper compares the values of the two).
#[derive(Clone, Copy, PartialEq, Eq)]
enum CompactMoves {
    On,
    Off,
    Check,
}

fn compact_moves() -> CompactMoves {
    static MODE: std::sync::OnceLock<CompactMoves> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("NIRENDER_COMPACT_MOVES").as_deref() {
        Ok("0") => CompactMoves::Off,
        Ok("check") => CompactMoves::Check,
        _ => CompactMoves::On,
    })
}

/// Sends the `INSTANCE_MOVE` record of a sent instance: its rotation, its exact translation,
/// its stored translation and its bound.
unsafe fn send_move(instance: &Instance, exact: [f64; 3]) {
    let mut words = [0u32; 26];
    words[0] = instance.id;
    words[1..13].copy_from_slice(&instance.rotation);
    for (i, value) in exact.iter().enumerate() {
        let bits = value.to_bits();
        words[13 + i * 2] = bits as u32;
        words[14 + i * 2] = (bits >> 32) as u32;
    }
    words[19..22].copy_from_slice(&instance.stored);
    words[22..26].copy_from_slice(&instance.bound.map(f32::to_bits));
    capture::instance_move(&words);
}

/// Sends the words of the `INSTANCE` record of an instance of the mode gpu.
unsafe fn send_instance(mt: &MainThread, shape: usize, instance: &Instance, exact: [f64; 3], relative: bool) {
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
    (*RECORD_COUNTS.get(mt))[RC_NAME_BYTES] += name.len().min(96) as u64;
    capture::instance(&words, &name[..name.len().min(96)]);
}

/// The lights of an effect state that the light manager can turn on for a draw: not an
/// ambient light; in the order of the manager's list (the last of the state first). A light
/// that is off or too dim stays in the list: its record has zero colours
/// (`note_light_once`), so that it gives light again when it is turned on, with no new work
/// of the instance. The light manager's other gates (`setState` 0x6BB200: the replace mode
/// of the base map, the lighting mode with `useAmbientDiffuse` for `updateLights` 0x6BB6C0)
/// are rules of the helper's material (`host/src/material.rs`, `derive`): the lights are sent,
/// and a material that the engine lights with no lights does not use them.
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
unsafe fn note_light_once(mt: &MainThread, t: &mut Tables, light: usize, key: u32) {
    let counts = &mut *LIGHT_COUNTS.get(mt);
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
    if crate::light_stream::note_values(mt, key, &values, crate::link_client::point_light_radius(light)) {
        counts[LC_SENT] += 1;
    }
}

/// The counts of the instance records, by `RC_*`: why a record went (the first
/// send or a new handle; a move of the transform; the bound alone; the colours alone; a move
/// and the colours), whether a moved instance is live or not; the bytes of the names; the
/// records that went as `INSTANCE_MOVE`.
static RECORD_COUNTS: MainCell<[u64; 9]> = MainCell::new([0; 9]);
const RC_FIRST: usize = 0;
const RC_MOVED: usize = 1;
const RC_BOUND: usize = 2;
const RC_COLOURS: usize = 3;
const RC_MOVED_COLOURS: usize = 4;
const RC_LIVE: usize = 5;
const RC_STILL: usize = 6;
const RC_NAME_BYTES: usize = 7;
const RC_COMPACT_MOVES: usize = 8;
/// The time from the decision to send a record to its end, on the main thread: the matrix
/// (the engine's conversion of the world transform), the exact translation, the record.
static RECORD_TICKS: MainCell<[i64; 3]> = MainCell::new([0; 3]);

unsafe fn note_record_cause(mt: &MainThread, instance: &Instance, moved: bool, bound: bool, colours: bool) {
    let c = &mut *RECORD_COUNTS.get(mt);
    let cause = match (instance.sent, moved, bound, colours) {
        (false, ..) => RC_FIRST,
        (true, true, _, false) => RC_MOVED,
        (true, true, _, true) => RC_MOVED_COLOURS,
        (true, false, true, false) => RC_BOUND,
        _ => RC_COLOURS,
    };
    c[cause] += 1;
    if instance.sent && moved {
        c[if instance.live { RC_LIVE } else { RC_STILL }] += 1;
    }
}

pub unsafe fn record_report(mt: &MainThread) -> String {
    let c = *RECORD_COUNTS.get(mt);
    let parts = (*RECORD_TICKS.get(mt)).map(|t| t as f64 * 1000.0 / capture::ticks_per_second() as f64);
    let ms: f64 = parts.iter().sum();
    let total: u64 = c[RC_FIRST..=RC_MOVED_COLOURS].iter().sum();
    format!(
        "instance records: {} ({:.3} ms, {:.2} us each: the matrix {:.3} ms, the exact translation {:.3}, the record {:.3}); first or new handle {}, moved {}, bound alone {}, colours alone {}, moved and colours {}; the moved: live {}, not live {}; name bytes {}; as INSTANCE_MOVE {}",
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
        c[RC_STILL],
        c[RC_NAME_BYTES],
        c[RC_COMPACT_MOVES]
    )
}

/// The counts of the lights, by `LC_*`: the light loops of the displays and their
/// lights; the notes of a light, the entries built (one a light a frame), those with zero
/// colours (off or too dim), and the records sent.
static LIGHT_COUNTS: MainCell<[u64; 6]> = MainCell::new([0; 6]);
const LC_LOOPS: usize = 0;
const LC_LOOP_LIGHTS: usize = 1;
const LC_NOTES: usize = 2;
const LC_BUILT: usize = 3;
const LC_DARK: usize = 4;
const LC_SENT: usize = 5;
/// The time of the light loops of the displays.
static LIGHT_TICKS: MainCell<i64> = MainCell::new(0);

pub unsafe fn light_report(mt: &MainThread) -> String {
    let c = *LIGHT_COUNTS.get(mt);
    let ms = *LIGHT_TICKS.get(mt) as f64 * 1000.0 / capture::ticks_per_second() as f64;
    format!(
        "lights: display loops {} ({:.2} lights each, {:.3} ms, {:.2} us each); notes {}, entries built {} ({} with zero colours), records sent {}",
        c[LC_LOOPS],
        c[LC_LOOP_LIGHTS] as f64 / c[LC_LOOPS].max(1) as f64,
        ms,
        ms * 1000.0 / c[LC_LOOPS].max(1) as f64,
        c[LC_NOTES],
        c[LC_BUILT],
        c[LC_DARK],
        c[LC_SENT]
    )
}

/// Why an instance is checked again.
#[derive(Clone, Copy)]
pub enum Recheck {
    Properties = 0,
    Effects = 1,
    /// A Lua mod changed the shape or a property of it (`nirender.changed`).
    Notified = 2,
}

/// The rechecks asked for (by cause: 0 a property state, 1 an effect state, 2 a Lua
/// change), the rechecks done (3), the instances they rejected (4), and the instances whose
/// live state changed (5).
static RECHECKS: MainCell<[u64; 6]> = MainCell::new([0; 6]);

/// The objects a Lua mod changed (`nirender.changed`, from
/// `mod/MWSE/mods/nirender/notifier.lua`), and the counts of the notes received, of the
/// scans of the table and of the shapes hit.
static CHANGED: MainCell<Option<HashSet<usize>>> = MainCell::new(None);
static CHANGE_NOTES: MainCell<[u64; 3]> = MainCell::new([0; 3]);
/// The change notes made on another thread, for the main thread's next `frame_tick`.
static OFF_THREAD_CHANGES: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

/// A change note from a thread that is not the main thread (a Lua handler of an event that
/// MWSE fires on the cell worker): queued, never applied here.
pub fn queue_change(address: usize) {
    if address != 0 {
        if let Ok(mut queue) = OFF_THREAD_CHANGES.lock() {
            queue.push(address);
        }
    }
}

/// A Lua mod changed the object at `address` (a shape, a property, a controller's target).
/// The main thread only: other threads use `queue_change`.
pub unsafe fn note_change(mt: &MainThread, address: usize) {
    if address == 0 {
        return;
    }
    (*CHANGED.get(mt)).get_or_insert_with(HashSet::default).insert(address);
    (*CHANGE_NOTES.get(mt))[0] += 1;
}

/// Once a frame: each retained shape that is a changed object, or whose property state holds
/// one, is checked again at its next display. One pass over the table, only in a frame
/// with changes.
unsafe fn apply_changes(mt: &MainThread) {
    let Some(changed) = (*CHANGED.get(mt)).as_mut() else { return };
    if changed.is_empty() {
        return;
    }
    let notes = &mut *CHANGE_NOTES.get(mt);
    notes[1] += 1;
    let t = tables(mt);
    for (&shape, instance) in t.instances.iter_mut() {
        // Only the table's own copy of the properties: the cell worker can have freed the
        // shape since the last drain of its events.
        let hit = changed.contains(&shape) || instance.properties.iter().any(|&property| property != 0 && changed.contains(&property));
        if hit && !instance.recheck {
            instance.recheck = true;
            (*RECHECKS.get(mt))[Recheck::Notified as usize] += 1;
            notes[2] += 1;
        }
    }
    changed.clear();
}

/// A shape got a new property state or effect state (`ports::state_update`): its retained
/// instance, if it has one, is classified again and noted at its next display.
pub unsafe fn recheck(mt: &MainThread, shape: usize, cause: Recheck) {
    let t = tables(mt);
    if let Some(instance) = t.instances.get_mut(&shape) {
        instance.recheck = true;
        (*RECHECKS.get(mt))[cause as usize] += 1;
    }
}

/// The switch that notes the material and the geometry of every retained shape at every
/// display, not every [`CHECK_FRAMES`] frames (`NIRENDER_NOTE_EVERY=1`,
/// `nirender.noteEvery(on)`); 255 means not read yet. Then the counters of the cost of the
/// skipped displays (`display_cost`).
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

/// The cost of the display hook is timed only after the first `display_cost` call: the
/// timer reads at every display have a cost of their own.
static COST_ON: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// The sorted blends of the table in the engine's main sorted queue: the shapes the queue
/// took (0), their places written at the flush (1), and the places of shapes the flush
/// displayed without a queue entry of their own (2: the children of a sort adjust node the
/// queue took).
static QUEUE_COUNTS: MainCell<[u64; 3]> = MainCell::new([0; 3]);

/// True when a retained shape that the engine displays now may go into the engine's main
/// sorted queue: mode gpu, the skip on, a capture running, a scene of the world camera. The
/// shape ports then push the shape's state and ask the renderer's accumulator, as the
/// engine's display does (`NiTriShape::Display`); the queue sorts the table's blends with
/// the engine's own, and its flush gives their places (`flush_place`).
pub unsafe fn may_queue(mt: &MainThread) -> bool {
    let t = tables(mt);
    t.mode == Some(Mode::Gpu) && t.skip && capture::recording() && matches!(t.scene_kind, SCENE_WORLD | SCENE_SORTED_ALPHA | SCENE_WATER)
}

/// Notes that the engine's main sorted queue took the shape in this frame.
pub unsafe fn note_queued(mt: &MainThread, shape: usize) {
    let t = tables(mt);
    if let Some(instance) = t.instances.get_mut(&shape) {
        instance.queued = t.frame;
        QUEUE_COUNTS.get(mt)[0] += 1;
    }
}

/// At the flush of the engine's main sorted queue: true when the queue took the shape in
/// this frame. Its display work is done; the caller writes its place (`flush_place`).
pub unsafe fn queued_now(mt: &MainThread, shape: usize) -> bool {
    let t = tables(mt);
    t.instances.get(&shape).is_some_and(|i| i.queued == t.frame)
}

/// Writes the place of a retained shape in the flush of the engine's main sorted queue
/// (`INSTANCE_QUEUED`). `own` is false for a shape the flush displays without a queue entry
/// of its own.
pub unsafe fn flush_place(mt: &MainThread, shape: usize, own: bool) {
    let t = tables(mt);
    let Some(instance) = t.instances.get(&shape) else { return };
    capture::instance_queued(instance.id);
    QUEUE_COUNTS.get(mt)[if own { 1 } else { 2 }] += 1;
}

/// The places of the table's blends in the engine's walk (`walk_place`).
static WALK_PLACES: MainCell<u64> = MainCell::new(0);

/// Writes the place of a retained shape in the engine's walk (`INSTANCE_WALK`) when its alpha
/// property blends: the engine draws such a shape at its display when the main queue does
/// not take it (the no-sorter bit, no accumulator), so the helper draws it at this place, in
/// the walk's order. The caller calls it only for a shape the queue did not take. An opaque
/// shape gets no place.
/// Engine-rules R2 (the skill wgpu-renderer-dev).
pub unsafe fn walk_place(mt: &MainThread, shape: usize) {
    let t = tables(mt);
    let Some(instance) = t.instances.get(&shape) else { return };
    let property_state: usize = rd(shape, GEOMETRY_PROPERTY_STATE);
    let alpha: usize = if property_state != 0 { rd(property_state, PS::propAlpha) } else { 0 };
    if alpha == 0 || rd::<u16>(alpha, PROPERTY_FLAGS) & 1 == 0 {
        return;
    }
    capture::instance_walk(instance.id);
    *WALK_PLACES.get(mt) += 1;
}

/// The display of a retained shape by the engine (mode gpu): what the draw of the stream
/// did for the instance, without the draw. True when the engine must not display the
/// shape; false when the shape is not retained after all (the mark goes).
/// Engine-rules R5 (the skill wgpu-renderer-dev).
pub unsafe fn skipped_display(mt: &MainThread, shape: usize, kind: u32) -> bool {
    use core::sync::atomic::Ordering::Relaxed;
    if !COST_ON.load(Relaxed) {
        return skipped_display_timed(mt, shape, kind);
    }
    let start = capture::ticks();
    let result = skipped_display_timed(mt, shape, kind);
    COST_TICKS.fetch_add(capture::ticks() - start, Relaxed);
    COST_CALLS.fetch_add(1, Relaxed);
    result
}

unsafe fn skipped_display_timed(mt: &MainThread, shape: usize, kind: u32) -> bool {
    let t = tables(mt);
    // Clear a stale mark here, on the shape the engine displays: `set_mode` and
    // `set_skin_table` write no mark into kept shapes.
    if t.mode != Some(Mode::Gpu) {
        mark(shape, false);
        return false;
    }
    if !t.skip || !capture::recording() {
        return false;
    }
    // A scene that draws into a target of the game: the local map's tiles, the character
    // preview, the race menu's head, the engine's water reflection. The helper draws its table
    // only in the scenes of the world camera, so here the engine draws the shape on the record
    // path (the draw is not a plain world shape there, `link_client`). The mark stays: the
    // shape is still retained in the world scenes. Without this a map tile drawn at a walking
    // cross lost its landscape (records entry 55).
    if matches!(t.scene_kind, SCENE_RACE_MENU_HEAD | SCENE_CHARACTER_PREVIEW | SCENE_LOCAL_MAP | SCENE_WATER_REFLECTION) {
        return false;
    }
    if let Some(instance) = t.instances.get_mut(&shape) {
        if instance.skinned && !t.skin_table {
            mark(shape, false);
            return false;
        }
        if instance.kind == 0 {
            instance.kind = kind;
        }
    }
    display_work(mt, shape, kind)
}

/// The work of a display of a retained shape by the engine (`skipped_display`): the
/// recheck, the live controllers, the notes when due, the transform, the scene values, the
/// lights, the visible flag.
unsafe fn display_work(mt: &MainThread, shape: usize, kind: u32) -> bool {
    let t = tables(mt);
    let renderer = crate::engine::renderer();
    let (frame, generation) = (t.frame, t.generation);
    // A new property or effect state since the classification: classify again.
    if let Some((skinned, was_live)) = t.instances.get_mut(&shape).filter(|i| i.recheck).map(|i| {
        i.recheck = false;
        (i.skinned, i.live)
    }) {
        let rechecks = &mut *RECHECKS.get(mt);
        rechecks[3] += 1;
        let mut live = false;
        if reject_reason(mt, shape, rd(shape, GEOMETRY_PROPERTY_STATE), rd(shape, GEOMETRY_EFFECT_STATE), skinned, &mut live).is_some() {
            rechecks[4] += 1;
            reject_pending(mt, shape, true);
            return false;
        }
        let t = tables(mt);
        if let Some(instance) = t.instances.get_mut(&shape) {
            if instance.live != live {
                rechecks[5] += 1;
            }
            instance.live = live || was_live;
            instance.generation = instance.generation.wrapping_sub(1);
        }
    }
    let t = tables(mt);
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
    // The copy of the properties for `apply_changes` (the shape is alive here).
    instance.properties = if property_state != 0 { core::array::from_fn(|i| rd(property_state, PS::propAlpha + i * 4)) } else { [0; PROPERTY_SLOTS] };
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
    let due = note_every() || instance.live || !instance.sent || instance.generation != generation || id.wrapping_add(frame) % CHECK_FRAMES == 0;
    if due {
        COST_NOTES.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        // A skinned geometry is packed with the bone influences of its skin.
        let skin = if instance.skinned { rd(shape, GEOMETRY_SKIN) } else { 0 };
        let geometry = crate::geometry_stream::note(mt, kind, rd(shape, GEOMETRY_MODEL_DATA), skin, shape);
        let material = crate::material_stream::note(mt, renderer, property_state, effect_state);
        if geometry == 0 || material == 0 || !crate::material_stream::last_modelled(mt) {
            let cause = if geometry == 0 {
                LATER_GEOMETRY
            } else if material == 0 {
                LATER_MATERIAL
            } else {
                LATER_NOT_MODELLED
            };
            reject_at_record(mt, shape, cause);
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
    let colours = crate::shape_stream::material_values(property_state);
    let moved = instance.transform != transform;
    let bound_changed = instance.bound.map(f32::to_bits) != bound.map(f32::to_bits);
    let colours_changed = instance.colours != colours;
    let changed = moved || bound_changed || colours_changed;
    if changed || !instance.sent {
        let record_ticks = &mut *RECORD_TICKS.get(mt);
        let record_start = capture::ticks();
        note_record_cause(mt, instance, moved, bound_changed, colours_changed);
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
            let Some(exact) = crate::camera_space::exact_translation(mt, shape, 0) else {
                reject_at_record(mt, shape, LATER_TRANSLATION);
                return false;
            };
            exact
        };
        if changed {
            t.value_updates += 1;
        }
        // The plugin makes the matrices of every draw relative when it owns the camera
        // space: the record says so whether or not a draw happened this frame. A move of a
        // sent instance with the same colours and space goes as the compact record.
        let relative = crate::camera_space::own();
        let moves = compact_moves();
        let move_only = instance.sent && !colours_changed && instance.relative == relative && moves != CompactMoves::Off;
        instance.sent = true;
        instance.relative = relative;
        let exact_end = capture::ticks();
        if move_only {
            send_move(instance, exact);
            (*RECORD_COUNTS.get(mt))[RC_COMPACT_MOVES] += 1;
        }
        if !move_only || moves == CompactMoves::Check {
            send_instance(mt, shape, instance, exact, relative);
        }
        t.records += 1;
        let send_end = capture::ticks();
        record_ticks[0] += matrix_end - record_start;
        record_ticks[1] += exact_end - matrix_end;
        record_ticks[2] += send_end - exact_end;
    }
    // The bones of a skinned instance, in each frame the engine displays it.
    if skinned && !send_bones(mt, shape, id) {
        return false;
    }
    // The table draws a skinned part with the scene's ambient light and fog: count the parts
    // whose own values differ (the engine's light manager and render state would set those).
    if skinned && t.scene_frame == frame {
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
        let sun = scene_sun(mt, t);
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
        note_light_once(mt, t, light, key);
        keys.push(key);
    }
    let counts = &mut *LIGHT_COUNTS.get(mt);
    counts[LC_LOOPS] += 1;
    counts[LC_LOOP_LIGHTS] += lights.len() as u64;
    *LIGHT_TICKS.get(mt) += capture::ticks() - light_start;
    t.scratch_lights = lights;
    let Some(instance) = t.instances.get_mut(&shape) else {
        t.scratch_keys = keys;
        return false;
    };
    // The ambient light of the instance's own effect state goes with its lights: the helper
    // lights each instance with its own, as the engine's light manager does for each draw,
    // without its gates (see `state_lights`).
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

/// The values of the frame for the world, at the start of the world (`world_walk`), in every
/// mode: the ambient light of the lights on the world camera's root and on the game's world
/// root (`TES3Game` +0x9C), the fog of the cell's property, and the weather's sky light. The
/// sky light affects the node "worldRoot" (`WeatherController::initSkyLight` 0x43BB50); the
/// engine's ambient lights affect the effects' root and the menu (`TES3Game::initGame`
/// 0x4185E6, 0x418D6E), so the world's ambient is black unless a mod adds one. One record in a
/// frame: the table's shapes then count only a difference (`scene_mixed`).
pub unsafe fn world_scene(mt: &MainThread, camera_root: usize) {
    let t = tables(mt);
    let frame = t.frame;
    if t.scene_frame == frame {
        return;
    }
    let renderer = crate::engine::renderer();
    if renderer == 0 {
        return;
    }
    let game: usize = rd(TES3_GAME, 0);
    let world_root: usize = if game != 0 { rd(game, GAME_WORLD_ROOT) } else { 0 };
    let ambient = crate::ports::light_manager::ambient_of_effect_lists(&[camera_root, world_root]);
    let cell_fog = crate::engine::game_fog_property();
    let fog = if cell_fog != 0 { crate::shape_stream::fog_values_of(renderer, cell_fog) } else { None };
    t.scene_frame = frame;
    t.scene_values = (ambient, fog);
    let sun = scene_sun(mt, t);
    capture::retained_scene(ambient, fog, sun.as_ref());
}

/// `TES3Game` and its world root (MWSE `TES3Game.h`).
const TES3_GAME: usize = 0x7C6CDC;
const GAME_WORLD_ROOT: usize = crate::gen::layouts::TES3Game::worldRoot;

/// `WorldController` (0x7C67DC) -> `weatherController` -> `sgSkyLight`: the weather's
/// directional light, the sun of the hour (MWSE `TES3WeatherController.h`).
const WORLD_CONTROLLER: usize = 0x7C67DC;
const WEATHER_CONTROLLER: usize = crate::gen::layouts::WorldController::weatherController;
const SKY_LIGHT: usize = crate::gen::layouts::WeatherController::sgSkyLight;

/// The sun of the frame for the helper's own draws (the distant land, the shadows): the
/// weather controller's sky light, converted as the engine converts a light for the device
/// (`light_entry_update`, the 26 words of a `D3DLIGHT8`). The helper draws the retained
/// shapes with no draw of the engine, so it needs the sun from here. None when the light is
/// off or dim, as `note_light_once`.
unsafe fn scene_sun(mt: &MainThread, t: &mut Tables) -> Option<[u32; 26]> {
    static LOGGED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    static LAST_DIFFUSE: MainCell<[u32; 3]> = MainCell::new([0; 3]);
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
    if diffuse == *LAST_DIFFUSE.get(mt) {
        return Some(values);
    }
    *LAST_DIFFUSE.get(mt) = diffuse;
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

pub unsafe fn report(mt: &MainThread) -> String {
    let t = tables(mt);
    let reasons = ["controller", "node kind", "property controller", "effect", "at the record", "depth function"];
    let mut rejected: Vec<String> = reasons.iter().zip(t.rejected_by.iter()).filter(|(_, n)| **n != 0).map(|(r, n)| format!("{r} {n}")).collect();
    let later = *LATER_BY.get(mt);
    if later.iter().any(|&n| n != 0) {
        rejected.push(format!("at the record by cause: geometry {}, material {}, not modelled {}, translation {}", later[0], later[1], later[2], later[3]));
    }
    let causes = ["not a plain world draw of a retained class", "no geometry", "no material", "path off", "not modelled"];
    let skipped: Vec<String> = causes.iter().zip(t.skipped_by.iter()).filter(|(_, n)| **n != 0).map(|(r, n)| format!("{r} {n}")).collect();
    format!(
        "instances: mode {}, {} retained ({} live, {} skinned: the table draws them {}, {} bone records, {} hidden with no bones, {} works with other scene values), {} rejected ({}), draws not taken ({}), {} fast draws, {} displays skipped ({} with other scene values), {} instance records, {} updates of the handles, {} of the values, {} deletes, {} light records, {} state records",
        mode(mt).name(),
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
            "rechecks: asked by a new property state {}, by a new effect state {}, by a Lua change {}; done {}, rejected {}, live changed {}; Lua change notes {}, scans {}, shapes hit {}\n",
            RECHECKS.get(mt)[0], RECHECKS.get(mt)[1], RECHECKS.get(mt)[2], RECHECKS.get(mt)[3], RECHECKS.get(mt)[4], RECHECKS.get(mt)[5], CHANGE_NOTES.get(mt)[0], CHANGE_NOTES.get(mt)[1], CHANGE_NOTES.get(mt)[2]
        )
        + &format!(
            "sorted queue: {} blends of the table taken, {} places written at the flush, {} places of shapes under a queued node; {} places of blends in the walk\n",
            QUEUE_COUNTS.get(mt)[0], QUEUE_COUNTS.get(mt)[1], QUEUE_COUNTS.get(mt)[2], *WALK_PLACES.get(mt)
        )
        + &{
            let engine = crate::shape_stream::engine_particle_records(mt);
            format!(
                "particles: the table {}, draws as instance records {}; {} particle instances ({} made), {} draws as INSTANCE_DRAW records; as the engine's shape records {} in scenes of the world camera ({} frames), {} in other scenes\n",
                if particle_table_on() { "on" } else { "off" },
                if particle_draws() { "on" } else { "off" },
                t.instances.values().filter(|i| i.points).count(),
                t.particle_instances_made,
                t.particle_draws,
                engine[0],
                engine[2],
                engine[1]
            )
        }
        + &crate::animation::report()
        + "\n"
        + &light_report(mt)
        + "\n"
        + &record_report(mt)
        + "\n" + &crate::worker_events::report()
        + "\n" + &crate::freed_fill::report()
        + "\n" + &crate::camera_space::skin_report(mt)
}
