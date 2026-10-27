//! The errors of the GPU, and the counts of two kinds of work, for the whole program (it has
//! one device).
//!
//! - wgpu ends the program at an error that no error scope takes. Here the log names the
//!   error (with the label of the object) and the program goes on: the draw or the pass that
//!   has the error does nothing.
//! - A lost device (a reset of the driver) ends all drawing. The log says so, and with a
//!   window a message box tells the player.
//! - `submit` and `frame_group` count the submits to the queue and the bind groups that are
//!   made for one frame, with their CPU time, for the marks of a scenario.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

static ERRORS: AtomicU64 = AtomicU64::new(0);
static LOST: AtomicBool = AtomicBool::new(false);
/// `test_loss` destroys the device: its loss is then handled as a loss for an unknown reason.
static TEST_LOSS: AtomicBool = AtomicBool::new(false);
static SUBMITS: AtomicU64 = AtomicU64::new(0);
static SUBMIT_NANOS: AtomicU64 = AtomicU64::new(0);
static GROUPS: AtomicU64 = AtomicU64::new(0);
static GROUP_NANOS: AtomicU64 = AtomicU64::new(0);

/// The errors that the log shows in full. After them, one line for each 1000.
const ERRORS_SHOWN: u64 = 20;

/// Sets the handlers of the errors and of the loss of `device`. `window`: the program draws
/// into a window, and a lost device gets a message box.
pub fn watch(device: &wgpu::Device, adapter: &wgpu::AdapterInfo, window: bool) {
    let name = format!("{} ({:?}, driver {} {})", adapter.name, adapter.backend, adapter.driver, adapter.driver_info);
    device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
        let count = ERRORS.fetch_add(1, Ordering::Relaxed) + 1;
        let kind = match &error {
            wgpu::Error::OutOfMemory { .. } => "out of memory",
            wgpu::Error::Validation { .. } => "validation",
            wgpu::Error::Internal { .. } => "internal",
        };
        if count <= ERRORS_SHOWN {
            println!("gpu error {count} ({kind}, adapter {name}): {error}");
            if count == ERRORS_SHOWN {
                println!("gpu error: the log shows no more errors in full, only their count");
            }
        } else if count % 1000 == 0 {
            println!("gpu error: {count} errors");
        }
    }));
    device.set_device_lost_callback(move |reason, message| {
        let destroyed = matches!(reason, wgpu::DeviceLostReason::Destroyed) && !TEST_LOSS.load(Ordering::Relaxed);
        if destroyed {
            // The program destroyed the device itself, at its end.
            return;
        }
        LOST.store(true, Ordering::Relaxed);
        println!("gpu: the device is lost ({reason:?}): {message}");
        if window {
            std::thread::spawn(lost_message);
        }
    });
}

/// The device is lost: nothing draws any more.
pub fn lost() -> bool {
    LOST.load(Ordering::Relaxed)
}

/// The number of errors since the start.
pub fn errors() -> u64 {
    ERRORS.load(Ordering::Relaxed)
}

/// Destroys the device, and its loss is handled as a real one: the test of the loss. The poll
/// gives the loss to the callback, as a submit or a poll does with a real loss.
pub fn test_loss(device: &wgpu::Device) {
    TEST_LOSS.store(true, Ordering::Relaxed);
    device.destroy();
    let _ = device.poll(wgpu::PollType::Poll);
    // The callback can come later than the poll (wgpu 30, a test of 2026-10-21): the loss
    // counts from here, as it would for the driver's own report.
    if !LOST.swap(true, Ordering::Relaxed) {
        println!("gpu: the device is lost (the test)");
    }
}

/// `queue.submit`, counted.
pub fn submit<I: IntoIterator<Item = wgpu::CommandBuffer>>(queue: &wgpu::Queue, buffers: I) -> wgpu::SubmissionIndex {
    let start = Instant::now();
    let index = queue.submit(buffers);
    SUBMITS.fetch_add(1, Ordering::Relaxed);
    SUBMIT_NANOS.fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
    index
}

/// `device.create_bind_group`, counted: for a bind group that is made for one frame.
pub fn frame_group(device: &wgpu::Device, descriptor: &wgpu::BindGroupDescriptor) -> wgpu::BindGroup {
    let start = Instant::now();
    let group = device.create_bind_group(descriptor);
    GROUPS.fetch_add(1, Ordering::Relaxed);
    GROUP_NANOS.fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
    group
}

/// Why a pass on the game's targets opened (`Player::open_pass`), for the census of the passes.
#[derive(Clone, Copy)]
pub enum PassReason {
    FrameStart,
    Flush,
    Target,
    Clear,
    WorldStart,
    AfterCopy,
    AfterExtension,
}

/// The names of [`PassReason`], in its order.
pub const PASS_REASONS: [&str; 7] = ["the frame's start", "a flush", "a target", "a clear", "the world's start", "after a copy", "after the extension"];

static PASSES: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];
static BEGUN_PASSES: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];
static DRAWN_PASSES: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];
static TABLE_PASSES: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];
static OTHER_PASSES: AtomicU64 = AtomicU64::new(0);
static ARRAY_BINDS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

/// The number of counts in [`passes`].
pub const PASS_COUNTS: usize = 33;

/// Who bound the group of the texture arrays, for the census of the passes.
#[derive(Clone, Copy)]
pub enum BindOrigin {
    /// A draw of the game (`Player::issue`).
    Game,
    /// The table's draws in the game's pass (the main view, the sorted blends).
    Table,
    /// A replay for the extension: its own passes (the reflection, the shadows) or its calls.
    Replay,
    /// A draw of the game before the world or after the post chain, with the small group of
    /// its own textures (`Renderer::small_array_group`).
    Small,
}

/// A pass on the game's targets was asked for.
pub fn note_pass(reason: PassReason) {
    PASSES[reason as usize].fetch_add(1, Ordering::Relaxed);
}

/// A pass on the game's targets began (`Player::begin_pass`). `reason`: that of the last ask
/// for it.
pub fn note_pass_begun(reason: PassReason) {
    BEGUN_PASSES[reason as usize].fetch_add(1, Ordering::Relaxed);
}

/// A pass on the game's targets ended with a draw of the game or of the table; `table`: with
/// a draw of the table.
pub fn note_drawn_pass(reason: PassReason, table: bool) {
    DRAWN_PASSES[reason as usize].fetch_add(1, Ordering::Relaxed);
    if table {
        TABLE_PASSES[reason as usize].fetch_add(1, Ordering::Relaxed);
    }
}

/// A pass of another part opened: a copy, a merge, the overlay, the map tiles, the extension's
/// own passes.
pub fn note_other_pass() {
    OTHER_PASSES.fetch_add(1, Ordering::Relaxed);
}

/// The bind group of the texture arrays was set in a pass.
pub fn note_array_bind(origin: BindOrigin) {
    ARRAY_BINDS[origin as usize].fetch_add(1, Ordering::Relaxed);
}

/// The census of the passes since the start, by [`PassReason`]: the passes asked for (0..7),
/// begun (7..14), with a draw (14..21) and with a draw of the table (21..28); then the other
/// passes, and the binds of the texture arrays by [`BindOrigin`] (29..33).
pub fn passes() -> [u64; PASS_COUNTS] {
    let mut out = [0u64; PASS_COUNTS];
    for (slot, count) in out.iter_mut().zip(PASSES.iter().chain(&BEGUN_PASSES).chain(&DRAWN_PASSES).chain(&TABLE_PASSES)) {
        *slot = count.load(Ordering::Relaxed);
    }
    out[28] = OTHER_PASSES.load(Ordering::Relaxed);
    for (slot, count) in out[29..].iter_mut().zip(ARRAY_BINDS.iter()) {
        *slot = count.load(Ordering::Relaxed);
    }
    out
}

/// The census as a line of the log: `delta` against the counts at the last mark, for `frames`
/// frames.
pub fn pass_line(delta: &[u64; PASS_COUNTS], frames: u64) -> String {
    let per = |n: u64| n as f64 / frames.max(1) as f64;
    let part = |counts: &[u64]| {
        let reasons: Vec<String> = PASS_REASONS.iter().zip(counts).filter(|(_, n)| **n != 0).map(|(name, n)| format!("{name} {:.2}", per(*n))).collect();
        format!("{:.2} ({})", per(counts.iter().sum()), reasons.join(", "))
    };
    format!(
        "passes: on the game's targets for each frame {} asked for, {} begun, {} with a draw, {} with a draw of the table; {:.2} other passes; {:.2} binds of the whole texture arrays ({:.2} by the game's draws, {:.2} by the table, {:.2} by the replays), {:.2} binds of a small group",
        part(&delta[..7]),
        part(&delta[7..14]),
        part(&delta[14..21]),
        part(&delta[21..28]),
        per(delta[28]),
        per(delta[29] + delta[30] + delta[31]),
        per(delta[29]),
        per(delta[30]),
        per(delta[31]),
        per(delta[32])
    )
}

/// The counts since the start: submits and their nanoseconds, bind groups made for a frame
/// and their nanoseconds.
pub fn work() -> [u64; 4] {
    [
        SUBMITS.load(Ordering::Relaxed),
        SUBMIT_NANOS.load(Ordering::Relaxed),
        GROUPS.load(Ordering::Relaxed),
        GROUP_NANOS.load(Ordering::Relaxed),
    ]
}

fn lost_message() {
    message_box(
        "The graphics device was lost: the driver was reset, or it stopped.\n\n\
         The wgpu renderer of MGE XE cannot draw any more. Quit the game and start it again.\n\n\
         The log of the renderer (nirender-host.log in the folder of the game) has the details.",
    );
}

/// The window that owns the boxes of this program: the game's window in the live mode, so
/// that a box stays above it (an owned window is above its owner); 0 for a replay and for the
/// program on its own.
static OWNER_WINDOW: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Names the window that owns the boxes ([`message_box`]).
pub fn set_owner_window(window: isize) {
    OWNER_WINDOW.store(window, Ordering::Relaxed);
}

/// Shows `text` to the player in a box with the caption "MGE XE", owned by the game's window,
/// and waits until it is closed. For what ends the drawing: a lost device, a stream of another
/// build. Public for the extension's own refusals. The rule of every box of the three parts
/// (the failure-path design of the Numidium docs, sections 5 and 5.1; the plugin's and
/// d3d8.dll's boxes follow it too): each line goes to the log first as "box: "; with
/// `MGE_XE_BOX_SECONDS=<n>` the box closes itself after n seconds, through MessageBoxTimeoutW
/// of user32 (not in the documented API, so taken by GetProcAddress; without it the box is
/// only logged); without the switch it waits for the click.
#[cfg(windows)]
pub fn message_box(text: &str) {
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(window: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> isize;
        fn GetProcAddress(module: isize, name: *const u8) -> *const core::ffi::c_void;
    }
    type MessageBoxTimeout = unsafe extern "system" fn(isize, *const u16, *const u16, u32, u16, u32) -> i32;
    const MB_ICONERROR: u32 = 0x10;
    const MB_SETFOREGROUND: u32 = 0x1_0000;
    const MB_TOPMOST: u32 = 0x4_0000;
    const TIMED_OUT: i32 = 32000;
    for line in text.lines() {
        println!("box: {line}");
    }
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let text = wide(text);
    let caption = wide("MGE XE");
    let owner = OWNER_WINDOW.load(Ordering::Relaxed);
    let flags = MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST;
    let seconds = crate::switches().box_seconds;
    // SAFETY: the strings are zero-ended UTF-16 that live until the call returns; the
    // function taken from user32 has the signature of MessageBoxTimeoutW.
    let answer = unsafe {
        if seconds == 0 {
            MessageBoxW(owner, text.as_ptr(), caption.as_ptr(), flags)
        } else {
            let user32 = GetModuleHandleW(wide("user32.dll").as_ptr());
            let timed = GetProcAddress(user32, c"MessageBoxTimeoutW".as_ptr().cast());
            if timed.is_null() {
                println!("box: not shown: this Windows has no MessageBoxTimeoutW");
                return;
            }
            let timed: MessageBoxTimeout = std::mem::transmute(timed);
            timed(owner, text.as_ptr(), caption.as_ptr(), flags, 0, seconds * 1000)
        }
    };
    if answer == TIMED_OUT {
        println!("box: closed after {seconds} s, no click");
    } else {
        println!("box: closed by a click");
    }
}

#[cfg(not(windows))]
pub fn message_box(text: &str) {
    for line in text.lines() {
        println!("box: {line}");
    }
}
