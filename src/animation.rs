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

/// The world camera of the frame: the values of its culling planes (the normal and the
/// constant of each), its position and its level of detail adjustment. Keep values, not the
/// planes' addresses: the tests run in the next frame's update, after the scene that noted them.
struct Camera {
    planes: Vec<[f64; 4]>,
    position: [f32; 3],
    lod_adjust: f32,
}
static CAMERA: MainCell<Camera> = MainCell::new(Camera { planes: Vec::new(), position: [0.0; 3], lod_adjust: 1.0 });

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
    let from_env = crate::switches::parsed("NIRENDER_ANIMATION", "the name of a policy", policy_of).unwrap_or(Policy::ViewShadows);
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
    let count = rd::<i32>(camera, C::countCullingPlanes).max(0) as usize;
    let storage: usize = rd(camera, C::cullingPlanes_storage);
    let this = &mut *CAMERA.get(t);
    this.planes = (0..count)
        .map(|i| {
            let plane: [f32; 4] = rd(rd::<usize>(storage, i * 4), 0);
            plane.map(f64::from)
        })
        .collect();
    this.position = rd(camera, WORLD_TRANSLATION);
    this.lod_adjust = rd(camera, C::fLODAdjust);
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
    // The rule of `NiBound::WhichSide` (0x6F2660) [I: transcribed, not compared]: outside a
    // plane when the signed distance of the centre is at most minus the radius.
    let [x, y, z, radius] = rd::<[f32; 4]>(bound, 0).map(f64::from);
    if !camera.planes.is_empty() && camera.planes.iter().all(|p| p[0] * x + p[1] * y + p[2] * z - p[3] > -radius) {
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
/// rules for a billboard). The planes are not walked: a parent's bound holds its child's. [`None`]
/// when nothing does, and when the chain is too long to read.
unsafe fn withheld(camera: &Camera, node: usize) -> Option<Withheld> {
    if rd::<u16>(node, AV::flags) & APP_CULLED != 0 {
        return Some(Withheld::Itself);
    }
    let mut child = node;
    let found = crate::parent_chain::first_answer(rd(node, AV::parentNode), crate::parent_chain::GUARD, |n| rd(n, AV::parentNode), |parent| {
        let reason = barred_by(camera, parent, child);
        child = parent;
        reason
    });
    match found {
        Ok(reason) => reason,
        Err(crate::parent_chain::ChainTooLong) => {
            crate::parent_chain::note_too_long("the animation flags");
            None
        }
    }
}

/// What keeps the walk at this parent from going on to this child.
unsafe fn barred_by(camera: &Camera, parent: usize, child: usize) -> Option<Withheld> {
    let flags = rd::<u16>(parent, AV::flags);
    if flags & APP_CULLED != 0 {
        return Some(Withheld::Above);
    }
    if crate::ports::ni::is_kind_of(parent, RTTI_SWITCH_NODE) {
        return (selected_child(camera, parent) != child).then_some(Withheld::NotSelected);
    }
    (flags & CONTAINS_GEOMETRY == 0).then_some(Withheld::NoGeometry)
}

/// The child that a switch displays, or 0. A level of detail selects it at its display
/// (`NiLODNode::Display` 0x7412B0) from the camera: here from the same values, as the index
/// that it keeps is old when its display did not run. Another switch keeps its index.
unsafe fn selected_child(camera: &Camera, switch: usize) -> usize {
    let count = rd::<u32>(switch, NODE::kChildren_endIndex) as usize;
    let index = if crate::ports::ni::is_kind_of(switch, RTTI_LOD_NODE) {
        let centre: [f32; 3] = rd(switch, LOD::worldCentre);
        let d: [f32; 3] = core::array::from_fn(|i| centre[i] - camera.position[i]);
        let levels: usize = rd(switch, LOD::lodData);
        let distance_squared = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) * (camera.lod_adjust * camera.lod_adjust);
        lod_index(count, distance_squared, |level| {
            let at = levels + level * LEVEL::SIZE;
            (rd(at, LEVEL::worldNearSquared), rd(at, LEVEL::worldFarSquared))
        })
    } else {
        rd::<i32>(switch, SWITCH::switchIndex)
    };
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
    format!(
        "animation: policy {}; answers: in view {}, by the shadows {}, all {}, no bound {}, not animated {}, withheld for an app-culled node {}, withheld below an app-culled node {}, withheld as not selected by a switch {}, withheld under no geometry {}; the chains: {} in {:.1} ms",
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
        CHAIN_NANOSECONDS.load(Ordering::Relaxed) as f64 / 1e6
    )
}

#[cfg(test)]
mod tests {
    use super::lod_index;

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
}
