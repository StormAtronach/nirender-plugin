//! The render link: the contract between the ports of this plugin (the client) and `own_host`
//! (the host).
//!
//! `contract/mge_render_link.h` is the source of the contract. This module has the same
//! structs and constants. `tools/check_contract.py` compares the sizes and the constants.

use core::mem::size_of;

pub const VERSION: u32 = 1;

pub const CAP_COMPARE: u32 = 0x1;
pub const CAP_USE_FACTS: u32 = 0x2;
pub const CAP_DRAW_STATE: u32 = 0x4;
pub const CAP_TEXTURES: u32 = 0x8;
pub const CAP_WORLD_SPACE: u32 = 0x10;
pub const CAP_CAMERA_SPACE: u32 = 0x20;
pub const CAP_COMMANDS: u32 = 0x40;
pub const CAP_CALLS: u32 = 0x80;

pub const KIND_FRAME: u32 = 0x1;
pub const KIND_BUFFERS: u32 = 0x2;
pub const KIND_OBJECTS: u32 = 0x4;
pub const KIND_TARGETS: u32 = 0x8;

pub const CALL_TEST_DEVICE: u32 = 1;
pub const CALL_BEGIN_SCENE: u32 = 2;
pub const CALL_END_SCENE: u32 = 3;
pub const CALL_CLEAR: u32 = 4;
pub const CALL_VIEWPORT: u32 = 5;
pub const CALL_PRESENT: u32 = 6;
pub const CALL_RENDER_TARGET: u32 = 7;
pub const CALL_CREATE_VERTEX_BUFFER: u32 = 8;
pub const CALL_CREATE_INDEX_BUFFER: u32 = 9;
pub const CALL_ADD_REF: u32 = 10;
pub const CALL_RELEASE: u32 = 11;
pub const CALL_BUFFER_LOCK: u32 = 12;
pub const CALL_BUFFER_UNLOCK: u32 = 13;
pub const CALL_BUFFER_DESCRIPTION: u32 = 14;
pub const CALL_TEXTURE_LEVEL: u32 = 15;
pub const CALL_CUBE_FACE: u32 = 16;
pub const CALL_SURFACE_DESCRIPTION: u32 = 17;
pub const CALL_SURFACE_LOCK: u32 = 18;
pub const CALL_SURFACE_UNLOCK: u32 = 19;
pub const CALL_CREATE_TEXTURE: u32 = 20;
pub const CALL_CREATE_DEPTH_BUFFER: u32 = 21;
pub const CALL_CREATE_IMAGE_SURFACE: u32 = 22;
pub const CALL_COPY_PIXELS: u32 = 23;
pub const CALL_GET_RENDER_TARGET: u32 = 24;
pub const CALL_GET_DEPTH_BUFFER: u32 = 25;
pub const CALL_DISCARD_BYTES: u32 = 26;

pub const COMMAND_RENDER_STATE: u32 = 1;
pub const COMMAND_STAGE_STATE: u32 = 2;
pub const COMMAND_TRANSFORM: u32 = 3;
pub const COMMAND_DRAW_INDEXED: u32 = 4;
pub const COMMAND_DRAW: u32 = 5;

pub const SCENE_HAS_CAMERA_ORIGIN: u32 = 0x1;
pub const SCENE_INTERIOR: u32 = 0x2;
pub const SCENE_SKY_HIDDEN: u32 = 0x4;

/// The engine's sorted queue stays open after the scene: a later scene of the same camera
/// flushes it (`ports::base::main_queue_open`).
pub const SCENE_END_QUEUE_OPEN: u32 = 0x1;

pub const TEXTURE_DYNAMIC: u32 = 0x1;

pub const SCENE_OTHER: u32 = 0;
pub const SCENE_WORLD: u32 = 1;
pub const SCENE_SORTED_ALPHA: u32 = 2;
pub const SCENE_WATER: u32 = 3;
pub const SCENE_SHADOW_VOLUME: u32 = 4;
pub const SCENE_SHADOW_CASTERS: u32 = 5;
pub const SCENE_FIRST_PERSON: u32 = 6;
pub const SCENE_UI: u32 = 7;
pub const SCENE_RACE_MENU_HEAD: u32 = 8;
pub const SCENE_SPLASH: u32 = 9;
pub const SCENE_CHARACTER_PREVIEW: u32 = 10;
pub const SCENE_LOCAL_MAP: u32 = 11;
pub const SCENE_WATER_REFLECTION: u32 = 12;

pub const DRAW_SHAPE: u32 = 1;
pub const DRAW_STRIPS: u32 = 2;
pub const DRAW_POINTS: u32 = 3;
pub const DRAW_LINES: u32 = 4;
pub const DRAW_SCREEN_POLYGON: u32 = 5;

pub const CLASS_UNKNOWN: u32 = 0;
pub const CLASS_SKY_DOME: u32 = 1;
pub const CLASS_SKY_NIGHT: u32 = 2;
pub const CLASS_SKY_CLOUDS: u32 = 3;
pub const CLASS_SKY_SUN: u32 = 4;
pub const CLASS_SKY_MOON: u32 = 5;
pub const CLASS_SKY_MOON_SHADOW: u32 = 6;
pub const CLASS_SKY_OTHER: u32 = 7;
pub const CLASS_SUN_GLARE: u32 = 8;
pub const CLASS_WEATHER_PARTICLES: u32 = 9;
pub const CLASS_WATER_PLANE: u32 = 10;
pub const CLASS_LANDSCAPE: u32 = 11;
pub const CLASS_OBJECT: u32 = 12;
pub const CLASS_SHADOW_VOLUME: u32 = 13;
pub const CLASS_SHADOW_OVERLAY: u32 = 14;
pub const CLASS_FIRST_PERSON: u32 = 15;
pub const CLASS_UI: u32 = 16;
pub const CLASS_WATER_RIPPLE: u32 = 17;

pub const STATE_SKINNED: u32 = 0x1;
pub const STATE_LATER_PASS: u32 = 0x2;

pub const DRAW_FROM_SORTED_QUEUE: u32 = 0x1;
pub const DRAW_SKINNED: u32 = 0x2;

pub const LIGHT_AMBIENT: u32 = 0;
pub const LIGHT_DIRECTIONAL: u32 = 1;
pub const LIGHT_POINT: u32 = 2;
pub const LIGHT_SPOT: u32 = 3;
pub const LIGHT_SUN: u32 = 4;

pub const EVENT_CLEAR: u32 = 1;
pub const EVENT_SET_TARGET: u32 = 2;
pub const EVENT_SWAP: u32 = 3;
/// The walk of the world camera's root left the sky root: the world of the frame starts here
/// (`world_walk`).
pub const EVENT_WORLD_WALK: u32 = 4;

pub const FACT_LIGHT_RADIUS: u32 = 0;
pub const FACT_SUN_LIGHT: u32 = 1;
pub const FACT_MOON_SHADOW: u32 = 2;
pub const FACT_WATER_PLANE: u32 = 3;
pub const FACT_UI_SCENE: u32 = 4;
pub const FACT_SCENE_KIND: u32 = 5;
pub const FACT_SKY_DRAW: u32 = 6;
pub const FACT_LAND_SPLAT: u32 = 7;
pub const FACT_LATER_VIEW: u32 = 8;
pub const FACT_STATE_DRAW: u32 = 9;
pub const FACT_STATE_BUFFERS: u32 = 10;
pub const FACT_STATE_TRANSFORM: u32 = 11;
pub const FACT_STATE_BLEND: u32 = 12;
pub const FACT_STATE_DEPTH: u32 = 13;
pub const FACT_STATE_LIGHTING: u32 = 14;
pub const FACT_STATE_MATERIAL: u32 = 15;
pub const FACT_STATE_STAGES: u32 = 16;
pub const FACT_STATE_LIGHTS: u32 = 17;
pub const FACT_STATE_TEXTURES: u32 = 18;
pub const FACT_STATE_TEXTURE_TRANSFORMS: u32 = 19;
pub const FACT_STATE_BONES: u32 = 20;
pub const FACT_STATE_AMBIENT: u32 = 21;
pub const FACT_TEXTURE_CREATE: u32 = 22;
pub const FACT_STATE_DEPTH_STENCIL: u32 = 23;
pub const FACT_STATE_SAMPLERS: u32 = 24;
pub const FACT_DEVICE_RENDER_STATES: u32 = 25;
pub const FACT_DEVICE_STAGE_STATES: u32 = 26;
pub const FACT_DEVICE_TEXTURES: u32 = 27;
pub const FACT_DEVICE_BUFFERS: u32 = 28;
pub const FACT_DEVICE_TRANSFORMS: u32 = 29;
pub const FACT_DEVICE_LIGHTS: u32 = 30;
pub const FACT_CLIENT_CAMERA_SPACE: u32 = 31;
pub const STATE_SAMPLER_STATES: u32 = 5;
pub const STATE_UNKNOWN: u32 = 0x7FFF_FFFF;
pub const STATE_STAGES: u32 = 8;
pub const STATE_STAGE_STATES: u32 = 17;
pub const FACT_COUNT: u32 = 32;

/// The names of the facts, for reports. The index is `FACT_*`.
pub const FACT_NAMES: [&str; 32] = [
    "light radius",
    "sun light",
    "moon shadow",
    "water plane",
    "ui scene",
    "scene kind",
    "sky draw",
    "land splat",
    "later view",
    "state draw",
    "state buffers",
    "state transform",
    "state blend",
    "state depth",
    "state lighting",
    "state material",
    "state stages",
    "state lights",
    "state textures",
    "state texture transforms",
    "state bones",
    "state ambient",
    "texture create",
    "state depth stencil",
    "state samplers",
    "device render states",
    "device stage states",
    "device textures",
    "device buffers",
    "device transforms",
    "device lights",
    "client camera space",
];

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SceneV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub camera: u32,
    pub scene_root: u32,
    pub render_target: u32,
    pub flags: u32,
    pub camera_origin: [f64; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LightV1 {
    pub light: u32,
    pub device_index: u32,
    pub light_type: u32,
    pub radius: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DrawV1 {
    pub struct_size: u32,
    pub kind: u32,
    pub object_class: u32,
    pub flags: u32,
    pub geometry: u32,
    pub geometry_data: u32,
    pub skin_instance: u32,
    pub property_state: u32,
    pub effect_state: u32,
    pub material_shininess: f32,
    pub light_count: u32,
    pub lights: *const LightV1,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DrawStateV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub primitive_type: u32,
    pub base_vertex_index: u32,
    pub min_index: u32,
    pub vertex_count: u32,
    pub start_index: u32,
    pub primitive_count: u32,
    pub vertex_buffer: u32,
    pub vertex_stride: u32,
    pub vertex_format: u32,
    pub index_buffer: u32,
    pub world: [f32; 16],
    pub depth_write: u32,
    pub cull_mode: u32,
    pub vertex_blend: u32,
    pub blend_enable: u32,
    pub source_blend: u32,
    pub destination_blend: u32,
    pub alpha_test_enable: u32,
    pub alpha_function: u32,
    pub alpha_reference: u32,
    pub lighting: u32,
    pub fog_enable: u32,
    pub diffuse_material_source: u32,
    pub emissive_material_source: u32,
    pub ambient: u32,
    pub material: [f32; 17],
    pub light_count: u32,
    pub lights: *const StateLightV1,
    pub stages: [[u32; STATE_STAGE_STATES as usize]; STATE_STAGES as usize],
    pub blend_worlds: [[f32; 16]; 3],
    pub textures: [u32; STATE_STAGES as usize],
    pub texture_transforms: [[f32; 16]; STATE_STAGES as usize],
    pub depth_enable: u32,
    pub depth_function: u32,
    pub stencil: [u32; 8],
    pub samplers: [[u32; STATE_SAMPLER_STATES as usize]; STATE_STAGES as usize],
    pub world_relative_mask: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct StateLightV1 {
    pub device_index: u32,
    pub d3d_light: [u32; 26],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TextureV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub width: u32,
    pub height: u32,
    pub levels: u32,
    pub d3d_format: u32,
    pub pixel_format: u32,
    pub pixel_format_tag: u32,
    pub bytes_per_pixel: u32,
    pub pixels: *const u8,
    pub level_offsets: *const u32,
    pub level_widths: *const u32,
    pub level_heights: *const u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FactCountV1 {
    pub both_yes: u32,
    pub both_no: u32,
    pub host_only: u32,
    pub packet_only: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CountersV1 {
    pub struct_size: u32,
    pub scenes: u32,
    pub draws: u32,
    pub draws_without_packet: u32,
    pub facts: [FactCountV1; FACT_COUNT as usize],
}

#[repr(C)]
pub struct HostV1 {
    pub struct_size: u32,
    pub version: u32,
    pub capabilities: u32,
    pub reserved: u32,
    pub scene_begin: unsafe extern "C" fn(scene: *const SceneV1),
    pub scene_end: unsafe extern "C" fn(),
    pub frame_event: unsafe extern "C" fn(kind: u32, a: u32, b: u32),
    pub draw: unsafe extern "C" fn(draw: *const DrawV1),
    pub read_counters: unsafe extern "C" fn(out: *mut CountersV1, reset: u32),
    /// Only a host with `CAP_USE_FACTS` has this member.
    pub use_facts: unsafe extern "C" fn(fact_mask: u32) -> u32,
    /// Only a host with `CAP_DRAW_STATE` has this member.
    pub draw_state: unsafe extern "C" fn(state: *const DrawStateV1),
    /// Only a host with `CAP_TEXTURES` has this member.
    pub texture_create: unsafe extern "C" fn(texture: *const TextureV1) -> u32,
    /// Only a host with `CAP_WORLD_SPACE` has this member.
    pub world_space: unsafe extern "C" fn() -> u32,
    /// Only a host with `CAP_CAMERA_SPACE` has this member.
    pub camera_space: unsafe extern "C" fn(origin: *mut f64) -> u32,
    /// Only a host with `CAP_COMMANDS` has this member.
    pub command: unsafe extern "C" fn(command: u32, a: u32, b: u32, c: u32),
    /// Only a host with `CAP_CALLS` has this member and the next one.
    pub call: unsafe extern "C" fn(call: u32, args: *const u32) -> u32,
    pub use_kinds: unsafe extern "C" fn(kind_mask: u32) -> u32,
}

// The sizes of the header. They are for the 32-bit target only.
#[cfg(target_pointer_width = "32")]
const _: () = {
    assert!(size_of::<SceneV1>() == 48);
    assert!(size_of::<LightV1>() == 16);
    assert!(size_of::<DrawV1>() == 48);
    assert!(size_of::<StateLightV1>() == 108);
    assert!(size_of::<DrawStateV1>() == 1728);
    assert!(size_of::<FactCountV1>() == 16);
    assert!(size_of::<CountersV1>() == 528);
    assert!(size_of::<TextureV1>() == 52);
    assert!(size_of::<HostV1>() == 68);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fact_names_are_in_the_table() {
        assert!(FACT_NAMES.len() <= FACT_COUNT as usize);
        assert_eq!(FACT_NAMES[FACT_LAND_SPLAT as usize], "land splat");
        assert_eq!(size_of::<LightV1>(), 16);
    }
}
