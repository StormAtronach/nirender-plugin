//! The live mode: the records come from a ring in shared memory while the game runs, and
//! the frames go into a window.

use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::platform::windows::WindowExtWindows;
use winit::raw_window_handle::{RawWindowHandle, Win32WindowHandle};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use super::{copy_picture, option, save_picture, switches, target_texture, Player, Renderer, Status, DEPTH_FORMAT};

extern "system" {
    fn OpenFileMappingA(access: u32, inherit: i32, name: *const u8) -> isize;
    fn MapViewOfFile(mapping: isize, access: u32, offset_high: u32, offset_low: u32, bytes: usize) -> *mut u8;
    fn IsWindow(window: isize) -> i32;
    fn OpenEventA(access: u32, inherit: i32, name: *const u8) -> isize;
    fn WaitForSingleObject(handle: isize, milliseconds: u32) -> u32;
}

/// The head of the shared memory. See `capture.rs` of the plugin.
const RING_MAGIC: u32 = 0x474E_5231;
const RING_HEAD: usize = 64;
const RING_WRITTEN: usize = 8;
const RING_READ: usize = 16;
const RING_CLOSED: usize = 24;
/// The classes of the game that the helper draws itself, for the plugin ([`Player::takes`]).
const RING_TAKES: usize = 32;
/// 1 while this program waits for a record: the plugin sets the event of the ring
/// after a write only then.
const RING_WAITING: usize = 36;
/// The reach of the shadow map for the plugin's animation policy (the bits of an f32).
const RING_SHADOW_REACH: usize = 40;

struct Ring {
    base: *mut u8,
    capacity: u64,
    /// The event the plugin sets when it writes while this program waits; 0 with a
    /// plugin that has none (this program then yields instead).
    event: isize,
}

impl Ring {
    /// Opens the ring that the plugin made. Waits for it for some seconds.
    fn open(name: &str) -> Ring {
        let mut text = name.as_bytes().to_vec();
        text.push(0);
        let start = Instant::now();
        loop {
            // FILE_MAP_ALL_ACCESS
            let mapping = unsafe { OpenFileMappingA(0xF001F, 0, text.as_ptr()) };
            if mapping != 0 {
                let base = unsafe { MapViewOfFile(mapping, 0xF001F, 0, 0, 0) };
                assert!(!base.is_null(), "the shared memory does not map");
                assert_eq!(unsafe { (base as *const u32).read() }, RING_MAGIC, "not a ring of the plugin");
                let capacity = unsafe { (base.add(4) as *const u32).read() } as u64;
                // EVENT_ALL_ACCESS.
                let mut event_name = format!("{name}-records").into_bytes();
                event_name.push(0);
                let event = unsafe { OpenEventA(0x1F0003, 0, event_name.as_ptr()) };
                return Ring { base, capacity, event };
            }
            assert!(start.elapsed() < Duration::from_secs(60), "no shared memory with the name {name}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Tells the plugin the classes of the game that the helper draws itself, and the
    /// reach of the shadow map.
    fn set_takes(&self, classes: u32, shadow_reach: f32) {
        unsafe {
            (*(self.base.add(RING_TAKES) as *const std::sync::atomic::AtomicU32)).store(classes, Ordering::Release);
            (*(self.base.add(RING_SHADOW_REACH) as *const std::sync::atomic::AtomicU32)).store(shadow_reach.to_bits(), Ordering::Release);
        }
    }

    fn counter(&self, offset: usize) -> &AtomicU64 {
        unsafe { &*(self.base.add(offset) as *const AtomicU64) }
    }

    fn closed(&self) -> bool {
        unsafe { (self.base.add(RING_CLOSED) as *const u32).read_volatile() != 0 }
    }

    /// Waits for a record, 1 ms at most: says so in the head of the ring, looks once
    /// more (a write between the look and the wait would be missed otherwise), then
    /// waits on the event, which the plugin sets after a write while the word is set.
    /// The store of the word and the load of the count are SeqCst, as the plugin's
    /// store of the count and load of the word are. Without the event it yields.
    fn wait_for_record(&self) {
        if self.event == 0 {
            std::thread::yield_now();
            return;
        }
        let waiting = unsafe { &*(self.base.add(RING_WAITING) as *const std::sync::atomic::AtomicU32) };
        waiting.store(1, Ordering::SeqCst);
        let read = self.counter(RING_READ).load(Ordering::Relaxed);
        if self.counter(RING_WRITTEN).load(Ordering::SeqCst) - read < 8 {
            unsafe { WaitForSingleObject(self.event, 1) };
        }
        waiting.store(0, Ordering::SeqCst);
    }

    fn copy_out(&self, at: u64, to: &mut [u8]) {
        let start = (at % self.capacity) as usize;
        let first = to.len().min(self.capacity as usize - start);
        unsafe {
            std::ptr::copy_nonoverlapping(self.base.add(RING_HEAD + start), to.as_mut_ptr(), first);
            std::ptr::copy_nonoverlapping(self.base.add(RING_HEAD), to.as_mut_ptr().add(first), to.len() - first);
        }
    }

    /// Takes the next record out of the ring into `data`. Returns its tag, or [`None`] when
    /// the ring has no complete record.
    fn pop(&self, data: &mut Vec<u8>) -> Option<u32> {
        let read = self.counter(RING_READ).load(Ordering::Relaxed);
        let available = self.counter(RING_WRITTEN).load(Ordering::Acquire) - read;
        if available < 8 {
            return None;
        }
        let mut head = [0u8; 8];
        self.copy_out(read, &mut head);
        let tag = u32::from_le_bytes(head[0..4].try_into().unwrap());
        let length = u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize;
        data.resize(length, 0);
        self.copy_out(read + 8, data);
        self.counter(RING_READ).store(read + 8 + length as u64, Ordering::Release);
        Some(tag)
    }
}

/// The reply memory. See `capture.rs` of the plugin.
const REPLY_HEAD: usize = 16;

struct Reply {
    base: *mut u8,
}

impl Reply {
    fn open(name: &str) -> Option<Reply> {
        let mut text = format!("{name}.reply").into_bytes();
        text.push(0);
        let mapping = unsafe { OpenFileMappingA(0xF001F, 0, text.as_ptr()) };
        if mapping == 0 {
            return None;
        }
        let base = unsafe { MapViewOfFile(mapping, 0xF001F, 0, 0, 0) };
        (!base.is_null()).then_some(Reply { base })
    }

    /// The game made the memory large enough for its back buffer and its textures.
    fn answer(&self, request: u32, width: u32, height: u32, pixels: &[u8]) {
        unsafe {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), self.base.add(REPLY_HEAD), pixels.len());
            (self.base.add(4) as *mut u32).write_volatile(width);
            (self.base.add(8) as *mut u32).write_volatile(height);
            (*(self.base as *const std::sync::atomic::AtomicU32)).store(request, Ordering::Release);
        }
    }
}

/// The tile memory of a stream: shared memory with the name of the ring and `.tiles`, which
/// the plugin makes. Slots of a head (the ticket, written last; the width; the height; one
/// unused word) and the pixels of a tile of 512 x 512 at most. The ticket picks the slot. See
/// `capture.rs` of the plugin.
const TILE_SLOTS: usize = 8;
const TILE_BYTES: usize = 512 * 512 * 4;
const TILE_HEAD: usize = 16;

struct Tiles {
    base: *mut u8,
}

impl Tiles {
    fn open(name: &str) -> Option<Tiles> {
        let mut text = format!("{name}.tiles").into_bytes();
        text.push(0);
        let mapping = unsafe { OpenFileMappingA(0xF001F, 0, text.as_ptr()) };
        if mapping == 0 {
            return None;
        }
        let base = unsafe { MapViewOfFile(mapping, 0xF001F, 0, 0, 0) };
        (!base.is_null()).then_some(Tiles { base })
    }

    fn answer(&self, ticket: u32, width: u32, height: u32, pixels: &[u8]) {
        let (width, height, pixels) = if pixels.len() > TILE_BYTES { (0, 0, &[][..]) } else { (width, height, pixels) };
        unsafe {
            let slot = self.base.add((ticket as usize % TILE_SLOTS) * (TILE_HEAD + TILE_BYTES));
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), slot.add(TILE_HEAD), pixels.len());
            (slot.add(4) as *mut u32).write_volatile(width);
            (slot.add(8) as *mut u32).write_volatile(height);
            (*(slot as *const std::sync::atomic::AtomicU32)).store(ticket, Ordering::Release);
        }
    }
}

/// The message memory of a stream: shared memory with the name of the ring and
/// `.messages`, which the plugin makes. The head: [`MESSAGES_MAGIC`], the number of data
/// bytes, the bytes that this program wrote (u64), the bytes that the game read (u64). Then
/// the data: records of the number, the length and the bytes, which can go around the
/// end. See `capture.rs` of the plugin.
const MESSAGES_MAGIC: u32 = 0x4753_4D31;
const MESSAGES_HEAD: usize = 32;

struct Messages {
    base: *mut u8,
    capacity: u64,
    /// A message did not fit: said one time.
    full: bool,
}

impl Messages {
    fn open(name: &str) -> Option<Messages> {
        let mut text = format!("{name}.messages").into_bytes();
        text.push(0);
        let mapping = unsafe { OpenFileMappingA(0xF001F, 0, text.as_ptr()) };
        if mapping == 0 {
            return None;
        }
        let base = unsafe { MapViewOfFile(mapping, 0xF001F, 0, 0, 0) };
        if base.is_null() || unsafe { (base as *const u32).read_volatile() } != MESSAGES_MAGIC {
            return None;
        }
        let capacity = unsafe { (base.add(4) as *const u32).read_volatile() } as u64;
        Some(Messages { base, capacity, full: false })
    }

    fn counter(&self, offset: usize) -> &std::sync::atomic::AtomicU64 {
        unsafe { &*(self.base.add(offset) as *const std::sync::atomic::AtomicU64) }
    }

    /// Puts one message into the memory, or drops it when the game has not read enough.
    fn put(&mut self, id: u32, bytes: &[u8]) {
        let total = 8 + bytes.len() as u64;
        let written = self.counter(8).load(Ordering::Relaxed);
        let read = self.counter(16).load(Ordering::Acquire);
        if total > self.capacity - (written - read) {
            if !self.full {
                self.full = true;
                println!("messages: no room for a message {id} of {} bytes: the game does not read; it is dropped", bytes.len());
            }
            return;
        }
        let mut at = written;
        for part in [&id.to_le_bytes()[..], &(bytes.len() as u32).to_le_bytes()[..], bytes] {
            for &byte in part {
                unsafe { self.base.add(MESSAGES_HEAD + (at % self.capacity) as usize).write(byte) };
                at += 1;
            }
        }
        self.counter(8).store(at, Ordering::Release);
    }
}

struct App {
    ring: Ring,
    reply: Option<Reply>,
    tiles: Option<Tiles>,
    messages: Option<Messages>,
    extension: Option<Box<dyn super::Extension>>,
    backends: wgpu::Backends,
    frames_wanted: u32,
    /// The size of the back buffer of the game, and its window when the host goes into it.
    size: (u32, u32),
    /// The head of the stream: the player takes it first, for the abilities of the stream.
    header: Vec<u8>,
    parent: Option<isize>,
    shot: Option<String>,
    shot_frame: u32,
    /// Frames to save a picture of, and the start of the names of the files.
    shots_at: Vec<u32>,
    shots_prefix: String,
    /// Each picture file also as the window shows it (`--window-shots`), for a test of the
    /// text on the window without a picture of the screen.
    window_shots: bool,
    /// A picture of a frame with a draw of the film player, or [`None`], and which such
    /// draw (the first is 1).
    shot_film: Option<String>,
    shot_film_frame: u32,
    window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    configuration: Option<wgpu::SurfaceConfiguration>,
    player: Option<Player>,
    /// The debug overlay: a panel on the picture of the window, when the game asks.
    overlay: Option<super::overlay::Overlay>,
    depth: Option<Rc<wgpu::TextureView>>,
    /// The back buffer of the game: the frames go into it, and then into the window. It
    /// keeps the last frame for a pixel read of the game.
    back_buffer: Option<wgpu::Texture>,
    /// A frame of the game is in progress.
    in_frame: bool,
    /// The frames that the window did not take: its surface gave no picture.
    not_shown: u32,
    /// The status of the surface when the window last gave no picture, or an empty text.
    surface_status: &'static str,
    data: Vec<u8>,
    frames: u32,
    first_frame: Option<Instant>,
    waited: Duration,
    /// All the time with no record to read, for the debug overlay: `waited` has only
    /// the waits that went back to the events of the window.
    idle: Duration,
    /// The time of the last record, and of the last picture with the sign that the game
    /// sends no frames, with the count of these pictures.
    last_record: Instant,
    sign_at: Instant,
    sign_step: u32,
    records: u64,
    bytes: u64,
    shot_done: bool,
    finished: bool,
}

/// The time without a record after which the window shows the sign that the game sends no
/// frames, and the time of one step of the sign.
const SIGN_AFTER: Duration = Duration::from_millis(500);
const SIGN_STEP: Duration = Duration::from_millis(125);

/// Gets the picture of the window for a frame. A surface that is outdated or lost is
/// configured again, one time for each frame. The log has each change of the status of a
/// surface that gives no picture.
fn acquire(surface: &wgpu::Surface, device: &wgpu::Device, configuration: Option<&wgpu::SurfaceConfiguration>, status: &mut &'static str, frame: u32) -> Option<wgpu::SurfaceTexture> {
    // A lost device takes no picture: a picture of the window that is acquired and never
    // shown ends the program at its end (wgpu-hal: "Trying to destroy a
    // SwapchainAcquireSemaphore that is still in use", a test of 2026-10-21).
    if super::gpu::lost() {
        return None;
    }
    let mut configured_again = false;
    loop {
        let (name, stale) = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(picture) | wgpu::CurrentSurfaceTexture::Suboptimal(picture) => {
                if !status.is_empty() {
                    println!("window: the surface gives a picture again at frame {frame}");
                    *status = "";
                }
                return Some(picture);
            }
            wgpu::CurrentSurfaceTexture::Outdated => ("outdated", true),
            wgpu::CurrentSurfaceTexture::Lost => ("lost", true),
            wgpu::CurrentSurfaceTexture::Timeout => ("not ready in time", false),
            wgpu::CurrentSurfaceTexture::Occluded => ("hidden", false),
            wgpu::CurrentSurfaceTexture::Validation => ("not valid", false),
        };
        if let (true, false, Some(configuration)) = (stale, configured_again, configuration.filter(|c| c.width != 0 && c.height != 0)) {
            surface.configure(device, configuration);
            configured_again = true;
            continue;
        }
        if *status != name {
            println!("window: no picture at frame {frame}: the surface is {name}{}", if configured_again { ", also after it was configured again" } else { "" });
            *status = name;
        }
        return None;
    }
}

impl App {
    /// Sets the size of the picture in the window, and makes the depth buffer for it.
    fn configure(&mut self, width: u32, height: u32) {
        let (Some(surface), Some(configuration), Some(player)) = (self.surface.as_ref(), self.configuration.as_mut(), self.player.as_ref()) else { return };
        if width == 0 || height == 0 || (configuration.width, configuration.height) == (width, height) && self.depth.is_some() {
            return;
        }
        configuration.width = width;
        configuration.height = height;
        surface.configure(&player.renderer.device, configuration);
        // The back buffer has the size of the picture of the game.
        let (width, height) = self.size;
        // With multisampling, the back buffer is the resolve target of the frame, and the
        // player reads it back into the multisampled colour target after a write outside
        // the pass (`Player::load_color`).
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING;
        self.back_buffer = Some(target_texture(&player.renderer.device, width, height, player.renderer.target_format, usage));
        // An extension can read the depth in a pass that does not have it as a target.
        let depth = target_texture(&player.renderer.device, width, height, DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
        self.depth = Some(Rc::new(depth.create_view(&Default::default())));
    }

    /// Shows the last picture of the game again, with the sign that the game sends no
    /// frames. The game does this at its start and when it loads: the window then has a
    /// part that moves. The frame in progress stays as it is.
    fn show_sign(&mut self) {
        self.sign_at = Instant::now();
        self.sign_step += 1;
        let (Some(surface), Some(player), Some(overlay)) = (self.surface.as_ref(), self.player.as_ref(), self.overlay.as_mut()) else { return };
        let Some(back_buffer) = player.shown_picture() else { return };
        let Some(frame) = acquire(surface, &player.renderer.device, self.configuration.as_ref(), &mut self.surface_status, self.frames) else { return };
        let (device, queue) = (&player.renderer.device, &player.renderer.queue);
        let size = (frame.texture.width(), frame.texture.height());
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_texture(
            back_buffer.as_image_copy(),
            frame.texture.as_image_copy(),
            wgpu::Extent3d { width: back_buffer.width().min(size.0), height: back_buffer.height().min(size.1), depth_or_array_layers: 1 },
        );
        overlay.draw(device, queue, &mut encoder, &frame.texture.create_view(&Default::default()), size, Some(self.sign_step), &[], &Status::default());
        crate::gpu::submit(queue, [encoder.finish()]);
        queue.present(frame);
    }

    fn finish(&mut self, event_loop: &ActiveEventLoop, why: &str) {
        self.finished = true;
        // The picture of the window that is in progress goes away before the surface.
        if let Some(player) = self.player.as_mut().filter(|_| !super::gpu::lost()) {
            player.end_frame(|_| {});
        }
        self.in_frame = false;
        let seconds = self.first_frame.map_or(0.0, |start| start.elapsed().as_secs_f64());
        println!("end: {why}");
        println!(
            "frames {}, {:.1} frames in a second, {:.2} ms for each frame; the host waited for the game {:.0} % of the time",
            self.frames,
            self.frames as f64 / seconds.max(0.001),
            seconds * 1000.0 / self.frames.max(1) as f64,
            self.waited.as_secs_f64() * 100.0 / seconds.max(0.001)
        );
        println!("frames that the window did not take: {}", self.not_shown);
        if let Some(line) = self.overlay.as_mut().and_then(|overlay| overlay.age_line()) {
            println!("{line}");
        }
        if let Some(player) = self.player.as_ref() {
            println!("time for each frame, in ms:");
            for (name, time) in super::TIME_NAMES.iter().zip(player.renderer.counts.times) {
                println!("  {:6.3}  {name}", super::ticks::duration(time).as_secs_f64() * 1000.0 / self.frames.max(1) as f64);
            }
        }
        println!("records {}, {:.1} MB, {:.2} MB for each frame", self.records, self.bytes as f64 / 1e6, self.bytes as f64 / 1e6 / self.frames.max(1) as f64);
        if let Some(player) = self.player.as_ref() {
            player.print_counts();
        }
        event_loop.exit();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let mut attributes =
            Window::default_attributes().with_title("nirender host").with_inner_size(PhysicalSize::new(self.size.0, self.size.1)).with_resizable(false).with_active(false);
        if let Some(parent) = self.parent.and_then(std::num::NonZeroIsize::new) {
            // A child window of the window of the game, over all of it.
            let handle = RawWindowHandle::Win32(Win32WindowHandle::new(parent));
            attributes = unsafe { attributes.with_parent_window(Some(handle)) }.with_decorations(false).with_position(PhysicalPosition::new(0, 0));
        }
        let window = Arc::new(event_loop.create_window(attributes).expect("a window"));
        if self.parent.is_some() {
            // A window that is not enabled takes no mouse and no keys: they go to the game.
            window.set_enable(false);
        }
        let (renderer, surface) = Renderer::new(
            self.backends,
            Some(window.clone()),
            wgpu::TextureFormat::Bgra8Unorm,
            crate::extension_shaders(self.extension.as_deref()),
            self.extension.as_ref().map_or(0, |extension| extension.texture_slots()),
            switches().samples.unwrap_or_else(|| self.extension.as_ref().map_or(1, |extension| extension.samples())),
        );
        let surface = surface.expect("a surface");
        let size = window.inner_size();
        self.configuration = Some(wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
            format: renderer.target_format,
            width: 0,
            height: 0,
            // The host shows a frame when the game gives one. It does not wait for the screen.
            present_mode: wgpu::PresentMode::AutoNoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            color_space: Default::default(),
        });
        assert!(renderer.surface_usages.contains(wgpu::TextureUsages::COPY_DST), "the window does not take a copy of a picture");
        self.overlay = Some(super::overlay::Overlay::new(&renderer.device, renderer.target_format));
        let mut player = Player::new(renderer);
        // The abilities of the stream are in its head: the player reads them as a replay does.
        player.feed(super::tag::HEADER, &self.header);
        player.extension = self.extension.take();
        player.renderer.anisotropy = player.extension.as_ref().map_or(1, |extension| extension.anisotropy());
        player.film_shot = self.shot_film.take();
        player.film_shot_at = self.shot_film_frame;
        player.reply = self.reply.take().map(|reply| Box::new(move |request, width, height, pixels: &[u8]| reply.answer(request, width, height, pixels)) as _);
        player.tile_out = self.tiles.take().map(|tiles| Box::new(move |ticket, width, height, pixels: &[u8]| tiles.answer(ticket, width, height, pixels)) as _);
        self.player = Some(player);
        self.surface = Some(surface);
        self.window = Some(window);
        self.configure(size.width, size.height);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested if !self.finished => self.finish(event_loop, "the window closed"),
            WindowEvent::Resized(size) => self.configure(size.width, size.height),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Poll);
        if self.player.is_none() || self.finished {
            return;
        }
        // The window of the game went away: the game ended, and the window of the host
        // with it.
        if self.parent.is_some_and(|parent| unsafe { IsWindow(parent) } == 0) {
            return self.finish(event_loop, "the window of the game is gone");
        }
        let size = self.size;
        let now = self.window.as_ref().unwrap().inner_size();
        self.configure(now.width, now.height);
        let waiting = Instant::now();
        let mut idle_since: Option<Instant> = None;
        // The clock is read once in 256 spins: a read in each spin is too costly. The
        // first 256 looks for a record yield; after them this
        // program blocks on the event, so that the game wakes it only at a real gap
        // (between the frames), not after every record.
        let mut spins: u32 = 0;
        let mut looks: u32 = 0;
        loop {
            // The device is lost: nothing draws any more. The records are read and dropped,
            // so that the game does not wait for this program. A request that the game waits
            // for gets the answer "no pixels" at once.
            if super::gpu::lost() {
                if let Some(player) = self.player.as_mut() {
                    player.fail_tile_reads();
                }
                match self.ring.pop(&mut self.data) {
                    Some(tag) => {
                        if let Some(player) = self.player.as_mut() {
                            player.answer_lost(tag, &self.data);
                        }
                        continue;
                    }
                    None if self.ring.closed() => return self.finish(event_loop, "the game closed the stream"),
                    None if waiting.elapsed() > Duration::from_millis(20) => return,
                    None => {
                        std::thread::yield_now();
                        continue;
                    }
                }
            }
            // No frame before the size of the game is known: the records before that
            // make the resources only.
            // The frame goes into the back buffer of this program. The window is not a
            // part of it: this program reads the stream also when the window is hidden,
            // because the game waits for it.
            if !self.in_frame && self.depth.is_some() && size.0 != 0 {
                self.player.as_mut().unwrap().begin_frame(self.back_buffer.as_ref().unwrap(), self.depth.clone().unwrap(), true);
                self.in_frame = true;
            }
            let clock = super::ticks::Clock::now();
            let popped = self.ring.pop(&mut self.data);
            if popped.is_some() {
                self.player.as_mut().unwrap().renderer.counts.times[super::TIME_READ] += clock.elapsed();
            }
            let Some(tag) = popped else {
                if self.ring.closed() {
                    return self.finish(event_loop, "the game closed the stream");
                }
                // The tile reads that the GPU finished go to the game between the records.
                self.player.as_mut().unwrap().deliver_tiles();
                let since = *idle_since.get_or_insert_with(|| {
                    // The last record came just before this idle time.
                    self.last_record = Instant::now();
                    self.last_record
                });
                spins = spins.wrapping_add(1);
                if spins % 256 == 0 && waiting.elapsed() > Duration::from_millis(20) {
                    // Back to the events of the window.
                    self.waited += waiting.elapsed();
                    self.idle += since.elapsed();
                    if self.last_record.elapsed() > SIGN_AFTER && self.sign_at.elapsed() > SIGN_STEP {
                        self.show_sign();
                    }
                    return;
                }
                looks += 1;
                if looks < 256 {
                    std::thread::yield_now();
                } else {
                    self.ring.wait_for_record();
                }
                continue;
            };
            looks = 0;
            if let Some(since) = idle_since.take() {
                self.idle += since.elapsed();
            }
            self.records += 1;
            self.bytes += 8 + self.data.len() as u64;
            let player = self.player.as_mut().unwrap();
            let clock = super::ticks::Clock::now();
            // The times of the frame in the game, and the time of this read: for the age
            // of the picture in the debug overlay.
            let stamps = (tag == super::tag::FRAME_END && self.data.len() >= 16).then(|| {
                let wide = |at: usize| i64::from_le_bytes(self.data[at..at + 8].try_into().unwrap());
                (wide(0), wide(8), super::overlay::ticks())
            });
            let inside_before = super::sum_except(&player.renderer.counts.times, super::TIME_RECORDS);
            let frame_end = player.feed(tag, &self.data);
            self.ring.set_takes(player.takes, player.shadow_reach);
            let inside_after = super::sum_except(&player.renderer.counts.times, super::TIME_RECORDS);
            // The time of a record without the parts that are measured on their own.
            player.renderer.counts.times[super::TIME_RECORDS] += clock.elapsed().saturating_sub(inside_after - inside_before);
            if let (Some(overlay), Some(record)) = (self.overlay.as_mut(), player.overlay_record.take()) {
                overlay.record(&record);
            }
            // The age of the pictures of a step of a scenario, after the lines of its mark.
            if tag == super::tag::MARK {
                if let Some(line) = self.overlay.as_mut().and_then(|overlay| overlay.age_line()) {
                    println!("  {line}");
                }
            }
            if !frame_end {
                continue;
            }
            // The frame is complete. It goes into the window when the window takes one.
            if !std::mem::take(&mut self.in_frame) {
                return;
            }
            let clock = super::ticks::Clock::now();
            let frame = acquire(self.surface.as_ref().unwrap(), &player.renderer.device, self.configuration.as_ref(), &mut self.surface_status, self.frames);
            player.renderer.counts.times[super::TIME_ACQUIRE] += clock.elapsed();
            self.not_shown += frame.is_none() as u32;
            let in_window = frame.is_some();
            let blue_first = matches!(player.renderer.target_format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
            let take_shot = self.shot.is_some() && !self.shot_done && self.frames + 1 >= self.shot_frame;
            let back_buffer = self.back_buffer.as_ref().unwrap();
            let (width, height) = (back_buffer.width(), back_buffer.height());
            let device = player.renderer.device.clone();
            let queue = player.renderer.queue.clone();
            // The lines of the extension for the debug overlay, when it shows.
            let mut lines = match (self.overlay.as_ref(), player.extension.as_ref()) {
                (Some(overlay), Some(extension)) if overlay.is_on() => extension.overlay_lines(),
                _ => Vec::new(),
            };
            // The room of the texture arrays (`runtime.renderer_texture_slots`).
            if self.overlay.as_ref().is_some_and(|overlay| overlay.is_on()) {
                let r = &player.renderer;
                lines.insert(
                    0,
                    format!(
                        "textures    {} slots in the frame of a room of {}, {} kept ({} free); {} found no slot",
                        r.frame_texture_views.len(),
                        r.array_cap,
                        r.pinned_textures.len() - r.pinned_free.len(),
                        r.pinned_free.len(),
                        r.counts.over_array_cap
                    ),
                );
            }
            let status = player.extension.as_ref().map(|extension| extension.status()).unwrap_or_default();
            let overlay = self.overlay.as_mut();
            let mut read = None;
            player.end_frame(|encoder| {
                // The picture goes into the window: the part that the two have.
                if let Some(frame) = frame.as_ref() {
                    encoder.copy_texture_to_texture(
                        back_buffer.as_image_copy(),
                        frame.texture.as_image_copy(),
                        wgpu::Extent3d { width: width.min(frame.texture.width()), height: height.min(frame.texture.height()), depth_or_array_layers: 1 },
                    );
                    // The debug overlay goes on the picture of the window, not into the
                    // back buffer: a picture file and a pixel read do not have it.
                    if let Some(overlay) = overlay {
                        overlay.draw(&device, &queue, encoder, &frame.texture.create_view(&Default::default()), (frame.texture.width(), frame.texture.height()), None, &lines, &status);
                    }
                }
                if take_shot {
                    read = Some(copy_picture(&device, encoder, back_buffer, width, height));
                }
            });
            if let Some((buffer, row)) = read {
                save_picture(&device, &buffer, row, width, height, blue_first, self.shot.as_ref().unwrap());
                self.shot_done = true;
            }
            // The picture files that the game asked for in this frame, and the one that
            // the command line asked for.
            let mut shots = std::mem::take(&mut player.shots);
            if self.shots_at.contains(&(self.frames + 1)) {
                shots.push(format!("{}-{:05}.png", self.shots_prefix, self.frames + 1));
            }
            if !shots.is_empty() {
                let mut encoder = device.create_command_encoder(&Default::default());
                let (buffer, row) = copy_picture(&device, &mut encoder, back_buffer, width, height);
                player.renderer.upload_indirect(&player.frame_args);
                crate::gpu::submit(&player.renderer.queue, player.renderer.commands.take().into_iter().chain([encoder.finish()]));
                for path in &shots {
                    save_picture(&device, &buffer, row, width, height, blue_first, path);
                    buffer.unmap();
                }
                // The same pictures as the window shows them: the overlay is drawn on a
                // copy of the back buffer, as on the picture of the window.
                if let (true, Some(overlay)) = (self.window_shots, self.overlay.as_mut()) {
                    let copy = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("window picture"),
                        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: back_buffer.format(),
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    let mut encoder = device.create_command_encoder(&Default::default());
                    encoder.copy_texture_to_texture(back_buffer.as_image_copy(), copy.as_image_copy(), wgpu::Extent3d { width, height, depth_or_array_layers: 1 });
                    overlay.draw(&device, &queue, &mut encoder, &copy.create_view(&Default::default()), (width, height), None, &lines, &status);
                    let (buffer, row) = copy_picture(&device, &mut encoder, &copy, width, height);
                    crate::gpu::submit(&queue, [encoder.finish()]);
                    for path in &shots {
                        let window = match path.strip_suffix(".png") {
                            Some(stem) => format!("{stem}-window.png"),
                            None => format!("{path}-window.png"),
                        };
                        save_picture(&device, &buffer, row, width, height, blue_first, &window);
                        buffer.unmap();
                    }
                }
            }
            // The facts of the last world draw stand for a frame with no world.
            if player.extension.is_some() {
                let facts = player.world_facts(false);
                let commands = player.renderer.commands.clone();
                let scene = facts.view(&device, &queue, &commands, player.renderer.target_format);
                if let Some(extension) = player.extension.as_mut() {
                    extension.after_frame(&scene);
                }
            }
            // The messages of the extension for its part in the game process.
            if let (Some(messages), Some(extension)) = (self.messages.as_mut(), player.extension.as_mut()) {
                for (id, bytes) in extension.messages() {
                    messages.put(id, &bytes);
                }
            }
            let clock = super::ticks::Clock::now();
            if let Some(frame) = frame {
                player.renderer.queue.present(frame);
            }
            player.renderer.counts.times[super::TIME_PRESENT] += clock.elapsed();
            if let Some(overlay) = self.overlay.as_mut() {
                if let (Some((start, end, read)), true) = (stamps, in_window) {
                    overlay.age(start, end, read, super::overlay::ticks());
                }
                overlay.frame(&device, self.idle, player.renderer.counts.draws as u64);
            }
            self.frames += 1;
            self.first_frame.get_or_insert_with(Instant::now);
            if self.frames_wanted != 0 && self.frames >= self.frames_wanted {
                return self.finish(event_loop, "all the frames that were asked for");
            }
            return;
        }
    }
}

pub fn run(name: &str, backends: wgpu::Backends, args: &[String], extension: Option<Box<dyn super::Extension>>) {
    println!("{}", super::cores::prefer_performance_cores());
    let ring = Ring::open(name);
    println!("ring: {} MB", ring.capacity >> 20);
    // The first record has the size of the picture and the window of the game.
    let mut data = Vec::new();
    let start = Instant::now();
    let tag = loop {
        if let Some(tag) = ring.pop(&mut data) {
            break tag;
        }
        assert!(start.elapsed() < Duration::from_secs(60) && !ring.closed(), "no record in the ring");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(tag, super::tag::HEADER, "the first record is not the head of the stream");
    let size = (super::word(&data, 0).max(64), super::word(&data, 4).max(64));
    let game_window = if data.len() >= 12 { super::word(&data, 8) as isize } else { 0 };
    let embed = args.iter().any(|a| a == "--embed");
    println!("picture {} x {}, window of the game {game_window:#x}{}", size.0, size.1, if embed { ", the host goes into it" } else { "" });
    let mut app = App {
        ring,
        reply: Reply::open(name),
        tiles: Tiles::open(name),
        messages: Messages::open(name),
        extension,
        back_buffer: None,
        backends,
        frames_wanted: option(args, "--frames").and_then(|text| text.parse().ok()).unwrap_or(0),
        size,
        header: data.clone(),
        parent: embed.then_some(game_window),
        shot: option(args, "--shot"),
        shot_frame: option(args, "--shot-frame").and_then(|text| text.parse().ok()).unwrap_or(60),
        shots_at: option(args, "--shots-at").map(|text| text.split(',').filter_map(|n| n.parse().ok()).collect()).unwrap_or_default(),
        shots_prefix: option(args, "--shots-prefix").unwrap_or_else(|| "frame".into()),
        window_shots: args.iter().any(|argument| argument == "--window-shots"),
        shot_film: option(args, "--shot-film"),
        shot_film_frame: option(args, "--shot-film-frame").and_then(|text| text.parse().ok()).unwrap_or(120),
        window: None,
        surface: None,
        configuration: None,
        player: None,
        overlay: None,
        depth: None,
        in_frame: false,
        not_shown: 0,
        surface_status: "",
        data: Vec::new(),
        frames: 0,
        first_frame: None,
        waited: Duration::ZERO,
        idle: Duration::ZERO,
        last_record: Instant::now(),
        sign_at: Instant::now(),
        sign_step: 0,
        records: 0,
        bytes: 0,
        shot_done: false,
        finished: false,
    };
    EventLoop::new().expect("an event loop").run_app(&mut app).expect("the event loop");
}
