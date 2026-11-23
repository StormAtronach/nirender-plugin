//! The record "debug shapes": the Lua shapes of the debug views (W323 part 2a; the addendum
//! designs/debug-views-addendum-lua-and-bounds-2026-11-23, section 2).
//!
//! This file is the one home of the record's layout and of the limit of the Lua segments of a
//! frame. The plugin crate has it as `debug_shapes_format`: the writer, behind
//! `nirender.debug.line`, `box`, `sphere` and `clear`. The host crate (`host/src/lib.rs`)
//! reads the same file with a `#[path]` module: the reader is the extension of mgeHost64
//! (`renderer/debug_view.rs`), which keeps the Lua shapes and turns them into segments. The
//! file uses nothing of either crate.
//!
//! Each call is one extension record with the id [`RECORD_ID`], not kept. Its bytes, all
//! little-endian:
//!
//! | offset | type | value |
//! | --- | --- | --- |
//! | 0 | u32 | [`VERSION`] |
//! | 4 | u32 | the kind ([`Kind`]) |
//! | 8 | u32 | the flags ([`FLAG_ON_TOP`]) |
//! | 12 | u32 | the tag: [`tag_hash`] of the string, 0 for none |
//! | 16 | f32 | the seconds that the shape stays; 0 for this frame only |
//! | 20 | 3 x f32 | the colour, red, green and blue, 0 to 1 |
//! | 32 | 6 x f64 | the values of the kind, in world units |
//!
//! The values: a line, its two ends; a box, its least and its most corner; a sphere, its
//! centre and its radius (the fourth value); a clear, none. A clear with the tag 0 removes
//! every Lua shape, else the shapes with its tag. The places are f64, as Lua gives them: the
//! reader subtracts the frame's origin in f64, so a far place does not shake.
//!
//! The record came with no format bump (the erratum to the addendum's 2.2, verdict 698): a new
//! id changes no layout and no meaning of an id, and a capture made before it has no record
//! of the id. The reader takes version 1 only. A later change of this layout or of a word's
//! meaning bumps [`crate::stream_format::STREAM_FORMAT`] by hand, with the bump named in the
//! commit message: no check sees this file.

/// The id of the record among the extension records of mgeHost64 (`renderer.rs`).
pub const RECORD_ID: u32 = 15;
/// The version of the record's layout.
pub const VERSION: u32 = 1;
/// The size of the record.
pub const RECORD_BYTES: usize = 80;
/// The most Lua segments in one frame. The plugin counts them before the stream ring and
/// drops a whole shape that would pass the limit; the reader keeps the same limit as a second
/// guard.
pub const MOST_SEGMENTS: u32 = 65_536;
/// The flag of a shape drawn over everything; without it the shape is tested against the
/// frame's depth.
pub const FLAG_ON_TOP: u32 = 1;

/// The kind of a record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Line = 1,
    Box = 2,
    Sphere = 3,
    Clear = 4,
}

impl Kind {
    /// The kind of a word; [`None`] for an unknown word.
    pub fn from_word(word: u32) -> Option<Kind> {
        match word {
            1 => Some(Kind::Line),
            2 => Some(Kind::Box),
            3 => Some(Kind::Sphere),
            4 => Some(Kind::Clear),
            _ => None,
        }
    }

    /// The segments of a shape of this kind: a line 1, a box 12 (its edges), a sphere 48 (three
    /// circles of 16), a clear none. The shapes of the host's `debug_lines.rs` give the same
    /// numbers; a test of the host compares them.
    pub const fn segments(self) -> u32 {
        match self {
            Kind::Line => 1,
            Kind::Box => 12,
            Kind::Sphere => 48,
            Kind::Clear => 0,
        }
    }
}

/// One record: a shape, or a clear.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub kind: Kind,
    pub on_top: bool,
    pub tag: u32,
    pub seconds: f32,
    pub color: [f32; 3],
    pub values: [f64; 6],
}

impl Shape {
    /// The bytes of the record.
    pub fn encode(&self) -> [u8; RECORD_BYTES] {
        let mut bytes = [0u8; RECORD_BYTES];
        let flags = if self.on_top { FLAG_ON_TOP } else { 0 };
        let words = [VERSION, self.kind as u32, flags, self.tag, self.seconds.to_bits()];
        for (i, word) in words.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        for (i, channel) in self.color.iter().enumerate() {
            bytes[20 + i * 4..24 + i * 4].copy_from_slice(&channel.to_le_bytes());
        }
        for (i, value) in self.values.iter().enumerate() {
            bytes[32 + i * 8..40 + i * 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    /// The shape of a record; [`None`] for a short record, another version or an unknown kind.
    pub fn decode(bytes: &[u8]) -> Option<Shape> {
        let bytes = bytes.get(..RECORD_BYTES)?;
        let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let real = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let double = |at: usize| f64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        if word(0) != VERSION {
            return None;
        }
        Some(Shape {
            kind: Kind::from_word(word(4))?,
            on_top: word(8) & FLAG_ON_TOP != 0,
            tag: word(12),
            seconds: real(16),
            color: [real(20), real(24), real(28)],
            values: core::array::from_fn(|i| double(32 + i * 8)),
        })
    }
}

/// The bytes that a record needs before it is read: the whole record of [`VERSION`], or the
/// version word of another version (which [`Shape::decode`] does not take), so that a reader
/// refuses a short record and not a layout that it does not read.
pub fn needed(bytes: &[u8]) -> usize {
    match bytes.get(..4) {
        Some(word) if u32::from_le_bytes(word.try_into().unwrap()) == VERSION => RECORD_BYTES,
        _ => 4,
    }
}

/// The tag of a string: its 32-bit FNV-1a hash; 0 for the empty string, and 1 for a string
/// whose hash is 0, as 0 means no tag. Two strings can share a hash; the Lua reference says
/// so.
pub fn tag_hash(tag: &str) -> u32 {
    if tag.is_empty() {
        return 0;
    }
    let hash = tag.bytes().fold(0x811C_9DC5u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    });
    hash.max(1)
}

/// The count of the Lua segments of a frame, against [`MOST_SEGMENTS`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameBudget {
    frame: u32,
    used: u32,
}

impl FrameBudget {
    pub const fn new() -> FrameBudget {
        FrameBudget { frame: 0, used: 0 }
    }

    /// True when a shape of `kind` fits in the frame `frame` with the shapes before it, and
    /// then counts its segments. A new frame starts at 0. A clear always fits.
    pub fn admit(&mut self, frame: u32, kind: Kind) -> bool {
        if frame != self.frame {
            self.frame = frame;
            self.used = 0;
        }
        let segments = kind.segments();
        if self.used + segments > MOST_SEGMENTS {
            return false;
        }
        self.used += segments;
        true
    }

    /// The segments counted in the frame.
    pub fn used(&self) -> u32 {
        self.used
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_decodes_to_its_shape() {
        let shape = Shape {
            kind: Kind::Sphere,
            on_top: true,
            tag: tag_hash("probe"),
            seconds: 2.0,
            color: [1.0, 0.5, 0.0],
            values: [-123456.789, 98765.4321, 1000.5, 250.0, 0.0, 0.0],
        };
        let bytes = shape.encode();
        assert_eq!(bytes.len(), RECORD_BYTES);
        assert_eq!(Shape::decode(&bytes), Some(shape));
        // The null: a short record, another version and an unknown kind give no shape.
        assert_eq!(Shape::decode(&bytes[..RECORD_BYTES - 1]), None);
        let mut other = bytes;
        other[0] = 2;
        assert_eq!(Shape::decode(&other), None);
        let mut unknown = bytes;
        unknown[4] = 9;
        assert_eq!(Shape::decode(&unknown), None);
        // A reader needs the whole record of this version, and the version word of another.
        assert_eq!(needed(&bytes[..3]), 4);
        assert_eq!(needed(&bytes[..4]), RECORD_BYTES);
        assert_eq!(needed(&other), 4);
    }

    #[test]
    fn a_tag_hash_is_never_zero_for_a_name() {
        assert_eq!(tag_hash(""), 0);
        // FNV-1a of "a" (the reference value of the algorithm).
        assert_eq!(tag_hash("a"), 0xE40C_292C);
        assert_ne!(tag_hash("probe"), tag_hash("other"));
    }

    /// The limit drops whole shapes: boxes alone fill 5,461 boxes, 65,532 segments; a line
    /// still fits after them, and a new frame starts at 0.
    #[test]
    fn the_budget_drops_whole_shapes() {
        let mut budget = FrameBudget::new();
        let admitted = (0..70_000 / 12).filter(|_| budget.admit(7, Kind::Box)).count();
        assert_eq!(admitted, 5_461);
        assert_eq!(budget.used(), 65_532);
        assert!(!budget.admit(7, Kind::Sphere));
        assert!(budget.admit(7, Kind::Line));
        assert!(budget.admit(7, Kind::Clear));
        assert!(budget.admit(8, Kind::Sphere));
        assert_eq!(budget.used(), 48);
    }
}
