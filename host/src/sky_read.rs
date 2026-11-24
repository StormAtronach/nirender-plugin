//! The read of the sky's BILERP maps (W632; the design page
//! `Numidium Project/designs/w632-sharp-moon-2026-11-24.md`, verdict 787).
//!
//! The maps are those that `NiTexturingProperty::ctor_fromFile` (0x6E1900) makes, with BILERP fixed
//! at 0x6E19B3: the moon's face and its shadow shape ("Moon Shadow TextureProperty",
//! tx_mooncircle_full_*.tga) in `Moon::ctor` (0x44479f, 0x444829), and the two maps of the sun's
//! glare in `WeatherController::initGlare` (0x43dc27, 0x43dc9e). The sky's NIF maps (the stars, the
//! clouds) are TRILERP and keep their filter.
//!
//! The engine reads level 0 of such a map with a bilinear filter: sharp, but a large map drawn small
//! changes at each sub-pixel move. [`SkyMapFilter::Area`] takes the mean of a grid of reads across
//! the pixel from a level that is still sharper than the pixel (`sky_map_read` in `shader.wgsl`).

use crate::material::FILTER_BILERP;

/// How a draw of the sky reads its BILERP maps: the sampler of the stage ([`level_filter`]) and, for
/// stage 0, the read of `shader.wgsl` (`sky_map_read`, with [`crate::AREA_READ_BIT`] in the record).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum SkyMapFilter {
    /// The engine's read: level 0 with a bilinear filter (`graphics.sky_map_filter = "engine"`).
    #[default]
    Engine,
    /// A filter between the levels in each stage: the sky rule that the owner rejected. A replay arm
    /// only.
    Trilinear,
    /// Stage 0 filters between the levels with its level moved by this bias, a power of 2 (-1 reads
    /// one level sharper). A replay arm only.
    Bias(f32),
    /// Stage 0 takes the mean of an n x n grid of reads of a sharp level across the pixel
    /// (`graphics.sky_map_filter = "area"`).
    Area,
    /// The area read with a tent over two pixels. A replay arm only.
    AreaTent,
}

impl SkyMapFilter {
    /// The filter of a name of the replay flag `--sky-map-filter` (`bias` takes `bias`), or None.
    pub fn parse(name: &str, bias: f32) -> Option<SkyMapFilter> {
        match name {
            "engine" => Some(SkyMapFilter::Engine),
            "trilinear" => Some(SkyMapFilter::Trilinear),
            "bias" => Some(SkyMapFilter::Bias(bias)),
            "area" => Some(SkyMapFilter::Area),
            "area-tent" => Some(SkyMapFilter::AreaTent),
            _ => None,
        }
    }

    /// The name of the filter for the reports, with the bias of [`SkyMapFilter::Bias`].
    pub fn name(self) -> String {
        match self {
            SkyMapFilter::Engine => "engine".into(),
            SkyMapFilter::Trilinear => "trilinear".into(),
            SkyMapFilter::Bias(bias) => format!("bias {bias}"),
            SkyMapFilter::Area => "area".into(),
            SkyMapFilter::AreaTent => "area-tent".into(),
        }
    }

    /// The code of stage 0's read in its `alpha.w` (`sky_map_read` in `shader.wgsl`): 1 the bias, 2
    /// the area read, 3 the area read with the tent; 0 for a filter with no read of its own.
    pub(crate) fn code(self) -> u32 {
        match self {
            SkyMapFilter::Bias(_) => 1,
            SkyMapFilter::Area => 2,
            SkyMapFilter::AreaTent => 3,
            SkyMapFilter::Engine | SkyMapFilter::Trilinear => 0,
        }
    }

    /// The bias of [`SkyMapFilter::Bias`], in stage 0's `luminance.z`; 0 for the others.
    pub(crate) fn bias(self) -> f32 {
        match self {
            SkyMapFilter::Bias(bias) => bias,
            _ => 0.0,
        }
    }
}

/// The filter of the draws of this object class (MGE_LINK_CLASS_*): Some for a class of the sky,
/// from the dome (1) to the sun's glare (8), None for any other. The plugin gives the class from the
/// weather controller's nodes, not from a texture's name.
pub(crate) fn of_class(filter: SkyMapFilter, class: u32) -> Option<SkyMapFilter> {
    crate::engine::scene::sky_class(class).then_some(filter)
}

/// The read of the map of one stage of a draw: `sky` is the draw's ([`of_class`]), `filter` is the
/// map's FILTER_* and `stage` its stage. None when the map does not take the sky's filter (the draw
/// is not the sky's, or the map is not BILERP). A read of stage 0 only (the bias and the area reads)
/// gives another stage the engine's read.
pub(crate) fn of_stage(sky: Option<SkyMapFilter>, filter: u32, stage: usize) -> Option<SkyMapFilter> {
    let sky = sky.filter(|_| filter == FILTER_BILERP)?;
    Some(if stage > 0 && sky.code() != 0 {
        SkyMapFilter::Engine
    } else {
        sky
    })
}

/// D3DTEXF_NONE and D3DTEXF_LINEAR, the filter between levels (D3DTSS_MIPFILTER).
const MIP_FILTER_NONE: u32 = 0;
const MIP_FILTER_LINEAR: u32 = 2;

/// The filter between the levels of a stage's texture, in `sampler[4]`: `levels` is the number of
/// levels of the texture, and `sky` is the read of its map ([`of_stage`]).
///
/// - A BILERP map of a draw of the sky takes W632's `sky_map_filter` (the design's section 4). The
///   engine's read reads level 0 only, as the engine reads such a map. Every other filter, the area
///   read among them, gets a linear filter between the levels that the map has, so that a read can
///   get to the level that it names.
/// - Any other texture with one level gets no filter between levels. The engine's filter for a
///   texture with one level was not read (an inference).
/// - Any other texture with levels keeps the material's filter.
pub(crate) fn level_filter(sampler: &mut [u32; 5], levels: u32, sky: Option<SkyMapFilter>) {
    sampler[4] = match sky {
        Some(SkyMapFilter::Engine) => MIP_FILTER_NONE,
        _ if levels <= 1 => MIP_FILTER_NONE,
        Some(_) => MIP_FILTER_LINEAR,
        None => sampler[4],
    };
}

/// The grid of the area read for a footprint of `f` texels of level 0 on a map of `levels` levels:
/// the level `s` and the reads `n` on each axis, or None for f <= 1 (the stage's own read). `s` is
/// ceil(log2(f / 4)), from 0 to the last level, so that the footprint is at most 4 texels of level s
/// when the map has the levels; n is that footprint rounded up, from 2 to 8. `sky_map_read` in
/// `shader.wgsl` is the code; this is its arithmetic for the test of the design's table.
#[cfg(test)]
fn area_grid(f: f32, levels: u32) -> Option<(u32, u32)> {
    if f <= 1.0 {
        return None;
    }
    let s = (f / 4.0).log2().ceil().clamp(0.0, levels.saturating_sub(1) as f32);
    let n = ((f / s.exp2()).ceil() as u32).clamp(2, 8);
    Some((s as u32, n))
}

/// The text of `sky_map_read` in `shader.wgsl`, between its marks.
#[cfg(test)]
fn read_text() -> &'static str {
    let shader = include_str!("shader.wgsl");
    let begin = "// SKY MAP READ BEGIN";
    let end = "// SKY MAP READ END";
    assert_eq!(shader.matches(begin).count(), 1);
    assert_eq!(shader.matches(end).count(), 1);
    &shader[shader.find(begin).unwrap()..shader.find(end).unwrap()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::{Create, CreateView};

    /// The rows of the design's table (section 3): IT Masser, IT Secunda, Lunar Lorkhan's Secunda on a
    /// map of 12 levels and on its own map of one level; a footprint over 6.0 on Masser's map takes
    /// 4 x 4; a footprint of at most one texel keeps the stage's own read.
    #[test]
    fn the_grid_of_the_area_read_follows_the_designs_table() {
        assert_eq!(area_grid(6.0, 12), Some((1, 3)));
        assert_eq!(area_grid(6.1, 12), Some((1, 4)));
        assert_eq!(area_grid(6.8, 11), Some((1, 4)));
        assert_eq!(area_grid(13.7, 12), Some((2, 4)));
        assert_eq!(area_grid(13.7, 1), Some((0, 8)));
        assert_eq!(area_grid(1.0, 12), None);
        assert_eq!(area_grid(0.25, 12), None);
        // A footprint just over one texel reads level 0 with the least grid.
        assert_eq!(area_grid(1.5, 12), Some((0, 2)));
        // The footprint in texels of level s is at most 4 while the map has the levels.
        for tenth in 11..2000 {
            let f = tenth as f32 / 10.0;
            let (s, n) = area_grid(f, 12).unwrap();
            assert!(f / (s as f32).exp2() <= 4.0 && n <= 4, "f {f}: s {s}, n {n}");
        }
    }

    /// The design's section 4: a BILERP map of a draw of the sky takes the filter, and the bias and
    /// the area reads are for stage 0 only: a sky BILERP map in another stage keeps the engine's
    /// read. The null: a TRILERP map of the sky, and a map of a draw that is not the sky's, take none.
    #[test]
    fn the_area_read_is_for_stage_0_only() {
        let bilerp = FILTER_BILERP;
        for filter in [SkyMapFilter::Bias(-1.0), SkyMapFilter::Area, SkyMapFilter::AreaTent] {
            assert_eq!(of_stage(Some(filter), bilerp, 0), Some(filter));
            for stage in 1..6 {
                assert_eq!(of_stage(Some(filter), bilerp, stage), Some(SkyMapFilter::Engine));
            }
        }
        for filter in [SkyMapFilter::Engine, SkyMapFilter::Trilinear] {
            for stage in 0..6 {
                assert_eq!(of_stage(Some(filter), bilerp, stage), Some(filter));
            }
        }
        for filter in [0, 2, 3, 4, 5] {
            assert_eq!(of_stage(Some(SkyMapFilter::Area), filter, 0), None, "filter {filter}");
        }
        assert_eq!(of_stage(None, bilerp, 0), None);
    }

    /// The classes of the sky: the dome (1) to the sun's glare (8); not no class (0), the weather's
    /// particles (9) or the water plane (10).
    #[test]
    fn the_sky_filter_takes_the_classes_1_to_8_with_the_glare() {
        for class in 1..=7 {
            assert_eq!(of_class(SkyMapFilter::Area, class), Some(SkyMapFilter::Area), "class {class}");
        }
        assert!(of_class(SkyMapFilter::Area, crate::engine::scene::CLASS_SUN_GLARE).is_some());
        for class in [0, 9, 10, 11] {
            assert_eq!(of_class(SkyMapFilter::Area, class), None, "class {class}");
        }
    }

    /// Each name of the flag gives its filter, and the name of the report gives it back.
    #[test]
    fn the_names_of_the_filter_go_both_ways() {
        for name in ["engine", "trilinear", "area", "area-tent"] {
            assert_eq!(SkyMapFilter::parse(name, 0.0).unwrap().name(), name);
        }
        assert_eq!(SkyMapFilter::parse("bias", -0.5), Some(SkyMapFilter::Bias(-0.5)));
        assert_eq!(SkyMapFilter::Bias(-1.0).name(), "bias -1");
        assert_eq!(SkyMapFilter::parse("on", 0.0), None);
    }

    /// The design's section 4: on a sky BILERP map the engine's read gets no filter between levels,
    /// and every other filter gets a linear one when the map has levels; a map with one level gets
    /// none. The null: a map that does not take the sky's filter keeps the material's filter
    /// (BILERP's 0, TRILERP's 2) when it has levels, and gets none with one level.
    #[test]
    fn a_sky_map_takes_the_sky_map_filter() {
        let bilerp = crate::material::sampler_states(3, 1);
        let trilerp = crate::material::sampler_states(3, 2);
        let with = |mut sampler: [u32; 5], levels, sky| {
            level_filter(&mut sampler, levels, sky);
            sampler[4]
        };
        assert_eq!((bilerp[4], trilerp[4]), (0, 2));
        for levels in [12, 1] {
            assert_eq!(with(bilerp, levels, Some(SkyMapFilter::Engine)), 0, "{levels} levels");
        }
        for sky in [
            SkyMapFilter::Trilinear,
            SkyMapFilter::Bias(-1.0),
            SkyMapFilter::Area,
            SkyMapFilter::AreaTent,
        ] {
            assert_eq!(with(bilerp, 7, Some(sky)), 2, "{sky:?}");
            assert_eq!(with(bilerp, 1, Some(sky)), 0, "{sky:?}");
        }
        assert_eq!(with(bilerp, 12, None), 0);
        assert_eq!(with(trilerp, 12, None), 2);
        assert_eq!(with(trilerp, 1, None), 0);
    }

    /// The pixels of each side of the GPU test, and the texels a pixel.
    const SIDE: u32 = 96;
    const MAP: u32 = 2048;
    const TEXELS_A_PIXEL: f32 = 6.0;

    /// Seeded noise, 4 channels a texel.
    fn noise() -> Vec<u8> {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        (0..MAP * MAP * 4)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 24) as u8
            })
            .collect()
    }

    /// The levels of a map of 4 bytes a texel, each a 2 x 2 box of the level above it.
    fn box_chain(level0: Vec<u8>) -> Vec<Vec<u8>> {
        let mut levels = vec![level0];
        let mut side = MAP as usize;
        while side > 1 {
            let above = levels.last().unwrap();
            let half = side / 2;
            let mut below = vec![0u8; half * half * 4];
            for y in 0..half {
                for x in 0..half {
                    for c in 0..4 {
                        let at = |dx: usize, dy: usize| above[((2 * y + dy) * side + 2 * x + dx) * 4 + c] as u32;
                        below[(y * half + x) * 4 + c] = ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
                    }
                }
            }
            levels.push(below);
            side = half;
        }
        levels
    }

    /// The reference of a pixel: the mean of the texels of level 0 under its square, from 64 x 64
    /// point samples. `shift` moves the quad by a part of a pixel, as in the GPU's picture.
    fn reference(level0: &[u8], shift: f32) -> Vec<[f32; 4]> {
        let mut out = Vec::with_capacity((SIDE * SIDE) as usize);
        for py in 0..SIDE {
            for px in 0..SIDE {
                let mut sum = [0f64; 4];
                for j in 0..64 {
                    for i in 0..64 {
                        let x = ((px as f32 + shift + (i as f32 + 0.5) / 64.0) * TEXELS_A_PIXEL) as usize % MAP as usize;
                        let y = ((py as f32 + shift + (j as f32 + 0.5) / 64.0) * TEXELS_A_PIXEL) as usize % MAP as usize;
                        for c in 0..4 {
                            sum[c] += level0[(y * MAP as usize + x) * 4 + c] as f64;
                        }
                    }
                }
                out.push(sum.map(|v| (v / 4096.0) as f32));
            }
        }
        out
    }

    /// The shader of the test: a quad over the target whose coordinate moves 6 texels of level 0 a
    /// pixel, `shift` of a pixel moved. `arm` 0: the engine's read (level 0, bilinear); 1: the read
    /// between the levels; 2: `sky_map_read` of `shader.wgsl` (the area read), its text taken whole.
    fn test_shader() -> String {
        format!(
            "@group(0) @binding(0) var map: texture_2d<f32>;
@group(0) @binding(1) var level_zero: sampler;
@group(0) @binding(2) var between: sampler;
struct Test {{ shift: f32, arm: u32, scale: f32, unused: f32 }}
@group(0) @binding(3) var<uniform> test: Test;

fn first_map_size() -> vec2f {{ return vec2f(textureDimensions(map, 0)); }}
fn first_map_levels() -> u32 {{ return textureNumLevels(map); }}
fn first_map_level(uv: vec2f, level: f32) -> vec4f {{ return textureSampleLevel(map, between, uv, level); }}
fn first_map_grad(uv: vec2f, dx: vec2f, dy: vec2f) -> vec4f {{ return textureSampleGrad(map, between, uv, dx, dy); }}

{}
@vertex
fn vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {{
    let corner = vec2f(f32(index & 1u), f32(index >> 1u)) * 4.0 - 1.0;
    return vec4f(corner, 0.0, 1.0);
}}

@fragment
fn fs(@builtin(position) position: vec4f) -> @location(0) vec4f {{
    let uv = (position.xy + test.shift) * test.scale;
    let dx = dpdx(uv);
    let dy = dpdy(uv);
    let engine = textureSampleLevel(map, level_zero, uv, 0.0);
    let trilinear = textureSampleGrad(map, between, uv, dx, dy);
    if test.arm == 0u {{
        return engine;
    }}
    if test.arm == 1u {{
        return trilinear;
    }}
    return sky_map_read(uv, dx, dy, 2u, 0.0, engine);
}}
",
            read_text()
        )
    }

    /// Test 1 of the design (section 5): a 2048 map of seeded noise with a box chain, drawn at 6 texels
    /// a pixel in 4 positions 1/3 px apart, against the mean of the level-0 texels under each pixel.
    /// The area read's mean error is at most 4 of 255 in each position; the engine's read (too sharp)
    /// and the read between the levels (too soft) each fail the bound in the same run: the nulls.
    #[test]
    #[ignore = "needs a GPU"]
    fn the_area_read_gives_the_mean_of_the_texels_under_the_pixel() {
        const BOUND: f32 = 4.0;
        let instance = wgpu::Instance::default();
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("an adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("a device");
        let level0 = noise();
        let levels = box_chain(level0.clone());
        let map = device.counted_texture(&wgpu::TextureDescriptor {
            label: Some("noise"),
            size: wgpu::Extent3d {
                width: MAP,
                height: MAP,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, pixels) in levels.iter().enumerate() {
            let side = (MAP >> level).max(1);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &map,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(side * 4),
                    rows_per_image: Some(side),
                },
                wgpu::Extent3d {
                    width: side,
                    height: side,
                    depth_or_array_layers: 1,
                },
            );
        }
        let sampler = |mipmap_filter, lod_max_clamp| {
            device.counted_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter,
                lod_max_clamp,
                ..Default::default()
            })
        };
        let level_zero = sampler(wgpu::MipmapFilterMode::Nearest, 0.0);
        let between = sampler(wgpu::MipmapFilterMode::Linear, 32.0);
        let module = device.counted_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("area read test"),
            source: wgpu::ShaderSource::Wgsl(test_shader().into()),
        });
        let format = wgpu::TextureFormat::Rgba32Float;
        let pipeline = device.counted_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("area read test"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let target = device.counted_texture(&wgpu::TextureDescriptor {
            label: Some("area read target"),
            size: wgpu::Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let row = SIDE * 16;
        let readback = device.counted_buffer(&wgpu::BufferDescriptor {
            label: Some("area read readback"),
            size: (row * SIDE) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let uniform = device.counted_buffer(&wgpu::BufferDescriptor {
            label: Some("area read test values"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let map_view = map.counted_view(&Default::default());
        let target_view = target.counted_view(&Default::default());
        let group = device.counted_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&map_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&level_zero),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&between),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let draw = |shift: f32, arm: u32| -> Vec<[f32; 4]> {
            let values = [shift.to_bits(), arm, (TEXELS_A_PIXEL / MAP as f32).to_bits(), 0];
            queue.write_buffer(&uniform, 0, bytemuck::cast_slice(&values));
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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
                pass.draw(0..3, 0..1);
            }
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: Some(SIDE),
                    },
                },
                wgpu::Extent3d {
                    width: SIDE,
                    height: SIDE,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, |result| result.expect("the readback maps"));
            device.poll(wgpu::PollType::wait_indefinitely()).expect("the device waits");
            let pixels: Vec<[f32; 4]> =
                bytemuck::cast_slice(&readback.slice(..).get_mapped_range().expect("the readback")).to_vec();
            readback.unmap();
            pixels
        };
        let mut lines = Vec::new();
        let mut fails = Vec::new();
        for step in 0..4 {
            let shift = step as f32 / 3.0;
            let wanted = reference(&level0, shift);
            let mut errors = [0f32; 3];
            for (arm, error) in errors.iter_mut().enumerate() {
                let got = draw(shift, arm as u32);
                let sum: f64 = got
                    .iter()
                    .zip(&wanted)
                    .flat_map(|(g, w)| (0..4).map(move |c| (g[c] * 255.0 - w[c]).abs() as f64))
                    .sum();
                *error = (sum / (got.len() * 4) as f64) as f32;
            }
            lines.push(format!(
                "shift {shift:.3} px: mean error of 255: engine {:.2}, trilinear {:.2}, area {:.2}",
                errors[0], errors[1], errors[2]
            ));
            if errors[2] > BOUND {
                fails.push(format!("the area read fails the bound at shift {shift:.3}"));
            }
            if errors[0] <= BOUND {
                fails.push(format!(
                    "the engine's read passes the bound at shift {shift:.3}: the null cannot show"
                ));
            }
            if errors[1] <= BOUND {
                fails.push(format!(
                    "the trilinear read passes the bound at shift {shift:.3}: make the bound tighter"
                ));
            }
        }
        println!("{}", lines.join("\n"));
        assert!(fails.is_empty(), "{}", fails.join("\n"));
    }
}
