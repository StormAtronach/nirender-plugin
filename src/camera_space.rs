//! World matrices relative to the camera, made with 64-bit arithmetic from the scene graph.
//!
//! This is a port of `camerarelative.cpp` of MGE XE. Keep the same steps in the same order:
//! the results must be equal bit for bit. It has two modes:
//!
//! - Compare: the hooks of MGE XE make the matrices. This module makes them too and counts
//!   the differences.
//! - Own: this module makes the matrices and MGE XE does not. The fact
//!   `FACT_CLIENT_CAMERA_SPACE` of the render link turns it on.
//!
//! Only the render thread uses the state of this module.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::engine::WORLD_CONTROLLER;
use crate::gen::layouts::{NiAVObject, NiBoneData, NiDX8Renderer as R, NiSkinData, NiSkinInstance, NiSkinPartition_Partition, NiTransform, PlayerAnimController, WorldController};
use crate::main_thread::{MainCell, MainThread};
use crate::mem::rd;
use crate::ports::renderer_frame;

// `NiTransform`: a row-major rotation, a translation and a scale.
const T_TRANSLATION: usize = NiTransform::translation;
const T_SCALE: usize = NiTransform::scale;
const T_WORDS: usize = 13;
const AV_PARENT: usize = NiAVObject::parentNode;
const AV_LOCAL_TRANSLATE: usize = NiAVObject::localTransform_localTranslate;
const AV_WORLD: usize = NiAVObject::worldTransform;
const SKIN_DATA: usize = NiSkinInstance::skinData;
const SKIN_ROOT_PARENT: usize = NiSkinInstance::rootParent;
const SKIN_BONES: usize = NiSkinInstance::bones;
// `NiSkinData`: the transform from the root parent to the skin, and the bone records.
const DATA_TRANSFORM: usize = NiSkinData::transformRootParentToSkin;
const DATA_BONE_COUNT: usize = NiSkinData::numBones;
const DATA_BONES: usize = NiSkinData::boneData;
const BONE_RECORD_SIZE: usize = NiBoneData::SIZE;
const PARTITION_BONES: usize = NiSkinPartition_Partition::bones;
const PARTITION_BONE_COUNT: usize = NiSkinPartition_Partition::numBones;
/// `WorldController`: the roots that the game copies the eye of the first person into (world,
/// arm and shadow camera).
const WC_CAMERA_ROOTS: [usize; 3] = [WorldController::worldCamera_sgCameraRoot, WorldController::armCamera_sgCameraRoot, WorldController::shadowCamera_sgCameraRoot];
/// `WorldController`: the world camera and the arm camera. Their scenes are relative.
const WC_OWNED_CAMERAS: [usize; 2] = [WorldController::worldCamera_cameraData_camera, WorldController::armCamera_cameraData_camera];
/// `PlayerAnimController`: the "Camera" node of the first-person model.
const CONTROLLER_HEAD_CAMERA: usize = PlayerAnimController::firstPersonHeadCamera;

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
/// Made from the exponent bits: `powi` calls the CRT's `pow` on i686, which is too slow here.
/// A float `1.f * 2^(E-127)` is `0.1f * 2^(E-126)`, so the step is `2^(E-150)`.
fn float_step(magnitude: f32) -> f32 {
    let biased = ((magnitude.to_bits() >> 23) & 0xFF) as i32;
    if magnitude == 0.0 || !magnitude.is_finite() {
        // frexp of 0 gives the exponent 0: the step 2^-24.
        return f32::from_bits(0x3380_0000);
    }
    if biased == 0 {
        // A subnormal: rare; the loop of the exponent.
        return float_step_by_loop(magnitude);
    }
    let power = biased - 150;
    if power >= -126 {
        f32::from_bits(((power + 127) as u32) << 23)
    } else {
        f32::from_bits(1u32 << (power + 149))
    }
}

/// `float_step` by halving and doubling: the step of a subnormal, and the reference of the test.
fn float_step_by_loop(magnitude: f32) -> f32 {
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
    (2f64.powi(exponent - 24)) as f32
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

static CACHE: MainCell<*mut CacheEntry> = MainCell::new(core::ptr::null_mut());
static CACHE_GENERATION: MainCell<u32> = MainCell::new(1);

/// Makes all cache entries old. The table is not cleared.
unsafe fn retire_cache(t: &MainThread) {
    let generation = &mut *CACHE_GENERATION.get(t);
    *generation = generation.wrapping_add(1);
    if *generation == 0 {
        *generation = 1;
    }
}

/// The entry of the node, or a free one. Null when the probes found none.
unsafe fn cache_slot(t: &MainThread, node: usize) -> *mut CacheEntry {
    if (*CACHE.get(t)).is_null() {
        let table = vec![CacheEntry { node: 0, generation: 0, stored: [0.0; 3], exact: [0.0; 3] }; CACHE_SLOTS].into_boxed_slice();
        *CACHE.get(t) = Box::leak(table).as_mut_ptr();
    }
    let mut index = (((node >> 4) as u32).wrapping_mul(2654435761) as usize) & (CACHE_SLOTS - 1);
    for _ in 0..CACHE_PROBE_LIMIT {
        let entry = (*CACHE.get(t)).add(index);
        if (*entry).generation != *CACHE_GENERATION.get(t) || (*entry).node == node {
            return entry;
        }
        index = (index + 1) & (CACHE_SLOTS - 1);
    }
    core::ptr::null_mut()
}

pub(crate) unsafe fn exact_translation(t: &MainThread, node: usize, depth: u32) -> Option<[f64; 3]> {
    if node == 0 || depth > MAX_CHAIN_DEPTH {
        return None;
    }
    let stored: [f32; 3] = rd(node, AV_WORLD + T_TRANSLATION);
    let slot = cache_slot(t, node);
    if !slot.is_null() && (*slot).generation == *CACHE_GENERATION.get(t) && (*slot).node == node && (*slot).stored == stored {
        return Some((*slot).exact);
    }

    // A root, or a node that the game placed without its parent chain: the stored
    // translation is the start.
    let mut exact = stored.map(f64::from);
    let parent: usize = rd(node, AV_PARENT);
    if parent != 0 && stored_follows_parent(node, parent) {
        if let Some(pt) = exact_translation(t, parent, depth + 1) {
            // world.t(child) = world.t(parent) + world.R(parent) * (local.t(child) * world.s(parent))
            let s = rd::<f32>(parent, AV_WORLD + T_SCALE) as f64;
            let l: [f32; 3] = rd(node, AV_LOCAL_TRANSLATE);
            let l = [l[0] as f64 * s, l[1] as f64 * s, l[2] as f64 * s];
            let m: [[f32; 3]; 3] = rd(parent, AV_WORLD);
            for i in 0..3 {
                exact[i] = pt[i] + m[i][0] as f64 * l[0] + m[i][1] as f64 * l[1] + m[i][2] as f64 * l[2];
            }
        }
    }

    // The walk of the parent can have used this slot. Find it again.
    let slot = cache_slot(t, node);
    if !slot.is_null() {
        *slot = CacheEntry { node, generation: *CACHE_GENERATION.get(t), stored, exact };
    }
    Some(exact)
}

/// The world transform of a node: the stored rotation and scale, the exact translation.
unsafe fn exact_transform(t: &MainThread, node: usize) -> DTransform {
    let mut out = from_ni(node + AV_WORLD);
    if let Some(exact) = exact_translation(t, node, 0) {
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

static EYE: MainCell<EyePair> = MainCell::new(EyePair { valid: false, frame: 0, roots: [0; 3], stored: [0.0; 3], exact: [0.0; 3] });
/// The number of the frame in progress. It goes up at the end of each frame.
static FRAME: MainCell<u32> = MainCell::new(1);

unsafe fn capture_eye(controller: usize) {
    // The engine updates its camera on the main thread; a call of another thread is counted.
    let Some(t) = MainThread::here() else {
        crate::sync::off_thread("camera space", "camera update");
        return;
    };
    let t = &t;
    let eye = &mut *EYE.get(t);
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
    let Some(exact) = exact_translation(t, head, 0) else { return };
    let head_stored: [f32; 3] = rd(head, AV_WORLD + T_TRANSLATION);
    eye.stored = rd(world_root, AV_LOCAL_TRANSLATE);
    for i in 0..3 {
        eye.exact[i] = exact[i] + (eye.stored[i] as f64 - head_stored[i] as f64);
    }
    eye.frame = *FRAME.get(t);
    eye.valid = true;
}

/// The listener of MWSE's event "the camera transforms updated" (0x542E60
/// `PlayerAnimController::updateCameraTransforms` returned): the capture.
unsafe extern "C" fn camera_transforms_updated(_user: *mut core::ffi::c_void, packet: *const core::ffi::c_void) {
    let packet = &*(packet as *const crate::mwse_native::CameraTransformsUpdatedV1);
    capture_eye(packet.player_animation_controller as usize);
}

/// True when the camera hangs from a root that the game wrote the copy into.
unsafe fn hangs_from_copied_root(eye: &EyePair, camera: usize) -> bool {
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
static INSTALLED: crate::sync::InstallState = crate::sync::InstallState::new();
/// The origin of the scene in progress: the exact position of its camera. [`None`] when the
/// camera is not the world camera or the arm camera.
static ORIGIN: MainCell<Option<[f64; 3]>> = MainCell::new(None);
/// The comparison of the origin of this scene is done.
static ORIGIN_CHECKED: MainCell<bool> = MainCell::new(false);
/// The transform of the geometry that is in `RenderShape` or `RenderTriStrips`.
static GEOMETRY_TRANSFORM: MainCell<usize> = MainCell::new(0);
/// The world matrix that goes to the device now is relative to the camera.
static PENDING: MainCell<bool> = MainCell::new(false);

/// origin equal, origin different, rigid equal, rigid different, skinned equal, skinned
/// different, rigid made, skinned made.
static COUNTS: MainCell<[u32; 8]> = MainCell::new([0; 8]);

unsafe fn count(t: &MainThread, index: usize) {
    (*COUNTS.get(t))[index] += 1;
}

/// Turns the comparison on or off. Listens for the eye at the first use.
///
/// # Safety
///
/// As for [`install`].
pub unsafe fn set_compare(on: bool) -> Result<(), String> {
    install()?;
    COMPARE.store(on, Ordering::Relaxed);
    Ok(())
}

/// Listens to MWSE's event for the eye. The other steps need no hook: the draw ports call
/// this module.
///
/// # Safety
///
/// No condition of its own: `INSTALLED` (a `sync::InstallState`) lets one caller install at a
/// time. It is `unsafe` because of the calls of MWSE's table.
pub unsafe fn install() -> Result<(), String> {
    if !INSTALLED.begin() {
        return Ok(());
    }
    let result = crate::mwse_native::subscribe(crate::mwse_native::NATIVE_EVENT_CAMERA_TRANSFORMS_UPDATED, camera_transforms_updated);
    INSTALLED.finish(result.is_ok());
    result.map(|_| ())
}

pub(crate) fn own() -> bool {
    INSTALLED.is_installed() && crate::link_client::client_camera_space() && crate::shadow::leg() != crate::shadow::Leg::Rust
}

fn compare() -> bool {
    COMPARE.load(Ordering::Relaxed) && !crate::link_client::client_camera_space() && crate::shadow::leg() != crate::shadow::Leg::Rust
}

/// True when the world matrix that goes to the device now is relative to the camera.
#[inline]
pub fn pending(t: &MainThread) -> bool {
    *PENDING.get(t)
}

/// A scene starts with the camera whose world translation is at `world_location`. Returns
/// the origin for the scene packet: the exact position of the camera, or [`None`] when the
/// scene of this camera is not relative.
///
/// # Safety
///
/// `world_location` is 0 or the address of the world translation of a live `NiCamera`, as
/// `NiCamera::Click` gives it; the cameras of the world controller and their parent chains are
/// live.
pub unsafe fn scene_begin(t: &MainThread, world_location: usize) -> Option<[f64; 3]> {
    *ORIGIN.get(t) = None;
    *ORIGIN_CHECKED.get(t) = false;
    if !INSTALLED.is_installed() || world_location == 0 {
        return None;
    }
    // `NiCamera::Click` gives the address of the world translation of the camera.
    let camera = world_location - AV_WORLD - T_TRANSLATION;
    let world_controller: usize = rd(WORLD_CONTROLLER, 0);
    if world_controller == 0 || !WC_OWNED_CAMERAS.iter().any(|&offset| rd::<usize>(world_controller, offset) == camera) {
        return None;
    }
    let location: [f32; 3] = rd(world_location, 0);
    let eye = &*EYE.get(t);
    let exact = if eye.valid && eye.frame == *FRAME.get(t) && hangs_from_copied_root(eye, camera) {
        Some([0, 1, 2].map(|i| eye.exact[i] + (location[i] as f64 - eye.stored[i] as f64)))
    } else {
        exact_translation(t, camera, 0)
    };
    let origin = exact.unwrap_or(location.map(f64::from));
    // The entries of the scene before can describe a pose that the game changed.
    retire_cache(t);
    *ORIGIN.get(t) = Some(origin);
    Some(origin)
}

/// The number of the frame in progress (it wraps).
pub fn frame(t: &MainThread) -> u32 {
    *FRAME.get(t)
}

/// The frame ends.
///
/// # Safety
///
/// No condition of its own: the function reads no engine memory. `t` is the proof of the main
/// thread.
pub unsafe fn frame_end(t: &MainThread) {
    let mut frame = FRAME.get(t);
    *frame = frame.wrapping_add(1);
    drop(frame);
    retire_cache(t);
}

/// `RenderShape` or `RenderTriStrips` starts for the geometry with this world transform. 0
/// for a draw that is not of a geometry.
///
/// # Safety
///
/// No condition of its own: the function reads no engine memory. `transform` is kept as a number
/// and not read here.
pub unsafe fn geometry_begin(t: &MainThread, transform: usize) {
    *GEOMETRY_TRANSFORM.get(t) = transform;
}

/// The origin of the scene, when the host has the scene in camera space.
unsafe fn active_origin(t: &MainThread) -> Option<[f64; 3]> {
    let origin = (*ORIGIN.get(t))?;
    let mut host_origin = [0f64; 3];
    if !crate::link_client::host_camera_space(&mut host_origin) {
        return None;
    }
    if !*ORIGIN_CHECKED.get(t) {
        *ORIGIN_CHECKED.get(t) = true;
        count(t, if host_origin.map(f64::to_bits) == origin.map(f64::to_bits) { 0 } else { 1 });
    }
    Some(origin)
}

// ---- rigid draws

/// The transform of a geometry with a translation relative to the origin.
unsafe fn rigid_relative(t: &MainThread, transform: usize, origin: &[f64; 3]) -> Option<[u32; T_WORDS]> {
    let exact = exact_translation(t, transform - AV_WORLD, 0)?;
    let mut relative: [u32; T_WORDS] = rd(transform, 0);
    for i in 0..3 {
        relative[T_TRANSLATION / 4 + i] = ((exact[i] - origin[i]) as f32).to_bits();
    }
    Some(relative)
}

/// Called in place of the call of `SetModelTransform` at a draw site. Returns true when this
/// module set the world matrix. On false the caller makes the call of the game.
/// `geometry_site`: the site is in a draw of a geometry (`DrawPrimitive`).
///
/// # Safety
///
/// `this` points at the live `NiDX8Renderer` with its device, and `transform` is 0 or the world
/// transform of a live `NiAVObject` whose parent chain is live.
pub unsafe fn take_model_transform(t: &MainThread, this: usize, transform: usize, geometry_site: bool) -> bool {
    if !own() || !geometry_site || transform == 0 || transform != *GEOMETRY_TRANSFORM.get(t) {
        return false;
    }
    let Some(origin) = active_origin(t) else { return false };
    let Some(relative) = rigid_relative(t, transform, &origin) else { return false };
    *PENDING.get(t) = true;
    renderer_frame::set_model_transform(this, relative.as_ptr() as usize);
    *PENDING.get(t) = false;
    count(t, 6);
    true
}

/// Called after the call of `SetModelTransform` at a draw site. In the compare mode it
/// counts if the matrix of the host is the matrix of this module.
///
/// # Safety
///
/// `this` points at the live `NiDX8Renderer`, and `transform` is 0 or the world transform of a live
/// `NiAVObject` whose parent chain is live.
pub unsafe fn check_model_transform(t: &MainThread, this: usize, transform: usize, geometry_site: bool) {
    if !compare() {
        return;
    }
    let Some(origin) = active_origin(t) else { return };
    let mine = if geometry_site && transform != 0 && transform == *GEOMETRY_TRANSFORM.get(t) {
        rigid_relative(t, transform, &origin)
    } else {
        None
    };
    let host_relative = crate::link_client::world_relative(t, 0);
    let equal = match mine {
        Some(relative) => {
            // The translation is the last row of the world matrix.
            let row: [u32; 3] = rd(this, R::currentWorldMatrix + 48);
            host_relative && row == [relative[9], relative[10], relative[11]]
        }
        None => !host_relative,
    };
    count(t, if equal { 2 } else { 3 });
}

// ---- skinned draws

/// Makes the transform of each bone of a partition, relative to the origin, and gives it to
/// `each` with the index of the bone in the partition. Returns false, with no call of
/// `each`, when the data of the skin is not complete.
///
/// transform(i) = shape * rootParentToSkin * inverse(rootParent) * bone * boneOffset
unsafe fn skinned_relative(t: &MainThread, 
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
    let shape = if geometry_site && transform == *GEOMETRY_TRANSFORM.get(t) { exact_transform(t, transform - AV_WORLD) } else { from_ni(transform) };
    let Some(inverse_root_parent) = invert(&exact_transform(t, root_parent)) else { return false };
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
        let bone = exact_transform(t, rd(bones, index * 4));
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

/// The palette of a skin for the shape record: the transform of each bone of the skin as a
/// world matrix of the device (as `write_world_matrix` makes one), relative to the origin
/// when this module owns the matrices. Returns whether the matrices are relative, or what
/// is missing when the data of the skin is not complete.
///
/// # Safety
///
/// `skin` is 0 or a live `NiSkinInstance` with its skin data and its bones, and `transform` is 0 or
/// the world transform of a live `NiAVObject` whose parent chain is live.
pub unsafe fn skin_palette(t: &MainThread, skin: usize, transform: usize, geometry_site: bool, out: &mut Vec<[f32; 16]>) -> Result<bool, &'static str> {
    let start = crate::capture::ticks();
    let result = build_skin_palette(t, skin, transform, geometry_site, out);
    if result.is_ok() {
        note_skin(t, skin, out.len(), crate::capture::ticks() - start);
    }
    result
}

/// The measure of the skinned parts: the parts, the bone matrices built, the bones and the
/// skeletons (root parents) that are new in their frame, and the ticks of the palettes.
#[derive(Default)]
struct SkinMeasure {
    parts: u64,
    matrices: u64,
    bones: u64,
    skeletons: u64,
    ticks: i64,
    /// The bones and skeletons of the frame `seen_frame`. Cleared at a new frame: the counts
    /// ask only for the frame's own, and a map kept for the session grew with every bone the
    /// game ever made (the leak hunt of 2026-10-23, UMDH: 590 KB after an hour).
    seen: Option<crate::ptr_hash::HashMap<usize, u32>>,
    seen_frame: u32,
}

static SKIN_MEASURE: MainCell<SkinMeasure> = MainCell::new(SkinMeasure { parts: 0, matrices: 0, bones: 0, skeletons: 0, ticks: 0, seen: None, seen_frame: 0 });

unsafe fn note_skin(t: &MainThread, skin: usize, matrices: usize, ticks: i64) {
    let measure = &mut *SKIN_MEASURE.get(t);
    let frame = *FRAME.get(t);
    measure.parts += 1;
    measure.matrices += matrices as u64;
    measure.ticks += ticks;
    let seen = measure.seen.get_or_insert_with(Default::default);
    if measure.seen_frame != frame {
        seen.clear();
        measure.seen_frame = frame;
    }
    let root_parent: usize = rd(skin, SKIN_ROOT_PARENT);
    if seen.insert(root_parent | 1, frame) != Some(frame) {
        measure.skeletons += 1;
    }
    let bones: usize = rd(skin, SKIN_BONES);
    for i in 0..matrices {
        if seen.insert(rd::<usize>(bones, i * 4), frame) != Some(frame) {
            measure.bones += 1;
        }
    }
}

/// # Safety
///
/// No condition of its own: the function reads no engine memory. `t` is the proof of the main
/// thread.
pub unsafe fn skin_report(t: &MainThread) -> String {
    let measure = &*SKIN_MEASURE.get(t);
    let ms = measure.ticks as f64 * 1000.0 / crate::capture::ticks_per_second() as f64;
    format!(
        "skins: {} parts, {} bone matrices, {} bones and {} skeletons new in their frame, the palettes {:.3} ms",
        measure.parts, measure.matrices, measure.bones, measure.skeletons, ms
    )
}

unsafe fn build_skin_palette(t: &MainThread, skin: usize, transform: usize, geometry_site: bool, out: &mut Vec<[f32; 16]>) -> Result<bool, &'static str> {
    out.clear();
    if transform == 0 {
        return Err("no skin instance or no transform");
    }
    let origin = if own() { active_origin(t) } else { None };
    let relative = origin.is_some();
    let origin = origin.unwrap_or([0.0; 3]);
    let shape = if geometry_site && transform == *GEOMETRY_TRANSFORM.get(t) { exact_transform(t, transform - AV_WORLD) } else { from_ni(transform) };
    skin_transforms(t, skin, &shape, |palette| {
        let scale = palette.s as f32;
        let mut matrix = [0f32; 16];
        for c in 0..3 {
            for r in 0..3 {
                matrix[c * 4 + r] = (palette.r[r][c] as f32) * scale;
            }
            matrix[12 + c] = (palette.t[c] - origin[c]) as f32;
        }
        matrix[15] = 1.0;
        out.push(matrix);
    })?;
    Ok(relative)
}

/// The bytes of one bone of `skin_bones`: 9 floats and 3 doubles.
pub const BONE_BYTES: usize = 9 * 4 + 3 * 8;

/// The bones of a skinned shape for the record `INSTANCE_BONES`: for each bone of the skin
/// the matrix of its rotation and scale (9 floats, by columns, as the palette has them) and
/// its exact translation in the world (3 doubles), from the shape's exact transform. Returns
/// the number of bones, or what is missing when the data of the skin is not complete. A shape
/// of 0 is missing data too: the function reads nothing through it (not the game's code).
///
/// # Safety
///
/// `shape` is 0 or points at a live `NiAVObject` whose parent chain is live. `skin` is 0 or a
/// live `NiSkinInstance` with its skin data and its bones.
pub unsafe fn skin_bones(t: &MainThread, shape: usize, skin: usize, out: &mut Vec<u8>) -> Result<u32, &'static str> {
    let start = crate::capture::ticks();
    out.clear();
    if shape == 0 {
        return Err("no shape");
    }
    let mut count = 0u32;
    skin_transforms(t, skin, &exact_transform(t, shape), |palette| {
        let scale = palette.s as f32;
        for c in 0..3 {
            for r in 0..3 {
                out.extend_from_slice(&((palette.r[r][c] as f32) * scale).to_le_bytes());
            }
        }
        for c in 0..3 {
            out.extend_from_slice(&palette.t[c].to_le_bytes());
        }
        count += 1;
    })?;
    note_skin(t, skin, count as usize, crate::capture::ticks() - start);
    Ok(count)
}

/// The world transform of each bone of a skin for a draw of its shape, whose world transform
/// is `shape`: the skin's transform to its root parent, the bone, the bone's transform from
/// the skin. Or what is missing when the data of the skin is not complete.
unsafe fn skin_transforms(t: &MainThread, skin: usize, shape: &DTransform, mut each: impl FnMut(&DTransform)) -> Result<(), &'static str> {
    if skin == 0 {
        return Err("no skin instance or no transform");
    }
    let data: usize = rd(skin, SKIN_DATA);
    let root_parent: usize = rd(skin, SKIN_ROOT_PARENT);
    let bones: usize = rd(skin, SKIN_BONES);
    if data == 0 {
        return Err("the skin instance has no skin data");
    }
    if root_parent == 0 {
        return Err("the skin instance has no root parent");
    }
    if bones == 0 {
        return Err("the skin instance has no bones");
    }
    let bone_records: usize = rd(data, DATA_BONES);
    let bone_count: u32 = rd(data, DATA_BONE_COUNT);
    if bone_records == 0 || bone_count == 0 || bone_count > 256 {
        return Err("the skin data has no bone records, or more than 256");
    }
    for i in 0..bone_count as usize {
        if rd::<usize>(bones, i * 4) == 0 {
            return Err("a bone of the skin instance is null");
        }
    }
    let inverse_root_parent = invert(&exact_transform(t, root_parent)).ok_or("the transform of the root parent has no inverse")?;
    let skin_to_root = combine(&combine(shape, &from_ni(data + DATA_TRANSFORM)), &inverse_root_parent);
    for i in 0..bone_count as usize {
        let bone = exact_transform(t, rd(bones, i * 4));
        each(&combine(&combine(&skin_to_root, &bone), &from_ni(bone_records + i * BONE_RECORD_SIZE)));
    }
    Ok(())
}

/// Called in place of the call of `SetSkinnedModelTransforms` at a draw site. Returns true
/// when this module set the world matrices of the bones.
///
/// # Safety
///
/// `this` points at the live `NiDX8Renderer` with its device; `skin` and `partition` are each 0 or
/// a live `NiSkinInstance` with its data and bones and a live skin partition of it; `transform` is
/// 0 or the world transform of a live `NiAVObject` whose parent chain is live.
pub unsafe fn take_skinned_transforms(t: &MainThread, this: usize, skin: usize, partition: usize, transform: usize, geometry_site: bool) -> bool {
    if !own() {
        return false;
    }
    let Some(origin) = active_origin(t) else { return false };
    let made = skinned_relative(t, skin, partition, transform, geometry_site, &origin, |index, relative| {
        *PENDING.get(t) = true;
        renderer_frame::set_bone_transform(this, relative.as_ptr() as usize, index);
        *PENDING.get(t) = false;
    });
    if made {
        renderer_frame::update_model_camera(this, transform);
        count(t, 7);
    }
    made
}

/// Called after the call of `SetSkinnedModelTransforms` at a draw site. In the compare mode
/// it counts if the first four matrices of the host are the matrices of this module.
///
/// # Safety
///
/// As for [`take_skinned_transforms`], without the renderer.
pub unsafe fn check_skinned_transforms(t: &MainThread, skin: usize, partition: usize, transform: usize, geometry_site: bool) {
    if !compare() {
        return;
    }
    let Some(origin) = active_origin(t) else { return };
    let mut equal = true;
    let made = skinned_relative(t, skin, partition, transform, geometry_site, &origin, |index, relative| {
        if index < 4 {
            // `write_world_matrix` writes only the world matrix of a renderer.
            let mut matrix = [0u32; 16];
            renderer_frame::write_world_matrix((matrix.as_mut_ptr() as usize).wrapping_sub(R::currentWorldMatrix), relative.as_ptr() as usize);
            equal = equal && crate::link_client::world_relative(t, index) && crate::link_client::world_matrix(t, index) == matrix;
        }
    });
    if !made {
        // The host must have used the matrices of the game.
        equal = !crate::link_client::world_relative(t, 0);
    }
    count(t, if equal { 4 } else { 5 });
}

/// The counters as one line of text.
///
/// # Safety
///
/// No condition of its own: the function reads no engine memory. `t` is the proof of the main
/// thread.
pub unsafe fn report(t: &MainThread, reset: bool) -> String {
    let counts = &mut *COUNTS.get(t);
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

    /// A shape of 0 gives "no shape" and reads nothing, with or without a skin.
    #[test]
    fn skin_bones_of_no_shape_is_missing_data() {
        let t = MainThread::for_test();
        let mut out = vec![1u8, 2, 3];
        assert_eq!(unsafe { skin_bones(&t, 0, 0, &mut out) }, Err("no shape"));
        assert!(out.is_empty());
    }

    #[test]
    fn float_step_is_one_unit_in_the_last_place() {
        assert_eq!(float_step(1.0), 2f32.powi(-23));
        assert_eq!(float_step(100000.0), 2f32.powi(17 - 24));
        assert_eq!(float_step(0.0), 2f32.powi(-24));
        // The bits give the step of the loop for every float of the world and beyond.
        let mut x = 1e-30f32;
        while x < 1e30 {
            for v in [x, -x, x * 1.5, x * 0.75] {
                assert_eq!(float_step(v).to_bits(), float_step_by_loop(v).to_bits(), "{v}");
            }
            x *= 1.37;
        }
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
        let t = &MainThread::for_test();
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
            retire_cache(t);
            let exact = exact_translation(t, c, 0).unwrap();
            assert_eq!(exact[0], 400000.0f64 + 0.01f32 as f64);
            // A child that the game placed in another way keeps its stored translation.
            crate::mem::wr(c, AV_WORLD + T_TRANSLATION, [5.0f32, 0.0, 0.0]);
            assert_eq!(exact_translation(t, c, 0).unwrap()[0], 5.0);
        }
    }
}
