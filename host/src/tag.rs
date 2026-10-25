//! The tags of the records of the stream, as `src/capture.rs` of the plugin writes them.

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

/// The least length of a record of the tag that the player reads without a check of its
/// own: a shorter record is skipped and counted (`Counts::short_records`), not read past
/// its end. The lengths are those of the fixed parts the plugin writes (`src/capture.rs`
/// of the plugin). 0 for a tag the player does not read, or reads with its own check.
pub fn least_length(tag: u32) -> usize {
    match tag {
        HEADER => 8,
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
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_record_with_a_fixed_head_has_a_least_length() {
        for tag in [HEADER, SCENE_BEGIN, FRAME_EVENT, READ_PIXELS, CALL, COMMAND, OBJECT, GEOMETRY, MATERIAL, SHAPE, LIGHT, INSTANCE, INSTANCE_DRAW, INSTANCE_DELETE, INSTANCE_LIGHTS, INSTANCE_STATE, RETAINED_SCENE, INSTANCE_BONES, COPY_TILE, INSTANCE_QUEUED, INSTANCE_WALK, LIGHT_GONE, WORLD_LIGHTS, INSTANCE_PARTICLES, GAMMA_RAMP, TEXTURE_GONE, GEOMETRY_GONE, MATERIAL_GONE] {
            assert!(least_length(tag) > 0, "tag {tag} has no least length");
        }
    }

    #[test]
    fn the_records_without_a_head_have_none() {
        for tag in [SCENE_END, FRAME_END, SHOT, MARK, OVERLAY, TEXTURE, TEXTURE_UPDATE, 0, 2, 7, 99] {
            assert_eq!(least_length(tag), 0, "tag {tag}");
        }
    }

    #[test]
    fn the_scene_record_is_the_packet_of_the_link() {
        // `SceneV1` of `src/link.rs` of the plugin is 48 bytes; the origin is read at 24.
        assert_eq!(least_length(SCENE_BEGIN), 48);
    }
}
