//! The data of a crash of the plugin for a player's report: the backtrace of a panic in the
//! log, then the crash log and the minidump that MWSE writes for every crash of the game.
//!
//! A panic of the Rust code ends the game: the profile has `panic = "abort"`, and the abort
//! of Rust on Windows is a fast fail (`int 29h`, the exit code 0xc0000409). A fast fail goes
//! to Windows Error Reporting directly. No exception filter of the process sees it, so the
//! crash logger of MWSE (`MWSEUnhandledExceptionFilter` in PatchUtil.cpp) writes nothing for
//! it. The panic hook raises an exception instead, after the message and the backtrace went
//! into the log: MWSE's filter writes the crash section of MWSE.log and `MWSE_MiniDump.dmp`
//! for it, as for a crash of the engine, and the game ends.
//!
//! The frames of the backtrace are `module+offset` lines. The plugin ships without its pdb
//! (the owner's rule), so no name is in the log; the pdb of the same build resolves the
//! offsets later.

use core::ffi::c_void;

use crate::win::{GetModuleFileNameW, GetModuleHandleExW, RtlCaptureStackBackTrace};

#[link(name = "kernel32")]
extern "system" {
    fn RaiseException(code: u32, flags: u32, count: u32, arguments: *const usize);
}

/// The code of the exception that the panic hook raises: a user code (the error bits, then
/// the letters N, I, R). The crash section of MWSE.log names it.
pub const PANIC_EXCEPTION: u32 = 0xE04E_4952;

/// GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT.
const MODULE_OF_ADDRESS: u32 = 6;
/// EXCEPTION_NONCONTINUABLE.
const NONCONTINUABLE: u32 = 1;
/// The most frames that `RtlCaptureStackBackTrace` takes on every version of Windows.
const MOST_FRAMES: usize = 62;

/// Puts the panic hook on: the message and the backtrace go into the log, then the
/// exception for MWSE's crash logger ends the game.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let message = format!("panic: {info}");
        crate::log!("{message}");
        for line in backtrace() {
            crate::log!("  {line}");
        }
        crate::log!("a release build of the plugin keeps no frame pointers: the backtrace above can be short; the crash log and the minidump of MWSE have the whole stack");
        crate::log!("the crash logger of MWSE takes it from here: MWSE.log and MWSE_MiniDump.dmp");
        raise_for_crash_logger(&message);
    }));
}

/// The frames of the caller's stack, the nearest first, as `module+offset` lines. On x86
/// the walk follows the frame chain, which stops at the first function without a frame
/// pointer: the lines are the frames up to there.
pub fn backtrace() -> Vec<String> {
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
        return format!("{address:#010x}");
    }

    let mut name = [0u16; 260];
    // SAFETY: the name buffer has room for `MAX_PATH` characters, and the call takes its length.
    let length = unsafe { GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) } as usize;
    let path = String::from_utf16_lossy(&name[..length]);
    let file = path.rsplit(['\\', '/']).next().unwrap_or(&path);
    format!("{file}+{:#x}", address - module as usize)
}

/// Raises the exception of a panic, with the message as its arguments, for the unhandled
/// exception filter of the process (MWSE's). It returns only when a handler continues the
/// program, and then the abort of the panic follows.
fn raise_for_crash_logger(message: &str) {
    let arguments = [message.as_ptr() as usize, message.len()];
    // SAFETY: the arguments live for the call, and the count is their number.
    unsafe { RaiseException(PANIC_EXCEPTION, NONCONTINUABLE, arguments.len() as u32, arguments.as_ptr()) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backtrace_names_this_module() {
        let lines = backtrace();
        assert!(!lines.is_empty());
        // The test binary holds this function: its first line is in it.
        assert!(lines[0].contains('+'), "{}", lines[0]);
        let module = lines[0].split('+').next().unwrap();
        assert!(module.ends_with(".exe"), "{}", lines[0]);
    }
}
