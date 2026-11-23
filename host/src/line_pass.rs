//! The line pass of the debug views (piece 3 of designs/debug-views-2026-11-11): the segments of
//! the frame's [`LineList`] drawn as lines on the picture after the post chain, so that a
//! picture file shows them and the post chain does not change them.
//!
//! A segment of the group [`LineGroup::Tested`] is hidden where the frame drew something nearer:
//! the fragment stage reads the copy of the depth that the post chain reads, and where the
//! game's depth is empty, the depth of what the extension drew before the world (MGE XE's
//! distant land), each with the depth terms of its projection ([`depth`]). The pass has no depth
//! target of its own, so it draws on the picture with one sample, whatever the frame's samples.
//!
//! Made at the first frame with a segment: the pipeline, the buffers and a depth of one pixel
//! for a frame with no depth copy. A frame with no line source makes nothing and costs nothing.

use super::*;
use crate::debug_lines::{LineGroup, LineList};
use crate::objects::{Create, CreateView};

/// The pass's name in the census, the pass report and the GPU time.
pub(crate) const NAME: &str = "debug lines";

/// A vertex of a line: its place relative to the camera, its colour and its group (0 tested,
/// 1 on top).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LineVertex {
    place: [f32; 3],
    color: [u8; 4],
    group: u32,
}

const SHADER: &str = "
// The view (its turn only: the places are relative to the camera) times the projection of the
// game, the layout of Direct3D; the depth terms of the game's projection (xy) and of the depth
// before the world (zw); x: 1 with a depth copy, y: 1 with a depth before the world.
struct Values { view_projection: mat4x4f, terms: vec4f, flags: vec4f }
@group(0) @binding(0) var<uniform> v: Values;
@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var before: texture_depth_2d;

struct Out {
    @builtin(position) position: vec4f,
    @location(0) color: vec4f,
    @location(1) distance: f32,
    @location(2) @interpolate(flat) group: u32,
}

@vertex fn vs(@location(0) place: vec3f, @location(1) color: vec4f, @location(2) group: u32) -> Out {
    // The 16 numbers of Direct3D's layout, read as WGSL's columns: this is `place * matrix`.
    var clip = v.view_projection * vec4f(place, 1.0);
    let distance = clip.w;
    // The pass's own depth: from a distance of 1 along the view, with no far plane. The test
    // against the frame is in the fragment stage.
    clip.z = 0.5 * (clip.w - 1.0);
    return Out(clip, color, distance, group);
}

@fragment fn fs(in: Out) -> @location(0) vec4f {
    if in.group == 0u && v.flags.x > 0.5 {
        let pixel = vec2i(in.position.xy);
        var nearest = 1.0e38;
        let d = textureLoad(depth, pixel, 0);
        if !depth_empty(d) {
            nearest = view_distance(d, v.terms.xy);
        } else if v.flags.y > 0.5 {
            let far = textureLoad(before, pixel, 0);
            if !depth_empty(far) {
                nearest = view_distance(far, v.terms.zw);
            }
        }
        // A margin for the precision of the depth: a line on a surface stays.
        if in.distance > nearest * 1.002 + 2.0 {
            discard;
        }
    }
    return in.color;
}
";

/// The depths that the pass tests against, with the depth terms of their projections.
pub(crate) struct LineDepths<'a> {
    /// The copy of the frame's depth (one sample), and the terms of the game's projection.
    pub(crate) frame: Option<(&'a wgpu::TextureView, [f32; 2])>,
    /// The depth of what the extension drew before the world, and the terms of its projection.
    pub(crate) before: Option<(&'a wgpu::TextureView, [f32; 2])>,
}

pub(crate) struct LinePass {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    /// The pipeline and the format of the target that it draws into.
    pipeline: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    uniform: wgpu::Buffer,
    vertices: Option<wgpu::Buffer>,
    /// A depth of one pixel, cleared, in place of a depth that the frame does not have.
    empty_depth: wgpu::TextureView,
    vertex_data: Vec<LineVertex>,
}

impl LinePass {
    pub(crate) fn new(device: &wgpu::Device) -> LinePass {
        let depth_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.counted_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(NAME),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                depth_entry(1),
                depth_entry(2),
            ],
        });
        let module = device.counted_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(NAME),
            source: wgpu::ShaderSource::Wgsl(format!("{}{}", depth::wgsl(), SHADER).into()),
        });
        let pipeline_layout = device.counted_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(NAME),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let uniform = device.counted_buffer(&wgpu::BufferDescriptor {
            label: Some(NAME),
            size: (16 + 4 + 4) * 4,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let empty_depth = device
            .counted_texture(&wgpu::TextureDescriptor {
                label: Some("debug lines: no depth"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .counted_view(&Default::default());
        LinePass {
            layout,
            pipeline_layout,
            module,
            pipeline: None,
            uniform,
            vertices: None,
            empty_depth,
            vertex_data: Vec::new(),
        }
    }

    fn pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        if self.pipeline.as_ref().is_none_or(|(made_for, _)| *made_for != format) {
            let _long = crate::LongStep::begin();
            let pipeline = device.counted_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(NAME),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: core::mem::size_of::<LineVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Unorm8x4, 2 => Uint32],
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::LineList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
            self.pipeline = Some((format, pipeline));
        }
        &self.pipeline.as_ref().unwrap().1
    }

    /// Draws the segments of `list` on `target` (one sample) with the camera at `eye` (the
    /// world space of the game), the game's `view` and `projection` (the layout of Direct3D).
    /// Returns the number of segments drawn into the pass (those of the tested group can still be
    /// hidden by the depth).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        list: &LineList,
        eye: [f64; 3],
        view: &[f32; 16],
        projection: &[f32; 16],
        depths: &LineDepths,
    ) -> usize {
        if list.is_empty() {
            return 0;
        }
        self.vertex_data.clear();
        for segment in list.segments() {
            let group = match segment.group {
                LineGroup::Tested => 0,
                LineGroup::OnTop => 1,
            };
            for end in [segment.a, segment.b] {
                let place = core::array::from_fn(|i| (end[i] - eye[i]) as f32);
                self.vertex_data.push(LineVertex {
                    place,
                    color: segment.color,
                    group,
                });
            }
        }
        let bytes: &[u8] = bytemuck::cast_slice(&self.vertex_data);
        if self.vertices.as_ref().is_none_or(|buffer| buffer.size() < bytes.len() as u64) {
            self.vertices = Some(device.counted_buffer(&wgpu::BufferDescriptor {
                label: Some(NAME),
                size: (bytes.len() as u64).next_power_of_two().max(4096),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let vertices = self.vertices.as_ref().unwrap();
        queue.write_buffer(vertices, 0, bytes);

        let turn = view_turn(view);
        let view_projection = multiply(&turn, projection);
        let (frame_terms, before_terms) = (
            depths.frame.map_or([0.0; 2], |(_, terms)| terms),
            depths.before.map_or([0.0; 2], |(_, terms)| terms),
        );
        let mut values = [0.0f32; 24];
        values[..16].copy_from_slice(&view_projection);
        values[16..20].copy_from_slice(&[frame_terms[0], frame_terms[1], before_terms[0], before_terms[1]]);
        values[20] = depths.frame.is_some() as u32 as f32;
        values[21] = depths.before.is_some() as u32 as f32;
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&values));

        let frame_depth = depths.frame.map_or(&self.empty_depth, |(view, _)| view);
        let before_depth = depths.before.map_or(&self.empty_depth, |(view, _)| view);
        let group = gpu::frame_group(
            device,
            &wgpu::BindGroupDescriptor {
                label: Some(NAME),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(frame_depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(before_depth),
                    },
                ],
            },
        );
        let count = self.vertex_data.len() as u32;
        let pipeline = self.pipeline(device, target.texture().format()).clone();
        let vertices = self.vertices.as_ref().unwrap();
        crate::gpu::note_other_pass(NAME);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(NAME),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.set_vertex_buffer(
            0,
            vertices.slice(..u64::from(count) * core::mem::size_of::<LineVertex>() as u64),
        );
        pass.draw(0..count, 0..1);
        list.len()
    }
}

/// The view's turn only: its translation set to 0, for places relative to the camera (the layout
/// of Direct3D: the translation is the last row).
fn view_turn(view: &[f32; 16]) -> [f32; 16] {
    let mut turn = *view;
    turn[12] = 0.0;
    turn[13] = 0.0;
    turn[14] = 0.0;
    turn
}

/// `a * b` with the layout of Direct3D (row after row).
fn multiply(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    core::array::from_fn(|i| {
        let (row, column) = (i / 4, i % 4);
        (0..4).map(|k| a[row * 4 + k] * b[k * 4 + column]).sum()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_shader_is_valid_wgsl() {
        crate::assert_wgsl("debug lines", &format!("{}{}", crate::depth::wgsl(), SHADER));
    }

    /// A place relative to the camera goes through the view's turn as a place of the world
    /// goes through the whole view: the translation is the camera's.
    #[test]
    fn the_turn_of_the_view_takes_places_relative_to_the_camera() {
        // A view that turns by 90 degrees about z and moves the camera to (100, 200, 300): the
        // translation row is -(eye * turn).
        let turn: [f32; 16] = [
            0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let eye = [100.0f32, 200.0, 300.0];
        let mut view = turn;
        for column in 0..3 {
            view[12 + column] = -(0..3).map(|k| eye[k] * turn[k * 4 + column]).sum::<f32>();
        }
        let place = [130.0f32, 260.0, 310.0];
        let through = |m: &[f32; 16], p: [f32; 3]| -> [f32; 3] {
            core::array::from_fn(|c| p[0] * m[c] + p[1] * m[4 + c] + p[2] * m[8 + c] + m[12 + c])
        };
        let whole = through(&view, place);
        let relative = through(&view_turn(&view), core::array::from_fn(|i| place[i] - eye[i]));
        for i in 0..3 {
            assert!((whole[i] - relative[i]).abs() < 1e-4, "{whole:?} {relative:?}");
        }
    }

    #[test]
    fn the_vertex_of_a_line_has_twenty_bytes() {
        assert_eq!(core::mem::size_of::<LineVertex>(), 20);
    }
}
