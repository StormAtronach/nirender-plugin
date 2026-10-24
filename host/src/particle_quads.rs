//! The quads of the particle systems made on the GPU (the particles plan, step 1): the
//! particles of a shape record (20 bytes each: the position, the size, the colour) go into
//! an input buffer of the frame, and a compute pass writes four vertices for each into a
//! vertex buffer of the frame, as `NiDX8Renderer::RenderPoints` writes them (the position,
//! the normal 0.3, the colour, the texture coordinates; 36 bytes). The draws then take the
//! path of any shape. `MGE_XE_PARTICLE_QUADS=cpu` keeps the corners made on the CPU
//! (`Renderer::particles`).
//!
//! The input and the vertices of a frame are not overwritten within the frame: each system
//! gets its own range, also a system drawn twice (the world and a reflection). The compute
//! passes go into the frame's commands, which the frame's submit sends before its draws.

use super::*;

/// The bytes of a particle of a shape record and of a vertex of a quad.
const PARTICLE: u64 = 20;
const VERTEX: u64 = 36;
/// The particles of a workgroup.
const GROUP: u32 = 64;
/// The bytes of the values of a system: a dynamic offset must be a multiple of 256.
const JOB_STRIDE: u64 = 256;

const SHADER: &str = r#"
struct Job {
    sum: vec4f,
    difference: vec4f,
    // x: the first particle in the input; y: the particles; z: the first vertex in the output.
    range: vec4u,
}

@group(0) @binding(0) var<storage, read> particles: array<u32>;
@group(0) @binding(1) var<storage, read_write> vertices: array<u32>;
@group(0) @binding(2) var<uniform> job: Job;

fn put(vertex: u32, at: vec3f, colour: u32, uv: vec2f) {
    let w = vertex * 9u;
    vertices[w] = bitcast<u32>(at.x);
    vertices[w + 1u] = bitcast<u32>(at.y);
    vertices[w + 2u] = bitcast<u32>(at.z);
    // The normal of every corner, as RenderPoints writes it.
    vertices[w + 3u] = bitcast<u32>(0.3);
    vertices[w + 4u] = bitcast<u32>(0.3);
    vertices[w + 5u] = bitcast<u32>(0.3);
    vertices[w + 6u] = colour;
    vertices[w + 7u] = bitcast<u32>(uv.x);
    vertices[w + 8u] = bitcast<u32>(uv.y);
}

@compute @workgroup_size(64)
fn quads(@builtin(global_invocation_id) id: vec3u) {
    let i = id.x;
    if (i >= job.range.y) {
        return;
    }
    let r = (job.range.x + i) * 5u;
    let position = vec3f(bitcast<f32>(particles[r]), bitcast<f32>(particles[r + 1u]), bitcast<f32>(particles[r + 2u]));
    let size = bitcast<f32>(particles[r + 3u]);
    let colour = particles[r + 4u];
    let s = job.sum.xyz * size;
    let d = job.difference.xyz * size;
    let v = job.range.z + i * 4u;
    put(v, position - s, colour, vec2f(0.0, 0.0));
    put(v + 1u, position - d, colour, vec2f(1.0, 0.0));
    put(v + 2u, s + position, colour, vec2f(1.0, 1.0));
    put(v + 3u, d + position, colour, vec2f(0.0, 1.0));
}
"#;

/// A system of the frame waiting for its compute pass.
struct Job {
    first_particle: u32,
    count: u32,
    first_vertex: u32,
    sum: [f32; 3],
    difference: [f32; 3],
    /// The vertex buffer the job writes: the one of the frame when the job came (a buffer
    /// that grows later in the frame does not move it).
    vertices: Rc<wgpu::Buffer>,
}

pub(crate) struct ParticleQuads {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    input: wgpu::Buffer,
    input_capacity: u64,
    vertices: Rc<wgpu::Buffer>,
    vertex_capacity: u64,
    jobs_buffer: wgpu::Buffer,
    jobs_capacity: u64,
    /// The particles of the frame not yet written to `input`, from `staged_from` on.
    staged: Vec<u8>,
    staged_from: u64,
    /// The particles and the vertices of the frame so far.
    particles_used: u64,
    vertices_used: u64,
    jobs: Vec<Job>,
    /// The systems and the particles made on the GPU, for the report.
    pub(crate) systems: u64,
    pub(crate) particles: u64,
}

fn storage(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false })
}

impl ParticleQuads {
    pub(crate) fn new(device: &wgpu::Device) -> ParticleQuads {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("particle quads"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let entry = |binding: u32, ty: wgpu::BufferBindingType, dynamic: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: dynamic, min_binding_size: None },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle quads"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Storage { read_only: true }, false),
                entry(1, wgpu::BufferBindingType::Storage { read_only: false }, false),
                entry(2, wgpu::BufferBindingType::Uniform, true),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("particle quads"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("particle quads"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("quads"),
            compilation_options: Default::default(),
            cache: None,
        });
        let input_capacity = 64 * 1024;
        let vertex_capacity = input_capacity / PARTICLE * 4 * VERTEX;
        let jobs_capacity = 64 * JOB_STRIDE;
        ParticleQuads {
            pipeline,
            layout,
            input: storage(device, "particle quads, the particles", input_capacity, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST),
            input_capacity,
            vertices: Rc::new(storage(device, "particle quads, the vertices", vertex_capacity, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX)),
            vertex_capacity,
            jobs_buffer: storage(device, "particle quads, the systems", jobs_capacity, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST),
            jobs_capacity,
            staged: Vec::new(),
            staged_from: 0,
            particles_used: 0,
            vertices_used: 0,
            jobs: Vec::new(),
            systems: 0,
            particles: 0,
        }
    }

    /// The start of a frame: the ranges of the frame before are free (its draws went with
    /// its submit, before this frame's compute passes).
    pub(crate) fn begin_frame(&mut self) {
        self.staged.clear();
        self.staged_from = 0;
        self.particles_used = 0;
        self.vertices_used = 0;
        self.jobs.clear();
    }

    /// Takes the particles of a system and gives the vertex buffer and the first vertex of
    /// its quads. `sum` and `difference` are `(right + up) * radius` and `(up - right) *
    /// radius`, as the CPU path makes them.
    pub(crate) fn add(&mut self, device: &wgpu::Device, particles: &[u8], sum: [f32; 3], difference: [f32; 3]) -> (Rc<wgpu::Buffer>, i32) {
        let count = (particles.len() as u64 / PARTICLE) as u32;
        let needed = (self.vertices_used + count as u64 * 4) * VERTEX;
        if needed > self.vertex_capacity {
            // A larger buffer for the rest of the frame; the jobs so far keep theirs.
            let capacity = needed.next_power_of_two();
            self.vertices = Rc::new(storage(device, "particle quads, the vertices", capacity, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX));
            self.vertex_capacity = capacity;
            self.vertices_used = 0;
        }
        let first_vertex = self.vertices_used as u32;
        self.jobs.push(Job { first_particle: self.particles_used as u32, count, first_vertex, sum, difference, vertices: self.vertices.clone() });
        self.staged.extend_from_slice(&particles[..count as usize * PARTICLE as usize]);
        self.particles_used += count as u64;
        self.vertices_used += count as u64 * 4;
        self.systems += 1;
        self.particles += count as u64;
        (self.vertices.clone(), first_vertex as i32)
    }

    /// Writes the staged particles and records the compute passes of the jobs that wait, into
    /// the frame's commands. Before the draws of the run that uses them.
    pub(crate) fn flush(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, commands: &FrameCommands) {
        if self.jobs.is_empty() {
            return;
        }
        let end = self.staged_from + self.staged.len() as u64;
        if end > self.input_capacity {
            // A larger input for the frame: the particles of the frame so far are written
            // again from the start, so every job of this flush reads one buffer.
            let capacity = end.next_power_of_two();
            self.input = storage(device, "particle quads, the particles", capacity, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
            self.input_capacity = capacity;
            self.rebase_jobs();
            queue.write_buffer(&self.input, 0, &self.staged);
        } else if !self.staged.is_empty() {
            queue.write_buffer(&self.input, self.staged_from, &self.staged);
        }
        self.staged_from += self.staged.len() as u64;
        self.staged.clear();
        let needed = self.jobs.len() as u64 * JOB_STRIDE;
        if needed > self.jobs_capacity {
            self.jobs_capacity = needed.next_power_of_two();
            self.jobs_buffer = storage(device, "particle quads, the systems", self.jobs_capacity, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        }
        let mut values = vec![0u8; needed as usize];
        for (n, job) in self.jobs.iter().enumerate() {
            let at = n * JOB_STRIDE as usize;
            let words: [u32; 12] = [
                job.sum[0].to_bits(),
                job.sum[1].to_bits(),
                job.sum[2].to_bits(),
                0,
                job.difference[0].to_bits(),
                job.difference[1].to_bits(),
                job.difference[2].to_bits(),
                0,
                job.first_particle,
                job.count,
                job.first_vertex,
                0,
            ];
            values[at..at + 48].copy_from_slice(bytemuck::cast_slice(&words));
        }
        queue.write_buffer(&self.jobs_buffer, 0, &values);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("particle quads") });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("particle quads"), timestamp_writes: None });
            pass.set_pipeline(&self.pipeline);
            let mut bound: Option<(Rc<wgpu::Buffer>, wgpu::BindGroup)> = None;
            for (n, job) in self.jobs.iter().enumerate() {
                if !bound.as_ref().is_some_and(|(buffer, _)| Rc::ptr_eq(buffer, &job.vertices)) {
                    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("particle quads"),
                        layout: &self.layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: self.input.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: job.vertices.as_entire_binding() },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &self.jobs_buffer, offset: 0, size: core::num::NonZeroU64::new(48) }),
                            },
                        ],
                    });
                    bound = Some((job.vertices.clone(), group));
                }
                if let Some((_, group)) = bound.as_ref() {
                    pass.set_bind_group(0, group, &[(n as u64 * JOB_STRIDE) as u32]);
                }
                pass.dispatch_workgroups(job.count.div_ceil(GROUP), 1, 1);
            }
        }
        commands.defer(encoder.finish());
        self.jobs.clear();
    }

    /// After the input grew: the jobs of the flush read from the new buffer, which gets the
    /// staged particles at the start (the jobs of earlier flushes keep the old buffer in
    /// their recorded passes).
    fn rebase_jobs(&mut self) {
        let base = self.jobs.first().map_or(0, |job| job.first_particle);
        for job in &mut self.jobs {
            job.first_particle -= base;
        }
        self.staged_from = 0;
        self.particles_used -= base as u64;
    }
}
