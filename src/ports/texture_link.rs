//! The creation of a source texture through the render link.
//!
//! `create` does the work of `NiDX8SourceTextureData_static::Create` (0x6BFA50). It makes no
//! call on the D3D8 device and no call on a D3D8 texture: it gives the pixel data to the host
//! of the link and keeps the handle that the host returns. In this version the handle is a
//! D3D8 texture pointer, thus the other code of the renderer can use it as before.
//!
//! MGE XE changes the game function in memory, thus the game function is not a reference for
//! a comparison. The picture test of the link is the test of this file.

use super::{ni, textures};
use crate::engine::{cdecl1, critical_section_lock, critical_section_unlock, get_image_converter, operator_new, pixel_format_copy, CREATE_CRITICAL_SECTION};
use crate::gen::layouts::{NiDX8PixelFormat as PF, NiDX8Renderer as R, NiDX8SourceTextureData as SD, NiPixelData as PD, NiSourceTexture as ST, NiTexture as T};
use crate::link::{TextureV1, TEXTURE_DYNAMIC};
use crate::mem::{rd, wr};
use crate::ports::ni::REF_COUNT;

/// `NiFormatPrefs::mipmapFlags`
const PREFS_MIPMAP_FLAGS: usize = 0x4;
/// `NiPalette::revisionID`
const PALETTE_REVISION: usize = 0xC;
/// `NiTexture::RendererData::linkedData`
const DATA_LINKED: usize = 0x4;
/// `global_NiTexture_bMipMapByDefault`
const MIPMAP_BY_DEFAULT: usize = 0x7DE7F8;
/// "NiDX8SourceTextureData::CreateSurf", the name that the game gives to the lock.
const NAME_CREATE_SURFACE: usize = 0x7C414C;
/// 0x6B0E20 `NiGetD3DFormatForNiPixelFormat` (cdecl)
const D3D_FORMAT_FOR_PIXEL_FORMAT: usize = 0x6B0E20;
/// 0x6ECA10 `sub_6ECA10` (cdecl): adds the size of a new texture to the totals of the game.
/// The argument is a record of 260 bytes and then the size.
const TEXTURE_MADE: usize = 0x6ECA10;

/// True for a number with one bit.
fn power_of_two(value: u32) -> bool {
    value != 0 && value & value.wrapping_sub(1) == 0
}

/// The work of 0x6BFA50 with the render link. Returns the new `NiDX8SourceTextureData`.
pub unsafe fn create(texture: usize, renderer: usize) -> usize {
    let this = operator_new(SD::SIZE);
    textures::source_data_ctor(this, texture, renderer);

    let filename: usize = rd(texture, ST::filenameOnPC);
    let mut pixel_data: usize =
        if filename != 0 { textures::converter_read_file(get_image_converter(), filename, 0) } else { rd(texture, ST::pixelData) };
    let prefs = texture + T::formatPrefs;
    // The pixel data that this function holds a reference to.
    let mut held = pixel_data;
    if pixel_data != 0 {
        wr(pixel_data, REF_COUNT, rd::<u32>(pixel_data, REF_COUNT).wrapping_add(1));
    }
    let mut format = 0usize;
    if pixel_data != 0
        && power_of_two(rd(rd::<usize>(pixel_data, PD::mipmapWidths), 0))
        && power_of_two(rd(rd::<usize>(pixel_data, PD::mipmapHeights), 0))
    {
        format = textures::find_matching_pixel_format(this, pixel_data + PD::format, prefs, renderer + R::arrayArrayTextureFormats);
    }
    if format == 0 {
        // No data, or a size or a format that the device cannot take: the default texture.
        pixel_data = rd(renderer, R::defaultTexturePixelData);
        if held != pixel_data {
            if held != 0 {
                ni::release(held);
            }
            held = pixel_data;
            if pixel_data != 0 {
                wr(pixel_data, REF_COUNT, rd::<u32>(pixel_data, REF_COUNT).wrapping_add(1));
            }
        }
        format = rd(renderer, R::defaultTextureFormats);
    }
    if pixel_data != 0 {
        wr::<u32>(this, SD::sourceRevisionID, rd(pixel_data, PD::revisionID));
        wr::<u32>(this, SD::format + PF::d3dFormat, rd(format, PF::d3dFormat));
        pixel_format_copy(this + SD::format, format);
        wr::<u32>(this, SD::width, rd(rd::<usize>(pixel_data, PD::mipmapWidths), 0));
        wr::<u32>(this, SD::height, rd(rd::<usize>(pixel_data, PD::mipmapHeights), 0));
        let mipmap_flags: u32 = rd(prefs, PREFS_MIPMAP_FLAGS);
        let mipmap = (mipmap_flags == 1 || (mipmap_flags == 2 && rd::<u8>(MIPMAP_BY_DEFAULT, 0) != 0)) as u8;
        wr(this, SD::bMipMap, mipmap);
        let converted = textures::converter_convert(get_image_converter(), pixel_data, format, pixel_data, mipmap);
        let levels: u32 = rd(converted, PD::mipmapLevels);
        let offsets: usize = rd(converted, PD::mipmapOffsets);
        let size: u32 = rd(offsets, levels as usize * 4);
        wr(this, SD::levels, levels);

        let description = TextureV1 {
            struct_size: core::mem::size_of::<TextureV1>() as u32,
            flags: if rd::<u8>(texture, ST::isStatic) == 0 { TEXTURE_DYNAMIC } else { 0 },
            width: rd(this, SD::width),
            height: rd(this, SD::height),
            levels,
            d3d_format: cdecl1(D3D_FORMAT_FOR_PIXEL_FORMAT, (this + SD::format) as u32),
            pixel_format: rd(converted, PD::format_format),
            pixel_format_tag: rd(converted, PD::format_channelMasks),
            bytes_per_pixel: rd(converted, PD::bytesPerPixel),
            pixels: rd(converted, PD::pixelsAllMips),
            level_offsets: offsets as *const u32,
            level_widths: rd(converted, PD::mipmapWidths),
            level_heights: rd(converted, PD::mipmapHeights),
        };
        critical_section_lock(CREATE_CRITICAL_SECTION, NAME_CREATE_SURFACE);
        let handle = crate::link_client::texture_create(&description);
        critical_section_unlock(CREATE_CRITICAL_SECTION);
        wr(this, SD::d3dTexture, handle);

        let mut record = [0u32; 66];
        record[65] = size;
        cdecl1(TEXTURE_MADE, record.as_mut_ptr() as u32);

        let palette: usize = rd(pixel_data, PD::palette);
        if palette != 0 {
            wr::<u32>(this, SD::palRevisionID, rd(palette, PALETTE_REVISION));
            let current: usize = rd(this, SD::palette);
            if current != palette {
                if current != 0 {
                    ni::release(current);
                }
                wr(this, SD::palette, palette);
                wr(palette, REF_COUNT, rd::<u32>(palette, REF_COUNT).wrapping_add(1));
            }
        }
        wr(this, SD::formattedSize, size);
        // The converter made new pixel data, or it returned the source. New data has no
        // reference, and this deletes it.
        if rd::<u32>(converted, REF_COUNT) == 0 {
            ni::delete_object(converted);
        }
    }

    // Puts the new data at the end of the list of renderer data of the texture.
    let mut slot = rd::<usize>(this, SD::vtbl_sgTexture) + T::rendererData;
    loop {
        let data: usize = rd(slot, 0);
        if data == 0 {
            break;
        }
        slot = data + DATA_LINKED;
    }
    wr(slot, 0, this);
    if held != 0 {
        ni::release(held);
    }
    this
}
