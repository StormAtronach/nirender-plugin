//! The [`Extension`] trait: a program that draws more than the game does (the helper of MGE XE),
//! and the types it sees: the parameters, the lights, the facts and the held draws of a frame.

use super::*;

/// The command buffers that an extension makes for a frame (a compute pass, a pass into a
/// texture of its own). They go to the GPU with the next submit of the program, before its
/// own command buffers, in the order that they came: one submit for each frame, as the best
/// practices of wgpu have it. A `queue.write_buffer` goes to the GPU before all the command
/// buffers of that submit: a buffer that two of them read must not get two values in a frame.
/// It also carries the frame's named GPU stamps ([`FrameCommands::stamps`]).
pub struct FrameCommands(pub(crate) core::cell::RefCell<Vec<wgpu::CommandBuffer>>, pub(crate) Rc<crate::stamps::Stamps>);

impl Default for FrameCommands {
    fn default() -> FrameCommands {
        FrameCommands(Default::default(), Rc::new(crate::stamps::Stamps::off()))
    }
}

impl FrameCommands {
    /// The command list of a frame with these stamps.
    pub(crate) fn with_stamps(stamps: Rc<crate::stamps::Stamps>) -> FrameCommands {
        FrameCommands(Default::default(), stamps)
    }

    /// The named GPU stamps of the frame, for the GPU time of the extension's parts with
    /// `MGE_XE_GPU_TIME=1`: a post shader, a pass of its own. Off without it.
    pub fn stamps(&self) -> &crate::stamps::Stamps {
        &self.1
    }

    /// Adds a command buffer for the next submit.
    pub fn defer(&self, buffer: wgpu::CommandBuffer) {
        self.0.borrow_mut().push(buffer);
    }

    /// Sends the command buffers that wait, then `buffer`, to the GPU at once: for work whose
    /// results the caller waits for (time stamps).
    pub fn submit_now(&self, queue: &wgpu::Queue, buffer: wgpu::CommandBuffer) {
        crate::gpu::submit(queue, self.take().into_iter().chain([buffer]));
    }

    /// The command buffers that wait, in their order. The list is then empty.
    pub(crate) fn take(&self) -> Vec<wgpu::CommandBuffer> {
        std::mem::take(&mut *self.0.borrow_mut())
    }
}

/// What a program that draws more than the game does gets at the start of the world of a
/// frame. The matrices have the layout of Direct3D: 16 numbers, row after row, for a
/// position as a row vector (`position * view * projection`).
pub struct SceneView<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    /// For the command buffers of the extension: [`FrameCommands::defer`].
    pub commands: &'a FrameCommands,
    /// The formats of the targets of the pass.
    pub target_format: wgpu::TextureFormat,
    pub depth_format: wgpu::TextureFormat,
    pub target_size: (u32, u32),
    /// The samples for each pixel of the targets of the frame ([`Extension::samples`], cut to
    /// what the adapter has): a pipeline that draws into the pass of the game takes
    /// `multisample`. A pass on a texture of the extension has 1.
    pub samples: u32,
    /// From the world space of the game to the camera, and the projection of the game.
    pub view: [f32; 16],
    pub projection: [f32; 16],
    /// The position of the camera in the world space of the game: the exact origin of the
    /// world camera that the plugin's scene record carries (64-bit), else the position that
    /// `view` gives (an older capture). One source of the eye for an extension: a position
    /// made again from the 32-bit `view` loses precision far from the origin of the world.
    pub eye: [f64; 3],
    /// The player is in an interior cell.
    pub interior: bool,
    /// The game draws the world in wireframe: its shared wireframe property is on (the console
    /// command ToggleWireframe), as the plugin's scene record says. An extension then draws
    /// lines too, as MGE XE on Direct3D 9 does. In an older capture without the flag, the
    /// fill mode of the game's last draw.
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
    /// For the calls of [`ReadingCall`]: the picture of the frame as it is before the call (a
    /// copy), and the depth of the draws of the game so far (in `blend`, the depth comes as
    /// an argument), when [`Extension::reads`] says that the call reads them. [`None`] in the
    /// other calls.
    pub picture: Option<&'a wgpu::TextureView>,
    pub depth: Option<&'a wgpu::TextureView>,
    /// For [`Extension::post`], [`Extension::draw_class`] and [`Extension::draw_held`]: the depth
    /// of what [`Extension::before_world`] drew, when it gave true in this frame (the renderer
    /// then clears the depth for the game). It has the projection of the extension, not that
    /// of the game.
    pub before_depth: Option<&'a wgpu::TextureView>,
    /// For [`Extension::draw_held`]: the lights of the frame, when the extension asked for them
    /// ([`FrameParameters::lights`]) and the frame has a view of the world with lights.
    pub lights: Option<FrameLights<'a>>,
}

impl SceneView<'_> {
    /// The multisample state of a pipeline that draws into the pass of the game. With
    /// `alpha_to_coverage`, the alpha that the fragment shader writes becomes the coverage of
    /// the pixel's samples (for a draw with an alpha test, as the transparency antialiasing
    /// of Direct3D 9 does); it needs more than one sample.
    pub fn multisample(&self, alpha_to_coverage: bool) -> wgpu::MultisampleState {
        wgpu::MultisampleState { count: self.samples, mask: !0, alpha_to_coverage_enabled: alpha_to_coverage && self.samples > 1 }
    }
}

/// A picture file that an extension wants of a frame. See [`Extension::pictures`].
pub struct PictureRequest {
    /// The file. Its extension gives the format: `.png`, `.bmp`, `.tga`, `.dds`, `.jpg`;
    /// PNG for another one.
    pub path: String,
    /// With the interface of the game (the picture at the end of the frame), or without it
    /// (the picture after [`Extension::post`]).
    pub with_ui: bool,
}

/// The text that an extension shows in the upper left corner of the window. See
/// [`Extension::status`].
#[derive(Default)]
pub struct Status {
    /// A message, and true when it is a warning or an error (red).
    pub message: Option<(String, bool)>,
    /// True to show the number of frames in a second that go to the window.
    pub frame_rate: bool,
}

/// The answers of an extension for a frame. See [`Extension::frame`].
#[derive(Clone, Default)]
pub struct FrameParameters {
    /// The values of the extension for the frame, for its shader functions: `frame.values`
    /// in WGSL (bind group 0, binding 0).
    pub values: [[f32; 4]; 32],
    /// A texture of the extension for its shader functions: `extension_texture` with
    /// `extension_sampler` (linear, clamp) in WGSL. White without one. The view must stay
    /// valid for the frame; the extension draws into the texture before the draws of the
    /// game read it (in `before_world`, in a pass of its own).
    pub texture: Option<Rc<wgpu::TextureView>>,
    /// The distances of the start and of the end of the fog for the draws of the world of
    /// the game, in place of those of the game: so that they go into the fog as what the
    /// extension draws does. [`None`]: the fog of the game.
    pub fog_range: Option<(f32, f32)>,
    /// True when the extension shades the draws of the world: the renderer calls
    /// [`Extension::shade`] for each of them, and its shader uses the functions of
    /// [`Extension::draw_shader`] for a draw that the call marks.
    pub shade: bool,
    /// The values for the sky functions of [`Extension::sky_shader`], or [`None`] for the sky
    /// of the game as it is. With values, the sky dome of the game (a sky draw without a
    /// texture) gets its shape and its colour from the functions, and the shadow of a moon
    /// gets its colour from them and keeps its alpha.
    pub sky: Option<[[f32; 4]; 8]>,
    /// True when the extension wants the call [`Extension::blend`] in this frame.
    pub blend: bool,
    /// The classes of the game (MGE_LINK_CLASS_*, one bit for each) whose draws into the
    /// back buffer the extension takes: see [`Extension::draw_class`].
    pub watched: u32,
    /// True when the extension wants the call [`Extension::post`] in this frame.
    pub post: bool,
    /// A class in `watched` and a plane for its call, `[a, b, c, d]` in the space of the
    /// world of the game, with the camera on the side where `a * x + b * y + c * z + d >= 0`:
    /// a surface of water. The world draws that blend and write no depth (particles, glows,
    /// smoke) go into the picture on the other side of the plane before the call (so that the
    /// picture of the call has them, for a refraction), and on the side of the camera after
    /// the call, over what the call drew: each of them, whether the game sends it before or
    /// after the call. [`None`]: the draws that blend come in their place in the order of the
    /// game.
    pub class_plane: Option<(u32, [f64; 4])>,
    /// True when the extension takes draws of the world: the renderer calls [`Extension::hold`]
    /// for each shape of the world, and gives the shapes it took to [`Extension::draw_held`].
    pub hold: bool,
    /// True when the extension lights what it draws with the lights of the frame, for each
    /// pixel: [`SceneView::lights`] in [`Extension::draw_held`].
    pub lights: bool,
    /// True when a pixel of a draw that the extension shades takes its lights from the
    /// clusters of the frame (`cluster_lights` in the shader), with the mask of the lights
    /// of the draw: the same picture as the lights of the draw. `MGE_XE_CLUSTERS` (0 or 1)
    /// overrides it.
    pub clustered_lights: bool,
    /// True when a lit draw of the world takes the point lights of the world that reach its
    /// pixel (`world_lights_here` in the shader), not the lights the engine chose for it.
    /// `MGE_XE_LIGHTS=world` makes it true.
    pub world_lights: bool,
    /// The reach of the extension's shadow map in units of the world, 0 without shadows:
    /// the plugin animates what the map reaches, so that a shadow moves while its caster
    /// is out of view.
    pub shadow_reach: f32,
}

/// The lights of the world of a frame, for an extension that lights what it draws as the
/// draws of the game are lit ([`FrameParameters::lights`]). The lights are those of the draws
/// after [`Extension::shade`], one copy of each light, binned into clusters of their own: tiles
/// of the picture by slices of the view depth of the game.
///
/// - `lights`: an array of [`LightData`] (storage). A position is relative to `origin`.
/// - `clusters` (uniform, 64 bytes): `grid: vec4u` (tiles across, tiles down, slices; w 2),
///   `screen: vec4f` (x and y: tiles for each pixel), `depth: vec4f` (the slice of a view
///   depth z is `log(max(z, depth.z)) * depth.x + depth.y`, clamped to the slices), then 16
///   bytes the extension's clusters do not use.
/// - `table` (storage, `array<vec2u>`): for the cluster `(slice * grid.y + y) * grid.x + x`,
///   the first entry in `indexes` and the number of entries.
/// - `indexes` (storage, `array<u32>`): the index of a light in `lights`.
///
/// A light is in each cluster that its reach touches: the cut-off of `shade` when
/// `cone.x` is more than 0 (`cone.x` is 1 over it), else the radius of the light of the game
/// (`cone.z`), else its range (`direction.w`), at most [`LIGHT_REACH_MAX`]. A directional light
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
/// ([`SceneView::picture`], [`SceneView::depth`]). See [`Extension::reads`].
#[derive(Clone, Copy)]
pub enum ReadingCall<'a> {
    /// `Extension::blend`: only the picture counts (its depth is an argument).
    Blend,
    /// [`Extension::draw_class`] for this class.
    Class(u32),
    /// [`Extension::draw_held`] with these shapes.
    Held(&'a [HeldDraw]),
    /// [`Extension::draw_held_in_place`] with this shape.
    HeldInPlace(&'a HeldDraw),
    /// [`Extension::post`].
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

/// The light that a pixel shows at the least: one level of 8 bits.
const LIGHT_VISIBLE: f32 = 1.0 / 256.0;

/// The reach of a point or spot light in the clusters: see [`FrameLights`].
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

/// The reach of a point or spot light in the clusters of the game's draws. With a cut-off
/// (the extension's fade: `cone[0]` is 1 over it), the cut-off. Without one (a light with no
/// radius, as a mod's light made in Lua; or the fade off), the distance where its own
/// attenuation brings its brightest colour under one level: the engine's light has no end,
/// and a draw takes a clustered light only when the light is one of its own, so the reach
/// only bins it.
pub(crate) fn cluster_reach(light: &LightData) -> f32 {
    let reach = if light.cone[0] > 0.0 { 1.0 / light.cone[0] } else { attenuation_reach(light) };
    reach.min(LIGHT_REACH_MAX).max(0.0)
}

/// The distance where `colour / (a0 + a1 d + a2 d^2)` falls under [`LIGHT_VISIBLE`], within
/// the range of Direct3D (`direction[3]`) when the light has one.
fn attenuation_reach(light: &LightData) -> f32 {
    let brightest = light.diffuse[0].max(light.diffuse[1]).max(light.diffuse[2]).abs();
    let [a0, a1, a2, _] = light.attenuation;
    // Where the attenuation is `brightest / LIGHT_VISIBLE`.
    let limit = brightest / LIGHT_VISIBLE;
    let reach = if a2 > 0.0 {
        ((a1 * a1 - 4.0 * a2 * (a0 - limit)).max(0.0).sqrt() - a1) / (2.0 * a2)
    } else if a1 > 0.0 {
        (limit - a0) / a1
    } else {
        LIGHT_REACH_MAX
    };
    let range = light.direction[3];
    if range > 0.0 { reach.min(range) } else { reach }
}

/// The facts of a draw that are not in its uniform, for [`Extension::shade`] and
/// [`Extension::hold`].
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

/// The kinds of a draw of the retained table for [`Extension::shade_table`]: the four blend
/// forms (opaque, alpha, alpha that adds, colour that adds), each without and with a depth
/// write. A slot of the table takes the shade of its kind (`retained::slot_kind`).
pub const TABLE_KINDS: [DrawFacts; 8] = {
    const FORMS: [(bool, u32, u32); 4] = [(false, 0, 0), (true, 5, 6), (true, 2, 6), (true, 5, 2)];
    let mut kinds = [DrawFacts { class: 12, blend_enable: false, source_blend: 0, destination_blend: 0, depth_write: false, specular_power: 0.0, emissive: [0.0; 4] }; 8];
    let mut k = 0;
    while k < 8 {
        let (blend, source, destination) = FORMS[k / 2];
        kinds[k].blend_enable = blend;
        kinds[k].source_blend = source;
        kinds[k].destination_blend = destination;
        kinds[k].depth_write = k % 2 == 1;
        k += 1;
    }
    kinds
};

/// What the extension's shade does to the retained draws of a frame, one time for the
/// frame ([`Extension::shade_table`]). The extension states it from its own state; the table
/// does not infer it from stand-in draws.
#[derive(Clone, Copy, Debug)]
pub struct TableShade {
    /// The factor for each ambient light of the table (the scene's, and the one of each
    /// instance's lights record): what `shade` does to [`DrawData::scene_ambient`] of a draw
    /// of the world.
    pub ambient_scale: [f32; 3],
    /// [`DrawData::extension`] 5 and 7 of a draw of the world that writes depth and does not
    /// blend: the sun of the shadow.
    pub extension5: [f32; 4],
    pub extension7: [f32; 4],
    /// The fog is on the distance from the camera ([`DrawData::fog`] w).
    pub radial_fog: bool,
    /// The bits of `counts.w` for a draw of each kind of [`TABLE_KINDS`].
    pub flags: [u32; 8],
}

impl Default for TableShade {
    fn default() -> TableShade {
        TableShade { ambient_scale: [1.0; 3], extension5: [0.0; 4], extension7: [0.0; 4], radial_fog: false, flags: [0; 8] }
    }
}

/// Where the parts of a vertex of a draw of the game are: offsets in bytes from the start of
/// the vertex, as the renderer reads the flexible vertex format of Direct3D 8.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VertexLayout {
    /// The bytes from one vertex to the next.
    pub stride: u64,
    /// Three floats.
    pub position: u64,
    /// Three floats, or [`None`].
    pub normal: Option<u64>,
    /// A D3DCOLOR (`Unorm8x4`: blue, green, red, alpha), or [`None`].
    pub color: Option<u64>,
    /// Two floats for each set of texture coordinates (four at most).
    pub coordinates: Vec<u64>,
}

impl VertexLayout {
    /// The layout of a format with no blend weights. [`None`] for one that the renderer does not
    /// draw, or that has blend weights.
    pub(crate) fn of(fvf: u32, stride: u32) -> Option<VertexLayout> {
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
    /// The map in the texturing property: [`material::BASE`] to `material::DECAL0 + 1`.
    pub map: usize,
    pub texture: Rc<wgpu::TextureView>,
    /// The set of texture coordinates of the map, in [`VertexLayout::coordinates`].
    pub set: u32,
    /// The sampler states: address U, address V, the magnify, minify and mip filters, as
    /// Direct3D 8 has them.
    pub sampler: [u32; 5],
}

/// A shape of the world of the game that the extension took ([`Extension::hold`]): the renderer
/// draws none of its passes, and the extension draws it in its own way in
/// [`Extension::draw_held`]. It is never an instance of another draw.
#[derive(Clone)]
pub struct HeldDraw {
    pub facts: DrawFacts,
    /// The buffer that holds the vertices of the shape (a pool of the renderer: other shapes
    /// are in it too), and the first vertex of the shape in it, for `draw_indexed`.
    pub vertices: Rc<wgpu::Buffer>,
    pub base_vertex: i32,
    /// The indexes, 16 bits each, in a buffer that holds other shapes too.
    pub indexes: Rc<wgpu::Buffer>,
    pub layout: VertexLayout,
    pub topology: wgpu::PrimitiveTopology,
    /// The parts of the shape: the first index in `indexes` and the number of indexes of
    /// each. A strip topology has one part for each strip.
    pub parts: Vec<(u32, u32)>,
    /// The uniform that the renderer made for the first pass of the shape: the world matrix,
    /// the view and the projection of the draw (see `origin`), the material, the fog and the
    /// texture stages. Its `light_range` counts in `lights`.
    pub data: DrawData,
    /// The origin that the world matrix is relative to, or [`None`]. With an origin, the view
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

    /// The room of the texture arrays from the settings of the extension: 0 is automatic
    /// (the adapter's room, at most `AUTO_TEXTURE_SLOTS`), another number the most slots
    /// (at least `MIN_TEXTURE_SLOTS`, at most the adapter's room). The renderer asks one time,
    /// when it makes the device.
    fn texture_slots(&self) -> u32 {
        0
    }

    /// The samples for each pixel of the targets of the frame, from the settings of the
    /// extension: 1, 2, 4 or 8. The renderer asks one time, when it makes the device, and
    /// takes the largest count that the adapter has for its formats, up to this one. With
    /// more than 1, the draws of the game and of the extension into the pass of the game
    /// go into multisampled targets; the picture and the depth that the extension reads are
    /// resolved (the colour as the average of the samples, the depth as the nearest sample).
    fn samples(&self) -> u32 {
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

    /// The text for the upper left corner of the window: a message and the frame rate. It
    /// goes on the picture of the window only, as the debug overlay: the back buffer, the
    /// picture files and the capture files do not have it. The renderer asks at the end of
    /// each frame that goes to the window.
    fn status(&self) -> Status {
        Status::default()
    }

    /// Messages for the part of the extension in the game process: (a number that the two
    /// parts agree on, the data). The renderer asks at the end of each live frame and puts
    /// them into the message memory of the stream; the plugin reads it at the end of a frame
    /// of the game and gives each message to `MGE_StreamMessage` of the `d3d8.dll` of MGE XE.
    /// A replay of a capture drops them.
    fn messages(&mut self) -> Vec<(u32, Vec<u8>)> {
        Vec::new()
    }

    /// The renderer calls this at the end of each live frame, also of a frame with no world
    /// (a loading screen), before `messages`: for work that must go on while the game shows
    /// no world, such as the loads and the uploads for a place where the camera will be. The
    /// scene has the facts of the last world draw. Its writes to the queue and its deferred
    /// commands go with the next frame.
    fn after_frame(&mut self, _scene: &SceneView) {}

    /// The renderer draws a capture file and not a live stream: the frames must be the
    /// same each time. The extension completes each load before a frame, with no limit on
    /// the work of a frame. The renderer calls this one time, before the first record.
    fn offline(&mut self) {}

    /// WGSL source with the two sky functions of the extension, or [`None`]. The renderer
    /// asks one time, before it makes its shader, and puts the source at the end of it:
    ///
    /// - `fn extension_sky_position(position: vec3f) -> vec3f`: the position of a vertex of
    ///   the sky dome in its model, for a dome of another shape.
    /// - `fn extension_sky_color(direction: vec3f, pixel: vec2f) -> vec3f`: the colour of
    ///   the sky in a direction from the camera (length 1, in the space of the world), for
    ///   the pixel at these coordinates.
    ///
    /// The source reads the values of [`FrameParameters::sky`] as `draw.extension[0]` to
    /// `draw.extension[7]`.
    fn sky_shader(&self) -> Option<String> {
        None
    }

    /// WGSL source with the two draw functions of the extension, or [`None`]. The renderer
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
    /// The functions read the uniform of the draw as `draw` (see [`DrawData`]); the colour of
    /// such a draw is not kept between 0 and 1 after the stages.
    fn draw_shader(&self) -> Option<String> {
        None
    }

    /// Changes the uniform of a world draw and its lights before the renderer puts the draw
    /// into the pass: the lights, the ambient light, the fog. Called for each draw of the
    /// world of the game into the back buffer with a perspective projection, when
    /// [`FrameParameters::shade`] is true. The extension sets bit 1 of `counts[3]` for a draw
    /// that its shader functions shade; bit 2 and the bits from 16 are its own.
    fn shade(&mut self, _draw: &mut DrawData, _lights: &mut [LightData], _facts: &DrawFacts) {}

    /// The shade of the retained table for a frame, one time at the start of its world, when
    /// [`FrameParameters::shade`] is true: as `shade` does to a draw of the world with this
    /// view, for each kind of [`TABLE_KINDS`]. The extension changes the lights of the table
    /// as `shade` changes the lights of such a draw.
    fn shade_table(&mut self, _view: &[f32; 16], _lights: &mut [LightData], _kinds: &[DrawFacts; 8]) -> TableShade {
        TableShade::default()
    }

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
    /// in [`FrameParameters::watched`], in place of the draws of that class: none of them is
    /// drawn. The pass and the matrices are those of that draw. A frame with no draw of the
    /// class gets the call too, before the sorted blends or at the world's end, with the
    /// matrices of the world (the engine culls a plane out of the view). `scene.picture` is the
    /// picture of the frame before the call and `scene.depth` is the depth of the game so
    /// far: for a refraction. `scene.before_depth` is the depth of what `before_world` drew,
    /// when it gave true. MGE XE draws its water in place of the water plane of the game
    /// (class 10).
    fn draw_class(&mut self, _class: u32, _scene: &SceneView, _pass: &mut wgpu::RenderPass<'static>, _replay: &mut dyn Replay) {}

    /// Takes a shape of the world of the game away from the renderer, when
    /// [`FrameParameters::hold`] is true: none of its passes is drawn, and the extension gets
    /// it in `draw_held`. Called one time for each shape record into the back buffer in the
    /// scenes of the world (with a perspective projection), but not for the sky, the water
    /// plane of the game, a skinned shape, a particle system or lines. `facts` are those of
    /// the first pass.
    fn hold(&mut self, _facts: &DrawFacts) -> bool {
        false
    }

    /// True when the extension draws every shape with these facts itself whenever it can
    /// (`hold`): whose the shape is, not whether the extension is ready this frame. The
    /// retained table asks this once for each instance it takes in, and offers a claimed
    /// instance to `hold` in each frame it is in view: a held instance is drawn by the
    /// extension (`draw_held` or `draw_held_in_place`), not by the table, in each view of
    /// that frame. Not asked for the sky, the water plane of the game or a skinned shape.
    fn claims(&self, _facts: &DrawFacts) -> bool {
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
    /// interface scene, or at the end of the frame without one), when [`FrameParameters::post`]
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

    /// The renderer calls this after `after_world`, when [`FrameParameters::blend`] was true
    /// and `before_world` gave true. The pass draws into the picture of the frame and has
    /// no depth target. `before` is the picture as it was after `before_world`, `depth`
    /// is the depth of the draws of the game (1 where the game drew nothing), and
    /// `scene.picture` is a copy of the picture as it is now. MGE XE lets the draws of the
    /// game near the end of their view range go into its distant land here.
    fn blend(&mut self, _scene: &SceneView, _pass: &mut wgpu::RenderPass<'static>, _before: &wgpu::TextureView, _depth: &wgpu::TextureView) {}

    /// What `call` reads of the picture and of the depth, asked just before the call: the
    /// renderer copies only these, and gives [`None`] in [`SceneView`] for the others. A picture
    /// that a draw only modulates needs no copy: a blend state reads the target.
    fn reads(&self, _call: ReadingCall) -> Reads {
        Reads::ALL
    }
}

/// The WGSL of the extension at the end of the shader: its sky functions and its draw
/// functions, or those that change nothing.
pub(crate) fn extension_shaders(extension: Option<&dyn Extension>) -> String {
    let sky = extension.and_then(|extension| extension.sky_shader()).unwrap_or_else(|| {
        "fn extension_sky_position(position: vec3f) -> vec3f { return position; }\nfn extension_sky_color(direction: vec3f, pixel: vec2f) -> vec3f { return vec3f(0.0); }\n".to_string()
    });
    let draw = extension.and_then(|extension| extension.draw_shader()).unwrap_or_else(|| {
        "fn extension_shade(in: VertexOut) -> vec4f { return in.vertex_color; }\nfn extension_finish(color: vec3f, alpha: f32, clear: f32) -> vec4f { return vec4f(mix(draw.fog_color.rgb, color, clear), alpha); }\n".to_string()
    });
    format!("{sky}\n{draw}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    fn point(diffuse: f32, attenuation: [f32; 3], cut: f32) -> LightData {
        let mut light = LightData::zeroed();
        light.diffuse = [diffuse, diffuse * 0.5, 0.0, 0.0];
        light.position[3] = 1.0;
        light.attenuation = [attenuation[0], attenuation[1], attenuation[2], 0.0];
        light.cone[0] = cut;
        light
    }

    #[test]
    fn a_clustered_light_with_a_cut_off_reaches_to_it() {
        assert_eq!(cluster_reach(&point(1.0, [0.0, 0.0, 1e-5], 1.0 / 512.0)), 512.0);
    }

    #[test]
    fn a_clustered_light_without_a_cut_off_reaches_where_its_attenuation_ends() {
        // 1 / (1e-5 d^2) = 1/256 at d = sqrt(256 / 1e-5) = 5059.6.
        let reach = cluster_reach(&point(1.0, [0.0, 0.0, 1e-5], 0.0));
        assert!((reach - 5059.6).abs() < 1.0, "{reach}");
        // Linear: 1 / (0.01 d) = 1/256 at d = 25600, past one cell.
        assert_eq!(cluster_reach(&point(1.0, [0.0, 0.01, 0.0], 0.0)), LIGHT_REACH_MAX);
        // A constant attenuation has no end: one cell.
        assert_eq!(cluster_reach(&point(1.0, [1.0, 0.0, 0.0], 0.0)), LIGHT_REACH_MAX);
        // A black light reaches nothing.
        assert_eq!(cluster_reach(&point(0.0, [0.0, 0.0, 1e-5], 0.0)), 0.0);
    }

    #[test]
    fn the_range_of_direct3d_bounds_the_reach_of_a_clustered_light() {
        let mut light = point(1.0, [0.0, 0.0, 1e-5], 0.0);
        light.direction[3] = 300.0;
        assert_eq!(cluster_reach(&light), 300.0);
    }
}
