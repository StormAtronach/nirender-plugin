//! The GPU objects that our code makes, by kind and by the line of code that makes them (the
//! owner's D288). A steady frame (no new content of the game, no load, no change of a setting)
//! must make none: an object made in each frame whose inputs did not change is work for
//! nothing, and on Direct3D 12 a bind group of the texture arrays holds 16384 places of a GPU
//! heap of 1,000,000 until the GPU has run its frame (risk 79: a replay lost the device).
//!
//! The code makes its objects through [`Create`] (on the device) and [`CreateView`] (on a
//! texture): `counted_*` in place of `create_*`. Each call counts its kind and its line
//! (`#[track_caller]`). [`end_frame`] closes a frame: in a steady frame that made objects, it
//! gives a warning line with the kinds and the lines, the first 10 times and then each 600th.
//! The report ([`report_lines`]) gives the totals, and the gate fails a replay whose steady
//! frames made objects. `tools/checks.py` refuses a `create_*` call of the device or a
//! texture outside this file.
//!
//! wgpu's own counters (the feature `counters`, `gpu::memory_lines`) count the objects that
//! live, not the objects made: an object made and freed in each frame leaves them flat.

use std::panic::Location;
use std::sync::Mutex;

use wgpu::util::DeviceExt;

/// The kinds of GPU objects counted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    BindGroup,
    BindGroupLayout,
    PipelineLayout,
    Buffer,
    Texture,
    View,
    Sampler,
    RenderPipeline,
    ComputePipeline,
    ShaderModule,
    QuerySet,
}

impl Kind {
    /// The name in the log lines.
    pub fn name(self) -> &'static str {
        match self {
            Kind::BindGroup => "bind group",
            Kind::BindGroupLayout => "bind group layout",
            Kind::PipelineLayout => "pipeline layout",
            Kind::Buffer => "buffer",
            Kind::Texture => "texture",
            Kind::View => "texture view",
            Kind::Sampler => "sampler",
            Kind::RenderPipeline => "render pipeline",
            Kind::ComputePipeline => "compute pipeline",
            Kind::ShaderModule => "shader module",
            Kind::QuerySet => "query set",
        }
    }
}

/// A kind and the line of code that made objects of it.
type Site = (Kind, &'static Location<'static>);

/// The steady frames that made objects whose line goes to the log: the first ones, then each
/// `WARN_EVERY`th.
const WARN_FIRST: u64 = 10;
const WARN_EVERY: u64 = 600;
/// The sites in a warning line and in the report.
const SITES_SHOWN: usize = 12;

/// The counts of the objects made: those of the frame in progress, and the totals.
#[derive(Default)]
pub struct Ledger {
    frame: Vec<(Site, u64)>,
    /// The objects made in steady frames, by site, since the start.
    steady: Vec<(Site, u64)>,
    frames: u64,
    steady_frames: u64,
    /// The steady frames that made objects.
    steady_frames_made: u64,
    /// The objects made in all frames.
    made: u64,
}

fn add(list: &mut Vec<(Site, u64)>, site: Site, count: u64) {
    match list.iter_mut().find(|(s, _)| s.0 == site.0 && s.1 == site.1) {
        Some((_, n)) => *n += count,
        None => list.push((site, count)),
    }
}

/// The sites, the most objects first: `count kind at file:line`.
fn site_list(list: &[(Site, u64)]) -> String {
    let mut sorted: Vec<&(Site, u64)> = list.iter().collect();
    sorted.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(a.0 .1.file().cmp(b.0 .1.file()))
            .then(a.0 .1.line().cmp(&b.0 .1.line()))
    });
    let shown: Vec<String> = sorted
        .iter()
        .take(SITES_SHOWN)
        .map(|((kind, at), n)| format!("{n} {} at {}:{}", kind.name(), at.file().replace('\\', "/"), at.line()))
        .collect();
    let more = sorted.len().saturating_sub(SITES_SHOWN);
    if more == 0 {
        shown.join(", ")
    } else {
        format!("{}, and {more} other sites", shown.join(", "))
    }
}

impl Ledger {
    pub const fn new() -> Ledger {
        Ledger {
            frame: Vec::new(),
            steady: Vec::new(),
            frames: 0,
            steady_frames: 0,
            steady_frames_made: 0,
            made: 0,
        }
    }

    /// An object of `kind` made at `at`.
    pub fn note(&mut self, kind: Kind, at: &'static Location<'static>) {
        add(&mut self.frame, (kind, at), 1);
        self.made += 1;
    }

    /// The end of a frame. `steady`: the frame had no new content, no load and no change of a
    /// setting. A steady frame that made objects gives its warning line, when it is one of the
    /// first 10 such frames or one of each 600th.
    pub fn end_frame(&mut self, steady: bool) -> Option<String> {
        self.frames += 1;
        let frame = std::mem::take(&mut self.frame);
        if !steady {
            return None;
        }
        self.steady_frames += 1;
        if frame.is_empty() {
            return None;
        }
        self.steady_frames_made += 1;
        for &(site, n) in &frame {
            add(&mut self.steady, site, n);
        }
        let n = self.steady_frames_made;
        (n <= WARN_FIRST || n.is_multiple_of(WARN_EVERY)).then(|| {
            let total: u64 = frame.iter().map(|(_, n)| n).sum();
            format!(
                "gpu objects: warning: frame {} is steady and made {total} GPU objects (steady frame {n} that made objects): {}",
                self.frames,
                site_list(&frame)
            )
        })
    }

    /// The steady frames that made objects.
    pub fn steady_frames_made(&self) -> u64 {
        self.steady_frames_made
    }

    /// The report: the frames, the steady ones, those that made objects, and their sites.
    pub fn report_lines(&self) -> Vec<String> {
        let steady_made: u64 = self.steady.iter().map(|(_, n)| n).sum();
        let mut lines = vec![format!(
            "gpu objects: {} made in {} frames; {} steady frames, {} of them made objects ({} objects)",
            self.made, self.frames, self.steady_frames, self.steady_frames_made, steady_made
        )];
        if !self.steady.is_empty() {
            lines.push(format!("gpu objects of the steady frames: {}", site_list(&self.steady)));
        }
        lines
    }
}

static LEDGER: Mutex<Ledger> = Mutex::new(Ledger::new());

/// An object made: the counts of the program.
pub fn note(kind: Kind, at: &'static Location<'static>) {
    if let Ok(mut ledger) = LEDGER.lock() {
        ledger.note(kind, at);
    }
}

/// The end of a frame of the program ([`Ledger::end_frame`]).
pub fn end_frame(steady: bool) -> Option<String> {
    LEDGER.lock().ok().and_then(|mut ledger| ledger.end_frame(steady))
}

/// The report of the program ([`Ledger::report_lines`]).
pub fn report_lines() -> Vec<String> {
    LEDGER.lock().map(|ledger| ledger.report_lines()).unwrap_or_default()
}

/// The objects of the device, counted ([`note`]): `counted_*` in place of `create_*`.
pub trait Create {
    fn counted_bind_group(&self, descriptor: &wgpu::BindGroupDescriptor) -> wgpu::BindGroup;
    fn counted_bind_group_layout(&self, descriptor: &wgpu::BindGroupLayoutDescriptor) -> wgpu::BindGroupLayout;
    fn counted_pipeline_layout(&self, descriptor: &wgpu::PipelineLayoutDescriptor) -> wgpu::PipelineLayout;
    fn counted_buffer(&self, descriptor: &wgpu::BufferDescriptor) -> wgpu::Buffer;
    fn counted_buffer_init(&self, descriptor: &wgpu::util::BufferInitDescriptor) -> wgpu::Buffer;
    fn counted_texture(&self, descriptor: &wgpu::TextureDescriptor) -> wgpu::Texture;
    fn counted_texture_with_data(
        &self,
        queue: &wgpu::Queue,
        descriptor: &wgpu::TextureDescriptor,
        order: wgpu::util::TextureDataOrder,
        data: &[u8],
    ) -> wgpu::Texture;
    fn counted_sampler(&self, descriptor: &wgpu::SamplerDescriptor) -> wgpu::Sampler;
    fn counted_render_pipeline(&self, descriptor: &wgpu::RenderPipelineDescriptor) -> wgpu::RenderPipeline;
    fn counted_compute_pipeline(&self, descriptor: &wgpu::ComputePipelineDescriptor) -> wgpu::ComputePipeline;
    fn counted_shader_module(&self, descriptor: wgpu::ShaderModuleDescriptor) -> wgpu::ShaderModule;
    fn counted_query_set(&self, descriptor: &wgpu::QuerySetDescriptor) -> wgpu::QuerySet;
}

impl Create for wgpu::Device {
    #[track_caller]
    fn counted_bind_group(&self, descriptor: &wgpu::BindGroupDescriptor) -> wgpu::BindGroup {
        note(Kind::BindGroup, Location::caller());
        self.create_bind_group(descriptor)
    }
    #[track_caller]
    fn counted_bind_group_layout(&self, descriptor: &wgpu::BindGroupLayoutDescriptor) -> wgpu::BindGroupLayout {
        note(Kind::BindGroupLayout, Location::caller());
        self.create_bind_group_layout(descriptor)
    }
    #[track_caller]
    fn counted_pipeline_layout(&self, descriptor: &wgpu::PipelineLayoutDescriptor) -> wgpu::PipelineLayout {
        note(Kind::PipelineLayout, Location::caller());
        self.create_pipeline_layout(descriptor)
    }
    #[track_caller]
    fn counted_buffer(&self, descriptor: &wgpu::BufferDescriptor) -> wgpu::Buffer {
        note(Kind::Buffer, Location::caller());
        self.create_buffer(descriptor)
    }
    #[track_caller]
    fn counted_buffer_init(&self, descriptor: &wgpu::util::BufferInitDescriptor) -> wgpu::Buffer {
        note(Kind::Buffer, Location::caller());
        self.create_buffer_init(descriptor)
    }
    #[track_caller]
    fn counted_texture(&self, descriptor: &wgpu::TextureDescriptor) -> wgpu::Texture {
        note(Kind::Texture, Location::caller());
        self.create_texture(descriptor)
    }
    #[track_caller]
    fn counted_texture_with_data(
        &self,
        queue: &wgpu::Queue,
        descriptor: &wgpu::TextureDescriptor,
        order: wgpu::util::TextureDataOrder,
        data: &[u8],
    ) -> wgpu::Texture {
        note(Kind::Texture, Location::caller());
        self.create_texture_with_data(queue, descriptor, order, data)
    }
    #[track_caller]
    fn counted_sampler(&self, descriptor: &wgpu::SamplerDescriptor) -> wgpu::Sampler {
        note(Kind::Sampler, Location::caller());
        self.create_sampler(descriptor)
    }
    #[track_caller]
    fn counted_render_pipeline(&self, descriptor: &wgpu::RenderPipelineDescriptor) -> wgpu::RenderPipeline {
        note(Kind::RenderPipeline, Location::caller());
        self.create_render_pipeline(descriptor)
    }
    #[track_caller]
    fn counted_compute_pipeline(&self, descriptor: &wgpu::ComputePipelineDescriptor) -> wgpu::ComputePipeline {
        note(Kind::ComputePipeline, Location::caller());
        self.create_compute_pipeline(descriptor)
    }
    #[track_caller]
    fn counted_shader_module(&self, descriptor: wgpu::ShaderModuleDescriptor) -> wgpu::ShaderModule {
        note(Kind::ShaderModule, Location::caller());
        self.create_shader_module(descriptor)
    }
    #[track_caller]
    fn counted_query_set(&self, descriptor: &wgpu::QuerySetDescriptor) -> wgpu::QuerySet {
        note(Kind::QuerySet, Location::caller());
        self.create_query_set(descriptor)
    }
}

/// The views of a texture, counted ([`note`]).
pub trait CreateView {
    fn counted_view(&self, descriptor: &wgpu::TextureViewDescriptor) -> wgpu::TextureView;
}

impl CreateView for wgpu::Texture {
    #[track_caller]
    fn counted_view(&self, descriptor: &wgpu::TextureViewDescriptor) -> wgpu::TextureView {
        note(Kind::View, Location::caller());
        self.create_view(descriptor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn here() -> &'static Location<'static> {
        Location::caller()
    }

    #[test]
    fn a_steady_frame_that_makes_objects_gives_a_warning_with_its_sites() {
        let mut ledger = Ledger::new();
        let at = here();
        // A frame with new content may make objects: no warning, no count.
        ledger.note(Kind::BindGroup, at);
        assert_eq!(ledger.end_frame(false), None);
        // A steady frame that makes none: no warning.
        assert_eq!(ledger.end_frame(true), None);
        // A steady frame that makes two groups at one line and a buffer at another.
        ledger.note(Kind::BindGroup, at);
        ledger.note(Kind::BindGroup, at);
        let other = here();
        ledger.note(Kind::Buffer, other);
        let line = ledger.end_frame(true).expect("a warning");
        assert!(
            line.starts_with("gpu objects: warning: frame 3 is steady and made 3 GPU objects"),
            "{line}"
        );
        assert!(
            line.contains(&format!("2 bind group at {}:{}", at.file().replace('\\', "/"), at.line())),
            "{line}"
        );
        assert!(
            line.contains(&format!("1 buffer at {}:{}", other.file().replace('\\', "/"), other.line())),
            "{line}"
        );
        assert_eq!(ledger.steady_frames_made(), 1);
        let report = ledger.report_lines();
        assert_eq!(
            report[0],
            "gpu objects: 4 made in 3 frames; 2 steady frames, 1 of them made objects (3 objects)"
        );
        assert!(
            report[1].starts_with("gpu objects of the steady frames: 2 bind group at "),
            "{}",
            report[1]
        );
    }

    #[test]
    fn the_warnings_are_the_first_ten_and_then_each_600th() {
        let mut ledger = Ledger::new();
        let at = here();
        let mut warned = Vec::new();
        for frame in 1..=1300u64 {
            ledger.note(Kind::View, at);
            if ledger.end_frame(true).is_some() {
                warned.push(frame);
            }
        }
        assert_eq!(warned, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 600, 1200]);
        assert_eq!(ledger.steady_frames_made(), 1300);
    }

    #[test]
    fn a_method_of_a_trait_with_track_caller_names_the_line_of_its_caller() {
        // The way of `Create`: the site is the code that asked for the object, not the trait.
        trait Probe {
            fn probe(&self) -> &'static Location<'static>;
        }
        impl Probe for u8 {
            #[track_caller]
            fn probe(&self) -> &'static Location<'static> {
                Location::caller()
            }
        }
        let line = line!();
        let at = 0u8.probe();
        assert!(at.file().ends_with("objects.rs") && at.line() == line + 1, "{at:?}");
    }
}
