//! What animates in mode gpu while the stream records (`active`). The engine lets an actor
//! or an animated node compute its pose only when a Click displayed it in the frame before:
//! `NiNode::Display` (0x6C9190) sets the flag 0x10 of a node, `BSAnimationNode::Display` the
//! bit 8 of an animation node, and three readers consume them. The engine's walk sets these
//! flags itself. In mode gpu the readers' ports (`ports/animation.rs`) also set the flag on
//! the nodes that the policy selects here. The test is the node's world bound against the
//! planes of the world camera and, by the policy, the reach of the shadow map that the
//! helper reports (`capture::helper_shadow_reach`). A node that the engine's walk does not reach
//! is not selected: it, or a node above it, is app-culled; a switch or a level of detail above
//! it has another child selected, or none; a plain node above it has no geometry.
//!
//! The port only adds to the flag: the engine's walk still sets it for what it reaches. So a
//! node that this test withholds starts at the first Click that displays it, one frame later.
//! The ports compare the two (`displayed_by_walk`, W97): a miss is a node that the walk displayed
//! and the policy did not select. Without the walk, a miss is an animation that stops until the
//! policy selects the node, so the walk can go only after runs with no miss. The likely setter of
//! a miss is the camera whose planes hold the node, of the Clicks since the world camera's one
//! before its last: another camera than the world camera (the arm camera of the first person,
//! a menu's head) explains a miss. `ShadowManager::addObjectsAndLights` 0x435DE0 is a fourth reader
//! of the walk's flag, with no port: an actor farther than 256 units casts its stencil shadow only
//! when its visual node has the flag (0x435F6C), then the reader clears it (W113).
//!
//! The camera of these tests is the world camera of the last world scene (`note_camera`), for
//! each reader and each root. The engine's flag comes from the Click of a root's own camera, and
//! its distance test in `BSAnimationManager::Update` 0x6EE690 from the camera of the caller:
//! `Cell::simulate` 0x4E3490 passes the arm camera in the first person.
//!
//! The policy (`NIRENDER_ANIMATION`, `nirender.animationPolicy`): `view` animates what is
//! in the camera's view; `shadows` (the default) also what the shadow map reaches, so that a
//! shadow moves while its caster is out of view; `all` animates everything loaded.

use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use crate::engine::vcall0;
use crate::gen::layouts::NiAVObject as AV;
use crate::gen::layouts::NiCamera as C;
use crate::gen::layouts::{NiLODData as LEVEL, NiLODNode as LOD, NiNode as NODE, NiSwitchNode as SWITCH};
use crate::main_thread::{MainCell, MainThread};
use crate::mem::rd;

const SLOT_GET_WORLD_BOUND: usize = crate::gen::layouts::InterfaceNiAVObject::GetWorldBound;
const WORLD_TRANSLATION: usize = crate::gen::layouts::NiAVObject::worldTransform_translation;
const BOUND_RADIUS: usize = crate::gen::layouts::NiBound::radius;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    View = 0,
    ViewShadows = 1,
    All = 2,
}

const NAMES: [&str; 3] = ["view", "shadows", "all"];
/// 255: not read yet (the environment is read at the first call).
static POLICY: AtomicU8 = AtomicU8::new(255);
static BY_VIEW: AtomicU64 = AtomicU64::new(0);
static BY_SHADOWS: AtomicU64 = AtomicU64::new(0);
static BY_ALL: AtomicU64 = AtomicU64::new(0);
static NO_BOUND: AtomicU64 = AtomicU64::new(0);
static NEITHER: AtomicU64 = AtomicU64::new(0);
static WITHHELD_ITSELF: AtomicU64 = AtomicU64::new(0);
static WITHHELD_ABOVE: AtomicU64 = AtomicU64::new(0);
static WITHHELD_NOT_SELECTED: AtomicU64 = AtomicU64::new(0);
static WITHHELD_NO_GEOMETRY: AtomicU64 = AtomicU64::new(0);
/// The time in the walks up the chain of a node: the part of this test that a Click does not do.
static CHAIN_WALKS: AtomicU64 = AtomicU64::new(0);
static CHAIN_NANOSECONDS: AtomicU64 = AtomicU64::new(0);

/// A reader of the display flag, for the comparison with the engine's walk (W97).
#[derive(Clone, Copy)]
pub enum Reader {
    /// `BSAnimationManager::Update` 0x6EE690, the flag 0x10 of a managed node.
    Manager = 0,
    /// `BSAnimationNode::UpdateControllers` 0x6F2180, the displayed bit.
    Controllers = 1,
    /// `BSAnimationNode::UpdateDownwardPass` 0x6F2060, the displayed bit.
    DownwardPass = 2,
}
const READERS: usize = 3;
const READER_NAMES: [&str; READERS] = ["the manager", "UpdateControllers", "UpdateDownwardPass"];
/// By reader: the nodes that the engine's walk displayed, those of them that the policy did not
/// select (the misses), and the nodes that the policy selected and the walk did not display.
static WALK_DISPLAYED: [AtomicU64; READERS] = [const { AtomicU64::new(0) }; READERS];
static MISSES: [AtomicU64; READERS] = [const { AtomicU64::new(0) }; READERS];
static POLICY_ONLY: [AtomicU64; READERS] = [const { AtomicU64::new(0) }; READERS];
/// The misses that the log names: the first ones only.
const MISSES_NAMED: u64 = 8;
static MISSES_LOGGED: AtomicU64 = AtomicU64::new(0);

/// The camera whose Click likely set the flag of a miss: by the planes of the cameras clicked
/// since the world camera's Click before its last one [I: an attribution by the planes, not a
/// record of the set].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Setter {
    /// A camera other than the world camera holds the node's bound: the arm camera of the first
    /// person, a menu's head, a mod's view.
    OtherCamera = 0,
    /// Only the world camera holds it: the policy and the walk differ on the same camera.
    WorldCamera = 1,
    /// No camera clicked in that time holds it.
    Unknown = 2,
}
const SETTER_NAMES: [&str; 3] = ["another camera", "the world camera", "no camera's planes"];
static MISSES_BY_SETTER: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

/// A camera's Click: its planes as values, the camera's address as a key only.
struct Clicked {
    camera: usize,
    sequence: u64,
    planes: Vec<[f64; 4]>,
}

/// The Clicks: their number, the numbers of the world camera's last two Clicks, the world camera
/// (a key only, from `note_camera`), and the cameras clicked since the earlier of those two.
struct Clicks {
    count: u64,
    world_last: u64,
    world_before: u64,
    world_camera: usize,
    cameras: Vec<Clicked>,
}
static CLICKS: MainCell<Clicks> = MainCell::new(Clicks {
    count: 0,
    world_last: 0,
    world_before: 0,
    world_camera: 0,
    cameras: Vec::new(),
});

/// The world camera of the frame: the values of its culling planes (the normal and the
/// constant of each), its position and its level of detail adjustment. Keep values, not the
/// planes' addresses: the tests run in the next frame's update, after the scene that noted them.
struct Camera {
    planes: Vec<[f64; 4]>,
    position: [f32; 3],
    lod_adjust: f32,
}
static CAMERA: MainCell<Camera> = MainCell::new(Camera {
    planes: Vec::new(),
    position: [0.0; 3],
    lod_adjust: 1.0,
});

fn policy_of(name: &str) -> Option<Policy> {
    match name {
        "view" => Some(Policy::View),
        "shadows" => Some(Policy::ViewShadows),
        "all" => Some(Policy::All),
        _ => None,
    }
}

pub fn policy() -> Policy {
    let value = POLICY.load(Ordering::Relaxed);
    if value != 255 {
        return match value {
            0 => Policy::View,
            2 => Policy::All,
            _ => Policy::ViewShadows,
        };
    }
    let from_env =
        crate::switches::parsed("NIRENDER_ANIMATION", "the name of a policy", policy_of).unwrap_or(Policy::ViewShadows);
    POLICY.store(from_env as u8, Ordering::Relaxed);
    from_env
}

/// Sets the policy by its name. Returns false for a name that is not one.
pub fn set_policy_name(name: &str) -> bool {
    match policy_of(name) {
        Some(policy) => {
            POLICY.store(policy as u8, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

pub fn policy_name() -> &'static str {
    NAMES[policy() as usize]
}

/// True while the policy answers for the flag: the plugin's table draws the world (mode gpu)
/// and the stream is live. In the other modes the engine's walk sets the flags itself.
pub unsafe fn active(t: &MainThread) -> bool {
    crate::instance_stream::mode(t) == crate::instance_stream::Mode::Gpu && crate::capture::recording()
}

/// The world camera of the scene that begins: its planes and its position, for the tests
/// of this frame. From `instance_stream::scene_begin`.
pub unsafe fn note_camera(t: &MainThread, camera: usize) {
    let this = &mut *CAMERA.get(t);
    this.planes = planes_of(camera);
    this.position = rd(camera, WORLD_TRANSLATION);
    this.lod_adjust = rd(camera, C::fLODAdjust);
    CLICKS.get(t).world_camera = camera;
}

/// The values of a camera's culling planes: the normal and the constant of each.
///
/// # Safety
///
/// `camera` points at a live `NiCamera` with its planes.
unsafe fn planes_of(camera: usize) -> Vec<[f64; 4]> {
    let count = rd::<i32>(camera, C::countCullingPlanes).max(0) as usize;
    let storage: usize = rd(camera, C::cullingPlanes_storage);
    (0..count)
        .map(|i| {
            let plane: [f32; 4] = rd(rd::<usize>(storage, i * 4), 0);
            plane.map(f64::from)
        })
        .collect()
}

/// A camera's Click (`world_walk::click_cull_show`, for every Click): its planes, for the setter
/// of a miss (W97). Only while the policy answers.
///
/// # Safety
///
/// `camera` points at a live `NiCamera` with its planes.
pub unsafe fn note_click(t: &MainThread, camera: usize) {
    if camera == 0 || !active(t) {
        return;
    }
    let planes = planes_of(camera);
    let mut clicks = CLICKS.get(t);
    clicks.count += 1;
    let sequence = clicks.count;
    if camera == clicks.world_camera {
        clicks.world_before = clicks.world_last;
        clicks.world_last = sequence;
    }
    let oldest = clicks.world_before;
    clicks
        .cameras
        .retain(|clicked| clicked.camera != camera && clicked.sequence >= oldest);
    clicks.cameras.push(Clicked {
        camera,
        sequence,
        planes,
    });
}

/// True when the bound (its centre and its radius) is within all the planes: the rule of
/// `NiBound::WhichSide` 0x6F2660 [I: transcribed, not compared]: outside a plane when the signed
/// distance of the centre is at most minus the radius. No planes hold nothing.
fn within(planes: &[[f64; 4]], [x, y, z, radius]: [f64; 4]) -> bool {
    !planes.is_empty() && planes.iter().all(|p| p[0] * x + p[1] * y + p[2] * z - p[3] > -radius)
}

/// The setter of a miss from the cameras whose planes hold the bound: another camera first, as the
/// world camera's planes made the policy's no.
fn setter_among(bound: [f64; 4], cameras: &[(bool, &[[f64; 4]])]) -> Setter {
    let holders = || cameras.iter().filter(|(_, planes)| within(planes, bound));
    if holders().any(|(world, _)| !world) {
        Setter::OtherCamera
    } else if holders().next().is_some() {
        Setter::WorldCamera
    } else {
        Setter::Unknown
    }
}

/// [`setter_among`] for a node of a miss, from the Clicks noted since the world camera's Click
/// before its last one.
///
/// # Safety
///
/// As [`displayed`].
unsafe fn setter_of_miss(t: &MainThread, node: usize) -> Setter {
    let bound = vcall0(node, SLOT_GET_WORLD_BOUND) as usize;
    if bound == 0 {
        return Setter::Unknown;
    }
    let values = rd::<[f32; 4]>(bound, 0).map(f64::from);
    let clicks = CLICKS.get(t);
    let cameras: Vec<(bool, &[[f64; 4]])> = clicks
        .cameras
        .iter()
        .map(|clicked| (clicked.camera == clicks.world_camera, clicked.planes.as_slice()))
        .collect();
    setter_among(values, &cameras)
}

/// The name of a node for the log, or "-".
///
/// # Safety
///
/// `node` points at a live `NiObjectNET` whose name is 0 or a C string.
unsafe fn node_name(node: usize) -> String {
    let at: usize = rd(node, crate::gen::layouts::NiObjectNET::name);
    if at == 0 {
        return String::from("-");
    }
    core::ffi::CStr::from_ptr(at as *const core::ffi::c_char)
        .to_string_lossy()
        .into_owned()
}

/// Whether the node animates this frame by the policy: its world bound within the planes of
/// the world camera, or within the reach of the shadow map (policy `shadows`), or always
/// (policy `all`). Not when the engine's walk would not reach the node (`withheld`). A chain
/// that the walk cannot read to its end counts as reached: the node animates, as it did
/// before this test. A node without a bound animates: none of the three readers can meet one
/// (see `reached`).
pub unsafe fn displayed(t: &MainThread, node: usize) -> bool {
    let policy = policy();
    if policy == Policy::All {
        BY_ALL.fetch_add(1, Ordering::Relaxed);
        return true;
    }
    let Some(answer) = reached(t, node, policy) else {
        NEITHER.fetch_add(1, Ordering::Relaxed);
        return false;
    };
    let began = std::time::Instant::now();
    let reason = withheld(&CAMERA.get(t), node);
    CHAIN_WALKS.fetch_add(1, Ordering::Relaxed);
    CHAIN_NANOSECONDS.fetch_add(began.elapsed().as_nanos() as u64, Ordering::Relaxed);
    match reason {
        Some(Withheld::Itself) => WITHHELD_ITSELF.fetch_add(1, Ordering::Relaxed),
        Some(Withheld::Above) => WITHHELD_ABOVE.fetch_add(1, Ordering::Relaxed),
        Some(Withheld::NotSelected) => WITHHELD_NOT_SELECTED.fetch_add(1, Ordering::Relaxed),
        Some(Withheld::NoGeometry) => WITHHELD_NO_GEOMETRY.fetch_add(1, Ordering::Relaxed),
        None => {
            answer.fetch_add(1, Ordering::Relaxed);
            return true;
        }
    };
    false
}

/// [`displayed`] for the node that `reader` reads, compared with the engine's walk: `walk` is the
/// reader's flag before the port writes it. Each reader clears its flag after its read
/// (0x6EE756, 0x6EE768, 0x6F21AB, 0x6F210F, 0x6F213F), so the flag is then only what the displays
/// of the engine's walk set since that read. A miss, the walk's yes and the policy's no, is a node
/// that the walk animates today and that would stop without the walk (W97): the test that must
/// show 0 before the walk goes.
///
/// # Safety
///
/// As [`displayed`].
pub unsafe fn displayed_by_walk(t: &MainThread, node: usize, reader: Reader, walk: bool) -> bool {
    let answer = displayed(t, node);
    let r = reader as usize;
    match (walk, answer) {
        (true, false) => {
            WALK_DISPLAYED[r].fetch_add(1, Ordering::Relaxed);
            MISSES[r].fetch_add(1, Ordering::Relaxed);
            let setter = setter_of_miss(t, node);
            MISSES_BY_SETTER[setter as usize].fetch_add(1, Ordering::Relaxed);
            if MISSES_LOGGED.fetch_add(1, Ordering::Relaxed) < MISSES_NAMED {
                crate::log!(
                    "animation: a miss of the policy {} at {}: the engine's walk displayed '{}', the policy did not select it; the likely setter: {}",
                    policy_name(),
                    READER_NAMES[r],
                    node_name(node),
                    SETTER_NAMES[setter as usize]
                );
            }
        }
        (true, true) => {
            WALK_DISPLAYED[r].fetch_add(1, Ordering::Relaxed);
        }
        (false, true) => {
            POLICY_ONLY[r].fetch_add(1, Ordering::Relaxed);
        }
        (false, false) => {}
    }
    answer
}

/// The count to raise when the policy selects the node by its bound: in the view, or in the
/// reach of the shadow map. [`None`] when neither holds.
///
/// A node without a bound answers `NO_BOUND`. The three readers' nodes are all
/// `BSAnimationNode`, whose `GetWorldBound` is `NiAVObject::GetWorldBound` 0x4430B0 (vtable
/// 0x750D68, slot 13: `lea eax, [ecx+worldBound]`), so it never gives 0; the engine's own
/// reader, `BSAnimationManager::Update` 0x6EE690, dereferences the result with no test
/// (0x6EE6F2). The count in the report shows that.
unsafe fn reached(t: &MainThread, node: usize, policy: Policy) -> Option<&'static AtomicU64> {
    let bound = vcall0(node, SLOT_GET_WORLD_BOUND) as usize;
    if bound == 0 {
        return Some(&NO_BOUND);
    }
    let camera = &*CAMERA.get(t);
    if within(&camera.planes, rd::<[f32; 4]>(bound, 0).map(f64::from)) {
        return Some(&BY_VIEW);
    }
    if policy == Policy::ViewShadows {
        let reach = crate::capture::helper_shadow_reach();
        if reach > 0.0 {
            let centre: [f32; 3] = rd(bound, 0);
            let radius: f32 = rd(bound, BOUND_RADIUS);
            let d: [f32; 3] = core::array::from_fn(|i| centre[i] - camera.position[i]);
            if (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() - radius <= reach {
                return Some(&BY_SHADOWS);
            }
        }
    }
    None
}

/// What keeps the engine's walk from a node.
#[derive(Debug, PartialEq, Eq)]
enum Withheld {
    /// The node is app-culled.
    Itself,
    /// A node above it is app-culled.
    Above,
    /// A switch or a level of detail above it does not display the child that leads to it.
    NotSelected,
    /// A plain node above it has no geometry under it.
    NoGeometry,
}

/// Bit 0 of the flags of a `NiAVObject`, which `NiAVObject::GetAppCulled` 0x442B30 reads.
const APP_CULLED: u16 = 0x01;
/// The flag of a node with geometry under it (`NiNode::IsVisualObject` 0x445A60).
const CONTAINS_GEOMETRY: u16 = 0x08;
/// The run-time types of `NiSwitchNode` and `NiLODNode`, which comes from it.
const RTTI_SWITCH_NODE: usize = 0x7DE1AC;
const RTTI_LOD_NODE: usize = 0x7DE270;

/// What keeps the engine's walk from the node, by the rules of `NiAVObject::CullShow` 0x6EB480,
/// `NiNode::Display` 0x6C9190 and `NiSwitchNode::Display` 0x6D8320 (`effect_chain` has the same
/// rules for a billboard). The planes are not walked: the walk reaches a node only through
/// ancestors in the view, and the node's own bound passes the same planes, so an ancestor's planes
/// only make the policy select more than the walk; so does a parent whose bound does not hold its
/// child's. [`None`] when nothing does, and when the chain is too long to read.
unsafe fn withheld(camera: &Camera, node: usize) -> Option<Withheld> {
    withheld_by(
        node,
        |parent| crate::ports::ni::is_kind_of(parent, RTTI_SWITCH_NODE),
        |switch| selected_child(camera, switch),
    )
}

/// [`withheld`] with the test of a switch and the child that a switch displays as parameters,
/// for a test on nodes that have no run-time type.
///
/// # Safety
///
/// `node` and the nodes above it are live `NiAVObject`s; `is_switch` and `selected` can read
/// each parent.
unsafe fn withheld_by(
    node: usize,
    is_switch: impl Fn(usize) -> bool,
    selected: impl Fn(usize) -> usize,
) -> Option<Withheld> {
    if rd::<u16>(node, AV::flags) & APP_CULLED != 0 {
        return Some(Withheld::Itself);
    }
    let mut child = node;
    let found = crate::parent_chain::first_answer(
        rd(node, AV::parentNode),
        crate::parent_chain::GUARD,
        |n| rd(n, AV::parentNode),
        |parent| {
            let reason = barred_by(rd(parent, AV::flags), is_switch(parent).then(|| selected(parent)), child);
            child = parent;
            reason
        },
    );
    match found {
        Ok(reason) => reason,
        Err(crate::parent_chain::ChainTooLong) => {
            crate::parent_chain::note_too_long("the animation flags");
            None
        }
    }
}

/// What keeps the walk at a parent with these flags from going on to this child. `selected` is
/// the child that the parent displays when it is a switch, and [`None`] for a plain node.
fn barred_by(flags: u16, selected: Option<usize>, child: usize) -> Option<Withheld> {
    if flags & APP_CULLED != 0 {
        return Some(Withheld::Above);
    }
    if let Some(selected) = selected {
        return (selected != child).then_some(Withheld::NotSelected);
    }
    (flags & CONTAINS_GEOMETRY == 0).then_some(Withheld::NoGeometry)
}

/// The child that a switch displays, or 0. A level of detail selects it at its display
/// (`NiLODNode::Display` 0x7412B0) from the camera: here from the same values, as the index
/// that it keeps is old when its display did not run. Another switch keeps its index; that of
/// `NiFltAnimationNode`, which `NiFltAnimationNode::Display` 0x6D90B0 sets from the time, is old
/// in the same way, and it is read as it is.
unsafe fn selected_child(camera: &Camera, switch: usize) -> usize {
    if !crate::ports::ni::is_kind_of(switch, RTTI_LOD_NODE) {
        return indexed_child(switch);
    }
    let count = rd::<u32>(switch, NODE::kChildren_endIndex) as usize;
    let levels: usize = rd(switch, LOD::lodData);
    if levels == 0 {
        return 0;
    }
    let centre: [f32; 3] = rd(switch, LOD::worldCentre);
    let d: [f32; 3] = core::array::from_fn(|i| centre[i] - camera.position[i]);
    let distance_squared = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) * (camera.lod_adjust * camera.lod_adjust);
    let index = lod_index(count, distance_squared, |level| {
        let at = levels + level * LEVEL::SIZE;
        (rd(at, LEVEL::worldNearSquared), rd(at, LEVEL::worldFarSquared))
    });
    child_at(switch, count, index)
}

/// The child at the switch's own index, read at each call. `NiSwitchNode::Display` 0x6D8320 reads
/// it at each display, and a mod's Lua can write it between two (MWSE's `setSwitchIndex`; the
/// engine's other writers are the NIF's load, a clone, and the displays of a level of detail and
/// of an animation node: W114).
///
/// # Safety
///
/// `switch` points at a live `NiSwitchNode` with its children.
unsafe fn indexed_child(switch: usize) -> usize {
    let count = rd::<u32>(switch, NODE::kChildren_endIndex) as usize;
    child_at(switch, count, rd(switch, SWITCH::switchIndex))
}

/// The child at `index` of `switch`, which has `count` slots; 0 out of range.
///
/// # Safety
///
/// `switch` points at a live `NiNode` whose child array holds `count` slots.
unsafe fn child_at(switch: usize, count: usize, index: i32) -> usize {
    if index < 0 || index as usize >= count {
        return 0;
    }
    rd(rd::<usize>(switch, NODE::kChildren_storage), index as usize * 4)
}

/// The rule of `NiLODNode::Display` 0x7412B0: the last level whose range holds the distance, or
/// -1. A range is the squares of its near and of its far, in the world, and holds its near.
fn lod_index(levels: usize, distance_squared: f32, range: impl Fn(usize) -> (f32, f32)) -> i32 {
    (0..levels)
        .rev()
        .find(|&level| {
            let (near, far) = range(level);
            near <= distance_squared && distance_squared < far
        })
        .map_or(-1, |level| level as i32)
}

/// One line for the log: the policy and the counts of the answers. The `withheld` counts are
/// the nodes that the bound or the shadow reach selected and the engine's walk would not reach.
pub fn report() -> String {
    let by_reader = |counts: &[AtomicU64; READERS]| {
        counts
            .iter()
            .map(|count| count.load(Ordering::Relaxed).to_string())
            .collect::<Vec<_>>()
            .join("/")
    };
    format!(
        "animation: policy {}; answers: in view {}, by the shadows {}, all {}, no bound {}, not animated {}, withheld for an app-culled node {}, withheld below an app-culled node {}, withheld as not selected by a switch {}, withheld under no geometry {}; the chains: {} in {:.1} ms; against the engine's walk (the manager/UpdateControllers/UpdateDownwardPass): displayed by the walk {}, misses {} (likely set by another camera {}, by the world camera {}, by no camera's planes {}), selected by the policy only {}",
        policy_name(),
        BY_VIEW.load(Ordering::Relaxed),
        BY_SHADOWS.load(Ordering::Relaxed),
        BY_ALL.load(Ordering::Relaxed),
        NO_BOUND.load(Ordering::Relaxed),
        NEITHER.load(Ordering::Relaxed),
        WITHHELD_ITSELF.load(Ordering::Relaxed),
        WITHHELD_ABOVE.load(Ordering::Relaxed),
        WITHHELD_NOT_SELECTED.load(Ordering::Relaxed),
        WITHHELD_NO_GEOMETRY.load(Ordering::Relaxed),
        CHAIN_WALKS.load(Ordering::Relaxed),
        CHAIN_NANOSECONDS.load(Ordering::Relaxed) as f64 / 1e6,
        by_reader(&WALK_DISPLAYED),
        by_reader(&MISSES),
        MISSES_BY_SETTER[Setter::OtherCamera as usize].load(Ordering::Relaxed),
        MISSES_BY_SETTER[Setter::WorldCamera as usize].load(Ordering::Relaxed),
        MISSES_BY_SETTER[Setter::Unknown as usize].load(Ordering::Relaxed),
        by_reader(&POLICY_ONLY)
    )
}

#[cfg(test)]
mod tests {
    use super::{
        barred_by, child_at, indexed_child, lod_index, setter_among, withheld_by, within, Setter, Withheld, APP_CULLED,
        CONTAINS_GEOMETRY,
    };
    use crate::gen::layouts::{NiAVObject as AV, NiNode as NODE, NiSwitchNode as SWITCH};
    use crate::mem::rd;

    const SQUARE: fn(f32) -> f32 = |value| value * value;

    /// The stops of the test level: one level, 0 to 1000.
    #[test]
    fn one_level_holds_its_near_and_not_its_far() {
        let range = |_| (0.0, SQUARE(1000.0));
        assert_eq!(lod_index(1, 0.0, range), 0);
        assert_eq!(lod_index(1, SQUARE(999.0), range), 0);
        assert_eq!(lod_index(1, SQUARE(1000.0), range), -1);
        assert_eq!(lod_index(1, SQUARE(4871.0), range), -1);
    }

    #[test]
    fn the_last_level_that_holds_the_distance_wins() {
        let ranges = [(0.0, SQUARE(2000.0)), (SQUARE(1000.0), SQUARE(3000.0))];
        let range = |level: usize| ranges[level];
        assert_eq!(lod_index(2, SQUARE(500.0), range), 0);
        assert_eq!(lod_index(2, SQUARE(1500.0), range), 1);
        assert_eq!(lod_index(2, SQUARE(2500.0), range), 1);
        assert_eq!(lod_index(2, SQUARE(3500.0), range), -1);
    }

    #[test]
    fn no_level_selects_nothing() {
        assert_eq!(lod_index(0, 0.0, |_| (0.0, 1.0)), -1);
    }

    /// The fake children of the fake switch: addresses that are compared, never read.
    const CHILDREN: [u32; 3] = [0x1000, 0x2000, 0x3000];

    /// The writes of the index between two calls, as a mod's `setSwitchIndex` makes them, and the child that each
    /// must give: index 1 gives child 1, a write of 2 gives child 2, -1 gives none, the count of children gives none.
    const SEQUENCE: [(i32, usize); 4] = [(1, 0x2000), (2, 0x3000), (-1, 0), (3, 0)];

    fn put(block: &mut [u8], offset: usize, value: u32) {
        block[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Runs `SEQUENCE` on a fake plain switch through `select`, and tells whether each call gave its child.
    fn follows_each_write(mut select: impl FnMut(usize) -> usize) -> bool {
        let mut children = Vec::new();
        for child in CHILDREN {
            children.extend_from_slice(&child.to_le_bytes());
        }
        let mut switch = vec![0u8; SWITCH::SIZE];
        put(&mut switch, NODE::kChildren_storage, children.as_ptr() as u32);
        put(&mut switch, NODE::kChildren_endIndex, CHILDREN.len() as u32);
        SEQUENCE.iter().all(|&(index, child)| {
            put(&mut switch, SWITCH::switchIndex, index as u32);
            select(switch.as_ptr() as usize) == child
        })
    }

    /// A plain switch's child follows each write of its index (W114).
    #[test]
    fn a_plain_switch_follows_each_write_of_its_index() {
        assert!(follows_each_write(|switch| unsafe { indexed_child(switch) }));
    }

    /// Two planes that hold the space x > 0 and y > 0 (normal, constant: n.p - c > -r is inside).
    const QUADRANT: [[f64; 4]; 2] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]];

    /// The planes rule of `reached` (W97): inside, across a plane by less than the radius, and
    /// outside by the radius or more; no planes hold nothing.
    #[test]
    fn a_bound_is_within_the_planes_until_its_radius_is_past_one() {
        assert!(within(&QUADRANT, [5.0, 5.0, 0.0, 1.0]));
        assert!(
            within(&QUADRANT, [-0.5, 5.0, 0.0, 1.0]),
            "a bound across a plane is in the view"
        );
        assert!(
            !within(&QUADRANT, [-1.0, 5.0, 0.0, 1.0]),
            "a bound that touches from outside is out"
        );
        assert!(!within(&QUADRANT, [5.0, -3.0, 0.0, 1.0]));
        assert!(!within(&[], [0.0, 0.0, 0.0, 1.0]));
    }

    /// The setter of a miss (W97): a set by a camera that is not the world camera, as the arm
    /// camera of the first person, is told apart from one by the world camera.
    #[test]
    fn a_miss_set_by_another_camera_is_told_apart() {
        let behind: [[f64; 4]; 1] = [[-1.0, 0.0, 0.0, 0.0]];
        let node_behind = [-5.0, 5.0, 0.0, 1.0];
        let world_only = [(true, &QUADRANT[..])];
        assert_eq!(setter_among(node_behind, &world_only), Setter::Unknown);
        let with_arm = [(true, &QUADRANT[..]), (false, &behind[..])];
        assert_eq!(setter_among(node_behind, &with_arm), Setter::OtherCamera);
        let node_ahead = [5.0, 5.0, 0.0, 1.0];
        assert_eq!(setter_among(node_ahead, &with_arm), Setter::WorldCamera);
        let both = [(true, &QUADRANT[..]), (false, &QUADRANT[..])];
        assert_eq!(
            setter_among(node_ahead, &both),
            Setter::OtherCamera,
            "another camera explains the set first"
        );
    }

    /// `barred_by` (W97): the rules of `CullShow`, `NiNode::Display` and `NiSwitchNode::Display`.
    #[test]
    fn a_parent_bars_its_child_by_the_rules_of_the_walk() {
        let child = 0x1000;
        assert_eq!(barred_by(APP_CULLED | CONTAINS_GEOMETRY, None, child), Some(Withheld::Above));
        assert_eq!(barred_by(0, None, child), Some(Withheld::NoGeometry));
        assert_eq!(barred_by(CONTAINS_GEOMETRY, None, child), None);
        assert_eq!(barred_by(0, Some(0x2000), child), Some(Withheld::NotSelected));
        assert_eq!(
            barred_by(0, Some(child), child),
            None,
            "a switch does not test the geometry flag"
        );
        assert_eq!(barred_by(APP_CULLED, Some(child), child), Some(Withheld::Above));
    }

    /// Nodes of the size of a `NiAVObject`, each with its flags and its parent (an index into
    /// the list, or none for the root).
    fn nodes(parts: &[(u16, Option<usize>)]) -> Vec<Vec<u8>> {
        let mut nodes: Vec<Vec<u8>> = parts.iter().map(|_| vec![0u8; AV::SIZE]).collect();
        let addresses: Vec<usize> = nodes.iter().map(|node| node.as_ptr() as usize).collect();
        for (node, &(flags, parent)) in nodes.iter_mut().zip(parts) {
            let parent = parent.map_or(0, |at| addresses[at]);
            node[AV::parentNode..AV::parentNode + 4].copy_from_slice(&(parent as u32).to_le_bytes());
            node[AV::flags..AV::flags + 2].copy_from_slice(&flags.to_le_bytes());
        }
        nodes
    }

    /// `withheld` up a chain of fake nodes (W97): the node itself, a parent with no geometry, a
    /// switch that displays another child, and a chain that the walk reaches.
    #[test]
    fn the_chain_withholds_a_node_that_the_walk_does_not_reach() {
        let none = |_: usize| false;
        let unread = |_: usize| -> usize { unreachable!("no switch") };
        let check = |parts: &[(u16, Option<usize>)]| {
            let nodes = nodes(parts);
            // SAFETY: each node is a block of the size of a NiAVObject whose parent is in the list.
            unsafe { withheld_by(nodes[0].as_ptr() as usize, none, unread) }
        };
        let g = CONTAINS_GEOMETRY;
        assert_eq!(check(&[(g, Some(1)), (g, Some(2)), (g, None)]), None);
        assert_eq!(check(&[(APP_CULLED, Some(1)), (g, None)]), Some(Withheld::Itself));
        assert_eq!(
            check(&[(g, Some(1)), (g | APP_CULLED, Some(2)), (g, None)]),
            Some(Withheld::Above)
        );
        assert_eq!(check(&[(g, Some(1)), (0, Some(2)), (g, None)]), Some(Withheld::NoGeometry));

        let chain = nodes(&[(g, Some(1)), (0, Some(2)), (g, None)]);
        let [node, switch] = [chain[0].as_ptr() as usize, chain[1].as_ptr() as usize];
        // SAFETY: as above; the switch is told by its address.
        unsafe {
            assert_eq!(withheld_by(node, |p| p == switch, |_| 0x7777), Some(Withheld::NotSelected));
            assert_eq!(withheld_by(node, |p| p == switch, |_| node), None);
        }
    }

    /// The null of the test above: a plant that keeps a copy of the first index it reads fails the sequence.
    #[test]
    fn a_kept_copy_of_the_index_fails_the_sequence() {
        let mut kept: Option<i32> = None;
        let plant = |switch: usize| unsafe {
            let index = *kept.get_or_insert_with(|| rd(switch, SWITCH::switchIndex));
            child_at(switch, CHILDREN.len(), index)
        };
        assert!(!follows_each_write(plant));
    }
}
