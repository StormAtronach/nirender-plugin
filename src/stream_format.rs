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
//! Bump [`STREAM_FORMAT`] with any change of the ring head, of a record tag's layout or
//! meaning, or of the HEADER record. Name the bump in the commit message.

/// The format of the stream that this build writes and reads.
pub const STREAM_FORMAT: u32 = 1;

/// The bytes of a HEADER record: the width and the height of the picture, the window of the
/// game, the abilities of the stream, the stream format, then the build id of the plugin as
/// ASCII, zero-padded. A HEADER of 16 bytes comes from a plugin before the handshake.
pub const HEADER_FORMAT_AT: usize = 16;
pub const HEADER_BUILD_ID_AT: usize = 20;
pub const HEADER_BUILD_ID_BYTES: usize = 32;
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
    }

    #[test]
    fn the_build_id_is_cut_to_its_field_and_zero_padded() {
        let long = "x".repeat(40);
        let bytes = build_id_bytes(&long);
        assert!(bytes.iter().all(|&byte| byte == b'x'));
        let short = build_id_bytes("v1 abc");
        assert_eq!(&short[..6], b"v1 abc");
        assert!(short[6..].iter().all(|&byte| byte == 0));
        assert_eq!(header_build_id(&header(STREAM_FORMAT, "v1 abc")), "v1 abc");
        assert_eq!(header(STREAM_FORMAT, "v1 abc").len(), HEADER_BYTES);
    }
}
