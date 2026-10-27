//! The plugin log. It is `nirender.log` in the game folder unless Lua gives another path.
//! The log of the run before is kept as `nirender.log.1` beside it, so that a player who
//! starts the game again after a crash still has it.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;

static LOG: Mutex<Option<File>> = Mutex::new(None);

/// Opens the log at `path`, one time: a second call keeps the file of the first, so that a
/// second install keeps what the first one wrote.
pub fn open(path: &str) {
    let mut guard = LOG.lock().unwrap();
    if guard.is_none() {
        *guard = open_new(path);
    }
}

pub fn write(line: &str) {
    let mut guard = LOG.lock().unwrap();
    if guard.is_none() {
        *guard = open_new("nirender.log");
    }
    if let Some(file) = guard.as_mut() {
        let _ = writeln!(file, "{}", line);
        let _ = file.flush();
    }
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
}
