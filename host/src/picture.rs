//! The pictures of the renderer: the target textures, the copy of a picture into a buffer,
//! and the picture files (PNG, JPEG).

use super::*;
use crate::objects::{Create, CreateView};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A depth texture with its two views, made once with it: the whole texture (a target of a
/// resolve or of a merge) and its depth aspect (what a pass reads). A frame that uses the
/// texture again makes no view ([`crate::objects`]).
pub(crate) struct DepthTarget {
    pub(crate) texture: wgpu::Texture,
    pub(crate) whole: wgpu::TextureView,
    pub(crate) depth: wgpu::TextureView,
}

impl DepthTarget {
    pub(crate) fn new(texture: wgpu::Texture) -> DepthTarget {
        let whole = texture.counted_view(&Default::default());
        let depth = texture.counted_view(&wgpu::TextureViewDescriptor {
            aspect: wgpu::TextureAspect::DepthOnly,
            ..Default::default()
        });
        DepthTarget { texture, whole, depth }
    }
}

/// The size of a texture that the game draws into: each side the size that the game asked
/// for, cut to `largest`, the device's largest side of a 2D texture. A larger texture is an
/// error of the device, and each submission whose commands use it is lost whole, the frame's
/// other draws with it. The game samples the texture from 0 to 1, so a smaller texture keeps
/// the picture; each side is cut on its own, which keeps the most pixels.
pub(crate) fn target_size(asked: (u32, u32), largest: u32) -> (u32, u32) {
    let largest = largest.max(1);
    (asked.0.clamp(1, largest), asked.1.clamp(1, largest))
}

/// `pixels` (4 bytes for each pixel, the first row at the top) of a picture of the size `from`,
/// as a picture of the size `to`: each pixel of `to` takes the nearest pixel of `from`. For a
/// texture made smaller than the game asked ([`target_size`]): the game's pixels go in, and the
/// texture's pixels come back out, each at the other size.
pub(crate) fn resample_nearest(pixels: &[u8], from: (u32, u32), to: (u32, u32)) -> Vec<u8> {
    let (from_width, from_height) = (from.0 as usize, from.1 as usize);
    let (to_width, to_height) = (to.0 as usize, to.1 as usize);
    let mut out = Vec::with_capacity(to_width * to_height * 4);
    for y in 0..to_height {
        let source_row = y * from_height / to_height.max(1);
        for x in 0..to_width {
            let source = (source_row * from_width + x * from_width / to_width.max(1)) * 4;
            out.extend_from_slice(&pixels[source..source + 4]);
        }
    }
    out
}

pub(crate) fn target_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    multisampled_texture(device, width, height, format, usage, 1)
}

/// A target with `samples` samples for each pixel.
pub(crate) fn multisampled_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    samples: u32,
) -> wgpu::Texture {
    device.counted_texture(&wgpu::TextureDescriptor {
        label: Some(if samples > 1 {
            "multisampled render target"
        } else {
            "render target"
        }),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

/// Adds the copy of a picture into a new buffer to the commands. Returns the buffer and the
/// length of a row in it.
pub(crate) fn copy_picture(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> (wgpu::Buffer, usize) {
    // A read of the picture is a request of its frame, with a buffer of its own.
    crate::objects::changed("a picture");
    let row = (width as usize * 4).next_multiple_of(256);
    let buffer = device.counted_buffer(&wgpu::BufferDescriptor {
        label: Some("picture read"),
        size: (row * height as usize) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row as u32),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    (buffer, row)
}

/// Saves a picture that `copy_picture` put into `buffer` as a file: the format of the
/// extension of `path` (`.bmp`, `.tga`, `.dds`, `.jpg`), a PNG file for another one.
/// `blue_first` for a texture format with the bytes blue, green, red.
///
/// The pixels are read on the caller's thread. When the picture writer runs
/// ([`start_writer`], the live mode), the file is written on its thread, and this call
/// returns before the disk has the file; else (a replay) the file is written before the return.
/// The line `picture: <path>`, or `picture: <path> was not written: <problem>`, comes when the
/// file is written or has failed.
pub(crate) fn save_picture(
    device: &wgpu::Device,
    buffer: &wgpu::Buffer,
    row: usize,
    width: u32,
    height: u32,
    blue_first: bool,
    path: &str,
) {
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.expect("the read of the picture"));
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let mapped = buffer.slice(..).get_mapped_range().expect("the bytes of the picture");
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        pixels.extend_from_slice(&mapped[y * row..y * row + width as usize * 4]);
    }
    for pixel in pixels.chunks_exact_mut(4) {
        if blue_first {
            pixel.swap(0, 2);
        }
        // The alpha of the target is of no use in a picture file.
        pixel[3] = 255;
    }
    drop(mapped);
    let job = PictureJob {
        path: path.to_string(),
        pixels,
        width,
        height,
    };
    match WRITER.get() {
        Some(writer) => {
            if let Err(full) = writer.queue(job) {
                println!("{}", full_line(path, full));
            }
        }
        None => println!("{}", write_job(&job, &write_picture)),
    }
}

/// The most bytes of pixels that wait for the picture writer, the picture in its write too.
/// A picture that does not fit is not written, and its line says so: the memory of the
/// helper stays bounded while the disk holds the writes. 256 MB are 16 pictures of 2560 x 1600,
/// or 64 pictures of 1280 x 800. An empty queue takes a picture of any size.
pub(crate) const QUEUE_BYTES: usize = 256 << 20;
/// The longest wait of the end of the live mode for the pictures in the queue
/// ([`wait_for_pictures`]): a disk that holds the writes longer loses them.
pub(crate) const END_WAIT: Duration = Duration::from_secs(10);

/// A picture file that waits for its write.
pub(crate) struct PictureJob {
    path: String,
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

/// The function that writes a picture file: [`write_picture`]; a test gives one that waits.
type WriteFn = dyn Fn(&str, &[u8], u32, u32) -> std::io::Result<()> + Send + Sync;
/// The function that gets the line of each picture: a print; a test keeps the lines.
type SayFn = dyn Fn(String) + Send + Sync;

/// The pictures that wait, and the bytes of their pixels with those of the picture in its write.
#[derive(Default)]
struct Queue {
    jobs: std::collections::VecDeque<PictureJob>,
    bytes: usize,
    /// The path of the picture in its write.
    writing: Option<String>,
}

/// The count and the bytes of a full queue, for the line of the picture that did not fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Full {
    pictures: usize,
    bytes: usize,
}

/// The picture writer: a thread of its own that writes the picture files in their order.
/// The frame thread only puts the pixels in the queue: an encode and a write to a disk
/// that holds the writes took more than the watchdog's 5 s on the frame thread (W425, the run
/// 20261123-011310-w425-on: a PNG file of 41 bytes and the end with the exit code 30).
pub(crate) struct PictureWriter {
    queue: Mutex<Queue>,
    /// Wakes the thread for a new job, and the end's wait for the last write.
    changed: Condvar,
    write: Box<WriteFn>,
    say: Box<SayFn>,
    limit: usize,
    /// The time of the write that runs: for the watchdog's cause of a stall.
    pub(crate) clock: crate::log_pipe::WriteClock,
}

impl PictureWriter {
    fn new(write: Box<WriteFn>, say: Box<SayFn>, limit: usize) -> PictureWriter {
        PictureWriter {
            queue: Mutex::new(Queue::default()),
            changed: Condvar::new(),
            write,
            say,
            limit,
            clock: crate::log_pipe::WriteClock::new(),
        }
    }

    /// Starts the thread of the writer. False when the thread cannot start.
    fn spawn(self: &Arc<Self>) -> bool {
        let writer = self.clone();
        std::thread::Builder::new()
            .name("picture-writer".into())
            .spawn(move || writer.run())
            .is_ok()
    }

    /// Puts a picture behind the others. An error, with the queue as it is, when its pixels do
    /// not fit the limit; an empty queue takes it.
    fn queue(&self, job: PictureJob) -> Result<(), Full> {
        let mut queue = lock(&self.queue);
        let size = job.pixels.len();
        if queue.bytes > 0 && queue.bytes + size > self.limit {
            return Err(Full {
                pictures: queue.jobs.len() + usize::from(queue.writing.is_some()),
                bytes: queue.bytes,
            });
        }
        queue.bytes += size;
        queue.jobs.push_back(job);
        self.changed.notify_all();
        Ok(())
    }

    /// The thread: each picture in its order, for the life of the program.
    fn run(&self) {
        loop {
            let job = {
                let mut queue = lock(&self.queue);
                loop {
                    if let Some(job) = queue.jobs.pop_front() {
                        queue.writing = Some(job.path.clone());
                        break job;
                    }
                    queue = self.changed.wait(queue).unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            };
            let line = {
                let _write = self.clock.begin();
                write_job(&job, &*self.write)
            };
            (self.say)(line);
            let mut queue = lock(&self.queue);
            queue.bytes -= job.pixels.len();
            queue.writing = None;
            self.changed.notify_all();
        }
    }

    /// Waits until each picture in the queue is written, for at most `limit`. The pictures
    /// that are not written then: the one in its write first, then the queue in its order.
    fn wait(&self, limit: Duration) -> Vec<String> {
        let deadline = Instant::now() + limit;
        let mut queue = lock(&self.queue);
        while !queue.jobs.is_empty() || queue.writing.is_some() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            queue = self
                .changed
                .wait_timeout(queue, left)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        queue
            .writing
            .iter()
            .cloned()
            .chain(queue.jobs.iter().map(|job| job.path.clone()))
            .collect()
    }
}

/// The lock of the queue, also after a panic of another thread under it: the queue has no
/// state that a panic leaves half changed.
fn lock(queue: &Mutex<Queue>) -> std::sync::MutexGuard<'_, Queue> {
    queue.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Writes the file of `job` with `write`; the line of its result.
fn write_job(job: &PictureJob, write: &WriteFn) -> String {
    match write(&job.path, &job.pixels, job.width, job.height) {
        Ok(()) => format!("picture: {}", job.path),
        Err(problem) => format!("picture: {} was not written: {problem}", job.path),
    }
}

/// The line of a picture that did not fit the queue.
fn full_line(path: &str, full: Full) -> String {
    format!(
        "picture: {path} was not written: {} pictures ({} MB) wait for the disk",
        full.pictures,
        full.bytes >> 20
    )
}

/// The line of a picture that the end did not wait for.
fn not_written_line(path: &str) -> String {
    format!(
        "picture: {path} was not written: the disk held the writes for the {} s of the end",
        END_WAIT.as_secs()
    )
}

/// The picture writer of the program, after [`start_writer`].
static WRITER: OnceLock<Arc<PictureWriter>> = OnceLock::new();

/// Starts the picture writer of the live mode (`cli::run_with`). Without the thread, each
/// picture is written by its caller, as in a replay.
pub(crate) fn start_writer() {
    let writer = Arc::new(PictureWriter::new(
        Box::new(write_picture),
        Box::new(|line| println!("{line}")),
        QUEUE_BYTES,
    ));
    if !writer.spawn() {
        println!("pictures: no writer thread; the frame thread writes each picture file");
        return;
    }
    let _ = WRITER.set(writer);
}

/// Waits, for at most [`END_WAIT`], until the picture writer has written each picture in its
/// queue, and writes a line for each picture that it has not written then. For the end of the
/// live mode, before the drain of the log. Nothing without the writer.
pub(crate) fn wait_for_pictures() {
    if let Some(writer) = WRITER.get() {
        for path in writer.wait(END_WAIT) {
            println!("{}", not_written_line(&path));
        }
    }
}

/// How long the write of the picture that the writer writes now has run; None while it
/// writes none, and without the writer.
pub(crate) fn write_held() -> Option<Duration> {
    WRITER.get().and_then(|writer| writer.clock.held())
}

/// Writes pixels (red, green, blue, alpha; the first row at the top) as a picture file.
pub(crate) fn write_picture(path: &str, pixels: &[u8], width: u32, height: u32) -> std::io::Result<()> {
    use std::io::Write;
    let kind = std::path::Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let (w, h) = (width as usize, height as usize);
    // The bytes blue, green, red of a row, for the formats that have them in that order.
    let bgr_row = |y: usize, with_alpha: bool| -> Vec<u8> {
        pixels[y * w * 4..(y + 1) * w * 4]
            .chunks_exact(4)
            .flat_map(|p| [p[2], p[1], p[0], 255].into_iter().take(if with_alpha { 4 } else { 3 }))
            .collect()
    };
    if kind == "jpg" || kind == "jpeg" {
        let rgb: Vec<u8> = pixels.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        let encoder =
            jpeg_encoder::Encoder::new_file(path, 90).map_err(|problem| std::io::Error::other(problem.to_string()))?;
        return encoder
            .encode(&rgb, width as u16, height as u16, jpeg_encoder::ColorType::Rgb)
            .map_err(|problem| std::io::Error::other(problem.to_string()));
    }
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    match kind.as_str() {
        "bmp" => {
            // 24 bits, the rows from the bottom, each row a multiple of 4 bytes.
            let row = (w * 3).next_multiple_of(4);
            let size = 54 + row * h;
            file.write_all(b"BM")?;
            for word in [
                size as u32,
                0,
                54,
                40,
                width,
                height,
                1 | (24 << 16),
                0,
                (row * h) as u32,
                2835,
                2835,
                0,
                0,
            ] {
                file.write_all(&word.to_le_bytes())?;
            }
            for y in (0..h).rev() {
                let mut bytes = bgr_row(y, false);
                bytes.resize(row, 0);
                file.write_all(&bytes)?;
            }
        }
        "tga" => {
            // True colour, 24 bits, the first row at the top.
            file.write_all(&[0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0])?;
            file.write_all(&(width as u16).to_le_bytes())?;
            file.write_all(&(height as u16).to_le_bytes())?;
            file.write_all(&[24, 0x20])?;
            for y in 0..h {
                file.write_all(&bgr_row(y, false))?;
            }
        }
        "dds" => {
            // No compression, 32 bits: the format A8R8G8B8 of Direct3D.
            file.write_all(b"DDS ")?;
            let mut head = [0u32; 31];
            head[0] = 124;
            head[1] = 0x1 | 0x2 | 0x4 | 0x8 | 0x1000;
            head[2] = height;
            head[3] = width;
            head[4] = width * 4;
            head[18] = 32;
            head[19] = 0x41;
            head[21] = 32;
            head[22] = 0x00FF_0000;
            head[23] = 0x0000_FF00;
            head[24] = 0x0000_00FF;
            head[25] = 0xFF00_0000;
            head[26] = 0x1000;
            for word in head {
                file.write_all(&word.to_le_bytes())?;
            }
            for y in 0..h {
                file.write_all(&bgr_row(y, true))?;
            }
        }
        _ => {
            // The fast compression: a picture of the game must not take a second.
            let mut encoder = png::Encoder::new(file, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_compression(png::Compression::Fast);
            encoder
                .write_header()
                .and_then(|mut writer| writer.write_image_data(pixels))
                .map_err(|problem| std::io::Error::other(problem.to_string()))?;
            return Ok(());
        }
    }
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(path: &str, bytes: usize) -> PictureJob {
        PictureJob {
            path: path.to_string(),
            pixels: vec![0; bytes],
            width: 1,
            height: 1,
        }
    }

    /// A disk that holds each write until the test lets it go.
    #[derive(Clone, Default)]
    struct Gate(Arc<(Mutex<bool>, Condvar)>);

    impl Gate {
        fn wait(&self) {
            let (open, changed) = &*self.0;
            let mut open = open.lock().unwrap();
            while !*open {
                open = changed.wait(open).unwrap();
            }
        }

        fn open(&self) {
            let (open, changed) = &*self.0;
            *open.lock().unwrap() = true;
            changed.notify_all();
        }
    }

    /// A writer whose writes wait for `gate`, with the lines that it says.
    fn held_writer(gate: &Gate, limit: usize) -> (Arc<PictureWriter>, Arc<Mutex<Vec<String>>>) {
        let said = Arc::new(Mutex::new(Vec::new()));
        let (gate, lines) = (gate.clone(), said.clone());
        let writer = Arc::new(PictureWriter::new(
            Box::new(move |path, _, _, _| {
                gate.wait();
                if path.ends_with(".fail") {
                    Err(std::io::Error::other("a test error"))
                } else {
                    Ok(())
                }
            }),
            Box::new(move |line| lines.lock().unwrap().push(line)),
            limit,
        ));
        (writer, said)
    }

    /// The fault of W425: the disk holds the write of a picture. The frame thread's call puts the
    /// pictures in the queue and returns; they are written in their order when the disk lets go.
    #[test]
    fn a_picture_returns_while_the_disk_holds_its_write() {
        let gate = Gate::default();
        let (writer, said) = held_writer(&gate, QUEUE_BYTES);
        assert!(writer.spawn());
        let began = Instant::now();
        for path in ["a.png", "b.fail", "c.png"] {
            assert_eq!(writer.queue(job(path, 1000)), Ok(()));
        }
        assert!(began.elapsed() < Duration::from_secs(1), "the queue waited for the disk");
        let deadline = Instant::now() + Duration::from_secs(5);
        while writer.clock.held().is_none() {
            assert!(Instant::now() < deadline, "the writer did not start its write");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(said.lock().unwrap().is_empty());

        gate.open();
        assert_eq!(writer.wait(Duration::from_secs(5)), Vec::<String>::new());
        assert_eq!(
            *said.lock().unwrap(),
            [
                "picture: a.png",
                "picture: b.fail was not written: a test error",
                "picture: c.png"
            ]
        );
        assert_eq!(writer.clock.held(), None);
        assert_eq!(lock(&writer.queue).bytes, 0);
    }

    /// The bound of the queue counts the bytes of the pictures that wait and of the one in its
    /// write; a picture over it is not taken. An empty queue takes a picture of any size.
    #[test]
    fn the_queue_keeps_to_its_bytes() {
        let gate = Gate::default();
        // No thread: the pictures stay in the queue.
        let (writer, _) = held_writer(&gate, 100);
        assert_eq!(writer.queue(job("a.png", 60)), Ok(()));
        assert_eq!(writer.queue(job("b.png", 60)), Err(Full { pictures: 1, bytes: 60 }));
        assert_eq!(writer.queue(job("c.png", 40)), Ok(()));
        assert_eq!(writer.queue(job("d.png", 1)), Err(Full { pictures: 2, bytes: 100 }));
        let (empty, _) = held_writer(&gate, 100);
        assert_eq!(empty.queue(job("big.png", 500)), Ok(()));
        assert_eq!(
            full_line(
                "b.png",
                Full {
                    pictures: 3,
                    bytes: 300 << 20
                }
            ),
            "picture: b.png was not written: 3 pictures (300 MB) wait for the disk"
        );
    }

    /// The end waits for the queue at most its limit, and names the pictures that are not
    /// written: the one in its write first.
    #[test]
    fn the_end_names_the_pictures_that_it_did_not_wait_for() {
        let gate = Gate::default();
        let (writer, said) = held_writer(&gate, QUEUE_BYTES);
        assert!(writer.spawn());
        assert_eq!(writer.queue(job("a.png", 10)), Ok(()));
        assert_eq!(writer.queue(job("b.png", 10)), Ok(()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while writer.clock.held().is_none() {
            assert!(Instant::now() < deadline, "the writer did not start its write");
            std::thread::sleep(Duration::from_millis(1));
        }
        let began = Instant::now();
        assert_eq!(writer.wait(Duration::from_millis(100)), ["a.png", "b.png"]);
        assert!(began.elapsed() < Duration::from_secs(2), "the end kept to its limit");
        assert_eq!(
            not_written_line("a.png"),
            "picture: a.png was not written: the disk held the writes for the 10 s of the end"
        );
        gate.open();
        assert_eq!(writer.wait(Duration::from_secs(5)), Vec::<String>::new());
        assert_eq!(*said.lock().unwrap(), ["picture: a.png", "picture: b.png"]);
    }

    #[test]
    fn a_target_side_over_the_largest_is_cut_on_its_own() {
        assert_eq!(target_size((16384, 1024), 2048), (2048, 1024));
        assert_eq!(target_size((4096, 4096), 2048), (2048, 2048));
        assert_eq!(target_size((512, 512), 2048), (512, 512));
        assert_eq!(target_size((0, 3), 2048), (1, 3));
        assert_eq!(target_size((2048, 2049), 2048), (2048, 2048));
    }

    /// A pixel with its column and row in its bytes.
    fn grid(width: u32, height: u32) -> Vec<u8> {
        (0..height)
            .flat_map(|y| (0..width).flat_map(move |x| [x as u8, y as u8, 0, 255]))
            .collect()
    }

    #[test]
    fn nearest_resampling_takes_the_nearest_pixel_both_ways() {
        let big = grid(4, 2);
        let small = resample_nearest(&big, (4, 2), (2, 1));
        assert_eq!(small, vec![0, 0, 0, 255, 2, 0, 0, 255]);
        let back = resample_nearest(&small, (2, 1), (4, 2));
        assert_eq!(back.len(), big.len());
        assert_eq!(
            back.chunks_exact(4).map(|p| p[0]).collect::<Vec<_>>(),
            vec![0, 0, 2, 2, 0, 0, 2, 2]
        );
        assert_eq!(resample_nearest(&big, (4, 2), (4, 2)), big);
    }
}
