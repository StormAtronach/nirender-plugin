//! What animates in mode gpu while the stream records (`active`). The engine lets an actor
//! or an animated node compute its pose only when a Click displayed it in the frame before:
//! `NiNode::Display` (0x6C9190) sets the flag 0x10 of a node, `BSAnimationNode::Display` the
//! bit 8 of an animation node, and three readers consume them. The engine's walk sets these
//! flags itself. In mode gpu the readers' ports (`ports/animation.rs`) also set the flag on
//! the nodes that the policy selects here. The test is the node's world bound against the
//! planes of the world camera and, by the policy, the reach of the shadow map that the
//! helper reports (`capture::helper_shadow_reach`).
//!
//! The policy (`NIRENDER_ANIMATION`, `nirender.animationPolicy`): `view` animates what is
//! in the camera's view; `shadows` (the default) also what the shadow map reaches, so that a
//! shadow moves while its caster is out of view; `all` animates everything loaded.

use core::ptr::addr_of;
use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use crate::engine::vcall0;
use crate::gen::layouts::NiCamera as C;
use crate::main_thread::MainThread;
use crate::mem::rd;

/// The vtable slot of `NiAVObject::GetWorldBound` (as `ports/scene.rs`).
const SLOT_GET_WORLD_BOUND: usize = 0x34;
/// `NiAVObject::worldTransform.translation` and `NiBound::radius`.
const WORLD_TRANSLATION: usize = 0x64;
const BOUND_RADIUS: usize = 0xC;

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
static NEITHER: AtomicU64 = AtomicU64::new(0);

/// The world camera of the frame: the values of its culling planes (the normal and the
/// constant of each) and its position. Keep values, not the planes' addresses: the tests run
/// in the next frame's update, after the scene that noted them.
struct Camera {
    planes: Vec<[f64; 4]>,
    position: [f32; 3],
}
static mut CAMERA: Camera = Camera { planes: Vec::new(), position: [0.0; 3] };

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
    let from_env = std::env::var("NIRENDER_ANIMATION").ok().and_then(|v| policy_of(&v)).unwrap_or(Policy::ViewShadows);
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
pub unsafe fn note_camera(camera: usize) {
    let count = rd::<i32>(camera, C::countCullingPlanes).max(0) as usize;
    let storage: usize = rd(camera, C::cullingPlanes_storage);
    let this = &mut *core::ptr::addr_of_mut!(CAMERA);
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
/// (policy `all`). A node without a bound animates.
pub unsafe fn displayed(node: usize) -> bool {
    let policy = policy();
    if policy == Policy::All {
        BY_ALL.fetch_add(1, Ordering::Relaxed);
        return true;
    }
    let bound = vcall0(node, SLOT_GET_WORLD_BOUND) as usize;
    if bound == 0 {
        return true;
    }
    let camera = &*addr_of!(CAMERA);
    // The rule of `NiBound::WhichSide` (0x6F2660) [I: transcribed, not compared]: outside a
    // plane when the signed distance of the centre is at most minus the radius.
    let [x, y, z, radius] = rd::<[f32; 4]>(bound, 0).map(f64::from);
    if !camera.planes.is_empty() && camera.planes.iter().all(|p| p[0] * x + p[1] * y + p[2] * z - p[3] > -radius) {
        BY_VIEW.fetch_add(1, Ordering::Relaxed);
        return true;
    }
    if policy == Policy::ViewShadows {
        let reach = crate::capture::helper_shadow_reach();
        if reach > 0.0 {
            let centre: [f32; 3] = rd(bound, 0);
            let radius: f32 = rd(bound, BOUND_RADIUS);
            let d: [f32; 3] = core::array::from_fn(|i| centre[i] - camera.position[i]);
            if (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() - radius <= reach {
                BY_SHADOWS.fetch_add(1, Ordering::Relaxed);
                return true;
            }
        }
    }
    NEITHER.fetch_add(1, Ordering::Relaxed);
    false
}

/// One line for the log: the policy and the counts of the answers.
pub fn report() -> String {
    format!(
        "animation: policy {}; answers: in view {}, by the shadows {}, all {}, not animated {}",
        policy_name(),
        BY_VIEW.load(Ordering::Relaxed),
        BY_SHADOWS.load(Ordering::Relaxed),
        BY_ALL.load(Ordering::Relaxed),
        NEITHER.load(Ordering::Relaxed)
    )
}
