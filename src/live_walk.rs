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
//! Off by default in this part (`NIRENDER_LIVE_WALK=1`, `nirender.liveWalk(true)`): with
//! the engine's walk still going it adds nothing. The test is a pair of captures of one
//! session, the walk off and on, replayed and equal to the pixel.

use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::collections::HashMap;

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
/// 255: not read yet (the environment is read at the first call).
static ON: AtomicU8 = AtomicU8::new(255);
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
    (*addr_of_mut!(LIST)).get_or_insert_with(HashMap::new)
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

pub fn on() -> bool {
    let value = ON.load(Ordering::Relaxed);
    if value != 255 {
        return value != 0;
    }
    let on = std::env::var("NIRENDER_LIVE_WALK").map_or(false, |v| v != "0");
    ON.store(on as u8, Ordering::Relaxed);
    on
}

/// Turns the walk on or off; returns the state before.
pub fn set_on(on: bool) -> bool {
    let before = self::on();
    ON.store(on as u8, Ordering::Relaxed);
    before
}

/// A display port serves the shape (a draw, or the notes of a live instance): the shape
/// joins the list. Not during the walk's own displays: the shape is on the list already.
pub unsafe fn note(shape: usize, kind: Kind) {
    if IN_WALK.load(Ordering::Relaxed) {
        return;
    }
    list().entry(shape).or_insert(Entry { kind, served: 0 });
}

/// The destructor of a geometry: the shape leaves the list.
pub unsafe fn forget(shape: usize) {
    if list().remove(&shape).is_some() {
        FORGOTTEN.fetch_add(1, Ordering::Relaxed);
    }
}

/// True when the walk displayed the shape in the run in progress: the engine's walk, which
/// reaches it after, must not display it again. Never during the walk's own displays.
pub unsafe fn served(shape: usize) -> bool {
    if !on() || IN_WALK.load(Ordering::Relaxed) {
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
/// runs (mode gpu, a stream going).
pub unsafe fn after_child(parent: usize, child: usize, camera: usize) {
    if !on() || child == 0 || child != crate::link_client::sky_root() {
        return;
    }
    if crate::instance_stream::mode() != crate::instance_stream::Mode::Gpu || !crate::capture::recording() {
        return;
    }
    walk(parent, camera);
}

unsafe fn walk(root: usize, camera: usize) {
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
    (*addr_of_mut!(NODE_INDEXES)).get_or_insert_with(HashMap::new).clear();
    (*addr_of_mut!(NODE_PLANES)).get_or_insert_with(HashMap::new).clear();
    (*addr_of_mut!(NODES_SERVED)).get_or_insert_with(std::collections::HashSet::new).clear();
    WALK_ROOT.store(root, Ordering::Relaxed);
    (*addr_of_mut!(TURNED)).get_or_insert_with(std::collections::HashSet::new).clear();
    shapes.extend(list().keys().copied());
    let ordered = &mut *addr_of_mut!(ORDERED);
    ordered.clear();
    for &shape in shapes.iter() {
        ordered.push((tree_path(shape, root), shape));
    }
    ordered.sort_unstable();
    IN_WALK.store(true, Ordering::Relaxed);
    let mut group: Option<Group> = None;
    for &(_, shape) in ordered.iter() {
        let adjust = match reachable(shape, root, planes) {
            Reach::Unreachable => {
                UNREACHABLE.fetch_add(1, Ordering::Relaxed);
                continue;
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
                let served = (*addr_of_mut!(NODES_SERVED)).get_or_insert_with(std::collections::HashSet::new);
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
        if let Some(entry) = list().get_mut(&shape) {
            entry.served = run;
        }
        vcall1(shape, SLOT_DISPLAY, camera as u32);
        DISPLAYED.fetch_add(1, Ordering::Relaxed);
    }
    if let Some(g) = group.take() {
        close_group(&g);
    }
    IN_WALK.store(false, Ordering::Relaxed);
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
static mut NODES_SERVED: Option<std::collections::HashSet<usize>> = None;
static WALK_ROOT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
static NODES_DISPLAYED: AtomicU64 = AtomicU64::new(0);
static CHILDREN_PREPARED: AtomicU64 = AtomicU64::new(0);
/// The billboard nodes turned to the camera in the run.
static mut TURNED: Option<std::collections::HashSet<usize>> = None;
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
    let verdicts = (*addr_of_mut!(NODE_PLANES)).get_or_insert_with(HashMap::new);
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
                        let turned = (*addr_of_mut!(TURNED)).get_or_insert_with(std::collections::HashSet::new);
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
unsafe fn is_kind(node: usize, name: &[u8]) -> bool {
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
    let indexes = (*addr_of_mut!(NODE_INDEXES)).get_or_insert_with(HashMap::new);
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
        "live walk: {}; listed {} (shapes {}, strips {}, particles {}, lines {}), forgotten {}; runs {}, displayed {}, out of view {}, unreachable {}; displays of the engine skipped as served {}; switch children prepared {}, nodes displayed whole {}",
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
