//! What animates in mode gpu while the stream records (`active`). The engine lets an actor
//! or an animated node compute its pose only when a Click displayed it in the frame before:
//! `NiNode::Display` (0x6C9190) sets the flag 0x10 of a node, `BSAnimationNode::Display` the
//! bit 8 of an animation node, and three readers consume them. The engine's walk sets these
//! flags itself. In mode gpu the readers' ports (`ports/animation.rs`) also set the flag on
//! the nodes that the policy selects here. The test is the node's world bound against the
//! planes of the world camera and, by the policy, the reach of the shadow map that the
//! helper reports (`capture::helper_shadow_reach`). A node that is app-culled, or below an
//! app-culled node, is not selected: the engine's walk does not reach it either.
//!
//! The policy (`NIRENDER_ANIMATION`, `nirender.animationPolicy`): `view` animates what is
//! in the camera's view; `shadows` (the default) also what the shadow map reaches, so that a
//! shadow moves while its caster is out of view; `all` animates everything loaded.

use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use crate::engine::vcall0;
use crate::gen::layouts::NiAVObject as AV;
use crate::gen::layouts::NiCamera as C;
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

/// The world camera of the frame: the values of its culling planes (the normal and the
/// constant of each) and its position. Keep values, not the planes' addresses: the tests run
/// in the next frame's update, after the scene that noted them.
struct Camera {
    planes: Vec<[f64; 4]>,
    position: [f32; 3],
}
static CAMERA: MainCell<Camera> = MainCell::new(Camera { planes: Vec::new(), position: [0.0; 3] });

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
}

/// Whether the node animates this frame by the policy: its world bound within the planes of
/// the world camera, or within the reach of the shadow map (policy `shadows`), or always
/// (policy `all`). Not when the engine's walk would not reach the node: it, or a node above
/// it, is app-culled (`NiAVObject::CullShow` 0x6EB480 skips an app-culled node and what is
/// below it). A chain that the walk cannot read to its end counts as not app-culled: the
/// node animates, as it did before this test. A node without a bound animates: none of the
/// three readers can meet one (see `reached`).
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
    match app_culled(node) {
        Some(Culled::Itself) => WITHHELD_ITSELF.fetch_add(1, Ordering::Relaxed),
        Some(Culled::Above) => WITHHELD_ABOVE.fetch_add(1, Ordering::Relaxed),
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

/// Where the app-culled flag of the engine's walk was found.
#[derive(Debug, PartialEq, Eq)]
enum Culled {
    Itself,
    Above,
}

/// Bit 0 of the flags of a `NiAVObject`, which `NiAVObject::GetAppCulled` 0x442B30 reads.
const APP_CULLED: u16 = 0x01;

/// The app-culled node that keeps the engine's walk from the node: the node itself, or the
/// nearest one above it. [`None`] when there is none, and when the chain is too long to read.
unsafe fn app_culled(node: usize) -> Option<Culled> {
    let culled = |node: usize| rd::<u16>(node, AV::flags) & APP_CULLED != 0;
    if culled(node) {
        return Some(Culled::Itself);
    }
    match crate::parent_chain::first_answer(rd(node, AV::parentNode), crate::parent_chain::GUARD, |n| rd(n, AV::parentNode), |n| culled(n).then_some(())) {
        Ok(found) => found.map(|()| Culled::Above),
        Err(crate::parent_chain::ChainTooLong) => {
            crate::parent_chain::note_too_long("the animation flags");
            None
        }
    }
}

/// One line for the log: the policy and the counts of the answers. The two `withheld` counts
/// are the nodes that the bound or the shadow reach selected and the engine's walk would not
/// reach: before the app-culled test they were given the flag.
pub fn report() -> String {
    format!(
        "animation: policy {}; answers: in view {}, by the shadows {}, all {}, no bound {}, not animated {}, withheld for an app-culled node {}, withheld below an app-culled node {}",
        policy_name(),
        BY_VIEW.load(Ordering::Relaxed),
        BY_SHADOWS.load(Ordering::Relaxed),
        BY_ALL.load(Ordering::Relaxed),
        NO_BOUND.load(Ordering::Relaxed),
        NEITHER.load(Ordering::Relaxed),
        WITHHELD_ITSELF.load(Ordering::Relaxed),
        WITHHELD_ABOVE.load(Ordering::Relaxed)
    )
}
