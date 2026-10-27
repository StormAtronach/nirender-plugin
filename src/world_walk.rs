//! The start of the world in the world camera's Click, from the engine's own walk of the
//! scene graph. The helper draws MGE XE's distant land after the sky and under the world; it
//! learns that place here, not from the draws.
//!
//! `TES3Game::initGame` attaches the weather's sky root as the first child of the world
//! camera's root (0x417F31, `AttachChild` with "first available"); the game's world root
//! comes after it. `NiCamera::Click` (0x6CC7B0) walks the root with one call of
//! `NiAVObject::CullShow` at 0x6CC874. The plugin owns MWSE's override of that call
//! (`mwse_native::NATIVE_OVERRIDE_CAMERA_ROOT_CULL_SHOW`): MWSE calls `click_cull_show` there.
//! For the root of the world scene (a plain `NiNode`), the function does what the engine does
//! for that one object, in the same order: the tests of `NiAVObject::CullShow` (0x6EB480) on
//! the root, then the loop of `NiNode::Display` (0x6C9190) over the children, each child
//! through the engine's `CullShow`. When the step of the sky root returns, the frame event
//! `EVENT_WORLD_WALK` goes into the stream. Every other root goes to the engine's `CullShow`.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::engine::{thiscall1, vcall0};
use crate::gen::layouts::{NiAVObject as AV, NiCamera as C, NiNode as N};
use crate::main_thread::MainThread;
use crate::mem::{rd, wr};

/// `NiAVObject::CullShow`, the function of the call in `NiCamera::Click` that MWSE's override
/// replaces.
const CULL_SHOW: usize = 0x6EB480;
const NODE_DISPLAY: usize = 0x6C9190;
const WHICH_SIDE: usize = 0x6F2660;
const SLOT_GET_APP_CULLED: usize = crate::gen::layouts::InterfaceNiAVObject::GetAppCulled;
const SLOT_IS_VISUAL_OBJECT: usize = crate::gen::layouts::InterfaceNiAVObject::IsVisualObject;
const SLOT_DISPLAY: usize = crate::gen::layouts::InterfaceNiAVObject::Display;
/// The flag that `NiNode::Display` sets on the node (the low byte of the flags).
const NODE_DISPLAYED: u8 = 0x10;

static INSTALLED: AtomicBool = AtomicBool::new(false);
/// The walks of the world root the plugin did, and the events it wrote.
static WALKS: AtomicU64 = AtomicU64::new(0);
static EVENTS: AtomicU64 = AtomicU64::new(0);

/// True when the plugin owns MWSE's override of the root's walk: the stream then carries the
/// event, and its header says so (`capture::HEADER_WORLD_WALK`).
pub fn installed() -> bool {
    INSTALLED.load(Ordering::Relaxed)
}

/// Claims MWSE's override. A refusal (another module owns it, or an MWSE without it) leaves the
/// engine's call: the stream then has no event, and the helper takes the world's start as an
/// older capture has it.
///
/// # Safety
///
/// One thread at a time calls this: the test of the installed flag and its store are not one atomic
/// step.
pub unsafe fn install() {
    if installed() {
        return;
    }
    if let Err(error) = crate::mwse_native::claim(crate::mwse_native::NATIVE_OVERRIDE_CAMERA_ROOT_CULL_SHOW, click_cull_show as *const () as usize) {
        crate::log!("world walk: {error}: the helper finds the world's start as before");
        return;
    }
    INSTALLED.store(true, Ordering::Relaxed);
    crate::log!("world walk: the world camera's Click marks the end of the sky's walk");
}

/// The report line.
pub fn report() -> String {
    format!("world walk: {} walks of the world root, {} events", WALKS.load(Ordering::Relaxed), EVENTS.load(Ordering::Relaxed))
}

/// The call of `CullShow` for the root of a camera's Click (thiscall: the root, the camera).
unsafe extern "thiscall" fn click_cull_show(root: usize, camera: usize) {
    if let Some(t) = MainThread::here() {
        if walk_due(&t, root) {
            walk_world_root(&t, root, camera);
            return;
        }
    }
    thiscall1(CULL_SHOW, root, camera as u32);
}

/// The root of the world scene, a plain `NiNode`, while the stream records.
unsafe fn walk_due(t: &MainThread, root: usize) -> bool {
    root != 0
        && crate::capture::recording()
        && crate::link_client::scene_kind_now(t) == crate::link::SCENE_WORLD
        && rd::<usize>(rd::<usize>(root, 0), SLOT_DISPLAY) == NODE_DISPLAY
}

/// `NiAVObject::CullShow` of the root with `NiNode::Display` in place of its `Display`, as
/// the engine runs them, and the event after the sky root's step (before the first child
/// when the sky root is not a child).
unsafe fn walk_world_root(t: &MainThread, root: usize, camera: usize) {
    WALKS.fetch_add(1, Ordering::Relaxed);
    if vcall0(root, SLOT_GET_APP_CULLED) & 0xFF != 0 {
        return;
    }
    // The planes the root is fully inside: its children skip them; the bits go back after.
    let count = rd::<i32>(camera, C::countCullingPlanes);
    let mut set = [0u32; 4];
    let mut plane = count - 1;
    while plane >= 0 {
        let bit = 1u32 << (plane & 31);
        let word = (plane >> 5) as usize * 4;
        let used = camera + C::usedCullingPlanesBitfield + word;
        if rd::<u32>(used, 0) & bit == 0 {
            let planes: usize = rd(camera, C::cullingPlanes_storage);
            let side = thiscall1(WHICH_SIDE, root + AV::worldBound, rd::<u32>(planes, plane as usize * 4));
            if side == 2 {
                restore(camera, &set);
                return;
            }
            if side == 1 {
                wr(used, 0, rd::<u32>(used, 0) | bit);
                set[word / 4] |= bit;
            }
        }
        plane -= 1;
    }
    node_display(t, root, camera);
    restore(camera, &set);
}

/// `NiNode::Display` of the root, with the event.
unsafe fn node_display(t: &MainThread, root: usize, camera: usize) {
    wr(root, AV::flags, rd::<u8>(root, AV::flags) | NODE_DISPLAYED);
    if vcall0(root, SLOT_IS_VISUAL_OBJECT) & 0xFF == 0 {
        return;
    }
    let sky = crate::link_client::sky_root_now(t);
    let children: usize = rd(root, N::kChildren_storage);
    let sky_is_child = sky != 0 && (0..rd::<u32>(root, N::kChildren_endIndex) as usize).any(|i| rd::<usize>(children, i * 4) == sky);
    if !sky_is_child {
        world_walk_event(t, root);
    }
    let mut i = 0;
    while i < rd::<u32>(root, N::kChildren_endIndex) as usize {
        let child: usize = rd(rd::<usize>(root, N::kChildren_storage), i * 4);
        if child != 0 {
            thiscall1(CULL_SHOW, child, camera as u32);
            if sky_is_child && child == sky {
                world_walk_event(t, root);
            }
        }
        i += 1;
    }
}

/// The bits `CullShow` set go back; it reads the count of the planes again.
unsafe fn restore(camera: usize, set: &[u32; 4]) {
    for plane in 0..rd::<i32>(camera, C::countCullingPlanes).max(0) {
        let bit = 1u32 << (plane & 31);
        let word = (plane >> 5) as usize * 4;
        if set[word / 4] & bit != 0 {
            let used = camera + C::usedCullingPlanesBitfield + word;
            wr(used, 0, rd::<u32>(used, 0) & !bit);
        }
    }
}

/// The values of the world for the frame (`instance_stream::world_scene`), then the event:
/// the helper's start of the world takes its sun, ambient and fog from them, in every mode.
unsafe fn world_walk_event(t: &MainThread, root: usize) {
    EVENTS.fetch_add(1, Ordering::Relaxed);
    crate::instance_stream::world_scene(t, root);
    let _lock = crate::sync::device_lock();
    crate::capture::frame_event(crate::link::EVENT_WORLD_WALK, 0, 0);
}
