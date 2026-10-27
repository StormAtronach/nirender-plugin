//! The plugin log. With the d3d8.dll of MGE XE the plugin's lines go into `mgeXE.log`,
//! each prefixed `nirender: `, through its export `MGE_LogLine` (the contract
//! `mge_render_link.h`; the logs-and-files design of the Numidium notes): the game process
//! has one log, one timeline. Without the export (an older d3d8.dll, or the d3d8.dll of this
//! repository) the log is `nirender.log` in the game folder unless Lua gives another path.
//! The log of the run before is kept as `nirender.log.1` beside it, so that a player who
//! starts the game again after a crash still has it.

use core::ffi::{c_char, c_void};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

static LOG: Mutex<Option<File>> = Mutex::new(None);

/// `MGE_LogLine` of the d3d8.dll of the game folder, as an address: [`UNKNOWN`] before the
/// first line, [`MISSING`] when that d3d8.dll has no such export.
static MGE_LOG_LINE: AtomicUsize = AtomicUsize::new(UNKNOWN);
const UNKNOWN: usize = 0;
const MISSING: usize = 1;

/// `void __cdecl MGE_LogLine(const char* text)`: d3d8.dll writes each line of `text` as
/// `nirender: <line>`; any thread may call it.
type LogLine = unsafe extern "C" fn(text: *const c_char);

/// The export of MGE XE for the plugin's lines, looked up at the first line; [`None`] without
/// it.
fn mge_log_line() -> Option<LogLine> {
    let mut found = MGE_LOG_LINE.load(Ordering::Acquire);
    if found == UNKNOWN {
        // Two first lines at once look up the same export; the first store stays (a value set
        // in the meantime, as a test's fake, is not overwritten).
        found = match MGE_LOG_LINE.compare_exchange(UNKNOWN, find_mge_log_line(), Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => MGE_LOG_LINE.load(Ordering::Acquire),
            Err(set) => set,
        };
    }
    // SAFETY: any value but the two markers is the address of the export, which has the
    // signature of the contract and lives as long as d3d8.dll, which the game never unloads.
    (found != MISSING).then(|| unsafe { core::mem::transmute::<usize, LogLine>(found) })
}

/// The address of `MGE_LogLine` in the d3d8.dll of the game folder, or [`MISSING`]. A test
/// build has no d3d8.dll of MGE XE.
fn find_mge_log_line() -> usize {
    if cfg!(test) {
        return MISSING;
    }
    // SAFETY: the module is a handle that GetModuleHandleW gave for a loaded DLL, and the name
    // is NUL-terminated.
    let export = unsafe {
        let module = crate::link_client::host_module();
        if module == 0 {
            return MISSING;
        }
        crate::win::GetProcAddress(module as *mut c_void, c"MGE_LogLine".as_ptr().cast())
    };
    if export.is_null() { MISSING } else { export as usize }
}

/// Names the plugin's own log at `path`, one time: a second call keeps the file of the first,
/// so that a second install keeps what the first one wrote. With `MGE_LogLine` the lines go
/// into `mgeXE.log`, and no file is made at `path`.
pub fn open(path: &str) {
    if mge_log_line().is_some() {
        return;
    }
    let mut guard = LOG.lock().unwrap();
    if guard.is_none() {
        *guard = open_new(path);
    }
}

pub fn write(line: &str) {
    if let Some(log_line) = mge_log_line() {
        // The text for C: without a NUL inside, NUL-terminated.
        let text: Vec<u8> = line.bytes().filter(|&byte| byte != 0).chain([0]).collect();
        // SAFETY: a NUL-terminated text that lives through the call; the export takes it from
        // any thread.
        unsafe { log_line(text.as_ptr().cast()) };
        return;
    }
    let mut guard = LOG.lock().unwrap();
    if guard.is_none() {
        *guard = open_new(&default_path());
    }
    if let Some(file) = guard.as_mut() {
        let _ = writeln!(file, "{}", line);
        let _ = file.flush();
    }
}

/// The log of a line written before any `open`: `nirender.log` in the folder of the game. A
/// test build writes it into a folder of its own under the temporary folder, so that
/// `cargo test` leaves the checkout clean (before, every test that logs left `nirender.log`
/// and its rotated `nirender.log.1` in the repository's root).
fn default_path() -> String {
    if cfg!(test) {
        let folder = std::env::temp_dir().join("nirender-test-log");
        let _ = std::fs::create_dir_all(&folder);
        return folder.join("nirender.log").to_string_lossy().into_owned();
    }
    "nirender.log".to_string()
}

/// A new, empty log at `path`; the log of the run before goes to `<path>.1`.
fn open_new(path: &str) -> Option<File> {
    rotate(path);
    OpenOptions::new().create(true).write(true).truncate(true).open(path).ok()
}

/// Keeps the file at `path` as `<path>.1`, in place of the one of the run before that.
pub fn rotate(path: &str) {
    let before = format!("{path}.1");
    // A rename replaces the file of the run before that. When it fails (the file is open
    // in another program), a copy keeps the log at least.
    if std::fs::rename(path, &before).is_err() {
        let _ = std::fs::copy(path, &before);
    }
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(&format!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_keeps_the_log_of_the_run_before() {
        let folder = std::env::temp_dir().join(format!("nirender-log-rotate-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("nirender.log");
        let path = path.to_str().unwrap();
        let before = format!("{path}.1");

        // No log yet: nothing to keep.
        rotate(path);
        assert!(!std::path::Path::new(&before).exists());

        std::fs::write(path, "run 1").unwrap();
        rotate(path);
        assert_eq!(std::fs::read_to_string(&before).unwrap(), "run 1");
        assert!(!std::path::Path::new(path).exists());

        // The second rotation replaces the kept log.
        std::fs::write(path, "run 2").unwrap();
        rotate(path);
        assert_eq!(std::fs::read_to_string(&before).unwrap(), "run 2");

        std::fs::remove_dir_all(&folder).unwrap();
    }

    /// The lines that a fake `MGE_LogLine` took, as text.
    static TAKEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

    unsafe extern "C" fn fake_log_line(text: *const c_char) {
        let text = core::ffi::CStr::from_ptr(text).to_string_lossy().into_owned();
        TAKEN.lock().unwrap().push(text);
    }

    /// With the export, a line goes to it whole (d3d8.dll splits a text at its line ends and
    /// puts the prefix on each), without a NUL inside; `open` makes no file then.
    #[test]
    fn with_the_export_the_lines_go_into_the_log_of_mge_xe() {
        let folder = std::env::temp_dir().join(format!("nirender-log-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("nirender.log");
        MGE_LOG_LINE.store(fake_log_line as LogLine as usize, Ordering::Release);
        open(path.to_str().unwrap());
        write("own device: made\nstream: two lines");
        write("a NUL\0 inside");
        // Back to the test build's own log for the other tests.
        MGE_LOG_LINE.store(MISSING, Ordering::Release);
        let taken = TAKEN.lock().unwrap().clone();
        assert!(taken.iter().any(|line| line == "own device: made\nstream: two lines"), "{taken:?}");
        assert!(taken.iter().any(|line| line == "a NUL inside"), "{taken:?}");
        assert!(!path.exists(), "open made a file with the export");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    /// A test build's log is under the temporary folder, never in the current folder (the
    /// checkout's root under `cargo test`).
    #[test]
    fn a_test_build_logs_under_the_temporary_folder() {
        let path = std::path::PathBuf::from(default_path());
        assert!(path.starts_with(std::env::temp_dir()), "{}", path.display());
        assert!(path.parent().is_some_and(|folder| folder.is_dir()));
    }
}
