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
use std::io::Write;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

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
    fn GetStdHandle(which: u32) -> isize;
    fn WriteFile(file: isize, bytes: *const u8, length: u32, written: *mut u32, overlapped: *mut c_void) -> i32;
    fn TerminateProcess(process: isize, code: u32) -> i32;
    fn Sleep(milliseconds: u32);
}

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
/// THREAD_ALL_ACCESS.
const THREAD_ALL_ACCESS: u32 = 0x1F_FFFF;
/// The filter's answer: the program ends.
const EXCEPTION_EXECUTE_HANDLER: i32 = 1;

/// The path of the minidump, set by [`install`].
static DUMP: OnceLock<PathBuf> = OnceLock::new();
/// The file name of this program as UTF-16 in lower case (ASCII), taken at [`install`]: the
/// callback of the dump writer compares the modules with it and allocates nothing.
static PROGRAM: OnceLock<Vec<u16>> = OnceLock::new();
/// The seconds that the stall dump may take: then the program ends without it (the dump
/// writer can wait for a lock that the suspended frame thread holds, [I]).
const STALL_DUMP_SECONDS: u32 = 10;
/// STD_OUTPUT_HANDLE.
const STD_OUTPUT: u32 = -11i32 as u32;
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
        println!("crash: the dump is held (a test: hold_next_dump); the plugin ends this program");
        let _ = std::io::stdout().flush();
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
    let program = std::env::current_exe().ok().and_then(|exe| exe.file_stem().map(OsStr::to_os_string));
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
    let _ = DUMP.set(dump);
    let program = std::env::current_exe().ok().and_then(|exe| exe.file_name().map(|name| name.to_string_lossy().into_owned()));
    let _ = PROGRAM.set(program.unwrap_or_default().encode_utf16().map(lower).collect());
    std::panic::set_hook(Box::new(|info| {
        // First the exit for the plugin: a dump that hangs is then ended with this code.
        crate::live::announce_exit(EXIT_PANIC);
        println!("panic: {info}");
        for line in backtrace() {
            println!("  {line}");
        }
        hold_if_asked();
        let dump = match DUMP.get() {
            Some(path) => write_dump_here(path).map(|()| path.as_path()),
            None => Err("no path for it: the crash data was not installed".to_string()),
        };
        report(dump);
        let _ = std::io::stdout().flush();
    }));
    // SAFETY: the filter is a plain function of this program; it stays for the program's life.
    unsafe { SetUnhandledExceptionFilter(Some(filter)) };
}

/// The unhandled exception filter: the exception and the backtrace go into the log, then the
/// minidump holds the exception, and the program ends.
unsafe extern "system" fn filter(pointers: *mut ExceptionPointers) -> i32 {
    let record = (*pointers).record;
    // First the exit for the plugin: a dump that hangs is then ended with this code.
    if let Some(code) = exit_of_exception(record) {
        crate::live::announce_exit(code);
    }
    if record.is_null() {
        println!("crash: an exception without a record");
    } else {
        println!("crash: exception {:#x} at {}", (*record).code, place((*record).address as usize));
    }
    for line in backtrace() {
        println!("  {line}");
    }
    hold_if_asked();
    let dump = match DUMP.get() {
        Some(path) => write_dump_at(path, GetCurrentThreadId(), pointers).map(|()| path.as_path()),
        None => Err("no path for it: the crash data was not installed".to_string()),
    };
    report(dump);
    let _ = std::io::stdout().flush();
    EXCEPTION_EXECUTE_HANDLER
}

/// Writes the minidump with the context of the stalled frame thread `frame_thread` as its
/// exception (the code [`STALL_EXCEPTION`]), from the watchdog thread, so that a debugger
/// opens it on the stalled thread, and ends the program with `exit_code`. Everything that
/// allocates comes before the frame thread is suspended (the file, a thread that ends the
/// program after [`STALL_DUMP_SECONDS`]): a thread suspended inside an allocation or a print
/// holds its lock. After the suspend: the dump to the raw handle, one fixed line to the raw
/// handle of the log, and `TerminateProcess`.
pub(crate) fn stall_dump_and_end(frame_thread: u32, exit_code: u32) -> ! {
    let file = DUMP.get().and_then(|path| std::fs::File::create(path).ok());
    // The last resort: the dump writer may wait for a lock that the suspended thread holds.
    std::thread::spawn(move || {
        // SAFETY: a sleep, then the end of this process.
        unsafe {
            Sleep(STALL_DUMP_SECONDS * 1000);
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
                    let mut record = ExceptionRecord { code: STALL_EXCEPTION, flags: 0, record: core::ptr::null_mut(), address: rip as *mut c_void, parameters: 0, information: [0; 15] };
                    let mut pointers = ExceptionPointers { record: &mut record, context: &mut context as *mut Context as *mut c_void };
                    if dump_to(file.as_raw_handle(), frame_thread, &mut pointers) {
                        b"minidump: written (the frame thread's context)\n"
                    } else {
                        b"no minidump: MiniDumpWriteDump failed\n"
                    }
                }
            }
        };
        let mut written = 0u32;
        WriteFile(GetStdHandle(STD_OUTPUT), line.as_ptr(), line.len() as u32, &mut written, core::ptr::null_mut());
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

    let mut record = ExceptionRecord { code: PANIC_EXCEPTION, flags: 0, record: core::ptr::null_mut(), address: rip as *mut c_void, parameters: 0, information: [0; 15] };
    let pointers = ExceptionPointers { record: &mut record, context: &mut context as *mut Context as *mut c_void };
    // SAFETY: a plain read of the id of this thread.
    let thread = unsafe { GetCurrentThreadId() };
    // Another thread writes the dump, so that the stack of this thread stays as the context
    // describes it: a dump of the calling thread with its own context fails (error 998).
    let pointers = &pointers as *const ExceptionPointers as usize;
    std::thread::scope(|scope| scope.spawn(|| write_dump_at(path, thread, pointers as *mut ExceptionPointers)).join().unwrap_or_else(|_| Err("the thread of the dump panicked".to_string())))
}

/// The frames of the caller's stack, the nearest first, as `module+offset` lines.
fn backtrace() -> Vec<String> {
    let mut frames = [core::ptr::null_mut(); MOST_FRAMES];
    // SAFETY: the array has room for `MOST_FRAMES` addresses, and the call writes at most that many.
    let count = unsafe { RtlCaptureStackBackTrace(1, MOST_FRAMES as u32, frames.as_mut_ptr(), core::ptr::null_mut()) };
    frames[..count as usize].iter().map(|&address| place(address as usize)).collect()
}

/// `module+offset` for an address, or the address alone when no module holds it.
fn place(address: usize) -> String {
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
    format!("{file}+{:#x}", address - module as usize)
}

/// Writes the minidump of the process at `path`, with the exception of the thread `thread`
/// when there is one (`pointers` null: none).
fn write_dump_at(path: &Path, thread: u32, pointers: *mut ExceptionPointers) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|problem| format!("{}: {problem}", path.display()))?;
    if !dump_to(file.as_raw_handle(), thread, pointers) {
        // SAFETY: a plain read of the thread's last error.
        return Err(format!("{}: MiniDumpWriteDump failed, error {}", path.display(), unsafe { GetLastError() }));
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
        let information = MinidumpExceptionInformation { thread, pointers, client_pointers: 0 };
        let exception = if pointers.is_null() { core::ptr::null() } else { &information as *const _ };
        let callback = MinidumpCallbackInformation { routine: Some(module_data_of_this_program), parameter: core::ptr::null_mut() };
        MiniDumpWriteDump(GetCurrentProcess() as *mut c_void, GetCurrentProcessId(), file, DUMP_KIND, exception, core::ptr::null(), &callback as *const _ as *const c_void) != 0
    }
}

/// The callback of the dump writer: every thread and module goes in, but the data segments
/// of the modules that are not this program stay out of the dump (the graphics driver's
/// DLLs have tens of MB of them).
unsafe extern "system" fn module_data_of_this_program(_parameter: *mut c_void, input: *const MinidumpCallbackInput, output: *mut MinidumpCallbackOutput) -> i32 {
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
    let start = path.iter().rposition(|&unit| unit == u16::from(b'\\') || unit == u16::from(b'/')).map_or(0, |at| at + 1);
    let file = &path[start..];
    file.len() == program.len() && file.iter().zip(program).all(|(&unit, &want)| lower(unit) == want)
}

/// The lower case of an ASCII letter in UTF-16; any other unit as it is.
fn lower(unit: u16) -> u16 {
    if (u16::from(b'A')..=u16::from(b'Z')).contains(&unit) { unit + 32 } else { unit }
}

fn report(dump: Result<&Path, String>) {
    match dump {
        Ok(path) => println!("minidump: {}", path.display()),
        Err(problem) => println!("no minidump: {problem}"),
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn a_dump_of_this_process_is_written() {
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
        let record = ExceptionRecord { code: 0xC000_0005, flags: 0, record: core::ptr::null_mut(), address: core::ptr::null_mut(), parameters: 0, information: [0; 15] };
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
}
