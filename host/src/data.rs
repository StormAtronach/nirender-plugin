//! The data of a frame as the shader sees it ([`DrawData`], [`LightData`], [`StageData`]), the
//! geometry of a shape, the keys of the pipelines, the counts of a run, the times, and the
//! constants of the buffers.

use super::*;

/// The geometry of a shape from a `GEOMETRY` record: resident buffers, keyed by the
/// geometry data of the game. See `geometry_stream` of the plugin.
pub(crate) struct Geometry {
    pub(crate) fvf: u32,
    pub(crate) stride: u32,
    /// The number of entries of the index list.
    pub(crate) entries: u32,
    /// The lengths of the strips, or empty for a triangle list.
    pub(crate) strips: Vec<u16>,
    /// The buffers that hold the vertices and the indexes: a chunk of a pool, or a buffer of
    /// its own (the particles of a frame). The ranges of a pool, to give back.
    pub(crate) vertex: Rc<wgpu::Buffer>,
    pub(crate) index: Rc<wgpu::Buffer>,
    pub(crate) vertex_range: Option<pool::Range>,
    pub(crate) index_range: Option<pool::Range>,
    /// The first vertex of the geometry in its buffer (the base vertex of a draw) and its
    /// first index entry (added to the first index of a draw).
    pub(crate) base_vertex: i32,
    pub(crate) first_index: u32,
    /// The unique id of the geometry data of a `GEOMETRY` record; [`None`] for the quads of
    /// particles. A `GEOMETRY_GONE` record removes the geometry only when its id is this one:
    /// a later data at the same address has another id.
    pub(crate) unique: Option<u32>,
    /// The UV controller of the data, when the plugin sends its steps
    /// ([`crate::tag::GEOMETRY_UV`]): the draws take the step from the held vertices.
    pub(crate) uv: Option<crate::uv_animation::UvAnimation>,
    /// The frame of the last draw of a record that names the ranges
    /// ([`Renderer::geometry_frame`]), 0 for none. A new version of the geometry in the same
    /// frame takes new ranges: a write into the ranges lands before all draws of the frame's
    /// one submit, also those recorded before it.
    pub(crate) drawn_in: u32,
}

pub(crate) const COMMAND_TRANSFORM: u32 = 3;

pub(crate) const CALL_CLEAR: u32 = 4;

pub(crate) const CALL_VIEWPORT: u32 = 5;

pub(crate) const CALL_RENDER_TARGET: u32 = 7;

pub(crate) const CALL_TEXTURE_LEVEL: u32 = 15;

pub(crate) const CALL_CREATE_TEXTURE: u32 = 20;

pub(crate) const CALL_COPY_PIXELS: u32 = 23;

pub(crate) const EVENT_SET_TARGET: u32 = 2;
/// The walk of the world camera's root left the sky root: the world starts here (the
/// plugin's `world_walk`; the header bit [`HEADER_WORLD_WALK`]).
pub(crate) const EVENT_WORLD_WALK: u32 = 4;
/// The engine drew the main sorted queue of the Click: the end of its flush (the plugin's
/// `queue_flush`; the header bit [`HEADER_QUEUE_FLUSH`]).
pub(crate) const EVENT_QUEUE_FLUSHED: u32 = 5;
/// Bits of the fourth word of the header: the draws of the engine's sorted queue are marked;
/// the world's start comes as [`EVENT_WORLD_WALK`]; the scene records carry [`SCENE_BACK_BUFFER`];
/// the end of the flush of the main sorted queue comes as [`EVENT_QUEUE_FLUSHED`]; the table's
/// blends that the engine draws in its walk have their places there ([`tag::INSTANCE_WALK`]);
/// the scene records carry [`SCENE_WIREFRAME`].
pub(crate) const HEADER_QUEUE_MARKED: u32 = 1;
pub(crate) const HEADER_WORLD_WALK: u32 = 2;
pub(crate) const HEADER_SCENE_TARGET: u32 = 4;
pub(crate) const HEADER_QUEUE_FLUSH: u32 = 8;
pub(crate) const HEADER_WALK_PLACES: u32 = 16;
pub(crate) const HEADER_SCENE_WIREFRAME: u32 = 32;
pub(crate) const HEADER_ENVMAP_KNOWN: u32 = 64;
pub(crate) const HEADER_ENVMAP_LIT: u32 = 128;
/// A flag of the scene record: the renderer's target at `BeginPaint` is the back buffer.
pub(crate) const SCENE_BACK_BUFFER: u32 = 0x8;
/// A flag of the scene record: the game draws the world in wireframe (its shared wireframe
/// property, the console command ToggleWireframe).
pub(crate) const SCENE_WIREFRAME: u32 = 0x10;

/// The value of a state that the game did not set.
pub(crate) const UNKNOWN: u32 = 0x7FFF_FFFF;

pub(crate) fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

pub(crate) fn float(bytes: &[u8], offset: usize) -> f32 {
    f32::from_bits(word(bytes, offset))
}

pub(crate) fn floats<const N: usize>(bytes: &[u8], offset: usize) -> [f32; N] {
    core::array::from_fn(|i| float(bytes, offset + i * 4))
}

pub(crate) fn double(bytes: &[u8], offset: usize) -> f64 {
    f64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

// ---- the data of a draw for the shader

/// A light of a draw, as the shader sees it. From the state packet of the draw: all of its
/// lights, in their order (the plugin takes up to 64).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct LightData {
    pub diffuse: [f32; 4],
    pub ambient: [f32; 4],
    /// The position, relative to the origin of the scene when the draw is; `w` is the type
    /// of the light: 1 a point light, 2 a spot light, 3 a directional light.
    pub position: [f32; 4],
    /// The direction (not of length 1); `w` is the range.
    pub direction: [f32; 4],
    /// The three numbers of the falloff; `w` is the falloff of a spot light.
    pub attenuation: [f32; 4],
    /// The cosines of the half angles of a spot light; `z` is the radius of the light of the
    /// game (from its first specular word, the light record's radius; 0 when the light has
    /// none). The extension's shade can change them: then `x` is 1 over its cut-off, 0 for
    /// none (`light_reach`).
    pub cone: [f32; 4],
}

/// A texture stage of a draw, as the shader sees it.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct StageData {
    pub color: [u32; 4],
    pub alpha: [u32; 4],
    pub coordinate: [u32; 4],
    pub bump: [f32; 4],
    pub luminance: [f32; 4],
    /// The texture and the sampler of the stage among the arrays of the frame.
    pub slots: [u32; 4],
}

/// The uniform of a draw: `Draw` in `shader.wgsl`, where each field has its meaning.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct DrawData {
    pub world: [[f32; 16]; 4],
    pub view: [f32; 16],
    pub projection: [f32; 16],
    pub material_diffuse: [f32; 4],
    pub material_ambient: [f32; 4],
    pub material_emissive: [f32; 4],
    pub scene_ambient: [f32; 4],
    pub fog_color: [f32; 4],
    /// The start, the end, 1 when the fog is on, 1 when the fog is on the distance from the
    /// camera and not on the depth.
    pub fog: [f32; 4],
    pub texture_factor: [f32; 4],
    /// Lighting on, the vertex has a normal, the vertex has a colour, the number of weights
    /// (4: a skinned draw, the four bones of a vertex are indexes into the palette).
    pub flags: [u32; 4],
    /// The diffuse source, the emissive source, the alpha test function (0 is off), the
    /// alpha reference.
    pub more: [u32; 4],
    /// The number of lights, the number of stages, the ambient source, and bits: 1 the
    /// extension shades the draw, 4 the sky dome, 8 the shadow of a moon, 128 the colour is
    /// not kept between 0 and 1 between the texture stages; 2 and from 16 (but 128) are the
    /// extension's own.
    pub counts: [u32; 4],
    /// The lights of the draw in the light buffer of the frame: the first and the number.
    /// z: the first bone of a skinned draw in the palette buffer of the frame. w: bit 1
    /// when the light of a pixel of the draw can come from the clusters of the frame
    /// (`cluster_lights` in the shader) in place of the lights of the draw; bit 2 when the
    /// world matrix of the draw comes from its instance.
    pub light_range: [u32; 4],
    /// The lights of the draw among the clustered lights of the frame, one bit for each
    /// slot (`Clusters::note`): a pixel of the draw takes a light of its cluster only when
    /// the draw has it, as the engine lights a draw with the lights of its effect state.
    pub light_mask: [[u32; 4]; 2],
    pub stages: [StageData; 6],
    pub texture_transforms: [[f32; 16]; 6],
    /// 1 / width and -1 / height of the viewport.
    pub pixel: [f32; 4],
    /// The values of [`FrameParameters::sky`].
    pub extension: [[f32; 4]; 8],
    /// A plane in the space of the world of the draw: a pixel on its negative side is not
    /// drawn. All 0: no plane.
    pub clip_plane: [f32; 4],
}

/// The shader for a draw in screen coordinates: the colour of the vertex times the texture.
pub(crate) const SCREEN_SHADER: &str = "
pub(crate) struct Out { @builtin(position) position: vec4f, @location(0) color: vec4f, @location(1) uv: vec2f }
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var picture_sampler: sampler;
@vertex fn vs(@location(0) position: vec4f, @location(1) color: vec4f, @location(2) uv: vec2f) -> Out {
    return Out(position, color, uv);
}
@fragment fn fs(in: Out) -> @location(0) vec4f {
    return in.color * textureSample(picture, picture_sampler, in.uv);
}
";

/// The parts of the work that the program measures. The parts do not overlap: their sum is
/// the time that the program is busy.
pub(crate) const TIME_NAMES: [&str; TIME_COUNT] = [
    "read of the stream",
    "records that are not draws",
    "draw: the data of the draw",
    "draw: the upload of its data",
    "draw: textures, samplers, pipeline",
    "draw: the commands of the pass",
    "items that are not draws (passes, clears)",
    "extension: frame",
    "extension: before the world",
    "extension: after the world and blend",
    "extension: a class of draws",
    "extension: post-processing",
    "end of the frame and submit",
    "wait for a picture of the window",
    "copy into the window and present",
    "clusters of the lights",
    "buffers made for a frame (geometry records, particles, films)",
];

pub(crate) const TIME_COUNT: usize = 17;

pub(crate) const TIME_BUFFERS: usize = 16;

pub(crate) const TIME_CLUSTERS: usize = 15;

pub(crate) const TIME_READ: usize = 0;

pub(crate) const TIME_RECORDS: usize = 1;

pub(crate) const TIME_DRAW_DATA: usize = 2;

pub(crate) const TIME_DRAW_UPLOAD: usize = 3;

pub(crate) const TIME_DRAW_GROUPS: usize = 4;

pub(crate) const TIME_DRAW_PASS: usize = 5;

pub(crate) const TIME_ITEMS: usize = 6;

pub(crate) const TIME_EXT_FRAME: usize = 7;

pub(crate) const TIME_EXT_BEFORE: usize = 8;

pub(crate) const TIME_EXT_AFTER: usize = 9;

pub(crate) const TIME_EXT_CLASS: usize = 10;

pub(crate) const TIME_EXT_POST: usize = 11;

pub(crate) const TIME_END: usize = 12;

pub(crate) const TIME_ACQUIRE: usize = 13;

pub(crate) const TIME_PRESENT: usize = 14;

/// The sum of the measured times without one of them: for a part that has other measured
/// parts inside it, its own time is the time of the whole less the change of this sum.
pub(crate) fn sum_except(times: &[u64; TIME_COUNT], skip: usize) -> u64 {
    times.iter().enumerate().filter(|(i, _)| *i != skip).map(|(_, t)| *t).sum()
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PipelineKey {
    pub(crate) fvf: u32,
    pub(crate) stride: u32,
    pub(crate) topology: u32,
    pub(crate) blend: [u32; 3],
    pub(crate) depth: [u32; 3],
    pub(crate) cull: u32,
    /// D3DRS_FILLMODE: 2 is wireframe.
    pub(crate) fill: u32,
    /// The stencil: enable, fail, depth fail, pass, function, mask, write mask.
    pub(crate) stencil: [u32; 7],
    /// D3DRS_COLORWRITEENABLE: red 1, green 2, blue 4, alpha 8.
    pub(crate) color_write: u32,
    /// The samples for each pixel of the targets of the pass: [`Renderer::samples`] for the
    /// targets of the frame, 1 for a texture that the game or an extension draws into.
    pub(crate) samples: u32,
}

/// A texture that the game draws into.
pub(crate) struct RenderTexture {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: Rc<wgpu::TextureView>,
    pub(crate) depth: Rc<wgpu::TextureView>,
    pub(crate) size: (u32, u32),
}

#[derive(Default)]
pub(crate) struct Counts {
    pub(crate) draws: u32,
    /// The runs of draws in a row with one pipeline and one stencil reference: what one
    /// indirect call could draw.
    pub(crate) pipeline_runs: u32,
    /// Copies of a draw (clipped, replayed) that found no room in the records or the
    /// instances of the frame, and were drawn as they are or not at all.
    pub(crate) no_record: u32,
    /// The draws of the frame not drawn because the records reached the device's limit of a
    /// storage binding (`record_limit`), and the retained slots past it.
    pub(crate) records_over_limit: u32,
    pub(crate) retained_over_limit: u32,
    /// The uploads of the retained segment of the instances: whole, and as runs of changed
    /// slots with their bytes (`retained_upload`).
    pub(crate) instance_uploads_whole: u64,
    pub(crate) instance_upload_runs: u64,
    pub(crate) instance_upload_bytes: u64,
    /// The textures that came again for a handle with a slot of the arrays.
    pub(crate) textures_replaced: u32,
    /// The pinned textures whose last use went (their slot is free for another).
    pub(crate) textures_unpinned: u32,
    /// The `TEXTURE_GONE` records: of a handle held here, and of one not held (a vertex
    /// buffer's handle is not sent; a texture made before the stream).
    pub(crate) textures_gone: u64,
    pub(crate) textures_gone_not_held: u64,
    /// Freed textures whose handle had a new texture by the start of the next frame: the new
    /// one stays.
    pub(crate) textures_gone_replaced: u64,
    /// The `GEOMETRY_GONE` records: of a geometry held here, and of none or of another unique
    /// id (a data never drawn, or a later data at the same address).
    pub(crate) geometries_gone: u64,
    pub(crate) geometries_gone_not_held: u64,
    /// The retained instances that still named a geometry that went: taken out of the table
    /// until a geometry comes under that handle again (a shape whose data changed while it
    /// lived, between two notes of its handles).
    pub(crate) geometry_orphans: u64,
    /// The materials removed after a `MATERIAL_GONE` record.
    pub(crate) materials_gone: u64,
    /// The frames whose arrays stayed from the frame before and the frames that laid them
    /// out again (`begin_frame_arrays`), and the groups of the arrays made.
    pub(crate) arrays_kept: u64,
    pub(crate) arrays_laid_out: u64,
    pub(crate) array_groups_made: u64,
    /// The most textures of a frame in the arrays, and the stages that found no slot.
    pub(crate) most_frame_textures: u32,
    pub(crate) over_array_cap: u32,
    /// The textures of the retained instances that found no pinned slot (white).
    pub(crate) pinned_over: u32,
    /// The most pinned slots at a time, and the free slots dropped from the end of the room.
    pub(crate) pinned_most: u32,
    pub(crate) pinned_dropped: u32,
    /// The stages of the retained slots whose pinned texture or sampler was not in the
    /// arrays of the frame yet: they drew from a slot of the frame for one frame.
    pub(crate) pinned_late: u32,
    /// The textures of retained slots pinned before their record came (the white view until
    /// then; `set_texture` fills the slot).
    pub(crate) pinned_before_record: u32,
    /// The pinned slots in use that held a texture and took another one under the same
    /// handle (`set_texture`): with other facts (size, levels, format), with the same.
    pub(crate) pinned_replaced: [u32; 2],
    /// The indirect calls, the draws in them, and the draws drawn one by one because the
    /// buffer of the arguments was full.
    pub(crate) indirect_calls: u32,
    pub(crate) indirect_draws: u32,
    pub(crate) indirect_over: u32,
    pub(crate) other_target: u32,
    pub(crate) no_layout: u32,
    pub(crate) textures: u32,
    pub(crate) texture_updates: u32,
    pub(crate) render_textures: u32,
    pub(crate) pixel_reads: u32,
    /// The map tiles copied, the tile reads given to the game, and the tiles not copied.
    pub(crate) tile_copies: u32,
    pub(crate) tile_reads: u32,
    pub(crate) tiles_lost: u32,
    /// The copies of the picture and of the depth for the calls of the extension.
    pub(crate) picture_copies: u32,
    pub(crate) depth_copies: u32,
    /// Where the time of the program goes, in the order of [`TIME_NAMES`], in ticks (`ticks`).
    pub(crate) times: [u64; TIME_COUNT],
    /// The frames that are complete.
    pub(crate) frames: u32,
    /// The geometry records, their bytes, the draws from them, the records that did not
    /// parse, and the draws of more index entries than their geometry has.
    pub(crate) geometry_records: u32,
    /// The records of a known geometry with the same layout, written into its ranges.
    pub(crate) geometry_in_place: u32,
    /// The records of a known geometry with the same layout that a draw of the same frame
    /// named already: written into new ranges.
    pub(crate) geometry_drawn_in_frame: u32,
    /// The `GEOMETRY_UV` records: with the state of the vertices just sent, with a new state,
    /// and those of a geometry not held (or of another unique id).
    pub(crate) uv_bases: u32,
    pub(crate) uv_steps: u32,
    pub(crate) uv_missing: u32,
    /// The `MORPH_TARGETS` records, and the `MORPH_WEIGHTS` records for a geometry not held
    /// or before any targets.
    pub(crate) morph_targets: u32,
    pub(crate) morph_missing: u32,
    pub(crate) geometry_bytes: u64,
    pub(crate) geometry_draws: u32,
    pub(crate) geometry_bad: u32,
    pub(crate) geometry_short: u32,
    pub(crate) material_records: u32,
    /// The records shorter than the fixed part of their tag ([`tag::least_length`]), skipped.
    pub(crate) short_records: u32,
    /// The shape records, the draws made from them, and the records without a geometry or
    /// a material.
    pub(crate) shape_records: u32,
    pub(crate) shape_draws: u32,
    pub(crate) shape_missing: u32,
    /// The shapes with a rule of the engine that this program does not follow: a material
    /// without the smooth flag (the game sets `D3DSHADE_FLAT` for it), and a material with
    /// the specular flag (the game sets `D3DRS_SPECULARENABLE`).
    pub(crate) shape_flat: u32,
    pub(crate) shape_specular: u32,
    /// The shape records with a palette of bones, and the records of particle systems.
    pub(crate) shape_skinned: u32,
    pub(crate) shape_particles: u32,
    /// The instance records, the draws from them, the draws of an instance the helper does
    /// not have, and the shape records skipped as twins of an instance draw.
    pub(crate) instance_records: u32,
    /// The `INSTANCE_MOVE` records, and those an `INSTANCE` record of the same instance
    /// followed (the plugin's check mode), with other values than the move gave.
    pub(crate) instance_moves: u32,
    pub(crate) moves_checked: u32,
    pub(crate) moves_different: u32,
    pub(crate) instance_draws: u32,
    /// The `INSTANCE_PARTICLES` records: the particles of a particle instance's draw.
    pub(crate) instance_particles: u32,
    pub(crate) instance_missing: u32,
    pub(crate) shape_twins: u32,
    /// The blended draws of the engine's path merged into the sorted list of the table.
    pub(crate) merged_blends: u64,
    /// The places of the table's blends in the engine's queue ([`tag::INSTANCE_QUEUED`]):
    /// drawn there, of an instance the table does not have, outside the world scene, and of
    /// an instance whose slots the table does not sort (drawn with the other draws); and the
    /// sorted blends of the table without a place, put between by depth.
    pub(crate) queued_places: u64,
    pub(crate) queued_unknown: u64,
    pub(crate) queued_outside: u64,
    pub(crate) queued_not_sorted: u64,
    pub(crate) sorted_without_place: u64,
    /// Of those, the ones in frames with no place at all (risk 34: a cause of the frame).
    pub(crate) sorted_without_place_no_places: u64,
    /// The places that came after the frame's first sorted blends (a later generation).
    pub(crate) queued_late_places: u64,
    /// The items of the table's sorted blends that went before a depth clear after the world
    /// (the engine's queue still open there, `Plan::depth_clear`).
    pub(crate) blends_before_clear: u64,
    /// The places of the table's blends in the engine's walk ([`tag::INSTANCE_WALK`]): taken in
    /// a world view, of an instance the table does not have, outside a world view, and after
    /// the main view of the table drew its blends (a part of the frame ran before them).
    pub(crate) walk_places: u64,
    pub(crate) walk_unknown: u64,
    pub(crate) walk_outside: u64,
    pub(crate) walk_late: u64,
    /// The scenes whose record named the target, and of them the ones where the target that
    /// the `SetRenderTarget` calls gave was another one.
    pub(crate) scene_targets: u64,
    pub(crate) scene_target_changed: u64,
    /// The calls of [`Extension::draw_class`]: at the first draw of the class, and at the
    /// world's end of a frame that brought no draw of the class.
    pub(crate) class_calls: u64,
    pub(crate) class_calls_without_draw: u64,
    /// The draws of a world view (the world camera's scenes, the first-person scene) with an
    /// orthographic projection: the old test of the projection would have left them out.
    pub(crate) orthographic_world_draws: u64,
    /// The light records, the lights destroyed, and the lights of a shape that no record came
    /// for.
    pub(crate) light_records: u32,
    pub(crate) lights_gone: u32,
    pub(crate) light_missing: u32,
    /// The lights and the entries of the clusters of the frames.
    pub(crate) cluster_lights: u64,
    pub(crate) cluster_entries: u64,
    /// The lights of the frames for the extension ([`FrameLights`]).
    pub(crate) extension_lights: u64,
    /// The frames with more clustered lights than a mask of a draw has bits.
    pub(crate) cluster_frames_many: u32,
    /// The GPU time of the render passes and of the whole frame, in nanoseconds, and the
    /// frames that were timed.
    pub(crate) gpu_passes: f64,
    pub(crate) gpu_frame: f64,
    pub(crate) gpu_frames: u32,
    /// The GPU time of each named span of the frame (`stamps::Stamps`), in nanoseconds, and
    /// how many there were.
    pub(crate) gpu_named: Vec<(&'static str, crate::stamps::StampKind, f64, u64)>,
    pub(crate) screen_draws: u32,
    pub(crate) screen_draws_lost: u32,
    pub(crate) target_copies: u32,
    pub(crate) target_draws: u32,
    /// The kept draws that were drawn again for an extension.
    pub(crate) replayed: u32,
    /// The shapes that the extension took ([`Extension::hold`]).
    pub(crate) held: u32,
    pub(crate) texture_updates_lost: u32,
    pub(crate) lost_reasons: HashMap<String, u32>,
    pub(crate) textures_decoded: u32,
    pub(crate) texture_formats: HashMap<u32, u32>,
    pub(crate) missing_textures: u32,
    pub(crate) more_stages: u32,
    /// The most lights that a draw had.
    pub(crate) most_lights: u32,
    pub(crate) coordinate_work: u32,
}

pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

pub(crate) const STAGES: usize = 6;

/// The lights that the light buffer of a frame holds at first.
pub(crate) const LIGHTS_AT_FIRST: usize = 4096;

pub(crate) const BONES_AT_FIRST: usize = 4096;

pub(crate) const INSTANCES_AT_FIRST: usize = 4096;

/// An instance of a draw: its world matrix (read when the draw has the instanced bit of
/// `light_range[3]`) and the record of the draw in the records of the frame.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Instance {
    pub(crate) world: [f32; 16],
    pub(crate) record: [u32; 4],
}

pub(crate) const INSTANCE_SIZE: usize = core::mem::size_of::<Instance>();

/// The size of a record of a draw: [`DrawData`], as the shader's `array<Draw>` has it.
pub(crate) const RECORD_SIZE: usize = core::mem::size_of::<DrawData>();

const _: () = assert!(INSTANCE_SIZE == 80 && RECORD_SIZE % 16 == 0);

pub(crate) const IDENTITY: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

pub(crate) const RECORDS_AT_FIRST: usize = 4096;

/// The automatic room of the texture arrays (the extension's `texture_slots` 0): at most the
/// adapter's room. A larger room costs time in the bind groups of each frame.
pub(crate) const AUTO_TEXTURE_SLOTS: u32 = 16384;

/// The least room of the texture arrays (the renderer needs it to start).
pub(crate) const MIN_TEXTURE_SLOTS: u32 = 256;

/// The slots that the textures of the retained instances leave to the frame's own draws (the
/// UI, the sky, the particles, the actors): the pinned slots and the frame's share one room.
pub(crate) const FRAME_RESERVE: usize = 256;

pub(crate) const SAMPLER_CAP: u32 = 64;

/// The arguments of one indexed indirect draw, as `multi_draw_indexed_indirect` reads them.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct IndirectArgs {
    pub(crate) index_count: u32,
    pub(crate) instance_count: u32,
    pub(crate) first_index: u32,
    pub(crate) base_vertex: i32,
    pub(crate) first_instance: u32,
}

pub(crate) const ARGS_SIZE: usize = core::mem::size_of::<IndirectArgs>();

const _: () = assert!(ARGS_SIZE == 20);

/// The size of a buffer of the pools of the game's geometry.
pub(crate) const VERTEX_CHUNK: u64 = 64 << 20;

pub(crate) const INDEX_CHUNK: u64 = 16 << 20;
