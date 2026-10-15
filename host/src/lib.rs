//! Draws a capture of the render link stream with wgpu, without the game.
//!
//! ```text
//! nirender-host <capture file> <picture.png> [--backend vulkan|dx12] [--frame N] [--anisotropy N]
//! nirender-host --live <name of the shared memory> [--backend vulkan|dx12] [--frames N] [--shot picture.png]
//!               [--embed]
//! ```
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
    /// The fill mode of the device of the game is wireframe at this call: the draw of the
    /// game that is current, or the last one, has the wireframe property (the console
    /// command ToggleWireframe puts the shared wireframe property of the engine on every
    /// mesh). MGE XE on Direct3D 9 draws with the render state the engine left, so its
    /// distant land, statics, grass and water are lines then.
    pub wireframe: bool,
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
    /// For the calls of `ReadingCall`: the picture of the frame as it is before the call (a
    /// copy), and the depth of the draws of the game so far (in `blend`, the depth comes as
    /// an argument), when `Extension::reads` says that the call reads them. `None` in the
    /// other calls.
    pub picture: Option<&'a wgpu::TextureView>,
    pub depth: Option<&'a wgpu::TextureView>,
    /// For `Extension::post`, `Extension::draw_class` and `Extension::draw_held`: the depth
    /// of what `Extension::before_world` drew, when it gave true in this frame (the renderer
    /// then clears the depth for the game). It has the projection of the extension, not that
    /// of the game.
    pub before_depth: Option<&'a wgpu::TextureView>,
    /// For `Extension::draw_held`: the lights of the frame, when the extension asked for them
    /// (`FrameParameters::lights`) and the frame has a view of the world with lights.
    pub lights: Option<FrameLights<'a>>,
}

/// A picture file that an extension wants of a frame. See `Extension::pictures`.
pub struct PictureRequest {
    /// The file. Its extension gives the format: `.png`, `.bmp`, `.tga`, `.dds`, `.jpg`;
    /// PNG for another one.
    pub path: String,
    /// With the interface of the game (the picture at the end of the frame), or without it
    /// (the picture after `Extension::post`).
    pub with_ui: bool,
}

/// The answers of an extension for a frame. See `Extension::frame`.
#[derive(Clone, Default)]
pub struct FrameParameters {
    /// The values of the extension for the frame, for its shader functions: `frame.values`
    /// in WGSL (bind group 3, binding 0).
    pub values: [[f32; 4]; 32],
    /// A texture of the extension for its shader functions: `extension_texture` with
    /// `extension_sampler` (linear, clamp) in WGSL. White without one. The view must stay
    /// valid for the frame; the extension draws into the texture before the draws of the
    /// game read it (in `before_world`, in a pass of its own).
    pub texture: Option<Rc<wgpu::TextureView>>,
    /// The distances of the start and of the end of the fog for the draws of the world of
    /// the game, in place of those of the game: so that they go into the fog as what the
    /// extension draws does. `None`: the fog of the game.
    pub fog_range: Option<(f32, f32)>,
    /// True when the extension shades the draws of the world: the renderer calls
    /// `Extension::shade` for each of them, and its shader uses the functions of
    /// `Extension::draw_shader` for a draw that the call marks.
    pub shade: bool,
    /// The values for the sky functions of `Extension::sky_shader`, or `None` for the sky
    /// of the game as it is. With values, the sky dome of the game (a sky draw without a
    /// texture) gets its shape and its colour from the functions, and the shadow of a moon
    /// gets its colour from them and keeps its alpha.
    pub sky: Option<[[f32; 4]; 8]>,
    /// True when the extension wants the call `Extension::blend` in this frame.
    pub blend: bool,
    /// The classes of the game (MGE_LINK_CLASS_*, one bit for each) whose draws into the
    /// back buffer the extension takes: see `Extension::draw_class`.
    pub watched: u32,
    /// True when the extension wants the call `Extension::post` in this frame.
    pub post: bool,
    /// A class in `watched` and a plane for its call, `[a, b, c, d]` in the space of the
    /// world of the game, with the camera on the side where `a * x + b * y + c * z + d >= 0`:
    /// a surface of water. The world draws that blend and write no depth (particles, glows,
    /// smoke) go into the picture on the other side of the plane before the call (so that the
    /// picture of the call has them, for a refraction), and on the side of the camera after
    /// the call, over what the call drew: each of them, whether the game sends it before or
    /// after the call. `None`: the draws that blend come in their place in the order of the
    /// game.
    pub class_plane: Option<(u32, [f64; 4])>,
    /// True when the extension takes draws of the world: the renderer calls `Extension::hold`
    /// for each shape of the world, and gives the shapes it took to `Extension::draw_held`.
    pub hold: bool,
    /// True when the extension lights what it draws with the lights of the frame, for each
    /// pixel: `SceneView::lights` in `Extension::draw_held`.
    pub lights: bool,
}

/// The lights of the world of a frame, for an extension that lights what it draws as the
/// draws of the game are lit (`FrameParameters::lights`). The lights are those of the draws
/// after `Extension::shade`, one copy of each light, binned into clusters of their own: tiles
/// of the picture by slices of the view depth of the game.
///
/// - `lights`: an array of `LightData` (storage). A position is relative to `origin`.
/// - `clusters` (uniform, 48 bytes): `grid: vec4u` (tiles across, tiles down, slices; w 2),
///   `screen: vec4f` (x and y: tiles for each pixel), `depth: vec4f` (the slice of a view
///   depth z is `log(max(z, depth.z)) * depth.x + depth.y`, clamped to the slices).
/// - `table` (storage, `array<vec2u>`): for the cluster `(slice * grid.y + y) * grid.x + x`,
///   the first entry in `indexes` and the number of entries.
/// - `indexes` (storage, `array<u32>`): the index of a light in `lights`.
///
/// A light is in each cluster that its reach touches: the cut-off of `shade` when
/// `cone.x` is more than 0 (`cone.x` is 1 over it), else the radius of the light of the game
/// (`cone.z`), else its range (`direction.w`), at most `LIGHT_REACH_MAX`. A directional light
/// is in each cluster.
pub struct FrameLights<'a> {
    pub lights: &'a wgpu::Buffer,
    pub clusters: &'a wgpu::Buffer,
    pub table: &'a wgpu::Buffer,
    pub indexes: &'a wgpu::Buffer,
    /// The point of the world that the positions of the lights are relative to.
    pub origin: [f64; 3],
}

/// A call of the extension that can read the picture of the frame and the depth of the game
/// (`SceneView::picture`, `SceneView::depth`). See `Extension::reads`.
#[derive(Clone, Copy)]
pub enum ReadingCall<'a> {
    /// `Extension::blend`: only the picture counts (its depth is an argument).
    Blend,
    /// `Extension::draw_class` for this class.
    Class(u32),
    /// `Extension::draw_held` with these shapes.
    Held(&'a [HeldDraw]),
    /// `Extension::draw_held_in_place` with this shape.
    HeldInPlace(&'a HeldDraw),
    /// `Extension::post`.
    Post,
}

/// What a call of the extension reads. A pass cannot read its own targets, so the renderer
/// gives a copy of each; it makes only the copies that the call reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Reads {
    pub picture: bool,
    pub depth: bool,
}

impl Reads {
    pub const ALL: Reads = Reads { picture: true, depth: true };
    pub const NONE: Reads = Reads { picture: false, depth: false };
    pub const DEPTH: Reads = Reads { picture: false, depth: true };
}

/// The most reach of a light in the clusters: one cell of the game.
pub const LIGHT_REACH_MAX: f32 = 8192.0;

/// The reach of a point or spot light in the clusters: see `FrameLights`.
pub fn light_reach(light: &LightData) -> f32 {
    let reach = if light.cone[0] > 0.0 {
        1.0 / light.cone[0]
    } else if light.cone[2] > 0.0 {
        light.cone[2]
    } else {
        light.direction[3]
    };
    reach.min(LIGHT_REACH_MAX).max(0.0)
}

/// The facts of the world scene of the game at a moment, for a call of the extension.
struct WorldFacts {
    target_size: (u32, u32),
    view: [f32; 16],
    projection: [f32; 16],
    interior: bool,
    wireframe: bool,
    fog_color: [f32; 4],
    fog_start: f32,
    fog_end: f32,
    sun: Option<([f32; 3], [f32; 3], [f32; 3])>,
    ambient: [f32; 3],
}

impl WorldFacts {
    fn view<'a>(&self, device: &'a wgpu::Device, queue: &'a wgpu::Queue, target_format: wgpu::TextureFormat) -> SceneView<'a> {
        SceneView {
            device,
            queue,
            target_format,
            depth_format: DEPTH_FORMAT,
            target_size: self.target_size,
            view: self.view,
            projection: self.projection,
            interior: self.interior,
            wireframe: self.wireframe,
            fog_color: self.fog_color,
            fog_start: self.fog_start,
            fog_end: self.fog_end,
            sun_direction: self.sun.map(|sun| sun.0),
            sun_color: self.sun.map_or([0.0; 3], |sun| sun.1),
            sun_ambient: self.sun.map_or([0.0; 3], |sun| sun.2),
            ambient: self.ambient,
            picture: None,
            depth: None,
            before_depth: None,
            lights: None,
        }
    }
}

/// A draw with its vertices in the record, ready for the pass.
struct ScreenDraw {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    buffer: wgpu::Buffer,
    vertices: u32,
}

/// What a frame has, in its order. `feed` collects the items and `run_items` does them.
enum Item {
    /// The game draws into this rendered texture, or into the back buffer.
    Target(Option<u32>),
    Clear(Option<wgpu::Color>, Option<f32>, Option<u32>),
    Viewport(([f32; 4], [f32; 2])),
    Draw(Rc<Draw>),
    Screen(ScreenDraw),
    WorldStart(Box<WorldFacts>),
    WorldEnd(Box<WorldFacts>),
    /// The extension draws in place of the draws of this class. The number of world draws
    /// before this item: those after it are not in the picture yet.
    Replace(u32, Box<WorldFacts>, usize),
    /// The post-processing of the extension, after the world views.
    Post(Box<WorldFacts>),
    /// A held shape that the extension draws in its place in the order of the game
    /// (`Extension::draw_held_in_place`), with the facts of the world there.
    HeldHere(Box<HeldDraw>, Box<WorldFacts>),
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
    /// Only the draws that blend and do not write depth: those that a surface drawn after
    /// them hides, whatever is nearer.
    pub translucent_only: bool,
    /// The depth test is on for each draw (less or equal), whatever the draw has: the sky
    /// of the game has it off, and in a reflection it must stay behind the land.
    pub force_depth_test: bool,
    /// The classes of the game to draw again, one bit for each (MGE_LINK_CLASS_*): the
    /// world draws of a frame have the sky too (classes 1 to 7). `ReplayView::WORLD` is
    /// all but the sky.
    pub classes: u32,
}

impl ReplayView {
    /// All classes but the sky (1 to 7) and the glare of the sun (8).
    pub const WORLD: u32 = !0x1FE;
    pub const SKY: u32 = 0xFE;
}

/// Draws the world draws of the frame again: those of the game in its world scene, its
/// scene of sorted draws and its scene of the water, into the back buffer, with the sky and
/// without the water plane of the game. The pass must have a colour target and a depth
/// target with the formats of `SceneView`.
pub trait Replay {
    fn replay(&mut self, pass: &mut wgpu::RenderPass<'static>, view: &ReplayView);
    /// The number of kept draws.
    fn count(&self) -> usize;
}

/// The most instances of one draw: the buffer of defaults has that many.
const INSTANCES_MOST: usize = 1024;

/// One draw of the game, ready for the pass.
#[derive(Clone)]
struct Draw {
    /// The class of the object of the draw: MGE_LINK_CLASS_*.
    class: u32,
    /// A draw of the world of the game into the back buffer: one of the draws of `Replay`.
    world: bool,
    key: PipelineKey,
    stencil_reference: u32,
    data: DrawData,
    /// The origin that the world matrices of the draw are relative to, or `None`.
    origin: Option<[f64; 3]>,
    textures: Rc<wgpu::BindGroup>,
    samplers: Rc<wgpu::BindGroup>,
    vertices: Rc<wgpu::Buffer>,
    indexes: Rc<wgpu::Buffer>,
    /// The handles of the textures of the stages and of the geometry record, as the stream has
    /// them. The sort of the opaque draws orders by these and not by the addresses of the
    /// buffers, so a replay of a capture gives the same order in each build.
    texture_handles: [u32; STAGES],
    geometry: u32,
    /// The first index and the number of indexes.
    range: (u32, u32),
    /// The instances: the first in the instance buffer of the frame and the number. (0, 1)
    /// for a draw of its own: its world matrix is in its data.
    instances: (u32, u32),
}

impl Draw {
    /// The draw blends and writes no depth: a surface drawn after it hides it.
    fn translucent(&self) -> bool {
        self.key.blend[0] != 0 && self.key.depth[1] == 0
    }
}

/// The kept draws with the renderer, for `Replay`.
struct Replayer<'a> {
    renderer: &'a mut Renderer,
    kept: &'a [Rc<Draw>],
}

/// A plane in the space of the world of the game, in the space of a draw whose world is
/// relative to `origin`.
fn plane_of_draw(plane: [f64; 4], origin: Option<[f64; 3]>) -> [f32; 4] {
    let origin = origin.unwrap_or([0.0; 3]);
    let d = plane[3] + plane[0] * origin[0] + plane[1] * origin[1] + plane[2] * origin[2];
    [plane[0] as f32, plane[1] as f32, plane[2] as f32, d as f32]
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
            if view.classes & (1 << kept.class.min(31)) == 0 || (view.opaque_only && (kept.key.blend[0] != 0 || kept.key.depth[1] == 0)) || (view.translucent_only && !kept.translucent()) {
                continue;
            }
            let mut data = kept.data;
            let origin = kept.origin.unwrap_or([0.0; 3]);
            // The clusters are those of the view of the world: not for another view.
            data.light_range[3] = 0;
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
                data.clip_plane = plane_of_draw(plane, kept.origin);
            }
            let mut key = kept.key;
            if view.force_depth_test {
                // D3DCMP_LESSEQUAL is 4.
                key.depth = [1, key.depth[1], 4];
            }
            if view.flip_cull {
                // D3DCULL_CW is 2 and D3DCULL_CCW is 3.
                key.cull = match key.cull {
                    2 => 3,
                    3 => 2,
                    other => other,
                };
            }
            let Some(pipeline) = self.renderer.pipeline(key) else { continue };
            let (group, offset) = self.renderer.draw_slot_now(&data);
            pass.set_pipeline(&pipeline);
            pass.set_stencil_reference(kept.stencil_reference);
            pass.set_bind_group(0, &*group, &[offset]);
            pass.set_bind_group(1, &*kept.textures, &[]);
            pass.set_bind_group(2, &*kept.samplers, &[]);
            pass.set_bind_group(3, &*self.renderer.frame_group, &[]);
            pass.set_vertex_buffer(0, kept.vertices.slice(..));
            pass.set_vertex_buffer(1, self.renderer.defaults.slice(..));
            pass.set_vertex_buffer(2, self.renderer.instance_buffer.slice(kept.instances.0 as u64 * 64..));
            let (first, count) = kept.range;
            let instances = 0..kept.instances.1;
            // `MGE_XE_REPLAY_DEBUG=1`: each draw that a replay issues, for a comparison of
            // the two paths of the shapes.
            if std::env::var("MGE_XE_REPLAY_DEBUG").is_ok_and(|v| v == "1") {
                println!(
                    "replay class {} blend {:?} depth {:?} cull {} fill {} origin {:?} range {:?} instances {:?} world {:?} view {:?} flags {:?} light_range {:?}",
                    kept.class, key.blend, key.depth, key.cull, key.fill, kept.origin, kept.range, kept.instances, data.world[0], &data.view[12..16], data.flags, data.light_range
                );
            }
            pass.set_index_buffer(kept.indexes.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(first..first + count, 0, instances);
            self.renderer.counts.replayed += 1;
        }
    }
}

/// The facts of a draw that are not in its uniform, for `Extension::shade` and
/// `Extension::hold`.
#[derive(Clone, Copy, Debug)]
pub struct DrawFacts {
    /// The class of the object of the draw: MGE_LINK_CLASS_*.
    pub class: u32,
    /// The alpha blend of the draw: on, and the two factors as Direct3D 8 numbers them
    /// (D3DBLEND_ONE is 2).
    pub blend_enable: bool,
    pub source_blend: u32,
    pub destination_blend: u32,
    /// The draw writes depth.
    pub depth_write: bool,
    /// The specular power of the material (`D3DMATERIAL8::Power`). A mod marks a surface of
    /// water with it.
    pub specular_power: f32,
    /// The emissive colour of the material, with its alpha.
    pub emissive: [f32; 4],
}

/// Where the parts of a vertex of a draw of the game are: offsets in bytes from the start of
/// the vertex, as the renderer reads the flexible vertex format of Direct3D 8.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VertexLayout {
    /// The bytes from one vertex to the next.
    pub stride: u64,
    /// Three floats.
    pub position: u64,
    /// Three floats, or `None`.
    pub normal: Option<u64>,
    /// A D3DCOLOR (`Unorm8x4`: blue, green, red, alpha), or `None`.
    pub color: Option<u64>,
    /// Two floats for each set of texture coordinates (four at most).
    pub coordinates: Vec<u64>,
}

impl VertexLayout {
    /// The layout of a format with no blend weights. `None` for one that the renderer does not
    /// draw, or that has blend weights.
    fn of(fvf: u32, stride: u32) -> Option<VertexLayout> {
        let attributes = attributes(fvf, stride)?;
        if attributes.iter().any(|a| a.location >= 7) {
            return None;
        }
        let at = |location: u32| attributes.iter().find(|a| a.location == location).map(|a| a.offset);
        Some(VertexLayout {
            stride: stride as u64,
            position: at(0)?,
            normal: at(1),
            color: at(2),
            coordinates: (3..7).map_while(at).collect(),
        })
    }
}

/// A map of the material of a held draw, with the texture of the game.
#[derive(Clone)]
pub struct HeldMap {
    /// The map in the texturing property: `material::BASE` to `material::DECAL0 + 1`.
    pub map: usize,
    pub texture: Rc<wgpu::TextureView>,
    /// The set of texture coordinates of the map, in `VertexLayout::coordinates`.
    pub set: u32,
    /// The sampler states: address U, address V, the magnify, minify and mip filters, as
    /// Direct3D 8 has them.
    pub sampler: [u32; 5],
}

/// A shape of the world of the game that the extension took (`Extension::hold`): the renderer
/// draws none of its passes, and the extension draws it in its own way in
/// `Extension::draw_held`. It is never an instance of another draw.
#[derive(Clone)]
pub struct HeldDraw {
    pub facts: DrawFacts,
    pub vertices: Rc<wgpu::Buffer>,
    /// The indexes, 16 bits each.
    pub indexes: Rc<wgpu::Buffer>,
    pub layout: VertexLayout,
    pub topology: wgpu::PrimitiveTopology,
    /// The parts of the shape: the first index and the number of indexes of each. A strip
    /// topology has one part for each strip.
    pub parts: Vec<(u32, u32)>,
    /// The uniform that the renderer made for the first pass of the shape: the world matrix,
    /// the view and the projection of the draw (see `origin`), the material, the fog and the
    /// texture stages. Its `light_range` counts in `lights`.
    pub data: DrawData,
    /// The origin that the world matrix is relative to, or `None`. With an origin, the view
    /// of `data` has the camera at 0.
    pub origin: Option<[f64; 3]>,
    /// The lights that the game lit the shape with, relative to `origin` when there is one.
    pub lights: Vec<LightData>,
    /// The maps of the material that have a texture.
    pub maps: Vec<HeldMap>,
    /// The depth test of the shape: on, write, function (D3DCMPFUNC).
    pub depth: [u32; 3],
    /// D3DCULL: 1 none, 2 clockwise, 3 counter-clockwise.
    pub cull: u32,
    /// The stencil of the shape: on, function, reference, mask, fail, depth fail, pass
    /// (Direct3D values).
    pub stencil: [u32; 7],
}

/// A program that draws more than the game does: for example the distant land of MGE XE.
pub trait Extension {
    /// The name and the version of the extension, for the first line of the log.
    fn name(&self) -> String {
        String::new()
    }

    /// The anisotropic filter for the textures that the game filters, from the settings of
    /// the extension: 1 is no such filter.
    fn anisotropy(&self) -> u16 {
        1
    }

    /// Data that the part of this program in the game process put into the stream, with the
    /// number that names its kind.
    fn data(&mut self, _id: u32, _bytes: &[u8]) {}

    /// Lines of text for the debug overlay, under the numbers of the renderer: for example
    /// the parameters that a person can change while the game runs. The overlay asks in
    /// each frame in which it shows.
    fn overlay_lines(&self) -> Vec<String> {
        Vec::new()
    }

    /// The renderer draws a capture file and not a live stream: the frames must be the
    /// same each time. The extension completes each load before a frame, with no limit on
    /// the work of a frame. The renderer calls this one time, before the first record.
    fn offline(&mut self) {}

    /// WGSL source with the two sky functions of the extension, or `None`. The renderer
    /// asks one time, before it makes its shader, and puts the source at the end of it:
    ///
    /// - `fn extension_sky_position(position: vec3f) -> vec3f`: the position of a vertex of
    ///   the sky dome in its model, for a dome of another shape.
    /// - `fn extension_sky_color(direction: vec3f, pixel: vec2f) -> vec3f`: the colour of
    ///   the sky in a direction from the camera (length 1, in the space of the world), for
    ///   the pixel at these coordinates.
    ///
    /// The source reads the values of `FrameParameters::sky` as `draw.extension[0]` to
    /// `draw.extension[7]`.
    fn sky_shader(&self) -> Option<String> {
        None
    }

    /// WGSL source with the two draw functions of the extension, or `None`. The renderer
    /// puts the source at the end of its shader, after the sky functions. For a draw with
    /// bit 1 of `counts.w` set (see `shade`):
    ///
    /// - `fn extension_shade(in: VertexOut) -> vec4f`: the colour of the pixel before the
    ///   texture stages, in place of the colour that the vertex stage made. `in` has the
    ///   position and the normal in the space of the lights (`world_position`,
    ///   `world_normal`) and the colour of the vertex.
    /// - `fn extension_finish(color: vec3f, alpha: f32, clear: f32) -> vec4f`: the pixel
    ///   after the stages and the alpha test; `clear` is 1 with no fog and 0 in full fog.
    ///
    /// The functions read the uniform of the draw as `draw` (see `DrawData`); the colour of
    /// such a draw is not kept between 0 and 1 after the stages.
    fn draw_shader(&self) -> Option<String> {
        None
    }

    /// Changes the uniform of a world draw and its lights before the renderer puts the draw
    /// into the pass: the lights, the ambient light, the fog. Called for each draw of the
    /// world of the game into the back buffer with a perspective projection, when
    /// `FrameParameters::shade` is true. The extension sets bit 1 of `counts[3]` for a draw
    /// that its shader functions shade; bit 2 and the bits from 16 are its own.
    fn shade(&mut self, _draw: &mut DrawData, _lights: &mut [LightData], _facts: &DrawFacts) {}

    /// The answers of the extension for a frame. The renderer asks at the first draw of the
    /// world scene of the game into the back buffer: before the sky, so the matrices of the
    /// scene are those of that draw and the sun is not known yet. The answers are for each
    /// draw of the frame after that.
    fn frame(&mut self, _scene: &SceneView) -> FrameParameters {
        FrameParameters::default()
    }

    /// The renderer calls this one time in a frame, at the start of the world of the game:
    /// after the sky and before the first draw of the world. The pass draws into the
    /// picture of the frame with the depth of the game; the matrices are those of the first
    /// draw of the world. `replay` draws the world draws of the frame again, for a picture
    /// from another view: the renderer has all of them at this moment, and a draw of the
    /// extension into the pass goes before them in the picture. Returns true when the depth
    /// target must be cleared after the call: the draws of the extension then do not hide
    /// the draws of the game.
    fn before_world(&mut self, scene: &SceneView, pass: &mut wgpu::RenderPass<'static>, replay: &mut dyn Replay) -> bool;

    /// The renderer calls this one time in a frame, at the end of the world scene of the
    /// game: its draws without a sort are in the picture and in the depth target, and the
    /// draws with a sort, the first-person view and the user interface are not. The pass and
    /// the matrices are those of the game. MGE XE draws its grass here.
    fn after_world(&mut self, _scene: &SceneView, _pass: &mut wgpu::RenderPass<'static>, _replay: &mut dyn Replay) {}

    /// The renderer calls this at the first draw of a frame into the back buffer of a class
    /// in `FrameParameters::watched`, in place of the draws of that class: none of them is
    /// drawn. The pass and the matrices are those of that draw. `scene.picture` is the
    /// picture of the frame before the call and `scene.depth` is the depth of the game so
    /// far: for a refraction. `scene.before_depth` is the depth of what `before_world` drew,
    /// when it gave true. MGE XE draws its water in place of the water plane of the game
    /// (class 10).
    fn draw_class(&mut self, _class: u32, _scene: &SceneView, _pass: &mut wgpu::RenderPass<'static>, _replay: &mut dyn Replay) {}

    /// Takes a shape of the world of the game away from the renderer, when
    /// `FrameParameters::hold` is true: none of its passes is drawn, and the extension gets
    /// it in `draw_held`. Called one time for each shape record into the back buffer in the
    /// scenes of the world (with a perspective projection), but not for the sky, the water
    /// plane of the game, a skinned shape, a particle system or lines. `facts` are those of
    /// the first pass.
    fn hold(&mut self, _facts: &DrawFacts) -> bool {
        false
    }

    /// The renderer calls this one time in a frame, at the end of the world scene of the
    /// game, with the shapes that `hold` took in the frame: after `after_world` and `blend`,
    /// before the draws of the game that blend (they come after the world scene) and its
    /// water plane. The pass and the matrices are those of the game; `scene.picture` is the
    /// picture of the frame before the call and `scene.depth` the depth of the game so far:
    /// for a refraction. `scene.before_depth` is the depth of what `before_world` drew, when
    /// it gave true. Called when a shape was held, or when `wants_held` says so.
    fn draw_held(&mut self, _scene: &SceneView, _pass: &mut wgpu::RenderPass<'static>, _held: &[HeldDraw], _replay: &mut dyn Replay) {}

    /// True when the extension draws this held shape in its place in the order of the game,
    /// in `draw_held_in_place`, and not in `draw_held`: for a shape whose look depends on the
    /// draws around it, as one with a stencil test of its own. The engine sets the stencil
    /// state of each shape as it draws it (`NiD3DRenderState::ApplyStencil`), so a mask is
    /// the shapes before it in the order of the scene graph, and a shape after it can draw
    /// over it through the same mask (a well: the mask of the opening, the water, then the
    /// shaft through the opening).
    fn held_in_place(&self, _held: &HeldDraw) -> bool {
        false
    }

    /// The renderer calls this for each held shape that `held_in_place` named, at the place of
    /// the shape in the order of the game: after the draws of the game before it, before
    /// those after it. The pass, `scene.picture` (a copy of the picture there),
    /// `scene.depth` and `scene.lights` are as for `draw_held`.
    fn draw_held_in_place(&mut self, _scene: &SceneView, _pass: &mut wgpu::RenderPass<'static>, _held: &[HeldDraw], _replay: &mut dyn Replay) {}

    /// True when the extension wants its call `draw_held` in this frame without a held shape,
    /// for draws of its own at that place in the frame: MGE XE draws the water among its
    /// distant statics there. Asked at the end of the world scene, after `before_world`.
    fn wants_held(&self) -> bool {
        false
    }

    /// The renderer calls this after the world views of the game (at the start of its user
    /// interface scene, or at the end of the frame without one), when `FrameParameters::post`
    /// was true. `scene.picture` is a copy of the picture of the frame and `scene.depth` a
    /// copy of the depth of the game. The extension records its passes into `encoder` and
    /// writes its result into `target`, the picture of the frame; the user interface of the
    /// game draws over it after. MGE XE runs its shader chain here.
    fn post(&mut self, _scene: &SceneView, _encoder: &mut wgpu::CommandEncoder, _target: &wgpu::TextureView) {}

    /// The picture files that the extension wants of this frame. The renderer calls this
    /// after `post` and at the end of the frame, and writes each file one time: a request
    /// gives its file in one call only. A picture without the interface of the game that
    /// comes at the end of the frame (a frame without `post`) has the interface.
    fn pictures(&mut self) -> Vec<PictureRequest> {
        Vec::new()
    }

    /// The renderer calls this after `after_world`, when `FrameParameters::blend` was true
    /// and `before_world` gave true. The pass draws into the picture of the frame and has
    /// no depth target. `before` is the picture as it was after `before_world`, `depth`
    /// is the depth of the draws of the game (1 where the game drew nothing), and
    /// `scene.picture` is a copy of the picture as it is now. MGE XE lets the draws of the
    /// game near the end of their view range go into its distant land here.
    fn blend(&mut self, _scene: &SceneView, _pass: &mut wgpu::RenderPass<'static>, _before: &wgpu::TextureView, _depth: &wgpu::TextureView) {}

    /// What `call` reads of the picture and of the depth, asked just before the call: the
    /// renderer copies only these, and gives `None` in `SceneView` for the others. A picture
    /// that a draw only modulates needs no copy: a blend state reads the target.
    fn reads(&self, _call: ReadingCall) -> Reads {
        Reads::ALL
    }
}

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

// ---- the capture file

mod tag {
    pub const TEXTURE: u32 = 1;
    // 2 was the bytes of a buffer of the device, and 7 the state packet of a draw of the
    // device: a capture from before the retained pipeline has them, and they do nothing.
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
}

/// The size of a light of a shape draw: its key, then the `D3DLIGHT8` of 26 words.
const STATE_LIGHT_SIZE: usize = 108;

/// The words of the stage of a texture effect in a shape record: the coordinate index with
/// its generation, the transform flags, the texture matrix.
const EFFECT_STAGE_WORDS: usize = 18;

/// One stage of a pass of a shape draw, as the texture pass of the game sets it.
#[derive(Clone, Copy)]
struct PassStage {
    /// Operation, argument 1, argument 2 (Direct3D values). The operation 1 (disable) ends
    /// the stages of the pass.
    color: [u32; 3],
    alpha: [u32; 3],
    /// The coordinate index with its generation in the high word, and the transform flags.
    coordinate: u32,
    transform: u32,
    /// The four values of the bump matrix, then the luminance scale and offset.
    bump: [f32; 6],
    texture: u32,
    /// Address U, address V, the magnify, minify and mip filters.
    sampler: [u32; 5],
    texture_transform: [f32; 16],
}

/// The state of one pass of a shape draw, from the derived state of its material: what the
/// pipeline of the game sets for the pass, and the shape is the same for each pass.
#[derive(Clone)]
struct Pass {
    /// Blend on, source, destination.
    blend: [u32; 3],
    stages: [PassStage; 8],
    /// The alpha test in place of that of the material (on, function, reference), and the
    /// colour channels that the draw writes (D3DCOLORWRITEENABLE: red 1, green 2, blue 4,
    /// alpha 8): for the draws of the Code Patch into the picture of the character.
    alpha_test: Option<[u32; 3]>,
    color_write: u32,
}

impl Pass {
    /// `effects` is the stages of the environment map and of the fog map from the shape
    /// record (the texture pass of the game makes them from the effect and the camera of
    /// the frame); without them a stage of an effect gets the generation of its kind and no
    /// transform.
    fn new(d: &material::Derived, pass: usize, m: &material::Material, textures: &HashMap<u32, Rc<wgpu::TextureView>>, effects: Option<&[[u32; EFFECT_STAGE_WORDS]; 2]>) -> Pass {
        const NONE: PassStage = PassStage { color: [1, 0, 0], alpha: [1, 0, 0], coordinate: 0, transform: 0, bump: [0.0; 6], texture: 0, sampler: [0; 5], texture_transform: [0.0; 16] };
        // D3DBLEND_ONE is 2 and D3DBLEND_ZERO is 1: the blend of a pass that does not blend.
        let blend = d.blends.get(pass).copied().unwrap_or([0; 3]);
        let blend = if blend[0] != 0 { [1, blend[1], blend[2]] } else { [0, 2, 1] };
        let mut stages = [NONE; 8];
        for (stage, st) in stages.iter_mut().zip(&d.passes[pass]) {
            let effect = effects
                .and_then(|e| match st.source {
                    8 => Some(&e[0]),
                    9 => Some(&e[1]),
                    _ => None,
                })
                .filter(|words| words[0] != 0);
            let (coordinate, transform, texture_transform) = match effect {
                Some(words) => (words[0], words[1], core::array::from_fn(|k| f32::from_bits(words[2 + k]))),
                None => {
                    // D3DTSS_TCI_CAMERASPACEREFLECTIONVECTOR for the environment map and
                    // D3DTSS_TCI_CAMERASPACEPOSITION for the fog map.
                    let generated = match st.source {
                        8 => 0x30000,
                        9 => 0x20000,
                        _ => 0,
                    };
                    (st.set | generated, 0, [0.0; 16])
                }
            };
            let (clamp, filter) = match st.source {
                8 => (m.environment.clamp, m.environment.filter),
                9 => (m.fog_map.clamp, m.fog_map.filter),
                10 => (3, 1),
                map => (m.maps[map as usize].clamp, m.maps[map as usize].filter),
            };
            let mut sampler = material::sampler_states(clamp, filter);
            // A texture with one level gets no mip filter, as the texture manager sets it.
            if textures.get(&st.texture).is_some_and(|t| t.texture().mip_level_count() <= 1) {
                sampler[4] = 0;
            }
            *stage = PassStage {
                color: st.color,
                alpha: st.alpha,
                coordinate,
                transform,
                // The stage of the bump map has the values of the material.
                bump: if st.source == 5 { [m.bump[2], m.bump[3], m.bump[4], m.bump[5], m.bump[0], m.bump[1]] } else { [0.5, 0.0, 0.0, 0.5, 1.0, 0.0] },
                texture: st.texture,
                sampler,
                texture_transform,
            };
        }
        // A pass after the first has the alpha test of its template, not that of the material.
        let alpha_test = if pass > 0 { d.alpha_tests.get(pass).copied() } else { None };
        Pass { blend, stages, alpha_test, color_write: 15 }
    }

    /// The draws of the Morrowind Code Patch for this pass into the picture of the character
    /// of the inventory (`patchDrawIndexedPrimitive`; see `shape_stream::code_patch_picture`
    /// of the plugin). A pass that does not blend is drawn as it is. A pass that blends is
    /// drawn with the alpha of the texture: the alpha test "greater than 0" (D3DCMP_GREATER
    /// is 5), the blend SRCALPHA and INVSRCALPHA (5 and 6), and the alpha of stage 0 from
    /// its first argument (D3DTOP_SELECTARG1 is 2). Then, for a material with an alpha that
    /// is not 1, it is drawn into the alpha channel only, with the blend ONE and ZERO (2
    /// and 1) and the alpha of stage 0 modulated (D3DTOP_MODULATE is 4): the alpha of the
    /// picture gets the number of the equipment slot, which the game put into the material.
    fn code_patch_picture(&self, material_alpha: f32) -> (Pass, Option<Pass>) {
        if self.blend[0] == 0 {
            return (self.clone(), None);
        }
        let draw = |blend: [u32; 3], alpha_operation: u32, color_write: u32| {
            let mut pass = self.clone();
            pass.blend = blend;
            pass.stages[0].alpha[0] = alpha_operation;
            pass.alpha_test = Some([1, 5, 0]);
            pass.color_write = color_write;
            pass
        };
        (draw([1, 5, 6], 2, 15), (material_alpha != 1.0).then(|| draw([1, 2, 1], 4, 8)))
    }
}

pub mod gpu;
pub mod material;
mod overlay;

/// The geometry of a shape from a `GEOMETRY` record: resident buffers, keyed by the
/// geometry data of the game. See `geometry_stream` of the plugin.
struct Geometry {
    fvf: u32,
    stride: u32,
    /// The number of entries of the index list.
    entries: u32,
    /// The lengths of the strips, or empty for a triangle list.
    strips: Vec<u16>,
    vertex: Rc<wgpu::Buffer>,
    index: Rc<wgpu::Buffer>,
}

const COMMAND_TRANSFORM: u32 = 3;
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
    /// game (from its first specular word; 0 when the light has none).
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
    /// The values of `FrameParameters::sky`.
    pub extension: [[f32; 4]; 8],
    /// A plane in the space of the world of the draw: a pixel on its negative side is not
    /// drawn. All 0: no plane.
    pub clip_plane: [f32; 4],
}

/// The WGSL of the extension at the end of the shader: its sky functions and its draw
/// functions, or those that change nothing.
fn extension_shaders(extension: Option<&dyn Extension>) -> String {
    let sky = extension.and_then(|extension| extension.sky_shader()).unwrap_or_else(|| {
        "fn extension_sky_position(position: vec3f) -> vec3f { return position; }\nfn extension_sky_color(direction: vec3f, pixel: vec2f) -> vec3f { return vec3f(0.0); }\n".to_string()
    });
    let draw = extension.and_then(|extension| extension.draw_shader()).unwrap_or_else(|| {
        "fn extension_shade(in: VertexOut) -> vec4f { return in.vertex_color; }\nfn extension_finish(color: vec3f, alpha: f32, clear: f32) -> vec4f { return vec4f(mix(draw.fog_color.rgb, color, clear), alpha); }\n".to_string()
    });
    format!("{sky}\n{draw}\n")
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

/// The parts of the work that the program measures. The parts do not overlap: their sum is
/// the time that the program is busy.
const TIME_NAMES: [&str; TIME_COUNT] = [
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
const TIME_COUNT: usize = 17;
const TIME_BUFFERS: usize = 16;
const TIME_CLUSTERS: usize = 15;
const TIME_READ: usize = 0;
const TIME_RECORDS: usize = 1;
const TIME_DRAW_DATA: usize = 2;
const TIME_DRAW_UPLOAD: usize = 3;
const TIME_DRAW_GROUPS: usize = 4;
const TIME_DRAW_PASS: usize = 5;
const TIME_ITEMS: usize = 6;
const TIME_EXT_FRAME: usize = 7;
const TIME_EXT_BEFORE: usize = 8;
const TIME_EXT_AFTER: usize = 9;
const TIME_EXT_CLASS: usize = 10;
const TIME_EXT_POST: usize = 11;
const TIME_END: usize = 12;
const TIME_ACQUIRE: usize = 13;
const TIME_PRESENT: usize = 14;

/// The sum of the measured times without one of them: for a part that has other measured
/// parts inside it, its own time is the time of the whole less the change of this sum.
fn sum_except(times: &[std::time::Duration; TIME_COUNT], skip: usize) -> std::time::Duration {
    times.iter().enumerate().filter(|(i, _)| *i != skip).map(|(_, t)| *t).sum()
}

/// The test of a D3DCMPFUNC value, for an extension that draws with the tests of a shape
/// (`HeldDraw::depth`, `HeldDraw::stencil`).
pub fn d3d_compare(value: u32) -> wgpu::CompareFunction {
    compare(value)
}

/// The operation of a D3DSTENCILOP value, for the same.
pub fn d3d_stencil_operation(value: u32) -> wgpu::StencilOperation {
    stencil_operation(value)
}

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
        if betas == 4 && fvf & 0x1000 != 0 {
            list.push(Attribute { location: 8, offset: offset + 12, format: Uint8x4 });
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
const INPUT_FORMATS: [wgpu::VertexFormat; 9] = {
    use wgpu::VertexFormat::*;
    [Float32x3, Float32x3, Unorm8x4, Float32x2, Float32x2, Float32x2, Float32x2, Float32x3, Uint8x4]
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

#[derive(Default)]
struct Counts {
    draws: u32,
    other_target: u32,
    no_layout: u32,
    textures: u32,
    texture_updates: u32,
    render_textures: u32,
    pixel_reads: u32,
    /// The copies of the picture and of the depth for the calls of the extension.
    picture_copies: u32,
    depth_copies: u32,
    /// Where the time of the program goes, in the order of `TIME_NAMES`.
    times: [std::time::Duration; TIME_COUNT],
    /// The frames that are complete.
    frames: u32,
    /// The geometry records, their bytes, the draws from them, the records that did not
    /// parse, and the draws of more index entries than their geometry has.
    geometry_records: u32,
    geometry_bytes: u64,
    geometry_draws: u32,
    geometry_bad: u32,
    geometry_short: u32,
    material_records: u32,
    /// The shape records, the draws made from them, and the records without a geometry or
    /// a material.
    shape_records: u32,
    shape_draws: u32,
    shape_missing: u32,
    /// The shapes with a rule of the engine that this program does not follow: a material
    /// without the smooth flag (the game sets `D3DSHADE_FLAT` for it), and a material with
    /// the specular flag (the game sets `D3DRS_SPECULARENABLE`).
    shape_flat: u32,
    shape_specular: u32,
    /// The shape records with a palette of bones, and the records of particle systems.
    shape_skinned: u32,
    shape_particles: u32,
    /// The light records, and the lights of a shape that no record came for.
    light_records: u32,
    light_missing: u32,
    /// The lights and the entries of the clusters of the frames, and the frames that kept
    /// the lights of the draws because a light had no cut-off.
    cluster_lights: u64,
    cluster_entries: u64,
    /// The lights of the frames for the extension (`FrameLights`).
    extension_lights: u64,
    cluster_frames_off: u32,
    /// The frames with more clustered lights than a mask of a draw has bits.
    cluster_frames_many: u32,
    /// The GPU time of the render passes and of the whole frame, in nanoseconds, and the
    /// frames that were timed.
    gpu_passes: f64,
    gpu_frame: f64,
    gpu_frames: u32,
    /// The opaque draws that were sorted, and the draws that became an instance of another.
    sorted_draws: u32,
    instanced_draws: u32,
    screen_draws: u32,
    screen_draws_lost: u32,
    target_copies: u32,
    target_draws: u32,
    /// The kept draws that were drawn again for an extension.
    replayed: u32,
    /// The shapes that the extension took (`Extension::hold`).
    held: u32,
    texture_updates_lost: u32,
    lost_reasons: HashMap<String, u32>,
    textures_decoded: u32,
    texture_formats: HashMap<u32, u32>,
    missing_textures: u32,
    more_stages: u32,
    /// The most lights that a draw had.
    most_lights: u32,
    coordinate_work: u32,
}

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;
const STAGES: usize = 6;
/// The size of a slot for the data of a draw: the size of `DrawData`, and a multiple of the
/// 256 bytes that a uniform offset must be a multiple of.
const SLOT_SIZE: usize = core::mem::size_of::<DrawData>().next_multiple_of(256);
/// The lights that the light buffer of a frame holds at first.
const LIGHTS_AT_FIRST: usize = 4096;

/// The scheduling of this program on Windows: it draws for the game from the background,
/// and a hybrid processor gave it an efficiency core in some runs, so that a frame took
/// twice its time. The thread of the frames goes to the cores of the highest efficiency
/// class, and the process out of the power throttling of a background process. A
/// processor with one class of cores gets no change of its cores. The priority of the
/// process stays as it is. `MGE_XE_CORES=0` leaves the scheduler alone.
#[cfg(windows)]
mod cores {
    #[repr(C)]
    struct CpuSetInformation {
        size: u32,
        kind: u32,
        id: u32,
        group: u16,
        logical_processor_index: u8,
        core_index: u8,
        last_level_cache_index: u8,
        numa_node_index: u8,
        efficiency_class: u8,
        all_flags: u8,
        reserved: u32,
        allocation_tag: u64,
    }
    #[repr(C)]
    struct PowerThrottlingState {
        version: u32,
        control_mask: u32,
        state_mask: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn GetCurrentThread() -> isize;
        fn SetProcessInformation(process: isize, class: u32, information: *const core::ffi::c_void, size: u32) -> i32;
        fn GetSystemCpuSetInformation(information: *mut CpuSetInformation, length: u32, returned: *mut u32, process: isize, flags: u32) -> i32;
        fn SetThreadSelectedCpuSets(thread: isize, ids: *const u32, count: u32) -> i32;
    }
    const PROCESS_POWER_THROTTLING: u32 = 4;
    const PROCESS_POWER_THROTTLING_EXECUTION_SPEED: u32 = 1;

    /// Returns one line for the log with what was done.
    pub fn prefer_performance_cores() -> String {
        if std::env::var("MGE_XE_CORES").is_ok_and(|v| v == "0") {
            return "cores: left to the scheduler (MGE_XE_CORES=0)".into();
        }
        // SAFETY: calls of the Windows API with valid arguments; the buffer is sized by the
        // first call.
        unsafe {
            let process = GetCurrentProcess();
            let state = PowerThrottlingState { version: 1, control_mask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED, state_mask: 0 };
            let throttling = SetProcessInformation(process, PROCESS_POWER_THROTTLING, &state as *const _ as *const core::ffi::c_void, core::mem::size_of::<PowerThrottlingState>() as u32) != 0;
            let mut needed = 0u32;
            GetSystemCpuSetInformation(core::ptr::null_mut(), 0, &mut needed, process, 0);
            let count = needed as usize / core::mem::size_of::<CpuSetInformation>();
            let mut sets: Vec<CpuSetInformation> = Vec::with_capacity(count.max(1));
            let got = GetSystemCpuSetInformation(sets.as_mut_ptr(), needed, &mut needed, process, 0) != 0;
            if got {
                sets.set_len(needed as usize / core::mem::size_of::<CpuSetInformation>());
            }
            let best = sets.iter().map(|s| s.efficiency_class).max().unwrap_or(0);
            let classes = sets.iter().map(|s| s.efficiency_class).min().unwrap_or(0) != best;
            let ids: Vec<u32> = sets.iter().filter(|s| s.efficiency_class == best).map(|s| s.id).collect();
            let pinned = classes && !ids.is_empty() && SetThreadSelectedCpuSets(GetCurrentThread(), ids.as_ptr(), ids.len() as u32) != 0;
            format!(
                "cores: {} of {} logical processors in the highest efficiency class{}; power throttling off {}",
                ids.len(),
                sets.len(),
                if pinned { ", the thread of the frames on them" } else if classes { ", not pinned" } else { ", one class, not pinned" },
                if throttling { "set" } else { "not set" }
            )
        }
    }
}
#[cfg(not(windows))]
mod cores {
    pub fn prefer_performance_cores() -> String {
        "cores: left to the scheduler".into()
    }
}

/// The bind group of the resources of the extension for a frame, and the clusters of the
/// lights of the frame.
fn frame_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, uniform: &wgpu::Buffer, sampler: &wgpu::Sampler, texture: &wgpu::TextureView, clusters: &Clusters) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("extension frame"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(texture) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 3, resource: clusters.uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: clusters.table.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 5, resource: clusters.indexes.as_entire_binding() },
        ],
    })
}

/// The grid of the clusters: tiles across and down the screen, slices of depth.
const CLUSTER_GRID: [u32; 3] = [16, 9, 24];
/// The most lights of a cluster.
const CLUSTER_LIGHTS: usize = 64;

/// The values of the clusters for the shader: the grid (w: 1 for the clusters of the draws, 2
/// for those of the extension, 0 when they are off), the tiles for each pixel across and
/// down, then the scale, the bias and the least depth of the slice of a depth.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ClusterData {
    grid: [u32; 4],
    screen: [f32; 4],
    depth: [f32; 4],
}

/// Bins lights into the grid of the clusters of a projection of Direct3D. Each item is the
/// entry of a light, its centre in the space of the view (`None` for a directional light,
/// which is in each cluster) and its reach. `lists` has a list for each cluster. Returns the
/// table (the first entry and the number for each cluster), the entries, and the values for
/// the shader with `w` in the grid.
fn bin_lights(lists: &mut [Vec<u32>], items: impl Iterator<Item = (u32, Option<[f32; 3]>, f32)>, projection: &[f32; 16], size: (u32, u32), w: u32) -> (Vec<[u32; 2]>, Vec<u32>, ClusterData) {
    for list in lists.iter_mut() {
        list.clear();
    }
    let (gx, gy, gz) = (CLUSTER_GRID[0] as usize, CLUSTER_GRID[1] as usize, CLUSTER_GRID[2] as usize);
    // The near and far planes of a projection of Direct3D.
    let near = if projection[10] != 0.0 { (-projection[14] / projection[10]).max(1.0) } else { 1.0 };
    let far = if projection[10] != 1.0 { (projection[14] / (1.0 - projection[10])).max(near * 2.0) } else { near * 1000.0 };
    let depth_scale = gz as f32 / (far / near).ln();
    let slice_of = |z: f32| ((z.max(near) / near).ln() * depth_scale).floor().clamp(0.0, gz as f32 - 1.0) as usize;
    let (sx, sy) = (projection[0], projection[5]);
    for (entry, centre, reach) in items {
        let (x0, x1, y0, y1, z0, z1);
        match centre {
            None => (x0, x1, y0, y1, z0, z1) = (0, gx - 1, 0, gy - 1, 0, gz - 1),
            Some(c) => {
                if c[2] + reach < near || c[2] - reach > far {
                    continue;
                }
                z0 = slice_of(c[2] - reach);
                z1 = slice_of(c[2] + reach);
                if c[2] - reach < near {
                    (x0, x1, y0, y1) = (0, gx - 1, 0, gy - 1);
                } else {
                    let nearest = c[2] - reach;
                    let (half_w, half_h) = (reach * sx / nearest, reach * sy / nearest);
                    let (cx, cy) = (c[0] * sx / c[2], c[1] * sy / c[2]);
                    let tile_x = |ndc: f32| (((ndc + 1.0) * 0.5 * gx as f32).floor()).clamp(0.0, gx as f32 - 1.0) as usize;
                    let tile_y = |ndc: f32| (((1.0 - ndc) * 0.5 * gy as f32).floor()).clamp(0.0, gy as f32 - 1.0) as usize;
                    (x0, x1) = (tile_x(cx - half_w), tile_x(cx + half_w));
                    (y0, y1) = (tile_y(cy + half_h), tile_y(cy - half_h));
                }
            }
        }
        for z in z0..=z1 {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let list = &mut lists[(z * gy + y) * gx + x];
                    if list.len() < CLUSTER_LIGHTS {
                        list.push(entry);
                    }
                }
            }
        }
    }
    let mut table: Vec<[u32; 2]> = Vec::with_capacity(lists.len());
    let mut indexes: Vec<u32> = Vec::new();
    for list in lists.iter() {
        table.push([indexes.len() as u32, list.len() as u32]);
        indexes.extend_from_slice(list);
    }
    let data = ClusterData {
        grid: [CLUSTER_GRID[0], CLUSTER_GRID[1], CLUSTER_GRID[2], w],
        screen: [gx as f32 / size.0.max(1) as f32, gy as f32 / size.1.max(1) as f32, 0.0, 0.0],
        depth: [depth_scale, -near.ln() * depth_scale, near, 0.0],
    };
    (table, indexes, data)
}

/// A storage buffer with room for `count` items of `item` bytes.
fn storage_buffer(device: &wgpu::Device, label: &str, count: usize, item: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: (count.max(1) * item) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
}

/// The clusters of the lights of the frame: for each cluster of the grid, the lights of
/// the frame whose sphere touches it, as indexes into the light buffer of the frame.
/// Binned on the CPU in one space, relative to the origin of the scene, for the view and
/// the projection of the scene (the start of the world).
struct Clusters {
    on: bool,
    uniform: wgpu::Buffer,
    table: wgpu::Buffer,
    indexes: wgpu::Buffer,
    index_capacity: usize,
    /// The lights of the frame that are candidates, by their key: the index in the light
    /// buffer of the frame, the slot of the light in the masks of the draws, and whether
    /// its position is relative to the origin of the scene (the light buffer of the frame
    /// has some relative and some not, as the draws are).
    unique: HashMap<u32, (u32, u32, bool)>,
    /// The origin of the scene of the candidates, which the relative positions are
    /// relative to.
    origin: Option<[f64; 3]>,
    /// The frame has more candidates than a mask has bits, or an index a cluster entry
    /// cannot hold: the frame keeps the lights of the draws.
    overflow: bool,
    lists: Vec<Vec<u32>>,
}

/// The bits of the mask of a draw over the clustered lights of the frame.
const CLUSTER_SLOTS: usize = 256;

impl Clusters {
    fn new(device: &wgpu::Device) -> Clusters {
        let cells = (CLUSTER_GRID[0] * CLUSTER_GRID[1] * CLUSTER_GRID[2]) as usize;
        let index_capacity = 1 << 16;
        Clusters {
            // Off unless `MGE_XE_CLUSTERS=1`: with the masks the clusters give the picture of
            // the lights of the draws, and at Narsis by night (13 lights for a draw at most)
            // the GPU takes the same time either way while the binning costs the helper
            // 0.4 ms a frame. They pay only with many more lights for each draw.
            on: std::env::var("MGE_XE_CLUSTERS").is_ok_and(|v| v == "1"),
            uniform: device.create_buffer(&wgpu::BufferDescriptor { label: Some("clusters"), size: core::mem::size_of::<ClusterData>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
            table: storage_buffer(device, "cluster table", cells, 8),
            indexes: storage_buffer(device, "cluster lights", index_capacity, 4),
            index_capacity,
            unique: HashMap::new(),
            origin: None,
            overflow: false,
            lists: (0..cells).map(|_| Vec::new()).collect(),
        }
    }

    /// Notes a light of the frame as a candidate, by its key and its index in the light
    /// buffer: the first draw with the key gives the values. `origin` is that of the scene
    /// of the draw; `relative` says that the position of the light is relative to it.
    /// Returns the slot of the light in the masks of the draws, or `None` when the frame
    /// has no slot left.
    fn note(&mut self, key: u32, index: u32, relative: bool, origin: Option<[f64; 3]>) -> Option<u32> {
        let next = self.unique.len() as u32;
        let (_, slot, _) = *self.unique.entry(key).or_insert((index, next, relative));
        self.origin = origin;
        if slot as usize >= CLUSTER_SLOTS || index >= 1 << 16 {
            self.overflow = true;
            return None;
        }
        Some(slot)
    }

    /// Bins the candidates into the grid for the view and the projection of the scene and
    /// writes the buffers. `size` is the size of the picture. Returns false when the
    /// clusters are off for the frame (no start of the world).
    fn build(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, lights: &[LightData], view: Option<([f32; 16], [f32; 16])>, size: (u32, u32), counts: &mut Counts) -> bool {
        let overflow = core::mem::replace(&mut self.overflow, false);
        if overflow {
            counts.cluster_frames_many += 1;
        }
        let Some((mut view, projection)) = view.filter(|_| self.on && !self.unique.is_empty() && !overflow) else {
            self.unique.clear();
            queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&ClusterData::zeroed()));
            return false;
        };
        // Relative to the origin of the scene, the camera: the view without its move.
        let origin = self.origin;
        if origin.is_some() {
            view[12] = 0.0;
            view[13] = 0.0;
            view[14] = 0.0;
        }
        // The candidates by their slot; an entry of a list is the slot in its high half and
        // the index in its low half.
        let mut candidates: Vec<(u32, u32, bool)> = self.unique.values().map(|&(index, slot, relative)| (slot, index, relative)).collect();
        candidates.sort_unstable();
        // A point light without a cut-off (the fade of the extension is off) has no end:
        // its tail would reach the pixels of objects the game never attached it to. The
        // frame keeps the lights of the draws then.
        if candidates.iter().any(|&(_, index, _)| lights.get(index as usize).is_some_and(|light| light.position[3] != 3.0 && light.cone[0] <= 0.0)) {
            self.unique.clear();
            counts.cluster_frames_off += 1;
            queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&ClusterData::zeroed()));
            return false;
        }
        counts.cluster_lights += candidates.len() as u64;
        let items = candidates.iter().filter_map(|&(slot, index, relative)| {
            let light = lights.get(index as usize)?;
            let centre = (light.position[3] != 3.0).then(|| {
                let mut position = light.position;
                if let (Some(o), false) = (origin, relative) {
                    for axis in 0..3 {
                        position[axis] = (position[axis] as f64 - o[axis]) as f32;
                    }
                }
                view_point(&view, position)
            });
            // The reach: the cut-off of the extension (its fade).
            Some((index | slot << 16, centre, 1.0 / light.cone[0]))
        });
        let (table, indexes, data) = bin_lights(&mut self.lists, items, &projection, size, 1);
        self.unique.clear();
        if indexes.len() > self.index_capacity {
            self.index_capacity = indexes.len().next_power_of_two();
            self.indexes = storage_buffer(device, "cluster lights", self.index_capacity, 4);
        }
        counts.cluster_entries += indexes.len() as u64;
        queue.write_buffer(&self.table, 0, bytemuck::cast_slice(&table));
        if !indexes.is_empty() {
            queue.write_buffer(&self.indexes, 0, bytemuck::cast_slice(&indexes));
        }
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&data));
        true
    }
}

/// A point in the space of a view (the layout of Direct3D).
fn view_point(view: &[f32; 16], p: [f32; 4]) -> [f32; 3] {
    [
        p[0] * view[0] + p[1] * view[4] + p[2] * view[8] + view[12],
        p[0] * view[1] + p[1] * view[5] + p[2] * view[9] + view[13],
        p[0] * view[2] + p[1] * view[6] + p[2] * view[10] + view[14],
    ]
}

/// The lights of the frame for the extension (`FrameLights`): one copy of each light that a
/// world draw has, its position relative to the origin of the scene (the light buffer of the
/// frame has some relative and some not, as the draws are), and clusters of their own.
struct ExtensionLights {
    /// The lights of the frame by their key: the index of the first copy in the light buffer
    /// of the frame, and whether its position is relative to the origin of the scene.
    noted: HashMap<u32, (u32, bool)>,
    lights: wgpu::Buffer,
    light_capacity: usize,
    uniform: wgpu::Buffer,
    table: wgpu::Buffer,
    indexes: wgpu::Buffer,
    index_capacity: usize,
    lists: Vec<Vec<u32>>,
    /// The origin of the scene of the noted lights, which the positions of the relative ones
    /// are relative to.
    scene_origin: Option<[f64; 3]>,
    /// The lights and the clusters are those of this frame, and the point that the
    /// positions are relative to.
    built: bool,
    origin: [f64; 3],
}

impl ExtensionLights {
    fn new(device: &wgpu::Device) -> ExtensionLights {
        let cells = (CLUSTER_GRID[0] * CLUSTER_GRID[1] * CLUSTER_GRID[2]) as usize;
        ExtensionLights {
            noted: HashMap::new(),
            lights: storage_buffer(device, "extension lights", 64, core::mem::size_of::<LightData>()),
            light_capacity: 64,
            uniform: device.create_buffer(&wgpu::BufferDescriptor { label: Some("extension clusters"), size: core::mem::size_of::<ClusterData>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
            table: storage_buffer(device, "extension cluster table", cells, 8),
            indexes: storage_buffer(device, "extension cluster lights", 1 << 12, 4),
            index_capacity: 1 << 12,
            lists: (0..cells).map(|_| Vec::new()).collect(),
            scene_origin: None,
            built: false,
            origin: [0.0; 3],
        }
    }

    /// The start of a frame: no lights.
    fn clear(&mut self) {
        self.noted.clear();
        self.built = false;
    }

    /// Notes a light of a world draw, by its key and its index in the light buffer of the
    /// frame: the first draw with the key gives the values. `origin` is that of the scene of
    /// the draw; `relative` says that the position of the light is relative to it.
    fn note(&mut self, key: u32, index: u32, relative: bool, origin: Option<[f64; 3]>) {
        self.noted.entry(key).or_insert((index, relative));
        self.scene_origin = origin;
    }

    /// Makes the lights of the frame for the extension, one copy of each relative to the
    /// origin of the scene, and bins them for the view and the projection that the extension
    /// gets (`SceneView`).
    fn build(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame_lights: &[LightData], view: [f32; 16], projection: [f32; 16], size: (u32, u32), counts: &mut Counts) {
        self.built = false;
        let noted = core::mem::take(&mut self.noted);
        if noted.is_empty() {
            return;
        }
        let origin = self.scene_origin;
        let mut view = view;
        // Relative to the origin of the scene, the camera: the view without its move.
        if origin.is_some() {
            view[12] = 0.0;
            view[13] = 0.0;
            view[14] = 0.0;
        }
        let mut first: Vec<(u32, bool)> = noted.into_values().collect();
        first.sort_unstable();
        let mut lights: Vec<LightData> = Vec::with_capacity(first.len());
        let mut items = Vec::with_capacity(first.len());
        for (index, relative) in first {
            let Some(&light) = frame_lights.get(index as usize) else { continue };
            let mut light = light;
            let centre = (light.position[3] != 3.0).then(|| {
                if let (Some(o), false) = (origin, relative) {
                    for axis in 0..3 {
                        light.position[axis] = (light.position[axis] as f64 - o[axis]) as f32;
                    }
                }
                view_point(&view, light.position)
            });
            items.push((lights.len() as u32, centre, light_reach(&light)));
            lights.push(light);
        }
        let (table, indexes, data) = bin_lights(&mut self.lists, items.into_iter(), &projection, size, 2);
        if lights.len() > self.light_capacity {
            self.light_capacity = lights.len().next_power_of_two();
            self.lights = storage_buffer(device, "extension lights", self.light_capacity, core::mem::size_of::<LightData>());
        }
        if indexes.len() > self.index_capacity {
            self.index_capacity = indexes.len().next_power_of_two();
            self.indexes = storage_buffer(device, "extension cluster lights", self.index_capacity, 4);
        }
        counts.extension_lights += lights.len() as u64;
        if !lights.is_empty() {
            queue.write_buffer(&self.lights, 0, bytemuck::cast_slice(&lights));
        }
        queue.write_buffer(&self.table, 0, bytemuck::cast_slice(&table));
        if !indexes.is_empty() {
            queue.write_buffer(&self.indexes, 0, bytemuck::cast_slice(&indexes));
        }
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&data));
        self.origin = origin.unwrap_or([0.0; 3]);
        self.built = true;
    }
}

/// A buffer for the lights of a frame.
fn light_buffer(device: &wgpu::Device, lights: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("lights"),
        size: (lights * core::mem::size_of::<LightData>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
const BONES_AT_FIRST: usize = 4096;
const INSTANCES_AT_FIRST: usize = 4096;
/// The world matrices of the instances of a frame.
fn instance_buffer(device: &wgpu::Device, instances: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("instances"),
        size: (instances * 64) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The render passes of a frame that get a pair of time stamps, and the pair of the frame
/// after them.
const TIMING_PASSES: u32 = 126;
const TIMING_QUERIES: u32 = 2 * (TIMING_PASSES + 1);

/// GPU time stamps: a pair for each render pass of the frame, and a pair for the frame
/// (when the adapter writes them outside a pass). They are resolved at the end of the
/// frame into one of three staging buffers, and read when the buffer is mapped, frames
/// later, so the program never waits for the GPU.
struct Timing {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    staging: Vec<wgpu::Buffer>,
    /// The staging buffers that are mapped or being mapped, and the passes of the frame
    /// of each: a pair that no pass of the frame wrote keeps the stamps of an older frame.
    busy: Vec<bool>,
    slot_passes: Vec<u32>,
    ready: std::sync::mpsc::Receiver<usize>,
    sender: std::sync::mpsc::Sender<usize>,
    /// The passes of the frame with a pair.
    passes: u32,
    /// Nanoseconds for each stamp step.
    period: f32,
    /// The adapter writes stamps outside a pass: the frame has its pair.
    frame_stamps: bool,
}

/// With `MGE_XE_GPU_TIME=1`: the stamps cost about 0.4 ms a frame at the end of the frame.
fn make_timing(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Timing> {
    if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) || std::env::var("MGE_XE_GPU_TIME").map_or(true, |v| v != "1") {
        return None;
    }
    let size = TIMING_QUERIES as u64 * 8;
    let buffer = |usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some("timing"), size, usage, mapped_at_creation: false });
    let (sender, ready) = std::sync::mpsc::channel();
    Some(Timing {
        set: device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("timing"), ty: wgpu::QueryType::Timestamp, count: TIMING_QUERIES }),
        resolve: buffer(wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST),
        staging: (0..3).map(|_| buffer(wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST)).collect(),
        busy: vec![false; 3],
        slot_passes: vec![0; 3],
        ready,
        sender,
        passes: 0,
        period: queue.get_timestamp_period(),
        frame_stamps: device.features().contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS),
    })
}

impl Timing {
    /// The pair of the next render pass of the frame, or `None` when the frame has more
    /// passes than pairs.
    fn next_pass(&mut self) -> Option<u32> {
        let index = self.passes;
        if index >= TIMING_PASSES {
            return None;
        }
        self.passes += 1;
        Some(index)
    }

    /// The start of a frame: the stamp of the frame, when the adapter can write it.
    fn begin(&mut self, encoder: &mut wgpu::CommandEncoder) {
        self.passes = 0;
        if self.frame_stamps {
            encoder.write_timestamp(&self.set, 2 * TIMING_PASSES);
        }
    }

    /// The end of a frame, before the submit: reads the staging buffers that are mapped,
    /// then resolves the stamps of this frame into a free one. Returns the buffer to map
    /// after the submit.
    fn end(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, counts: &mut Counts) -> Option<usize> {
        let _ = device.poll(wgpu::PollType::Poll);
        while let Ok(slot) = self.ready.try_recv() {
            {
                let mapped = self.staging[slot].slice(..).get_mapped_range().expect("the time stamps");
                let stamp = |i: u32| u64::from_le_bytes(mapped[i as usize * 8..i as usize * 8 + 8].try_into().unwrap());
                let span = |i: u32| -> f64 {
                    let (begin, end) = (stamp(2 * i), stamp(2 * i + 1));
                    if begin != 0 && end > begin { (end - begin) as f64 * self.period as f64 } else { 0.0 }
                };
                counts.gpu_passes += (0..self.slot_passes[slot]).map(span).sum::<f64>();
                counts.gpu_frame += span(TIMING_PASSES);
                counts.gpu_frames += 1;
            }
            self.staging[slot].unmap();
            self.busy[slot] = false;
        }
        if self.frame_stamps {
            encoder.write_timestamp(&self.set, 2 * TIMING_PASSES + 1);
        }
        let slot = self.busy.iter().position(|busy| !busy)?;
        encoder.clear_buffer(&self.resolve, 0, None);
        encoder.resolve_query_set(&self.set, 0..TIMING_QUERIES, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.staging[slot], 0, TIMING_QUERIES as u64 * 8);
        self.busy[slot] = true;
        self.slot_passes[slot] = self.passes;
        Some(slot)
    }

    /// After the submit: asks for the map of the buffer of the frame.
    fn map(&self, slot: usize) {
        let sender = self.sender.clone();
        self.staging[slot].slice(..).map_async(wgpu::MapMode::Read, move |result| {
            if result.is_ok() {
                let _ = sender.send(slot);
            }
        });
    }
}
/// The bones of the skinned draws of the frame, matrices.
fn bone_buffer(device: &wgpu::Device, bones: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bones"),
        size: (bones * 64) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
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
    /// The data of the draws of the frame, one block for each buffer of slots, written to
    /// the GPU once before the submit (`flush_draw_slots`).
    draw_staging: Vec<Vec<u8>>,
    /// The draws keep the order of the game: it decides the ties of the depth test (the lines
    /// of the collision boxes lie on their meshes). `MGE_XE_SORT=1` sorts the opaque draws of
    /// a run by pipeline and textures and joins them into instances, for a comparison; it
    /// saved no time of the frame in the measures of plan step 1c.6.
    sort_draws: bool,
    /// The lights of all draws of the frame, and the number of them that the buffer holds.
    light_buffer: wgpu::Buffer,
    light_capacity: usize,
    bone_buffer: wgpu::Buffer,
    bone_capacity: usize,
    /// The lights that came as records, by key: a `D3DLIGHT8`.
    lights_known: HashMap<u32, [u8; 104]>,
    /// The GPU time stamps, when the adapter has them.
    timing: Option<Timing>,
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
    /// The resources of the extension for the frame: bind group 3 of each draw.
    frame_layout: wgpu::BindGroupLayout,
    frame_uniform: wgpu::Buffer,
    frame_sampler: wgpu::Sampler,
    frame_group: Rc<wgpu::BindGroup>,
    clusters: Clusters,
    extension_lights: ExtensionLights,
    samplers: HashMap<[u32; 5], Rc<wgpu::Sampler>>,
    /// The textures and the samplers of a draw, by the texture handles and the sampler values
    /// of its stages. The D3D12 backend has a small number of places for samplers.
    texture_groups: HashMap<[u32; STAGES], Rc<wgpu::BindGroup>>,
    /// The samplers of a draw, by the sampler values of its stages. They are apart from the
    /// textures: the D3D12 backend has places for 2048 samplers only, and the sets of sampler
    /// values are few.
    sampler_groups: HashMap<[[u32; 5]; STAGES], Rc<wgpu::BindGroup>>,
    /// The geometry records, by the geometry data of the game.
    geometries: HashMap<u32, Geometry>,
    /// The index buffer of the quads of the particle systems, as the game keeps one: the
    /// number of quads it has, and the buffer. It grows to the largest system seen.
    quad_indexes: Option<(u32, Rc<wgpu::Buffer>)>,
    /// The material records by handle, and the states derived from them by (handle, texture
    /// sets of the geometry).
    materials: HashMap<u32, material::Material>,
    derived: HashMap<(u32, u32), Rc<material::Derived>>,
    defaults: wgpu::Buffer,
    /// The world matrices of the instances of the frame. `MGE_XE_INSTANCING=0` keeps one
    /// draw for each.
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    instancing: bool,
    /// For a draw in screen coordinates: the pipeline, the layout of its group, and its
    /// sampler.
    screen_pipeline: Option<wgpu::RenderPipeline>,
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
        extension_shader: String,
    ) -> (Renderer, Option<wgpu::Surface<'static>>) {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = backends;
        // The WGPU_* variables of the environment can change the flags, for a test (for
        // example WGPU_VALIDATION_INDIRECT_CALL=0).
        descriptor.flags = descriptor.flags.with_env();
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
        println!("adapter: {} ({:?}, driver {} {})", info.name, info.backend, info.driver, info.driver_info);
        let mut surface_usages = wgpu::TextureUsages::empty();
        if let Some(surface) = surface.as_ref() {
            // The game gives colours that are ready for the screen: no sRGB format.
            let capabilities = surface.get_capabilities(&adapter);
            if !capabilities.formats.contains(&target_format) {
                target_format = capabilities.formats.iter().copied().find(|format| !format.is_srgb()).unwrap_or(capabilities.formats[0]);
            }
            surface_usages = capabilities.usages;
            println!("window: the present modes of the surface are {:?}", capabilities.present_modes);
        }
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                // The draws of an extension from the GPU (the distant statics of MGE XE) need the
                // first instance and the count of indirect draws; without them it draws as before.
                required_features: wgpu::Features::TEXTURE_COMPRESSION_BC
                    | (adapter.features()
                        & (wgpu::Features::POLYGON_MODE_LINE
                            | wgpu::Features::TIMESTAMP_QUERY
                            | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
                            | wgpu::Features::INDIRECT_FIRST_INSTANCE
                            | wgpu::Features::MULTI_DRAW_INDIRECT_COUNT)),
                // The largest texture that the adapter takes, up to 16384: an atlas of the
                // distant land of MGE XE has that size.
                // A post shader of MGE XE reads up to 22 textures: 10 of its own, the picture,
                // the depth, the waves, and the outputs of its passes.
                required_limits: wgpu::Limits {
                    max_texture_dimension_2d: adapter.limits().max_texture_dimension_2d.min(16384),
                    max_sampled_textures_per_shader_stage: adapter.limits().max_sampled_textures_per_shader_stage.min(32),
                    // One buffer for the geometry of many meshes of an extension (the pool of
                    // the distant statics of MGE XE).
                    max_buffer_size: adapter.limits().max_buffer_size,
                    ..wgpu::Limits::default()
                },
                memory_hints: wgpu::MemoryHints::default(),
                ..Default::default()
            },
        ))
        .expect("no device");
        gpu::watch(&device, &info, surface.is_some());

        let wireframe = device.features().contains(wgpu::Features::POLYGON_MODE_LINE);
        let source = format!("{}\n{extension_shader}", include_str!("shader.wgsl"));
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("shader.wgsl"), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
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
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("extension frame"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&draw_layout), Some(&texture_layout), Some(&sampler_layout), Some(&frame_layout)],
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
        // The inputs a vertex does not have, for each instance of a draw.
        let defaults = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &vec![0u8; 64 * INSTANCES_MOST],
            usage: wgpu::BufferUsages::VERTEX,
        });
        let frame_uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("extension frame"), size: 512, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let frame_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("extension frame"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let clusters = Clusters::new(&device);
        let extension_lights = ExtensionLights::new(&device);
        let frame_group = Rc::new(frame_group(&device, &frame_layout, &frame_uniform, &frame_sampler, &white, &clusters));
        let renderer = Renderer {
            frame_layout,
            frame_uniform,
            clusters,
            extension_lights,
            frame_sampler,
            frame_group,
            light_buffer: light_buffer(&device, LIGHTS_AT_FIRST),
            instance_buffer: instance_buffer(&device, INSTANCES_AT_FIRST),
            instance_capacity: INSTANCES_AT_FIRST,
            instancing: std::env::var("MGE_XE_INSTANCING").map_or(true, |v| v != "0"),
            lights_known: HashMap::new(),
            timing: make_timing(&device, &queue),
            bone_buffer: bone_buffer(&device, BONES_AT_FIRST),
            bone_capacity: BONES_AT_FIRST,
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
            draw_staging: Vec::new(),
            sort_draws: std::env::var("MGE_XE_SORT").is_ok_and(|v| v == "1"),
            light_capacity: LIGHTS_AT_FIRST,
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
            geometries: HashMap::new(),
            quad_indexes: None,
            materials: HashMap::new(),
            derived: HashMap::new(),
            defaults,
            screen_pipeline: None,
            screen_layout: None,
            screen_sampler: None,
            counts: Counts::default(),
        };
        (renderer, surface)
    }

    /// Puts the data of a draw into the next slot. Returns the group to bind and the offset
    /// of the slot.
    /// The bind group of a buffer of draw slots: the slots, and the lights of the frame.
    fn slot_group(&self, buffer: &wgpu::Buffer) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.draw_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer, offset: 0, size: wgpu::BufferSize::new(core::mem::size_of::<DrawData>() as u64) }),
                },
                wgpu::BindGroupEntry { binding: 1, resource: self.light_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.bone_buffer.as_entire_binding() },
            ],
        })
    }

    /// Puts the world matrices of the instances of the frame into the instance buffer.
    fn upload_instances(&mut self, instances: &[[f32; 16]]) {
        if instances.len() > self.instance_capacity {
            self.instance_capacity = instances.len().next_power_of_two();
            self.instance_buffer = instance_buffer(&self.device, self.instance_capacity);
        }
        if !instances.is_empty() {
            self.queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(instances));
        }
    }

    /// Puts the bones of the skinned draws of the frame into the bone buffer, as
    /// `upload_lights` does with the lights.
    fn upload_bones(&mut self, bones: &[[f32; 16]]) {
        if bones.len() > self.bone_capacity {
            self.bone_capacity = bones.len().next_power_of_two();
            self.bone_buffer = bone_buffer(&self.device, self.bone_capacity);
            self.draw_buffers = self.draw_buffers.iter().map(|(buffer, _)| (buffer.clone(), Rc::new(self.slot_group(buffer)))).collect();
        }
        if !bones.is_empty() {
            self.queue.write_buffer(&self.bone_buffer, 0, bytemuck::cast_slice(bones));
        }
    }

    /// Puts the lights of the frame into the light buffer: all of them each time, because
    /// a write is before the commands of the frame in any case. A larger buffer gets the
    /// bind groups of the slots again.
    fn upload_lights(&mut self, lights: &[LightData]) {
        if lights.len() > self.light_capacity {
            self.light_capacity = lights.len().next_power_of_two();
            self.light_buffer = light_buffer(&self.device, self.light_capacity);
            self.draw_buffers = self.draw_buffers.iter().map(|(buffer, _)| (buffer.clone(), Rc::new(self.slot_group(buffer)))).collect();
        }
        if !lights.is_empty() {
            self.queue.write_buffer(&self.light_buffer, 0, bytemuck::cast_slice(lights));
        }
    }

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
            let group = self.slot_group(&buffer);
            self.draw_buffers.push((Rc::new(buffer), Rc::new(group)));
        }
        let (_, group) = &self.draw_buffers[index];
        if index == self.draw_staging.len() {
            self.draw_staging.push(Vec::with_capacity(SLOTS_IN_BUFFER * SLOT_SIZE));
        }
        let staging = &mut self.draw_staging[index];
        let at = slot * SLOT_SIZE;
        if staging.len() < at + SLOT_SIZE {
            staging.resize(at + SLOT_SIZE, 0);
        }
        staging[at..at + core::mem::size_of::<DrawData>()].copy_from_slice(bytemuck::bytes_of(data));
        (group.clone(), at as u32)
    }

    /// `draw_slot`, with the data written to the GPU at once: for a draw of a pass that the
    /// extension submits before the flush of the frame (the replay of the kept draws into
    /// its reflection). The flush comes after that submit, so the pass would read the slot
    /// as the frame before left it: a draw that is not this one, by chance of the slot
    /// numbers.
    fn draw_slot_now(&mut self, data: &DrawData) -> (Rc<wgpu::BindGroup>, u32) {
        let (group, at) = self.draw_slot(data);
        let index = (self.draw_slots - 1) / SLOTS_IN_BUFFER;
        self.queue.write_buffer(&self.draw_buffers[index].0, at as u64, bytemuck::bytes_of(data));
        (group, at)
    }

    /// Writes the data of the draws since the last write to the GPU: one write for each
    /// buffer of slots. Before a submit.
    fn flush_draw_slots(&mut self) {
        for (index, staging) in self.draw_staging.iter_mut().enumerate() {
            if !staging.is_empty() {
                self.queue.write_buffer(&self.draw_buffers[index].0, 0, staging);
            }
        }
        // The slots go on after the ones written: a later write of the same buffer in the
        // frame must not undo the earlier.
        for staging in &mut self.draw_staging {
            let kept = staging.len();
            staging.clear();
            staging.resize(kept, 0);
        }
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

    /// The pipeline for a draw in screen coordinates: a frame of a film. It does not blend:
    /// the film player of the game is the only caller of `DrawPrimitiveUP`
    /// (`CutscenePlayer_static::drawRect`), and it sets `D3DRS_ALPHABLENDENABLE` to 0 before
    /// its draws (`CutscenePlayer_static::setupD3DState` 0x64ECD0).
    fn screen_pipeline(&mut self) -> wgpu::RenderPipeline {
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
        if let Some(pipeline) = &self.screen_pipeline {
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
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.screen_pipeline = Some(pipeline.clone());
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

    /// A geometry record: resident buffers for the draws that name it.
    fn geometry(&mut self, data: &[u8]) {
        let clock = std::time::Instant::now();
        let (handle, fvf, stride, vertices, entries, strips) = (word(data, 0), word(data, 12), word(data, 16), word(data, 20), word(data, 24), word(data, 28));
        let lengths = ((strips as usize * 2) + 3) & !3;
        let (vertex_at, index_at) = (32 + lengths, 32 + lengths + vertices as usize * stride as usize);
        let index_end = index_at + entries as usize * 2;
        if data.len() < index_end || stride == 0 {
            self.counts.geometry_bad += 1;
            return;
        }
        let strip_lengths: Vec<u16> = (0..strips as usize).map(|i| u16::from_le_bytes([data[32 + i * 2], data[33 + i * 2]])).collect();
        let make = |bytes: &[u8], usage: wgpu::BufferUsages| Rc::new(self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytes, usage }));
        let vertex = make(&data[vertex_at..index_at], wgpu::BufferUsages::VERTEX);
        let index = make(&data[index_at..index_end], wgpu::BufferUsages::INDEX);
        self.counts.geometry_records += 1;
        self.counts.geometry_bytes += data.len() as u64;
        self.geometries.insert(handle, Geometry { fvf, stride, entries, strips: strip_lengths, vertex, index });
        self.counts.times[TIME_BUFFERS] += clock.elapsed();
    }

    /// The geometry of a particle system for this frame, from the particles of its shape
    /// record: four vertices for each particle, a quad that faces the camera, as
    /// `NiDX8Renderer::RenderPoints` writes them (the position, the normal 0.3, the
    /// colour, the texture coordinates; 36 bytes). The quads are `(right + up) * radius`
    /// and `(up - right) * radius` from the position, scaled by the size of the particle.
    /// The buffer replaces the one of the last frame under the same key.
    fn particles(&mut self, handle: u32, radius: f32, right: [f32; 3], up: [f32; 3], particles: &[u8]) {
        let clock = std::time::Instant::now();
        const STRIDE: usize = 36;
        const NORMAL: f32 = 0.3;
        let count = particles.len() / 20;
        let sum: [f32; 3] = core::array::from_fn(|i| (right[i] + up[i]) * radius);
        let difference: [f32; 3] = core::array::from_fn(|i| (up[i] - right[i]) * radius);
        let mut bytes = Vec::with_capacity(count * 4 * STRIDE);
        for p in particles.chunks_exact(20) {
            let position = [float(p, 0), float(p, 4), float(p, 8)];
            let size = float(p, 12);
            let colour = &p[16..20];
            let s: [f32; 3] = core::array::from_fn(|i| sum[i] * size);
            let d: [f32; 3] = core::array::from_fn(|i| difference[i] * size);
            let corners = [
                ([position[0] - s[0], position[1] - s[1], position[2] - s[2]], [0.0f32, 0.0f32]),
                ([position[0] - d[0], position[1] - d[1], position[2] - d[2]], [1.0, 0.0]),
                ([s[0] + position[0], s[1] + position[1], s[2] + position[2]], [1.0, 1.0]),
                ([d[0] + position[0], d[1] + position[1], d[2] + position[2]], [0.0, 1.0]),
            ];
            for (at, uv) in corners {
                for v in at.iter().chain([NORMAL; 3].iter()) {
                    bytes.extend_from_slice(&v.to_le_bytes());
                }
                bytes.extend_from_slice(colour);
                bytes.extend_from_slice(&uv[0].to_le_bytes());
                bytes.extend_from_slice(&uv[1].to_le_bytes());
            }
        }
        let quads = count as u32;
        if self.quad_indexes.as_ref().is_none_or(|(have, _)| *have < quads) {
            let indexes: Vec<u8> = (0..quads).flat_map(|q| [0u16, 1, 2, 0, 2, 3].map(|i| (q * 4) as u16 + i)).flat_map(|i| i.to_le_bytes()).collect();
            let buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("quads"), contents: &indexes, usage: wgpu::BufferUsages::INDEX });
            self.quad_indexes = Some((quads, Rc::new(buffer)));
        }
        let index = self.quad_indexes.as_ref().map(|(_, buffer)| buffer.clone()).expect("the quad index buffer was just made");
        let vertex = Rc::new(self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("particles"), contents: &bytes, usage: wgpu::BufferUsages::VERTEX }));
        self.counts.shape_particles += 1;
        // D3DFVF_XYZ | D3DFVF_NORMAL | D3DFVF_DIFFUSE | D3DFVF_TEX1.
        let geometry = Geometry { fvf: 0x152, stride: STRIDE as u32, entries: quads * 6, strips: Vec::new(), vertex, index };
        self.geometries.insert(handle, geometry);
        self.counts.times[TIME_BUFFERS] += clock.elapsed();
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
            // The world matrix of an instance: four columns at the locations 9 to 12.
            let from_instances: Vec<wgpu::VertexAttribute> =
                (0..4).map(|column| wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: column as u64 * 16, shader_location: 9 + column }).collect();
            let buffers = [
                Some(wgpu::VertexBufferLayout { array_stride: key.stride as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &from_vertex }),
                Some(wgpu::VertexBufferLayout { array_stride: 64, step_mode: wgpu::VertexStepMode::Instance, attributes: &from_defaults }),
                Some(wgpu::VertexBufferLayout { array_stride: 64, step_mode: wgpu::VertexStepMode::Instance, attributes: &from_instances }),
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

/// The facts of the shape record in progress, or of the last one: what is the same for each
/// pass of its draw.
struct ShapeFacts {
    /// The geometry handle of the record.
    geometry: u32,
    fvf: u32,
    stride: u32,
    world: [f32; 16],
    /// The world matrix is relative to the origin of the scene.
    relative: bool,
    /// The `D3DMATERIAL8`: diffuse, ambient, specular, emissive with the alpha, then power.
    material: [f32; 17],
    /// `D3DRS_AMBIENT` of the draw.
    ambient: u32,
    /// The fill mode of the material (D3DFILLMODE: 2 lines, 3 solid).
    fill: u32,
    /// The lights of the draw from their records: the key, then the `D3DLIGHT8`,
    /// `STATE_LIGHT_SIZE` bytes each.
    lights: Vec<u8>,
}

/// What the stream said before a draw.
struct Stream {
    view: [f32; 16],
    projection: [f32; 16],
    viewport: Option<([f32; 4], [f32; 2])>,
    back_buffer: bool,
    origin: Option<[f64; 3]>,
    /// The player is in an interior cell, from the scene packet.
    interior: bool,
    /// The game hides the sky (the sky root is culled), from the scene packet: the world
    /// scene has no sky draw.
    sky_hidden: bool,
    /// The fog distances of the extension for the draws of the world, or `None`.
    fog_range: Option<(f32, f32)>,
    /// The class of the object of the draws that follow: MGE_LINK_CLASS_* of its packet.
    object_class: u32,
    /// The sky values of the extension for the draw in progress, or `None`.
    sky: Option<[[f32; 4]; 8]>,
    shape: ShapeFacts,
    /// The fog of the shape record in progress (flag 32): the colour, the start, the end
    /// and the range fog flag, as the render state of the game gets them from the fog
    /// property of the draw. `None` for a draw without fog.
    shape_fog: Option<[u32; 4]>,
    /// The fog of the last shape record that had one: the fog states of the device of the
    /// game stay as the last draw with fog set them.
    fog: [u32; 4],
    /// `D3DRS_AMBIENTMATERIALSOURCE` as the last shape record with a value for it set it:
    /// the light manager of the game leaves the state of the draw before for a material
    /// without one (see `material::Derived::sources`).
    ambient_source: u32,
    /// The bones of the skinned shape in progress in the bones of the frame: the first and
    /// the number, or `None` for a draw that is not skinned.
    shape_bones: Option<(u32, u32)>,
}

impl Stream {
    /// The data of one pass of the shape in progress for the shader.
    fn draw_data(&self, derived: &material::Derived, pass: &Pass, counts: &mut Counts, frame_lights: &mut Vec<LightData>) -> DrawData {
        let shape = &self.shape;
        let mut data = DrawData::zeroed();
        let fvf = shape.fvf;
        // The world matrix is relative to the camera when the scene has an origin and the
        // record says so. Then the camera is at 0.
        let relative = self.origin.filter(|_| shape.relative);
        data.world[0] = shape.world;
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

        let material = &shape.material;
        data.material_diffuse = material[0..4].try_into().unwrap();
        data.material_ambient = material[4..8].try_into().unwrap();
        data.material_emissive = material[12..16].try_into().unwrap();
        data.scene_ambient = color(known(shape.ambient, 0));
        // The fog of the draw: the colour, the start, the end, the range fog flag.
        if let (true, Some(fog)) = (derived.fog, self.shape_fog) {
            data.fog_color = color(fog[0]);
            data.fog = [f32::from_bits(fog[1]), f32::from_bits(fog[2]), 1.0, (fog[3] != 0) as u32 as f32];
            // The fog of the extension is for the world only: a perspective projection.
            if let (Some((start, end)), true) = (self.fog_range, self.projection[11] == 1.0) {
                data.fog[0] = start;
                data.fog[1] = end;
                // The fog of the extension is on the distance from the camera, as the fog
                // of what the extension draws: the two do not change with the direction
                // of the camera.
                data.fog[3] = 1.0;
            }
        }
        // D3DRS_TEXTUREFACTOR: white, the value of a new texture stage of the game. No stage
        // of a derived state has the factor as an argument or as its blend.
        data.texture_factor = [1.0; 4];
        if let Some(sky) = self.sky {
            data.extension = sky;
            if pass.stages[0].texture == 0 {
                // The sky dome has no texture.
                data.counts[3] |= 4;
            } else if self.object_class == 6 && pass.blend[2] == 6 {
                // MGE_LINK_CLASS_SKY_MOON_SHADOW with D3DBLEND_INVSRCALPHA: the disc that keeps
                // the stars out of a moon.
                data.counts[3] |= 8;
            }
        }

        let has_color = fvf & 0x40 != 0;
        data.flags = [derived.lighting as u32, (fvf & 0x10 != 0) as u32, has_color as u32, 0];
        let alpha_test = pass.alpha_test.unwrap_or(derived.alpha_test);
        // D3DMCS_COLOR1 (1) is the default of the diffuse source and D3DMCS_MATERIAL (0) of
        // the emissive source.
        data.more = [
            if has_color { derived.sources[0].unwrap_or(1) } else { 0 },
            if has_color { derived.sources[1].unwrap_or(0) } else { 0 },
            if alpha_test[0] != 0 { alpha_test[1] } else { 0 },
            alpha_test[2],
        ];

        let lights = shape.lights.len() / STATE_LIGHT_SIZE;
        counts.most_lights = counts.most_lights.max(lights as u32);
        data.light_range = [frame_lights.len() as u32, lights as u32, 0, 0];
        data.counts[0] = lights as u32;
        if let Some((first, _)) = self.shape_bones {
            // A skinned shape: the vertex blend reads four bones of the palette of the frame.
            data.flags[3] = 4;
            data.light_range[2] = first;
        }
        for light in shape.lights.chunks_exact(STATE_LIGHT_SIZE) {
            // The `D3DLIGHT8` after the key: the type, the diffuse, specular and ambient
            // colours, the position, the direction, the range, the falloff, the three
            // attenuations and the two angles of a spot light.
            let l = &light[4..];
            let mut position: [f32; 3] = floats(l, 13 * 4);
            if let Some(origin) = relative {
                for axis in 0..3 {
                    position[axis] = (position[axis] as f64 - origin[axis]) as f32;
                }
            }
            let direction: [f32; 3] = floats(l, 16 * 4);
            let attenuation: [f32; 3] = floats(l, 21 * 4);
            frame_lights.push(LightData {
                diffuse: floats(l, 4),
                ambient: floats(l, 9 * 4),
                position: [position[0], position[1], position[2], word(l, 0) as f32],
                direction: [direction[0], direction[1], direction[2], float(l, 19 * 4)],
                attenuation: [attenuation[0], attenuation[1], attenuation[2], float(l, 20 * 4)],
                // The plugin puts the radius of a light of the game into its first specular word.
                cone: [(float(l, 24 * 4) * 0.5).cos(), (float(l, 25 * 4) * 0.5).cos(), float(l, 5 * 4), 0.0],
            });
        }

        let mut stages = 0;
        while stages < 8 {
            let stage = &pass.stages[stages];
            if stage.color[0] == 1 {
                break;
            }
            if stages < STAGES {
                // The coordinate index has the way to generate it in its high word. The
                // transform flags are the number of parts, and 256 for a projected coordinate.
                let (coordinate, transform) = (stage.coordinate, stage.transform);
                if coordinate >> 16 != 0 || transform != 0 {
                    counts.coordinate_work += 1;
                }
                // The result argument of a stage is D3DTA_CURRENT (1).
                data.stages[stages] = StageData {
                    color: [stage.color[0], stage.color[1], stage.color[2], 1],
                    alpha: [stage.alpha[0], stage.alpha[1], stage.alpha[2], 0],
                    coordinate: [coordinate >> 16 & 3, coordinate & 3, transform & 7, (transform & 256 != 0) as u32],
                    bump: [stage.bump[0], stage.bump[1], stage.bump[2], stage.bump[3]],
                    luminance: [stage.bump[4], stage.bump[5], 0.0, 0.0],
                };
                data.texture_transforms[stages] = stage.texture_transform;
            }
            stages += 1;
        }
        if stages > STAGES {
            counts.more_stages += 1;
        }
        data.counts[1] = stages.min(STAGES) as u32;
        data.counts[2] = if has_color { self.ambient_source } else { 0 };
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
    /// A part of the frame in progress went to the GPU before its end, so the back buffer
    /// has a picture that is not complete. `last_picture` then has the last complete one.
    sent_in_part: bool,
    last_picture: Option<wgpu::Texture>,
    /// Takes the answer to a pixel read: the number of the request, the width, the height
    /// and the pixels with 4 bytes each, blue first. `None` without a game to answer.
    reply: Option<Box<dyn FnMut(u32, u32, u32, &[u8])>>,
    /// The picture files that the game asked for: the frame in progress goes into them.
    shots: Vec<String>,
    /// The pictures of the frame from before the interface of the game, for an extension:
    /// the buffer, the length of a row, the size, the file. Saved at the end of the frame.
    early_pictures: Vec<(wgpu::Buffer, usize, u32, u32, String)>,
    /// The last `OVERLAY` record of the stream, for the live mode to take: the debug overlay
    /// on or off, with the numbers of the game process.
    overlay_record: Option<Vec<u8>>,
    /// A picture file for a frame with a draw of the film player (`--shot-film`, the
    /// `--shot-film-frame`th such draw: a film starts from black): a film holds the main loop
    /// of the game, so the game cannot ask for it in time.
    film_shot: Option<String>,
    film_shot_at: u32,
    film_draws: u32,
    /// A program that draws more than the game does.
    extension: Option<Box<dyn Extension>>,
    /// The items of the frame that are not done yet, and the world draws of the frame for
    /// `Replay`.
    items: Vec<Item>,
    world_draws: Vec<Rc<Draw>>,
    /// The draws of the world scene that are not the sky and came before the sky (see
    /// `draw`): they go into the items after the start of the world.
    held_draws: Vec<Item>,
    /// The shapes of the frame that the extension took, for `Extension::draw_held`.
    taken: Vec<HeldDraw>,
    /// The lights of all draws of the frame, in the order of the draws.
    frame_lights: Vec<LightData>,
    /// The bones of the skinned draws of the frame, for the bone buffer.
    frame_bones: Vec<[f32; 16]>,
    /// The world matrices of the instances of the frame; the first is an identity, for
    /// the draws of their own.
    frame_instances: Vec<[f32; 16]>,
    /// The collect: the answers of the extension for the frame (`None` before the world
    /// scene), the world began, the world ended, the kind of the scene in progress (1 is the
    /// world), the sun and the ambient light of the first draw of the world, the texture
    /// that the game draws into (`None` for the back buffer), and whether the game named a
    /// rendered texture as its target.
    parameters: Option<FrameParameters>,
    world_begun: bool,
    /// A draw of a sky class went into the back buffer in the world scene of this frame.
    sky_seen: bool,
    world_ended: bool,
    /// The classes that the extension took in this frame, one bit for each.
    replaced: u32,
    /// The plane of `FrameParameters::class_plane` when its call is in the items of the
    /// frame: before the call the world draws that blend and write no depth are drawn only
    /// on its far side, and after it (`split`) only on the side of the camera.
    plane_ahead: Option<[f64; 4]>,
    split: Option<[f64; 4]>,
    /// The post-processing of this frame is in the items.
    posted: bool,
    /// The view and the projection of the last world draw of the frame: the matrices of
    /// the world for a call after the world views, when the stream has those of the user
    /// interface.
    world_matrices: Option<([f32; 16], [f32; 16])>,
    scene_kind: u32,
    world_sun: Option<([f32; 3], [f32; 3], [f32; 3])>,
    world_ambient: [f32; 3],
    target: Option<u32>,
    wants_texture: bool,
    /// The pass: its target, its size, its viewport, and the picture after
    /// `Extension::before_world` for `Extension::blend` with true when it is of this frame.
    pass_target: Option<u32>,
    target_size: (u32, u32),
    viewport: Option<([f32; 4], [f32; 2])>,
    before: Option<wgpu::Texture>,
    before_kept: bool,
    /// The picture and the depth of the frame before a call of `Extension::draw_class`.
    picture_copy: Option<wgpu::Texture>,
    depth_copy: Option<wgpu::Texture>,
    /// The depth of the game at the end of its world scene, for `Extension::post`: the game
    /// clears its depth before its later scenes.
    world_depth: Option<wgpu::Texture>,
    world_depth_kept: bool,
    /// The depth of the draws of `Extension::before_world`, for `Extension::post`: the
    /// renderer clears the depth after them.
    before_depth: Option<wgpu::Texture>,
    before_depth_kept: bool,
    /// The counts at the last mark. See `tag::MARK`.
    mark_times: [std::time::Duration; TIME_COUNT],
    mark_frames: u32,
    mark_draws: u32,
    mark_gpu: (f64, f64, u32),
    /// The size of the back buffer of the game, from the first record. 0 when not known.
    size: (u32, u32),
    /// The size of the picture of the frame.
    frame_size: (u32, u32),
}

impl Player {
    fn new(renderer: Renderer) -> Player {
        Player {
            renderer,
            stream: Stream {
                view: [0.0; 16],
                projection: [0.0; 16],
                viewport: None,
                back_buffer: true,
                origin: None,
                interior: false,
                sky_hidden: false,
                fog_range: None,
                object_class: 0,
                sky: None,
                // D3DFILL_SOLID is 3.
                shape: ShapeFacts { geometry: 0, fvf: 0, stride: 0, world: [0.0; 16], relative: false, material: [0.0; 17], ambient: 0, fill: 3, lights: Vec::new() },
                shape_fog: None,
                fog: [0; 4],
                ambient_source: 0,
                shape_bones: None,
            },
            encoder: None,
            pass: None,
            color: None,
            depth: None,
            back_buffer: None,
            sent_in_part: false,
            last_picture: None,
            reply: None,
            shots: Vec::new(),
            early_pictures: Vec::new(),
            overlay_record: None,
            film_shot: None,
            film_shot_at: 1,
            film_draws: 0,
            extension: None,
            items: Vec::new(),
            world_draws: Vec::new(),
            held_draws: Vec::new(),
            taken: Vec::new(),
            frame_lights: Vec::new(),
            frame_bones: Vec::new(),
            frame_instances: Vec::new(),
            parameters: None,
            world_begun: false,
            sky_seen: false,
            world_ended: false,
            replaced: 0,
            plane_ahead: None,
            split: None,
            posted: false,
            world_matrices: None,
            scene_kind: 0,
            world_sun: None,
            world_ambient: [0.0; 3],
            target: None,
            wants_texture: false,
            pass_target: None,
            target_size: (0, 0),
            viewport: None,
            before: None,
            before_kept: false,
            picture_copy: None,
            depth_copy: None,
            world_depth: None,
            world_depth_kept: false,
            before_depth: None,
            before_depth_kept: false,
            mark_times: [std::time::Duration::ZERO; TIME_COUNT],
            mark_frames: 0,
            mark_draws: 0,
            mark_gpu: (0.0, 0.0, 0),
            size: (0, 0),
            frame_size: (0, 0),
        }
    }

    /// Starts a pass on the targets of the frame: it draws on what is there, or it clears first.
    fn open_pass(&mut self, clear_color: Option<wgpu::Color>, clear_depth: Option<f32>, clear_stencil: Option<u32>) {
        drop(self.pass.take());
        let rendered = self.pass_target.and_then(|handle| self.renderer.targets.get(&handle));
        let views = match rendered {
            Some(texture) => Some((texture.view.clone(), texture.depth.clone(), texture.size)),
            None => self.color.clone().zip(self.depth.clone()).map(|(color, depth)| (color, depth, self.frame_size)),
        };
        let (Some(encoder), Some((color, depth, size))) = (self.encoder.as_mut(), views) else { return };
        self.target_size = size;
        let (color, depth) = (&*color, &*depth);
        let stamps = self.renderer.timing.as_mut().and_then(Timing::next_pass);
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
                timestamp_writes: stamps.map(|index| wgpu::RenderPassTimestampWrites {
                    query_set: &self.renderer.timing.as_ref().unwrap().set,
                    beginning_of_pass_write_index: Some(2 * index),
                    end_of_pass_write_index: Some(2 * index + 1),
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        self.pass = Some(pass);
        self.apply_viewport();
    }

    /// Gives the viewport of the stream to the pass, cut to the size of the targets.
    fn apply_viewport(&mut self) {
        let (Some(pass), Some((rectangle, range))) = (self.pass.as_mut(), self.viewport) else { return };
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
        self.items.clear();
        self.world_draws.clear();
        self.frame_lights.clear();
        self.renderer.extension_lights.clear();
        self.frame_bones.clear();
        self.frame_instances.clear();
        self.frame_instances.push(core::array::from_fn(|i| if i % 5 == 0 { 1.0 } else { 0.0 }));
        self.parameters = None;
        self.world_begun = false;
        self.sky_seen = false;
        self.held_draws.clear();
        self.taken.clear();
        self.world_ended = false;
        self.replaced = 0;
        self.plane_ahead = None;
        self.split = None;
        self.posted = false;
        self.world_matrices = None;
        self.world_depth_kept = false;
        self.before_depth_kept = false;
        self.target = None;
        self.pass_target = None;
        self.before_kept = false;
        self.stream.fog_range = None;
        self.stream.sky = None;
        self.stream.back_buffer = true;
        self.sent_in_part = false;
        let mut encoder = self.renderer.device.create_command_encoder(&Default::default());
        if let Some(timing) = self.renderer.timing.as_mut() {
            timing.begin(&mut encoder);
        }
        self.encoder = Some(encoder);
        self.renderer.draw_slots = 0;
        self.renderer.draw_staging.clear();
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
        self.run_items();
        let in_frame = self.drawing();
        drop(self.pass.take());
        if let Some(encoder) = self.encoder.take() {
            self.renderer.flush_draw_slots();
            // The first part of a frame: the back buffer has the last complete picture
            // until these commands run. A copy of it stays for the window (see
            // `shown_picture`).
            let copy = (in_frame && !std::mem::replace(&mut self.sent_in_part, true)).then(|| self.keep_picture()).flatten();
            self.renderer.queue.submit(copy.into_iter().chain([encoder.finish()]));
        }
        let result = between(self);
        if in_frame {
            self.encoder = Some(self.renderer.device.create_command_encoder(&Default::default()));
            self.open_pass(None, None, None);
        }
        result
    }

    /// The commands that copy the back buffer into `last_picture`.
    fn keep_picture(&mut self) -> Option<wgpu::CommandBuffer> {
        let source = self.back_buffer.as_ref()?;
        let (width, height) = (source.width(), source.height());
        if self.last_picture.as_ref().is_none_or(|kept| (kept.width(), kept.height()) != (width, height)) {
            self.last_picture = Some(target_texture(&self.renderer.device, width, height, source.format(), wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC));
        }
        let mut encoder = self.renderer.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_texture(source.as_image_copy(), self.last_picture.as_ref()?.as_image_copy(), wgpu::Extent3d { width, height, depth_or_array_layers: 1 });
        Some(encoder.finish())
    }

    /// The last complete picture of the game: the back buffer, or the copy from before the
    /// frame in progress when a part of that frame is in the back buffer.
    fn shown_picture(&self) -> Option<&wgpu::Texture> {
        if self.sent_in_part { self.last_picture.as_ref() } else { self.back_buffer.as_ref() }
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

    /// A shape record: the draws of the object from its geometry and material records, one
    /// for each pass of the state derived from its material.
    fn shape(&mut self, data: &[u8]) {
        let (geometry, material, flags, light_count, ambient) = (word(data, 0), word(data, 4), word(data, 8), word(data, 12) as usize, word(data, 16));
        self.renderer.counts.shape_records += 1;
        let colours: [f32; 14] = core::array::from_fn(|i| float(data, 84 + i * 4));
        let bone_count = word(data, 140) as usize;
        let lights_at = 20 + 64 + 56 + 4;
        // The stages of the texture effects (flag 16): the environment map, the fog map.
        let effects_at = lights_at + light_count * 4;
        let has_effects = flags & 16 != 0;
        // The fog of the draw (flag 32): colour, start, end, range fog.
        let fog_at = effects_at + if has_effects { 2 * EFFECT_STAGE_WORDS * 4 } else { 0 };
        let has_fog = flags & 32 != 0;
        let bones_at = fog_at + if has_fog { 16 } else { 0 };
        if data.len() < bones_at + bone_count * 64 {
            self.renderer.counts.shape_missing += 1;
            return;
        }
        let effects: Option<[[u32; EFFECT_STAGE_WORDS]; 2]> = has_effects.then(|| core::array::from_fn(|e| core::array::from_fn(|i| word(data, effects_at + (e * EFFECT_STAGE_WORDS + i) * 4))));
        self.stream.shape_fog = has_fog.then(|| core::array::from_fn(|i| word(data, fog_at + i * 4)));
        if let Some(fog) = self.stream.shape_fog {
            self.stream.fog = fog;
        }
        // The lights of the draw from their records: the key, then the D3DLIGHT8.
        let lights = &mut self.stream.shape.lights;
        lights.clear();
        for i in 0..light_count {
            let key = word(data, lights_at + i * 4);
            match self.renderer.lights_known.get(&key) {
                Some(values) => {
                    lights.extend_from_slice(&key.to_le_bytes());
                    lights.extend_from_slice(values);
                }
                None => self.renderer.counts.light_missing += 1,
            }
        }
        self.stream.shape.ambient = ambient;
        // The bones of a skinned shape go to the palette of the frame.
        self.stream.shape_bones = (bone_count != 0).then(|| {
            let first = self.frame_bones.len() as u32;
            for b in 0..bone_count {
                self.frame_bones.push(floats(data, bones_at + b * 64));
            }
            self.renderer.counts.shape_skinned += 1;
            (first, bone_count as u32)
        });
        // A particle system: its geometry for the frame comes from the record itself.
        if flags & 8 != 0 {
            let tail = bones_at + bone_count * 64;
            let count = if data.len() >= tail + 32 { word(data, tail) as usize } else { 0 };
            let at = tail + 32;
            if count == 0 || data.len() < at + count * 20 {
                self.renderer.counts.shape_missing += 1;
                return;
            }
            let axes: [f32; 6] = core::array::from_fn(|i| float(data, tail + 8 + i * 4));
            self.renderer.particles(geometry, float(data, tail + 4), [axes[0], axes[1], axes[2]], [axes[3], axes[4], axes[5]], &data[at..at + count * 20]);
        }
        let Some((fvf, stride, entries, strips)) = self.renderer.geometries.get(&geometry).map(|g| (g.fvf, g.stride, g.entries, g.strips.clone())) else {
            self.renderer.counts.shape_missing += 1;
            return;
        };
        let sets = (fvf >> 8) & 0xF;
        let Some(m) = self.renderer.materials.get(&material).cloned() else {
            self.renderer.counts.shape_missing += 1;
            return;
        };
        self.renderer.counts.shape_flat += (m.flags & material::SMOOTH == 0) as u32;
        self.renderer.counts.shape_specular += (m.flags & material::SPECULAR != 0) as u32;
        let derived = match self.renderer.derived.get(&(material, sets)) {
            Some(d) => d.clone(),
            None => {
                let d = Rc::new(material::derive(&m, sets));
                self.renderer.derived.insert((material, sets), d.clone());
                d
            }
        };
        let shape = &mut self.stream.shape;
        shape.geometry = geometry;
        shape.fvf = fvf;
        shape.stride = stride;
        shape.world = core::array::from_fn(|i| float(data, 20 + i * 4));
        shape.relative = flags & 1 != 0;
        shape.material = material::d3d_material(&colours);
        shape.fill = derived.fill;
        if let Some(source) = derived.sources[2] {
            self.stream.ambient_source = source;
        }
        if self.hold_shape(&m, &derived, flags, effects.as_ref(), entries, &strips) {
            self.stream.shape_bones = None;
            return;
        }
        for pass in 0..derived.passes.len() {
            let state = Pass::new(&derived, pass, &m, &self.renderer.textures, effects.as_ref());
            // The flag 128: the draw goes into the picture of the character with the Code
            // Patch, which draws a pass that blends in a way of its own. The alpha of the
            // material is the last of its colours.
            let (state, second) = if flags & 128 != 0 { state.code_patch_picture(colours[13]) } else { (state, None) };
            // The primitive type of Direct3D, the first index and the number of indexes.
            // The flag 64: lines, as a line list of the index entries of the geometry.
            if flags & 64 != 0 {
                for state in core::iter::once(&state).chain(&second) {
                    self.draw(&derived, state, 2, 0, entries / 2 * 2);
                }
            } else if strips.is_empty() {
                for state in core::iter::once(&state).chain(&second) {
                    self.draw(&derived, state, 4, 0, entries / 3 * 3);
                }
            } else {
                // The Code Patch makes its draws at each draw call: for each strip.
                let mut start = 0u32;
                for &length in &strips {
                    for state in core::iter::once(&state).chain(&second) {
                        self.draw(&derived, state, 5, start, (length as u32).max(2));
                    }
                    start += length as u32;
                }
            }
            self.renderer.counts.shape_draws += 1;
        }
        self.stream.shape_bones = None;
    }

    /// Gives the shape in progress to the extension when it takes it: see `Extension::hold`.
    /// True when the extension took it; then none of its passes is drawn.
    fn hold_shape(&mut self, m: &material::Material, derived: &material::Derived, flags: u32, effects: Option<&[[u32; EFFECT_STAGE_WORDS]; 2]>, entries: u32, strips: &[u16]) -> bool {
        let class = self.stream.object_class;
        // Flag 8 is a particle system, 64 lines, 128 a draw into the picture of the
        // character. Classes 1 to 8 are the sky and the glare of the sun, 10 the water plane.
        let world = self.extension.is_some()
            && self.stream.back_buffer
            && self.target.is_none()
            && matches!(self.scene_kind, 1..=3)
            && !(1..=8).contains(&class)
            && class != 10
            && self.stream.projection[11] == 1.0
            && flags & (8 | 64 | 128) == 0
            && self.stream.shape_bones.is_none()
            && !derived.passes.is_empty();
        if !world {
            return false;
        }
        // The answers of the frame, as `draw` asks for them at the first draw of the world
        // scene.
        if self.parameters.is_none() && self.scene_kind == 1 {
            self.ask_frame();
        }
        let Some((true, fog_range)) = self.parameters.as_ref().map(|p| (p.hold, p.fog_range)) else { return false };
        let shape = &self.stream.shape;
        let Some(layout) = VertexLayout::of(shape.fvf, shape.stride) else { return false };
        let Some((vertices, indexes)) = self.renderer.geometries.get(&shape.geometry).filter(|g| entries <= g.entries).map(|g| (g.vertex.clone(), g.index.clone())) else { return false };
        let pass = Pass::new(derived, 0, m, &self.renderer.textures, effects);
        let facts = DrawFacts {
            class,
            blend_enable: pass.blend[0] != 0,
            source_blend: pass.blend[1],
            destination_blend: pass.blend[2],
            depth_write: derived.depth[1] != 0,
            specular_power: shape.material[16],
            emissive: shape.material[12..16].try_into().unwrap(),
        };
        if !self.extension.as_mut().is_some_and(|extension| extension.hold(&facts)) {
            return false;
        }
        let (topology, parts) = if strips.is_empty() {
            (wgpu::PrimitiveTopology::TriangleList, vec![(0, entries / 3 * 3)])
        } else {
            let mut start = 0u32;
            let parts = strips
                .iter()
                .map(|&length| {
                    let part = (start, (length as u32).max(2));
                    start += length as u32;
                    part
                })
                .collect();
            (wgpu::PrimitiveTopology::TriangleStrip, parts)
        };
        let sets = layout.coordinates.len() as u32;
        let maps = m
            .maps
            .iter()
            .enumerate()
            .filter(|(_, map)| map.present && map.texture != 0)
            .filter_map(|(index, map)| {
                let texture = self.renderer.textures.get(&map.texture)?.clone();
                let mut sampler = material::sampler_states(map.clamp, map.filter);
                // A texture with one level gets no mip filter, as the texture manager sets it.
                if texture.texture().mip_level_count() <= 1 {
                    sampler[4] = 0;
                }
                Some(HeldMap { map: index, texture, set: map.set.min(sets.saturating_sub(1)), sampler })
            })
            .collect();
        self.stream.fog_range = fog_range;
        self.stream.sky = None;
        let mut lights = Vec::new();
        let data = self.stream.draw_data(derived, &pass, &mut self.renderer.counts, &mut lights);
        let origin = self.stream.origin.filter(|_| self.stream.shape.relative);
        let held = HeldDraw {
            facts,
            vertices,
            indexes,
            layout,
            topology,
            parts,
            data,
            origin,
            lights,
            maps,
            depth: [(derived.depth[0] != 0) as u32, (derived.depth[1] != 0) as u32, derived.depth[2]],
            cull: derived.cull,
            stencil: derived.stencil,
        };
        if self.extension.as_ref().is_some_and(|extension| extension.held_in_place(&held)) {
            let facts = self.world_facts(false);
            self.items.push(Item::HeldHere(Box::new(held), Box::new(facts)));
        } else {
            self.taken.push(held);
        }
        self.renderer.counts.held += 1;
        true
    }

    /// Prints the counts since the last mark, for one step of a scenario.
    fn mark(&mut self, text: &str) {
        let counts = &self.renderer.counts;
        let frames = counts.frames - self.mark_frames;
        let draws = counts.draws - self.mark_draws;
        let busy: std::time::Duration = counts.times.iter().zip(self.mark_times).map(|(now, before)| *now - before).sum();
        let per_frame = |time: std::time::Duration| time.as_secs_f64() * 1000.0 / frames.max(1) as f64;
        println!("mark {text}: {frames} frames, {:.1} draws for each frame, busy {:.3} ms for each frame:", draws as f64 / frames.max(1) as f64, per_frame(busy));
        for ((name, now), before) in TIME_NAMES.iter().zip(counts.times).zip(self.mark_times) {
            println!("  {:6.3}  {name}", per_frame(now - before));
        }
        let timed = counts.gpu_frames - self.mark_gpu.2;
        if timed != 0 {
            let per_frame = |now: f64, before: f64| (now - before) / 1e6 / timed as f64;
            println!("  gpu: {:.3} ms in the passes, {:.3} ms the frame, for each of {timed} timed frames", per_frame(counts.gpu_passes, self.mark_gpu.0), per_frame(counts.gpu_frame, self.mark_gpu.1));
        }
        self.mark_gpu = (counts.gpu_passes, counts.gpu_frame, counts.gpu_frames);
        self.mark_times = counts.times;
        self.mark_frames = counts.frames;
        self.mark_draws = counts.draws;
    }

    /// Ends the frame and sends its commands to the GPU. `before_submit` can add commands,
    /// for example a copy of the picture.
    fn end_frame(&mut self, before_submit: impl FnOnce(&mut wgpu::CommandEncoder)) {
        self.post_item();
        self.run_items();
        // The pictures that an extension wants and did not get after its post-processing.
        if let Some(extension) = self.extension.as_mut() {
            self.shots.extend(extension.pictures().into_iter().map(|request| request.path));
        }
        let clock = std::time::Instant::now();
        drop(self.pass.take());
        self.color = None;
        self.depth = None;
        if let Some(mut encoder) = self.encoder.take() {
            before_submit(&mut encoder);
            let renderer = &mut self.renderer;
            let slot = renderer.timing.as_mut().and_then(|timing| timing.end(&renderer.device, &mut encoder, &mut renderer.counts));
            renderer.flush_draw_slots();
            renderer.queue.submit([encoder.finish()]);
            if let (Some(timing), Some(slot)) = (renderer.timing.as_ref(), slot) {
                timing.map(slot);
            }
        }
        let blue_first = matches!(self.renderer.target_format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
        for (buffer, row, width, height, path) in std::mem::take(&mut self.early_pictures) {
            save_picture(&self.renderer.device, &buffer, row, width, height, blue_first, &path);
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
            tag::MARK => self.mark(&String::from_utf8_lossy(data)),
            tag::OVERLAY => self.overlay_record = Some(data.to_vec()),
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
            tag::GEOMETRY if data.len() >= 32 => self.renderer.geometry(data),
            tag::MATERIAL if data.len() >= 4 => {
                if let Some(parsed) = material::Material::parse(&data[4..]) {
                    let handle = word(data, 0);
                    // A record with a handle that is known has the same content: a hold of a
                    // capture file sends it again.
                    self.renderer.materials.entry(handle).or_insert(parsed);
                    self.renderer.counts.material_records += 1;
                }
            }
            tag::SCENE_BEGIN => {
                self.stream.origin = (word(data, 20) & 1 != 0).then(|| [double(data, 24), double(data, 32), double(data, 40)]);
                self.stream.interior = word(data, 20) & 2 != 0;
                self.stream.sky_hidden = word(data, 20) & 4 != 0;
                self.scene_kind = word(data, 4);
                // The user interface scene of the game: the world views are done.
                if self.scene_kind == 7 {
                    self.post_item();
                }
            }
            tag::SCENE_END => {
                let world_scene = self.scene_kind == 1 && self.target.is_none() && self.drawing();
                // A world scene in which the game drew nothing of the world (the console
                // command ToggleWorld culls the world roots; a view with nothing in it): the
                // world starts at the end of the scene, so that the extension draws its
                // distant land under the sky. MGE XE on Direct3D 9 does the same: its stage
                // 0 runs from `EndScene` when no draw started it (mged3d8device.cpp 466).
                if self.extension.is_some() && world_scene && !self.world_begun {
                    if self.parameters.is_none() {
                        self.ask_frame();
                    }
                    self.world_begun = true;
                    let facts = self.world_facts(true);
                    self.items.push(Item::WorldStart(Box::new(facts)));
                }
                // The draws held for the start of the world, when the start came only here.
                if self.scene_kind == 1 {
                    self.items.append(&mut self.held_draws);
                }
                // The end of the world scene of the game, in the back buffer.
                if world_scene && self.world_begun && !self.world_ended {
                    self.world_ended = true;
                    let facts = self.world_facts(false);
                    self.items.push(Item::WorldEnd(Box::new(facts)));
                }
                self.scene_kind = 0;
            }
            tag::FRAME_EVENT => {
                if word(data, 0) == EVENT_SET_TARGET {
                    self.wants_texture = word(data, 4) != 0;
                }
            }
            // The kind and the class of the object of the shape records that follow.
            tag::OBJECT if data.len() >= 8 => {
                self.stream.object_class = word(data, 4);
                self.stream.shape_fog = None;
                self.stream.shape_bones = None;
            }
            tag::SHAPE if data.len() >= 144 => self.shape(data),
            tag::LIGHT if data.len() >= 108 => {
                self.renderer.lights_known.insert(word(data, 0), data[4..108].try_into().unwrap());
                self.renderer.counts.light_records += 1;
            }
            tag::FRAME_END => {
                self.renderer.counts.frames += 1;
                return true;
            }
            tag::CALL => {
                let arguments = &data[16..];
                match word(data, 0) {
                    CALL_VIEWPORT => {
                        let viewport = &arguments[word(data, 12) as usize * 4..];
                        if viewport.len() >= 24 {
                            let rectangle = [word(viewport, 0) as f32, word(viewport, 4) as f32, word(viewport, 8) as f32, word(viewport, 12) as f32];
                            let viewport = (rectangle, [float(viewport, 16), float(viewport, 20)]);
                            self.stream.viewport = Some(viewport);
                            if self.stream.back_buffer {
                                self.items.push(Item::Viewport(viewport));
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
                                self.items.push(Item::Target(texture));
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
                            // The alpha of the back buffer is 1: see the colour write of a draw.
                            let c = color(word(arguments, 12));
                            wgpu::Color { r: c[0] as f64, g: c[1] as f64, b: c[2] as f64, a: 1.0 }
                        });
                        let clear_depth = (flags & 2 != 0).then(|| float(arguments, 16));
                        let clear_stencil = (flags & 4 != 0).then(|| word(arguments, 20));
                        self.items.push(Item::Clear(clear_color, clear_depth, clear_stencil));
                    }
                    _ => {}
                }
            }
            // The view and the projection of the camera (D3DTS_VIEW is 2 and
            // D3DTS_PROJECTION is 3). The draws of the game come as shape records and as
            // draws with their vertices, each with its own state; a command of another
            // kind, in a capture from before, does nothing.
            tag::COMMAND => match word(data, 0) {
                COMMAND_TRANSFORM if data.len() >= 16 + 64 => match word(data, 4) {
                    2 => self.stream.view = floats(data, 16),
                    3 => self.stream.projection = floats(data, 16),
                    _ => {}
                },
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
        self.film_draws += 1;
        if self.film_draws >= self.film_shot_at {
            if let Some(path) = self.film_shot.take() {
                self.shots.push(path);
            }
        }
        let count = vertices.len() / stride;
        let size = self.size_of_target();
        let (width, height) = (size.0 as f32, size.1 as f32);
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
        let renderer = &mut self.renderer;
        let pipeline = renderer.screen_pipeline();
        let clock = std::time::Instant::now();
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
        renderer.counts.times[TIME_BUFFERS] += clock.elapsed();
        self.items.push(Item::Screen(ScreenDraw { pipeline, group, buffer, vertices: (list.len() / 10) as u32 }));
    }

    /// The size of the target that the game draws into.
    fn size_of_target(&self) -> (u32, u32) {
        self.target.and_then(|handle| self.renderer.targets.get(&handle)).map_or(self.frame_size, |texture| texture.size)
    }

    /// Does the items of the frame that are not done yet. See `Item`.
    fn run_items(&mut self) {
        let items = std::mem::take(&mut self.items);
        if !self.drawing() {
            return;
        }
        let clock = std::time::Instant::now();
        let inside_before = sum_except(&self.renderer.counts.times, TIME_ITEMS);
        self.run_items_inner(items);
        let inside_after = sum_except(&self.renderer.counts.times, TIME_ITEMS);
        self.renderer.counts.times[TIME_ITEMS] += clock.elapsed().saturating_sub(inside_after - inside_before);
    }

    /// Sorts each run of opaque draws that follow each other by pipeline and textures. An
    /// opaque draw does not blend, writes the depth with the test "less" or "less or
    /// equal", has no stencil and writes all colours: among such draws the order does not
    /// change the picture, but for the ties of the depth test. No draw goes across a draw
    /// of another kind: the place of that one in the order is a part of the picture. The
    /// cursor of the game is such a draw: the last of the menu scene, with the depth
    /// written and not tested, over 1000 draws that blend.
    /// With `instances` (the world matrices of the instances of the frame), opaque draws
    /// that follow each other in the sorted run with the same pipeline, textures, samplers,
    /// buffers, range, lights and data but the world matrix become one draw with as many
    /// instances: the world matrices go to `instances`, the first at the index the draw
    /// keeps. A skinned draw and a draw with a vertex blend keep their own.
    fn sort_items(items: &mut Vec<Item>, counts: &mut Counts, lights: &[LightData], mut instances: Option<&mut Vec<[f32; 16]>>) {
        // The depth function is a D3DCMPFUNC: 2 is "less" and 4 is "less or equal".
        let opaque = |item: &Item| {
            matches!(item, Item::Draw(draw) if draw.key.blend[0] == 0
                && draw.key.depth[0] != 0
                && draw.key.depth[1] != 0
                && matches!(draw.key.depth[2], 2 | 4)
                && draw.key.stencil[0] == 0
                && draw.key.color_write & 7 == 7)
        };
        let same_lights = |a: &Draw, b: &Draw| {
            let (first_a, first_b, count) = (a.data.light_range[0] as usize, b.data.light_range[0] as usize, a.data.light_range[1] as usize);
            b.data.light_range[1] as usize == count
                && (count == 0 || lights.get(first_a..first_a + count).zip(lights.get(first_b..first_b + count)).is_some_and(|(x, y)| bytemuck::cast_slice::<LightData, u8>(x) == bytemuck::cast_slice::<LightData, u8>(y)))
        };
        let same_data = |a: &Draw, b: &Draw| {
            let mut x = a.data;
            let mut y = b.data;
            x.world[0] = [0.0; 16];
            y.world[0] = [0.0; 16];
            x.light_range[0] = 0;
            y.light_range[0] = 0;
            bytemuck::bytes_of(&x) == bytemuck::bytes_of(&y)
        };
        let can_join = |a: &Draw, b: &Draw| {
            a.data.flags[3] == 0
                && b.data.flags[3] == 0
                && a.key == b.key
                && a.stencil_reference == b.stencil_reference
                && a.range == b.range
                && a.origin == b.origin
                && Rc::ptr_eq(&a.textures, &b.textures)
                && Rc::ptr_eq(&a.samplers, &b.samplers)
                && Rc::ptr_eq(&a.vertices, &b.vertices)
                && Rc::ptr_eq(&a.indexes, &b.indexes)
                && same_lights(a, b)
                && same_data(a, b)
        };
        let order = |draw: &Draw| {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            draw.key.hash(&mut hasher);
            (hasher.finish(), draw.texture_handles, draw.geometry)
        };
        let mut start = 0;
        while start < items.len() {
            if !opaque(&items[start]) {
                start += 1;
                continue;
            }
            let mut end = start;
            while end < items.len() && opaque(&items[end]) {
                end += 1;
            }
            let mut first: Vec<Rc<Draw>> = items[start..end].iter().map(|item| if let Item::Draw(d) = item { d.clone() } else { unreachable!() }).collect();
            counts.sorted_draws += first.len() as u32;
            first.sort_by_cached_key(|d| order(d));
            if let Some(instances) = instances.as_deref_mut() {
                let mut joined: Vec<Rc<Draw>> = Vec::with_capacity(first.len());
                let mut at = 0;
                while at < first.len() {
                    let mut end = at + 1;
                    while end < first.len() && end - at < INSTANCES_MOST && can_join(&first[at], &first[end]) {
                        end += 1;
                    }
                    if end - at < 2 {
                        joined.push(first[at].clone());
                    } else {
                        let mut draw = (*first[at]).clone();
                        draw.instances = (instances.len() as u32, (end - at) as u32);
                        draw.data.light_range[3] |= 2;
                        instances.extend(first[at..end].iter().map(|d| d.data.world[0]));
                        counts.instanced_draws += (end - at - 1) as u32;
                        joined.push(Rc::new(draw));
                    }
                    at = end;
                }
                first = joined;
            }
            let kept: Vec<Item> = first.into_iter().map(Item::Draw).collect();
            let removed = end - start - kept.len();
            items.splice(start..end, kept);
            start = end - removed;
        }
    }

    fn run_items_inner(&mut self, items: Vec<Item>) {
        let mut items = items;
        self.plane_ahead = self.parameters.as_ref().and_then(|p| p.class_plane).filter(|(class, _)| items.iter().any(|item| matches!(item, Item::Replace(c, ..) if c == class))).map(|(_, plane)| plane);
        if self.renderer.sort_draws {
            let instancing = self.renderer.instancing;
            Self::sort_items(&mut items, &mut self.renderer.counts, &self.frame_lights, instancing.then_some(&mut self.frame_instances));
        }
        self.renderer.upload_lights(&self.frame_lights);
        self.renderer.upload_bones(&self.frame_bones);
        self.renderer.upload_instances(&self.frame_instances);
        // The resources of the extension for the frame.
        let (values, texture) = self.parameters.as_ref().map_or(([[0.0f32; 4]; 32], None), |p| (p.values, p.texture.clone()));
        self.renderer.queue.write_buffer(&self.renderer.frame_uniform, 0, bytemuck::cast_slice(&values));
        let texture = texture.unwrap_or_else(|| self.renderer.white.clone());
        let renderer = &mut self.renderer;
        let clock = std::time::Instant::now();
        // The view and the projection of the scene: those of the start of the world, as the
        // lights of the extension have them.
        let scene = items.iter().find_map(|item| if let Item::WorldStart(facts) = item { Some((facts.view, facts.projection)) } else { None });
        renderer.clusters.build(&renderer.device, &renderer.queue, &self.frame_lights, scene, self.frame_size, &mut renderer.counts);
        renderer.counts.times[TIME_CLUSTERS] += clock.elapsed();
        renderer.frame_group = Rc::new(frame_group(&renderer.device, &renderer.frame_layout, &renderer.frame_uniform, &renderer.frame_sampler, &texture, &renderer.clusters));
        for item in items {
            match item {
                Item::Target(target) => {
                    self.pass_target = target;
                    self.open_pass(None, None, None);
                }
                Item::Clear(color, depth, stencil) => self.open_pass(color, depth, stencil),
                Item::Viewport(viewport) => {
                    self.viewport = Some(viewport);
                    self.apply_viewport();
                }
                Item::Draw(draw) => self.issue(&draw),
                Item::Screen(screen) => {
                    let Some(pass) = self.pass.as_mut() else { continue };
                    pass.set_pipeline(&screen.pipeline);
                    pass.set_bind_group(0, &screen.group, &[]);
                    pass.set_vertex_buffer(0, screen.buffer.slice(..));
                    pass.draw(0..screen.vertices, 0..1);
                    self.renderer.counts.screen_draws += 1;
                }
                Item::WorldStart(facts) => self.start_world(&facts),
                Item::WorldEnd(facts) => self.end_world(&facts),
                Item::HeldHere(held, facts) => self.give_held_here(*held, &facts),
                Item::Replace(class, facts, drawn) => self.replace_class(class, &facts, drawn),
                Item::Post(facts) => self.post_process(&facts),
            }
        }
    }

    /// Puts a draw into the pass.
    fn issue(&mut self, draw: &Draw) {
        let renderer = &mut self.renderer;
        let clock = std::time::Instant::now();
        // A world draw that blends and writes no depth (not the sky), with the call of a
        // class with a plane in the frame: on the far side before the call, on the side of
        // the camera after it. The call draws the other side again. A draw that writes depth
        // needs no split: the depth test of the call keeps it in front where it is.
        let side = match (self.split, self.plane_ahead) {
            (Some(plane), _) => Some(plane),
            (None, Some(plane)) => Some(plane.map(|v| -v)),
            (None, None) => None,
        };
        let split = draw.world && draw.translucent() && ReplayView::WORLD & (1 << draw.class.min(31)) != 0;
        let clipped = side.filter(|_| split).map(|plane| {
            let mut data = draw.data;
            data.clip_plane = plane_of_draw(plane, draw.origin);
            data
        });
        let (draw_group, draw_offset) = renderer.draw_slot(clipped.as_ref().unwrap_or(&draw.data));
        renderer.counts.times[TIME_DRAW_UPLOAD] += clock.elapsed();
        let clock = std::time::Instant::now();
        let Some(pipeline) = renderer.pipeline(draw.key) else {
            renderer.counts.no_layout += 1;
            return;
        };
        renderer.counts.times[TIME_DRAW_GROUPS] += clock.elapsed();
        let clock = std::time::Instant::now();
        let Some(pass) = self.pass.as_mut() else { return };
        pass.set_pipeline(&pipeline);
        pass.set_stencil_reference(draw.stencil_reference);
        pass.set_bind_group(0, &*draw_group, &[draw_offset]);
        pass.set_bind_group(1, &*draw.textures, &[]);
        pass.set_bind_group(2, &*draw.samplers, &[]);
        pass.set_bind_group(3, &*renderer.frame_group, &[]);
        pass.set_vertex_buffer(0, draw.vertices.slice(..));
        pass.set_vertex_buffer(1, renderer.defaults.slice(..));
        pass.set_vertex_buffer(2, renderer.instance_buffer.slice(draw.instances.0 as u64 * 64..));
        let (first, count) = draw.range;
        pass.set_index_buffer(draw.indexes.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(first..first + count, 0, 0..draw.instances.1);
        renderer.counts.times[TIME_DRAW_PASS] += clock.elapsed();
        renderer.counts.draws += 1;
    }

    /// The post-processing item, one time in a frame, after the world began.
    fn post_item(&mut self) {
        if self.posted || !self.world_begun || !self.drawing() || !self.parameters.as_ref().is_some_and(|p| p.post) {
            return;
        }
        self.posted = true;
        let mut facts = self.world_facts(false);
        if let Some((view, projection)) = self.world_matrices {
            facts.view = view;
            facts.projection = projection;
        }
        self.items.push(Item::Post(Box::new(facts)));
    }

    /// The facts of the world scene as the stream has them now. With `first`, the sun and
    /// the ambient light come from the shape record in progress (the first draw of the
    /// world), or from the last one for a world scene without a draw; without it, they are
    /// those of that draw.
    fn world_facts(&mut self, first: bool) -> WorldFacts {
        let shape = &self.stream.shape;
        if first {
            // The first light of the draw that is a sun or another directional light: type 3,
            // or type 4 for the light that the plugin names as the sun.
            let sun = shape.lights.chunks_exact(STATE_LIGHT_SIZE).map(|light| &light[4..]).find(|l| matches!(word(l, 0), 3 | 4)).map(|l| {
                let diffuse: [f32; 3] = floats(l, 4);
                let light_ambient: [f32; 3] = floats(l, 9 * 4);
                // The game does not give a direction of length 1.
                let direction: [f32; 3] = floats(l, 16 * 4);
                let length = direction.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
                (direction.map(|v| v / length), diffuse, light_ambient)
            });
            let ambient = color(known(shape.ambient, 0));
            if std::env::var("MGE_XE_REPLAY_DEBUG").is_ok_and(|v| v == "1") {
                println!("world start: lights {} sun {:?} ambient {:?}", shape.lights.len() / STATE_LIGHT_SIZE, sun, ambient);
            }
            self.world_sun = sun;
            self.world_ambient = [ambient[0], ambient[1], ambient[2]];
        }
        // The fog and the fill mode as the device of the game has them now: the fog states
        // of the last draw with fog, and the fill mode of the last draw (D3DFILL_WIREFRAME
        // is 2).
        let fog = self.stream.fog;
        WorldFacts {
            target_size: self.size_of_target(),
            view: self.stream.view,
            projection: self.stream.projection,
            interior: self.stream.interior,
            wireframe: self.stream.shape.fill == 2,
            fog_color: color(fog[0]),
            fog_start: f32::from_bits(fog[1]),
            fog_end: f32::from_bits(fog[2]),
            sun: self.world_sun,
            ambient: self.world_ambient,
        }
    }

    /// Asks the extension for its answers for the frame. See `Extension::frame`.
    fn ask_frame(&mut self) {
        let Some(mut extension) = self.extension.take() else { return };
        let facts = self.world_facts(true);
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let scene = facts.view(&device, &queue, format);
        let clock = std::time::Instant::now();
        self.parameters = Some(extension.frame(&scene));
        self.renderer.counts.times[TIME_EXT_FRAME] += clock.elapsed();
        self.extension = Some(extension);
    }

    /// Gives the extension its call at the end of the world scene. See `Extension::after_world`.
    fn end_world(&mut self, facts: &WorldFacts) {
        let Some(mut extension) = self.extension.take() else { return };
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let mut scene = facts.view(&device, &queue, format);
        if let Some(pass) = self.pass.as_mut() {
            let world = std::mem::take(&mut self.world_draws);
            let mut replayer = Replayer { renderer: &mut self.renderer, kept: &world };
            let clock = std::time::Instant::now();
            extension.after_world(&scene, pass, &mut replayer);
            self.renderer.counts.times[TIME_EXT_AFTER] += clock.elapsed();
            self.world_draws = world;
        }
        // The depth of the world, for the post-processing: the game clears its depth before
        // its later scenes.
        let mut world_depth_now = false;
        if self.parameters.as_ref().is_some_and(|p| p.post) {
            world_depth_now = self.copy_depth(1).is_some();
            self.world_depth_kept = world_depth_now;
            self.open_pass(None, None, None);
        }
        // The blend: a pass on the picture only, so that a shader can read the depth. A copy
        // of the picture when the extension reads it.
        let picture = if self.before_kept && extension.reads(ReadingCall::Blend).picture { self.copy_picture() } else { None };
        if let (true, Some(before), Some(color), Some(depth), Some(encoder)) = (self.before_kept, self.before.as_ref(), self.color.clone(), self.depth.clone(), self.encoder.as_mut()) {
            drop(self.pass.take());
            scene.picture = picture.as_ref();
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
            let clock = std::time::Instant::now();
            extension.blend(&scene, &mut pass, &before, &depth);
            drop(pass);
            self.renderer.counts.times[TIME_EXT_AFTER] += clock.elapsed();
            self.open_pass(None, None, None);
        }
        self.extension = Some(extension);
        // The blend pass has no depth target: the copy of the world depth is the depth now.
        self.give_held(facts, world_depth_now);
    }

    /// Gives the extension the shapes that it took in the frame. See `Extension::draw_held`.
    /// `world_depth_now`: the copy of the world depth is the depth as it is now.
    fn give_held(&mut self, facts: &WorldFacts, world_depth_now: bool) {
        if self.taken.is_empty() && !self.extension.as_ref().is_some_and(|extension| extension.wants_held()) {
            return;
        }
        let Some(mut extension) = self.extension.take() else { return };
        let (picture, depth) = self.copy_for(extension.reads(ReadingCall::Held(&self.taken)), world_depth_now);
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let mut scene = facts.view(&device, &queue, format);
        let before_depth = self.before_depth.as_ref().filter(|_| self.before_depth_kept).map(|copy| copy.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() }));
        scene.picture = picture.as_ref();
        scene.depth = depth.as_ref();
        scene.before_depth = before_depth.as_ref();
        // The lights for the extension, binned for the view that it draws with.
        let renderer = &mut self.renderer;
        renderer.extension_lights.build(&renderer.device, &renderer.queue, &self.frame_lights, facts.view, facts.projection, self.frame_size, &mut renderer.counts);
        let own = &self.renderer.extension_lights;
        let lights = own.built.then(|| (own.lights.clone(), own.uniform.clone(), own.table.clone(), own.indexes.clone(), own.origin));
        scene.lights = lights.as_ref().map(|(lights, clusters, table, indexes, origin)| FrameLights { lights, clusters, table, indexes, origin: *origin });
        if let Some(pass) = self.pass.as_mut() {
            let world = std::mem::take(&mut self.world_draws);
            let mut replayer = Replayer { renderer: &mut self.renderer, kept: &world };
            let clock = std::time::Instant::now();
            extension.draw_held(&scene, pass, &self.taken, &mut replayer);
            self.renderer.counts.times[TIME_EXT_CLASS] += clock.elapsed();
            self.world_draws = world;
        }
        self.extension = Some(extension);
    }

    /// Gives the extension a held shape in its place. See `Extension::draw_held_in_place`.
    fn give_held_here(&mut self, held: HeldDraw, facts: &WorldFacts) {
        let Some(mut extension) = self.extension.take() else { return };
        let (picture, depth) = self.copy_for(extension.reads(ReadingCall::HeldInPlace(&held)), false);
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let mut scene = facts.view(&device, &queue, format);
        scene.picture = picture.as_ref();
        scene.depth = depth.as_ref();
        let own = &self.renderer.extension_lights;
        let lights = own.built.then(|| (own.lights.clone(), own.uniform.clone(), own.table.clone(), own.indexes.clone(), own.origin));
        scene.lights = lights.as_ref().map(|(lights, clusters, table, indexes, origin)| FrameLights { lights, clusters, table, indexes, origin: *origin });
        if let Some(pass) = self.pass.as_mut() {
            let world = std::mem::take(&mut self.world_draws);
            let mut replayer = Replayer { renderer: &mut self.renderer, kept: &world };
            let clock = std::time::Instant::now();
            extension.draw_held_in_place(&scene, pass, core::slice::from_ref(&held), &mut replayer);
            self.renderer.counts.times[TIME_EXT_CLASS] += clock.elapsed();
            self.world_draws = world;
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
        self.renderer.counts.picture_copies += 1;
        self.before_kept = true;
    }

    /// Copies what a call of the extension reads (`Extension::reads`): the picture of the
    /// frame and the depth of the game, outside of the pass. With `world_depth_now`, the copy
    /// of the world depth is the depth as it is now, and is given for it. The pass is open
    /// again after a copy.
    fn copy_for(&mut self, reads: Reads, world_depth_now: bool) -> (Option<wgpu::TextureView>, Option<wgpu::TextureView>) {
        let picture = if reads.picture { self.copy_picture() } else { None };
        let depth = match (reads.depth, world_depth_now) {
            (false, _) => None,
            (true, true) => self.world_depth_view(),
            (true, false) => self.copy_depth(0),
        };
        if self.pass.is_none() {
            self.open_pass(None, None, None);
        }
        (picture, depth)
    }

    /// The copy of the depth at the end of the world scene, when it was kept in this frame.
    fn world_depth_view(&self) -> Option<wgpu::TextureView> {
        self.world_depth
            .as_ref()
            .filter(|_| self.world_depth_kept)
            .map(|copy| copy.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() }))
    }

    /// Copies the picture of the frame, outside of the pass. The pass stays closed.
    fn copy_picture(&mut self) -> Option<wgpu::TextureView> {
        let (Some(source), Some(encoder)) = (self.back_buffer.as_ref(), self.encoder.as_mut()) else { return None };
        if !source.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            return None;
        }
        let size = source.size();
        if self.picture_copy.as_ref().is_none_or(|copy| copy.size() != size) {
            self.picture_copy = Some(target_texture(&self.renderer.device, size.width, size.height, source.format(), wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST));
        }
        drop(self.pass.take());
        encoder.copy_texture_to_texture(source.as_image_copy(), self.picture_copy.as_ref().unwrap().as_image_copy(), size);
        self.renderer.counts.picture_copies += 1;
        self.picture_copy.as_ref().map(|copy| copy.create_view(&Default::default()))
    }

    /// Copies the depth as it is now, outside of the pass: into the copy of the moment (0),
    /// the copy of the world (1, for `Extension::post`) or the copy of the draws of
    /// `Extension::before_world` (2). The caller opens the pass again.
    fn copy_depth(&mut self, which: u32) -> Option<wgpu::TextureView> {
        let (Some(source), Some(encoder)) = (self.depth.as_ref().map(|depth| depth.texture().clone()), self.encoder.as_mut()) else { return None };
        if !source.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            return None;
        }
        let size = source.size();
        let copy = match which {
            1 => &mut self.world_depth,
            2 => &mut self.before_depth,
            _ => &mut self.depth_copy,
        };
        if copy.as_ref().is_none_or(|copy| copy.size() != size) {
            *copy = Some(target_texture(&self.renderer.device, size.width, size.height, source.format(), wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST));
        }
        drop(self.pass.take());
        encoder.copy_texture_to_texture(source.as_image_copy(), copy.as_ref().unwrap().as_image_copy(), size);
        self.renderer.counts.depth_copies += 1;
        copy.as_ref().map(|copy| copy.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() }))
    }

    /// Gives the extension its post-processing call. See `Extension::post`.
    fn post_process(&mut self, facts: &WorldFacts) {
        let Some(mut extension) = self.extension.take() else { return };
        // The depth of the end of the world scene, when it was kept; else a copy of that of now.
        let mut reads = extension.reads(ReadingCall::Post);
        let world_depth = self.world_depth_view().filter(|_| reads.depth);
        reads.depth &= world_depth.is_none();
        let (picture, depth_now) = self.copy_for(reads, false);
        let depth = world_depth.or(depth_now);
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let mut scene = facts.view(&device, &queue, format);
        let before_depth = self.before_depth.as_ref().filter(|_| self.before_depth_kept).map(|copy| copy.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() }));
        scene.picture = picture.as_ref();
        scene.depth = depth.as_ref();
        scene.before_depth = before_depth.as_ref();
        if let (Some(color), Some(encoder)) = (self.color.clone(), self.encoder.as_mut()) {
            drop(self.pass.take());
            let clock = std::time::Instant::now();
            extension.post(&scene, encoder, &color);
            self.renderer.counts.times[TIME_EXT_POST] += clock.elapsed();
            // The pictures that the extension wants: one without the interface of the game
            // is the picture as it is now.
            for request in extension.pictures() {
                match self.back_buffer.as_ref().filter(|source| !request.with_ui && source.usage().contains(wgpu::TextureUsages::COPY_SRC)) {
                    Some(source) => {
                        let (buffer, row) = copy_picture(&device, encoder, source, source.width(), source.height());
                        self.early_pictures.push((buffer, row, source.width(), source.height(), request.path));
                    }
                    None => self.shots.push(request.path),
                }
            }
        }
        self.extension = Some(extension);
        self.open_pass(None, None, None);
    }

    /// Gives the extension its call in place of the draws of a class. See
    /// `Extension::draw_class`. `drawn` is the number of world draws before the call. With
    /// a plane for the class (`FrameParameters::class_plane`): the world draws that blend,
    /// write no depth and come after the call go into the picture first, on the far side of
    /// the plane (those before the call are there already, on that side only); after the
    /// call each of them that came before it goes on the side of the camera, over the call's
    /// drawing.
    fn replace_class(&mut self, class: u32, facts: &WorldFacts, drawn: usize) {
        let Some(mut extension) = self.extension.take() else { return };
        let plane = self.parameters.as_ref().and_then(|p| p.class_plane).filter(|(c, _)| *c == class).map(|(_, plane)| plane);
        let drawn = drawn.min(self.world_draws.len());
        if let Some(plane) = plane {
            self.replay_blended(drawn.., plane.map(|v| -v), facts.target_size);
            self.plane_ahead = None;
            self.split = Some(plane);
        }
        let (picture, depth) = self.copy_for(extension.reads(ReadingCall::Class(class)), false);
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let mut scene = facts.view(&device, &queue, format);
        let before_depth = self.before_depth.as_ref().filter(|_| self.before_depth_kept).map(|copy| copy.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() }));
        scene.picture = picture.as_ref();
        scene.depth = depth.as_ref();
        scene.before_depth = before_depth.as_ref();
        if let Some(pass) = self.pass.as_mut() {
            let world = std::mem::take(&mut self.world_draws);
            let mut replayer = Replayer { renderer: &mut self.renderer, kept: &world };
            let clock = std::time::Instant::now();
            extension.draw_class(class, &scene, pass, &mut replayer);
            self.renderer.counts.times[TIME_EXT_CLASS] += clock.elapsed();
            self.world_draws = world;
        }
        if let Some(plane) = plane {
            self.replay_blended(..drawn, plane, facts.target_size);
        }
        self.extension = Some(extension);
    }

    /// Draws the world draws in `range` that blend and write no depth again, into the pass,
    /// only where they are on the positive side of `plane`.
    fn replay_blended(&mut self, range: impl std::slice::SliceIndex<[Rc<Draw>], Output = [Rc<Draw>]>, plane: [f64; 4], target_size: (u32, u32)) {
        let Some(pass) = self.pass.as_mut() else { return };
        let world = std::mem::take(&mut self.world_draws);
        let mut replayer = Replayer { renderer: &mut self.renderer, kept: &world[range] };
        replayer.replay(
            pass,
            &ReplayView {
                world: None,
                view: None,
                projection: None,
                target_size,
                flip_cull: false,
                clip_plane: Some(plane),
                opaque_only: false,
                translucent_only: true,
                force_depth_test: false,
                classes: ReplayView::WORLD,
            },
        );
        self.world_draws = world;
    }

    /// Gives the extension its call at the start of the world. See `Extension::before_world`.
    fn start_world(&mut self, facts: &WorldFacts) {
        let Some(mut extension) = self.extension.take() else { return };
        let (device, queue, format) = (self.renderer.device.clone(), self.renderer.queue.clone(), self.renderer.target_format);
        let scene = facts.view(&device, &queue, format);
        let clear_depth = match self.pass.as_mut() {
            Some(pass) => {
                let world = std::mem::take(&mut self.world_draws);
                let mut replayer = Replayer { renderer: &mut self.renderer, kept: &world };
                let clock = std::time::Instant::now();
                let clear_depth = extension.before_world(&scene, pass, &mut replayer);
                self.renderer.counts.times[TIME_EXT_BEFORE] += clock.elapsed();
                self.world_draws = world;
                clear_depth
            }
            None => false,
        };
        let blend = self.parameters.as_ref().is_some_and(|p| p.blend);
        self.extension = Some(extension);
        if clear_depth && blend {
            self.keep_before();
        }
        if clear_depth {
            // The depth of the draws of the extension, for its post-processing and for the
            // draws that it makes with the held shapes.
            if self.parameters.as_ref().is_some_and(|p| p.post || p.hold || p.watched != 0) {
                self.before_depth_kept = self.copy_depth(2).is_some();
            }
            // The draws of the game start with a depth target of their own.
            self.open_pass(None, Some(1.0), None);
        }
    }

    /// Collects one draw of a pass of the shape in progress: `topology` is the primitive
    /// type of Direct3D (2 a line list, 4 a triangle list, 5 a triangle strip), over
    /// `indexes` entries of the index list of the geometry from `first_index`.
    fn draw(&mut self, derived: &material::Derived, pass: &Pass, topology: u32, first_index: u32, indexes: u32) {
        if !self.stream.back_buffer {
            self.renderer.counts.other_target += 1;
            return;
        }
        let in_back_buffer = self.target.is_none();
        // The first draw of the world scene (kind 1) into the back buffer: the answers of
        // the extension for the frame.
        if self.extension.is_some() && self.parameters.is_none() && in_back_buffer && self.scene_kind == 1 {
            self.ask_frame();
        }
        // The world of the game starts with the first draw of its world scene into the
        // back buffer that is not a part of the sky and comes after the sky: the classes 1
        // to 7 are the sky and 8 is the glare of the sun. The world scene is one
        // `NiCamera::Click`: the engine draws a geometry without alpha blending as it comes
        // to it in the scene graph (`NiAlphaAccumulator::RegisterObject` 0x429B90 keeps
        // the blended ones for the end), so a draw can come before the sky (the lines of
        // the cell borders); it is not the world. With the sky hidden (the scene packet:
        // a plain interior cell, or ToggleSky) there is no sky draw: the world starts at
        // the first draw.
        let sky = (1..=8).contains(&self.stream.object_class);
        if in_back_buffer && self.scene_kind == 1 && sky {
            self.sky_seen = true;
        }
        if self.extension.is_some() && !self.world_begun && in_back_buffer && self.scene_kind == 1 && !sky && (self.sky_seen || self.stream.sky_hidden) {
            self.world_begun = true;
            let facts = self.world_facts(true);
            self.items.push(Item::WorldStart(Box::new(facts)));
            // The draws from before the sky, in their order: the extension draws its
            // distant land under them and starts the depth of the world, so a draw before
            // the start would lose its depth.
            self.items.append(&mut self.held_draws);
        }
        let hold = self.extension.is_some() && !self.world_begun && in_back_buffer && self.scene_kind == 1 && !sky;
        // A draw of a class that the extension takes: the extension draws at the first one.
        let class = self.stream.object_class;
        let (watched, shade, fog_range, sky) = self.parameters.as_ref().map_or((0, false, None, None), |p| (p.watched, p.shade, p.fog_range, p.sky));
        if in_back_buffer && class < 32 && watched & (1 << class) != 0 {
            if self.replaced & (1 << class) == 0 {
                self.replaced |= 1 << class;
                let facts = self.world_facts(false);
                self.items.push(Item::Replace(class, Box::new(facts), self.world_draws.len()));
            }
            return;
        }
        let renderer = &mut self.renderer;
        let shape = &self.stream.shape;
        let stencil = derived.stencil;
        let key = PipelineKey {
            fvf: shape.fvf,
            stride: shape.stride,
            topology,
            blend: pass.blend,
            depth: [(derived.depth[0] != 0) as u32, (derived.depth[1] != 0) as u32, derived.depth[2]],
            cull: derived.cull,
            fill: derived.fill,
            // The back buffer of the game has no alpha (X8R8G8B8): a draw into it writes
            // none, and the alpha there stays 1, as a read of such a target gives.
            color_write: if in_back_buffer { pass.color_write & 7 } else { pass.color_write },
            // On, fail, depth fail, pass, function, mask, write mask.
            stencil: if stencil[0] == 0 { [0; 7] } else { [1, stencil[4], stencil[5], stencil[6], stencil[1], stencil[3], !0] },
        };
        let stencil_reference = stencil[2];
        // The buffers of the geometry record of the shape.
        let Some((vertex_buffer, index_buffer)) = renderer.geometries.get(&shape.geometry).filter(|g| first_index + indexes <= g.entries).map(|g| (g.vertex.clone(), g.index.clone())) else {
            renderer.counts.geometry_short += 1;
            return;
        };
        renderer.counts.geometry_draws += 1;
        let clock = std::time::Instant::now();
        // The answers of the extension: the light for each pixel and the fog distances for
        // the world in the back buffer (not for the sky, which is before the world, and not
        // for a texture that the game draws into), the sky for a sky draw in the back buffer
        // (MGE_LINK_CLASS_SKY_DOME is 1 and MGE_LINK_CLASS_SKY_OTHER is 7).
        let world = self.world_begun && in_back_buffer;
        if world && self.stream.projection[11] == 1.0 {
            self.world_matrices = Some((self.stream.view, self.stream.projection));
        }
        self.stream.fog_range = fog_range.filter(|_| world);
        self.stream.sky = sky.filter(|_| in_back_buffer && (1..=7).contains(&self.stream.object_class));
        let mut draw_data = self.stream.draw_data(derived, pass, &mut renderer.counts, &mut self.frame_lights);
        let mut shaded = false;
        if shade && world && self.stream.projection[11] == 1.0 {
            if let Some(extension) = self.extension.as_mut() {
                let (first, count) = (draw_data.light_range[0] as usize, draw_data.light_range[1] as usize);
                let facts = DrawFacts {
                    class,
                    blend_enable: key.blend[0] != 0,
                    source_blend: key.blend[1],
                    destination_blend: key.blend[2],
                    depth_write: key.depth[1] != 0,
                    specular_power: shape.material[16],
                    emissive: shape.material[12..16].try_into().unwrap(),
                };
                extension.shade(&mut draw_data, &mut self.frame_lights[first..first + count], &facts);
                shaded = true;
            }
        }
        // The lights of a draw have keys: they are candidates for the clusters of the frame,
        // for the extension (`FrameLights`), and the draw that the extension shades can take
        // its light from them.
        let masked = shaded && renderer.clusters.on;
        let extension_lights = self.parameters.as_ref().is_some_and(|p| p.lights) && world && self.stream.projection[11] == 1.0;
        let first = draw_data.light_range[0] as usize;
        let relative = shape.relative && self.stream.origin.is_some();
        for (i, light) in shape.lights.chunks_exact(STATE_LIGHT_SIZE).enumerate() {
            if masked {
                if let Some(slot) = renderer.clusters.note(word(light, 0), (first + i) as u32, relative, self.stream.origin) {
                    draw_data.light_mask[slot as usize / 128][slot as usize / 32 % 4] |= 1 << (slot % 32);
                }
            }
            if extension_lights {
                renderer.extension_lights.note(word(light, 0), (first + i) as u32, relative, self.stream.origin);
            }
        }
        if masked {
            draw_data.light_range[3] = 1;
        }
        renderer.counts.times[TIME_DRAW_DATA] += clock.elapsed();
        let clock = std::time::Instant::now();
        let mut handles = [0u32; STAGES];
        let mut sampler_values = [[0u32; 5]; STAGES];
        for stage in 0..STAGES {
            let handle = pass.stages[stage].texture;
            let used = stage < draw_data.counts[1] as usize && handle != 0;
            if used && !renderer.textures.contains_key(&handle) {
                renderer.counts.missing_textures += 1;
            } else if used {
                handles[stage] = handle;
                sampler_values[stage] = pass.stages[stage].sampler;
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
        renderer.counts.times[TIME_DRAW_GROUPS] += clock.elapsed();
        if self.target.is_some() {
            renderer.counts.target_draws += 1;
        }
        let draw = Rc::new(Draw {
            class,
            // The world scene of the game is 1, with the sky at its start; its sorted draws
            // come in a scene of their own (2), or with the water plane in the scene of the
            // water (3). The water plane of the game (MGE_LINK_CLASS_WATER_PLANE, 10) is not
            // a thing of the world.
            world: in_back_buffer && matches!(self.scene_kind, 1..=3) && class != 10,
            key,
            stencil_reference,
            data: draw_data,
            origin: self.stream.origin.filter(|_| shape.relative),
            textures: texture_group,
            samplers: sampler_group,
            vertices: vertex_buffer,
            indexes: index_buffer,
            texture_handles: handles,
            geometry: shape.geometry,
            range: (first_index, indexes),
            instances: (0, 1),
        });
        if draw.world {
            self.world_draws.push(draw.clone());
        }
        if hold {
            self.held_draws.push(Item::Draw(draw));
        } else {
            self.items.push(Item::Draw(draw));
        }
    }

    fn print_counts(&self) {
        let c = &self.renderer.counts;
        println!("draws {}, pipelines {}, textures {}, samplers {}", c.draws, self.renderer.pipelines.len(), c.textures, self.renderer.samplers.len());
        println!(
            "geometry: {} records ({:.1} MB), {} draws from them, {} records that did not parse, {} draws of more index entries than their geometry has",
            c.geometry_records,
            c.geometry_bytes as f64 / 1e6,
            c.geometry_draws,
            c.geometry_bad,
            c.geometry_short
        );
        println!("material: {} records", c.material_records);
        println!("shapes: {} records, {} draws from them, {} without a geometry or a material", c.shape_records, c.shape_draws, c.shape_missing);
        println!("rules of the engine not followed: {} shapes with flat shading, {} shapes with specular lighting", c.shape_flat, c.shape_specular);
        println!("sorted draws {}, instanced away {}, skinned shapes {}, particle systems {}", c.sorted_draws, c.instanced_draws, c.shape_skinned, c.shape_particles);
        println!("shapes held for the extension {}", c.held);
        println!("lights: {} records, {} lights of a shape without a record", c.light_records, c.light_missing);
        println!(
            "clusters: {:.1} lights and {:.0} entries for each frame, {} frames without (a light had no cut-off), {} frames with more lights than a mask holds",
            c.cluster_lights as f64 / c.frames.max(1) as f64,
            c.cluster_entries as f64 / c.frames.max(1) as f64,
            c.cluster_frames_off,
            c.cluster_frames_many
        );
        println!("lights for the extension: {:.1} for each frame", c.extension_lights as f64 / c.frames.max(1) as f64);
        println!(
            "copies for the extension: {:.2} of the picture and {:.2} of the depth for each frame",
            c.picture_copies as f64 / c.frames.max(1) as f64,
            c.depth_copies as f64 / c.frames.max(1) as f64
        );
        // With `MGE_XE_GPU_TIME=1`: the GPU time of all timed frames (a replay has no marks).
        if c.gpu_frames != 0 {
            println!("gpu: {:.3} ms in the passes, {:.3} ms the frame, for each of {} timed frames", c.gpu_passes / 1e6 / c.gpu_frames as f64, c.gpu_frame / 1e6 / c.gpu_frames as f64, c.gpu_frames);
        }
        println!("not drawn: other target {}, vertex format {}", c.other_target, c.no_layout);
        println!(
            "not complete: draws with a texture that is not in the stream {}, more than 6 stages {}; draws with a made or moved texture coordinate {}; the most lights of a draw {}",
            c.missing_textures, c.more_stages, c.coordinate_work, c.most_lights
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

/// Saves a picture that `copy_picture` put into `buffer` as a file: the format of the
/// extension of `path` (`.bmp`, `.tga`, `.dds`, `.jpg`), a PNG file for another one.
/// `blue_first` for a texture format with the bytes blue, green, red.
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
    drop(mapped);
    match write_picture(path, &pixels, width, height) {
        Ok(()) => println!("picture: {path}"),
        Err(problem) => println!("picture: {path} was not written: {problem}"),
    }
}

/// Writes pixels (red, green, blue, alpha; the first row at the top) as a picture file.
fn write_picture(path: &str, pixels: &[u8], width: u32, height: u32) -> std::io::Result<()> {
    use std::io::Write;
    let kind = std::path::Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let (w, h) = (width as usize, height as usize);
    // The bytes blue, green, red of a row, for the formats that have them in that order.
    let bgr_row = |y: usize, with_alpha: bool| -> Vec<u8> {
        pixels[y * w * 4..(y + 1) * w * 4].chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], 255].into_iter().take(if with_alpha { 4 } else { 3 })).collect()
    };
    if kind == "jpg" || kind == "jpeg" {
        let rgb: Vec<u8> = pixels.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        let encoder = jpeg_encoder::Encoder::new_file(path, 90).map_err(|problem| std::io::Error::other(problem.to_string()))?;
        return encoder.encode(&rgb, width as u16, height as u16, jpeg_encoder::ColorType::Rgb).map_err(|problem| std::io::Error::other(problem.to_string()));
    }
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    match kind.as_str() {
        "bmp" => {
            // 24 bits, the rows from the bottom, each row a multiple of 4 bytes.
            let row = (w * 3).next_multiple_of(4);
            let size = 54 + row * h;
            file.write_all(b"BM")?;
            for word in [size as u32, 0, 54, 40, width, height, 1 | (24 << 16), 0, (row * h) as u32, 2835, 2835, 0, 0] {
                file.write_all(&word.to_le_bytes())?;
            }
            for y in (0..h).rev() {
                let mut bytes = bgr_row(y, false);
                bytes.resize(row, 0);
                file.write_all(&bytes)?;
            }
        }
        "tga" => {
            // True colour, 24 bits, the first row at the top.
            file.write_all(&[0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0])?;
            file.write_all(&(width as u16).to_le_bytes())?;
            file.write_all(&(height as u16).to_le_bytes())?;
            file.write_all(&[24, 0x20])?;
            for y in 0..h {
                file.write_all(&bgr_row(y, false))?;
            }
        }
        "dds" => {
            // No compression, 32 bits: the format A8R8G8B8 of Direct3D.
            file.write_all(b"DDS ")?;
            let mut head = [0u32; 31];
            head[0] = 124;
            head[1] = 0x1 | 0x2 | 0x4 | 0x8 | 0x1000;
            head[2] = height;
            head[3] = width;
            head[4] = width * 4;
            head[18] = 32;
            head[19] = 0x41;
            head[21] = 32;
            head[22] = 0x00FF_0000;
            head[23] = 0x0000_FF00;
            head[24] = 0x0000_00FF;
            head[25] = 0xFF00_0000;
            head[26] = 0x1000;
            for word in head {
                file.write_all(&word.to_le_bytes())?;
            }
            for y in 0..h {
                file.write_all(&bgr_row(y, true))?;
            }
        }
        _ => {
            // The fast compression: a picture of the game must not take a second.
            let mut encoder = png::Encoder::new(file, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_compression(png::Compression::Fast);
            encoder.write_header().and_then(|mut writer| writer.write_image_data(pixels)).map_err(|problem| std::io::Error::other(problem.to_string()))?;
            return Ok(());
        }
    }
    file.flush()
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

/// `run` with a program that draws more than the game does: in the live mode, and for a
/// capture file.
pub fn run_with(args: Vec<String>, extension: Option<Box<dyn Extension>>) {
    if let Some(path) = option(&args, "--log") {
        log_to(&path);
    }
    println!("nirender-host v{}{}", env!("CARGO_PKG_VERSION"), extension.as_ref().map_or(String::new(), |e| format!(", {}", e.name())));
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

    let mut player = Player::new(Renderer::new(backends, None, wgpu::TextureFormat::Rgba8Unorm, extension_shaders(extension.as_deref())).0);
    // The filter of the extension's settings, as in the live mode; the argument is for a test.
    player.renderer.anisotropy =
        option(&args, "--anisotropy").and_then(|text| text.parse().ok()).or(extension.as_ref().map(|extension| extension.anisotropy())).unwrap_or(1);
    if let Some(mut extension) = extension {
        extension.offline();
        player.extension = Some(extension);
    }
    let device = player.renderer.device.clone();
    let target = target_texture(&device, width, height, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
    let depth = target_texture(&device, width, height, DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
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
        /// A picture of a frame with a draw of the film player, or `None`, and which such
        /// draw (the first is 1).
        shot_film: Option<String>,
        shot_film_frame: u32,
        window: Option<Arc<Window>>,
        surface: Option<wgpu::Surface<'static>>,
        configuration: Option<wgpu::SurfaceConfiguration>,
        player: Option<Player>,
        /// The debug overlay: a panel on the picture of the window, when the game asks.
        overlay: Option<super::overlay::Overlay>,
        depth: Option<Rc<wgpu::TextureView>>,
        /// The back buffer of the game: the frames go into it, and then into the window. It
        /// keeps the last frame for a pixel read of the game.
        back_buffer: Option<wgpu::Texture>,
        /// A frame of the game is in progress.
        in_frame: bool,
        /// The frames that the window did not take: its surface gave no picture.
        not_shown: u32,
        /// The status of the surface when the window last gave no picture, or an empty text.
        surface_status: &'static str,
        data: Vec<u8>,
        frames: u32,
        first_frame: Option<Instant>,
        waited: Duration,
        /// All the time with no record to read, for the debug overlay: `waited` has only
        /// the waits that went back to the events of the window.
        idle: Duration,
        /// The time of the last record, and of the last picture with the sign that the game
        /// sends no frames, with the count of these pictures.
        last_record: Instant,
        sign_at: Instant,
        sign_step: u32,
        records: u64,
        bytes: u64,
        shot_done: bool,
        finished: bool,
    }

    /// The time without a record after which the window shows the sign that the game sends no
    /// frames, and the time of one step of the sign.
    const SIGN_AFTER: Duration = Duration::from_millis(500);
    const SIGN_STEP: Duration = Duration::from_millis(125);

    /// Gets the picture of the window for a frame. A surface that is outdated or lost is
    /// configured again, one time for each frame. The log has each change of the status of a
    /// surface that gives no picture.
    fn acquire(surface: &wgpu::Surface, device: &wgpu::Device, configuration: Option<&wgpu::SurfaceConfiguration>, status: &mut &'static str, frame: u32) -> Option<wgpu::SurfaceTexture> {
        let mut configured_again = false;
        loop {
            let (name, stale) = match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(picture) | wgpu::CurrentSurfaceTexture::Suboptimal(picture) => {
                    if !status.is_empty() {
                        println!("window: the surface gives a picture again at frame {frame}");
                        *status = "";
                    }
                    return Some(picture);
                }
                wgpu::CurrentSurfaceTexture::Outdated => ("outdated", true),
                wgpu::CurrentSurfaceTexture::Lost => ("lost", true),
                wgpu::CurrentSurfaceTexture::Timeout => ("not ready in time", false),
                wgpu::CurrentSurfaceTexture::Occluded => ("hidden", false),
                wgpu::CurrentSurfaceTexture::Validation => ("not valid", false),
            };
            if let (true, false, Some(configuration)) = (stale, configured_again, configuration.filter(|c| c.width != 0 && c.height != 0)) {
                surface.configure(device, configuration);
                configured_again = true;
                continue;
            }
            if *status != name {
                println!("window: no picture at frame {frame}: the surface is {name}{}", if configured_again { ", also after it was configured again" } else { "" });
                *status = name;
            }
            return None;
        }
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
            let depth = target_texture(&player.renderer.device, width, height, DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
            self.depth = Some(Rc::new(depth.create_view(&Default::default())));
        }

        /// Shows the last picture of the game again, with the sign that the game sends no
        /// frames. The game does this at its start and when it loads: the window then has a
        /// part that moves. The frame in progress stays as it is.
        fn show_sign(&mut self) {
            self.sign_at = Instant::now();
            self.sign_step += 1;
            let (Some(surface), Some(player), Some(overlay)) = (self.surface.as_ref(), self.player.as_ref(), self.overlay.as_mut()) else { return };
            let Some(back_buffer) = player.shown_picture() else { return };
            let Some(frame) = acquire(surface, &player.renderer.device, self.configuration.as_ref(), &mut self.surface_status, self.frames) else { return };
            let (device, queue) = (&player.renderer.device, &player.renderer.queue);
            let size = (frame.texture.width(), frame.texture.height());
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_texture(
                back_buffer.as_image_copy(),
                frame.texture.as_image_copy(),
                wgpu::Extent3d { width: back_buffer.width().min(size.0), height: back_buffer.height().min(size.1), depth_or_array_layers: 1 },
            );
            overlay.draw(device, queue, &mut encoder, &frame.texture.create_view(&Default::default()), size, Some(self.sign_step), &[]);
            queue.submit([encoder.finish()]);
            queue.present(frame);
        }

        fn finish(&mut self, event_loop: &ActiveEventLoop, why: &str) {
            self.finished = true;
            // The picture of the window that is in progress goes away before the surface.
            if let Some(player) = self.player.as_mut().filter(|_| !super::gpu::lost()) {
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
            if let Some(line) = self.overlay.as_mut().and_then(|overlay| overlay.age_line()) {
                println!("{line}");
            }
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
            let (renderer, surface) = Renderer::new(self.backends, Some(window.clone()), wgpu::TextureFormat::Bgra8Unorm, crate::extension_shaders(self.extension.as_deref()));
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
            self.overlay = Some(super::overlay::Overlay::new(&renderer.device, renderer.target_format));
            let mut player = Player::new(renderer);
            player.extension = self.extension.take();
            player.renderer.anisotropy = player.extension.as_ref().map_or(1, |extension| extension.anisotropy());
            player.film_shot = self.shot_film.take();
            player.film_shot_at = self.shot_film_frame;
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
            let mut idle_since: Option<Instant> = None;
            loop {
                // The device is lost: nothing draws any more. The records are read and dropped,
                // so that the game does not wait for this program.
                if super::gpu::lost() {
                    match self.ring.pop(&mut self.data) {
                        Some(_) => continue,
                        None if self.ring.closed() => return self.finish(event_loop, "the game closed the stream"),
                        None if waiting.elapsed() > Duration::from_millis(20) => return,
                        None => {
                            std::thread::yield_now();
                            continue;
                        }
                    }
                }
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
                    let since = *idle_since.get_or_insert(clock);
                    if waiting.elapsed() > Duration::from_millis(20) {
                        // Back to the events of the window.
                        self.waited += waiting.elapsed();
                        self.idle += since.elapsed();
                        if self.last_record.elapsed() > SIGN_AFTER && self.sign_at.elapsed() > SIGN_STEP {
                            self.show_sign();
                        }
                        return;
                    }
                    std::thread::yield_now();
                    continue;
                };
                if let Some(since) = idle_since.take() {
                    self.idle += since.elapsed();
                }
                self.last_record = clock;
                self.records += 1;
                self.bytes += 8 + self.data.len() as u64;
                let player = self.player.as_mut().unwrap();
                let clock = Instant::now();
                // The times of the frame in the game, and the time of this read: for the age
                // of the picture in the debug overlay.
                let stamps = (tag == super::tag::FRAME_END && self.data.len() >= 16).then(|| {
                    let wide = |at: usize| i64::from_le_bytes(self.data[at..at + 8].try_into().unwrap());
                    (wide(0), wide(8), super::overlay::ticks())
                });
                let inside_before = super::sum_except(&player.renderer.counts.times, super::TIME_RECORDS);
                let frame_end = player.feed(tag, &self.data);
                let inside_after = super::sum_except(&player.renderer.counts.times, super::TIME_RECORDS);
                // The time of a record without the parts that are measured on their own.
                player.renderer.counts.times[super::TIME_RECORDS] += clock.elapsed().saturating_sub(inside_after - inside_before);
                if let (Some(overlay), Some(record)) = (self.overlay.as_mut(), player.overlay_record.take()) {
                    overlay.record(&record);
                }
                // The age of the pictures of a step of a scenario, after the lines of its mark.
                if tag == super::tag::MARK {
                    if let Some(line) = self.overlay.as_mut().and_then(|overlay| overlay.age_line()) {
                        println!("  {line}");
                    }
                }
                if !frame_end {
                    continue;
                }
                // The frame is complete. It goes into the window when the window takes one.
                if !std::mem::take(&mut self.in_frame) {
                    return;
                }
                let clock = Instant::now();
                let frame = acquire(self.surface.as_ref().unwrap(), &player.renderer.device, self.configuration.as_ref(), &mut self.surface_status, self.frames);
                player.renderer.counts.times[super::TIME_ACQUIRE] += clock.elapsed();
                self.not_shown += frame.is_none() as u32;
                let in_window = frame.is_some();
                let blue_first = matches!(player.renderer.target_format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
                let take_shot = self.shot.is_some() && !self.shot_done && self.frames + 1 >= self.shot_frame;
                let back_buffer = self.back_buffer.as_ref().unwrap();
                let (width, height) = (back_buffer.width(), back_buffer.height());
                let device = player.renderer.device.clone();
                let queue = player.renderer.queue.clone();
                // The lines of the extension for the debug overlay, when it shows.
                let lines = match (self.overlay.as_ref(), player.extension.as_ref()) {
                    (Some(overlay), Some(extension)) if overlay.is_on() => extension.overlay_lines(),
                    _ => Vec::new(),
                };
                let overlay = self.overlay.as_mut();
                let mut read = None;
                player.end_frame(|encoder| {
                    // The picture goes into the window: the part that the two have.
                    if let Some(frame) = frame.as_ref() {
                        encoder.copy_texture_to_texture(
                            back_buffer.as_image_copy(),
                            frame.texture.as_image_copy(),
                            wgpu::Extent3d { width: width.min(frame.texture.width()), height: height.min(frame.texture.height()), depth_or_array_layers: 1 },
                        );
                        // The debug overlay goes on the picture of the window, not into the
                        // back buffer: a picture file and a pixel read do not have it.
                        if let Some(overlay) = overlay {
                            overlay.draw(&device, &queue, encoder, &frame.texture.create_view(&Default::default()), (frame.texture.width(), frame.texture.height()), None, &lines);
                        }
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
                if let Some(overlay) = self.overlay.as_mut() {
                    if let (Some((start, end, read)), true) = (stamps, in_window) {
                        overlay.age(start, end, read, super::overlay::ticks());
                    }
                    overlay.frame(&device, self.idle, player.renderer.counts.draws as u64);
                }
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
        println!("{}", super::cores::prefer_performance_cores());
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
            shot_film: option(args, "--shot-film"),
            shot_film_frame: option(args, "--shot-film-frame").and_then(|text| text.parse().ok()).unwrap_or(120),
            window: None,
            surface: None,
            configuration: None,
            player: None,
            overlay: None,
            depth: None,
            in_frame: false,
            not_shown: 0,
            surface_status: "",
            data: Vec::new(),
            frames: 0,
            first_frame: None,
            waited: Duration::ZERO,
            idle: Duration::ZERO,
            last_record: Instant::now(),
            sign_at: Instant::now(),
            sign_step: 0,
            records: 0,
            bytes: 0,
            shot_done: false,
            finished: false,
        };
        EventLoop::new().expect("an event loop").run_app(&mut app).expect("the event loop");
    }
}

#[cfg(test)]
mod tests {
    use super::VertexLayout;

    #[test]
    fn layout_of_a_format() {
        // D3DFVF_XYZ | D3DFVF_NORMAL | D3DFVF_DIFFUSE | D3DFVF_TEX2: 12 + 12 + 4 + 2 * 8 bytes.
        let layout = VertexLayout::of(0x2 | 0x10 | 0x40 | 0x200, 44).unwrap();
        assert_eq!(layout, VertexLayout { stride: 44, position: 0, normal: Some(12), color: Some(24), coordinates: vec![28, 36] });
        // D3DFVF_XYZ | D3DFVF_TEX1, with no normal and no colour.
        let layout = VertexLayout::of(0x2 | 0x100, 20).unwrap();
        assert_eq!(layout, VertexLayout { stride: 20, position: 0, normal: None, color: None, coordinates: vec![12] });
        // Transformed vertices (D3DFVF_XYZRHW) and blend weights (D3DFVF_XYZB4 with the
        // indexes) have no layout for a held draw.
        assert!(VertexLayout::of(0x4 | 0x40, 20).is_none());
        assert!(VertexLayout::of(0xC | 0x1000 | 0x10, 40).is_none());
    }

    #[test]
    fn the_reach_of_a_light() {
        use super::{LIGHT_REACH_MAX, LightData, light_reach};
        let light = |cut: f32, radius: f32, range: f32| LightData { direction: [0.0, 0.0, 0.0, range], cone: [cut, 0.0, radius, 0.0], ..bytemuck::Zeroable::zeroed() };
        // The cut-off of the light for each pixel, else the radius of the game, else the range.
        assert_eq!(light_reach(&light(1.0 / 512.0, 300.0, 1000.0)), 512.0);
        assert_eq!(light_reach(&light(0.0, 300.0, 1000.0)), 300.0);
        assert_eq!(light_reach(&light(0.0, 0.0, 1000.0)), 1000.0);
        // A range without an end ends at one cell.
        assert_eq!(light_reach(&light(0.0, 0.0, f32::MAX)), LIGHT_REACH_MAX);
        assert_eq!(light_reach(&light(0.0, 0.0, f32::NAN)), LIGHT_REACH_MAX);
    }

    /// A material with an alpha test and a decal: the base pass tests as the material says,
    /// the pass of the decal blends by its alpha with no test (its template, slot 1).
    #[test]
    fn a_later_pass_has_the_alpha_test_of_its_template() {
        use super::material::{derive, Material, ALPHA_TEST, BASE, DECAL0, DEPTH_TEST, WORDS};
        let mut words = [0u32; WORDS];
        words[1] = ALPHA_TEST | DEPTH_TEST;
        // "Greater than" (4) at 128, one decal.
        (words[4], words[5], words[7]) = (4, 128, 1);
        for (map, texture) in [(BASE, 1), (DECAL0, 2)] {
            words[8 + map * 5] = texture;
            words[12 + map * 5] = 1;
        }
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let d = derive(&Material::parse(&bytes).unwrap(), 1);
        assert_eq!(d.passes.len(), 2);
        assert_eq!(d.alpha_tests, vec![[1, 5, 128], [0, 0, 0]]);
        assert_eq!(d.blends[1], [1, 5, 6]);
    }
}
