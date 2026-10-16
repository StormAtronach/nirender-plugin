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
//! object root, not skinned, not from the sorted queue, not blended, without a texture
//! effect, without projected lights or shadows, with no controller on it, on any node above
//! it or on any of its properties, and with no node above it that changes what it draws
//! (a billboard, a switch, a level of detail, a BSP node, an animation node, a particle
//! node). The class of a shape is decided once and kept until the shape is deleted.
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
//! (default `on`); `nirender.retained(mode)` sets it from Lua.
//!
//! The translation of an instance is the stored one of the shape's world transform (the
//! engine's 32-bit floats: the world matrix of a draw of the own device has it, as the host
//! of the own device reports no camera space and `camera_space` makes no relative matrix
//! there) and the exact one of the camera-relative matrices (`camera_space::exact_translation`,
//! 64-bit from the scene graph). The helper takes the stored one for a draw whose record
//! is not relative, and the exact one less the origin of the scene for a relative draw, as
//! the plugin does for a draw of the stream: the picture is the same to the bit. A parent
//! that moves by less than the step of a 32-bit float at the position of the shape leaves
//! the stored translation as it was and is not seen: a difference under one step of a
//! float, not checked.

use std::collections::{HashMap, HashSet};

use crate::capture;
use crate::engine::thiscall1;
use crate::gen::layouts::{NiCamera as C, NiDynamicEffectState as DES};
use crate::link::SCENE_WORLD;
use crate::mem::rd;

/// `NiObjectNET::controllers`, `NiAVObject::parentNode`, `NiAVObject::worldTransform` and
/// the translation in a transform.
const CONTROLLERS: usize = 0x10;
const PARENT: usize = 0x18;
const WORLD: usize = 0x40;
const WORLD_TRANSLATION: usize = WORLD + 0x24;
/// `NiAVObject::worldBound`: the centre and the radius.
const WORLD_BOUND: usize = 0x1c;
/// `NiBound::WhichSide(plane)`: 0 across the plane, 1 in front, 2 behind (out of the view).
const WHICH_SIDE: usize = 0x6F2660;
/// The flag of an instance record of the mode gpu: the helper draws it from its table.
const BLOCK: u32 = 0x100;
/// The property pointers of a `NiPropertyState`: alpha first, 12 of them.
const STATE_FIRST: usize = 0x8;
const STATE_COUNT: usize = 12;
/// `NiProperty::flags` (u16); bit 0 of the alpha property is the blend.
const PROPERTY_FLAGS: usize = 0x14;
/// `NiRTTI`: the name, then the base.
const RTTI_NAME: usize = 0x0;
const RTTI_BASE: usize = 0x4;
/// The most nodes above a shape that the walk looks at.
const MAX_DEPTH: usize = 32;
/// The frames between two checks of the handles of an instance.
const CHECK_FRAMES: u32 = 64;

/// The names of the node kinds that change what or how they draw: a shape under one of
/// them is not retained.
const REJECTED_KINDS: [&[u8]; 6] = [b"NiBillboardNode", b"NiSwitchNode", b"NiLODNode", b"NiBSPNode", b"BSAnimationNode", b"BSParticleNode"];

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
    /// The world bound of the shape: the centre and the radius.
    bound: [f32; 4],
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
    fast_draws: u64,
    records: u64,
    /// The instance records sent again: for changed handles, and for changed values of
    /// the draw (the transform, the colours).
    handle_updates: u64,
    value_updates: u64,
    deletes: u64,
    rejected_by: [u64; REASONS],
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
        fast_draws: 0,
        records: 0,
        handle_updates: 0,
        value_updates: 0,
        deletes: 0,
        rejected_by: [0; REASONS],
    })
}

/// The mode, from `NIRENDER_RETAINED` at the first call.
pub unsafe fn mode() -> Mode {
    let t = tables();
    *t.mode.get_or_insert_with(|| std::env::var("NIRENDER_RETAINED").ok().and_then(|v| Mode::parse(&v)).unwrap_or(Mode::On))
}

/// Sets the mode and returns the one before. A change of the mode sends the `INSTANCE`
/// record of each shape again at its next draw: the record says which mode draws it.
pub unsafe fn set_mode(mode: Mode) -> Mode {
    let before = self::mode();
    let t = tables();
    t.mode = Some(mode);
    if mode != before {
        for instance in t.instances.values_mut() {
            instance.sent = false;
            instance.lights.clear();
        }
    }
    before
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
}

/// A scene of the game begins: the camera of the world scene is kept for the check at its
/// end.
pub unsafe fn scene_begin(kind: u32, camera: usize) {
    tables().world_camera = if kind == SCENE_WORLD { camera } else { 0 };
}

/// A scene of the game ends. At the end of the world scene in mode gpu: an instance the
/// helper shows that the engine did not draw although it is in the view of the camera is
/// hidden (`INSTANCE_STATE`); an instance the engine drew was made visible at its draw.
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
        if !instance.filled || !instance.sent || !instance.visible || instance.drawn == frame {
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
    if !instance.filled || !instance.sent || instance.generation != t.generation || instance.id.wrapping_add(t.frame) % CHECK_FRAMES == 0 {
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
pub unsafe fn consider(shape: usize, eligible: bool, class: u32, geometry: u32, material: u32, property_state: usize, effect_state: usize) {
    if !eligible || geometry == 0 || material == 0 || !enabled() || !crate::material_stream::last_modelled() {
        return;
    }
    let t = tables();
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
    if let Some(reason) = reject_reason(shape, property_state, effect_state) {
        t.rejected.insert(shape);
        t.rejected_by[reason] += 1;
        return;
    }
    let id = t.next_id;
    t.next_id = t.next_id.wrapping_add(1).max(1);
    let generation = t.generation;
    t.instances.insert(shape, Instance { id, class, geometry, material, rotation: [0; 12], stored: [0; 3], colours: [0; 14], bound: [0.0; 4], lights: Vec::new(), drawn: t.frame, visible: true, sent: false, filled: false, generation });
    t.pending = Some((shape, id));
}

/// Why the shape is not retained, or `None` when it is of the safe class.
unsafe fn reject_reason(shape: usize, property_state: usize, effect_state: usize) -> Option<usize> {
    if rd::<usize>(shape, CONTROLLERS) != 0 {
        return Some(REASON_CONTROLLER);
    }
    let mut node: usize = rd(shape, PARENT);
    for _ in 0..MAX_DEPTH {
        if node == 0 {
            break;
        }
        if rd::<usize>(node, CONTROLLERS) != 0 {
            return Some(REASON_CONTROLLER);
        }
        if kind_rejected(node) {
            return Some(REASON_NODE_KIND);
        }
        node = rd(node, PARENT);
    }
    if property_state != 0 {
        for slot in 0..STATE_COUNT {
            let property: usize = rd(property_state, STATE_FIRST + slot * 4);
            if property != 0 && rd::<usize>(property, CONTROLLERS) != 0 {
                return Some(REASON_PROPERTY_CONTROLLER);
            }
        }
        let alpha: usize = rd(property_state, STATE_FIRST);
        if alpha != 0 && rd::<u16>(alpha, PROPERTY_FLAGS) & 1 != 0 {
            return Some(REASON_BLEND);
        }
    }
    if effect_state != 0 && (rd::<usize>(effect_state, DES::environmentMap) != 0 || rd::<usize>(effect_state, DES::fogMap) != 0) {
        return Some(REASON_EFFECT);
    }
    None
}

/// True when the run-time type of the node, or a base of it, is one of the rejected kinds.
unsafe fn kind_rejected(node: usize) -> bool {
    let mut rtti = crate::ports::ni::get_rtti(node);
    for _ in 0..16 {
        if rtti == 0 {
            return false;
        }
        let name: usize = rd(rtti, RTTI_NAME);
        if name != 0 {
            let text = core::ffi::CStr::from_ptr(name as *const core::ffi::c_char).to_bytes();
            if REJECTED_KINDS.iter().any(|kind| text.ends_with(kind)) {
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

/// The shape of a pending draw turned out not retainable at the record (a texture effect,
/// a kind of draw other than a shape): it goes to the rejected set.
pub unsafe fn reject_pending(shape: usize, reason_later: bool) {
    let t = tables();
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
pub unsafe fn draw(shape: usize, id: u32, world: &[f32; 16], relative: bool, colours: &[u32; 14], fog: Option<[u32; 4]>, ambient: u32, lights: &[u32]) -> bool {
    let t = tables();
    let gpu = t.mode == Some(Mode::Gpu);
    let Some(instance) = t.instances.get_mut(&shape) else { return false };
    let rotation: [u32; 12] = core::array::from_fn(|i| world[i].to_bits());
    let stored: [u32; 3] = rd(shape, WORLD_TRANSLATION);
    let bound: [f32; 4] = rd(shape, WORLD_BOUND);
    let changed = !instance.filled || instance.rotation != rotation || instance.stored != stored || instance.colours != *colours || instance.bound.map(f32::to_bits) != bound.map(f32::to_bits);
    if changed || !instance.sent {
        let Some(exact) = crate::camera_space::exact_translation(shape, 0) else {
            reject_pending(shape, true);
            return false;
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
        instance.filled = true;
        instance.sent = true;
        // The flag as the shape record of the draw has it: the helper takes the translation
        // of the two the same way. In mode gpu the helper draws the instance from its table.
        let flags = if relative { crate::shape_stream::RELATIVE } else { 0 } | if gpu { BLOCK } else { 0 };
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
        // The helper draws the instance: the changes of its lights and its visibility.
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
    capture::instance_draw(id, ambient, fog, lights);
    true
}

pub unsafe fn report() -> String {
    let t = tables();
    let reasons = ["controller", "node kind", "property controller", "blend", "effect", "at the record"];
    let rejected: Vec<String> = reasons.iter().zip(t.rejected_by.iter()).filter(|(_, n)| **n != 0).map(|(r, n)| format!("{r} {n}")).collect();
    format!(
        "instances: mode {}, {} retained, {} rejected ({}), {} fast draws, {} instance records, {} updates of the handles, {} of the values, {} deletes, {} light records, {} state records",
        mode().name(),
        t.instances.len(),
        t.rejected.len(),
        rejected.join(", "),
        t.fast_draws,
        t.records,
        t.handle_updates,
        t.value_updates,
        t.deletes,
        t.light_records,
        t.states
    )
}
