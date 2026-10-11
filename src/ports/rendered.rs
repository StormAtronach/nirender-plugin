//! Rendered textures and cube maps (`NiDX8RenderedTextureData`, `NiDX8RenderedCubeMapData`,
//! `NiDX8SourceCubeMapData`), their render targets, `NiDX8PixelFormat`, and the screenshot
//! and blt functions of `NiDX8Renderer`.

use core::mem::transmute;

use super::{hash_map, ni};
use crate::d3d::{
    com_add_ref, com_release, cube_texture_get_cube_map_surface, device_copy_rects, device_create_cube_texture, device_create_texture,
    device_get_depth_stencil_surface, device_resource_manager_discard_bytes, surface_get_desc, surface_lock_rect, surface_unlock_rect,
    texture_get_surface_level, D3DFMT_A1R5G5B5, D3DFMT_A4R4G4B4, D3DFMT_A8R8G8B8, D3DFMT_DXT1, D3DFMT_DXT3, D3DFMT_DXT5, D3DFMT_L6V5U5,
    D3DFMT_P8, D3DFMT_R5G6B5, D3DFMT_R8G8B8, D3DFMT_V8U8, D3DFMT_X1R5G5B5, D3DFMT_X8L8V8U8, D3DFMT_X8R8G8B8, D3DLOCK_READONLY,
    D3DPOOL_DEFAULT, D3DPOOL_MANAGED, D3DUSAGE_RENDERTARGET,
};
use crate::engine::{
    self, cdecl1, cdecl3, critical_section_lock, critical_section_unlock, debug_print1, debug_print2, directx_error_name,
    get_image_converter, ni_object_release, ni_pointer_dtor, operator_delete, pixel_data_ctor_args, pixel_format_copy,
    pixel_format_ctor_bump_luma, pixel_format_ctor_format, pixel_format_ctor_palette, pixel_format_ctor_rgba, texture_renderer_data_dtor,
    thiscall0, thiscall1, thiscall2, thiscall3, thiscall4, virtual_at, CREATE_CRITICAL_SECTION, PIXEL_FORMAT_MASK_A, PIXEL_FORMAT_MASK_B,
    PIXEL_FORMAT_SHIFT_A, PIXEL_FORMAT_SHIFT_B,
};
use crate::gen::layouts::{
    NiDX8PixelFormat as PF, NiDX8RenderTarget as RT, NiDX8RenderedTextureData as RTD, NiDX8Renderer as R,
    NiDX8SourceTextureData as STD, NiPixelData as PD, NiPixelFormat, NiRefObject, NiTexture, THashMap_Node,
};
use crate::mem::{rd, wr};
use crate::shadow::Regions;

// ---------------------------------------------------------------------------------------
// Engine data at fixed addresses
// ---------------------------------------------------------------------------------------

/// Vtable of `NiDX8RenderedTextureData`.
const VTBL_RENDERED_TEXTURE_DATA: usize = 0x74F7A0;
/// Vtable of `NiDX8RenderedCubeMapData`.
const VTBL_RENDERED_CUBE_MAP_DATA: usize = 0x74F7CC;
/// Vtable of `NiDX8SourceCubeMapData`.
const VTBL_SOURCE_CUBE_MAP_DATA: usize = 0x74F7F8;

/// `dword_7C55C0`, `dword_7C55D0`: the mask and the shift of the bit field of `compareBits`
/// that identifies the 16-bit layout.
const COMPARE_MASK_C: usize = 0x7C55C0;
const COMPARE_SHIFT_C: usize = 0x7C55D0;

/// `global_NiTexture_bMipMapByDefault`
const MIPMAP_BY_DEFAULT: usize = 0x7DE7F8;
/// Import slot of `OutputDebugStringA`.
const IMPORT_OUTPUT_DEBUG_STRING_A: usize = 0x74605C;

// Message strings of the game code.
const STR_CREATE_SURF: usize = 0x7C42E8;
const STR_CREATE_SURF_FAILED: usize = 0x7C42A0;
const STR_DEPTH_STENCIL_FAILED: usize = 0x7C41F8;
const STR_GET_SURFACE_LEVEL_FAILED: usize = 0x7C4250;
const STR_CUBE_CREATE_SURF_FAILED: usize = 0x7C43A8;
const STR_CUBE_COPY_FAILED: usize = 0x7C4358;
const STR_24_BIT: usize = 0x7C34FC;

// ---------------------------------------------------------------------------------------
// Offsets the layouts do not have
// ---------------------------------------------------------------------------------------

/// The second and the third group of `NiDX8Renderer::arrayArrayTextureFormats`. Each group has
/// 14 formats. `CreateSurf` of a rendered texture reads the second group, at +0x38.
/// `Create` of a source cube map gives the third group, at +0x70, to `FindMatchingPixelFormat`.
const RENDERED_TEXTURE_FORMATS: usize = R::arrayArrayTextureFormats + 0x38;
const CUBE_MAP_FORMATS: usize = R::arrayArrayTextureFormats + 0x70;

/// `NiRenderedCubeMap`: the face that is the render target now.
const RENDERED_CUBE_MAP_CURRENT_FACE: usize = 0x34;
/// `NiSourceCubeMap`: six file names, then six `NiPixelData` pointers.
const SOURCE_CUBE_MAP_FILE_NAMES: usize = 0x58;
const SOURCE_CUBE_MAP_PIXEL_DATA: usize = 0x70;

/// `NiDX8RenderedCubeMapData`: a `NiDX8RenderedTextureData`, the cube texture and six targets.
const RENDERED_CUBE_SIZE: usize = 0x9C;
const RENDERED_CUBE_TEXTURE: usize = 0x80;
const RENDERED_CUBE_TARGETS: usize = 0x84;

/// `NiDX8SourceCubeMapData`: a `NiDX8SourceTextureData`, the cube texture and the revision
/// of the pixel data of each face.
const SOURCE_CUBE_SIZE: usize = 0x84;
const SOURCE_CUBE_TEXTURE: usize = 0x68;
const SOURCE_CUBE_FACE_REVISIONS: usize = 0x6C;

/// `NiPalette::revisionID`
const PALETTE_REVISION: usize = 0x0C;
/// `NiBltSource::rendererData` and the surface in that data.
const BLT_SOURCE_RENDERER_DATA: usize = 0x14;
const BLT_DATA_SURFACE: usize = 0x14;
/// `NiRect<unsigned int>`: left, right, top, bottom.
const RECT_LEFT: usize = 0;
const RECT_RIGHT: usize = 4;
const RECT_TOP: usize = 8;
const RECT_BOTTOM: usize = 12;

// Virtual functions, as offsets in the vtable.
const V_TEXTURE_GET_WIDTH: usize = 0x2C;
const V_TEXTURE_GET_HEIGHT: usize = 0x30;
const V_DATA_GET_D3D_TEXTURE: usize = 0x18;
const V_RENDERER_SET_RENDER_TARGET: usize = 0x3C;
const V_RENDERER_CREATE_BLT_SOURCE_DATA: usize = 0x108;
const V_CONVERTER_READ_IMAGE_FILE: usize = 0x08;
const V_CONVERTER_CONVERT_PIXEL_DATA: usize = 0x10;

const DISCARD_BYTES: u32 = 0x80000;
const CREATE_TRIES: u32 = 60;

// ---------------------------------------------------------------------------------------
// Calls of game functions that only this file calls
// ---------------------------------------------------------------------------------------

unsafe fn texture_get_width(texture: usize) -> u32 {
    thiscall0(virtual_at(texture, V_TEXTURE_GET_WIDTH), texture)
}

unsafe fn texture_get_height(texture: usize) -> u32 {
    thiscall0(virtual_at(texture, V_TEXTURE_GET_HEIGHT), texture)
}

/// 0x6B6D00 `NiDX8TextureData::ctor_args`
unsafe fn texture_data_ctor_args(this: usize, texture: usize, renderer: usize) {
    thiscall2(0x6B6D00, this, texture as u32, renderer as u32);
}

/// 0x6B6F70 `NiDX8TextureData::FindMatchingPixelFormat`
unsafe fn find_matching_pixel_format(this: usize, source_format: usize, prefs: usize, formats: usize) -> usize {
    thiscall3(0x6B6F70, this, source_format as u32, prefs as u32, formats as u32) as usize
}

/// 0x6BFDE0 `NiDX8SourceTextureData::ctor`
unsafe fn source_texture_data_ctor(this: usize, texture: usize, renderer: usize) {
    thiscall2(0x6BFDE0, this, texture as u32, renderer as u32);
}

/// 0x6BFE90 `NiDX8SourceTextureData::dtor`
unsafe fn source_texture_data_dtor(this: usize) {
    thiscall0(0x6BFE90, this);
}

/// 0x6BFF40 `NiDX8SourceTextureData::CopyDataToSurface`. It is `__cdecl`.
unsafe fn copy_data_to_surface(pixel_data: usize, level: u32, surface: usize) {
    cdecl3(0x6BFF40, pixel_data as u32, level, surface as u32);
}

/// 0x6C0160 `NiDX8SourceTextureData::setPalette`. Returns 1 when the palette changed.
unsafe fn source_texture_data_set_palette(this: usize, palette: usize) -> u8 {
    thiscall1(0x6C0160, this, palette as u32) as u8
}

/// Does what `eh vector destructor iterator` (0x72759E) does with an array of six `NiPointer`.
/// It calls `NiPointer::dtor` (0x40A880) for each, from the last to the first. It does not
/// call `engine::eh_vector_destructor_iterator`: the port stays as the shadow test verified it.
unsafe fn destroy_pointers(pointers: &mut [usize; 6]) {
    for index in (0..6).rev() {
        ni_pointer_dtor(pointers.as_mut_ptr().add(index) as usize);
    }
}

/// 0x6ECA10 `sub_6ECA10`. It is `__cdecl`. It adds the size at +0x104 of its argument to two
/// byte counters of the textures (0x7DE7FC and 0x7DE800).
unsafe fn count_texture_bytes(info: usize) {
    cdecl1(0x6ECA10, info as u32);
}

/// 0x6C0BE0 `sub_6C0BE0`: `CreateSurf` of a rendered cube map. Returns the pixel format.
unsafe fn rendered_cube_create_surf(this: usize, texture: usize) -> usize {
    thiscall1(0x6C0BE0, this, texture as u32) as usize
}

/// 0x6C0D00 `sub_6C0D00`: makes the render target of one face of a rendered cube map.
unsafe fn rendered_cube_create_face_target(this: usize, face: u32, pixel_format: usize, depth_stencil: usize) -> usize {
    thiscall3(0x6C0D00, this, face, pixel_format as u32, depth_stencil as u32) as usize
}

// ---------------------------------------------------------------------------------------
// Shared pieces of the game code
// ---------------------------------------------------------------------------------------

/// `compareBits[0]` of a new pixel format: `mask_a & (2 << shift_a) | mask_b & (1 << shift_b)`.
/// `shl` uses the low five bits of the count.
fn compare_bits(shift_a: u32, mask_a: u32, shift_b: u32, mask_b: u32) -> u32 {
    (2u32.wrapping_shl(shift_a) & mask_a) | (1u32.wrapping_shl(shift_b) & mask_b)
}

/// `compare_bits` with `dword_7DE8FC`, `dword_7C55B0`, `dword_7C55B8` and `dword_7C55B4`: the
/// shift and the mask of two bit fields of `NiPixelFormat::compareBits`.
unsafe fn default_compare_bits() -> u32 {
    compare_bits(rd(PIXEL_FORMAT_SHIFT_A, 0), rd(PIXEL_FORMAT_MASK_A, 0), rd(PIXEL_FORMAT_SHIFT_B, 0), rd(PIXEL_FORMAT_MASK_B, 0))
}

/// Writes the `NiDX8PixelFormat` member of a new `NiDX8RenderedTextureData`. The writes are in
/// the order of the writes of the game code (0x6C0497 and 0x6C01FA).
unsafe fn init_rendered_pixel_format(this: usize, compare: u32) {
    wr::<u32>(this, RTD::pixelFormat_bitsPerPixel, 8);
    wr::<u32>(this, RTD::pixelFormat_channelMasks + 12, 0);
    wr::<u32>(this, RTD::pixelFormat_channelMasks + 8, 0);
    wr::<u32>(this, RTD::pixelFormat_channelMasks + 4, 0);
    wr::<u32>(this, RTD::pixelFormat_channelMasks, 0);
    wr::<u32>(this, RTD::pixelFormat_format, 2);
    wr::<u32>(this, RTD::pixelFormat_compareBits + 4, 0);
    wr::<u32>(this, RTD::pixelFormat_extraData, 0);
    wr::<u32>(this, RTD::pixelFormat_d3dFormat, 0);
    wr::<u32>(this, RTD::pixelFormat_compareBits, compare);
}

/// Puts `data` at the end of the list of renderer data of its texture.
unsafe fn append_renderer_data(data: usize) {
    let mut slot = rd::<usize>(data, RTD::vtbl_sgTexture) + NiTexture::rendererData;
    let mut next: usize = rd(slot, 0);
    while next != 0 {
        slot = next + RTD::vtbl_linkedData;
        next = rd(next, RTD::vtbl_linkedData);
    }
    wr(slot, 0, data);
}

// ---------------------------------------------------------------------------------------
// NiDX8PixelFormat
// ---------------------------------------------------------------------------------------

/// 0x6AB410 `NiDX8PixelFormat::ctor`. It writes the `NiPixelFormat` part only. Returns `this`.
pub unsafe fn pixel_format_ctor(this: usize) -> usize {
    crate::shadow::touch(&pixel_format_ctor_port::STATE);
    wr::<u32>(this, PF::bitsPerPixel, 8);
    wr::<u32>(this, PF::channelMasks + 12, 0);
    wr::<u32>(this, PF::channelMasks + 8, 0);
    wr::<u32>(this, PF::channelMasks + 4, 0);
    wr::<u32>(this, PF::channelMasks, 0);
    wr::<u32>(this, PF::format, 2);
    let compare = default_compare_bits();
    wr::<u32>(this, PF::compareBits + 4, 0);
    wr::<u32>(this, PF::compareBits, compare);
    this
}

/// The constructor of `NiPixelFormat` that `setPixelFormat` calls for a `D3DFORMAT`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FormatCtor {
    /// Masks of red, green, blue and alpha, and the bits per pixel.
    Rgba(u32, u32, u32, u32, u32),
    Palette(u32),
    BumpLuma(u32, u32, u32, u32),
    Format(u32),
}

fn format_ctor_for(d3d_format: u32) -> FormatCtor {
    match d3d_format {
        D3DFMT_R8G8B8 | D3DFMT_X8R8G8B8 => FormatCtor::Rgba(0xFF_0000, 0xFF00, 0xFF, 0, 32),
        D3DFMT_R5G6B5 => FormatCtor::Rgba(0xF800, 0x7E0, 0x1F, 0, 16),
        D3DFMT_X1R5G5B5 => FormatCtor::Rgba(0x7C00, 0x3E0, 0x1F, 0, 16),
        D3DFMT_A1R5G5B5 => FormatCtor::Rgba(0x7C00, 0x3E0, 0x1F, 0x8000, 16),
        D3DFMT_A4R4G4B4 => FormatCtor::Rgba(0xF00, 0xF0, 0xF, 0xF000, 16),
        D3DFMT_P8 => FormatCtor::Palette(0),
        D3DFMT_V8U8 => FormatCtor::BumpLuma(0xFF, 0xFF00, 0, 16),
        D3DFMT_L6V5U5 => FormatCtor::BumpLuma(0x1F, 0x3E0, 0xFC00, 16),
        D3DFMT_X8L8V8U8 => FormatCtor::BumpLuma(0xFF, 0xFF00, 0xFF_0000, 32),
        D3DFMT_DXT1 => FormatCtor::Format(6),
        D3DFMT_DXT3 => FormatCtor::Format(7),
        D3DFMT_DXT5 => FormatCtor::Format(8),
        _ => FormatCtor::Rgba(0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000, 32),
    }
}

/// 0x4B52D0 `NiDX8PixelFormat::setPixelFormat`. Returns what the constructor of
/// `NiPixelFormat` returns.
pub unsafe fn set_pixel_format(this: usize, d3d_format: u32) -> usize {
    crate::shadow::touch(&set_pixel_format_port::STATE);
    wr(this, PF::d3dFormat, d3d_format);
    wr::<u32>(this, PF::extraData, 0);
    match format_ctor_for(d3d_format) {
        FormatCtor::Rgba(red, green, blue, alpha, bits) => pixel_format_ctor_rgba(this, red, green, blue, alpha, bits),
        FormatCtor::Palette(has_alpha) => pixel_format_ctor_palette(this, has_alpha),
        FormatCtor::BumpLuma(red, green, blue, bits) => pixel_format_ctor_bump_luma(this, red, green, blue, bits),
        FormatCtor::Format(format) => pixel_format_ctor_format(this, format),
    }
}

/// The result of `NiGetD3DFormatForNiPixelFormat`, and true when it writes the debug string.
/// `layout` is the bit field of `compareBits`. The function compares its low byte. The function
/// reads it only for 16 bits per pixel.
fn d3d_format_for(format: u32, bits_per_pixel: u8, layout: impl FnOnce() -> u32) -> (u32, bool) {
    match format {
        0 => match bits_per_pixel {
            8 => (0, false),
            16 => (D3DFMT_R5G6B5 + (layout() as u8 != 6) as u32, false),
            _ => (D3DFMT_X8R8G8B8, false),
        },
        1 => match bits_per_pixel {
            8 => (0, false),
            16 => {
                let layout = layout() as u8;
                if layout == 6 {
                    (D3DFMT_R5G6B5, false)
                } else {
                    (D3DFMT_A1R5G5B5 + (layout == 4) as u32, false)
                }
            }
            24 => (D3DFMT_X8R8G8B8, true),
            _ => (D3DFMT_A8R8G8B8, false),
        },
        2 | 3 => (D3DFMT_P8, false),
        4 => (D3DFMT_X8R8G8B8, false),
        5 => (D3DFMT_A8R8G8B8, false),
        6 => (D3DFMT_DXT1, false),
        7 => (D3DFMT_DXT3, false),
        8 => (D3DFMT_DXT5, false),
        10 if bits_per_pixel == 16 => (D3DFMT_V8U8, false),
        11 if bits_per_pixel == 16 => (D3DFMT_L6V5U5, false),
        11 if bits_per_pixel == 32 => (D3DFMT_X8L8V8U8, false),
        _ => (0, false),
    }
}

/// 0x6B0E20 `NiGetD3DFormatForNiPixelFormat` (cdecl). Returns a `D3DFORMAT`, 0 for none.
pub unsafe fn get_d3d_format_for_pixel_format(format: usize) -> u32 {
    crate::shadow::touch(&get_d3d_format_for_pixel_format_port::STATE);
    let layout = || (rd::<u32>(format, NiPixelFormat::compareBits) & rd::<u32>(COMPARE_MASK_C, 0)).wrapping_shr(rd(COMPARE_SHIFT_C, 0));
    let (result, debug_string) = d3d_format_for(rd(format, NiPixelFormat::format), rd(format, NiPixelFormat::bitsPerPixel), layout);
    if debug_string {
        let f: unsafe extern "system" fn(usize) = transmute(rd::<usize>(IMPORT_OUTPUT_DEBUG_STRING_A, 0));
        f(STR_24_BIT);
    }
    result
}

// ---------------------------------------------------------------------------------------
// NiDX8RenderTarget, NiDX8TextureRenderTarget
// ---------------------------------------------------------------------------------------

/// 0x6A9DA0 `NiDX8RenderTarget::dtor`: runs the destructors of its two `NiPointer` members.
pub unsafe fn render_target_dtor(this: usize) {
    crate::shadow::touch(&render_target_dtor_port::STATE);
    let object: usize = rd(this, RT::field_18);
    if object != 0 {
        ni::release(object);
    }
    let object: usize = rd(this, RT::field_C);
    if object != 0 {
        ni::release(object);
    }
}

/// The call is a skip when the call deletes an object: a virtual deleting destructor cannot
/// run twice.
unsafe fn render_target_dtor_regions(set: &mut Regions, this: usize) {
    let first: usize = rd(this, RT::field_18);
    let second: usize = rd(this, RT::field_C);
    for object in [first, second] {
        if object != 0 {
            let count: u32 = rd(object, NiRefObject::refCount);
            if count == 1 || (first == second && count == 2) {
                set.skip();
            }
            set.add(object, NiRefObject::SIZE);
        }
    }
}

/// 0x6C07D0 `NiDX8TextureRenderTarget::dtor` (cdecl): releases the objects of a render target
/// of a texture and frees the render target.
pub unsafe fn texture_render_target_dtor(target: usize) {
    crate::shadow::touch(&texture_render_target_dtor_port::STATE);
    if target == 0 {
        return;
    }
    let object: usize = rd(target, RT::field_C);
    if object != 0 {
        ni::release(object);
        wr::<u32>(target, RT::field_C, 0);
    }
    let texture: usize = rd(target, RT::d3dTexture);
    if texture != 0 {
        com_release(texture);
    }
    let surface: usize = rd(target, RT::d3dSurfaceBackBuffer);
    if surface != 0 {
        com_release(surface);
    }
    let object: usize = rd(target, RT::field_18);
    if object != 0 {
        ni::release(object);
        wr::<u32>(target, RT::field_18, 0);
    }
    let surface: usize = rd(target, RT::d3dSurfaceDepthStencil);
    if surface != 0 {
        com_release(surface);
    }
    let object: usize = rd(target, RT::field_1C);
    if object != 0 {
        com_release(object);
    }
    render_target_dtor(target);
    operator_delete(target);
}

/// The call is a skip when the target is not 0: its surfaces get their last `Release`.
unsafe fn texture_render_target_dtor_regions(set: &mut Regions, target: usize) {
    if target != 0 {
        set.skip();
    }
}

// ---------------------------------------------------------------------------------------
// NiDX8RenderedTextureData
// ---------------------------------------------------------------------------------------

/// 0x6C0480 `NiDX8RenderedTextureData::ctor`. Returns `this`.
pub unsafe fn rendered_texture_data_ctor(this: usize, texture: usize, renderer: usize) -> usize {
    crate::shadow::touch(&rendered_texture_data_ctor_port::STATE);
    texture_data_ctor_args(this, texture, renderer);
    init_rendered_pixel_format(this, default_compare_bits());
    wr::<u32>(this, RTD::d3dTexture, 0);
    wr::<u32>(this, RTD::target, 0);
    wr::<usize>(this, RTD::vtbl_vtbl, VTBL_RENDERED_TEXTURE_DATA);
    this
}

/// 0x6C05D0 `NiDX8RenderedTextureData::CreateSurf`: makes the render target texture.
/// Returns the `NiDX8PixelFormat` of the renderer it used, or 0. `out_format` gets the index
/// of that format.
pub unsafe fn rendered_texture_data_create_surf(this: usize, texture: usize, out_format: usize) -> usize {
    crate::shadow::touch(&rendered_texture_data_create_surf_port::STATE);
    if texture == 0 {
        return 0;
    }
    wr(this, RTD::width, texture_get_width(texture));
    wr(this, RTD::height, texture_get_height(texture));
    wr::<u32>(this, RTD::levels, 1);
    let formats = rd::<usize>(this, RTD::renderer) + RENDERED_TEXTURE_FORMATS;
    let alpha: u32 = rd(texture, NiTexture::formatPrefs_alphaFormat);
    wr::<u32>(out_format, 0, 14);
    let order: [u32; 6] = if alpha == 1 || alpha == 2 { [5, 4, 3, 2, 1, 0] } else { [2, 5, 1, 0, 4, 3] };
    let mut found = false;
    for index in order {
        if rd::<u32>(formats, index as usize * 4) != 0 {
            wr(out_format, 0, index);
            found = true;
            break;
        }
    }
    if !found {
        return 0;
    }
    let pixel_format: usize = rd(formats, rd::<u32>(out_format, 0) as usize * 4);
    let d3d_format: u32 = rd(pixel_format, PF::d3dFormat);
    let device: usize = rd(rd(this, RTD::renderer), R::d3dDevice);
    let out = this + RTD::d3dTexture;

    critical_section_lock(CREATE_CRITICAL_SECTION, STR_CREATE_SURF);
    let mut result =
        device_create_texture(device, rd(this, RTD::width), rd(this, RTD::height), 1, D3DUSAGE_RENDERTARGET, d3d_format, D3DPOOL_DEFAULT, out);
    critical_section_unlock(CREATE_CRITICAL_SECTION);
    let mut tries = 0;
    while result != 0 {
        critical_section_lock(CREATE_CRITICAL_SECTION, STR_CREATE_SURF);
        device_resource_manager_discard_bytes(device, DISCARD_BYTES);
        result =
            device_create_texture(device, rd(this, RTD::width), rd(this, RTD::height), 1, D3DUSAGE_RENDERTARGET, d3d_format, D3DPOOL_DEFAULT, out);
        critical_section_unlock(CREATE_CRITICAL_SECTION);
        tries += 1;
        if tries >= CREATE_TRIES {
            if result == 0 {
                break;
            }
            debug_print1(STR_CREATE_SURF_FAILED, directx_error_name(result));
            wr::<u32>(out, 0, 0);
            return 0;
        }
    }
    pixel_format
}

/// The call is not a skip. The texture that the Rust run makes goes to the engine run. With
/// a texture, the call writes the object and `out_format`.
unsafe fn rendered_texture_data_create_surf_regions(set: &mut Regions, this: usize, texture: usize, out_format: usize) {
    if texture != 0 {
        set.add(this, RTD::SIZE);
        set.add(out_format, 4);
    }
}

/// 0x6C01B0 `NiDX8RenderedTextureData::Create` (cdecl, static): makes the renderer data of a
/// `NiRenderedTexture`, with its texture, its depth buffer and its render target. Returns 0
/// when it cannot make a device object.
pub unsafe fn rendered_texture_data_create(texture: usize, renderer: usize) -> usize {
    crate::shadow::touch(&rendered_texture_data_create_port::STATE);
    let data = engine::operator_new(RTD::SIZE);
    if data != 0 {
        texture_data_ctor_args(data, texture, renderer);
        init_rendered_pixel_format(data, default_compare_bits());
        wr::<usize>(data, RTD::vtbl_vtbl, VTBL_RENDERED_TEXTURE_DATA);
        wr::<u32>(data, RTD::d3dTexture, 0);
        wr::<u32>(data, RTD::target, 0);
    }
    let mut format_index: u32 = 14;
    let pixel_format = rendered_texture_data_create_surf(data, texture, &mut format_index as *mut u32 as usize);
    if pixel_format == 0 {
        if data != 0 {
            ni::delete_object(data);
        }
        return 0;
    }
    let target = engine::operator_new(RT::SIZE);
    if target != 0 {
        wr::<u32>(target, RT::field_C, 0);
        wr::<u32>(target, RT::field_18, 0);
    }
    wr(data, RTD::target, target);

    // The game code does not give this local a value before the call.
    let mut depth_stencil: usize = 0;
    device_get_depth_stencil_surface(rd(renderer, R::d3dDevice), &mut depth_stencil as *mut usize as usize);
    let depth_format: u32 = if depth_stencil != 0 {
        let mut desc = [0u32; 8];
        surface_get_desc(depth_stencil, desc.as_mut_ptr() as usize);
        com_release(depth_stencil);
        desc[0]
    } else {
        rd(renderer, R::depthStencilFormats + format_index as usize * 4)
    };

    let mut tries = 0;
    while tries < CREATE_TRIES {
        let device: usize = rd(renderer, R::d3dDevice);
        device_resource_manager_discard_bytes(device, DISCARD_BYTES);
        let out = rd::<usize>(data, RTD::target) + RT::d3dSurfaceDepthStencil;
        // The game code reads the vtable of the device before the two calls on the texture.
        let device_vtable: usize = rd(rd(renderer, R::d3dDevice), 0);
        let height = texture_get_height(texture);
        let width = texture_get_width(texture);
        let device: usize = rd(renderer, R::d3dDevice);
        let create: unsafe extern "system" fn(usize, u32, u32, u32, u32, usize) -> u32 = transmute(rd::<usize>(device_vtable, 0x68));
        let result = match crate::link_client::target_call(crate::link::CALL_CREATE_DEPTH_BUFFER, &[width, height, depth_format, 0, out as u32]) {
            Some(result) => result,
            None => create(device, width, height, depth_format, 0, out),
        };
        tries += 1;
        if tries >= CREATE_TRIES {
            if result != 0 {
                debug_print1(STR_DEPTH_STENCIL_FAILED, directx_error_name(result));
                return 0;
            }
            break;
        }
        if result == 0 {
            break;
        }
    }

    wr(rd::<usize>(data, RTD::target), RT::width, texture_get_width(texture));
    wr(rd::<usize>(data, RTD::target), RT::height, texture_get_height(texture));
    wr(rd::<usize>(data, RTD::target), RT::DX8PixelFormat, pixel_format);
    let d3d_texture = thiscall0(virtual_at(data, V_DATA_GET_D3D_TEXTURE), data);
    wr(rd::<usize>(data, RTD::target), RT::d3dTexture, d3d_texture);
    let d3d_texture: usize = rd(rd(data, RTD::target), RT::d3dTexture);
    if d3d_texture != 0 {
        com_add_ref(d3d_texture);
    }
    let target: usize = rd(data, RTD::target);
    let result = texture_get_surface_level(rd(target, RT::d3dTexture), 0, target + RT::d3dSurfaceBackBuffer);
    if result != 0 {
        debug_print1(STR_GET_SURFACE_LEVEL_FAILED, directx_error_name(result));
        wr::<u32>(rd::<usize>(data, RTD::target), RT::d3dSurfaceBackBuffer, 0);
        ni::delete_object(data);
        return 0;
    }
    let slot = rd::<usize>(data, RTD::target) + RT::field_18;
    let object: usize = rd(slot, 0);
    if object != 0 {
        ni::release(object);
        wr::<u32>(slot, 0, 0);
    }
    wr::<u32>(rd::<usize>(data, RTD::target), RT::field_1C, 0);
    append_renderer_data(data);
    data
}

/// 0x6C0520 `NiDX8RenderedTextureData::dtor`
pub unsafe fn rendered_texture_data_dtor(this: usize) {
    crate::shadow::touch(&rendered_texture_data_dtor_port::STATE);
    wr::<usize>(this, RTD::vtbl_vtbl, VTBL_RENDERED_TEXTURE_DATA);
    if rd::<usize>(this, RTD::target) != 0 {
        let renderer: usize = rd(this, RTD::renderer);
        thiscall1(virtual_at(renderer, V_RENDERER_SET_RENDER_TARGET), renderer, 0);
    }
    texture_render_target_dtor(rd(this, RTD::target));
    let texture: usize = rd(this, RTD::d3dTexture);
    wr::<u32>(this, RTD::target, 0);
    if texture != 0 {
        com_release(texture);
        wr::<u32>(this, RTD::d3dTexture, 0);
    }
    texture_renderer_data_dtor(this);
}

/// The call is a skip when the object has a render target or a texture. They get their last
/// `Release`, and the render target of the device changes. The call is also a skip when the
/// object has linked data: the virtual deleting destructor of the linked data runs.
unsafe fn rendered_texture_data_dtor_regions(set: &mut Regions, this: usize, size: usize) {
    if rd::<usize>(this, RTD::target) != 0 || rd::<usize>(this, RTD::d3dTexture) != 0 || rd::<usize>(this, RTD::vtbl_linkedData) != 0 {
        set.skip();
    }
    set.add(this, size);
}

/// 0x6C0500 `NiDX8RenderedTextureData::deleting_dtor`. Returns `this`.
pub unsafe fn rendered_texture_data_deleting_dtor(this: usize, flags: u8) -> usize {
    crate::shadow::touch(&rendered_texture_data_deleting_dtor_port::STATE);
    rendered_texture_data_dtor(this);
    if flags & 1 != 0 {
        operator_delete(this);
    }
    this
}

/// 0x6C05B0 `NiDX8RenderedTextureData::GetD3DTexture`
pub unsafe fn rendered_texture_data_get_d3d_texture(this: usize) -> usize {
    crate::shadow::touch(&rendered_texture_data_get_d3d_texture_port::STATE);
    rd(this, RTD::d3dTexture)
}

/// 0x6C05C0 `NiDX8RenderedTextureData::IsLoaded`
pub unsafe fn rendered_texture_data_is_loaded(this: usize) -> u8 {
    crate::shadow::touch(&rendered_texture_data_is_loaded_port::STATE);
    (rd::<usize>(this, RTD::d3dTexture) != 0) as u8
}

/// 0x6C09A0 `NiDX8RenderedTextureData::GetTarget`
pub unsafe fn rendered_texture_data_get_target(this: usize) -> usize {
    crate::shadow::touch(&rendered_texture_data_get_target_port::STATE);
    rd(this, RTD::target)
}

// ---------------------------------------------------------------------------------------
// NiDX8RenderedCubeMapData
// ---------------------------------------------------------------------------------------

/// 0x6C09D0 `NiDX8RenderedCubeMapData::ctor` in the database. It is the static `Create`
/// (cdecl). It makes the renderer data of a `NiRenderedCubeMap` with a render target for each
/// face.
pub unsafe fn rendered_cube_map_data_create(texture: usize, renderer: usize) -> usize {
    crate::shadow::touch(&rendered_cube_map_data_create_port::STATE);
    let data = engine::operator_new(RENDERED_CUBE_SIZE);
    if data != 0 {
        rendered_texture_data_ctor(data, texture, renderer);
        wr::<usize>(data, RTD::vtbl_vtbl, VTBL_RENDERED_CUBE_MAP_DATA);
        wr::<u32>(data, RENDERED_CUBE_TEXTURE, 0);
        for face in 0..6 {
            wr::<u32>(data, RENDERED_CUBE_TARGETS + face * 4, 0);
        }
    }
    let pixel_format = rendered_cube_create_surf(data, texture);
    if pixel_format == 0 {
        if data != 0 {
            ni::delete_object(data);
        }
        return 0;
    }
    let target = rendered_cube_create_face_target(data, 0, pixel_format, 0);
    wr(data, RENDERED_CUBE_TARGETS, target);
    if target == 0 {
        ni::delete_object(data);
        return 0;
    }
    // The other faces share the depth buffer of the first.
    let depth_stencil: usize = rd(target, RT::d3dSurfaceDepthStencil);
    let mut face = 1;
    loop {
        let target = rendered_cube_create_face_target(data, face, pixel_format, depth_stencil);
        wr(data, RENDERED_CUBE_TARGETS + face as usize * 4, target);
        if target == 0 {
            ni::delete_object(data);
            return 0;
        }
        face += 1;
        if face >= 6 {
            break;
        }
    }
    append_renderer_data(data);
    data
}

/// 0x6C0B20 `NiDX8RenderedCubeMapData::dtor`
pub unsafe fn rendered_cube_map_data_dtor(this: usize) {
    crate::shadow::touch(&rendered_cube_map_data_dtor_port::STATE);
    wr::<usize>(this, RTD::vtbl_vtbl, VTBL_RENDERED_CUBE_MAP_DATA);
    let texture: usize = rd(this, RENDERED_CUBE_TEXTURE);
    if texture != 0 {
        com_release(texture);
        wr::<u32>(this, RENDERED_CUBE_TEXTURE, 0);
    }
    for face in 0..6 {
        texture_render_target_dtor(rd(this, RENDERED_CUBE_TARGETS + face * 4));
        wr::<u32>(this, RENDERED_CUBE_TARGETS + face * 4, 0);
    }
    rendered_texture_data_dtor(this);
}

/// The call is a skip when the object has the cube texture or a face target: they get their
/// last `Release`. The call is also a skip when the destructor of the base class is a skip.
unsafe fn rendered_cube_map_data_dtor_regions(set: &mut Regions, this: usize) {
    if rd::<usize>(this, RENDERED_CUBE_TEXTURE) != 0 {
        set.skip();
    }
    for face in 0..6 {
        if rd::<usize>(this, RENDERED_CUBE_TARGETS + face * 4) != 0 {
            set.skip();
        }
    }
    rendered_texture_data_dtor_regions(set, this, RENDERED_CUBE_SIZE);
}

/// 0x6C0B00 `NiDX8RenderedCubeMapData::deleting_dtor`. Returns `this`.
pub unsafe fn rendered_cube_map_data_deleting_dtor(this: usize, flags: u8) -> usize {
    crate::shadow::touch(&rendered_cube_map_data_deleting_dtor_port::STATE);
    rendered_cube_map_data_dtor(this);
    if flags & 1 != 0 {
        operator_delete(this);
    }
    this
}

/// 0x6C0BB0 `NiDX8RenderedCubeMapData::GetD3DTexture`
pub unsafe fn rendered_cube_map_data_get_d3d_texture(this: usize) -> usize {
    crate::shadow::touch(&rendered_cube_map_data_get_d3d_texture_port::STATE);
    rd(this, RENDERED_CUBE_TEXTURE)
}

/// 0x6C0BC0 `NiDX8RenderedCubeMapData::IsLoaded`
pub unsafe fn rendered_cube_map_data_is_loaded(this: usize) -> u8 {
    crate::shadow::touch(&rendered_cube_map_data_is_loaded_port::STATE);
    (rd::<usize>(this, RENDERED_CUBE_TEXTURE) != 0) as u8
}

/// 0x6C0BD0 `NiDX8RenderedCubeMapData::GetTarget`: the target of the current face of the
/// cube map.
pub unsafe fn rendered_cube_map_data_get_target(this: usize) -> usize {
    crate::shadow::touch(&rendered_cube_map_data_get_target_port::STATE);
    let face: u32 = rd(rd(this, RTD::vtbl_sgTexture), RENDERED_CUBE_MAP_CURRENT_FACE);
    rd(this, RENDERED_CUBE_TARGETS + (face as usize).wrapping_mul(4))
}

// ---------------------------------------------------------------------------------------
// NiDX8SourceCubeMapData
// ---------------------------------------------------------------------------------------

/// Copies the levels of `pixel_data` to the surfaces of one face of a cube texture
/// (0x6C14A7 and 0x6C16E3). `texture_slot` is the address of the pointer to the texture.
/// When `GetCubeMapSurface` fails, the function calls `Release` of the cube texture and
/// returns. It does not copy the other levels of the face. It does not write 0 to
/// `texture_slot`.
unsafe fn copy_face(this: usize, texture_slot: usize, face: u32, pixel_data: usize) {
    let mut level: u32 = 0;
    if rd::<u32>(this, STD::levels) == 0 {
        return;
    }
    loop {
        let mut surface: usize = 0;
        let texture: usize = rd(texture_slot, 0);
        let result = cube_texture_get_cube_map_surface(texture, face, level, &mut surface as *mut usize as usize);
        if result != 0 {
            debug_print2(STR_CUBE_COPY_FAILED, level as usize, directx_error_name(result));
            com_release(rd(texture_slot, 0));
            return;
        }
        copy_data_to_surface(pixel_data, level, surface);
        com_release(surface);
        level += 1;
        if level >= rd::<u32>(this, STD::levels) {
            return;
        }
    }
}

/// 0x6C10A0 `NiDX8SourceCubeMapData::Create` (cdecl, static): makes the renderer data of a
/// `NiSourceCubeMap`. The six faces must be square and of one size that is a power of two.
/// They must have the same number of levels and the same format.
pub unsafe fn source_cube_map_data_create(texture: usize, renderer: usize) -> usize {
    crate::shadow::touch(&source_cube_map_data_create_port::STATE);
    let data = engine::operator_new(SOURCE_CUBE_SIZE);
    if data != 0 {
        source_texture_data_ctor(data, texture, renderer);
        wr::<usize>(data, STD::vtbl_vtbl, VTBL_SOURCE_CUBE_MAP_DATA);
        wr::<u32>(data, SOURCE_CUBE_TEXTURE, 0);
        for face in 0..6 {
            wr::<u32>(data, SOURCE_CUBE_FACE_REVISIONS + face * 4, 0);
        }
    }
    let mut sources = [0usize; 6];
    let prefs = texture + NiTexture::formatPrefs;
    let mut levels: u32 = 0;
    let mut size: u32 = 0;
    let mut first_format: usize = 0;
    let mut matched = true;

    for face in 0..6 {
        if sources[face] != 0 {
            ni_object_release(sources[face]);
            sources[face] = 0;
        }
        let file_name: usize = rd(texture, SOURCE_CUBE_MAP_FILE_NAMES + face * 4);
        let pixel_data: usize = if file_name != 0 {
            let converter = get_image_converter();
            thiscall2(virtual_at(converter, V_CONVERTER_READ_IMAGE_FILE), converter, file_name as u32, 0) as usize
        } else {
            rd(texture, SOURCE_CUBE_MAP_PIXEL_DATA + face * 4)
        };
        let old = sources[face];
        if old != pixel_data {
            if old != 0 {
                ni_object_release(old);
            }
            sources[face] = pixel_data;
            if pixel_data != 0 {
                wr(pixel_data, NiRefObject::refCount, rd::<u32>(pixel_data, NiRefObject::refCount).wrapping_add(1));
            }
        }
        let source = sources[face];
        if source == 0 {
            matched = false;
            break;
        }
        wr::<u32>(data, SOURCE_CUBE_FACE_REVISIONS + face * 4, rd(source, PD::revisionID));
        let width: u32 = rd(rd(source, PD::mipmapWidths), 0);
        if size == 0 {
            if width != rd::<u32>(rd(source, PD::mipmapHeights), 0) {
                matched = false;
                break;
            }
            let width: u32 = rd(rd(source, PD::mipmapWidths), 0);
            if width == 0 || width & width.wrapping_sub(1) != 0 {
                matched = false;
                break;
            }
            size = rd(rd(source, PD::mipmapWidths), 0);
            levels = rd(source, PD::mipmapLevels);
        } else if size != width || size != rd::<u32>(rd(source, PD::mipmapHeights), 0) || levels != rd::<u32>(source, PD::mipmapLevels) {
            matched = false;
            break;
        }
        if first_format != 0 {
            if rd::<u32>(first_format, NiPixelFormat::compareBits) != rd::<u32>(source, PD::format_compareBits)
                || rd::<u32>(first_format, NiPixelFormat::compareBits + 4) != rd::<u32>(source, PD::format_compareBits + 4)
            {
                matched = false;
                break;
            }
        } else {
            first_format = source + PD::format;
        }
    }

    let mut dest_format = 0;
    if matched {
        dest_format = find_matching_pixel_format(data, sources[0] + PD::format, prefs, renderer + CUBE_MAP_FORMATS);
    }
    if dest_format == 0 {
        if data != 0 {
            ni::delete_object(data);
        }
        destroy_pointers(&mut sources);
        return 0;
    }

    let mipmap_flags: u32 = rd(prefs, 4);
    let mipmap =
        mipmap_flags == 1 || (mipmap_flags == 2 && rd::<u8>(MIPMAP_BY_DEFAULT, 0) != 0 && rd::<u8>(renderer, R::maybe_bMipmapCubeMaps) != 0);
    wr::<u8>(data, STD::bMipMap, mipmap as u8);
    wr::<u32>(data, STD::format_d3dFormat, rd(dest_format, PF::d3dFormat));
    pixel_format_copy(data + STD::format, dest_format);
    wr(data, STD::height, size);
    wr(data, STD::width, size);

    let mut converted = [0usize; 6];
    for face in 0..6 {
        let source = sources[face];
        let converter = get_image_converter();
        let mipmap = rd::<u8>(data, STD::bMipMap) as u32;
        let result =
            thiscall4(virtual_at(converter, V_CONVERTER_CONVERT_PIXEL_DATA), converter, source as u32, dest_format as u32, source as u32, mipmap)
                as usize;
        // The assignment to a `NiPointer` in a local array (0x6C136E).
        ni::pointer_assign(&mut converted[face] as *mut usize as usize, result);
        let pixel_data = converted[face];
        if pixel_data == 0 {
            ni::delete_object(data);
            destroy_pointers(&mut converted);
            destroy_pointers(&mut sources);
            return 0;
        }
        // The offset after the last level: the size of all levels.
        let bytes: u32 = rd(rd(pixel_data, PD::mipmapOffsets), rd::<u32>(pixel_data, PD::mipmapLevels) as usize * 4);
        wr(data, STD::formattedSize, rd::<u32>(data, STD::formattedSize).wrapping_add(bytes));
    }

    let level_count: u32 = if rd::<u8>(renderer, R::maybe_bMipmapCubeMaps) != 0 { rd(converted[0], PD::mipmapLevels) } else { 1 };
    let d3d_format: u32 = rd(data, STD::format_d3dFormat);
    let device: usize = rd(rd(data, STD::renderer), R::d3dDevice);
    let texture_slot = data + SOURCE_CUBE_TEXTURE;
    wr(data, STD::levels, level_count);
    let result = device_create_cube_texture(device, rd(data, STD::width), level_count, 0, d3d_format, D3DPOOL_MANAGED, texture_slot);
    if result != 0 {
        debug_print1(STR_CUBE_CREATE_SURF_FAILED, directx_error_name(result));
        wr::<u32>(texture_slot, 0, 0);
    }
    if rd::<usize>(texture_slot, 0) == 0 {
        ni::delete_object(data);
        destroy_pointers(&mut converted);
        destroy_pointers(&mut sources);
        return 0;
    }
    for face in 0..6 {
        copy_face(data, texture_slot, face as u32, converted[face]);
    }

    // The game code gives a block of its stack. Only the size at +0x104 has a value.
    let mut info = [0u32; 0x42];
    info[0x41] = rd(data, STD::formattedSize);
    count_texture_bytes(info.as_mut_ptr() as usize);
    source_texture_data_set_palette(data, rd(sources[0], PD::palette));
    append_renderer_data(data);
    destroy_pointers(&mut converted);
    destroy_pointers(&mut sources);
    data
}

/// 0x6C15B0 `NiDX8SourceCubeMapData::dtor`
pub unsafe fn source_cube_map_data_dtor(this: usize) {
    crate::shadow::touch(&source_cube_map_data_dtor_port::STATE);
    wr::<usize>(this, STD::vtbl_vtbl, VTBL_SOURCE_CUBE_MAP_DATA);
    let texture: usize = rd(this, SOURCE_CUBE_TEXTURE);
    if texture != 0 {
        com_release(texture);
        wr::<u32>(this, SOURCE_CUBE_TEXTURE, 0);
    }
    source_texture_data_dtor(this);
}

/// The call is a skip when the object has a texture: the texture gets its last `Release`.
/// The call is a skip when the object has a palette: the call can delete the palette. The
/// call is a skip when the object has linked data: the virtual deleting destructor of the
/// linked data runs.
unsafe fn source_cube_map_data_dtor_regions(set: &mut Regions, this: usize) {
    if rd::<usize>(this, SOURCE_CUBE_TEXTURE) != 0
        || rd::<usize>(this, STD::d3dTexture) != 0
        || rd::<usize>(this, STD::palette) != 0
        || rd::<usize>(this, STD::vtbl_linkedData) != 0
    {
        set.skip();
    }
    set.add(this, SOURCE_CUBE_SIZE);
}

/// 0x6C1610 `NiDX8SourceCubeMapData::deleting_dtor`. Returns `this`.
pub unsafe fn source_cube_map_data_deleting_dtor(this: usize, flags: u8) -> usize {
    crate::shadow::touch(&source_cube_map_data_deleting_dtor_port::STATE);
    source_cube_map_data_dtor(this);
    if flags & 1 != 0 {
        operator_delete(this);
    }
    this
}

/// 0x6C1630 `NiDX8SourceCubeMapData::GetD3DTexture`
pub unsafe fn source_cube_map_data_get_d3d_texture(this: usize) -> usize {
    crate::shadow::touch(&source_cube_map_data_get_d3d_texture_port::STATE);
    rd(this, SOURCE_CUBE_TEXTURE)
}

/// 0x6C1640 `NiDX8SourceCubeMapData::IsLoaded`
pub unsafe fn source_cube_map_data_is_loaded(this: usize) -> u8 {
    crate::shadow::touch(&source_cube_map_data_is_loaded_port::STATE);
    (rd::<usize>(this, SOURCE_CUBE_TEXTURE) != 0) as u8
}

/// The first of the six pixel data pointers of a `NiSourceCubeMap` that is not 0.
unsafe fn first_face_pixel_data(texture: usize) -> usize {
    for face in 0..6 {
        let pixel_data: usize = rd(texture, SOURCE_CUBE_MAP_PIXEL_DATA + face * 4);
        if pixel_data != 0 {
            return pixel_data;
        }
    }
    0
}

/// 0x6C1650 `NiDX8SourceCubeMapData::Update`: copies again each face whose pixel data has a
/// new revision, and all faces when the palette changed. The game code leaves no result in
/// EAX.
pub unsafe fn source_cube_map_data_update(this: usize) {
    crate::shadow::touch(&source_cube_map_data_update_port::STATE);
    let mut palette_changed = false;
    let texture: usize = rd(this, STD::vtbl_sgTexture);
    let first = first_face_pixel_data(texture);
    if first != 0 && source_texture_data_set_palette(this, rd(first, PD::palette)) != 0 {
        palette_changed = true;
    }
    for face in 0..6 {
        let source: usize = rd(texture, SOURCE_CUBE_MAP_PIXEL_DATA + face * 4);
        if source == 0 {
            continue;
        }
        let revision_slot = this + SOURCE_CUBE_FACE_REVISIONS + face * 4;
        if rd::<u32>(source, PD::revisionID) == rd::<u32>(revision_slot, 0) && !palette_changed {
            continue;
        }
        let converter = get_image_converter();
        let mipmap = rd::<u8>(this, STD::bMipMap) as u32;
        let converted = thiscall4(
            virtual_at(converter, V_CONVERTER_CONVERT_PIXEL_DATA),
            converter,
            source as u32,
            (this + STD::format) as u32,
            source as u32,
            mipmap,
        ) as usize;
        wr::<u32>(revision_slot, 0, rd(source, PD::revisionID));
        if rd::<usize>(this, SOURCE_CUBE_TEXTURE) != 0 {
            copy_face(this, this + SOURCE_CUBE_TEXTURE, face as u32, converted);
        }
        if converted != 0 {
            // A local NiPointer: one reference more, then one less.
            let count = rd::<u32>(converted, NiRefObject::refCount).wrapping_add(1);
            wr(converted, NiRefObject::refCount, count);
            let count = count.wrapping_sub(1);
            wr(converted, NiRefObject::refCount, count);
            if count == 0 {
                ni::delete_object(converted);
            }
        }
    }
}

/// The call is a skip when the call copies a face: that needs `LockRect` of its surfaces and
/// new pixel data.
/// The call copies a face when a face has a new revision or `setPalette` reports a change.
unsafe fn source_cube_map_data_update_regions(set: &mut Regions, this: usize) {
    let texture: usize = rd(this, STD::vtbl_sgTexture);
    let first = first_face_pixel_data(texture);
    if first != 0 {
        let palette: usize = rd(first, PD::palette);
        if palette != 0 && (palette != rd::<usize>(this, STD::palette) || rd::<u32>(palette, PALETTE_REVISION) != rd::<u32>(this, STD::palRevisionID))
        {
            set.skip();
        }
    }
    for face in 0..6 {
        let source: usize = rd(texture, SOURCE_CUBE_MAP_PIXEL_DATA + face * 4);
        if source != 0 && rd::<u32>(source, PD::revisionID) != rd::<u32>(this, SOURCE_CUBE_FACE_REVISIONS + face * 4) {
            set.skip();
        }
    }
    set.add(this, SOURCE_CUBE_SIZE);
}

// ---------------------------------------------------------------------------------------
// NiDX8Renderer
// ---------------------------------------------------------------------------------------

/// Makes the renderer data of each rendered texture again. The pictures in the textures are
/// lost: the game draws a rendered texture again only when it needs a new picture. This is
/// not a function of the game; the capture uses it, so that the making of each rendered
/// texture is in the stream. Returns the number of textures.
///
/// It takes the textures from the list of the textures that are alive. The map of rendered
/// textures of the renderer is no source for this: no function takes a texture out of that
/// map when the texture dies, so the map can have textures that are gone.
pub unsafe fn remake_rendered_textures(this: usize) -> u32 {
    let list = crate::ports::textures::textures_with_data(VTBL_RENDERED_TEXTURE_DATA as u32);
    for &texture in &list {
        crate::ports::textures::renderer_purge_texture(this, texture);
        create_rendered_texture_renderer_data(this, texture);
    }
    list.len() as u32
}

/// 0x6B03B0 `NiDX8Renderer::CreateRenderedTextureRendererData`. Returns 1 when the texture
/// has renderer data.
pub unsafe fn create_rendered_texture_renderer_data(this: usize, texture: usize) -> u8 {
    crate::shadow::touch(&create_rendered_texture_renderer_data_port::STATE);
    if rd::<usize>(texture, NiTexture::rendererData) != 0 {
        return 1;
    }
    let data = rendered_texture_data_create(texture, this);
    if data == 0 {
        return 0;
    }
    hash_map::set(this + R::hashmap_maybeRenderedTextures, texture as u32, data);
    1
}

/// The call is a skip when the call makes the data: `Create` reads the depth buffer of the
/// device, and that adds a reference.
unsafe fn create_rendered_texture_renderer_data_regions(set: &mut Regions, texture: usize) {
    if rd::<usize>(texture, NiTexture::rendererData) == 0 {
        set.skip();
    }
}

/// 0x6B0480 `NiDX8Renderer::CreateRenderedCubeMapRendererData`. Returns 1 when the cube map
/// has renderer data.
pub unsafe fn create_rendered_cube_map_renderer_data(this: usize, texture: usize) -> u8 {
    crate::shadow::touch(&create_rendered_cube_map_renderer_data_port::STATE);
    let map = this + R::hashmap_maybeRenderedCubemaps;
    let node = hash_map::find(map, texture as u32);
    if node != 0 && rd::<usize>(node, THashMap_Node::data) != 0 {
        return 1;
    }
    let data = rendered_cube_map_data_create(texture, this);
    if data == 0 {
        return 0;
    }
    hash_map::set(map, texture as u32, data);
    1
}

/// The call is a skip when the call makes the data: the face targets take surfaces of the
/// cube texture and a reference on the depth buffer.
unsafe fn create_rendered_cube_map_renderer_data_regions(set: &mut Regions, this: usize, texture: usize) {
    let node = hash_map::find(this + R::hashmap_maybeRenderedCubemaps, texture as u32);
    if node == 0 || rd::<usize>(node, THashMap_Node::data) == 0 {
        set.skip();
    }
}

/// The part of the back buffer that `TakeScreenshot` reads: left, top, width and height.
/// Returns `None` when the rectangle starts outside the buffer. `rect` is left, right, top,
/// bottom.
fn screenshot_area(rect: Option<[u32; 4]>, buffer_width: u32, buffer_height: u32) -> Option<[u32; 4]> {
    let Some([left, right, top, bottom]) = rect else {
        return Some([0, 0, buffer_width, buffer_height]);
    };
    if left >= buffer_width || top >= buffer_height {
        return None;
    }
    let mut width = right.wrapping_sub(left).wrapping_add(1);
    if width.wrapping_add(left) > buffer_width {
        width = buffer_width.wrapping_sub(left);
    }
    let mut height = bottom.wrapping_sub(top).wrapping_add(1);
    if height.wrapping_add(top) > buffer_height {
        height = buffer_height.wrapping_sub(top);
    }
    Some([left, top, width, height])
}

unsafe fn screenshot_area_of(this: usize, rect: usize) -> Option<[u32; 4]> {
    let rect = if rect != 0 { Some([rd(rect, RECT_LEFT), rd(rect, RECT_RIGHT), rd(rect, RECT_TOP), rd(rect, RECT_BOTTOM)]) } else { None };
    screenshot_area(rect, rd(this, R::backbufferRenderTarget_width), rd(this, R::backbufferRenderTarget_height))
}

/// `rep movsd` for `bytes / 4` words, then `rep movsb` for the last `bytes & 3` bytes.
unsafe fn copy_row(to: usize, from: usize, bytes: u32) {
    let words = (bytes >> 2) as usize;
    for word in 0..words {
        wr::<u32>(to, word * 4, rd(from, word * 4));
    }
    for byte in words * 4..words * 4 + (bytes & 3) as usize {
        wr::<u8>(to, byte, rd(from, byte));
    }
}

/// 0x6AC050 `NiDX8Renderer::TakeScreenshot`: returns a new `NiPixelData` with a part of the
/// back buffer, or 0. `rect` is a `NiRect<unsigned int>`, or 0 for the full back buffer. The
/// game code gives the width and the height as the right and bottom of the rectangle it locks.
pub unsafe fn take_screenshot(this: usize, rect: usize) -> usize {
    crate::shadow::touch(&take_screenshot_port::STATE);
    let Some([left, top, width, height]) = screenshot_area_of(this, rect) else {
        return 0;
    };
    let block = engine::operator_new(PD::SIZE);
    let pixel_data =
        if block != 0 { pixel_data_ctor_args(block, width, height, rd(this, R::backbufferRenderTarget_DX8PixelFormat), 1) } else { 0 };
    let surface: usize = rd(this, R::backbufferRenderTarget_d3dSurfaceBackBuffer);
    let mut desc = [0u32; 8];
    surface_get_desc(surface, desc.as_mut_ptr() as usize);
    let lock_rect = [left, top, width, height];
    // D3DLOCKED_RECT: the pitch, then the address of the bits.
    let mut locked = [0u32; 2];
    if surface_lock_rect(surface, locked.as_mut_ptr() as usize, lock_rect.as_ptr() as usize, D3DLOCK_READONLY) != 0 {
        return 0;
    }
    let mut from = locked[1] as usize;
    let row_bytes = rd::<u32>(rd(pixel_data, PD::mipmapWidths), 0).wrapping_mul(rd(pixel_data, PD::bytesPerPixel));
    let mut to = (rd::<u32>(rd(pixel_data, PD::mipmapOffsets), 0) as usize).wrapping_add(rd(pixel_data, PD::pixelsAllMips));
    for _ in 0..height {
        copy_row(to, from, row_bytes);
        to = to.wrapping_add(row_bytes as usize);
        from = from.wrapping_add(locked[0] as usize);
    }
    surface_unlock_rect(surface);
    pixel_data
}

/// The call is a skip when the call reads the back buffer: `LockRect` of a surface.
unsafe fn take_screenshot_regions(set: &mut Regions, this: usize, rect: usize) {
    if screenshot_area_of(this, rect).is_some() {
        set.skip();
    }
}

/// The source rectangle (left, top, right, bottom) of `ImageBlt`. `rect` is left, right,
/// top, bottom, or `None` for the full source.
fn blt_source_rect(rect: Option<[u32; 4]>, source: [u32; 2], buffer: [u32; 2], dest: [u32; 2]) -> [u32; 4] {
    let [source_width, source_height] = source;
    let (left, top, mut right, mut bottom) = match rect {
        Some([left, right, top, bottom]) => (left.min(source_width), top.min(source_height), right.min(source_width), bottom.min(source_height)),
        None => (0, 0, source_width, source_height),
    };
    let over = right.wrapping_sub(buffer[0]).wrapping_sub(left).wrapping_add(dest[0]);
    if over as i32 > 0 {
        right = right.wrapping_sub(over);
    }
    let over = bottom.wrapping_sub(buffer[1]).wrapping_sub(top).wrapping_add(dest[1]);
    if over as i32 > 0 {
        bottom = bottom.wrapping_sub(over);
    }
    [left, top, right, bottom]
}

/// 0x6AC1D0 `NiDX8Renderer::ImageBlt`: copies a part of a `NiBltSource` to the back buffer.
pub unsafe fn image_blt(this: usize, source: usize, rect: usize, dest_x: u32, dest_y: u32) {
    crate::shadow::touch(&image_blt_port::STATE);
    if source == 0 {
        return;
    }
    let back_buffer: usize = rd(this, R::backbufferRenderTarget_d3dSurfaceBackBuffer);
    let mut data: usize = rd(source, BLT_SOURCE_RENDERER_DATA);
    if data == 0 {
        thiscall1(virtual_at(this, V_RENDERER_CREATE_BLT_SOURCE_DATA), this, source as u32);
        data = rd(source, BLT_SOURCE_RENDERER_DATA);
    }
    let surface: usize = rd(data, BLT_DATA_SURFACE);
    if surface == 0 {
        return;
    }
    let rect = if rect != 0 { Some([rd(rect, RECT_LEFT), rd(rect, RECT_RIGHT), rd(rect, RECT_TOP), rd(rect, RECT_BOTTOM)]) } else { None };
    let source_width = texture_get_width(source);
    let source_height = texture_get_height(source);
    let buffer = [rd(this, R::backbufferRenderTarget_width), rd(this, R::backbufferRenderTarget_height)];
    let point = [dest_x, dest_y];
    let source_rect = blt_source_rect(rect, [source_width, source_height], buffer, point);
    device_copy_rects(rd(this, R::d3dDevice), surface, source_rect.as_ptr() as usize, 1, back_buffer, point.as_ptr() as usize);
}

/// The call is a skip when the source has no renderer data: the call makes the data. The call
/// is a skip when the source has a surface: the call does `CopyRects`.
unsafe fn image_blt_regions(set: &mut Regions, source: usize) {
    if source != 0 {
        let data: usize = rd(source, BLT_SOURCE_RENDERER_DATA);
        if data == 0 || rd::<usize>(data, BLT_DATA_SURFACE) != 0 {
            set.skip();
        }
    }
}

// ---------------------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------------------

port_thiscall!(set_pixel_format_port, 0x4B52D0, "NiDX8PixelFormat::setPixelFormat",
    fn(this, d3d_format: u32) -> usize,
    compare_eax = true,
    regions = |set| set.add(this, PF::SIZE),
    rust = set_pixel_format);

port_thiscall!(render_target_dtor_port, 0x6A9DA0, "NiDX8RenderTarget::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| render_target_dtor_regions(set, this),
    rust = render_target_dtor);

port_thiscall!(pixel_format_ctor_port, 0x6AB410, "NiDX8PixelFormat::ctor",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| set.add(this, NiPixelFormat::SIZE),
    rust = pixel_format_ctor);

port_thiscall!(take_screenshot_port, 0x6AC050, "NiDX8Renderer::TakeScreenshot",
    fn(this, rect: usize) -> usize,
    compare_eax = true,
    regions = |set| take_screenshot_regions(set, this, rect),
    rust = take_screenshot);

port_thiscall!(image_blt_port, 0x6AC1D0, "NiDX8Renderer::ImageBlt",
    fn(this, source: usize, rect: usize, dest_x: u32, dest_y: u32) -> (),
    compare_eax = false,
    regions = |set| image_blt_regions(set, source),
    rust = image_blt);

port_thiscall!(create_rendered_texture_renderer_data_port, 0x6B03B0, "NiDX8Renderer::CreateRenderedTextureRendererData",
    fn(this, texture: usize) -> u8,
    compare_eax = true,
    regions = |set| create_rendered_texture_renderer_data_regions(set, texture),
    rust = create_rendered_texture_renderer_data);

port_thiscall!(create_rendered_cube_map_renderer_data_port, 0x6B0480, "NiDX8Renderer::CreateRenderedCubeMapRendererData",
    fn(this, texture: usize) -> u8,
    compare_eax = true,
    regions = |set| create_rendered_cube_map_renderer_data_regions(set, this, texture),
    rust = create_rendered_cube_map_renderer_data);

port_cdecl!(get_d3d_format_for_pixel_format_port, 0x6B0E20, "NiGetD3DFormatForNiPixelFormat",
    fn(format: usize) -> u32,
    compare_eax = true,
    regions = |set| {},
    rust = get_d3d_format_for_pixel_format);

// The call is always a skip: `Create` reads the depth buffer of the device, and that adds a
// reference. Or it deletes the new object through its virtual destructor.
port_cdecl!(rendered_texture_data_create_port, 0x6C01B0, "NiDX8RenderedTextureData_static::Create",
    fn(texture: usize, renderer: usize) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = rendered_texture_data_create);

port_thiscall!(rendered_texture_data_ctor_port, 0x6C0480, "NiDX8RenderedTextureData::ctor",
    fn(this, texture: usize, renderer: usize) -> usize,
    compare_eax = true,
    regions = |set| set.add(this, RTD::SIZE),
    rust = rendered_texture_data_ctor);

port_thiscall!(rendered_texture_data_deleting_dtor_port, 0x6C0500, "NiDX8RenderedTextureData::deleting_dtor",
    fn(this, flags: u8) -> usize,
    compare_eax = true,
    regions = |set| {
        rendered_texture_data_dtor_regions(set, this, RTD::SIZE);
        if flags & 1 != 0 {
            set.allocates();
        }
    },
    rust = rendered_texture_data_deleting_dtor);

port_thiscall!(rendered_texture_data_dtor_port, 0x6C0520, "NiDX8RenderedTextureData::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| rendered_texture_data_dtor_regions(set, this, RTD::SIZE),
    rust = rendered_texture_data_dtor);

port_thiscall!(rendered_texture_data_get_d3d_texture_port, 0x6C05B0, "NiDX8RenderedTextureData::GetD3DTexture",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| {},
    rust = rendered_texture_data_get_d3d_texture);

port_thiscall!(rendered_texture_data_is_loaded_port, 0x6C05C0, "NiDX8RenderedTextureData::IsLoaded",
    fn(this) -> u8,
    compare_eax = true,
    regions = |set| {},
    rust = rendered_texture_data_is_loaded);

port_thiscall!(rendered_texture_data_create_surf_port, 0x6C05D0, "NiDX8RenderedTextureData::CreateSurf",
    fn(this, texture: usize, out_format: usize) -> usize,
    compare_eax = true,
    regions = |set| rendered_texture_data_create_surf_regions(set, this, texture, out_format),
    rust = rendered_texture_data_create_surf);

port_cdecl!(texture_render_target_dtor_port, 0x6C07D0, "NiDX8TextureRenderTarget::dtor",
    fn(target: usize) -> (),
    compare_eax = false,
    regions = |set| texture_render_target_dtor_regions(set, target),
    rust = texture_render_target_dtor);

port_thiscall!(rendered_texture_data_get_target_port, 0x6C09A0, "NiDX8RenderedTextureData::GetTarget",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| {},
    rust = rendered_texture_data_get_target);

// The call is always a skip: each face target takes a surface of the cube texture and a
// reference on the depth buffer. Or the call deletes the new object through its virtual
// destructor.
port_cdecl!(rendered_cube_map_data_create_port, 0x6C09D0, "NiDX8RenderedCubeMapData::ctor",
    fn(texture: usize, renderer: usize) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = rendered_cube_map_data_create);

port_thiscall!(rendered_cube_map_data_deleting_dtor_port, 0x6C0B00, "NiDX8RenderedCubeMapData::deleting_dtor",
    fn(this, flags: u8) -> usize,
    compare_eax = true,
    regions = |set| {
        rendered_cube_map_data_dtor_regions(set, this);
        if flags & 1 != 0 {
            set.allocates();
        }
    },
    rust = rendered_cube_map_data_deleting_dtor);

port_thiscall!(rendered_cube_map_data_dtor_port, 0x6C0B20, "NiDX8RenderedCubeMapData::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| rendered_cube_map_data_dtor_regions(set, this),
    rust = rendered_cube_map_data_dtor);

port_thiscall!(rendered_cube_map_data_get_d3d_texture_port, 0x6C0BB0, "NiDX8RenderedCubeMapData::GetD3DTexture",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| {},
    rust = rendered_cube_map_data_get_d3d_texture);

port_thiscall!(rendered_cube_map_data_is_loaded_port, 0x6C0BC0, "NiDX8RenderedCubeMapData::IsLoaded",
    fn(this) -> u8,
    compare_eax = true,
    regions = |set| {},
    rust = rendered_cube_map_data_is_loaded);

port_thiscall!(rendered_cube_map_data_get_target_port, 0x6C0BD0, "NiDX8RenderedCubeMapData::GetTarget",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| {},
    rust = rendered_cube_map_data_get_target);

// The call is always a skip: the call copies the faces with `LockRect` of their surfaces, and
// a face can come from a file. The failure paths delete the new object through its virtual
// destructor.
port_cdecl!(source_cube_map_data_create_port, 0x6C10A0, "NiDX8SourceCubeMapData_static::Create",
    fn(texture: usize, renderer: usize) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = source_cube_map_data_create);

port_thiscall!(source_cube_map_data_dtor_port, 0x6C15B0, "NiDX8SourceCubeMapData::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| source_cube_map_data_dtor_regions(set, this),
    rust = source_cube_map_data_dtor);

port_thiscall!(source_cube_map_data_deleting_dtor_port, 0x6C1610, "NiDX8SourceCubeMapData::deleting_dtor",
    fn(this, flags: u8) -> usize,
    compare_eax = true,
    regions = |set| {
        source_cube_map_data_dtor_regions(set, this);
        if flags & 1 != 0 {
            set.allocates();
        }
    },
    rust = source_cube_map_data_deleting_dtor);

port_thiscall!(source_cube_map_data_get_d3d_texture_port, 0x6C1630, "NiDX8SourceCubeMapData::GetD3DTexture",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| {},
    rust = source_cube_map_data_get_d3d_texture);

port_thiscall!(source_cube_map_data_is_loaded_port, 0x6C1640, "NiDX8SourceCubeMapData::IsLoaded",
    fn(this) -> u8,
    compare_eax = true,
    regions = |set| {},
    rust = source_cube_map_data_is_loaded);

port_thiscall!(source_cube_map_data_update_port, 0x6C1650, "NiDX8SourceCubeMapData::Update",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| source_cube_map_data_update_regions(set, this),
    rust = source_cube_map_data_update);

pub static PORTS: &[&crate::shadow::PortState] = &[
    &set_pixel_format_port::STATE,
    &render_target_dtor_port::STATE,
    &pixel_format_ctor_port::STATE,
    &take_screenshot_port::STATE,
    &image_blt_port::STATE,
    &create_rendered_texture_renderer_data_port::STATE,
    &create_rendered_cube_map_renderer_data_port::STATE,
    &get_d3d_format_for_pixel_format_port::STATE,
    &rendered_texture_data_create_port::STATE,
    &rendered_texture_data_ctor_port::STATE,
    &rendered_texture_data_deleting_dtor_port::STATE,
    &rendered_texture_data_dtor_port::STATE,
    &rendered_texture_data_get_d3d_texture_port::STATE,
    &rendered_texture_data_is_loaded_port::STATE,
    &rendered_texture_data_create_surf_port::STATE,
    &texture_render_target_dtor_port::STATE,
    &rendered_texture_data_get_target_port::STATE,
    &rendered_cube_map_data_create_port::STATE,
    &rendered_cube_map_data_deleting_dtor_port::STATE,
    &rendered_cube_map_data_dtor_port::STATE,
    &rendered_cube_map_data_get_d3d_texture_port::STATE,
    &rendered_cube_map_data_is_loaded_port::STATE,
    &rendered_cube_map_data_get_target_port::STATE,
    &source_cube_map_data_create_port::STATE,
    &source_cube_map_data_dtor_port::STATE,
    &source_cube_map_data_deleting_dtor_port::STATE,
    &source_cube_map_data_get_d3d_texture_port::STATE,
    &source_cube_map_data_is_loaded_port::STATE,
    &source_cube_map_data_update_port::STATE,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_bits_of_a_new_pixel_format() {
        assert_eq!(compare_bits(0, 0xF, 4, 0xF0), 0x12);
        assert_eq!(compare_bits(0, 0x1, 4, 0xF0), 0x10);
        // A shift uses its count modulo 32.
        assert_eq!(compare_bits(33, !0, 32, !0), 4 | 1);
    }

    #[test]
    fn constructor_for_each_d3d_format() {
        let argb = FormatCtor::Rgba(0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000, 32);
        let xrgb = FormatCtor::Rgba(0xFF_0000, 0xFF00, 0xFF, 0, 32);
        assert_eq!(format_ctor_for(20), xrgb);
        assert_eq!(format_ctor_for(21), argb);
        assert_eq!(format_ctor_for(22), xrgb);
        assert_eq!(format_ctor_for(23), FormatCtor::Rgba(0xF800, 0x7E0, 0x1F, 0, 16));
        assert_eq!(format_ctor_for(24), FormatCtor::Rgba(0x7C00, 0x3E0, 0x1F, 0, 16));
        assert_eq!(format_ctor_for(25), FormatCtor::Rgba(0x7C00, 0x3E0, 0x1F, 0x8000, 16));
        assert_eq!(format_ctor_for(26), FormatCtor::Rgba(0xF00, 0xF0, 0xF, 0xF000, 16));
        assert_eq!(format_ctor_for(41), FormatCtor::Palette(0));
        assert_eq!(format_ctor_for(60), FormatCtor::BumpLuma(0xFF, 0xFF00, 0, 16));
        assert_eq!(format_ctor_for(61), FormatCtor::BumpLuma(0x1F, 0x3E0, 0xFC00, 16));
        assert_eq!(format_ctor_for(62), FormatCtor::BumpLuma(0xFF, 0xFF00, 0xFF_0000, 32));
        assert_eq!(format_ctor_for(u32::from_le_bytes(*b"DXT1")), FormatCtor::Format(6));
        assert_eq!(format_ctor_for(u32::from_le_bytes(*b"DXT3")), FormatCtor::Format(7));
        assert_eq!(format_ctor_for(u32::from_le_bytes(*b"DXT5")), FormatCtor::Format(8));
        for other in [0, 19, 27, 40, 42, 59, 63, u32::from_le_bytes(*b"DXT2"), u32::from_le_bytes(*b"DXT4"), 0x8000_0000] {
            assert_eq!(format_ctor_for(other), argb, "format {other}");
        }
    }

    #[test]
    fn d3d_format_for_each_pixel_format() {
        let none = || -> u32 { panic!("the layout is read for 16 bits only") };
        // RGB
        assert_eq!(d3d_format_for(0, 8, none), (0, false));
        assert_eq!(d3d_format_for(0, 16, || 6), (23, false));
        assert_eq!(d3d_format_for(0, 16, || 5), (24, false));
        assert_eq!(d3d_format_for(0, 16, || 0x106), (23, false));
        for bits in [0, 7, 9, 24, 32, 33, 255] {
            assert_eq!(d3d_format_for(0, bits, none), (22, false), "{bits} bits");
        }
        // RGBA
        assert_eq!(d3d_format_for(1, 8, none), (0, false));
        assert_eq!(d3d_format_for(1, 16, || 6), (23, false));
        assert_eq!(d3d_format_for(1, 16, || 4), (26, false));
        assert_eq!(d3d_format_for(1, 16, || 5), (25, false));
        assert_eq!(d3d_format_for(1, 24, none), (22, true));
        for bits in [0, 7, 9, 23, 25, 32, 33, 255] {
            assert_eq!(d3d_format_for(1, bits, none), (21, false), "{bits} bits");
        }
        assert_eq!(d3d_format_for(2, 8, none), (41, false));
        assert_eq!(d3d_format_for(3, 8, none), (41, false));
        assert_eq!(d3d_format_for(4, 8, none), (22, false));
        assert_eq!(d3d_format_for(5, 8, none), (21, false));
        assert_eq!(d3d_format_for(6, 0, none).0, u32::from_le_bytes(*b"DXT1"));
        assert_eq!(d3d_format_for(7, 0, none).0, u32::from_le_bytes(*b"DXT3"));
        assert_eq!(d3d_format_for(8, 0, none).0, u32::from_le_bytes(*b"DXT5"));
        assert_eq!(d3d_format_for(9, 16, none), (0, false));
        assert_eq!(d3d_format_for(10, 16, none), (60, false));
        assert_eq!(d3d_format_for(10, 32, none), (0, false));
        assert_eq!(d3d_format_for(11, 16, none), (61, false));
        assert_eq!(d3d_format_for(11, 32, none), (62, false));
        assert_eq!(d3d_format_for(11, 24, none), (0, false));
        assert_eq!(d3d_format_for(12, 16, none), (0, false));
    }

    #[test]
    fn screenshot_area_is_cut_to_the_back_buffer() {
        assert_eq!(screenshot_area(None, 800, 600), Some([0, 0, 800, 600]));
        assert_eq!(screenshot_area(Some([10, 19, 20, 49]), 800, 600), Some([10, 20, 10, 30]));
        assert_eq!(screenshot_area(Some([790, 900, 590, 900]), 800, 600), Some([790, 590, 10, 10]));
        assert_eq!(screenshot_area(Some([799, 799, 599, 599]), 800, 600), Some([799, 599, 1, 1]));
        assert_eq!(screenshot_area(Some([800, 900, 0, 10]), 800, 600), None);
        assert_eq!(screenshot_area(Some([0, 10, 600, 700]), 800, 600), None);
    }

    #[test]
    fn copy_row_copies_words_then_bytes() {
        let from: [u8; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let mut to = [0u8; 12];
        unsafe { copy_row(to.as_mut_ptr() as usize, from.as_ptr() as usize, 7) };
        assert_eq!(to, [1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 0, 0]);
        unsafe { copy_row(to.as_mut_ptr() as usize, from.as_ptr() as usize, 12) };
        assert_eq!(to, from);
    }

    #[test]
    fn blt_rectangle_is_cut_to_the_source_and_the_back_buffer() {
        // The full source, inside the buffer.
        assert_eq!(blt_source_rect(None, [64, 32], [800, 600], [10, 20]), [0, 0, 64, 32]);
        // The full source, across the right and bottom edges.
        assert_eq!(blt_source_rect(None, [64, 32], [800, 600], [780, 590]), [0, 0, 20, 10]);
        // A rectangle larger than the source.
        assert_eq!(blt_source_rect(Some([8, 100, 4, 100]), [64, 32], [800, 600], [0, 0]), [8, 4, 64, 32]);
        assert_eq!(blt_source_rect(Some([100, 100, 100, 100]), [64, 32], [800, 600], [0, 0]), [64, 32, 64, 32]);
        assert_eq!(blt_source_rect(Some([8, 40, 4, 20]), [64, 32], [800, 600], [790, 0]), [8, 4, 18, 20]);
    }

    #[test]
    fn accessors_read_the_members() {
        let mut data = [0u8; RENDERED_CUBE_SIZE];
        let mut cube_map = [0u8; 0x40];
        let (d, c) = (data.as_mut_ptr() as usize, cube_map.as_mut_ptr() as usize);
        unsafe {
            assert_eq!(rendered_texture_data_is_loaded(d), 0);
            assert_eq!(rendered_cube_map_data_is_loaded(d), 0);
            assert_eq!(source_cube_map_data_is_loaded(d), 0);
            wr::<u32>(d, 0x68, 0x1111);
            wr::<u32>(d, 0x78, 0x2222);
            wr::<u32>(d, 0x7C, 0x3333);
            wr::<u32>(d, 0x80, 0x4444);
            assert_eq!(source_cube_map_data_get_d3d_texture(d), 0x1111);
            assert_eq!(source_cube_map_data_is_loaded(d), 1);
            assert_eq!(rendered_texture_data_get_d3d_texture(d), 0x2222);
            assert_eq!(rendered_texture_data_is_loaded(d), 1);
            assert_eq!(rendered_texture_data_get_target(d), 0x3333);
            assert_eq!(rendered_cube_map_data_get_d3d_texture(d), 0x4444);
            assert_eq!(rendered_cube_map_data_is_loaded(d), 1);
            wr(d, 8, c);
            for face in 0..6u32 {
                wr::<u32>(d, 0x84 + face as usize * 4, 0x100 + face);
            }
            wr::<u32>(c, 0x34, 4);
            assert_eq!(rendered_cube_map_data_get_target(d), 0x104);
        }
    }

    /// A fake `NiRefObject` whose deleting destructor counts its calls.
    static DELETED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

    unsafe extern "thiscall" fn fake_deleting_dtor(_this: usize, _flags: u32) -> u32 {
        DELETED.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        0
    }

    #[test]
    fn render_target_dtor_releases_its_two_objects() {
        let vtable = [fake_deleting_dtor as unsafe extern "thiscall" fn(usize, u32) -> u32 as usize];
        let mut kept = [vtable.as_ptr() as usize as u32, 2];
        let mut last = [vtable.as_ptr() as usize as u32, 1];
        let mut target = [0u8; RT::SIZE];
        let t = target.as_mut_ptr() as usize;
        unsafe {
            wr(t, RT::field_18, kept.as_mut_ptr() as usize);
            wr(t, RT::field_C, last.as_mut_ptr() as usize);
            let mut set = Regions::new();
            render_target_dtor_regions(&mut set, t);
            assert_eq!(set.as_slice().len(), 2);
            render_target_dtor(t);
        }
        assert_eq!(kept[1], 1);
        assert_eq!(last[1], 0);
        assert_eq!(DELETED.load(core::sync::atomic::Ordering::Relaxed), 1);
        // The destructor does not clear the members.
        assert_eq!(unsafe { rd::<usize>(t, RT::field_C) }, last.as_ptr() as usize);
    }

    #[test]
    fn renderer_data_goes_to_the_end_of_the_list() {
        let mut texture = [0u8; NiTexture::SIZE];
        let mut first = [0u8; RTD::SIZE];
        let mut second = [0u8; RTD::SIZE];
        let (t, a, b) = (texture.as_mut_ptr() as usize, first.as_mut_ptr() as usize, second.as_mut_ptr() as usize);
        unsafe {
            wr(a, RTD::vtbl_sgTexture, t);
            wr(b, RTD::vtbl_sgTexture, t);
            append_renderer_data(a);
            assert_eq!(rd::<usize>(t, NiTexture::rendererData), a);
            append_renderer_data(b);
            assert_eq!(rd::<usize>(t, NiTexture::rendererData), a);
            assert_eq!(rd::<usize>(a, RTD::vtbl_linkedData), b);
        }
    }
}
