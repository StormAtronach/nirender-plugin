//! World matrices relative to the camera.
//!
//! Far from the origin of the world, a 32-bit float has steps of some game units, and the
//! picture shakes. The remedy is to give the device world matrices whose translation is
//! relative to the camera, made with 64-bit arithmetic from the scene graph.
//!
//! This is a port of `camerarelative.cpp` of MGE XE: the same steps in the same order, so the
//! results are equal bit for bit. It has two uses:
//!
//! - Compare: the hooks of MGE XE make the matrices, and this module makes them too and counts
//!   the differences.
//! - Own: this module makes the matrices and MGE XE does not. The fact
//!   `FACT_CLIENT_CAMERA_SPACE` of the render link turns it on.
//!
//! Only the render thread uses the state of this module.

use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::engine::{thiscall0, WORLD_CONTROLLER};
use crate::gen::layouts::NiDX8Renderer as R;
use crate::hook::{self, Cleanup};
use crate::mem::rd;
use crate::ports::renderer_frame;

// `NiTransform`: a row-major rotation, a translation and a scale.
const T_TRANSLATION: usize = 0x24;
const T_SCALE: usize = 0x30;
const T_WORDS: usize = 13;
// `NiAVObject`
const AV_PARENT: usize = 0x18;
const AV_LOCAL_TRANSLATE: usize = 0x30;
const AV_WORLD: usize = 0x40;
// `NiSkinInstance`
const SKIN_DATA: usize = 0x8;
const SKIN_ROOT_PARENT: usize = 0xC;
const SKIN_BONES: usize = 0x10;
// `NiSkinData`: the transform from the root parent to the skin, and the bone records.
const DATA_TRANSFORM: usize = 0xC;
const DATA_BONE_COUNT: usize = 0x40;
const DATA_BONES: usize = 0x44;
const BONE_RECORD_SIZE: usize = 0x4C;
// `NiSkinPartition::Partition`
const PARTITION_BONES: usize = 0x4;
const PARTITION_BONE_COUNT: usize = 0x20;
/// `WorldController`: the roots that the game copies the eye of the first person into (world,
/// arm and shadow camera).
const WC_CAMERA_ROOTS: [usize; 3] = [0x130, 0x15C, 0x2BC];
/// `WorldController`: the world camera and the arm camera. Their scenes are relative.
const WC_OWNED_CAMERAS: [usize; 2] = [0x134, 0x160];
/// `PlayerAnimController`: the "Camera" node of the first-person model.
const CONTROLLER_HEAD_CAMERA: usize = 0xD4;
/// 0x542E60 `PlayerAnimController::updateCameraTransforms`, and its first instruction.
const UPDATE_CAMERA_TRANSFORMS: usize = 0x542E60;
const UPDATE_CAMERA_TRANSFORMS_HEAD: [u8; 6] = [0x64, 0xA1, 0x00, 0x00, 0x00, 0x00];

const MAX_CHAIN_DEPTH: u32 = 64;
const GUARD_STEPS: f32 = 16.0;
const CACHE_SLOTS: usize = 8192;
const CACHE_PROBE_LIMIT: usize = 16;
const EYE_MAX_PARENT_DEPTH: u32 = 3;

// ---- transforms with 64-bit numbers

#[derive(Clone, Copy)]
struct DTransform {
    r: [[f64; 3]; 3],
    t: [f64; 3],
    s: f64,
}

unsafe fn from_ni(transform: usize) -> DTransform {
    let r: [[f32; 3]; 3] = rd(transform, 0);
    let t: [f32; 3] = rd(transform, T_TRANSLATION);
    DTransform { r: r.map(|row| row.map(f64::from)), t: t.map(f64::from), s: rd::<f32>(transform, T_SCALE) as f64 }
}

/// `NiTransform::operator*`: `(a * b)(p) = a(b(p))`.
fn combine(a: &DTransform, b: &DTransform) -> DTransform {
    let mut out = DTransform { r: [[0.0; 3]; 3], t: [0.0; 3], s: a.s * b.s };
    for i in 0..3 {
        for j in 0..3 {
            out.r[i][j] = a.r[i][0] * b.r[0][j] + a.r[i][1] * b.r[1][j] + a.r[i][2] * b.r[2][j];
        }
        out.t[i] = (a.r[i][0] * b.t[0] + a.r[i][1] * b.t[1] + a.r[i][2] * b.t[2]) * a.s + a.t[i];
    }
    out
}

/// `NiTransform::Invert` for a rotation with axes of length 1 at right angles.
fn invert(a: &DTransform) -> Option<DTransform> {
    if a.s == 0.0 {
        return None;
    }
    let mut out = DTransform { r: [[0.0; 3]; 3], t: [0.0; 3], s: 1.0 / a.s };
    for i in 0..3 {
        for j in 0..3 {
            out.r[i][j] = a.r[j][i];
        }
    }
    for i in 0..3 {
        out.t[i] = -(out.r[i][0] * a.t[0] + out.r[i][1] * a.t[1] + out.r[i][2] * a.t[2]) * out.s;
    }
    Some(out)
}

// ---- the exact world translation of a node

/// The step of a float of this size: `ldexp(1, exponent - 24)` with the exponent of `frexp`.
fn float_step(magnitude: f32) -> f32 {
    let exponent = if magnitude == 0.0 || !magnitude.is_finite() {
        0
    } else {
        let mut value = magnitude.abs() as f64;
        let mut exponent = 0;
        while value >= 1.0 {
            value *= 0.5;
            exponent += 1;
        }
        while value < 0.5 {
            value *= 2.0;
            exponent -= 1;
        }
        exponent
    };
    2f64.powi(exponent - 24) as f32
}

fn within_guard(computed: f32, stored: f32) -> bool {
    let delta = (computed - stored).abs();
    let magnitude = if stored.abs() > computed.abs() { stored.abs() } else { computed.abs() };
    delta <= GUARD_STEPS * float_step(magnitude)
}

/// True when the game made the stored translation of the node from its parent and its local
/// translation. The arithmetic is that of `NiAVObject::UpdateWorldData`, in 32 bits and in
/// its order. When it is false, the game put the node there in another way, and the stored
/// value is the start of the chain.
unsafe fn stored_follows_parent(node: usize, parent: usize) -> bool {
    let s: f32 = rd(parent, AV_WORLD + T_SCALE);
    let l: [f32; 3] = rd(node, AV_LOCAL_TRANSLATE);
    let m: [[f32; 3]; 3] = rd(parent, AV_WORLD);
    let pt: [f32; 3] = rd(parent, AV_WORLD + T_TRANSLATION);
    let st: [f32; 3] = rd(node, AV_WORLD + T_TRANSLATION);
    (0..3).all(|i| {
        let rotated = m[i][0] * l[0] + m[i][1] * l[1] + m[i][2] * l[2];
        within_guard(pt[i] + rotated * s, st[i])
    })
}

/// A cache entry: the exact translation of a node while its stored translation stays.
#[derive(Clone, Copy)]
struct CacheEntry {
    node: usize,
    generation: u32,
    stored: [f32; 3],
    exact: [f64; 3],
}

static mut CACHE: *mut CacheEntry = core::ptr::null_mut();
static mut CACHE_GENERATION: u32 = 1;

/// Makes all cache entries old. The table is not cleared.
unsafe fn retire_cache() {
    let generation = &mut *addr_of_mut!(CACHE_GENERATION);
    *generation = generation.wrapping_add(1);
    if *generation == 0 {
        *generation = 1;
    }
}

/// The entry of the node, or a free one. Null when the probes found none.
unsafe fn cache_slot(node: usize) -> *mut CacheEntry {
    if (*addr_of!(CACHE)).is_null() {
        let table = vec![CacheEntry { node: 0, generation: 0, stored: [0.0; 3], exact: [0.0; 3] }; CACHE_SLOTS].into_boxed_slice();
        *addr_of_mut!(CACHE) = Box::leak(table).as_mut_ptr();
    }
    let mut index = (((node >> 4) as u32).wrapping_mul(2654435761) as usize) & (CACHE_SLOTS - 1);
    for _ in 0..CACHE_PROBE_LIMIT {
        let entry = (*addr_of!(CACHE)).add(index);
        if (*entry).generation != *addr_of!(CACHE_GENERATION) || (*entry).node == node {
            return entry;
        }
        index = (index + 1) & (CACHE_SLOTS - 1);
    }
    core::ptr::null_mut()
}

unsafe fn exact_translation(node: usize, depth: u32) -> Option<[f64; 3]> {
    if node == 0 || depth > MAX_CHAIN_DEPTH {
        return None;
    }
    let stored: [f32; 3] = rd(node, AV_WORLD + T_TRANSLATION);
    let slot = cache_slot(node);
    if !slot.is_null() && (*slot).generation == *addr_of!(CACHE_GENERATION) && (*slot).node == node && (*slot).stored == stored {
        return Some((*slot).exact);
    }

    // A root, or a node that the game placed without its parent chain: the stored
    // translation is the start.
    let mut t = stored.map(f64::from);
    let parent: usize = rd(node, AV_PARENT);
    if parent != 0 && stored_follows_parent(node, parent) {
        if let Some(pt) = exact_translation(parent, depth + 1) {
            // world.t(child) = world.t(parent) + world.R(parent) * (local.t(child) * world.s(parent))
            let s = rd::<f32>(parent, AV_WORLD + T_SCALE) as f64;
            let l: [f32; 3] = rd(node, AV_LOCAL_TRANSLATE);
            let l = [l[0] as f64 * s, l[1] as f64 * s, l[2] as f64 * s];
            let m: [[f32; 3]; 3] = rd(parent, AV_WORLD);
            for i in 0..3 {
                t[i] = pt[i] + m[i][0] as f64 * l[0] + m[i][1] as f64 * l[1] + m[i][2] as f64 * l[2];
            }
        }
    }

    // The walk of the parent can have used this slot. Find it again.
    let slot = cache_slot(node);
    if !slot.is_null() {
        *slot = CacheEntry { node, generation: *addr_of!(CACHE_GENERATION), stored, exact: t };
    }
    Some(t)
}

/// The world transform of a node: the stored rotation and scale, the exact translation.
unsafe fn exact_transform(node: usize) -> DTransform {
    let mut out = from_ni(node + AV_WORLD);
    if let Some(exact) = exact_translation(node, 0) {
        out.t = exact;
    }
    out
}

// ---- the eye of the first person

/// In first person the game copies the stored position of the "Camera" node into the camera
/// roots. This is the copy together with the exact position of the node at that moment. It
/// is valid only in the frame in which the game made the copy.
struct EyePair {
    valid: bool,
    frame: u32,
    roots: [usize; 3],
    stored: [f32; 3],
    exact: [f64; 3],
}

static mut EYE: EyePair = EyePair { valid: false, frame: 0, roots: [0; 3], stored: [0.0; 3], exact: [0.0; 3] };
/// The number of the frame in progress. It goes up at the end of each frame.
static mut FRAME: u32 = 1;
static mut UPDATE_CAMERA_TRANSFORMS_ORIGINAL: usize = 0;

unsafe fn capture_eye(controller: usize) {
    let eye = &mut *addr_of_mut!(EYE);
    eye.valid = false;
    let head: usize = rd(controller, CONTROLLER_HEAD_CAMERA);
    let world_controller: usize = rd(WORLD_CONTROLLER, 0);
    if head == 0 || world_controller == 0 {
        return;
    }
    eye.roots = WC_CAMERA_ROOTS.map(|offset| rd(world_controller, offset));
    let world_root = eye.roots[0];
    if world_root == 0 {
        return;
    }
    let Some(exact) = exact_translation(head, 0) else { return };
    let head_stored: [f32; 3] = rd(head, AV_WORLD + T_TRANSLATION);
    eye.stored = rd(world_root, AV_LOCAL_TRANSLATE);
    for i in 0..3 {
        eye.exact[i] = exact[i] + (eye.stored[i] as f64 - head_stored[i] as f64);
    }
    eye.frame = *addr_of!(FRAME);
    eye.valid = true;
}

/// The detour of `PlayerAnimController::updateCameraTransforms`: the game function, then the
/// capture.
unsafe extern "fastcall" fn update_camera_transforms_detour(controller: usize, _edx: usize) {
    thiscall0(*addr_of!(UPDATE_CAMERA_TRANSFORMS_ORIGINAL), controller);
    capture_eye(controller);
}

/// True when the camera hangs from a root that the game wrote the copy into.
unsafe fn hangs_from_copied_root(camera: usize) -> bool {
    let eye = &*addr_of!(EYE);
    let mut node: usize = rd(camera, AV_PARENT);
    let mut depth = 0;
    while node != 0 && depth < EYE_MAX_PARENT_DEPTH {
        if eye.roots.contains(&node) {
            return true;
        }
        node = rd(node, AV_PARENT);
        depth += 1;
    }
    false
}

// ---- the state of the scene

/// The module compares its matrices with the matrices of the hooks of the host.
static COMPARE: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// The origin of the scene in progress: the exact position of its camera. `None` when the
/// camera is not the world camera or the arm camera.
static mut ORIGIN: Option<[f64; 3]> = None;
/// The comparison of the origin of this scene is done.
static mut ORIGIN_CHECKED: bool = false;
/// The transform of the geometry that is in `RenderShape` or `RenderTriStrips`.
static mut GEOMETRY_TRANSFORM: usize = 0;
/// The world matrix that goes to the device now is relative to the camera.
static mut PENDING: bool = false;

/// origin equal, origin different, rigid equal, rigid different, skinned equal, skinned
/// different, rigid made, skinned made.
static mut COUNTS: [u32; 8] = [0; 8];

unsafe fn count(index: usize) {
    (*addr_of_mut!(COUNTS))[index] += 1;
}

/// Turns the comparison on or off. Puts the detour for the eye in place at the first use.
pub unsafe fn set_compare(on: bool) -> Result<(), String> {
    install()?;
    COMPARE.store(on, Ordering::Relaxed);
    Ok(())
}

/// Puts the detour for the eye in place. The other steps need no hook: the draw ports call
/// this module.
pub unsafe fn install() -> Result<(), String> {
    if INSTALLED.load(Ordering::Relaxed) {
        return Ok(());
    }
    let detour = update_camera_transforms_detour as *const () as usize;
    let original = hook::install_raw(UPDATE_CAMERA_TRANSFORMS, &UPDATE_CAMERA_TRANSFORMS_HEAD, &[], detour, 0, Cleanup::Callee)?;
    *addr_of_mut!(UPDATE_CAMERA_TRANSFORMS_ORIGINAL) = original;
    INSTALLED.store(true, Ordering::Relaxed);
    Ok(())
}

fn own() -> bool {
    INSTALLED.load(Ordering::Relaxed) && crate::link_client::client_camera_space() && crate::shadow::leg() != crate::shadow::Leg::Rust
}

fn compare() -> bool {
    COMPARE.load(Ordering::Relaxed) && !crate::link_client::client_camera_space() && crate::shadow::leg() != crate::shadow::Leg::Rust
}

/// True when the world matrix that goes to the device now is relative to the camera.
#[inline]
pub fn pending() -> bool {
    unsafe { *addr_of!(PENDING) }
}

/// A scene starts with the camera whose world translation is at `world_location`. Returns
/// the origin for the scene packet: the exact position of the camera, or `None` when the
/// scene of this camera is not relative.
pub unsafe fn scene_begin(world_location: usize) -> Option<[f64; 3]> {
    *addr_of_mut!(ORIGIN) = None;
    *addr_of_mut!(ORIGIN_CHECKED) = false;
    if !INSTALLED.load(Ordering::Relaxed) || world_location == 0 {
        return None;
    }
    // `NiCamera::Click` gives the address of the world translation of the camera.
    let camera = world_location - AV_WORLD - T_TRANSLATION;
    let world_controller: usize = rd(WORLD_CONTROLLER, 0);
    if world_controller == 0 || !WC_OWNED_CAMERAS.iter().any(|&offset| rd::<usize>(world_controller, offset) == camera) {
        return None;
    }
    let location: [f32; 3] = rd(world_location, 0);
    let eye = &*addr_of!(EYE);
    let exact = if eye.valid && eye.frame == *addr_of!(FRAME) && hangs_from_copied_root(camera) {
        Some([0, 1, 2].map(|i| eye.exact[i] + (location[i] as f64 - eye.stored[i] as f64)))
    } else {
        exact_translation(camera, 0)
    };
    let origin = exact.unwrap_or(location.map(f64::from));
    // The entries of the scene before can describe a pose that the game changed.
    retire_cache();
    *addr_of_mut!(ORIGIN) = Some(origin);
    Some(origin)
}

/// The frame ends.
pub unsafe fn frame_end() {
    *addr_of_mut!(FRAME) = (*addr_of!(FRAME)).wrapping_add(1);
    retire_cache();
}

/// `RenderShape` or `RenderTriStrips` starts for the geometry with this world transform. 0
/// for a draw that is not of a geometry.
pub unsafe fn geometry_begin(transform: usize) {
    *addr_of_mut!(GEOMETRY_TRANSFORM) = transform;
}

/// The origin of the scene, when the host has the scene in camera space.
unsafe fn active_origin() -> Option<[f64; 3]> {
    let origin = (*addr_of!(ORIGIN))?;
    let mut host_origin = [0f64; 3];
    if !crate::link_client::host_camera_space(&mut host_origin) {
        return None;
    }
    if !*addr_of!(ORIGIN_CHECKED) {
        *addr_of_mut!(ORIGIN_CHECKED) = true;
        count(if host_origin.map(f64::to_bits) == origin.map(f64::to_bits) { 0 } else { 1 });
    }
    Some(origin)
}

// ---- rigid draws

/// The transform of a geometry with a translation relative to the origin.
unsafe fn rigid_relative(transform: usize, origin: &[f64; 3]) -> Option<[u32; T_WORDS]> {
    let exact = exact_translation(transform - AV_WORLD, 0)?;
    let mut relative: [u32; T_WORDS] = rd(transform, 0);
    for i in 0..3 {
        relative[T_TRANSLATION / 4 + i] = ((exact[i] - origin[i]) as f32).to_bits();
    }
    Some(relative)
}

/// Called in place of the call of `SetModelTransform` at a draw site. Returns true when this
/// module set the world matrix. On false the caller makes the call of the game.
/// `geometry_site`: the site is in a draw of a geometry (`DrawPrimitive`).
pub unsafe fn take_model_transform(this: usize, transform: usize, geometry_site: bool) -> bool {
    if !own() || !geometry_site || transform == 0 || transform != *addr_of!(GEOMETRY_TRANSFORM) {
        return false;
    }
    let Some(origin) = active_origin() else { return false };
    let Some(relative) = rigid_relative(transform, &origin) else { return false };
    *addr_of_mut!(PENDING) = true;
    renderer_frame::set_model_transform(this, relative.as_ptr() as usize);
    *addr_of_mut!(PENDING) = false;
    count(6);
    true
}

/// Called after the call of `SetModelTransform` at a draw site. In the compare mode it
/// counts if the matrix of the host is the matrix of this module.
pub unsafe fn check_model_transform(this: usize, transform: usize, geometry_site: bool) {
    if !compare() {
        return;
    }
    let Some(origin) = active_origin() else { return };
    let mine = if geometry_site && transform != 0 && transform == *addr_of!(GEOMETRY_TRANSFORM) {
        rigid_relative(transform, &origin)
    } else {
        None
    };
    let host_relative = crate::link_client::world_relative(0);
    let equal = match mine {
        Some(relative) => {
            // The translation is the last row of the world matrix.
            let row: [u32; 3] = rd(this, R::currentWorldMatrix + 48);
            host_relative && row == [relative[9], relative[10], relative[11]]
        }
        None => !host_relative,
    };
    count(if equal { 2 } else { 3 });
}

// ---- skinned draws

/// Makes the transform of each bone of a partition, relative to the origin, and gives it to
/// `each` with the index of the bone in the partition. Returns false, with no call of
/// `each`, when the data of the skin is not complete.
///
/// transform(i) = shape * rootParentToSkin * inverse(rootParent) * bone * boneOffset
unsafe fn skinned_relative(
    skin: usize,
    partition: usize,
    transform: usize,
    geometry_site: bool,
    origin: &[f64; 3],
    mut each: impl FnMut(u32, &[u32; T_WORDS]),
) -> bool {
    if skin == 0 || partition == 0 || transform == 0 {
        return false;
    }
    let data: usize = rd(skin, SKIN_DATA);
    let root_parent: usize = rd(skin, SKIN_ROOT_PARENT);
    let bones: usize = rd(skin, SKIN_BONES);
    let partition_bones: usize = rd(partition, PARTITION_BONES);
    if data == 0 || root_parent == 0 || bones == 0 || partition_bones == 0 {
        return false;
    }
    let bone_records: usize = rd(data, DATA_BONES);
    if bone_records == 0 {
        return false;
    }
    let shape = if geometry_site && transform == *addr_of!(GEOMETRY_TRANSFORM) { exact_transform(transform - AV_WORLD) } else { from_ni(transform) };
    let Some(inverse_root_parent) = invert(&exact_transform(root_parent)) else { return false };
    let count: u16 = rd(partition, PARTITION_BONE_COUNT);
    let bone_count: u32 = rd(data, DATA_BONE_COUNT);
    for i in 0..count as usize {
        let index = rd::<u16>(partition_bones, i * 2) as usize;
        if index as u32 >= bone_count || rd::<usize>(bones, index * 4) == 0 {
            return false;
        }
    }

    let skin_to_root = combine(&combine(&shape, &from_ni(data + DATA_TRANSFORM)), &inverse_root_parent);
    for i in 0..count as usize {
        let index = rd::<u16>(partition_bones, i * 2) as usize;
        let bone = exact_transform(rd(bones, index * 4));
        let palette = combine(&combine(&skin_to_root, &bone), &from_ni(bone_records + index * BONE_RECORD_SIZE));
        let mut relative = [0u32; T_WORDS];
        for r in 0..3 {
            for c in 0..3 {
                relative[r * 3 + c] = (palette.r[r][c] as f32).to_bits();
            }
            relative[T_TRANSLATION / 4 + r] = ((palette.t[r] - origin[r]) as f32).to_bits();
        }
        relative[T_SCALE / 4] = (palette.s as f32).to_bits();
        each(i as u32, &relative);
    }
    true
}

/// Called in place of the call of `SetSkinnedModelTransforms` at a draw site. Returns true
/// when this module set the world matrices of the bones.
pub unsafe fn take_skinned_transforms(this: usize, skin: usize, partition: usize, transform: usize, geometry_site: bool) -> bool {
    if !own() {
        return false;
    }
    let Some(origin) = active_origin() else { return false };
    let made = skinned_relative(skin, partition, transform, geometry_site, &origin, |index, relative| {
        *addr_of_mut!(PENDING) = true;
        renderer_frame::set_bone_transform(this, relative.as_ptr() as usize, index);
        *addr_of_mut!(PENDING) = false;
    });
    if made {
        renderer_frame::update_model_camera(this, transform);
        count(7);
    }
    made
}

/// Called after the call of `SetSkinnedModelTransforms` at a draw site. In the compare mode
/// it counts if the first four matrices of the host are the matrices of this module.
pub unsafe fn check_skinned_transforms(skin: usize, partition: usize, transform: usize, geometry_site: bool) {
    if !compare() {
        return;
    }
    let Some(origin) = active_origin() else { return };
    let mut equal = true;
    let made = skinned_relative(skin, partition, transform, geometry_site, &origin, |index, relative| {
        if index < 4 {
            // `write_world_matrix` writes only the world matrix of a renderer.
            let mut matrix = [0u32; 16];
            renderer_frame::write_world_matrix((matrix.as_mut_ptr() as usize).wrapping_sub(R::currentWorldMatrix), relative.as_ptr() as usize);
            equal = equal && crate::link_client::world_relative(index) && crate::link_client::world_matrix(index) == matrix;
        }
    });
    if !made {
        // The host must have used the matrices of the game.
        equal = !crate::link_client::world_relative(0);
    }
    count(if equal { 4 } else { 5 });
}

/// The counters as one line of text.
pub unsafe fn report(reset: bool) -> String {
    let counts = &mut *addr_of_mut!(COUNTS);
    let text = format!(
        "camera_space: origin_equal {} origin_different {} rigid_equal {} rigid_different {} skinned_equal {} skinned_different {} rigid_made {} skinned_made {}",
        counts[0], counts[1], counts[2], counts[3], counts[4], counts[5], counts[6], counts[7]
    );
    if reset {
        *counts = [0; 8];
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_step_is_one_unit_in_the_last_place() {
        assert_eq!(float_step(1.0), 2f32.powi(-23));
        assert_eq!(float_step(100000.0), 2f32.powi(17 - 24));
        assert_eq!(float_step(0.0), 2f32.powi(-24));
        assert!(within_guard(100000.0, 100000.1));
        assert!(!within_guard(100000.0, 100001.0));
    }

    #[test]
    fn combine_and_invert_agree() {
        let a = DTransform { r: [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]], t: [10.0, 20.0, 30.0], s: 2.0 };
        let identity = combine(&a, &invert(&a).unwrap());
        for i in 0..3 {
            assert!(identity.t[i].abs() < 1e-12);
            for j in 0..3 {
                assert!((identity.r[i][j] - if i == j { 1.0 } else { 0.0 }).abs() < 1e-12);
            }
        }
        assert!((identity.s - 1.0).abs() < 1e-12);
        assert!(invert(&DTransform { s: 0.0, ..a }).is_none());
    }

    #[test]
    fn a_child_gets_the_exact_sum() {
        // Two nodes: a parent far from the origin and a child with a small local translation.
        let mut parent = [0u8; 0x90];
        let mut child = [0u8; 0x90];
        unsafe {
            let p = parent.as_mut_ptr() as usize;
            let c = child.as_mut_ptr() as usize;
            let identity: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
            crate::mem::wr(p, AV_WORLD, identity);
            crate::mem::wr(p, AV_WORLD + T_TRANSLATION, [400000.0f32, 0.0, 0.0]);
            crate::mem::wr(p, AV_WORLD + T_SCALE, 1.0f32);
            crate::mem::wr(c, AV_PARENT, p);
            crate::mem::wr(c, AV_LOCAL_TRANSLATE, [0.01f32, 0.0, 0.0]);
            // The game stores the rounded sum.
            crate::mem::wr(c, AV_WORLD + T_TRANSLATION, [400000.0f32 + 0.01f32, 0.0, 0.0]);
            retire_cache();
            let exact = exact_translation(c, 0).unwrap();
            assert_eq!(exact[0], 400000.0f64 + 0.01f32 as f64);
            // A child that the game placed in another way keeps its stored translation.
            crate::mem::wr(c, AV_WORLD + T_TRANSLATION, [5.0f32, 0.0, 0.0]);
            assert_eq!(exact_translation(c, 0).unwrap()[0], 5.0);
        }
    }
}
