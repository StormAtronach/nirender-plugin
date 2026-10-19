//! The live walk (plan F8, part 2). In mode gpu the helper draws the table, and the engine's
//! world walk still serves a few hundred shapes: the live instances (a controller on the
//! shape or on a property: `OnPreDisplay`, the notes, the record), the skinned actors
//! (their palette in their draw each frame) and their blended parts, the particle
//! systems, the lines, and every rejected shape. This module keeps the list of them (each
//! display port notes the shape it serves, the destructor of a geometry forgets it) and
//! walks the list itself at the display of the world root, after the sky root (so that the
//! sky's draws come first, as in the engine's order): the ancestors (app culled, the child
//! a switch or a level-of-detail node selects), the world bound against every culling
//! plane of the camera (engine-rules R16), then the shape's own `Display` through its
//! vtable (the port: the notes of a live instance, or the states, the accumulator and the
//! draw). The engine's walk, which still runs in this part, skips a shape the walk served
//! in the same run. Part 4 cuts the engine's walk; the list walk is then the only walk of
//! the world, and a shape made at run time registers through the update pass.
//!
//! On by default from the cut (F8 part 4, entry 13) to 2026-10-19, with the table walk and
//! the sky-only walk: the engine's walk of the world stops after the sky root. Off by default
//! since 2026-10-19 (the owner's decision B, records entry 37: the engine traverses the
//! world, the helper draws from its table); `NIRENDER_ENGINE_WALK=0` or
//! `nirender.engineWalk(false)` gives the cut back. The switches of the single walks retired
//! at the owner's review of CP4 (2026-10-18).

use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use crate::ptr_hash::{HashMap, HashSet};

use crate::engine::{thiscall1, vcall0, vcall1, ni_pointer_set};
use crate::gen::layouts::{NiAVObject as AV, NiCamera as C, NiNode as N};
use crate::mem::rd;
use crate::ports::scene::{
    CAMERA_LOD_ADJUST, CAMERA_TRANSLATION, LOD_DATA, LOD_DATA_SIZE, LOD_WORLD_CENTRE, LOD_WORLD_FAR_SQUARED, LOD_WORLD_NEAR_SQUARED,
    ROTATE_TO_CAMERA, SLOT_DISPLAY, SLOT_FINISH_ACCUMULATING, SLOT_GET_APP_CULLED, SLOT_START_ACCUMULATING, SORTING_OFF, SORTING_SUBSORT,
    SORT_ADJUST_ACCUMULATOR, SORT_ADJUST_MODE, SWITCH_CHILD_FROM_TIMING, SWITCH_INDEX, SWITCH_SAVED_TIME, WHICH_SIDE,
};
use crate::ports::scene::switch_prepare_child;

/// MWSE's sort adjust mode (`SORTING_ORDERED_SUBTREE_MWSE`): with an alpha accumulator its
/// `Display` override registers the whole node as one object (R26).
const SORTING_ORDERED_SUBTREE_MWSE: u32 = 64;
use crate::gen::layouts::NiRenderer as R;
use crate::mem::wr;

/// `NiRTTI`: the name, then the base.
const RTTI_NAME: usize = 0x0;
const RTTI_BASE: usize = 0x4;
/// The most nodes above a shape that the walk looks at.
const MAX_DEPTH: usize = 64;
/// The side `WhichSide` returns for a bound wholly on the negative side of a plane.
const OUTSIDE: u32 = 2;

/// The measure per class (plan section 2, the scope of the world's takeover): the displays of
/// the live walk by what they display (`DC_*`), each [count, ticks], and the time of the
/// sky's display under the world root.
static mut DISPLAY_CLASSES: [[i64; 2]; 4] = [[0; 2]; 4];
const DC_SKINNED: usize = 0;
const DC_PARTICLES: usize = 1;
const DC_LINES: usize = 2;
const DC_OTHER: usize = 3;
static mut SKY_TICKS: [i64; 2] = [0; 2];

/// The sky root's display took these ticks (`ports::scene::node_display`).
pub unsafe fn note_sky_ticks(ticks: i64) {
    let sky = &mut *addr_of_mut!(SKY_TICKS);
    sky[0] += 1;
    sky[1] += ticks;
}

pub unsafe fn class_report() -> String {
    let ms = |t: i64| t as f64 * 1000.0 / crate::capture::ticks_per_second() as f64;
    let c = *addr_of!(DISPLAY_CLASSES);
    let sky = *addr_of!(SKY_TICKS);
    let names = ["skinned", "particles", "lines", "other"];
    let parts: Vec<String> = (0..4).map(|i| format!("{} {} ({:.3} ms)", names[i], c[i][0], ms(c[i][1]))).collect();
    format!("classes (the takeover's measure): live walk displays {}; the sky's display {} ({:.3} ms)", parts.join(", "), sky[0], ms(sky[1]))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Shape = 0,
    Strips = 1,
    Particles = 2,
    Lines = 3,
}

impl Kind {
    /// The kind of a draw of the link (`link::DRAW_*`), for the note of a live instance.
    pub fn of_draw(kind: u32) -> Kind {
        match kind {
            crate::link::DRAW_STRIPS => Kind::Strips,
            crate::link::DRAW_POINTS => Kind::Particles,
            crate::link::DRAW_LINES => Kind::Lines,
            _ => Kind::Shape,
        }
    }
}

struct Entry {
    kind: Kind,
    /// The run of the walk that displayed the shape: the engine's walk skips it in that run.
    served: u64,
}

static mut LIST: Option<HashMap<usize, Entry>> = None;
/// The shapes of the run in progress (the list is not iterated while a display notes).
static mut SCRATCH: Vec<usize> = Vec::new();
static mut PLANES: Vec<usize> = Vec::new();
/// The index of each node in its parent, found once a run.
static mut NODE_INDEXES: Option<HashMap<usize, u32>> = None;
/// The shapes of the run with their paths from the root, sorted: the engine's order.
static mut ORDERED: Vec<(Vec<u32>, usize)> = Vec::new();
static IN_WALK: AtomicBool = AtomicBool::new(false);
/// The camera of the run in progress.
static CAMERA: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
static RUN: AtomicU64 = AtomicU64::new(0);
/// The scenes of the game, counted at each begin, and the scene of the last walk: a shape
/// is served in the scene of the walk only (the engine's displays before the walk in the
/// next frame, the sky's, were skipped as served from the frame before, 2026-10-17).
static SCENE_SERIAL: AtomicU64 = AtomicU64::new(0);
static RUN_SCENE: AtomicU64 = AtomicU64::new(0);
static DISPLAYED: AtomicU64 = AtomicU64::new(0);
static OUT_OF_VIEW: AtomicU64 = AtomicU64::new(0);
static UNREACHABLE: AtomicU64 = AtomicU64::new(0);
static SKIPPED_AS_SERVED: AtomicU64 = AtomicU64::new(0);
static FORGOTTEN: AtomicU64 = AtomicU64::new(0);

unsafe fn list() -> &'static mut HashMap<usize, Entry> {
    (*addr_of_mut!(LIST)).get_or_insert_with(HashMap::default)
}

/// A scene of the game begins (every kind).
pub fn scene_begin() {
    SCENE_SERIAL.fetch_add(1, Ordering::Relaxed);
}

/// True when the walk displayed the node whole in the world walk in progress: the engine's
/// cull skips it (`ports::scene::cull_show`). Cheap when the walk is off or served nothing.
pub unsafe fn node_served(node: usize) -> bool {
    if WALK_ROOT.load(Ordering::Relaxed) == 0 {
        return false;
    }
    (*addr_of!(NODES_SERVED)).as_ref().is_some_and(|set| set.contains(&node))
}

/// A node's display ended: at the end of the world root's display the served nodes go
/// (the flush of the accumulator after it displays them).
pub unsafe fn after_node(node: usize) {
    if node != 0 && WALK_ROOT.load(Ordering::Relaxed) == node {
        WALK_ROOT.store(0, Ordering::Relaxed);
        if let Some(set) = (*addr_of_mut!(NODES_SERVED)).as_mut() {
            set.clear();
        }
    }
}

/// True during the walk's own displays.
pub fn in_walk() -> bool {
    IN_WALK.load(Ordering::Relaxed)
}

/// The live walk: on, but with the engine's walk.
pub fn on() -> bool {
    !engine_walk_on()
}

/// The engine's walk of the world (entry 13; the default since 2026-10-19, entry 37): on, the
/// table walk, this walk and the sky-only walk are off, and the engine displays the world as
/// before the cut. `NIRENDER_ENGINE_WALK=0` or `nirender.engineWalk(false)` gives the cut.
/// Mode `off` stays the picture reference.
static ENGINE_WALK: AtomicU8 = AtomicU8::new(255);

pub fn engine_walk_on() -> bool {
    let value = ENGINE_WALK.load(Ordering::Relaxed);
    if value != 255 {
        return value != 0;
    }
    let on = std::env::var("NIRENDER_ENGINE_WALK").map_or(true, |v| v != "0");
    ENGINE_WALK.store(on as u8, Ordering::Relaxed);
    on
}

/// Sets the engine's walk; returns the state before. A change gives the table a full pass
/// and the sky-only walk a discovery frame.
pub fn set_engine_walk(on: bool) -> bool {
    let before = engine_walk_on();
    ENGINE_WALK.store(on as u8, Ordering::Relaxed);
    if on != before {
        DISCOVERED.store(false, Ordering::Relaxed);
        crate::instance_stream::table_full();
    }
    before
}

/// A display port serves the shape (a draw, or the notes of a live instance): the shape
/// joins the list. Not during the walk's own displays: the shape is on the list already.
pub unsafe fn note(shape: usize, kind: Kind) {
    if IN_WALK.load(Ordering::Relaxed) {
        return;
    }
    if !list().contains_key(&shape) {
        list().insert(shape, Entry { kind, served: 0 });
        ORDER_DIRTY.store(true, Ordering::Relaxed);
    }
}

/// The destructor of a geometry: the shape leaves the list and the candidates. True when it
/// was in the list.
pub unsafe fn forget(shape: usize) -> bool {
    let listed = list().remove(&shape).is_some();
    if listed {
        FORGOTTEN.fetch_add(1, Ordering::Relaxed);
        ORDER_DIRTY.store(true, Ordering::Relaxed);
    }
    if let Some(candidates) = (*addr_of_mut!(CANDIDATES)).as_mut() {
        candidates.remove(&shape);
    }
    unpend(shape);
    if let Some(far) = (*addr_of_mut!(FAR)).as_mut() {
        far.remove(&shape);
    }
    // A shape waiting for its forced display (plan C26: it was kept after the destructor).
    if let Some(force) = (*addr_of_mut!(FORCE)).as_mut() {
        force.remove(&shape);
    }
    listed
}

/// True when the walk displayed the shape in the run in progress: the engine's walk, which
/// reaches it after, must not display it again. Never during the walk's own displays, and
/// never after the world root's display: the alpha accumulator's flush displays the shapes
/// it took again, the walk's shapes too.
pub unsafe fn served(shape: usize) -> bool {
    if !on() || IN_WALK.load(Ordering::Relaxed) || WALK_ROOT.load(Ordering::Relaxed) == 0 {
        return false;
    }
    if RUN_SCENE.load(Ordering::Relaxed) != SCENE_SERIAL.load(Ordering::Relaxed) {
        return false;
    }
    let run = RUN.load(Ordering::Relaxed);
    let served = list().get(&shape).is_some_and(|entry| entry.served == run);
    if served {
        SKIPPED_AS_SERVED.fetch_add(1, Ordering::Relaxed);
    }
    served
}

/// `NiNode::Display` displayed a child: after the sky root, under the world root, the walk
/// runs (mode gpu, a stream going). Returns true when the world root's display stops here
/// (the sky-only walk, part 3): the table walk and the live walk served the world.
pub unsafe fn after_child(parent: usize, child: usize, camera: usize) -> bool {
    if child == 0 || child != crate::link_client::sky_root() {
        return false;
    }
    crate::instance_stream::table_walk(parent, camera);
    if !on() {
        return false;
    }
    if crate::instance_stream::mode() != crate::instance_stream::Mode::Gpu || !crate::capture::recording() {
        return false;
    }
    walk(parent, camera);
    if !sky_only_on() || !crate::instance_stream::table_walk_on() {
        return false;
    }
    // The discovery frame: the engine's walk displays the world once, so that every shape in
    // view is classified (the shapes loaded before the switch, or after a change of the mode).
    if !DISCOVERED.swap(true, Ordering::Relaxed) {
        DISCOVERY_FRAMES.fetch_add(1, Ordering::Relaxed);
        crate::log!("sky-only walk: a discovery frame (the engine walks the world once)");
        return false;
    }
    if !LOGGED_ROOT.swap(true, Ordering::Relaxed) {
        log_root(parent, child);
    }
    CUT_RUNS.fetch_add(1, Ordering::Relaxed);
    true
}

/// The sky-only walk (F8 part 3, entry 13) cuts the world root's display after the sky root
/// when the table walk and the live walk are on in mode gpu. On since the cut (part 4); off
/// with the engine's walk.
/// False until the first frame of the switch (or of a mode) ran uncut.
static DISCOVERED: AtomicBool = AtomicBool::new(false);
static LOGGED_ROOT: AtomicBool = AtomicBool::new(false);
static DISCOVERY_FRAMES: AtomicU64 = AtomicU64::new(0);
static CUT_RUNS: AtomicU64 = AtomicU64::new(0);
static CANDIDATES_ADDED: AtomicU64 = AtomicU64::new(0);
static CANDIDATES_DROPPED: AtomicU64 = AtomicU64::new(0);
static PRUNED: AtomicU64 = AtomicU64::new(0);
/// The shapes the update pass updated that neither the table nor the list knows: the live
/// walk displays them once (their display port classifies them). The destructor removes one.
static mut CANDIDATES: Option<HashSet<usize>> = None;
/// Why a candidate waits: not under the world root (the UI, a node not attached), an
/// app-culled node on its path (the collision roots, a disabled reference), or a switch,
/// level or flt node that selects another child.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wait {
    Outside,
    Culled,
    Switch,
}

/// The candidates whose path is closed: the engine's walk would not display them. Each is
/// filed under the nodes of its path (`PENDING_BELOW`); an app-cull or tree mark of one of
/// them checks its shapes again (`release_pending`). The switch-closed ones are checked each
/// frame (a level follows the camera; a script can write a switch's index).
static mut PENDING: Option<HashMap<usize, (Vec<usize>, Wait)>> = None;
static mut PENDING_BELOW: Option<HashMap<usize, HashSet<usize>>> = None;
static mut PENDING_SWITCH: Option<HashSet<usize>> = None;
/// The candidates with an open path beyond the camera's far distance (the engine's walk does
/// not display them either): a quarter is checked each frame.
static mut FAR: Option<HashSet<usize>> = None;
static mut FAR_ROUND: Vec<usize> = Vec::new();
/// The candidates of the run with an open path within the far distance: displayed whatever
/// the view (their display classifies them).
static mut FORCE: Option<HashSet<usize>> = None;
static PENDING_ADDED: AtomicU64 = AtomicU64::new(0);
static PENDING_RELEASED: AtomicU64 = AtomicU64::new(0);
static FORCED: AtomicU64 = AtomicU64::new(0);
static FAR_ADDED: AtomicU64 = AtomicU64::new(0);
static WALK_TICKS: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(0);
/// The parts of the walk's time: the candidates, the order, the reach and the displays.
static PART_TICKS: [core::sync::atomic::AtomicI64; 3] = [
    core::sync::atomic::AtomicI64::new(0),
    core::sync::atomic::AtomicI64::new(0),
    core::sync::atomic::AtomicI64::new(0),
];
/// The order of the list (`ORDERED`) is kept between frames; it is built again when the list
/// changes, the root changes, or a tree mark hits a node above a listed shape (`ABOVE`).
static ORDER_DIRTY: AtomicBool = AtomicBool::new(true);
static mut ORDER_ROOT: usize = 0;
static mut ABOVE: Option<HashSet<usize>> = None;
static ORDER_BUILDS: AtomicU64 = AtomicU64::new(0);
/// The nodes whose pending release opened nothing, with the run: skipped for 16 runs.
static mut RELEASE_IDLE: Option<HashMap<usize, u64>> = None;
const RELEASE_IDLE_RUNS: u64 = 16;

/// The tree marks of the frame (`instance_stream::table_walk`): a node above a listed shape
/// makes the order of the list stale.
pub unsafe fn note_tree_marks(nodes: &[usize]) {
    if ORDER_DIRTY.load(Ordering::Relaxed) {
        return;
    }
    if let Some(above) = (*addr_of!(ABOVE)).as_ref() {
        if nodes.iter().any(|node| above.contains(node)) {
            ORDER_DIRTY.store(true, Ordering::Relaxed);
        }
    }
}

unsafe fn pending() -> &'static mut HashMap<usize, (Vec<usize>, Wait)> {
    (*addr_of_mut!(PENDING)).get_or_insert_with(HashMap::default)
}

unsafe fn pending_below() -> &'static mut HashMap<usize, HashSet<usize>> {
    (*addr_of_mut!(PENDING_BELOW)).get_or_insert_with(HashMap::default)
}

unsafe fn pending_switch() -> &'static mut HashSet<usize> {
    (*addr_of_mut!(PENDING_SWITCH)).get_or_insert_with(HashSet::default)
}

unsafe fn far() -> &'static mut HashSet<usize> {
    (*addr_of_mut!(FAR)).get_or_insert_with(HashSet::default)
}

unsafe fn force() -> &'static mut HashSet<usize> {
    (*addr_of_mut!(FORCE)).get_or_insert_with(HashSet::default)
}

/// A pending shape leaves the set and the maps of its nodes.
unsafe fn unpend(shape: usize) -> bool {
    let Some((path, wait)) = pending().remove(&shape) else { return false };
    if wait == Wait::Switch {
        pending_switch().remove(&shape);
    }
    let below = pending_below();
    for node in path {
        if let Some(set) = below.get_mut(&node) {
            set.remove(&shape);
            if set.is_empty() {
                below.remove(&node);
            }
        }
    }
    true
}

unsafe fn pend(shape: usize, path: Vec<usize>, wait: Wait) {
    let below = pending_below();
    for &node in path.iter() {
        below.entry(node).or_default().insert(shape);
    }
    if wait == Wait::Switch {
        pending_switch().insert(shape);
    }
    pending().insert(shape, (path, wait));
    PENDING_ADDED.fetch_add(1, Ordering::Relaxed);
}

/// The app-cull or tree marks of the frame (`instance_stream::table_walk`): the pending shapes
/// below each marked node are checked again; one whose path opened is a candidate again. A
/// node is a key only (it can be gone).
pub unsafe fn release_pending(nodes: &[usize], root: usize) {
    if !sky_only_on() || (*addr_of!(PENDING_BELOW)).as_ref().map_or(true, |m| m.is_empty()) {
        return;
    }
    let run = RUN.load(Ordering::Relaxed);
    let idle = (*addr_of_mut!(RELEASE_IDLE)).get_or_insert_with(HashMap::default);
    for &node in nodes {
        if idle.get(&node).is_some_and(|&at| run < at + RELEASE_IDLE_RUNS) {
            continue;
        }
        let Some(set) = pending_below().get(&node) else { continue };
        let below: Vec<usize> = set.iter().copied().collect();
        let mut opened = false;
        for shape in below {
            if release_one(shape, root) {
                opened = true;
            }
        }
        if opened {
            idle.remove(&node);
        } else {
            idle.insert(node, run);
        }
    }
}

/// A pending shape checked again: true when its path opened (it is a candidate again).
unsafe fn release_one(shape: usize, root: usize) -> bool {
    {
        if !pending().contains_key(&shape) {
            return false;
        }
        let (state, path) = path_state(shape, root);
        let wait = pending().get(&shape).map(|p| p.1);
        if state.is_none() {
            unpend(shape);
            (*addr_of_mut!(CANDIDATES)).get_or_insert_with(HashSet::default).insert(shape);
            PENDING_RELEASED.fetch_add(1, Ordering::Relaxed);
            return true;
        } else if state != wait {
            // Closed for another reason now (a node moved, a cull became a switch).
            unpend(shape);
            pend(shape, path, state.unwrap_or(Wait::Outside));
            return true;
        }
    }
    false
}

/// The state of the path from the shape up to `root`: `None` when open (no node app culled,
/// each switch, level and flt node selects the path's child), else why it is closed, with the
/// nodes from the shape's parent up.
unsafe fn path_state(shape: usize, root: usize) -> (Option<Wait>, Vec<usize>) {
    let mut nodes = Vec::new();
    let mut culled = false;
    let mut switched = false;
    let mut node = shape;
    for _ in 0..MAX_DEPTH {
        if node == root {
            let state = if culled { Some(Wait::Culled) } else if switched { Some(Wait::Switch) } else { None };
            return (state, nodes);
        }
        if node == 0 {
            break;
        }
        if vcall0(node, SLOT_GET_APP_CULLED) & 0xFF != 0 {
            culled = true;
        }
        let parent: usize = rd(node, AV::parentNode);
        if parent == 0 {
            break;
        }
        if let Some(switch) = switch_kind(parent) {
            let index = switch_index(parent, switch, planes_camera());
            let selected: usize = if index >= 0 && index < rd::<i32>(parent, N::kChildren_endIndex) {
                rd(rd::<usize>(parent, N::kChildren_storage), index as usize * 4)
            } else {
                0
            };
            if selected != node {
                switched = true;
            }
        }
        nodes.push(parent);
        node = parent;
    }
    (Some(Wait::Outside), nodes)
}

/// The nodes whose path up to the root is open (true) or closed (false), in the frame.
static mut OPEN_MEMO: Option<HashMap<usize, bool>> = None;

/// True when the path from `node` up to `root` is open: no node app culled, each switch, level
/// and flt node selects the path's child. Each node is answered once a frame (`memo`).
unsafe fn path_open(node: usize, root: usize, memo: &mut HashMap<usize, bool>) -> bool {
    let mut chain: Vec<usize> = Vec::new();
    let mut at = node;
    let mut answer = false;
    for _ in 0..MAX_DEPTH {
        if at == root {
            answer = true;
            break;
        }
        if at == 0 {
            break;
        }
        if let Some(&known) = memo.get(&at) {
            answer = known;
            break;
        }
        if vcall0(at, SLOT_GET_APP_CULLED) & 0xFF != 0 {
            answer = false;
            chain.push(at);
            break;
        }
        let parent: usize = rd(at, AV::parentNode);
        if parent == 0 {
            chain.push(at);
            break;
        }
        if let Some(switch) = switch_kind(parent) {
            let index = switch_index(parent, switch, planes_camera());
            let selected: usize = if index >= 0 && index < rd::<i32>(parent, N::kChildren_endIndex) {
                rd(rd::<usize>(parent, N::kChildren_storage), index as usize * 4)
            } else {
                0
            };
            if selected != at {
                answer = false;
                chain.push(at);
                break;
            }
        }
        chain.push(at);
        at = parent;
    }
    for n in chain {
        memo.insert(n, answer);
    }
    answer
}

/// True when the shape's world bound reaches within the camera's far distance (the test of
/// `BSAnimationManager::Update`).
unsafe fn within_far(shape: usize, camera: usize) -> bool {
    if camera == 0 {
        return true;
    }
    let dx = rd::<f32>(shape, AV::worldBound) - rd::<f32>(camera, CAMERA_TRANSLATION);
    let dy = rd::<f32>(shape, AV::worldBound + 4) - rd::<f32>(camera, CAMERA_TRANSLATION + 4);
    let dz = rd::<f32>(shape, AV::worldBound + 8) - rd::<f32>(camera, CAMERA_TRANSLATION + 8);
    let radius = rd::<f32>(shape, AV::worldBound + 12);
    (dx * dx + dy * dy + dz * dz).sqrt() - radius < rd::<f32>(camera, C::kViewFrustum_far)
}

/// A candidate with an open path: within the far distance it joins the list and is displayed
/// in this run whatever the view; beyond it waits in `FAR`.
unsafe fn admit(shape: usize, camera: usize) {
    if !within_far(shape, camera) {
        if far().insert(shape) {
            FAR_ADDED.fetch_add(1, Ordering::Relaxed);
        }
        return;
    }
    far().remove(&shape);
    force().insert(shape);
    let kind = if is_kind(shape, b"NiTriStrips") {
        Kind::Strips
    } else if is_kind(shape, b"NiParticles") {
        Kind::Particles
    } else if is_kind(shape, b"NiLines") {
        Kind::Lines
    } else {
        Kind::Shape
    };
    list().insert(shape, Entry { kind, served: 0 });
    ORDER_DIRTY.store(true, Ordering::Relaxed);
    CANDIDATES_ADDED.fetch_add(1, Ordering::Relaxed);
}

pub fn sky_only_on() -> bool {
    !engine_walk_on()
}

/// The mode of the table changed: the next frame of the sky-only walk is a discovery frame.
pub fn rediscover() {
    DISCOVERED.store(false, Ordering::Relaxed);
}

/// The update pass updated a shape that the table does not know
/// (`instance_stream::note_world_data`): unknown to the list too, it is a candidate. Only
/// with the sky-only walk on (without it the engine's walk finds the shape).
pub unsafe fn note_update(shape: usize) {
    if !sky_only_on() || list().contains_key(&shape) {
        return;
    }
    (*addr_of_mut!(CANDIDATES)).get_or_insert_with(HashSet::default).insert(shape);
}

/// The candidates of the frame, the switch-pending shapes and a quarter of the far ones: a
/// shape with an open path is admitted (`admit`), one with a closed path waits (`pend`). A
/// candidate pending already is skipped (the update pass touches the UI's shapes each frame).
unsafe fn take_candidates(root: usize, camera: usize) {
    let drained: Vec<usize> = (*addr_of_mut!(CANDIDATES)).as_mut().map_or(Vec::new(), |c| c.drain().collect());
    for shape in drained {
        if crate::instance_stream::is_instance(shape) || list().contains_key(&shape) || pending().contains_key(&shape) {
            continue;
        }
        let (state, path) = path_state(shape, root);
        match state {
            None => admit(shape, camera),
            Some(wait) => {
                if wait == Wait::Outside {
                    CANDIDATES_DROPPED.fetch_add(1, Ordering::Relaxed);
                }
                far().remove(&shape);
                pend(shape, path, wait);
            }
        }
    }
    // The switch-closed: a level follows the camera, a script writes an index. A memo of the
    // nodes (once a frame) answers most of them with a look-up; a shape whose path opened gets
    // the full check.
    if !pending_switch().is_empty() {
        let memo = (*addr_of_mut!(OPEN_MEMO)).get_or_insert_with(HashMap::default);
        memo.clear();
        let shapes: Vec<usize> = pending_switch().iter().copied().collect();
        for shape in shapes {
            if !path_open(shape, root, memo) {
                continue;
            }
            let (state, path) = path_state(shape, root);
            if state.is_none() {
                unpend(shape);
                PENDING_RELEASED.fetch_add(1, Ordering::Relaxed);
                admit(shape, camera);
            } else if state != Some(Wait::Switch) {
                unpend(shape);
                pend(shape, path, state.unwrap_or(Wait::Outside));
            }
        }
    }
    // A quarter of the far ones.
    let round = &mut *addr_of_mut!(FAR_ROUND);
    if round.is_empty() && !far().is_empty() {
        round.extend(far().iter().copied());
    }
    let take = round.len().div_ceil(4).max(if round.is_empty() { 0 } else { 1 });
    for _ in 0..take.min(round.len()) {
        let Some(shape) = round.pop() else { break };
        if far().contains(&shape) && within_far(shape, camera) {
            far().remove(&shape);
            let (state, path) = path_state(shape, root);
            match state {
                None => admit(shape, camera),
                Some(wait) => pend(shape, path, wait),
            }
        }
    }
}

/// The children of the world root, once: the sky root's place among them (the children
/// before it stay the engine's).
unsafe fn log_root(root: usize, sky: usize) {
    let count = rd::<u32>(root, N::kChildren_endIndex) as usize;
    let storage: usize = rd(root, N::kChildren_storage);
    let mut names = Vec::new();
    for i in 0..count.min(32) {
        let child: usize = rd(storage, i * 4);
        let name_at: usize = if child != 0 { rd(child, 8) } else { 0 };
        let name = if name_at != 0 { core::ffi::CStr::from_ptr(name_at as *const core::ffi::c_char).to_string_lossy().into_owned() } else { String::new() };
        names.push(format!("{i}: '{name}'{}", if child == sky { " (the sky root)" } else { "" }));
    }
    crate::log!("sky-only walk: the world root 0x{root:x} has {count} children: {}", names.join(", "));
}

unsafe fn walk(root: usize, camera: usize) {
    let start = crate::capture::ticks();
    walk_timed(root, camera);
    WALK_TICKS.fetch_add(crate::capture::ticks() - start, Ordering::Relaxed);
}

unsafe fn walk_timed(root: usize, camera: usize) {
    let run = RUN.fetch_add(1, Ordering::Relaxed) + 1;
    RUN_SCENE.store(SCENE_SERIAL.load(Ordering::Relaxed), Ordering::Relaxed);
    CAMERA.store(camera, Ordering::Relaxed);
    let planes = &mut *addr_of_mut!(PLANES);
    planes.clear();
    let count = rd::<i32>(camera, C::countCullingPlanes).max(0) as usize;
    let storage: usize = rd(camera, C::cullingPlanes_storage);
    planes.extend((0..count).map(|i| rd::<usize>(storage, i * 4)));
    let shapes = &mut *addr_of_mut!(SCRATCH);
    shapes.clear();
    (*addr_of_mut!(NODE_PLANES)).get_or_insert_with(HashMap::default).clear();
    (*addr_of_mut!(NODES_SERVED)).get_or_insert_with(HashSet::default).clear();
    WALK_ROOT.store(root, Ordering::Relaxed);
    (*addr_of_mut!(TURNED)).get_or_insert_with(HashSet::default).clear();
    force().clear();
    let t0 = crate::capture::ticks();
    take_candidates(root, camera);
    let t1 = crate::capture::ticks();
    PART_TICKS[0].fetch_add(t1 - t0, Ordering::Relaxed);
    let ordered = &mut *addr_of_mut!(ORDERED);
    if ORDER_DIRTY.swap(false, Ordering::Relaxed) || *addr_of!(ORDER_ROOT) != root {
        ORDER_ROOT = root;
        ORDER_BUILDS.fetch_add(1, Ordering::Relaxed);
        (*addr_of_mut!(NODE_INDEXES)).get_or_insert_with(HashMap::default).clear();
        shapes.extend(list().keys().copied());
        ordered.clear();
        let above = (*addr_of_mut!(ABOVE)).get_or_insert_with(HashSet::default);
        above.clear();
        for &shape in shapes.iter() {
            ordered.push((tree_path(shape, root), shape));
            let mut node: usize = rd(shape, AV::parentNode);
            for _ in 0..MAX_DEPTH {
                if node == 0 || node == root || !above.insert(node) {
                    break;
                }
                node = rd(node, AV::parentNode);
            }
        }
        ordered.sort_unstable();
    }
    let t2 = crate::capture::ticks();
    PART_TICKS[1].fetch_add(t2 - t1, Ordering::Relaxed);
    IN_WALK.store(true, Ordering::Relaxed);
    let mut group: Option<Group> = None;
    for &(_, shape) in ordered.iter() {
        if !list().contains_key(&shape) || !crate::pointer_check::shape(shape, "the live walk") {
            continue;
        }
        let adjust = match reachable(shape, root, planes) {
            Reach::Unreachable => {
                UNREACHABLE.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            // A new shape is displayed once whatever the view: its display classifies it.
            Reach::OutOfView if force().contains(&shape) => {
                FORCED.fetch_add(1, Ordering::Relaxed);
                0
            }
            Reach::OutOfView => {
                OUT_OF_VIEW.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            Reach::InView(adjust) => adjust,
        };
        // MWSE's mode 64 with an alpha accumulator: the node is the unit. The walk displays
        // it once (MWSE's override registers it whole) and none of its shapes.
        if adjust != 0 && rd::<u32>(adjust, SORT_ADJUST_MODE) == SORTING_ORDERED_SUBTREE_MWSE {
            let renderer: usize = rd(camera, C::spRenderer);
            let accumulator: usize = if renderer != 0 { rd(renderer, R::accumulator) } else { 0 };
            if accumulator != 0 && is_kind(accumulator, b"NiAlphaAccumulator") {
                if let Some(g) = group.take() {
                    close_group(&g);
                }
                let served = (*addr_of_mut!(NODES_SERVED)).get_or_insert_with(HashSet::default);
                if served.insert(adjust) {
                    vcall1(adjust, SLOT_DISPLAY, camera as u32);
                    NODES_DISPLAYED.fetch_add(1, Ordering::Relaxed);
                }
                continue;
            }
        }
        if group.as_ref().map_or(0, |g| g.node) != adjust {
            if let Some(g) = group.take() {
                close_group(&g);
            }
            if adjust != 0 {
                group = Some(open_group(adjust, camera));
            }
        }
        let class = match list().get_mut(&shape) {
            Some(entry) => {
                entry.served = run;
                match entry.kind {
                    Kind::Particles => DC_PARTICLES,
                    Kind::Lines => DC_LINES,
                    _ if crate::instance_stream::is_skinned_instance(shape) => DC_SKINNED,
                    _ => DC_OTHER,
                }
            }
            None => DC_OTHER,
        };
        let display_start = crate::capture::ticks();
        vcall1(shape, SLOT_DISPLAY, camera as u32);
        let classes = &mut *addr_of_mut!(DISPLAY_CLASSES);
        classes[class][0] += 1;
        classes[class][1] += crate::capture::ticks() - display_start;
        DISPLAYED.fetch_add(1, Ordering::Relaxed);
        // A candidate the display classified as a retained instance is the table's now.
        if !crate::instance_stream::engine_serves(shape) && list().remove(&shape).is_some() {
            PRUNED.fetch_add(1, Ordering::Relaxed);
            ORDER_DIRTY.store(true, Ordering::Relaxed);
        }
    }
    if let Some(g) = group.take() {
        close_group(&g);
    }
    IN_WALK.store(false, Ordering::Relaxed);
    PART_TICKS[2].fetch_add(crate::capture::ticks() - t2, Ordering::Relaxed);
    if run <= 4 {
        let renderer: usize = rd(camera, C::spRenderer);
        let accumulator: usize = if renderer != 0 { rd(renderer, crate::gen::layouts::NiRenderer::accumulator) } else { 0 };
        crate::log!(
            "live walk: run {} in scene kind {} from root {:#x} (parent of the sky root {:#x}) with camera {:#x}; accumulator {:#x}; {} listed, displayed {}, out of view {}, unreachable {}",
            run,
            crate::link_client::scene_kind_now(),
            root,
            crate::link_client::sky_root(),
            camera,
            accumulator,
            shapes.len(),
            DISPLAYED.load(Ordering::Relaxed),
            OUT_OF_VIEW.load(Ordering::Relaxed),
            UNREACHABLE.load(Ordering::Relaxed)
        );
    }
}

enum Reach {
    /// In the view, under this sort adjust node (0 for none: the nearest of the path).
    InView(usize),
    OutOfView,
    Unreachable,
}

/// The remaining planes after a node of the run: `None` when the node is out of the view.
/// The path of a node is unique, so its verdict holds for the run.
static mut NODE_PLANES: Option<HashMap<usize, Option<u32>>> = None;
/// The nodes of a path, the root's child first.
static mut PATH: Vec<usize> = Vec::new();
/// The nodes the walk displayed whole in the run (MWSE's mode 64), and the world root of
/// the run: the engine's cull skips such a node until the root's display ends.
static mut NODES_SERVED: Option<HashSet<usize>> = None;
static WALK_ROOT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
static NODES_DISPLAYED: AtomicU64 = AtomicU64::new(0);
static CHILDREN_PREPARED: AtomicU64 = AtomicU64::new(0);
/// The billboard nodes turned to the camera in the run.
static mut TURNED: Option<HashSet<usize>> = None;
/// The side `WhichSide` returns for a bound wholly in front of a plane.
const INSIDE: u32 = 1;

/// Whether the engine's walk from `root` would display the shape: no node of the path is
/// app culled (the root passed that test before its display), each switch node of the
/// path selects the child of the path, and the bounds pass the planes as `CullShow` tests
/// them: top down, each node against the planes no node above it is wholly in front of
/// (a node out of the view ends its subtree; a shape under a node that passed every plane
/// is not tested at all, whatever its own bound: the glows of Enhanced Detection carry a
/// bound of their own, 2026-10-17).
unsafe fn reachable(shape: usize, root: usize, planes: &[usize]) -> Reach {
    // The path, bottom up: the app-cull flags and the switches on the way.
    let path = &mut *addr_of_mut!(PATH);
    path.clear();
    let mut node = shape;
    let mut reached = false;
    let mut adjust = 0usize;
    for _ in 0..MAX_DEPTH {
        if node == root {
            reached = true;
            break;
        }
        if node == 0 || vcall0(node, SLOT_GET_APP_CULLED) & 0xFF != 0 {
            return Reach::Unreachable;
        }
        let parent: usize = rd(node, AV::parentNode);
        if parent == 0 {
            return Reach::Unreachable;
        }
        if let Some(switch) = switch_kind(parent) {
            let index = switch_index(parent, switch, planes_camera());
            if index < 0 || index >= rd::<i32>(parent, N::kChildren_endIndex) {
                return Reach::Unreachable;
            }
            if rd::<usize>(rd::<usize>(parent, N::kChildren_storage), index as usize * 4) != node {
                return Reach::Unreachable;
            }
        }
        if adjust == 0 && parent != root && is_kind(parent, b"NiSortAdjustNode") {
            adjust = parent;
        }
        path.push(node);
        node = parent;
    }
    if !reached {
        return Reach::Unreachable;
    }
    // Top down: the planes that remain after each node; the shape is the last of the path.
    let verdicts = (*addr_of_mut!(NODE_PLANES)).get_or_insert_with(HashMap::default);
    let mut remaining: u32 = if planes.len() >= 32 { u32::MAX } else { (1u32 << planes.len()) - 1 };
    for &node in path.iter().rev() {
        let after = match verdicts.get(&node) {
            Some(&after) => after,
            None => {
                let mut after = Some(remaining);
                for (index, &plane) in planes.iter().enumerate() {
                    if remaining & (1 << index) == 0 {
                        continue;
                    }
                    match thiscall1(WHICH_SIDE, node + AV::worldBound, plane as u32) {
                        OUTSIDE => {
                            after = None;
                            break;
                        }
                        INSIDE => {
                            if let Some(mask) = after.as_mut() {
                                *mask &= !(1 << index);
                            }
                        }
                        _ => {}
                    }
                }
                if node != shape {
                    verdicts.insert(node, after);
                    // A switch, a level of detail or a flt animation node in view does its
                    // display's work: the level writes its index, and the selected child
                    // gets the lazy update before its own bound is tested (R26).
                    if after.is_some() {
                        if let Some(switch) = switch_kind(node) {
                            let index = switch_index(node, switch, planes_camera());
                            if matches!(switch, Switch::LevelOfDetail) {
                                wr(node, SWITCH_INDEX, index);
                            }
                            if index >= 0 && index < rd::<i32>(node, N::kChildren_endIndex) {
                                let child: usize = rd(rd::<usize>(node, N::kChildren_storage), index as usize * 4);
                                if child != 0 {
                                    switch_prepare_child(node, index, child);
                                    CHILDREN_PREPARED.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                    if after.is_some() && is_kind(node, b"NiBillboardNode") {
                        let turned = (*addr_of_mut!(TURNED)).get_or_insert_with(HashSet::default);
                        if turned.insert(node) {
                            thiscall1(ROTATE_TO_CAMERA, node, planes_camera() as u32);
                        }
                    }
                }
                after
            }
        };
        match after {
            None => return Reach::OutOfView,
            Some(mask) => remaining = mask,
        }
    }
    Reach::InView(adjust)
}

/// True when the run-time type of the node is `name` or comes from it.
pub(crate) unsafe fn is_kind(node: usize, name: &[u8]) -> bool {
    let mut rtti = crate::ports::ni::get_rtti(node);
    for _ in 0..16 {
        if rtti == 0 {
            return false;
        }
        if name_is(rd(rtti, RTTI_NAME), name) {
            return true;
        }
        rtti = rd(rtti, RTTI_BASE);
    }
    false
}

/// The accumulator of the renderer around the displays of a group of shapes under one
/// sort adjust node, as `NiSortAdjustNode::Display` sets it: none for sorting off, the
/// node's own (started) for sub-sort. `open` returns what `close` needs.
struct Group {
    node: usize,
    pointer: usize,
    before: usize,
    own_started: bool,
}

unsafe fn add_ref(object: usize) {
    wr(object, 4, rd::<i32>(object, 4) + 1);
}

unsafe fn release_ref(object: usize) {
    let count = rd::<i32>(object, 4) - 1;
    wr(object, 4, count);
    if count == 0 {
        vcall1(object, 0, 1);
    }
}

unsafe fn open_group(node: usize, camera: usize) -> Group {
    let renderer = rd::<usize>(camera, C::spRenderer);
    let pointer = renderer + R::accumulator;
    let before = rd::<usize>(pointer, 0);
    if before != 0 {
        add_ref(before);
    }
    let mut own_started = false;
    let mode = rd::<u32>(node, SORT_ADJUST_MODE);
    if mode == SORTING_OFF {
        ni_pointer_set(pointer, 0);
    } else if mode == SORTING_SUBSORT {
        let own = rd::<usize>(node, SORT_ADJUST_ACCUMULATOR);
        let current = rd::<usize>(pointer, 0);
        if current != own {
            if current != 0 {
                crate::engine::ni_object_release(current);
            }
            wr(pointer, 0, own);
            if own != 0 {
                add_ref(own);
            }
        }
        if own != 0 && vcall1(own, SLOT_START_ACCUMULATING, camera as u32) & 0xFF != 0 {
            own_started = true;
        } else {
            let current = rd::<usize>(pointer, 0);
            if current != 0 {
                release_ref(current);
                wr(pointer, 0, 0);
            }
        }
    }
    Group { node, pointer, before, own_started }
}

unsafe fn close_group(group: &Group) {
    if group.own_started {
        vcall0(rd::<usize>(group.node, SORT_ADJUST_ACCUMULATOR), SLOT_FINISH_ACCUMULATING);
    }
    let after = rd::<usize>(group.pointer, 0);
    if after != group.before {
        if after != 0 {
            release_ref(after);
        }
        wr(group.pointer, 0, group.before);
        if group.before != 0 {
            add_ref(group.before);
        }
    }
    if group.before != 0 {
        release_ref(group.before);
    }
}

/// The path of a shape from the root: the index of each node of the path in its parent,
/// the root's child first. Sorted, the paths give the engine's depth-first order. A shape
/// that does not reach the root sorts last (it is unreachable anyway).
unsafe fn tree_path(shape: usize, root: usize) -> Vec<u32> {
    let indexes = (*addr_of_mut!(NODE_INDEXES)).get_or_insert_with(HashMap::default);
    let mut path = Vec::with_capacity(8);
    let mut node = shape;
    for _ in 0..MAX_DEPTH {
        if node == root {
            path.reverse();
            return path;
        }
        if node == 0 {
            break;
        }
        let parent: usize = rd(node, AV::parentNode);
        if parent == 0 {
            break;
        }
        let index = match indexes.get(&node) {
            Some(&index) => index,
            None => {
                let count = rd::<u32>(parent, N::kChildren_endIndex);
                let storage: usize = rd(parent, N::kChildren_storage);
                let index = (0..count).find(|&i| rd::<usize>(storage, i as usize * 4) == node).unwrap_or(u32::MAX);
                indexes.insert(node, index);
                index
            }
        };
        path.push(index);
        node = parent;
    }
    vec![u32::MAX]
}

/// The camera of the run (for the level of detail by distance).
fn planes_camera() -> usize {
    CAMERA.load(Ordering::Relaxed)
}

#[derive(Clone, Copy)]
enum Switch {
    Plain,
    LevelOfDetail,
    FltAnimation,
}

/// The kinds of a switch node, for the table walk (`instance_stream`).
#[derive(Clone, Copy)]
pub enum SwitchKind {
    Plain,
    LevelOfDetail,
    FltAnimation,
}

pub unsafe fn switch_kind_of(node: usize) -> Option<SwitchKind> {
    switch_kind(node).map(|s| match s {
        Switch::Plain => SwitchKind::Plain,
        Switch::LevelOfDetail => SwitchKind::LevelOfDetail,
        Switch::FltAnimation => SwitchKind::FltAnimation,
    })
}

pub unsafe fn switch_index_of(node: usize, kind: SwitchKind, camera: usize) -> i32 {
    let switch = match kind {
        SwitchKind::Plain => Switch::Plain,
        SwitchKind::LevelOfDetail => Switch::LevelOfDetail,
        SwitchKind::FltAnimation => Switch::FltAnimation,
    };
    switch_index(node, switch, camera)
}

/// The most derived switch kind of the node's run-time type, or `None` for a node that
/// displays all its children.
unsafe fn switch_kind(node: usize) -> Option<Switch> {
    let mut rtti = crate::ports::ni::get_rtti(node);
    for _ in 0..16 {
        if rtti == 0 {
            return None;
        }
        let name: usize = rd(rtti, RTTI_NAME);
        if name_is(name, b"NiLODNode") {
            return Some(Switch::LevelOfDetail);
        }
        if name_is(name, b"NiFltAnimationNode") {
            return Some(Switch::FltAnimation);
        }
        if name_is(name, b"NiSwitchNode") {
            return Some(Switch::Plain);
        }
        rtti = rd(rtti, RTTI_BASE);
    }
    None
}

unsafe fn name_is(name: usize, want: &[u8]) -> bool {
    if name == 0 {
        return false;
    }
    want.iter().enumerate().all(|(i, &b)| rd::<u8>(name, i) == b) && rd::<u8>(name, want.len()) == 0
}

/// The child the switch node selects, as its display selects it: the index of a plain
/// switch; the level by the distance of a level-of-detail node (the same arithmetic as
/// `NiLODNode::Display`, without its write); the child by the saved time of a flt
/// animation node (the engine's own function, which the display calls again: it writes
/// the same index).
unsafe fn switch_index(node: usize, switch: Switch, camera: usize) -> i32 {
    match switch {
        Switch::Plain => rd(node, SWITCH_INDEX),
        Switch::FltAnimation => {
            thiscall1(SWITCH_CHILD_FROM_TIMING, node, rd::<u32>(node, SWITCH_SAVED_TIME));
            rd(node, SWITCH_INDEX)
        }
        Switch::LevelOfDetail => {
            let adjust = rd::<f32>(camera, CAMERA_LOD_ADJUST);
            let dx = rd::<f32>(node, LOD_WORLD_CENTRE) - rd::<f32>(camera, CAMERA_TRANSLATION);
            let dy = rd::<f32>(node, LOD_WORLD_CENTRE + 4) - rd::<f32>(camera, CAMERA_TRANSLATION + 4);
            let dz = rd::<f32>(node, LOD_WORLD_CENTRE + 8) - rd::<f32>(camera, CAMERA_TRANSLATION + 8);
            let levels = rd::<usize>(node, LOD_DATA);
            let mut index = rd::<i32>(node, N::kChildren_endIndex) - 1;
            let distance = ((dz * dz + dy * dy) + dx * dx) * (adjust * adjust);
            while index >= 0 {
                let level = levels + index as usize * LOD_DATA_SIZE;
                if rd::<f32>(level, LOD_WORLD_NEAR_SQUARED) <= distance && distance < rd::<f32>(level, LOD_WORLD_FAR_SQUARED) {
                    break;
                }
                index -= 1;
            }
            index
        }
    }
}

pub unsafe fn report() -> String {
    let list = &*addr_of!(LIST);
    let mut by_kind = [0usize; 4];
    if let Some(list) = list {
        for entry in list.values() {
            by_kind[entry.kind as usize] += 1;
        }
    }
    format!(
        "live walk: {}; listed {} (shapes {}, strips {}, particles {}, lines {}), forgotten {}; runs {}, displayed {}, out of view {}, unreachable {}; displays of the engine skipped as served {}; switch children prepared {}, nodes displayed whole {}; sky-only walk {}: cut runs {}, discovery frames {}, candidates added {}, dropped {}, pruned {}, pending {} (switch {}, added {}, released {}), far {} (added {}), forced displays {}; time {:.3} ms ({:.4} ms a run: candidates {:.4}, order {:.4} ({} builds), reach and displays {:.4})",
        if on() { "on" } else { "off" },
        by_kind.iter().sum::<usize>(),
        by_kind[0],
        by_kind[1],
        by_kind[2],
        by_kind[3],
        FORGOTTEN.load(Ordering::Relaxed),
        RUN.load(Ordering::Relaxed),
        DISPLAYED.load(Ordering::Relaxed),
        OUT_OF_VIEW.load(Ordering::Relaxed),
        UNREACHABLE.load(Ordering::Relaxed),
        SKIPPED_AS_SERVED.load(Ordering::Relaxed),
        CHILDREN_PREPARED.load(Ordering::Relaxed),
        NODES_DISPLAYED.load(Ordering::Relaxed),
        if sky_only_on() { "on" } else { "off" },
        CUT_RUNS.load(Ordering::Relaxed),
        DISCOVERY_FRAMES.load(Ordering::Relaxed),
        CANDIDATES_ADDED.load(Ordering::Relaxed),
        CANDIDATES_DROPPED.load(Ordering::Relaxed),
        PRUNED.load(Ordering::Relaxed),
        (*addr_of!(PENDING)).as_ref().map_or(0, |p| p.len()),
        (*addr_of!(PENDING_SWITCH)).as_ref().map_or(0, |p| p.len()),
        PENDING_ADDED.load(Ordering::Relaxed),
        PENDING_RELEASED.load(Ordering::Relaxed),
        (*addr_of!(FAR)).as_ref().map_or(0, |p| p.len()),
        FAR_ADDED.load(Ordering::Relaxed),
        FORCED.load(Ordering::Relaxed),
        WALK_TICKS.load(Ordering::Relaxed) as f64 * 1000.0 / crate::capture::ticks_per_second() as f64,
        WALK_TICKS.load(Ordering::Relaxed) as f64 * 1000.0 / crate::capture::ticks_per_second() as f64 / RUN.load(Ordering::Relaxed).max(1) as f64,
        PART_TICKS[0].load(Ordering::Relaxed) as f64 * 1000.0 / crate::capture::ticks_per_second() as f64 / RUN.load(Ordering::Relaxed).max(1) as f64,
        PART_TICKS[1].load(Ordering::Relaxed) as f64 * 1000.0 / crate::capture::ticks_per_second() as f64 / RUN.load(Ordering::Relaxed).max(1) as f64,
        ORDER_BUILDS.load(Ordering::Relaxed),
        PART_TICKS[2].load(Ordering::Relaxed) as f64 * 1000.0 / crate::capture::ticks_per_second() as f64 / RUN.load(Ordering::Relaxed).max(1) as f64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_of_draws() {
        assert!(matches!(Kind::of_draw(crate::link::DRAW_SHAPE), Kind::Shape));
        assert!(matches!(Kind::of_draw(crate::link::DRAW_STRIPS), Kind::Strips));
        assert!(matches!(Kind::of_draw(crate::link::DRAW_POINTS), Kind::Particles));
        assert!(matches!(Kind::of_draw(crate::link::DRAW_LINES), Kind::Lines));
    }

    #[test]
    fn names() {
        let name = b"NiLODNode\0";
        assert!(unsafe { name_is(name.as_ptr() as usize, b"NiLODNode") });
        assert!(!unsafe { name_is(name.as_ptr() as usize, b"NiLOD") });
        assert!(!unsafe { name_is(name.as_ptr() as usize, b"NiLODNodeX") });
        assert!(!unsafe { name_is(0, b"NiLODNode") });
    }
}
