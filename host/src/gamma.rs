//! The gamma ramp of the device (`IDirect3DDevice8::SetGammaRamp`). The engine sets it from its
//! gamma setting (`NiDX8Application::SetGamma` 0x429C60: one ramp for red, green and blue). A
//! Direct3D 8 device applies the ramp to what it shows. It is not in the back buffer, so a
//! picture file and a pixel read do not have it. Here the ramp is a look-up table in the copy
//! of the back buffer into the window. An identity ramp (gamma 1) keeps the plain copy.

use super::*;

/// The values of a ramp: red, green and blue, 256 each (a `D3DGAMMARAMP`).
pub(crate) const RAMP_VALUES: usize = 768;

/// The ramp of a `GAMMA_RAMP` record (768 little-endian u16), or [`None`] for a short record.
pub(crate) fn read_ramp(data: &[u8]) -> Option<Vec<u16>> {
    (data.len() >= RAMP_VALUES * 2).then(|| data[..RAMP_VALUES * 2].chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect())
}

/// True when the ramp gives each 8-bit value back: the table changes no pixel of the window.
/// The engine's ramp for gamma 1 truncates `i / 255 * 65535`, so a value can be `i * 257 - 1`.
pub(crate) fn is_identity(ramp: &[u16]) -> bool {
    ramp.len() == RAMP_VALUES && ramp.iter().enumerate().all(|(at, &value)| ((value as u32 * 255 + 32767) / 65535) as usize == at % 256)
}

/// The pass of the look-up table: the back buffer into the picture of the window.
pub(crate) struct Gamma {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    /// The ramp as 768 floats from 0 to 1 (192 vectors of 4).
    table: wgpu::Buffer,
}

/// A triangle over the whole target, from the index of the vertex.
const SHADER: &str = "
struct Quad { @builtin(position) position: vec4f }
@vertex fn vs(@builtin(vertex_index) index: u32) -> Quad {
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    return Quad(vec4f(x, y, 0.0, 1.0));
}
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var<uniform> ramp: array<vec4f, 192>;
fn entry(channel: u32, value: f32) -> f32 {
    let at = channel * 256u + u32(round(clamp(value, 0.0, 1.0) * 255.0));
    return ramp[at / 4u][at % 4u];
}
@fragment fn fs(in: Quad) -> @location(0) vec4f {
    let colour = textureLoad(picture, vec2i(in.position.xy), 0);
    return vec4f(entry(0u, colour.r), entry(1u, colour.g), entry(2u, colour.b), colour.a);
}
";

impl Gamma {
    pub(crate) fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Gamma {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gamma ramp"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("gamma ramp"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("gamma ramp"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gamma ramp"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: target_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let table = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gamma ramp"),
            size: (RAMP_VALUES * 4) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Gamma { layout, pipeline, table }
    }

    /// Writes the ramp into the table. The write lands before the command buffers of the next
    /// submit.
    pub(crate) fn set(&self, queue: &wgpu::Queue, ramp: &[u16]) {
        let floats: Vec<u8> = ramp.iter().take(RAMP_VALUES).flat_map(|&value| (value as f32 / 65535.0).to_le_bytes()).collect();
        queue.write_buffer(&self.table, 0, &floats);
    }

    /// Draws `source` (the back buffer) through the table into `target` (the picture of the
    /// window): the part that the two have, as the plain copy; black outside it.
    pub(crate) fn draw(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, source: &wgpu::Texture, target: &wgpu::Texture) {
        let source_view = source.create_view(&Default::default());
        let target_view = target.create_view(&Default::default());
        let group = gpu::frame_group(
            device,
            &wgpu::BindGroupDescriptor {
                label: Some("gamma ramp"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&source_view) },
                    wgpu::BindGroupEntry { binding: 1, resource: self.table.as_entire_binding() },
                ],
            },
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gamma ramp"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_scissor_rect(0, 0, source.width().min(target.width()), source.height().min(target.height()));
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ramp of `NiDX8Application::SetGamma` for a gamma setting.
    fn engine_ramp(gamma: f32) -> Vec<u16> {
        let exponent = if gamma < 1.0 { gamma * 0.99 + 0.01 } else { gamma * 5.0 - 4.0 };
        let one: Vec<u16> = (0..256).map(|i| ((i as f64 * 0.0039215689).powf(1.0 / exponent as f64) * 65535.0) as u16).collect();
        one.iter().chain(&one).chain(&one).copied().collect()
    }

    #[test]
    fn the_engine_ramp_of_gamma_1_is_the_identity() {
        assert!(is_identity(&engine_ramp(1.0)));
        let exact: Vec<u16> = (0..RAMP_VALUES).map(|at| (at % 256) as u16 * 257).collect();
        assert!(is_identity(&exact));
    }

    #[test]
    fn other_gamma_settings_are_not_the_identity() {
        for gamma in [0.0, 0.5, 0.9, 1.1, 1.5, 2.0] {
            assert!(!is_identity(&engine_ramp(gamma)), "gamma {gamma}");
        }
        assert!(!is_identity(&[0; 10]));
    }

    #[test]
    fn a_record_gives_the_768_values_in_order() {
        let bytes: Vec<u8> = (0..RAMP_VALUES as u16).flat_map(|value| (value * 3).to_le_bytes()).collect();
        let ramp = read_ramp(&bytes).unwrap();
        assert_eq!(ramp.len(), RAMP_VALUES);
        assert_eq!((ramp[0], ramp[1], ramp[767]), (0, 3, 767 * 3));
        assert!(read_ramp(&bytes[..1535]).is_none());
    }
}
