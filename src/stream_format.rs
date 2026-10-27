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
//!
//! The words of the head of the ring (128 bytes; the constants have two copies, `capture.rs`
//! of the plugin and `live.rs` of the host crate, to be moved here after the merges):
//!
//! | offset | type | writer | word |
//! | --- | --- | --- | --- |
//! | 0 | u32 | plugin | the magic |
//! | 4 | u32 | plugin | the capacity of the data |
//! | 8 | u64 | plugin | the bytes written |
//! | 16 | u64 | helper | the bytes read |
//! | 24 | u32 | plugin | 1 when the game closed the stream |
//! | 32 | u32 | helper | the classes the helper draws itself |
//! | 36 | u32 | helper | 1 while the helper waits for a record |
//! | 40 | f32 | helper | the reach of the shadow map |
//! | 44 | u32 | helper | the busy time of a frame, microseconds |
//! | 48 | u64 | helper | the bytes read at the last frame shown |
//! | 56 | u64 | helper | the time of the last frame shown, ticks |
//! | 64 | i64 | plugin | the start of the next frame of the game, ticks |
//! | 72 | u32 | plugin | the game waits for the helper (Ryker's `fix/plugin-limits`) |
//! | 76 | u32 | helper | [`RING_ALLOWANCE`]: the seconds a step of the helper's frame thread may take |
//!
//! A new word in the free area (80 to 127), read as 0 from an older writer whose 0 means the
//! behaviour before the word, needs no bump of the stream format; a change of the meaning or
//! the offset of a word, of the start of the records or of a record's layout bumps it.

/// The watchdog of the helper found its frame thread stalled and ended the program.
pub const EXIT_STALLED: u32 = 30;
/// The plugin ended the helper, which read no record for 10 s.
pub const EXIT_NOT_ANSWERING: u32 = 31;
/// The seconds without a step of the frame thread after which the watchdog ends the helper.
pub const STALL_SECONDS: u64 = 5;
/// The seconds a step may take while the extension of the helper loads (the distant land
/// after a game load): the watchdog's limit then, and the allowance it writes.
pub const LOADING_SECONDS: u64 = 30;

/// The word of the ring head where the helper writes the seconds that a step of its frame
/// thread may take: [`STALL_SECONDS`] when its frames start, [`LOADING_SECONDS`] while its
/// extension loads. 0 before the frames start (and from a helper older than the word).
pub const RING_ALLOWANCE: usize = 76;
/// The seconds the plugin gives the helper over its allowance before it ends it: the
/// plugin's limit is the later of the two, as the last resort is.
pub const ALLOWANCE_MARGIN_SECONDS: u64 = 5;
/// The seconds the plugin gives the helper while the allowance word is 0: the helper's own
/// limit for its first record (`live.rs`).
pub const FIRST_RECORD_SECONDS: u64 = 60;

/// The seconds without a read, while records wait, after which the plugin ends the helper
/// with [`EXIT_NOT_ANSWERING`], for the allowance word of the ring head.
pub fn not_answering_limit(allowance: u32) -> u64 {
    if allowance == 0 {
        FIRST_RECORD_SECONDS
    } else {
        u64::from(allowance) + ALLOWANCE_MARGIN_SECONDS
    }
}

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

    #[test]
    fn the_plugin_gives_the_helper_its_allowance_and_5_s() {
        assert_eq!(not_answering_limit(0), 60);
        assert_eq!(not_answering_limit(STALL_SECONDS as u32), 10);
        assert_eq!(not_answering_limit(LOADING_SECONDS as u32), 35);
    }
}
