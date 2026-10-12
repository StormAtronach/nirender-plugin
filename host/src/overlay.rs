//! The debug overlay of the live mode: the frame rate, the long frames, and the memory of the
//! game and of this program, as a panel on the picture of the window. The game turns it on
//! and off with an `OVERLAY` record, which also has the numbers of the game process. The
//! panel is drawn into the picture of the window only: the back buffer, the picture files
//! and the capture files do not have it. It takes no input.
//!
//! The text and the graph are egui's, drawn by its wgpu renderer.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::wgpu;

/// The frames of the graph, and of the count of the long frames.
const HISTORY: usize = 600;
/// A long frame: more than this many times the mean of the frames in the graph.
const LONG: f32 = 2.0;

/// What the game process says about itself, from the `OVERLAY` record of the plugin.
#[derive(Clone, Copy, Default)]
struct Game {
    working_set: u64,
    private: u64,
    address_used: u64,
    address_total: u64,
    largest_free: u64,
    /// The time of the game thread for each frame, in ms: in the stream, in the ports of
    /// the renderer, and in waits for this program.
    stream_ms: f32,
    ports_ms: f32,
    wait_ms: f32,
}

/// The numbers of the panel. They change two times in a second, so that they can be read.
#[derive(Default)]
struct Shown {
    rate: f32,
    mean_ms: f32,
    worst_second_ms: f32,
    worst_ms: f32,
    long_frames: usize,
    waited: f32,
    draws: f32,
    working_set: u64,
    private: u64,
    gpu: Option<(u64, u64)>,
}

pub struct Overlay {
    on: bool,
    context: egui::Context,
    renderer: egui_wgpu::Renderer,
    game: Option<Game>,
    /// The end of the last frame, and the time of each of the last frames in ms.
    last: Option<Instant>,
    history: VecDeque<f32>,
    /// The start of the part of a second in progress, with the counts at its start.
    part: Instant,
    part_frames: u32,
    part_waited: Duration,
    part_draws: u64,
    part_worst: f32,
    /// The parts since the last read of the memory: one read in a second.
    parts: u32,
    shown: Shown,
}

#[repr(C)]
#[derive(Default)]
struct ProcessMemory {
    size: u32,
    page_faults: u32,
    peak_working_set: usize,
    working_set: usize,
    quota: [usize; 4],
    page_file: usize,
    peak_page_file: usize,
    private: usize,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> isize;
    fn K32GetProcessMemoryInfo(process: isize, counters: *mut ProcessMemory, size: u32) -> i32;
}

/// The working set and the private bytes of this process.
fn process_memory() -> (u64, u64) {
    let mut memory = ProcessMemory { size: core::mem::size_of::<ProcessMemory>() as u32, ..Default::default() };
    // SAFETY: the structure has the size that it says.
    if unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut memory, memory.size) } == 0 {
        return (0, 0);
    }
    (memory.working_set as u64, memory.private as u64)
}

fn megabytes(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

impl Overlay {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Overlay {
        let context = egui::Context::default();
        context.set_visuals(egui::Visuals::dark());
        Overlay {
            on: false,
            context,
            renderer: egui_wgpu::Renderer::new(device, format, egui_wgpu::RendererOptions::default()),
            game: None,
            last: None,
            history: VecDeque::with_capacity(HISTORY),
            part: Instant::now(),
            part_frames: 0,
            part_waited: Duration::ZERO,
            part_draws: 0,
            part_worst: 0.0,
            parts: 0,
            shown: Shown::default(),
        }
    }

    /// An `OVERLAY` record: the first word turns the panel on or off; then the numbers of
    /// the game process (see `tag::OVERLAY`).
    pub fn record(&mut self, data: &[u8]) {
        if data.len() < 4 {
            return;
        }
        self.on = crate::word(data, 0) != 0;
        if data.len() >= 60 {
            let wide = |at: usize| u64::from_le_bytes(data[at..at + 8].try_into().unwrap());
            self.game = Some(Game {
                working_set: wide(8),
                private: wide(16),
                address_used: wide(24),
                address_total: wide(32),
                largest_free: wide(40),
                stream_ms: crate::float(data, 48),
                ports_ms: crate::float(data, 52),
                wait_ms: crate::float(data, 56),
            });
        }
    }

    /// A frame went to the window. `waited` and `draws` are the totals of the program: the
    /// time it waited for the game, and its draws.
    pub fn frame(&mut self, device: &wgpu::Device, waited: Duration, draws: u64) {
        let now = Instant::now();
        if let Some(last) = self.last.replace(now) {
            let ms = (now - last).as_secs_f32() * 1000.0;
            if self.history.len() == HISTORY {
                self.history.pop_front();
            }
            self.history.push_back(ms);
            self.part_worst = self.part_worst.max(ms);
        }
        self.part_frames += 1;
        let seconds = (now - self.part).as_secs_f32();
        if seconds < 0.5 {
            return;
        }
        let frames = self.part_frames.max(1) as f32;
        let mean = self.history.iter().sum::<f32>() / self.history.len().max(1) as f32;
        self.shown.rate = frames / seconds;
        self.shown.mean_ms = seconds * 1000.0 / frames;
        self.shown.worst_second_ms = self.part_worst;
        self.shown.worst_ms = self.history.iter().copied().fold(0.0, f32::max);
        self.shown.long_frames = self.history.iter().filter(|&&ms| ms > LONG * mean).count();
        self.shown.waited = (waited - self.part_waited).as_secs_f32() * 100.0 / seconds;
        self.shown.draws = (draws - self.part_draws) as f32 / frames;
        self.part = now;
        self.part_frames = 0;
        self.part_waited = waited;
        self.part_draws = draws;
        self.part_worst = 0.0;
        // The memory: one read in a second, and only for a panel that shows. The report of
        // the GPU memory goes through all allocations.
        self.parts += 1;
        if self.on && self.parts >= 2 {
            self.parts = 0;
            (self.shown.working_set, self.shown.private) = process_memory();
            self.shown.gpu = device.generate_allocator_report().map(|report| (report.total_allocated_bytes, report.total_reserved_bytes));
        }
    }

    /// Draws the panel into the picture of the window, when it is on.
    pub fn draw(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, size: (u32, u32)) {
        if !self.on {
            return;
        }
        // The text has the same part of the height of a picture of any size: 1 at 1080 lines.
        let scale = (size.1 as f32 / 1080.0).max(0.75);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0 as f32 / scale, size.1 as f32 / scale))),
            ..Default::default()
        };
        self.context.set_pixels_per_point(scale);
        let (shown, game, history) = (&self.shown, self.game, &self.history);
        let output = self.context.run_ui(input, |ui| {
            egui::Area::new(egui::Id::new("nirender overlay")).fixed_pos(egui::pos2(8.0, 8.0)).show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).fill(egui::Color32::from_black_alpha(190)).show(ui, |ui| {
                    ui.style_mut().override_font_id = Some(egui::FontId::monospace(13.0));
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    ui.label(format!("{:6.1} frames in a second   {:6.2} ms", shown.rate, shown.mean_ms));
                    ui.label(format!("worst frame {:6.1} ms in the last half second, {:6.1} ms in the last {} frames", shown.worst_second_ms, shown.worst_ms, history.len()));
                    ui.label(format!("long frames {:3} of {} (more than {LONG} times the mean)", shown.long_frames, history.len()));
                    graph(ui, history);
                    ui.label(format!("renderer    waits for the game {:3.0} % of the time, {:.0} draws in a frame", shown.waited, shown.draws));
                    ui.label(format!("            memory {:6.0} MB in use, {:6.0} MB private", megabytes(shown.working_set), megabytes(shown.private)));
                    match shown.gpu {
                        Some((allocated, reserved)) => ui.label(format!("            GPU    {:6.0} MB in use, {:6.0} MB reserved", megabytes(allocated), megabytes(reserved))),
                        None => ui.label("            GPU    no report from this backend"),
                    };
                    match game {
                        Some(game) => {
                            ui.label(format!("game        memory {:6.0} MB in use, {:6.0} MB private", megabytes(game.working_set), megabytes(game.private)));
                            ui.label(format!(
                                "            address space {:6.0} of {:.0} MB, largest free block {:.0} MB",
                                megabytes(game.address_used),
                                megabytes(game.address_total),
                                megabytes(game.largest_free)
                            ));
                            ui.label(format!(
                                "            game thread in a frame: renderer {:5.2} ms (stream {:5.2} ms), waits {:5.2} ms",
                                game.ports_ms, game.stream_ms, game.wait_ms
                            ));
                        }
                        None => {
                            ui.label("game        no numbers yet");
                        }
                    }
                });
            });
        });
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                self.renderer.update_texture(device, queue, *id, delta);
            }
        }
        let jobs = self.context.tessellate(output.shapes, output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [size.0, size.1], pixels_per_point: output.pixels_per_point };
        // The buffers of a paint callback: the panel has none.
        let _ = self.renderer.update_buffers(device, queue, encoder, &jobs, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("overlay"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &jobs, &screen);
        }
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}

/// The time of each frame of the history as a bar: the height of the graph is 4 times the
/// mean, so a long frame stands out and a frame at the mean is a quarter of the height.
fn graph(ui: &mut egui::Ui, history: &VecDeque<f32>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(HISTORY as f32, 48.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(120));
    let mean = history.iter().sum::<f32>() / history.len().max(1) as f32;
    let top = (4.0 * mean).max(0.001);
    for (i, &ms) in history.iter().enumerate() {
        let x = rect.left() + i as f32 + 0.5;
        let height = (ms / top).min(1.0) * rect.height();
        let color = if ms > LONG * mean { egui::Color32::from_rgb(240, 110, 90) } else { egui::Color32::from_rgb(120, 200, 140) };
        painter.line_segment([egui::pos2(x, rect.bottom()), egui::pos2(x, rect.bottom() - height)], egui::Stroke::new(1.0, color));
    }
}
