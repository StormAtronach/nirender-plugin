//! The creation of a source texture: `NiDX8SourceTextureData_static::Create` (0x6BFA50).
//!
//! With the textures of the render link in use, `create` makes no call on the D3D8 device and
//! no call on a D3D8 texture: it gives the pixel data to the host of the link and keeps the
//! handle that the host returns. The handle is a D3D8 texture pointer, so the other code of
//! the renderer can use it.
//!
//! Without the link, `create` makes the texture on the device as the game function does.
//! MGE XE changes the game function in memory in two places: it puts a call of its own
//! function in the place of `CreateTexture`, and a second call before the end. The port
//! calls those two functions of MGE XE when the calls are in the game code.
//!
//! A comparison with the game function is not possible: with MGE XE the function releases
//! the first texture that it makes, and the second run of a comparison would get it.

use super::{ni, textures};
use crate::main_thread::{MainCell, MainThread};
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
/// 0x6ECA10 `game_textureMemoryAdd_Claude_` (cdecl): adds the size of a new texture to the totals of the game.
/// The argument is a record of 260 bytes and then the size.
const TEXTURE_MADE: usize = 0x6ECA10;

/// The place of the `CreateTexture` call in the game function, and the bytes that MGE XE
/// writes there (`patchLoadTexture2D`): 4 pushes, `mov eax, function`, `call eax`, a jump.
const CREATE_TEXTURE_CALL: usize = 0x6BFC4B;
const MGE_CREATE_HEAD: [u8; 5] = [0x52, 0x55, 0x56, 0x53, 0xB8];
const MGE_CREATE_TAIL: [u8; 4] = [0xFF, 0xD0, 0xEB, 0x0E];
/// The place where the game function lets go of the converted pixel data, and the bytes
/// that MGE XE writes there.
const RELEASE_CONVERTED: usize = 0x6BFD3B;
const MGE_UPLOAD_HEAD: [u8; 8] = [0x8B, 0x54, 0x24, 0x10, 0x52, 0x55, 0x56, 0xB8];
const MGE_UPLOAD_TAIL: [u8; 4] = [0xFF, 0xD0, 0xEB, 0x08];
/// `D3DPOOL_MANAGED`
const POOL_MANAGED: u32 = 1;

/// The function that the bytes at `at` call, when they are `head`, an address and `tail`.
unsafe fn patched_call<const N: usize>(at: usize, head: &[u8; N], tail: &[u8; 4]) -> Option<usize> {
    if rd::<[u8; N]>(at, 0) == *head && rd::<[u8; 4]>(at, N + 4) == *tail {
        Some(rd(at, N))
    } else {
        None
    }
}

/// Adds the size of the new texture to the totals of the game.
unsafe fn texture_made(converted: usize) {
    let levels: u32 = rd(converted, PD::mipmapLevels);
    let mut record = [0u32; 66];
    record[65] = rd(rd::<usize>(converted, PD::mipmapOffsets), levels as usize * 4);
    cdecl1(TEXTURE_MADE, record.as_mut_ptr() as u32);
}

/// Makes the D3D8 texture of `this` on the device and copies each level of `converted` into
/// it. Returns the D3D8 format.
unsafe fn make_on_device(this: usize, converted: usize) -> u32 {
    let mut d3d_format = 0;
    if rd::<usize>(this, SD::d3dTexture) == 0 {
        d3d_format = cdecl1(D3D_FORMAT_FOR_PIXEL_FORMAT, (this + SD::format) as u32);
        wr::<u32>(this, SD::levels, rd(converted, PD::mipmapLevels));
        let device: usize = rd(rd::<usize>(this, SD::renderer), R::d3dDevice);
        critical_section_lock(CREATE_CRITICAL_SECTION, NAME_CREATE_SURFACE);
        let result = match patched_call(CREATE_TEXTURE_CALL, &MGE_CREATE_HEAD, &MGE_CREATE_TAIL) {
            // `patchLoadTexture2DCreate` of MGE XE: device, texture data, pixel data, format.
            Some(function) => {
                let function: unsafe extern "system" fn(usize, usize, usize, u32) -> u32 = core::mem::transmute(function);
                function(device, this, converted, d3d_format)
            }
            None => crate::d3d::device_create_texture(
                device,
                rd(this, SD::width),
                rd(this, SD::height),
                rd(this, SD::levels),
                0,
                d3d_format,
                POOL_MANAGED,
                this + SD::d3dTexture,
            ),
        };
        critical_section_unlock(CREATE_CRITICAL_SECTION);
        if result != 0 {
            wr::<usize>(this, SD::d3dTexture, 0);
        }
        texture_made(converted);
        if rd::<usize>(this, SD::d3dTexture) == 0 {
            return d3d_format;
        }
    }
    let levels: u32 = rd(converted, PD::mipmapLevels);
    wr(this, SD::levels, levels);
    if levels != 0 {
        let mut level = 0u32;
        loop {
            let mut surface = 0usize;
            let d3d_texture: usize = rd(this, SD::d3dTexture);
            if crate::d3d::texture_get_surface_level(d3d_texture, level, &mut surface as *mut usize as usize) != 0 {
                // The game code lets go of the texture and keeps its address.
                crate::d3d::com_release(rd(this, SD::d3dTexture));
                break;
            }
            textures::source_data_copy_data_to_surface(converted, level, surface);
            crate::d3d::com_release(surface);
            level += 1;
            if level >= rd::<u32>(this, SD::levels) {
                break;
            }
        }
    }
    d3d_format
}

/// True for a number with one bit.
fn power_of_two(value: u32) -> bool {
    value != 0 && value & value.wrapping_sub(1) == 0
}

/// The source textures whose `create` is in progress on the main thread, innermost last.
static IN_PROGRESS: MainCell<Vec<usize>> = MainCell::new(Vec::new());
/// The nested calls that took the default texture without a read, for the log.
static NESTED: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Marks a source texture as in progress while it lives.
struct InProgress(usize);

impl InProgress {
    /// Marks the texture, and tells whether a `create` of it was already in progress.
    unsafe fn enter(t: &MainThread, texture: usize) -> (InProgress, bool) {
        let list = IN_PROGRESS.get(t);
        let nested = list.contains(&texture);
        list.push(texture);
        (InProgress(texture), nested)
    }
}

impl Drop for InProgress {
    fn drop(&mut self) {
        // The mark goes at the end of the `create` that made it, on the main thread.
        let Some(t) = MainThread::here() else { return };
        let list = IN_PROGRESS.get(&t);
        if let Some(at) = list.iter().rposition(|&texture| texture == self.0) {
            list.remove(at);
        }
    }
}

/// 0x6BFA50 `NiDX8SourceTextureData_static::Create` (cdecl). Returns the new
/// `NiDX8SourceTextureData`.
pub unsafe fn create(t: &MainThread, texture: usize, renderer: usize) -> usize {
    crate::shadow::touch(&create_port::STATE);
    // The engine's error menu for a file that cannot be read draws frames, and a frame can
    // call `create` for the same texture again. A nested call for a texture in progress
    // takes the default texture and reads no file.
    let (_in_progress, nested) = InProgress::enter(t, texture);
    let link = crate::link_client::link_textures();
    let this = operator_new(SD::SIZE);
    textures::source_data_ctor(this, texture, renderer);

    let filename: usize = rd(texture, ST::filenameOnPC);
    let mut pixel_data: usize = if nested {
        if NESTED.fetch_add(1, core::sync::atomic::Ordering::Relaxed) < 4 {
            crate::log!("texture link: a nested create of the source texture 0x{texture:x} during its own create (an error menu draws frames): the default texture, no read");
        }
        0
    } else if filename != 0 {
        textures::converter_read_file(get_image_converter(), filename, 0)
    } else {
        rd(texture, ST::pixelData)
    };
    let unreadable = nested || (filename != 0 && pixel_data == 0);
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
        let d3d_format = if link { 0 } else { make_on_device(this, converted) };
        if link {
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
        texture_made(converted);
        }

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
        match patched_call(RELEASE_CONVERTED, &MGE_UPLOAD_HEAD, &MGE_UPLOAD_TAIL) {
            // `patchLoadTexture2DUpload` of MGE XE: texture data, pixel data, format. It moves
            // a static texture to the default pool, and it deletes the converted data.
            Some(function) if !link => {
                let function: unsafe extern "system" fn(usize, usize, u32) = core::mem::transmute(function);
                function(this, converted, d3d_format);
            }
            // The converter made new pixel data, or it returned the source. New data has no
            // reference, and this deletes it.
            _ => {
                if rd::<u32>(converted, REF_COUNT) == 0 {
                    ni::delete_object(converted);
                }
            }
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
    if unreadable {
        textures::mark_unreadable(this);
    }
    this
}

// The call is always a skip: see the head of this file.
port_cdecl!(create_port, 0x6BFA50, "NiDX8SourceTextureData_static::Create",
    fn(texture: usize, renderer: usize) -> usize,
    compare_eax = false,
    regions = |set| set.skip(),
    rust_main = create);

pub static PORTS: &[&crate::shadow::PortState] = &[&create_port::STATE];
