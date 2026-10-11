//! Draws a capture of the render link stream with wgpu, without the game.
//!
//!     nirender-host <capture file> <picture.png> [--backend vulkan|dx12] [--frame N]
//!
//! The capture comes from `nirender.capture` of the plugin (`src/capture.rs` there has the
//! format). This program reads the records in their order, keeps the textures and the buffers,
//! and draws one frame into a texture, which it saves as a PNG file.
//!
//! It is a spike. What it does not do is in the counts that it prints at the end.

use std::collections::HashMap;
use std::rc::Rc;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

// ---- the capture file

mod tag {
    pub const TEXTURE: u32 = 1;
    pub const BUFFER_DATA: u32 = 2;
    pub const SCENE_BEGIN: u32 = 3;
    pub const FRAME_EVENT: u32 = 5;
    pub const DRAW_STATE: u32 = 7;
    pub const COMMAND: u32 = 8;
    pub const CALL: u32 = 9;
    pub const FRAME_END: u32 = 10;
}

const COMMAND_RENDER_STATE: u32 = 1;
const COMMAND_TRANSFORM: u32 = 3;
const COMMAND_DRAW_INDEXED: u32 = 4;
const CALL_CLEAR: u32 = 4;
const CALL_VIEWPORT: u32 = 5;
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
    pub const DEPTH_ENABLE: usize = 1524;
    pub const DEPTH_FUNCTION: usize = 1528;
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
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct StageData {
    color: [u32; 4],
    alpha: [u32; 4],
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
fn attributes(fvf: u32) -> Option<Vec<Attribute>> {
    use wgpu::VertexFormat::*;
    let position = fvf & 0xE;
    if position == 0x4 || position == 0 {
        // Vertices that are transformed already.
        return None;
    }
    let mut list = vec![Attribute { location: 0, offset: 0, format: Float32x3 }];
    let mut offset = 12u64;
    let betas = (position as u64 - 2) / 2;
    if betas != 0 {
        // The last beta is the matrix indexes when the format says so.
        let weights = if fvf & 0x1000 != 0 { betas - 1 } else { betas };
        let format = match weights {
            0 => None,
            1 => Some(Float32),
            2 => Some(Float32x2),
            _ => Some(Float32x3),
        };
        if let Some(format) = format {
            list.push(Attribute { location: 7, offset, format });
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
    texture_formats: HashMap<u32, u32>,
    missing_textures: u32,
    more_stages: u32,
    more_lights: u32,
    coordinate_work: u32,
}

const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;
const STAGES: usize = 6;

struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    shader: wgpu::ShaderModule,
    draw_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    pipelines: HashMap<PipelineKey, Rc<wgpu::RenderPipeline>>,
    textures: HashMap<u32, Rc<wgpu::TextureView>>,
    white: Rc<wgpu::TextureView>,
    samplers: HashMap<[u32; 5], Rc<wgpu::Sampler>>,
    buffers: HashMap<u32, BufferData>,
    defaults: wgpu::Buffer,
    counts: Counts,
}

impl Renderer {
    fn new(backends: wgpu::Backends) -> Renderer {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends, ..Default::default() });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("no graphics adapter for this backend");
        let info = adapter.get_info();
        println!("adapter: {} ({:?})", info.name, info.backend);
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::TEXTURE_COMPRESSION_BC,
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
            },
            None,
        ))
        .expect("no device");

        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let mut entries = Vec::new();
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
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: STAGES as u32 + i,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &entries });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&draw_layout, &texture_layout],
            push_constant_ranges: &[],
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
        let defaults = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &[0u8; 64],
            usage: wgpu::BufferUsages::VERTEX,
        });
        Renderer {
            device,
            queue,
            shader,
            draw_layout,
            texture_layout,
            pipeline_layout,
            pipelines: HashMap::new(),
            textures: HashMap::new(),
            white,
            samplers: HashMap::new(),
            buffers: HashMap::new(),
            defaults,
            counts: Counts::default(),
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
        // The size of a block, or 0 for 4 bytes for each pixel.
        let (format, block) = match d3d_format {
            0x3154_5844 => (wgpu::TextureFormat::Bc1RgbaUnorm, 8),
            0x3354_5844 => (wgpu::TextureFormat::Bc2RgbaUnorm, 16),
            0x3554_5844 => (wgpu::TextureFormat::Bc3RgbaUnorm, 16),
            21 | 22 => (wgpu::TextureFormat::Bgra8Unorm, 0),
            _ => {
                *self.counts.texture_formats.entry(d3d_format).or_default() += 1;
                return;
            }
        };
        if block != 0 && (width % 4 != 0 || height % 4 != 0) {
            *self.counts.texture_formats.entry(d3d_format).or_default() += 1;
            return;
        }
        let mut bytes = Vec::new();
        for level in 0..levels {
            let (w, h) = (word(data, widths + level * 4) as usize, word(data, heights + level * 4) as usize);
            let size = if block != 0 { w.div_ceil(4).max(1) * h.div_ceil(4).max(1) * block } else { w * h * 4 };
            let from = word(data, offsets + level * 4) as usize;
            let to = (word(data, offsets + (level + 1) * 4) as usize).min(pixels.len());
            let start = bytes.len();
            bytes.extend_from_slice(&pixels[from.min(to)..to.min(from + size)]);
            bytes.resize(start + size, 0);
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
        self.counts.textures += 1;
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
        let sampler = self.samplers.entry(values).or_insert_with(|| {
            Rc::new(device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: address(values[0]),
                address_mode_v: address(values[1]),
                mag_filter: filter(values[2]),
                min_filter: filter(values[3]),
                mipmap_filter: filter(values[4]),
                ..Default::default()
            }))
        });
        sampler.clone()
    }

    fn pipeline(&mut self, key: PipelineKey) -> Option<Rc<wgpu::RenderPipeline>> {
        if !self.pipelines.contains_key(&key) {
            let present = attributes(key.fvf)?;
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
            let from_defaults: Vec<wgpu::VertexAttribute> = (0..8u32)
                .filter(|location| !present.iter().any(|a| a.location == *location))
                .map(|location| wgpu::VertexAttribute { format: INPUT_FORMATS[location as usize], offset: 0, shader_location: location })
                .collect();
            let buffers = [
                wgpu::VertexBufferLayout { array_stride: key.stride as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &from_vertex },
                wgpu::VertexBufferLayout { array_stride: 64, step_mode: wgpu::VertexStepMode::Instance, attributes: &from_defaults },
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
                vertex: wgpu::VertexState { module: &self.shader, entry_point: "vs", compilation_options: Default::default(), buffers: &buffers },
                primitive: wgpu::PrimitiveState { topology, front_face: wgpu::FrontFace::Cw, cull_mode, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: depth_on && key.depth[1] != 0,
                    depth_compare: if depth_on { compare(key.depth[2]) } else { wgpu::CompareFunction::Always },
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: "fs",
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: TARGET_FORMAT, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview: None,
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
    state: Vec<u8>,
}

impl Stream {
    /// The data of the shader for the draw of the current state packet.
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

        let material: [f32; 17] = floats(s, state::MATERIAL);
        data.material_diffuse = material[0..4].try_into().unwrap();
        data.material_ambient = material[4..8].try_into().unwrap();
        data.material_emissive = material[12..16].try_into().unwrap();
        data.scene_ambient = color(known(word(s, state::AMBIENT), 0));
        let state_of = |index: u32| self.render_states.get(&index).copied();
        // D3DRS_FOGCOLOR 34, FOGSTART 36, FOGEND 37, TEXTUREFACTOR 60.
        if let (1, Some(fog_color), Some(start), Some(end)) = (known(word(s, state::FOG_ENABLE), 0), state_of(34), state_of(36), state_of(37)) {
            data.fog_color = color(fog_color);
            data.fog = [f32::from_bits(start), f32::from_bits(end), 1.0, 0.0];
        }
        data.texture_factor = color(state_of(60).unwrap_or(0xFFFF_FFFF));

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
        for i in 0..lights.min(8) {
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
                attenuation: [attenuation[0], attenuation[1], attenuation[2], 0.0],
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
                // D3DTA_TEXTURE is 2 and D3DTA_CURRENT is 1: the defaults of Direct3D.
                let coordinate = known(word(s, at + 9 * 4), stages as u32);
                if coordinate >> 16 != 0 || known(word(s, at + 10 * 4), 0) != 0 {
                    counts.coordinate_work += 1;
                }
                data.stages[stages] = StageData {
                    color: [op, known(word(s, at + 4), 2), known(word(s, at + 8), 1), known(word(s, at + 24), 1)],
                    alpha: [known(word(s, at + 12), 2), known(word(s, at + 16), 2), known(word(s, at + 20), 1), coordinate & 3],
                };
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("use: nirender-host <capture file> <picture.png> [--backend vulkan|dx12] [--frame N]");
        std::process::exit(2);
    }
    let option = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let backends = match option("--backend").as_deref() {
        Some("dx12") => wgpu::Backends::DX12,
        Some("vulkan") => wgpu::Backends::VULKAN,
        _ => wgpu::Backends::PRIMARY,
    };
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
    let wanted = option("--frame").and_then(|text| text.parse().ok()).unwrap_or(frames.saturating_sub(1));
    // The size of the back buffer: the largest viewport of the capture.
    let (mut width, mut height) = (0u32, 0u32);
    for (tag, data) in &records {
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

    let mut renderer = Renderer::new(backends);
    let target_descriptor = |format, usage| wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    };
    let target = renderer
        .device
        .create_texture(&target_descriptor(TARGET_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC));
    let target_view = target.create_view(&Default::default());
    let depth = renderer.device.create_texture(&target_descriptor(DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT));
    let depth_view = depth.create_view(&Default::default());

    let mut stream = Stream {
        view: [0.0; 16],
        projection: [0.0; 16],
        render_states: HashMap::new(),
        viewport: None,
        back_buffer: true,
        origin: None,
        state: Vec::new(),
    };
    let mut encoder = renderer.device.create_command_encoder(&Default::default());
    let mut pass: Option<wgpu::RenderPass<'static>> = None;
    // The pass that draws on what is in the target, or that clears it first.
    let begin = |encoder: &mut wgpu::CommandEncoder, clear_color: Option<wgpu::Color>, clear_depth: Option<f32>, stream: &Stream| {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: clear_color.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations { load: clear_depth.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear), store: wgpu::StoreOp::Store }),
                    stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            })
            .forget_lifetime();
        if let Some((rectangle, range)) = stream.viewport {
            pass.set_viewport(rectangle[0], rectangle[1], rectangle[2], rectangle[3], range[0], range[1]);
        }
        pass
    };

    let mut frame = 0;
    if wanted == 0 {
        pass = Some(begin(&mut encoder, Some(wgpu::Color::BLACK), Some(1.0), &stream));
    }
    for (tag, data) in &records {
        let drawing = frame == wanted;
        match *tag {
            tag::TEXTURE => renderer.texture(data),
            tag::BUFFER_DATA => renderer.buffer_data(data),
            tag::SCENE_BEGIN => {
                stream.origin = (word(data, 20) & 1 != 0).then(|| [double(data, 24), double(data, 32), double(data, 40)]);
            }
            tag::FRAME_EVENT => {
                if word(data, 0) == EVENT_SET_TARGET {
                    stream.back_buffer = word(data, 4) == 0;
                }
            }
            tag::DRAW_STATE => stream.state = data.to_vec(),
            tag::FRAME_END => {
                frame += 1;
                drop(pass.take());
                if frame == wanted {
                    pass = Some(begin(&mut encoder, Some(wgpu::Color::BLACK), Some(1.0), &stream));
                }
            }
            tag::CALL => {
                let arguments = &data[16..];
                match word(data, 0) {
                    CALL_VIEWPORT => {
                        let viewport = &arguments[word(data, 12) as usize * 4..];
                        if viewport.len() >= 24 {
                            let rectangle = [word(viewport, 0) as f32, word(viewport, 4) as f32, word(viewport, 8) as f32, word(viewport, 12) as f32];
                            stream.viewport = Some((rectangle, [float(viewport, 16), float(viewport, 20)]));
                            if let (Some(pass), true) = (pass.as_mut(), stream.back_buffer) {
                                pass.set_viewport(rectangle[0], rectangle[1], rectangle[2], rectangle[3], float(viewport, 16), float(viewport, 20));
                            }
                        }
                    }
                    CALL_CLEAR if drawing && stream.back_buffer => {
                        // Rectangles, their address, flags, colour, depth, stencil.
                        let flags = word(arguments, 8);
                        let clear_color = (flags & 1 != 0).then(|| {
                            let c = color(word(arguments, 12));
                            wgpu::Color { r: c[0] as f64, g: c[1] as f64, b: c[2] as f64, a: c[3] as f64 }
                        });
                        let clear_depth = (flags & 2 != 0).then(|| float(arguments, 16));
                        drop(pass.take());
                        pass = Some(begin(&mut encoder, clear_color, clear_depth, &stream));
                    }
                    _ => {}
                }
            }
            tag::COMMAND => match word(data, 0) {
                COMMAND_RENDER_STATE => {
                    stream.render_states.insert(word(data, 4), word(data, 8));
                }
                COMMAND_TRANSFORM if data.len() >= 16 + 64 => match word(data, 4) {
                    2 => stream.view = floats(data, 16),
                    3 => stream.projection = floats(data, 16),
                    _ => {}
                },
                COMMAND_DRAW_INDEXED if drawing && data.len() >= 16 + 20 && stream.state.len() >= state::SIZE => {
                    if !stream.back_buffer {
                        renderer.counts.other_target += 1;
                        continue;
                    }
                    let s = &stream.state;
                    // Primitive type, least index, vertices, first index, primitives.
                    let (topology, first_index, primitives) = (word(data, 16), word(data, 28), word(data, 32));
                    let indexes = match topology {
                        4 => primitives * 3,
                        5 => primitives + 2,
                        2 => primitives * 2,
                        3 => primitives + 1,
                        1 => primitives,
                        _ => {
                            renderer.counts.topology += 1;
                            continue;
                        }
                    };
                    let key = PipelineKey {
                        fvf: word(s, state::VERTEX_FORMAT),
                        stride: word(s, state::VERTEX_STRIDE),
                        topology,
                        blend: [
                            (known(word(s, state::BLEND_ENABLE), 0) != 0) as u32,
                            known(word(s, state::SOURCE_BLEND), 2),
                            known(word(s, state::DESTINATION_BLEND), 1),
                        ],
                        depth: [
                            (known(word(s, state::DEPTH_ENABLE), 1) != 0) as u32,
                            (known(word(s, state::DEPTH_WRITE), 1) != 0) as u32,
                            known(word(s, state::DEPTH_FUNCTION), 4),
                        ],
                        cull: known(word(s, state::CULL_MODE), 3),
                    };
                    let (vertex_handle, index_handle) = (word(s, state::VERTEX_BUFFER), word(s, state::INDEX_BUFFER));
                    let Some(vertex_buffer) = renderer.gpu_buffer(vertex_handle, false) else {
                        renderer.counts.no_buffer += 1;
                        continue;
                    };
                    let Some(index_buffer) = renderer.gpu_buffer(index_handle, true) else {
                        renderer.counts.no_buffer += 1;
                        continue;
                    };
                    let draw_data = stream.draw_data(&mut renderer.counts);
                    let uniform = renderer.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: bytemuck::bytes_of(&draw_data),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                    let draw_group = renderer.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: None,
                        layout: &renderer.draw_layout,
                        entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
                    });
                    let mut views = Vec::new();
                    let mut samplers = Vec::new();
                    for stage in 0..STAGES {
                        let handle = word(s, state::TEXTURES + stage * 4);
                        let used = stage < draw_data.counts[1] as usize && handle != 0;
                        if used && !renderer.textures.contains_key(&handle) {
                            renderer.counts.missing_textures += 1;
                        }
                        views.push(renderer.textures.get(&handle).filter(|_| used).unwrap_or(&renderer.white).clone());
                        let values: [u32; 5] = core::array::from_fn(|i| word(s, state::SAMPLERS + stage * 20 + i * 4));
                        samplers.push(renderer.sampler(values));
                    }
                    let mut entries = Vec::new();
                    for stage in 0..STAGES {
                        entries.push(wgpu::BindGroupEntry { binding: stage as u32, resource: wgpu::BindingResource::TextureView(&views[stage]) });
                        entries.push(wgpu::BindGroupEntry {
                            binding: (STAGES + stage) as u32,
                            resource: wgpu::BindingResource::Sampler(&samplers[stage]),
                        });
                    }
                    let texture_group =
                        renderer.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &renderer.texture_layout, entries: &entries });
                    let Some(pipeline) = renderer.pipeline(key) else {
                        renderer.counts.no_layout += 1;
                        continue;
                    };
                    let Some(pass) = pass.as_mut() else { continue };
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &draw_group, &[]);
                    pass.set_bind_group(1, &texture_group, &[]);
                    pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, renderer.defaults.slice(..));
                    pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(first_index..first_index + indexes, word(s, state::BASE_VERTEX) as i32, 0..1);
                    renderer.counts.draws += 1;
                }
                _ => {}
            },
            _ => {}
        }
    }
    drop(pass);

    // The picture: the target into a buffer, and the buffer into a PNG file.
    let row = (width as usize * 4).next_multiple_of(256);
    let read = renderer.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * height as usize) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::ImageCopyTexture { texture: &target, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::ImageCopyBuffer { buffer: &read, layout: wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(row as u32), rows_per_image: None } },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    renderer.queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |result| result.expect("the read of the picture"));
    renderer.device.poll(wgpu::Maintain::Wait);
    let mapped = read.slice(..).get_mapped_range();
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        pixels.extend_from_slice(&mapped[y * row..y * row + width as usize * 4]);
    }
    // The alpha of the target is of no use in a picture file.
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(&args[2]).expect("the picture file")), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&pixels).unwrap();

    let c = &renderer.counts;
    println!("draws {}, pipelines {}, textures {}, samplers {}", c.draws, renderer.pipelines.len(), c.textures, renderer.samplers.len());
    println!(
        "not drawn: other target {}, vertex format {}, no buffer data {}, primitive type {}",
        c.other_target, c.no_layout, c.no_buffer, c.topology
    );
    println!(
        "not complete: draws with a texture that is not in the capture {}, more than 6 stages {}, more than 8 lights {}, texture coordinate work {}",
        c.missing_textures, c.more_stages, c.more_lights, c.coordinate_work
    );
    for (format, count) in &c.texture_formats {
        println!("texture format {format:#x} not made: {count}");
    }
    println!("picture: {}", args[2]);
}
