//! The data of a crash of this program for a player's report: the log of the run before
//! ([`rotate_log`]), the backtrace of a panic in the log, and a minidump of the process.
//!
//! A panic ends the program: the helper's profile has `panic = "abort"`, and the abort of
//! Rust on Windows is a fast fail (`int 29h`, the exit code 0xc0000409) that no exception
//! filter of the process sees. So the panic hook writes the minidump itself, after the
//! message and the backtrace went into the log. A crash that is not a panic (an access
//! violation in a driver, for example) comes to the unhandled exception filter of the
//! process, which writes the same minidump, with the exception in it, and a line in the log.
//! A stack overflow is the exception: the runtime ends the program on its guard page before
//! any filter runs, so it leaves the log line of the runtime and no dump.
//!
//! Before its dump, each path announces the exit code that the program will end with
//! (`live::announce_exit`, the ring word `stream_format::RING_ENDING`): the panic hook
//! [`EXIT_PANIC`], the filter the exception code, the watchdog 30 (`watchdog.rs`). A dump that
//! hangs (a thread suspended inside the loader, the failure-path design, section 6.1) is then
//! ended by the plugin after `ENDING_SECONDS` with that code, so that the box names the crash.
//!
//! The frames of the backtrace are `module+offset` lines. The program ships without its pdb
//! (the owner's rule), so no name is in the log; the pdb of the same build resolves the
//! offsets later, and the minidump opens in a debugger with it.

use core::ffi::c_void;
use std::ffi::OsStr;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use crate::log_pipe::ExitLines;
use crate::stream_format::EXIT_PANIC;

/// EXCEPTION_RECORD.
#[repr(C)]
struct ExceptionRecord {
    code: u32,
    flags: u32,
    record: *mut ExceptionRecord,
    address: *mut c_void,
    parameters: u32,
    information: [usize; 15],
}

/// EXCEPTION_POINTERS.
#[repr(C)]
struct ExceptionPointers {
    record: *mut ExceptionRecord,
    context: *mut c_void,
}

/// MINIDUMP_EXCEPTION_INFORMATION: the minidump structures of dbghelp.h are packed to 4
/// bytes, so the pointer is at byte 4.
#[repr(C, packed(4))]
struct MinidumpExceptionInformation {
    thread: u32,
    pointers: *mut ExceptionPointers,
    client_pointers: i32,
}

/// MINIDUMP_CALLBACK_INFORMATION, packed to 4 bytes as the other minidump structures.
#[repr(C, packed(4))]
struct MinidumpCallbackInformation {
    routine: Option<Callback>,
    parameter: *mut c_void,
}

/// The head of MINIDUMP_CALLBACK_INPUT (packed to 4 bytes): the process id, its handle, the
/// kind of the call; then the data of the kind, for a module its full path first.
#[repr(C, packed(4))]
struct MinidumpCallbackInput {
    process_id: u32,
    process: *mut c_void,
    kind: u32,
    module_path: *const u16,
}

/// MINIDUMP_CALLBACK_OUTPUT for a module: the flags of what the dump writes of it.
#[repr(C, packed(4))]
struct MinidumpCallbackOutput {
    module_write_flags: u32,
}

type Callback = unsafe extern "system" fn(*mut c_void, *const MinidumpCallbackInput, *mut MinidumpCallbackOutput) -> i32;

/// The kinds of the call of the dump writer (MINIDUMP_CALLBACK_TYPE).
const MODULE_CALLBACK: u32 = 0;
const THREAD_CALLBACK: u32 = 1;
const THREAD_EX_CALLBACK: u32 = 2;
const INCLUDE_THREAD_CALLBACK: u32 = 3;
const INCLUDE_MODULE_CALLBACK: u32 = 4;
/// ModuleWriteDataSeg: the data segments of a module go into the dump.
const MODULE_WRITE_DATA_SEG: u32 = 0x2;

/// CONTEXT of x64: 1232 bytes, aligned to 16. The program counter (Rip) is at byte 248.
#[repr(C, align(16))]
struct Context([u8; 1232]);

const RIP_AT: usize = 248;

type Filter = unsafe extern "system" fn(*mut ExceptionPointers) -> i32;

#[link(name = "kernel32")]
extern "system" {
    fn OpenThread(access: u32, inherit: i32, thread_id: u32) -> isize;
    fn SuspendThread(thread: isize) -> u32;
    fn ResumeThread(thread: isize) -> u32;
    fn GetThreadContext(thread: isize, context: *mut Context) -> i32;
    fn RtlCaptureContext(context: *mut Context);
    fn RtlCaptureStackBackTrace(skip: u32, count: u32, frames: *mut *mut c_void, hash: *mut u32) -> u16;
    fn GetModuleHandleExW(flags: u32, address: *const c_void, module: *mut *mut c_void) -> i32;
    fn GetModuleFileNameW(module: *mut c_void, name: *mut u16, size: u32) -> u32;
    fn SetUnhandledExceptionFilter(filter: Option<Filter>) -> *mut c_void;
    fn GetCurrentProcess() -> isize;
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThreadId() -> u32;
    fn GetLastError() -> u32;
    fn TerminateProcess(process: isize, code: u32) -> i32;
    fn Sleep(milliseconds: u32);
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
}

/// LOAD_LIBRARY_SEARCH_SYSTEM32: the load looks in System32 only.
const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x800;

#[link(name = "dbghelp")]
extern "system" {
    fn MiniDumpWriteDump(
        process: *mut c_void,
        process_id: u32,
        file: *mut c_void,
        kind: u32,
        exception: *const MinidumpExceptionInformation,
        user_streams: *const c_void,
        callback: *const c_void,
    ) -> i32;
}

/// GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT.
const MODULE_OF_ADDRESS: u32 = 6;
/// The most frames that `RtlCaptureStackBackTrace` takes on every version of Windows.
const MOST_FRAMES: usize = 62;
/// What the minidump holds: the stacks, the data segments of this program (the callback
/// leaves the other modules' out), the handles, the memory map, the threads' times and the
/// unloaded modules. Not the heap. A player must be able to attach the file to a bug report
/// (25 MB at most on GitHub): with the data segments of every module, the graphics driver's
/// among them, a dump was 40 MB on 2026-10-27, with or without the memory that the stacks
/// point to.
const DUMP_KIND: u32 = 0x1825;
/// The code of the exception that the panic hook puts into the dump: a user code (the error
/// bits, then the letters N, I, R).
const PANIC_EXCEPTION: u32 = 0xE04E_4952;
/// The code of the exception of a stall in the dump of the watchdog (the letters N, I, S).
const STALL_EXCEPTION: u32 = 0xE04E_4953;
/// CONTEXT_FULL of x64, the flags that `GetThreadContext` reads at byte 48 of the context.
const CONTEXT_FULL: u32 = 0x10_000B;
const CONTEXT_FLAGS_AT: usize = 48;
/// CONTEXT_CONTROL of x64: Rip, Rsp and the flags, the least that a sample needs.
const CONTEXT_CONTROL: u32 = 0x10_0001;
/// THREAD_ALL_ACCESS.
const THREAD_ALL_ACCESS: u32 = 0x1F_FFFF;
/// THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION: what a sample needs.
const THREAD_SAMPLE_ACCESS: u32 = 0x2 | 0x8 | 0x40;
/// The samples of the frame thread's instruction pointer that a stall keeps.
pub(crate) const SAMPLES_KEPT: usize = 16;
/// The samples that go into the exception record of the dump: its information has 15 words,
/// the first holds the phase.
const SAMPLES_IN_RECORD: usize = 14;
/// The filter's answer: the program ends.
const EXCEPTION_EXECUTE_HANDLER: i32 = 1;

/// The path of the minidump, set by [`install`].
static DUMP: OnceLock<PathBuf> = OnceLock::new();
/// The file name of this program as UTF-16 in lower case (ASCII), taken at [`install`]: the
/// callback of the dump writer compares the modules with it and allocates nothing.
static PROGRAM: OnceLock<Vec<u16>> = OnceLock::new();
/// The seconds that the stall dump may take: then the program ends without it (the dump
/// writer can wait for a lock that the suspended frame thread holds, \[I\]).
const STALL_DUMP_SECONDS: u32 = 10;
/// For the test of the plugin's limit on a dump that hangs ([`hold_next_dump`]): the next
/// crash path holds where its dump would be.
static HOLD_DUMP: AtomicBool = AtomicBool::new(false);

/// The next crash of this program holds for good where its dump would be, after its log lines
/// and the announcement of its exit, as a dump that a held loader stops does. For a test only
/// (the tune command `crash panic hold` of MGE XE): the plugin must end the program after
/// `stream_format::ENDING_SECONDS` with the announced code.
pub fn hold_next_dump() {
    HOLD_DUMP.store(true, Ordering::Release);
}

/// Holds this thread for good when [`hold_next_dump`] asked for it. A dump that hangs has
/// suspended the watchdog's thread too, so the hold disarms the watchdog: else it would end
/// the program after its limit with the code 30, and the test would not reach the plugin's
/// limit.
fn hold_if_asked() {
    if HOLD_DUMP.load(Ordering::Acquire) {
        crate::watchdog::disarm();
        crate::log_pipe::write_direct(b"crash: the dump is held (a test: hold_next_dump); the plugin ends this program\n");
        loop {
            // SAFETY: a plain sleep.
            unsafe { Sleep(u32::MAX) };
        }
    }
}

/// The exit code that an unhandled exception gives the process: its code, when the record is
/// there.
///
/// # Safety
///
/// `record` is null or points to an exception record that lives for the call.
unsafe fn exit_of_exception(record: *const ExceptionRecord) -> Option<u32> {
    record.as_ref().map(|record| record.code)
}

/// Keeps the file at `path` as `<path>.1` (`nirender-host.log.1` for `nirender-host.log`),
/// in place of the one of the run before that: a player who starts the game again after a
/// crash still has the log of the run that crashed.
pub fn rotate_log(path: &Path) {
    let mut before = path.as_os_str().to_owned();
    before.push(".1");
    // A rename replaces the file of the run before that. When it fails (the file is open
    // in another program), a copy keeps the log at least.
    if std::fs::rename(path, &before).is_err() {
        let _ = std::fs::copy(path, &before);
    }
}

/// The path of the minidump: the name of this program with `.dmp`, in the folder of the log
/// at `log`, or in the current folder without a log.
pub(crate) fn dump_path(log: Option<&str>) -> PathBuf {
    let program = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.file_stem().map(OsStr::to_os_string));
    let mut name = program.unwrap_or_else(|| "nirender-host".into());
    name.push(".dmp");

    let folder = match log.and_then(|log| Path::new(log).parent()) {
        Some(folder) => folder.to_path_buf(),
        None => PathBuf::from("."),
    };
    folder.join(name)
}

/// Puts the panic hook and the exception filter of the process on. A crash writes the
/// minidump at `dump`, after its lines in the log.
pub(crate) fn install(dump: PathBuf) {
    hold_dump_libraries();
    let _ = DUMP.set(dump);
    let program = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.file_name().map(|name| name.to_string_lossy().into_owned()));
    let _ = PROGRAM.set(program.unwrap_or_default().encode_utf16().map(lower).collect());
    std::panic::set_hook(Box::new(|info| {
        // First the exit for the plugin: a dump that hangs is then ended with this code.
        crate::live::announce_exit(EXIT_PANIC);
        let mut lines = ExitLines::new();
        lines.line(&format!("panic: {info}"));
        for line in backtrace() {
            lines.line(&format!("  {line}"));
        }
        hold_if_asked();
        let dump = match DUMP.get() {
            Some(path) => write_dump_here(path).map(|()| path.as_path()),
            None => Err("no path for it: the crash data was not installed".to_string()),
        };
        report(dump, &mut lines);
        lines.end();
    }));
    // SAFETY: the filter is a plain function of this program; it stays for the program's life.
    unsafe { SetUnhandledExceptionFilter(Some(filter)) };
}

/// The DLLs that the dump writer (dbgcore) loads at each dump and unloads after it, on
/// Windows 11 (seen with cdb, 2026-10-27).
const DUMP_LIBRARIES: [&str; 6] = [
    "advapi32.dll",
    "msvcrt.dll",
    "sechost.dll",
    "psapi.dll",
    "version.dll",
    "verifier.dll",
];

/// Loads the DLLs of [`DUMP_LIBRARIES`] for the program's life, so that a dump finds them
/// loaded. `MiniDumpWriteDump` of the own process suspends every other thread, then loads
/// these DLLs. A thread suspended inside the loader (a thread that starts or ends, a DLL
/// that loads) holds the loader, and a load that must wait for it waits forever: the dump
/// never ends, and no thread of the process can end it. The load of a DLL that is already
/// loaded does not wait (a reproducer of 100 dumps a run with 16 threads that start threads,
/// 2026-10-27: 9 of 20 runs hung without this, 0 of 50 with it; with dbghelp and dbgcore
/// alone held, 3 of 10, since the program loads those two at its start in any case). On
/// another Windows version the list can differ; a DLL that is not there is left out, and the
/// test `the_dump_writer_loads_no_dll` names a DLL that the list lacks.
///
/// Each DLL comes from System32 only: version.dll and verifier.dll are not KnownDLLs, and a
/// bare name would find a DLL of that name in the folder of numidium.exe (the game's folder,
/// where version.dll is a common name of a mod's proxy) first.
fn hold_dump_libraries() {
    for name in DUMP_LIBRARIES {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        // SAFETY: the name is a NUL-ended UTF-16 string that lives for the call. The handle
        // is never freed: the DLL stays loaded until the program ends.
        unsafe { LoadLibraryExW(wide.as_ptr(), core::ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
    }
}

/// The unhandled exception filter: the exception and the backtrace go into the log, then the
/// minidump holds the exception, and the program ends.
unsafe extern "system" fn filter(pointers: *mut ExceptionPointers) -> i32 {
    let record = (*pointers).record;
    // First the exit for the plugin: a dump that hangs is then ended with this code.
    if let Some(code) = exit_of_exception(record) {
        crate::live::announce_exit(code);
    }
    let mut lines = ExitLines::new();
    if record.is_null() {
        lines.line("crash: an exception without a record");
    } else {
        lines.line(&format!(
            "crash: exception {:#x} at {}",
            (*record).code,
            place((*record).address as usize)
        ));
    }
    for line in backtrace() {
        lines.line(&format!("  {line}"));
    }
    hold_if_asked();
    let dump = match DUMP.get() {
        Some(path) => write_dump_at(path, GetCurrentThreadId(), pointers).map(|()| path.as_path()),
        None => Err("no path for it: the crash data was not installed".to_string()),
    };
    report(dump, &mut lines);
    // The process ends after the filter: the log pipe's writer puts the crash lines into the
    // file first.
    lines.end();
    EXCEPTION_EXECUTE_HANDLER
}

/// What the watchdog knows of a stalled step: the phase of the frame thread and the last
/// samples of its instruction pointer, the oldest first. A fixed size, so that nothing
/// allocates while the frame thread is suspended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct StallEvidence {
    /// The number of the phase (`watchdog::Phase`).
    pub phase: u8,
    pub count: usize,
    pub samples: [usize; SAMPLES_KEPT],
}

impl StallEvidence {
    /// Keeps `rip` as the newest sample; the oldest goes when [`SAMPLES_KEPT`] are held.
    pub(crate) fn push(&mut self, rip: usize) {
        if self.count < SAMPLES_KEPT {
            self.samples[self.count] = rip;
            self.count += 1;
        } else {
            self.samples.copy_within(1.., 0);
            self.samples[SAMPLES_KEPT - 1] = rip;
        }
    }

    /// The samples held, the oldest first.
    pub(crate) fn held(&self) -> &[usize] {
        &self.samples[..self.count]
    }
}

/// The exception record of a stall dump: the code, the place of the frame thread, and in the
/// information words the phase (word 0) and the newest samples, the oldest first (words 1 to 14).
fn stall_record(rip: usize, evidence: &StallEvidence) -> ExceptionRecord {
    let mut record = ExceptionRecord {
        code: STALL_EXCEPTION,
        flags: 0,
        record: core::ptr::null_mut(),
        address: rip as *mut c_void,
        parameters: 0,
        information: [0; 15],
    };
    let held = evidence.held();
    let newest = &held[held.len().saturating_sub(SAMPLES_IN_RECORD)..];
    record.information[0] = evidence.phase as usize;
    record.information[1..=newest.len()].copy_from_slice(newest);
    record.parameters = 1 + newest.len() as u32;
    record
}

/// The handle with which the frame thread `thread` is sampled, or 0 when it cannot be opened.
pub(crate) fn open_for_samples(thread: u32) -> isize {
    // SAFETY: the id is of a thread of this process; the call has no other argument that can fail
    // in a way that matters (a refused open gives 0).
    unsafe { OpenThread(THREAD_SAMPLE_ACCESS, 0, thread) }
}

/// The instruction pointer of the thread behind `handle` now. The thread stays suspended only
/// for the call of `GetThreadContext`: the function holds no lock, allocates nothing and
/// prints nothing between the suspend and the resume, so that a thread suspended inside an
/// allocation or a print cannot make the caller wait for it.
pub(crate) fn sample_rip(handle: isize) -> Option<usize> {
    if handle == 0 {
        return None;
    }
    let mut context = Context([0; 1232]);
    context.0[CONTEXT_FLAGS_AT..CONTEXT_FLAGS_AT + 4].copy_from_slice(&CONTEXT_CONTROL.to_le_bytes());
    // SAFETY: the handle is of a thread of this process that was opened with the sample access;
    // the context has the size and the alignment of CONTEXT with its flags set. A suspend that
    // fails leaves nothing to resume.
    unsafe {
        if SuspendThread(handle) == u32::MAX {
            return None;
        }
        let taken = GetThreadContext(handle, &mut context) != 0;
        ResumeThread(handle);
        taken.then(|| usize::from_le_bytes(context.0[RIP_AT..RIP_AT + 8].try_into().unwrap_or_default()))
    }
}

/// Writes the minidump with the context of the stalled frame thread `frame_thread` as its
/// exception (the code [`STALL_EXCEPTION`]), from the watchdog thread, so that a debugger
/// opens it on the stalled thread, and ends the program with `exit_code`. The phase and the
/// samples of `evidence` are in the exception record ([`stall_record`]). Everything that
/// allocates comes before the frame thread is suspended (the file, a thread that ends the
/// program after [`STALL_DUMP_SECONDS`]): a thread suspended inside an allocation or a print
/// holds its lock. After the suspend: the dump to the raw handle, one fixed line into the log
/// without a lock ([`crate::log_pipe::write_direct`]), the drain of the log pipe, and
/// `TerminateProcess`.
pub(crate) fn stall_dump_and_end(frame_thread: u32, exit_code: u32, evidence: StallEvidence) -> ! {
    let file = DUMP.get().and_then(|path| std::fs::File::create(path).ok());
    // The last resort: the dump writer may wait for a lock that the suspended thread holds.
    // The drain takes no lock and does not allocate, so the stall lines in the log pipe get to
    // the file also on this path.
    std::thread::spawn(move || {
        // SAFETY: a sleep, then the end of this process.
        unsafe {
            Sleep(STALL_DUMP_SECONDS * 1000);
            crate::log_pipe::drain(crate::log_pipe::DRAIN_LIMIT);
            TerminateProcess(GetCurrentProcess(), exit_code);
        }
    });
    // SAFETY: the id names a thread of this process; the context has the size and the
    // alignment of CONTEXT, with its flags set; the handles live until the process ends.
    unsafe {
        let line: &[u8] = match file.as_ref() {
            None => b"no minidump: the file cannot be made\n",
            Some(file) => {
                let thread = OpenThread(THREAD_ALL_ACCESS, 0, frame_thread);
                let mut context = Context([0; 1232]);
                context.0[CONTEXT_FLAGS_AT..CONTEXT_FLAGS_AT + 4].copy_from_slice(&CONTEXT_FULL.to_le_bytes());
                let taken = thread != 0 && SuspendThread(thread) != u32::MAX && GetThreadContext(thread, &mut context) != 0;
                if !taken {
                    b"no minidump: the context of the frame thread cannot be read\n"
                } else {
                    let rip = usize::from_le_bytes(context.0[RIP_AT..RIP_AT + 8].try_into().unwrap_or_default());
                    let mut record = stall_record(rip, &evidence);
                    let mut pointers = ExceptionPointers {
                        record: &mut record,
                        context: &mut context as *mut Context as *mut c_void,
                    };
                    if dump_to(file.as_raw_handle(), frame_thread, &mut pointers) {
                        b"minidump: written (the frame thread's context)\n"
                    } else {
                        b"no minidump: MiniDumpWriteDump failed\n"
                    }
                }
            }
        };
        // Without a lock and without an allocation: behind the stall lines in the log pipe,
        // then the drain, so that `TerminateProcess` does not drop them.
        crate::log_pipe::write_direct(line);
        crate::log_pipe::drain(crate::log_pipe::DRAIN_LIMIT);
        TerminateProcess(GetCurrentProcess(), exit_code);
    }
    // `TerminateProcess` of this process does not return.
    loop {
        core::hint::spin_loop();
    }
}

/// Writes the minidump of the process at `path` with the place of the caller as its
/// exception, so that a debugger opens it on the stack of this thread: for a panic, which
/// is no exception.
fn write_dump_here(path: &Path) -> Result<(), String> {
    let mut context = Context([0; 1232]);
    // SAFETY: the context has the size and the alignment of CONTEXT.
    unsafe { RtlCaptureContext(&mut context) };
    let rip = usize::from_le_bytes(context.0[RIP_AT..RIP_AT + 8].try_into().expect("8 bytes of the context"));

    let mut record = ExceptionRecord {
        code: PANIC_EXCEPTION,
        flags: 0,
        record: core::ptr::null_mut(),
        address: rip as *mut c_void,
        parameters: 0,
        information: [0; 15],
    };
    let pointers = ExceptionPointers {
        record: &mut record,
        context: &mut context as *mut Context as *mut c_void,
    };
    // SAFETY: a plain read of the id of this thread.
    let thread = unsafe { GetCurrentThreadId() };
    // Another thread writes the dump, so that the stack of this thread stays as the context
    // describes it: a dump of the calling thread with its own context fails (error 998).
    let pointers = &pointers as *const ExceptionPointers as usize;
    std::thread::scope(|scope| {
        scope
            .spawn(|| write_dump_at(path, thread, pointers as *mut ExceptionPointers))
            .join()
            .unwrap_or_else(|_| Err("the thread of the dump panicked".to_string()))
    })
}

/// The frames of the caller's stack, the nearest first, as `module+offset` lines.
fn backtrace() -> Vec<String> {
    let mut frames = [core::ptr::null_mut(); MOST_FRAMES];
    // SAFETY: the array has room for `MOST_FRAMES` addresses, and the call writes at most that many.
    let count = unsafe { RtlCaptureStackBackTrace(1, MOST_FRAMES as u32, frames.as_mut_ptr(), core::ptr::null_mut()) };
    frames[..count as usize]
        .iter()
        .map(|&address| place(address as usize))
        .collect()
}

/// `module+offset` for an address, or the address alone when no module holds it. When the
/// module exports a function at or below the offset, the nearest one follows:
/// `ntdll.dll+0x1664e4 (NtWriteFile+0x14)`.
pub(crate) fn place(address: usize) -> String {
    let mut module = core::ptr::null_mut();
    // SAFETY: the flags ask for the module that holds the address, without a reference count
    // on it.
    let found = unsafe { GetModuleHandleExW(MODULE_OF_ADDRESS, address as *const c_void, &mut module) } != 0;
    if !found {
        return format!("{address:#018x}");
    }

    let mut name = [0u16; 260];
    // SAFETY: the name buffer has room for `MAX_PATH` characters, and the call takes its length.
    let length = unsafe { GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) } as usize;
    let path = String::from_utf16_lossy(&name[..length]);
    let file = path.rsplit(['\\', '/']).next().unwrap_or(&path);
    let offset = address - module as usize;
    match nearest_export(module as usize, offset) {
        Some((export, distance)) => format!("{file}+{offset:#x} ({export}+{distance:#x})"),
        None => format!("{file}+{offset:#x}"),
    }
}

/// The export of the loaded module at `base` that is nearest at or below `offset`, and the
/// distance to it. None when the module has no export directory, or no export at or below the
/// offset. The export need not hold the address: in a function that the module does not
/// export, the result is the export before it.
fn nearest_export(base: usize, offset: usize) -> Option<(String, usize)> {
    // Every read is checked against the size of the image, which the module's headers give.
    let read_u16 = |rva: usize, size: usize| -> Option<u16> {
        // SAFETY: `base` is a loaded module, and `rva + 2` is inside its image.
        (rva + 2 <= size).then(|| unsafe { ((base + rva) as *const u16).read_unaligned() })
    };
    let read_u32 = |rva: usize, size: usize| -> Option<u32> {
        // SAFETY: `base` is a loaded module, and `rva + 4` is inside its image.
        (rva + 4 <= size).then(|| unsafe { ((base + rva) as *const u32).read_unaligned() })
    };

    // The DOS header and the PE headers are at least one page, mapped with the module.
    let headers = 0x1000;
    let pe = read_u32(0x3c, headers)? as usize;
    if read_u32(pe, headers)? != u32::from_le_bytes(*b"PE\0\0") {
        return None;
    }
    let optional = pe + 24;
    let image = read_u32(optional + 56, headers)? as usize;
    // PE32+ (0x20B) has its data directories at 112 bytes into the optional header, PE32 at 96.
    let directories = optional + if read_u16(optional, image)? == 0x20B { 112 } else { 96 };
    let exports = read_u32(directories, image)? as usize;
    let exports_size = read_u32(directories + 4, image)? as usize;
    if exports == 0 {
        return None;
    }

    let names = read_u32(exports + 24, image)? as usize;
    let functions = read_u32(exports + 28, image)? as usize;
    let name_table = read_u32(exports + 32, image)? as usize;
    let ordinal_table = read_u32(exports + 36, image)? as usize;
    let mut best: Option<(usize, usize)> = None;
    for index in 0..names {
        let ordinal = read_u16(ordinal_table + 2 * index, image)? as usize;
        let function = read_u32(functions + 4 * ordinal, image)? as usize;
        // An address inside the export directory is a forwarder: the name of an export of another module.
        let forwarder = (exports..exports + exports_size).contains(&function);
        if forwarder || function > offset || best.is_some_and(|(nearest, _)| function <= nearest) {
            continue;
        }
        best = Some((function, read_u32(name_table + 4 * index, image)? as usize));
    }

    let (function, name) = best?;
    let bytes: Vec<u8> = (name..image.min(name + 256))
        .map_while(|rva| {
            // SAFETY: `rva` is inside the image.
            let byte = unsafe { *((base + rva) as *const u8) };
            (byte != 0).then_some(byte)
        })
        .collect();
    Some((String::from_utf8_lossy(&bytes).into_owned(), offset - function))
}

/// Writes the minidump of the process at `path`, with the exception of the thread `thread`
/// when there is one (`pointers` null: none).
fn write_dump_at(path: &Path, thread: u32, pointers: *mut ExceptionPointers) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|problem| format!("{}: {problem}", path.display()))?;
    if !dump_to(file.as_raw_handle(), thread, pointers) {
        // SAFETY: a plain read of the thread's last error.
        return Err(format!("{}: MiniDumpWriteDump failed, error {}", path.display(), unsafe {
            GetLastError()
        }));
    }
    Ok(())
}

/// Writes the minidump of the process into the open file `file`, with the exception of the
/// thread `thread` when there is one (`pointers` null: none). Allocates nothing itself.
fn dump_to(file: std::os::windows::io::RawHandle, thread: u32, pointers: *mut ExceptionPointers) -> bool {
    // SAFETY: the process is the current one; `pointers` is an exception of `thread` that
    // lives for the call, or null for a dump without an exception; the file is open for
    // writing; the callback is a plain function of this program.
    unsafe {
        let information = MinidumpExceptionInformation {
            thread,
            pointers,
            client_pointers: 0,
        };
        let exception = if pointers.is_null() {
            core::ptr::null()
        } else {
            &information as *const _
        };
        let callback = MinidumpCallbackInformation {
            routine: Some(module_data_of_this_program),
            parameter: core::ptr::null_mut(),
        };
        MiniDumpWriteDump(
            GetCurrentProcess() as *mut c_void,
            GetCurrentProcessId(),
            file,
            DUMP_KIND,
            exception,
            core::ptr::null(),
            &callback as *const _ as *const c_void,
        ) != 0
    }
}

/// The callback of the dump writer: every thread and module goes in, but the data segments
/// of the modules that are not this program stay out of the dump (the graphics driver's
/// DLLs have tens of MB of them).
unsafe extern "system" fn module_data_of_this_program(
    _parameter: *mut c_void,
    input: *const MinidumpCallbackInput,
    output: *mut MinidumpCallbackOutput,
) -> i32 {
    if input.is_null() || output.is_null() {
        return 0;
    }
    match (*input).kind {
        MODULE_CALLBACK => {
            let path = (*input).module_path;
            if !path.is_null() && !is_this_program(path) {
                (*output).module_write_flags &= !MODULE_WRITE_DATA_SEG;
            }
            1
        }
        THREAD_CALLBACK | THREAD_EX_CALLBACK | INCLUDE_THREAD_CALLBACK | INCLUDE_MODULE_CALLBACK => 1,
        _ => 0,
    }
}

/// Whether the module at `path` (a UTF-16 text that ends with 0) is the file of this program.
/// Allocates nothing: it runs in the dump writer, also after the frame thread is suspended.
unsafe fn is_this_program(path: *const u16) -> bool {
    let mut length = 0;
    while *path.add(length) != 0 {
        length += 1;
    }
    let path = core::slice::from_raw_parts(path, length);
    PROGRAM.get().is_some_and(|program| same_file(path, program))
}

/// Whether the file part of `path` (after its last `\\` or `/`) is `program`, which is in
/// lower case: the case of ASCII letters does not count.
fn same_file(path: &[u16], program: &[u16]) -> bool {
    let start = path
        .iter()
        .rposition(|&unit| unit == u16::from(b'\\') || unit == u16::from(b'/'))
        .map_or(0, |at| at + 1);
    let file = &path[start..];
    file.len() == program.len() && file.iter().zip(program).all(|(&unit, &want)| lower(unit) == want)
}

/// The lower case of an ASCII letter in UTF-16; any other unit as it is.
fn lower(unit: u16) -> u16 {
    if (u16::from(b'A')..=u16::from(b'Z')).contains(&unit) {
        unit + 32
    } else {
        unit
    }
}

fn report(dump: Result<&Path, String>, lines: &mut ExitLines) {
    match dump {
        Ok(path) => lines.line(&format!("minidump: {}", path.display())),
        Err(problem) => lines.line(&format!("no minidump: {problem}")),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn a_module_is_this_program_by_its_file_name_in_any_case() {
        let program: Vec<u16> = "numidium.exe".encode_utf16().collect();
        let wide = |text: &str| text.encode_utf16().collect::<Vec<u16>>();
        assert!(same_file(&wide("D:\\Game\\Numidium.EXE"), &program));
        assert!(same_file(&wide("numidium.exe"), &program));
        assert!(!same_file(&wide("D:\\Game\\nvoglv64.dll"), &program));
        assert!(!same_file(&wide("D:\\Game\\numidium.exe.bak"), &program));
    }

    /// The stall of 2026-11-09 sat at NtWriteFile+0x14: the place of that address names the
    /// export. An address of this program, which exports nothing, has no export name.
    #[test]
    fn a_place_in_ntdll_names_the_nearest_export() {
        extern "system" {
            fn GetModuleHandleW(name: *const u16) -> isize;
            fn GetProcAddress(module: isize, name: *const u8) -> *const c_void;
        }
        let ntdll_name: Vec<u16> = "ntdll.dll\0".encode_utf16().collect();
        // SAFETY: ntdll is loaded in every process; both names end with a zero.
        let write_file =
            unsafe { GetProcAddress(GetModuleHandleW(ntdll_name.as_ptr()), b"NtWriteFile\0".as_ptr()) } as usize;
        assert_ne!(write_file, 0);
        let text = place(write_file + 0x14);
        assert!(text.starts_with("ntdll.dll+0x"), "{text}");
        assert!(text.ends_with(" (NtWriteFile+0x14)"), "{text}");

        let here = place(a_place_in_ntdll_names_the_nearest_export as fn() as usize);
        assert!(!here.contains('('), "{here}");
    }

    #[test]
    fn the_evidence_keeps_the_newest_samples_oldest_first() {
        let mut evidence = StallEvidence::default();
        assert!(evidence.held().is_empty());
        for rip in 1..=SAMPLES_KEPT + 3 {
            evidence.push(rip);
        }
        assert_eq!(evidence.held(), &(4..=SAMPLES_KEPT + 3).collect::<Vec<usize>>()[..]);
    }

    #[test]
    fn the_stall_record_carries_the_phase_and_the_newest_14_samples() {
        let mut evidence = StallEvidence {
            phase: 2,
            ..StallEvidence::default()
        };
        for rip in 100..100 + 5 {
            evidence.push(rip);
        }
        let record = stall_record(0xABC, &evidence);
        assert_eq!(
            (record.code, record.address as usize, record.parameters),
            (STALL_EXCEPTION, 0xABC, 6)
        );
        assert_eq!(&record.information[..6], &[2, 100, 101, 102, 103, 104]);
        assert!(record.information[6..].iter().all(|&word| word == 0));
        // 16 samples do not fit the 15 words: the phase and the newest 14.
        let mut full = StallEvidence {
            phase: 6,
            ..StallEvidence::default()
        };
        for rip in 1..=SAMPLES_KEPT {
            full.push(rip);
        }
        let record = stall_record(1, &full);
        assert_eq!(record.parameters, 15);
        assert_eq!(record.information[0], 6);
        assert_eq!((record.information[1], record.information[14]), (3, SAMPLES_KEPT));
    }

    /// The sampler against two real threads: one that spins, one that waits. Prints the time of
    /// a look, which the report of the sampler quotes.
    #[test]
    fn a_sample_of_a_busy_and_of_a_waiting_thread_names_a_module_and_costs_little() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{mpsc, Arc};
        use std::time::{Duration, Instant};

        let stop = Arc::new(AtomicBool::new(false));
        let (id_sender, id_receiver) = mpsc::channel();
        let spinning = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                // SAFETY: a plain read of the id of this thread.
                id_sender.send(unsafe { GetCurrentThreadId() }).unwrap();
                while !stop.load(Ordering::Relaxed) {
                    core::hint::spin_loop();
                }
            })
        };
        let (wait_sender, wait_receiver) = mpsc::channel::<()>();
        let (id_sender_2, id_receiver_2) = mpsc::channel();
        let waiting = std::thread::spawn(move || {
            // SAFETY: a plain read of the id of this thread.
            id_sender_2.send(unsafe { GetCurrentThreadId() }).unwrap();
            let _ = wait_receiver.recv();
        });
        for (name, id) in [
            ("spinning", id_receiver.recv().unwrap()),
            ("waiting", id_receiver_2.recv().unwrap()),
        ] {
            let handle = open_for_samples(id);
            assert_ne!(handle, 0, "the {name} thread opens");
            let mut worst = Duration::ZERO;
            for _ in 0..50 {
                let began = Instant::now();
                let rip = sample_rip(handle).unwrap_or_else(|| panic!("the {name} thread gives its place"));
                worst = worst.max(began.elapsed());
                assert!(place(rip).contains('+'), "{name}: {} names a module", place(rip));
            }
            println!("a look at the {name} thread: the worst of 50 took {worst:?}");
            // Far under a frame (16 ms); the limit leaves room for a busy test machine.
            assert!(worst < Duration::from_millis(20), "{name}: {worst:?}");
        }
        stop.store(true, Ordering::Relaxed);
        drop(wait_sender);
        spinning.join().unwrap();
        waiting.join().unwrap();
        // No thread behind a handle of 0.
        assert_eq!(sample_rip(0), None);
    }

    #[test]
    fn rotate_keeps_the_log_of_the_run_before() {
        let folder = std::env::temp_dir().join(format!("nirender-host-log-rotate-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("nirender-host.log");
        let before = folder.join("nirender-host.log.1");

        // No log yet: nothing to keep.
        rotate_log(&path);
        assert!(!before.exists());

        std::fs::write(&path, "run 1").unwrap();
        rotate_log(&path);
        assert_eq!(std::fs::read_to_string(&before).unwrap(), "run 1");
        assert!(!path.exists());

        // The second rotation replaces the kept log.
        std::fs::write(&path, "run 2").unwrap();
        rotate_log(&path);
        assert_eq!(std::fs::read_to_string(&before).unwrap(), "run 2");

        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn the_dump_goes_beside_the_log() {
        let dump = dump_path(Some("C:/game/nirender-host.log"));
        assert_eq!(dump.parent(), Some(Path::new("C:/game")));
        assert_eq!(dump.extension().and_then(|e| e.to_str()), Some("dmp"));
        assert_eq!(dump_path(None).parent(), Some(Path::new(".")));
    }

    /// The longest time that a test of a dump may take in its child process.
    const CHILD_SECONDS: u64 = 60;

    /// Runs the ignored test `name` of this module alone, in a child process of this test
    /// program, and fails when the test fails, does not run, or takes longer than
    /// [`CHILD_SECONDS`] (the child is then ended).
    ///
    /// A dump of this process suspends every other thread of it while dbghelp writes. Two dumps
    /// in one process (the harness runs tests on parallel threads) can each suspend the other
    /// inside dbghelp, whose functions are single-threaded, and then both wait for ever
    /// (merge_step, 2026-11-03: 14 minutes). In a child of its own a dump shares its process with
    /// no other test. A timer in this process could not end a hang: the dump suspends it too.
    fn in_child(name: &str) {
        in_child_of(module_path!(), name);
    }

    /// [`in_child`] for the ignored test `name` of the test module `module` (its
    /// `module_path!()`).
    pub(crate) fn in_child_of(module: &str, name: &str) {
        let (test, log, child) = start_child(module, name);
        let status = end_of_child(child, CHILD_SECONDS).unwrap_or_else(|| {
            panic!(
                "{test} did not end in {CHILD_SECONDS} s in its child process: it hangs (its output: {})",
                log.display()
            )
        });
        let output = std::fs::read_to_string(&log).unwrap_or_default();
        std::fs::remove_file(&log).ok();
        assert!(status.success(), "{test} failed in its child process:\n{output}");
        assert!(
            output.contains("test result: ok. 1 passed"),
            "{test} did not run in its child process:\n{output}"
        );
    }

    /// Runs the ignored test `name` of the test module `module` alone in a child process, for a
    /// test that ends its process itself. The child must end with the exit code `code` within
    /// `seconds`; else it is ended, and the test fails. Gives the process id of the child.
    pub(crate) fn in_child_ending_with(module: &str, name: &str, code: u32, seconds: u64) -> u32 {
        let (test, log, child) = start_child(module, name);
        let id = child.id();
        let status = end_of_child(child, seconds);
        let output = std::fs::read_to_string(&log).unwrap_or_default();
        std::fs::remove_file(&log).ok();
        let status = status.unwrap_or_else(|| {
            panic!("{test} did not end in {seconds} s in its child process: it waits (its output:\n{output})")
        });
        assert_eq!(
            status.code().map(|exit| exit as u32),
            Some(code),
            "{test}: the exit code of its child process (its output:\n{output})"
        );
        id
    }

    /// Starts this test program for the ignored test `name` of the test module `module` alone.
    /// Gives the full name of the test, the file of the child's output and the child.
    fn start_child(module: &str, name: &str) -> (String, PathBuf, std::process::Child) {
        let module = module.split_once("::").map_or(module, |(_, rest)| rest);
        let test = format!("{module}::{name}");
        // The child's output goes to a file, not a pipe: a pipe that nobody reads while this
        // test waits would stop a child that writes more than the pipe holds.
        let log = std::env::temp_dir().join(format!("nirender-host-{name}-{}.txt", std::process::id()));
        let file = std::fs::File::create(&log).unwrap();
        let program = std::env::current_exe().unwrap();
        let child = std::process::Command::new(program)
            .args([test.as_str(), "--exact", "--ignored", "--test-threads=1", "--nocapture"])
            .stdout(file.try_clone().unwrap())
            .stderr(file)
            .spawn()
            .unwrap();
        (test, log, child)
    }

    /// The exit status of `child` when it ends within `seconds`; else the child is ended, and
    /// the result is none.
    fn end_of_child(mut child: std::process::Child, seconds: u64) -> Option<std::process::ExitStatus> {
        let started = std::time::Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return Some(status);
            }
            if started.elapsed().as_secs() >= seconds {
                child.kill().ok();
                child.wait().ok();
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    #[test]
    fn a_dump_of_this_process_is_written() {
        in_child("a_dump_of_this_process_is_written_in_this_process");
    }

    #[test]
    fn the_dump_writer_loads_no_dll() {
        in_child("the_dump_writer_loads_no_dll_in_this_process");
    }

    /// The other tests run on threads that start while the dump runs: without the DLLs of the
    /// dump held, the dump could wait for the loader forever (see `hold_dump_libraries`).
    #[test]
    #[ignore = "a dump of this process: run alone by a_dump_of_this_process_is_written"]
    fn a_dump_of_this_process_is_written_in_this_process() {
        hold_dump_libraries();
        let folder = std::env::temp_dir().join(format!("nirender-host-dump-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("test.dmp");

        // SAFETY: a plain read of the id of this thread.
        let thread = unsafe { GetCurrentThreadId() };
        write_dump_at(&path, thread, core::ptr::null_mut()).unwrap();
        let head = std::fs::read(&path).unwrap();
        assert_eq!(&head[..4], b"MDMP", "the signature of a minidump");

        // With the place of the caller as the exception (cdb opened such a dump on the stack
        // of the test, 2026-10-27).
        write_dump_here(&path).unwrap();
        assert_eq!(&std::fs::read(&path).unwrap()[..4], b"MDMP");

        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn the_backtrace_names_this_program() {
        let lines = backtrace();
        assert!(!lines.is_empty());
        let module = lines[0].split('+').next().unwrap();
        assert!(module.ends_with(".exe"), "{}", lines[0]);
    }

    #[test]
    fn the_filter_announces_the_code_of_the_exception() {
        let record = ExceptionRecord {
            code: 0xC000_0005,
            flags: 0,
            record: core::ptr::null_mut(),
            address: core::ptr::null_mut(),
            parameters: 0,
            information: [0; 15],
        };
        // SAFETY: the record lives for the calls; null is allowed.
        unsafe {
            assert_eq!(exit_of_exception(&record), Some(0xC000_0005));
            assert_eq!(exit_of_exception(core::ptr::null()), None);
        }
    }

    #[test]
    fn no_dump_is_held_without_the_test_ask() {
        // Without hold_next_dump the check returns at once (the test would hang, not fail).
        assert!(!HOLD_DUMP.load(Ordering::Acquire));
        hold_if_asked();
    }

    #[test]
    fn the_libraries_of_the_dump_stay_loaded() {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetModuleHandleW(name: *const u16) -> isize;
        }
        hold_dump_libraries();
        for name in DUMP_LIBRARIES {
            let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            // SAFETY: a NUL-ended name; the call takes no reference.
            assert_ne!(unsafe { GetModuleHandleW(wide.as_ptr()) }, 0, "{name} is not loaded");
        }
    }

    /// UNICODE_STRING.
    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum: u16,
        buffer: *const u16,
    }

    /// LDR_DLL_NOTIFICATION_DATA: the same head for a load and an unload.
    #[repr(C)]
    struct DllNotification {
        flags: u32,
        full_name: *const UnicodeString,
        base_name: *const UnicodeString,
        base: *mut c_void,
        size: u32,
    }

    type DllNotify = unsafe extern "system" fn(reason: u32, data: *const DllNotification, context: *mut c_void);
    type Register =
        unsafe extern "system" fn(flags: u32, notify: DllNotify, context: *mut c_void, cookie: *mut *mut c_void) -> i32;
    type Unregister = unsafe extern "system" fn(cookie: *mut c_void) -> i32;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleA(name: *const u8) -> isize;
        // The signature of gpu.rs's declaration.
        fn GetProcAddress(module: isize, name: *const u8) -> *const c_void;
    }

    /// A function of ntdll by its name. The import library of ntdll does not have the
    /// notification functions of the loader.
    fn ntdll_function(name: &core::ffi::CStr) -> *const c_void {
        // SAFETY: NUL-ended names; ntdll is in every process.
        let function = unsafe { GetProcAddress(GetModuleHandleA(c"ntdll.dll".as_ptr().cast()), name.as_ptr().cast()) };
        assert!(!function.is_null(), "ntdll has no {name:?}");
        function
    }

    /// LDR_DLL_NOTIFICATION_REASON_LOADED.
    const DLL_LOADED: u32 = 1;
    /// The most loads and the most characters of a name that the recorder keeps.
    const MOST_LOADS: usize = 16;
    const MOST_NAME: usize = 64;

    /// The DLLs loaded on the watched thread: fixed room, since the loader calls the recorder
    /// while the dump writer has the other threads suspended (one of them can hold the heap).
    struct Loads {
        names: [[u16; MOST_NAME]; MOST_LOADS],
        count: usize,
    }

    static LOADS: std::sync::Mutex<Loads> = std::sync::Mutex::new(Loads {
        names: [[0; MOST_NAME]; MOST_LOADS],
        count: 0,
    });
    static WATCHED_THREAD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    unsafe extern "system" fn record_load(reason: u32, data: *const DllNotification, _context: *mut c_void) {
        if reason != DLL_LOADED
            || data.is_null()
            || GetCurrentThreadId() != WATCHED_THREAD.load(std::sync::atomic::Ordering::SeqCst)
        {
            return;
        }
        let name = &*(*data).base_name;
        let length = (usize::from(name.length) / 2).min(MOST_NAME);
        if let Ok(mut loads) = LOADS.lock() {
            if loads.count < MOST_LOADS {
                let at = loads.count;
                loads.names[at][..length].copy_from_slice(core::slice::from_raw_parts(name.buffer, length));
                loads.count += 1;
            }
        }
    }

    /// The dump writer finds every DLL it needs loaded (`hold_dump_libraries`): no DLL loads on
    /// the thread of a dump. A DLL that loads there is one that the list lacks on this
    /// Windows, and the test names it. Without the hold it names the six.
    #[test]
    #[ignore = "a dump of this process: run alone by the_dump_writer_loads_no_dll"]
    fn the_dump_writer_loads_no_dll_in_this_process() {
        hold_dump_libraries();
        let folder = std::env::temp_dir().join(format!("nirender-host-loads-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("test.dmp");

        // SAFETY: a plain read of the id of this thread.
        let thread = unsafe { GetCurrentThreadId() };
        WATCHED_THREAD.store(thread, std::sync::atomic::Ordering::SeqCst);
        // SAFETY: the two functions of ntdll have these signatures (LdrRegisterDllNotification,
        // LdrUnregisterDllNotification, documented by Microsoft).
        let (register, unregister) = unsafe {
            (
                core::mem::transmute::<*const c_void, Register>(ntdll_function(c"LdrRegisterDllNotification")),
                core::mem::transmute::<*const c_void, Unregister>(ntdll_function(c"LdrUnregisterDllNotification")),
            )
        };
        let mut cookie = core::ptr::null_mut();
        // SAFETY: the recorder is a plain function of this program; the registration ends below.
        assert_eq!(unsafe { register(0, record_load, core::ptr::null_mut(), &mut cookie) }, 0);
        let written = write_dump_at(&path, thread, core::ptr::null_mut());
        // SAFETY: the cookie of the registration above.
        unsafe { unregister(cookie) };
        WATCHED_THREAD.store(0, std::sync::atomic::Ordering::SeqCst);
        written.unwrap();

        let loads = LOADS.lock().unwrap();
        let names: Vec<String> = loads.names[..loads.count]
            .iter()
            .map(|name| String::from_utf16_lossy(&name[..name.iter().position(|&c| c == 0).unwrap_or(MOST_NAME)]))
            .collect();
        assert!(
            names.is_empty(),
            "the dump writer loaded DLLs that hold_dump_libraries lacks: {names:?}"
        );
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
