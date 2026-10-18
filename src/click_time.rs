//! The time of the engine's draw of each camera (`NiCamera::Click` 0x6CC7B0), for the
//! comparison of the engine's walk of the world with the cut (plan 8e, 2026-10-18). The
//! table walk and the live walk run inside the world camera's draw (at the world root's
//! display, `live_walk::after_child`), so the world camera's time holds them with the cut,
//! and the engine's walk without it. The time is counted by camera, with the kind of the
//! scene that the camera's draw began (`note_scene`).

use crate::ptr_hash::HashMap;

const CLICK: usize = 0x6CC7B0;

static mut ORIGINAL: usize = 0;

#[derive(Default)]
struct Camera {
    kind: u32,
    calls: u64,
    ticks: i64,
}

static mut CAMERAS: Option<HashMap<usize, Camera>> = None;
/// The draws inside a draw (counted twice: in their camera and in the outer one).
static mut DEPTH: u32 = 0;
static mut NESTED: u64 = 0;

unsafe fn cameras() -> &'static mut HashMap<usize, Camera> {
    (*core::ptr::addr_of_mut!(CAMERAS)).get_or_insert_with(HashMap::default)
}

unsafe extern "thiscall" fn detour(this: usize, dont_finish: u32) {
    let original: unsafe extern "thiscall" fn(usize, u32) = core::mem::transmute(*core::ptr::addr_of!(ORIGINAL));
    if DEPTH > 0 {
        NESTED += 1;
    }
    DEPTH += 1;
    let start = crate::capture::ticks();
    original(this, dont_finish);
    let elapsed = crate::capture::ticks() - start;
    DEPTH -= 1;
    let camera = cameras().entry(this).or_default();
    camera.calls += 1;
    camera.ticks += elapsed;
}

/// A scene began for this camera (`link_client`, at `BeginPaint`): its kind is kept.
pub unsafe fn note_scene(camera: usize, kind: u32) {
    if *core::ptr::addr_of!(ORIGINAL) != 0 {
        cameras().entry(camera).or_default().kind = kind;
    }
}

pub unsafe fn install() {
    match crate::hook::install(CLICK, detour as *const () as usize, 1, crate::hook::Cleanup::Callee) {
        Ok(trampoline) => {
            *core::ptr::addr_of_mut!(ORIGINAL) = trampoline;
            crate::log!("click time: the draw of each camera is timed (NiCamera::Click {CLICK:#x})");
        }
        Err(error) => crate::log!("click time: no detour on NiCamera::Click: {error}"),
    }
}

/// The cameras by their time, the longest first: the kind of the scene, the draws, the time
/// and the time of a draw.
pub unsafe fn report() -> String {
    let ms = |t: i64| t as f64 * 1000.0 / crate::capture::ticks_per_second() as f64;
    let mut list: Vec<(&usize, &Camera)> = cameras().iter().collect();
    list.sort_by_key(|(_, c)| -c.ticks);
    let parts: Vec<String> = list
        .iter()
        .take(8)
        .map(|(&at, c)| format!("{at:#x} kind {}: {} draws, {:.3} ms, {:.4} ms each", c.kind, c.calls, ms(c.ticks), ms(c.ticks) / c.calls.max(1) as f64))
        .collect();
    format!("click time (plan 8e): {}; draws inside a draw {}", parts.join("; "), *core::ptr::addr_of!(NESTED))
}
