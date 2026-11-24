//! The methods of [`Renderer`]: the device, the pipelines, the textures and the samplers, the
//! buffers of the frame and of the retained instances, and the helpers of its buffers. The
//! struct is in lib.rs.

use super::*;
use crate::objects::{Create, CreateView};
use std::sync::atomic::{AtomicU64, Ordering};

/// The view of level 0 of a texture that takes a chain (W632, the Architect's W1): the engine's
/// look of a texture with one level, until the texture's chain is in its levels. The view has
/// one level, so a sampler with a mip filter samples level 0 only, as with the filter off.
fn level0_view(texture: &wgpu::Texture) -> Rc<wgpu::TextureView> {
    Rc::new(texture.counted_view(&wgpu::TextureViewDescriptor {
        mip_level_count: Some(1),
        ..Default::default()
    }))
}

/// The bind group of the frame: the resources of the extension, the clusters of the lights,
/// the lights and the bones of the frame, and the buffers of the retained instances.
#[track_caller]
pub(crate) fn frame_group(
    kept: &mut crate::gpu::KeptGroup,
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
    texture: &wgpu::TextureView,
    clusters: &Clusters,
    lights: &wgpu::Buffer,
    bones: &wgpu::Buffer,
    records: &wgpu::Buffer,
    light_indexes: &wgpu::Buffer,
    scenes: &wgpu::Buffer,
    retained_lights: &wgpu::Buffer,
    retained_bones: &wgpu::Buffer,
    influences: &wgpu::Buffer,
    effect_entries: &wgpu::Buffer,
) -> wgpu::BindGroup {
    kept.get(
        device,
        &wgpu::BindGroupDescriptor {
            label: Some("extension frame"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(texture),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: clusters.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: clusters.table.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: clusters.indexes.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: lights.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: bones.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: records.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: light_indexes.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: scenes,
                        offset: 0,
                        size: core::num::NonZeroU64::new(SCENE_SIZE as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: retained_lights.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: retained_bones.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 13,
                    resource: influences.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 14,
                    resource: effect_entries.as_entire_binding(),
                },
            ],
        },
    )
}

/// The buffer of the arguments of the indirect draws of a frame.
pub(crate) fn indirect_buffer(device: &wgpu::Device, draws: usize) -> wgpu::Buffer {
    device.counted_buffer(&wgpu::BufferDescriptor {
        label: Some("indirect draws"),
        size: (draws * ARGS_SIZE) as u64,
        usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Ends a run of draws: the arguments of the run go after those of the frame, and the pass
/// gets one indirect call for them. The arguments reach the GPU before the submit
/// (`upload_indirect`).
pub(crate) fn flush_run(
    pass: &mut wgpu::RenderPass<'static>,
    renderer: &mut Renderer,
    run_args: &mut Vec<IndirectArgs>,
    frame_args: &mut Vec<IndirectArgs>,
) {
    if run_args.is_empty() {
        return;
    }
    let offset = frame_args.len();
    frame_args.extend_from_slice(run_args);
    pass.multi_draw_indexed_indirect(&renderer.indirect_buffer, (offset * ARGS_SIZE) as u64, run_args.len() as u32);
    renderer.counts.indirect_calls += 1;
    renderer.counts.indirect_draws += run_args.len() as u32;
    run_args.clear();
}

/// `MGE_XE_DRAW_DUMP=<geometry handle, hex>`: the words of the draw data of each draw of
/// that geometry, on the path it takes, for a comparison of the two paths.
pub(crate) fn dump_wanted() -> u32 {
    switches().draw_dump
}

/// The lights of a draw of the dumped geometry, one line each: the position, the diffuse,
/// the attenuation and the range.
pub(crate) fn light_dump(path: &str, lights: &[LightData]) {
    for (i, l) in lights.iter().enumerate() {
        println!(
            "light dump {path} {i}: type {} position {:?} diffuse {:?} ambient {:?} attenuation {:?} range {} radius {}",
            l.position[3],
            &l.position[..3],
            &l.diffuse[..3],
            &l.ambient[..3],
            &l.attenuation[..3],
            l.direction[3],
            l.cone[2]
        );
    }
}

pub(crate) fn draw_dump(path: &str, geometry: u32, data: &DrawData) {
    let wanted = dump_wanted();
    if wanted == 0 || geometry != wanted {
        return;
    }
    let words: &[u32] = bytemuck::cast_slice(bytemuck::bytes_of(data));
    let text: Vec<String> = words.iter().map(|w| format!("{w:08x}")).collect();
    println!("draw dump {path} geometry {wanted:#x}: {}", text.join(" "));
}

/// The records of the draws of a frame.
pub(crate) fn record_buffer(device: &wgpu::Device, records: usize) -> wgpu::Buffer {
    device.counted_buffer(&wgpu::BufferDescriptor {
        label: Some("draw records"),
        size: (records * RECORD_SIZE) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The inputs a vertex does not have, zeros, one entry for each instance the instance buffer
/// can hold: a draw's instances index both buffers, so this one must be as long.
pub(crate) fn defaults_buffer(device: &wgpu::Device, instances: usize) -> wgpu::Buffer {
    device.counted_buffer(&wgpu::BufferDescriptor {
        label: Some("vertex defaults"),
        size: (instances * 64) as u64,
        usage: wgpu::BufferUsages::VERTEX,
        mapped_at_creation: false,
    })
}

/// The instances of a frame.
pub(crate) fn instance_buffer(device: &wgpu::Device, instances: usize) -> wgpu::Buffer {
    device.counted_buffer(&wgpu::BufferDescriptor {
        label: Some("instances"),
        size: (instances * INSTANCE_SIZE) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The render passes of a frame that get a pair of time stamps, and the pair of the frame
/// after them.
pub(crate) const TIMING_PASSES: u32 = 126;

/// The named pairs of a frame (`stamps::Stamps`), after the pair of the frame.
pub(crate) const TIMING_NAMED: u32 = 96;

pub(crate) const TIMING_QUERIES: u32 = 2 * (TIMING_PASSES + 1 + TIMING_NAMED);

/// The first query of the named pairs.
const NAMED_FIRST: u32 = 2 * (TIMING_PASSES + 1);

/// GPU time stamps: a pair for each render pass of the frame, and a pair for the frame
/// (when the adapter writes them outside a pass). They are resolved at the end of the
/// frame into one of three staging buffers, and read when the buffer is mapped, frames
/// later, so the program never waits for the GPU.
pub(crate) struct Timing {
    pub(crate) set: wgpu::QuerySet,
    pub(crate) resolve: wgpu::Buffer,
    pub(crate) staging: Vec<wgpu::Buffer>,
    /// The staging buffers that are mapped or being mapped, and the passes of the frame
    /// of each: a pair that no pass of the frame wrote keeps the stamps of an older frame.
    pub(crate) busy: Vec<bool>,
    pub(crate) slot_passes: Vec<u32>,
    pub(crate) ready: std::sync::mpsc::Receiver<usize>,
    pub(crate) sender: std::sync::mpsc::Sender<usize>,
    /// The passes of the frame with a pair.
    pub(crate) passes: u32,
    /// Nanoseconds for each stamp step.
    pub(crate) period: f32,
    /// The adapter writes stamps outside a pass: the frame has its pair.
    pub(crate) frame_stamps: bool,
    /// The named stamps of the frame in progress (shared with the extension), and the names
    /// of each staging buffer's frame.
    pub(crate) stamps: Rc<crate::stamps::Stamps>,
    pub(crate) slot_names: Vec<Vec<(&'static str, crate::stamps::StampKind)>>,
}

/// Only with `MGE_XE_GPU_TIME=1`: the stamps cost time at the end of each frame.
pub(crate) fn make_timing(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Timing> {
    if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) || !switches().gpu_time {
        return None;
    }
    let size = TIMING_QUERIES as u64 * 8;
    let buffer = |usage| {
        device.counted_buffer(&wgpu::BufferDescriptor {
            label: Some("timing"),
            size,
            usage,
            mapped_at_creation: false,
        })
    };
    let (sender, ready) = std::sync::mpsc::channel();
    let set = device.counted_query_set(&wgpu::QuerySetDescriptor {
        label: Some("timing"),
        ty: wgpu::QueryType::Timestamp,
        count: TIMING_QUERIES,
    });
    let in_encoders = device.features().contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);
    let in_passes = device.features().contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES);
    let stamps = Rc::new(crate::stamps::Stamps::new(
        set.clone(),
        NAMED_FIRST,
        TIMING_NAMED,
        in_encoders,
        in_passes,
    ));
    Some(Timing {
        set,
        resolve: buffer(wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST),
        staging: (0..3)
            .map(|_| buffer(wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST))
            .collect(),
        busy: vec![false; 3],
        slot_passes: vec![0; 3],
        ready,
        sender,
        passes: 0,
        period: queue.get_timestamp_period(),
        frame_stamps: device.features().contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS),
        stamps,
        slot_names: vec![Vec::new(); 3],
    })
}

impl Timing {
    /// The pair of the next render pass of the frame, or [`None`] when the frame has more
    /// passes than pairs.
    pub(crate) fn next_pass(&mut self) -> Option<u32> {
        let index = self.passes;
        if index >= TIMING_PASSES {
            return None;
        }
        self.passes += 1;
        Some(index)
    }

    /// The start of a frame: the stamp of the frame, when the adapter can write it.
    pub(crate) fn begin(&mut self, encoder: &mut wgpu::CommandEncoder) {
        self.passes = 0;
        self.stamps.clear();
        if self.frame_stamps {
            encoder.write_timestamp(&self.set, 2 * TIMING_PASSES);
        }
    }

    /// The end of a frame, before the submit: reads the staging buffers that are mapped,
    /// then resolves the stamps of this frame into a free one. Returns the buffer to map
    /// after the submit.
    pub(crate) fn end(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        counts: &mut Counts,
    ) -> Option<usize> {
        let _ = device.poll(wgpu::PollType::Poll);
        while let Ok(slot) = self.ready.try_recv() {
            {
                let mapped = self.staging[slot].slice(..).get_mapped_range().expect("the time stamps");
                let stamp = |i: u32| u64::from_le_bytes(mapped[i as usize * 8..i as usize * 8 + 8].try_into().unwrap());
                let span = |i: u32| -> f64 {
                    let (begin, end) = (stamp(2 * i), stamp(2 * i + 1));
                    if begin != 0 && end > begin {
                        (end - begin) as f64 * self.period as f64
                    } else {
                        0.0
                    }
                };
                counts.gpu_passes += (0..self.slot_passes[slot]).map(span).sum::<f64>();
                counts.gpu_frame += span(TIMING_PASSES);
                counts.gpu_frames += 1;
                for (i, &(name, kind)) in self.slot_names[slot].iter().enumerate() {
                    let time = span(NAMED_FIRST / 2 + i as u32);
                    match counts.gpu_named.iter_mut().find(|(n, k, _, _)| *n == name && *k == kind) {
                        Some(entry) => {
                            entry.2 += time;
                            entry.3 += 1;
                        }
                        None => counts.gpu_named.push((name, kind, time, 1)),
                    }
                }
            }
            self.staging[slot].unmap();
            self.busy[slot] = false;
        }
        if self.frame_stamps {
            encoder.write_timestamp(&self.set, 2 * TIMING_PASSES + 1);
        }
        let slot = self.busy.iter().position(|busy| !busy)?;
        encoder.clear_buffer(&self.resolve, 0, None);
        encoder.resolve_query_set(&self.set, 0..TIMING_QUERIES, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.staging[slot], 0, TIMING_QUERIES as u64 * 8);
        self.busy[slot] = true;
        self.slot_passes[slot] = self.passes;
        self.slot_names[slot] = self.stamps.take_names();
        Some(slot)
    }

    /// After the submit: asks for the map of the buffer of the frame.
    pub(crate) fn map(&self, slot: usize) {
        let sender = self.sender.clone();
        self.staging[slot].slice(..).map_async(wgpu::MapMode::Read, move |result| {
            if result.is_ok() {
                let _ = sender.send(slot);
            }
        });
    }
}

/// The bones of the skinned draws of the frame, matrices.
pub(crate) fn bone_buffer(device: &wgpu::Device, bones: usize) -> wgpu::Buffer {
    device.counted_buffer(&wgpu::BufferDescriptor {
        label: Some("bones"),
        size: (bones * 64) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The window that the frames go into.
pub(crate) enum WindowTarget {
    /// A window of this program: a child of the game's window, or a window of its own.
    Own(std::sync::Arc<winit::window::Window>),
    /// The game's window itself (`MGE_XE_WINDOW=game`).
    Game(std::num::NonZeroIsize),
}

impl Renderer {
    /// True when a draw of this object class (MGE_LINK_CLASS_*) filters between the levels of its
    /// textures ([`crate::mip_chain::sky_levels`]).
    pub(crate) fn sky_levels(&self, class: u32) -> bool {
        crate::mip_chain::sky_levels(self.sky_mip_filter, class)
    }

    /// The maps of a material that have a texture, for a shape that the extension holds
    /// ([`HeldDraw::maps`]): `sets` is the number of texture coordinate sets of its geometry.
    pub(crate) fn held_maps(&self, m: &material::Material, sets: u32) -> Vec<HeldMap> {
        m.maps
            .iter()
            .enumerate()
            .filter(|(_, map)| map.present && map.texture != 0)
            .filter_map(|(index, map)| {
                let texture = self.textures.get(&map.texture)?.clone();
                let mut sampler = material::sampler_states(map.clamp, map.filter);
                crate::mip_chain::level_filter(
                    &mut sampler,
                    texture.texture().mip_level_count(),
                    self.chains.contains_key(&map.texture),
                    // The extension holds no draw of the sky (`hold_shape`).
                    false,
                );
                Some(HeldMap {
                    map: index,
                    texture,
                    set: map.set.min(sets.saturating_sub(1)),
                    sampler,
                })
            })
            .collect()
    }

    /// Makes the device through the one request of the crate (`request`). With a window,
    /// also the surface of the window, and the format of the target is a format of that
    /// surface. When no backend serves, the refusals come back for the caller to show and
    /// to end the program with `EXIT_GPU`.
    pub(crate) fn new(
        backends: wgpu::Backends,
        window: Option<WindowTarget>,
        target_format: wgpu::TextureFormat,
        extension_shader: String,
        texture_slots: u32,
        samples: u32,
    ) -> Result<(Renderer, Option<wgpu::Surface<'static>>), Vec<request::Refusal>> {
        let request = request::request(backends, window, target_format, texture_slots, samples)?;
        request.report();
        let request::Request {
            info,
            surface,
            device,
            queue,
            target_format,
            present_modes,
            depth_format,
            samples,
            array_cap,
            sampler_cap,
            tier,
            ..
        } = request;
        gpu::watch(&device, &info);

        let multisample =
            (samples > 1).then(|| multisample::Multisample::new(&device, target_format, depth_format, samples));
        let wireframe = device.features().contains(wgpu::Features::POLYGON_MODE_LINE);
        let source = world_source_of(&extension_shader, tier);
        let (shader, world_fault) = world_module(&device, &source);
        let texture_entry = |binding: u32, count| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count,
        };
        let sampler_entry = |binding: u32, count| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count,
        };
        // Group 1: the binding arrays of tier A, or the 6 textures and 6 samplers of tier C.
        let array_entries: Vec<wgpu::BindGroupLayoutEntry> = match tier {
            request::TextureTier::A => vec![
                texture_entry(0, std::num::NonZeroU32::new(array_cap)),
                sampler_entry(1, std::num::NonZeroU32::new(sampler_cap)),
            ],
            request::TextureTier::C => (0..STAGES as u32)
                .map(|stage| texture_entry(stage, None))
                .chain((0..STAGES as u32).map(|stage| sampler_entry(STAGES as u32 + stage, None)))
                .collect(),
        };
        let array_layout = device.counted_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(if tier == request::TextureTier::A {
                "draw texture arrays"
            } else {
                "draw textures of the stages"
            }),
            entries: &array_entries,
        });
        let frame_layout = device.counted_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("extension frame"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // The clusters: only a pixel reads them. The vertex stage has room for 8
                // storage buffers, the default limit of a stage (binding 13).
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // The lights and the bones of the frame.
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // The records of the draws of the frame.
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // The retained instances: the light indexes, the scene of the view (a
                // dynamic offset selects it), the lights.
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: core::num::NonZeroU64::new(SCENE_SIZE as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // The bones of the skinned retained instances.
                wgpu::BindGroupLayoutEntry {
                    binding: 12,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // The bone influences of the shapes that the game skins in software. The vertex
                // stage reads 7 storage buffers with it.
                wgpu::BindGroupLayoutEntry {
                    binding: 13,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // The texture matrices of the table's texture effects. The vertex stage reads 8
                // storage buffers with it, the limit of `request::fixed_limits`.
                wgpu::BindGroupLayoutEntry {
                    binding: 14,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.counted_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("draw"),
            // From the group that changes least to the one that changes most: the frame, the
            // arrays of the frame. The record of a draw is in the frame group, named by the
            // instance of the draw.
            bind_group_layouts: &[Some(&frame_layout), Some(&array_layout)],
            immediate_size: 0,
        });
        let white = Rc::new(
            device
                .counted_texture_with_data(
                    &queue,
                    &wgpu::TextureDescriptor {
                        label: Some("white"),
                        size: wgpu::Extent3d {
                            width: 1,
                            height: 1,
                            depth_or_array_layers: 1,
                        },
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
                .counted_view(&Default::default()),
        );
        let defaults = defaults_buffer(&device, RETAINED_AT_FIRST + INSTANCES_AT_FIRST);
        let frame_uniform = device.counted_buffer(&wgpu::BufferDescriptor {
            label: Some("extension frame"),
            size: 512,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_sampler = device.counted_sampler(&wgpu::SamplerDescriptor {
            label: Some("extension frame"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let clusters = Clusters::new(&device);
        let extension_lights = ExtensionLights::new(&device);
        let (first_lights, first_bones) = (light_buffer(&device, LIGHTS_AT_FIRST), bone_buffer(&device, BONES_AT_FIRST));
        let first_records = record_buffer(&device, RETAINED_AT_FIRST + RECORDS_AT_FIRST);
        let record_limit = ((device.limits().max_storage_buffer_binding_size as u64).min(device.limits().max_buffer_size)
            as usize
            / RECORD_SIZE)
            .min(switches::switches().record_limit.unwrap_or(usize::MAX));
        println!(
            "draw records: room for {record_limit} in one binding ({} MB)",
            record_limit * RECORD_SIZE / (1 << 20)
        );
        let first_light_indexes = light_index_buffer(&device, LIGHT_INDEXES_AT_FIRST);
        let first_scenes = scene_buffer(&device);
        let first_retained_lights = retained_light_buffer(&device, RETAINED_LIGHTS_AT_FIRST);
        let first_indirect = indirect_buffer(&device, RECORDS_AT_FIRST);
        let first_retained_bones = bone_buffer(&device, RETAINED_BONES_AT_FIRST);
        let first_effect_entries = effect_entry_buffer(&device, EFFECT_ENTRIES_AT_FIRST);
        let influences = crate::influences::Influences::new(&device);
        let mut frame_kept = crate::gpu::KeptGroup::default();
        let frame_group = Rc::new(frame_group(
            &mut frame_kept,
            &device,
            &frame_layout,
            &frame_uniform,
            &frame_sampler,
            &white,
            &clusters,
            &first_lights,
            &first_bones,
            &first_records,
            &first_light_indexes,
            &first_scenes,
            &first_retained_lights,
            &first_retained_bones,
            &influences.buffer,
            &first_effect_entries,
        ));
        let timing = make_timing(&device, &queue);
        let stamps = timing
            .as_ref()
            .map_or_else(|| Rc::new(crate::stamps::Stamps::off()), |timing| timing.stamps.clone());
        let maker = PipelineMaker {
            device: device.clone(),
            layout: pipeline_layout.clone(),
            vertex: shader.clone(),
            target_format,
            depth_format,
            wireframe,
            planted: source.contains(crate::CORE_MOD_FAULT_PLANT),
            fault_key: crate::world_key_fault().then(|| crate::stage_shaders::first_world_key(samples)),
            times: Default::default(),
        };
        let stage_shaders = crate::stage_shaders::StageShaders::new(switches().stage_shaders, &source);
        let renderer = Renderer {
            maker,
            stage_shaders,
            world: WorldShader {
                fault: world_fault,
                ..WorldShader::default()
            },
            frame_layout,
            frame_uniform,
            clusters,
            extension_lights,
            frame_sampler,
            frame_group,
            frame_kept,
            light_buffer: first_lights,
            instance_buffer: instance_buffer(&device, RETAINED_AT_FIRST + INSTANCES_AT_FIRST),
            instance_capacity: RETAINED_AT_FIRST + INSTANCES_AT_FIRST,
            lights_known: HashMap::new(),
            timing,
            stamps: stamps.clone(),
            bone_buffer: first_bones,
            bone_capacity: BONES_AT_FIRST,
            retained_bone_buffer: first_retained_bones,
            effect_entry_buffer: first_effect_entries,
            effect_entry_capacity: EFFECT_ENTRIES_AT_FIRST,
            retained_bone_capacity: RETAINED_BONES_AT_FIRST,
            device: Rc::new(device),
            target_format,
            present_modes,
            anisotropy: 1,
            mipmaps_for_one_level: true,
            sky_mip_filter: false,
            samples,
            depth_format,
            multisample,
            depth_merge: None,
            line_pass: None,
            line_list: debug_lines::LineList::default(),
            light_lines: [0; 2],
            bound_lines: [0; 2],
            queue,
            shader,
            runs_dump: switches().runs_dump,
            indirect: switches().indirect,
            indirect_buffer: Rc::new(first_indirect),
            indirect_capacity: RECORDS_AT_FIRST,
            indirect_written: 0,
            record_buffer: Rc::new(first_records),
            record_capacity: RETAINED_AT_FIRST + RECORDS_AT_FIRST,
            record_limit,
            record_staging: Vec::new(),
            record_count: 0,
            record_written: 0,
            record_extras: 0,
            instance_len: 0,
            instance_extras: 0,
            light_capacity: LIGHTS_AT_FIRST,
            light_index_buffer: first_light_indexes,
            light_index_capacity: LIGHT_INDEXES_AT_FIRST,
            retained_light_buffer: first_retained_lights,
            retained_light_capacity: RETAINED_LIGHTS_AT_FIRST,
            scene_buffer: first_scenes,
            scene_main: SceneData::zeroed(),
            scene_slots_used: 1,
            retained_records: RETAINED_AT_FIRST,
            retained_instances: RETAINED_AT_FIRST,
            retained_instance_copy: Vec::new(),
            record_generation: 0,
            instance_generation: 0,
            force_record_growth: false,
            force_segment_growth: false,
            pinned_textures: Vec::new(),
            pinned_free: Vec::new(),
            pinned_index: FxHashMap::default(),
            pinned_in_frame: 0,
            pinned_samplers: Vec::new(),
            pinned_samplers_in_frame: 0,
            pipelines: FxHashMap::default(),
            debug_variant: 0,
            variant_shader: None,
            textures: FxHashMap::default(),
            texture_objects: FxHashMap::default(),
            decoded: Default::default(),
            chains: Default::default(),
            chain_worker: None,
            chain_generation: 0,
            wait_for_chains: false,
            targets: FxHashMap::default(),
            surfaces: HashMap::new(),
            white,
            samplers: FxHashMap::default(),
            array_layout,
            array_cap: array_cap as usize,
            sampler_cap: sampler_cap as usize,
            frame_texture_slots: FxHashMap::default(),
            frame_texture_views: Vec::new(),
            frame_sampler_slots: FxHashMap::default(),
            frame_samplers: Vec::new(),
            array_group: None,
            array_variants: FxHashMap::default(),
            small_array_group: None,
            array_groups_dropped: false,
            small_slots_dropped: None,
            tier,
            stage_groups: StageGroups::default(),
            geometries: FxHashMap::default(),
            gone_textures: Vec::new(),
            // Storage too: the morph pass writes the positions of a morphed geometry in place.
            vertex_pool: pool::Pool::new(
                "game vertices",
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE,
                VERTEX_CHUNK,
            ),
            index_pool: pool::Pool::new("game indexes", wgpu::BufferUsages::INDEX, INDEX_CHUNK),
            influences,
            retired: Vec::new(),
            retired_indexes: Vec::new(),
            retired_influences: Vec::new(),
            geometry_frame: 1,
            quad_indexes: None,
            particle_quads: None,
            morphs: None,
            materials: FxHashMap::default(),
            derived: FxHashMap::default(),
            defaults,
            screen_pipelines: HashMap::new(),
            screen_layout: None,
            screen_sampler: None,
            counts: Counts::default(),
            commands: Rc::new(FrameCommands::with_stamps(stamps.clone())),
        };
        Ok((renderer, surface))
    }

    /// The arrays of the textures and the samplers at the start of a frame. They stay from
    /// frame to frame with their group, so a frame with no new texture or sampler makes no
    /// group. They are laid out again (slot 0 white and a plain sampler, then the pinned
    /// slots, `pin_into_frame`) when the pinned part grew past its place, when the frame's own
    /// part grew past twice [`FRAME_RESERVE`], or when the room runs short. True when they
    /// were laid out again.
    pub(crate) fn begin_frame_arrays(&mut self) -> bool {
        // Tier C has no arrays: the groups of the sets stay with their textures.
        if self.tier == request::TextureTier::C {
            return false;
        }
        let laid_out = !self.frame_texture_views.is_empty();
        let pinned_grew =
            self.pinned_textures.len() > self.pinned_in_frame || self.pinned_samplers.len() > self.pinned_samplers_in_frame;
        let own = self.frame_texture_views.len().saturating_sub(self.pinned_in_frame + 1);
        let room_short = self.frame_texture_views.len() + FRAME_RESERVE > self.array_cap
            || self.frame_samplers.len() + 8 > self.sampler_cap;
        if laid_out && !pinned_grew && own <= 2 * FRAME_RESERVE && !room_short {
            self.counts.arrays_kept += 1;
            return false;
        }
        self.counts.arrays_laid_out += 1;
        self.frame_texture_slots.clear();
        self.frame_texture_views.clear();
        self.frame_sampler_slots.clear();
        self.frame_samplers.clear();
        self.forget_array_groups(array_change::LAID_OUT);
        self.frame_texture_views.push(self.white.clone());
        let plain = self.sampler(material::sampler_states(0, 0));
        self.frame_samplers.push(plain);
        true
    }

    /// The texture of a handle. A handle is the address of an object of the game's side:
    /// a released handle comes back for a later texture. A pinned slot of the arrays, and a
    /// slot of the frame, that holds the handle takes the new view.
    pub(crate) fn set_texture(&mut self, handle: u32, view: Rc<wgpu::TextureView>) {
        // Tier C: the groups that held the handle (another view, or the white of a texture
        // not known yet) are made again at their next use.
        if self.tier == request::TextureTier::C && !self.textures.get(&handle).is_some_and(|old| Rc::ptr_eq(old, &view)) {
            self.stage_groups.forget(handle);
        }
        let mut changed = false;
        if let Some(pinned) = self.pinned_textures.iter_mut().find(|p| p.handle == handle) {
            if !Rc::ptr_eq(&pinned.view, &view) {
                // A slot in use that held a texture takes another one under the same handle.
                // A handle is the serial of one object of the plugin's device, so this is the
                // same object with new content (a map tile). Other facts (size, levels,
                // format) mark a handle that named two objects: the instances of the slot
                // draw the wrong texture (the fault of 2026-10-27, addresses as handles).
                if pinned.uses != 0 && pinned.view.texture() == view.texture() {
                    // W632: another view of the same texture (its level 0, or all its levels
                    // once its chain is in them): the same object, not a handle that named two.
                    self.counts.pinned_new_view += 1;
                } else if pinned.uses != 0 && !Rc::ptr_eq(&pinned.view, &self.white) {
                    let (old, new) = (pinned.view.texture(), view.texture());
                    let facts = |t: &wgpu::Texture| (t.width(), t.height(), t.mip_level_count(), t.format());
                    let same = facts(old) == facts(new);
                    self.counts.pinned_replaced[same as usize] += 1;
                    if self.counts.pinned_replaced[0] + self.counts.pinned_replaced[1] <= 20 {
                        println!(
                            "texture slots: frame {}: the pinned slot of {handle:#x} ({} uses) takes another texture: {:?} before, {:?} now ({})",
                            self.counts.frames,
                            pinned.uses,
                            facts(old),
                            facts(new),
                            if same { "same facts" } else { "other facts" }
                        );
                    }
                }
                pinned.view = view.clone();
                changed = true;
            }
        }
        if let Some(&slot) = self.frame_texture_slots.get(&handle) {
            if !Rc::ptr_eq(&self.frame_texture_views[slot as usize], &view) {
                self.frame_texture_views[slot as usize] = view.clone();
                changed = true;
            }
        }
        if changed {
            self.forget_array_groups(array_change::TEXTURE_REPLACED);
            self.counts.textures_replaced += 1;
        }
        self.textures.insert(handle, view);
    }

    /// The slot of a texture of the game in the arrays of the frame: a new one for a handle
    /// the frame did not have. 0 (white) for no texture, an unknown handle, or a frame with
    /// more textures than the arrays hold.
    pub(crate) fn frame_texture_slot(&mut self, handle: u32) -> u32 {
        // Tier C: no slots; the group of the draw's set holds its textures.
        if handle == 0 || self.tier == request::TextureTier::C {
            return 0;
        }
        if let Some(&slot) = self.frame_texture_slots.get(&handle) {
            return slot;
        }
        let Some(view) = self.textures.get(&handle) else { return 0 };
        if self.frame_texture_views.len() >= self.array_cap {
            if self.counts.over_array_cap == self.counts.pinned_over {
                println!("texture slots: the room of {} is full in a frame; textures past it draw white: raise [runtime] renderer_texture_slots of mgeXE.toml", self.array_cap);
            }
            self.counts.over_array_cap += 1;
            return 0;
        }
        let slot = self.frame_texture_views.len() as u32;
        self.frame_texture_views.push(view.clone());
        self.frame_texture_slots.insert(handle, slot);
        self.counts.most_frame_textures = self.counts.most_frame_textures.max(slot + 1);
        // The groups made so far lack this texture.
        self.forget_array_groups(array_change::TEXTURE_IN);
        slot
    }

    /// The slot of a sampler state in the arrays of the frame.
    pub(crate) fn frame_sampler_slot(&mut self, values: [u32; 5]) -> u32 {
        if self.tier == request::TextureTier::C {
            return 0;
        }
        if let Some(&slot) = self.frame_sampler_slots.get(&values) {
            return slot;
        }
        if self.frame_samplers.len() >= self.sampler_cap {
            return 0;
        }
        let sampler = self.sampler(values);
        let slot = self.frame_samplers.len() as u32;
        self.frame_samplers.push(sampler);
        self.frame_sampler_slots.insert(values, slot);
        self.forget_array_groups(array_change::SAMPLER_IN);
        slot
    }

    /// Drops the groups of the arrays (the group of the frame, its variants for the targets,
    /// and the small group): a view or a sampler of the arrays changed, or a slot came. The
    /// next use makes them again. The change of the arrays, `change` (one of [`array_change`]),
    /// marks the frame as changed in the count of the GPU objects ([`crate::objects::changed`]):
    /// the groups made again are its work, with or without a record of the game.
    pub(crate) fn forget_array_groups(&mut self, change: &'static str) {
        crate::objects::changed(change);
        self.array_group = None;
        self.array_variants.clear();
        if let Some((slots, _)) = self.small_array_group.take() {
            self.small_slots_dropped = Some(slots);
        }
        self.array_groups_dropped = true;
    }

    /// At the end of a frame whose change dropped the groups of the arrays: the group of the
    /// frame, and the small group for the slots that the dropped one had, are made again now.
    /// They bind the arrays that the next draws bind, so the group counts in the frame of the
    /// change, not in the first steady frame that draws with the arrays ([`crate::objects`]).
    /// The variants for the rendered targets wait for their first use.
    pub(crate) fn make_dropped_array_groups(&mut self) {
        if !std::mem::take(&mut self.array_groups_dropped) || self.tier == request::TextureTier::C {
            return;
        }
        if self.array_group.is_none() && !self.frame_texture_views.is_empty() {
            self.array_group_for(None);
        }
        if self.small_array_group.is_none() {
            if let Some(slots) = self.small_slots_dropped.take() {
                self.small_array_group(&slots);
            }
        }
    }

    /// The group of the arrays of the frame for a pass into `target` (a rendered texture of
    /// the game, or [`None`] for the back buffer or a target of the extension): the texture of
    /// the target shows white in it. Made once, kept for the frame.
    pub(crate) fn array_group_for(&mut self, target: Option<u32>) -> Rc<wgpu::BindGroup> {
        let hide = target.and_then(|handle| self.frame_texture_slots.get(&handle).copied());
        match hide {
            Some(slot) => {
                if let Some(group) = self.array_variants.get(&slot) {
                    return group.clone();
                }
            }
            None => {
                if let Some(group) = self.array_group.as_ref() {
                    return group.clone();
                }
            }
        }
        let mut views: Vec<&wgpu::TextureView> = self.frame_texture_views.iter().map(|view| &**view).collect();
        if let Some(slot) = hide {
            views[slot as usize] = &self.white;
        }
        let samplers: Vec<&wgpu::Sampler> = self.frame_samplers.iter().map(|sampler| &**sampler).collect();
        self.counts.array_groups_made += 1;
        let group = Rc::new(crate::gpu::frame_group(
            &self.device,
            &wgpu::BindGroupDescriptor {
                label: Some("draw texture arrays"),
                layout: &self.array_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureViewArray(&views),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::SamplerArray(&samplers),
                    },
                ],
            },
        ));
        match hide {
            Some(slot) => {
                self.array_variants.insert(slot, group.clone());
            }
            None => self.array_group = Some(group.clone()),
        }
        group
    }

    /// Tier C: the group of a set of the stages' textures and samplers, made at its first use
    /// and kept until one of its textures takes another view or goes ([`StageGroups::forget`]).
    /// A handle without a texture, and the target of the pass (`hide`), show white; a stage
    /// without a texture has the handle 0 and the sampler state 0, as in tier A.
    pub(crate) fn stage_group(
        &mut self,
        textures: [u32; STAGES],
        samplers: [[u32; 5]; STAGES],
        hide: Option<u32>,
    ) -> Rc<wgpu::BindGroup> {
        // A target that is not one of the set's textures changes nothing: one group for both.
        let hide = hide.filter(|handle| textures.contains(handle)).unwrap_or(0);
        let key = StageKey {
            textures,
            samplers,
            hide,
        };
        if let Some(group) = self.stage_groups.get(&key) {
            return group;
        }
        let views: Vec<Rc<wgpu::TextureView>> = textures
            .iter()
            .map(|&handle| {
                if handle == 0 || handle == hide {
                    self.white.clone()
                } else {
                    self.textures.get(&handle).cloned().unwrap_or_else(|| self.white.clone())
                }
            })
            .collect();
        let sampler_objects: Vec<Rc<wgpu::Sampler>> = samplers.iter().map(|&values| self.sampler(values)).collect();
        let mut entries: Vec<wgpu::BindGroupEntry> = views
            .iter()
            .enumerate()
            .map(|(stage, view)| wgpu::BindGroupEntry {
                binding: stage as u32,
                resource: wgpu::BindingResource::TextureView(view),
            })
            .collect();
        entries.extend(
            sampler_objects
                .iter()
                .enumerate()
                .map(|(stage, sampler)| wgpu::BindGroupEntry {
                    binding: (STAGES + stage) as u32,
                    resource: wgpu::BindingResource::Sampler(sampler),
                }),
        );
        self.counts.array_groups_made += 1;
        let group = Rc::new(crate::gpu::frame_group(
            &self.device,
            &wgpu::BindGroupDescriptor {
                label: Some("draw textures of the stages"),
                layout: &self.array_layout,
                entries: &entries,
            },
        ));
        self.stage_groups.insert(key, group.clone());
        group
    }

    /// A group of the arrays with only the textures of these slots of the frame, for draws
    /// into the back buffer, and the slot of each texture in it. The slot 0 of the frame stays
    /// 0: every draw samples all its stages, and an unused stage has the slot 0. The samplers
    /// are those of the frame. A bind of a group costs wgpu work for each of its textures.
    ///
    /// The group stays for the next frames that ask for the same slots, until a change of the
    /// arrays drops it with the group of the frame ([`Renderer::forget_array_groups`]): a
    /// steady frame makes no group. A group holds a place for each element of the layout's
    /// arrays in the backend (Direct3D 12 fills the rest of an array with null views), and that
    /// place comes back only when the GPU has run the frames that used it.
    pub(crate) fn small_array_group(&mut self, slots: &[u32]) -> crate::SmallArrays {
        let mut slots: Vec<u32> = slots.iter().copied().filter(|&slot| slot != 0).collect();
        slots.sort_unstable();
        slots.dedup();
        if let Some((kept, small)) = &self.small_array_group {
            if *kept == slots {
                self.counts.small_groups_kept += 1;
                return small.clone();
            }
        }
        let mut remap = FxHashMap::default();
        remap.insert(0, 0);
        let view = |slot: u32| {
            self.frame_texture_views
                .get(slot as usize)
                .map_or(&*self.white, |view| &**view)
        };
        let mut views: Vec<&wgpu::TextureView> = vec![view(0)];
        for &slot in &slots {
            remap.insert(slot, views.len() as u32);
            views.push(view(slot));
        }
        let samplers: Vec<&wgpu::Sampler> = self.frame_samplers.iter().map(|sampler| &**sampler).collect();
        self.counts.array_groups_made += 1;
        let group = Rc::new(crate::gpu::frame_group(
            &self.device,
            &wgpu::BindGroupDescriptor {
                label: Some("draw texture arrays, small"),
                layout: &self.array_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureViewArray(&views),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::SamplerArray(&samplers),
                    },
                ],
            },
        ));
        let small = (group, Rc::new(remap));
        self.small_array_group = Some((slots, small.clone()));
        small
    }

    /// Room in the buffer of the indirect draws for `draws` arguments. A new buffer when the
    /// room is short: only between the parts of a frame (the calls of a part name the buffer
    /// of their time, and the part is submitted before the next begins).
    pub(crate) fn reserve_indirect(&mut self, draws: usize) {
        if draws > self.indirect_capacity {
            self.indirect_capacity = draws.next_power_of_two();
            self.indirect_buffer = Rc::new(indirect_buffer(&self.device, self.indirect_capacity));
            self.indirect_written = 0;
        }
    }

    /// Puts the arguments of the indirect draws made since the last upload on the GPU.
    /// Before a submit.
    pub(crate) fn upload_indirect(&mut self, args: &[IndirectArgs]) {
        if args.len() > self.indirect_written {
            self.queue.write_buffer(
                &self.indirect_buffer,
                (self.indirect_written * ARGS_SIZE) as u64,
                bytemuck::cast_slice(&args[self.indirect_written..]),
            );
            self.indirect_written = args.len();
        }
    }

    /// Puts the instances of the frame into the instance buffer, with room after them for
    /// `copies` instances that the frame makes while it draws (`append_instances`).
    pub(crate) fn upload_instances(&mut self, instances: &[Instance], copies: usize) {
        let needed = self.retained_instances + instances.len() + copies;
        if needed > self.instance_capacity {
            self.instance_capacity = needed.next_power_of_two();
            self.instance_buffer = instance_buffer(&self.device, self.instance_capacity);
            self.defaults = defaults_buffer(&self.device, self.instance_capacity);
            self.instance_extras = 0;
            self.instance_generation = self.instance_generation.wrapping_add(1);
        }
        self.instance_len = instances.len();
        if !instances.is_empty() {
            self.queue.write_buffer(
                &self.instance_buffer,
                (self.retained_instances * INSTANCE_SIZE) as u64,
                bytemuck::cast_slice(instances),
            );
        }
    }

    /// Instances made while the frame draws, in a row, from the end of the room: the first,
    /// or [`None`] when the room is full.
    pub(crate) fn append_instances(&mut self, instances: &[Instance]) -> Option<u32> {
        if self.retained_instances + self.instance_len + self.instance_extras + instances.len() > self.instance_capacity {
            return None;
        }
        self.instance_extras += instances.len();
        let first = self.instance_capacity - self.instance_extras;
        self.queue.write_buffer(
            &self.instance_buffer,
            (first * INSTANCE_SIZE) as u64,
            bytemuck::cast_slice(instances),
        );
        Some(first as u32)
    }

    /// The record of a draw of the frame: its data, in the records of the frame after
    /// `upload_records`. Returns its index, or [`None`] when the records reached the device's
    /// limit of a storage binding: the draw is then not drawn (counted, and said once), and
    /// the rest of the frame draws.
    pub(crate) fn record(&mut self, data: &DrawData) -> Option<u32> {
        if self.retained_records + self.record_count >= self.record_limit {
            if self.counts.records_over_limit == 0 {
                println!(
                    "draw records: the frame reached the limit of {} records; draws past it are not drawn",
                    self.record_limit
                );
            }
            self.counts.records_over_limit += 1;
            return None;
        }
        self.record_staging.extend_from_slice(bytemuck::bytes_of(data));
        self.record_count += 1;
        Some((self.retained_records + self.record_count - 1) as u32)
    }

    /// Puts the records made since the last upload on the GPU, with room after them for
    /// `copies` records of `record_now`. Before the draws of a part of the frame.
    pub(crate) fn upload_records(&mut self, copies: usize) {
        // At most the device's limit: the copies past it find no room (`record_now`).
        let needed = (self.retained_records + self.record_count + copies).min(self.record_limit);
        if needed > self.record_capacity || std::mem::take(&mut self.force_record_growth) {
            self.record_capacity = needed.next_power_of_two().min(self.record_limit).max(self.record_capacity);
            self.record_buffer = Rc::new(record_buffer(&self.device, self.record_capacity));
            self.record_written = 0;
            self.record_extras = 0;
            self.record_generation = self.record_generation.wrapping_add(1);
        }
        if self.record_count > self.record_written {
            self.queue.write_buffer(
                &self.record_buffer,
                ((self.retained_records + self.record_written) * RECORD_SIZE) as u64,
                &self.record_staging[self.record_written * RECORD_SIZE..],
            );
            self.record_written = self.record_count;
        }
    }

    /// A record made while the frame draws, written at once, from the end of the room:
    /// its index, or [`None`] when the room is full.
    pub(crate) fn record_now(&mut self, data: &DrawData) -> Option<u32> {
        if self.retained_records + self.record_count + self.record_extras >= self.record_capacity {
            return None;
        }
        self.record_extras += 1;
        let index = self.record_capacity - self.record_extras;
        self.queue
            .write_buffer(&self.record_buffer, (index * RECORD_SIZE) as u64, bytemuck::bytes_of(data));
        Some(index as u32)
    }

    /// A copy of a draw for this part of the frame: a record with `data`, and instances with
    /// the world matrices of the draw's that name it. The first instance, or [`None`] when the
    /// room of the records or of the instances is full.
    pub(crate) fn copy_of_draw(&mut self, data: &DrawData, instances: &[Instance]) -> Option<u32> {
        let began = std::time::Instant::now();
        let first = self.record_now(data).and_then(|record| {
            let copies: Vec<Instance> = instances
                .iter()
                .map(|instance| Instance {
                    world: instance.world,
                    record: [record, 0, 0, 0],
                })
                .collect();
            self.append_instances(&copies)
        });
        // W125: the time and the writes of the copies, for the report and the most of a frame.
        let seconds = began.elapsed().as_secs_f64();
        self.counts.draw_copy_seconds += seconds;
        self.counts.frame_write_seconds += seconds;
        if first.is_some() {
            self.counts.frame_writes += 2;
        }
        first
    }

    /// Puts the bones of the skinned draws of the frame into the bone buffer, as
    /// `upload_lights` does with the lights.
    pub(crate) fn upload_bones(&mut self, bones: &[[f32; 16]]) {
        if bones.len() > self.bone_capacity {
            self.bone_capacity = bones.len().next_power_of_two();
            self.bone_buffer = bone_buffer(&self.device, self.bone_capacity);
        }
        if !bones.is_empty() {
            self.queue.write_buffer(&self.bone_buffer, 0, bytemuck::cast_slice(bones));
        }
    }

    /// Puts the lights of the frame into the light buffer: all of them each time, because
    /// a write is before the commands of the frame in any case. The frame group, made after
    /// this, binds the buffer.
    pub(crate) fn upload_lights(&mut self, lights: &[LightData]) {
        if lights.len() > self.light_capacity {
            self.light_capacity = lights.len().next_power_of_two();
            self.light_buffer = light_buffer(&self.device, self.light_capacity);
        }
        if !lights.is_empty() {
            self.queue.write_buffer(&self.light_buffer, 0, bytemuck::cast_slice(lights));
        }
    }

    /// Makes a texture from a texture record.
    pub(crate) fn texture(&mut self, data: &[u8]) {
        let handle = word(data, 0);
        let (width, height, levels, d3d_format) = (word(data, 8), word(data, 12), word(data, 16), word(data, 20));
        let levels = levels as usize;
        let offsets = 36;
        let widths = offsets + (levels + 1) * 4;
        let heights = widths + levels * 4;
        let pixels = &data[heights + levels * 4..];
        // The size of a block (or 0), and the bytes of a pixel in the stream.
        let Some((format, block, source_bytes)) = texture_format(d3d_format) else {
            *self.counts.texture_formats.entry(d3d_format).or_default() += 1;
            return;
        };
        // The GPU takes a block format only for a size that is a multiple of 4.
        let decode = block != 0 && (width % 4 != 0 || height % 4 != 0);
        if decode && !crate::d3d::cpu_decodes(d3d_format) {
            *self.counts.texture_formats.entry(d3d_format).or_default() += 1;
            return;
        }
        let format = if decode { wgpu::TextureFormat::Bgra8Unorm } else { format };
        if decode {
            self.decoded.insert(handle);
        } else {
            self.decoded.remove(&handle);
        }
        let mut bytes = Vec::new();
        for level in 0..levels {
            let (w, h) = (
                word(data, widths + level * 4) as usize,
                word(data, heights + level * 4) as usize,
            );
            let size = if block != 0 {
                w.div_ceil(4).max(1) * h.div_ceil(4).max(1) * block
            } else {
                w * h * source_bytes
            };
            let from = word(data, offsets + level * 4) as usize;
            let to = (word(data, offsets + (level + 1) * 4) as usize).min(pixels.len());
            let start = bytes.len();
            if decode {
                let mut source = pixels[from.min(to)..to.min(from + size)].to_vec();
                source.resize(size, 0);
                bytes.extend_from_slice(&decode_dxt(d3d_format, w.max(1), h.max(1), &source));
                self.counts.textures_decoded += (level == 0) as u32;
            } else if expands(d3d_format) {
                let mut source = pixels[from.min(to)..to.min(from + size)].to_vec();
                source.resize(size, 0);
                bytes.extend_from_slice(&gpu_pixels(d3d_format, &source));
            } else {
                bytes.extend_from_slice(&pixels[from.min(to)..to.min(from + size)]);
                bytes.resize(start + size, 0);
            }
            if d3d_format == 22 {
                // No alpha in this format.
                for pixel in bytes[start..].chunks_exact_mut(4) {
                    pixel[3] = 255;
                }
            }
        }
        // W632: a texture with one level is made with all its levels, the levels below level 0
        // with zeros, and shows through a view of level 0 (the engine's look) until the worker's
        // chain is in it. A record that comes without its pixels gives its job at the update
        // of level 0.
        let mut made_levels = levels as u32;
        self.chains.remove(&handle);
        let chain =
            levels == 1 && self.mipmaps_for_one_level && width.max(height) > 1 && crate::mip_chain::takes_chain(d3d_format);
        let level0 = (chain && !pixels.is_empty()).then(|| bytes.clone());
        if chain {
            let below = crate::mip_chain::zero_levels_below(d3d_format, decode, width, height);
            made_levels += below.len() as u32;
            for level in &below {
                bytes.extend_from_slice(level);
                self.counts.chain_bytes += level.len() as u64;
            }
            self.counts.chain_textures += 1;
        }
        let texture = self.device.counted_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor {
                label: Some("game texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: made_levels,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                // A test reads the levels back.
                usage: if cfg!(test) {
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC
                } else {
                    wgpu::TextureUsages::TEXTURE_BINDING
                },
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::MipMajor,
            &bytes,
        );
        let view = if chain {
            level0_view(&texture)
        } else {
            Rc::new(texture.counted_view(&Default::default()))
        };
        self.set_texture(handle, view);
        self.texture_objects.insert(handle, (texture, d3d_format));
        self.counts.textures += 1;
        if chain {
            self.chains.insert(
                handle,
                crate::mip_chain::ChainState {
                    generation: 0,
                    d3d_format,
                    decoded: decode,
                    width,
                    height,
                    given: 0,
                    full: None,
                },
            );
            if let Some(level0) = level0 {
                self.give_chain(handle, level0);
            }
        }
    }

    /// Gives the job of a texture's chain (W632): a new generation, so that a chain of an
    /// earlier job is stale.
    fn give_chain(&mut self, handle: u32, level0: Vec<u8>) {
        let Some(state) = self.chains.get_mut(&handle) else { return };
        self.chain_generation += 1;
        state.generation = self.chain_generation;
        state.given = self.counts.frames;
        let job = crate::mip_chain::Job {
            handle,
            generation: state.generation,
            d3d_format: state.d3d_format,
            decoded: state.decoded,
            width: state.width,
            height: state.height,
            level0,
        };
        let worker = self.chain_worker.get_or_insert_with(crate::mip_chain::Worker::start);
        if worker.give(job) {
            self.counts.chain_jobs += 1;
            self.counts.note_chain_frame(1, 0);
        } else {
            self.counts.chain_jobs_lost += 1;
        }
    }

    /// The chains that the worker made, at a frame's start (W632): each goes into the levels of
    /// its texture, and the texture then shows through the view of all its levels. A chain of an
    /// earlier generation (the texture was made again or its level 0 written since) is dropped.
    /// The replay waits for every job given before ([`Renderer::wait_for_chains`]).
    pub(crate) fn take_chains(&mut self) {
        let Some(worker) = self.chain_worker.as_mut() else { return };
        let finished = worker.take(self.wait_for_chains);
        if finished.is_empty() {
            return;
        }
        let clock = ticks::Clock::now();
        for done in finished {
            self.counts.chain_seconds += done.seconds;
            let current = self.chains.get(&done.handle).is_some_and(|s| s.generation == done.generation);
            let Some((texture, _)) = self.texture_objects.get(&done.handle).filter(|_| current) else {
                self.counts.chain_stale += 1;
                continue;
            };
            let texture = texture.clone();
            let state = &self.chains[&done.handle];
            let (d3d_format, decoded, width, height, given) =
                (state.d3d_format, state.decoded, state.width, state.height, state.given);
            for (index, bytes) in done.levels.iter().enumerate() {
                let level = 1 + index as u32;
                let (w, h) = crate::mip_chain::size_of_level(width, height, level);
                let (row, rows, extent) = crate::mip_chain::level_layout(d3d_format, decoded, w, h);
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: level,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    bytes,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: Some(rows),
                    },
                    wgpu::Extent3d {
                        width: extent.0,
                        height: extent.1,
                        depth_or_array_layers: 1,
                    },
                );
            }
            let full = Rc::new(texture.counted_view(&Default::default()));
            self.set_texture(done.handle, full.clone());
            if let Some(state) = self.chains.get_mut(&done.handle) {
                state.full = Some(full);
            }
            self.counts.chain_swaps += 1;
            self.counts.note_chain_frame(0, 1);
            self.counts.chain_most_delay = self.counts.chain_most_delay.max(self.counts.frames.saturating_sub(given));
        }
        self.counts.times[crate::TIME_CHAINS] += clock.elapsed();
    }

    /// The pipeline for a draw in screen coordinates: a frame of a film. It does not blend:
    /// the film player of the game is the only caller of `DrawPrimitiveUP`
    /// (`CutscenePlayer_static::drawRect`), and it sets `D3DRS_ALPHABLENDENABLE` to 0 before
    /// its draws (`CutscenePlayer_static::setupD3DState` 0x64ECD0). `samples` is that of the
    /// pass that the draw goes into.
    pub(crate) fn screen_pipeline(&mut self, samples: u32) -> wgpu::RenderPipeline {
        if self.screen_layout.is_none() {
            self.screen_layout = Some(self.device.counted_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("screen draw"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            }));
            self.screen_sampler = Some(self.device.counted_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }));
        }
        if let Some(pipeline) = self.screen_pipelines.get(&samples) {
            return pipeline.clone();
        }
        let module = self.device.counted_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("screen draw"),
            source: wgpu::ShaderSource::Wgsl(SCREEN_SHADER.into()),
        });
        let layout = self.device.counted_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("screen draw"),
            bind_group_layouts: &[self.screen_layout.as_ref()],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x2];
        let _long = crate::LongStep::begin();
        let pipeline = self.device.counted_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("screen draw"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 40,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: self.depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.screen_pipelines.insert(samples, pipeline.clone());
        pipeline
    }

    /// Makes a texture without pixels: the game made it with `CreateTexture` and writes each
    /// level later. `levels` 0 is all the levels down to 1 x 1.
    pub(crate) fn empty_texture(&mut self, handle: u32, width: u32, height: u32, levels: u32, d3d_format: u32) {
        let Some((format, block, _)) = texture_format(d3d_format) else {
            *self.counts.texture_formats.entry(d3d_format).or_default() += 1;
            return;
        };
        let (width, height) = (width.max(1), height.max(1));
        // The GPU takes a block format only for a size that is a multiple of 4. For each other
        // size this program makes the pixels of each level itself.
        let decode = block != 0 && (width % 4 != 0 || height % 4 != 0);
        if decode && !crate::d3d::cpu_decodes(d3d_format) {
            *self.counts.texture_formats.entry(d3d_format).or_default() += 1;
            return;
        }
        let format = if decode { wgpu::TextureFormat::Bgra8Unorm } else { format };
        if decode {
            self.decoded.insert(handle);
            self.counts.textures_decoded += 1;
        } else {
            self.decoded.remove(&handle);
        }
        // A texture that the game makes keeps the game's levels.
        self.chains.remove(&handle);
        let all = 32 - width.max(height).leading_zeros();
        let levels = if levels == 0 { all } else { levels.min(all) };
        let texture = self.device.counted_texture(&wgpu::TextureDescriptor {
            label: Some("game render target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.set_texture(handle, Rc::new(texture.counted_view(&Default::default())));
        self.texture_objects.insert(handle, (texture, d3d_format));
        self.counts.textures += 1;
    }

    /// Makes a texture that the game draws into, with a depth buffer of its size: the size that
    /// the game asked for, each side cut to the device's largest ([`target_size`]).
    pub(crate) fn render_texture(&mut self, handle: u32, width: u32, height: u32) {
        let asked = (width.max(1), height.max(1));
        let largest = self.device.limits().max_texture_dimension_2d;
        let (width, height) = target_size(asked, largest);
        if (width, height) != asked {
            self.counts.render_textures_cut += 1;
            if !self.counts.render_texture_cut_sizes.contains(&asked) {
                self.counts.render_texture_cut_sizes.push(asked);
                println!("rendered texture: the game asked for {} x {}, and the device's largest side is {largest}: made {width} x {height}", asked.0, asked.1);
            }
        }
        let texture = target_texture(
            &self.device,
            width,
            height,
            self.target_format,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = target_texture(
            &self.device,
            width,
            height,
            self.depth_format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let view = Rc::new(texture.counted_view(&Default::default()));
        self.set_texture(handle, view.clone());
        self.texture_objects.remove(&handle);
        self.chains.remove(&handle);
        self.targets.insert(
            handle,
            RenderTexture {
                texture,
                view,
                depth: Rc::new(depth.counted_view(&Default::default())),
                depth_texture: depth,
                size: (width, height),
                asked,
            },
        );
        self.counts.render_textures += 1;
    }

    /// The game copied pixels of its own into a texture that it draws into. `pixels` has 4
    /// bytes for each pixel, blue first, at the size that the game asked for.
    pub(crate) fn target_pixels(&mut self, handle: u32, width: u32, height: u32, pixels: &[u8]) {
        let Some(target) = self.targets.get(&handle) else { return };
        if target.asked != (width, height) || pixels.len() < (width * height * 4) as usize {
            self.counts.texture_updates_lost += 1;
            return;
        }
        let game_pixels = &pixels[..(width * height * 4) as usize];
        let (width, height) = target.size;
        let mut bytes = if target.size == target.asked {
            game_pixels.to_vec()
        } else {
            resample_nearest(game_pixels, target.asked, target.size)
        };
        if !matches!(
            self.target_format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        ) {
            for pixel in bytes.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.counts.target_copies += 1;
    }

    /// The game wrote a level of a texture again.
    pub(crate) fn texture_update(&mut self, data: &[u8]) {
        let (handle, level, width, height, size) = (
            word(data, 0),
            word(data, 4),
            word(data, 8),
            word(data, 12),
            word(data, 20) as usize,
        );
        let Some((texture, d3d_format)) = self.texture_objects.get(&handle) else {
            self.counts.texture_updates_lost += 1;
            *self
                .counts
                .lost_reasons
                .entry(format!(
                    "the texture is not one that this program made, format {:#x}",
                    word(data, 16)
                ))
                .or_default() += 1;
            return;
        };
        let decode = self.decoded.contains(&handle);
        let (block, row, rows, expected) = crate::d3d::level_layout(*d3d_format, width, height);
        if level >= texture.mip_level_count() || size < expected || data.len() < 24 + expected {
            self.counts.texture_updates_lost += 1;
            let reason = if level >= texture.mip_level_count() {
                "the texture has no such level"
            } else {
                "the record has too few bytes"
            };
            *self
                .counts
                .lost_reasons
                .entry(format!(
                    "{reason}, format {d3d_format:#x}, level {level} of {}, {width} x {height}",
                    texture.mip_level_count()
                ))
                .or_default() += 1;
            return;
        }
        let mut pixels = std::borrow::Cow::Borrowed(&data[24..24 + expected]);
        let (mut row, mut block) = (row, block);
        if decode {
            pixels = std::borrow::Cow::Owned(decode_dxt(*d3d_format, width as usize, height as usize, &pixels));
            (row, block) = (width * 4, 0);
        }
        let rows = if decode { height } else { rows };
        if expands(*d3d_format) {
            pixels = std::borrow::Cow::Owned(gpu_pixels(*d3d_format, &pixels));
        } else if *d3d_format == 22 {
            // No alpha in this format.
            for pixel in pixels.to_mut().chunks_exact_mut(4) {
                pixel[3] = 255;
            }
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(rows),
            },
            wgpu::Extent3d {
                width: if block != 0 { width.next_multiple_of(4) } else { width },
                height: if block != 0 { height.next_multiple_of(4) } else { height },
                depth_or_array_layers: 1,
            },
        );
        // W632, the Architect's W2: a new level 0 of a texture that takes a chain. Its levels
        // below hold the old picture, so it shows through the view of level 0 again from this
        // frame, until the job of the new level 0 is done.
        if level == 0 && self.chains.contains_key(&handle) {
            let one = level0_view(texture);
            let level0 = pixels.into_owned();
            if self.chains.get_mut(&handle).and_then(|state| state.full.take()).is_some() {
                self.set_texture(handle, one);
            }
            self.give_chain(handle, level0);
            self.counts.chain_updates += 1;
        }
        self.counts.texture_updates += 1;
    }

    /// A geometry record: resident buffers for the draws that name it. A new version of a
    /// geometry with the same layout goes into the ranges it has, so the draws and the table's
    /// slots that name them stay: true then.
    pub(crate) fn geometry(&mut self, data: &[u8]) -> bool {
        let clock = ticks::Clock::now();
        let (handle, fvf, stride, vertices, entries, strips) = (
            word(data, 0),
            word(data, 12),
            word(data, 16),
            word(data, 20),
            word(data, 24),
            word(data, 28),
        );
        let lengths = ((strips as usize * 2) + 3) & !3;
        let (vertex_at, index_at) = (32 + lengths, 32 + lengths + vertices as usize * stride as usize);
        let index_end = index_at + entries as usize * 2;
        if data.len() < index_end || stride == 0 {
            self.counts.geometry_bad += 1;
            return false;
        }
        let strip_lengths: Vec<u16> = (0..strips as usize)
            .map(|i| u16::from_le_bytes([data[32 + i * 2], data[33 + i * 2]]))
            .collect();
        self.counts.geometry_records += 1;
        self.counts.geometry_bytes += data.len() as u64;
        let (vertex_bytes, index_bytes) = (&data[vertex_at..index_at], &data[index_at..index_end]);
        // A shape that the game skins in software: its bone influences after the indexes. Its
        // vertices name them, so its record takes new ranges (`influences.rs`).
        let list = if fvf & FVF_INFLUENCES != 0 {
            let list = data
                .len()
                .checked_sub(index_end + 4)
                .filter(|_| stride >= 28)
                .and_then(|left| {
                    let bytes = word(data, index_end) as usize * crate::influences::ENTRY_BYTES as usize;
                    (bytes <= left).then(|| &data[index_end + 4..index_end + 4 + bytes])
                });
            let Some(list) = list else {
                self.counts.geometry_bad += 1;
                return false;
            };
            Some(list)
        } else {
            None
        };
        let same_layout = self.geometries.get(&handle).filter(|old| {
            list.is_none()
                && old.unique == Some(word(data, 4))
                && old.fvf == fvf
                && old.stride == stride
                && old.entries == entries
                && old.strips == strip_lengths
        });
        // A draw of this frame named the ranges already (the screen polygon of each blob of the
        // fog of war): the write would land before that draw too.
        let drawn_in_frame = same_layout.is_some_and(|old| old.drawn_in == self.geometry_frame);
        if drawn_in_frame {
            self.counts.geometry_drawn_in_frame += 1;
        }
        if let Some((Some(vertex_range), Some(index_range))) = same_layout
            .filter(|_| !drawn_in_frame)
            .map(|old| (old.vertex_range.as_ref(), old.index_range.as_ref()))
        {
            let fits = |range: &pool::Range, bytes: &[u8]| (bytes.len() as u64).next_multiple_of(4).max(4) == range.size;
            if fits(vertex_range, vertex_bytes) && fits(index_range, index_bytes) {
                pool::Pool::rewrite(&self.queue, vertex_range, vertex_bytes);
                pool::Pool::rewrite(&self.queue, index_range, index_bytes);
                // The new positions: their bound, and no morph until the next step.
                if let Some(old) = self.geometries.get_mut(&handle) {
                    old.bound = crate::bound::geometry_bound(fvf, stride, vertex_bytes, None);
                    old.morph_sphere = None;
                }
                self.counts.geometry_in_place += 1;
                self.counts.times[TIME_BUFFERS] += clock.elapsed();
                return true;
            }
        }
        // The bound from the record's own bytes, before the list's entries are moved.
        let bound = crate::bound::geometry_bound(fvf, stride, vertex_bytes, list);
        let (moved, influence_range) = match list {
            Some(list) => {
                let (range, first) = self.influences.put(&self.device, &self.queue, list);
                (
                    Some(crate::influences::moved_vertices(vertex_bytes, stride as usize, first)),
                    Some(range),
                )
            }
            None => (None, None),
        };
        let vertex_bytes = moved.as_deref().unwrap_or(vertex_bytes);
        let vertex_range = self.vertex_pool.take(&self.device, &self.queue, vertex_bytes, stride as u64);
        let index_range = self.index_pool.take(&self.device, &self.queue, index_bytes, 4);
        let geometry = Geometry {
            influence_range,
            bones_needed: list.map_or(0, crate::influences::bones_needed),
            fvf,
            stride,
            entries,
            strips: strip_lengths,
            vertex: vertex_range.buffer.clone(),
            index: index_range.buffer.clone(),
            base_vertex: (vertex_range.offset / stride as u64) as i32,
            first_index: (index_range.offset / 2) as u32,
            vertex_range: Some(vertex_range),
            index_range: Some(index_range),
            unique: Some(word(data, 4)),
            uv: None,
            drawn_in: 0,
            particles: false,
            bound,
            morph_sphere: None,
        };
        self.put_geometry(handle, geometry);
        self.counts.times[TIME_BUFFERS] += clock.elapsed();
        false
    }

    /// Puts a geometry under its handle. The ranges of the geometry it replaces go back to the
    /// pools after the frame: the draws of this frame may still use them.
    fn put_geometry(&mut self, handle: u32, geometry: Geometry) {
        if let Some(old) = self.geometries.insert(handle, geometry) {
            self.retired.extend(old.vertex_range);
            self.retired_indexes.extend(old.index_range);
            self.retired_influences.extend(old.influence_range);
        }
    }

    /// A geometry data of the game was destroyed (`GEOMETRY_GONE`): its geometry goes, and its
    /// ranges go back to the pools after the frame. Only a geometry of the same unique id goes
    /// (the quads of particles have none: they go too). True when a geometry went.
    pub(crate) fn geometry_gone(&mut self, handle: u32, unique: u32) -> bool {
        if !self
            .geometries
            .get(&handle)
            .is_some_and(|g| g.unique.is_none_or(|id| id == unique))
        {
            self.counts.geometries_gone_not_held += 1;
            return false;
        }
        if let Some(old) = self.geometries.remove(&handle) {
            self.retired.extend(old.vertex_range);
            self.retired_indexes.extend(old.index_range);
            self.retired_influences.extend(old.influence_range);
        }
        if let Some(morphs) = self.morphs.as_mut() {
            morphs.gone(handle, unique);
        }
        self.counts.geometries_gone += 1;
        true
    }

    /// The targets of a geometry's morph ([`tag::MORPH_TARGETS`]): the handle, the unique id,
    /// the targets, the vertices of a target, then the positions.
    pub(crate) fn morph_targets(&mut self, data: &[u8]) {
        let (handle, unique, count, vertices) = (word(data, 0), word(data, 4), word(data, 8), word(data, 12));
        let floats = (count as usize) * (vertices as usize) * 3;
        if data.len() < 16 + floats * 4 {
            self.counts.short_records += 1;
            return;
        }
        let values: Vec<f32> = data[16..16 + floats * 4]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let device = self.device.clone();
        let morphs = self.morphs.get_or_insert_with(|| morph::Morphs::new(&device));
        morphs.put_targets(&self.device, &self.queue, handle, unique, count, vertices, &values);
        self.counts.morph_targets += 1;
    }

    /// The weights of a new revision of a geometry ([`tag::MORPH_WEIGHTS`]): the handle, the
    /// unique id, the number of weights, then the weights.
    pub(crate) fn morph_weights(&mut self, data: &[u8]) {
        let (handle, unique, count) = (word(data, 0), word(data, 4), word(data, 8) as usize);
        if data.len() < 12 + count * 4 {
            self.counts.short_records += 1;
            return;
        }
        let weights: Vec<f32> = (0..count).map(|i| float(data, 12 + i * 4)).collect();
        let (Some(morphs), Some(geometry)) = (
            self.morphs.as_mut(),
            self.geometries.get(&handle).filter(|g| g.unique == Some(unique)),
        ) else {
            self.counts.morph_missing += 1;
            return;
        };
        if morphs.step(&self.device, handle, unique, &weights, geometry) {
            let sphere = morphs.sphere(handle, &weights);
            if let Some(geometry) = self.geometries.get_mut(&handle) {
                // The job writes the positions: with no sphere of them there is no bound until
                // the next record.
                match sphere {
                    Some(sphere) => geometry.morph_sphere = Some(sphere),
                    None => geometry.bound = None,
                }
            }
        }
    }

    /// The compute pass of the frame's morphs, into the frame's commands: before its draws.
    pub(crate) fn flush_morphs(&mut self) {
        if let Some(morphs) = self.morphs.as_mut() {
            morphs.flush(&self.device, &self.queue, &self.commands);
        }
    }

    /// The morph counts for the report: the steps made and those not made.
    pub(crate) fn morph_counts(&self) -> (u64, u64) {
        self.morphs.as_ref().map_or((0, 0), |m| (m.steps, m.skipped))
    }

    /// A texture or a level surface of the plugin's device was freed (`TEXTURE_GONE`). A level
    /// surface goes at once. A texture goes at the start of the next frame
    /// (`drop_gone_textures`): the items of the frame in progress run at its end, and a
    /// target change among them can name the texture (the game draws into a rendered texture,
    /// sets another target, then frees it, in one frame). A pinned slot keeps its view until
    /// the instances that use it go.
    pub(crate) fn texture_gone(&mut self, handle: u32) {
        let surface = self.surfaces.remove(&handle).is_some();
        match self.textures.get(&handle) {
            Some(view) => self.gone_textures.push((handle, view.clone())),
            None if surface => self.counts.textures_gone += 1,
            None => self.counts.textures_gone_not_held += 1,
        }
    }

    /// The textures freed in the frame before go. A handle that got a new texture since its
    /// free (the address came back) keeps the new one.
    pub(crate) fn drop_gone_textures(&mut self) {
        for (handle, view) in std::mem::take(&mut self.gone_textures) {
            if !self.textures.get(&handle).is_some_and(|now| Rc::ptr_eq(now, &view)) {
                self.counts.textures_gone_replaced += 1;
                continue;
            }
            self.textures.remove(&handle);
            self.stage_groups.forget(handle);
            self.texture_objects.remove(&handle);
            self.targets.remove(&handle);
            self.decoded.remove(&handle);
            self.chains.remove(&handle);
            // The arrays stay from frame to frame: a slot of the frame's own part lets the
            // texture go. A pinned slot goes with its last use (`unpin_texture`).
            if let Some(&slot) = self.frame_texture_slots.get(&handle) {
                if slot as usize > self.pinned_in_frame {
                    self.frame_texture_views[slot as usize] = self.white.clone();
                    self.frame_texture_slots.remove(&handle);
                    self.forget_array_groups(array_change::TEXTURE_OUT);
                }
            }
            self.counts.textures_gone += 1;
        }
    }

    /// One line of what the helper holds for the handles of the game: textures, geometry,
    /// materials, and the frees so far. The bytes of the textures are those of their colour
    /// ([`texture_bytes`]); the depth of a rendered texture is not in them.
    pub(crate) fn held_line(&self) -> String {
        let mb = |bytes: u64| bytes as f64 / 1e6;
        let written: u64 = self.texture_objects.values().map(|(texture, _)| bytes_of(texture)).sum();
        let rendered: u64 = self.targets.values().map(|target| bytes_of(&target.texture)).sum();
        format!(
            "held: {} textures ({} with pixels written here, {:.1} MB; {} rendered, {:.1} MB), {} level surfaces; {} geometries, the vertex pool {:.1} of {:.1} MB, the index pool {:.1} of {:.1} MB; {} materials ({} derived); {} texture groups of the stages (tier c); gone: {} materials, {} textures ({} not held, {} replaced before their removal), {} geometries ({} not held), {} instances whose geometry went",
            self.textures.len(),
            self.texture_objects.len(),
            mb(written),
            self.targets.len(),
            mb(rendered),
            self.surfaces.len(),
            self.geometries.len(),
            mb(self.vertex_pool.used),
            mb(self.vertex_pool.capacity()),
            mb(self.index_pool.used),
            mb(self.index_pool.capacity()),
            self.materials.len(),
            self.derived.len(),
            self.stage_groups.len(),
            self.counts.materials_gone,
            self.counts.textures_gone,
            self.counts.textures_gone_not_held,
            self.counts.textures_gone_replaced,
            self.counts.geometries_gone,
            self.counts.geometries_gone_not_held,
            self.counts.geometry_orphans
        )
    }

    /// The geometry of a particle system for this frame, from the particles of its shape
    /// record: four vertices for each particle, a quad that faces the camera, as
    /// `NiDX8Renderer::RenderPoints` writes them (the position, the normal 0.3, the
    /// colour, the texture coordinates; 36 bytes). The quads are `(right + up) * radius`
    /// and `(up - right) * radius` from the position, scaled by the size of the particle.
    /// The buffer replaces the one of the last frame under the same key. The corners are made
    /// on the GPU (`particle_quads`).
    pub(crate) fn particles(&mut self, handle: u32, radius: f32, right: [f32; 3], up: [f32; 3], particles: &[u8]) {
        let clock = ticks::Clock::now();
        const STRIDE: usize = 36;
        let count = particles.len() / 20;
        let sum: [f32; 3] = core::array::from_fn(|i| (right[i] + up[i]) * radius);
        let difference: [f32; 3] = core::array::from_fn(|i| (up[i] - right[i]) * radius);
        let index = self.quad_index_buffer(count as u32);
        let device = self.device.clone();
        let quads = self
            .particle_quads
            .get_or_insert_with(|| particle_quads::ParticleQuads::new(&device));
        let (vertex, place) = quads.add(&device, particles, sum, difference);
        self.counts.shape_particles += 1;
        // D3DFVF_XYZ | D3DFVF_NORMAL | D3DFVF_DIFFUSE | D3DFVF_TEX1.
        let geometry = Geometry {
            fvf: 0x152,
            stride: STRIDE as u32,
            entries: place.indices,
            strips: Vec::new(),
            vertex,
            index,
            vertex_range: None,
            index_range: None,
            base_vertex: place.first_vertex as i32,
            first_index: 0,
            unique: None,
            influence_range: None,
            bones_needed: 0,
            uv: None,
            drawn_in: 0,
            particles: true,
            // The positions come with each record: so does the bound.
            bound: crate::bound::particle_sphere(radius, right, up, particles).map(|sphere| crate::bound::ModelBound {
                sphere,
                spread: 1.0,
                excess: 0.0,
            }),
            morph_sphere: None,
        };
        self.put_geometry(handle, geometry);
        self.counts.times[TIME_BUFFERS] += clock.elapsed();
    }

    /// The index buffer of the quads, two triangles of four vertices each, for at least
    /// `quads` quads: the game keeps one and it grows to the largest system seen.
    fn quad_index_buffer(&mut self, quads: u32) -> Rc<wgpu::Buffer> {
        if self.quad_indexes.as_ref().is_none_or(|(have, _)| *have < quads) {
            let indexes: Vec<u8> = (0..quads)
                .flat_map(|q| [0u16, 1, 2, 0, 2, 3].map(|i| (q * 4) as u16 + i))
                .flat_map(|i| i.to_le_bytes())
                .collect();
            let buffer = self.device.counted_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("quads"),
                contents: &indexes,
                usage: wgpu::BufferUsages::INDEX,
            });
            self.quad_indexes = Some((quads, Rc::new(buffer)));
        }
        self.quad_indexes
            .as_ref()
            .map(|(_, buffer)| buffer.clone())
            .expect("the quad index buffer was just made")
    }

    /// The systems and the particles whose quads the GPU made, when it made any.
    pub(crate) fn particle_quads_made(&self) -> Option<(u64, u64)> {
        self.particle_quads.as_ref().map(|quads| (quads.systems, quads.particles))
    }

    /// The start of a frame for the particle quads made on the GPU.
    pub(crate) fn begin_frame_particles(&mut self) {
        if let Some(quads) = self.particle_quads.as_mut() {
            quads.begin_frame();
        }
    }

    /// The compute passes of the particle systems that wait, into the frame's commands:
    /// before the draws of a run of items.
    pub(crate) fn flush_particles(&mut self) {
        if let Some(quads) = self.particle_quads.as_mut() {
            quads.flush(&self.device, &self.queue, &self.commands);
        }
    }

    pub(crate) fn sampler(&mut self, values: [u32; 5]) -> Rc<wgpu::Sampler> {
        let device = &self.device;
        let anisotropy = self.anisotropy;
        // D3DTEXF_NONE is 0: a texture without levels in use keeps its filter.
        let forced = anisotropy > 1
            && filter(values[2]) == wgpu::FilterMode::Linear
            && filter(values[3]) == wgpu::FilterMode::Linear
            && known(values[4], 0) != 0;
        let sampler = self.samplers.entry(values).or_insert_with(|| {
            Rc::new(device.counted_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: address(values[0]),
                address_mode_v: address(values[1]),
                // MGE XE gives each texture that the game filters its anisotropic filter, with
                // a linear filter between the levels. wgpu wants all filters linear for that.
                anisotropy_clamp: if forced { anisotropy } else { 1 },
                // D3DTEXF_NONE for the filter between levels: Direct3D reads the first level only.
                lod_max_clamp: if known(values[4], 0) == 0 { 0.0 } else { 32.0 },
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

    /// The pipeline of a draw's key, in the frame's world variant ([`Renderer::set_debug_variant`]).
    /// The variant is set here, where the draw is drawn: a key that the table keeps has none.
    pub(crate) fn pipeline(&mut self, key: PipelineKey) -> Option<Rc<wgpu::RenderPipeline>> {
        self.pipeline_of(PipelineKey {
            debug: self.debug_variant,
            ..key
        })
    }

    fn pipeline_of(&mut self, key: PipelineKey) -> Option<Rc<wgpu::RenderPipeline>> {
        // A stage key's own pipeline, or the uber-shader's while it is not made or when it failed.
        if key.stages != 0 {
            if let Some(pipeline) = self.stage_shaders.pipeline(key, &self.maker, &mut self.pipelines) {
                return Some(pipeline);
            }
            return self.pipeline_of(PipelineKey { stages: 0, ..key });
        }
        if let Some(pipeline) = self.pipelines.get(&key) {
            return Some(pipeline.clone());
        }
        // The first draw of a key of the uber set that the thread made (`crate::stage_shaders`).
        if key.debug == 0 {
            if let Some(pipeline) = self.stage_shaders.uber_pipeline(key) {
                let pipeline = Rc::new(pipeline);
                self.pipelines.insert(key, pipeline.clone());
                return Some(pipeline);
            }
        }
        attributes(key.fvf, key.stride)?;
        let _long = crate::LongStep::begin();
        let built = if key.debug == 0 {
            // While the extension may still give another text, a fault waits for the settle
            // between two frames (`Renderer::settle_world_shader`), and no other pipeline of the
            // text is built until then: its draws wait too. Once no other text is left, a failed
            // key is kept as it is, as before W317.
            if self.world.fault.is_some() && !self.world.exhausted {
                return None;
            }
            let (built, error) = self.maker.guarded_build(key, &self.shader, &[], Making::GeneralPipeline);
            if let Some(error) = error {
                if !self.world.exhausted {
                    self.world.fault = Some(error);
                    self.world.fault_key = Some(key);
                    return None;
                }
            }
            built
        } else {
            // A variant without its module draws as normal play.
            let Some(fragment) = self.variant_shader() else {
                return self.pipeline_of(PipelineKey { debug: 0, ..key });
            };
            self.maker.general(key, &fragment)
        };
        let pipeline = Rc::new(built?);
        self.pipelines.insert(key, pipeline.clone());
        Some(pipeline)
    }

    /// The world variant of the frame's debug view ([`crate::debug_lines::world_variant`]). A
    /// change drops the old variant: its module, its pipelines and what the stage shaders keep
    /// of it. So at most one variant exists beside normal play.
    pub(crate) fn set_debug_variant(&mut self, variant: u32) {
        if variant == self.debug_variant {
            return;
        }
        let old = self.debug_variant;
        self.debug_variant = variant;
        self.variant_shader = None;
        let before = self.pipelines.len();
        self.pipelines.retain(|key, _| key.debug == 0);
        self.stage_shaders.set_variant(variant);
        println!(
            "debug view: world variant {variant}; {} pipelines of variant {old} dropped",
            before - self.pipelines.len()
        );
    }

    /// The world shader's module of the frame's variant, made at its first use. None when its
    /// text or its module fails.
    fn variant_shader(&mut self) -> Option<wgpu::ShaderModule> {
        if self.variant_shader.is_none() {
            let variant = self.debug_variant;
            let module = self
                .stage_shaders
                .world_text(variant)
                .and_then(|source| self.maker.module("debug view", source));
            if module.is_some() {
                println!("debug view: the world shader's module of variant {variant} made");
            }
            self.variant_shader = Some(module);
        }
        self.variant_shader.clone().flatten()
    }

    /// A live session makes the stage pipelines away from the frame, on a thread. The thread starts
    /// once the world shader's text has settled ([`Renderer::settle_world_shader`]), so that the
    /// first pipelines of a text are built alone, inside the extension's build marker.
    pub(crate) fn start_stage_shaders(&mut self) {
        self.world.stages_wanted = true;
    }

    /// Starts the thread of the stage pipelines of the text now, when a live session wants it.
    fn start_wanted_stage_shaders(&mut self) {
        if self.world.stages_wanted && !self.world.stages_running {
            self.stage_shaders.start(self.maker.clone(), self.samples);
            self.world.stages_running = true;
        }
    }

    /// Settles the world shader's text between two frames (W317). A text of the extension holds
    /// when its module, the pipeline of [`crate::stage_shaders::first_world_key`] and the pipeline
    /// of the key that failed (when a key failed) are made with no error; then the extension hears
    /// it ([`Extension::world_shader_held`]). The extension hears of each such build before it
    /// starts ([`Extension::world_shader_building`]).
    ///
    /// A text fails at that test, or later at the pipeline of a key ([`Renderer::pipeline`]). Then
    /// the key that failed is built one time with the extension's built-in text
    /// ([`Extension::built_in_text`]): when it fails with that text too, the fault is no fault of
    /// the code of others, and the text and the key are kept as they are
    /// ([`Extension::world_shader_kept`]). Else the extension gives the next text to try
    /// ([`Extension::shader_fallback`]) until one holds or none is left. A change of the
    /// extension's own ([`Extension::world_shader_changed`]) is tried the same way.
    ///
    /// With no extension, the first call keeps a failed key as it is. Once the text settled, this
    /// costs one call of the extension a frame.
    pub(crate) fn settle_world_shader(&mut self, extension: Option<&mut (dyn Extension + '_)>) {
        let Some(extension) = extension else {
            if !self.world.held {
                self.world.held = true;
                self.world.exhausted = true;
                self.start_wanted_stage_shaders();
            }
            return;
        };
        if let Some(text) = extension.world_shader_changed() {
            println!("core mods: the world shader takes the text {} of the extension", text.label);
            self.switch_world_shader(&text);
            self.world.attempt = 0;
            self.world.trial_key = None;
        }
        loop {
            if self.world.fault.is_none() {
                if self.world.held {
                    return;
                }
                extension.world_shader_building(self.world.attempt);
                if let Some((error, key)) = self.hold_test() {
                    self.world.fault = Some(error);
                    self.world.fault_key = Some(key);
                } else {
                    self.world.held = true;
                    extension.world_shader_held(self.world.attempt);
                    // A later fault starts its own walk from the extension's text now.
                    self.world.attempt = 0;
                    self.world.trial_key = None;
                    self.start_wanted_stage_shaders();
                    return;
                }
            }
            let Some(error) = self.world.fault.take() else {
                continue;
            };
            let key = self
                .world
                .fault_key
                .take()
                .unwrap_or_else(|| crate::stage_shaders::first_world_key(self.samples));
            if self.world.attempt == 0 {
                self.world.trial_key = Some(key);
                if let Some(probe_error) = self.probe_built_in(&mut *extension, key) {
                    println!(
                        "core mods: the world shader's pipeline failed with the built-in text too ({probe_error}); \
                         no core mod is dropped"
                    );
                    extension.world_shader_kept(&probe_error);
                    self.keep_failed_pipelines();
                    return;
                }
            }
            self.world.attempt += 1;
            match extension.shader_fallback(self.world.attempt, &error) {
                Some(text) => {
                    println!(
                        "core mods: the world shader failed ({error}); it tries the text {}",
                        text.label
                    );
                    self.switch_world_shader(&text);
                }
                None => {
                    println!(
                        "core mods: the world shader failed ({error}); the extension has no other text, so a failed \
                         pipeline is kept as it is"
                    );
                    self.keep_failed_pipelines();
                    return;
                }
            }
        }
    }

    /// No other text is tried: a pipeline that fails is kept as it is, as before W317.
    fn keep_failed_pipelines(&mut self) {
        self.world.exhausted = true;
        self.world.held = true;
        self.world.trial_key = None;
        self.start_wanted_stage_shaders();
    }

    /// The error of `key` built one time with the extension's built-in text, or None when it
    /// builds or the extension has no such text. The session's text stays.
    fn probe_built_in(&self, extension: &mut (dyn Extension + '_), key: PipelineKey) -> Option<String> {
        let text = extension.built_in_text()?;
        let source = world_source_of(&text.extension_text(), self.tier);
        let (module, module_error) = world_module(&self.maker.device, &source);
        if module_error.is_some() {
            return module_error;
        }
        let probe = PipelineMaker {
            vertex: module.clone(),
            planted: false,
            ..self.maker.clone()
        };
        probe.guarded_build(key, &module, &[], Making::GeneralPipeline).1
    }

    /// The pipelines of the first key of the world and of the key of the walk, made and kept: None
    /// when they hold, else the error of the device and the key that failed.
    fn hold_test(&mut self) -> Option<(String, PipelineKey)> {
        let first = crate::stage_shaders::first_world_key(self.samples);
        let keys = std::iter::once(first).chain(self.world.trial_key.filter(|key| *key != first));
        for key in keys {
            let (pipeline, error) = self.maker.guarded_build(key, &self.shader, &[], Making::GeneralPipeline);
            if let Some(error) = error {
                return Some((error, key));
            }
            if let Some(pipeline) = pipeline {
                self.pipelines.insert(key, Rc::new(pipeline));
            }
        }
        None
    }

    /// Makes the world shader of `text` and drops every pipeline and stage shader of the text
    /// before: the next draws build theirs from the new module. The fault of the new module, if
    /// any, waits in `self.world.fault` for [`Renderer::settle_world_shader`].
    fn switch_world_shader(&mut self, text: &ShaderText) {
        let source = world_source_of(&text.extension_text(), self.tier);
        let (shader, fault) = world_module(&self.maker.device, &source);
        self.maker.vertex = shader.clone();
        self.maker.planted = source.contains(crate::CORE_MOD_FAULT_PLANT);
        self.shader = shader;
        self.pipelines.clear();
        self.variant_shader = None;
        let variant = self.debug_variant;
        let before = std::mem::replace(&mut self.stage_shaders, crate::stage_shaders::StageShaders::new(false, ""));
        // The session's last report line has only the new text's counts: this line keeps the old text's.
        println!("world shader switch: the text before: {}", before.report());
        self.stage_shaders = before.switched(&source);
        self.stage_shaders.set_variant(variant);
        // The new thread starts once the new text settles.
        self.world.stages_running = false;
        self.world.fault = fault;
        self.world.fault_key = None;
        self.world.held = false;
        self.world.exhausted = false;
    }

    /// The start of a frame: the stage pipelines that the thread made, and the list's write.
    pub(crate) fn begin_frame_stage_shaders(&mut self) {
        self.stage_shaders.receive(&mut self.pipelines);
        self.stage_shaders.after_frame();
    }

    /// The line of the report on the time of the pipelines' making ([`MakingTimes::report`]).
    pub(crate) fn making_report(&self) -> String {
        self.maker.times.report()
    }
}

/// What a [`PipelineMaker`] makes, for its times.
#[derive(Clone, Copy)]
pub(crate) enum Making {
    /// A fragment module of a stage key or a world variant.
    Module,
    /// The pipeline of a key with a stage key.
    StagePipeline,
    /// A pipeline of the world shader's general text or of a world variant.
    GeneralPipeline,
}

/// The times of the work of a [`PipelineMaker`] and its clones, by what it makes and by thread:
/// the frame's thread, or the thread of the stage pipelines ([`crate::stage_shaders::THREAD_NAME`]).
/// For the gate of W530: on Vulkan, a pipeline whose module declares override constants is
/// written to SPIR-V again for each pipeline.
#[derive(Default)]
pub(crate) struct MakingTimes {
    count: [[AtomicU64; 2]; 3],
    ticks: [[AtomicU64; 2]; 3],
    largest: [[AtomicU64; 2]; 3],
}

impl MakingTimes {
    fn add(&self, making: Making, ticks: u64) {
        let thread = usize::from(std::thread::current().name() == Some(crate::stage_shaders::THREAD_NAME));
        let kind = making as usize;
        self.count[kind][thread].fetch_add(1, Ordering::Relaxed);
        self.ticks[kind][thread].fetch_add(ticks, Ordering::Relaxed);
        self.largest[kind][thread].fetch_max(ticks, Ordering::Relaxed);
    }

    /// "pipeline making, in ms: modules: the frame's thread N (sum S, largest L), the stage thread
    /// ...; stage pipelines: ...; general pipelines: ...".
    pub(crate) fn report(&self) -> String {
        let ms = |ticks: &AtomicU64| ticks::duration(ticks.load(Ordering::Relaxed)).as_secs_f64() * 1000.0;
        let class = |kind: usize, thread: usize| {
            format!(
                "{} (sum {:.3}, largest {:.3})",
                self.count[kind][thread].load(Ordering::Relaxed),
                ms(&self.ticks[kind][thread]),
                ms(&self.largest[kind][thread])
            )
        };
        let kinds: Vec<String> = ["modules", "stage pipelines", "general pipelines"]
            .iter()
            .enumerate()
            .map(|(kind, name)| {
                format!(
                    "{name}: the frame's thread {}, the stage thread {}",
                    class(kind, 0),
                    class(kind, 1)
                )
            })
            .collect();
        format!("pipeline making, in ms: {}", kinds.join("; "))
    }
}

/// The world shader's text with the extension's text, in the form of the device's texture tier.
fn world_source_of(extension_text: &str, tier: request::TextureTier) -> String {
    let source = request::world_shader_source(extension_text);
    if tier == request::TextureTier::C {
        request::tier_c_shader_source(&source)
    } else {
        source
    }
}

/// The world shader's module, made under [`gpu::guarded`], and its error.
fn world_module(device: &wgpu::Device, source: &str) -> (wgpu::ShaderModule, Option<String>) {
    gpu::guarded(device, || {
        device.counted_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader.wgsl"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        })
    })
}

/// Where the world shader's text stands (W317, [`Renderer::settle_world_shader`]).
#[derive(Default)]
pub(crate) struct WorldShader {
    /// The text of the extension that the world shader has: 0 the text of the start or of the
    /// extension's last change, n the n-th text of [`Extension::shader_fallback`].
    attempt: u32,
    /// The text held, and the extension heard it; or the extension had no other text.
    held: bool,
    /// The first error of the world shader's module or of a pipeline since the last switch.
    fault: Option<String>,
    /// The key of the pipeline of `fault`; None for an error of the module, which the first world
    /// key stands for.
    fault_key: Option<PipelineKey>,
    /// The key whose fault started the walk of the extension's texts: each text of the walk holds
    /// only when this key builds with it too.
    trial_key: Option<PipelineKey>,
    /// The extension had no other text, or the fault is no core mod's: a pipeline that fails is
    /// kept as it is, as before W317.
    exhausted: bool,
    /// A live session wants the thread of the stage pipelines ([`Renderer::start_stage_shaders`]).
    stages_wanted: bool,
    /// The thread of the stage pipelines of the text now runs.
    stages_running: bool,
}

/// What a pipeline of the world needs besides its key: the device, the layout, the vertex
/// module and the formats of the targets. A clone goes to the thread of the stage shaders
/// (`crate::stage_shaders`).
#[derive(Clone)]
pub(crate) struct PipelineMaker {
    device: wgpu::Device,
    layout: wgpu::PipelineLayout,
    vertex: wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
    wireframe: bool,
    /// The world shader's text has [`crate::CORE_MOD_FAULT_PLANT`]: each pipeline names a fragment
    /// entry point that the module lacks, so that it fails at the pipeline as a core mod that
    /// fails in the driver does (W317's test).
    planted: bool,
    /// `MGE_XE_CORE_MOD_FAULT=world key`: the pipeline of this key fails in the same way whatever
    /// the text, a fault that is no core mod's (the null of the probe of
    /// [`Renderer::settle_world_shader`]).
    fault_key: Option<PipelineKey>,
    /// The times of the work of this maker and its clones.
    times: std::sync::Arc<MakingTimes>,
}

impl PipelineMaker {
    /// A fragment module of a stage key or a world variant, or None when the device refuses it.
    /// `label` names the module and starts the line of its error.
    pub(crate) fn module(&self, label: &str, source: &str) -> Option<wgpu::ShaderModule> {
        let clock = ticks::Clock::now();
        let (module, error) = crate::gpu::guarded(&self.device, || {
            self.device.counted_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        });
        self.times.add(Making::Module, clock.elapsed());
        match error {
            None => Some(module),
            Some(error) => {
                println!("{label}: a fragment module failed: {error}");
                None
            }
        }
    }

    /// The pipeline of a key with a fragment module of a stage part and the constants of the key's
    /// facts ([`crate::stage_shaders::StageKey::constants`]), or None when it fails. The keys of the
    /// uber set (`stage_shaders::UBER_SET`) come here too, with the general text and no constants.
    pub(crate) fn make(
        &self,
        key: PipelineKey,
        fragment: &wgpu::ShaderModule,
        constants: &[(&str, f64)],
    ) -> Option<wgpu::RenderPipeline> {
        let making = match key.stages {
            0 => Making::GeneralPipeline,
            _ => Making::StagePipeline,
        };
        match self.guarded_build(key, fragment, constants, making) {
            (pipeline, None) => pipeline,
            (_, Some(error)) => {
                println!("stage shaders: a pipeline failed: {error}");
                None
            }
        }
    }

    /// [`PipelineMaker::build`] under the error scopes of [`crate::gpu::guarded`], its time added
    /// to `making`: the pipeline, or None for a key that has no pipeline, and the error of the
    /// device. A pipeline with an error is invalid: each draw with it fails.
    pub(crate) fn guarded_build(
        &self,
        key: PipelineKey,
        fragment: &wgpu::ShaderModule,
        constants: &[(&str, f64)],
        making: Making,
    ) -> (Option<wgpu::RenderPipeline>, Option<String>) {
        let clock = ticks::Clock::now();
        let built = crate::gpu::guarded(&self.device, || self.build(key, fragment, constants));
        self.times.add(making, clock.elapsed());
        built
    }

    /// The pipeline of a key with a world variant's text, made at its first draw on the frame's
    /// thread. An error goes to the device's handler, as the numbered GPU errors.
    fn general(&self, key: PipelineKey, fragment: &wgpu::ShaderModule) -> Option<wgpu::RenderPipeline> {
        let clock = ticks::Clock::now();
        let pipeline = self.build(key, fragment, &[]);
        self.times.add(Making::GeneralPipeline, clock.elapsed());
        pipeline
    }

    /// The pipeline of a key, with the world shader's `vs` and the `fs` of `fragment`. The
    /// constants go to the fragment stage only: the vertex stage is the general module, which
    /// declares no override (the Architect's section 6.2).
    fn build(
        &self,
        key: PipelineKey,
        fragment: &wgpu::ShaderModule,
        constants: &[(&str, f64)],
    ) -> Option<wgpu::RenderPipeline> {
        let present = attributes(key.fvf, key.stride)?;
        let topology = match key.topology {
            1 => wgpu::PrimitiveTopology::PointList,
            2 => wgpu::PrimitiveTopology::LineList,
            3 => wgpu::PrimitiveTopology::LineStrip,
            4 => wgpu::PrimitiveTopology::TriangleList,
            5 => wgpu::PrimitiveTopology::TriangleStrip,
            _ => return None,
        };
        let from_vertex: Vec<wgpu::VertexAttribute> = present
            .iter()
            .map(|a| wgpu::VertexAttribute {
                format: a.format,
                offset: a.offset,
                shader_location: a.location,
            })
            .collect();
        // An input that the vertex does not have comes from a buffer of zeros.
        let from_defaults: Vec<wgpu::VertexAttribute> = (0..INPUT_FORMATS.len() as u32)
            .filter(|location| !present.iter().any(|a| a.location == *location))
            .map(|location| wgpu::VertexAttribute {
                format: INPUT_FORMATS[location as usize],
                offset: 0,
                shader_location: location,
            })
            .collect();
        // The world matrix of an instance: four columns at the locations 9 to 12; the
        // record of the draw at 13.
        let mut from_instances: Vec<wgpu::VertexAttribute> = (0..4)
            .map(|column| wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: column as u64 * 16,
                shader_location: 9 + column,
            })
            .collect();
        from_instances.push(wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Uint32x2,
            offset: 64,
            shader_location: 13,
        });
        let buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: key.stride as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &from_vertex,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: 64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &from_defaults,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: INSTANCE_SIZE as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &from_instances,
            }),
        ];
        let blend = (key.blend[0] != 0).then(|| {
            let component = wgpu::BlendComponent {
                src_factor: blend_factor(key.blend[1]),
                dst_factor: blend_factor(key.blend[2]),
                operation: wgpu::BlendOperation::Add,
            };
            wgpu::BlendState {
                color: component,
                alpha: component,
            }
        });
        // Direct3D culls the faces with this winding. A front face of the game is clockwise
        // [I: the cull values come from the cull table of the render state, ctor 0x6B7D70,
        // by transcription].
        let cull_mode = match key.cull {
            2 => Some(wgpu::Face::Front),
            3 => Some(wgpu::Face::Back),
            _ => None,
        };
        let depth_on = key.depth[0] != 0;
        let pipeline = self.device.counted_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("game draw"),
            layout: Some(&self.layout),
            vertex: wgpu::VertexState {
                module: &self.vertex,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology,
                front_face: wgpu::FrontFace::Cw,
                cull_mode,
                polygon_mode: if key.fill == 2 && self.wireframe {
                    wgpu::PolygonMode::Line
                } else {
                    wgpu::PolygonMode::Fill
                },
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: self.depth_format,
                depth_write_enabled: Some(depth_on && key.depth[1] != 0),
                depth_compare: Some(if depth_on {
                    depth::compare(compare(key.depth[2]))
                } else {
                    wgpu::CompareFunction::Always
                }),
                stencil: if key.stencil[0] == 0 {
                    Default::default()
                } else {
                    // Direct3D 8 has one stencil rule for the two sides of a triangle.
                    let face = wgpu::StencilFaceState {
                        compare: compare(key.stencil[4]),
                        fail_op: stencil_operation(key.stencil[1]),
                        depth_fail_op: stencil_operation(key.stencil[2]),
                        pass_op: stencil_operation(key.stencil[3]),
                    };
                    wgpu::StencilState {
                        front: face,
                        back: face,
                        read_mask: key.stencil[5] & 0xFF,
                        write_mask: key.stencil[6] & 0xFF,
                    }
                },
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: key.samples,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: fragment,
                entry_point: Some(if self.planted || self.fault_key == Some(key) {
                    "fs_core_mod_fault_plant"
                } else {
                    "fs"
                }),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants,
                    ..Default::default()
                },
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.target_format,
                    blend,
                    write_mask: wgpu::ColorWrites::from_bits_truncate(key.color_write),
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Some(pipeline)
    }
}

/// The bytes of a 2D texture or texture array as its format packs them: each level of each
/// layer in whole blocks, for each sample. 0 for a format with no one block size (a depth with
/// a stencil).
pub(crate) fn texture_bytes(
    width: u32,
    height: u32,
    layers: u32,
    levels: u32,
    samples: u32,
    format: wgpu::TextureFormat,
) -> u64 {
    let Some(block_bytes) = format.block_copy_size(None) else {
        return 0;
    };
    let (block_width, block_height) = format.block_dimensions();
    let level_bytes = |level: u32| {
        let blocks_across = (width >> level).max(1).div_ceil(block_width) as u64;
        let blocks_down = (height >> level).max(1).div_ceil(block_height) as u64;
        blocks_across * blocks_down * block_bytes as u64
    };
    (0..levels).map(level_bytes).sum::<u64>() * layers as u64 * samples as u64
}

/// The bytes of a texture of the helper ([`texture_bytes`]).
fn bytes_of(texture: &wgpu::Texture) -> u64 {
    let size = texture.size();
    texture_bytes(
        size.width,
        size.height,
        size.depth_or_array_layers,
        texture.mip_level_count(),
        texture.sample_count(),
        texture.format(),
    )
}

/// The changes of the texture arrays that drop their groups ([`Renderer::forget_array_groups`]),
/// by their names in the count of the GPU objects.
pub(crate) mod array_change {
    pub(crate) const LAID_OUT: &str = "the texture arrays laid out";
    pub(crate) const TEXTURE_REPLACED: &str = "a texture of the arrays replaced";
    pub(crate) const TEXTURE_IN: &str = "a texture into the arrays";
    pub(crate) const SAMPLER_IN: &str = "a sampler state into the arrays";
    pub(crate) const TEXTURE_OUT: &str = "a texture out of the arrays";
    pub(crate) const PINNED_IN: &str = "a pinned texture into the arrays";
    pub(crate) const PINNED_OUT: &str = "a pinned texture out of the arrays";
    pub(crate) const PINNED_SAMPLER_IN: &str = "a pinned sampler state into the arrays";
}

#[cfg(test)]
mod array_change_tests {
    use super::*;

    /// A sampler state new to the arrays drops their groups and marks the frame as changed. The
    /// null: the same state again has its slot, and marks nothing. Needs a GPU, and no other
    /// test that notes changes at the same time:
    /// `cargo test -p nirender-host -- --ignored sampler_state --test-threads=1`.
    #[test]
    #[ignore = "needs a GPU"]
    fn a_sampler_state_new_to_the_arrays_marks_its_frame_as_changed() {
        let (mut renderer, _) = Renderer::new(
            wgpu::Backends::all(),
            None,
            wgpu::TextureFormat::Rgba8Unorm,
            crate::extension::extension_shaders(None),
            0,
            1,
        )
        .expect("a device");
        assert_ne!(renderer.tier, request::TextureTier::C, "tier C has no texture arrays");
        renderer.begin_frame_arrays();
        crate::objects::take_noted_change();

        let states = material::sampler_states(1, 1);
        let slot = renderer.frame_sampler_slot(states);
        assert_eq!(crate::objects::take_noted_change(), Some(array_change::SAMPLER_IN));

        assert_eq!(renderer.frame_sampler_slot(states), slot);
        assert_eq!(crate::objects::take_noted_change(), None);
    }
}

#[cfg(test)]
mod world_fallback_tests {
    use super::*;
    use crate::extension::{PLAIN_DRAW, PLAIN_SKY};

    /// An extension with a list of fallback texts and a built-in text, which notes what the
    /// renderer asked and heard.
    #[derive(Default)]
    struct Texts {
        fallbacks: Vec<ShaderText>,
        built_in: Option<ShaderText>,
        asked: Vec<u32>,
        building: Vec<u32>,
        held: Vec<u32>,
        kept: Vec<String>,
    }

    impl Extension for Texts {
        fn before_world(&mut self, _: &SceneView, _: &mut wgpu::RenderPass<'static>, _: &mut dyn Replay) -> bool {
            false
        }

        fn shader_fallback(&mut self, attempt: u32, _error: &str) -> Option<ShaderText> {
            self.asked.push(attempt);
            self.fallbacks.get(attempt as usize - 1).cloned()
        }

        fn world_shader_held(&mut self, attempt: u32) {
            self.held.push(attempt);
        }

        fn world_shader_building(&mut self, attempt: u32) {
            self.building.push(attempt);
        }

        fn built_in_text(&mut self) -> Option<ShaderText> {
            self.built_in.clone()
        }

        fn world_shader_kept(&mut self, error: &str) {
            self.kept.push(error.to_string());
        }
    }

    /// A text with the plant: each of its world pipelines fails on the device.
    fn planted(label: &str) -> ShaderText {
        ShaderText {
            sky: Some(format!("{CORE_MOD_FAULT_PLANT}\n{PLAIN_SKY}")),
            draw: Some(PLAIN_DRAW.to_string()),
            label: label.to_string(),
        }
    }

    fn plain(label: &str) -> ShaderText {
        ShaderText {
            label: label.to_string(),
            ..ShaderText::default()
        }
    }

    fn renderer_of(text: &ShaderText) -> Renderer {
        let (renderer, _) = Renderer::new(
            wgpu::Backends::all(),
            None,
            wgpu::TextureFormat::Rgba8Unorm,
            text.extension_text(),
            0,
            1,
        )
        .expect("a device");
        renderer
    }

    /// The plant fails a pipeline with an error of the device that the guard takes; the null: the
    /// same text without the plant makes the pipeline. Needs a GPU:
    /// `cargo test -p nirender-host -- --ignored world_fallback --test-threads=1`.
    #[test]
    #[ignore = "needs a GPU"]
    fn world_fallback_the_guard_takes_a_failed_world_pipeline() {
        let failed = renderer_of(&planted("planted")).hold_test();
        assert!(
            failed
                .as_ref()
                .is_some_and(|(error, _)| error.contains("fs_core_mod_fault_plant")),
            "{failed:?}"
        );
        assert_eq!(renderer_of(&plain("plain")).hold_test(), None);
    }

    /// A text holds only when the key that started the walk builds with it too. The null: with no
    /// key of a walk, the same renderer holds on the first world key.
    #[test]
    #[ignore = "needs a GPU"]
    fn world_fallback_a_text_holds_only_when_the_key_of_the_walk_builds() {
        let mut renderer = renderer_of(&plain("plain"));
        let first = crate::stage_shaders::first_world_key(renderer.samples);
        let other = PipelineKey { color_write: 0, ..first };
        renderer.maker.fault_key = Some(other);
        assert_eq!(renderer.hold_test(), None);
        renderer.world.trial_key = Some(other);
        assert!(renderer.hold_test().is_some_and(|(_, key)| key == other));
    }

    /// A key that fails with the built-in text too is no core mod's fault: the extension hears it,
    /// no text is asked for, and the failed key is kept. The null: the same plant of a text (a
    /// core mod's fault) passes the probe, and the walk drops the text.
    #[test]
    #[ignore = "needs a GPU"]
    fn world_fallback_a_fault_of_the_built_in_text_too_drops_nothing() {
        let mut renderer = renderer_of(&plain("start"));
        let first = crate::stage_shaders::first_world_key(renderer.samples);
        renderer.maker.fault_key = Some(first);
        let mut extension = Texts {
            fallbacks: vec![plain("without A")],
            built_in: Some(plain("built in")),
            ..Texts::default()
        };
        renderer.settle_world_shader(Some(&mut extension));
        assert_eq!((extension.asked.as_slice(), extension.held.as_slice()), (&[][..], &[][..]));
        assert_eq!(extension.kept.len(), 1, "{:?}", extension.kept);
        assert!(renderer.world.exhausted && renderer.world.held);
        assert!(renderer.pipeline(first).is_some(), "the failed pipeline is kept");

        let mut renderer = renderer_of(&planted("start"));
        let mut extension = Texts {
            fallbacks: vec![plain("without A")],
            built_in: Some(plain("built in")),
            ..Texts::default()
        };
        renderer.settle_world_shader(Some(&mut extension));
        assert_eq!((extension.asked.as_slice(), extension.held.as_slice()), (&[1][..], &[1][..]));
        assert!(extension.kept.is_empty());
    }

    /// While a fault waits for the settle, no other world pipeline is built: the fault keeps the
    /// key of the first build that failed. The null: after the settle the other key builds.
    #[test]
    #[ignore = "needs a GPU"]
    fn world_fallback_no_pipeline_is_built_while_a_fault_waits() {
        let mut renderer = renderer_of(&planted("start"));
        let first = crate::stage_shaders::first_world_key(renderer.samples);
        let other = PipelineKey { color_write: 0, ..first };
        assert!(renderer.pipeline(first).is_none());
        assert!(renderer.pipeline(other).is_none());
        assert_eq!(renderer.world.fault_key, Some(first));
        let mut extension = Texts {
            fallbacks: vec![plain("all built in")],
            ..Texts::default()
        };
        renderer.settle_world_shader(Some(&mut extension));
        assert!(renderer.pipeline(other).is_some());
    }

    /// With no extension the first settle keeps a failed key as it is, as before W317.
    #[test]
    #[ignore = "needs a GPU"]
    fn world_fallback_with_no_extension_keeps_the_failed_pipeline() {
        let mut renderer = renderer_of(&planted("start"));
        renderer.settle_world_shader(None);
        assert!(renderer.world.exhausted && renderer.world.held);
        let first = crate::stage_shaders::first_world_key(renderer.samples);
        assert!(renderer.pipeline(first).is_some(), "the failed pipeline is kept");
        assert!(renderer.pipelines.contains_key(&first));
    }

    /// A text that fails gives way to the extension's next ones until one holds; the extension
    /// hears which. The null: a text that holds asks for none.
    #[test]
    #[ignore = "needs a GPU"]
    fn world_fallback_tries_the_texts_until_one_holds() {
        let mut renderer = renderer_of(&planted("start"));
        let mut extension = Texts {
            fallbacks: vec![planted("without A"), plain("without B"), plain("all built in")],
            ..Texts::default()
        };
        renderer.settle_world_shader(Some(&mut extension));
        assert_eq!(
            (extension.asked.as_slice(), extension.held.as_slice()),
            (&[1, 2][..], &[2][..])
        );
        assert_eq!(extension.building, [0, 1, 2]);
        assert!(renderer.world.held && !renderer.world.exhausted && !renderer.maker.planted);
        // A later fault starts its own walk.
        assert_eq!(renderer.world.attempt, 0);
        let key = crate::stage_shaders::first_world_key(renderer.samples);
        assert!(renderer.pipelines.contains_key(&key));
        // A settled text costs nothing more.
        renderer.settle_world_shader(Some(&mut extension));
        assert_eq!(extension.asked, [1, 2]);

        let mut renderer = renderer_of(&plain("start"));
        let mut extension = Texts::default();
        renderer.settle_world_shader(Some(&mut extension));
        assert_eq!((extension.asked.as_slice(), extension.held.as_slice()), (&[][..], &[0][..]));
    }

    /// With no text left, the renderer keeps a failed pipeline as before W317 and asks no more.
    #[test]
    #[ignore = "needs a GPU"]
    fn world_fallback_with_no_text_left_keeps_the_failed_pipeline() {
        let mut renderer = renderer_of(&planted("start"));
        let mut extension = Texts::default();
        renderer.settle_world_shader(Some(&mut extension));
        assert_eq!((extension.asked.as_slice(), extension.held.as_slice()), (&[1][..], &[][..]));
        assert!(renderer.world.exhausted && renderer.world.held);
        let key = crate::stage_shaders::first_world_key(renderer.samples);
        assert!(renderer.pipeline(key).is_some(), "the failed pipeline is kept");
        renderer.settle_world_shader(Some(&mut extension));
        assert_eq!(extension.asked, [1]);
    }

    /// A key that fails while the extension may still give a text draws nothing and waits for the
    /// next frame's switch.
    #[test]
    #[ignore = "needs a GPU"]
    fn world_fallback_a_failed_key_waits_for_the_switch() {
        let mut renderer = renderer_of(&planted("start"));
        let key = crate::stage_shaders::first_world_key(renderer.samples);
        assert!(renderer.pipeline(key).is_none());
        assert!(renderer.world.fault.is_some());
        let mut extension = Texts {
            fallbacks: vec![plain("all built in")],
            ..Texts::default()
        };
        renderer.settle_world_shader(Some(&mut extension));
        assert_eq!(extension.held, [1]);
        assert!(renderer.pipeline(key).is_some());
    }

    #[test]
    fn the_text_of_an_extension_without_functions_is_the_plain_text() {
        assert_eq!(ShaderText::default().extension_text(), format!("{PLAIN_SKY}\n{PLAIN_DRAW}\n"));
        assert_eq!(
            crate::extension::extension_shaders(None),
            ShaderText::default().extension_text()
        );
    }
}

#[cfg(test)]
mod held_tests {
    use super::texture_bytes;
    use crate::wgpu::TextureFormat;

    #[test]
    fn held_bytes_of_a_plain_texture_with_its_levels() {
        // 4x4 RGBA8: 64 bytes, then 2x2 16 and 1x1 4.
        assert_eq!(texture_bytes(4, 4, 1, 3, 1, TextureFormat::Rgba8Unorm), 84);
    }

    #[test]
    fn held_bytes_of_a_block_format_count_whole_blocks() {
        // BC1: 8 bytes a 4x4 block. 8x8 is 4 blocks, 4x4 one, and 2x2 and 1x1 one each.
        assert_eq!(texture_bytes(8, 8, 1, 4, 1, TextureFormat::Bc1RgbaUnorm), 32 + 8 + 8 + 8);
        // BC7: 16 bytes a block; a 6x6 texture takes 2x2 blocks.
        assert_eq!(texture_bytes(6, 6, 1, 1, 1, TextureFormat::Bc7RgbaUnorm), 64);
    }

    #[test]
    fn held_bytes_of_layers_and_samples() {
        assert_eq!(texture_bytes(4, 4, 6, 1, 1, TextureFormat::Rgba8Unorm), 6 * 64);
        assert_eq!(texture_bytes(4, 4, 1, 1, 4, TextureFormat::Rgba8Unorm), 4 * 64);
    }

    #[test]
    fn held_bytes_of_a_depth_with_a_stencil_are_none() {
        assert_eq!(texture_bytes(4, 4, 1, 1, 1, TextureFormat::Depth24PlusStencil8), 0);
    }
}

#[cfg(test)]
mod making_tests {
    use super::{Making, MakingTimes};

    /// Each time goes to its kind and thread: the thread of the stage pipelines by its name, any
    /// other thread is the frame's. The null: a thread with another name counts as the frame's.
    #[test]
    fn a_time_goes_to_its_kind_and_thread() {
        let times = std::sync::Arc::new(MakingTimes::default());
        times.add(Making::GeneralPipeline, 5);
        for name in [crate::stage_shaders::THREAD_NAME, "another thread"] {
            let shared = times.clone();
            std::thread::Builder::new()
                .name(name.into())
                .spawn(move || shared.add(Making::StagePipeline, 7))
                .unwrap()
                .join()
                .unwrap();
        }
        let line = times.report();
        assert!(
            line.starts_with(
                "pipeline making, in ms: modules: the frame's thread 0 (sum 0.000, largest 0.000), the stage thread 0 "
            ),
            "{line}"
        );
        assert!(line.contains("; stage pipelines: the frame's thread 1 ("), "{line}");
        assert!(line.contains("), the stage thread 1 ("), "{line}");
        assert!(line.contains("; general pipelines: the frame's thread 1 ("), "{line}");
        assert!(line.ends_with("), the stage thread 0 (sum 0.000, largest 0.000)"), "{line}");
    }
}

#[cfg(test)]
mod link_decode_tests {
    use super::*;

    const DXT1: u32 = 0x3154_5844;

    /// The bytes of level 0 of a DXT1 texture of this size.
    fn level_bytes(width: u32, height: u32) -> usize {
        width.div_ceil(4) as usize * height.div_ceil(4) as usize * 8
    }

    /// A TEXTURE record of the link with one DXT1 level of zero bytes.
    fn texture_record(handle: u32, width: u32, height: u32) -> Vec<u8> {
        let size = level_bytes(width, height) as u32;
        let words = [handle, 0, width, height, 1, DXT1, 0, 0, 0, 0, size, width, height];
        let mut data: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        data.resize(data.len() + size as usize, 0);
        data
    }

    /// A TEXTURE_UPDATE record of level 0 with zero bytes.
    fn update_record(handle: u32, width: u32, height: u32) -> Vec<u8> {
        let size = level_bytes(width, height) as u32;
        let words = [handle, 0, width, height, DXT1, size];
        let mut data: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        data.resize(data.len() + size as usize, 0);
        data
    }

    fn renderer() -> Renderer {
        let (renderer, _) = Renderer::new(
            wgpu::Backends::all(),
            None,
            wgpu::TextureFormat::Rgba8Unorm,
            crate::extension::extension_shaders(None),
            0,
            1,
        )
        .expect("a device");
        renderer
    }

    /// Sends the update and gives the validation error of the device, if there is one.
    fn update(renderer: &mut Renderer, handle: u32, width: u32, height: u32) -> Option<wgpu::Error> {
        let scope = renderer.device.push_error_scope(wgpu::ErrorFilter::Validation);
        renderer.texture_update(&update_record(handle, width, height));
        pollster::block_on(scope.pop())
    }

    /// W632, the Architect's W2: a texture that takes a chain shows its view of level 0 until its
    /// job is done; an update of its level 0 shows the view of level 0 again, from that frame
    /// until the new job is done, and a chain of the old job is stale. Needs a GPU:
    /// `cargo test -p nirender-host -- --ignored chain_view --test-threads=1`.
    #[test]
    #[ignore = "needs a GPU"]
    fn chain_view_an_update_shows_level_0_until_its_new_chain() {
        let mut renderer = renderer();
        renderer.texture(&texture_record(9, 8, 8));
        let shows = |r: &Renderer, view: &Rc<wgpu::TextureView>| Rc::ptr_eq(&r.textures[&9], view);
        assert!(renderer.chains[&9].full.is_none(), "a full view before the job is done");
        let level0 = renderer.textures[&9].clone();
        renderer.wait_for_chains = true;
        renderer.take_chains();
        let full = renderer.chains[&9].full.clone().expect("the chain is in");
        assert!(shows(&renderer, &full) && !shows(&renderer, &level0));
        assert_eq!(renderer.textures[&9].texture().mip_level_count(), 4);
        // An update of level 0: the view of level 0 again, and a new job.
        assert!(update(&mut renderer, 9, 8, 8).is_none());
        assert!(renderer.chains[&9].full.is_none() && !shows(&renderer, &full));
        // Two updates before a take: the first job's chain is stale.
        assert!(update(&mut renderer, 9, 8, 8).is_none());
        renderer.take_chains();
        let c = &renderer.counts;
        assert_eq!((c.chain_jobs, c.chain_swaps, c.chain_stale, c.chain_updates), (3, 2, 1, 2));
        assert!(renderer.chains[&9].full.as_ref().is_some_and(|full| shows(&renderer, full)));
    }

    /// The bytes of one level of a block texture, read back: `rows` rows of blocks of `row` bytes.
    fn read_level(renderer: &Renderer, texture: &wgpu::Texture, level: u32, row: u32, rows: u32, extent: u32) -> Vec<u8> {
        const ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = row.div_ceil(ALIGN) * ALIGN;
        let buffer = renderer.device.counted_buffer(&wgpu::BufferDescriptor {
            label: Some("a test's level"),
            size: (padded * rows) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = renderer.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(rows),
                },
            },
            wgpu::Extent3d {
                width: extent,
                height: extent,
                depth_or_array_layers: 1,
            },
        );
        renderer.queue.submit([encoder.finish()]);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |result| result.expect("the read of a level"));
        let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
        let mapped = buffer.slice(..).get_mapped_range().expect("the bytes of a level");
        (0..rows as usize)
            .flat_map(|r| mapped[r * padded as usize..r * padded as usize + row as usize].to_vec())
            .collect()
    }

    /// The Architect's check of W632: the chain leaves level 0 as the record has it, byte for byte,
    /// and its level 1 decodes near the 2 x 2 mean of level 0. A DXT5 texture of 8 x 8 with one
    /// level: a grey ramp with an alpha ramp, encoded. The null: a level 0 that the chain wrote
    /// again, or in another form, differs in its bytes. Needs a GPU:
    /// `cargo test -p nirender-host -- --ignored chain_view --test-threads=1`.
    #[test]
    #[ignore = "needs a GPU"]
    fn chain_view_level_0_stays_the_records() {
        const DXT5: u32 = 0x3554_5844;
        let pixels: Vec<u8> = (0..64usize)
            .flat_map(|i| {
                // A grey ramp (colours on one line, as a DXT block fits them) and an alpha ramp, both
                // gentle: a block's 4 colours then step less than the bound below.
                let (x, y) = ((i % 8) as u8, (i / 8) as u8);
                let grey = 60 + (x + y) * 3;
                [grey, grey, grey, 255 - x * 5]
            })
            .collect();
        let level0 = crate::mip_chain::encode(DXT5, 8, 8, &pixels);
        assert_eq!(level0.len(), 64);
        let words = [9, 0, 8, 8, 1, DXT5, 0, 0, 0, 0, 64, 8, 8];
        let mut record: Vec<u8> = words.iter().flat_map(|word: &u32| word.to_le_bytes()).collect();
        record.extend_from_slice(&level0);
        let mut renderer = renderer();
        renderer.texture(&record);
        renderer.wait_for_chains = true;
        renderer.take_chains();
        assert!(renderer.chains[&9].full.is_some(), "the chain is in");
        let texture = renderer.texture_objects[&9].0.clone();
        // Level 0: 2 rows of 2 blocks of 16 bytes.
        assert_eq!(read_level(&renderer, &texture, 0, 32, 2, 8), level0);
        // Level 1: 1 block for 4 x 4 pixels: the worker's bytes, and near the 2 x 2 mean of level
        // 0's decoded pixels.
        let level1 = read_level(&renderer, &texture, 1, 16, 1, 4);
        assert_eq!(level1, crate::mip_chain::levels_below(DXT5, false, 8, 8, &level0)[0]);
        let source = crate::d3d::decode_dxt(DXT5, 8, 8, &level0);
        let made = crate::d3d::decode_dxt(DXT5, 4, 4, &level1);
        let mut most = 0i32;
        for y in 0..4 {
            for x in 0..4 {
                for c in 0..4 {
                    let at = |px: usize, py: usize| source[(py * 8 + px) * 4 + c] as i32;
                    let mean =
                        (at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1) + 2) / 4;
                    most = most.max((made[(y * 4 + x) * 4 + c] as i32 - mean).abs());
                }
            }
        }
        // The bound of the encoder's plane test (mip_chain::tests::the_blocks_decode_near_their_pixels).
        assert!(most <= 10, "level 1 is {most} levels from the 2 x 2 mean");
        println!("level 1: the largest difference from the 2 x 2 mean, {most} levels");
    }

    /// The switch off: a texture with one level keeps its one level, with no job.
    #[test]
    #[ignore = "needs a GPU"]
    fn chain_view_the_switch_off_keeps_one_level() {
        let mut renderer = renderer();
        renderer.mipmaps_for_one_level = false;
        renderer.texture(&texture_record(9, 8, 8));
        assert!(!renderer.chains.contains_key(&9) && renderer.chain_worker.is_none());
        assert_eq!(renderer.textures[&9].texture().mip_level_count(), 1);
    }

    /// A link texture that this program decodes is marked, so that its update is decoded too and
    /// is not lost. The null: without the mark, the update goes as DXT1 blocks into the decoded
    /// texture and the mark check fails. Needs a GPU:
    /// `cargo test -p nirender-host -- --ignored link_decode --test-threads=1`.
    #[test]
    #[ignore = "needs a GPU"]
    fn link_decode_an_update_of_a_decoded_link_texture_is_decoded() {
        let mut renderer = renderer();
        renderer.texture(&texture_record(7, 6, 6));
        assert!(renderer.decoded.contains(&7), "the 6 x 6 DXT1 texture has no decode mark");
        let error = update(&mut renderer, 7, 6, 6);
        assert!(error.is_none(), "{error:?}");
        assert_eq!(
            (renderer.counts.texture_updates, renderer.counts.texture_updates_lost),
            (1, 0)
        );
    }

    /// The same handle made again with a size that the GPU takes as blocks loses its mark, so its
    /// update goes as blocks.
    #[test]
    #[ignore = "needs a GPU"]
    fn link_decode_a_handle_made_again_as_blocks_loses_its_mark() {
        let mut renderer = renderer();
        renderer.texture(&texture_record(7, 6, 6));
        renderer.texture(&texture_record(7, 8, 8));
        assert!(!renderer.decoded.contains(&7), "the 8 x 8 DXT1 texture keeps the decode mark");
        let error = update(&mut renderer, 7, 8, 8);
        assert!(error.is_none(), "{error:?}");
        assert_eq!(
            (renderer.counts.texture_updates, renderer.counts.texture_updates_lost),
            (1, 0)
        );
    }
}
