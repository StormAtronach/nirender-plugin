//! The game's resolution changed at run time by its window (the owner's D23; the design
//! `notes/designs-2026-10/resolution-switch.md` 5.2 of the Numidium project): a windowed game's
//! window dragged to another size, maximised or restored. The game's own Options menu changes
//! the resolution with `TES3Game::restartRenderer` 0x41A700, which makes the renderer again
//! (a second `CreateDevice` of the own device: `own_device` keeps the stream, the helper and
//! the device, and the stream gets a SIZE record). A resize of the window asks for the same
//! restart at the start of the next frame (MWSE's native event FRAME_STARTED), with the size
//! of the window's inside, under the load screen's render lock as the menu takes it
//! (`ui_MenuVideo_resChangeMessage_onResult` 0x6383E0). During a drag the helper scales the
//! last picture into the window; the restart comes once, at its end.
//!
//! A windowed game's window has no sizing frame and no maximise box (`WinMain` 0x416E10, the
//! style 0x10CA0000): the plugin adds them at the first device, with the inside of the window
//! kept at its size ([`frame_for_sizing`]).

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use crate::gen::layouts::{TES3Game, WorldController};
use crate::mem::{rd, wr};

#[link(name = "user32")]
extern "system" {
    fn SetWindowLongA(window: usize, index: i32, value: u32) -> u32;
    fn GetWindowLongA(window: usize, index: i32) -> u32;
    fn CallWindowProcA(previous: usize, window: usize, message: u32, wparam: usize, lparam: isize) -> isize;
    fn GetClientRect(window: usize, rect: *mut i32) -> i32;
    fn AdjustWindowRect(rect: *mut i32, style: u32, menu: i32) -> i32;
    fn SetWindowPos(window: usize, after: usize, x: i32, y: i32, width: i32, height: i32, flags: u32) -> i32;
}

/// The size of the back buffer of the device, and whether the game is windowed: the
/// window's resize asks for a restart only in a windowed game, and only for another size.
static WIDTH: AtomicU32 = AtomicU32::new(0);
static HEIGHT: AtomicU32 = AtomicU32::new(0);
static WINDOWED: AtomicBool = AtomicBool::new(false);
/// The player drags the window (between WM_ENTERSIZEMOVE and WM_EXITSIZEMOVE).
static DRAGGING: AtomicBool = AtomicBool::new(false);
/// The size that the window asks for, both 0 for none: taken at the next frame's start.
static WANTED: AtomicU32 = AtomicU32::new(0);
/// The window procedure before the plugin's, and the window it is on.
static PREVIOUS: AtomicUsize = AtomicUsize::new(0);
static WINDOW: AtomicUsize = AtomicUsize::new(0);
/// The listener of the frame's start, once.
static SUBSCRIBED: AtomicBool = AtomicBool::new(false);

const GWL_WNDPROC: i32 = -4;
const GWL_STYLE: i32 = -16;
const WM_SIZE: u32 = 0x0005;
const WM_ENTERSIZEMOVE: u32 = 0x0231;
const WM_EXITSIZEMOVE: u32 = 0x0232;
const SIZE_RESTORED: usize = 0;
const SIZE_MAXIMIZED: usize = 2;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_POPUP: u32 = 0x8000_0000;

/// The device took a back buffer of `width` x `height`, windowed or not. Called by
/// `own_device::accept_parameters` under the state's guard: atomics only.
pub fn note_back_buffer(width: u32, height: u32, windowed: bool) {
    WIDTH.store(width, Ordering::Relaxed);
    HEIGHT.store(height, Ordering::Relaxed);
    WINDOWED.store(windowed, Ordering::Relaxed);
}

/// The two sizes packed into one word for [`WANTED`]: 16 bits each.
fn pack(width: u32, height: u32) -> u32 {
    (width.min(0xFFFF) << 16) | height.min(0xFFFF)
}

fn unpack(word: u32) -> (u32, u32) {
    (word >> 16, word & 0xFFFF)
}

/// True when a window of `size` asks for a restart: a windowed game, a size with pixels, and
/// another size than the back buffer's.
fn asks_restart(size: (u32, u32), back_buffer: (u32, u32), windowed: bool) -> bool {
    windowed && size.0 > 0 && size.1 > 0 && size != back_buffer
}

/// Notes the size that the window asks for.
fn ask(size: (u32, u32)) {
    let back_buffer = (WIDTH.load(Ordering::Relaxed), HEIGHT.load(Ordering::Relaxed));
    if asks_restart(size, back_buffer, WINDOWED.load(Ordering::Relaxed)) {
        WANTED.store(pack(size.0, size.1), Ordering::Relaxed);
    }
}

/// The plugin's window procedure on the game's top window, before the game's: the end of a
/// drag, and a maximise or a restore outside a drag (no WM_EXITSIZEMOVE comes then), ask for
/// a restart at the window's new size.
unsafe extern "system" fn window_procedure(window: usize, message: u32, wparam: usize, lparam: isize) -> isize {
    match message {
        WM_ENTERSIZEMOVE => DRAGGING.store(true, Ordering::Relaxed),
        WM_EXITSIZEMOVE => {
            DRAGGING.store(false, Ordering::Relaxed);
            let mut rect = [0i32; 4];
            GetClientRect(window, rect.as_mut_ptr());
            ask(((rect[2] - rect[0]).max(0) as u32, (rect[3] - rect[1]).max(0) as u32));
        }
        WM_SIZE if !DRAGGING.load(Ordering::Relaxed) && (wparam == SIZE_RESTORED || wparam == SIZE_MAXIMIZED) => {
            ask(((lparam as usize & 0xFFFF) as u32, ((lparam as usize >> 16) & 0xFFFF) as u32));
        }
        _ => {}
    }
    CallWindowProcA(PREVIOUS.load(Ordering::Relaxed), window, message, wparam, lparam)
}

/// The plugin's window procedure on the game's top window `window`, and the listener of the
/// frame's start, once. Outside the state's guard: the style's change sends messages.
///
/// # Safety
///
/// `window` is the game's top window, on the main thread.
pub unsafe fn watch(window: usize) {
    if window == 0 || WINDOW.swap(window, Ordering::Relaxed) == window {
        return;
    }
    let previous = SetWindowLongA(window, GWL_WNDPROC, window_procedure as *const () as usize as u32);
    PREVIOUS.store(previous as usize, Ordering::Relaxed);
    if !SUBSCRIBED.swap(true, Ordering::Relaxed) {
        if let Err(error) = crate::mwse_native::subscribe(crate::mwse_native::NATIVE_EVENT_FRAME_STARTED, frame_started) {
            crate::log!("resolution: {error}: a resize of the window does not change the game's resolution");
        }
    }
    crate::log!("resolution: the game's window {window:#x} is watched for its size");
}

/// A windowed game's window gets a sizing frame and a maximise box, with the inside of the
/// window kept at its size (no restart at the start). A window without a caption (MGE XE's
/// borderless window, a popup) stays as it is.
///
/// # Safety
///
/// `window` is the game's top window, on the main thread, outside the state's guard.
pub unsafe fn frame_for_sizing(window: usize) {
    let style = GetWindowLongA(window, GWL_STYLE);
    if style & WS_POPUP != 0 || style & WS_THICKFRAME != 0 {
        return;
    }
    let mut client = [0i32; 4];
    GetClientRect(window, client.as_mut_ptr());
    let sized = style | WS_THICKFRAME | WS_MAXIMIZEBOX;
    let mut outer = client;
    AdjustWindowRect(outer.as_mut_ptr(), sized, 0);
    SetWindowLongA(window, GWL_STYLE, sized);
    // SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED
    SetWindowPos(window, 0, 0, 0, outer[2] - outer[0], outer[3] - outer[1], 0x0002 | 0x0004 | 0x0010 | 0x0020);
    crate::log!("resolution: the game's window can be resized (a sizing frame and a maximise box)");
}

/// The start of a frame (MWSE's native event, on the main thread): the size that the window
/// asked for, when it is still another than the back buffer's, restarts the renderer at it.
unsafe extern "C" fn frame_started(_user: *mut core::ffi::c_void, _packet: *const core::ffi::c_void) {
    let wanted = WANTED.swap(0, Ordering::Relaxed);
    if wanted == 0 {
        return;
    }
    let size = unpack(wanted);
    let back_buffer = (WIDTH.load(Ordering::Relaxed), HEIGHT.load(Ordering::Relaxed));
    if asks_restart(size, back_buffer, WINDOWED.load(Ordering::Relaxed)) {
        restart(size.0, size.1);
    }
}

/// `TES3Game::restartRenderer` at `width` x `height`, as the Options menu does it
/// (`ui_MenuVideo_resChangeMessage_onResult` 0x6383E0): the world controller's view size and a
/// bit depth of 32, then the restart under the load screen's render lock.
unsafe fn restart(width: u32, height: u32) {
    let game: usize = rd(0x7C6CDC, 0);
    let world: usize = rd(crate::engine::WORLD_CONTROLLER, 0);
    if game == 0 || world == 0 {
        return;
    }
    crate::log!("resolution: the window asks for {width} x {height}: the game makes its renderer again");
    wr::<u32>(world, WorldController::viewWidth, width);
    wr::<u32>(world, WorldController::viewHeight, height);
    wr::<u32>(world, WorldController::bitDepth, 32);
    let screens: usize = rd(game, TES3Game::loadScreenManager);
    // LoadScreenManager::renderMutexLock 0x458D70 and renderMutexUnlock 0x458DA0.
    let lock = core::mem::transmute::<usize, unsafe extern "thiscall" fn(usize)>(0x458D70);
    let unlock = core::mem::transmute::<usize, unsafe extern "thiscall" fn(usize)>(0x458DA0);
    let restart_renderer = core::mem::transmute::<usize, unsafe extern "thiscall" fn(usize, u32, u32, u32)>(0x41A700);
    if screens != 0 {
        lock(screens);
    }
    restart_renderer(game, width, height, 32);
    if screens != 0 {
        unlock(screens);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_packs_into_one_word() {
        assert_eq!(unpack(pack(2560, 1440)), (2560, 1440));
        assert_eq!(unpack(pack(70000, 1)), (0xFFFF, 1));
    }

    #[test]
    fn only_a_windowed_game_of_another_size_restarts() {
        assert!(asks_restart((1600, 900), (1920, 1080), true));
        // The engine's own resize of its window at a restart: the same size, nothing.
        assert!(!asks_restart((1920, 1080), (1920, 1080), true));
        // Fullscreen: the window covers its monitor and the game keeps its size.
        assert!(!asks_restart((1600, 900), (1920, 1080), false));
        // A minimised window has no inside.
        assert!(!asks_restart((0, 0), (1920, 1080), true));
    }
}
