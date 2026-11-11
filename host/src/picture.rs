//! The pictures of the renderer: the target textures, the copy of a picture into a buffer,
//! and the picture files (PNG, JPEG).

use super::*;
use crate::objects::{Create, CreateView};

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
    match write_picture(path, &pixels, width, height) {
        Ok(()) => println!("picture: {path}"),
        Err(problem) => println!("picture: {path} was not written: {problem}"),
    }
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
    use super::{resample_nearest, target_size};

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
