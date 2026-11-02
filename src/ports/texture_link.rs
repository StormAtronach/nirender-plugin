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

const PREFS_MIPMAP_FLAGS: usize = crate::gen::layouts::NiFormatPrefs::mipmapFlags;
const PALETTE_REVISION: usize = crate::gen::layouts::NiPalette::revisionID;
const DATA_LINKED: usize = crate::gen::layouts::NiTexture_RendererData::linkedData;
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

/// The Direct3D format of a texture made from `converted`, from `table_format` of the engine's
/// table ([`crate::d3d::format_with_tag`]). The tag is in the converted pixel data, not in the
/// format of the texture data, which is a copy of the renderer's DXT5 entry.
unsafe fn format_of_pixels(converted: usize, table_format: u32) -> u32 {
    crate::d3d::format_with_tag(rd(converted, PD::format_format), rd(converted, PD::format_channelMasks), table_format)
}

/// The Direct3D format of the texture of `this`, made from `converted`.
unsafe fn d3d_format_of(this: usize, converted: usize) -> u32 {
    format_of_pixels(converted, cdecl1(D3D_FORMAT_FOR_PIXEL_FORMAT, (this + SD::format) as u32))
}

/// The `CreateTexture` call of `NiDX8SourceTextureData_static::Create`. The game function runs
/// as it is on the cell worker, so its format comes from the engine's table there, which has no
/// BC7. Its arguments are those of MGE XE's `patchLoadTexture2DCreate`.
unsafe extern "system" fn create_texture_call(device: usize, this: usize, converted: usize, table_format: u32) -> u32 {
    crate::d3d::device_create_texture(
        device,
        rd(this, SD::width),
        rd(this, SD::height),
        rd(this, SD::levels),
        0,
        format_of_pixels(converted, table_format),
        POOL_MANAGED,
        this + SD::d3dTexture,
    )
}

/// The bytes that put `function` at [`CREATE_TEXTURE_CALL`]: the shape of MGE XE's patch, so that
/// [`patched_call`] finds it. The jump lands past the call that it replaces.
fn create_call_bytes(function: u32) -> [u8; 13] {
    let mut bytes = [0u8; 13];
    bytes[..5].copy_from_slice(&MGE_CREATE_HEAD);
    bytes[5..9].copy_from_slice(&function.to_le_bytes());
    bytes[9..].copy_from_slice(&MGE_CREATE_TAIL);
    bytes
}

/// Puts `create_texture_call` in the game function, for a game without a Direct3D device: MGE XE
/// patches this place only on Direct3D 9. True when the patch went in, or a patch of this shape is
/// there already; false when another change holds the place.
///
/// # Safety
///
/// The game's code is mapped at its address, and no thread runs the function yet.
pub unsafe fn install_format_rule() -> bool {
    /// `lea eax, [esi + d3dTexture]; mov ecx, [ebx]; push eax`
    const GAME_CODE: [u8; 6] = [0x8D, 0x46, SD::d3dTexture as u8, 0x8B, 0x0B, 0x50];
    if patched_call(CREATE_TEXTURE_CALL, &MGE_CREATE_HEAD, &MGE_CREATE_TAIL).is_some() {
        return true;
    }
    if rd::<[u8; 6]>(CREATE_TEXTURE_CALL, 0) != GAME_CODE {
        return false;
    }
    crate::win::patch(CREATE_TEXTURE_CALL, &create_call_bytes(create_texture_call as *const () as u32))
}

/// Makes the D3D8 texture of `this` on the device and copies each level of `converted` into
/// it. Returns the D3D8 format.
unsafe fn make_on_device(this: usize, converted: usize) -> u32 {
    let mut d3d_format = 0;
    if rd::<usize>(this, SD::d3dTexture) == 0 {
        d3d_format = d3d_format_of(this, converted);
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

/// The pixel format of the device for `pixel_data`, as `create` picks it: 0 for no data, or a
/// size or a format that the device cannot take. `create` takes the default texture for those.
unsafe fn fitting_format(this: usize, pixel_data: usize, prefs: usize, renderer: usize) -> usize {
    if pixel_data != 0
        && power_of_two(rd(rd::<usize>(pixel_data, PD::mipmapWidths), 0))
        && power_of_two(rd(rd::<usize>(pixel_data, PD::mipmapHeights), 0))
    {
        textures::find_matching_pixel_format(this, pixel_data + PD::format, prefs, renderer + R::arrayArrayTextureFormats)
    } else {
        0
    }
}

/// The record of a texture of the link, with the converted pixels of `this`.
unsafe fn description_of(this: usize, texture: usize, converted: usize) -> TextureV1 {
    TextureV1 {
        struct_size: core::mem::size_of::<TextureV1>() as u32,
        flags: if rd::<u8>(texture, ST::isStatic) == 0 { TEXTURE_DYNAMIC } else { 0 },
        width: rd(this, SD::width),
        height: rd(this, SD::height),
        levels: rd(converted, PD::mipmapLevels),
        d3d_format: d3d_format_of(this, converted),
        pixel_format: rd(converted, PD::format_format),
        pixel_format_tag: rd(converted, PD::format_channelMasks),
        bytes_per_pixel: rd(converted, PD::bytesPerPixel),
        pixels: rd(converted, PD::pixelsAllMips),
        level_offsets: rd::<usize>(converted, PD::mipmapOffsets) as *const u32,
        level_widths: rd(converted, PD::mipmapWidths),
        level_heights: rd(converted, PD::mipmapHeights),
    }
}

/// Why a capture file lacks a texture ([`read_for_file`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileLacks {
    /// The game could not read its file: the texture has the default pixels.
    Unreadable,
    /// No file name and no pixel data.
    NoSource,
    /// The file does not open now.
    NoFile,
    /// The file opens, and the readers or the converter give nothing.
    NotRead,
}

/// The file name of a source texture, for the log.
///
/// # Safety
///
/// `texture` is a live `NiSourceTexture`.
pub unsafe fn file_name_of(texture: usize) -> String {
    let filename: usize = rd(texture, ST::filenameOnPC);
    if filename == 0 {
        return String::from("(no file)");
    }
    core::ffi::CStr::from_ptr(filename as *const core::ffi::c_char).to_string_lossy().into_owned()
}

/// Opens a file as the engine's `ReadImageFile` 0x6F6570 does, and closes it. That function
/// shows the error menu of the engine when the file does not open, which changes the game.
unsafe fn file_opens(filename: usize) -> bool {
    const GET_FILE: usize = 0x7C29E4;
    const READ_MODE: u32 = 0;
    const BUFFER_SIZE: u32 = 0x2800;
    const V_IS_OPEN: usize = 4;
    const V_DELETING_DTOR: usize = 0;
    let file = crate::engine::cdecl3(rd(GET_FILE, 0), filename as u32, READ_MODE, BUFFER_SIZE) as usize;
    if file == 0 {
        return false;
    }
    let open = crate::engine::vcall0(file, V_IS_OPEN) & 0xFF != 0;
    crate::engine::vcall1(file, V_DELETING_DTOR, 1);
    open
}

/// Gives `write` the record of a texture, with the pixels that `create` made for it, for a
/// capture file: the same file, the same format, the same conversion (`fitting_format`,
/// `description_of`), in memory that goes at the return. The game's objects stay as they are:
/// `data`, the renderer data of `texture`, is read only, a file's pixel data is new and goes
/// here, and a texture's own pixel data goes through the converter with no destination, so that
/// nothing can write into it. Nothing else sees it: MGE XE's patches and the counter of the
/// texture memory are inside `create`.
///
/// # Safety
///
/// `texture` is a live `NiSourceTexture`, and `data` its live `NiDX8SourceTextureData`. On the
/// game's thread, without the device lock: the readers take their critical section, and the
/// lock's holder must not wait for another thread (`sync::device_lock`).
pub unsafe fn read_for_file(texture: usize, data: usize, write: impl FnOnce(&TextureV1)) -> Result<(), FileLacks> {
    if textures::is_unreadable(data) {
        return Err(FileLacks::Unreadable);
    }
    let converter = get_image_converter();
    let filename: usize = rd(texture, ST::filenameOnPC);
    let own_pixels: usize = rd(texture, ST::pixelData);
    let mut pixel_data = if filename != 0 {
        if !file_opens(filename) {
            return Err(FileLacks::NoFile);
        }
        textures::converter_read_file(converter, filename, 0)
    } else if own_pixels != 0 {
        own_pixels
    } else {
        return Err(FileLacks::NoSource);
    };
    if pixel_data == 0 {
        return Err(FileLacks::NotRead);
    }
    wr(pixel_data, REF_COUNT, rd::<u32>(pixel_data, REF_COUNT).wrapping_add(1));
    let mut held = pixel_data;

    let renderer: usize = rd(data, SD::renderer);
    let mut format = fitting_format(data, pixel_data, texture + T::formatPrefs, renderer);
    if format == 0 {
        // As `create`: the texture of the game has the default pixels.
        pixel_data = rd(renderer, R::defaultTexturePixelData);
        format = rd(renderer, R::defaultTextureFormats);
        ni::release(held);
        held = pixel_data;
        wr(pixel_data, REF_COUNT, rd::<u32>(pixel_data, REF_COUNT).wrapping_add(1));
    }
    // Only pixels read here may be the converter's destination.
    let destination = if filename != 0 && pixel_data != rd::<usize>(renderer, R::defaultTexturePixelData) { pixel_data } else { 0 };
    let converted = textures::converter_convert(converter, pixel_data, format, destination, rd(data, SD::bMipMap));
    let result = if converted != 0 {
        write(&description_of(data, texture, converted));
        // New pixel data has no reference; the source has the one from above.
        if rd::<u32>(converted, REF_COUNT) == 0 {
            ni::delete_object(converted);
        }
        Ok(())
    } else {
        Err(FileLacks::NotRead)
    };
    ni::release(held);
    result
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
        let mut list = IN_PROGRESS.get(t);
        let nested = list.contains(&texture);
        list.push(texture);
        (InProgress(texture), nested)
    }
}

impl Drop for InProgress {
    fn drop(&mut self) {
        // The mark goes at the end of the `create` that made it, on the main thread.
        let Some(t) = MainThread::here() else { return };
        let mut list = IN_PROGRESS.get(&t);
        if let Some(at) = list.iter().rposition(|&texture| texture == self.0) {
            list.remove(at);
        }
    }
}

/// 0x6BFA50 `NiDX8SourceTextureData_static::Create` (cdecl). Returns the new
/// `NiDX8SourceTextureData`.
///
/// # Safety
///
/// `texture` points at a live `NiSourceTexture` (its file name, pixel data and format preferences
/// are read), and `renderer` at the live renderer that the source texture data is made for. When
/// the texture names a file, the engine's image converter is set: the function reads the file
/// through it.
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
    let mut format = fitting_format(this, pixel_data, prefs, renderer);
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

        let description = description_of(this, texture, converted);
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
