//! Direct3D 8 values to wgpu: the blend factors, the compare functions, the stencil
//! operations, the sampler modes, the texture formats and their decoders (DXT, 16-bit),
//! the flexible vertex formats, and the D3DCOLOR. Pure functions of the values the
//! stream carries; nothing here touches a device.

use crate::UNKNOWN;

/// A D3DCOLOR as red, green, blue, alpha.
pub(crate) fn color(argb: u32) -> [f32; 4] {
    [(argb >> 16 & 255) as f32 / 255.0, (argb >> 8 & 255) as f32 / 255.0, (argb & 255) as f32 / 255.0, (argb >> 24) as f32 / 255.0]
}

pub(crate) fn known(value: u32, default: u32) -> u32 {
    if value == UNKNOWN {
        default
    } else {
        value
    }
}

pub(crate) fn blend_factor(value: u32) -> wgpu::BlendFactor {
    use wgpu::BlendFactor::*;
    match value {
        1 => Zero,
        3 => Src,
        4 => OneMinusSrc,
        5 => SrcAlpha,
        6 => OneMinusSrcAlpha,
        7 => DstAlpha,
        8 => OneMinusDstAlpha,
        9 => Dst,
        10 => OneMinusDst,
        11 => SrcAlphaSaturated,
        _ => One,
    }
}

pub(crate) fn compare(value: u32) -> wgpu::CompareFunction {
    use wgpu::CompareFunction::*;
    match value {
        1 => Never,
        2 => Less,
        3 => Equal,
        4 => LessEqual,
        5 => Greater,
        6 => NotEqual,
        7 => GreaterEqual,
        _ => Always,
    }
}

/// 'BC7 ', MGE XE's FourCC of a BC7 texture (`MGE_D3DFMT_BC7`); the plugin sends it for a
/// texture that MGE XE's DDS reader patch marks as BC7.
pub(crate) const D3DFMT_BC7: u32 = 0x2037_4342;

/// True for a block format that [`decode_dxt`] can make pixels of: the DXT formats. A BC7
/// texture of a size that the GPU cannot take as blocks has no such way.
pub(crate) fn cpu_decodes(d3d_format: u32) -> bool {
    matches!(d3d_format, 0x3154_5844 | 0x3254_5844 | 0x3354_5844 | 0x3454_5844 | 0x3554_5844)
}

/// How a texture update of a level lies in the stream, from [`texture_format`]: the bytes of a
/// block (0 for no blocks), the bytes of a row for the GPU, the rows, and the bytes in the stream.
pub(crate) fn level_layout(d3d_format: u32, width: u32, height: u32) -> (u32, u32, u32, usize) {
    let (block, source_bytes) = texture_format(d3d_format).map_or((0, 4), |f| (f.1 as u32, f.2 as u32));
    if block != 0 {
        let (across, down) = (width.div_ceil(4), height.div_ceil(4));
        (block, across * block, down, (across * down * block) as usize)
    } else {
        // Each format without blocks has 4 bytes a pixel on the GPU.
        (0, width * 4, height, (width * height * source_bytes) as usize)
    }
}

/// The format of the GPU for a D3DFORMAT, the bytes of one of its blocks (0 for a format
/// without blocks), and the bytes of a pixel in the stream. [`None`] for a format that this
/// program does not make.
pub(crate) fn texture_format(d3d_format: u32) -> Option<(wgpu::TextureFormat, usize, usize)> {
    Some(match d3d_format {
        0x3154_5844 => (wgpu::TextureFormat::Bc1RgbaUnorm, 8, 0),
        0x3354_5844 => (wgpu::TextureFormat::Bc2RgbaUnorm, 16, 0),
        0x3554_5844 => (wgpu::TextureFormat::Bc3RgbaUnorm, 16, 0),
        // MGE XE's BC7 ('BC7 '): DXGI_FORMAT_BC7_UNORM and its sRGB form both, as MGE XE's
        // Direct3D 9 path shows them (the engine treats the texels as stored).
        D3DFMT_BC7 => (wgpu::TextureFormat::Bc7RgbaUnorm, 16, 0),
        21 | 22 => (wgpu::TextureFormat::Bgra8Unorm, 0, 4),
        // D3DFMT_R5G6B5, X1R5G5B5, A1R5G5B5 and A4R4G4B4: this program makes 4 bytes of them.
        23..=26 => (wgpu::TextureFormat::Bgra8Unorm, 0, 2),
        // D3DFMT_V8U8, L6V5U5 and X8L8V8U8, bump maps: this program makes 4 bytes of them
        // (`bump_pixels`).
        60 | 61 => (wgpu::TextureFormat::Rgba8Snorm, 0, 2),
        62 => (wgpu::TextureFormat::Rgba8Snorm, 0, 4),
        _ => return None,
    })
}

/// True for a format whose pixels this program makes again ([`gpu_pixels`]).
pub(crate) fn expands(d3d_format: u32) -> bool {
    (23..=26).contains(&d3d_format) || is_bump(d3d_format)
}

fn is_bump(d3d_format: u32) -> bool {
    (60..=62).contains(&d3d_format)
}

/// The pixels for the GPU of a format that [`expands`] names.
pub(crate) fn gpu_pixels(d3d_format: u32, source: &[u8]) -> Vec<u8> {
    if is_bump(d3d_format) {
        bump_pixels(d3d_format, source)
    } else {
        expand_16_bits(d3d_format, source)
    }
}

/// Makes pixels with the bytes U, V, luminance, alpha for a GPU format with four signed numbers,
/// from V8U8, L6V5U5 or X8L8V8U8. The GPU has no format with a sign on U and V only. So the
/// luminance, from 0 to 1 in the source, is written as a number from 0 to 127, which the GPU reads
/// as 0 to 1. Direct3D reads 1 for a channel that a format does not have: the byte 127, for the
/// alpha of each format and for the luminance of V8U8.
fn bump_pixels(d3d_format: u32, source: &[u8]) -> Vec<u8> {
    // A number from 0 to `top` as a number from 0 to 127, rounded.
    let luminance = |value: u32, top: u32| ((value * 127 + top / 2) / top) as u8;
    if d3d_format == 60 {
        return source.as_chunks::<2>().0.iter().flat_map(|pixel| [pixel[0], pixel[1], 127, 127]).collect();
    }
    if d3d_format == 62 {
        return source.as_chunks::<4>().0.iter().flat_map(|pixel| [pixel[0], pixel[1], luminance(pixel[2] as u32, 255), 127]).collect();
    }
    // A number with a sign and 5 bits as one with 8 bits: 15 is 127, and -16 reads as -1.
    let signed = |value: u16| ((((value & 31) as i32 ^ 16) - 16) * 127 / 15).clamp(-128, 127) as i8 as u8;
    source
        .as_chunks::<2>()
        .0
        .iter()
        .flat_map(|pair| {
            let p = u16::from_le_bytes(*pair);
            [signed(p), signed(p >> 5), luminance((p >> 10) as u32, 63), 127]
        })
        .collect()
}

/// Makes pixels with the bytes blue, green, red, alpha from a level of a DXT1, DXT3 or DXT5
/// texture. The GPU takes a block format only for a size that is a multiple of 4; for each
/// other size this program makes the pixels itself.
pub(crate) fn decode_dxt(d3d_format: u32, width: usize, height: usize, source: &[u8]) -> Vec<u8> {
    let block = if d3d_format == 0x3154_5844 { 8 } else { 16 };
    let mut pixels = vec![0u8; width * height * 4];
    let across = width.div_ceil(4).max(1);
    for (index, data) in source.chunks_exact(block).enumerate() {
        let (block_x, block_y) = (index % across * 4, index / across * 4);
        if block_y >= height {
            break;
        }
        let colors = &data[block - 8..];
        let (c0, c1) = (u16::from_le_bytes([colors[0], colors[1]]), u16::from_le_bytes([colors[2], colors[3]]));
        let wide = |c: u16| [((c & 31) * 255 / 31) as u32, ((c >> 5 & 63) * 255 / 63) as u32, ((c >> 11) * 255 / 31) as u32];
        let (a, b) = (wide(c0), wide(c1));
        // Blue, green, red, alpha for each of the 4 colours of the block.
        let mut palette = [[0u8; 4]; 4];
        for channel in 0..3 {
            palette[0][channel] = a[channel] as u8;
            palette[1][channel] = b[channel] as u8;
            if c0 > c1 || block == 16 {
                palette[2][channel] = ((2 * a[channel] + b[channel]) / 3) as u8;
                palette[3][channel] = ((a[channel] + 2 * b[channel]) / 3) as u8;
            } else {
                palette[2][channel] = ((a[channel] + b[channel]) / 2) as u8;
            }
        }
        let transparent = block == 8 && c0 <= c1;
        for entry in palette.iter_mut() {
            entry[3] = 255;
        }
        if transparent {
            palette[3] = [0, 0, 0, 0];
        }
        // The alpha of DXT5: two values and 3 bits for each pixel.
        let mut alphas = [255u8; 8];
        let mut alpha_bits = 0u64;
        if d3d_format == 0x3554_5844 || d3d_format == 0x3454_5844 {
            let (a0, a1) = (data[0] as u32, data[1] as u32);
            alphas[0] = a0 as u8;
            alphas[1] = a1 as u8;
            for i in 2..8u32 {
                alphas[i as usize] = if a0 > a1 {
                    (((8 - i) * a0 + (i - 1) * a1) / 7) as u8
                } else if i < 6 {
                    (((6 - i) * a0 + (i - 1) * a1) / 5) as u8
                } else if i == 6 {
                    0
                } else {
                    255
                };
            }
            for (i, byte) in data[2..8].iter().enumerate() {
                alpha_bits |= (*byte as u64) << (8 * i);
            }
        }
        let bits = u32::from_le_bytes([colors[4], colors[5], colors[6], colors[7]]);
        for n in 0..16 {
            let (x, y) = (block_x + n % 4, block_y + n / 4);
            if x >= width || y >= height {
                continue;
            }
            let mut pixel = palette[(bits >> (2 * n) & 3) as usize];
            match d3d_format {
                // DXT3: 4 bits of alpha for each pixel.
                0x3354_5844 | 0x3254_5844 => pixel[3] = (data[n / 2] >> (4 * (n % 2)) & 15) * 17,
                0x3554_5844 | 0x3454_5844 => pixel[3] = alphas[(alpha_bits >> (3 * n) & 7) as usize],
                _ => {}
            }
            pixels[(y * width + x) * 4..][..4].copy_from_slice(&pixel);
        }
    }
    pixels
}

/// Makes pixels with the bytes blue, green, red, alpha from pixels with 16 bits.
pub(crate) fn expand_16_bits(d3d_format: u32, source: &[u8]) -> Vec<u8> {
    let wide = |value: u16, bits: u32| -> u8 { ((value as u32 * 255 + ((1 << bits) - 1) / 2) / ((1 << bits) - 1)) as u8 };
    let mut pixels = Vec::with_capacity(source.len() * 2);
    for pair in source.chunks_exact(2) {
        let p = u16::from_le_bytes([pair[0], pair[1]]);
        let (red, green, blue, alpha) = match d3d_format {
            23 => (wide(p >> 11, 5), wide(p >> 5 & 63, 6), wide(p & 31, 5), 255),
            24 => (wide(p >> 10 & 31, 5), wide(p >> 5 & 31, 5), wide(p & 31, 5), 255),
            25 => (wide(p >> 10 & 31, 5), wide(p >> 5 & 31, 5), wide(p & 31, 5), if p >> 15 != 0 { 255 } else { 0 }),
            _ => (wide(p >> 8 & 15, 4), wide(p >> 4 & 15, 4), wide(p & 15, 4), wide(p >> 12, 4)),
        };
        pixels.extend_from_slice(&[blue, green, red, alpha]);
    }
    pixels
}

/// The test of a D3DCMPFUNC value, for an extension that draws with the tests of a shape
/// (`HeldDraw::depth`, `HeldDraw::stencil`).
pub fn d3d_compare(value: u32) -> wgpu::CompareFunction {
    compare(value)
}

/// The operation of a D3DSTENCILOP value, for the same.
pub fn d3d_stencil_operation(value: u32) -> wgpu::StencilOperation {
    stencil_operation(value)
}

/// D3DSTENCILOP_*.
pub(crate) fn stencil_operation(value: u32) -> wgpu::StencilOperation {
    use wgpu::StencilOperation::*;
    match value {
        2 => Zero,
        3 => Replace,
        4 => IncrementClamp,
        5 => DecrementClamp,
        6 => Invert,
        7 => IncrementWrap,
        8 => DecrementWrap,
        _ => Keep,
    }
}

pub(crate) fn address(value: u32) -> wgpu::AddressMode {
    match value {
        2 => wgpu::AddressMode::MirrorRepeat,
        3 | 4 => wgpu::AddressMode::ClampToEdge,
        _ => wgpu::AddressMode::Repeat,
    }
}

pub(crate) fn filter(value: u32) -> wgpu::FilterMode {
    if value >= 2 && value != UNKNOWN {
        wgpu::FilterMode::Linear
    } else {
        wgpu::FilterMode::Nearest
    }
}

/// One vertex attribute of a flexible vertex format: the shader location, the offset in the
/// vertex, and the format.
pub(crate) struct Attribute {
    pub(crate) location: u32,
    pub(crate) offset: u64,
    pub(crate) format: wgpu::VertexFormat,
}

/// The attributes that a flexible vertex format of Direct3D 8 has. [`None`] for a format that
/// this program does not draw.
pub(crate) fn attributes(fvf: u32, stride: u32) -> Option<Vec<Attribute>> {
    use wgpu::VertexFormat::*;
    let position = fvf & 0xE;
    if position == 0x4 || position == 0 {
        // Vertices that are transformed already.
        return None;
    }
    let mut list = vec![Attribute { location: 0, offset: 0, format: Float32x3 }];
    let mut offset = 12u64;
    // D3DFVF_XYZ is 2. D3DFVF_XYZB1 to XYZB5 are 6 to 14: one float more for each step.
    let betas = (position as u64).saturating_sub(4) / 2;
    if betas != 0 {
        // The last beta is the matrix indexes when the format says so.
        let weights = if fvf & 0x1000 != 0 { betas - 1 } else { betas };
        // The shader input has three floats. With fewer weights, the floats after them are
        // the next part of the vertex; the shader does not read those.
        if weights != 0 && offset + 12 <= stride as u64 {
            list.push(Attribute { location: 7, offset, format: Float32x3 });
        }
        if betas == 4 && fvf & 0x1000 != 0 {
            list.push(Attribute { location: 8, offset: offset + 12, format: Uint8x4 });
        }
        offset += betas * 4;
    }
    if fvf & 0x10 != 0 {
        list.push(Attribute { location: 1, offset, format: Float32x3 });
        offset += 12;
    }
    if fvf & 0x40 != 0 {
        list.push(Attribute { location: 2, offset, format: Unorm8x4 });
        offset += 4;
    }
    if fvf & 0x80 != 0 {
        offset += 4;
    }
    let sets = (fvf >> 8 & 0xF).min(4);
    for set in 0..sets {
        list.push(Attribute { location: 3 + set, offset, format: Float32x2 });
        offset += 8;
    }
    Some(list)
}

/// The formats of the shader inputs, by location, for the inputs that a vertex does not have.
pub(crate) const INPUT_FORMATS: [wgpu::VertexFormat; 9] = {
    use wgpu::VertexFormat::*;
    [Float32x3, Float32x3, Unorm8x4, Float32x2, Float32x2, Float32x2, Float32x2, Float32x3, Uint8x4]
};

#[cfg(test)]
mod tests {
    use super::*;

    /// What the GPU reads from a byte of a signed format.
    fn read_signed(byte: u8) -> f32 {
        (byte as i8 as f32 / 127.0).max(-1.0)
    }

    #[test]
    fn x8l8v8u8_keeps_u_and_v_and_makes_the_luminance_a_positive_signed_number() {
        assert_eq!(texture_format(62), Some((wgpu::TextureFormat::Rgba8Snorm, 0, 4)));
        assert_eq!(level_layout(62, 4, 2), (0, 16, 2, 32));
        assert!(expands(62));
        // U -128, V 127, and the four luminance values at the ends. X becomes an alpha of 1.
        let source = [0x80, 0x7f, 0, 9, 0x80, 0x7f, 127, 9, 0x7f, 0x80, 128, 9, 0x7f, 0x80, 255, 9];
        let made = gpu_pixels(62, &source);
        assert_eq!(made, [0x80, 0x7f, 0, 127, 0x80, 0x7f, 63, 127, 0x7f, 0x80, 64, 127, 0x7f, 0x80, 127, 127]);
        assert_eq!((read_signed(made[0]), read_signed(made[1]), read_signed(made[3])), (-1.0, 1.0, 1.0));
        for (index, value) in [0.0f32, 127.0, 128.0, 255.0].into_iter().enumerate() {
            assert!((read_signed(made[index * 4 + 2]) - value / 255.0).abs() <= 1.0 / 254.0, "luminance {value}");
        }
    }

    #[test]
    fn l6v5u5_makes_four_bytes_with_its_numbers_at_their_ends() {
        assert_eq!(texture_format(61), Some((wgpu::TextureFormat::Rgba8Snorm, 0, 2)));
        assert_eq!(level_layout(61, 4, 2), (0, 16, 2, 16));
        assert!(expands(61));
        let pixel = |u: u16, v: u16, l: u16| (u | v << 5 | l << 10).to_le_bytes();
        // U -16, V 15, L 63; U 15, V -15, L 0; U 0, V -1, L 32.
        let source = [pixel(0b10000, 0b01111, 63), pixel(0b01111, 0b10001, 0), pixel(0, 0b11111, 32)].concat();
        let made = gpu_pixels(61, &source);
        assert_eq!(made, [0x80, 127, 127, 127, 127, 0x81, 0, 127, 0, 0xf8, 65, 127]);
        assert_eq!(read_signed(made[0]), -1.0);
        assert!((read_signed(made[9]) + 1.0 / 15.0).abs() <= 1.0 / 254.0);
    }

    #[test]
    fn v8u8_reads_a_luminance_and_an_alpha_of_1() {
        assert_eq!(texture_format(60), Some((wgpu::TextureFormat::Rgba8Snorm, 0, 2)));
        assert_eq!(level_layout(60, 4, 2), (0, 16, 2, 16));
        assert!(expands(60));
        let made = gpu_pixels(60, &[0x80, 0x7f, 0x7f, 0x80]);
        assert_eq!(made, [0x80, 0x7f, 127, 127, 0x7f, 0x80, 127, 127]);
        assert_eq!((read_signed(made[0]), read_signed(made[1]), read_signed(made[2]), read_signed(made[3])), (-1.0, 1.0, 1.0, 1.0));
    }

    #[test]
    fn the_formats_of_16_bits_still_expand_as_before() {
        assert_eq!(gpu_pixels(23, &[0xff, 0xff]), expand_16_bits(23, &[0xff, 0xff]));
        assert!(!expands(21));
    }

    #[test]
    fn bc7_is_a_block_format_of_16_bytes_that_the_cpu_does_not_decode() {
        assert_eq!(texture_format(D3DFMT_BC7), Some((wgpu::TextureFormat::Bc7RgbaUnorm, 16, 0)));
        // The update of a level of a block format: the blocks of BC7 are the size of those of DXT5.
        assert_eq!(level_layout(D3DFMT_BC7, 64, 64), (16, 256, 16, 4096));
        assert_eq!(level_layout(D3DFMT_BC7, 2, 2), (16, 16, 1, 16));
        assert_eq!(level_layout(0x3554_5844, 64, 64), (16, 256, 16, 4096));
        assert_eq!(level_layout(0x3154_5844, 64, 64), (8, 128, 16, 2048));
        assert!(!cpu_decodes(D3DFMT_BC7));
        for dxt in [0x3154_5844, 0x3254_5844, 0x3354_5844, 0x3454_5844, 0x3554_5844] {
            assert!(cpu_decodes(dxt));
        }
        // A value between the DXT FourCCs is no DXT format.
        assert!(!cpu_decodes(0x3200_0000));
    }
}
