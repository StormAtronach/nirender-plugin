//! The tags of the records of the stream, as `src/capture.rs` of the plugin writes them, and
//! the length that a record of each tag needs before the player reads it ([`needed`]).

use crate::retained::BONE_BYTES;
use crate::{word, CALL_CLEAR, CALL_COPY_PIXELS, CALL_CREATE_TEXTURE, CALL_RENDER_TARGET, CALL_TEXTURE_LEVEL, CALL_VIEWPORT, COMMAND_TRANSFORM, EFFECT_STAGE_WORDS};

pub const TEXTURE: u32 = 1;
// Tags 2 and 7 are retired: the reader skips them.
pub const SCENE_BEGIN: u32 = 3;
pub const SCENE_END: u32 = 4;
pub const FRAME_EVENT: u32 = 5;
pub const OBJECT: u32 = 6;
pub const COMMAND: u32 = 8;
pub const CALL: u32 = 9;
pub const FRAME_END: u32 = 10;
pub const HEADER: u32 = 11;
pub const TEXTURE_UPDATE: u32 = 12;
pub const READ_PIXELS: u32 = 13;
pub const SHOT: u32 = 14;
pub const DRAW_UP: u32 = 15;
pub const EXTENSION: u32 = 16;
pub const MARK: u32 = 17;
pub const GEOMETRY: u32 = 18;
pub const MATERIAL: u32 = 19;
pub const SHAPE: u32 = 20;
pub const LIGHT: u32 = 21;
pub const OVERLAY: u32 = 22;
/// A retained instance of the plugin, a draw of one, and its end (`instance_stream`
/// of the plugin).
pub const INSTANCE: u32 = 23;
pub const INSTANCE_DRAW: u32 = 24;
pub const INSTANCE_DELETE: u32 = 25;
/// The lights of a retained instance (id, count, keys), and its visibility (id, 0 or 1).
pub const INSTANCE_LIGHTS: u32 = 26;
pub const INSTANCE_STATE: u32 = 27;
/// The ambient light and the fog of the frame for the retained draws.
pub const RETAINED_SCENE: u32 = 28;
/// The bones of a skinned retained instance: id, count, bound, then each bone (9 floats,
/// 3 doubles: `retained::BONE_BYTES`).
pub const INSTANCE_BONES: u32 = 29;
/// A map tile: the ticket, the rendered texture, the texture of the tile
/// (`player/tiles.rs`).
pub const COPY_TILE: u32 = 30;
/// The place of a sorted blend of the table in the flush of the engine's queue: id
/// (`Player::instance_queued`).
pub const INSTANCE_QUEUED: u32 = 31;
/// The place of a blend of the table in the engine's walk: id (`Player::walk_place`).
pub const INSTANCE_WALK: u32 = 32;
/// A light of the game was destroyed: its key (`Player::light_gone`).
pub const LIGHT_GONE: u32 = 33;
/// The keys of the lights of the world that are on (`RetainedSet::set_world_lights`).
pub const WORLD_LIGHTS: u32 = 34;
/// The particles of a particle instance's draw, before its [`INSTANCE_DRAW`]: the key of its
/// geometry, the count, the radius, the camera's right and up, then 20 bytes a particle
/// (`Renderer::particles` makes the quads under the key).
pub const INSTANCE_PARTICLES: u32 = 35;
/// The gamma ramp of the device: 768 u16, red, green and blue (`gamma.rs`).
pub const GAMMA_RAMP: u32 = 36;
/// A texture or a level surface of the plugin's device was freed: its handle
/// (`Renderer::texture_gone`). A later texture can get the same handle.
pub const TEXTURE_GONE: u32 = 37;
/// A geometry data of the game was destroyed: its handle and its unique id
/// (`Renderer::geometry_gone`). A later data can get the same handle.
pub const GEOMETRY_GONE: u32 = 38;
/// A material names a device texture that was freed: its handle
/// (`Player::drop_gone_materials`). The plugin sends the record again for a later draw.
pub const MATERIAL_GONE: u32 = 39;
/// A move of a known instance (`Player::instance_move`): the id, the rows 0 to 2 of the world
/// matrix (12 floats), the exact translation (3 doubles), the stored translation (3 floats)
/// and the world bound (4 floats); the rest stays that of its last [`INSTANCE`].
pub const INSTANCE_MOVE: u32 = 40;
/// The state of the UV controller of a geometry (`Player::geometry_uv_record`): the handle,
/// the unique id, the texture set in the low half and the flags in the high half (1: the
/// state of the vertices the last `GEOMETRY` record of the handle had), then the U offset,
/// the V offset, the U tiling and the V tiling (`uv_animation`).
pub const GEOMETRY_UV: u32 = 41;
/// The targets of the morph controller of a geometry (`Renderer::morph_targets`): the handle,
/// the unique id, the number of targets, the vertices of a target, then the positions of each
/// target (3 floats a vertex), one target after the other (`morph.rs`).
pub const MORPH_TARGETS: u32 = 42;
/// The weights of the targets for a new revision of a geometry (`Renderer::morph_weights`):
/// the handle, the unique id, the number of weights, then the weights.
pub const MORPH_WEIGHTS: u32 = 43;
/// The game made its renderer again at a new size: the width and the height of the picture and
/// the game's window, as the HEADER has them (`Player::size`; the live loop makes the back
/// buffer again between two frames).
pub const SIZE: u32 = 44;

/// Every tag of this build, the two retired ones (2 and 7) among them. A new tag goes here
/// with its constant, read by the player or not: a test asks that each constant is here.
pub const KNOWN: [u32; 44] = [
    TEXTURE, 2, SCENE_BEGIN, SCENE_END, FRAME_EVENT, OBJECT, 7, COMMAND, CALL, FRAME_END, HEADER, TEXTURE_UPDATE,
    READ_PIXELS, SHOT, DRAW_UP, EXTENSION, MARK, GEOMETRY, MATERIAL, SHAPE, LIGHT, OVERLAY, INSTANCE, INSTANCE_DRAW,
    INSTANCE_DELETE, INSTANCE_LIGHTS, INSTANCE_STATE, RETAINED_SCENE, INSTANCE_BONES, COPY_TILE, INSTANCE_QUEUED,
    INSTANCE_WALK, LIGHT_GONE, WORLD_LIGHTS, INSTANCE_PARTICLES, GAMMA_RAMP, TEXTURE_GONE, GEOMETRY_GONE,
    MATERIAL_GONE, INSTANCE_MOVE, GEOMETRY_UV, MORPH_TARGETS, MORPH_WEIGHTS, SIZE,
];

/// A tag that this build knows ([`KNOWN`]). A record of another tag comes from a plugin of
/// another build: the player counts it (`Counts::unknown_records`).
pub fn is_known(tag: u32) -> bool {
    KNOWN.contains(&tag)
}

/// The least length of a record of the tag: its fixed part, which holds the counts of the
/// variable parts ([`needed`] takes it from there). The lengths are those of the fixed parts
/// the plugin writes (`src/capture.rs` of the plugin). 0 for a tag the player does not read,
/// or reads with a check of its own on the whole record.
pub fn least_length(tag: u32) -> usize {
    match tag {
        HEADER => 8,
        // The handle, the unique id, the width, the height, the levels, the format, then
        // the table of the offsets of the levels (levels + 1 words), the widths and the
        // heights (a word each for each level).
        TEXTURE => 36,
        // The handle, the level, the width, the height, the format, the size.
        TEXTURE_UPDATE => 24,
        // `SceneV1` of the link: 48 bytes, with the origin at 24.
        SCENE_BEGIN => 48,
        FRAME_EVENT => 8,
        READ_PIXELS => 8,
        // The head of a call: kind, result, handle, the index of the data.
        CALL => 16,
        COMMAND => 16,
        OBJECT => 12,
        EXTENSION => 4,
        DRAW_UP => 20,
        GEOMETRY => 32,
        MATERIAL => 4,
        SHAPE => 144,
        LIGHT => 108,
        INSTANCE => 176,
        INSTANCE_DRAW => 12,
        INSTANCE_DELETE => 4,
        INSTANCE_LIGHTS => 8,
        INSTANCE_STATE => 8,
        RETAINED_SCENE => 8,
        INSTANCE_BONES => 24,
        COPY_TILE => 12,
        INSTANCE_QUEUED => 4,
        INSTANCE_WALK => 4,
        LIGHT_GONE => 4,
        WORLD_LIGHTS => 4,
        // The key, the count, the radius, the camera's right and up.
        INSTANCE_PARTICLES => 36,
        GAMMA_RAMP => 1536,
        TEXTURE_GONE => 4,
        GEOMETRY_GONE => 8,
        MATERIAL_GONE => 4,
        INSTANCE_MOVE => 104,
        GEOMETRY_UV => 28,
        MORPH_TARGETS => 16,
        MORPH_WEIGHTS => 12,
        SIZE => 12,
        _ => 0,
    }
}

/// The length that a record of the tag needs before the player reads it: its fixed part
/// ([`least_length`]), and the variable parts that the counts and the indexes of the fixed
/// part name, as the reader of the tag takes them (`player/feed.rs`, `renderer.rs`). A record
/// shorter than this is refused by `Player::feed` with one line, never read past its end.
/// While the record is shorter than its fixed part, the fixed part is the answer (the counts
/// cannot be read yet). The parts whose end a reader takes from the record's own length are
/// not in it, and those reads stop at the end: the pixels of a texture (each level is cut to
/// what the record has, and the rest of the level is zero), the particles of a particle
/// instance (the count is cut to the whole particles that the record has).
pub fn needed(tag: u32, data: &[u8]) -> usize {
    let head = least_length(tag);
    if data.len() < head {
        return head;
    }
    let count = |at: usize| word(data, at) as usize;
    let sum = |parts: &[usize]| parts.iter().fold(0usize, |total, part| total.saturating_add(*part));
    match tag {
        // The offsets (levels + 1), the widths and the heights of the levels.
        TEXTURE => sum(&[36, count(16).saturating_add(1).saturating_mul(4), count(16).saturating_mul(8)]),
        // The strip lengths (u16 each, rounded to words), the vertices, the index entries.
        GEOMETRY => {
            let lengths = (count(28).saturating_mul(2).saturating_add(3)) & !3;
            sum(&[32, lengths, count(20).saturating_mul(count(16)), count(24).saturating_mul(2)])
        }
        MORPH_TARGETS => sum(&[16, count(8).saturating_mul(count(12)).saturating_mul(12)]),
        MORPH_WEIGHTS => sum(&[12, count(8).saturating_mul(4)]),
        INSTANCE_LIGHTS => sum(&[8, count(4).saturating_mul(4)]),
        INSTANCE_BONES => sum(&[24, count(4).saturating_mul(BONE_BYTES)]),
        // The keys of the lights, the texture effects (flag 16), the fog (flag 32), the bones.
        SHAPE => {
            let flags = word(data, 8);
            let effects = if flags & 16 != 0 { 2 * EFFECT_STAGE_WORDS * 4 } else { 0 };
            let fog = if flags & 32 != 0 { 16 } else { 0 };
            sum(&[144, count(12).saturating_mul(4), effects, fog, count(140).saturating_mul(64)])
        }
        // The texture effects, the fog, then the count of the lights and the keys; the bones
        // of a skinned draw after them are checked by the reader.
        INSTANCE_DRAW => {
            let flags = word(data, 8);
            let effects = if flags & 16 != 0 { 2 * EFFECT_STAGE_WORDS * 4 } else { 0 };
            let fog = if flags & 32 != 0 { 16 } else { 0 };
            let count_at = sum(&[12, effects, fog]);
            if data.len() < count_at.saturating_add(4) {
                return count_at.saturating_add(4);
            }
            sum(&[count_at, 4, count(count_at).saturating_mul(4)])
        }
        // The arguments after the head, by the kind of the call: the words that the reader
        // takes, and for the viewport the index of its data.
        CALL => match word(data, 0) {
            CALL_VIEWPORT => sum(&[16, count(12).saturating_mul(4)]),
            CALL_CREATE_TEXTURE => 16 + 20,
            CALL_TEXTURE_LEVEL => 16 + 8,
            CALL_RENDER_TARGET => 16 + 4,
            CALL_COPY_PIXELS => 16 + 16,
            CALL_CLEAR => 16 + 24,
            _ => 16,
        },
        // A transform: the 16 floats of the matrix after the head. A transform with a null
        // matrix is the head alone (the plugin's `bytes_at` of a null address is empty): it
        // is valid, and the reader takes no matrix from it.
        COMMAND => {
            if word(data, 0) == COMMAND_TRANSFORM && data.len() != 16 {
                16 + 64
            } else {
                16
            }
        }
        _ => head,
    }
}

/// The name of a tag, for the line of a refused record.
pub fn name(tag: u32) -> &'static str {
    match tag {
        TEXTURE => "texture",
        SCENE_BEGIN => "scene begin",
        SCENE_END => "scene end",
        FRAME_EVENT => "frame event",
        OBJECT => "object",
        COMMAND => "command",
        CALL => "call",
        FRAME_END => "frame end",
        HEADER => "header",
        TEXTURE_UPDATE => "texture update",
        READ_PIXELS => "read pixels",
        SHOT => "shot",
        DRAW_UP => "draw up",
        EXTENSION => "extension",
        MARK => "mark",
        GEOMETRY => "geometry",
        MATERIAL => "material",
        SHAPE => "shape",
        LIGHT => "light",
        OVERLAY => "overlay",
        INSTANCE => "instance",
        INSTANCE_DRAW => "instance draw",
        INSTANCE_DELETE => "instance delete",
        INSTANCE_LIGHTS => "instance lights",
        INSTANCE_STATE => "instance state",
        RETAINED_SCENE => "retained scene",
        INSTANCE_BONES => "instance bones",
        COPY_TILE => "copy tile",
        INSTANCE_QUEUED => "instance queued",
        INSTANCE_WALK => "instance walk",
        LIGHT_GONE => "light gone",
        WORLD_LIGHTS => "world lights",
        INSTANCE_PARTICLES => "instance particles",
        GAMMA_RAMP => "gamma ramp",
        TEXTURE_GONE => "texture gone",
        GEOMETRY_GONE => "geometry gone",
        MATERIAL_GONE => "material gone",
        INSTANCE_MOVE => "instance move",
        GEOMETRY_UV => "geometry uv",
        MORPH_TARGETS => "morph targets",
        MORPH_WEIGHTS => "morph weights",
        SIZE => "size",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every tag the player reads: all the known tags but the two retired ones (2 and 7).
    const READ_TAGS: [u32; 42] = [
        HEADER, TEXTURE, TEXTURE_UPDATE, SCENE_BEGIN, SCENE_END, FRAME_EVENT, OBJECT, COMMAND, CALL, FRAME_END, READ_PIXELS, SHOT, DRAW_UP, EXTENSION, MARK, GEOMETRY, MATERIAL, SHAPE, LIGHT, OVERLAY, INSTANCE, INSTANCE_DRAW, INSTANCE_DELETE, INSTANCE_LIGHTS,
        INSTANCE_STATE, RETAINED_SCENE, INSTANCE_BONES, COPY_TILE, INSTANCE_QUEUED, INSTANCE_WALK, LIGHT_GONE, WORLD_LIGHTS, INSTANCE_PARTICLES, GAMMA_RAMP, TEXTURE_GONE, GEOMETRY_GONE, MATERIAL_GONE, INSTANCE_MOVE, GEOMETRY_UV, MORPH_TARGETS, MORPH_WEIGHTS, SIZE,
    ];

    #[test]
    fn the_test_reads_every_known_tag_but_the_retired_ones() {
        let known: Vec<u32> = (1..=SIZE).filter(|&tag| is_known(tag) && tag != 2 && tag != 7).collect();
        let mut read = READ_TAGS.to_vec();
        read.sort();
        assert_eq!(read, known);
    }

    #[test]
    fn a_transform_with_a_null_matrix_is_valid() {
        let mut record = vec![0u8; 16];
        record[0..4].copy_from_slice(&COMMAND_TRANSFORM.to_le_bytes());
        assert_eq!(needed(COMMAND, &record), 16);
        record.resize(40, 0);
        assert_eq!(needed(COMMAND, &record), 80);
    }

    /// A record of the tag with every part the reader takes: the fixed part, and variable
    /// parts with counts of 1 or 2, so that a cut inside any part is a short record.
    fn full_record(tag: u32) -> Vec<u8> {
        let mut record = vec![0u8; least_length(tag)];
        let put = |record: &mut Vec<u8>, at: usize, value: u32| record[at..at + 4].copy_from_slice(&value.to_le_bytes());
        match tag {
            TEXTURE => {
                put(&mut record, 16, 2);
                record.resize(36 + 3 * 4 + 2 * 8, 0);
            }
            GEOMETRY => {
                put(&mut record, 16, 8);
                put(&mut record, 20, 2);
                put(&mut record, 24, 3);
                put(&mut record, 28, 1);
                record.resize(32 + 4 + 16 + 6, 0);
            }
            MORPH_TARGETS => {
                put(&mut record, 8, 1);
                put(&mut record, 12, 2);
                record.resize(16 + 24, 0);
            }
            MORPH_WEIGHTS => {
                put(&mut record, 8, 2);
                record.resize(20, 0);
            }
            INSTANCE_LIGHTS => {
                put(&mut record, 4, 2);
                record.resize(16, 0);
            }
            INSTANCE_BONES => {
                put(&mut record, 4, 1);
                record.resize(24 + BONE_BYTES, 0);
            }
            SHAPE => {
                put(&mut record, 8, 16 | 32);
                put(&mut record, 12, 1);
                put(&mut record, 140, 1);
                record.resize(144 + 4 + 2 * EFFECT_STAGE_WORDS * 4 + 16 + 64, 0);
            }
            INSTANCE_DRAW => {
                put(&mut record, 8, 16 | 32);
                let count_at = 12 + 2 * EFFECT_STAGE_WORDS * 4 + 16;
                record.resize(count_at + 8, 0);
                put(&mut record, count_at, 1);
            }
            CALL => {
                put(&mut record, 0, CALL_CLEAR);
                record.resize(16 + 24, 0);
            }
            COMMAND => {
                put(&mut record, 0, COMMAND_TRANSFORM);
                record.resize(80, 0);
            }
            _ => {}
        }
        record
    }

    #[test]
    fn every_cut_of_every_record_is_refused_and_the_whole_is_taken() {
        for tag in READ_TAGS {
            let record = full_record(tag);
            for cut in 0..record.len() {
                // The head of a transform alone is a whole record: a transform with a null
                // matrix (`a_transform_with_a_null_matrix_is_valid`).
                if tag == COMMAND && cut == 16 {
                    continue;
                }
                let needed = needed(tag, &record[..cut]);
                assert!(needed > cut, "tag {tag} ({}): a record of {cut} bytes passes with {needed} needed", name(tag));
                assert!(needed <= record.len(), "tag {tag} ({}): {needed} needed from a cut of {cut} bytes, more than the whole record of {}", name(tag), record.len());
            }
            assert!(needed(tag, &record) <= record.len(), "tag {tag} ({}): the whole record is refused", name(tag));
        }
    }

    #[test]
    fn a_viewport_call_needs_its_data_at_the_index() {
        let mut record = vec![0u8; 16];
        record[0..4].copy_from_slice(&CALL_VIEWPORT.to_le_bytes());
        record[12..16].copy_from_slice(&3u32.to_le_bytes());
        assert_eq!(needed(CALL, &record), 16 + 12);
        // A count that does not fit into the memory: the record is refused, no overflow.
        record[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(needed(CALL, &record) > record.len());
    }

    #[test]
    fn a_short_read_gives_zero_and_counts() {
        let before = crate::reads_past_end();
        assert_eq!(word(&[1, 2], 0), 0);
        assert_eq!(crate::double(&[1, 2, 3, 4], 0), 0.0);
        assert_eq!(word(&[1, 0, 0, 0], usize::MAX), 0);
        assert_eq!(crate::reads_past_end(), before + 3);
    }

    #[test]
    fn every_record_with_a_fixed_head_has_a_least_length() {
        for tag in [HEADER, TEXTURE, TEXTURE_UPDATE, SCENE_BEGIN, FRAME_EVENT, READ_PIXELS, CALL, COMMAND, OBJECT, GEOMETRY, MATERIAL, SHAPE, LIGHT, INSTANCE, INSTANCE_DRAW, INSTANCE_DELETE, INSTANCE_LIGHTS, INSTANCE_STATE, RETAINED_SCENE, INSTANCE_BONES, COPY_TILE, INSTANCE_QUEUED, INSTANCE_WALK, LIGHT_GONE, WORLD_LIGHTS, INSTANCE_PARTICLES, GAMMA_RAMP, TEXTURE_GONE, GEOMETRY_GONE, MATERIAL_GONE, INSTANCE_MOVE, GEOMETRY_UV, MORPH_TARGETS, MORPH_WEIGHTS, SIZE] {
            assert!(least_length(tag) > 0, "tag {tag} has no least length");
        }
    }

    #[test]
    fn the_records_without_a_head_have_none() {
        for tag in [SCENE_END, FRAME_END, SHOT, MARK, OVERLAY, 0, 2, 7, 99] {
            assert_eq!(least_length(tag), 0, "tag {tag}");
        }
    }

    /// Each `pub const` tag of this file is in [`KNOWN`], so that a new tag the player does
    /// not read is not counted as a record of another build.
    #[test]
    fn every_tag_constant_is_known() {
        let source = include_str!("tag.rs");
        let constants: Vec<u32> = source
            .lines()
            .filter_map(|line| line.strip_prefix("pub const ")?.split_once(": u32 = "))
            .filter_map(|(_, value)| value.trim_end_matches(';').parse().ok())
            .collect();
        assert!(constants.len() >= 41, "the tag constants were not found");
        for tag in constants {
            assert!(is_known(tag), "tag {tag} is a constant but not in KNOWN");
        }
        assert!(!is_known(0) && !is_known(45));
    }

    #[test]
    fn the_scene_record_is_the_packet_of_the_link() {
        // `SceneV1` of `src/link.rs` of the plugin is 48 bytes; the origin is read at 24.
        assert_eq!(least_length(SCENE_BEGIN), 48);
    }
}
