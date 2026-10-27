//! The numbers that the plugin and the helper agree on outside the records: the exit codes
//! of the helper that the plugin reads, and the ids of the messages of the helper for the
//! game process. This file is the one home of them. The plugin crate has it as
//! `stream_format`, and the host crate (`host/src/lib.rs`) reads the same file with a
//! `#[path]` module: the two crates share no dependency, and a copy would drift. The file
//! uses nothing of either crate.
//!
//! The exit codes of numidium.exe (the failure-path design, section 4):
//!
//! | code | meaning |
//! | --- | --- |
//! | 0 | ended as asked |
//! | 1 to 9 | its IPC host mode (`error.rs` of mgeHost64) |
//! | 10 to 29 | a refusal with a message box of its own: 10 the stream format, 11 the extension format, 20 the device or the window (`feature/version-handshake`) |
//! | 30 | the watchdog of the helper: its frame thread made no progress for [`STALL_SECONDS`]; numidium.dmp written |
//! | 31 | the plugin ended the helper after 10 s without a read of the ring |
//! | 0xc0000409 | a Rust panic (the fast fail of the abort); numidium.dmp written by the panic hook |
//! | other 0xC... | an exception (0xc0000005 an access violation); numidium.dmp written by the exception filter |
//!
//! The plugin's `check_host` reads them: for 10 to 29 the helper showed its message and the
//! plugin adds no second box ([`helper_said_why`]); for the rest the plugin runs the failure
//! path (`failure.rs`: the autosave, then its box).

/// The watchdog of the helper found its frame thread stalled and ended the program.
pub const EXIT_STALLED: u32 = 30;
/// The plugin ended the helper, which read no record for 10 s.
pub const EXIT_NOT_ANSWERING: u32 = 31;
/// The seconds without a step of the frame thread after which the watchdog ends the helper.
pub const STALL_SECONDS: u64 = 5;

/// True when the helper ended with one of its refusals (10 to 29): it named the reason in a
/// message box and in its log, and the plugin adds no second box.
pub fn helper_said_why(exit_code: u32) -> bool {
    (10..=29).contains(&exit_code)
}

/// The messages of the helper for the game process (`Extension::messages` of the host crate
/// and the message memory of the stream): 1 is the distant land of MGE XE
/// (`MGE_StreamMessage`); 2 says that the helper lost its graphics device (the data: the
/// version word 1). The plugin ends the stream on it and runs the failure path.
pub const MESSAGE_LOST_DEVICE: u32 = 2;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refusals_are_the_codes_10_to_29() {
        assert!(!helper_said_why(0));
        assert!(!helper_said_why(3));
        assert!(helper_said_why(10));
        assert!(helper_said_why(20));
        assert!(helper_said_why(29));
        assert!(!helper_said_why(EXIT_STALLED));
        assert!(!helper_said_why(EXIT_NOT_ANSWERING));
        assert!(!helper_said_why(0xc000_0409));
    }
}
