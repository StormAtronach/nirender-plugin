//! wgpu's `log` lines in this program's log. The logger takes the one `log` slot of the process
//! at the start of [`crate::run_with`], before the extension is made: the helper of MGE XE sets
//! a tracing subscriber on its loader thread, whose `try_init` also asks for the slot, and fails
//! after its own subscriber is set (the start page of D386, section 5.2).
//!
//! The level is "warn" for every crate, and "debug" for wgpu and naga while the device is
//! requested ([`RequestLog`]): the reason why a backend did not load is only in wgpu's debug
//! lines ([`backend_failure`]). Outside the request the level of the `log` crate is "warn", so
//! that a debug line in a frame path of wgpu costs one compare. The lines go to stdout, as
//! `wgpu: <level> <target>: <message>`. The request keeps [`REQUEST_LINES_KEPT`] lines and the
//! time after it [`WARNINGS_KEPT`]; the rest are counted, and [`dropped_line`] gives the count
//! (at the end of the request, at the end of the stream and in a refusal).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

/// The lines of the request that are written; the rest are counted.
const REQUEST_LINES_KEPT: u32 = 300;
/// The warnings after the request that are written; the rest are counted.
const WARNINGS_KEPT: u32 = 200;
/// The start of wgpu-core's line for a backend that did not load (`Instance::new`).
const FAILED_BACKEND: &str = "Instance::new: failed to create ";

static REQUESTING: AtomicBool = AtomicBool::new(false);
static REQUEST_LINES: AtomicU32 = AtomicU32::new(0);
static WARNINGS: AtomicU32 = AtomicU32::new(0);
/// wgpu's words for the backend of the try in progress that did not load.
static BACKEND_FAILURE: Mutex<Option<String>> = Mutex::new(None);

struct Logger;

static LOGGER: Logger = Logger;

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        wanted(metadata.level(), metadata.target(), REQUESTING.load(Ordering::Relaxed))
    }

    fn log(&self, record: &log::Record) {
        let requesting = REQUESTING.load(Ordering::Relaxed);
        if !wanted(record.level(), record.target(), requesting) {
            return;
        }
        let message = record.args().to_string();
        if requesting {
            if let Some(failure) = backend_failure(record.target(), &message) {
                if let Ok(mut kept) = BACKEND_FAILURE.lock() {
                    *kept = Some(failure);
                }
            }
        }
        let (count, kept) = if requesting {
            (&REQUEST_LINES, REQUEST_LINES_KEPT)
        } else {
            (&WARNINGS, WARNINGS_KEPT)
        };
        if count.fetch_add(1, Ordering::Relaxed) < kept {
            println!("wgpu: {} {}: {message}", record.level(), record.target());
        }
    }

    fn flush(&self) {}
}

/// Whether a line of `level` from `target` goes into the log: every warning and error, and
/// wgpu's and naga's lines down to "debug" during the request.
fn wanted(level: log::Level, target: &str, requesting: bool) -> bool {
    level <= log::Level::Warn
        || (requesting && level <= log::Level::Debug && (target.starts_with("wgpu") || target.starts_with("naga")))
}

/// wgpu's words for a backend that did not load, from its debug line in `Instance::new`
/// (wgpu-core `instance.rs`), without the name of the function.
fn backend_failure(target: &str, message: &str) -> Option<String> {
    (target == "wgpu_core::instance" && message.starts_with(FAILED_BACKEND))
        .then(|| message.trim_start_matches("Instance::new: ").to_string())
}

/// Takes the `log` slot, at the level "warn". Another logger in the slot keeps it: wgpu's lines
/// then go there, and the line says so.
pub(crate) fn install() {
    match log::set_logger(&LOGGER) {
        Ok(()) => {
            log::set_max_level(log::LevelFilter::Warn);
            println!("wgpu log: warnings in this log; wgpu's debug lines while the device is requested");
        }
        Err(_) => println!("wgpu log: another logger holds the slot; wgpu's lines go there"),
    }
}

/// The time of the request of the device: wgpu's debug lines go into the log. The level goes
/// back to "warn" when the guard drops, and the count of the lines that were not written is
/// then written.
pub(crate) struct RequestLog;

impl RequestLog {
    pub(crate) fn begin() -> RequestLog {
        REQUEST_LINES.store(0, Ordering::Relaxed);
        REQUESTING.store(true, Ordering::Relaxed);
        if log::max_level() != log::LevelFilter::Off {
            log::set_max_level(log::LevelFilter::Debug);
        }
        RequestLog
    }
}

impl Drop for RequestLog {
    fn drop(&mut self) {
        REQUESTING.store(false, Ordering::Relaxed);
        if log::max_level() != log::LevelFilter::Off {
            log::set_max_level(log::LevelFilter::Warn);
        }
        if let Some(line) = dropped_line() {
            println!("{line}");
        }
    }
}

/// The device is being requested ([`RequestLog`]).
pub(crate) fn requesting() -> bool {
    REQUESTING.load(Ordering::Relaxed)
}

/// wgpu's words for why the backend of the last try did not load; `None` when it loaded or
/// wgpu said nothing. Each try takes its own.
pub(crate) fn take_backend_failure() -> Option<String> {
    BACKEND_FAILURE.lock().ok().and_then(|mut kept| kept.take())
}

/// The line of the counts of wgpu's lines that were not written, or `None` when every line was.
pub(crate) fn dropped_line() -> Option<String> {
    dropped_words(
        REQUEST_LINES.load(Ordering::Relaxed).saturating_sub(REQUEST_LINES_KEPT),
        WARNINGS.load(Ordering::Relaxed).saturating_sub(WARNINGS_KEPT),
    )
}

fn dropped_words(request: u32, warnings: u32) -> Option<String> {
    let mut parts = Vec::new();
    if request > 0 {
        parts.push(format!(
            "{request} lines of the request (the first {REQUEST_LINES_KEPT} are written)"
        ));
    }
    if warnings > 0 {
        parts.push(format!("{warnings} warnings (the first {WARNINGS_KEPT} are written)"));
    }
    (!parts.is_empty()).then(|| format!("wgpu log: not written: {}", parts.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_always_and_debug_lines_of_wgpu_only_during_the_request() {
        use log::Level::{Debug, Error, Info, Trace, Warn};
        assert!(wanted(Error, "winit", false));
        assert!(wanted(Warn, "wgpu_hal::vulkan", false));
        assert!(!wanted(Info, "wgpu_core::instance", false));
        assert!(wanted(Debug, "wgpu_core::instance", true));
        assert!(wanted(Info, "naga::front", true));
        assert!(!wanted(Debug, "winit", true));
        assert!(!wanted(Trace, "wgpu_core::instance", true));
    }

    #[test]
    fn the_reason_of_a_backend_that_did_not_load_is_taken_from_wgpu_cores_line() {
        assert_eq!(
            backend_failure(
                "wgpu_core::instance",
                "Instance::new: failed to create Vulkan backend: InstanceError { message: \"missing Vulkan entry points\" }"
            )
            .as_deref(),
            Some("failed to create Vulkan backend: InstanceError { message: \"missing Vulkan entry points\" }")
        );
        assert_eq!(
            backend_failure("wgpu_core::instance", "Instance::new: created Vulkan backend"),
            None
        );
        assert_eq!(backend_failure("wgpu_hal::vulkan", "Instance::new: failed to create x"), None);
    }

    #[test]
    fn the_count_line_names_what_was_not_written() {
        assert_eq!(dropped_words(0, 0), None);
        assert_eq!(
            dropped_words(4, 0).as_deref(),
            Some("wgpu log: not written: 4 lines of the request (the first 300 are written)")
        );
        assert_eq!(
            dropped_words(4, 2).as_deref(),
            Some("wgpu log: not written: 4 lines of the request (the first 300 are written); 2 warnings (the first 200 are written)")
        );
    }
}
