//! The format of the stream between the plugin and the helper: one number that both compare.
//!
//! This file is the one home of the number. The plugin crate has it as `stream_format`, and
//! the host crate (`host/src/lib.rs`) reads the same file with a `#[path]` module: the two
//! crates share no dependency, and a copy would drift. The file uses nothing of either crate.
//!
//! The plugin writes the number into the HEADER record, the first record of the ring and of
//! every capture file, after the plugin's build id. The live helper reads it before anything
//! else and stops on a difference (`live::run`); a replay prints the difference and goes on,
//! because the gate's accepted captures come from before the number existed.
//!
//! Bump [`STREAM_FORMAT`] with a change of the meaning or the offset of a word of the ring
//! head, of the start of the records, of a record tag's layout or meaning, or of the HEADER
//! record; name the bump in the commit message. A new word in the free area of the ring head,
//! read as 0 from an older writer whose 0 means the behaviour before the word, needs none.
//!
//! The file is also the one home of the exit codes of the helper that the plugin reads and of
//! the ids of the messages of the helper for the game process.
//!
//! The exit codes of numidium.exe (the failure-path design, section 4):
//!
//! | code | meaning |
//! | --- | --- |
//! | 0 | ended as asked |
//! | 1 to 9 | its IPC host mode (`error.rs` of mgeHost64) |
//! | 10 to 29 | a refusal with a message box of its own: 10 the stream format, 11 the extension format, 20 the GPU or the window ([`EXIT_GPU`]) |
//! | 30 | the watchdog of the helper: its frame thread made no progress for [`STALL_SECONDS`]; numidium.dmp written |
//! | 31 | the plugin ended the helper: no read of the ring for [`not_answering_limit`] of its allowance |
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
//! The free area of the ring head: 80 to 127.

/// The format of the stream that this build writes and reads.
pub const STREAM_FORMAT: u32 = 1;

/// The bytes of a HEADER record: the width and the height of the picture, the window of the
/// game, the abilities of the stream, the stream format, then the build id of the plugin as
/// ASCII, zero-padded. A HEADER of 16 bytes comes from a plugin before the handshake.
pub const HEADER_FORMAT_AT: usize = 16;
pub const HEADER_BUILD_ID_AT: usize = 20;
pub const HEADER_BUILD_ID_BYTES: usize = 64;
pub const HEADER_BYTES: usize = HEADER_BUILD_ID_AT + HEADER_BUILD_ID_BYTES;

/// The exit codes of the helper when the parts do not match. The helper's own table: 0 ok,
/// 1 to 9 its IPC host mode (`error.rs` of mgeHost64), 10 to 29 a refusal with a message box
/// (10 the stream format, 11 the extension format or no hello record, 20 to 29 the device or
/// the window), 30 to 39 a stall. The plugin's `check_host` reads them with
/// [`helper_said_why`]: the helper showed its message, the plugin shows none.
pub const EXIT_STREAM_FORMAT: u32 = 10;
pub const EXIT_EXTENSION_FORMAT: u32 = 11;
/// The GPU or the window cannot be used: no adapter on any backend, no binding arrays with
/// their room, no BC compression, too few sampled textures, no surface. The live start and
/// `--probe` exit with it; d3d8.dll reads it from the probe as `MGE_PROBE_EXIT_GPU` of
/// `mgeversion.h` (a unit test of mgeHost64 compares the two).
pub const EXIT_GPU: u32 = 20;

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

/// The format number of a HEADER record: 0 for a record from before the handshake.
pub fn header_format(header: &[u8]) -> u32 {
    match header.get(HEADER_FORMAT_AT..HEADER_FORMAT_AT + 4) {
        Some(bytes) => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        None => 0,
    }
}

/// The build id of the plugin that wrote a HEADER record, or a text for a plugin before the
/// handshake.
pub fn header_build_id(header: &[u8]) -> String {
    let Some(bytes) = header.get(HEADER_BUILD_ID_AT..HEADER_BUILD_ID_AT + HEADER_BUILD_ID_BYTES) else {
        return "a build from before the handshake".to_string();
    };
    let end = bytes.iter().position(|&byte| byte == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// The build id as the HEADER record carries it: ASCII, cut to the field, zero-padded.
pub fn build_id_bytes(build_id: &str) -> [u8; HEADER_BUILD_ID_BYTES] {
    let mut bytes = [0u8; HEADER_BUILD_ID_BYTES];
    for (slot, byte) in bytes.iter_mut().zip(build_id.bytes().filter(u8::is_ascii)) {
        *slot = byte;
    }
    bytes
}

/// What a HEADER record says about its writer, when the reader takes it.
#[derive(Debug, PartialEq, Eq)]
pub struct Writer {
    pub format: u32,
    pub build_id: String,
}

/// Compares the format of a HEADER record with this build's. `reader` names the reader
/// (its program and its build id) for the message of a refusal, which names both builds.
pub fn accept_header(header: &[u8], reader: &str) -> Result<Writer, String> {
    let writer = Writer { format: header_format(header), build_id: header_build_id(header) };
    if writer.format == STREAM_FORMAT {
        return Ok(writer);
    }
    let writes = if writer.format == 0 {
        "writes a stream with no format number".to_string()
    } else {
        format!("writes the stream format {}", writer.format)
    };
    Err(format!(
        "The parts of the wgpu renderer of MGE XE do not match.\n\
         akulakhan.dll ({}) {writes}.\n\
         {reader} reads the stream format {STREAM_FORMAT}.\n\
         Install akulakhan.dll, numidium.exe and d3d8.dll from one archive.",
        writer.build_id
    ))
}

/// The messages of the helper for the game process (`Extension::messages` of the host crate
/// and the message memory of the stream): 1 is the distant land of MGE XE
/// (`MGE_StreamMessage`); 2 says that the helper lost its graphics device (the data: the
/// version word 1). The plugin ends the stream on it and runs the failure path.
pub const MESSAGE_LOST_DEVICE: u32 = 2;

#[cfg(test)]
mod tests {
    use super::*;

    fn header(format: u32, build_id: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        for word in [1920u32, 1080, 0x1234, 7, format] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.extend_from_slice(&build_id_bytes(build_id));
        bytes
    }

    #[test]
    fn a_header_of_this_format_passes_with_its_build_id() {
        let accepted = accept_header(&header(STREAM_FORMAT, "nirender v0.3.2 81ab9a6"), "numidium.exe (x)").unwrap();
        assert_eq!(accepted, Writer { format: STREAM_FORMAT, build_id: "nirender v0.3.2 81ab9a6".into() });
    }

    #[test]
    fn a_header_of_another_format_is_refused_with_both_builds_named() {
        let message = accept_header(&header(STREAM_FORMAT + 1, "nirender v9.9.9 abcdef0"), "numidium.exe (numidium v0.21.0 846ed77)").unwrap_err();
        assert!(message.contains("akulakhan.dll (nirender v9.9.9 abcdef0) writes the stream format 2"), "{message}");
        assert!(message.contains("numidium.exe (numidium v0.21.0 846ed77) reads the stream format 1"), "{message}");
        assert!(message.contains("one archive"), "{message}");
    }

    #[test]
    fn a_header_from_before_the_handshake_has_the_format_0_and_is_refused() {
        let old = header(0, "")[..16].to_vec();
        assert_eq!(header_format(&old), 0);
        assert_eq!(header_build_id(&old), "a build from before the handshake");
        let message = accept_header(&old, "numidium.exe (x)").unwrap_err();
        assert!(message.contains("akulakhan.dll (a build from before the handshake) writes a stream with no format number"), "{message}");
    }

    #[test]
    fn the_refusals_of_the_helper_are_the_codes_10_to_29() {
        assert!(helper_said_why(EXIT_STREAM_FORMAT) && helper_said_why(EXIT_EXTENSION_FORMAT) && helper_said_why(20));
        // The IPC host mode's codes, a stall and the codes of Windows for a crash are not.
        assert!(!helper_said_why(0) && !helper_said_why(3) && !helper_said_why(30) && !helper_said_why(0xC000_0409));
        assert!(!helper_said_why(EXIT_STALLED) && !helper_said_why(EXIT_NOT_ANSWERING));
    }

    #[test]
    fn the_plugin_gives_the_helper_its_allowance_and_5_s() {
        assert_eq!(not_answering_limit(0), 60);
        assert_eq!(not_answering_limit(STALL_SECONDS as u32), 10);
        assert_eq!(not_answering_limit(LOADING_SECONDS as u32), 35);
    }

    #[test]
    fn the_build_id_is_cut_to_its_field_and_zero_padded() {
        let long = "x".repeat(HEADER_BUILD_ID_BYTES + 8);
        let bytes = build_id_bytes(&long);
        assert!(bytes.iter().all(|&byte| byte == b'x'));
        let short = build_id_bytes("v1 abc");
        assert_eq!(&short[..6], b"v1 abc");
        assert!(short[6..].iter().all(|&byte| byte == 0));
        assert_eq!(header_build_id(&header(STREAM_FORMAT, "v1 abc")), "v1 abc");
        assert_eq!(header(STREAM_FORMAT, "v1 abc").len(), HEADER_BYTES);
    }
}
