//! Textures: `NiDX8TextureManager`, the base class `NiDX8TextureData`, `NiDX8SourceTextureData`
//! (a texture made from pixel data), `NiDX8BltSourceData` (an image surface for a blit) and
//! the texture entry points of `NiDX8Renderer`.

use super::{hash_map, ni};
use crate::d3d::{
    com_add_ref, com_release, device_create_image_surface, surface_get_desc, surface_lock_rect, surface_unlock_rect, texture_get_surface_level,
};
use crate::engine::{thiscall4, vcall0, vcall1, vcall2, virtual_at, VTBL_NI_REF_OBJECT};
use crate::gen::layouts;
use crate::gen::layouts::{
    NiDX8PixelFormat as PF, NiDX8Renderer as R, NiDX8SourceTextureData as SD, NiDX8TextureData as TD, NiDX8TextureManager as TM,
    NiPixelData as PD, NiSourceTexture as ST, NiTexture as T, THashMap as HM, THashMap_Node as HN,
};
use crate::main_thread::MainThread;
use crate::mem::{rd, wr};
use crate::ports::ni::REF_COUNT;
use crate::shadow::Regions;

// ---- members that are not in `gen::layouts`

const PALETTE_REVISION: usize = layouts::NiPalette::revisionID;
const PREFS_LAYOUT: usize = layouts::NiFormatPrefs::pixelLayout;
const PREFS_ALPHA: usize = layouts::NiFormatPrefs::alphaFormat;
/// `NiDX8RenderTarget::DX8PixelFormat` of the current render target of the renderer
/// (0x6B66D7).
const RENDER_TARGET_PIXEL_FORMAT: usize = layouts::NiDX8RenderTarget::DX8PixelFormat;

// `NiBltSource` (0x6B66AC, 0x6B66C8, 0x6B67AC)
const BLT_FILENAME: usize = layouts::NiBltSource::pcFilename;
const BLT_PIXEL_DATA: usize = layouts::NiBltSource::pixelData;
const BLT_RENDERER_DATA: usize = layouts::NiBltSource::rendererData;

// `NiDX8BltSourceData` (0x6B672E)
const BD_SIZE: usize = layouts::NiDX8BltSourceData::SIZE;
const BD_LINKED: usize = layouts::NiDX8BltSourceData::linkedData;
const BD_SOURCE: usize = layouts::NiDX8BltSourceData::bltSource;
const BD_RENDERER: usize = layouts::NiDX8BltSourceData::renderer;
const BD_SURFACE: usize = layouts::NiDX8BltSourceData::surface;
const BD_FIELD_18: usize = layouts::NiDX8BltSourceData::unknown_18;
const BD_WIDTH: usize = layouts::NiDX8BltSourceData::width;
const BD_HEIGHT: usize = layouts::NiDX8BltSourceData::height;

/// A value in a hash map of the texture manager: a device object that the value holds (0x6BC3E2).
const MAP_VALUE_DEVICE_OBJECT: usize = 0x2C;

// `D3DLOCKED_RECT` and `D3DSURFACE_DESC`, as arrays of 32-bit words.
const LOCK_PITCH: usize = 0;
const LOCK_BITS: usize = 1;
const DESC_HEIGHT: usize = 7;

// ---- vtables and their slots

const VTBL_TEXTURE_MANAGER: u32 = 0x74F6FC;
const VTBL_HASH_MAP: u32 = 0x74F710;
const VTBL_RENDERER_DATA: u32 = 0x74F6B0;
const VTBL_TEXTURE_DATA: u32 = 0x74F688;
const VTBL_SOURCE_TEXTURE_DATA: u32 = 0x74F770;
const VTBL_BLT_SOURCE_DATA: u32 = 0x74F670;
const VTBL_BLT_RENDERER_DATA: u32 = 0x74F67C;
/// `NiSourceTexture::ms_RTTI`
const RTTI_SOURCE_TEXTURE: usize = 0x7DE3C0;

// `NiDX8TextureData`. `Update` is in the vtable of `NiDX8SourceTextureData` and of its subclasses.
const VT_DATA_GET_PIXEL_FORMAT: usize = layouts::InterfaceNiDX8TextureData::GetPixelFormat;
const VT_DATA_GET_LEVELS: usize = layouts::InterfaceNiDX8TextureData::GetLevels;
const VT_DATA_GET_D3D_TEXTURE: usize = layouts::InterfaceNiDX8TextureData::GetD3DTexture;
const VT_DATA_GET_AS_RENDERED: usize = layouts::InterfaceNiDX8TextureData::GetAsRenderedTexture;
const VT_DATA_UPDATE: usize = layouts::InterfaceNiDX8SourceTextureData::Update;
const VT_RENDERER_CREATE_SOURCE_DATA: usize = layouts::InterfaceNiRenderer::CreateSourceTextureRendererData;
const VT_CONVERTER_READ_FILE: usize = layouts::NiImageConverter_vtbl::ReadImageFile;
const VT_CONVERTER_CAN_CONVERT: usize = layouts::NiImageConverter_vtbl::CanConvertPixelData;
const VT_CONVERTER_CONVERT: usize = layouts::NiImageConverter_vtbl::ConvertPixelData;

// The game code gives these texts to its debug output. The debug output does nothing.
const TEXT_UPDATE_LEVEL: usize = 0x7C40B8;
const TEXT_COPY_LOCK: usize = 0x7C4170;
const TEXT_COPY_DESC: usize = 0x7C41B8;
const TEXT_BLT_CREATE: usize = 0x7C37F4;
const TEXT_BLT_DESC: usize = 0x7C37C0;
const TEXT_BLT_LOCK: usize = 0x7C3788;

const _: () = assert!(TM::hashMap_10 == 0x10 && TM::hashMap_20 == 0x20 && TM::hashmap_30 == 0x30 && TM::SIZE == 0x40);
const _: () = assert!(TD::SIZE == 0x50 && SD::SIZE == 0x68 && SD::d3dTexture == 0x60 && SD::palette == 0x58);
const _: () = assert!(HN::key == 0 && HN::data == 4 && HN::nextNode == 8 && T::rendererData == 0x20);

// ---- engine functions and data at fixed addresses

/// The game functions and the game data that the unit tests replace. A function that
/// `engine` or `ni` has comes from there. The others are only in this file.
#[cfg(not(test))]
mod eng {
    use crate::engine::{cdecl2, PIXEL_FORMAT_MASK_A, PIXEL_FORMAT_MASK_B, PIXEL_FORMAT_SHIFT_A, PIXEL_FORMAT_SHIFT_B};
    use crate::mem::rd;

    pub use crate::engine::{
        debug_print1, debug_print2, directx_error_name, dx8_resource_release, dx8_resource_release_thunk, get_image_converter, ni_object_release,
        operator_delete, operator_new, texture_renderer_data_dtor,
    };
    pub use crate::ports::ni::ref_object_count_add;

    /// `global_NiTexture_listHead` (0x7DE7F0)
    pub unsafe fn texture_list_head() -> usize {
        rd(0x7DE7F0, 0)
    }
    /// 0x6BFA50 `NiDX8SourceTextureData_static::Create`. It is `__cdecl`.
    pub unsafe fn source_texture_data_create(texture: usize, renderer: usize) -> usize {
        cdecl2(0x6BFA50, texture as u32, renderer as u32) as usize
    }
    /// 0x6C10A0 `NiDX8SourceCubeMapData_static::Create`. It is `__cdecl`.
    pub unsafe fn source_cube_map_data_create(texture: usize, renderer: usize) -> usize {
        cdecl2(0x6C10A0, texture as u32, renderer as u32) as usize
    }
    /// 0x6ECA50 `game_textureMemoryRemove_Claude_`. It is `__cdecl`. It takes the size of a
    /// texture off the totals of the game; its one caller in the game is
    /// `NiDX8SourceTextureData::dtor`.
    pub unsafe fn texture_freed(texture: usize, size: u32) {
        cdecl2(0x6ECA50, texture as u32, size);
    }
    /// `dword_7DE8FC`, `dword_7C55B0`, `dword_7C55B8`, `dword_7C55B4`. The game makes the compare
    /// bits of a new pixel format from them (0x6B6D2F).
    pub unsafe fn format_bit_globals() -> [u32; 4] {
        [rd(PIXEL_FORMAT_SHIFT_A, 0), rd(PIXEL_FORMAT_MASK_A, 0), rd(PIXEL_FORMAT_SHIFT_B, 0), rd(PIXEL_FORMAT_MASK_B, 0)]
    }
    /// The Morrowind Code Patch (MCP-039) changes four 8-bit displacements in
    /// `FindMatchingPixelFormat`. They are at 0x6B70A8, 0x6B70DF, 0x6B7357 and 0x6B737A. Each is
    /// the offset of a format in the list. This function reads them from the game code. Thus the
    /// port follows the patch state of the game it runs in. The database has 8, 4, 0x14, 8.
    /// `FOLLOWED` in `src/ports/mod.rs` declares these four addresses.
    pub unsafe fn mcp039_offsets() -> [usize; 4] {
        [rd::<u8>(0x6B70A8, 0) as usize, rd::<u8>(0x6B70DF, 0) as usize, rd::<u8>(0x6B7357, 0) as usize, rd::<u8>(0x6B737A, 0) as usize]
    }
}

/// Replacements for the unit tests. The unit tests run without the game.
#[cfg(test)]
mod eng {
    use crate::mem::{rd, wr};

    pub unsafe fn operator_new(size: usize) -> usize {
        Box::leak(vec![0xCDu8; size].into_boxed_slice()).as_mut_ptr() as usize
    }
    pub unsafe fn operator_delete(block: usize) {
        super::tests::note(0xDE1E7E, block as u32);
    }
    pub unsafe fn ref_object_count_add(_delta: i32) {}
    pub unsafe fn texture_list_head() -> usize {
        0
    }
    pub unsafe fn get_image_converter() -> usize {
        super::tests::converter()
    }
    pub unsafe fn source_texture_data_create(_texture: usize, _renderer: usize) -> usize {
        0
    }
    pub unsafe fn source_cube_map_data_create(_texture: usize, _renderer: usize) -> usize {
        0
    }
    pub unsafe fn texture_renderer_data_dtor(data: usize) {
        wr(data, 0, super::VTBL_RENDERER_DATA);
    }
    pub unsafe fn ni_object_release(object: usize) {
        wr(object, 4, rd::<u32>(object, 4).wrapping_sub(1));
    }
    pub unsafe fn directx_error_name(_result: u32) -> usize {
        0
    }
    pub unsafe fn debug_print1(_text: usize, _a: usize) {}
    pub unsafe fn debug_print2(_text: usize, _a: usize, _b: usize) {}
    pub unsafe fn texture_freed(_texture: usize, _size: u32) {}
    pub unsafe fn dx8_resource_release(_value: usize) {}
    pub unsafe fn dx8_resource_release_thunk(_value: usize) {}
    pub const TEST_FORMAT_BITS: [u32; 4] = [3, 0x18, 2, 0xFF];
    pub unsafe fn format_bit_globals() -> [u32; 4] {
        TEST_FORMAT_BITS
    }
    pub unsafe fn mcp039_offsets() -> [usize; 4] {
        [8, 4, 0x14, 8]
    }
}

// ---- calls through a vtable

#[inline(always)]
unsafe fn data_get_pixel_format(data: usize) -> usize {
    vcall0(data, VT_DATA_GET_PIXEL_FORMAT) as usize
}

#[inline(always)]
unsafe fn data_get_levels(data: usize) -> u32 {
    vcall0(data, VT_DATA_GET_LEVELS)
}

#[inline(always)]
pub(crate) unsafe fn data_get_d3d_texture(data: usize) -> usize {
    vcall0(data, VT_DATA_GET_D3D_TEXTURE) as usize
}

#[inline(always)]
pub(crate) unsafe fn data_get_as_rendered(data: usize) -> usize {
    vcall0(data, VT_DATA_GET_AS_RENDERED) as usize
}

#[inline(always)]
unsafe fn data_update(data: usize) {
    vcall0(data, VT_DATA_UPDATE);
}

#[inline(always)]
unsafe fn renderer_create_source_data(renderer: usize, texture: usize) -> u8 {
    vcall1(renderer, VT_RENDERER_CREATE_SOURCE_DATA, texture as u32) as u8
}

/// `NiImageConverter` slot 0x8: pixel data from a file.
#[inline(always)]
pub(crate) unsafe fn converter_read_file(converter: usize, filename: usize, arg: u32) -> usize {
    vcall2(converter, VT_CONVERTER_READ_FILE, filename as u32, arg) as usize
}

/// `NiImageConverter` slot 0xC. Returns true when it can convert the source format to the other.
#[inline(always)]
unsafe fn converter_can_convert(converter: usize, source: usize, dest: usize) -> bool {
    vcall2(converter, VT_CONVERTER_CAN_CONVERT, source as u32, dest as u32) & 0xFF != 0
}

/// `NiImageConverter` slot 0x10: pixel data in the wanted format. This is the source when the
/// source fits.
#[inline(always)]
pub(crate) unsafe fn converter_convert(converter: usize, source: usize, format: usize, scratch: usize, mipmap: u8) -> usize {
    thiscall4(virtual_at(converter, VT_CONVERTER_CONVERT), converter, source as u32, format as u32, scratch as u32, mipmap as u32) as usize
}

// ---- shared pieces the game code has inline

/// For every value of a map: releases its device object, then destroys and frees the value.
/// The nodes stay. The port reads the value of a node before the `hashKey` call for the next
/// bucket. [`hash_map::next`] has no such read, thus this function has that step in its body.
#[inline(always)]
unsafe fn map_destroy_values(map: usize, value_dtor: unsafe fn(usize)) {
    let mut node = hash_map::first(map);
    while node != 0 {
        let mut next: usize = rd(node, HN::nextNode);
        let value: usize = rd(node, HN::data);
        if next == 0 {
            next = hash_map::next_bucket(map, rd(node, HN::key));
        }
        if value != 0 {
            let object: usize = rd(value, MAP_VALUE_DEVICE_OBJECT);
            if object != 0 {
                com_release(object);
            }
            value_dtor(value);
            eng::operator_delete(value);
        }
        node = next;
    }
}

/// Copies rows of `row_bytes` bytes to a locked surface. The rows are `pitch` bytes apart.
#[inline(always)]
unsafe fn copy_rows(mut source: usize, bits: usize, pitch: u32, row_bytes: u32, height: u32) {
    let mut row = 0u32;
    while row < height {
        let dest = (row.wrapping_mul(pitch) as usize).wrapping_add(bits);
        core::ptr::copy_nonoverlapping(source as *const u8, dest as *mut u8, row_bytes as usize);
        source = source.wrapping_add(row_bytes as usize);
        row += 1;
    }
}

// ---- NiDX8TextureManager

/// 0x6BC8A0 `NiDX8TextureManager::clearHashMap`: removes every node of a hash map.
///
/// The body does not call [`hash_map::drain_and_zero_count_with`]. The game function reads
/// `arrayBuckets` one time before the first bucket (0x6BC8AD), then after each `_delete`.
/// [`hash_map::drain_with`] reads `arrayBuckets` again for each bucket.
///
/// # Safety
///
/// `map` points at a live hash map of the texture manager whose nodes come from the engine's
/// `operator new`; nothing uses its nodes after the call.
pub unsafe fn clear_hash_map(map: usize) {
    crate::shadow::touch(&clear_hash_map_port::STATE);
    let mut i = 0u32;
    if rd::<u32>(map, HM::bucketCount) != 0 {
        let mut buckets: usize = rd(map, HM::arrayBuckets);
        loop {
            let mut node: usize = rd(buckets, i as usize * 4);
            while node != 0 {
                wr::<usize>(buckets, i as usize * 4, rd(node, HN::nextNode));
                hash_map::dtor_key_value_pair(map, node);
                eng::operator_delete(node);
                buckets = rd(map, HM::arrayBuckets);
                node = rd(buckets, i as usize * 4);
            }
            i += 1;
            if i >= rd::<u32>(map, HM::bucketCount) {
                break;
            }
        }
    }
    wr::<u32>(map, HM::count, 0);
}

/// 0x6BC840 `NiDX8TextureManager::HashMap::dtor`
///
/// # Safety
///
/// `map` points at a live hash map of the texture manager that nothing uses after the call.
pub unsafe fn hash_map_dtor(map: usize) {
    crate::shadow::touch(&hash_map_dtor_port::STATE);
    wr(map, HM::vtbl, VTBL_HASH_MAP);
    clear_hash_map(map);
    eng::operator_delete(rd(map, HM::arrayBuckets));
}

/// 0x6BC8F0 `NiDX8TextureManager::deleting_dtor_6BC8F0`: the deleting destructor of the hash map.
///
/// # Safety
///
/// `map` points at a live hash map of the texture manager that nothing uses after the call; with
/// bit 0 of `flags` it is a block of the engine's allocation, which the function frees.
pub unsafe fn hash_map_deleting_dtor(map: usize, flags: u32) -> usize {
    crate::shadow::touch(&hash_map_deleting_dtor_port::STATE);
    hash_map_dtor(map);
    if flags & 1 != 0 {
        eng::operator_delete(map);
    }
    map
}

/// 0x6BC100 `NiDX8TextureManager::ctor`. Returns `this`.
///
/// # Safety
///
/// `this` points at a block of the size of `NiDX8TextureManager` that is not constructed yet, and
/// `renderer` at the live `NiDX8Renderer`.
pub unsafe fn manager_ctor(this: usize, renderer: usize) -> usize {
    crate::shadow::touch(&manager_ctor_port::STATE);
    wr(this, TM::vtbl, VTBL_NI_REF_OBJECT);
    wr::<u32>(this, TM::refCount, 0);
    eng::ref_object_count_add(1);
    for map in [this + TM::hashMap_10, this + TM::hashMap_20, this + TM::hashmap_30] {
        wr(map, HM::vtbl, VTBL_HASH_MAP);
        wr::<u32>(map, HM::bucketCount, 37);
        wr::<u32>(map, HM::count, 0);
        let buckets = eng::operator_new(0x94);
        let bytes = rd::<u32>(map, HM::bucketCount).wrapping_shl(2);
        wr(map, HM::arrayBuckets, buckets);
        core::ptr::write_bytes(buckets as *mut u8, 0, bytes as usize);
    }
    wr(this, TM::vtbl, VTBL_TEXTURE_MANAGER);
    wr(this, TM::renderer, renderer);
    let device: usize = rd(renderer, R::d3dDevice);
    wr(this, TM::deviceInterface, device);
    com_add_ref(device);
    // The game code has the body of `clearHashMap` inline three times.
    clear_hash_map(this + TM::hashMap_10);
    clear_hash_map(this + TM::hashMap_20);
    clear_hash_map(this + TM::hashmap_30);
    this
}

/// 0x6BC330 `NiDX8TextureManager::dtor`
///
/// # Safety
///
/// `this` points at a live `NiDX8TextureManager` that nothing uses after the call: the function
/// releases the device resources of its maps.
pub unsafe fn manager_dtor(this: usize) {
    crate::shadow::touch(&manager_dtor_port::STATE);
    wr(this, TM::vtbl, VTBL_TEXTURE_MANAGER);
    let maps = [this + TM::hashMap_10, this + TM::hashMap_20, this + TM::hashmap_30];
    for map in maps {
        map_destroy_values(map, eng::dx8_resource_release);
    }
    for map in maps {
        clear_hash_map(map);
    }
    com_release(rd(this, TM::deviceInterface));
    wr::<u32>(this, TM::deviceInterface, 0);
    for map in [maps[2], maps[1], maps[0]] {
        wr(map, HM::vtbl, VTBL_HASH_MAP);
        clear_hash_map(map);
        eng::operator_delete(rd(map, HM::arrayBuckets));
    }
    wr(this, TM::vtbl, VTBL_NI_REF_OBJECT);
    eng::ref_object_count_add(-1);
}

/// 0x6BC310 `NiDX8TextureManager::deleting_dtor`
///
/// # Safety
///
/// `this` points at a live `NiDX8TextureManager` that nothing uses after the call; with bit 0 of
/// `flags` it is a block of the engine's allocation, which the function frees.
pub unsafe fn manager_deleting_dtor(this: usize, flags: u32) -> usize {
    crate::shadow::touch(&manager_deleting_dtor_port::STATE);
    manager_dtor(this);
    if flags & 1 != 0 {
        eng::operator_delete(this);
    }
    this
}

/// 0x6BC590 `NiDX8TextureManager::GetTexture`. The game does not implement it.
///
/// # Safety
///
/// No condition: the function returns 0. It is `unsafe` as every port function is.
pub unsafe fn manager_get_texture(_this: usize, _a1: u32, _a2: u32, _a3: u32, _a4: u32, _a5: u32, _a6: u32) -> u32 {
    crate::shadow::touch(&manager_get_texture_port::STATE);
    0
}

/// 0x6BC5A0 `NiDX8TextureManager::PrepareTextureForRendering`: returns the device texture of a
/// texture. It makes or updates the device texture when necessary.
///
/// # Safety
///
/// `this` points at a live `NiDX8TextureManager` with its renderer; `texture` is 0 or a live
/// `NiTexture`; `out_created` and `out_mipmapped` point at bytes that the function writes.
pub unsafe fn manager_prepare_texture_for_rendering(this: usize, texture: usize, out_created: usize, out_mipmapped: usize) -> usize {
    crate::shadow::touch(&manager_prepare_texture_for_rendering_port::STATE);
    wr::<u8>(out_created, 0, 0);
    wr::<u8>(out_mipmapped, 0, 0);
    if texture == 0 {
        return 0;
    }
    let mut data: usize = rd(texture, T::rendererData);
    if data == 0 {
        if !ni::is_kind_of(texture, RTTI_SOURCE_TEXTURE) {
            return 0;
        }
        data = eng::source_texture_data_create(texture, rd(this, TM::renderer));
        wr::<u8>(out_created, 0, 1);
    }
    let rendered = data_get_as_rendered(data);
    if rendered != 0 {
        return data_get_d3d_texture(rendered);
    }
    if data_get_d3d_texture(data) == 0 {
        data_update(data);
        wr::<u8>(out_created, 0, 1);
    } else if rd::<u8>(texture, ST::isStatic) == 0 {
        match source_data_size_change(texture, data) {
            SizeChange::None => data_update(data),
            SizeChange::Remake => {
                // A new renderer data at the new size, as at the first use of the texture.
                wr::<usize>(texture, T::rendererData, rd(data, TD::vtbl_linkedData));
                ni::delete_object(data);
                eng::source_texture_data_create(texture, rd(this, TM::renderer));
                data = rd(texture, T::rendererData);
                if data == 0 {
                    return 0;
                }
                wr::<u8>(out_created, 0, 1);
            }
            SizeChange::Unusable => {}
        }
    }
    wr::<u8>(out_mipmapped, 0, (data_get_levels(data) > 1) as u8);
    data_get_d3d_texture(data)
}

/// What a new revision of the pixel data of a texture that is not static asks of its source
/// renderer data ([`source_data_size_change`]).
#[derive(Debug, PartialEq)]
enum SizeChange {
    /// No new revision, or the same size: `Update` copies the pixels.
    None,
    /// Another size: the renderer data is made again (`Create` makes the device texture from
    /// the size of the pixel data).
    Remake,
    /// Another size that `Create` would refuse (not a power of two): the old picture stays,
    /// and the revision counts as seen.
    Unusable,
}

/// Unlike the engine, which copies pixel data of any size into the device texture of the old
/// size (`NiDX8SourceTextureData::Update` 0x6C0050; past the end of its memory when the new
/// data is larger): a new revision whose first level has another width or height than the
/// renderer data (`width` and `height`, which `Create` 0x6BFA50 takes from the pixel data)
/// asks for a new renderer data. A mod can give a texture that is not static new pixel data
/// of another size (MWSE's `pixelData` is writable).
unsafe fn source_data_size_change(texture: usize, data: usize) -> SizeChange {
    if rd::<u32>(data, TD::vtbl_vtbl) != VTBL_SOURCE_TEXTURE_DATA {
        return SizeChange::None;
    }
    let pixel_data: usize = rd(texture, ST::pixelData);
    if pixel_data == 0 || rd::<u32>(pixel_data, PD::revisionID) == rd::<u32>(data, SD::sourceRevisionID) {
        return SizeChange::None;
    }
    let width: u32 = rd(rd::<usize>(pixel_data, PD::mipmapWidths), 0);
    let height: u32 = rd(rd::<usize>(pixel_data, PD::mipmapHeights), 0);
    if width == rd::<u32>(data, SD::width) && height == rd::<u32>(data, SD::height) {
        return SizeChange::None;
    }
    if width.is_power_of_two() && height.is_power_of_two() {
        return SizeChange::Remake;
    }
    static REFUSED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    if REFUSED.fetch_add(1, core::sync::atomic::Ordering::Relaxed) < 8 {
        crate::log!("textures: new pixel data of {width} x {height} for a texture of {} x {}: not a power of two, the texture keeps its picture", rd::<u32>(data, SD::width), rd::<u32>(data, SD::height));
    }
    wr::<u32>(data, SD::sourceRevisionID, rd(pixel_data, PD::revisionID));
    SizeChange::Unusable
}

/// 0x6BC670 `NiDX8TextureManager::PrecacheTexture`
///
/// # Safety
///
/// `this` points at a live `NiDX8TextureManager` with its renderer, and `texture` is 0 or a live
/// `NiTexture`.
pub unsafe fn manager_precache_texture(this: usize, texture: usize, _a3: u32, _a4: u32) -> u8 {
    crate::shadow::touch(&manager_precache_texture_port::STATE);
    if texture == 0 {
        return 0;
    }
    let mut data: usize = rd(texture, T::rendererData);
    if data == 0 {
        if !ni::is_kind_of(texture, RTTI_SOURCE_TEXTURE) {
            return 0;
        }
        data = eng::source_texture_data_create(texture, rd(this, TM::renderer));
    }
    let rendered = data_get_as_rendered(data);
    if rendered == 0 {
        return 1;
    }
    (data_get_d3d_texture(rendered) != 0) as u8
}

/// 0x6BC6F0 `NiDX8TextureManager::GetFormatFromTexture`: the extra data of the pixel format
/// of the texture, or 14.
///
/// # Safety
///
/// `this` points at a live `NiDX8TextureManager` with its renderer, and `texture` at a live
/// `NiTexture`: the function reads it before any test, so it is not 0.
pub unsafe fn manager_get_format_from_texture(this: usize, texture: usize) -> u32 {
    crate::shadow::touch(&manager_get_format_from_texture_port::STATE);
    let data: usize = rd(texture, T::rendererData);
    if data != 0 {
        return rd(data_get_pixel_format(data), PF::extraData);
    }
    if ni::is_kind_of(texture, RTTI_SOURCE_TEXTURE) && renderer_create_source_data(rd(this, TM::renderer), texture) != 0 {
        let data: usize = rd(texture, T::rendererData);
        if data != 0 {
            return rd(data_get_pixel_format(data), PF::extraData);
        }
    }
    14
}

/// 0x6BC770 `NiDX8TextureManager::Recreate`: the first map loses its values and its nodes.
///
/// # Safety
///
/// `this` points at a live `NiDX8TextureManager`; nothing uses the device resources of its first
/// map after the call.
pub unsafe fn manager_recreate(t: &MainThread, this: usize) {
    crate::shadow::touch(&manager_recreate_port::STATE);
    crate::instance_stream::textures_changed(t);
    let map = this + TM::hashMap_10;
    map_destroy_values(map, eng::dx8_resource_release_thunk);
    clear_hash_map(map);
}

// ---- NiDX8TextureData

/// 0x6B6D00 `NiDX8TextureData::ctor_args`. Returns `this`.
///
/// # Safety
///
/// `this` points at a block of the size of `NiDX8TextureData` that is not constructed yet;
/// `texture` at a live `NiTexture`, and `renderer` at the live `NiDX8Renderer`.
pub unsafe fn texture_data_ctor(this: usize, texture: usize, renderer: usize) -> usize {
    crate::shadow::touch(&texture_data_ctor_port::STATE);
    wr(this, TD::vtbl_vtbl, VTBL_RENDERER_DATA);
    wr(this, TD::vtbl_sgTexture, texture);
    wr::<u32>(this, TD::vtbl_linkedData, 0);
    wr::<u32>(this, TD::format_bitsPerPixel, 8);
    wr::<[u32; 4]>(this, TD::format_channelMasks, [0; 4]);
    wr::<u32>(this, TD::format_format, 2);
    let [shift_a, mask_a, shift_b, mask_b] = eng::format_bit_globals();
    let bits = (2u32.wrapping_shl(shift_a) & mask_a) | (1u32.wrapping_shl(shift_b) & mask_b);
    wr::<u32>(this, TD::format_compareBits + 4, 0);
    wr::<u32>(this, TD::format_extraData, 0);
    wr::<u32>(this, TD::format_d3dFormat, 0);
    wr::<u32>(this, TD::format_compareBits, bits);
    wr(this, TD::vtbl_vtbl, VTBL_TEXTURE_DATA);
    wr(this, TD::renderer, renderer);
    wr::<u32>(this, TD::width, 0);
    wr::<u32>(this, TD::height, 0);
    wr::<u32>(this, TD::levels, 0);
    wr::<u32>(this, TD::d3dPalette, 0);
    wr::<u32>(this, TD::d3dPalRevisionId, 0);
    this
}

/// 0x6B6DC0 `NiDX8TextureData::deleting_dtor`
///
/// # Safety
///
/// `this` points at a live `NiDX8TextureData` that nothing uses after the call; with bit 0 of
/// `flags` it is a block of the engine's allocation, which the function frees.
pub unsafe fn texture_data_deleting_dtor(this: usize, flags: u32) -> usize {
    crate::shadow::touch(&texture_data_deleting_dtor_port::STATE);
    eng::texture_renderer_data_dtor(this);
    if flags & 1 != 0 {
        eng::operator_delete(this);
    }
    this
}

/// 0x6B6E10 `NiDX8TextureData::GetHeight`
///
/// # Safety
///
/// `this` points at a live `NiDX8TextureData`.
pub unsafe fn texture_data_get_height(this: usize) -> u32 {
    crate::shadow::touch(&texture_data_get_height_port::STATE);
    rd(this, TD::height)
}

/// 0x6B6E30 `NiDX8TextureData::GetPixelFormat`
///
/// # Safety
///
/// `this` points at a live `NiDX8TextureData`.
pub unsafe fn texture_data_get_pixel_format(this: usize) -> usize {
    crate::shadow::touch(&texture_data_get_pixel_format_port::STATE);
    this + TD::format
}

/// 0x6B6E40 `NiDX8TextureData::GetD3DPalette`
///
/// # Safety
///
/// `this` points at a live `NiDX8TextureData`.
pub unsafe fn texture_data_get_d3d_palette(this: usize) -> u32 {
    crate::shadow::touch(&texture_data_get_d3d_palette_port::STATE);
    rd(this, TD::d3dPalette)
}

/// 0x6B6E50 `NiDX8TextureData_static::FindClosestPixelFormat`: the format of the list that
/// best fits the preferences. It has no pixel data to examine. Returns 0 when no format fits.
///
/// # Safety
///
/// `prefs` points at the live format preferences of a texture, and `formats` at the renderer's
/// array of texture formats (each 0 or a live `NiDX8PixelFormat`), as long as the function indexes
/// it.
pub unsafe fn find_closest_pixel_format(_renderer: usize, prefs: usize, formats: usize) -> usize {
    crate::shadow::touch(&find_closest_pixel_format_port::STATE);
    let mut layout: u32 = rd(prefs, PREFS_LAYOUT);
    let alpha: i32 = rd(prefs, PREFS_ALPHA);
    let f = |i: usize| rd::<usize>(formats, i * 4);
    if layout == 5 {
        layout = 2;
    } else if layout == 0 {
        if alpha == 3 || alpha == 0 {
            if f(6) != 0 {
                return f(6);
            }
        } else if (alpha == 1 || alpha == 2) && f(7) != 0 {
            return f(7);
        }
    } else if layout > 3 {
        return 0;
    }
    // The first non-zero format of a list, or 0.
    let first = |list: &[usize]| list.iter().map(|&i| f(i)).find(|&p| p != 0).unwrap_or(0);

    if layout == 0 || layout == 3 {
        if (0..=1).contains(&alpha) {
            if f(8) != 0 {
                return f(8);
            }
        } else if alpha == 2 && f(9) != 0 {
            return f(9);
        }
    }
    if layout == 2 {
        return match alpha {
            0 => first(&[2, 5, 1, 0, 3, 4]),
            1 => first(&[5, 3, 4]),
            2 => first(&[5, 4]),
            _ => 0,
        };
    }
    // Layouts 0 and 3 come here when no format above fitted. Layout 1 starts here.
    match alpha {
        0 => first(&[1, 0, 2, 5, 3, 4]),
        1 => first(&[3, 4, 5]),
        2 => first(&[4, 5]),
        _ => 0,
    }
}

/// 0x6B6F70 `NiDX8TextureData::FindMatchingPixelFormat`: the format of the list that best fits
/// the preferences. The image converter must be able to make it from the source format.
///
/// # Safety
///
/// `source` points at a live pixel format, `prefs` at the live format preferences of a texture, and
/// `formats` at the renderer's array of texture formats (each 0 or live), as long as the function
/// indexes it; the engine's image converter is set.
pub unsafe fn find_matching_pixel_format(_this: usize, source: usize, prefs: usize, formats: usize) -> usize {
    crate::shadow::touch(&find_matching_pixel_format_port::STATE);
    let format: u32 = rd(source, 0);
    let converter = eng::get_image_converter();
    let layout: u32 = rd(prefs, PREFS_LAYOUT);
    let f = |i: usize| rd::<usize>(formats, i * 4);
    let can = |dest: usize| dest != 0 && converter_can_convert(converter, source, dest);
    let last = |dest: usize| if can(dest) { dest } else { 0 };
    macro_rules! attempt {
        ($($dest:expr),+) => {
            $(
                let dest: usize = $dest;
                if can(dest) {
                    return dest;
                }
            )+
        };
    }

    if layout == 4 {
        if format == 11 || format == 1 {
            attempt!(f(13), f(12));
            return last(f(11));
        }
        attempt!(f(11), f(13));
        return last(f(12));
    }
    match format {
        0 | 2 | 4 => {
            if layout == 3 {
                attempt!(f(8), f(9), f(10));
            }
            if rd::<u32>(prefs, PREFS_LAYOUT) == 1 {
                let patched = eng::mcp039_offsets();
                attempt!(rd(formats, patched[0]), f(0), rd(formats, patched[1]), f(5));
            } else {
                attempt!(f(2), f(5), f(1), f(0));
            }
            attempt!(f(3));
            return last(f(4));
        }
        1 | 3 | 5 => {}
        6 => {
            if layout != 1 && layout != 2 {
                attempt!(f(8));
            }
        }
        7 => {
            if layout != 1 && layout != 2 {
                attempt!(f(9));
            }
        }
        8 => {
            if layout != 1 && layout != 2 {
                attempt!(f(10));
            }
        }
        10 => {
            attempt!(f(11), f(13));
            return last(f(12));
        }
        11 => {
            attempt!(f(12), f(13));
            return last(f(11));
        }
        _ => return 0,
    }

    let alpha_one = || rd::<u32>(prefs, PREFS_ALPHA) == 1;
    if rd::<u32>(prefs, PREFS_LAYOUT) == 3 {
        if alpha_one() {
            attempt!(f(8), f(9), f(10));
        } else {
            attempt!(f(9), f(10), f(8));
        }
    }
    if rd::<u32>(prefs, PREFS_LAYOUT) == 1 {
        if alpha_one() {
            attempt!(f(3), f(4));
            return last(f(5));
        }
        let patched = eng::mcp039_offsets();
        attempt!(rd(formats, patched[2]), rd(formats, patched[3]));
        return last(f(3));
    }
    attempt!(f(5));
    if alpha_one() {
        attempt!(f(3));
        return last(f(4));
    }
    attempt!(f(4));
    last(f(3))
}

/// 0x6B74F0 `NiDX8TextureData_static::ClearTextureData`: every texture loses its renderer data.
///
/// # Safety
///
/// The engine's list of live textures holds live textures; nothing uses the renderer data that the
/// function deletes after the call.
pub unsafe fn clear_texture_data(_renderer: usize) {
    crate::shadow::touch(&clear_texture_data_port::STATE);
    let mut texture = eng::texture_list_head();
    while texture != 0 {
        let data: usize = rd(texture, T::rendererData);
        if data != 0 {
            wr::<usize>(texture, T::rendererData, rd(data, TD::vtbl_linkedData));
            ni::delete_object(data);
        }
        texture = rd(texture, T::nextTexture);
    }
}

/// The textures that are alive and whose first renderer data has this method table.
///
/// # Safety
///
/// The engine's list of live textures holds live textures.
pub unsafe fn textures_with_data(table: u32) -> Vec<usize> {
    let mut list = Vec::new();
    let mut texture = eng::texture_list_head();
    while texture != 0 {
        let data: usize = rd(texture, T::rendererData);
        if data != 0 && rd::<u32>(data, TD::vtbl_vtbl) == table {
            list.push(texture);
        }
        texture = rd(texture, T::nextTexture);
    }
    list
}

/// The renderer data of the source textures whose file could not be read: they hold the
/// default texture. A purge keeps them. A new read opens the engine's error menu again, and
/// the frames of that menu inside a display leave the accumulator of the display without
/// its camera. The cell worker also makes and deletes renderer data, so the list has a lock.
static UNREADABLE: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

/// Marks renderer data that holds the default texture because the file of its texture could
/// not be read.
pub fn mark_unreadable(data: usize) {
    if let Ok(mut list) = UNREADABLE.lock() {
        list.push(data);
    }
}

/// True when `data` has the mark of `mark_unreadable`.
pub fn is_unreadable(data: usize) -> bool {
    UNREADABLE.lock().is_ok_and(|list| list.contains(&data))
}

/// Removes the mark of renderer data that is deleted.
fn forget_unreadable(data: usize) {
    if let Ok(mut list) = UNREADABLE.lock() {
        if let Some(at) = list.iter().position(|&d| d == data) {
            list.swap_remove(at);
        }
    }
}

/// Makes each source texture lose its renderer data: the game makes it again at the next use
/// of the texture. A rendered texture keeps its data: the game does not make that again by
/// itself. This is not a function of the game; the capture uses it.
///
/// A source texture that the game cannot make again keeps its data.
/// `NiDX8SourceTextureData_static::Create` (0x6BFA50) reads the file of the texture when it
/// has a file name. If not, it takes the pixel data of the texture. With neither, it takes
/// the default texture of the renderer. A static texture made from pixel data has neither
/// after its first use. Without its renderer data it stays the default texture. A texture
/// whose file could not be read also keeps its data (`is_unreadable`).
///
/// Returns the number of textures that lost their data, the number that kept it because
/// the game cannot make it again, and the number that kept it because its file could not
/// be read.
///
/// # Safety
///
/// The engine's list of live textures holds live textures; nothing uses the renderer data that the
/// function deletes during the call (the game makes it again at the next use of the texture).
pub unsafe fn purge_source_textures() -> (u32, u32, u32) {
    let (mut dropped, mut kept, mut unreadable) = (0, 0, 0);
    let mut texture = eng::texture_list_head();
    while texture != 0 {
        let data: usize = rd(texture, T::rendererData);
        if data != 0 && rd::<u32>(data, TD::vtbl_vtbl) == VTBL_SOURCE_TEXTURE_DATA {
            if rd::<usize>(texture, ST::filenameOnPC) == 0 && rd::<usize>(texture, ST::pixelData) == 0 {
                kept += 1;
            } else if is_unreadable(data) {
                unreadable += 1;
            } else {
                wr::<usize>(texture, T::rendererData, rd(data, TD::vtbl_linkedData));
                ni::delete_object(data);
                dropped += 1;
            }
        }
        texture = rd(texture, T::nextTexture);
    }
    (dropped, kept, unreadable)
}

// ---- NiDX8SourceTextureData

/// 0x6BFDE0 `NiDX8SourceTextureData::ctor`. Returns `this`.
///
/// # Safety
///
/// `this` points at a block of the size of `NiDX8SourceTextureData` that is not constructed yet;
/// `texture` at a live `NiSourceTexture`, and `renderer` at the live `NiDX8Renderer`.
pub unsafe fn source_data_ctor(this: usize, texture: usize, renderer: usize) -> usize {
    crate::shadow::touch(&source_data_ctor_port::STATE);
    texture_data_ctor(this, texture, renderer);
    wr::<u32>(this, SD::palette, 0);
    wr(this, SD::vtbl_vtbl, VTBL_SOURCE_TEXTURE_DATA);
    wr::<u32>(this, SD::d3dTexture, 0);
    wr::<u32>(this, SD::sourceRevisionID, 0);
    wr::<u32>(this, SD::palRevisionID, 0);
    wr::<u8>(this, SD::bMipMap, 0);
    wr::<u32>(this, SD::formattedSize, 0);
    let palette: usize = rd(this, SD::palette);
    if palette != 0 {
        ni::release(palette);
        wr::<u32>(this, SD::palette, 0);
    }
    wr::<u32>(this, SD::width, 0);
    wr::<u32>(this, SD::height, 0);
    this
}

/// 0x6BFE90 `NiDX8SourceTextureData::dtor`
///
/// # Safety
///
/// `this` points at a live `NiDX8SourceTextureData` that nothing uses after the call.
pub unsafe fn source_data_dtor(t: &MainThread, this: usize) {
    crate::shadow::touch(&source_data_dtor_port::STATE);
    crate::instance_stream::textures_changed(t);
    forget_unreadable(this);
    wr(this, SD::vtbl_vtbl, VTBL_SOURCE_TEXTURE_DATA);
    let texture: usize = rd(this, SD::d3dTexture);
    if texture != 0 {
        if com_release(texture) == 0 {
            eng::texture_freed(rd(this, SD::d3dTexture), rd(this, SD::formattedSize));
        }
        wr::<u32>(this, SD::d3dTexture, 0);
    }
    let palette: usize = rd(this, SD::palette);
    if palette != 0 {
        ni::release(palette);
    }
    eng::texture_renderer_data_dtor(this);
}

/// 0x6BFE70 `NiDX8SourceTextureData::deleting_dtor`
///
/// # Safety
///
/// `this` points at a live `NiDX8SourceTextureData` that nothing uses after the call; with bit 0 of
/// `flags` it is a block of the engine's allocation, which the function frees.
pub unsafe fn source_data_deleting_dtor(t: &MainThread, this: usize, flags: u32) -> usize {
    crate::shadow::touch(&source_data_deleting_dtor_port::STATE);
    source_data_dtor(t, this);
    if flags & 1 != 0 {
        eng::operator_delete(this);
    }
    this
}

/// 0x6BFF20 `NiDX8SourceTextureData::GetD3DTexture`
///
/// # Safety
///
/// `this` points at a live `NiDX8SourceTextureData`.
pub unsafe fn source_data_get_d3d_texture(this: usize) -> usize {
    crate::shadow::touch(&source_data_get_d3d_texture_port::STATE);
    rd(this, SD::d3dTexture)
}

/// 0x6BFF30 `NiDX8SourceTextureData::IsLoaded`
///
/// # Safety
///
/// `this` points at a live `NiDX8SourceTextureData`.
pub unsafe fn source_data_is_loaded(this: usize) -> u8 {
    crate::shadow::touch(&source_data_is_loaded_port::STATE);
    (rd::<usize>(this, SD::d3dTexture) != 0) as u8
}

/// 0x6BFF40 `NiDX8SourceTextureData::CopyDataToSurface`: copies one mipmap level of the pixel
/// data to a surface.
///
/// # Safety
///
/// `pixel_data` points at a live `NiPixelData` with at least `level + 1` levels, and `surface` at a
/// live surface of the device.
pub unsafe fn source_data_copy_data_to_surface(pixel_data: usize, level: u32, surface: usize) {
    crate::shadow::touch(&source_data_copy_data_to_surface_port::STATE);
    let mut desc = [0u32; 8];
    let result = surface_get_desc(surface, desc.as_mut_ptr() as usize);
    if result != 0 {
        eng::debug_print1(TEXT_COPY_DESC, eng::directx_error_name(result));
        return;
    }
    let mut locked = [0u32; 2];
    let result = surface_lock_rect(surface, locked.as_mut_ptr() as usize, 0, 0);
    if result != 0 {
        eng::debug_print1(TEXT_COPY_LOCK, eng::directx_error_name(result));
        return;
    }
    let level = level as usize;
    let row_bytes = rd::<u32>(rd::<usize>(pixel_data, PD::mipmapWidths), level * 4).wrapping_mul(rd(pixel_data, PD::bytesPerPixel));
    let format: i32 = rd(pixel_data, PD::format_format);
    let offsets: usize = rd(pixel_data, PD::mipmapOffsets);
    let pixels: usize = rd(pixel_data, PD::pixelsAllMips);
    if locked[LOCK_PITCH] == row_bytes || (6..=8).contains(&format) {
        // The rows have the pitch of the surface, or the data is compressed. The port copies one
        // block.
        let start: u32 = rd(offsets, level * 4);
        let size = rd::<u32>(offsets, level * 4 + 4).wrapping_sub(start);
        core::ptr::copy_nonoverlapping(pixels.wrapping_add(start as usize) as *const u8, locked[LOCK_BITS] as *mut u8, size as usize);
    } else {
        let source = (rd::<u32>(offsets, level * 4) as usize).wrapping_add(pixels);
        copy_rows(source, locked[LOCK_BITS] as usize, locked[LOCK_PITCH], row_bytes, desc[DESC_HEIGHT]);
    }
    surface_unlock_rect(surface);
}

/// 0x6C0050 `NiDX8SourceTextureData::Update`: when the pixel data or its palette changed, it
/// copies every level to the device texture again.
///
/// # Safety
///
/// `this` points at a live `NiDX8SourceTextureData` whose texture is a live `NiSourceTexture`.
pub unsafe fn source_data_update(this: usize) {
    crate::shadow::touch(&source_data_update_port::STATE);
    let pixel_data: usize = rd(rd::<usize>(this, SD::vtbl_sgTexture), ST::pixelData);
    if pixel_data == 0 {
        return;
    }
    let palette: usize = rd(pixel_data, PD::palette);
    let mut palette_changed = false;
    if palette != 0 {
        let revision: u32 = rd(palette, PALETTE_REVISION);
        if rd::<u32>(this, SD::palRevisionID) != revision {
            palette_changed = true;
        }
        wr(this, SD::palRevisionID, revision);
        let current: usize = rd(this, SD::palette);
        if current != palette {
            if current != 0 {
                eng::ni_object_release(current);
            }
            wr(this, SD::palette, palette);
            wr(palette, REF_COUNT, rd::<u32>(palette, REF_COUNT).wrapping_add(1));
            palette_changed = true;
        }
    }
    if rd::<u32>(pixel_data, PD::revisionID) == rd::<u32>(this, SD::sourceRevisionID) && !palette_changed {
        return;
    }
    let converter = eng::get_image_converter();
    let converted = converter_convert(converter, pixel_data, this + SD::format, pixel_data, rd(this, SD::bMipMap));
    // Unlike the engine: the pixels go into the device texture only when they have its size
    // (`Create` made it from `width` and `height`). Pixel data of another size keeps its
    // revision unseen: the next `PrepareTextureForRendering` makes the renderer data again
    // (`source_data_size_change`). The engine copies them into the old texture, past the
    // end of its levels.
    let texture: usize = rd(this, SD::d3dTexture);
    let fits = converted != 0
        && rd::<u32>(rd::<usize>(converted, PD::mipmapWidths), 0) == rd::<u32>(this, SD::width)
        && rd::<u32>(rd::<usize>(converted, PD::mipmapHeights), 0) == rd::<u32>(this, SD::height);
    if fits {
        wr::<u32>(this, SD::sourceRevisionID, rd(pixel_data, PD::revisionID));
    }
    if fits && texture != 0 && rd::<usize>(this, SD::vtbl_sgTexture) != 0 {
        // At most the levels of the device texture (`levels`, from `Create`).
        let levels = rd::<u32>(converted, PD::mipmapLevels).min(rd::<u32>(this, SD::levels));
        let mut level = 0u32;
        while level < levels {
            let mut surface = 0usize;
            let result = texture_get_surface_level(texture, level, core::ptr::addr_of_mut!(surface) as usize);
            if result != 0 {
                // Unlike the engine, which releases the device texture here and keeps its
                // address (the destructor releases it again): the texture stays.
                eng::debug_print2(TEXT_UPDATE_LEVEL, level as usize, eng::directx_error_name(result));
                break;
            }
            source_data_copy_data_to_surface(converted, level, surface);
            com_release(surface);
            level += 1;
        }
    }
    if converted != 0 {
        // A smart pointer takes the data and then releases it.
        let count = rd::<u32>(converted, REF_COUNT).wrapping_add(1);
        wr(converted, REF_COUNT, count);
        wr(converted, REF_COUNT, count.wrapping_sub(1));
        if count.wrapping_sub(1) == 0 {
            ni::delete_object(converted);
        }
    }
}

/// 0x6C0160 `NiDX8SourceTextureData::setPalette`. Returns 1 when the palette or its
/// revision is new.
///
/// # Safety
///
/// `this` points at a live `NiDX8SourceTextureData`, and `palette` is 0 or a live `NiPalette`.
pub unsafe fn source_data_set_palette(this: usize, palette: usize) -> u8 {
    crate::shadow::touch(&source_data_set_palette_port::STATE);
    let mut changed = 0u8;
    if palette != 0 {
        let revision: u32 = rd(palette, PALETTE_REVISION);
        if rd::<u32>(this, SD::palRevisionID) != revision {
            changed = 1;
        }
        wr(this, SD::palRevisionID, revision);
        let current: usize = rd(this, SD::palette);
        if current != palette {
            if current != 0 {
                ni::release(current);
            }
            wr(this, SD::palette, palette);
            wr(palette, REF_COUNT, rd::<u32>(palette, REF_COUNT).wrapping_add(1));
            changed = 1;
        }
    }
    changed
}

// ---- NiDX8BltSourceData

/// 0x6B68F0 `NiDX8BltSourceData::CreateSurface`: makes an image surface with the first level of
/// the pixel data. Returns 1 when it made the surface.
///
/// # Safety
///
/// `this` points at live blit source data whose renderer has its device; `pixel_data` at a live
/// `NiPixelData`, and `pixel_format` at a live `NiDX8PixelFormat`.
pub unsafe fn blt_data_create_surface(this: usize, pixel_data: usize, pixel_format: usize) -> u8 {
    crate::shadow::touch(&blt_data_create_surface_port::STATE);
    let slot = this + BD_SURFACE;
    let d3d_format: u32 = rd(pixel_format, PF::d3dFormat);
    let width: u32 = rd(rd::<usize>(pixel_data, PD::mipmapWidths), 0);
    let height: u32 = rd(rd::<usize>(pixel_data, PD::mipmapHeights), 0);
    let device: usize = rd(rd::<usize>(this, BD_RENDERER), R::d3dDevice);
    let result = device_create_image_surface(device, width, height, d3d_format, slot);
    if result != 0 {
        eng::debug_print1(TEXT_BLT_CREATE, eng::directx_error_name(result));
        return 0;
    }
    let mut desc = [0u32; 8];
    let surface: usize = rd(slot, 0);
    let result = surface_get_desc(surface, desc.as_mut_ptr() as usize);
    if result != 0 {
        eng::debug_print1(TEXT_BLT_DESC, eng::directx_error_name(result));
        com_release(rd(slot, 0));
        wr::<u32>(slot, 0, 0);
        return 0;
    }
    let mut locked = [0u32; 2];
    let surface: usize = rd(slot, 0);
    let result = surface_lock_rect(surface, locked.as_mut_ptr() as usize, 0, 0);
    if result != 0 {
        eng::debug_print1(TEXT_BLT_LOCK, eng::directx_error_name(result));
        com_release(rd(slot, 0));
        wr::<u32>(slot, 0, 0);
        return 0;
    }
    let row_bytes = rd::<u32>(pixel_data, PD::bytesPerPixel).wrapping_mul(rd(rd::<usize>(pixel_data, PD::mipmapWidths), 0));
    let source = (rd::<u32>(rd::<usize>(pixel_data, PD::mipmapOffsets), 0) as usize).wrapping_add(rd(pixel_data, PD::pixelsAllMips));
    copy_rows(source, locked[LOCK_BITS] as usize, locked[LOCK_PITCH], row_bytes, desc[DESC_HEIGHT]);
    surface_unlock_rect(rd(slot, 0));
    1
}

/// 0x6B6690 `NiDX8BltSourceData::Create`: makes the renderer data of a blit source and puts it
/// at the end of its list. Returns the data, or 0 when there is no pixel data.
///
/// # Safety
///
/// `blt_source` points at a live `NiBltSource`, and `renderer` at the live `NiDX8Renderer`; when
/// the source names a file, the engine's image converter is set.
pub unsafe fn blt_data_create(blt_source: usize, renderer: usize) -> usize {
    crate::shadow::touch(&blt_data_create_port::STATE);
    let filename: usize = rd(blt_source, BLT_FILENAME);
    let source: usize = if filename != 0 {
        let converter = eng::get_image_converter();
        converter_read_file(converter, filename, 0)
    } else {
        rd(blt_source, BLT_PIXEL_DATA)
    };
    let pixel_format: usize = rd(rd::<usize>(renderer, R::currentRenderTarget), RENDER_TARGET_PIXEL_FORMAT);
    if source == 0 {
        return 0;
    }
    let converter = eng::get_image_converter();
    let converted = converter_convert(converter, source, pixel_format, source, 0);
    if converted == 0 {
        return 0;
    }
    let data = eng::operator_new(BD_SIZE);
    if data != 0 {
        wr(data, BD_RENDERER, renderer);
        wr(data, BD_SOURCE, blt_source);
        wr::<u32>(data, BD_LINKED, 0);
        wr(data, 0, VTBL_BLT_SOURCE_DATA);
        wr::<u32>(data, BD_SURFACE, 0);
        wr::<u32>(data, BD_FIELD_18, 0);
        wr::<u32>(data, BD_WIDTH, 0);
        wr::<u32>(data, BD_HEIGHT, 0);
    }
    wr::<u32>(data, BD_WIDTH, rd(rd::<usize>(source, PD::mipmapWidths), 0));
    wr::<u32>(data, BD_HEIGHT, rd(rd::<usize>(source, PD::mipmapHeights), 0));
    if rd::<usize>(data, BD_SURFACE) == 0 {
        blt_data_create_surface(data, converted, pixel_format);
    }
    // A smart pointer holds the converted data, then the source.
    let count = rd::<u32>(converted, REF_COUNT).wrapping_add(1);
    wr(converted, REF_COUNT, count);
    let mut held = converted;
    if converted != source {
        wr(converted, REF_COUNT, count.wrapping_sub(1));
        if count.wrapping_sub(1) == 0 {
            ni::delete_object(converted);
        }
        wr(source, REF_COUNT, rd::<u32>(source, REF_COUNT).wrapping_add(1));
        held = source;
    }
    let mut slot = blt_source + BLT_RENDERER_DATA;
    let mut next: usize = rd(slot, 0);
    while next != 0 {
        slot = next + BD_LINKED;
        next = rd(slot, 0);
    }
    wr(slot, 0, data);
    ni::release(held);
    data
}

/// 0x6B6810 `NiDX8BltSourceData::dtor`
///
/// # Safety
///
/// `this` points at live blit source data that nothing uses after the call.
pub unsafe fn blt_data_dtor(this: usize) {
    crate::shadow::touch(&blt_data_dtor_port::STATE);
    wr(this, 0, VTBL_BLT_SOURCE_DATA);
    let surface: usize = rd(this, BD_SURFACE);
    if surface != 0 {
        com_release(surface);
        wr::<u32>(this, BD_SURFACE, 0);
    }
    let linked: usize = rd(this, BD_LINKED);
    wr(this, 0, VTBL_BLT_RENDERER_DATA);
    if linked != 0 {
        ni::delete_object(linked);
    }
}

/// 0x6B67F0 `NiDX8BltSourceData::deleting_dtor`
///
/// # Safety
///
/// `this` points at live blit source data that nothing uses after the call; with bit 0 of `flags`
/// it is a block of the engine's allocation, which the function frees.
pub unsafe fn blt_data_deleting_dtor(this: usize, flags: u32) -> usize {
    crate::shadow::touch(&blt_data_deleting_dtor_port::STATE);
    blt_data_dtor(this);
    if flags & 1 != 0 {
        eng::operator_delete(this);
    }
    this
}

// ---- NiDX8Renderer

/// 0x6B0380 `NiDX8Renderer::CreateSourceTextureRendererData`
///
/// # Safety
///
/// `this` points at the live `NiDX8Renderer`, and `texture` at a live `NiSourceTexture`.
pub unsafe fn renderer_create_source_texture_renderer_data(this: usize, texture: usize) -> u8 {
    crate::shadow::touch(&renderer_create_source_texture_renderer_data_port::STATE);
    if rd::<usize>(texture, T::rendererData) == 0 && eng::source_texture_data_create(texture, this) == 0 {
        return 0;
    }
    1
}

/// 0x6B0580 `NiDX8Renderer::CreateSourceCubeMapRendererData`
///
/// # Safety
///
/// `this` points at the live `NiDX8Renderer`, and `texture` at a live `NiSourceCubeMap`.
pub unsafe fn renderer_create_source_cube_map_renderer_data(this: usize, texture: usize) -> u8 {
    crate::shadow::touch(&renderer_create_source_cube_map_renderer_data_port::STATE);
    if rd::<usize>(texture, T::rendererData) != 0 {
        return 1;
    }
    (eng::source_cube_map_data_create(texture, this) != 0) as u8
}

/// 0x6B05B0 `NiDX8Renderer::CreateBltSourceRendererData`
///
/// # Safety
///
/// `this` points at the live `NiDX8Renderer`, and `blt_source` at a live `NiBltSource`.
pub unsafe fn renderer_create_blt_source_renderer_data(this: usize, blt_source: usize) -> u8 {
    crate::shadow::touch(&renderer_create_blt_source_renderer_data_port::STATE);
    if rd::<usize>(blt_source, BLT_RENDERER_DATA) == 0 && blt_data_create(blt_source, this) == 0 {
        return 0;
    }
    1
}

/// 0x6AC390 `NiDX8Renderer::PurgeAllTextures`
///
/// # Safety
///
/// As for `clear_texture_data`: the engine's list of live textures holds live textures, and nothing
/// uses their renderer data after the call.
pub unsafe fn renderer_purge_all_textures(this: usize, _a2: u32) -> u8 {
    crate::shadow::touch(&renderer_purge_all_textures_port::STATE);
    clear_texture_data(this);
    1
}

/// 0x6AC3A0 `NiDX8Renderer::PurgeTexture`: the texture loses the head of its renderer data.
///
/// # Safety
///
/// `texture` points at a live `NiTexture`; nothing uses its first renderer data after the call (the
/// function deletes it).
pub unsafe fn renderer_purge_texture(_this: usize, texture: usize) -> u8 {
    crate::shadow::touch(&renderer_purge_texture_port::STATE);
    let data: usize = rd(texture, T::rendererData);
    if data != 0 {
        wr::<usize>(texture, T::rendererData, rd(data, TD::vtbl_linkedData));
        ni::delete_object(data);
    }
    1
}

/// 0x6AC400 `NiDX8Renderer::GetTextureMemoryStats`. The game does not implement it.
///
/// # Safety
///
/// No condition: the function returns 0. It is `unsafe` as every port function is.
pub unsafe fn renderer_get_texture_memory_stats(_this: usize, _a1: u32, _a2: u32) -> u8 {
    crate::shadow::touch(&renderer_get_texture_memory_stats_port::STATE);
    0
}

/// 0x6AC410 `NiDX8Renderer::GetTextureStats_maybe`. The game does not implement it.
///
/// # Safety
///
/// No condition: the function returns 0. It is `unsafe` as every port function is.
pub unsafe fn renderer_get_texture_stats(_this: usize, _a1: u32, _a2: u32, _a3: u32, _a4: u32, _a5: u32) -> u8 {
    crate::shadow::touch(&renderer_get_texture_stats_port::STATE);
    0
}

/// 0x6ABD00 `NiDX8Renderer::FindClosestPixelFormat`
///
/// # Safety
///
/// `this` points at the live `NiDX8Renderer` with its texture formats, and `prefs` at the live
/// format preferences of a texture.
pub unsafe fn renderer_find_closest_pixel_format(this: usize, prefs: usize) -> usize {
    crate::shadow::touch(&renderer_find_closest_pixel_format_port::STATE);
    find_closest_pixel_format(this, prefs, this + R::arrayArrayTextureFormats)
}

// ---- the memory a call can write

/// Returns true when a hash map has a node.
unsafe fn map_has_nodes(map: usize) -> bool {
    hash_map::first(map) != 0
}

/// `clearHashMap`: the call is a skip when the map has nodes and the allocator hooks are not
/// active, because the call frees the nodes. The call writes the map and its bucket array.
unsafe fn clear_hash_map_regions(set: &mut Regions, map: usize) {
    if map_has_nodes(map) {
        set.allocates();
    }
    set.add(map, HM::SIZE);
    set.add(rd(map, HM::arrayBuckets), rd::<u32>(map, HM::bucketCount) as usize * 4);
}

/// The destructor of a hash map: the call is a skip when the map has nodes. The destructor
/// frees the bucket array, and the Rust run leaves an empty array to the engine run. The
/// shadow test cannot put the array back as a region after the free. A map that the call
/// frees gets no region.
unsafe fn hash_map_dtor_regions(set: &mut Regions, map: usize, freed: bool) {
    if map_has_nodes(map) {
        set.skip();
        return;
    }
    set.allocates();
    if !freed {
        set.add(map, HM::SIZE);
    }
}

/// `Recreate`: the call is a skip when the call destroys a value. Then the call releases a
/// device object and an engine destructor runs. If not, the call frees the nodes.
unsafe fn manager_recreate_regions(set: &mut Regions, this: usize) {
    let map = this + TM::hashMap_10;
    let buckets: usize = rd(map, HM::arrayBuckets);
    for i in 0..rd::<u32>(map, HM::bucketCount) as usize {
        let mut node: usize = rd(buckets, i * 4);
        while node != 0 {
            if rd::<usize>(node, HN::data) != 0 {
                set.skip();
                return;
            }
            node = rd(node, HN::nextNode);
        }
    }
    clear_hash_map_regions(set, map);
}

/// Returns true when `Update` passes its checks and does the conversion and the upload.
unsafe fn update_uploads(data: usize) -> bool {
    let texture: usize = rd(data, SD::vtbl_sgTexture);
    let pixel_data: usize = rd(texture, ST::pixelData);
    if pixel_data == 0 {
        return false;
    }
    let palette: usize = rd(pixel_data, PD::palette);
    if palette != 0 && (rd::<u32>(data, SD::palRevisionID) != rd::<u32>(palette, PALETTE_REVISION) || rd::<usize>(data, SD::palette) != palette) {
        return true;
    }
    rd::<u32>(pixel_data, PD::revisionID) != rd::<u32>(data, SD::sourceRevisionID)
}

/// `Update`: the call is a skip when the call uploads. Then the image converter runs, and the
/// call locks surfaces and can delete the converted data. If not, the call writes its
/// palette revision at most.
unsafe fn source_data_update_regions(set: &mut Regions, this: usize) {
    if update_uploads(this) {
        set.skip();
        return;
    }
    set.add(this, SD::SIZE);
}

/// `PrepareTextureForRendering`: the call is a skip when the call makes the renderer data.
/// That needs file access, a device texture and `LockRect`. The call is also a skip when the
/// data is of a class whose `Update` is not the one of this module. The call is also a skip
/// when the data has no device texture, and when `Update` uploads.
unsafe fn manager_prepare_regions(set: &mut Regions, texture: usize, out_created: usize, out_mipmapped: usize) {
    set.add(out_created, 1);
    set.add(out_mipmapped, 1);
    if texture == 0 {
        return;
    }
    let data: usize = rd(texture, T::rendererData);
    if data == 0 {
        set.skip();
        return;
    }
    if data_get_as_rendered(data) != 0 {
        return;
    }
    let loaded = data_get_d3d_texture(data) != 0;
    if loaded && rd::<u8>(texture, ST::isStatic) != 0 {
        return;
    }
    if rd::<usize>(rd::<usize>(data, 0), VT_DATA_UPDATE) != 0x6C0050 || !loaded || update_uploads(data) {
        set.skip();
        return;
    }
    set.add(data, SD::SIZE);
}

/// The call is a skip when the texture has no renderer data: the call makes the data.
unsafe fn skip_without_renderer_data(set: &mut Regions, texture: usize) {
    if texture != 0 && rd::<usize>(texture, T::rendererData) == 0 {
        set.skip();
    }
}

/// `ClearTextureData`: the call is a skip when a texture has renderer data. Then the virtual
/// deleting destructor of the renderer data runs.
unsafe fn clear_texture_data_regions(set: &mut Regions) {
    let mut texture = eng::texture_list_head();
    while texture != 0 {
        if rd::<usize>(texture, T::rendererData) != 0 {
            set.skip();
            return;
        }
        texture = rd(texture, T::nextTexture);
    }
}

/// The deleting destructor of the base class: the call is a skip when the object has linked
/// data. Then the virtual deleting destructor of the linked data runs. With flag 1 the call
/// frees the object, and the object gets no region.
unsafe fn texture_data_deleting_dtor_regions(set: &mut Regions, this: usize, flags: u32) {
    if rd::<usize>(this, TD::vtbl_linkedData) != 0 {
        set.skip();
        return;
    }
    if flags & 1 != 0 {
        set.allocates();
    } else {
        set.add(this, TD::SIZE);
    }
}

/// The destructor of source texture data: the call is a skip when the call releases the
/// device texture or deletes linked data. The call is also a skip when the palette loses its
/// last reference.
unsafe fn source_data_dtor_regions(set: &mut Regions, this: usize, freed: bool) {
    let palette: usize = rd(this, SD::palette);
    if rd::<usize>(this, SD::d3dTexture) != 0
        || rd::<usize>(this, SD::vtbl_linkedData) != 0
        || (palette != 0 && rd::<u32>(palette, REF_COUNT) == 1)
    {
        set.skip();
        return;
    }
    if freed {
        set.allocates();
    } else {
        set.add(this, SD::SIZE);
    }
    set.add(palette, 8);
}

/// `setPalette`: the call is a skip when the old palette loses its last reference. Then the
/// virtual deleting destructor of the old palette runs.
unsafe fn source_data_set_palette_regions(set: &mut Regions, this: usize, palette: usize) {
    set.add(this, SD::SIZE);
    if palette == 0 {
        return;
    }
    let current: usize = rd(this, SD::palette);
    if current != palette {
        if current != 0 && rd::<u32>(current, REF_COUNT) == 1 {
            set.skip();
            return;
        }
        set.add(current, 8);
        set.add(palette, 8);
    }
}

/// `NiDX8BltSourceData::Create`: the call is a skip when the source has a file name or pixel
/// data. Then the call reads a file or converts pixel data, makes an image surface and locks
/// it. Only the call that finds no pixel data can run twice. That call writes nothing.
unsafe fn blt_data_create_regions(set: &mut Regions, blt_source: usize) {
    if rd::<usize>(blt_source, BLT_FILENAME) != 0 || rd::<usize>(blt_source, BLT_PIXEL_DATA) != 0 {
        set.skip();
    }
}

/// The destructor of blit data: the call is a skip when the call releases the surface or
/// deletes linked data.
unsafe fn blt_data_dtor_regions(set: &mut Regions, this: usize, freed: bool) {
    if rd::<usize>(this, BD_SURFACE) != 0 || rd::<usize>(this, BD_LINKED) != 0 {
        set.skip();
        return;
    }
    if freed {
        set.allocates();
    } else {
        set.add(this, BD_SIZE);
    }
}

// ---- the ports

// The call is always a skip: it makes three bucket arrays and the device gets a reference.
port_thiscall!(manager_ctor_port, 0x6BC100, "NiDX8TextureManager::ctor",
    fn(this, renderer: usize) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = manager_ctor);

// The call is always a skip: it releases the device and destroys the object.
port_thiscall!(manager_deleting_dtor_port, 0x6BC310, "NiDX8TextureManager::deleting_dtor",
    fn(this, flags: u32) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = manager_deleting_dtor);

// The call is always a skip: it releases the device and frees the bucket arrays.
port_thiscall!(manager_dtor_port, 0x6BC330, "NiDX8TextureManager::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = manager_dtor);

port_thiscall!(manager_get_texture_port, 0x6BC590, "NiDX8TextureManager::GetTexture",
    fn(this, a1: u32, a2: u32, a3: u32, a4: u32, a5: u32, a6: u32) -> u32,
    compare_eax = true,
    regions = |set| (),
    rust = manager_get_texture);

port_thiscall!(manager_prepare_texture_for_rendering_port, 0x6BC5A0, "NiDX8TextureManager::PrepareTextureForRendering",
    fn(this, texture: usize, out_created: usize, out_mipmapped: usize) -> usize,
    compare_eax = true,
    regions = |set| manager_prepare_regions(set, texture, out_created, out_mipmapped),
    rust = manager_prepare_texture_for_rendering);

port_thiscall!(manager_precache_texture_port, 0x6BC670, "NiDX8TextureManager::PrecacheTexture",
    fn(this, texture: usize, a3: u32, a4: u32) -> u8,
    compare_eax = true,
    regions = |set| skip_without_renderer_data(set, texture),
    rust = manager_precache_texture);

port_thiscall!(manager_get_format_from_texture_port, 0x6BC6F0, "NiDX8TextureManager::GetFormatFromTexture",
    fn(this, texture: usize) -> u32,
    compare_eax = true,
    regions = |set| skip_without_renderer_data(set, texture),
    rust = manager_get_format_from_texture);

port_thiscall!(manager_recreate_port, 0x6BC770, "NiDX8TextureManager::Recreate",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| manager_recreate_regions(set, this),
    rust_main = manager_recreate);

port_thiscall!(hash_map_dtor_port, 0x6BC840, "NiDX8TextureManager::HashMap::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| hash_map_dtor_regions(set, this, false),
    rust = hash_map_dtor);

port_thiscall!(clear_hash_map_port, 0x6BC8A0, "NiDX8TextureManager::clearHashMap",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| clear_hash_map_regions(set, this),
    rust = clear_hash_map);

port_thiscall!(hash_map_deleting_dtor_port, 0x6BC8F0, "NiDX8TextureManager::deleting_dtor_6BC8F0",
    fn(this, flags: u32) -> usize,
    compare_eax = true,
    regions = |set| hash_map_dtor_regions(set, this, flags & 1 != 0),
    rust = hash_map_deleting_dtor);

port_thiscall!(texture_data_ctor_port, 0x6B6D00, "NiDX8TextureData::ctor_args",
    fn(this, texture: usize, renderer: usize) -> usize,
    compare_eax = true,
    regions = |set| set.add(this, TD::SIZE),
    rust = texture_data_ctor);

port_thiscall!(texture_data_deleting_dtor_port, 0x6B6DC0, "NiDX8TextureData::deleting_dtor",
    fn(this, flags: u32) -> usize,
    compare_eax = true,
    regions = |set| texture_data_deleting_dtor_regions(set, this, flags),
    rust = texture_data_deleting_dtor);

port_thiscall!(texture_data_get_height_port, 0x6B6E10, "NiDX8TextureData::GetHeight",
    fn(this) -> u32,
    compare_eax = true,
    regions = |set| (),
    rust = texture_data_get_height);

port_thiscall!(texture_data_get_pixel_format_port, 0x6B6E30, "NiDX8TextureData::GetPixelFormat",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| (),
    rust = texture_data_get_pixel_format);

port_thiscall!(texture_data_get_d3d_palette_port, 0x6B6E40, "NiDX8TextureData::GetD3DPalette",
    fn(this) -> u32,
    compare_eax = true,
    regions = |set| (),
    rust = texture_data_get_d3d_palette);

port_cdecl!(find_closest_pixel_format_port, 0x6B6E50, "NiDX8TextureData_static::FindClosestPixelFormat",
    fn(renderer: usize, prefs: usize, formats: usize) -> usize,
    compare_eax = true,
    regions = |set| (),
    rust = find_closest_pixel_format);

port_thiscall!(find_matching_pixel_format_port, 0x6B6F70, "NiDX8TextureData::FindMatchingPixelFormat",
    fn(this, source: usize, prefs: usize, formats: usize) -> usize,
    compare_eax = true,
    regions = |set| (),
    rust = find_matching_pixel_format);

port_cdecl!(clear_texture_data_port, 0x6B74F0, "NiDX8TextureData_static::ClearTextureData",
    fn(renderer: usize) -> (),
    compare_eax = false,
    regions = |set| clear_texture_data_regions(set),
    rust = clear_texture_data);

port_thiscall!(source_data_ctor_port, 0x6BFDE0, "NiDX8SourceTextureData::ctor",
    fn(this, texture: usize, renderer: usize) -> usize,
    compare_eax = true,
    regions = |set| set.add(this, SD::SIZE),
    rust = source_data_ctor);

port_thiscall!(source_data_deleting_dtor_port, 0x6BFE70, "NiDX8SourceTextureData::deleting_dtor",
    fn(this, flags: u32) -> usize,
    compare_eax = true,
    regions = |set| source_data_dtor_regions(set, this, flags & 1 != 0),
    rust_main = source_data_deleting_dtor);

port_thiscall!(source_data_dtor_port, 0x6BFE90, "NiDX8SourceTextureData::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| source_data_dtor_regions(set, this, false),
    rust_main = source_data_dtor);

port_thiscall!(source_data_get_d3d_texture_port, 0x6BFF20, "NiDX8SourceTextureData::GetD3DTexture",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| (),
    rust = source_data_get_d3d_texture);

port_thiscall!(source_data_is_loaded_port, 0x6BFF30, "NiDX8SourceTextureData::IsLoaded",
    fn(this) -> u8,
    compare_eax = true,
    regions = |set| (),
    rust = source_data_is_loaded);

// The call is always a skip: it locks the surface.
port_cdecl!(source_data_copy_data_to_surface_port, 0x6BFF40, "NiDX8SourceTextureData::CopyDataToSurface",
    fn(pixel_data: usize, level: u32, surface: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = source_data_copy_data_to_surface);

port_thiscall!(source_data_update_port, 0x6C0050, "NiDX8SourceTextureData::Update",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| source_data_update_regions(set, this),
    rust = source_data_update);

port_thiscall!(source_data_set_palette_port, 0x6C0160, "NiDX8SourceTextureData::setPalette",
    fn(this, palette: usize) -> u8,
    compare_eax = true,
    regions = |set| source_data_set_palette_regions(set, this, palette),
    rust = source_data_set_palette);

port_cdecl!(blt_data_create_port, 0x6B6690, "NiDX8BltSourceData::Create",
    fn(blt_source: usize, renderer: usize) -> usize,
    compare_eax = true,
    regions = |set| blt_data_create_regions(set, blt_source),
    rust = blt_data_create);

port_thiscall!(blt_data_deleting_dtor_port, 0x6B67F0, "NiDX8BltSourceData::deleting_dtor",
    fn(this, flags: u32) -> usize,
    compare_eax = true,
    regions = |set| blt_data_dtor_regions(set, this, flags & 1 != 0),
    rust = blt_data_deleting_dtor);

port_thiscall!(blt_data_dtor_port, 0x6B6810, "NiDX8BltSourceData::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| blt_data_dtor_regions(set, this, false),
    rust = blt_data_dtor);

// The call is always a skip: it makes an image surface and locks it.
port_thiscall!(blt_data_create_surface_port, 0x6B68F0, "NiDX8BltSourceData::CreateSurface",
    fn(this, pixel_data: usize, pixel_format: usize) -> u8,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = blt_data_create_surface);

port_thiscall!(renderer_create_source_texture_renderer_data_port, 0x6B0380, "NiDX8Renderer::CreateSourceTextureRendererData",
    fn(this, texture: usize) -> u8,
    compare_eax = true,
    regions = |set| skip_without_renderer_data(set, texture),
    rust = renderer_create_source_texture_renderer_data);

port_thiscall!(renderer_create_source_cube_map_renderer_data_port, 0x6B0580, "NiDX8Renderer::CreateSourceCubeMapRendererData",
    fn(this, texture: usize) -> u8,
    compare_eax = true,
    regions = |set| skip_without_renderer_data(set, texture),
    rust = renderer_create_source_cube_map_renderer_data);

port_thiscall!(renderer_create_blt_source_renderer_data_port, 0x6B05B0, "NiDX8Renderer::CreateBltSourceRendererData",
    fn(this, blt_source: usize) -> u8,
    compare_eax = true,
    regions = |set| if rd::<usize>(blt_source, BLT_RENDERER_DATA) == 0 { blt_data_create_regions(set, blt_source) },
    rust = renderer_create_blt_source_renderer_data);

port_thiscall!(renderer_purge_all_textures_port, 0x6AC390, "NiDX8Renderer::PurgeAllTextures",
    fn(this, a2: u32) -> u8,
    compare_eax = true,
    regions = |set| clear_texture_data_regions(set),
    rust = renderer_purge_all_textures);

// The call is a skip when the texture has renderer data: the virtual deleting destructor of
// the renderer data runs.
port_thiscall!(renderer_purge_texture_port, 0x6AC3A0, "NiDX8Renderer::PurgeTexture",
    fn(this, texture: usize) -> u8,
    compare_eax = true,
    regions = |set| if rd::<usize>(texture, T::rendererData) != 0 { set.skip() },
    rust = renderer_purge_texture);

port_thiscall!(renderer_get_texture_memory_stats_port, 0x6AC400, "NiDX8Renderer::GetTextureMemoryStats",
    fn(this, a1: u32, a2: u32) -> u8,
    compare_eax = true,
    regions = |set| (),
    rust = renderer_get_texture_memory_stats);

port_thiscall!(renderer_get_texture_stats_port, 0x6AC410, "NiDX8Renderer::GetTextureStats_maybe",
    fn(this, a1: u32, a2: u32, a3: u32, a4: u32, a5: u32) -> u8,
    compare_eax = true,
    regions = |set| (),
    rust = renderer_get_texture_stats);

port_thiscall!(renderer_find_closest_pixel_format_port, 0x6ABD00, "NiDX8Renderer::FindClosestPixelFormat",
    fn(this, prefs: usize) -> usize,
    compare_eax = true,
    regions = |set| (),
    rust = renderer_find_closest_pixel_format);

pub static PORTS: &[&crate::shadow::PortState] = &[
    &manager_ctor_port::STATE,
    &manager_deleting_dtor_port::STATE,
    &manager_dtor_port::STATE,
    &manager_get_texture_port::STATE,
    &manager_prepare_texture_for_rendering_port::STATE,
    &manager_precache_texture_port::STATE,
    &manager_get_format_from_texture_port::STATE,
    &manager_recreate_port::STATE,
    &hash_map_dtor_port::STATE,
    &clear_hash_map_port::STATE,
    &hash_map_deleting_dtor_port::STATE,
    &texture_data_ctor_port::STATE,
    &texture_data_deleting_dtor_port::STATE,
    &texture_data_get_height_port::STATE,
    &texture_data_get_pixel_format_port::STATE,
    &texture_data_get_d3d_palette_port::STATE,
    &find_closest_pixel_format_port::STATE,
    &find_matching_pixel_format_port::STATE,
    &clear_texture_data_port::STATE,
    &source_data_ctor_port::STATE,
    &source_data_deleting_dtor_port::STATE,
    &source_data_dtor_port::STATE,
    &source_data_get_d3d_texture_port::STATE,
    &source_data_is_loaded_port::STATE,
    &source_data_copy_data_to_surface_port::STATE,
    &source_data_update_port::STATE,
    &source_data_set_palette_port::STATE,
    &blt_data_create_port::STATE,
    &blt_data_deleting_dtor_port::STATE,
    &blt_data_dtor_port::STATE,
    &blt_data_create_surface_port::STATE,
    &renderer_create_source_texture_renderer_data_port::STATE,
    &renderer_create_source_cube_map_renderer_data_port::STATE,
    &renderer_create_blt_source_renderer_data_port::STATE,
    &renderer_purge_all_textures_port::STATE,
    &renderer_purge_texture_port::STATE,
    &renderer_get_texture_memory_stats_port::STATE,
    &renderer_get_texture_stats_port::STATE,
    &renderer_find_closest_pixel_format_port::STATE,
];

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;
    use std::sync::OnceLock;

    use super::*;

    thread_local! {
        /// What the fake functions received: (kind, value).
        static NOTES: RefCell<Vec<(u32, u32)>> = const { RefCell::new(Vec::new()) };
    }

    pub fn note(kind: u32, value: u32) {
        NOTES.with(|n| n.borrow_mut().push((kind, value)));
    }

    #[test]
    fn the_mark_of_unreadable_data_lives_until_the_data_is_deleted() {
        let (data, other) = (0x7FFF_0010usize, 0x7FFF_0020usize);
        assert!(!is_unreadable(data));
        mark_unreadable(data);
        assert!(is_unreadable(data));
        assert!(!is_unreadable(other));
        forget_unreadable(other);
        assert!(is_unreadable(data));
        forget_unreadable(data);
        assert!(!is_unreadable(data));
    }

    fn notes() -> Vec<(u32, u32)> {
        NOTES.with(|n| core::mem::take(&mut *n.borrow_mut()))
    }

    /// A leaked block of zeroed words. Returns its address.
    fn block(words: usize) -> usize {
        Box::leak(vec![0u32; words].into_boxed_slice()).as_mut_ptr() as usize
    }

    /// An object whose vtable has the given functions at the given slots.
    fn object(words: usize, slots: &[(usize, usize)]) -> usize {
        let vtable = block(0x40);
        let object = block(words);
        unsafe {
            for &(slot, function) in slots {
                wr(vtable, slot, function);
            }
            wr(object, 0, vtable);
        }
        object
    }

    /// The fake image converter returns true when the first word of the wanted format is not 0.
    extern "thiscall" fn can_convert(_this: usize, _source: usize, dest: usize) -> u32 {
        note(0xCA, dest as u32);
        // Only the low byte is the result.
        0xAB00 | (unsafe { rd::<u32>(dest, 0) } != 0) as u32
    }

    /// The fake conversion gives the source pixel data back.
    extern "thiscall" fn convert_to_itself(_this: usize, source: usize, _format: usize, _scratch: usize, _mipmap: u32) -> usize {
        source
    }

    pub fn converter() -> usize {
        static CONVERTER: OnceLock<usize> = OnceLock::new();
        *CONVERTER.get_or_init(|| {
            object(1, &[(VT_CONVERTER_CAN_CONVERT, can_convert as *const () as usize), (VT_CONVERTER_CONVERT, convert_to_itself as *const () as usize)])
        })
    }

    /// A list of 14 formats. The converter can make the formats in `usable`. The formats in
    /// `present` exist.
    fn format_list(present: &[usize], usable: &[usize]) -> (usize, [usize; 14]) {
        let list = block(14);
        let mut formats = [0usize; 14];
        for &i in present {
            formats[i] = block(PF::SIZE / 4);
            unsafe {
                wr(formats[i], 0, usable.contains(&i) as u32);
                wr(list, i * 4, formats[i]);
            }
        }
        (list, formats)
    }

    fn prefs(layout: u32, alpha: u32) -> [u32; 3] {
        [layout, 0, alpha]
    }

    #[test]
    fn find_closest_takes_the_first_format_of_the_order_of_the_layout() {
        let all: Vec<usize> = (0..14).collect();
        let closest = |layout: u32, alpha: u32, present: &[usize]| -> Option<usize> {
            let (list, formats) = format_list(present, &[]);
            let p = prefs(layout, alpha);
            let found = unsafe { find_closest_pixel_format(0, p.as_ptr() as usize, list) };
            formats.iter().position(|&f| f != 0 && f == found)
        };
        // Layout 0: the palettised formats first.
        assert_eq!(closest(0, 0, &all), Some(6));
        assert_eq!(closest(0, 3, &all), Some(6));
        assert_eq!(closest(0, 1, &all), Some(7));
        assert_eq!(closest(0, 2, &all), Some(7));
        // Without them: the compressed formats, then the 16-bit order.
        assert_eq!(closest(0, 0, &[0, 1, 2, 3, 4, 5, 8, 9]), Some(8));
        assert_eq!(closest(0, 2, &[0, 1, 2, 3, 4, 5, 8, 9]), Some(9));
        assert_eq!(closest(0, 2, &[0, 1, 2, 3, 4, 5, 8]), Some(4));
        assert_eq!(closest(0, 3, &[0, 1, 2, 3, 4, 5, 8, 9]), None);
        assert_eq!(closest(3, 1, &all), Some(8));
        // Layout 1.
        assert_eq!(closest(1, 0, &all), Some(1));
        assert_eq!(closest(1, 0, &[0, 2, 3]), Some(0));
        assert_eq!(closest(1, 0, &[3, 4, 5]), Some(5));
        assert_eq!(closest(1, 0, &[3, 4]), Some(3));
        assert_eq!(closest(1, 1, &all), Some(3));
        assert_eq!(closest(1, 1, &[5]), Some(5));
        assert_eq!(closest(1, 2, &all), Some(4));
        assert_eq!(closest(1, 2, &[3, 5]), Some(5));
        assert_eq!(closest(1, 3, &all), None);
        // Layout 2, and layout 5 that becomes layout 2.
        assert_eq!(closest(2, 0, &all), Some(2));
        assert_eq!(closest(5, 0, &[0, 1, 3, 4, 5]), Some(5));
        assert_eq!(closest(2, 0, &[0, 1, 3, 4]), Some(1));
        assert_eq!(closest(2, 1, &[3, 4]), Some(3));
        assert_eq!(closest(2, 2, &[3, 4]), Some(4));
        assert_eq!(closest(2, 2, &[3]), None);
        // Unknown layouts.
        assert_eq!(closest(4, 0, &all), None);
        assert_eq!(closest(6, 0, &all), None);
    }

    #[test]
    fn find_matching_asks_the_converter_in_the_order_of_the_game() {
        let matching = |format: u32, layout: u32, alpha: u32, present: &[usize], usable: &[usize]| -> (Option<usize>, Vec<usize>) {
            let (list, formats) = format_list(present, usable);
            let p = prefs(layout, alpha);
            let source = [format, 0, 0, 0, 0, 0, 0, 0];
            notes();
            let found = unsafe { find_matching_pixel_format(0, source.as_ptr() as usize, p.as_ptr() as usize, list) };
            let asked = notes().iter().map(|&(_, dest)| formats.iter().position(|&f| f == dest as usize).unwrap()).collect();
            (formats.iter().position(|&f| f != 0 && f == found), asked)
        };
        let all: Vec<usize> = (0..14).collect();
        // Layout 4: the bump formats.
        assert_eq!(matching(11, 4, 0, &all, &[11]), (Some(11), vec![13, 12, 11]));
        assert_eq!(matching(0, 4, 0, &all, &[12]), (Some(12), vec![11, 13, 12]));
        assert_eq!(matching(0, 4, 0, &[11, 13], &[]), (None, vec![11, 13]));
        // RGB sources.
        assert_eq!(matching(0, 3, 0, &all, &[10]), (Some(10), vec![8, 9, 10]));
        assert_eq!(matching(2, 1, 0, &all, &[3]), (Some(3), vec![2, 0, 1, 5, 3]));
        assert_eq!(matching(4, 0, 0, &all, &[4]), (Some(4), vec![2, 5, 1, 0, 3, 4]));
        assert_eq!(matching(4, 0, 0, &all, &[]), (None, vec![2, 5, 1, 0, 3, 4]));
        // Compressed sources keep their format unless the preferences force a layout.
        assert_eq!(matching(6, 0, 0, &all, &[8]), (Some(8), vec![8]));
        assert_eq!(matching(7, 0, 0, &all, &[9]), (Some(9), vec![9]));
        assert_eq!(matching(8, 0, 0, &all, &[10]), (Some(10), vec![10]));
        assert_eq!(matching(6, 2, 0, &all, &[8, 3]), (Some(3), vec![5, 4, 3]));
        // RGBA sources.
        assert_eq!(matching(1, 3, 1, &all, &[10]), (Some(10), vec![8, 9, 10]));
        assert_eq!(matching(1, 3, 0, &all, &[4]), (Some(4), vec![9, 10, 8, 5, 4]));
        assert_eq!(matching(3, 1, 1, &all, &[5]), (Some(5), vec![3, 4, 5]));
        assert_eq!(matching(3, 1, 0, &all, &[3]), (Some(3), vec![5, 2, 3]));
        assert_eq!(matching(5, 0, 1, &all, &[4]), (Some(4), vec![5, 3, 4]));
        assert_eq!(matching(5, 0, 0, &all, &[]), (None, vec![5, 4, 3]));
        // Bump sources.
        assert_eq!(matching(10, 0, 0, &all, &[12]), (Some(12), vec![11, 13, 12]));
        assert_eq!(matching(11, 0, 0, &all, &[11]), (Some(11), vec![12, 13, 11]));
        // No such source format.
        assert_eq!(matching(9, 0, 0, &all, &all), (None, vec![]));
        assert_eq!(matching(12, 0, 0, &all, &all), (None, vec![]));
    }

    #[test]
    fn constructors_fill_the_objects() {
        let mut data = [0xEEu8; SD::SIZE];
        let d = data.as_mut_ptr() as usize;
        unsafe {
            assert_eq!(source_data_ctor(d, 0x1111, 0x2222), d);
            assert_eq!(rd::<u32>(d, SD::vtbl_vtbl), VTBL_SOURCE_TEXTURE_DATA);
            assert_eq!(rd::<u32>(d, SD::vtbl_linkedData), 0);
            assert_eq!(rd::<u32>(d, SD::vtbl_sgTexture), 0x1111);
            assert_eq!(rd::<u32>(d, SD::renderer), 0x2222);
            assert_eq!(rd::<u32>(d, SD::format_format), 2);
            assert_eq!(rd::<[u32; 4]>(d, SD::format_channelMasks), [0; 4]);
            assert_eq!(rd::<u32>(d, SD::format_bitsPerPixel), 8);
            // (2 << 3) & 0x18 | (1 << 2) & 0xFF
            assert_eq!(rd::<[u32; 2]>(d, SD::format_compareBits), [0x14, 0]);
            assert_eq!(rd::<[u32; 7]>(d, SD::format_extraData), [0; 7]);
            assert_eq!(rd::<u8>(d, SD::bMipMap), 0);
            assert_eq!(rd::<[u32; 5]>(d, SD::formattedSize), [0; 5]);
            // The constructor does not write the padding and the unnamed member.
            assert_eq!(rd::<u32>(d, SD::field_C), 0xEEEE_EEEE);
            assert_eq!(rd::<u8>(d, SD::pad_51), 0xEE);

            assert_eq!(texture_data_get_height(d), 0);
            wr::<u32>(d, SD::height, 64);
            wr::<u32>(d, SD::d3dPalette, 5);
            assert_eq!(texture_data_get_height(d), 64);
            assert_eq!(texture_data_get_d3d_palette(d), 5);
            assert_eq!(texture_data_get_pixel_format(d), d + 0x14);
            assert_eq!(source_data_is_loaded(d), 0);
            wr::<u32>(d, SD::d3dTexture, 0x3333);
            assert_eq!(source_data_is_loaded(d), 1);
            assert_eq!(source_data_get_d3d_texture(d), 0x3333);
        }
    }

    extern "thiscall" fn deleted(this: usize, flags: u32) -> u32 {
        note(0xDD, this as u32);
        assert_eq!(flags, 1);
        0
    }

    #[test]
    fn set_palette_moves_the_reference() {
        let mut data = [0u8; SD::SIZE];
        let d = data.as_mut_ptr() as usize;
        let old = object(4, &[(0, deleted as *const () as usize)]);
        let new = object(4, &[(0, deleted as *const () as usize)]);
        unsafe {
            wr::<u32>(old, REF_COUNT, 2);
            wr::<u32>(new, REF_COUNT, 1);
            wr::<u32>(new, PALETTE_REVISION, 9);
            assert_eq!(source_data_set_palette(d, 0), 0);
            wr(d, SD::palette, old);
            assert_eq!(source_data_set_palette(d, new), 1);
            assert_eq!((rd::<u32>(old, REF_COUNT), rd::<u32>(new, REF_COUNT)), (1, 2));
            assert_eq!(rd::<usize>(d, SD::palette), new);
            assert_eq!(rd::<u32>(d, SD::palRevisionID), 9);
            assert!(notes().is_empty());
            // Same palette, same revision.
            assert_eq!(source_data_set_palette(d, new), 0);
            wr::<u32>(new, PALETTE_REVISION, 10);
            assert_eq!(source_data_set_palette(d, new), 1);
            assert_eq!(rd::<u32>(new, REF_COUNT), 2);
            // When the old palette loses its last reference, the port deletes the palette.
            assert_eq!(source_data_set_palette(d, old), 1);
            assert_eq!((rd::<u32>(old, REF_COUNT), rd::<u32>(new, REF_COUNT)), (2, 1));
            wr::<u32>(old, REF_COUNT, 1);
            assert_eq!(source_data_set_palette(d, new), 1);
            assert_eq!(notes(), vec![(0xDD, old as u32)]);
        }
    }

    extern "thiscall" fn map_hash(_map: usize, key: u32) -> u32 {
        key % 5
    }
    extern "thiscall" fn map_dtor_pair(_map: usize, node: usize) -> u32 {
        note(0xD0, unsafe { rd::<u32>(node, HN::key) });
        0
    }
    extern "system" fn released(this: usize) -> u32 {
        note(0x8E, this as u32);
        0
    }

    #[test]
    fn recreate_destroys_the_values_then_the_nodes() {
        let manager = block(TM::SIZE / 4);
        let map = manager + TM::hashMap_10;
        let map_vtable = block(8);
        let buckets = block(5);
        let device_object = object(1, &[(crate::d3d::COM_RELEASE, released as *const () as usize)]);
        unsafe {
            wr(map_vtable, hash_map::SLOT_HASH_KEY, map_hash as *const () as usize);
            wr(map_vtable, hash_map::SLOT_DTOR_KEY_VALUE_PAIR, map_dtor_pair as *const () as usize);
            wr(map, HM::vtbl, map_vtable);
            wr::<u32>(map, HM::bucketCount, 5);
            wr(map, HM::arrayBuckets, buckets);
            // Keys 1 and 6 in bucket 1, key 3 in bucket 3. Key 6 has a value with a device object.
            let node = |key: u32, value: usize, next: usize| {
                let n = block(3);
                wr(n, HN::key, key);
                wr(n, HN::data, value);
                wr(n, HN::nextNode, next);
                n
            };
            let value = block(0x10);
            wr(value, MAP_VALUE_DEVICE_OBJECT, device_object);
            let n6 = node(6, value, 0);
            let n1 = node(1, 0, n6);
            let n3 = node(3, 0, 0);
            wr(buckets, 4, n1);
            wr(buckets, 12, n3);
            wr::<u32>(map, HM::count, 3);

            let mut set = Regions::new();
            manager_recreate_regions(&mut set, manager);
            assert!(set.as_slice().is_empty(), "a value is destroyed: the call is skipped");

            notes();
            manager_recreate(&MainThread::for_test(), manager);
            assert_eq!(
                notes(),
                vec![
                    (0x8E, device_object as u32),
                    (0xDE1E7E, value as u32),
                    (0xD0, 1),
                    (0xDE1E7E, n1 as u32),
                    (0xD0, 6),
                    (0xDE1E7E, n6 as u32),
                    (0xD0, 3),
                    (0xDE1E7E, n3 as u32),
                ]
            );
            assert_eq!(rd::<u32>(map, HM::count), 0);
            assert_eq!(rd::<[u32; 5]>(buckets, 0), [0; 5]);

            hash_map_dtor(map);
            assert_eq!(rd::<u32>(map, HM::vtbl), VTBL_HASH_MAP);
            assert_eq!(notes(), vec![(0xDE1E7E, buckets as u32)]);
        }
    }

    thread_local! {
        /// The pitch, the height and the pixel buffer of the fake surface.
        static SURFACE: RefCell<(u32, u32, usize)> = const { RefCell::new((0, 0, 0)) };
    }

    extern "system" fn surf_get_desc(_this: usize, desc: usize) -> u32 {
        unsafe { wr::<u32>(desc, 28, SURFACE.with(|s| s.borrow().1)) };
        0
    }
    extern "system" fn surf_lock_rect(_this: usize, locked: usize, rect: usize, flags: u32) -> u32 {
        assert_eq!((rect, flags), (0, 0));
        note(0x10C, 0);
        let (pitch, _, bits) = SURFACE.with(|s| *s.borrow());
        unsafe {
            wr::<u32>(locked, 0, pitch);
            wr::<usize>(locked, 4, bits);
        }
        0
    }
    extern "system" fn surf_unlock_rect(_this: usize) -> u32 {
        note(0x0101, 0);
        0
    }

    /// Pixel data with two levels: 3x2 and 1x1 pixels of 3 bytes.
    fn pixel_data(format: u32, pixels: &[u8]) -> usize {
        let data = block(PD::SIZE / 4);
        let widths = block(2);
        let heights = block(2);
        let offsets = block(3);
        unsafe {
            wr::<[u32; 2]>(widths, 0, [3, 1]);
            wr::<[u32; 2]>(heights, 0, [2, 1]);
            wr::<[u32; 3]>(offsets, 0, [0, 18, 21]);
            wr(data, PD::format_format, format);
            wr(data, PD::pixelsAllMips, pixels.as_ptr() as usize);
            wr(data, PD::mipmapWidths, widths);
            wr(data, PD::mipmapHeights, heights);
            wr(data, PD::mipmapOffsets, offsets);
            wr::<u32>(data, PD::mipmapLevels, 2);
            wr::<u32>(data, PD::bytesPerPixel, 3);
        }
        data
    }

    #[test]
    fn copy_data_to_surface_copies_rows_or_one_block() {
        let pixels: Vec<u8> = (1..=21).collect();
        let surface = object(
            1,
            &[(crate::d3d::SURFACE_GET_DESC, surf_get_desc as *const () as usize), (crate::d3d::SURFACE_LOCK_RECT, surf_lock_rect as *const () as usize), (crate::d3d::SURFACE_UNLOCK_RECT, surf_unlock_rect as *const () as usize)],
        );
        let run = |format: u32, level: u32, pitch: u32, height: u32| -> Vec<u8> {
            let mut target = vec![0u8; 40];
            SURFACE.with(|s| *s.borrow_mut() = (pitch, height, target.as_mut_ptr() as usize));
            notes();
            unsafe { source_data_copy_data_to_surface(pixel_data(format, &pixels), level, surface) };
            assert_eq!(notes(), vec![(0x10C, 0), (0x0101, 0)]);
            target
        };
        // The pitch is larger than a row: the port copies each row and does not write the gap.
        let target = run(0, 0, 16, 2);
        assert_eq!(&target[..9], &pixels[..9]);
        assert_eq!(&target[9..16], &[0; 7]);
        assert_eq!(&target[16..25], &pixels[9..18]);
        assert_eq!(&target[25..], &[0; 15]);
        // The pitch is equal to a row: the port copies the full level as one block, for any height.
        let target = run(0, 0, 9, 0);
        assert_eq!(&target[..18], &pixels[..18]);
        assert_eq!(&target[18..], &[0; 22]);
        // Compressed data: the port copies the full level as one block, for any pitch.
        for format in [6, 7, 8] {
            let target = run(format, 1, 16, 1);
            assert_eq!(&target[..3], &pixels[18..21]);
            assert_eq!(&target[3..], &[0; 37]);
        }
        // The second level, one row at a time.
        let target = run(5, 1, 4, 1);
        assert_eq!(&target[..4], &[19, 20, 21, 0]);
    }

    extern "system" fn dev_create_image_surface(_this: usize, width: u32, height: u32, format: u32, out: usize) -> u32 {
        note(0x6C, width << 16 | height << 8 | format);
        unsafe { wr::<usize>(out, 0, TEST_SURFACE.with(|s| *s.borrow())) };
        0
    }

    thread_local! {
        static TEST_SURFACE: RefCell<usize> = const { RefCell::new(0) };
    }

    #[test]
    fn create_surface_makes_the_surface_and_fills_it() {
        let pixels: Vec<u8> = (1..=21).collect();
        let surface = object(
            1,
            &[(crate::d3d::SURFACE_GET_DESC, surf_get_desc as *const () as usize), (crate::d3d::SURFACE_LOCK_RECT, surf_lock_rect as *const () as usize), (crate::d3d::SURFACE_UNLOCK_RECT, surf_unlock_rect as *const () as usize)],
        );
        TEST_SURFACE.with(|s| *s.borrow_mut() = surface);
        let device = object(1, &[(crate::d3d::DEVICE_CREATE_IMAGE_SURFACE, dev_create_image_surface as *const () as usize)]);
        let renderer = block(R::d3dDevice / 4 + 1);
        let data = block(BD_SIZE / 4);
        let mut format = [0u32; PF::SIZE / 4];
        format[PF::d3dFormat / 4] = 22;
        let mut target = vec![0u8; 40];
        SURFACE.with(|s| *s.borrow_mut() = (10, 2, target.as_mut_ptr() as usize));
        unsafe {
            wr(renderer, R::d3dDevice, device);
            wr(data, BD_RENDERER, renderer);
            notes();
            assert_eq!(blt_data_create_surface(data, pixel_data(0, &pixels), format.as_ptr() as usize), 1);
            assert_eq!(notes(), vec![(0x6C, 3 << 16 | 2 << 8 | 22), (0x10C, 0), (0x0101, 0)]);
            assert_eq!(rd::<usize>(data, BD_SURFACE), surface);
        }
        assert_eq!(&target[..9], &pixels[..9]);
        assert_eq!(target[9], 0);
        assert_eq!(&target[10..19], &pixels[9..18]);
        assert_eq!(&target[19..], &[0; 21]);
    }

    extern "thiscall" fn data_as_rendered(_this: usize) -> u32 {
        0
    }
    extern "thiscall" fn data_d3d_texture(this: usize) -> u32 {
        unsafe { rd(this, SD::d3dTexture) }
    }
    extern "thiscall" fn data_levels(this: usize) -> u32 {
        unsafe { rd(this, SD::levels) }
    }
    extern "thiscall" fn data_updated(this: usize) -> u32 {
        note(0x0D, this as u32);
        0
    }
    extern "thiscall" fn data_pixel_format(this: usize) -> u32 {
        (this + SD::format) as u32
    }

    #[test]
    fn prepare_texture_updates_what_is_not_static() {
        let data = object(
            SD::SIZE / 4,
            &[
                (VT_DATA_GET_PIXEL_FORMAT, data_pixel_format as *const () as usize),
                (VT_DATA_GET_LEVELS, data_levels as *const () as usize),
                (VT_DATA_GET_D3D_TEXTURE, data_d3d_texture as *const () as usize),
                (VT_DATA_GET_AS_RENDERED, data_as_rendered as *const () as usize),
                (VT_DATA_UPDATE, data_updated as *const () as usize),
            ],
        );
        let texture = block(ST::SIZE / 4);
        let manager = block(TM::SIZE / 4);
        let mut flags = [7u8; 2];
        let (created, mipmapped) = (flags.as_mut_ptr() as usize, flags.as_mut_ptr() as usize + 1);
        unsafe {
            assert_eq!(manager_prepare_texture_for_rendering(manager, 0, created, mipmapped), 0);
            assert_eq!(flags, [0, 0]);

            wr(texture, T::rendererData, data);
            wr::<u32>(data, SD::d3dTexture, 0x5555);
            wr::<u32>(data, SD::levels, 4);
            wr::<u32>(data, SD::format_extraData, 21);
            notes();
            assert_eq!(manager_prepare_texture_for_rendering(manager, texture, created, mipmapped), 0x5555);
            assert_eq!(flags, [0, 1]);
            assert_eq!(notes(), vec![(0x0D, data as u32)]);

            wr::<u8>(texture, ST::isStatic, 1);
            wr::<u32>(data, SD::levels, 1);
            assert_eq!(manager_prepare_texture_for_rendering(manager, texture, created, mipmapped), 0x5555);
            assert_eq!(flags, [0, 0]);
            assert!(notes().is_empty());

            // No device texture: the update makes it, also for a static texture.
            wr::<u32>(data, SD::d3dTexture, 0);
            assert_eq!(manager_prepare_texture_for_rendering(manager, texture, created, mipmapped), 0);
            assert_eq!(flags, [1, 0]);
            assert_eq!(notes(), vec![(0x0D, data as u32)]);

            assert_eq!(manager_precache_texture(manager, texture, 0, 0), 1);
            assert_eq!(manager_precache_texture(manager, 0, 0, 0), 0);
            assert_eq!(manager_get_format_from_texture(manager, texture), 21);
        }
    }

    /// A source renderer data of `width` x `height` with `levels` levels and the revision 5,
    /// for a texture whose pixel data is `pixels` (revision `revision`).
    fn source_data_for(width: u32, height: u32, levels: u32, pixels: usize, revision: u32) -> (usize, usize) {
        let texture = block(ST::SIZE / 4);
        let data = block(SD::SIZE / 4);
        unsafe {
            wr(texture, ST::pixelData, pixels);
            wr::<u32>(pixels, PD::revisionID, revision);
            wr::<u32>(data, TD::vtbl_vtbl, VTBL_SOURCE_TEXTURE_DATA);
            wr(data, SD::vtbl_sgTexture, texture);
            wr(data, SD::width, width);
            wr(data, SD::height, height);
            wr(data, SD::levels, levels);
            wr::<u32>(data, SD::sourceRevisionID, 5);
        }
        (texture, data)
    }

    /// The audit's U3-02: new pixel data of another size asks for a new renderer data, and
    /// a size that `Create` would refuse keeps the old picture without asking again.
    #[test]
    fn a_new_size_of_the_pixel_data_asks_for_a_new_renderer_data() {
        let pixels = [0u8; 21];
        let new = pixel_data(0, &pixels); // 3 x 2: not a power of two
        unsafe {
            // The same revision: nothing to do, whatever the size.
            let (texture, data) = source_data_for(8, 8, 1, new, 5);
            assert_eq!(source_data_size_change(texture, data), SizeChange::None);
            // A new revision of the same size: an update.
            let (texture, data) = source_data_for(3, 2, 2, new, 6);
            assert_eq!(source_data_size_change(texture, data), SizeChange::None);
            // Not a source renderer data: the update as before.
            wr::<u32>(data, TD::vtbl_vtbl, 0x1234);
            assert_eq!(source_data_size_change(texture, data), SizeChange::None);
            // Another size that is not a power of two: the picture stays, the revision is seen.
            let (texture, data) = source_data_for(4, 4, 3, new, 7);
            assert_eq!(source_data_size_change(texture, data), SizeChange::Unusable);
            assert_eq!(rd::<u32>(data, SD::sourceRevisionID), 7);
            assert_eq!(source_data_size_change(texture, data), SizeChange::None);
            // Another size that is a power of two: a new renderer data.
            wr::<[u32; 2]>(rd::<usize>(new, PD::mipmapWidths), 0, [16, 8]);
            wr::<[u32; 2]>(rd::<usize>(new, PD::mipmapHeights), 0, [8, 4]);
            let (texture, data) = source_data_for(4, 4, 3, new, 8);
            assert_eq!(source_data_size_change(texture, data), SizeChange::Remake);
        }
    }

    extern "system" fn texture_surface_level(_this: usize, level: u32, _out: usize) -> u32 {
        note(0x5E, level);
        // A failure ends the loop of the update at once.
        1
    }
    extern "system" fn texture_release(_this: usize) -> u32 {
        note(0x5E1E, 0);
        0
    }

    /// The audit's U3-02: `Update` copies only pixel data of the size of the device texture,
    /// and a failed level read does not release the texture.
    #[test]
    fn update_copies_only_pixel_data_of_the_size_of_the_texture() {
        let pixels = [0u8; 21];
        let new = pixel_data(0, &pixels); // 3 x 2, 2 levels
        let device_texture = object(
            1,
            &[(crate::d3d::TEXTURE_GET_SURFACE_LEVEL, texture_surface_level as *const () as usize), (crate::d3d::COM_RELEASE, texture_release as *const () as usize)],
        );
        unsafe {
            // The texture holds its pixel data: the fake conversion gives it back, and the end
            // of `Update` releases the converted data.
            wr::<u32>(new, REF_COUNT, 1);
            // Another size: no level is read, and the revision stays unseen.
            let (_texture, data) = source_data_for(4, 4, 3, new, 6);
            wr(data, SD::d3dTexture, device_texture);
            notes();
            source_data_update(data);
            assert!(notes().iter().all(|n| n.0 != 0x5E && n.0 != 0x5E1E));
            assert_eq!(rd::<u32>(data, SD::sourceRevisionID), 5);
            // The same size: the update reads level 0, which fails; the texture stays.
            let (_texture, data) = source_data_for(3, 2, 2, new, 6);
            wr(data, SD::d3dTexture, device_texture);
            notes();
            source_data_update(data);
            let got = notes();
            assert!(got.contains(&(0x5E, 0)));
            assert!(!got.iter().any(|n| n.0 == 0x5E1E));
            assert_eq!(rd::<usize>(data, SD::d3dTexture), device_texture);
            assert_eq!(rd::<u32>(data, SD::sourceRevisionID), 6);
        }
    }

    #[test]
    fn purge_texture_takes_the_head_of_the_renderer_data() {
        let first = object(3, &[(0, deleted as *const () as usize)]);
        let texture = block(T::SIZE / 4);
        unsafe {
            wr::<u32>(first, TD::vtbl_linkedData, 0x7777);
            wr(texture, T::rendererData, first);
            notes();
            assert_eq!(renderer_purge_texture(0, texture), 1);
            assert_eq!(rd::<u32>(texture, T::rendererData), 0x7777);
            assert_eq!(notes(), vec![(0xDD, first as u32)]);
            wr::<u32>(texture, T::rendererData, 0);
            assert_eq!(renderer_purge_texture(0, texture), 1);
            assert!(notes().is_empty());
        }
    }
}
