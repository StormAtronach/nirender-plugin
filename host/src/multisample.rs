//! The targets of the frame with more than one sample for each pixel ([`Extension::samples`]).
//! The pass of the game draws into a multisampled colour target and a multisampled depth
//! target; the colour resolves into the back buffer at the end of each pass (the
//! `resolve_target` of the pass, the average of the samples). This file has the two passes
//! that wgpu does not do by itself:
//! - the resolve of the depth into a texture with one sample, for the readers of the depth
//!   (the post-processing, the water, the blend): the nearest sample of each pixel, as the
//!   depth resolve of DXVK that the Direct3D 9 path of MGE XE calls (`ResolveDepthMinV1`,
//!   `VK_RESOLVE_MODE_MIN_BIT`);
//! - the load of the back buffer into the multisampled colour target, after the extension
//!   wrote the back buffer outside the pass (its post-processing): the draws after it (the
//!   interface of the game) go into the multisampled target again.

use super::*;

pub(crate) struct Multisample {
    depth_layout: wgpu::BindGroupLayout,
    depth_pipeline: wgpu::RenderPipeline,
    color_layout: wgpu::BindGroupLayout,
    color_pipeline: wgpu::RenderPipeline,
}

/// A triangle over the whole target, from the index of the vertex.
const VERTEX: &str = "
struct Quad { @builtin(position) position: vec4f }
@vertex fn vs(@builtin(vertex_index) index: u32) -> Quad {
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    return Quad(vec4f(x, y, 0.0, 1.0));
}
";

const DEPTH_SHADER: &str = "
@group(0) @binding(0) var samples: texture_depth_multisampled_2d;
@fragment fn fs(in: Quad) -> @builtin(frag_depth) f32 {
    let at = vec2i(in.position.xy);
    var nearest = textureLoad(samples, at, 0);
    let count = i32(textureNumSamples(samples));
    for (var s = 1; s < count; s++) {
        nearest = min(nearest, textureLoad(samples, at, s));
    }
    return nearest;
}
";

const COLOR_SHADER: &str = "
@group(0) @binding(0) var picture: texture_2d<f32>;
@fragment fn fs(in: Quad) -> @location(0) vec4f {
    return textureLoad(picture, vec2i(in.position.xy), 0);
}
";

impl Multisample {
    pub(crate) fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat, samples: u32) -> Multisample {
        let depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("depth resolve"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: true },
                count: None,
            }],
        });
        let depth_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("depth resolve"), source: wgpu::ShaderSource::Wgsl(format!("{VERTEX}{DEPTH_SHADER}").into()) });
        let depth_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("depth resolve"), bind_group_layouts: &[Some(&depth_layout)], immediate_size: 0 });
        let depth_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("depth resolve"),
            layout: Some(&depth_pipeline_layout),
            vertex: wgpu::VertexState { module: &depth_module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &depth_module, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[] }),
            multiview_mask: None,
            cache: None,
        });
        let color_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("colour load"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            }],
        });
        let color_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("colour load"), source: wgpu::ShaderSource::Wgsl(format!("{VERTEX}{COLOR_SHADER}").into()) });
        let color_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("colour load"), bind_group_layouts: &[Some(&color_layout)], immediate_size: 0 });
        let color_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("colour load"),
            layout: Some(&color_pipeline_layout),
            vertex: wgpu::VertexState { module: &color_module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &color_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: target_format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Multisample { depth_layout, depth_pipeline, color_layout, color_pipeline }
    }

    /// Writes the nearest sample of each pixel of `source` (a view of the multisampled depth,
    /// its depth aspect only) into `target` (a depth texture with one sample).
    pub(crate) fn resolve_depth(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, source: &wgpu::TextureView, target: &wgpu::TextureView) {
        let group = gpu::frame_group(device, &wgpu::BindGroupDescriptor {
            label: Some("depth resolve"),
            layout: &self.depth_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(source) }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("depth resolve"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: target,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0), store: wgpu::StoreOp::Discard }),
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.depth_pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Writes the picture of `source` (one sample) into every sample of `target` (the
    /// multisampled colour target).
    pub(crate) fn load_color(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, source: &wgpu::TextureView, target: &wgpu::TextureView) {
        let group = gpu::frame_group(device, &wgpu::BindGroupDescriptor {
            label: Some("colour load"),
            layout: &self.color_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(source) }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("colour load"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.color_pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}
