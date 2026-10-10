//! The plugin log, `nirender.log` in the game folder unless Lua names another path.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;

static LOG: Mutex<Option<File>> = Mutex::new(None);

pub fn open(path: &str) {
    let file = OpenOptions::new().create(true).write(true).truncate(true).open(path).ok();
    *LOG.lock().unwrap() = file;
}

pub fn write(line: &str) {
    let mut guard = LOG.lock().unwrap();
    if guard.is_none() {
        *guard = OpenOptions::new().create(true).write(true).truncate(true).open("nirender.log").ok();
    }
    if let Some(file) = guard.as_mut() {
        let _ = writeln!(file, "{}", line);
        let _ = file.flush();
    }
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(&format!($($arg)*)) };
}
