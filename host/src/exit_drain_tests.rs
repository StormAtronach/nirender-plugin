//! The drain of the log pipe at each end of the live helper (W217): the last line before the
//! end reaches the log. The ends are the end of the live mode (`cli::end_of_live`, then
//! `ExitProcess`), `gpu::exit_now`, the panic hook, the exception filter and the watchdog's
//! stall (`watchdog::stalled` with `crash::stall_dump_and_end`), with the exit codes 30 and 33.
//!
//! Each end runs in a child process of the test program. The child's log pipe writes into a
//! named pipe of the parent test (the sink), which holds the writes: the parent reads nothing
//! until [`HOLD`] after the child's connect. The child first writes more than the sink holds,
//! so that its log writer waits in a write to the sink, then the last line, then it ends. The
//! last line reaches the parent only when the end waits for the writer: a drain. The
//! `ExitProcess` and `TerminateProcess` of an end without a drain drop it.

use std::io::Read;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

/// The variable that gives the child the name of the sink: a word between the test and its
/// child, not a switch of the program, so not of the form `NIRENDER_*` (check_readme.py).
const SINK_VARIABLE: &str = "EXIT_DRAIN_TEST_SINK";
/// The room of the sink: far less than the text that the child writes before its last line.
const SINK_BYTES: u32 = 4096;
/// The time that the parent holds the sink's writes after the child's connect.
const HOLD: Duration = Duration::from_millis(500);
/// The line that the child writes just before its end.
const LAST_LINE: &str = "W217: the last line before the end";
/// The longest time of a child: the hold, the drain and a minidump.
const CHILD_SECONDS: u64 = 30;
/// The exit code of the end of the live mode in the test: `ExitProcess` with it after
/// `cli::end_of_live`, as `main` returns after `cli::run_with`.
const LIVE_END: u32 = 52;
/// The exit code with which the panic test ends its child after the panic hook returned.
const HOOK_RETURNED: u32 = 53;
/// EXCEPTION_ACCESS_VIOLATION.
const ACCESS_VIOLATION: u32 = 0xC000_0005;

const PIPE_ACCESS_INBOUND: u32 = 0x1;
const GENERIC_WRITE: u32 = 0x4000_0000;
const OPEN_EXISTING: u32 = 3;
const INVALID_HANDLE: isize = -1;

#[link(name = "kernel32")]
extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        instances: u32,
        out_size: u32,
        in_size: u32,
        timeout: u32,
        attributes: *const core::ffi::c_void,
    ) -> isize;
    fn ConnectNamedPipe(pipe: isize, overlapped: *mut core::ffi::c_void) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        attributes: *const core::ffi::c_void,
        disposition: u32,
        flags: u32,
        template: isize,
    ) -> isize;
    fn CloseHandle(handle: isize) -> i32;
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentProcess() -> isize;
    fn TerminateProcess(process: isize, code: u32) -> i32;
    fn RaiseException(code: u32, flags: u32, count: u32, arguments: *const usize);
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// What the parent saw of one end: the child's exit code and the text of the sink.
struct Ended {
    code: Option<u32>,
    sink: String,
}

/// Runs the ignored test `name` of this module in a child process with a sink that holds its
/// writes for `hold`, and gives the child's exit code and the sink's text.
fn end_in_child(name: &str, hold: Duration) -> Ended {
    let pipe = format!(r"\\.\pipe\nirender-w217-{}-{name}", std::process::id());
    // SAFETY: the name ends with a zero; one instance, synchronous, no security attributes.
    let server = unsafe {
        CreateNamedPipeW(
            wide(&pipe).as_ptr(),
            PIPE_ACCESS_INBOUND,
            0,
            1,
            0,
            SINK_BYTES,
            0,
            core::ptr::null(),
        )
    };
    assert_ne!(server, INVALID_HANDLE, "the sink {pipe}");

    let module = module_path!().split_once("::").map_or(module_path!(), |(_, rest)| rest);
    let output = std::env::temp_dir().join(format!("nirender-host-{name}-{}.txt", std::process::id()));
    let file = std::fs::File::create(&output).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            format!("{module}::{name}").as_str(),
            "--exact",
            "--ignored",
            "--test-threads=1",
            "--nocapture",
        ])
        .env(SINK_VARIABLE, &pipe)
        .stdout(file.try_clone().unwrap())
        .stderr(file)
        .spawn()
        .unwrap();
    let child_id = child.id();

    // The reader: the connect of the child, the hold, then every byte until the pipe breaks
    // at the child's end.
    let reader = std::thread::spawn(move || {
        // SAFETY: the server handle of this test; a client that connected before the call
        // gives ERROR_PIPE_CONNECTED, which is a connect too.
        unsafe { ConnectNamedPipe(server, core::ptr::null_mut()) };
        std::thread::sleep(hold);
        // SAFETY: the server handle of this test; the file owns it from here and closes it.
        let mut sink = unsafe { std::fs::File::from_raw_handle(server as RawHandle) };
        // The read ends at the broken pipe of the child's end, which the standard library
        // takes as the end of the file; the bytes before an error stay.
        let mut bytes = Vec::new();
        let _ = sink.read_to_end(&mut bytes);
        bytes
    });

    let status = crate::crash::tests::end_of_child(child, CHILD_SECONDS);
    // A child that ended before its connect leaves the reader in its connect: a connect of
    // this test ends it, and the sink is empty. With the child connected, the one instance is
    // busy, and this open fails.
    // SAFETY: the name ends with a zero; the handle is closed at once.
    let client = unsafe { CreateFileW(wide(&pipe).as_ptr(), GENERIC_WRITE, 0, core::ptr::null(), OPEN_EXISTING, 0, 0) };
    if client != INVALID_HANDLE {
        // SAFETY: the handle of the open above.
        unsafe { CloseHandle(client) };
    }
    let sink = String::from_utf8_lossy(&reader.join().unwrap()).into_owned();
    let child_output = std::fs::read_to_string(&output).unwrap_or_default();
    std::fs::remove_file(&output).ok();
    std::fs::remove_file(dump_of(child_id)).ok();
    assert!(status.is_some(), "{name} did not end in {CHILD_SECONDS} s: {child_output}");
    Ended {
        code: status.and_then(|status| status.code()).map(|code| code as u32),
        sink,
    }
}

/// The minidump of the child `child`, for the ends that write one.
fn dump_of(child: u32) -> PathBuf {
    std::env::temp_dir().join(format!("nirender-host-w217-{child}.dmp"))
}

/// The start of each child: its log pipe into the parent's sink, more text than the sink holds
/// (the log writer then waits in a write to the sink), then the last line.
fn start_in_child() {
    let pipe = std::env::var(SINK_VARIABLE).expect("the name of the sink");
    // SAFETY: the name ends with a zero; the handle stays open for the child's life.
    let sink = unsafe { CreateFileW(wide(&pipe).as_ptr(), GENERIC_WRITE, 0, core::ptr::null(), OPEN_EXISTING, 0, 0) };
    assert_ne!(sink, INVALID_HANDLE, "the sink {pipe}");
    crate::log_pipe::start(sink);
    for n in 0..1000 {
        println!("line {n:04} before the end, more than the sink holds");
    }
    println!("{LAST_LINE}");
}

/// The end of a child that its own code ends: `TerminateProcess` with `code`.
fn end_child_with(code: u32) -> ! {
    // SAFETY: the handle of the own process; nothing runs after the call.
    unsafe { TerminateProcess(GetCurrentProcess(), code) };
    unreachable!("TerminateProcess of the own process returns")
}

/// A thread that waits for ever, in place of the frame thread of a stall.
fn waiting_thread() -> u32 {
    let (sent, got) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // SAFETY: a plain read of the own thread's id.
        let _ = sent.send(unsafe { GetCurrentThreadId() });
        loop {
            std::thread::park();
        }
    });
    got.recv().unwrap()
}

/// The checks of an end: the exit code, the last line, and the end's own lines.
fn assert_ended(ended: &Ended, code: u32, lines: &[&str]) {
    let tail: String = ended
        .sink
        .chars()
        .rev()
        .take(2000)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    assert_eq!(ended.code, Some(code), "the exit code; the end of the sink:\n{tail}");
    assert!(
        ended.sink.contains(LAST_LINE),
        "the last line did not reach the log; the end of the sink:\n{tail}"
    );
    for line in lines {
        assert!(
            ended.sink.contains(line),
            "no '{line}' in the log; the end of the sink:\n{tail}"
        );
    }
}

#[test]
fn the_end_of_the_live_mode_drains_the_log() {
    let ended = end_in_child("the_end_of_the_live_mode_drains_the_log_in_this_process", HOLD);
    assert_ended(&ended, LIVE_END, &[]);
}

#[test]
#[ignore = "ends its process: run alone by the_end_of_the_live_mode_drains_the_log"]
fn the_end_of_the_live_mode_drains_the_log_in_this_process() {
    start_in_child();
    crate::cli::end_of_live();
    std::process::exit(LIVE_END as i32);
}

#[test]
fn exit_now_drains_the_log() {
    let ended = end_in_child("exit_now_drains_the_log_in_this_process", HOLD);
    assert_ended(&ended, crate::stream_format::EXIT_GPU, &[]);
}

#[test]
#[ignore = "ends its process: run alone by exit_now_drains_the_log"]
fn exit_now_drains_the_log_in_this_process() {
    start_in_child();
    crate::gpu::exit_now(crate::stream_format::EXIT_GPU);
}

#[test]
fn the_panic_hook_drains_the_log() {
    let ended = end_in_child("the_panic_hook_drains_the_log_in_this_process", HOLD);
    assert_ended(&ended, HOOK_RETURNED, &["panic: ", "W217 panic"]);
}

#[test]
#[ignore = "ends its process: run alone by the_panic_hook_drains_the_log"]
fn the_panic_hook_drains_the_log_in_this_process() {
    crate::crash::install(dump_of(std::process::id()));
    start_in_child();
    let _ = std::panic::catch_unwind(|| panic!("W217 panic"));
    end_child_with(HOOK_RETURNED);
}

#[test]
fn the_exception_filter_drains_the_log() {
    let ended = end_in_child("the_exception_filter_drains_the_log_in_this_process", HOLD);
    assert_ended(&ended, ACCESS_VIOLATION, &["crash: exception 0xc0000005"]);
}

#[test]
#[ignore = "crashes its process: run alone by the_exception_filter_drains_the_log"]
fn the_exception_filter_drains_the_log_in_this_process() {
    crate::crash::install(dump_of(std::process::id()));
    start_in_child();
    // SAFETY: an exception that nobody handles: the filter of crash::install ends the process.
    unsafe { RaiseException(ACCESS_VIOLATION, 1, 0, core::ptr::null()) };
}

#[test]
fn a_stall_drains_the_log() {
    let ended = end_in_child("a_stall_drains_the_log_in_this_process", HOLD);
    assert_ended(
        &ended,
        crate::stream_format::EXIT_STALLED,
        &["stalled for 5 s:", "the exit code 30", "minidump"],
    );
}

#[test]
#[ignore = "ends its process: run alone by a_stall_drains_the_log"]
fn a_stall_drains_the_log_in_this_process() {
    crate::crash::install(dump_of(std::process::id()));
    let frame_thread = waiting_thread();
    start_in_child();
    crate::watchdog::stalled(frame_thread, 5);
}

/// The stall while the disk holds the log writer's write (W580): the sink holds it for longer
/// than `DISK_HELD_SECONDS`, the exit code is 33, and the drain still brings the last line.
#[test]
fn a_stall_on_the_disk_drains_the_log() {
    let held = Duration::from_secs(crate::stream_format::DISK_HELD_SECONDS) + Duration::from_secs(1);
    let ended = end_in_child("a_stall_on_the_disk_drains_the_log_in_this_process", held);
    assert_ended(
        &ended,
        crate::stream_format::EXIT_STALLED_DISK,
        &["a write to the disk has waited", "the exit code 33", "minidump"],
    );
}

#[test]
#[ignore = "ends its process: run alone by a_stall_on_the_disk_drains_the_log"]
fn a_stall_on_the_disk_drains_the_log_in_this_process() {
    crate::crash::install(dump_of(std::process::id()));
    let frame_thread = waiting_thread();
    start_in_child();
    // The stall comes when the log writer's write has waited for the watchdog's limit.
    let limit = Duration::from_secs(crate::stream_format::DISK_HELD_SECONDS);
    let deadline = Instant::now() + limit + Duration::from_secs(5);
    while crate::log_pipe::write_held().is_none_or(|held| held < limit) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    crate::watchdog::stalled(frame_thread, 5);
}
