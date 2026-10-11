//! Draws a capture of the render link stream with wgpu, without the game.
//!
//!     nirender-host <capture file> <picture.png> [--backend vulkan|dx12] [--frame N] [--anisotropy N]
//!     nirender-host --live <name of the shared memory> [--backend vulkan|dx12] [--frames N] [--shot picture.png]
//!                   [--embed]
//!
//! With `--embed` the window of the host is a child of the window of the game and covers it:
//! the picture of the host is where the picture of the game was.
//!
//! The capture comes from `nirender.capture` of the plugin (`src/capture.rs` there has the
//! format). This program reads the records in their order, keeps the textures and the buffers,
//! and draws one frame into a texture, which it saves as a PNG file. In the live mode the
//! records come from a ring in shared memory while the game runs, and each frame goes into a
//! window.
//!
//! It is a spike. What it does not do is in the counts that it prints at the end.

use std::collections::HashMap;
use std::rc::Rc;

/// The wgpu of this crate, for a program that draws with it through `Extension`.
pub use wgpu;

/// What a program that draws more than the game does gets at the start of the world of a
/// frame. The matrices have the layout of Direct3D: 16 numbers, row after row, for a
/// position as a row vector (`position * view * projection`).
pub struct SceneView<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    /// The formats of the targets of the pass.
    pub target_format: wgpu::TextureFormat,
    pub depth_format: wgpu::TextureFormat,
    pub target_size: (u32, u32),
    /// From the world space of the game to the camera, and the projection of the game.
    pub view: [f32; 16],
    pub projection: [f32; 16],
    /// The player is in an interior cell.
    pub interior: bool,
    /// The fog of the game: its colour, and the distances of its start and its end.
    pub fog_color: [f32; 4],
    pub fog_start: f32,
    pub fog_end: f32,
    /// The direction that the light of the sun goes in (length 1), and its colour, when the draw of
    /// the game that starts the world has such a light.
    pub sun_direction: Option<[f32; 3]>,
    pub sun_color: [f32; 3],
    /// The ambient colour of that light.
    pub sun_ambient: [f32; 3],
    /// The ambient light of the scene.
    pub ambient: [f32; 3],
}

/// How the renderer draws the kept world draws of the frame again. See `Replay`.
pub struct ReplayView {
    /// A change of the world before the view of the game, in the space of the world of the
    /// game and with the layout of Direct3D: a mirror for a reflection. `None`: no change.
    pub world: Option<[f64; 16]>,
    /// Another view matrix and not that of the game, for a camera at another place: the
    /// light of a shadow map. `None`: the view of each draw.
    pub view: Option<[f32; 16]>,
    /// Another projection. `None`: that of each draw.
    pub projection: Option<[f32; 16]>,
    /// The size of the target of the pass.
    pub target_size: (u32, u32),
    /// True for a mirror: the front side of a triangle is its other side.
    pub flip_cull: bool,
    /// `[a, b, c, d]` in the space of the world of the game: only where
    /// `a * x + b * y + c * z + d >= 0` is drawn.
    pub clip_plane: Option<[f64; 4]>,
    /// Only the draws that write depth and do not blend.
    pub opaque_only: bool,
}

/// Draws the kept draws of the world of the frame again: those of the game in its world
/// scene and its scene of sorted draws, into the back buffer, up to this moment. The pass
/// must have a colour target and a depth target with the formats of `SceneView`.
pub trait Replay {
    fn replay(&mut self, pass: &mut wgpu::RenderPass<'static>, view: &ReplayView);
    /// The number of kept draws.
    fn count(&self) -> usize;
}

/// One kept draw of the world of a frame.
struct Kept {
    key: PipelineKey,
    stencil_reference: u32,
    data: DrawData,
    /// The origin that the world matrices of the draw are relative to, or `None`.
    origin: Option<[f64; 3]>,
    textures: Rc<wgpu::BindGroup>,
    samplers: Rc<wgpu::BindGroup>,
    vertices: Rc<wgpu::Buffer>,
    indexes: Option<Rc<wgpu::Buffer>>,
    fan: bool,
    /// The first index or vertex, the number of them, and the base vertex.
    range: (u32, u32, i32),
}

/// The kept draws with the renderer, for `Replay`.
struct Replayer<'a> {
    renderer: &'a mut Renderer,
    kept: &'a [Kept],
}

/// `a * b` for two matrices with the layout of Direct3D.
fn multiply64(a: &[f64; 16], b: &[f64; 16]) -> [f64; 16] {
    core::array::from_fn(|i| (0..4).map(|k| a[i / 4 * 4 + k] * b[k * 4 + i % 4]).sum())
}

impl Replay for Replayer<'_> {
    fn count(&self) -> usize {
        self.kept.len()
    }

    fn replay(&mut self, pass: &mut wgpu::RenderPass<'static>, view: &ReplayView) {
        for kept in self.kept {
            if view.opaque_only && (kept.key.blend[0] != 0 || kept.key.depth[1] == 0) {
                continue;
            }
            let mut data = kept.data;
            let origin = kept.origin.unwrap_or([0.0; 3]);
            if let Some(matrix) = view.view {
                data.view = matrix;
            }
            if let Some(world) = view.world {
                // The change is in the space of the world of the game. The world of the draw
                // can be relative to an origin: to that space, the change, and back.
                let mut to = [0.0f64; 16];
                let mut from = [0.0f64; 16];
                for i in 0..4 {
                    to[i * 5] = 1.0;
                    from[i * 5] = 1.0;
                }
                for axis in 0..3 {
                    to[12 + axis] = origin[axis];
                    from[12 + axis] = -origin[axis];
                }
                let change = multiply64(&multiply64(&to, &world), &from);
                let old: [f64; 16] = data.view.map(|v| v as f64);
                data.view = multiply64(&change, &old).map(|v| v as f32);
            }
            if let Some(projection) = view.projection {
                data.projection = projection;
            }
            data.pixel = [1.0 / view.target_size.0.max(1) as f32, -1.0 / view.target_size.1.max(1) as f32, 0.0, 0.0];
            if let Some(plane) = view.clip_plane {
                let d = plane[3] + plane[0] * origin[0] + plane[1] * origin[1] + plane[2] * origin[2];
                data.clip_plane = [plane[0] as f32, plane[1] as f32, plane[2] as f32, d as f32];
            }
            let mut key = kept.key;
            if view.flip_cull {
                // D3DCULL_CW is 2 and D3DCULL_CCW is 3.
                key.cull = match key.cull {
                    2 => 3,
                    3 => 2,
                    other => other,
                };
            }
            let Some(pipeline) = self.renderer.pipeline(key) else { continue };
            let (group, offset) = self.renderer.draw_slot(&data);
            pass.set_pipeline(&pipeline);
            pass.set_stencil_reference(kept.stencil_reference);
            pass.set_bind_group(0, &*group, &[offset]);
            pass.set_bind_group(1, &*kept.textures, &[]);
            pass.set_bind_group(2, &*kept.samplers, &[]);
            pass.set_vertex_buffer(0, kept.vertices.slice(..));
            pass.set_vertex_buffer(1, self.renderer.defaults.slice(..));
            let (first, count, base) = kept.range;
            if let Some(indexes) = kept.indexes.as_ref() {
                pass.set_index_buffer(indexes.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(first..first + count, base, 0..1);
            } else if kept.fan {
                pass.set_index_buffer(self.renderer.fan_indexes.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..count, first as i32, 0..1);
            } else {
                pass.draw(first..first + count, 0..1);
            }
            self.renderer.counts.replayed += 1;
        }
    }
}

/// The light for each pixel of MGE XE, for the draws of the world of a frame.
#[derive(Clone, Copy)]
pub struct PerPixel {
    /// The numbers that the light of the sun and the ambient light are multiplied with: the
    /// settings of MGE XE for the weather.
    pub sun: f32,
    pub ambient: f32,
    /// A point light ends at this number times its radius. 0: a light has no end.
    pub fade: f32,
}

/// A program that draws more than the game does: for example the distant land of MGE XE.
pub trait Extension {
    /// The renderer calls this one time in a frame, before the first draw of the world of
    /// the game: the first draw into the back buffer with fog, with depth writes and with a
    /// perspective projection. The sky of the game is in the picture then. The pass has the
    /// colour target and the depth target of the frame.
    ///
    /// Returns true when the depth target must be cleared before the draws of the game: the
    /// extension drew with a projection of its own.
    fn before_world(&mut self, scene: &SceneView, pass: &mut wgpu::RenderPass<'static>) -> bool;

    /// The renderer calls this one time in a frame, at the end of the world scene of the
    /// game: its draws without a sort are in the picture and in the depth target, and the
    /// draws with a sort, the first-person view and the user interface are not. The pass and
    /// the matrices are those of the game. MGE XE draws its grass here.
    fn after_world(&mut self, _scene: &SceneView, _pass: &mut wgpu::RenderPass<'static>) {}

    /// Data that the part of this program in the game process put into the stream, with the
    /// number that the two parts agree on. See `nirender_stream_extension` of the plugin.
    fn data(&mut self, _id: u32, _bytes: &[u8]) {}

    /// WGSL source with the two sky functions of the extension, or `None`. The renderer
    /// asks one time, before it makes its shader, and puts the source at the end of it:
    ///
    /// - `fn extension_sky_position(position: vec3f) -> vec3f`: the position of a vertex of
    ///   the sky dome in its model, for a dome of another shape.
    /// - `fn extension_sky_color(direction: vec3f, pixel: vec2f) -> vec3f`: the colour of
    ///   the sky in a direction from the camera (length 1, in the space of the world), for
    ///   the pixel at these coordinates.
    ///
    /// The source reads the values of `sky` as `draw.extension[0]` to `draw.extension[7]`.
    fn sky_shader(&self) -> Option<String> {
        None
    }

    /// The values for the sky functions in this frame, or `None` for the sky of the game as
    /// it is. The renderer asks at the first sky draw of a frame. With values, the sky dome
    /// of the game (a sky draw without a texture) gets its shape and its colour from the
    /// functions, and the shadow of a moon gets its colour from them and keeps its alpha.
    fn sky(&mut self) -> Option<[[f32; 4]; 8]> {
        None
    }

    /// True when the extension wants the call `blend` in this frame. The renderer asks after
    /// a `before_world` that gave true, and then keeps the picture of that moment.
    fn wants_blend(&self) -> bool {
        false
    }

    /// The renderer calls this after `after_world`, when `wants_blend` gave true. The pass
    /// draws into the picture of the frame and has no depth target. `before` is the picture
    /// as it was after `before_world`, and `depth` is the depth of the draws of the game:
    /// 1 where the game drew nothing. MGE XE lets the draws of the game near the end of
    /// their view range go into its distant land here.
    fn blend(&mut self, _scene: &SceneView, _pass: &mut wgpu::RenderPass<'static>, _before: &wgpu::TextureView, _depth: &wgpu::TextureView) {}

    /// `Some` when the draws of the world of this frame get their light for each pixel, as
    /// the shader of MGE XE makes it, and not for each vertex as the game does. The renderer
    /// asks after `before_world`.
    fn per_pixel(&self) -> Option<PerPixel> {
        None
    }

    /// The distances of the start and of the end of the fog for the draws of the world of
    /// the game in this frame, or `None` for the fog of the game. The renderer asks after
    /// `before_world`.
    fn fog_range(&self) -> Option<(f32, f32)> {
        None
    }
}

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

// ---- the capture file

mod tag {
    pub const TEXTURE: u32 = 1;
    pub const BUFFER_DATA: u32 = 2;
    pub const SCENE_BEGIN: u32 = 3;
    pub const SCENE_END: u32 = 4;
    pub const FRAME_EVENT: u32 = 5;
    pub const OBJECT: u32 = 6;
    pub const DRAW_STATE: u32 = 7;
    pub const COMMAND: u32 = 8;
    pub const CALL: u32 = 9;
    pub const FRAME_END: u32 = 10;
    pub const HEADER: u32 = 11;
    pub const TEXTURE_UPDATE: u32 = 12;
    pub const READ_PIXELS: u32 = 13;
    pub const SHOT: u32 = 14;
    pub const DRAW_UP: u32 = 15;
    pub const EXTENSION: u32 = 16;
}

const COMMAND_RENDER_STATE: u32 = 1;
const COMMAND_STAGE_STATE: u32 = 2;
const COMMAND_TRANSFORM: u32 = 3;
const COMMAND_DRAW_INDEXED: u32 = 4;
const COMMAND_DRAW: u32 = 5;
const CALL_CLEAR: u32 = 4;
const CALL_VIEWPORT: u32 = 5;
const CALL_RENDER_TARGET: u32 = 7;
const CALL_TEXTURE_LEVEL: u32 = 15;
const CALL_CREATE_TEXTURE: u32 = 20;
const CALL_COPY_PIXELS: u32 = 23;
const EVENT_SET_TARGET: u32 = 2;
/// The value of a state that the game did not set.
const UNKNOWN: u32 = 0x7FFF_FFFF;

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn float(bytes: &[u8], offset: usize) -> f32 {
    f32::from_bits(word(bytes, offset))
}

fn floats<const N: usize>(bytes: &[u8], offset: usize) -> [f32; N] {
    core::array::from_fn(|i| float(bytes, offset + i * 4))
}

fn double(bytes: &[u8], offset: usize) -> f64 {
    f64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

/// The offsets of `MgeLinkDrawStateV1`. The struct has 1728 bytes.
mod state {
    /// The place of a render state of Direct3D in the packet, when the packet has it.
    pub fn of_render_state(render_state: u32) -> Option<usize> {
        Some(match render_state {
            7 => DEPTH_ENABLE,
            14 => DEPTH_WRITE,
            15 => ALPHA_TEST_ENABLE,
            19 => SOURCE_BLEND,
            20 => DESTINATION_BLEND,
            22 => CULL_MODE,
            23 => DEPTH_FUNCTION,
            24 => ALPHA_REFERENCE,
            25 => ALPHA_FUNCTION,
            27 => BLEND_ENABLE,
            28 => FOG_ENABLE,
            // The stencil states of Direct3D are 52 to 59, in the order of the packet.
            52..=59 => STENCIL + (render_state as usize - 52) * 4,
            137 => LIGHTING,
            139 => AMBIENT,
            145 => DIFFUSE_SOURCE,
            148 => EMISSIVE_SOURCE,
            151 => VERTEX_BLEND,
            _ => return None,
        })
    }

    /// The place of a texture stage state of Direct3D in the packet.
    pub fn of_stage_state(stage: u32, stage_state: u32) -> Option<usize> {
        const ORDER: [u32; 17] = [1, 2, 3, 4, 5, 6, 26, 27, 28, 11, 24, 7, 8, 9, 10, 22, 23];
        let index = ORDER.iter().position(|s| *s == stage_state)?;
        (stage < 8).then(|| STAGES + stage as usize * STAGE_SIZE + index * 4)
    }

    pub const SIZE: usize = 1728;
    pub const BASE_VERTEX: usize = 12;
    pub const VERTEX_BUFFER: usize = 32;
    pub const VERTEX_STRIDE: usize = 36;
    pub const VERTEX_FORMAT: usize = 40;
    pub const INDEX_BUFFER: usize = 44;
    pub const WORLD: usize = 48;
    pub const DEPTH_WRITE: usize = 112;
    pub const CULL_MODE: usize = 116;
    pub const VERTEX_BLEND: usize = 120;
    pub const BLEND_ENABLE: usize = 124;
    pub const SOURCE_BLEND: usize = 128;
    pub const DESTINATION_BLEND: usize = 132;
    pub const ALPHA_TEST_ENABLE: usize = 136;
    pub const ALPHA_FUNCTION: usize = 140;
    pub const ALPHA_REFERENCE: usize = 144;
    pub const LIGHTING: usize = 148;
    pub const FOG_ENABLE: usize = 152;
    pub const DIFFUSE_SOURCE: usize = 156;
    pub const EMISSIVE_SOURCE: usize = 160;
    pub const AMBIENT: usize = 164;
    pub const MATERIAL: usize = 168;
    pub const LIGHT_COUNT: usize = 236;
    pub const STAGES: usize = 244;
    pub const STAGE_SIZE: usize = 17 * 4;
    pub const BLEND_WORLDS: usize = 788;
    pub const TEXTURES: usize = 980;
    pub const TEXTURE_TRANSFORMS: usize = 1012;
    pub const DEPTH_ENABLE: usize = 1524;
    pub const DEPTH_FUNCTION: usize = 1528;
    /// Enable, fail, depth fail, pass, function, reference, mask, write mask.
    pub const STENCIL: usize = 1532;
    pub const SAMPLERS: usize = 1564;
    pub const WORLD_RELATIVE_MASK: usize = 1724;
    /// A light after the struct: the device index, then a D3DLIGHT8 of 26 words.
    pub const LIGHT_SIZE: usize = 27 * 4;
}

// ---- the data of a draw for the shader

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LightData {
    diffuse: [f32; 4],
    ambient: [f32; 4],
    position: [f32; 4],
    direction: [f32; 4],
    attenuation: [f32; 4],
    cone: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct StageData {
    color: [u32; 4],
    alpha: [u32; 4],
    coordinate: [u32; 4],
    bump: [f32; 4],
    luminance: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DrawData {
    world: [[f32; 16]; 4],
    view: [f32; 16],
    projection: [f32; 16],
    material_diffuse: [f32; 4],
    material_ambient: [f32; 4],
    material_emissive: [f32; 4],
    scene_ambient: [f32; 4],
    fog_color: [f32; 4],
    fog: [f32; 4],
    texture_factor: [f32; 4],
    flags: [u32; 4],
    more: [u32; 4],
    counts: [u32; 4],
    lights: [LightData; 8],
    stages: [StageData; 6],
    texture_transforms: [[f32; 16]; 6],
    pixel: [f32; 4],
    extension: [[f32; 4]; 8],
    clip_plane: [f32; 4],
}

/// A D3DCOLOR as red, green, blue, alpha.
fn color(argb: u32) -> [f32; 4] {
    [(argb >> 16 & 255) as f32 / 255.0, (argb >> 8 & 255) as f32 / 255.0, (argb & 255) as f32 / 255.0, (argb >> 24) as f32 / 255.0]
}

fn known(value: u32, default: u32) -> u32 {
    if value == UNKNOWN {
        default
    } else {
        value
    }
}

// ---- Direct3D 8 values to wgpu

fn blend_factor(value: u32) -> wgpu::BlendFactor {
    use wgpu::BlendFactor::*;
    match value {
        1 => Zero,
        3 => Src,
        4 => OneMinusSrc,
        5 => SrcAlpha,
        6 => OneMinusSrcAlpha,
        7 => DstAlpha,
        8 => OneMinusDstAlpha,
        9 => Dst,
        10 => OneMinusDst,
        11 => SrcAlphaSaturated,
        _ => One,
    }
}

fn compare(value: u32) -> wgpu::CompareFunction {
    use wgpu::CompareFunction::*;
    match value {
        1 => Never,
        2 => Less,
        3 => Equal,
        4 => LessEqual,
        5 => Greater,
        6 => NotEqual,
        7 => GreaterEqual,
        _ => Always,
    }
}

/// The format of the GPU for a D3DFORMAT, the bytes of one of its blocks (0 for a format
/// without blocks), and the bytes of a pixel in the stream. `None` for a format that this
/// program does not make.
fn texture_format(d3d_format: u32) -> Option<(wgpu::TextureFormat, usize, usize)> {
    Some(match d3d_format {
        0x3154_5844 => (wgpu::TextureFormat::Bc1RgbaUnorm, 8, 0),
        0x3354_5844 => (wgpu::TextureFormat::Bc2RgbaUnorm, 16, 0),
        0x3554_5844 => (wgpu::TextureFormat::Bc3RgbaUnorm, 16, 0),
        21 | 22 => (wgpu::TextureFormat::Bgra8Unorm, 0, 4),
        // D3DFMT_R5G6B5, X1R5G5B5, A1R5G5B5 and A4R4G4B4: this program makes 4 bytes of them.
        23..=26 => (wgpu::TextureFormat::Bgra8Unorm, 0, 2),
        // D3DFMT_V8U8, a bump map: two numbers with a sign, U in the first byte.
        60 => (wgpu::TextureFormat::Rg8Snorm, 0, 2),
        _ => return None,
    })
}

/// True for a format with 16 bits that this program makes 4 bytes of.
fn expands(d3d_format: u32) -> bool {
    (23..=26).contains(&d3d_format)
}

/// The bytes of a pixel on the GPU, for a format without blocks.
fn gpu_bytes(d3d_format: u32) -> usize {
    if d3d_format == 60 {
        2
    } else {
        4
    }
}

/// Makes pixels with the bytes blue, green, red, alpha from a level of a DXT1, DXT3 or DXT5
/// texture. The GPU takes a block format only for a size that is a multiple of 4; for each
/// other size this program makes the pixels itself.
fn decode_dxt(d3d_format: u32, width: usize, height: usize, source: &[u8]) -> Vec<u8> {
    let block = if d3d_format == 0x3154_5844 { 8 } else { 16 };
    let mut pixels = vec![0u8; width * height * 4];
    let across = width.div_ceil(4).max(1);
    for (index, data) in source.chunks_exact(block).enumerate() {
        let (block_x, block_y) = (index % across * 4, index / across * 4);
        if block_y >= height {
            break;
        }
        let colors = &data[block - 8..];
        let (c0, c1) = (u16::from_le_bytes([colors[0], colors[1]]), u16::from_le_bytes([colors[2], colors[3]]));
        let wide = |c: u16| [((c & 31) * 255 / 31) as u32, ((c >> 5 & 63) * 255 / 63) as u32, ((c >> 11) * 255 / 31) as u32];
        let (a, b) = (wide(c0), wide(c1));
        // Blue, green, red, alpha for each of the 4 colours of the block.
        let mut palette = [[0u8; 4]; 4];
        for channel in 0..3 {
            palette[0][channel] = a[channel] as u8;
            palette[1][channel] = b[channel] as u8;
            if c0 > c1 || block == 16 {
                palette[2][channel] = ((2 * a[channel] + b[channel]) / 3) as u8;
                palette[3][channel] = ((a[channel] + 2 * b[channel]) / 3) as u8;
            } else {
                palette[2][channel] = ((a[channel] + b[channel]) / 2) as u8;
            }
        }
        let transparent = block == 8 && c0 <= c1;
        for entry in palette.iter_mut() {
            entry[3] = 255;
        }
        if transparent {
            palette[3] = [0, 0, 0, 0];
        }
        // The alpha of DXT5: two values and 3 bits for each pixel.
        let mut alphas = [255u8; 8];
        let mut alpha_bits = 0u64;
        if d3d_format == 0x3554_5844 || d3d_format == 0x3454_5844 {
            let (a0, a1) = (data[0] as u32, data[1] as u32);
            alphas[0] = a0 as u8;
            alphas[1] = a1 as u8;
            for i in 2..8u32 {
                alphas[i as usize] = if a0 > a1 {
                    (((8 - i) * a0 + (i - 1) * a1) / 7) as u8
                } else if i < 6 {
                    (((6 - i) * a0 + (i - 1) * a1) / 5) as u8
                } else if i == 6 {
                    0
                } else {
                    255
                };
            }
            for (i, byte) in data[2..8].iter().enumerate() {
                alpha_bits |= (*byte as u64) << (8 * i);
            }
        }
        let bits = u32::from_le_bytes([colors[4], colors[5], colors[6], colors[7]]);
        for n in 0..16 {
            let (x, y) = (block_x + n % 4, block_y + n / 4);
            if x >= width || y >= height {
                continue;
            }
            let mut pixel = palette[(bits >> (2 * n) & 3) as usize];
            match d3d_format {
                // DXT3: 4 bits of alpha for each pixel.
                0x3354_5844 | 0x3254_5844 => pixel[3] = (data[n / 2] >> (4 * (n % 2)) & 15) * 17,
                0x3554_5844 | 0x3454_5844 => pixel[3] = alphas[(alpha_bits >> (3 * n) & 7) as usize],
                _ => {}
            }
            pixels[(y * width + x) * 4..][..4].copy_from_slice(&pixel);
        }
    }
    pixels
}

/// Makes pixels with the bytes blue, green, red, alpha from pixels with 16 bits.
fn expand_16_bits(d3d_format: u32, source: &[u8]) -> Vec<u8> {
    let wide = |value: u16, bits: u32| -> u8 { ((value as u32 * 255 + ((1 << bits) - 1) / 2) / ((1 << bits) - 1)) as u8 };
    let mut pixels = Vec::with_capacity(source.len() * 2);
    for pair in source.chunks_exact(2) {
        let p = u16::from_le_bytes([pair[0], pair[1]]);
        let (red, green, blue, alpha) = match d3d_format {
            23 => (wide(p >> 11, 5), wide(p >> 5 & 63, 6), wide(p & 31, 5), 255),
            24 => (wide(p >> 10 & 31, 5), wide(p >> 5 & 31, 5), wide(p & 31, 5), 255),
            25 => (wide(p >> 10 & 31, 5), wide(p >> 5 & 31, 5), wide(p & 31, 5), if p >> 15 != 0 { 255 } else { 0 }),
            _ => (wide(p >> 8 & 15, 4), wide(p >> 4 & 15, 4), wide(p & 15, 4), wide(p >> 12, 4)),
        };
        pixels.extend_from_slice(&[blue, green, red, alpha]);
    }
    pixels
}

/// The shader for a draw in screen coordinates: the colour of the vertex times the texture.
const SCREEN_SHADER: &str = "
struct Out { @builtin(position) position: vec4f, @location(0) color: vec4f, @location(1) uv: vec2f }
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var picture_sampler: sampler;
@vertex fn vs(@location(0) position: vec4f, @location(1) color: vec4f, @location(2) uv: vec2f) -> Out {
    return Out(position, color, uv);
}
@fragment fn fs(in: Out) -> @location(0) vec4f {
    return in.color * textureSample(picture, picture_sampler, in.uv);
}
";

/// The parts of the work that the program measures.
const TIME_NAMES: [&str; 8] = [
    "read of the stream",
    "records that are not draws",
    "draw: the data of the draw",
    "draw: the upload of its data",
    "draw: textures, samplers, pipeline",
    "draw: the commands of the pass",
    "end of the frame and submit",
    "copy into the window and present",
];
const TIME_READ: usize = 0;
const TIME_RECORDS: usize = 1;
const TIME_DRAW_DATA: usize = 2;
const TIME_DRAW_UPLOAD: usize = 3;
const TIME_DRAW_GROUPS: usize = 4;
const TIME_DRAW_PASS: usize = 5;
const TIME_END: usize = 6;
const TIME_PRESENT: usize = 7;

/// The number of indexes in the index buffer for triangle fans.
const FAN_INDEXES: u32 = 3 * 1024;

/// D3DSTENCILOP_*.
fn stencil_operation(value: u32) -> wgpu::StencilOperation {
    use wgpu::StencilOperation::*;
    match value {
        2 => Zero,
        3 => Replace,
        4 => IncrementClamp,
        5 => DecrementClamp,
        6 => Invert,
        7 => IncrementWrap,
        8 => DecrementWrap,
        _ => Keep,
    }
}

fn address(value: u32) -> wgpu::AddressMode {
    match value {
        2 => wgpu::AddressMode::MirrorRepeat,
        3 | 4 => wgpu::AddressMode::ClampToEdge,
        _ => wgpu::AddressMode::Repeat,
    }
}

fn filter(value: u32) -> wgpu::FilterMode {
    if value >= 2 && value != UNKNOWN {
        wgpu::FilterMode::Linear
    } else {
        wgpu::FilterMode::Nearest
    }
}

/// One vertex attribute of a flexible vertex format: the shader location, the offset in the
/// vertex, and the format.
struct Attribute {
    location: u32,
    offset: u64,
    format: wgpu::VertexFormat,
}

/// The attributes that a flexible vertex format of Direct3D 8 has. `None` for a format that
/// this program does not draw.
fn attributes(fvf: u32, stride: u32) -> Option<Vec<Attribute>> {
    use wgpu::VertexFormat::*;
    let position = fvf & 0xE;
    if position == 0x4 || position == 0 {
        // Vertices that are transformed already.
        return None;
    }
    let mut list = vec![Attribute { location: 0, offset: 0, format: Float32x3 }];
    let mut offset = 12u64;
    // D3DFVF_XYZ is 2. D3DFVF_XYZB1 to XYZB5 are 6 to 14: one float more for each step.
    let betas = (position as u64).saturating_sub(4) / 2;
    if betas != 0 {
        // The last beta is the matrix indexes when the format says so.
        let weights = if fvf & 0x1000 != 0 { betas - 1 } else { betas };
        // The shader input has three floats. With fewer weights, the floats after them are
        // the next part of the vertex; the shader does not read those.
        if weights != 0 && offset + 12 <= stride as u64 {
            list.push(Attribute { location: 7, offset, format: Float32x3 });
        }
        offset += betas * 4;
    }
    if fvf & 0x10 != 0 {
        list.push(Attribute { location: 1, offset, format: Float32x3 });
        offset += 12;
    }
    if fvf & 0x40 != 0 {
        list.push(Attribute { location: 2, offset, format: Unorm8x4 });
        offset += 4;
    }
    if fvf & 0x80 != 0 {
        offset += 4;
    }
    let sets = (fvf >> 8 & 0xF).min(4);
    for set in 0..sets {
        list.push(Attribute { location: 3 + set, offset, format: Float32x2 });
        offset += 8;
    }
    Some(list)
}

/// The formats of the shader inputs, by location, for the inputs that a vertex does not have.
const INPUT_FORMATS: [wgpu::VertexFormat; 8] = {
    use wgpu::VertexFormat::*;
    [Float32x3, Float32x3, Unorm8x4, Float32x2, Float32x2, Float32x2, Float32x2, Float32x3]
};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PipelineKey {
    fvf: u32,
    stride: u32,
    topology: u32,
    blend: [u32; 3],
    depth: [u32; 3],
    cull: u32,
    /// D3DRS_FILLMODE: 2 is wireframe.
    fill: u32,
    /// The stencil: enable, fail, depth fail, pass, function, mask, write mask.
    stencil: [u32; 7],
    /// D3DRS_COLORWRITEENABLE: red 1, green 2, blue 4, alpha 8.
    color_write: u32,
}

/// A texture that the game draws into.
struct RenderTexture {
    texture: wgpu::Texture,
    view: Rc<wgpu::TextureView>,
    depth: Rc<wgpu::TextureView>,
    size: (u32, u32),
}

struct BufferData {
    bytes: Vec<u8>,
    gpu: Option<Rc<wgpu::Buffer>>,
    index: bool,
}

#[derive(Default)]
struct Counts {
    draws: u32,
    other_target: u32,
    no_layout: u32,
    no_buffer: u32,
    topology: u32,
    textures: u32,
    texture_updates: u32,
    render_textures: u32,
    pixel_reads: u32,
    /// Where the time of the program goes, in the order of `TIME_NAMES`.
    times: [std::time::Duration; 8],
    screen_draws: u32,
    screen_draws_lost: u32,
    target_copies: u32,
    target_draws: u32,
    /// The kept draws that were drawn again for an extension.
    replayed: u32,
    texture_updates_lost: u32,
    lost_reasons: HashMap<String, u32>,
    textures_decoded: u32,
    texture_formats: HashMap<u32, u32>,
    missing_textures: u32,
    more_stages: u32,
    more_lights: u32,
    coordinate_work: u32,
}

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;
const STAGES: usize = 6;
/// The size of a slot for the data of a draw: the size of `DrawData`, and a multiple of the
/// 256 bytes that a uniform offset must be a multiple of.
const SLOT_SIZE: usize = core::mem::size_of::<DrawData>().next_multiple_of(256);
const SLOTS_IN_BUFFER: usize = 2048;

struct Renderer {
    device: Rc<wgpu::Device>,
    /// The format of the picture that the pipelines draw into.
    target_format: wgpu::TextureFormat,
    /// What the picture of the window can be used for.
    surface_usages: wgpu::TextureUsages,
    /// The anisotropic filter that MGE XE forces, for a comparison with its picture. 1 for the
    /// filters that the game asks for.
    anisotropy: u16,
    /// The device can draw the edges of triangles only.
    wireframe: bool,
    queue: wgpu::Queue,
    shader: wgpu::ShaderModule,
    draw_layout: wgpu::BindGroupLayout,
    /// The buffers for the data of the draws of a frame, each with its group, and the number
    /// of draws of the frame so far. A draw has one slot in a buffer.
    draw_buffers: Vec<(Rc<wgpu::Buffer>, Rc<wgpu::BindGroup>)>,
    draw_slots: usize,
    texture_layout: wgpu::BindGroupLayout,
    sampler_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    pipelines: HashMap<PipelineKey, Rc<wgpu::RenderPipeline>>,
    textures: HashMap<u32, Rc<wgpu::TextureView>>,
    /// The textures that the game draws into, by the handle of the texture, and the texture of
    /// each surface that the game got from a texture.
    targets: HashMap<u32, RenderTexture>,
    surfaces: HashMap<u32, u32>,
    /// The textures with a block format that this program made the pixels of: their size is
    /// not a multiple of 4.
    decoded: std::collections::HashSet<u32>,
    /// The textures themselves, for a write of a level later.
    texture_objects: HashMap<u32, (wgpu::Texture, u32)>,
    white: Rc<wgpu::TextureView>,
    samplers: HashMap<[u32; 5], Rc<wgpu::Sampler>>,
    /// The textures and the samplers of a draw, by the texture handles and the sampler values
    /// of its stages. The D3D12 backend has a small number of places for samplers.
    texture_groups: HashMap<[u32; STAGES], Rc<wgpu::BindGroup>>,
    /// The samplers of a draw, by the sampler values of its stages. They are apart from the
    /// textures: the D3D12 backend has places for 2048 samplers only, and the sets of sampler
    /// values are few.
    sampler_groups: HashMap<[[u32; 5]; STAGES], Rc<wgpu::BindGroup>>,
    buffers: HashMap<u32, BufferData>,
    defaults: wgpu::Buffer,
    /// The indexes of a triangle fan as a triangle list.
    fan_indexes: wgpu::Buffer,
    /// For a draw in screen coordinates: the pipelines without and with blend, the layout of
    /// its group, and its sampler.
    screen_pipelines: [Option<wgpu::RenderPipeline>; 2],
    screen_layout: Option<wgpu::BindGroupLayout>,
    screen_sampler: Option<wgpu::Sampler>,
    counts: Counts,
}

impl Renderer {
    /// Makes the device. With a window, also the surface of the window, and the format of
    /// the target is a format of that surface.
    fn new(
        backends: wgpu::Backends,
        window: Option<std::sync::Arc<winit::window::Window>>,
        mut target_format: wgpu::TextureFormat,
        sky_shader: Option<String>,
    ) -> (Renderer, Option<wgpu::Surface<'static>>) {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = backends;
        // The new shader compiler of Direct3D, a part of this program: the old one is slow.
        descriptor.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::StaticDxc;
        let instance = wgpu::Instance::new(descriptor);
        let surface = window.map(|window| instance.create_surface(window).expect("a surface for the window"));
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: surface.as_ref(),
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .expect("no graphics adapter for this backend");
        let info = adapter.get_info();
        println!("adapter: {} ({:?})", info.name, info.backend);
        let mut surface_usages = wgpu::TextureUsages::empty();
        if let Some(surface) = surface.as_ref() {
            // The game gives colours that are ready for the screen: no sRGB format.
            let capabilities = surface.get_capabilities(&adapter);
            if !capabilities.formats.contains(&target_format) {
                target_format = capabilities.formats.iter().copied().find(|format| !format.is_srgb()).unwrap_or(capabilities.formats[0]);
            }
            surface_usages = capabilities.usages;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::TEXTURE_COMPRESSION_BC | (adapter.features() & wgpu::Features::POLYGON_MODE_LINE),
                // The largest texture that the adapter takes, up to 16384: an atlas of the
                // distant land of MGE XE has that size.
                required_limits: wgpu::Limits { max_texture_dimension_2d: adapter.limits().max_texture_dimension_2d.min(16384), ..wgpu::Limits::default() },
                memory_hints: wgpu::MemoryHints::default(),
                ..Default::default()
            },
        ))
        .expect("no device");

        let wireframe = device.features().contains(wgpu::Features::POLYGON_MODE_LINE);
        // The sky functions of an extension, or the two that change nothing.
        let sky_shader = sky_shader.unwrap_or_else(|| {
            "fn extension_sky_position(position: vec3f) -> vec3f { return position; }\nfn extension_sky_color(direction: vec3f, pixel: vec2f) -> vec3f { return vec3f(0.0); }\n".to_string()
        });
        let source = format!("{}\n{sky_shader}", include_str!("shader.wgsl"));
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("shader.wgsl"), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: None },
                count: None,
            }],
        });
        let mut entries = Vec::new();
        let mut sampler_entries = Vec::new();
        for i in 0..STAGES as u32 {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: i,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            sampler_entries.push(wgpu::BindGroupLayoutEntry {
                binding: i,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &entries });
        let sampler_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &sampler_entries });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&draw_layout), Some(&texture_layout), Some(&sampler_layout)],
            immediate_size: 0,
        });
        let white = Rc::new(device
            .create_texture_with_data(
                &queue,
                &wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::MipMajor,
                &[255, 255, 255, 255],
            )
            .create_view(&Default::default()));
        let fan: Vec<u16> = (0..FAN_INDEXES as u16 / 3).flat_map(|n| [0, n + 1, n + 2]).collect();
        let fan_indexes = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&fan),
            usage: wgpu::BufferUsages::INDEX,
        });
        let defaults = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &[0u8; 64],
            usage: wgpu::BufferUsages::VERTEX,
        });
        let renderer = Renderer {
            device: Rc::new(device),
            target_format,
            surface_usages,
            anisotropy: 1,
            wireframe,
            queue,
            shader,
            draw_layout,
            draw_buffers: Vec::new(),
            draw_slots: 0,
            texture_layout,
            sampler_layout,
            pipeline_layout,
            pipelines: HashMap::new(),
            textures: HashMap::new(),
            texture_objects: HashMap::new(),
            decoded: Default::default(),
            targets: HashMap::new(),
            surfaces: HashMap::new(),
            white,
            samplers: HashMap::new(),
            texture_groups: HashMap::new(),
            sampler_groups: HashMap::new(),
            buffers: HashMap::new(),
            defaults,
            fan_indexes,
            screen_pipelines: [None, None],
            screen_layout: None,
            screen_sampler: None,
            counts: Counts::default(),
        };
        (renderer, surface)
    }

    /// Puts the data of a draw into the next slot. Returns the group to bind and the offset
    /// of the slot.
    fn draw_slot(&mut self, data: &DrawData) -> (Rc<wgpu::BindGroup>, u32) {
        let (index, slot) = (self.draw_slots / SLOTS_IN_BUFFER, self.draw_slots % SLOTS_IN_BUFFER);
        self.draw_slots += 1;
        if index == self.draw_buffers.len() {
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: (SLOTS_IN_BUFFER * SLOT_SIZE) as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.draw_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(core::mem::size_of::<DrawData>() as u64),
                    }),
                }],
            });
            self.draw_buffers.push((Rc::new(buffer), Rc::new(group)));
        }
        let (buffer, group) = &self.draw_buffers[index];
        self.queue.write_buffer(buffer, (slot * SLOT_SIZE) as u64, bytemuck::bytes_of(data));
        (group.clone(), (slot * SLOT_SIZE) as u32)
    }

    /// Makes a texture from a texture record.
    fn texture(&mut self, data: &[u8]) {
        let handle = word(data, 0);
        let (width, height, levels, d3d_format) = (word(data, 8), word(data, 12), word(data, 16), word(data, 20));
        let levels = levels as usize;
        let offsets = 36;
        let widths = offsets + (levels + 1) * 4;
        let heights = widths + levels * 4;
        let pixels = &data[heights + levels * 4..];
        // The size of a block (or 0), and the bytes of a pixel in the stream.
        let Some((format, block, source_bytes)) = texture_format(d3d_format) else {
            *self.counts.texture_formats.entry(d3d_format).or_default() += 1;
            return;
        };
        // The GPU takes a block format only for a size that is a multiple of 4.
        let decode = block != 0 && (width % 4 != 0 || height % 4 != 0);
        let format = if decode { wgpu::TextureFormat::Bgra8Unorm } else { format };
        let mut bytes = Vec::new();
        for level in 0..levels {
            let (w, h) = (word(data, widths + level * 4) as usize, word(data, heights + level * 4) as usize);
            let size = if block != 0 { w.div_ceil(4).max(1) * h.div_ceil(4).max(1) * block } else { w * h * source_bytes };
            let from = word(data, offsets + level * 4) as usize;
            let to = (word(data, offsets + (level + 1) * 4) as usize).min(pixels.len());
            let start = bytes.len();
            if decode {
                let mut source = pixels[from.min(to)..to.min(from + size)].to_vec();
                source.resize(size, 0);
                bytes.extend_from_slice(&decode_dxt(d3d_format, w.max(1), h.max(1), &source));
                self.counts.textures_decoded += (level == 0) as u32;
            } else if expands(d3d_format) {
                let mut source = pixels[from.min(to)..to.min(from + size)].to_vec();
                source.resize(size, 0);
                bytes.extend_from_slice(&expand_16_bits(d3d_format, &source));
            } else {
                bytes.extend_from_slice(&pixels[from.min(to)..to.min(from + size)]);
                bytes.resize(start + size, 0);
            }
            if d3d_format == 22 {
                // No alpha in this format.
                for pixel in bytes[start..].chunks_exact_mut(4) {
                    pixel[3] = 255;
                }
            }
        }
        let texture = self.device.create_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: levels as u32,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::MipMajor,
            &bytes,
        );
        self.textures.insert(handle, Rc::new(texture.create_view(&Default::default())));
        self.texture_objects.insert(handle, (texture, d3d_format));
        // A group can have the texture that had this handle before.
        self.texture_groups.clear();
        self.counts.textures += 1;
    }

    /// The pipeline for a draw in screen coordinates, with the usual blend of alpha or
    /// without blend.
    fn screen_pipeline(&mut self, blend: bool) -> wgpu::RenderPipeline {
        if self.screen_layout.is_none() {
            self.screen_layout = Some(self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            }));
            self.screen_sampler = Some(self.device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }));
        }
        if let Some(pipeline) = &self.screen_pipelines[blend as usize] {
            return pipeline.clone();
        }
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl(SCREEN_SHADER.into()) });
        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[self.screen_layout.as_ref()],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x2];
        let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: 40, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attributes })],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.target_format,
                    blend: blend.then_some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.screen_pipelines[blend as usize] = Some(pipeline.clone());
        pipeline
    }

    /// Makes a texture without pixels: the game made it with `CreateTexture` and writes each
    /// level later. `levels` 0 is all the levels down to 1 x 1.
    fn empty_texture(&mut self, handle: u32, width: u32, height: u32, levels: u32, d3d_format: u32) {
        let Some((format, block, _)) = texture_format(d3d_format) else {
            *self.counts.texture_formats.entry(d3d_format).or_default() += 1;
            return;
        };
        let (width, height) = (width.max(1), height.max(1));
        // The GPU takes a block format only for a size that is a multiple of 4. For each other
        // size this program makes the pixels of each level itself.
        let decode = block != 0 && (width % 4 != 0 || height % 4 != 0);
        let format = if decode { wgpu::TextureFormat::Bgra8Unorm } else { format };
        if decode {
            self.decoded.insert(handle);
            self.counts.textures_decoded += 1;
        } else {
            self.decoded.remove(&handle);
        }
        let all = 32 - width.max(height).leading_zeros();
        let levels = if levels == 0 { all } else { levels.min(all) };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.textures.insert(handle, Rc::new(texture.create_view(&Default::default())));
        self.texture_objects.insert(handle, (texture, d3d_format));
        self.texture_groups.clear();
        self.counts.textures += 1;
    }

    /// Makes a texture that the game draws into, with a depth buffer of its size.
    fn render_texture(&mut self, handle: u32, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        let texture = target_texture(&self.device, width, height, self.target_format, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC);
        let depth = target_texture(&self.device, width, height, DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let view = Rc::new(texture.create_view(&Default::default()));
        self.textures.insert(handle, view.clone());
        self.texture_objects.remove(&handle);
        self.targets.insert(handle, RenderTexture { texture, view, depth: Rc::new(depth.create_view(&Default::default())), size: (width, height) });
        self.texture_groups.clear();
        self.counts.render_textures += 1;
    }

    /// The game copied pixels of its own into a texture that it draws into. `pixels` has 4
    /// bytes for each pixel, blue first.
    fn target_pixels(&mut self, handle: u32, width: u32, height: u32, pixels: &[u8]) {
        let Some(target) = self.targets.get(&handle) else { return };
        if target.size != (width, height) || pixels.len() < (width * height * 4) as usize {
            self.counts.texture_updates_lost += 1;
            return;
        }
        let mut bytes = pixels[..(width * height * 4) as usize].to_vec();
        if !matches!(self.target_format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb) {
            for pixel in bytes.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &target.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: Some(height) },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.counts.target_copies += 1;
    }

    /// The game wrote a level of a texture again.
    fn texture_update(&mut self, data: &[u8]) {
        let (handle, level, width, height, size) = (word(data, 0), word(data, 4), word(data, 8), word(data, 12), word(data, 20) as usize);
        let Some((texture, d3d_format)) = self.texture_objects.get(&handle) else {
            self.counts.texture_updates_lost += 1;
            *self.counts.lost_reasons.entry(format!("the texture is not one that this program made, format {:#x}", word(data, 16))).or_default() += 1;
            return;
        };
        let decode = self.decoded.contains(&handle);
        let block = match d3d_format {
            0x3154_5844 => 8,
            0x3354_5844 | 0x3554_5844 => 16,
            _ => 0,
        };
        let source_bytes = texture_format(*d3d_format).map_or(4, |f| f.2 as u32);
        let (row, rows, expected) = if block != 0 {
            (width.div_ceil(4) * block, height.div_ceil(4), (width.div_ceil(4) * height.div_ceil(4) * block) as usize)
        } else {
            (width * gpu_bytes(*d3d_format) as u32, height, (width * height * source_bytes) as usize)
        };
        if level >= texture.mip_level_count() || size < expected || data.len() < 24 + expected {
            self.counts.texture_updates_lost += 1;
            let reason = if level >= texture.mip_level_count() { "the texture has no such level" } else { "the record has too few bytes" };
            *self.counts.lost_reasons.entry(format!("{reason}, format {d3d_format:#x}, level {level} of {}, {width} x {height}", texture.mip_level_count())).or_default() += 1;
            return;
        }
        let mut pixels = std::borrow::Cow::Borrowed(&data[24..24 + expected]);
        let (mut row, mut block) = (row, block);
        if decode {
            pixels = std::borrow::Cow::Owned(decode_dxt(*d3d_format, width as usize, height as usize, &pixels));
            (row, block) = (width * 4, 0);
        }
        let rows = if decode { height } else { rows };
        if expands(*d3d_format) {
            pixels = std::borrow::Cow::Owned(expand_16_bits(*d3d_format, &pixels));
        } else if *d3d_format == 22 {
            // No alpha in this format.
            for pixel in pixels.to_mut().chunks_exact_mut(4) {
                pixel[3] = 255;
            }
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &pixels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(rows) },
            wgpu::Extent3d { width: if block != 0 { width.next_multiple_of(4) } else { width }, height: if block != 0 { height.next_multiple_of(4) } else { height }, depth_or_array_layers: 1 },
        );
        self.counts.texture_updates += 1;
    }

    /// Keeps the bytes of a buffer data record.
    fn buffer_data(&mut self, data: &[u8]) {
        let (handle, offset, size) = (word(data, 0), word(data, 4) as usize, word(data, 8) as usize);
        let buffer = self.buffers.entry(handle).or_insert(BufferData { bytes: Vec::new(), gpu: None, index: false });
        if buffer.bytes.len() < offset + size {
            buffer.bytes.resize(offset + size, 0);
        }
        buffer.bytes[offset..offset + size].copy_from_slice(&data[12..12 + size]);
        buffer.gpu = None;
    }

    /// The buffer on the GPU with the bytes that the game wrote last.
    fn gpu_buffer(&mut self, handle: u32, index: bool) -> Option<Rc<wgpu::Buffer>> {
        let buffer = self.buffers.get_mut(&handle)?;
        if buffer.gpu.is_none() || buffer.index != index {
            buffer.index = index;
            buffer.gpu = Some(Rc::new(self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: &buffer.bytes,
                usage: if index { wgpu::BufferUsages::INDEX } else { wgpu::BufferUsages::VERTEX },
            })));
        }
        buffer.gpu.clone()
    }

    fn sampler(&mut self, values: [u32; 5]) -> Rc<wgpu::Sampler> {
        let device = &self.device;
        let anisotropy = self.anisotropy;
        // D3DTEXF_NONE is 0: a texture without levels in use keeps its filter.
        let forced = anisotropy > 1
            && filter(values[2]) == wgpu::FilterMode::Linear
            && filter(values[3]) == wgpu::FilterMode::Linear
            && known(values[4], 0) != 0;
        let sampler = self.samplers.entry(values).or_insert_with(|| {
            Rc::new(device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: address(values[0]),
                address_mode_v: address(values[1]),
                // MGE XE gives each texture that the game filters its anisotropic filter, with
                // a linear filter between the levels. wgpu wants all filters linear for that.
                anisotropy_clamp: if forced { anisotropy } else { 1 },
                // D3DTEXF_NONE for the filter between levels: Direct3D reads the first level only.
                lod_max_clamp: if known(values[4], 0) == 0 { 0.0 } else { 32.0 },
                mag_filter: filter(values[2]),
                min_filter: filter(values[3]),
                mipmap_filter: if filter(values[4]) == wgpu::FilterMode::Linear || forced {
                    wgpu::MipmapFilterMode::Linear
                } else {
                    wgpu::MipmapFilterMode::Nearest
                },
                ..Default::default()
            }))
        });
        sampler.clone()
    }

    fn pipeline(&mut self, key: PipelineKey) -> Option<Rc<wgpu::RenderPipeline>> {
        if !self.pipelines.contains_key(&key) {
            let present = attributes(key.fvf, key.stride)?;
            let topology = match key.topology {
                1 => wgpu::PrimitiveTopology::PointList,
                2 => wgpu::PrimitiveTopology::LineList,
                3 => wgpu::PrimitiveTopology::LineStrip,
                4 => wgpu::PrimitiveTopology::TriangleList,
                5 => wgpu::PrimitiveTopology::TriangleStrip,
                _ => return None,
            };
            let from_vertex: Vec<wgpu::VertexAttribute> =
                present.iter().map(|a| wgpu::VertexAttribute { format: a.format, offset: a.offset, shader_location: a.location }).collect();
            // An input that the vertex does not have comes from a buffer of zeros.
            let from_defaults: Vec<wgpu::VertexAttribute> = (0..INPUT_FORMATS.len() as u32)
                .filter(|location| !present.iter().any(|a| a.location == *location))
                .map(|location| wgpu::VertexAttribute { format: INPUT_FORMATS[location as usize], offset: 0, shader_location: location })
                .collect();
            let buffers = [
                Some(wgpu::VertexBufferLayout { array_stride: key.stride as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &from_vertex }),
                Some(wgpu::VertexBufferLayout { array_stride: 64, step_mode: wgpu::VertexStepMode::Instance, attributes: &from_defaults }),
            ];
            let blend = (key.blend[0] != 0).then(|| {
                let component = wgpu::BlendComponent {
                    src_factor: blend_factor(key.blend[1]),
                    dst_factor: blend_factor(key.blend[2]),
                    operation: wgpu::BlendOperation::Add,
                };
                wgpu::BlendState { color: component, alpha: component }
            });
            // Direct3D culls the faces with this winding. A front face of the game is clockwise.
            let cull_mode = match key.cull {
                2 => Some(wgpu::Face::Front),
                3 => Some(wgpu::Face::Back),
                _ => None,
            };
            let depth_on = key.depth[0] != 0;
            let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState { module: &self.shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &buffers },
                primitive: wgpu::PrimitiveState {
                    topology,
                    front_face: wgpu::FrontFace::Cw,
                    cull_mode,
                    polygon_mode: if key.fill == 2 && self.wireframe { wgpu::PolygonMode::Line } else { wgpu::PolygonMode::Fill },
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_on && key.depth[1] != 0),
                    depth_compare: Some(if depth_on { compare(key.depth[2]) } else { wgpu::CompareFunction::Always }),
                    stencil: if key.stencil[0] == 0 {
                        Default::default()
                    } else {
                        // Direct3D 8 has one stencil rule for the two sides of a triangle.
                        let face = wgpu::StencilFaceState {
                            compare: compare(key.stencil[4]),
                            fail_op: stencil_operation(key.stencil[1]),
                            depth_fail_op: stencil_operation(key.stencil[2]),
                            pass_op: stencil_operation(key.stencil[3]),
                        };
                        wgpu::StencilState { front: face, back: face, read_mask: key.stencil[5] & 0xFF, write_mask: key.stencil[6] & 0xFF }
                    },
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: self.target_format,
                        blend,
                        write_mask: wgpu::ColorWrites::from_bits_truncate(key.color_write),
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
            self.pipelines.insert(key, Rc::new(pipeline));
        }
        self.pipelines.get(&key).cloned()
    }
}

/// What the stream said before a draw.
struct Stream {
    view: [f32; 16],
    projection: [f32; 16],
    render_states: HashMap<u32, u32>,
    viewport: Option<([f32; 4], [f32; 2])>,
    back_buffer: bool,
    origin: Option<[f64; 3]>,
    /// The player is in an interior cell, from the scene packet.
    interior: bool,
    /// The fog distances of the extension for the draws of the world, or `None`.
    fog_range: Option<(f32, f32)>,
    /// The light for each pixel for the draw in progress, or `None`.
    per_pixel: Option<PerPixel>,
    /// The class of the object of the draws that follow: MGE_LINK_CLASS_* of its packet.
    object_class: u32,
    /// The sky values of the extension for the draw in progress, or `None`.
    sky: Option<[[f32; 4]; 8]>,
    state: Vec<u8>,
}

impl Stream {
    /// The data of the shader for the draw of the current state packet.
    /// Packs the lights of the draw for the light of a pixel, as `buildPplDrawData` and
    /// `decodeMorrowindPointLight` of MGE XE do (`ffeshader.cpp`).
    ///
    /// MGE XE takes at most 8 lights with the sun, the newest first. The lights of the state
    /// packet are in that order, so these are the last 8 of the packet in their order. The
    /// game gives a point light in one of four forms of its three falloff numbers, and each
    /// form has its own sum; one constant is for all lights of the draw, from the last light
    /// that has one.
    fn pixel_lights(&self, data: &mut DrawData, per_pixel: PerPixel, relative: Option<[f64; 3]>) {
        let s = &self.state;
        let lights = word(s, state::LIGHT_COUNT) as usize;
        let taken = lights.min(8);
        let mut shared_constant = 0.33f32;
        // The z axis of the camera in the space of the lights: MGE XE moves one form of
        // light 25 units along it.
        let camera_z = [self.view[2], self.view[6], self.view[10]];
        // A scene with white ambient light (the sky dome, a menu) has a fixed ambient light
        // and no sun.
        let white = word(s, state::AMBIENT) == 0xFFFF_FFFF;
        for slot in 0..taken {
            let at = state::SIZE + (lights - taken + slot) * state::LIGHT_SIZE + 4;
            if at + 104 > s.len() {
                break;
            }
            let diffuse: [f32; 4] = floats(s, at + 4);
            let ambient: [f32; 4] = floats(s, at + 9 * 4);
            let mut light = LightData::zeroed();
            if word(s, at) == 3 {
                let direction: [f32; 3] = floats(s, at + 16 * 4);
                let length = (direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2]).sqrt().max(1e-12);
                let (sun, amb) = if white { (0.0, 0.0) } else { (per_pixel.sun, per_pixel.ambient) };
                light.diffuse = [diffuse[0] * sun, diffuse[1] * sun, diffuse[2] * sun, 0.0];
                light.ambient = [ambient[0] * amb, ambient[1] * amb, ambient[2] * amb, 0.0];
                light.position = [0.0, 0.0, 0.0, 3.0];
                light.direction = [direction[0] / length, direction[1] / length, direction[2] / length, 0.0];
            } else {
                let mut position: [f32; 3] = floats(s, at + 13 * 4);
                if let Some(origin) = relative {
                    for axis in 0..3 {
                        position[axis] = (position[axis] as f64 - origin[axis]) as f32;
                    }
                }
                let falloff: [f32; 3] = floats(s, at + 21 * 4);
                let mut color = [diffuse[0], diffuse[1], diffuse[2]];
                let (mut wrap, mut quadratic, mut bias) = (0.0f32, 0.0f32, 0.0f32);
                if falloff[0] > 0.0 {
                    shared_constant = falloff[0];
                    quadratic = falloff[2];
                } else if falloff[2] > 0.0 {
                    color = color.map(|c| c * shared_constant);
                    wrap = 1.0 + 1e-4 / falloff[2].sqrt();
                    quadratic = shared_constant * falloff[2];
                } else if falloff[1] == 0.100_000_01 {
                    quadratic = 5e-5;
                } else if falloff[1] > 0.0 {
                    color = [0.25 + 1e-4 / falloff[1]; 3];
                    wrap = 1.0;
                    quadratic = 0.5555 * falloff[1] * falloff[1];
                    bias = 25.0;
                }
                // The plugin puts the radius of a light of the game into the first number of
                // the specular colour of the light.
                let radius = float(s, at + 5 * 4);
                let cutoff = if per_pixel.fade > 0.0 && radius > 0.0 { (per_pixel.fade * radius.max(16.0)).min(8192.0) } else { 0.0 };
                light.diffuse = [color[0], color[1], color[2], 0.0];
                light.ambient = [wrap, 0.0, 0.0, 0.0];
                light.position = [position[0] + bias * camera_z[0], position[1] + bias * camera_z[1], position[2] + bias * camera_z[2], 1.0];
                light.attenuation = [0.0, 0.0, quadratic, 0.0];
                light.cone = [if cutoff > 0.0 { 1.0 / cutoff } else { 0.0 }, 0.0, 0.0, 0.0];
            }
            data.lights[slot] = light;
            data.counts[0] = slot as u32 + 1;
        }
        for light in &mut data.lights[..data.counts[0] as usize] {
            light.attenuation[0] = shared_constant;
        }
        data.scene_ambient = if white {
            [1.25, 1.25, 1.25, 1.0]
        } else {
            [data.scene_ambient[0] * per_pixel.ambient, data.scene_ambient[1] * per_pixel.ambient, data.scene_ambient[2] * per_pixel.ambient, data.scene_ambient[3]]
        };
        // A draw that adds to the picture gets darker in the fog and no fog colour:
        // D3DBLEND_ONE is 2.
        let adds = known(word(s, state::BLEND_ENABLE), 0) != 0 && (known(word(s, state::SOURCE_BLEND), 2) == 2 || known(word(s, state::DESTINATION_BLEND), 1) == 2);
        data.counts[3] = 1 | if adds { 2 } else { 0 };
    }

    fn draw_data(&self, counts: &mut Counts) -> DrawData {
        let s = &self.state;
        let mut data = DrawData::zeroed();
        let fvf = word(s, state::VERTEX_FORMAT);
        // The world matrices are relative to the camera when the scene has an origin and the
        // packet says so. Then the camera is at 0.
        let relative = self.origin.filter(|_| word(s, state::WORLD_RELATIVE_MASK) & 1 != 0);
        data.world[0] = floats(s, state::WORLD);
        for i in 0..3 {
            data.world[i + 1] = floats(s, state::BLEND_WORLDS + i * 64);
        }
        data.view = self.view;
        if relative.is_some() {
            data.view[12] = 0.0;
            data.view[13] = 0.0;
            data.view[14] = 0.0;
        }
        data.projection = self.projection;
        if let Some((rectangle, _)) = self.viewport {
            data.pixel = [1.0 / rectangle[2].max(1.0), -1.0 / rectangle[3].max(1.0), 0.0, 0.0];
        }

        let material: [f32; 17] = floats(s, state::MATERIAL);
        data.material_diffuse = material[0..4].try_into().unwrap();
        data.material_ambient = material[4..8].try_into().unwrap();
        data.material_emissive = material[12..16].try_into().unwrap();
        data.scene_ambient = color(known(word(s, state::AMBIENT), 0));
        let state_of = |index: u32| self.render_states.get(&index).copied();
        // D3DRS_FOGCOLOR 34, FOGSTART 36, FOGEND 37, TEXTUREFACTOR 60.
        if let (1, Some(fog_color), Some(start), Some(end)) = (known(word(s, state::FOG_ENABLE), 0), state_of(34), state_of(36), state_of(37)) {
            data.fog_color = color(fog_color);
            // D3DRS_RANGEFOGENABLE is 48.
            data.fog = [f32::from_bits(start), f32::from_bits(end), 1.0, (state_of(48).unwrap_or(0) != 0) as u32 as f32];
            // The fog of the extension is for the world only: a perspective projection.
            if let (Some((start, end)), true) = (self.fog_range, self.projection[11] == 1.0) {
                data.fog[0] = start;
                data.fog[1] = end;
                // The fog of the extension is on the distance from the camera, as the fog
                // of what the extension draws: the two do not change with the direction
                // of the camera.
                data.fog[3] = 1.0;
            }
            // So is the fog of the shader for the light of a pixel.
            if self.per_pixel.is_some() && self.projection[11] == 1.0 {
                data.fog[3] = 1.0;
            }
        }
        data.texture_factor = color(state_of(60).unwrap_or(0xFFFF_FFFF));
        if let Some(sky) = self.sky {
            data.extension = sky;
            if word(s, state::TEXTURES) == 0 {
                // The sky dome has no texture.
                data.counts[3] |= 4;
            } else if self.object_class == 6 && known(word(s, state::DESTINATION_BLEND), 0) == 6 {
                // MGE_LINK_CLASS_SKY_MOON_SHADOW with D3DBLEND_INVSRCALPHA: the disc that keeps
                // the stars out of a moon.
                data.counts[3] |= 8;
            }
        }

        let has_color = fvf & 0x40 != 0;
        data.flags = [
            (known(word(s, state::LIGHTING), 1) != 0) as u32,
            (fvf & 0x10 != 0) as u32,
            has_color as u32,
            known(word(s, state::VERTEX_BLEND), 0).min(3),
        ];
        let alpha_test = known(word(s, state::ALPHA_TEST_ENABLE), 0) != 0;
        data.more = [
            if has_color { known(word(s, state::DIFFUSE_SOURCE), 1) } else { 0 },
            if has_color { known(word(s, state::EMISSIVE_SOURCE), 0) } else { 0 },
            if alpha_test { known(word(s, state::ALPHA_FUNCTION), 8) } else { 0 },
            known(word(s, state::ALPHA_REFERENCE), 0),
        ];

        let lights = word(s, state::LIGHT_COUNT) as usize;
        if lights > 8 {
            counts.more_lights += 1;
        }
        // The light for each pixel is for a draw of the world: a perspective projection.
        let per_pixel = self.per_pixel.filter(|_| self.projection[11] == 1.0);
        if let Some(per_pixel) = per_pixel {
            self.pixel_lights(&mut data, per_pixel, relative);
        }
        for i in 0..if per_pixel.is_some() { 0 } else { lights.min(8) } {
            let at = state::SIZE + i * state::LIGHT_SIZE + 4;
            if at + 104 > s.len() {
                break;
            }
            let mut position: [f32; 3] = floats(s, at + 13 * 4);
            if let Some(origin) = relative {
                for axis in 0..3 {
                    position[axis] = (position[axis] as f64 - origin[axis]) as f32;
                }
            }
            let direction: [f32; 3] = floats(s, at + 16 * 4);
            let attenuation: [f32; 3] = floats(s, at + 21 * 4);
            data.lights[i] = LightData {
                diffuse: floats(s, at + 4),
                ambient: floats(s, at + 9 * 4),
                position: [position[0], position[1], position[2], word(s, at) as f32],
                direction: [direction[0], direction[1], direction[2], float(s, at + 19 * 4)],
                // The falloff, and the two angles of a spot light: words 20, 24 and 25.
                attenuation: [attenuation[0], attenuation[1], attenuation[2], float(s, at + 20 * 4)],
                cone: [(float(s, at + 24 * 4) * 0.5).cos(), (float(s, at + 25 * 4) * 0.5).cos(), 0.0, 0.0],
            };
            data.counts[0] = i as u32 + 1;
        }

        let mut stages = 0;
        while stages < 8 {
            let at = state::STAGES + stages * state::STAGE_SIZE;
            let op = known(word(s, at), 1);
            if op == 1 {
                break;
            }
            if stages < STAGES {
                // The coordinate index has the way to generate it in its high word. The
                // transform flags are the number of parts, and 256 for a projected coordinate.
                let coordinate = known(word(s, at + 9 * 4), stages as u32);
                let transform = known(word(s, at + 10 * 4), 0);
                if coordinate >> 16 != 0 || transform != 0 {
                    counts.coordinate_work += 1;
                }
                // D3DTA_TEXTURE is 2 and D3DTA_CURRENT is 1: the defaults of Direct3D.
                data.stages[stages] = StageData {
                    color: [op, known(word(s, at + 4), 2), known(word(s, at + 8), 1), known(word(s, at + 24), 1)],
                    alpha: [known(word(s, at + 12), 2), known(word(s, at + 16), 2), known(word(s, at + 20), 1), 0],
                    coordinate: [coordinate >> 16 & 3, coordinate & 3, transform & 7, (transform & 256 != 0) as u32],
                    // The four values of the bump matrix, then the scale and the offset: floats.
                    bump: core::array::from_fn(|i| f32::from_bits(known(word(s, at + (11 + i) * 4), 0))),
                    luminance: [f32::from_bits(known(word(s, at + 15 * 4), 0)), f32::from_bits(known(word(s, at + 16 * 4), 0)), 0.0, 0.0],
                };
                data.texture_transforms[stages] = floats(s, state::TEXTURE_TRANSFORMS + stages * 64);
            }
            stages += 1;
        }
        if stages > STAGES {
            counts.more_stages += 1;
        }
        data.counts[1] = stages.min(STAGES) as u32;
        // D3DRS_AMBIENTMATERIALSOURCE is 147. The game sends it as a command.
        data.counts[2] = if has_color { state_of(147).unwrap_or(0) } else { 0 };
        data
    }
}

/// Reads the records of a stream in their order and draws the frames.
struct Player {
    renderer: Renderer,
    stream: Stream,
    /// The frame in progress: its commands, its pass, and its two targets.
    encoder: Option<wgpu::CommandEncoder>,
    pass: Option<wgpu::RenderPass<'static>>,
    color: Option<Rc<wgpu::TextureView>>,
    depth: Option<Rc<wgpu::TextureView>>,
    /// The texture of `color`, for a read of its pixels. It stays after the frame.
    back_buffer: Option<wgpu::Texture>,
    /// Takes the answer to a pixel read: the number of the request, the width, the height
    /// and the pixels with 4 bytes each, blue first. `None` without a game to answer.
    reply: Option<Box<dyn FnMut(u32, u32, u32, &[u8])>>,
    /// The picture files that the game asked for: the frame in progress goes into them.
    shots: Vec<String>,
    /// A program that draws more than the game does, and true when it had its call in this
    /// frame.
    extension: Option<Box<dyn Extension>>,
    world_started: bool,
    /// The extension had its call at the end of the world scene in this frame.
    world_ended: bool,
    /// The kind of the scene in progress, from its packet: 1 is the world.
    scene_kind: u32,
    /// The sun and the ambient light of the draw that started the world: the direction, the
    /// colour and the ambient colour of the sun, and the ambient light of the scene.
    world_sun: Option<([f32; 3], [f32; 3], [f32; 3])>,
    world_ambient: [f32; 3],
    /// The light for each pixel of the frame, from the extension.
    frame_per_pixel: Option<PerPixel>,
    /// The picture after `Extension::before_world`, for `Extension::blend`, and true when it
    /// is that of this frame.
    before: Option<wgpu::Texture>,
    before_kept: bool,
    /// The sky values of the extension for this frame: `None` before the first sky draw.
    frame_sky: Option<Option<[[f32; 4]; 8]>>,
    /// The world draws of the frame, for `Replay`. Only with an extension.
    kept: Vec<Kept>,
    /// The texture that the game draws into now. `None` for the back buffer.
    target: Option<u32>,
    /// The game named a rendered texture as its target.
    wants_texture: bool,
    /// The size of the back buffer of the game, from the first record. 0 when not known.
    size: (u32, u32),
    /// The size of the target of the pass in progress, and the size of the picture of the frame.
    target_size: (u32, u32),
    frame_size: (u32, u32),
}

impl Player {
    fn new(renderer: Renderer) -> Player {
        Player {
            renderer,
            stream: Stream {
                view: [0.0; 16],
                projection: [0.0; 16],
                render_states: HashMap::new(),
                viewport: None,
                back_buffer: true,
                origin: None,
                interior: false,
                fog_range: None,
                per_pixel: None,
                object_class: 0,
                sky: None,
                state: Vec::new(),
            },
            encoder: None,
            pass: None,
            color: None,
            depth: None,
            back_buffer: None,
            reply: None,
            shots: Vec::new(),
            extension: None,
            world_started: false,
            world_ended: false,
            scene_kind: 0,
            world_sun: None,
            world_ambient: [0.0; 3],
            frame_per_pixel: None,
            before: None,
            before_kept: false,
            frame_sky: None,
            kept: Vec::new(),
            target: None,
            wants_texture: false,
            size: (0, 0),
            target_size: (0, 0),
            frame_size: (0, 0),
        }
    }

    /// Starts a pass on the targets of the frame: it draws on what is there, or it clears first.
    fn open_pass(&mut self, clear_color: Option<wgpu::Color>, clear_depth: Option<f32>, clear_stencil: Option<u32>) {
        drop(self.pass.take());
        let rendered = self.target.and_then(|handle| self.renderer.targets.get(&handle));
        let views = match rendered {
            Some(texture) => Some((texture.view.clone(), texture.depth.clone(), texture.size)),
            None => self.color.clone().zip(self.depth.clone()).map(|(color, depth)| (color, depth, self.frame_size)),
        };
        let (Some(encoder), Some((color, depth, size))) = (self.encoder.as_mut(), views) else { return };
        self.target_size = size;
        let (color, depth) = (&*color, &*depth);
        let pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: clear_color.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations { load: clear_depth.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear), store: wgpu::StoreOp::Store }),
                    stencil_ops: Some(wgpu::Operations { load: clear_stencil.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear), store: wgpu::StoreOp::Store }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        self.pass = Some(pass);
        self.apply_viewport();
    }

    /// Gives the viewport of the stream to the pass, cut to the size of the targets.
    fn apply_viewport(&mut self) {
        let (Some(pass), Some((rectangle, range))) = (self.pass.as_mut(), self.stream.viewport) else { return };
        let (width, height) = (self.target_size.0 as f32, self.target_size.1 as f32);
        let (x, y) = (rectangle[0].min(width - 1.0).max(0.0), rectangle[1].min(height - 1.0).max(0.0));
        let (w, h) = (rectangle[2].min(width - x), rectangle[3].min(height - y));
        if w >= 1.0 && h >= 1.0 {
            pass.set_viewport(x, y, w, h, range[0], range[1]);
        }
    }

    /// Starts a frame that draws into these targets. Until the next start, the records
    /// change only the state and the resources. With `keep`, the frame starts with the
    /// picture that is in the colour target: the game can read the last frame before it
    /// clears the target.
    fn begin_frame(&mut self, color: &wgpu::Texture, depth: Rc<wgpu::TextureView>, keep: bool) {
        self.frame_size = (color.width(), color.height());
        self.target = None;
        self.world_started = false;
        self.world_ended = false;
        self.frame_per_pixel = None;
        self.before_kept = false;
        self.frame_sky = None;
        self.kept.clear();
        self.stream.fog_range = None;
        self.stream.back_buffer = true;
        self.encoder = Some(self.renderer.device.create_command_encoder(&Default::default()));
        self.renderer.draw_slots = 0;
        self.color = Some(Rc::new(color.create_view(&Default::default())));
        self.back_buffer = Some(color.clone());
        self.depth = Some(depth);
        if keep {
            self.open_pass(None, None, None);
        } else {
            self.open_pass(Some(wgpu::Color::BLACK), Some(1.0), Some(0));
        }
    }

    /// Sends the commands of the frame up to now to the GPU. The frame goes on after `between`.
    fn flush<T>(&mut self, between: impl FnOnce(&mut Player) -> T) -> T {
        let in_frame = self.drawing();
        drop(self.pass.take());
        if let Some(encoder) = self.encoder.take() {
            self.renderer.queue.submit([encoder.finish()]);
        }
        let result = between(self);
        if in_frame {
            self.encoder = Some(self.renderer.device.create_command_encoder(&Default::default()));
            self.open_pass(None, None, None);
        }
        result
    }

    /// The pixels of a rendered texture, or of the back buffer with the handle 0: the width,
    /// the height and 4 bytes for each pixel, blue first.
    fn read_pixels(&mut self, handle: u32) -> Option<(u32, u32, Vec<u8>)> {
        let texture = match handle {
            0 => self.back_buffer.clone()?,
            _ => self.renderer.targets.get(&handle)?.texture.clone(),
        };
        if !texture.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            return None;
        }
        let (width, height) = (texture.width(), texture.height());
        let blue_first = matches!(texture.format(), wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
        self.flush(|player| {
            let device = player.renderer.device.clone();
            let mut encoder = device.create_command_encoder(&Default::default());
            let (buffer, row) = copy_picture(&device, &mut encoder, &texture, width, height);
            player.renderer.queue.submit([encoder.finish()]);
            buffer.slice(..).map_async(wgpu::MapMode::Read, |result| result.expect("the read of the pixels"));
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            let mapped = buffer.slice(..).get_mapped_range().expect("the bytes of the pixels");
            let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
            for y in 0..height as usize {
                pixels.extend_from_slice(&mapped[y * row..y * row + width as usize * 4]);
            }
            if !blue_first {
                for pixel in pixels.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                }
            }
            player.renderer.counts.pixel_reads += 1;
            Some((width, height, pixels))
        })
    }

    fn drawing(&self) -> bool {
        self.encoder.is_some()
    }

    /// Ends the frame and sends its commands to the GPU. `before_submit` can add commands,
    /// for example a copy of the picture.
    fn end_frame(&mut self, before_submit: impl FnOnce(&mut wgpu::CommandEncoder)) {
        let clock = std::time::Instant::now();
        drop(self.pass.take());
        self.color = None;
        self.depth = None;
        if let Some(mut encoder) = self.encoder.take() {
            before_submit(&mut encoder);
            self.renderer.queue.submit([encoder.finish()]);
        }
        self.renderer.counts.times[TIME_END] += clock.elapsed();
    }

    /// Does one record. Returns true for the record that ends a frame.
    fn feed(&mut self, tag: u32, data: &[u8]) -> bool {
        match tag {
            tag::HEADER => self.size = (word(data, 0), word(data, 4)),
            tag::TEXTURE => self.renderer.texture(data),
            tag::TEXTURE_UPDATE => self.renderer.texture_update(data),
            tag::SHOT => self.shots.push(String::from_utf8_lossy(data).into_owned()),
            tag::EXTENSION if data.len() >= 4 => {
                if let Some(extension) = self.extension.as_mut() {
                    extension.data(word(data, 0), &data[4..]);
                }
            }
            tag::DRAW_UP if self.drawing() && data.len() >= 20 => self.draw_up(data),
            tag::READ_PIXELS => {
                // The game waits for the answer. Without the target, the answer has no size.
                let answer = self.read_pixels(word(data, 4));
                if let Some(reply) = self.reply.as_mut() {
                    match answer {
                        Some((width, height, pixels)) => reply(word(data, 0), width, height, &pixels),
                        None => reply(word(data, 0), 0, 0, &[]),
                    }
                }
            }
            tag::BUFFER_DATA => self.renderer.buffer_data(data),
            tag::SCENE_BEGIN => {
                self.stream.origin = (word(data, 20) & 1 != 0).then(|| [double(data, 24), double(data, 32), double(data, 40)]);
                self.stream.interior = word(data, 20) & 2 != 0;
                self.scene_kind = word(data, 4);
            }
            tag::SCENE_END => {
                // The end of the world scene of the game, in the back buffer.
                if self.scene_kind == 1 && self.world_started && !self.world_ended && self.target.is_none() && self.drawing() {
                    self.world_ended = true;
                    self.end_world();
                }
                self.scene_kind = 0;
            }
            tag::FRAME_EVENT => {
                if word(data, 0) == EVENT_SET_TARGET {
                    self.wants_texture = word(data, 4) != 0;
                }
            }
            tag::DRAW_STATE => self.stream.state = data.to_vec(),
            // The kind, the class, the flags and the geometry of the object.
            tag::OBJECT if data.len() >= 8 => self.stream.object_class = word(data, 4),
            tag::FRAME_END => return true,
            tag::CALL => {
                let arguments = &data[16..];
                match word(data, 0) {
                    CALL_VIEWPORT => {
                        let viewport = &arguments[word(data, 12) as usize * 4..];
                        if viewport.len() >= 24 {
                            let rectangle = [word(viewport, 0) as f32, word(viewport, 4) as f32, word(viewport, 8) as f32, word(viewport, 12) as f32];
                            self.stream.viewport = Some((rectangle, [float(viewport, 16), float(viewport, 20)]));
                            if self.stream.back_buffer {
                                self.apply_viewport();
                            }
                        }
                    }
                    CALL_CREATE_TEXTURE => {
                        // Width, height, levels, usage, format, pool. D3DUSAGE_RENDERTARGET is 1.
                        let handle = word(data, 8);
                        if handle != 0 && word(arguments, 12) & 1 != 0 {
                            self.renderer.render_texture(handle, word(arguments, 0), word(arguments, 4));
                        } else if handle != 0 {
                            self.renderer.empty_texture(handle, word(arguments, 0), word(arguments, 4), word(arguments, 8), word(arguments, 16));
                        }
                    }
                    CALL_TEXTURE_LEVEL => {
                        // Texture, level; the handle that the call gave is the surface.
                        if word(data, 8) != 0 && word(arguments, 4) == 0 {
                            self.renderer.surfaces.insert(word(data, 8), word(arguments, 0));
                        }
                    }
                    CALL_RENDER_TARGET => {
                        // The colour surface and the depth surface. A surface of a texture
                        // that this program made is that texture; each other one is the
                        // back buffer.
                        let texture = self.renderer.surfaces.get(&word(arguments, 0)).copied().filter(|handle| self.renderer.targets.contains_key(handle));
                        // A rendered texture that is not known here: its draws go nowhere.
                        self.stream.back_buffer = texture.is_some() || !self.wants_texture;
                        if texture != self.target {
                            self.target = texture;
                            if self.drawing() {
                                self.open_pass(None, None, None);
                            }
                        }
                    }
                    CALL_COPY_PIXELS => {
                        // Source surface, rectangles, their number, destination surface,
                        // points. With pixels, the source is an image surface: the game read
                        // a rendered texture, changed the pixels, and now writes them back.
                        let pixels = &arguments[(word(data, 12) as usize * 4).min(arguments.len())..];
                        // The record names the texture, or the surface is one that a call
                        // of the stream gave.
                        let named = (pixels.len() >= 20 && word(pixels, 16) != 0).then(|| word(pixels, 16));
                        let texture = named.or_else(|| self.renderer.surfaces.get(&word(arguments, 12)).copied());
                        if let (Some(texture), true) = (texture, pixels.len() >= 20) {
                            // The draws into the texture come first: a write of a texture is
                            // done before the commands that are not sent yet.
                            self.flush(|player| player.renderer.target_pixels(texture, word(pixels, 0), word(pixels, 4), &pixels[20..]));
                        }
                    }
                    CALL_CLEAR if self.drawing() && self.stream.back_buffer => {
                        // Rectangles, their address, flags, colour, depth, stencil.
                        let flags = word(arguments, 8);
                        let clear_color = (flags & 1 != 0).then(|| {
                            let c = color(word(arguments, 12));
                            wgpu::Color { r: c[0] as f64, g: c[1] as f64, b: c[2] as f64, a: c[3] as f64 }
                        });
                        let clear_depth = (flags & 2 != 0).then(|| float(arguments, 16));
                        let clear_stencil = (flags & 4 != 0).then(|| word(arguments, 20));
                        self.open_pass(clear_color, clear_depth, clear_stencil);
                    }
                    _ => {}
                }
            }
            tag::COMMAND => match word(data, 0) {
                // A state command after a state packet is for the draws that follow: the Code
                // Patch changes the blend and the alpha test around its draws into a
                // rendered texture.
                COMMAND_RENDER_STATE => {
                    self.stream.render_states.insert(word(data, 4), word(data, 8));
                    if let Some(at) = state::of_render_state(word(data, 4)).filter(|at| at + 4 <= self.stream.state.len()) {
                        self.stream.state[at..at + 4].copy_from_slice(&data[8..12]);
                    }
                }
                COMMAND_STAGE_STATE => {
                    if let Some(at) = state::of_stage_state(word(data, 4), word(data, 8)).filter(|at| at + 4 <= self.stream.state.len()) {
                        self.stream.state[at..at + 4].copy_from_slice(&data[12..16]);
                    }
                }
                COMMAND_TRANSFORM if data.len() >= 16 + 64 => match word(data, 4) {
                    2 => self.stream.view = floats(data, 16),
                    3 => self.stream.projection = floats(data, 16),
                    _ => {}
                },
                COMMAND_DRAW_INDEXED if self.drawing() && data.len() >= 16 + 20 && self.stream.state.len() >= state::SIZE => self.draw(data, true),
                // A draw without an index buffer: a screen polygon, for example the picture
                // of a loading screen or of the main menu.
                COMMAND_DRAW if self.drawing() && data.len() >= 16 + 12 && self.stream.state.len() >= state::SIZE => self.draw(data, false),
                _ => {}
            },
            _ => {}
        }
        false
    }

    /// A draw with its vertices in the record. This program does the vertex format of the
    /// films of the game: a position on the screen, a colour and one texture coordinate
    /// (D3DFVF_XYZRHW | D3DFVF_DIFFUSE | D3DFVF_TEX1, 28 bytes).
    fn draw_up(&mut self, data: &[u8]) {
        let (texture, vertex_format, stride, kind, primitives) = (word(data, 0), word(data, 4), word(data, 8) as usize, word(data, 12), word(data, 16) as usize);
        let vertices = &data[20..];
        if vertex_format != 0x144 || stride != 28 || !self.stream.back_buffer || !(4..=6).contains(&kind) {
            self.renderer.counts.screen_draws_lost += 1;
            return;
        }
        let count = vertices.len() / stride;
        let (width, height) = (self.target_size.0 as f32, self.target_size.1 as f32);
        // The centre of a pixel is at a whole number in Direct3D 8, and at a half here.
        let vertex = |i: usize| -> [f32; 10] {
            let v = &vertices[i * stride..];
            let c = color(word(v, 16));
            [(float(v, 0) + 0.5) / width * 2.0 - 1.0, 1.0 - (float(v, 4) + 0.5) / height * 2.0, float(v, 8).clamp(0.0, 1.0), 1.0, c[0], c[1], c[2], c[3], float(v, 20), float(v, 24)]
        };
        // A triangle list from a list, a strip or a fan.
        let mut list: Vec<f32> = Vec::new();
        for n in 0..primitives {
            let corners = match kind {
                4 => [n * 3, n * 3 + 1, n * 3 + 2],
                5 if n % 2 == 0 => [n, n + 1, n + 2],
                5 => [n + 1, n, n + 2],
                _ => [0, n + 1, n + 2],
            };
            if corners.iter().all(|i| *i < count) {
                for i in corners {
                    list.extend_from_slice(&vertex(i));
                }
            }
        }
        if list.is_empty() {
            return;
        }
        // D3DRS_ALPHABLENDENABLE is 27.
        let blend = self.stream.render_states.get(&27).copied().unwrap_or(0) != 0;
        let renderer = &mut self.renderer;
        let pipeline = renderer.screen_pipeline(blend);
        let buffer = renderer.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&list), usage: wgpu::BufferUsages::VERTEX });
        let view = renderer.textures.get(&texture).unwrap_or(&renderer.white).clone();
        let group = renderer.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: renderer.screen_layout.as_ref().unwrap(),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(renderer.screen_sampler.as_ref().unwrap()) },
            ],
        });
        let Some(pass) = self.pass.as_mut() else { return };
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(0..(list.len() / 10) as u32, 0..1);
        renderer.counts.screen_draws += 1;
    }

    /// Gives the extension its call at the end of the world scene. See `Extension::after_world`.
    fn end_world(&mut self) {
        let Some(mut extension) = self.extension.take() else { return };
        let state_of = |index: u32| self.stream.render_states.get(&index).copied();
        let scene = SceneView {
            device: &self.renderer.device,
            queue: &self.renderer.queue,
            target_format: self.renderer.target_format,
            depth_format: DEPTH_FORMAT,
            target_size: self.target_size,
            view: self.stream.view,
            projection: self.stream.projection,
            interior: self.stream.interior,
            fog_color: color(state_of(34).unwrap_or(0)),
            fog_start: f32::from_bits(state_of(36).unwrap_or(0)),
            fog_end: f32::from_bits(state_of(37).unwrap_or(0)),
            sun_direction: self.world_sun.map(|sun| sun.0),
            sun_color: self.world_sun.map_or([0.0; 3], |sun| sun.1),
            sun_ambient: self.world_sun.map_or([0.0; 3], |sun| sun.2),
            ambient: self.world_ambient,
        };
        if let Some(pass) = self.pass.as_mut() {
            extension.after_world(&scene, pass);
        }
        // The blend: a pass on the picture only, so that a shader can read the depth.
        if let (true, Some(before), Some(color), Some(depth), Some(encoder)) = (self.before_kept, self.before.as_ref(), self.color.clone(), self.depth.clone(), self.encoder.as_mut()) {
            drop(self.pass.take());
            let before = before.create_view(&Default::default());
            let depth = depth.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("blend"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            extension.blend(&scene, &mut pass, &before, &depth);
            drop(pass);
            self.extension = Some(extension);
            self.open_pass(None, None, None);
            return;
        }
        self.extension = Some(extension);
    }

    /// Keeps the picture of the frame as it is now, for `Extension::blend`.
    fn keep_before(&mut self) {
        let (Some(source), Some(encoder)) = (self.back_buffer.as_ref(), self.encoder.as_mut()) else { return };
        if !source.usage().contains(wgpu::TextureUsages::COPY_SRC) || !self.depth.as_ref().is_some_and(|depth| depth.texture().usage().contains(wgpu::TextureUsages::TEXTURE_BINDING)) {
            return;
        }
        let size = source.size();
        if self.before.as_ref().is_none_or(|before| before.size() != size) {
            self.before = Some(target_texture(&self.renderer.device, size.width, size.height, source.format(), wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST));
        }
        // A copy is not a part of a pass.
        drop(self.pass.take());
        encoder.copy_texture_to_texture(source.as_image_copy(), self.before.as_ref().unwrap().as_image_copy(), size);
        self.before_kept = true;
    }

    /// Gives the extension its call of the frame. See `Extension::before_world`.
    fn start_world(&mut self) {
        let Some(mut extension) = self.extension.take() else { return };
        let s = &self.stream.state;
        let state_of = |index: u32| self.stream.render_states.get(&index).copied();
        // The first light of the draw that is a sun or another directional light: type 3,
        // or type 4 for the light that the plugin names as the sun.
        let mut sun = None;
        for i in 0..(word(s, state::LIGHT_COUNT) as usize).min(8) {
            let at = state::SIZE + i * state::LIGHT_SIZE + 4;
            if at + 104 <= s.len() && matches!(word(s, at), 3 | 4) {
                let diffuse: [f32; 3] = floats(s, at + 4);
                let light_ambient: [f32; 3] = floats(s, at + 9 * 4);
                // The game does not give a direction of length 1.
                let direction: [f32; 3] = floats(s, at + 16 * 4);
                let length = direction.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
                sun = Some((direction.map(|v| v / length), diffuse, light_ambient));
                break;
            }
        }
        let ambient = color(known(word(s, state::AMBIENT), 0));
        self.world_sun = sun;
        self.world_ambient = [ambient[0], ambient[1], ambient[2]];
        let scene = SceneView {
            device: &self.renderer.device,
            queue: &self.renderer.queue,
            target_format: self.renderer.target_format,
            depth_format: DEPTH_FORMAT,
            target_size: self.target_size,
            view: self.stream.view,
            projection: self.stream.projection,
            interior: self.stream.interior,
            fog_color: color(state_of(34).unwrap_or(0)),
            fog_start: f32::from_bits(state_of(36).unwrap_or(0)),
            fog_end: f32::from_bits(state_of(37).unwrap_or(0)),
            sun_direction: sun.map(|(direction, _, _)| direction),
            sun_color: sun.map_or([0.0; 3], |(_, diffuse, _)| diffuse),
            sun_ambient: sun.map_or([0.0; 3], |(_, _, light_ambient)| light_ambient),
            ambient: [ambient[0], ambient[1], ambient[2]],
        };
        let clear_depth = match self.pass.as_mut() {
            Some(pass) => extension.before_world(&scene, pass),
            None => false,
        };
        self.stream.fog_range = extension.fog_range();
        self.frame_per_pixel = extension.per_pixel();
        let keep = clear_depth && extension.wants_blend();
        self.extension = Some(extension);
        if keep {
            self.keep_before();
        }
        if clear_depth {
            // The draws of the game start with a depth target of their own.
            self.open_pass(None, Some(1.0), None);
        }
    }

    /// Draws with the current state packet. `data` is the draw command: with an index buffer,
    /// or the vertices in their order.
    fn draw(&mut self, data: &[u8], indexed: bool) {
        let renderer = &mut self.renderer;
        if !self.stream.back_buffer {
            renderer.counts.other_target += 1;
            return;
        }
        // The world of the game starts with its first draw into the back buffer that has fog,
        // writes depth and has a perspective projection. The sky is before it.
        if self.extension.is_some() && !self.world_started && self.target.is_none() {
            let s = &self.stream.state;
            if known(word(s, state::FOG_ENABLE), 0) != 0 && known(word(s, state::DEPTH_WRITE), 1) != 0 && self.stream.projection[11] == 1.0 {
                self.world_started = true;
                self.start_world();
            }
        }
        let renderer = &mut self.renderer;
        let s = &self.stream.state;
        // With an index buffer: primitive type, least index, vertices, first index, primitives.
        // Without: primitive type, first vertex, primitives.
        let (topology, first_index, primitives) = if indexed { (word(data, 16), word(data, 28), word(data, 32)) } else { (word(data, 16), word(data, 20), word(data, 24)) };
        // The GPU has no triangle fan. Without an index buffer, a fan is a triangle list with
        // the indexes 0, n + 1, n + 2 for the triangle n.
        let fan = topology == 6 && !indexed && primitives * 3 <= FAN_INDEXES;
        let (topology, primitives) = if fan { (4, primitives) } else { (topology, primitives) };
        let indexes = match topology {
            4 => primitives * 3,
            5 => primitives + 2,
            2 => primitives * 2,
            3 => primitives + 1,
            1 => primitives,
            _ => {
                renderer.counts.topology += 1;
                return;
            }
        };
        let key = PipelineKey {
            fvf: word(s, state::VERTEX_FORMAT),
            stride: word(s, state::VERTEX_STRIDE),
            topology,
            blend: [(known(word(s, state::BLEND_ENABLE), 0) != 0) as u32, known(word(s, state::SOURCE_BLEND), 2), known(word(s, state::DESTINATION_BLEND), 1)],
            depth: [
                (known(word(s, state::DEPTH_ENABLE), 1) != 0) as u32,
                (known(word(s, state::DEPTH_WRITE), 1) != 0) as u32,
                known(word(s, state::DEPTH_FUNCTION), 4),
            ],
            cull: known(word(s, state::CULL_MODE), 3),
            // D3DRS_FILLMODE is 8. The game sends it as a command.
            fill: self.stream.render_states.get(&8).copied().unwrap_or(3),
            color_write: 15,
            stencil: {
                let v: [u32; 8] = core::array::from_fn(|i| word(s, state::STENCIL + i * 4));
                if known(v[0], 0) == 0 {
                    [0; 7]
                } else {
                    [1, known(v[1], 1), known(v[2], 1), known(v[3], 1), known(v[4], 8), known(v[6], !0), known(v[7], !0)]
                }
            },
        };
        let stencil_reference = known(word(s, state::STENCIL + 5 * 4), 0);
        // D3DRS_COLORWRITEENABLE is 168.
        let key = PipelineKey { color_write: self.stream.render_states.get(&168).copied().unwrap_or(15) & 15, ..key };
        let (vertex_handle, index_handle) = (word(s, state::VERTEX_BUFFER), word(s, state::INDEX_BUFFER));
        let Some(vertex_buffer) = renderer.gpu_buffer(vertex_handle, false) else {
            renderer.counts.no_buffer += 1;
            return;
        };
        let index_buffer = if indexed { renderer.gpu_buffer(index_handle, true) } else { None };
        if indexed && index_buffer.is_none() {
            renderer.counts.no_buffer += 1;
            return;
        }
        let clock = std::time::Instant::now();
        // The light for each pixel is for the world in the back buffer: not for the sky,
        // which is before the world, and not for a texture that the game draws into.
        self.stream.per_pixel = self.frame_per_pixel.filter(|_| self.world_started && self.target.is_none());
        // The sky of the extension is for a sky draw in the back buffer: MGE_LINK_CLASS_SKY_DOME
        // is 1 and MGE_LINK_CLASS_SKY_OTHER is 7.
        self.stream.sky = None;
        if self.target.is_none() && (1..=7).contains(&self.stream.object_class) {
            let extension = self.extension.as_mut();
            self.stream.sky = *self.frame_sky.get_or_insert_with(|| extension.and_then(|extension| extension.sky()));
        }
        let draw_data = self.stream.draw_data(&mut renderer.counts);
        renderer.counts.times[TIME_DRAW_DATA] += clock.elapsed();
        let clock = std::time::Instant::now();
        let (draw_group, draw_offset) = renderer.draw_slot(&draw_data);
        renderer.counts.times[TIME_DRAW_UPLOAD] += clock.elapsed();
        let clock = std::time::Instant::now();
        let mut handles = [0u32; STAGES];
        let mut sampler_values = [[0u32; 5]; STAGES];
        for stage in 0..STAGES {
            let handle = word(s, state::TEXTURES + stage * 4);
            let used = stage < draw_data.counts[1] as usize && handle != 0;
            if used && !renderer.textures.contains_key(&handle) {
                renderer.counts.missing_textures += 1;
            } else if used {
                handles[stage] = handle;
                sampler_values[stage] = core::array::from_fn(|i| word(s, state::SAMPLERS + stage * 20 + i * 4));
            }
        }
        if !renderer.texture_groups.contains_key(&handles) {
            let views: Vec<Rc<wgpu::TextureView>> = handles.iter().map(|handle| renderer.textures.get(handle).unwrap_or(&renderer.white).clone()).collect();
            let entries: Vec<wgpu::BindGroupEntry> =
                (0..STAGES).map(|stage| wgpu::BindGroupEntry { binding: stage as u32, resource: wgpu::BindingResource::TextureView(&views[stage]) }).collect();
            let group = renderer.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &renderer.texture_layout, entries: &entries });
            renderer.texture_groups.insert(handles, Rc::new(group));
        }
        let texture_group = renderer.texture_groups[&handles].clone();
        if !renderer.sampler_groups.contains_key(&sampler_values) {
            let samplers: Vec<Rc<wgpu::Sampler>> = sampler_values.iter().map(|values| renderer.sampler(*values)).collect();
            let entries: Vec<wgpu::BindGroupEntry> =
                (0..STAGES).map(|stage| wgpu::BindGroupEntry { binding: stage as u32, resource: wgpu::BindingResource::Sampler(&samplers[stage]) }).collect();
            let group = renderer.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &renderer.sampler_layout, entries: &entries });
            renderer.sampler_groups.insert(sampler_values, Rc::new(group));
        }
        let sampler_group = renderer.sampler_groups[&sampler_values].clone();
        let Some(pipeline) = renderer.pipeline(key) else {
            renderer.counts.no_layout += 1;
            return;
        };
        renderer.counts.times[TIME_DRAW_GROUPS] += clock.elapsed();
        // The world scene of the game is 1 and its scene of sorted draws is 2. The water
        // plane of the game (MGE_LINK_CLASS_WATER_PLANE, 10) is not a thing of the world.
        if self.extension.is_some() && self.world_started && self.target.is_none() && matches!(self.scene_kind, 1 | 2) && self.stream.object_class != 10 {
            self.kept.push(Kept {
                key,
                stencil_reference,
                data: draw_data,
                origin: self.stream.origin.filter(|_| word(s, state::WORLD_RELATIVE_MASK) & 1 != 0),
                textures: texture_group.clone(),
                samplers: sampler_group.clone(),
                vertices: vertex_buffer.clone(),
                indexes: index_buffer.clone(),
                fan,
                range: if indexed { (first_index, indexes, word(s, state::BASE_VERTEX) as i32) } else { (first_index, indexes, 0) },
            });
        }
        let clock = std::time::Instant::now();
        let Some(pass) = self.pass.as_mut() else { return };
        pass.set_pipeline(&pipeline);
        pass.set_stencil_reference(stencil_reference);
        pass.set_bind_group(0, &*draw_group, &[draw_offset]);
        pass.set_bind_group(1, &*texture_group, &[]);
        pass.set_bind_group(2, &*sampler_group, &[]);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, renderer.defaults.slice(..));
        if let Some(index_buffer) = index_buffer.as_ref() {
            pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(first_index..first_index + indexes, word(s, state::BASE_VERTEX) as i32, 0..1);
        } else if fan {
            pass.set_index_buffer(renderer.fan_indexes.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..indexes, first_index as i32, 0..1);
        } else {
            pass.draw(first_index..first_index + indexes, 0..1);
        }
        renderer.counts.times[TIME_DRAW_PASS] += clock.elapsed();
        renderer.counts.draws += 1;
        if self.target.is_some() {
            renderer.counts.target_draws += 1;
        }
    }

    fn print_counts(&self) {
        let c = &self.renderer.counts;
        println!("draws {}, pipelines {}, textures {}, samplers {}", c.draws, self.renderer.pipelines.len(), c.textures, self.renderer.samplers.len());
        println!("not drawn: other target {}, vertex format {}, no buffer data {}, primitive type {}", c.other_target, c.no_layout, c.no_buffer, c.topology);
        println!(
            "not complete: draws with a texture that is not in the stream {}, more than 6 stages {}, more than 8 lights {}; draws with a made or moved texture coordinate {}",
            c.missing_textures, c.more_stages, c.more_lights, c.coordinate_work
        );
        println!("texture levels written again {}, not written {}; textures with a block format made as pixels {}", c.texture_updates, c.texture_updates_lost, c.textures_decoded);
        for (reason, count) in &c.lost_reasons {
            println!("  not written: {reason}: {count}");
        }
        println!(
            "rendered textures made {}, draws into them {}, copies of pixels into them {}, pixel reads {}",
            c.render_textures, c.target_draws, c.target_copies, c.pixel_reads
        );
        println!("draws in screen coordinates {}, not drawn {}", c.screen_draws, c.screen_draws_lost);
        for (format, count) in &c.texture_formats {
            println!("texture format {format:#x} not made: {count}");
        }
    }
}

fn target_texture(device: &wgpu::Device, width: u32, height: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

/// Adds the copy of a picture into a new buffer to the commands. Returns the buffer and the
/// length of a row in it.
fn copy_picture(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture, width: u32, height: u32) -> (wgpu::Buffer, usize) {
    let row = (width as usize * 4).next_multiple_of(256);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * height as usize) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row as u32), rows_per_image: None } },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    (buffer, row)
}

/// Saves a picture that `copy_picture` put into `buffer` as a PNG file. `blue_first` for a
/// texture format with the bytes blue, green, red.
fn save_picture(device: &wgpu::Device, buffer: &wgpu::Buffer, row: usize, width: u32, height: u32, blue_first: bool, path: &str) {
    buffer.slice(..).map_async(wgpu::MapMode::Read, |result| result.expect("the read of the picture"));
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let mapped = buffer.slice(..).get_mapped_range().expect("the bytes of the picture");
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        pixels.extend_from_slice(&mapped[y * row..y * row + width as usize * 4]);
    }
    for pixel in pixels.chunks_exact_mut(4) {
        if blue_first {
            pixel.swap(0, 2);
        }
        // The alpha of the target is of no use in a picture file.
        pixel[3] = 255;
    }
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path).expect("the picture file")), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&pixels).unwrap();
    println!("picture: {path}");
}

fn option(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

extern "system" {
    fn SetStdHandle(which: u32, handle: *mut core::ffi::c_void) -> i32;
}

/// Sends the text of this program, and the text of a failure, into a file.
fn log_to(path: &str) {
    use std::os::windows::io::IntoRawHandle;
    if let Ok(file) = std::fs::File::create(path) {
        let handle = file.into_raw_handle();
        // STD_OUTPUT_HANDLE and STD_ERROR_HANDLE
        unsafe {
            SetStdHandle(-11i32 as u32, handle as _);
            SetStdHandle(-12i32 as u32, handle as _);
        }
    }
}

/// Runs the renderer as the program `nirender-host` does. `args` are the arguments of the
/// command line, with the name of the program first.
///
/// With `--live <name>` it draws the stream of the shared memory with that name into a
/// window until the stream ends: this is the call for a program that has the renderer as a
/// part, for example the helper of MGE XE. Without it, it draws a frame of a capture file
/// into a picture file.
pub fn run(args: Vec<String>) {
    run_with(args, None)
}

/// `run` with a program that draws more than the game does. The extension works in the
/// live mode only.
pub fn run_with(args: Vec<String>, extension: Option<Box<dyn Extension>>) {
    if let Some(path) = option(&args, "--log") {
        log_to(&path);
    }
    let backends = match option(&args, "--backend").as_deref() {
        Some("dx12") => wgpu::Backends::DX12,
        Some("vulkan") => wgpu::Backends::VULKAN,
        _ => wgpu::Backends::PRIMARY,
    };
    if let Some(name) = option(&args, "--live") {
        return live::run(&name, backends, &args, extension);
    }
    if args.len() < 3 {
        eprintln!("use: nirender-host <capture file> <picture.png> [--backend vulkan|dx12] [--frame N]");
        eprintln!("     nirender-host --live <name of the shared memory> [--backend vulkan|dx12] [--frames N] [--shot picture.png]");
        std::process::exit(2);
    }
    let file = std::fs::read(&args[1]).expect("the capture file");
    assert_eq!(&file[..8], b"NIRCAP01", "not a capture file");
    let mut records = Vec::new();
    let mut at = 8;
    while at + 8 <= file.len() {
        let (tag, length) = (word(&file, at), word(&file, at + 4) as usize);
        records.push((tag, &file[at + 8..at + 8 + length]));
        at += 8 + length;
    }
    let frames = records.iter().filter(|(tag, _)| *tag == tag::FRAME_END).count();
    let wanted = option(&args, "--frame").and_then(|text| text.parse().ok()).unwrap_or(frames.saturating_sub(1));
    // The size of the back buffer: the first record, or the largest viewport of an old capture.
    let (mut width, mut height) = (0u32, 0u32);
    for (tag, data) in &records {
        if *tag == tag::HEADER {
            (width, height) = (word(data, 0), word(data, 4));
            break;
        }
        if *tag == tag::CALL && word(data, 0) == CALL_VIEWPORT {
            let viewport = &data[16 + word(data, 12) as usize * 4..];
            if viewport.len() >= 24 {
                width = width.max(word(viewport, 0) + word(viewport, 8));
                height = height.max(word(viewport, 4) + word(viewport, 12));
            }
        }
    }
    if width == 0 || height == 0 {
        (width, height) = (1280, 720);
    }
    println!("{} records, {frames} frames, frame {wanted} into {width} x {height}", records.len());

    let mut player = Player::new(Renderer::new(backends, None, wgpu::TextureFormat::Rgba8Unorm, None).0);
    player.renderer.anisotropy = option(&args, "--anisotropy").and_then(|text| text.parse().ok()).unwrap_or(1);
    let device = player.renderer.device.clone();
    let target = target_texture(&device, width, height, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
    let depth = target_texture(&device, width, height, DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT);
    let mut frame = 0;
    for (tag, data) in &records {
        // Each frame up to the wanted one is drawn: the game can draw into a rendered texture
        // in one frame and use it in the frames after that.
        if frame <= wanted && !player.drawing() {
            player.begin_frame(&target, Rc::new(depth.create_view(&Default::default())), false);
        }
        if player.feed(*tag, data) {
            if player.drawing() {
                let mut read = None;
                player.end_frame(|encoder| {
                    if frame == wanted {
                        read = Some(copy_picture(&device, encoder, &target, width, height));
                    }
                });
                if let Some((buffer, row)) = read {
                    save_picture(&device, &buffer, row, width, height, false, &args[2]);
                }
            }
            frame += 1;
        }
    }
    player.print_counts();
}

/// The live mode: the records come from a ring in shared memory while the game runs, and
/// the frames go into a window.
mod live {
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use winit::application::ApplicationHandler;
    use winit::dpi::{PhysicalPosition, PhysicalSize};
    use winit::platform::windows::WindowExtWindows;
    use winit::raw_window_handle::{RawWindowHandle, Win32WindowHandle};
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
    use winit::window::{Window, WindowId};

    use super::{copy_picture, option, save_picture, target_texture, Player, Renderer, DEPTH_FORMAT};

    extern "system" {
        fn OpenFileMappingA(access: u32, inherit: i32, name: *const u8) -> isize;
        fn MapViewOfFile(mapping: isize, access: u32, offset_high: u32, offset_low: u32, bytes: usize) -> *mut u8;
        fn IsWindow(window: isize) -> i32;
    }

    /// The head of the shared memory. See `capture.rs` of the plugin.
    const RING_MAGIC: u32 = 0x474E_5231;
    const RING_HEAD: usize = 64;
    const RING_WRITTEN: usize = 8;
    const RING_READ: usize = 16;
    const RING_CLOSED: usize = 24;

    struct Ring {
        base: *mut u8,
        capacity: u64,
    }

    impl Ring {
        /// Opens the ring that the plugin made. Waits for it for some seconds.
        fn open(name: &str) -> Ring {
            let mut text = name.as_bytes().to_vec();
            text.push(0);
            let start = Instant::now();
            loop {
                // FILE_MAP_ALL_ACCESS
                let mapping = unsafe { OpenFileMappingA(0xF001F, 0, text.as_ptr()) };
                if mapping != 0 {
                    let base = unsafe { MapViewOfFile(mapping, 0xF001F, 0, 0, 0) };
                    assert!(!base.is_null(), "the shared memory does not map");
                    assert_eq!(unsafe { (base as *const u32).read() }, RING_MAGIC, "not a ring of the plugin");
                    let capacity = unsafe { (base.add(4) as *const u32).read() } as u64;
                    return Ring { base, capacity };
                }
                assert!(start.elapsed() < Duration::from_secs(60), "no shared memory with the name {name}");
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        fn counter(&self, offset: usize) -> &AtomicU64 {
            unsafe { &*(self.base.add(offset) as *const AtomicU64) }
        }

        fn closed(&self) -> bool {
            unsafe { (self.base.add(RING_CLOSED) as *const u32).read_volatile() != 0 }
        }

        fn copy_out(&self, at: u64, to: &mut [u8]) {
            let start = (at % self.capacity) as usize;
            let first = to.len().min(self.capacity as usize - start);
            unsafe {
                std::ptr::copy_nonoverlapping(self.base.add(RING_HEAD + start), to.as_mut_ptr(), first);
                std::ptr::copy_nonoverlapping(self.base.add(RING_HEAD), to.as_mut_ptr().add(first), to.len() - first);
            }
        }

        /// Takes the next record out of the ring into `data`. Returns its tag, or `None` when
        /// the ring has no complete record.
        fn pop(&self, data: &mut Vec<u8>) -> Option<u32> {
            let read = self.counter(RING_READ).load(Ordering::Relaxed);
            let available = self.counter(RING_WRITTEN).load(Ordering::Acquire) - read;
            if available < 8 {
                return None;
            }
            let mut head = [0u8; 8];
            self.copy_out(read, &mut head);
            let tag = u32::from_le_bytes(head[0..4].try_into().unwrap());
            let length = u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize;
            data.resize(length, 0);
            self.copy_out(read + 8, data);
            self.counter(RING_READ).store(read + 8 + length as u64, Ordering::Release);
            Some(tag)
        }
    }

    /// The reply memory. See `capture.rs` of the plugin.
    const REPLY_HEAD: usize = 16;

    struct Reply {
        base: *mut u8,
    }

    impl Reply {
        fn open(name: &str) -> Option<Reply> {
            let mut text = format!("{name}.reply").into_bytes();
            text.push(0);
            let mapping = unsafe { OpenFileMappingA(0xF001F, 0, text.as_ptr()) };
            if mapping == 0 {
                return None;
            }
            let base = unsafe { MapViewOfFile(mapping, 0xF001F, 0, 0, 0) };
            (!base.is_null()).then_some(Reply { base })
        }

        /// The game made the memory large enough for its back buffer and its textures.
        fn answer(&self, request: u32, width: u32, height: u32, pixels: &[u8]) {
            unsafe {
                std::ptr::copy_nonoverlapping(pixels.as_ptr(), self.base.add(REPLY_HEAD), pixels.len());
                (self.base.add(4) as *mut u32).write_volatile(width);
                (self.base.add(8) as *mut u32).write_volatile(height);
                (*(self.base as *const std::sync::atomic::AtomicU32)).store(request, Ordering::Release);
            }
        }
    }

    struct App {
        ring: Ring,
        reply: Option<Reply>,
        extension: Option<Box<dyn super::Extension>>,
        backends: wgpu::Backends,
        frames_wanted: u32,
        /// The size of the back buffer of the game, and its window when the host goes into it.
        size: (u32, u32),
        parent: Option<isize>,
        shot: Option<String>,
        shot_frame: u32,
        /// Frames to save a picture of, and the start of the names of the files.
        shots_at: Vec<u32>,
        shots_prefix: String,
        window: Option<Arc<Window>>,
        surface: Option<wgpu::Surface<'static>>,
        configuration: Option<wgpu::SurfaceConfiguration>,
        player: Option<Player>,
        depth: Option<Rc<wgpu::TextureView>>,
        /// The back buffer of the game: the frames go into it, and then into the window. It
        /// keeps the last frame for a pixel read of the game.
        back_buffer: Option<wgpu::Texture>,
        /// A frame of the game is in progress.
        in_frame: bool,
        /// The frames that the window did not take: it was hidden or had no size.
        not_shown: u32,
        data: Vec<u8>,
        frames: u32,
        first_frame: Option<Instant>,
        waited: Duration,
        records: u64,
        bytes: u64,
        shot_done: bool,
        finished: bool,
    }

    impl App {
        /// Sets the size of the picture in the window, and makes the depth buffer for it.
        fn configure(&mut self, width: u32, height: u32) {
            let (Some(surface), Some(configuration), Some(player)) = (self.surface.as_ref(), self.configuration.as_mut(), self.player.as_ref()) else { return };
            if width == 0 || height == 0 || (configuration.width, configuration.height) == (width, height) && self.depth.is_some() {
                return;
            }
            configuration.width = width;
            configuration.height = height;
            surface.configure(&player.renderer.device, configuration);
            // The back buffer has the size of the picture of the game.
            let (width, height) = self.size;
            let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
            self.back_buffer = Some(target_texture(&player.renderer.device, width, height, player.renderer.target_format, usage));
            // An extension can read the depth in a pass that does not have it as a target.
            let depth = target_texture(&player.renderer.device, width, height, DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
            self.depth = Some(Rc::new(depth.create_view(&Default::default())));
        }

        fn finish(&mut self, event_loop: &ActiveEventLoop, why: &str) {
            self.finished = true;
            // The picture of the window that is in progress goes away before the surface.
            if let Some(player) = self.player.as_mut() {
                player.end_frame(|_| {});
            }
            self.in_frame = false;
            let seconds = self.first_frame.map_or(0.0, |start| start.elapsed().as_secs_f64());
            println!("end: {why}");
            println!(
                "frames {}, {:.1} frames in a second, {:.2} ms for each frame; the host waited for the game {:.0} % of the time",
                self.frames,
                self.frames as f64 / seconds.max(0.001),
                seconds * 1000.0 / self.frames.max(1) as f64,
                self.waited.as_secs_f64() * 100.0 / seconds.max(0.001)
            );
            println!("frames that the window did not take: {}", self.not_shown);
            if let Some(player) = self.player.as_ref() {
                println!("time for each frame, in ms:");
                for (name, time) in super::TIME_NAMES.iter().zip(player.renderer.counts.times) {
                    println!("  {:6.3}  {name}", time.as_secs_f64() * 1000.0 / self.frames.max(1) as f64);
                }
            }
            println!("records {}, {:.1} MB, {:.2} MB for each frame", self.records, self.bytes as f64 / 1e6, self.bytes as f64 / 1e6 / self.frames.max(1) as f64);
            if let Some(player) = self.player.as_ref() {
                player.print_counts();
            }
            event_loop.exit();
        }
    }

    impl ApplicationHandler for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_some() {
                return;
            }
            let mut attributes =
                Window::default_attributes().with_title("nirender host").with_inner_size(PhysicalSize::new(self.size.0, self.size.1)).with_resizable(false).with_active(false);
            if let Some(parent) = self.parent.and_then(std::num::NonZeroIsize::new) {
                // A child window of the window of the game, over all of it.
                let handle = RawWindowHandle::Win32(Win32WindowHandle::new(parent));
                attributes = unsafe { attributes.with_parent_window(Some(handle)) }.with_decorations(false).with_position(PhysicalPosition::new(0, 0));
            }
            let window = Arc::new(event_loop.create_window(attributes).expect("a window"));
            if self.parent.is_some() {
                // A window that is not enabled takes no mouse and no keys: they go to the game.
                window.set_enable(false);
            }
            let (renderer, surface) = Renderer::new(self.backends, Some(window.clone()), wgpu::TextureFormat::Bgra8Unorm, self.extension.as_ref().and_then(|extension| extension.sky_shader()));
            let surface = surface.expect("a surface");
            let size = window.inner_size();
            self.configuration = Some(wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
                format: renderer.target_format,
                width: 0,
                height: 0,
                // The host shows a frame when the game gives one. It does not wait for the screen.
                present_mode: wgpu::PresentMode::AutoNoVsync,
                desired_maximum_frame_latency: 2,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: vec![],
                color_space: Default::default(),
            });
            assert!(renderer.surface_usages.contains(wgpu::TextureUsages::COPY_DST), "the window does not take a copy of a picture");
            let mut player = Player::new(renderer);
            player.extension = self.extension.take();
            player.reply = self.reply.take().map(|reply| Box::new(move |request, width, height, pixels: &[u8]| reply.answer(request, width, height, pixels)) as _);
            self.player = Some(player);
            self.surface = Some(surface);
            self.window = Some(window);
            self.configure(size.width, size.height);
        }

        fn window_event(&mut self, event_loop: &ActiveEventLoop, _window: WindowId, event: WindowEvent) {
            match event {
                WindowEvent::CloseRequested if !self.finished => self.finish(event_loop, "the window closed"),
                WindowEvent::Resized(size) => self.configure(size.width, size.height),
                _ => {}
            }
        }

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            event_loop.set_control_flow(ControlFlow::Poll);
            if self.player.is_none() || self.finished {
                return;
            }
            // The window of the game went away: the game ended, and the window of the host
            // with it.
            if self.parent.is_some_and(|parent| unsafe { IsWindow(parent) } == 0) {
                return self.finish(event_loop, "the window of the game is gone");
            }
            let size = self.size;
            let now = self.window.as_ref().unwrap().inner_size();
            self.configure(now.width, now.height);
            let waiting = Instant::now();
            loop {
                // No frame before the size of the game is known: the records before that
                // make the resources only.
                // The frame goes into the back buffer of this program. The window is not a
                // part of it: this program reads the stream also when the window is hidden,
                // because the game waits for it.
                if !self.in_frame && self.depth.is_some() && size.0 != 0 {
                    self.player.as_mut().unwrap().begin_frame(self.back_buffer.as_ref().unwrap(), self.depth.clone().unwrap(), true);
                    self.in_frame = true;
                }
                let clock = Instant::now();
                let popped = self.ring.pop(&mut self.data);
                if popped.is_some() {
                    self.player.as_mut().unwrap().renderer.counts.times[super::TIME_READ] += clock.elapsed();
                }
                let Some(tag) = popped else {
                    if self.ring.closed() {
                        return self.finish(event_loop, "the game closed the stream");
                    }
                    if waiting.elapsed() > Duration::from_millis(20) {
                        // Back to the events of the window.
                        self.waited += waiting.elapsed();
                        return;
                    }
                    std::thread::yield_now();
                    continue;
                };
                self.records += 1;
                self.bytes += 8 + self.data.len() as u64;
                let player = self.player.as_mut().unwrap();
                let clock = Instant::now();
                let draws_before: std::time::Duration = player.renderer.counts.times[super::TIME_DRAW_DATA..=super::TIME_DRAW_PASS].iter().sum();
                let frame_end = player.feed(tag, &self.data);
                let draws_after: std::time::Duration = player.renderer.counts.times[super::TIME_DRAW_DATA..=super::TIME_DRAW_PASS].iter().sum();
                // The time of a record without the measured parts of a draw.
                player.renderer.counts.times[super::TIME_RECORDS] += clock.elapsed().saturating_sub(draws_after - draws_before);
                if !frame_end {
                    continue;
                }
                // The frame is complete. It goes into the window when the window takes one.
                if !std::mem::take(&mut self.in_frame) {
                    return;
                }
                let frame = match self.surface.as_ref().unwrap().get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Some(frame),
                    _ => None,
                };
                self.not_shown += frame.is_none() as u32;
                let blue_first = matches!(player.renderer.target_format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
                let take_shot = self.shot.is_some() && !self.shot_done && self.frames + 1 >= self.shot_frame;
                let back_buffer = self.back_buffer.as_ref().unwrap();
                let (width, height) = (back_buffer.width(), back_buffer.height());
                let device = player.renderer.device.clone();
                let mut read = None;
                player.end_frame(|encoder| {
                    // The picture goes into the window: the part that the two have.
                    if let Some(frame) = frame.as_ref() {
                        encoder.copy_texture_to_texture(
                            back_buffer.as_image_copy(),
                            frame.texture.as_image_copy(),
                            wgpu::Extent3d { width: width.min(frame.texture.width()), height: height.min(frame.texture.height()), depth_or_array_layers: 1 },
                        );
                    }
                    if take_shot {
                        read = Some(copy_picture(&device, encoder, back_buffer, width, height));
                    }
                });
                if let Some((buffer, row)) = read {
                    save_picture(&device, &buffer, row, width, height, blue_first, self.shot.as_ref().unwrap());
                    self.shot_done = true;
                }
                // The picture files that the game asked for in this frame, and the one that
                // the command line asked for.
                let mut shots = std::mem::take(&mut player.shots);
                if self.shots_at.contains(&(self.frames + 1)) {
                    shots.push(format!("{}-{:05}.png", self.shots_prefix, self.frames + 1));
                }
                if !shots.is_empty() {
                    let mut encoder = device.create_command_encoder(&Default::default());
                    let (buffer, row) = copy_picture(&device, &mut encoder, back_buffer, width, height);
                    player.renderer.queue.submit([encoder.finish()]);
                    for path in &shots {
                        save_picture(&device, &buffer, row, width, height, blue_first, path);
                        buffer.unmap();
                    }
                }
                let clock = Instant::now();
                if let Some(frame) = frame {
                    player.renderer.queue.present(frame);
                }
                player.renderer.counts.times[super::TIME_PRESENT] += clock.elapsed();
                self.frames += 1;
                self.first_frame.get_or_insert_with(Instant::now);
                if self.frames_wanted != 0 && self.frames >= self.frames_wanted {
                    return self.finish(event_loop, "all the frames that were asked for");
                }
                return;
            }
        }
    }

    pub fn run(name: &str, backends: wgpu::Backends, args: &[String], extension: Option<Box<dyn super::Extension>>) {
        let ring = Ring::open(name);
        println!("ring: {} MB", ring.capacity >> 20);
        // The first record has the size of the picture and the window of the game.
        let mut data = Vec::new();
        let start = Instant::now();
        let tag = loop {
            if let Some(tag) = ring.pop(&mut data) {
                break tag;
            }
            assert!(start.elapsed() < Duration::from_secs(60) && !ring.closed(), "no record in the ring");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(tag, super::tag::HEADER, "the first record is not the head of the stream");
        let size = (super::word(&data, 0).max(64), super::word(&data, 4).max(64));
        let game_window = if data.len() >= 12 { super::word(&data, 8) as isize } else { 0 };
        let embed = args.iter().any(|a| a == "--embed");
        println!("picture {} x {}, window of the game {game_window:#x}{}", size.0, size.1, if embed { ", the host goes into it" } else { "" });
        let mut app = App {
            ring,
            reply: Reply::open(name),
            extension,
            back_buffer: None,
            backends,
            frames_wanted: option(args, "--frames").and_then(|text| text.parse().ok()).unwrap_or(0),
            size,
            parent: embed.then_some(game_window),
            shot: option(args, "--shot"),
            shot_frame: option(args, "--shot-frame").and_then(|text| text.parse().ok()).unwrap_or(60),
            shots_at: option(args, "--shots-at").map(|text| text.split(',').filter_map(|n| n.parse().ok()).collect()).unwrap_or_default(),
            shots_prefix: option(args, "--shots-prefix").unwrap_or_else(|| "frame".into()),
            window: None,
            surface: None,
            configuration: None,
            player: None,
            depth: None,
            in_frame: false,
            not_shown: 0,
            data: Vec::new(),
            frames: 0,
            first_frame: None,
            waited: Duration::ZERO,
            records: 0,
            bytes: 0,
            shot_done: false,
            finished: false,
        };
        EventLoop::new().expect("an event loop").run_app(&mut app).expect("the event loop");
    }
}
