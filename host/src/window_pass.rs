//! The picture of the game into a window of another size: scaled, with its aspect kept and
//! black bars where the window's aspect differs (the release bar's item: a fullscreen game
//! covers its monitor without a change of the display mode, and a lower fullscreen size fills
//! it as Direct3D 9's change of the mode did). [`fit`] gives the rectangle; [`Scale`] draws
//! the back buffer into it with a linear filter, through the gamma ramp of the device when the
//! game set one. A window of the picture's size keeps the exact copy of `live::to_window`.

use super::*;
use wgpu::util::DeviceExt;

/// The rectangle of the window that the picture covers: x, y, width, height. The largest with
/// the picture's aspect, centred, in whole pixels; the picture's own size and place when the
/// window has its size. A window or a picture with no pixels gives the whole window.
pub(crate) fn fit(picture: (u32, u32), window: (u32, u32)) -> [u32; 4] {
    let ((pw, ph), (ww, wh)) = (picture, window);
    if pw == 0 || ph == 0 || ww == 0 || wh == 0 {
        return [0, 0, ww, wh];
    }
    // The picture's width at the window's height, and its height at the window's width, as
    // whole numbers rounded to the nearest; the side that fits decides.
    let width_at_height = ((pw as u64 * wh as u64 + ph as u64 / 2) / ph as u64) as u32;
    let (w, h) = if width_at_height <= ww {
        (width_at_height, wh)
    } else {
        (ww, (((ph as u64 * ww as u64 + pw as u64 / 2) / pw as u64) as u32).min(wh))
    };
    [(ww - w) / 2, (wh - h) / 2, w, h]
}

/// The draw of the back buffer into the rectangle of [`fit`]: a triangle over the target, a
/// viewport of the rectangle, a linear sample at each pixel, then the ramp of the device (the
/// gamma pass's table, or an identity table when the game set none).
const SHADER: &str = "
struct Quad { @builtin(position) position: vec4f, @location(0) at: vec2f }
@vertex fn vs(@builtin(vertex_index) index: u32) -> Quad {
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    return Quad(vec4f(x, y, 0.0, 1.0), vec2f((x + 1.0) * 0.5, (1.0 - y) * 0.5));
}
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var<uniform> ramp: array<vec4f, 192>;
@group(0) @binding(2) var linear: sampler;
fn entry(channel: u32, value: f32) -> f32 {
    let at = channel * 256u + u32(round(clamp(value, 0.0, 1.0) * 255.0));
    return ramp[at / 4u][at % 4u];
}
@fragment fn fs(in: Quad) -> @location(0) vec4f {
    let colour = textureSample(picture, linear, in.at);
    return vec4f(entry(0u, colour.r), entry(1u, colour.g), entry(2u, colour.b), colour.a);
}
";

/// The pipeline of the scaled draw, its sampler and its identity ramp.
pub(crate) struct Scale {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    identity: wgpu::Buffer,
}

impl Scale {
    pub(crate) fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Scale {
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scaled picture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: fragment,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: fragment,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: fragment, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("scaled picture"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("scaled picture"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scaled picture"),
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("scaled picture"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let floats: Vec<u8> = identity_ramp().iter().flat_map(|value| value.to_le_bytes()).collect();
        let identity = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("identity ramp"), contents: &floats, usage: wgpu::BufferUsages::UNIFORM });
        Scale { layout, pipeline, sampler, identity }
    }

    /// Draws `source` (the back buffer) into the rectangle `rect` of `target` (the picture of
    /// the window), black outside it, through `ramp` (the gamma pass's table) or the identity.
    pub(crate) fn draw(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, source: &wgpu::Texture, target: &wgpu::Texture, rect: [u32; 4], ramp: Option<&wgpu::Buffer>) {
        let source_view = source.create_view(&Default::default());
        let target_view = target.create_view(&Default::default());
        let group = gpu::frame_group(
            device,
            &wgpu::BindGroupDescriptor {
                label: Some("scaled picture"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&source_view) },
                    wgpu::BindGroupEntry { binding: 1, resource: ramp.unwrap_or(&self.identity).as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            },
        );
        crate::gpu::note_other_pass();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scaled picture"),
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
        let [x, y, w, h] = rect;
        // A rectangle with no pixels (an extreme aspect): the window stays black.
        if w == 0 || h == 0 {
            return;
        }
        pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The back buffer into the picture of the window at its own size, the part that the two have:
/// through the gamma ramp of the device, or a plain copy for an identity ramp. The window shots
/// take it as it is (their picture has the back buffer's size).
pub(crate) fn exact(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, gamma: Option<&gamma::Gamma>, back_buffer: &wgpu::Texture, window: &wgpu::Texture) {
    match gamma {
        Some(gamma) => gamma.draw(device, encoder, back_buffer, window),
        None => encoder.copy_texture_to_texture(
            back_buffer.as_image_copy(),
            window.as_image_copy(),
            wgpu::Extent3d { width: back_buffer.width().min(window.width()), height: back_buffer.height().min(window.height()), depth_or_array_layers: 1 },
        ),
    }
}

/// The back buffer into the window of the helper at each frame: [`exact`] when the window has
/// the picture's size, else the scaled draw of [`Scale`] into the rectangle of [`fit`]. The
/// log has one line when the rectangle changes.
#[derive(Default)]
pub(crate) struct ToWindow {
    scale: Option<Scale>,
    fitted: Option<[u32; 4]>,
}

impl ToWindow {
    pub(crate) fn draw(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, gamma: Option<&gamma::Gamma>, back_buffer: &wgpu::Texture, window: &wgpu::Texture) {
        let picture = (back_buffer.width(), back_buffer.height());
        let rect = fit(picture, (window.width(), window.height()));
        if self.fitted != Some(rect) {
            self.fitted = Some(rect);
            println!("window: the picture {} x {} drawn at {} x {} at {}, {} in {} x {}", picture.0, picture.1, rect[2], rect[3], rect[0], rect[1], window.width(), window.height());
        }
        // Only a window of the picture's size: a larger one (one more column) would keep
        // undefined pixels beside the copy, and the scaled draw clears them black.
        if (window.width(), window.height()) == picture {
            return exact(device, encoder, gamma, back_buffer, window);
        }
        let scale = self.scale.get_or_insert_with(|| Scale::new(device, window.format()));
        scale.draw(device, encoder, back_buffer, window, rect, gamma.map(gamma::Gamma::table));
    }
}

/// The ramp that changes no value: 256 steps from 0 to 1 for red, green and blue.
fn identity_ramp() -> Vec<f32> {
    (0..gamma::RAMP_VALUES).map(|at| (at % 256) as f32 / 255.0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scale_shader_is_valid_wgsl() {
        crate::assert_wgsl("scaled picture", SHADER);
    }

    #[test]
    fn a_window_of_the_pictures_size_takes_it_whole() {
        assert_eq!(fit((1920, 1080), (1920, 1080)), [0, 0, 1920, 1080]);
    }

    #[test]
    fn a_picture_of_the_windows_aspect_fills_it() {
        // 1920 x 1080 fullscreen on a 2560 x 1440 monitor: scaled up, no bars.
        assert_eq!(fit((1920, 1080), (2560, 1440)), [0, 0, 2560, 1440]);
        // A mode larger than the window (a game unaware of a 150 % scale): scaled down.
        assert_eq!(fit((3840, 2160), (2560, 1440)), [0, 0, 2560, 1440]);
    }

    #[test]
    fn another_aspect_gets_black_bars() {
        // A wider picture: bars at the top and the bottom.
        assert_eq!(fit((1920, 1080), (2560, 1600)), [0, 80, 2560, 1440]);
        // A narrower picture (5:4 on 16:9): bars at the left and the right.
        assert_eq!(fit((1280, 1024), (2560, 1440)), [380, 0, 1800, 1440]);
        // 4:3 on 16:10.
        assert_eq!(fit((1024, 768), (1680, 1050)), [140, 0, 1400, 1050]);
    }

    #[test]
    fn odd_sizes_are_centred_in_whole_pixels() {
        let [x, y, w, h] = fit((1366, 768), (1921, 1081));
        assert!(x + w <= 1921 && y + h <= 1081, "{x} {y} {w} {h}");
        assert_eq!((w, h), (1921, 1080));
        assert_eq!((x, y), (0, 0));
        assert_eq!(fit((800, 600), (1001, 600)), [100, 0, 800, 600]);
        // Rounded to the nearest, not truncated: 768 * 1280 / 1366 is 719.6.
        assert_eq!(fit((1366, 768), (1280, 1024)), [0, 152, 1280, 720]);
        // No pixels: the whole window.
        assert_eq!(fit((0, 0), (640, 480)), [0, 0, 640, 480]);
    }

    #[test]
    fn the_identity_ramp_changes_no_value() {
        let ramp = identity_ramp();
        assert_eq!(ramp.len(), gamma::RAMP_VALUES);
        assert_eq!((ramp[0], ramp[255], ramp[256], ramp[767]), (0.0, 1.0, 0.0, 1.0));
    }
}
