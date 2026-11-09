//! The log of the live program goes through a pipe. Its stdout and stderr write into a pipe
//! with room for [`PIPE_BYTES`], and a thread of its own (the log writer) copies the pipe into
//! the log file. A print of the frame thread then copies into memory and returns, also while
//! the disk holds the writes of the file: on 2026-11-09 a busy hard disk held one line of the
//! frame thread for more than 4 s, and the watchdog ended the program twice (the frame thread
//! in `NtWriteFile` in all 16 samples).
//!
//! `TerminateProcess` and `ExitProcess` end the writer with the lines that are still in the
//! pipe, so each end of the program drains the pipe first ([`drain`], with a limit). A thread
//! that must write a line while another thread can hold the lock of stdout (the watchdog,
//! after a stall) writes with [`write_direct`], which takes no lock of the standard library.
//! The ends after a panic or a crash write their lines with [`ExitLines`] for the same reason.
//!
//! The pipe is a named pipe whose read end is opened for overlapped I/O. Windows lets only one
//! call at a time into a handle opened for synchronous I/O: a look into the pipe
//! (`PeekNamedPipe`) by the drain or by the watchdog would wait for the writer's read, which
//! waits for the next line. The tests found this with an anonymous pipe (`CreatePipe`), whose
//! ends are synchronous. The write end stays synchronous, as stdout needs. For the same reason
//! [`write_direct`] never writes into the log file: the writer thread can wait in a write there.
//!
//! Only the live mode uses the pipe (`cli::run_with`): a replay has no frame to keep, and it
//! keeps the plain file.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// The room of the pipe: the text of a busy trace (the skin trace, about 60 KB a second) for
/// more than two minutes of a disk that holds the writes.
pub(crate) const PIPE_BYTES: u32 = 8 << 20;
/// The longest wait of an end of the program for the writer to empty the pipe.
pub(crate) const DRAIN_LIMIT: Duration = Duration::from_secs(2);
/// The time between two looks of [`drain`].
const DRAIN_LOOK_MS: u32 = 2;
/// The room that [`write_direct`] leaves free in the pipe. A print that does not fit into the
/// rest of the pipe waits inside the write end, and a second write there waits behind it. The
/// prints of the program are far shorter than this \[I\].
const DIRECT_MARGIN: u64 = 64 * 1024;

const STD_OUTPUT: u32 = -11i32 as u32;
const STD_ERROR: u32 = -12i32 as u32;
const PIPE_ACCESS_INBOUND: u32 = 0x1;
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x8_0000;
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
/// PIPE_TYPE_BYTE, PIPE_READMODE_BYTE and PIPE_WAIT are 0.
const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x8;
const GENERIC_WRITE: u32 = 0x4000_0000;
const OPEN_EXISTING: u32 = 3;
const INVALID_HANDLE: isize = -1;
const ERROR_IO_PENDING: u32 = 997;
const STATUS_PENDING: usize = 0x103;

/// OVERLAPPED of the 64-bit Windows API.
#[repr(C)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: isize,
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateNamedPipeW(name: *const u16, open_mode: u32, pipe_mode: u32, instances: u32, out_size: u32, in_size: u32, timeout: u32, attributes: *const core::ffi::c_void) -> isize;
    fn CreateFileW(name: *const u16, access: u32, share: u32, attributes: *const core::ffi::c_void, disposition: u32, flags: u32, template: isize) -> isize;
    fn CreateEventW(attributes: *const core::ffi::c_void, manual_reset: i32, initial: i32, name: *const u16) -> isize;
    fn ReadFile(file: isize, buffer: *mut u8, length: u32, read: *mut u32, overlapped: *mut Overlapped) -> i32;
    fn GetOverlappedResult(file: isize, overlapped: *mut Overlapped, transferred: *mut u32, wait: i32) -> i32;
    fn WriteFile(file: isize, buffer: *const u8, length: u32, written: *mut u32, overlapped: *mut Overlapped) -> i32;
    fn PeekNamedPipe(pipe: isize, buffer: *mut u8, size: u32, read: *mut u32, available: *mut u32, left: *mut u32) -> i32;
    fn GetLastError() -> u32;
    fn SetStdHandle(which: u32, handle: *mut core::ffi::c_void) -> i32;
    fn GetStdHandle(which: u32) -> isize;
    fn CloseHandle(handle: isize) -> i32;
    fn Sleep(milliseconds: u32);
}

/// The read of the writer thread. The kernel writes the state of the read into it while the
/// read waits, so it lives in an [`Arc`] that the writer thread holds until its last read ends.
struct PendingRead {
    overlapped: UnsafeCell<Overlapped>,
}

// SAFETY: the writer thread starts the reads; other threads only read the state word, which
// the kernel writes.
unsafe impl Send for PendingRead {}
// SAFETY: as above.
unsafe impl Sync for PendingRead {}

impl PendingRead {
    /// True while the writer waits in a read for the next bytes: it holds no bytes then.
    fn waits(&self) -> bool {
        // SAFETY: a read of the state word, which the kernel writes while the read waits.
        unsafe { core::ptr::addr_of!((*self.overlapped.get()).internal).read_volatile() == STATUS_PENDING }
    }
}

impl Drop for PendingRead {
    fn drop(&mut self) {
        // SAFETY: the event belongs to this read, and no read uses it any more.
        unsafe { CloseHandle(self.overlapped.get_mut().event) };
    }
}

/// A pipe in front of a file.
pub(crate) struct LogPipe {
    /// The read end: overlapped, so that a look into the pipe never waits for the writer.
    read: isize,
    /// The write end: synchronous, for stdout and stderr.
    write: isize,
    file: isize,
    room: u32,
    reading: Arc<PendingRead>,
}

/// The number of the next pipe of this process, for its name.
static NEXT_PIPE: AtomicU32 = AtomicU32::new(0);

impl LogPipe {
    /// A pipe with room for `room` bytes in front of the file handle `file`. The writer thread
    /// does not run yet ([`LogPipe::spawn_writer`]). None when the pipe cannot be made.
    pub(crate) fn new(file: isize, room: u32) -> Option<LogPipe> {
        let name = format!(r"\\.\pipe\nirender-log-{}-{}", std::process::id(), NEXT_PIPE.fetch_add(1, Ordering::Relaxed));
        let name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let read_mode = PIPE_ACCESS_INBOUND | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE;
        // SAFETY: the name ends with a zero; one instance, no security attributes.
        let read = unsafe { CreateNamedPipeW(name.as_ptr(), read_mode, PIPE_REJECT_REMOTE_CLIENTS, 1, 0, room, 0, core::ptr::null()) };
        if read == INVALID_HANDLE {
            return None;
        }
        // SAFETY: the name ends with a zero; the pipe of that name was made above.
        let write = unsafe { CreateFileW(name.as_ptr(), GENERIC_WRITE, 0, core::ptr::null(), OPEN_EXISTING, 0, 0) };
        // SAFETY: a manual-reset event without a name, as an overlapped read needs.
        let event = unsafe { CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()) };
        if write == INVALID_HANDLE || event == 0 {
            // SAFETY: the handles that were made above.
            unsafe {
                CloseHandle(read);
                if write != INVALID_HANDLE {
                    CloseHandle(write);
                }
            }
            return None;
        }

        let overlapped = Overlapped { internal: 0, internal_high: 0, offset: 0, offset_high: 0, event };
        let reading = Arc::new(PendingRead { overlapped: UnsafeCell::new(overlapped) });
        Some(LogPipe { read, write, file, room, reading })
    }

    /// Starts the thread that copies the pipe into the file. False when the thread cannot start.
    pub(crate) fn spawn_writer(&self) -> bool {
        let (read, file, reading) = (self.read, self.file, self.reading.clone());
        std::thread::Builder::new().name("log-writer".into()).spawn(move || copy_out(read, file, &reading)).is_ok()
    }

    /// The bytes in the pipe now.
    fn waiting(&self) -> u64 {
        let mut available = 0u32;
        // SAFETY: a look into the read end of the own pipe that reads no bytes.
        let looked = unsafe { PeekNamedPipe(self.read, core::ptr::null_mut(), 0, core::ptr::null_mut(), &mut available, core::ptr::null_mut()) };
        if looked == 0 {
            0
        } else {
            u64::from(available)
        }
    }

    /// The pipe is empty and the writer holds no bytes. The look into the pipe comes first:
    /// bytes that arrive between the two looks end the writer's read, and the second look sees
    /// that.
    fn idle(&self) -> bool {
        self.waiting() == 0 && self.reading.waits()
    }

    /// Waits until the writer has put every byte of the pipe into the file, for at most
    /// `limit`. False when the limit came first: the disk still holds a write.
    pub(crate) fn drain(&self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            if self.idle() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            // SAFETY: a plain sleep.
            unsafe { Sleep(DRAIN_LOOK_MS) };
        }
    }

    /// Writes `bytes` behind the lines in the pipe, so that the order of the lines stays. When
    /// the pipe is nearly full, the line is dropped and the result is false: a write into a
    /// full pipe, or into the file, can wait for the disk.
    pub(crate) fn write_direct(&self, bytes: &[u8]) -> bool {
        if self.waiting() + bytes.len() as u64 + DIRECT_MARGIN > u64::from(self.room) {
            return false;
        }
        write_all(self.write, bytes);
        true
    }

    /// [`LogPipe::write_direct`], but a line without room waits for room until `deadline`.
    fn write_direct_by(&self, bytes: &[u8], deadline: Instant) -> bool {
        loop {
            if self.write_direct(bytes) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            // SAFETY: a plain sleep.
            unsafe { Sleep(DRAIN_LOOK_MS) };
        }
    }
}

/// The writer thread: it copies the pipe into the file until the pipe breaks.
fn copy_out(read: isize, file: isize, reading: &PendingRead) {
    let overlapped = reading.overlapped.get();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        // SAFETY: the buffer and the OVERLAPPED live until the read ends: the wait below
        // returns only then.
        let started = unsafe { ReadFile(read, buffer.as_mut_ptr(), buffer.len() as u32, core::ptr::null_mut(), overlapped) } != 0;
        // SAFETY: a plain read of the thread's last error.
        if !started && unsafe { GetLastError() } != ERROR_IO_PENDING {
            return;
        }
        let mut got = 0u32;
        // SAFETY: the read started above with this OVERLAPPED; the call waits for its end.
        let read_ok = unsafe { GetOverlappedResult(read, overlapped, &mut got, 1) } != 0;
        if !read_ok || got == 0 {
            return;
        }
        write_all(file, &buffer[..got as usize]);
    }
}

/// Writes all of `bytes` to the handle, in as many calls as it takes. A failed call ends the
/// write: the log has no other place for the text.
fn write_all(handle: isize, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        let mut done = 0u32;
        // SAFETY: the bytes live for the call; the handle is a file or a pipe of this program.
        let wrote = unsafe { WriteFile(handle, bytes.as_ptr(), bytes.len() as u32, &mut done, core::ptr::null_mut()) } != 0;
        if !wrote || done == 0 {
            return;
        }
        bytes = &bytes[done as usize..];
    }
}

impl Drop for LogPipe {
    fn drop(&mut self) {
        // SAFETY: the handles are the own pipe's; the file belongs to the caller. The read end
        // closes first: it ends the writer's read and breaks the pipe, so that a print that
        // waits in the write end ends too. A close of the synchronous write end waits for a
        // write in it to end \[I\].
        unsafe {
            CloseHandle(self.read);
            CloseHandle(self.write);
        }
    }
}

/// The pipe of the program, after [`start`].
static PIPE: OnceLock<LogPipe> = OnceLock::new();

/// Puts the pipe between stdout and stderr and the log file `file` (the handle that
/// `cli::log_to` gave them), and starts the writer. Without a pipe or a thread, the file stays
/// as it is.
pub(crate) fn start(file: isize) {
    let Some(pipe) = LogPipe::new(file, PIPE_BYTES) else {
        println!("log: no pipe; the log is written by each thread itself");
        return;
    };
    if !pipe.spawn_writer() {
        println!("log: no writer thread; the log is written by each thread itself");
        return;
    }
    let write = pipe.write;
    if PIPE.set(pipe).is_err() {
        return;
    }
    // SAFETY: the handle is the write end of the pipe, which stays open for the program's life.
    unsafe {
        SetStdHandle(STD_OUTPUT, write as _);
        SetStdHandle(STD_ERROR, write as _);
    }
    println!("log: through a pipe of {} MB and the log-writer thread", PIPE_BYTES >> 20);
}

/// Waits, for at most `limit`, until the log file has every line that is in the pipe. True
/// without a pipe. For each end of the program: `TerminateProcess` and `ExitProcess` drop the
/// lines that are still in the pipe.
pub(crate) fn drain(limit: Duration) -> bool {
    PIPE.get().is_none_or(|pipe| pipe.drain(limit))
}

/// Writes `text` into the log without a lock of the standard library: a thread that is held
/// inside a print holds the lock of stdout. Into the pipe, behind its lines; without a pipe,
/// into the handle of stdout. False when the pipe is nearly full and the text is dropped.
pub(crate) fn write_direct(text: &[u8]) -> bool {
    match PIPE.get() {
        Some(pipe) => pipe.write_direct(text),
        None => {
            // SAFETY: a read of the own process's handle of stdout.
            write_all(unsafe { GetStdHandle(STD_OUTPUT) }, text);
            true
        }
    }
}

/// The longest time that the lines of one end of the program wait for room in the pipe, all
/// of them together. The drain after them has its own limit, [`DRAIN_LIMIT`].
pub(crate) const EXIT_LINES_WAIT: Duration = Duration::from_secs(1);

/// The lines of an end of the program after a panic or a crash. A print of the standard
/// library takes the lock of stdout, and another thread can hold that lock while it waits in
/// a full pipe for the disk: the end would wait as long as the disk. These lines take no lock
/// ([`write_direct`]). A line waits for room in the pipe until one deadline for all the lines
/// of the end ([`EXIT_LINES_WAIT`]); after it, a line is dropped and counted. The minidump
/// keeps what the dropped lines had.
pub(crate) struct ExitLines {
    deadline: Instant,
    dropped: u32,
}

impl ExitLines {
    pub(crate) fn new() -> ExitLines {
        ExitLines { deadline: Instant::now() + EXIT_LINES_WAIT, dropped: 0 }
    }

    /// Writes `text` and a newline as one write, so that the line stays whole between the
    /// lines of other threads.
    pub(crate) fn line(&mut self, text: &str) {
        let mut bytes = Vec::with_capacity(text.len() + 1);
        bytes.extend_from_slice(text.as_bytes());
        bytes.push(b'\n');
        let written = match PIPE.get() {
            Some(pipe) => pipe.write_direct_by(&bytes, self.deadline),
            None => write_direct(&bytes),
        };
        if !written {
            self.dropped += 1;
        }
    }

    /// The count of the dropped lines last, when the pipe has room for it, then the drain.
    pub(crate) fn end(self) {
        if self.dropped > 0 {
            write_direct(format!("log: {} lines of this end were dropped: the pipe had no room\n", self.dropped).as_bytes());
        }
        drain(DRAIN_LIMIT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sink that holds every write after its first `room` bytes until the test reads it: a
    /// pipe that nobody reads, as a disk that holds the writes. Only the test reads it, so its
    /// synchronous ends wait for nobody else.
    struct HeldSink {
        read: isize,
        write: isize,
    }

    extern "system" {
        fn CreatePipe(read: *mut isize, write: *mut isize, attributes: *const core::ffi::c_void, size: u32) -> i32;
    }

    impl HeldSink {
        fn new(room: u32) -> HeldSink {
            let (mut read, mut write) = (0isize, 0isize);
            // SAFETY: the two handles are written by the call.
            assert_ne!(unsafe { CreatePipe(&mut read, &mut write, core::ptr::null(), room) }, 0, "the sink");
            HeldSink { read, write }
        }

        /// Reads everything that arrives, until `total` bytes came or the time is up.
        fn read_all(&self, total: usize, limit: Duration) -> Vec<u8> {
            let deadline = Instant::now() + limit;
            let mut all = Vec::new();
            let mut buffer = vec![0u8; 64 * 1024];
            while all.len() < total && Instant::now() < deadline {
                let mut available = 0u32;
                // SAFETY: a look into the sink's read end.
                unsafe { PeekNamedPipe(self.read, core::ptr::null_mut(), 0, core::ptr::null_mut(), &mut available, core::ptr::null_mut()) };
                if available == 0 {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
                let mut got = 0u32;
                // SAFETY: the buffer has room for its length; the bytes are there, so the read does not wait.
                unsafe { ReadFile(self.read, buffer.as_mut_ptr(), buffer.len() as u32, &mut got, core::ptr::null_mut()) };
                all.extend_from_slice(&buffer[..got as usize]);
            }
            all
        }
    }

    impl Drop for HeldSink {
        fn drop(&mut self) {
            // SAFETY: the sink's own handles. The read end closes first: it breaks the pipe and
            // ends a write of the log writer that waits in the write end.
            unsafe {
                CloseHandle(self.read);
                CloseHandle(self.write);
            }
        }
    }

    fn lines(count: usize) -> Vec<u8> {
        (0..count).flat_map(|n| format!("line {n:06} of the frame thread\n").into_bytes()).collect()
    }

    /// The fault of 2026-11-09: the disk holds the writes, and a print of the frame thread must
    /// still return at once. 1 MB of lines goes into the pipe while the sink takes 4 KB only.
    #[test]
    fn a_print_returns_while_the_disk_holds_the_writes() {
        let sink = HeldSink::new(4096);
        let pipe = LogPipe::new(sink.write, PIPE_BYTES).expect("the pipe");
        assert!(pipe.spawn_writer());
        let text = lines(30_000);
        // The prints run on a thread of their own: a print that waits for the disk then fails
        // the test, and the read of the sink below lets it go.
        let (done, waiter) = std::sync::mpsc::channel();
        let (write, prints) = (pipe.write, text.clone());
        std::thread::spawn(move || {
            for line in prints.chunks(35) {
                write_all(write, line);
            }
            let _ = done.send(());
        });
        let returned = waiter.recv_timeout(Duration::from_millis(500)).is_ok();
        if !returned {
            sink.read_all(text.len(), Duration::from_secs(10));
        }
        assert!(returned, "{} bytes of prints did not return in 500 ms with the disk held", text.len());
        let began = Instant::now();
        assert!(!pipe.drain(Duration::from_millis(50)), "the drain must not report an empty pipe while the disk holds it");
        assert!(began.elapsed() < Duration::from_millis(500), "the drain kept to its limit");

        // The disk lets go: every byte arrives, in order, and the drain ends.
        let got = sink.read_all(text.len(), Duration::from_secs(10));
        assert_eq!(got.len(), text.len());
        assert!(got == text, "the bytes arrived out of order");
        assert!(pipe.drain(Duration::from_secs(2)));
    }

    /// The second look of `idle`: the pipe can be empty while the writer still holds bytes in
    /// its buffer and waits in a write to the disk. The drain must not end then. The lines go
    /// into the pipe before the writer starts, so that its first read takes all of them (they
    /// fit its buffer), and the held sink stops that write after its first bytes.
    #[test]
    fn the_drain_waits_for_the_bytes_that_the_writer_holds() {
        let sink = HeldSink::new(4096);
        let pipe = LogPipe::new(sink.write, PIPE_BYTES).expect("the pipe");
        let text = lines(1700);
        assert!(text.len() < 64 * 1024 && text.len() > 32 * 1024, "the lines fit one read of the writer");
        write_all(pipe.write, &text);
        assert!(pipe.spawn_writer());

        // The writer reads the pipe empty, then waits in its write to the held sink.
        let deadline = Instant::now() + Duration::from_secs(5);
        while pipe.waiting() != 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        let emptied = pipe.waiting() == 0;
        let drained_early = emptied && pipe.drain(Duration::from_millis(50));

        // The disk lets go: every byte arrives, and the drain ends.
        let got = sink.read_all(text.len(), Duration::from_secs(10));
        assert!(emptied, "the writer did not take the lines out of the pipe");
        assert!(!drained_early, "the drain ended while the writer held bytes");
        assert!(got == text, "{} of {} bytes arrived, or out of order", got.len(), text.len());
        assert!(pipe.drain(Duration::from_secs(2)));
    }

    /// The drain of an idle pipe ends at once, also while the writer waits in its read for the
    /// next line. With an anonymous pipe, this drain waited for ever: the test runs in a child
    /// process, which is ended after a limit.
    #[test]
    fn the_drain_of_an_idle_pipe_ends_at_once() {
        crate::crash::tests::in_child_of(module_path!(), "the_drain_of_an_idle_pipe_ends_at_once_in_this_process");
    }

    #[test]
    #[ignore = "can wait for ever: run alone by the_drain_of_an_idle_pipe_ends_at_once"]
    fn the_drain_of_an_idle_pipe_ends_at_once_in_this_process() {
        let sink = HeldSink::new(1 << 20);
        let pipe = LogPipe::new(sink.write, 1 << 16).expect("the pipe");
        assert!(pipe.spawn_writer());
        let text = lines(100);
        write_all(pipe.write, &text);
        assert!(pipe.drain(Duration::from_secs(2)));
        assert_eq!(sink.read_all(text.len(), Duration::from_secs(2)), text);
        let began = Instant::now();
        assert!(pipe.drain(Duration::from_secs(2)));
        assert!(began.elapsed() < Duration::from_millis(100));
    }

    /// A direct line goes behind the lines in the pipe while the pipe has room. When the pipe
    /// is nearly full, the line is dropped at once: its writer never waits for the disk.
    #[test]
    fn a_direct_line_never_waits_for_a_full_pipe() {
        let sink = HeldSink::new(1 << 20);
        let room = 1 << 17;
        let pipe = LogPipe::new(sink.write, room).expect("the pipe");
        // No writer thread: the pipe fills and stays full.
        assert!(pipe.write_direct(b"first\n"));
        assert_eq!(pipe.waiting(), 6, "a line with room goes into the pipe");
        let filler = vec![b'x'; (u64::from(room) - DIRECT_MARGIN) as usize];
        write_all(pipe.write, &filler);
        let before = pipe.waiting();

        let began = Instant::now();
        assert!(!pipe.write_direct(b"stalled for 5 s\n"), "a line without room is dropped");
        assert!(began.elapsed() < Duration::from_millis(100));
        assert_eq!(pipe.waiting(), before);
        assert!(sink.read_all(1, Duration::from_millis(100)).is_empty(), "nothing goes into the file");
    }

    /// The longest time of an end of the program in the tests below: the lines wait at most
    /// [`EXIT_LINES_WAIT`] and the drain at most [`DRAIN_LIMIT`]; the rest is room for the
    /// minidump and a busy test machine.
    const EXIT_SECONDS: u64 = 20;

    /// The exit code with which the panic test ends its child after the panic hook returned.
    const HOOK_RETURNED: u32 = 42;

    /// Puts the pipe of the program between stdout and a held sink (a disk that holds the
    /// writes), then lets a thread print until it waits inside a print into the full pipe:
    /// that thread holds the lock of stdout from then on. For a test in a child process: the
    /// stdout of the process stays held.
    fn hold_stdout_in_a_full_pipe() {
        let sink = HeldSink::new(4096);
        start(sink.write);
        // The sink stays open for the life of the process: its end would let the writes go.
        std::mem::forget(sink);
        std::thread::spawn(|| loop {
            println!("{}", "x".repeat(100));
        });
        let pipe = PIPE.get().expect("the pipe of the program");
        let deadline = Instant::now() + Duration::from_secs(10);
        while pipe.waiting() + DIRECT_MARGIN <= u64::from(PIPE_BYTES) {
            assert!(Instant::now() < deadline, "the pipe did not fill");
            std::thread::sleep(Duration::from_millis(1));
        }
        // The thread is inside the print that does not fit.
        std::thread::sleep(Duration::from_millis(100));
    }

    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn TerminateProcess(process: isize, code: u32) -> i32;
        fn RaiseException(code: u32, flags: u32, count: u32, arguments: *const usize);
    }

    /// `exit_now` ends the program while another thread holds the lock of stdout in a full
    /// pipe: it takes no lock of stdout, and the drain keeps to its limit.
    #[test]
    fn the_end_of_a_refusal_does_not_wait_for_stdout() {
        crate::crash::tests::in_child_ending_with(
            module_path!(),
            "the_end_of_a_refusal_does_not_wait_for_stdout_in_this_process",
            7,
            EXIT_SECONDS,
        );
    }

    #[test]
    #[ignore = "holds stdout for good: run alone by the_end_of_a_refusal_does_not_wait_for_stdout"]
    fn the_end_of_a_refusal_does_not_wait_for_stdout_in_this_process() {
        hold_stdout_in_a_full_pipe();
        crate::gpu::exit_now(7);
    }

    /// The panic hook writes its lines, the minidump and its report while another thread holds
    /// the lock of stdout in a full pipe, and returns: the lines wait for room until their
    /// deadline, then they are dropped.
    #[test]
    fn the_panic_hook_does_not_wait_for_stdout() {
        let child = crate::crash::tests::in_child_ending_with(
            module_path!(),
            "the_panic_hook_does_not_wait_for_stdout_in_this_process",
            HOOK_RETURNED,
            EXIT_SECONDS,
        );
        std::fs::remove_file(exit_dump(child)).ok();
    }

    #[test]
    #[ignore = "holds stdout for good: run alone by the_panic_hook_does_not_wait_for_stdout"]
    fn the_panic_hook_does_not_wait_for_stdout_in_this_process() {
        crate::crash::install(exit_dump(std::process::id()));
        hold_stdout_in_a_full_pipe();
        let _ = std::panic::catch_unwind(|| panic!("a test of the end after a panic"));
        // SAFETY: the handle of the own process; nothing runs after the call.
        unsafe { TerminateProcess(GetCurrentProcess(), HOOK_RETURNED) };
    }

    /// The exception filter writes its lines, the minidump and its report while another thread
    /// holds the lock of stdout in a full pipe, and the process ends with the code of the
    /// exception. The test raises an access violation with `RaiseException`: the filter gets
    /// the same record as for a fault of the processor.
    #[test]
    fn the_crash_filter_does_not_wait_for_stdout() {
        let child = crate::crash::tests::in_child_ending_with(
            module_path!(),
            "the_crash_filter_does_not_wait_for_stdout_in_this_process",
            ACCESS_VIOLATION,
            EXIT_SECONDS,
        );
        std::fs::remove_file(exit_dump(child)).ok();
    }

    /// EXCEPTION_ACCESS_VIOLATION.
    const ACCESS_VIOLATION: u32 = 0xC000_0005;

    #[test]
    #[ignore = "holds stdout for good and crashes: run alone by the_crash_filter_does_not_wait_for_stdout"]
    fn the_crash_filter_does_not_wait_for_stdout_in_this_process() {
        crate::crash::install(exit_dump(std::process::id()));
        hold_stdout_in_a_full_pipe();
        // SAFETY: an exception that nobody handles: the filter of crash::install ends the process.
        unsafe { RaiseException(ACCESS_VIOLATION, 1, 0, core::ptr::null()) };
    }

    /// The minidump of an end test in the child process `child`.
    fn exit_dump(child: u32) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("nirender-host-exit-test-{child}.dmp"))
    }

    /// The direct write of the program takes no lock of the standard library: it returns while
    /// another thread holds the lock of stdout (the watchdog's line after a stall).
    #[test]
    fn a_direct_line_does_not_wait_for_the_lock_of_stdout() {
        let held = std::io::stdout().lock();
        let (done, waiter) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            write_direct(b"");
            let _ = done.send(());
        });
        let returned = waiter.recv_timeout(Duration::from_secs(2)).is_ok();
        drop(held);
        assert!(returned, "write_direct waited for the lock of stdout");
    }
}
