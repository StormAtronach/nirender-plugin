//! The depth of the first-person scene merged into the copy of the world depth that the
//! post-processing of the extension reads (`Player::merge_first_person_depth`). The game
//! draws the arms and the weapon after a clear of the depth; without the merge a shader of
//! the post chain that reads the depth (a fog, an occlusion) shades them by the depth of
//! what is behind them.
//!
//! A pixel of the first person goes in when it is nearer: its depth becomes a distance with
//! the projection of the first person, and that distance a depth with the projection of the
//! world. A projection of Direct3D gives the depth `d = a + b / z` for a distance `z` in front
//! of the camera, with `a` and `b` its places 10 and 14.

use super::*;

pub(crate) struct DepthMerge {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

const SHADER: &str = "
struct Quad { @builtin(position) position: vec4f }
@vertex fn vs(@builtin(vertex_index) index: u32) -> Quad {
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    return Quad(vec4f(x, y, 0.0, 1.0));
}

// x, y: a and b of the projection of the first person; z, w: those of the world.
// mode.x: 1 when both projections are perspective (else the depth goes in as it is).
struct Values { projections: vec4f, mode: vec4f }
@group(0) @binding(0) var<uniform> v: Values;
@group(0) @binding(1) var arms: texture_depth_2d;

@fragment fn fs(in: Quad) -> @builtin(frag_depth) f32 {
    let d = textureLoad(arms, vec2i(in.position.xy), 0);
    // The clear of the first-person scene: nothing of the arms here.
    if d >= 1.0 {
        discard;
    }
    if v.mode.x < 0.5 {
        return d;
    }
    let z = v.projections.y / (d - v.projections.x);
    return clamp(v.projections.z + v.projections.w / z, 0.0, 1.0);
}
";

impl DepthMerge {
    pub(crate) fn new(device: &wgpu::Device) -> DepthMerge {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("first-person depth merge"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("first-person depth merge"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("first-person depth merge"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("first-person depth merge"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            // The nearer of the two: the depth test keeps the world where it is nearer.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[] }),
            multiview_mask: None,
            cache: None,
        });
        DepthMerge { layout, pipeline }
    }

    /// Writes the depth of `arms` (the depth aspect of a copy with one sample) into `world`
    /// where it is nearer. The projections have the layout of Direct3D.
    pub(crate) fn merge(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, arms: &wgpu::TextureView, world: &wgpu::TextureView, arms_projection: &[f32; 16], world_projection: &[f32; 16]) {
        let perspective = arms_projection[11] == 1.0 && world_projection[11] == 1.0 && arms_projection[14] != 0.0;
        let values: [f32; 8] = [arms_projection[10], arms_projection[14], world_projection[10], world_projection[14], perspective as u32 as f32, 0.0, 0.0, 0.0];
        let buffer = wgpu::util::DeviceExt::create_buffer_init(device, &wgpu::util::BufferInitDescriptor { label: Some("first-person depth merge"), contents: bytemuck::cast_slice(&values), usage: wgpu::BufferUsages::UNIFORM });
        let group = gpu::frame_group(device, &wgpu::BindGroupDescriptor {
            label: Some("first-person depth merge"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(arms) },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("first-person depth merge"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: world,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}
