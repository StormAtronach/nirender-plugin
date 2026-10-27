//! The depth of the late sorted blends in the post chain's world depth (copy 1): a sorted-blends
//! item after the end of the world (the engine flushed its queue in a later scene) draws after
//! the extension's held draws (the water). Copy 1 takes only what that run wrote: the depth
//! before the run (`before`) and after it (`after`), each the nearest sample of a pixel, and
//! where they differ the nearer of copy 1 and `after`, as `renderDepthAdditional` of the
//! Direct3D 9 path merges its later scenes. The water's depth is in both, so it never enters
//! (depth design, "The post chain's world depth").

use super::*;

pub(crate) struct LateDepth {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
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

const FRAGMENT: &str = "
@group(0) @binding(0) var before: texture_depth_2d;
@group(0) @binding(1) var after: texture_depth_2d;
@fragment fn fs(in: Quad) -> @builtin(frag_depth) f32 {
    let at = vec2i(in.position.xy);
    let d = textureLoad(after, at, 0);
    if d == textureLoad(before, at, 0) {
        discard;
    }
    return d;
}
";

impl LateDepth {
    pub(crate) fn new(device: &wgpu::Device, depth_format: wgpu::TextureFormat) -> LateDepth {
        let texture = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("late depth"), entries: &[texture(0), texture(1)] });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("late depth"), source: wgpu::ShaderSource::Wgsl(format!("{VERTEX}{FRAGMENT}").into()) });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("late depth"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("late depth"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(true),
                // The nearer of copy 1 and the late depth.
                depth_compare: Some(depth::compare(wgpu::CompareFunction::Less)),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[] }),
            multiview_mask: None,
            cache: None,
        });
        LateDepth { layout, pipeline }
    }

    /// Writes into `target` (copy 1) the depth of `after` where it differs from `before`, if
    /// it is nearer.
    pub(crate) fn merge(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, before: &wgpu::TextureView, after: &wgpu::TextureView, target: &wgpu::TextureView) {
        let group = gpu::frame_group(device, &wgpu::BindGroupDescriptor {
            label: Some("late depth"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(before) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(after) }],
        });
        crate::gpu::note_other_pass();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("late depth"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: target,
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

#[cfg(test)]
mod tests {
    #[test]
    fn the_late_depth_shader_is_valid_wgsl() {
        crate::assert_wgsl("late depth", &format!("{}{}", super::VERTEX, super::FRAGMENT));
    }
}
