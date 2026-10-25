//! The methods of [`Renderer`]: the device, the pipelines, the textures and the samplers, the
//! buffers of the frame and of the retained instances, and the helpers of its buffers. The
//! struct is in lib.rs.

use super::*;

/// The bind group of the frame: the resources of the extension, the clusters of the lights,
/// the lights and the bones of the frame, and the buffers of the retained instances.
pub(crate) fn frame_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, uniform: &wgpu::Buffer, sampler: &wgpu::Sampler, texture: &wgpu::TextureView, clusters: &Clusters, lights: &wgpu::Buffer, bones: &wgpu::Buffer, records: &wgpu::Buffer, light_indexes: &wgpu::Buffer, scenes: &wgpu::Buffer, retained_lights: &wgpu::Buffer, retained_bones: &wgpu::Buffer) -> wgpu::BindGroup {
    crate::gpu::frame_group(device, &wgpu::BindGroupDescriptor {
        label: Some("extension frame"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(texture) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 3, resource: clusters.uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: clusters.table.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 5, resource: clusters.indexes.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 6, resource: lights.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 7, resource: bones.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 8, resource: records.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 9, resource: light_indexes.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: scenes, offset: 0, size: core::num::NonZeroU64::new(SCENE_SIZE as u64) }) },
            wgpu::BindGroupEntry { binding: 11, resource: retained_lights.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 12, resource: retained_bones.as_entire_binding() },
        ],
    })
}

/// The buffer of the arguments of the indirect draws of a frame.
pub(crate) fn indirect_buffer(device: &wgpu::Device, draws: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor { label: Some("indirect draws"), size: (draws * ARGS_SIZE) as u64, usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
}

/// Ends a run of draws: the arguments of the run go after those of the frame, and the pass
/// gets one indirect call for them. The arguments reach the GPU before the submit
/// (`upload_indirect`).
pub(crate) fn flush_run(pass: &mut wgpu::RenderPass<'static>, renderer: &mut Renderer, run_args: &mut Vec<IndirectArgs>, frame_args: &mut Vec<IndirectArgs>) {
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
        println!("light dump {path} {i}: type {} position {:?} diffuse {:?} ambient {:?} attenuation {:?} range {} radius {}", l.position[3], &l.position[..3], &l.diffuse[..3], &l.ambient[..3], &l.attenuation[..3], l.direction[3], l.cone[2]);
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
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("draw records"),
        size: (records * RECORD_SIZE) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The inputs a vertex does not have, zeros, one entry for each instance the instance buffer
/// can hold: a draw's instances index both buffers, so this one must be as long.
pub(crate) fn defaults_buffer(device: &wgpu::Device, instances: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor { label: Some("vertex defaults"), size: (instances * 64) as u64, usage: wgpu::BufferUsages::VERTEX, mapped_at_creation: false })
}

/// The instances of a frame.
pub(crate) fn instance_buffer(device: &wgpu::Device, instances: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
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
    let buffer = |usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some("timing"), size, usage, mapped_at_creation: false });
    let (sender, ready) = std::sync::mpsc::channel();
    let set = device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("timing"), ty: wgpu::QueryType::Timestamp, count: TIMING_QUERIES });
    let in_encoders = device.features().contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);
    let in_passes = device.features().contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES);
    let stamps = Rc::new(crate::stamps::Stamps::new(set.clone(), NAMED_FIRST, TIMING_NAMED, in_encoders, in_passes));
    Some(Timing {
        set,
        resolve: buffer(wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST),
        staging: (0..3).map(|_| buffer(wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST)).collect(),
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
    pub(crate) fn end(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, counts: &mut Counts) -> Option<usize> {
        let _ = device.poll(wgpu::PollType::Poll);
        while let Ok(slot) = self.ready.try_recv() {
            {
                let mapped = self.staging[slot].slice(..).get_mapped_range().expect("the time stamps");
                let stamp = |i: u32| u64::from_le_bytes(mapped[i as usize * 8..i as usize * 8 + 8].try_into().unwrap());
                let span = |i: u32| -> f64 {
                    let (begin, end) = (stamp(2 * i), stamp(2 * i + 1));
                    if begin != 0 && end > begin { (end - begin) as f64 * self.period as f64 } else { 0.0 }
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
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bones"),
        size: (bones * 64) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl Renderer {
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
                // A texture with one level gets no mip filter [I: the engine's filter for a
                // texture with one level was not read].
                if texture.texture().mip_level_count() <= 1 {
                    sampler[4] = 0;
                }
                Some(HeldMap { map: index, texture, set: map.set.min(sets.saturating_sub(1)), sampler })
            })
            .collect()
    }

    /// Makes the device. With a window, also the surface of the window, and the format of
    /// the target is a format of that surface.
    pub(crate) fn new(
        backends: wgpu::Backends,
        window: Option<std::sync::Arc<winit::window::Window>>,
        mut target_format: wgpu::TextureFormat,
        extension_shader: String,
        texture_slots: u32,
        samples: u32,
    ) -> (Renderer, Option<wgpu::Surface<'static>>) {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = backends;
        // The WGPU_* variables of the environment can change the flags, for a test (for
        // example WGPU_VALIDATION_INDIRECT_CALL=0).
        // The arguments of the indirect draws are this program's own: no validation of them
        // by a compute pass (`WGPU_VALIDATION_INDIRECT_CALL=1` puts it back).
        descriptor.flags.remove(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL);
        descriptor.flags = descriptor.flags.with_env();
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
        println!("adapter: {} ({:?}, driver {} {})", info.name, info.backend, info.driver, info.driver_info);
        let mut surface_usages = wgpu::TextureUsages::empty();
        if let Some(surface) = surface.as_ref() {
            // The game gives colours that are ready for the screen: no sRGB format.
            let capabilities = surface.get_capabilities(&adapter);
            if !capabilities.formats.contains(&target_format) {
                target_format = capabilities.formats.iter().copied().find(|format| !format.is_srgb()).unwrap_or(capabilities.formats[0]);
            }
            surface_usages = capabilities.usages;
            println!("window: the present modes of the surface are {:?}", capabilities.present_modes);
        }
        // The samples for each pixel of the targets of the frame: the largest count up to the
        // wanted one that the adapter has for the colour format, with a resolve, and for the
        // depth format.
        let mut samples = samples.clamp(1, 8).next_power_of_two();
        while samples > 1 {
            let color = adapter.get_texture_format_features(target_format).flags;
            let depth = adapter.get_texture_format_features(DEPTH_FORMAT).flags;
            if color.sample_count_supported(samples) && color.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE) && depth.sample_count_supported(samples) {
                break;
            }
            samples /= 2;
        }
        println!("multisampling: {samples} samples for each pixel of the frame");
        // The textures and the samplers of a frame are in binding arrays. The adapter must
        // have them; without them this program stops here with the reason, and the game
        // side is for Direct3D 9 (`[runtime] renderer_wgpu = false`).
        let array_features = wgpu::Features::TEXTURE_BINDING_ARRAY | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING | wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY;
        // The samplers of the array count in the elements of the arrays too.
        // The room of the texture arrays: the setting of the extension (0 automatic), the test
        // switch MGE_XE_ARRAY_CAP over it, at most the adapter's room.
        let wanted = switches::switches().array_cap.unwrap_or(if texture_slots == 0 { AUTO_TEXTURE_SLOTS } else { texture_slots.max(MIN_TEXTURE_SLOTS) });
        let array_cap = adapter.limits().max_binding_array_elements_per_shader_stage.saturating_sub(SAMPLER_CAP).min(wanted);
        let sampler_cap = adapter.limits().max_binding_array_sampler_elements_per_shader_stage.min(SAMPLER_CAP);
        println!("texture arrays: the adapter has the features: {}, room for {} textures and {} samplers", adapter.features().contains(array_features), array_cap, sampler_cap);
        assert!(
            adapter.features().contains(array_features) && array_cap >= MIN_TEXTURE_SLOTS && sampler_cap >= 8,
            "this adapter lacks the binding arrays of textures (the features TEXTURE_BINDING_ARRAY, SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING, PARTIALLY_BOUND_BINDING_ARRAY, room for 256 textures and 8 samplers): the wgpu renderer needs them; set [runtime] renderer_wgpu = false for Direct3D 9"
        );
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("nirender"),
                // The draws of an extension from the GPU (the distant statics of MGE XE) need the
                // first instance and the count of indirect draws, when the adapter has them.
                required_features: wgpu::Features::TEXTURE_COMPRESSION_BC
                    | (adapter.features()
                        & (wgpu::Features::POLYGON_MODE_LINE
                            | wgpu::Features::TIMESTAMP_QUERY
                            | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
                            | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES
                            | wgpu::Features::INDIRECT_FIRST_INSTANCE
                            | wgpu::Features::MULTI_DRAW_INDIRECT_COUNT))
                    | array_features,
                // The largest texture that the adapter takes, up to 16384: an atlas of the
                // distant land of MGE XE has that size.
                // A post shader of MGE XE reads up to 22 textures: 10 of its own, the picture,
                // the depth, the waves, and the outputs of its passes.
                required_limits: wgpu::Limits {
                    max_texture_dimension_2d: adapter.limits().max_texture_dimension_2d.min(16384),
                    max_sampled_textures_per_shader_stage: adapter.limits().max_sampled_textures_per_shader_stage.min(32),
                    // One buffer for the geometry of many meshes of an extension (the pool of
                    // the distant statics of MGE XE).
                    max_buffer_size: adapter.limits().max_buffer_size,
                    // The records of the draws are one storage binding: the adapter's room, not
                    // the default of 128 MiB. `record_limit` guards it.
                    max_storage_buffer_binding_size: adapter.limits().max_storage_buffer_binding_size,
                    max_binding_array_elements_per_shader_stage: array_cap + sampler_cap,
                    max_binding_array_sampler_elements_per_shader_stage: sampler_cap,
                    ..wgpu::Limits::default()
                },
                memory_hints: wgpu::MemoryHints::default(),
                ..Default::default()
            },
        ))
        .expect("no device");
        gpu::watch(&device, &info, surface.is_some());

        let multisample = (samples > 1).then(|| multisample::Multisample::new(&device, target_format, samples));
        let wireframe = device.features().contains(wgpu::Features::POLYGON_MODE_LINE);
        // The text of the extension reads the record of the draw as `draw.field`; the record
        // is in the storage buffer of the frame (`draws[record]`, see shader.wgsl).
        let extension_shader = retained::rewrite_extension_text(&extension_shader.replace("draw.", "draws[record]."));
        let text = include_str!("shader.wgsl").to_string();
        let source = format!("{text}\n{extension_shader}");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("shader.wgsl"), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let array_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("draw texture arrays"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: std::num::NonZeroU32::new(array_cap),
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: std::num::NonZeroU32::new(sampler_cap) },
            ],
        });
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("extension frame"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                // The lights and the bones of the frame.
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                // The records of the draws of the frame.
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                // The retained instances: the light indexes, the scene of the view (a
                // dynamic offset selects it), the lights.
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: core::num::NonZeroU64::new(SCENE_SIZE as u64) },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                // The bones of the skinned retained instances. The vertex stage reads 8
                // storage buffers with it: the default limit of a stage.
                wgpu::BindGroupLayoutEntry {
                    binding: 12,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("draw"),
            // From the group that changes least to the one that changes most: the frame, the
            // arrays of the frame. The record of a draw is in the frame group, named by the
            // instance of the draw.
            bind_group_layouts: &[Some(&frame_layout), Some(&array_layout)],
            immediate_size: 0,
        });
        let white = Rc::new(device
            .create_texture_with_data(
                &queue,
                &wgpu::TextureDescriptor {
                    label: Some("white"),
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
        let defaults = defaults_buffer(&device, RETAINED_AT_FIRST + INSTANCES_AT_FIRST);
        let frame_uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("extension frame"), size: 512, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let frame_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
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
        let record_limit = ((device.limits().max_storage_buffer_binding_size as u64).min(device.limits().max_buffer_size) as usize / RECORD_SIZE).min(switches::switches().record_limit.unwrap_or(usize::MAX));
        println!("draw records: room for {record_limit} in one binding ({} MB)", record_limit * RECORD_SIZE / (1 << 20));
        let first_light_indexes = light_index_buffer(&device, LIGHT_INDEXES_AT_FIRST);
        let first_scenes = scene_buffer(&device);
        let first_retained_lights = retained_light_buffer(&device, RETAINED_LIGHTS_AT_FIRST);
        let first_indirect = indirect_buffer(&device, RECORDS_AT_FIRST);
        let first_retained_bones = bone_buffer(&device, RETAINED_BONES_AT_FIRST);
        let frame_group = Rc::new(frame_group(&device, &frame_layout, &frame_uniform, &frame_sampler, &white, &clusters, &first_lights, &first_bones, &first_records, &first_light_indexes, &first_scenes, &first_retained_lights, &first_retained_bones));
        let timing = make_timing(&device, &queue);
        let stamps = timing.as_ref().map_or_else(|| Rc::new(crate::stamps::Stamps::off()), |timing| timing.stamps.clone());
        let renderer = Renderer {
            frame_layout,
            frame_uniform,
            clusters,
            extension_lights,
            frame_sampler,
            frame_group,
            light_buffer: first_lights,
            instance_buffer: instance_buffer(&device, RETAINED_AT_FIRST + INSTANCES_AT_FIRST),
            instance_capacity: RETAINED_AT_FIRST + INSTANCES_AT_FIRST,
            lights_known: HashMap::new(),
            timing,
            stamps: stamps.clone(),
            bone_buffer: first_bones,
            bone_capacity: BONES_AT_FIRST,
            retained_bone_buffer: first_retained_bones,
            retained_bone_capacity: RETAINED_BONES_AT_FIRST,
            device: Rc::new(device),
            target_format,
            surface_usages,
            anisotropy: 1,
            samples,
            multisample,
            depth_merge: None,
            wireframe,
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
            record_generation: 0,
            instance_generation: 0,
            pinned_textures: Vec::new(),
            pinned_free: Vec::new(),
            pinned_index: FxHashMap::default(),
            pinned_in_frame: 0,
            pinned_samplers: Vec::new(),
            pinned_samplers_in_frame: 0,
            pipeline_layout,
            pipelines: FxHashMap::default(),
            textures: FxHashMap::default(),
            texture_objects: FxHashMap::default(),
            decoded: Default::default(),
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
            geometries: FxHashMap::default(),
            gone_textures: Vec::new(),
            vertex_pool: pool::Pool::new("game vertices", wgpu::BufferUsages::VERTEX, VERTEX_CHUNK),
            index_pool: pool::Pool::new("game indexes", wgpu::BufferUsages::INDEX, INDEX_CHUNK),
            retired: Vec::new(),
            retired_indexes: Vec::new(),
            quad_indexes: None,
            particle_quads: None,
            materials: FxHashMap::default(),
            derived: FxHashMap::default(),
            defaults,
            screen_pipelines: HashMap::new(),
            screen_layout: None,
            screen_sampler: None,
            counts: Counts::default(),
            commands: Rc::new(FrameCommands::with_stamps(stamps.clone())),
        };
        (renderer, surface)
    }

    /// The arrays of the textures and the samplers at the start of a frame. They stay from
    /// frame to frame with their group, so a frame with no new texture or sampler makes no
    /// group. They are laid out again (slot 0 white and a plain sampler, then the pinned
    /// slots, `pin_into_frame`) when the pinned part grew past its place, when the frame's own
    /// part grew past twice [`FRAME_RESERVE`], or when the room runs short. True when they
    /// were laid out again.
    pub(crate) fn begin_frame_arrays(&mut self) -> bool {
        let laid_out = !self.frame_texture_views.is_empty();
        let pinned_grew = self.pinned_textures.len() > self.pinned_in_frame || self.pinned_samplers.len() > self.pinned_samplers_in_frame;
        let own = self.frame_texture_views.len().saturating_sub(self.pinned_in_frame + 1);
        let room_short = self.frame_texture_views.len() + FRAME_RESERVE > self.array_cap || self.frame_samplers.len() + 8 > self.sampler_cap;
        if laid_out && !pinned_grew && own <= 2 * FRAME_RESERVE && !room_short {
            self.counts.arrays_kept += 1;
            return false;
        }
        self.counts.arrays_laid_out += 1;
        self.frame_texture_slots.clear();
        self.frame_texture_views.clear();
        self.frame_sampler_slots.clear();
        self.frame_samplers.clear();
        self.array_group = None;
        self.array_variants.clear();
        self.frame_texture_views.push(self.white.clone());
        let plain = self.sampler(material::sampler_states(0, 0));
        self.frame_samplers.push(plain);
        true
    }

    /// The texture of a handle. A handle is the address of an object of the game's side:
    /// a released handle comes back for a later texture. A pinned slot of the arrays, and a
    /// slot of the frame, that holds the handle takes the new view.
    pub(crate) fn set_texture(&mut self, handle: u32, view: Rc<wgpu::TextureView>) {
        let mut changed = false;
        if let Some(pinned) = self.pinned_textures.iter_mut().find(|p| p.handle == handle) {
            if !Rc::ptr_eq(&pinned.view, &view) {
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
            self.array_group = None;
            self.array_variants.clear();
            self.counts.textures_replaced += 1;
        }
        self.textures.insert(handle, view);
    }

    /// The slot of a texture of the game in the arrays of the frame: a new one for a handle
    /// the frame did not have. 0 (white) for no texture, an unknown handle, or a frame with
    /// more textures than the arrays hold.
    pub(crate) fn frame_texture_slot(&mut self, handle: u32) -> u32 {
        if handle == 0 {
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
        self.array_group = None;
        self.array_variants.clear();
        slot
    }

    /// The slot of a sampler state in the arrays of the frame.
    pub(crate) fn frame_sampler_slot(&mut self, values: [u32; 5]) -> u32 {
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
        self.array_group = None;
        self.array_variants.clear();
        slot
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
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureViewArray(&views) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::SamplerArray(&samplers) },
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
            self.queue.write_buffer(&self.indirect_buffer, (self.indirect_written * ARGS_SIZE) as u64, bytemuck::cast_slice(&args[self.indirect_written..]));
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
            self.queue.write_buffer(&self.instance_buffer, (self.retained_instances * INSTANCE_SIZE) as u64, bytemuck::cast_slice(instances));
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
        self.queue.write_buffer(&self.instance_buffer, (first * INSTANCE_SIZE) as u64, bytemuck::cast_slice(instances));
        Some(first as u32)
    }

    /// The record of a draw of the frame: its data, in the records of the frame after
    /// `upload_records`. Returns its index, or [`None`] when the records reached the device's
    /// limit of a storage binding: the draw is then not drawn (counted, and said once), and
    /// the rest of the frame draws.
    pub(crate) fn record(&mut self, data: &DrawData) -> Option<u32> {
        if self.retained_records + self.record_count >= self.record_limit {
            if self.counts.records_over_limit == 0 {
                println!("draw records: the frame reached the limit of {} records; draws past it are not drawn", self.record_limit);
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
        if needed > self.record_capacity {
            self.record_capacity = needed.next_power_of_two().min(self.record_limit);
            self.record_buffer = Rc::new(record_buffer(&self.device, self.record_capacity));
            self.record_written = 0;
            self.record_extras = 0;
            self.record_generation = self.record_generation.wrapping_add(1);
        }
        if self.record_count > self.record_written {
            self.queue.write_buffer(&self.record_buffer, ((self.retained_records + self.record_written) * RECORD_SIZE) as u64, &self.record_staging[self.record_written * RECORD_SIZE..]);
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
        self.queue.write_buffer(&self.record_buffer, (index * RECORD_SIZE) as u64, bytemuck::bytes_of(data));
        Some(index as u32)
    }

    /// A copy of a draw for this part of the frame: a record with `data`, and instances with
    /// the world matrices of the draw's that name it. The first instance, or [`None`] when the
    /// room of the records or of the instances is full.
    pub(crate) fn copy_of_draw(&mut self, data: &DrawData, instances: &[Instance]) -> Option<u32> {
        let record = self.record_now(data)?;
        let copies: Vec<Instance> = instances.iter().map(|instance| Instance { world: instance.world, record: [record, 0, 0, 0] }).collect();
        self.append_instances(&copies)
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
        let format = if decode { wgpu::TextureFormat::Bgra8Unorm } else { format };
        let mut bytes = Vec::new();
        for level in 0..levels {
            let (w, h) = (word(data, widths + level * 4) as usize, word(data, heights + level * 4) as usize);
            let size = if block != 0 { w.div_ceil(4).max(1) * h.div_ceil(4).max(1) * block } else { w * h * source_bytes };
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
                bytes.extend_from_slice(&expand_16_bits(d3d_format, &source));
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
        let texture = self.device.create_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor {
                label: Some("game texture"),
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
        self.set_texture(handle, Rc::new(texture.create_view(&Default::default())));
        self.texture_objects.insert(handle, (texture, d3d_format));
        self.counts.textures += 1;
    }

    /// The pipeline for a draw in screen coordinates: a frame of a film. It does not blend:
    /// the film player of the game is the only caller of `DrawPrimitiveUP`
    /// (`CutscenePlayer_static::drawRect`), and it sets `D3DRS_ALPHABLENDENABLE` to 0 before
    /// its draws (`CutscenePlayer_static::setupD3DState` 0x64ECD0). `samples` is that of the
    /// pass that the draw goes into.
    pub(crate) fn screen_pipeline(&mut self, samples: u32) -> wgpu::RenderPipeline {
        if self.screen_layout.is_none() {
            self.screen_layout = Some(self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
            self.screen_sampler = Some(self.device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }));
        }
        if let Some(pipeline) = self.screen_pipelines.get(&samples) {
            return pipeline.clone();
        }
        let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("screen draw"), source: wgpu::ShaderSource::Wgsl(SCREEN_SHADER.into()) });
        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("screen draw"),
            bind_group_layouts: &[self.screen_layout.as_ref()],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x2];
        let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("screen draw"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: 40, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attributes })],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
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
        let format = if decode { wgpu::TextureFormat::Bgra8Unorm } else { format };
        if decode {
            self.decoded.insert(handle);
            self.counts.textures_decoded += 1;
        } else {
            self.decoded.remove(&handle);
        }
        let all = 32 - width.max(height).leading_zeros();
        let levels = if levels == 0 { all } else { levels.min(all) };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("game render target"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.set_texture(handle, Rc::new(texture.create_view(&Default::default())));
        self.texture_objects.insert(handle, (texture, d3d_format));
        self.counts.textures += 1;
    }

    /// Makes a texture that the game draws into, with a depth buffer of its size.
    pub(crate) fn render_texture(&mut self, handle: u32, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        let texture = target_texture(&self.device, width, height, self.target_format, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC);
        let depth = target_texture(&self.device, width, height, DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let view = Rc::new(texture.create_view(&Default::default()));
        self.set_texture(handle, view.clone());
        self.texture_objects.remove(&handle);
        self.targets.insert(handle, RenderTexture { texture, view, depth: Rc::new(depth.create_view(&Default::default())), size: (width, height) });
        self.counts.render_textures += 1;
    }

    /// The game copied pixels of its own into a texture that it draws into. `pixels` has 4
    /// bytes for each pixel, blue first.
    pub(crate) fn target_pixels(&mut self, handle: u32, width: u32, height: u32, pixels: &[u8]) {
        let Some(target) = self.targets.get(&handle) else { return };
        if target.size != (width, height) || pixels.len() < (width * height * 4) as usize {
            self.counts.texture_updates_lost += 1;
            return;
        }
        let mut bytes = pixels[..(width * height * 4) as usize].to_vec();
        if !matches!(self.target_format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb) {
            for pixel in bytes.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &target.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: Some(height) },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.counts.target_copies += 1;
    }

    /// The game wrote a level of a texture again.
    pub(crate) fn texture_update(&mut self, data: &[u8]) {
        let (handle, level, width, height, size) = (word(data, 0), word(data, 4), word(data, 8), word(data, 12), word(data, 20) as usize);
        let Some((texture, d3d_format)) = self.texture_objects.get(&handle) else {
            self.counts.texture_updates_lost += 1;
            *self.counts.lost_reasons.entry(format!("the texture is not one that this program made, format {:#x}", word(data, 16))).or_default() += 1;
            return;
        };
        let decode = self.decoded.contains(&handle);
        let block = match d3d_format {
            0x3154_5844 => 8,
            0x3354_5844 | 0x3554_5844 => 16,
            _ => 0,
        };
        let source_bytes = texture_format(*d3d_format).map_or(4, |f| f.2 as u32);
        let (row, rows, expected) = if block != 0 {
            (width.div_ceil(4) * block, height.div_ceil(4), (width.div_ceil(4) * height.div_ceil(4) * block) as usize)
        } else {
            (width * gpu_bytes(*d3d_format) as u32, height, (width * height * source_bytes) as usize)
        };
        if level >= texture.mip_level_count() || size < expected || data.len() < 24 + expected {
            self.counts.texture_updates_lost += 1;
            let reason = if level >= texture.mip_level_count() { "the texture has no such level" } else { "the record has too few bytes" };
            *self.counts.lost_reasons.entry(format!("{reason}, format {d3d_format:#x}, level {level} of {}, {width} x {height}", texture.mip_level_count())).or_default() += 1;
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
            pixels = std::borrow::Cow::Owned(expand_16_bits(*d3d_format, &pixels));
        } else if *d3d_format == 22 {
            // No alpha in this format.
            for pixel in pixels.to_mut().chunks_exact_mut(4) {
                pixel[3] = 255;
            }
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &pixels,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(rows) },
            wgpu::Extent3d { width: if block != 0 { width.next_multiple_of(4) } else { width }, height: if block != 0 { height.next_multiple_of(4) } else { height }, depth_or_array_layers: 1 },
        );
        self.counts.texture_updates += 1;
    }

    /// A geometry record: resident buffers for the draws that name it.
    pub(crate) fn geometry(&mut self, data: &[u8]) {
        let clock = ticks::Clock::now();
        let (handle, fvf, stride, vertices, entries, strips) = (word(data, 0), word(data, 12), word(data, 16), word(data, 20), word(data, 24), word(data, 28));
        let lengths = ((strips as usize * 2) + 3) & !3;
        let (vertex_at, index_at) = (32 + lengths, 32 + lengths + vertices as usize * stride as usize);
        let index_end = index_at + entries as usize * 2;
        if data.len() < index_end || stride == 0 {
            self.counts.geometry_bad += 1;
            return;
        }
        let strip_lengths: Vec<u16> = (0..strips as usize).map(|i| u16::from_le_bytes([data[32 + i * 2], data[33 + i * 2]])).collect();
        let vertex_range = self.vertex_pool.take(&self.device, &self.queue, &data[vertex_at..index_at], stride as u64);
        let index_range = self.index_pool.take(&self.device, &self.queue, &data[index_at..index_end], 4);
        self.counts.geometry_records += 1;
        self.counts.geometry_bytes += data.len() as u64;
        let geometry = Geometry {
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
        };
        self.put_geometry(handle, geometry);
        self.counts.times[TIME_BUFFERS] += clock.elapsed();
    }

    /// Puts a geometry under its handle. The ranges of the geometry it replaces go back to the
    /// pools after the frame: the draws of this frame may still use them.
    fn put_geometry(&mut self, handle: u32, geometry: Geometry) {
        if let Some(old) = self.geometries.insert(handle, geometry) {
            self.retired.extend(old.vertex_range);
            self.retired_indexes.extend(old.index_range);
        }
    }

    /// A geometry data of the game was destroyed (`GEOMETRY_GONE`): its geometry goes, and its
    /// ranges go back to the pools after the frame. Only a geometry of the same unique id goes
    /// (the quads of particles have none: they go too). True when a geometry went.
    pub(crate) fn geometry_gone(&mut self, handle: u32, unique: u32) -> bool {
        if !self.geometries.get(&handle).is_some_and(|g| g.unique.is_none_or(|id| id == unique)) {
            self.counts.geometries_gone_not_held += 1;
            return false;
        }
        if let Some(old) = self.geometries.remove(&handle) {
            self.retired.extend(old.vertex_range);
            self.retired_indexes.extend(old.index_range);
        }
        self.counts.geometries_gone += 1;
        true
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
            self.texture_objects.remove(&handle);
            self.targets.remove(&handle);
            self.decoded.remove(&handle);
            // The arrays stay from frame to frame: a slot of the frame's own part lets the
            // texture go. A pinned slot goes with its last use (`unpin_texture`).
            if let Some(&slot) = self.frame_texture_slots.get(&handle) {
                if slot as usize > self.pinned_in_frame {
                    self.frame_texture_views[slot as usize] = self.white.clone();
                    self.frame_texture_slots.remove(&handle);
                    self.array_group = None;
                    self.array_variants.clear();
                }
            }
            self.counts.textures_gone += 1;
        }
    }

    /// One line of what the helper holds for the handles of the game: textures, geometry,
    /// materials, and the frees so far.
    pub(crate) fn held_line(&self) -> String {
        let mb = |bytes: u64| bytes as f64 / 1e6;
        format!(
            "held: {} textures ({} with pixels written here, {} rendered), {} level surfaces; {} geometries, the vertex pool {:.1} of {:.1} MB, the index pool {:.1} of {:.1} MB; {} materials ({} derived); gone: {} materials, {} textures ({} not held, {} replaced before their removal), {} geometries ({} not held), {} instances whose geometry went",
            self.textures.len(),
            self.texture_objects.len(),
            self.targets.len(),
            self.surfaces.len(),
            self.geometries.len(),
            mb(self.vertex_pool.used),
            mb(self.vertex_pool.capacity()),
            mb(self.index_pool.used),
            mb(self.index_pool.capacity()),
            self.materials.len(),
            self.derived.len(),
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
    /// on the GPU (`particle_quads`), or on the CPU with `MGE_XE_PARTICLE_QUADS=cpu`.
    pub(crate) fn particles(&mut self, handle: u32, radius: f32, right: [f32; 3], up: [f32; 3], particles: &[u8]) {
        let clock = ticks::Clock::now();
        const STRIDE: usize = 36;
        const NORMAL: f32 = 0.3;
        let count = particles.len() / 20;
        let sum: [f32; 3] = core::array::from_fn(|i| (right[i] + up[i]) * radius);
        let difference: [f32; 3] = core::array::from_fn(|i| (up[i] - right[i]) * radius);
        let index = self.quad_index_buffer(count as u32);
        if !switches().particle_quads_cpu {
            let device = self.device.clone();
            let quads = self.particle_quads.get_or_insert_with(|| particle_quads::ParticleQuads::new(&device));
            let (vertex, base_vertex) = quads.add(&device, particles, sum, difference);
            self.counts.shape_particles += 1;
            // D3DFVF_XYZ | D3DFVF_NORMAL | D3DFVF_DIFFUSE | D3DFVF_TEX1.
            let geometry = Geometry { fvf: 0x152, stride: STRIDE as u32, entries: count as u32 * 6, strips: Vec::new(), vertex, index, vertex_range: None, index_range: None, base_vertex, first_index: 0, unique: None };
            self.put_geometry(handle, geometry);
            self.counts.times[TIME_BUFFERS] += clock.elapsed();
            return;
        }
        let mut bytes = Vec::with_capacity(count * 4 * STRIDE);
        for p in particles.chunks_exact(20) {
            let position = [float(p, 0), float(p, 4), float(p, 8)];
            let size = float(p, 12);
            let colour = &p[16..20];
            let s: [f32; 3] = core::array::from_fn(|i| sum[i] * size);
            let d: [f32; 3] = core::array::from_fn(|i| difference[i] * size);
            let corners = [
                ([position[0] - s[0], position[1] - s[1], position[2] - s[2]], [0.0f32, 0.0f32]),
                ([position[0] - d[0], position[1] - d[1], position[2] - d[2]], [1.0, 0.0]),
                ([s[0] + position[0], s[1] + position[1], s[2] + position[2]], [1.0, 1.0]),
                ([d[0] + position[0], d[1] + position[1], d[2] + position[2]], [0.0, 1.0]),
            ];
            for (at, uv) in corners {
                for v in at.iter().chain([NORMAL; 3].iter()) {
                    bytes.extend_from_slice(&v.to_le_bytes());
                }
                bytes.extend_from_slice(colour);
                bytes.extend_from_slice(&uv[0].to_le_bytes());
                bytes.extend_from_slice(&uv[1].to_le_bytes());
            }
        }
        let quads = count as u32;
        let vertex = Rc::new(self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("particles"), contents: &bytes, usage: wgpu::BufferUsages::VERTEX }));
        self.counts.shape_particles += 1;
        // D3DFVF_XYZ | D3DFVF_NORMAL | D3DFVF_DIFFUSE | D3DFVF_TEX1.
        let geometry = Geometry { fvf: 0x152, stride: STRIDE as u32, entries: quads * 6, strips: Vec::new(), vertex, index, vertex_range: None, index_range: None, base_vertex: 0, first_index: 0, unique: None };
        self.put_geometry(handle, geometry);
        self.counts.times[TIME_BUFFERS] += clock.elapsed();
    }

    /// The index buffer of the quads, two triangles of four vertices each, for at least
    /// `quads` quads: the game keeps one and it grows to the largest system seen.
    fn quad_index_buffer(&mut self, quads: u32) -> Rc<wgpu::Buffer> {
        if self.quad_indexes.as_ref().is_none_or(|(have, _)| *have < quads) {
            let indexes: Vec<u8> = (0..quads).flat_map(|q| [0u16, 1, 2, 0, 2, 3].map(|i| (q * 4) as u16 + i)).flat_map(|i| i.to_le_bytes()).collect();
            let buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("quads"), contents: &indexes, usage: wgpu::BufferUsages::INDEX });
            self.quad_indexes = Some((quads, Rc::new(buffer)));
        }
        self.quad_indexes.as_ref().map(|(_, buffer)| buffer.clone()).expect("the quad index buffer was just made")
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
            Rc::new(device.create_sampler(&wgpu::SamplerDescriptor {
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

    pub(crate) fn pipeline(&mut self, key: PipelineKey) -> Option<Rc<wgpu::RenderPipeline>> {
        if let Some(pipeline) = self.pipelines.get(&key) {
            return Some(pipeline.clone());
        }
        {
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
            // The world matrix of an instance: four columns at the locations 9 to 12; the
            // record of the draw at 13.
            let mut from_instances: Vec<wgpu::VertexAttribute> =
                (0..4).map(|column| wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: column as u64 * 16, shader_location: 9 + column }).collect();
            from_instances.push(wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32x2, offset: 64, shader_location: 13 });
            let buffers = [
                Some(wgpu::VertexBufferLayout { array_stride: key.stride as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &from_vertex }),
                Some(wgpu::VertexBufferLayout { array_stride: 64, step_mode: wgpu::VertexStepMode::Instance, attributes: &from_defaults }),
                Some(wgpu::VertexBufferLayout { array_stride: INSTANCE_SIZE as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &from_instances }),
            ];
            let blend = (key.blend[0] != 0).then(|| {
                let component = wgpu::BlendComponent {
                    src_factor: blend_factor(key.blend[1]),
                    dst_factor: blend_factor(key.blend[2]),
                    operation: wgpu::BlendOperation::Add,
                };
                wgpu::BlendState { color: component, alpha: component }
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
            let pipeline = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("game draw"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState { module: &self.shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &buffers },
                primitive: wgpu::PrimitiveState {
                    topology,
                    front_face: wgpu::FrontFace::Cw,
                    cull_mode,
                    polygon_mode: if key.fill == 2 && self.wireframe { wgpu::PolygonMode::Line } else { wgpu::PolygonMode::Fill },
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_on && key.depth[1] != 0),
                    depth_compare: Some(if depth_on { compare(key.depth[2]) } else { wgpu::CompareFunction::Always }),
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
                        wgpu::StencilState { front: face, back: face, read_mask: key.stencil[5] & 0xFF, write_mask: key.stencil[6] & 0xFF }
                    },
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: key.samples, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: self.target_format,
                        blend,
                        write_mask: wgpu::ColorWrites::from_bits_truncate(key.color_write),
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
            let pipeline = Rc::new(pipeline);
            self.pipelines.insert(key, pipeline.clone());
            Some(pipeline)
        }
    }
}
