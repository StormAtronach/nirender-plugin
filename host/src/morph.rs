//! The morph controllers of the engine (`NiGeomMorpherController`) on the GPU.
//!
//! The engine's morph sets the positions of its geometry data to the sum of its targets, each
//! times its weight at the controller's time, and gives the data a new revision
//! (`NiMorpherController::GenMorphInterp` 0x725ED0, from `OnPreDisplay` 0x717370). The plugin
//! sends the targets of a geometry once ([`crate::tag::MORPH_TARGETS`]) and then the weights
//! of each new revision ([`crate::tag::MORPH_WEIGHTS`]). A compute pass of the frame writes
//! the sum into the positions of the geometry's vertices in its pool, before the frame's
//! draws; the normals, the colours and the texture sets stay those of the geometry record.

use super::*;

/// The values of a job: a dynamic offset must be a multiple of 256.
const JOB_STRIDE: u64 = 256;
/// The vertices of a workgroup.
const GROUP: u32 = 64;

const SHADER: &str = r#"
struct Job {
    // x: the first float of the targets; y: the targets; z: the vertices; w: the first weight.
    range: vec4u,
    // x: the first word of the vertices in their buffer; y: the words of a vertex.
    place: vec4u,
}

@group(0) @binding(0) var<storage, read> targets: array<f32>;
@group(0) @binding(1) var<storage, read> weights: array<f32>;
@group(0) @binding(2) var<storage, read_write> vertices: array<u32>;
@group(0) @binding(3) var<uniform> job: Job;

@compute @workgroup_size(64)
fn morph(@builtin(global_invocation_id) id: vec3u) {
    let v = id.x;
    if (v >= job.range.z) {
        return;
    }
    var p = vec3f(0.0);
    for (var t = 0u; t < job.range.y; t++) {
        let at = job.range.x + (t * job.range.z + v) * 3u;
        p += weights[job.range.w + t] * vec3f(targets[at], targets[at + 1u], targets[at + 2u]);
    }
    let w = job.place.x + v * job.place.y;
    vertices[w] = bitcast<u32>(p.x);
    vertices[w + 1u] = bitcast<u32>(p.y);
    vertices[w + 2u] = bitcast<u32>(p.z);
}
"#;

/// The targets of a geometry in the targets buffer.
struct Targets {
    unique: u32,
    count: u32,
    vertices: u32,
    /// The first float in `data` and in the buffer.
    first: u32,
}

/// A morph of a frame: the targets, the weights, and the vertices to write.
struct Job {
    first: u32,
    count: u32,
    vertices: u32,
    first_weight: u32,
    buffer: Rc<wgpu::Buffer>,
    first_word: u32,
    stride_words: u32,
}

pub(crate) struct Morphs {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    targets: HashMap<u32, Targets>,
    /// The floats of the targets, as the buffer has them. A geometry that goes leaves its
    /// floats until the buffer is made again.
    data: Vec<f32>,
    buffer: wgpu::Buffer,
    capacity: usize,
    weights: Vec<f32>,
    weights_buffer: wgpu::Buffer,
    weights_capacity: usize,
    jobs: Vec<Job>,
    /// The job of each handle in this frame: a later step of the frame replaces its weights.
    job_of: HashMap<u32, usize>,
    jobs_buffer: wgpu::Buffer,
    jobs_capacity: u64,
    /// The steps of the session, and the steps not made: no targets of that unique id, no
    /// geometry, or a vertex buffer over the device's limit of a storage binding.
    pub(crate) steps: u64,
    pub(crate) skipped: u64,
}

fn buffer(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false })
}

impl Morphs {
    pub(crate) fn new(device: &wgpu::Device) -> Morphs {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("morphs"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let entry = |binding: u32, ty: wgpu::BufferBindingType, dynamic: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: dynamic, min_binding_size: None },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("morphs"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Storage { read_only: true }, false),
                entry(1, wgpu::BufferBindingType::Storage { read_only: true }, false),
                entry(2, wgpu::BufferBindingType::Storage { read_only: false }, false),
                entry(3, wgpu::BufferBindingType::Uniform, true),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("morphs"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("morphs"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("morph"),
            compilation_options: Default::default(),
            cache: None,
        });
        let capacity = 256 * 1024;
        let weights_capacity = 4096;
        let jobs_capacity = 64 * JOB_STRIDE;
        Morphs {
            pipeline,
            layout,
            targets: HashMap::new(),
            data: Vec::new(),
            buffer: buffer(device, "morphs, the targets", capacity as u64 * 4, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST),
            capacity,
            weights: Vec::new(),
            weights_buffer: buffer(device, "morphs, the weights", weights_capacity as u64 * 4, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST),
            weights_capacity,
            jobs: Vec::new(),
            job_of: HashMap::new(),
            jobs_buffer: buffer(device, "morphs, the jobs", jobs_capacity, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST),
            jobs_capacity,
            steps: 0,
            skipped: 0,
        }
    }

    /// The targets of a geometry (`MORPH_TARGETS`): `count` targets of `vertices` positions.
    pub(crate) fn put_targets(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, handle: u32, unique: u32, count: u32, vertices: u32, floats: &[f32]) {
        self.targets.remove(&handle);
        let first = self.data.len();
        if first + floats.len() > self.capacity {
            self.compact();
            let first = self.data.len();
            if first + floats.len() > self.capacity {
                self.capacity = (first + floats.len()).next_power_of_two();
                self.buffer = buffer(device, "morphs, the targets", self.capacity as u64 * 4, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
            }
            // The jobs of this frame name floats of the old order: a new buffer or a new order
            // takes them out; their geometries keep the vertices of their records.
            self.jobs.clear();
            self.job_of.clear();
            self.weights.clear();
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.data));
        }
        let first = self.data.len();
        self.data.extend_from_slice(floats);
        queue.write_buffer(&self.buffer, first as u64 * 4, bytemuck::cast_slice(floats));
        self.targets.insert(handle, Targets { unique, count, vertices, first: first as u32 });
    }

    /// The floats of the geometries that are held, from the start; the others go.
    fn compact(&mut self) {
        let mut data = Vec::with_capacity(self.data.len());
        for targets in self.targets.values_mut() {
            let (from, len) = (targets.first as usize, (targets.count * targets.vertices * 3) as usize);
            targets.first = data.len() as u32;
            data.extend_from_slice(&self.data[from..from + len]);
        }
        self.data = data;
    }

    /// A geometry went (`GEOMETRY_GONE`): its targets go.
    pub(crate) fn gone(&mut self, handle: u32, unique: u32) {
        if self.targets.get(&handle).is_some_and(|targets| targets.unique == unique) {
            self.targets.remove(&handle);
        }
    }

    /// The weights of a new revision of a geometry (`MORPH_WEIGHTS`): a job of this frame writes
    /// the sum into the positions of `geometry`'s vertices. False when it cannot: the
    /// geometry's vertices stay those of its record.
    pub(crate) fn step(&mut self, device: &wgpu::Device, handle: u32, unique: u32, weights: &[f32], geometry: &Geometry) -> bool {
        let Some(targets) = self.targets.get(&handle).filter(|t| t.unique == unique && t.count as usize == weights.len()) else {
            self.skipped += 1;
            return false;
        };
        let Some(range) = geometry.vertex_range.as_ref() else {
            self.skipped += 1;
            return false;
        };
        if geometry.vertex.size() > device.limits().max_storage_buffer_binding_size as u64 || geometry.stride % 4 != 0 {
            self.skipped += 1;
            return false;
        }
        let vertices = targets.vertices.min((range.size / geometry.stride as u64) as u32);
        let first_weight = self.weights.len() as u32;
        self.weights.extend_from_slice(weights);
        let job = Job {
            first: targets.first,
            count: targets.count,
            vertices,
            first_weight,
            buffer: geometry.vertex.clone(),
            first_word: (geometry.base_vertex as u32) * geometry.stride / 4,
            stride_words: geometry.stride / 4,
        };
        match self.job_of.get(&handle) {
            Some(&n) => self.jobs[n] = job,
            None => {
                self.job_of.insert(handle, self.jobs.len());
                self.jobs.push(job);
            }
        }
        self.steps += 1;
        true
    }

    /// Records the compute pass of the frame's jobs into the frame's commands, before the
    /// draws, and starts the next frame's list.
    pub(crate) fn flush(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, commands: &FrameCommands) {
        if self.jobs.is_empty() {
            return;
        }
        if self.weights.len() > self.weights_capacity {
            self.weights_capacity = self.weights.len().next_power_of_two();
            self.weights_buffer = buffer(device, "morphs, the weights", self.weights_capacity as u64 * 4, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
        }
        queue.write_buffer(&self.weights_buffer, 0, bytemuck::cast_slice(&self.weights));
        let needed = self.jobs.len() as u64 * JOB_STRIDE;
        if needed > self.jobs_capacity {
            self.jobs_capacity = needed.next_power_of_two();
            self.jobs_buffer = buffer(device, "morphs, the jobs", self.jobs_capacity, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        }
        let mut values = vec![0u8; needed as usize];
        for (n, job) in self.jobs.iter().enumerate() {
            let words: [u32; 8] = [job.first, job.count, job.vertices, job.first_weight, job.first_word, job.stride_words, 0, 0];
            values[n * JOB_STRIDE as usize..n * JOB_STRIDE as usize + 32].copy_from_slice(bytemuck::cast_slice(&words));
        }
        queue.write_buffer(&self.jobs_buffer, 0, &values);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("morphs") });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("morphs"), timestamp_writes: None });
            pass.set_pipeline(&self.pipeline);
            let mut bound: Option<(Rc<wgpu::Buffer>, wgpu::BindGroup)> = None;
            for (n, job) in self.jobs.iter().enumerate() {
                if !bound.as_ref().is_some_and(|(buffer, _)| Rc::ptr_eq(buffer, &job.buffer)) {
                    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("morphs"),
                        layout: &self.layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: self.buffer.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: self.weights_buffer.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 2, resource: job.buffer.as_entire_binding() },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &self.jobs_buffer, offset: 0, size: core::num::NonZeroU64::new(32) }),
                            },
                        ],
                    });
                    bound = Some((job.buffer.clone(), group));
                }
                if let Some((_, group)) = bound.as_ref() {
                    pass.set_bind_group(0, group, &[(n as u64 * JOB_STRIDE) as u32]);
                }
                pass.dispatch_workgroups(job.vertices.div_ceil(GROUP), 1, 1);
            }
        }
        commands.defer(encoder.finish());
        self.jobs.clear();
        self.job_of.clear();
        self.weights.clear();
    }
}
