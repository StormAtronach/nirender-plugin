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
