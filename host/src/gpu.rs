//! The errors of the GPU, and the counts of two kinds of work, for the whole program (it has
//! one device).
//!
//! - wgpu ends the program at an error that no error scope takes. Here the log names the
//!   error (with the label of the object) and the program goes on: the draw or the pass that
//!   has the error does nothing.
//! - A lost device (a reset of the driver) ends all drawing. The log says so, and the live
//!   loop tells the game process (`live.rs`), whose plugin saves the game and shows the box.
//! - `submit` and `frame_group` count the submits to the queue and the bind groups that are
//!   made for one frame, with their CPU time, for the marks of a scenario.
//! - `memory_lines` reports the GPU memory that wgpu's allocator holds, by the kind of object
//!   (its label), and the counts of wgpu's objects, for a long run (risk 43, the growth of the
//!   helper's memory; `MGE_XE_MEMORY_MINUTES`).

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

/// Sets the handlers of the errors and of the loss of `device`.
pub fn watch(device: &wgpu::Device, adapter: &wgpu::AdapterInfo) {
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

/// The window that owns the boxes of this program: the game's window in the live mode, so
/// that a box stays above it (an owned window is above its owner; the owner saw the box come
/// to the front in run 2b of records entry 192); 0 for a replay and for the program on its
/// own. An owner in another process joins the input of the two threads: the box answers only
/// while the game's thread takes its messages; the game's thread still runs its frames after
/// a refusal (the plugin waits for the ring with a time limit), so it does \[I\].
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
    for line in text.lines() {
        println!("box: {line}");
    }
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let text = wide(text);
    let caption = wide("MGE XE");
    let owner = OWNER_WINDOW.load(Ordering::Relaxed);
    let flags = MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST;
    let seconds = crate::switches().box_seconds;
    let started = Instant::now();
    // SAFETY: the strings are zero-ended UTF-16 that live until the call returns; the
    // function taken from user32 has the signature of MessageBoxTimeoutW.
    unsafe {
        if seconds == 0 {
            MessageBoxW(owner, text.as_ptr(), caption.as_ptr(), flags);
        } else {
            let user32 = GetModuleHandleW(wide("user32.dll").as_ptr());
            let timed = GetProcAddress(user32, c"MessageBoxTimeoutW".as_ptr().cast());
            if timed.is_null() {
                println!("box: not shown: this Windows has no MessageBoxTimeoutW");
                return;
            }
            let timed: MessageBoxTimeout = std::mem::transmute(timed);
            timed(owner, text.as_ptr(), caption.as_ptr(), flags, 0, seconds * 1000);
        }
    }
    // The elapsed time tells a timeout from a click: MessageBoxTimeoutW answers IDOK on the
    // timeout on this machine, not the documented 32000 [V Rob's run, 2026-10-27].
    if seconds != 0 && started.elapsed() >= std::time::Duration::from_secs(u64::from(seconds)) {
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

/// Ends this program on a refusal with the exit `code` (10 to 29: the stream, the extension,
/// the GPU) and the box `text`; the caller logged its reason lines. A refusal is an exit in
/// progress, not a stall (the failure-path design, section 6.0), so in this order: the
/// watchdog is disarmed, the exit is announced to the plugin (`stream_format::RING_ENDING`),
/// the box waits for the player, the game's window gets WM_CLOSE (the close rule of
/// section 5; without the game's window, a replay, only the box), and the program ends with
/// the code. Public for the extension's own refusals.
pub fn refuse(code: u32, text: &str) -> ! {
    crate::watchdog::disarm();
    crate::live::announce_exit(code);
    println!("refusal: the exit code {code} is announced; the box, then the end");
    message_box(text);
    let owner = OWNER_WINDOW.load(Ordering::Relaxed);
    if owner != 0 {
        close_window(owner);
    }
    exit_now(code)
}

#[cfg(windows)]
fn close_window(window: isize) {
    #[link(name = "user32")]
    extern "system" {
        fn PostMessageW(window: isize, message: u32, wparam: usize, lparam: isize) -> i32;
    }
    const WM_CLOSE: u32 = 0x10;
    println!("box: the game's window gets WM_CLOSE");
    // SAFETY: PostMessageW takes any window handle; a window that is gone fails the call.
    unsafe { PostMessageW(window, WM_CLOSE, 0, 0) };
}

#[cfg(not(windows))]
fn close_window(_window: isize) {}

/// Ends this program at once with `code`, for a refusal: TerminateProcess, not the orderly
/// exit, so that no thread of the program (the loader, the device) can hold the end. The
/// plugin reads the code. TerminateProcess drops what stdout still buffers, so the log is
/// flushed first: the box lines and the refusal lines must be in the file.
#[cfg(windows)]
pub fn exit_now(code: u32) -> ! {
    use std::io::Write;
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn TerminateProcess(process: isize, code: u32) -> i32;
    }
    let _ = std::io::stdout().flush();
    // SAFETY: the handle of the own process is always valid; nothing runs after the call.
    unsafe { TerminateProcess(GetCurrentProcess(), code) };
    std::process::exit(code as i32)
}

#[cfg(not(windows))]
pub fn exit_now(code: u32) -> ! {
    std::process::exit(code as i32)
}

/// The lines of the memory report: the GPU memory of wgpu's allocator (`generate_allocator_report`:
/// gpu-allocator on Vulkan and on Direct3D 12; [`None`] on a backend without it), by the kind
/// of object, and the counts of wgpu's objects and of the memory they hold (wgpu's `counters`).
pub fn memory_lines(device: &wgpu::Device) -> Vec<String> {
    let mut lines = match device.generate_allocator_report() {
        Some(report) => {
            let allocations: Vec<(&str, u64)> = report.allocations.iter().map(|allocation| (allocation.name.as_str(), allocation.size)).collect();
            allocator_lines(&allocations, report.blocks.len(), report.total_allocated_bytes, report.total_reserved_bytes)
        }
        None => vec!["memory: the backend gives no allocator report".to_string()],
    };
    let counters = device.get_internal_counters().hal;
    lines.push(counter_line(&[
        ("buffers", counters.buffers.read()),
        // No count of the textures: wgpu-hal 30.0.1 on Vulkan adds to it only for an external
        // texture (`add_raw_texture`) and takes from it at each `destroy_texture`, so it runs
        // below zero. The texture memory below and the allocator's kinds count them.
        ("texture views", counters.texture_views.read()),
        ("bind groups", counters.bind_groups.read()),
        ("bind group layouts", counters.bind_group_layouts.read()),
        ("render pipelines", counters.render_pipelines.read()),
        ("compute pipelines", counters.compute_pipelines.read()),
        ("pipeline layouts", counters.pipeline_layouts.read()),
        ("samplers", counters.samplers.read()),
        ("command encoders", counters.command_encoders.read()),
        ("shader modules", counters.shader_modules.read()),
        ("query sets", counters.query_sets.read()),
        ("fences", counters.fences.read()),
    ], counters.buffer_memory.read(), counters.texture_memory.read(), counters.memory_allocations.read()));
    lines
}

/// The kinds of allocations that the report names one by one, the largest first; the rest is
/// one sum.
const KINDS_SHOWN: usize = 16;

/// The kind of an allocation: its name (the object's label, or wgpu's "Unlabeled buffer") with
/// each run of digits as `#`, so that the slots and the pages of one kind are one kind.
fn allocation_kind(name: &str) -> String {
    let mut kind = String::with_capacity(name.len());
    let mut in_digits = false;
    for character in name.trim().chars() {
        if character.is_ascii_digit() {
            if !in_digits {
                kind.push('#');
            }
            in_digits = true;
        } else {
            kind.push(character);
            in_digits = false;
        }
    }
    if kind.is_empty() { "(no name)".to_string() } else { kind }
}

fn megabytes(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

/// The allocator's part of the report, from the `(name, bytes)` of each live allocation, the
/// count of the memory blocks, and the totals: one line of the totals, then one line for each
/// kind, the largest first.
fn allocator_lines(allocations: &[(&str, u64)], blocks: usize, allocated: u64, reserved: u64) -> Vec<String> {
    let mut kinds: std::collections::BTreeMap<String, (u64, u32)> = std::collections::BTreeMap::new();
    for &(name, size) in allocations {
        let entry = kinds.entry(allocation_kind(name)).or_default();
        entry.0 += size;
        entry.1 += 1;
    }
    let mut sorted: Vec<(String, (u64, u32))> = kinds.into_iter().collect();
    sorted.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then_with(|| a.0.cmp(&b.0)));
    let mut lines = vec![format!(
        "memory: wgpu's allocator: {:.1} MB in {} allocations, {:.1} MB reserved in {} blocks ({:.1} MB free in them), {} kinds",
        megabytes(allocated),
        allocations.len(),
        megabytes(reserved),
        blocks,
        megabytes(reserved.saturating_sub(allocated)),
        sorted.len()
    )];
    for (kind, (bytes, count)) in sorted.iter().take(KINDS_SHOWN) {
        lines.push(format!("memory:   {:.1} MB in {count}: {kind}", megabytes(*bytes)));
    }
    if sorted.len() > KINDS_SHOWN {
        let (bytes, count) = sorted[KINDS_SHOWN..].iter().fold((0u64, 0u32), |sum, (_, (bytes, count))| (sum.0 + bytes, sum.1 + count));
        lines.push(format!("memory:   {:.1} MB in {count}: the {} other kinds", megabytes(bytes), sorted.len() - KINDS_SHOWN));
    }
    lines
}

/// The counts of wgpu's objects (the names and values of `counters`), and the memory of the
/// buffers and the textures and the count of the device memory allocations.
fn counter_line(counts: &[(&str, isize)], buffer_memory: isize, texture_memory: isize, allocations: isize) -> String {
    let objects: Vec<String> = counts.iter().map(|(name, value)| format!("{name} {value}")).collect();
    format!(
        "memory: wgpu's objects: {}; buffer memory {:.1} MB, texture memory {:.1} MB, {allocations} device memory allocations",
        objects.join(", "),
        buffer_memory as f64 / (1024.0 * 1024.0),
        texture_memory as f64 / (1024.0 * 1024.0)
    )
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    #[test]
    fn the_kinds_of_allocations_join_their_numbered_slots() {
        assert_eq!(allocation_kind("statics pool 12"), "statics pool #");
        assert_eq!(allocation_kind("texture array 3 page 40"), "texture array # page #");
        assert_eq!(allocation_kind("  Unlabeled buffer "), "Unlabeled buffer");
        assert_eq!(allocation_kind(""), "(no name)");
    }

    #[test]
    fn the_report_sums_each_kind_and_shows_the_largest_first() {
        const MB: u64 = 1024 * 1024;
        let allocations = [("statics pool 1", 64 * MB), ("statics pool 2", 64 * MB), ("shadow map", 16 * MB), ("Unlabeled buffer", 2 * MB)];
        let lines = allocator_lines(&allocations, 3, 146 * MB, 200 * MB);
        assert_eq!(lines[0], "memory: wgpu's allocator: 146.0 MB in 4 allocations, 200.0 MB reserved in 3 blocks (54.0 MB free in them), 3 kinds");
        assert_eq!(lines[1], "memory:   128.0 MB in 2: statics pool #");
        assert_eq!(lines[2], "memory:   16.0 MB in 1: shadow map");
        assert_eq!(lines[3], "memory:   2.0 MB in 1: Unlabeled buffer");
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn the_kinds_after_the_shown_ones_are_one_sum() {
        let names: Vec<String> = (0..KINDS_SHOWN + 3).map(|index| format!("kind {}", char::from(b'a' + index as u8))).collect();
        let allocations: Vec<(&str, u64)> = names.iter().enumerate().map(|(index, name)| (name.as_str(), 1000 - index as u64)).collect();
        let lines = allocator_lines(&allocations, 1, 0, 0);
        assert_eq!(lines.len(), 1 + KINDS_SHOWN + 1);
        assert!(lines.last().unwrap().ends_with(": the 3 other kinds"), "{}", lines.last().unwrap());
        assert!(lines.last().unwrap().contains(" in 3: "));
    }

    /// A real device (Vulkan, else any): the report names two labelled objects by their kind
    /// and counts them. Needs a GPU: `cargo test -p nirender-host -- --ignored memory`.
    #[test]
    #[ignore = "needs a GPU"]
    fn a_device_reports_its_allocations_by_kind_and_counts_its_objects() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("an adapter");
        let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("a device");
        let _buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some("memory test pool 7"), size: 4 << 20, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
        let _texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("memory test page 3"),
            size: wgpu::Extent3d { width: 256, height: 256, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let lines = memory_lines(&device);
        for line in &lines {
            println!("{line}");
        }
        assert!(lines.iter().any(|line| line.ends_with(": memory test pool #")), "{lines:?}");
        assert!(lines.iter().any(|line| line.ends_with(": memory test page #")), "{lines:?}");
        // wgpu's own buffers count too (its zero-init and staging buffers).
        let objects = lines.last().unwrap();
        let buffers: isize = objects.split("buffers ").nth(1).and_then(|rest| rest.split(',').next()).and_then(|n| n.parse().ok()).unwrap_or(0);
        assert!(buffers >= 1 && !objects.contains("texture memory 0.0 MB"), "{objects}");
    }

    #[test]
    fn the_counter_line_names_each_count() {
        let line = counter_line(&[("buffers", 12), ("bind groups", 3)], 2 * 1024 * 1024, 0, 7);
        assert_eq!(line, "memory: wgpu's objects: buffers 12, bind groups 3; buffer memory 2.0 MB, texture memory 0.0 MB, 7 device memory allocations");
    }
}
