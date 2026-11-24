//! The smaller levels of a texture that has only one (W632).
//!
//! A texture file with one level gives the GPU nothing smaller to sample when the texture is
//! drawn small, so each move of the camera by less than a pixel samples another set of its
//! texels: the picture flickers. The engine makes no levels for block-compressed data
//! (`NiDevImageConverter::convertPixelDataWithMipmaps` 0x6F7380 makes them only for palette,
//! RGB24 and RGBA32 data), so it shows the same flicker. This program makes the chain that a
//! texture tool would make: each level is the level above it through a 2 x 2 box filter of the
//! stored values, in the format of level 0. Level 0 stays as the record has it.
//!
//! A block format keeps its blocks: each smaller level is encoded again, also a level of 2 x 2
//! or 1 x 1, which is one block with its pixels repeated (the GPU reads only the level's own
//! pixels). The encoder is a principal-axis fit for the colours and a range fit for the alpha;
//! it matches `d3d::decode_dxt`.

use crate::d3d::decode_dxt;

const DXT1: u32 = 0x3154_5844;
const DXT3: u32 = 0x3354_5844;
const DXT5: u32 = 0x3554_5844;

/// True for a D3D format whose one level this program makes a chain for: DXT1, DXT3, DXT5 and
/// the 32-bit formats with 8 bits for each channel (A8R8G8B8, X8R8G8B8).
pub(crate) fn takes_chain(d3d_format: u32) -> bool {
    matches!(d3d_format, DXT1 | DXT3 | DXT5 | 21 | 22)
}

/// D3DTEXF_NONE and D3DTEXF_LINEAR, the filter between levels (D3DTSS_MIPFILTER).
const MIP_FILTER_NONE: u32 = 0;
const MIP_FILTER_LINEAR: u32 = 2;

/// The filter between the levels of a stage's texture, in `sampler[4]` (the Architect's W632 ruling (b)).
///
/// - A texture whose levels this program made (`chained`) gets a linear filter between them, whatever
///   filter the material names. Its file has one level, so in the engine a filter between levels has
///   nothing to filter: BILERP and TRILERP give the same picture for it. The moons are such a case:
///   `NiTexturingProperty::ctor_fromFile` (0x6E1910) gives their map FILTER_BILERP (0x6E19B3).
/// - A texture of a sky draw (`sky`: a class of the sky, with `graphics.sky_mip_filter` on) that has
///   levels gets a linear filter between them too: the Architect's W632 re-ruling. The engine gives
///   the sky's maps no filter between levels, so a large level 0 drawn small reads only level 0.
/// - Any other texture with one level gets none.
/// - Any other texture with levels of its own keeps the material's filter.
pub(crate) fn level_filter(sampler: &mut [u32; 5], levels: u32, chained: bool, sky: bool) {
    if chained || (sky && levels > 1) {
        sampler[4] = MIP_FILTER_LINEAR;
    } else if levels <= 1 {
        sampler[4] = MIP_FILTER_NONE;
    }
}

/// True when the levels are blocks: a block format that is not decoded into 4 bytes a pixel.
fn as_blocks(d3d_format: u32, decoded: bool) -> bool {
    !decoded && matches!(d3d_format, DXT1 | DXT3 | DXT5)
}

/// The bytes of a level of this size in the form that the GPU takes.
fn level_size(d3d_format: u32, decoded: bool, width: usize, height: usize) -> usize {
    if as_blocks(d3d_format, decoded) {
        let block = if d3d_format == DXT1 { 8 } else { 16 };
        width.div_ceil(4) * height.div_ceil(4) * block
    } else {
        width * height * 4
    }
}

/// The sizes of the levels below level 0, from the size of level 0.
fn sizes_below(width: u32, height: u32) -> Vec<(usize, usize)> {
    let (mut w, mut h) = (width.max(1) as usize, height.max(1) as usize);
    let mut sizes = Vec::new();
    while w > 1 || h > 1 {
        (w, h) = ((w / 2).max(1), (h / 2).max(1));
        sizes.push((w, h));
    }
    sizes
}

/// The levels below level 0 with zeros: for a texture whose record comes without its pixels
/// (they follow as an update of level 0, which makes the chain then).
pub(crate) fn zero_levels_below(d3d_format: u32, decoded: bool, width: u32, height: u32) -> Vec<Vec<u8>> {
    sizes_below(width, height)
        .into_iter()
        .map(|(w, h)| vec![0u8; level_size(d3d_format, decoded, w, h)])
        .collect()
}

/// How a level of this size goes into the GPU texture: the bytes of a row, the rows, and the
/// extent (a block format's extent is whole blocks).
pub(crate) fn level_layout(d3d_format: u32, decoded: bool, width: u32, height: u32) -> (u32, u32, (u32, u32)) {
    if as_blocks(d3d_format, decoded) {
        let block = if d3d_format == DXT1 { 8 } else { 16 };
        (
            width.div_ceil(4) * block,
            height.div_ceil(4),
            (width.next_multiple_of(4), height.next_multiple_of(4)),
        )
    } else {
        (width * 4, height, (width, height))
    }
}

/// The size of the level `level` (1 is the first below level 0).
pub(crate) fn size_of_level(width: u32, height: u32, level: u32) -> (u32, u32) {
    ((width >> level).max(1), (height >> level).max(1))
}

/// A texture that takes a chain: made with all its levels, shown through a view of level 0 until
/// its job is done, then through `full` (the Architect's W1).
pub(crate) struct ChainState {
    /// The generation of the last job given for it: a made chain of another is stale.
    pub(crate) generation: u64,
    pub(crate) d3d_format: u32,
    pub(crate) decoded: bool,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// The frame in which the last job was given.
    pub(crate) given: u32,
    /// The view of all the levels, once a chain is in them; None while the view of level 0 shows.
    pub(crate) full: Option<std::rc::Rc<wgpu::TextureView>>,
}

/// A chain to make: the texture's handle, the generation of the handle when the job was given,
/// and level 0 in the form that the GPU takes.
pub(crate) struct Job {
    pub(crate) handle: u32,
    pub(crate) generation: u64,
    pub(crate) d3d_format: u32,
    pub(crate) decoded: bool,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) level0: Vec<u8>,
}

/// A made chain: the levels below level 0, and the worker's time for them.
pub(crate) struct Done {
    pub(crate) handle: u32,
    pub(crate) generation: u64,
    pub(crate) levels: Vec<Vec<u8>>,
    pub(crate) seconds: f64,
}

/// The thread that makes the chains, in the order of the jobs (W632, the Architect's W1 to W5).
/// The frame thread gives the jobs and takes the made chains at a frame's start; it never waits
/// in the live mode. A drop stops the thread after the job in hand: the jobs still in the queue
/// are not made.
pub(crate) struct Worker {
    jobs: Option<std::sync::mpsc::Sender<Job>>,
    done: std::sync::mpsc::Receiver<Done>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// The jobs given and not yet taken back.
    pending: usize,
}

impl Worker {
    pub(crate) fn start() -> Worker {
        let (jobs, queue) = std::sync::mpsc::channel::<Job>();
        let (finished, done) = std::sync::mpsc::channel::<Done>();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = std::thread::Builder::new()
            .name("mip chains".into())
            .spawn(move || {
                while let Ok(job) = queue.recv() {
                    if stopped.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    let began = std::time::Instant::now();
                    let levels = levels_below(job.d3d_format, job.decoded, job.width, job.height, &job.level0);
                    let made = Done {
                        handle: job.handle,
                        generation: job.generation,
                        levels,
                        seconds: began.elapsed().as_secs_f64(),
                    };
                    if finished.send(made).is_err() {
                        break;
                    }
                }
            })
            .ok();
        Worker {
            jobs: Some(jobs),
            done,
            stop,
            thread,
            pending: 0,
        }
    }

    /// Gives a job. False when the thread is gone (it could not start, or it ended).
    pub(crate) fn give(&mut self, job: Job) -> bool {
        let given = self.thread.is_some() && self.jobs.as_ref().is_some_and(|jobs| jobs.send(job).is_ok());
        self.pending += given as usize;
        given
    }

    /// The made chains: those ready now, or with `wait` all the jobs given before.
    pub(crate) fn take(&mut self, wait: bool) -> Vec<Done> {
        let mut made = Vec::new();
        while self.pending > 0 {
            let next = if wait {
                self.done.recv().ok()
            } else {
                self.done.try_recv().ok()
            };
            let Some(done) = next else { break };
            self.pending -= 1;
            made.push(done);
        }
        made
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.jobs = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The levels below level 0, each in the form that the GPU takes: blocks for a block format
/// that is not decoded, else 4 bytes (blue, green, red, alpha) a pixel. `level0` is level 0 in
/// that form; missing bytes count as zeros.
pub(crate) fn levels_below(d3d_format: u32, decoded: bool, width: u32, height: u32, level0: &[u8]) -> Vec<Vec<u8>> {
    let (mut w, mut h) = (width.max(1) as usize, height.max(1) as usize);
    let blocks = as_blocks(d3d_format, decoded);
    let mut top = level0[..level0.len().min(level_size(d3d_format, decoded, w, h))].to_vec();
    top.resize(level_size(d3d_format, decoded, w, h), 0);
    let mut pixels = if blocks { decode_dxt(d3d_format, w, h, &top) } else { top };
    let mut levels = Vec::new();
    while w > 1 || h > 1 {
        let (next, nw, nh) = half(&pixels, w, h);
        levels.push(if blocks {
            encode(d3d_format, nw, nh, &next)
        } else {
            next.clone()
        });
        (pixels, w, h) = (next, nw, nh);
    }
    levels
}

/// The level below: each pixel is the rounded mean of a 2 x 2 square of the level above. A side
/// of 1 stays 1; an odd side leaves its last row or column out, as a box filter does.
fn half(pixels: &[u8], width: usize, height: usize) -> (Vec<u8>, usize, usize) {
    let (w, h) = ((width / 2).max(1), (height / 2).max(1));
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        let (y0, y1) = ((2 * y).min(height - 1), (2 * y + 1).min(height - 1));
        for x in 0..w {
            let (x0, x1) = ((2 * x).min(width - 1), (2 * x + 1).min(width - 1));
            for c in 0..4 {
                let at = |x: usize, y: usize| pixels[(y * width + x) * 4 + c] as u32;
                let sum = at(x0, y0) + at(x1, y0) + at(x0, y1) + at(x1, y1);
                out[(y * w + x) * 4 + c] = ((sum + 2) / 4) as u8;
            }
        }
    }
    (out, w, h)
}

/// The blocks of a level of 4 bytes a pixel. A block past the edge repeats the edge's pixels.
pub(crate) fn encode(d3d_format: u32, width: usize, height: usize, pixels: &[u8]) -> Vec<u8> {
    let block_bytes = if d3d_format == DXT1 { 8 } else { 16 };
    let (across, down) = (width.div_ceil(4), height.div_ceil(4));
    let mut out = Vec::with_capacity(across * down * block_bytes);
    for by in 0..down {
        for bx in 0..across {
            let mut block = [[0u8; 4]; 16];
            for (n, pixel) in block.iter_mut().enumerate() {
                let (x, y) = ((bx * 4 + n % 4).min(width - 1), (by * 4 + n / 4).min(height - 1));
                pixel.copy_from_slice(&pixels[(y * width + x) * 4..][..4]);
            }
            match d3d_format {
                DXT1 => out.extend_from_slice(&color_block(&block, true)),
                DXT3 => {
                    out.extend_from_slice(&explicit_alpha(&block));
                    out.extend_from_slice(&color_block(&block, false));
                }
                _ => {
                    out.extend_from_slice(&alpha_block(&block));
                    out.extend_from_slice(&color_block(&block, false));
                }
            }
        }
    }
    out
}

/// A colour as the decoder widens it from 5, 6, 5 bits: blue, green, red.
fn wide(c: u16) -> [i32; 3] {
    [
        ((c & 31) * 255 / 31) as i32,
        ((c >> 5 & 63) * 255 / 63) as i32,
        ((c >> 11) * 255 / 31) as i32,
    ]
}

/// A colour (blue, green, red, each 0 to 255) in 5, 6, 5 bits.
fn narrow(c: [f32; 3]) -> u16 {
    let bits = |v: f32, top: f32| ((v.clamp(0.0, 255.0) * top / 255.0) + 0.5) as u16;
    bits(c[2], 31.0) << 11 | bits(c[1], 63.0) << 5 | bits(c[0], 31.0)
}

/// The 8 bytes of the colours of a block. `punch`: DXT1, where a pixel with an alpha under 128
/// takes the transparent entry (the block then has 3 colours and the transparent one).
fn color_block(block: &[[u8; 4]; 16], punch: bool) -> [u8; 8] {
    let mut clear = [false; 16];
    let mut solid = [[0f32; 3]; 16];
    let mut count = 0;
    for (n, p) in block.iter().enumerate() {
        clear[n] = punch && p[3] < 128;
        if !clear[n] {
            solid[count] = [p[0] as f32, p[1] as f32, p[2] as f32];
            count += 1;
        }
    }
    if count == 0 {
        // Every pixel transparent: equal ends (3 colours) and the transparent entry for all.
        return [0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF];
    }
    let (a, b) = ends(&solid[..count]);
    let (mut c0, mut c1) = (narrow(a), narrow(b));
    let three = count < 16;
    // DXT1: 4 colours need c0 > c1, 3 colours and the transparent entry need c0 <= c1. The
    // colours of DXT3 and DXT5 are always 4.
    if (three && c0 > c1) || (!three && c0 < c1) {
        (c0, c1) = (c1, c0);
    }
    let (p0, p1) = (wide(c0), wide(c1));
    let four = !punch || c0 > c1;
    let mix = |m0: i32, m1: i32, d: i32| -> [i32; 3] { core::array::from_fn(|k| (m0 * p0[k] + m1 * p1[k]) / d) };
    let (palette, entries) = if four {
        ([p0, p1, mix(2, 1, 3), mix(1, 2, 3)], 4)
    } else {
        ([p0, p1, mix(1, 1, 2), [0; 3]], 3)
    };
    let mut bits = 0u32;
    for (n, pixel) in block.iter().enumerate() {
        let index = if clear[n] {
            3
        } else {
            let mut best = (i32::MAX, 0u32);
            for (i, e) in palette[..entries].iter().enumerate() {
                let d = (0..3).map(|k| (e[k] - pixel[k] as i32).pow(2)).sum::<i32>();
                if d < best.0 {
                    best = (d, i as u32);
                }
            }
            best.1
        };
        bits |= index << (2 * n);
    }
    let mut out = [0u8; 8];
    out[..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    out[4..].copy_from_slice(&bits.to_le_bytes());
    out
}

/// The two ends of the colours along their principal axis: the mean plus the axis times the
/// largest and the smallest projection.
fn ends(colors: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let n = colors.len() as f32;
    let mean: [f32; 3] = core::array::from_fn(|k| colors.iter().map(|c| c[k]).sum::<f32>() / n);
    let mut cov = [[0f32; 3]; 3];
    for c in colors {
        let d: [f32; 3] = core::array::from_fn(|k| c[k] - mean[k]);
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += d[i] * d[j];
            }
        }
    }
    let mut axis = [1f32, 1.0, 1.0];
    for _ in 0..4 {
        let next: [f32; 3] = core::array::from_fn(|i| (0..3).map(|j| cov[i][j] * axis[j]).sum());
        let length = next.iter().map(|v| v * v).sum::<f32>().sqrt();
        if length < 1e-6 {
            break;
        }
        axis = next.map(|v| v / length);
    }
    let along = |c: &[f32; 3]| (0..3).map(|k| (c[k] - mean[k]) * axis[k]).sum::<f32>();
    let (low, high) = colors
        .iter()
        .map(along)
        .fold((f32::MAX, f32::MIN), |(lo, hi), t| (lo.min(t), hi.max(t)));
    (
        core::array::from_fn(|k| mean[k] + axis[k] * high),
        core::array::from_fn(|k| mean[k] + axis[k] * low),
    )
}

/// The 8 bytes of the alpha of a DXT5 block: the largest and the smallest alpha, then 3 bits a
/// pixel for the nearest of the 8 values between them.
fn alpha_block(block: &[[u8; 4]; 16]) -> [u8; 8] {
    let a0 = block.iter().map(|p| p[3]).max().unwrap_or(255) as u32;
    let a1 = block.iter().map(|p| p[3]).min().unwrap_or(255) as u32;
    let mut out = [0u8; 8];
    out[0] = a0 as u8;
    out[1] = a1 as u8;
    if a0 == a1 {
        return out;
    }
    let values: [u32; 8] = core::array::from_fn(|i| match i as u32 {
        0 => a0,
        1 => a1,
        i => ((8 - i) * a0 + (i - 1) * a1) / 7,
    });
    let mut bits = 0u64;
    for (n, pixel) in block.iter().enumerate() {
        let a = pixel[3] as u32;
        let mut best = (u32::MAX, 0u64);
        for (i, v) in values.iter().enumerate() {
            if v.abs_diff(a) < best.0 {
                best = (v.abs_diff(a), i as u64);
            }
        }
        bits |= best.1 << (3 * n);
    }
    out[2..].copy_from_slice(&bits.to_le_bytes()[..6]);
    out
}

/// The 8 bytes of the alpha of a DXT3 block: 4 bits a pixel.
fn explicit_alpha(block: &[[u8; 4]; 16]) -> [u8; 8] {
    let mut out = [0u8; 8];
    for (n, pixel) in block.iter().enumerate() {
        let value = ((pixel[3] as u32 * 15 + 127) / 255) as u8;
        out[n / 2] |= value << (4 * (n % 2));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A level of 4 bytes a pixel from a function of the place.
    fn level(width: usize, height: usize, pixel: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
        let pixel = &pixel;
        (0..height).flat_map(|y| (0..width).flat_map(move |x| pixel(x, y))).collect()
    }

    fn mean_error(a: &[u8], b: &[u8], channels: core::ops::Range<usize>) -> f64 {
        let mut sum = 0u64;
        let mut count = 0u64;
        for (p, q) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
            for c in channels.clone() {
                sum += p[c].abs_diff(q[c]) as u64;
                count += 1;
            }
        }
        sum as f64 / count as f64
    }

    #[test]
    fn a_full_chain_goes_down_to_one_pixel() {
        assert_eq!(sizes_below(256, 256).len(), 8);
        assert_eq!(sizes_below(2048, 1024).len(), 11);
        assert_eq!(sizes_below(6, 3), vec![(3, 1), (1, 1)]);
        assert_eq!(sizes_below(8, 2), vec![(4, 1), (2, 1), (1, 1)]);
        assert_eq!(sizes_below(1, 1), vec![]);
    }

    /// The levels of a block format stay blocks, also below 4 x 4: one block each.
    #[test]
    fn the_small_levels_of_a_block_format_are_one_block() {
        let top = encode(DXT5, 8, 8, &level(8, 8, |x, y| [x as u8 * 30, y as u8 * 30, 90, 255]));
        let below = levels_below(DXT5, false, 8, 8, &top);
        assert_eq!(below.iter().map(Vec::len).collect::<Vec<_>>(), vec![16, 16, 16]);
        let below = levels_below(DXT1, false, 16, 4, &vec![0u8; 4 * 8]);
        assert_eq!(below.iter().map(Vec::len).collect::<Vec<_>>(), vec![2 * 8, 8, 8, 8]);
        let zeros = zero_levels_below(DXT1, false, 16, 4);
        assert_eq!(zeros.iter().map(Vec::len).collect::<Vec<_>>(), vec![2 * 8, 8, 8, 8]);
        // Decoded: 4 bytes a pixel.
        let below = levels_below(DXT5, true, 6, 6, &vec![0u8; 6 * 6 * 4]);
        assert_eq!(below.iter().map(Vec::len).collect::<Vec<_>>(), vec![3 * 3 * 4, 4]);
    }

    /// The Architect's ruling (b): a chained texture filters between its levels whatever the material
    /// names; another texture with one level gets no such filter; a texture with levels of its own
    /// keeps the material's filter (the null: BILERP's 0 and TRILERP's 2 stay as they are).
    #[test]
    fn a_chained_texture_filters_between_its_levels() {
        let bilerp = crate::material::sampler_states(3, 1);
        let trilerp = crate::material::sampler_states(3, 2);
        let with = |mut sampler: [u32; 5], levels, chained| {
            level_filter(&mut sampler, levels, chained, false);
            sampler[4]
        };
        assert_eq!((bilerp[4], trilerp[4]), (0, 2));
        assert_eq!(with(bilerp, 12, true), 2);
        assert_eq!(with(trilerp, 1, false), 0);
        assert_eq!(with(bilerp, 12, false), 0);
        assert_eq!(with(trilerp, 12, false), 2);
    }

    /// The Architect's W632 re-ruling: a texture of a sky draw with levels (its own or a chain's)
    /// filters between them, also under BILERP; a sky texture with one level and no chain gets no
    /// filter between levels.
    #[test]
    fn a_sky_texture_with_levels_filters_between_them() {
        let bilerp = crate::material::sampler_states(3, 1);
        let trilerp = crate::material::sampler_states(3, 2);
        let with = |mut sampler: [u32; 5], levels, chained| {
            level_filter(&mut sampler, levels, chained, true);
            sampler[4]
        };
        assert_eq!(with(bilerp, 7, false), 2);
        assert_eq!(with(bilerp, 12, true), 2);
        assert_eq!(with(bilerp, 1, false), 0);
        assert_eq!(with(trilerp, 1, false), 0);
        assert_eq!(with(trilerp, 7, false), 2);
    }

    /// The null: a texture of one colour gives levels of that colour.
    #[test]
    fn one_colour_stays_one_colour() {
        let colour = [40u8, 120, 200, 255];
        let top = level(16, 16, |_, _| colour);
        for format in [DXT1, DXT3, DXT5] {
            let blocks = encode(format, 16, 16, &top);
            let mut size = 16;
            for below in levels_below(format, false, 16, 16, &blocks) {
                size /= 2;
                let pixels = decode_dxt(format, size, size, &below);
                let want = level(size, size, |_, _| colour);
                assert!(mean_error(&pixels, &want, 0..4) <= 4.0, "format {format:#x} size {size}");
            }
        }
        let below = levels_below(21, false, 4, 4, &top[..4 * 4 * 4]);
        assert_eq!(below[1], colour.to_vec());
    }

    #[test]
    fn the_box_filter_is_the_rounded_mean() {
        let top = level(2, 2, |x, y| [(x * 10 + y * 20) as u8, 255, 0, (x * 101) as u8]);
        let (out, w, h) = half(&top, 2, 2);
        assert_eq!((w, h), (1, 1));
        assert_eq!(out, vec![15, 255, 0, 51]);
        // An odd side leaves its last column out; a side of 1 stays 1.
        let (out, w, h) = half(&level(3, 1, |x, _| [x as u8 * 100, 0, 0, 0]), 3, 1);
        assert_eq!((w, h, out[0]), (1, 1, 50));
    }

    /// The encoder against the decoder, for each format. Colours along one line (what a block
    /// of the format can hold): a tight bound. Two independent gradients in each block: one
    /// line's error, a loose bound.
    #[test]
    fn the_blocks_decode_near_their_pixels() {
        let line = level(16, 16, |x, y| {
            [
                (x * 12 + y) as u8,
                (x * 8 + y) as u8,
                (200 - x * 6) as u8,
                (255 - x * 8) as u8,
            ]
        });
        let plane = level(16, 16, |x, y| [(x * 16) as u8, (y * 16) as u8, ((x + y) * 8) as u8, 255]);
        for (format, alpha_bound) in [(DXT1, 0.0), (DXT3, 9.0), (DXT5, 6.0)] {
            let pixels = decode_dxt(format, 16, 16, &encode(format, 16, 16, &line));
            let colour = mean_error(&pixels, &line, 0..3);
            assert!(colour <= 4.0, "format {format:#x}: colour error on a line {colour}");
            if format == DXT1 {
                // The line's alpha is 135 or more: every pixel stays opaque.
                assert!(
                    pixels.chunks_exact(4).all(|p| p[3] == 255),
                    "an opaque DXT1 pixel lost its alpha"
                );
            } else {
                let alpha = mean_error(&pixels, &line, 3..4);
                assert!(alpha <= alpha_bound, "format {format:#x}: alpha error {alpha}");
            }
            let pixels = decode_dxt(format, 16, 16, &encode(format, 16, 16, &plane));
            let colour = mean_error(&pixels, &plane, 0..3);
            assert!(colour <= 10.0, "format {format:#x}: colour error on a plane {colour}");
        }
    }

    fn job(handle: u32, generation: u64, side: u32) -> Job {
        Job {
            handle,
            generation,
            d3d_format: 21,
            decoded: false,
            width: side,
            height: side,
            level0: vec![200; (side * side * 4) as usize],
        }
    }

    /// The worker gives the chains back in the order of the jobs, each with its handle and
    /// generation; a take that waits gets every job given before.
    #[test]
    fn the_worker_gives_the_chains_in_order() {
        let mut worker = Worker::start();
        assert!(worker.give(job(1, 10, 8)));
        assert!(worker.give(job(2, 11, 4)));
        let made = worker.take(true);
        let keys: Vec<(u32, u64, usize)> = made.iter().map(|d| (d.handle, d.generation, d.levels.len())).collect();
        assert_eq!(keys, vec![(1, 10, 3), (2, 11, 2)]);
        assert_eq!(made[0].levels[2], vec![200; 4]);
        assert!(worker.take(true).is_empty(), "a take with no job given waits for nothing");
    }

    /// A drop stops the worker after the job in hand: the jobs still in the queue are not made.
    #[test]
    fn a_drop_does_not_make_the_queued_jobs() {
        let one = job(0, 0, 1024);
        let began = std::time::Instant::now();
        std::hint::black_box(levels_below(one.d3d_format, false, 1024, 1024, &one.level0));
        let one_job = began.elapsed().as_secs_f64();
        let mut worker = Worker::start();
        for handle in 0..64 {
            assert!(worker.give(job(handle, handle as u64, 1024)));
        }
        let began = std::time::Instant::now();
        drop(worker);
        // The null: a drop that made the queue would take about 64 jobs' time.
        let dropped = began.elapsed().as_secs_f64();
        assert!(dropped < 16.0 * one_job, "the drop took {dropped} s, one job {one_job} s");
    }

    /// The CPU time of a chain (W632 step 2, the Architect's bound: a 2048 DXT5 chain at most
    /// 2 ms inline, else a worker). Release only:
    /// `cargo test --release --lib mip_chain::tests::chain_time -- --ignored --nocapture`.
    #[test]
    #[ignore = "a bench: run in release"]
    fn chain_time() {
        for (format, side) in [(DXT5, 256usize), (DXT5, 1024), (DXT5, 2048), (DXT1, 2048), (21, 1024)] {
            let pixels = level(side, side, |x, y| {
                [(x * 7 + y) as u8, (y * 5) as u8, (x ^ y) as u8, (x + y * 3) as u8]
            });
            let top = if format == 21 {
                pixels
            } else {
                encode(format, side, side, &pixels)
            };
            let runs = 5;
            let began = std::time::Instant::now();
            for _ in 0..runs {
                std::hint::black_box(levels_below(format, false, side as u32, side as u32, &top));
            }
            let ms = began.elapsed().as_secs_f64() * 1e3 / runs as f64;
            println!("chain time: format {format:#x} {side} x {side}: {ms:.2} ms");
        }
    }

    /// DXT1: a pixel under alpha 128 takes the transparent entry; the others stay opaque.
    #[test]
    fn a_dxt1_block_keeps_its_cut_out() {
        // Each block has both: two columns cut out, two opaque.
        let top = level(8, 8, |x, _| if x % 4 < 2 { [0, 0, 0, 0] } else { [30, 160, 90, 255] });
        let pixels = decode_dxt(DXT1, 8, 8, &encode(DXT1, 8, 8, &top));
        for (n, p) in pixels.chunks_exact(4).enumerate() {
            let want = if n % 8 % 4 < 2 { 0 } else { 255 };
            assert_eq!(p[3], want, "pixel {n}");
        }
    }
}
