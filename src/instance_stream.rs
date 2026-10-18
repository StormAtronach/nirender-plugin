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
    /// The shape got a new property or effect state since its classification (plan 8b):
    /// its next display classifies it again and notes it.
    recheck: bool,
    /// The kind of its draw (`link::DRAW_SHAPE` or `DRAW_STRIPS`), 0 until known.
    kind: u32,
    /// The table walk (entry 13): the ancestors from the shape's parent up, each with its
    /// kind (`CHAIN_*`).
    chain: Vec<(usize, u8)>,
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
            } else if remark && instance.filled && !instance.skinned {
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
/// display runs its controllers and notes it), or a skinned one (its draw carries the
/// palette of its bones each frame). An instance the table draws is not the engine's.
pub unsafe fn engine_serves(shape: usize) -> bool {
    tables().instances.get(&shape).map_or(true, |instance| instance.live || instance.skinned)
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
    apply_changes();
    let t = tables();
    t.frame = t.frame.wrapping_add(1);
}

/// The shape is deleted (the destructor of `NiGeometry`): its instance goes, with a record
/// for the helper when the helper had it.
pub unsafe fn forget(shape: usize) {
    let t = tables();
    t.rejected.remove(&shape);
    if let Some(instance) = t.instances.remove(&shape) {
        table_forget(shape, &instance);
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
    t.instances.insert(shape, Instance { id, class, geometry, material, rotation: [0; 12], stored: [0; 3], colours: [0; 14], bound: [0.0; 4], transform: [0; TRANSFORM_WORDS], lights: Vec::new(), drawn: t.frame, visible: true, sent: false, filled: false, generation, skinned, live, recheck: false, kind: 0, chain: Vec::new(), walk_frame: 0, bucketed: false, counted_state: 0, counted_lights: Vec::new(), source_states: (0, 0), source_keys: Vec::new(), renote: false, effect_state: 0 });
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
            note_new(shape);
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
            if table_walk_on() {
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
        if table_walk_on() {
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
    let colours = if table && !due { instance.colours } else { crate::shape_stream::material_values(property_state) };
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
        capture::retained_scene(ambient, fog);
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

/// The table walk (F8 parts 3 and 4, option B, entry 13): on by default since the cut;
/// `NIRENDER_TABLE_WALK=0` or `nirender.tableWalk(false)` turns it off alone (an A/B until
/// CP4); the engine's walk (`live_walk::engine_walk_on`) turns it off with the other walks.
static TABLE_WALK: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(255);
/// The table walk's counters (`TW_*`) and its time.
static mut TABLE_COUNTS: [u64; 16] = [0; 16];
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
static TABLE_TICKS: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(0);
/// The part of `TABLE_TICKS` spent on the lists (the marks, the per-frame set, the sweep).
static TABLE_LIST_TICKS: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(0);
/// The sweep finds logged so far (the first 16 are).
static mut SWEEP_LOGGED: u32 = 0;
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

pub fn table_walk_on() -> bool {
    use core::sync::atomic::Ordering::Relaxed;
    if crate::live_walk::engine_walk_on() {
        return false;
    }
    let value = TABLE_WALK.load(Relaxed);
    if value != 255 {
        return value != 0;
    }
    let on = std::env::var("NIRENDER_TABLE_WALK").map_or(true, |v| v != "0");
    TABLE_WALK.store(on as u8, Relaxed);
    on
}

/// Sets the table walk; returns the state before. Turned on, its next run is a full pass
/// (no marks were kept while it was off).
pub fn set_table_walk(on: bool) -> bool {
    let before = table_walk_on();
    TABLE_WALK.store(on as u8, core::sync::atomic::Ordering::Relaxed);
    if on && !before {
        unsafe { TABLE_FULL = true };
    }
    before
}

/// The next table walk works every instance (`live_walk::set_engine_walk`).
pub fn table_full() {
    unsafe { TABLE_FULL = true };
}

/// The table walk works only the marked instances (entry 13). `NIRENDER_TABLE_MARKS=0` or
/// `nirender.tableMarks(false)`: it works every instance each frame (the walk of step 1), for
/// an A/B.
static TABLE_MARKS: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(255);

pub fn table_marks_on() -> bool {
    use core::sync::atomic::Ordering::Relaxed;
    let value = TABLE_MARKS.load(Relaxed);
    if value != 255 {
        return value != 0;
    }
    let on = std::env::var("NIRENDER_TABLE_MARKS").map_or(true, |v| v != "0");
    TABLE_MARKS.store(on as u8, Relaxed);
    on
}

/// Sets the marks; returns the state before. The next run is a full pass.
pub fn set_table_marks(on: bool) -> bool {
    let before = table_marks_on();
    TABLE_MARKS.store(on as u8, core::sync::atomic::Ordering::Relaxed);
    unsafe { TABLE_FULL = true };
    before
}

/// The dirty marks of the table walk (entry 13; the owner, 2026-10-17: events over tracking
/// where the engine offers them). The engine's functions store a pointer here and read
/// nothing more: a probe that read a node's children inside the app-cull hook crashed the
/// game at the night sky's load and at a cell change. The walk reads at display time, and
/// it uses a marked node only as a key (the node can be gone by then). A fixed array, so a
/// hook never allocates; a full array sets `overflow`, and the next walk is a full pass.
const MARK_CAPACITY: usize = 4096;

struct Marks {
    items: [usize; MARK_CAPACITY],
    len: usize,
    overflow: bool,
    count: u64,
}

impl Marks {
    const fn new() -> Marks {
        Marks { items: [0; MARK_CAPACITY], len: 0, overflow: false, count: 0 }
    }

    fn push(&mut self, at: usize) {
        self.count += 1;
        if self.len < MARK_CAPACITY {
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
        core::mem::replace(&mut self.overflow, false)
    }
}

/// A retained shape's world data was updated (`NiGeometry::UpdateWorldData`), or its instance
/// must be checked again (`recheck`, a Lua change).
static mut SHAPE_MARKS: Marks = Marks::new();
/// An object's app-cull flag changed (`NiAVObject::SetAppCulled`), or a Lua mod changed the
/// object (the notifier; `flags` is written with no call).
static mut CULL_MARKS: Marks = Marks::new();
/// A node's children changed, or an object was attached (the four tree functions of NiNode;
/// the destructor of a node detaches its children through one of them).
static mut TREE_MARKS: Marks = Marks::new();
/// A shape joined the table (its first draw marked it for the helper).
static mut NEW_MARKS: Marks = Marks::new();
/// A property, a texture effect or a shape's data changed by a time-varying writer
/// (`ports::controller_events`): the shapes that use it note their material and colours.
static mut SOURCE_MARKS: Marks = Marks::new();
/// The properties of a subtree changed in place (`ActorAnimController::setOpacity`): the
/// shapes below are checked again (their material and colours noted).
static mut MATERIAL_MARKS: Marks = Marks::new();
/// The next walk works every instance: the first run, the switch turned on, a change of the
/// mode or of the marks switch, a lost mark.
static mut TABLE_FULL: bool = true;
/// The generation of the tables at the last walk: a new one (a reset, a change of the device
/// textures) asks every instance for a check of its handles, so the walk is a full pass.
static mut TABLE_GENERATION: u32 = 0;

/// The retained shapes below each node of their chains: a node's app-cull or tree mark
/// reaches its shapes through this map, without reading the node.
static mut TABLE_BELOW: Option<HashMap<usize, HashSet<usize>>> = None;
/// The shapes worked every frame: those under a switch, a level, an flt or a billboard node
/// (the choice and the turn follow the camera or the time), and the live ones (their
/// controllers run at the display).
static mut TABLE_DYNAMIC: Option<HashSet<usize>> = None;
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
            }
        }
    }
    for &(node, _) in fresh.iter() {
        below.entry(node).or_default().insert(shape);
    }
    core::mem::swap(&mut instance.chain, fresh);
    if instance.live || instance.chain.iter().any(|&(_, kind)| kind != CHAIN_PLAIN) {
        table_dynamic().insert(shape);
    } else {
        table_dynamic().remove(&shape);
    }
    true
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
/// app-culled, and each switch, level and flt node selects the chain's child. Top down, as
/// the walk goes: a switch's selected child gets the lazy update, a level writes its index,
/// a billboard turns (once a frame each).
unsafe fn table_reachable(shape: usize, chain: &[(usize, u8)], camera: usize, touched: &mut HashSet<usize>) -> bool {
    if rd::<u16>(shape, FLAGS) & APP_CULLED != 0 {
        return false;
    }
    for (at, &(node, kind)) in chain.iter().enumerate().rev() {
        if rd::<u16>(node, FLAGS) & APP_CULLED != 0 {
            return false;
        }
        let child = if at == 0 { shape } else { chain[at - 1].0 };
        match kind {
            CHAIN_SWITCH | CHAIN_LOD | CHAIN_FLT => {
                let switch = match kind {
                    CHAIN_SWITCH => crate::live_walk::SwitchKind::Plain,
                    CHAIN_LOD => crate::live_walk::SwitchKind::LevelOfDetail,
                    _ => crate::live_walk::SwitchKind::FltAnimation,
                };
                let first = touched.insert(node);
                let index = crate::live_walk::switch_index_of(node, switch, camera);
                if first && kind == CHAIN_LOD {
                    crate::mem::wr(node, crate::ports::scene::SWITCH_INDEX, index);
                }
                if index < 0 || index >= rd::<i32>(node, crate::gen::layouts::NiNode::kChildren_endIndex) {
                    return false;
                }
                let selected: usize = rd(rd::<usize>(node, crate::gen::layouts::NiNode::kChildren_storage), index as usize * 4);
                if selected != child {
                    return false;
                }
                if first {
                    crate::ports::scene::switch_prepare_child(node, index, selected);
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
    lost |= (*core::ptr::addr_of_mut!(SHAPE_MARKS)).drain(drained);
    // The live walk's pending shapes below a node of an app-cull or tree mark (F8 part 3).
    crate::live_walk::release_pending(&culls, root);
    crate::live_walk::release_pending(&tree, root);
    crate::live_walk::note_tree_marks(&tree);
    if lost {
        counts[TW_LOST] += 1;
    }
    let full = lost || !table_marks_on() || note_every() || TABLE_FULL || TABLE_GENERATION != generation;
    TABLE_FULL = false;
    TABLE_GENERATION = generation;
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
        // 3. App-cull: the object itself, and the shapes below it. A node with no retained
        //    shape below (the HUD, the water, the effect roots) costs one look-up.
        for &at in culls.iter() {
            take(at, work);
            if let Some(set) = table_below().get(&at) {
                counts[TW_CULL_SHAPES] += set.len() as u64;
                scratch.clear();
                scratch.extend(set.iter().copied());
                for &shape in scratch.iter() {
                    take(shape, work);
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
            take(shape, work);
        }
        // 4b. The shapes of the source marks (their `renote` is set above).
        for &key in sources.iter() {
            take(key, work);
            if let Some(set) = table_sources().get(&key) {
                scratch.clear();
                scratch.extend(set.iter().copied());
                for &shape in scratch.iter() {
                    take(shape, work);
                }
            }
        }
        // 5. The per-frame shapes.
        dynamic_from = work.len();
        scratch.clear();
        scratch.extend(table_dynamic().iter().copied());
        for &shape in scratch.iter() {
            take(shape, work);
        }
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
    let gate = view_gate_on() && read_view_planes(camera);
    let gate_early = gate && view_gate() == 1;
    if gate {
        let planes = &*core::ptr::addr_of!(VIEW_PLANES);
        table_deferred().retain(|&shape| {
            if !tables().instances.contains_key(&shape) {
                return false;
            }
            if bound_in_view(rd(shape, AV_WORLD_BOUND), planes) {
                take(shape, work);
            }
            true
        });
    }
    let in_sweep = |at: usize| at >= sweep_from && at < sweep_end;
    counts[TW_ITEMS] += work.len() as u64;
    TABLE_LIST_TICKS.fetch_add(capture::ticks() - start, core::sync::atomic::Ordering::Relaxed);

    if places_on() {
        if !gate {
            read_view_planes(camera);
        }
        let places = &mut *core::ptr::addr_of_mut!(PLACES);
        // `NiAVObject::worldTransform.translation` of the camera.
        places.camera = rd(camera, 0x64);
        places.reach = capture::helper_shadow_reach();
    }
    let touched = (*core::ptr::addr_of_mut!(TABLE_TOUCHED)).get_or_insert_with(HashSet::default);
    touched.clear();
    let chain = &mut *core::ptr::addr_of_mut!(TABLE_CHAIN);
    table_frame_values();
    for at in 0..work.len() {
        let shape = work[at];
        // The lights of the frame come from the counted effect states: the count follows the
        // shape's state first, marked for the helper or not, reached or not. A state no shape
        // holds can be freed, and the lights read every counted state (the fault of dm17: in
        // mode off the marks are cleared, and the full pass at the return skipped the unmarked
        // shapes before their counts followed). The sources follow too (again at the sweep
        // and at a recheck: a state updated in place keeps its pointer).
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
        // Only the shapes the engine path skips (marked for the helper): a skinned shape is
        // never marked and stays the engine's draw.
        if rd::<u16>(shape, FLAGS) & RETAINED_BIT == 0 {
            continue;
        }
        // The gate of the view, early: an instance out of the view waits before the chain and
        // the reach (the reach's work on the switches, the levels and the billboards then
        // happens in the view only, as in the engine's walk).
        if gate_early && tables().instances.get(&shape).is_some_and(|i| i.filled) && out_of_view(shape, &*core::ptr::addr_of!(VIEW_PLANES)) {
            table_deferred().insert(shape);
            counts[TW_DEFERRED] += 1;
            continue;
        }
        if tables().instances.get(&shape).map_or(false, |i| i.chain.is_empty()) && set_chain(shape) {
            counts[TW_CHAINS] += 1;
        }
        let (filled, kind) = {
            let Some(instance) = tables().instances.get_mut(&shape) else { continue };
            if instance.live {
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
                i.visible && i.sent && !i.live && !i.recheck && !i.renote && i.generation == generation && i.id.wrapping_add(frame) % CHECK_FRAMES != 0
            }) && !chain.iter().any(|&(_, kind)| kind == CHAIN_BILLBOARD)
                // A waiting instance can have moved while out of the view: it gets the work.
                && !(gate && table_deferred().contains(&shape));
            if reach_only {
                if let Some(instance) = tables().instances.get_mut(&shape) {
                    instance.drawn = frame;
                }
                counts[TW_REACH_ONLY] += 1;
            } else if gate && !gate_early && out_of_view(shape, &*core::ptr::addr_of!(VIEW_PLANES)) {
                // The helper does not draw it: its work waits until the view reaches it.
                table_deferred().insert(shape);
                counts[TW_DEFERRED] += 1;
            } else if places_on() {
                if gate {
                    table_deferred().remove(&shape);
                }
                let source = if in_sweep(at) { 2 } else if at >= dynamic_from { 1 } else { 0 };
                let records = tables().records;
                let begin = capture::ticks();
                display_work(shape, kind, true);
                let place = place_of(shape, &*core::ptr::addr_of!(VIEW_PLANES));
                let places = &mut *core::ptr::addr_of_mut!(PLACES);
                places.worked[source][place] += 1;
                places.ticks[source][place] += capture::ticks() - begin;
                places.records[source][place] += (tables().records != records) as u64;
                places.live[place] += tables().instances.get(&shape).is_some_and(|i| i.live) as u64;
                counts[TW_WORKED] += 1;
            } else {
                if gate {
                    table_deferred().remove(&shape);
                }
                display_work(shape, kind, true);
                counts[TW_WORKED] += 1;
            }
        } else if let Some(instance) = tables().instances.get_mut(&shape) {
            if gate {
                table_deferred().remove(&shape);
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
    // The check of the gate (with the measure of the places): an instance the helper shows in
    // the view must have the shape's transform of this frame.
    if places_on() && !(*core::ptr::addr_of!(VIEW_PLANES)).is_empty() {
        let planes = &*core::ptr::addr_of!(VIEW_PLANES);
        let places = &mut *core::ptr::addr_of_mut!(PLACES);
        for (&shape, instance) in tables().instances.iter() {
            if !instance.filled || !instance.visible || rd::<u16>(shape, FLAGS) & RETAINED_BIT == 0 || !bound_in_view(rd(shape, AV_WORLD_BOUND), planes) {
                continue;
            }
            places.checked += 1;
            if rd::<[u32; TRANSFORM_WORDS]>(shape, WORLD) != instance.transform {
                places.stale += 1;
            }
        }
    }
    table_frame_lights();
    TABLE_TICKS.fetch_add(capture::ticks() - start, core::sync::atomic::Ordering::Relaxed);
}

/// Once a frame before the instances: the scene values (the ambient light of the effect
/// state and the weather's fog, as the first shape gave them before; the values are the same
/// for every shape in a frame: `scene_mixed` counted 0 in every run).
unsafe fn table_frame_values() {
    let t = tables();
    let renderer = crate::engine::renderer();
    if renderer == 0 || t.scene_frame == t.frame {
        return;
    }
    let Some(shape) = t.instances.keys().copied().find(|&s| rd::<u16>(s, FLAGS) & RETAINED_BIT != 0) else { return };
    let effect_state: usize = rd(shape, GEOMETRY_EFFECT_STATE);
    let property_state: usize = rd(shape, GEOMETRY_PROPERTY_STATE);
    let ambient = crate::ports::light_manager::ambient_of_state(effect_state);
    let cell_fog = crate::engine::game_fog_property();
    let fog = if cell_fog != 0 {
        crate::shape_stream::fog_values_of(renderer, cell_fog)
    } else {
        crate::shape_stream::fog_values(renderer, property_state)
    };
    t.scene_frame = t.frame;
    t.scene_values = (ambient, fog);
    capture::retained_scene(ambient, fog);
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
        }
        if table_walk_on() {
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

/// The places of the worked instances (`NIRENDER_TABLE_PLACES=1`, a measure of 2026-10-18):
/// for each source of the work (a mark or a full pass, the per-frame set, the sweep) and
/// each place of the shape after the work (in the camera's view; out of it within the reach
/// of the helper's shadow map; neither), the instances worked, the time of their work and
/// the INSTANCE records they sent; and the live ones by place.
struct Places {
    worked: [[u64; 3]; 3],
    ticks: [[i64; 3]; 3],
    records: [[u64; 3]; 3],
    live: [u64; 3],
    camera: [f32; 3],
    reach: f32,
    /// The instances shown in the view after the walk, and those whose transform was not the
    /// shape's (the check of the gate).
    checked: u64,
    stale: u64,
}
static mut PLACES: Places = Places { worked: [[0; 3]; 3], ticks: [[0; 3]; 3], records: [[0; 3]; 3], live: [0; 3], camera: [0.0; 3], reach: 0.0, checked: 0, stale: 0 };
static PLACES_ON: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(255);

/// `NiAVObject::worldBound`: the centre and the radius.
const AV_WORLD_BOUND: usize = crate::gen::layouts::NiAVObject::worldBound;

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

/// The place of the gate: 0 off, 1 early (before the chain and the reach: the default), 2 late
/// (after the reach; the first form, for the A/B). `NIRENDER_TABLE_VIEW=0` / `late`.
fn view_gate() -> u8 {
    let value = VIEW_GATE.load(core::sync::atomic::Ordering::Relaxed);
    if value != 255 {
        return value;
    }
    let gate = match std::env::var("NIRENDER_TABLE_VIEW").as_deref() {
        Ok("0") => 0,
        Ok("late") => 2,
        _ => 1,
    };
    VIEW_GATE.store(gate, core::sync::atomic::Ordering::Relaxed);
    gate
}

fn view_gate_on() -> bool {
    view_gate() != 0
}

unsafe fn table_deferred() -> &'static mut HashSet<usize> {
    (*core::ptr::addr_of_mut!(TABLE_DEFERRED)).get_or_insert_with(HashSet::default)
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

fn places_on() -> bool {
    let value = PLACES_ON.load(core::sync::atomic::Ordering::Relaxed);
    if value != 255 {
        return value != 0;
    }
    let on = std::env::var("NIRENDER_TABLE_PLACES").is_ok_and(|v| v == "1");
    PLACES_ON.store(on as u8, core::sync::atomic::Ordering::Relaxed);
    on
}

/// 0 in the camera's view (the shape's world bound against the culling planes, as
/// `CullShow` tests it), 1 out of it within the shadow map's reach, 2 neither.
unsafe fn place_of(shape: usize, planes: &[[f64; 4]]) -> usize {
    let bound: [f32; 4] = rd(shape, AV_WORLD_BOUND);
    if !planes.is_empty() && bound_in_view(bound, planes) {
        return 0;
    }
    let places = &*core::ptr::addr_of!(PLACES);
    let centre = [bound[0], bound[1], bound[2]];
    let radius = bound[3];
    let d: [f32; 3] = core::array::from_fn(|i| centre[i] - places.camera[i]);
    if places.reach > 0.0 && (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() - radius <= places.reach {
        1
    } else {
        2
    }
}

/// The line of the places, or an empty text when the measure is off.
pub unsafe fn places_report() -> String {
    if !places_on() {
        return String::new();
    }
    let p = &*core::ptr::addr_of!(PLACES);
    let runs = (*core::ptr::addr_of!(TABLE_COUNTS))[TW_RUNS].max(1) as f64;
    let per_second = capture::ticks_per_second() as f64;
    let sources = ["marks", "per-frame", "sweep"];
    let places = ["in view", "shadow reach", "neither"];
    let mut parts = Vec::new();
    for (s, source) in sources.iter().enumerate() {
        for (q, place) in places.iter().enumerate() {
            parts.push(format!(
                "{source}/{place} {:.1} ({:.4} ms, records {:.1})",
                p.worked[s][q] as f64 / runs,
                p.ticks[s][q] as f64 * 1000.0 / per_second / runs,
                p.records[s][q] as f64 / runs
            ));
        }
    }
    format!(
        "table walk places (a run): {}; live in view {:.1}, shadow reach {:.1}, neither {:.1}; shadow reach {:.0}; the check: shown in the view {}, with a stale transform {}\n",
        parts.join(", "),
        p.live[0] as f64 / runs,
        p.live[1] as f64 / runs,
        p.live[2] as f64 / runs,
        p.reach,
        p.checked,
        p.stale
    )
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
    format!(
        "table walk (entry 13): {}, marks {}; runs {}, full passes {} (marks lost in {}); items {:.1} a run (swept {:.1}), worked {:.1}, reach only {:.1}, waiting out of the view {:.1} (gate {}, {} now), hidden {}, not under the root {}; chains changed {}, sweep finds {}; marks: shape {}, app-cull {} (shapes reached {}), tree {} (shapes reached {}), material {} (shapes reached {}), source {} (shapes reached {}), new {}; per-frame shapes {} (live {}, switch {}, level {}, flt {}, billboard {}), nodes with shapes below {}, sources {}, effect states {}; time {:.3} ms ({:.4} ms a run, the lists {:.4})",
        if table_walk_on() { "on" } else { "off" },
        if table_marks_on() { "on" } else { "off" },
        c[TW_RUNS], c[TW_FULL], c[TW_LOST],
        c[TW_ITEMS] as f64 / runs, c[TW_SWEPT] as f64 / runs, c[TW_WORKED] as f64 / runs, c[TW_REACH_ONLY] as f64 / runs, c[TW_DEFERRED] as f64 / runs,
        ["off", "early", "late"][view_gate().min(2) as usize], table_deferred().len(), c[TW_HIDDEN], c[TW_OFF_ROOT],
        c[TW_CHAINS], c[TW_SWEEP_FINDS],
        marks[0], marks[1], c[TW_CULL_SHAPES], marks[2], c[TW_TREE_SHAPES], marks[4], c[TW_MATERIAL_SHAPES], marks[5], c[TW_SOURCE_SHAPES], marks[3],
        table_dynamic().len(), kinds[0], kinds[1], kinds[2], kinds[3], kinds[4], table_below().len(), table_sources().len(), table_state_counts().len(),
        ms, ms / runs, list_ms / runs
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
        + &places_report()
        + &crate::ports::controller_events::report()
        + "\n" + &crate::script_names::report()
        + "\n" + &event_report()
        + "\n"
        + &crate::live_walk::report()
}
