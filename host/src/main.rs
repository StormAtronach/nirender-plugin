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
    pub const HEADER: u32 = 11;
    pub const TEXTURE_UPDATE: u32 = 12;
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
    pub const TEXTURE_TRANSFORMS: usize = 1012;
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
    coordinate: [u32; 4],
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
    texture_updates_lost: u32,
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
    counts: Counts,
}

impl Renderer {
    /// Makes the device. With a window, also the surface of the window, and the format of
    /// the target is a format of that surface.
    fn new(
        backends: wgpu::Backends,
        window: Option<std::sync::Arc<winit::window::Window>>,
        mut target_format: wgpu::TextureFormat,
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
                required_features: wgpu::Features::TEXTURE_COMPRESSION_BC,
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                ..Default::default()
            },
        ))
        .expect("no device");

        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
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
            white,
            samplers: HashMap::new(),
            texture_groups: HashMap::new(),
            sampler_groups: HashMap::new(),
            buffers: HashMap::new(),
            defaults,
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
        self.texture_objects.insert(handle, (texture, d3d_format));
        // A group can have the texture that had this handle before.
        self.texture_groups.clear();
        self.counts.textures += 1;
    }

    /// The game wrote a level of a texture again.
    fn texture_update(&mut self, data: &[u8]) {
        let (handle, level, width, height, size) = (word(data, 0), word(data, 4), word(data, 8), word(data, 12), word(data, 20) as usize);
        let Some((texture, d3d_format)) = self.texture_objects.get(&handle) else {
            self.counts.texture_updates_lost += 1;
            return;
        };
        let block = match d3d_format {
            0x3154_5844 => 8,
            0x3354_5844 | 0x3554_5844 => 16,
            _ => 0,
        };
        let (row, rows, expected) = if block != 0 {
            (width.div_ceil(4) * block, height.div_ceil(4), (width.div_ceil(4) * height.div_ceil(4) * block) as usize)
        } else {
            (width * 4, height, (width * height * 4) as usize)
        };
        if level >= texture.mip_level_count() || size < expected || data.len() < 24 + expected {
            self.counts.texture_updates_lost += 1;
            return;
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &data[24..24 + expected],
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
                primitive: wgpu::PrimitiveState { topology, front_face: wgpu::FrontFace::Cw, cull_mode, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_on && key.depth[1] != 0),
                    depth_compare: Some(if depth_on { compare(key.depth[2]) } else { wgpu::CompareFunction::Always }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: self.target_format, blend, write_mask: wgpu::ColorWrites::ALL })],
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
            // D3DRS_RANGEFOGENABLE is 48.
            data.fog = [f32::from_bits(start), f32::from_bits(end), 1.0, (state_of(48).unwrap_or(0) != 0) as u32 as f32];
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
    /// The size of the back buffer of the game, from the first record. 0 when not known.
    size: (u32, u32),
    /// The size of the targets of the frame in progress.
    target_size: (u32, u32),
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
                state: Vec::new(),
            },
            encoder: None,
            pass: None,
            color: None,
            depth: None,
            size: (0, 0),
            target_size: (0, 0),
        }
    }

    /// Starts a pass on the targets of the frame: it draws on what is there, or it clears first.
    fn open_pass(&mut self, clear_color: Option<wgpu::Color>, clear_depth: Option<f32>) {
        drop(self.pass.take());
        let (Some(encoder), Some(color), Some(depth)) = (self.encoder.as_mut(), self.color.as_ref(), self.depth.as_ref()) else { return };
        let mut pass = encoder
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
                    stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
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
    /// change only the state and the resources.
    fn begin_frame(&mut self, color: Rc<wgpu::TextureView>, depth: Rc<wgpu::TextureView>, size: (u32, u32)) {
        self.target_size = size;
        self.encoder = Some(self.renderer.device.create_command_encoder(&Default::default()));
        self.renderer.draw_slots = 0;
        self.color = Some(color);
        self.depth = Some(depth);
        self.open_pass(Some(wgpu::Color::BLACK), Some(1.0));
    }

    fn drawing(&self) -> bool {
        self.encoder.is_some()
    }

    /// Ends the frame and sends its commands to the GPU. `before_submit` can add commands,
    /// for example a copy of the picture.
    fn end_frame(&mut self, before_submit: impl FnOnce(&mut wgpu::CommandEncoder)) {
        drop(self.pass.take());
        self.color = None;
        self.depth = None;
        if let Some(mut encoder) = self.encoder.take() {
            before_submit(&mut encoder);
            self.renderer.queue.submit([encoder.finish()]);
        }
    }

    /// Does one record. Returns true for the record that ends a frame.
    fn feed(&mut self, tag: u32, data: &[u8]) -> bool {
        match tag {
            tag::HEADER => self.size = (word(data, 0), word(data, 4)),
            tag::TEXTURE => self.renderer.texture(data),
            tag::TEXTURE_UPDATE => self.renderer.texture_update(data),
            tag::BUFFER_DATA => self.renderer.buffer_data(data),
            tag::SCENE_BEGIN => {
                self.stream.origin = (word(data, 20) & 1 != 0).then(|| [double(data, 24), double(data, 32), double(data, 40)]);
            }
            tag::FRAME_EVENT => {
                if word(data, 0) == EVENT_SET_TARGET {
                    self.stream.back_buffer = word(data, 4) == 0;
                }
            }
            tag::DRAW_STATE => self.stream.state = data.to_vec(),
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
                    CALL_CLEAR if self.drawing() && self.stream.back_buffer => {
                        // Rectangles, their address, flags, colour, depth, stencil.
                        let flags = word(arguments, 8);
                        let clear_color = (flags & 1 != 0).then(|| {
                            let c = color(word(arguments, 12));
                            wgpu::Color { r: c[0] as f64, g: c[1] as f64, b: c[2] as f64, a: c[3] as f64 }
                        });
                        let clear_depth = (flags & 2 != 0).then(|| float(arguments, 16));
                        self.open_pass(clear_color, clear_depth);
                    }
                    _ => {}
                }
            }
            tag::COMMAND => match word(data, 0) {
                COMMAND_RENDER_STATE => {
                    self.stream.render_states.insert(word(data, 4), word(data, 8));
                }
                COMMAND_TRANSFORM if data.len() >= 16 + 64 => match word(data, 4) {
                    2 => self.stream.view = floats(data, 16),
                    3 => self.stream.projection = floats(data, 16),
                    _ => {}
                },
                COMMAND_DRAW_INDEXED if self.drawing() && data.len() >= 16 + 20 && self.stream.state.len() >= state::SIZE => self.draw(data),
                _ => {}
            },
            _ => {}
        }
        false
    }

    /// Draws with the current state packet. `data` is the draw command.
    fn draw(&mut self, data: &[u8]) {
        let renderer = &mut self.renderer;
        if !self.stream.back_buffer {
            renderer.counts.other_target += 1;
            return;
        }
        let s = &self.stream.state;
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
        };
        let (vertex_handle, index_handle) = (word(s, state::VERTEX_BUFFER), word(s, state::INDEX_BUFFER));
        let (Some(vertex_buffer), Some(index_buffer)) = (renderer.gpu_buffer(vertex_handle, false), renderer.gpu_buffer(index_handle, true)) else {
            renderer.counts.no_buffer += 1;
            return;
        };
        let draw_data = self.stream.draw_data(&mut renderer.counts);
        let (draw_group, draw_offset) = renderer.draw_slot(&draw_data);
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
        let Some(pass) = self.pass.as_mut() else { return };
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &*draw_group, &[draw_offset]);
        pass.set_bind_group(1, &*texture_group, &[]);
        pass.set_bind_group(2, &*sampler_group, &[]);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, renderer.defaults.slice(..));
        pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(first_index..first_index + indexes, word(s, state::BASE_VERTEX) as i32, 0..1);
        renderer.counts.draws += 1;
    }

    fn print_counts(&self) {
        let c = &self.renderer.counts;
        println!("draws {}, pipelines {}, textures {}, samplers {}", c.draws, self.renderer.pipelines.len(), c.textures, self.renderer.samplers.len());
        println!("not drawn: other target {}, vertex format {}, no buffer data {}, primitive type {}", c.other_target, c.no_layout, c.no_buffer, c.topology);
        println!(
            "not complete: draws with a texture that is not in the stream {}, more than 6 stages {}, more than 8 lights {}; draws with a made or moved texture coordinate {}",
            c.missing_textures, c.more_stages, c.more_lights, c.coordinate_work
        );
        println!("texture levels written again {}, not written {}", c.texture_updates, c.texture_updates_lost);
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let backends = match option(&args, "--backend").as_deref() {
        Some("dx12") => wgpu::Backends::DX12,
        Some("vulkan") => wgpu::Backends::VULKAN,
        _ => wgpu::Backends::PRIMARY,
    };
    if let Some(name) = option(&args, "--live") {
        return live::run(&name, backends, &args);
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

    let mut player = Player::new(Renderer::new(backends, None, wgpu::TextureFormat::Rgba8Unorm).0);
    player.renderer.anisotropy = option(&args, "--anisotropy").and_then(|text| text.parse().ok()).unwrap_or(1);
    let device = player.renderer.device.clone();
    let target = target_texture(&device, width, height, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
    let depth = target_texture(&device, width, height, DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT);
    let mut frame = 0;
    for (tag, data) in &records {
        if frame == wanted && !player.drawing() {
            player.begin_frame(Rc::new(target.create_view(&Default::default())), Rc::new(depth.create_view(&Default::default())), (width, height));
        }
        if player.feed(*tag, data) {
            if player.drawing() {
                let mut read = None;
                player.end_frame(|encoder| read = Some(copy_picture(&device, encoder, &target, width, height)));
                let (buffer, row) = read.unwrap();
                save_picture(&device, &buffer, row, width, height, false, &args[2]);
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

    struct App {
        ring: Ring,
        backends: wgpu::Backends,
        frames_wanted: u32,
        /// The size of the back buffer of the game, and its window when the host goes into it.
        size: (u32, u32),
        parent: Option<isize>,
        shot: Option<String>,
        shot_frame: u32,
        window: Option<Arc<Window>>,
        surface: Option<wgpu::Surface<'static>>,
        configuration: Option<wgpu::SurfaceConfiguration>,
        player: Option<Player>,
        depth: Option<Rc<wgpu::TextureView>>,
        frame: Option<wgpu::SurfaceTexture>,
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
            let depth = target_texture(&player.renderer.device, width, height, DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT);
            self.depth = Some(Rc::new(depth.create_view(&Default::default())));
        }

        fn finish(&mut self, event_loop: &ActiveEventLoop, why: &str) {
            self.finished = true;
            // The picture of the window that is in progress goes away before the surface.
            if let Some(player) = self.player.as_mut() {
                player.end_frame(|_| {});
            }
            drop(self.frame.take());
            let seconds = self.first_frame.map_or(0.0, |start| start.elapsed().as_secs_f64());
            println!("end: {why}");
            println!(
                "frames {}, {:.1} frames in a second, {:.2} ms for each frame; the host waited for the game {:.0} % of the time",
                self.frames,
                self.frames as f64 / seconds.max(0.001),
                seconds * 1000.0 / self.frames.max(1) as f64,
                self.waited.as_secs_f64() * 100.0 / seconds.max(0.001)
            );
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
                Window::default_attributes().with_title("nirender host").with_inner_size(PhysicalSize::new(self.size.0, self.size.1)).with_resizable(false);
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
            let (renderer, surface) = Renderer::new(self.backends, Some(window.clone()), wgpu::TextureFormat::Bgra8Unorm);
            let surface = surface.expect("a surface");
            let size = window.inner_size();
            self.configuration = Some(wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | (renderer.surface_usages & wgpu::TextureUsages::COPY_SRC),
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
            self.player = Some(Player::new(renderer));
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
            let size = self.size;
            let now = self.window.as_ref().unwrap().inner_size();
            self.configure(now.width, now.height);
            let waiting = Instant::now();
            loop {
                // No frame before the size of the game is known: the records before that
                // make the resources only.
                if self.frame.is_none() && self.depth.is_some() && size.0 != 0 {
                    let frame = match self.surface.as_ref().unwrap().get_current_texture() {
                        wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                        _ => return,
                    };
                    let view = Rc::new(frame.texture.create_view(&Default::default()));
                    let frame_size = (frame.texture.width(), frame.texture.height());
                    self.player.as_mut().unwrap().begin_frame(view, self.depth.clone().unwrap(), frame_size);
                    self.frame = Some(frame);
                }
                let Some(tag) = self.ring.pop(&mut self.data) else {
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
                if !player.feed(tag, &self.data) {
                    continue;
                }
                // The frame is complete.
                let Some(frame) = self.frame.take() else { return };
                let can_copy = player.renderer.surface_usages.contains(wgpu::TextureUsages::COPY_SRC);
                let blue_first = matches!(player.renderer.target_format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
                let take_shot = can_copy && self.shot.is_some() && !self.shot_done && self.frames + 1 >= self.shot_frame;
                let (width, height) = (frame.texture.width(), frame.texture.height());
                let device = player.renderer.device.clone();
                let mut read = None;
                player.end_frame(|encoder| {
                    if take_shot {
                        read = Some(copy_picture(&device, encoder, &frame.texture, width, height));
                    }
                });
                if let Some((buffer, row)) = read {
                    save_picture(&device, &buffer, row, width, height, blue_first, self.shot.as_ref().unwrap());
                    self.shot_done = true;
                }
                player.renderer.queue.present(frame);
                self.frames += 1;
                self.first_frame.get_or_insert_with(Instant::now);
                if self.frames_wanted != 0 && self.frames >= self.frames_wanted {
                    return self.finish(event_loop, "all the frames that were asked for");
                }
                return;
            }
        }
    }

    pub fn run(name: &str, backends: wgpu::Backends, args: &[String]) {
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
            backends,
            frames_wanted: option(args, "--frames").and_then(|text| text.parse().ok()).unwrap_or(0),
            size,
            parent: embed.then_some(game_window),
            shot: option(args, "--shot"),
            shot_frame: option(args, "--shot-frame").and_then(|text| text.parse().ok()).unwrap_or(60),
            window: None,
            surface: None,
            configuration: None,
            player: None,
            depth: None,
            frame: None,
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
