//! `NiDX8Renderer`: creation, loss and destruction of the device.
//!
//! The game code has C++ exception frames in `Create`, `Recreate`, `dtor`, `GetAdapters`,
//! `createDefaultTextures`, `probeTextureFormats` and `videoAdapterEnum`. They are not here:
//! each one only frees a block from `operator new` when the constructor that runs on it
//! throws, or runs the member destructors of `dtor` when one of them throws.

use core::mem::transmute;

use crate::gen::layouts::{
    NiDX8AdapterDesc as Adapter, NiDX8DeviceDesc as DeviceDesc, NiDX8PixelFormat as PF, NiDX8RenderState as RS,
    NiDX8RenderTarget as RT, NiDX8Renderer as R, NiDX8Renderer_BatchedObject as Batched, NiDX8SystemDesc as SystemDesc,
    NiPixelData, NiRefObject, THashMap as Map, THashMap_Node as Node, D3DCAPS8 as Caps,
};
use crate::mem::{rd, wr};
use crate::ports::{light_manager, render_state, texture_pipeline};
use crate::shadow::Regions;

// ---------------------------------------------------------------------------------------
// Engine data at fixed addresses
// ---------------------------------------------------------------------------------------

/// `data_IDirect3D8`: the `IDirect3D8` of the game.
const DIRECT3D: usize = 0x7DD8C0;
/// `lpDirect3DCreate8`: the address of `Direct3DCreate8` in D3D8.DLL, or 0.
const DIRECT3D_CREATE8: usize = 0x7DD8C4;
/// `data_NiRTTI_NiDX8Renderer`
const RTTI_NI_DX8_RENDERER: usize = 0x7DD89C;
/// `global_NiDX8Renderer_errorString`: 256 bytes.
const ERROR_STRING: usize = 0x7DEAA8;
const ERROR_STRING_SIZE: usize = 0x100;
/// `vtbl_sg_NiDX8Renderer`
const VTABLE_RENDERER: usize = 0x74F4D0;
/// The vtables the destructor gives the four hash maps before it empties them.
const VTABLE_MAP_CUBE_MAPS: usize = 0x74F5E4;
const VTABLE_MAP_TEXTURES: usize = 0x74F5F8;
const VTABLE_MAP_SKIN_PARTITIONS: usize = 0x74F60C;
const VTABLE_MAP_GEOM_BUFFERS: usize = 0x74F620;
/// `dword_7C3834`: the number of texture stages the destructor clears.
const TEXTURE_STAGE_COUNT: usize = 0x7C3834;
/// `dword_7DD958`: the number of vertex streams the destructor clears.
const STREAM_COUNT: usize = 0x7DD958;
/// The values the inline constructor of `NiPixelFormat` in `Create` reads for `compareBits`.
const PIXEL_FORMAT_SHIFT_A: usize = 0x7DE8FC;
const PIXEL_FORMAT_MASK_A: usize = 0x7C55B0;
const PIXEL_FORMAT_MASK_B: usize = 0x7C55B4;
const PIXEL_FORMAT_SHIFT_B: usize = 0x7C55B8;
/// The empty C string.
const EMPTY_STRING: usize = 0x7C6180;
/// `data_zeroVec3`, `data_vecXAxis`, `data_vecYAxis`, `data_vecZAxis`
const VEC_ZERO: usize = 0x7DE6CC;
const VEC_X_AXIS: usize = 0x7DE690;
const VEC_Y_AXIS: usize = 0x7DE6A4;
const VEC_Z_AXIS: usize = 0x7DE6C0;

/// "D3D8.DLL" and "Direct3DCreate8".
const STR_D3D8_DLL: usize = 0x7C2BE8;
const STR_DIRECT3D_CREATE8: usize = 0x7C2BD8;
/// The texts of `nullsub_36`, the debug print the release build has as an empty function.
const LOG_D3D_OK: usize = 0x7C2C14;
const LOG_D3D_FAILED: usize = 0x7C2BF4;
const LOG_INVALID_DEVICE_ID: usize = 0x7C30A4;
/// The texts `strncpy` puts into the error string.
const ERR_NO_DIRECT3D: usize = 0x7C30E4;
const ERR_INVALID_ADAPTER: usize = 0x7C3080;
const ERR_INVALID_DEVICE_TYPE: usize = 0x7C301C;
const ERR_DEVICE_NOT_VALID: usize = 0x7C2FB0;
const ERR_NO_REFERENCE_DEVICE: usize = 0x7C2E80;
const ERR_NO_HARDWARE_DEVICE: usize = 0x7C2DA8;
const ERR_NO_DEVICE_CAPS: usize = 0x7C2CC0;
const ERR_TOO_FEW_TEXTURES: usize = 0x7C3478;
const ERR_NO_FRAMEBUFFER_FORMAT: usize = 0x7C33E4;
const ERR_NO_DEPTH_FORMAT: usize = 0x7C335C;
const ERR_NO_DISPLAY_MODE: usize = 0x7C3280;

/// The strings `GetDriverInfo` reads.
const STR_PURE: usize = 0x7C34F4;
const STR_MT: usize = 0x7C34F0;
const STR_HWVP: usize = 0x7C34E8;
const STR_MIXVP: usize = 0x7C34E0;
const STR_SWVP: usize = 0x7C34D8;
const STR_HAL: usize = 0x7C34C8;
const STR_REF: usize = 0x7C34CC;
const STR_SW: usize = 0x7C34D0;
const STR_UNKNOWN_DEVICE: usize = 0x7C34D4;
/// "%s (%s-%s)"
const STR_DRIVER_FORMAT: usize = 0x7C34BC;

// Members the layouts do not have.
/// `NiTexture::rendererData`
const TEXTURE_RENDERER_DATA: usize = 0x20;
/// `NiTexture::RendererData::linkedData`: the next renderer data of the same texture.
const RENDERER_DATA_NEXT: usize = 0x04;
/// The two vertex buffers of a geometry buffer of the renderer (`+0x14`, `+0x24`).
const GEOM_BUFFER_VERTEX_BUFFER: usize = 0x14;
const GEOM_BUFFER_INDEX_BUFFER: usize = 0x24;
/// `NiSkinPartition::Partition`: its geometry buffer.
const PARTITION_GEOM_BUFFER: usize = 0x28;
/// The value of a node of the `std::map` of `TES3Game` (`map_74`): the adapter description.
const STD_MAP_NODE_VALUE: usize = 0x1C;
/// `TES3Game::map_74`
const GAME_ADAPTER_MAP: usize = 0x74;
/// `D3DADAPTER_IDENTIFIER8`: 0x42C bytes; `Description` at 0x200.
const ADAPTER_IDENTIFIER_WORDS: usize = 0x10B;
const ADAPTER_IDENTIFIER_DESCRIPTION: usize = 0x200;
/// `D3DSURFACE_DESC`: Format, Type, Usage, Pool, Size, MultiSampleType, Width, Height.
const SURFACE_DESC_FORMAT: usize = 0;
const SURFACE_DESC_WIDTH: usize = 6;
const SURFACE_DESC_HEIGHT: usize = 7;

const D3DDEVTYPE_HAL: u32 = 1;
const D3DDEVTYPE_REF: u32 = 2;
const D3DFMT_P8: i32 = 41;
const D3DFMT_D15S1: u32 = 73;
const D3DFMT_D24S8: u32 = 75;
const D3DFMT_D24X4S4: u32 = 79;
const FOURCC_DXT1: u32 = 0x3154_5844;
const FOURCC_DXT2: u32 = 0x3254_5844;
const FOURCC_DXT3: u32 = 0x3354_5844;
const FOURCC_DXT4: u32 = 0x3454_5844;
const FOURCC_DXT5: u32 = 0x3554_5844;
const FOURCC_YUY2: u32 = 0x3259_5559;
const FOURCC_UYVY: u32 = 0x5956_5955;
/// `D3DCREATE_MULTITHREADED`
const D3DCREATE_MULTITHREADED: u32 = 4;

/// The sizes `Create` gives `operator new`.
const SIZE_VERTEX_BUFFER_MANAGER: usize = 0x3C;
const SIZE_INDEX_BUFFER_MANAGER: usize = 0x4C;
const SIZE_TEXTURE_MANAGER: usize = 0x40;
const SIZE_LIGHT_MANAGER: usize = 0x74;
const SIZE_NI_PIXEL_FORMAT: usize = 0x20;

/// Texture formats in one group of `arrayArrayTextureFormats`.
const FORMATS_PER_GROUP: usize = 14;
const FORMAT_GROUPS: usize = 4;

// ---------------------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------------------

unsafe fn thiscall0(addr: usize, this: usize) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize) -> u32>(addr)(this)
}

unsafe fn thiscall1(addr: usize, this: usize, a: u32) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32) -> u32>(addr)(this, a)
}

unsafe fn thiscall2(addr: usize, this: usize, a: u32, b: u32) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32, u32) -> u32>(addr)(this, a, b)
}

unsafe fn thiscall3(addr: usize, this: usize, a: u32, b: u32, c: u32) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32, u32, u32) -> u32>(addr)(this, a, b, c)
}

unsafe fn thiscall4(addr: usize, this: usize, a: u32, b: u32, c: u32, d: u32) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32, u32, u32, u32) -> u32>(addr)(this, a, b, c, d)
}

unsafe fn thiscall5(addr: usize, this: usize, a: u32, b: u32, c: u32, d: u32, e: u32) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32, u32, u32, u32, u32) -> u32>(addr)(this, a, b, c, d, e)
}

/// A virtual function of `object` at `offset` in its vtable.
unsafe fn virtual_at(object: usize, offset: usize) -> usize {
    rd(rd::<usize>(object, 0), offset)
}

/// The virtual deleting destructor of an engine object, with "free the memory".
unsafe fn delete_object(object: usize) {
    thiscall1(virtual_at(object, 0), object, 1);
}

/// `Release` of a COM object.
unsafe fn com_release(object: usize) -> u32 {
    transmute::<usize, unsafe extern "system" fn(usize) -> u32>(virtual_at(object, 0x08))(object)
}

/// 0x727692 `operator new`
unsafe fn operator_new(size: usize) -> usize {
    crate::engine::operator_new(size)
}

/// 0x727530 `operator delete`
unsafe fn operator_delete(block: usize) {
    transmute::<usize, unsafe extern "C" fn(usize)>(0x727530)(block)
}

/// 0x6E9A00 `nullsub_36`: the debug print, a single `retn`. The game code gives it a text
/// and at most one value.
unsafe fn debug_print(text: usize) {
    transmute::<usize, unsafe extern "C" fn(usize)>(0x6E9A00)(text)
}

unsafe fn debug_print1(text: usize, value: usize) {
    transmute::<usize, unsafe extern "C" fn(usize, usize)>(0x6E9A00)(text, value)
}

/// 0x6E9A00 again: the destructor of `NiBound`, with the bound in ECX.
unsafe fn bound_dtor(bound: usize) {
    thiscall0(0x6E9A00, bound);
}

// The C runtime and Win32, through the import table of the game.

/// `strncpy` (import slot 0x746284)
unsafe fn strncpy(to: usize, from: usize, count: usize) -> usize {
    transmute::<usize, unsafe extern "C" fn(usize, usize, usize) -> usize>(rd::<usize>(0x746284, 0))(to, from, count)
}

/// `sprintf` (import slot 0x746274) with three string arguments.
unsafe fn sprintf3(to: usize, format: usize, a: usize, b: usize, c: usize) -> u32 {
    transmute::<usize, unsafe extern "C" fn(usize, usize, usize, usize, usize) -> u32>(rd::<usize>(0x746274, 0))(to, format, a, b, c)
}

/// `LoadLibraryA` (import slot 0x7460E4)
unsafe fn load_library_a(name: usize) -> usize {
    transmute::<usize, unsafe extern "system" fn(usize) -> usize>(rd::<usize>(0x7460E4, 0))(name)
}

/// `GetProcAddress` (import slot 0x7460C4)
unsafe fn get_proc_address(module: usize, name: usize) -> usize {
    transmute::<usize, unsafe extern "system" fn(usize, usize) -> usize>(rd::<usize>(0x7460C4, 0))(module, name)
}

/// `GetModuleHandleA` (import slot 0x746064)
unsafe fn get_module_handle_a(name: usize) -> usize {
    transmute::<usize, unsafe extern "system" fn(usize) -> usize>(rd::<usize>(0x746064, 0))(name)
}

/// `FreeLibrary` (import slot 0x7460BC)
unsafe fn free_library(module: usize) -> u32 {
    transmute::<usize, unsafe extern "system" fn(usize) -> u32>(rd::<usize>(0x7460BC, 0))(module)
}

/// `std::string::assign(const char *, unsigned)` of MSVCP60 (import slot 0x7461CC)
unsafe fn std_string_assign(string: usize, text: usize, length: u32) {
    thiscall2(rd::<usize>(0x7461CC, 0), string, text as u32, length);
}

/// `std::string::string(const std::string &)` of MSVCP60 (import slot 0x7461C8)
unsafe fn std_string_copy_ctor(string: usize, from: usize) {
    thiscall1(rd::<usize>(0x7461C8, 0), string, from as u32);
}

/// `std::string::_Tidy(bool)` of MSVCP60 (import slot 0x7461C0)
unsafe fn std_string_tidy(string: usize, built: u32) {
    thiscall1(rd::<usize>(0x7461C0, 0), string, built);
}

// Engine functions.

/// 0x6A9A70 `NiDX8Renderer::ctor`. Another mod changes it in memory, so it is not ported.
unsafe fn renderer_ctor(this: usize) -> usize {
    thiscall0(0x6A9A70, this) as usize
}

/// 0x6F4F80 `NiRenderer::dtor`
unsafe fn ni_renderer_dtor(this: usize) {
    thiscall0(0x6F4F80, this);
}

/// 0x6B26A0 `NiDX8SystemDesc::ctor`
unsafe fn system_desc_ctor(this: usize, direct3d: usize) -> usize {
    thiscall1(0x6B26A0, this, direct3d as u32) as usize
}

/// 0x6B2800 `NiDX8SystemDesc::dtor`
unsafe fn system_desc_dtor(this: usize) {
    thiscall0(0x6B2800, this);
}

/// 0x6B1EC0 `NiDX8DeviceDesc::getScreenFormatEnum`
unsafe fn device_desc_get_screen_format_enum(desc: usize, windowed: u32, depth: u32) -> u32 {
    thiscall2(0x6B1EC0, desc, windowed, depth)
}

/// 0x6B21D0 `NiDX8DeviceDesc::findClosestDepthStencilForScreenFormat`
unsafe fn device_desc_find_depth_stencil(desc: usize, format: u32, target_format: u32, depth_bits: u32, stencil_bits: u32) -> u32 {
    thiscall4(0x6B21D0, desc, format, target_format, depth_bits, stencil_bits)
}

/// 0x6B25D0 `NiDX8AdapterDesc::findClosestMode`. Returns the mode or 0; changes the rate.
unsafe fn adapter_desc_find_closest_mode(adapter: usize, format: u32, width: u32, height: u32, refresh_rate: usize) -> usize {
    thiscall4(0x6B25D0, adapter, format, width, height, refresh_rate as u32) as usize
}

/// 0x6ED7C0 `NiPixelFormat::ctor_RGBA`
unsafe fn pixel_format_ctor_rgba(this: usize, red: u32, green: u32, blue: u32, alpha: u32, bits: u32) -> usize {
    thiscall5(0x6ED7C0, this, red, green, blue, alpha, bits) as usize
}

/// 0x6ED9D0 `NiPixelFormat::ctor_Palette`
unsafe fn pixel_format_ctor_palette(this: usize, has_alpha: u32) -> usize {
    thiscall1(0x6ED9D0, this, has_alpha) as usize
}

/// 0x6EDD30 `NiPixelFormat::ctor_BumpLuma`
unsafe fn pixel_format_ctor_bump_luma(this: usize, red: u32, green: u32, blue: u32, bits: u32) -> usize {
    thiscall4(0x6EDD30, this, red, green, blue, bits) as usize
}

/// 0x6EDA40 `NiPixelFormat::ctor_Format`
unsafe fn pixel_format_ctor_format(this: usize, format: u32) -> usize {
    thiscall1(0x6EDA40, this, format) as usize
}

/// 0x6EDEE0 `NiPixelFormat::ctor_RGBA_copy`. Returns `this`.
unsafe fn pixel_format_ctor_rgba_copy(this: usize, red: u32, green: u32, blue: u32, alpha: u32, bits: u32) -> usize {
    thiscall5(0x6EDEE0, this, red, green, blue, alpha, bits) as usize
}

/// 0x6EDF10 `NiPixelFormat::ctor_BumpLuma_copy`. Returns `this`.
unsafe fn pixel_format_ctor_bump_luma_copy(this: usize, red: u32, green: u32, blue: u32, bits: u32) -> usize {
    thiscall4(0x6EDF10, this, red, green, blue, bits) as usize
}

/// 0x6BFDA0 `NiPixelFormat::copy`
unsafe fn pixel_format_copy(this: usize, from: usize) {
    thiscall1(0x6BFDA0, this, from as u32);
}

/// 0x6B0E20 `NiGetD3DFormatForNiPixelFormat` (cdecl)
unsafe fn d3d_format_for_pixel_format(format: usize) -> u32 {
    transmute::<usize, unsafe extern "C" fn(usize) -> u32>(0x6B0E20)(format)
}

/// 0x6AB410 `NiDX8PixelFormat::ctor`
unsafe fn dx8_pixel_format_ctor(this: usize) {
    thiscall0(0x6AB410, this);
}

/// 0x4B52D0 `NiDX8PixelFormat::setPixelFormat`
unsafe fn dx8_pixel_format_set(this: usize, format: u32) {
    thiscall1(0x4B52D0, this, format);
}

/// 0x6D4FC0 `NiPixelData::ctor_args`
unsafe fn pixel_data_ctor(this: usize, width: u32, height: u32, format: usize, mip_map_levels: u32) -> usize {
    thiscall4(0x6D4FC0, this, width, height, format as u32, mip_map_levels) as usize
}

/// 0x6BD780 `NiDX8VertexBufferManager::ctor`
unsafe fn vertex_buffer_manager_ctor(this: usize, device: usize) -> usize {
    thiscall1(0x6BD780, this, device as u32) as usize
}

/// 0x6BC970 `NiDX8IndexBufferManager::ctor`
unsafe fn index_buffer_manager_ctor(this: usize, device: usize) -> usize {
    thiscall1(0x6BC970, this, device as u32) as usize
}

/// 0x6BC100 `NiDX8TextureManager::ctor`
unsafe fn texture_manager_ctor(this: usize, renderer: usize) -> usize {
    thiscall1(0x6BC100, this, renderer as u32) as usize
}

/// 0x6B81E0 `NiDX8RenderState_static::Create` (cdecl)
unsafe fn render_state_create(device: usize, caps: usize, a: u32, b: u32, stencil: u32) -> usize {
    transmute::<usize, unsafe extern "C" fn(usize, usize, u32, u32, u32) -> usize>(0x6B81E0)(device, caps, a, b, stencil)
}

/// 0x6B8680 `NiDX8RenderState::Initialize_maybe`
unsafe fn render_state_initialize(render_state: usize) {
    thiscall0(0x6B8680, render_state);
}

/// 0x6BD370 `NiDX8IndexBufferManager::Recreate`
unsafe fn index_buffer_manager_recreate(manager: usize) {
    thiscall0(0x6BD370, manager);
}

/// 0x6BE910 `NiDX8VertexBufferManager::Recreate`
unsafe fn vertex_buffer_manager_recreate(manager: usize) {
    thiscall0(0x6BE910, manager);
}

/// 0x6BC770 `NiDX8TextureManager::Recreate`
unsafe fn texture_manager_recreate(manager: usize) {
    thiscall0(0x6BC770, manager);
}

/// 0x6BC8A0 `NiDX8TextureManager::clearHashMap`: frees every node of a hash map.
unsafe fn clear_hash_map(map: usize) {
    thiscall0(0x6BC8A0, map);
}

/// 0x6B74F0 `NiDX8TextureData_static::ClearTextureData` (cdecl)
unsafe fn clear_texture_data(renderer: usize) {
    transmute::<usize, unsafe extern "C" fn(usize)>(0x6B74F0)(renderer)
}

/// 0x6B68A0 `sub_6B68A0` (cdecl): deletes the first renderer data of every texture in the
/// list at 0x7DE11C.
unsafe fn clear_listed_renderer_data(renderer: usize) {
    transmute::<usize, unsafe extern "C" fn(usize)>(0x6B68A0)(renderer)
}

/// 0x6B1140 `sub_6B1140`: takes the node with `key` out of a hash map and frees it.
unsafe fn hash_map_remove(map: usize, key: usize) {
    thiscall1(0x6B1140, map, key as u32);
}

/// 0x6B12B0 `sub_6B12B0`: gives the key and the value at the position `*position` of a
/// hash map and moves the position to the next node.
unsafe fn hash_map_get_next(map: usize, position: usize, key: usize, value: usize) {
    thiscall3(0x6B12B0, map, position as u32, key as u32, value as u32);
}

/// 0x404630 `NiObject::release`
unsafe fn ni_object_release(object: usize) {
    thiscall0(0x404630, object);
}

/// 0x72759E `eh vector destructor iterator`
unsafe fn vector_destructor(array: usize, size: usize, count: u32, dtor: usize) {
    transmute::<usize, unsafe extern "system" fn(usize, usize, u32, usize)>(0x72759E)(array, size, count, dtor)
}

/// 0x40A880 `NiPointer::dtor`, for the vector iterator.
const NI_POINTER_DTOR: usize = 0x40A880;

/// 0x42A770 `sub_42A770`: `erase(first, last)` of the adapter map of `TES3Game`.
unsafe fn adapter_map_erase(map: usize, out: usize, first: usize, last: usize) {
    thiscall3(0x42A770, map, out as u32, first as u32, last as u32);
}

/// 0x42A8A0 `StdMap_NiDX8Application::insert`
unsafe fn adapter_map_insert(map: usize, result: usize, pair: usize) {
    thiscall2(0x42A8A0, map, result as u32, pair as u32);
}

/// 0x42B230 `StdMap::insert_result::copy`
unsafe fn insert_result_copy(to: usize, node: usize, valid: usize) {
    thiscall2(0x42B230, to, node as u32, valid as u32);
}

// ---------------------------------------------------------------------------------------
// Direct3D, through the vtable on each call
// ---------------------------------------------------------------------------------------

unsafe fn d3d_get_adapter_identifier(d3d: usize, adapter: u32, flags: u32, identifier: usize) -> u32 {
    let f: unsafe extern "system" fn(usize, u32, u32, usize) -> u32 = transmute(virtual_at(d3d, 0x14));
    f(d3d, adapter, flags, identifier)
}

unsafe fn d3d_check_device_format(d3d: usize, adapter: u32, device_type: u32, adapter_format: u32, usage: u32, resource_type: u32, format: u32) -> u32 {
    let f: unsafe extern "system" fn(usize, u32, u32, u32, u32, u32, u32) -> u32 = transmute(virtual_at(d3d, 0x28));
    f(d3d, adapter, device_type, adapter_format, usage, resource_type, format)
}

unsafe fn d3d_check_device_multi_sample_type(d3d: usize, adapter: u32, device_type: u32, format: u32, windowed: u32, samples: u32) -> u32 {
    let f: unsafe extern "system" fn(usize, u32, u32, u32, u32, u32) -> u32 = transmute(virtual_at(d3d, 0x2C));
    f(d3d, adapter, device_type, format, windowed, samples)
}

unsafe fn d3d_create_device(d3d: usize, adapter: u32, device_type: u32, focus: u32, behavior: u32, parameters: usize, device: usize) -> u32 {
    let f: unsafe extern "system" fn(usize, u32, u32, u32, u32, usize, usize) -> u32 = transmute(virtual_at(d3d, 0x3C));
    f(d3d, adapter, device_type, focus, behavior, parameters, device)
}

unsafe fn device_reset(device: usize, parameters: usize) -> u32 {
    transmute::<usize, unsafe extern "system" fn(usize, usize) -> u32>(virtual_at(device, 0x38))(device, parameters)
}

unsafe fn device_get_render_target(device: usize, surface: usize) -> u32 {
    transmute::<usize, unsafe extern "system" fn(usize, usize) -> u32>(virtual_at(device, 0x80))(device, surface)
}

unsafe fn device_get_depth_stencil_surface(device: usize, surface: usize) -> u32 {
    transmute::<usize, unsafe extern "system" fn(usize, usize) -> u32>(virtual_at(device, 0x84))(device, surface)
}

unsafe fn device_set_texture(device: usize, stage: u32, texture: u32) -> u32 {
    transmute::<usize, unsafe extern "system" fn(usize, u32, u32) -> u32>(virtual_at(device, 0xF4))(device, stage, texture)
}

unsafe fn device_set_stream_source(device: usize, stream: u32, buffer: u32, stride: u32) -> u32 {
    transmute::<usize, unsafe extern "system" fn(usize, u32, u32, u32) -> u32>(virtual_at(device, 0x14C))(device, stream, buffer, stride)
}

unsafe fn device_set_indices(device: usize, buffer: u32, base: u32) -> u32 {
    transmute::<usize, unsafe extern "system" fn(usize, u32, u32) -> u32>(virtual_at(device, 0x154))(device, buffer, base)
}

/// `IDirect3DSurface8::GetDesc`
unsafe fn surface_get_desc(surface: usize, desc: usize) -> u32 {
    transmute::<usize, unsafe extern "system" fn(usize, usize) -> u32>(virtual_at(surface, 0x20))(surface, desc)
}

// ---------------------------------------------------------------------------------------
// Small pieces the functions share
// ---------------------------------------------------------------------------------------

/// Puts a text into the error string of the renderer and gives the debug print its text.
unsafe fn creation_failed(error: usize, log: usize) {
    strncpy(ERROR_STRING, error, 0xFF);
    debug_print(log);
}

/// The sequence that `CreateD3D`, `GetAdapters` and `Create` have inline: gets
/// `Direct3DCreate8` from D3D8.DLL when the game does not have it, and makes the
/// `IDirect3D8` (SDK version 120). False when a step gave 0.
unsafe fn load_direct3d() -> bool {
    let mut create: usize = rd(DIRECT3D_CREATE8, 0);
    if create == 0 {
        let module = load_library_a(STR_D3D8_DLL);
        if module == 0 {
            return false;
        }
        create = get_proc_address(module, STR_DIRECT3D_CREATE8);
        wr(DIRECT3D_CREATE8, 0, create);
        if create == 0 {
            return false;
        }
    }
    let direct3d = transmute::<usize, unsafe extern "system" fn(u32) -> usize>(create)(120);
    wr(DIRECT3D, 0, direct3d);
    direct3d != 0
}

/// The device description of the adapter and device type the renderer has. The game code
/// reads `caps.DeviceType` of the result before it tests the result for 0.
unsafe fn selected_device_desc(this: usize) -> usize {
    let system: usize = rd(this, R::systemDesc);
    let index: usize = rd(this, R::currentAdapterIndex);
    let adapter: usize = rd(rd(system, SystemDesc::adapterList_storage), index * 4);
    if rd::<u32>(this, R::d3dDeviceType) == D3DDEVTYPE_HAL {
        rd(adapter, Adapter::HALDeviceDesc)
    } else {
        rd(adapter, Adapter::REFDeviceDesc)
    }
}

/// The first node of a hash map, or 0.
unsafe fn hash_map_first(map: usize) -> usize {
    let bucket_count: u32 = rd(map, Map::bucketCount);
    if bucket_count > 0 {
        let buckets: usize = rd(map, Map::arrayBuckets);
        let mut index = 0u32;
        loop {
            if rd::<usize>(buckets, index as usize * 4) != 0 {
                return rd(buckets, index as usize * 4);
            }
            index += 1;
            if index >= bucket_count {
                break;
            }
        }
    }
    0
}

/// The first node in the buckets after the bucket of `key`, or 0.
unsafe fn hash_map_first_after(map: usize, key: usize) -> usize {
    let mut index = thiscall1(virtual_at(map, 4), map, key as u32).wrapping_add(1);
    let bucket_count: u32 = rd(map, Map::bucketCount);
    if index < bucket_count {
        let buckets: usize = rd(map, Map::arrayBuckets);
        loop {
            let node: usize = rd(buckets, index as usize * 4);
            if node != 0 {
                return node;
            }
            index += 1;
            if index >= bucket_count {
                break;
            }
        }
    }
    0
}

/// Frees every node of a hash map and sets its count to 0, as `dtor` has it inline.
unsafe fn hash_map_drain(map: usize) {
    let mut index = 0u32;
    if rd::<u32>(map, Map::bucketCount) > 0 {
        loop {
            let mut buckets: usize = rd(map, Map::arrayBuckets);
            let mut node: usize = rd(buckets, index as usize * 4);
            while node != 0 {
                wr::<u32>(buckets, index as usize * 4, rd(node, Node::nextNode));
                thiscall1(virtual_at(map, 0x10), map, node as u32);
                operator_delete(node);
                buckets = rd(map, Map::arrayBuckets);
                node = rd(buckets, index as usize * 4);
            }
            index += 1;
            if index >= rd::<u32>(map, Map::bucketCount) {
                break;
            }
        }
    }
    wr::<u32>(map, Map::count, 0);
}

/// Releases the two device buffers of a geometry buffer and frees it.
unsafe fn free_geom_buffer(buffer: usize) {
    let vertex_buffer: usize = rd(buffer, GEOM_BUFFER_VERTEX_BUFFER);
    if vertex_buffer != 0 {
        com_release(vertex_buffer);
    }
    let index_buffer: usize = rd(buffer, GEOM_BUFFER_INDEX_BUFFER);
    if index_buffer != 0 {
        com_release(index_buffer);
    }
    operator_delete(buffer);
}

/// Takes one reference from an object and deletes it at 0.
unsafe fn release_reference(object: usize) {
    let count = rd::<u32>(object, NiRefObject::refCount).wrapping_sub(1);
    wr(object, NiRefObject::refCount, count);
    if count == 0 {
        delete_object(object);
    }
}

// ---------------------------------------------------------------------------------------
// CreateD3D, ReleaseD3D, GetAdapters, GetRTTI
// ---------------------------------------------------------------------------------------

/// 0x6A99E0 `NiDX8Renderer::CreateD3D`. Returns the `IDirect3D8` of the game.
pub unsafe fn create_d3d() -> usize {
    crate::shadow::touch(&create_d3d_port::STATE);
    let direct3d: usize = rd(DIRECT3D, 0);
    if direct3d != 0 {
        return direct3d;
    }
    if load_direct3d() {
        debug_print(LOG_D3D_OK);
    } else {
        debug_print(LOG_D3D_FAILED);
    }
    rd(DIRECT3D, 0)
}

/// 0x6A9A50 `NiDX8Renderer::ReleaseD3D`. Returns 0.
pub unsafe fn release_d3d() -> u32 {
    crate::shadow::touch(&release_d3d_port::STATE);
    let direct3d: usize = rd(DIRECT3D, 0);
    if direct3d != 0 {
        com_release(direct3d);
        wr::<u32>(DIRECT3D, 0, 0);
    }
    0
}

/// 0x6A9C50 `NiDX8Renderer::GetRTTI`
pub unsafe fn get_rtti(_this: usize) -> usize {
    crate::shadow::touch(&get_rtti_port::STATE);
    RTTI_NI_DX8_RENDERER
}

/// 0x6B0FC0 `NiDX8Renderer_static::GetAdapters`. Returns a new `NiDX8SystemDesc` or 0.
pub unsafe fn get_adapters() -> usize {
    crate::shadow::touch(&get_adapters_port::STATE);
    if rd::<usize>(DIRECT3D, 0) == 0 {
        load_direct3d();
    }
    let direct3d: usize = rd(DIRECT3D, 0);
    if direct3d == 0 {
        return 0;
    }
    let block = operator_new(SystemDesc::SIZE);
    if block == 0 {
        return 0;
    }
    system_desc_ctor(block, direct3d)
}

/// The game has no `IDirect3D8`: the call loads D3D8.DLL. Else the system description is
/// new memory from `operator new`, and its constructor only asks `IDirect3D8`.
unsafe fn get_adapters_regions(set: &mut Regions) {
    if rd::<usize>(DIRECT3D, 0) == 0 {
        set.skip();
        return;
    }
    set.allocates();
}

// ---------------------------------------------------------------------------------------
// D3DFORMAT_toString
// ---------------------------------------------------------------------------------------

/// 0x6B2920 `NiDX8Renderer_static::D3DFORMAT_toString`. Returns the address of the name in
/// the game. The game code compares the format as a signed number, then as unsigned in the
/// two tables; both orders give "UNKNOWN" for a value that is not in the list.
pub unsafe fn d3dformat_to_string(format: u32) -> usize {
    crate::shadow::touch(&d3dformat_to_string_port::STATE);
    match format {
        0 => 0x7C3778,           // D3DFMT_UNKNOWN
        20 => 0x7C36BC,          // R8G8B8
        21 => 0x7C3768,          // A8R8G8B8
        22 => 0x7C3758,          // X8R8G8B8
        23 => 0x7C3748,          // R5G6B5
        24 => 0x7C3738,          // X1R5G5B5
        25 => 0x7C3728,          // A1R5G5B5
        26 => 0x7C3718,          // A4R4G4B4
        27 => 0x7C36AC,          // R3G3B2
        28 => 0x7C370C,          // A8
        29 => 0x7C369C,          // A8R3G3B2
        30 => 0x7C368C,          // X4R4G4B4
        40 => 0x7C3680,          // A8P8
        41 => 0x7C3700,          // P8
        50 => 0x7C36F4,          // L8
        51 => 0x7C36E8,          // A8L8
        52 => 0x7C3674,          // A4L4
        60 => 0x7C36DC,          // V8U8
        61 => 0x7C36CC,          // L6V5U5
        62 => 0x7C3664,          // X8L8V8U8
        63 => 0x7C3654,          // Q8W8V8U8
        64 => 0x7C3644,          // V16U16
        65 => 0x7C360C,          // W11V11U10
        70 => 0x7C3630,          // D16_LOCKABLE
        71 => 0x7C3600,          // D32
        73 => 0x7C35F0,          // D15S1
        75 => 0x7C3620,          // D24S8
        77 => 0x7C35D4,          // D24X8
        79 => 0x7C35C4,          // D24X4S4
        80 => 0x7C35E4,          // D16
        100 => 0x7C35B0,         // VERTEXDATA
        101 => 0x7C3590,         // INDEX16
        102 => 0x7C35A0,         // INDEX32
        FOURCC_DXT1 => 0x7C3584, // DXT1
        FOURCC_DXT2 => 0x7C3578, // DXT2
        FOURCC_YUY2 => 0x7C356C, // YUY2
        FOURCC_DXT3 => 0x7C3554, // DXT3
        FOURCC_DXT4 => 0x7C3560, // DXT4
        FOURCC_DXT5 => 0x7C3548, // DXT5
        FOURCC_UYVY => 0x7C353C, // UYVY
        _ => 0x7941B8,           // "UNKNOWN"
    }
}

// ---------------------------------------------------------------------------------------
// InitializePresentParams
// ---------------------------------------------------------------------------------------

/// The `D3DFORMAT` of a frame buffer format of NetImmerse (1 to 11), else `D3DFMT_UNKNOWN`.
fn frame_buffer_d3d_format(format: u32) -> u32 {
    if (1..=11).contains(&format) {
        format + 19
    } else {
        0
    }
}

/// `D3DMULTISAMPLE_TYPE` of a sample count: 2 to 16 stay, the rest is "none".
fn multi_sample_type(samples: u32) -> u32 {
    if (2..=16).contains(&samples) {
        samples
    } else {
        0
    }
}

/// `D3DSWAPEFFECT`: with more than one sample always "discard" (1).
fn swap_effect_value(samples: u32, swap_effect: u32) -> u32 {
    if samples == 0 || samples == 1 {
        match swap_effect {
            2 => 2, // flip
            3 => 3, // copy
            4 => 4, // copy vsync
            _ => 1,
        }
    } else {
        1
    }
}

/// `-1` asks for `D3DPRESENT_RATE_UNLIMITED`.
fn refresh_rate_value(refresh_rate: u32) -> u32 {
    if refresh_rate == 0xFFFF_FFFF {
        0x7FFF_FFFF
    } else {
        refresh_rate
    }
}

/// `D3DPRESENT_INTERVAL_*` of the interval enum of NetImmerse.
fn presentation_interval_value(interval: u32) -> u32 {
    match interval {
        0 => 0x8000_0000, // immediate
        1 => 1,
        2 => 2,
        3 => 4,
        4 => 8,
        _ => 0,
    }
}

/// `InitializePresentParams` with the address of the `IDirect3D8` it asks.
#[allow(clippy::too_many_arguments)]
unsafe fn initialize_present_params_on(
    this: usize,
    direct3d: usize,
    width: u32,
    height: u32,
    flags: u32,
    samples: u32,
    backbuffer_format: u32,
    depth_stencil_format: u32,
    backbuffer_count: u32,
    swap_effect: u32,
    refresh_rate: u32,
    presentation_interval: u32,
) -> u8 {
    let device_desc: usize = rd(this, R::maybe_currentDeviceDesc);
    // Bit 2 of the flags is "full screen".
    let windowed = !(flags >> 2) & 1;
    // The game code puts the byte back into the argument and gives the whole word on.
    let windowed_word = (flags & 0xFFFF_FF00) | windowed;
    let depth: u32 = if flags & 0x10 != 0 { 16 } else { 32 };

    let mut format = backbuffer_format;
    if format == 0 {
        format = device_desc_get_screen_format_enum(device_desc, windowed_word, depth);
        if format == 0 {
            creation_failed(ERR_NO_FRAMEBUFFER_FORMAT, 0x7C33A0);
            return 0;
        }
    }
    let d3d_format = frame_buffer_d3d_format(format);

    let mut depth_stencil = depth_stencil_format;
    if depth_stencil == 0 {
        depth_stencil = device_desc_find_depth_stencil(rd(this, R::maybe_currentDeviceDesc), d3d_format, d3d_format, depth, flags & 8);
        if depth_stencil == 0 {
            creation_failed(ERR_NO_DEPTH_FORMAT, 0x7C3318);
            return 0;
        }
    }

    for word in 0..13 {
        wr::<u32>(this, R::d3dPresentParameters + word * 4, 0);
    }
    wr(this, R::d3dPresentParameters_BackBufferWidth, width);
    wr(this, R::d3dPresentParameters_BackBufferHeight, height);
    wr(this, R::d3dPresentParameters_BackBufferFormat, d3d_format);
    let sample_type = multi_sample_type(samples);
    wr(this, R::d3dPresentParameters_AutoDepthStencilFormat, depth_stencil);
    wr(this, R::d3dPresentParameters_MultiSampleType, sample_type);
    // Bit 0 of the flags is "no depth buffer".
    wr(this, R::d3dPresentParameters_EnableAutoDepthStencil, !flags & 1);
    wr(this, R::d3dPresentParameters_BackBufferCount, backbuffer_count);
    wr(this, R::d3dPresentParameters_Windowed, windowed);
    wr(this, R::d3dPresentParameters_SwapEffect, swap_effect_value(samples, swap_effect));
    // D3DPRESENTFLAG_LOCKABLE_BACKBUFFER when the sample count is exactly 1.
    wr(this, R::d3dPresentParameters_Flags, (samples == 1) as u32);
    wr::<u32>(this, R::d3dPresentParameters_hDeviceWindow, rd(this, R::windowDevice));
    let rate_slot = this + R::d3dPresentParameters_FullScreen_RefreshRateInHz;
    wr(rate_slot, 0, refresh_rate_value(refresh_rate));
    let adapter: u32 = rd(this, R::currentAdapterIndex);
    wr(this, R::d3dPresentParameters_PresentationInterval, presentation_interval_value(presentation_interval));

    let d3d: usize = rd(direct3d, 0);
    if d3d_check_device_multi_sample_type(d3d, adapter, rd(this, R::d3dDeviceType), d3d_format, windowed, sample_type) != 0 {
        debug_print(0x7C32C0);
        wr::<u32>(this, R::d3dPresentParameters_MultiSampleType, 0);
    }
    let count: u32 = rd(this, R::d3dPresentParameters_BackBufferCount);
    if count == 0 {
        wr::<u32>(this, R::d3dPresentParameters_BackBufferCount, 1);
    } else if count > 3 {
        wr::<u32>(this, R::d3dPresentParameters_BackBufferCount, 3);
    }

    let asked_rate: u32 = rd(rate_slot, 0);
    if rd::<u32>(this, R::d3dPresentParameters_Windowed) != 0 {
        wr::<u32>(rate_slot, 0, 0);
    } else {
        let mode = adapter_desc_find_closest_mode(
            rd(this, R::maybe_currentAdapter),
            rd(this, R::d3dPresentParameters_BackBufferFormat),
            rd(this, R::d3dPresentParameters_BackBufferWidth),
            rd(this, R::d3dPresentParameters_BackBufferHeight),
            rate_slot,
        );
        if mode == 0 {
            creation_failed(ERR_NO_DISPLAY_MODE, 0x7C3238);
            return 0;
        }
        if asked_rate != rd::<u32>(rate_slot, 0) {
            debug_print(0x7C31D8);
        }
    }

    if rd::<u32>(this, R::d3dPresentParameters_Windowed) != 0 {
        wr::<u32>(this, R::d3dPresentParameters_PresentationInterval, 0);
        return 1;
    }
    // The device description from the start of the function.
    let intervals: u32 = rd(device_desc, DeviceDesc::caps + Caps::PresentationIntervals);
    if rd::<u32>(this, R::d3dPresentParameters_PresentationInterval) & intervals == 0 {
        wr::<u32>(this, R::d3dPresentParameters_PresentationInterval, 1);
    }
    1
}

/// 0x6AB470 `NiDX8Renderer::InitializePresentParams`: fills `d3dPresentParameters`.
/// Returns 0 when the adapter has no format or mode for the request.
#[allow(clippy::too_many_arguments)]
pub unsafe fn initialize_present_params(
    this: usize,
    width: u32,
    height: u32,
    flags: u32,
    samples: u32,
    backbuffer_format: u32,
    depth_stencil_format: u32,
    backbuffer_count: u32,
    swap_effect: u32,
    refresh_rate: u32,
    presentation_interval: u32,
) -> u8 {
    crate::shadow::touch(&initialize_present_params_port::STATE);
    initialize_present_params_on(
        this,
        DIRECT3D,
        width,
        height,
        flags,
        samples,
        backbuffer_format,
        depth_stencil_format,
        backbuffer_count,
        swap_effect,
        refresh_rate,
        presentation_interval,
    )
}

/// The renderer and the error string. The calls to `IDirect3D8` and to the descriptions
/// only read.
unsafe fn initialize_present_params_regions(set: &mut Regions, this: usize) {
    set.add(this, R::SIZE);
    set.add(ERROR_STRING, ERROR_STRING_SIZE);
}

// ---------------------------------------------------------------------------------------
// validateDeviceCaps
// ---------------------------------------------------------------------------------------

/// 0x6AB960 `NiDX8Renderer::validateDeviceCaps`: the capability flags (`GetFlags`), the
/// number of hardware bones and whether cube maps can have mip maps.
pub unsafe fn validate_device_caps(this: usize) -> u8 {
    crate::shadow::touch(&validate_device_caps_port::STATE);
    let device_desc = selected_device_desc(this);
    if rd::<u32>(device_desc, DeviceDesc::caps) == 0 || device_desc == 0 {
        creation_failed(ERR_NO_DEVICE_CAPS, 0x7C2C80);
        return 0;
    }
    let caps = device_desc + DeviceDesc::caps;
    if rd::<u32>(caps, Caps::MaxTextureBlendStages) < 2 || rd::<u32>(caps, Caps::MaxSimultaneousTextures) < 2 {
        creation_failed(ERR_TOO_FEW_TEXTURES, 0x7C3420);
        return 0;
    }
    wr::<u32>(this, R::capabilityFlags, 0x6D);
    if rd::<u32>(this, R::d3dPresentParameters_EnableAutoDepthStencil) != 0 {
        let format: u32 = rd(this, R::d3dPresentParameters_AutoDepthStencilFormat);
        if format == D3DFMT_D15S1 || format == D3DFMT_D24X4S4 || format == D3DFMT_D24S8 {
            // With stencil.
            wr::<u32>(this, R::capabilityFlags, 0xED);
        }
    }
    // D3DPRASTERCAPS_WBUFFER
    if rd::<u32>(caps, Caps::RasterCaps) & 0x40000 != 0 {
        wr::<u32>(this, R::capabilityFlags, rd::<u32>(this, R::capabilityFlags) | 0x100);
    }
    // D3DPTEXTURECAPS_SQUAREONLY
    if rd::<u8>(caps, Caps::TextureCaps) & 0x20 != 0 {
        wr::<u32>(this, R::capabilityFlags, rd::<u32>(this, R::capabilityFlags) | 2);
    }
    wr::<u32>(this, R::countHWBones, rd(caps, Caps::MaxVertexBlendMatrices));
    // D3DPTEXTURECAPS_MIPCUBEMAP
    wr::<u8>(this, R::maybe_bMipmapCubeMaps, rd::<u8>(caps, Caps::TextureCaps + 2) & 1);
    1
}

unsafe fn validate_device_caps_regions(set: &mut Regions, this: usize) {
    set.add(this, R::SIZE);
    set.add(ERROR_STRING, ERROR_STRING_SIZE);
}

// ---------------------------------------------------------------------------------------
// GetDriverInfo
// ---------------------------------------------------------------------------------------

/// The length of a C string.
unsafe fn c_string_length(text: usize) -> usize {
    let mut length = 0;
    while rd::<u8>(text, length) != 0 {
        length += 1;
    }
    length
}

/// Copies `count` bytes from `from` to the end of the C string at `to`, as the inline
/// `strcat` of the game code does with a dword, a word and a byte.
unsafe fn append_bytes(to: usize, from: usize, count: usize) {
    let end = to + c_string_length(to);
    for at in 0..count {
        wr::<u8>(end, at, rd(from, at));
    }
}

/// The text of the behaviour flags of the device in `field_28C` (32 bytes).
unsafe fn write_behavior_text(this: usize, empty: usize, texts: [usize; 5]) {
    let text = this + R::field_28C;
    let mut at = 0;
    loop {
        let byte: u8 = rd(empty, at);
        wr(text, at, byte);
        at += 1;
        if byte == 0 {
            break;
        }
    }
    let [pure, mt, hwvp, mixvp, swvp] = texts;
    if rd::<u8>(this, R::d3dBehaviorFlags) & 0x10 != 0 {
        append_bytes(text, pure, 5); // "PURE"
    }
    if rd::<u8>(this, R::d3dBehaviorFlags) & 0x04 != 0 {
        append_bytes(text, mt, 4); // " MT"
    }
    if rd::<u8>(this, R::d3dBehaviorFlags) & 0x40 != 0 {
        append_bytes(text, hwvp, 6); // " HWVP"
    }
    if rd::<u8>(this, R::d3dBehaviorFlags) & 0x80 != 0 {
        append_bytes(text, mixvp, 7); // " MIXVP"
    }
    if rd::<u8>(this, R::d3dBehaviorFlags) & 0x20 != 0 {
        append_bytes(text, swvp, 6); // " SWVP"
    }
}

/// 0x6ABBA0 `NiDX8Renderer::GetDriverInfo`: "description (HAL-PURE MT HWVP)" in
/// `driverDesc`. Returns `driverDesc`.
pub unsafe fn get_driver_info(this: usize) -> usize {
    crate::shadow::touch(&get_driver_info_port::STATE);
    let description = this + R::driverDesc;
    let mut identifier = [0u32; ADAPTER_IDENTIFIER_WORDS];
    let identifier = identifier.as_mut_ptr() as usize;
    let adapter: u32 = rd(this, R::currentAdapterIndex);
    wr::<u8>(description, 0, 0);
    let d3d: usize = rd(DIRECT3D, 0);
    if d3d_get_adapter_identifier(d3d, adapter, 0, identifier) != 0 {
        return description;
    }
    write_behavior_text(this, EMPTY_STRING, [STR_PURE, STR_MT, STR_HWVP, STR_MIXVP, STR_SWVP]);
    let device_type = match rd::<u32>(this, R::d3dDeviceType) {
        1 => STR_HAL,
        2 => STR_REF,
        3 => STR_SW,
        _ => STR_UNKNOWN_DEVICE,
    };
    sprintf3(description, STR_DRIVER_FORMAT, identifier + ADAPTER_IDENTIFIER_DESCRIPTION, device_type, this + R::field_28C);
    description
}

// ---------------------------------------------------------------------------------------
// Texture formats and default textures
// ---------------------------------------------------------------------------------------

/// The `NiPixelFormat` of the slot `index` of a texture format group: a new block from
/// `operator new`, or 0 when the block is 0. `None` for the two slots with no format.
unsafe fn probe_pixel_format(index: u32) -> Option<usize> {
    if index == 6 || index == 7 {
        return None;
    }
    let block = operator_new(SIZE_NI_PIXEL_FORMAT);
    if block == 0 {
        return Some(0);
    }
    Some(match index {
        0 => pixel_format_ctor_rgba_copy(block, 0x7C00, 0x3E0, 0x1F, 0, 16),
        1 => pixel_format_ctor_rgba_copy(block, 0xF800, 0x7E0, 0x1F, 0, 16),
        3 => pixel_format_ctor_rgba_copy(block, 0x7C00, 0x3E0, 0x1F, 0x8000, 16),
        4 => pixel_format_ctor_rgba_copy(block, 0xF00, 0xF0, 0xF, 0xF000, 16),
        5 => pixel_format_ctor_rgba_copy(block, 0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000, 32),
        // The three constructors by format leave the block address in EDI.
        8 => {
            pixel_format_ctor_format(block, 6);
            block
        }
        9 => {
            pixel_format_ctor_format(block, 7);
            block
        }
        10 => {
            pixel_format_ctor_format(block, 8);
            block
        }
        11 => pixel_format_ctor_bump_luma_copy(block, 0xFF, 0xFF00, 0, 16),
        12 => pixel_format_ctor_bump_luma_copy(block, 0x1F, 0x3E0, 0xFC00, 16),
        13 => pixel_format_ctor_bump_luma_copy(block, 0xFF, 0xFF00, 0xFF_0000, 32),
        // 2 and 14
        _ => pixel_format_ctor_rgba_copy(block, 0xFF_0000, 0xFF00, 0xFF, 0, 32),
    })
}

/// 0x6B05E0 `NiDX8Renderer::probeTextureFormats`: one `NiDX8PixelFormat` in `formats` for
/// each of the 14 texture formats the device has for `resource_type` (3 = texture, 5 = cube
/// texture) with the format of the back buffer; 0 for the others. The game code does not
/// read the second argument.
pub unsafe fn probe_texture_formats(this: usize, formats: usize, _unused: u32, resource_type: u32) {
    crate::shadow::touch(&probe_texture_formats_port::STATE);
    let backbuffer_format: usize = rd(this, R::backbufferRenderTarget_DX8PixelFormat);
    let adapter_format: u32 = rd(backbuffer_format, PF::d3dFormat);
    let mut index = 0u32;
    loop {
        if let Some(pixel_format) = probe_pixel_format(index) {
            let mut dx8_format = operator_new(PF::SIZE);
            if dx8_format != 0 {
                // Slots 0, 1, 3, 4, 5 and 8 call the copy function; the others have the
                // eight words inline.
                if matches!(index, 0 | 1 | 3 | 4 | 5 | 8) {
                    pixel_format_copy(dx8_format, pixel_format);
                } else {
                    for word in 0..8 {
                        wr::<u32>(dx8_format, word * 4, rd(pixel_format, word * 4));
                    }
                }
                wr::<u32>(dx8_format, PF::extraData, 0);
                let d3d_format = d3d_format_for_pixel_format(pixel_format);
                wr(dx8_format, PF::d3dFormat, d3d_format);
            }
            // With no block the game code writes to address 0x20, as this does.
            wr(dx8_format, PF::extraData, index);

            if pixel_format != 0 && dx8_format != 0 {
                let old: usize = rd(formats, index as usize * 4);
                if old != 0 {
                    operator_delete(old);
                }
                let d3d: usize = rd(DIRECT3D, 0);
                let result = d3d_check_device_format(
                    d3d,
                    rd(this, R::currentAdapterIndex),
                    rd(this, R::d3dDeviceType),
                    adapter_format,
                    0,
                    resource_type,
                    rd(dx8_format, PF::d3dFormat),
                );
                if result != 0 {
                    operator_delete(dx8_format);
                    dx8_format = 0;
                }
                wr(formats, index as usize * 4, dx8_format);
                operator_delete(pixel_format);
            }
        }
        index += 1;
        if index >= FORMATS_PER_GROUP as u32 {
            break;
        }
    }
}

/// The formats are in the renderer; the rest is memory from `operator new` and
/// `operator delete`, and `CheckDeviceFormat` only asks.
unsafe fn probe_texture_formats_regions(set: &mut Regions, this: usize, formats: usize) {
    set.allocates();
    set.add(this, R::SIZE);
    if formats < this || formats + FORMATS_PER_GROUP * 4 > this + R::SIZE {
        set.add(formats, FORMATS_PER_GROUP * 4);
    }
}

/// The order in which `createDefaultTextures` looks for a format of a group.
const DEFAULT_FORMAT_ORDER: [usize; 6] = [1, 0, 2, 4, 3, 5];

/// The first format of group `group` in the order of `DEFAULT_FORMAT_ORDER`, or 0.
unsafe fn default_texture_format(this: usize, group: usize) -> usize {
    for slot in DEFAULT_FORMAT_ORDER {
        let format: usize = rd(this, R::arrayArrayTextureFormats + (group * FORMATS_PER_GROUP + slot) * 4);
        if format != 0 {
            return format;
        }
    }
    0
}

/// 0x6ABA70 `NiDX8Renderer::createDefaultTextures`: for each of the four groups the
/// default format and a white 4 x 4 `NiPixelData` in it. Returns whether the first group
/// has a format.
pub unsafe fn create_default_textures(this: usize) -> u8 {
    crate::shadow::touch(&create_default_textures_port::STATE);
    for group in 0..FORMAT_GROUPS {
        let format_slot = this + R::defaultTextureFormats + group * 4;
        let slot = this + R::defaultTexturePixelData + group * 4;
        wr::<u32>(format_slot, 0, 0);
        let format = default_texture_format(this, group);
        if format != 0 {
            wr(format_slot, 0, format);
        }
        if rd::<usize>(format_slot, 0) != 0 {
            let block = operator_new(NiPixelData::SIZE);
            let pixel_data = if block != 0 { pixel_data_ctor(block, 4, 4, rd(format_slot, 0), 1) } else { 0 };
            let old: usize = rd(slot, 0);
            if old != pixel_data {
                if old != 0 {
                    release_reference(old);
                }
                wr(slot, 0, pixel_data);
                if pixel_data != 0 {
                    wr(pixel_data, NiRefObject::refCount, rd::<u32>(pixel_data, NiRefObject::refCount).wrapping_add(1));
                }
            }
            // The pixels of the first mip map level: all bits set.
            let pixel_data: usize = rd(slot, 0);
            let offset: usize = rd(rd(pixel_data, NiPixelData::mipmapOffsets), 0);
            let pixels = offset.wrapping_add(rd(pixel_data, NiPixelData::pixelsAllMips));
            let size = (rd::<u32>(pixel_data, NiPixelData::bytesPerPixel) << 4) as usize;
            core::ptr::write_bytes(pixels as *mut u8, 0xFF, size);
        }
    }
    (rd::<u32>(this, R::defaultTextureFormats) != 0) as u8
}

/// 0x6AB8C0 `NiDX8Renderer::initializeTextureDefaults`: the texture formats of the device,
/// the default textures and the depth format for each format of the second group.
pub unsafe fn initialize_texture_defaults(this: usize) -> u8 {
    crate::shadow::touch(&initialize_texture_defaults_port::STATE);
    let formats = this + R::arrayArrayTextureFormats;
    probe_texture_formats(this, formats, 0, 3);
    probe_texture_formats(this, formats + FORMATS_PER_GROUP * 4, 1, 3);
    probe_texture_formats(this, formats + FORMATS_PER_GROUP * 8, 0, 5);
    probe_texture_formats(this, formats + FORMATS_PER_GROUP * 12, 1, 5);
    create_default_textures(this);
    for index in 0..FORMATS_PER_GROUP {
        // `depthStencilFormats - 0xC8`: the second group of texture formats.
        let format: usize = rd(this, R::depthStencilFormats - 0xC8 + index * 4);
        let depth_stencil = if format != 0 {
            device_desc_find_depth_stencil(
                rd(this, R::maybe_currentDeviceDesc),
                rd(this, R::d3dPresentParameters_BackBufferFormat),
                rd(format, PF::d3dFormat),
                0x20,
                8,
            )
        } else {
            0
        };
        wr(this, R::depthStencilFormats + index * 4, depth_stencil);
    }
    1
}

// ---------------------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------------------

/// The device type and the vertex processing of a device choice of `Create` (0 to 4).
unsafe fn select_device(this: usize, choice: u32) {
    let (device_type, behavior) = match choice {
        0 => (D3DDEVTYPE_HAL, 0x50), // pure, hardware vertex processing
        1 => (D3DDEVTYPE_HAL, 0x40), // hardware vertex processing
        2 => (D3DDEVTYPE_HAL, 0x80), // mixed
        3 => (D3DDEVTYPE_HAL, 0x20), // software
        _ => (D3DDEVTYPE_REF, 0x20),
    };
    wr::<u32>(this, R::d3dDeviceType, device_type);
    wr::<u32>(this, R::d3dBehaviorFlags, behavior);
}

/// The members of the `NiDX8PixelFormat` that `Create` makes for the back buffer.
unsafe fn init_backbuffer_pixel_format(format: usize, d3d_format: u32) {
    wr::<u32>(format, PF::bitsPerPixel, 8);
    wr::<u32>(format, PF::channelMasks + 12, 0);
    wr::<u32>(format, PF::channelMasks + 8, 0);
    wr::<u32>(format, PF::channelMasks + 4, 0);
    wr::<u32>(format, PF::channelMasks, 0);
    wr::<u32>(format, PF::format, 2);
    let a = 2u32.wrapping_shl(rd(PIXEL_FORMAT_SHIFT_A, 0)) & rd::<u32>(PIXEL_FORMAT_MASK_A, 0);
    let b = 1u32.wrapping_shl(rd(PIXEL_FORMAT_SHIFT_B, 0)) & rd::<u32>(PIXEL_FORMAT_MASK_B, 0);
    wr::<u32>(format, PF::compareBits + 4, 0);
    wr::<u32>(format, PF::compareBits, a | b);
    wr::<u32>(format, PF::d3dFormat, d3d_format);
    wr::<u32>(format, PF::extraData, 0);

    // The comparisons with 41 and with "DXT1" are signed.
    let signed = d3d_format as i32;
    if signed <= D3DFMT_P8 {
        match d3d_format {
            41 => pixel_format_ctor_palette(format, 0),
            20 | 22 => pixel_format_ctor_rgba(format, 0xFF_0000, 0xFF00, 0xFF, 0, 32),
            23 => pixel_format_ctor_rgba(format, 0xF800, 0x7E0, 0x1F, 0, 16),
            24 => pixel_format_ctor_rgba(format, 0x7C00, 0x3E0, 0x1F, 0, 16),
            25 => pixel_format_ctor_rgba(format, 0x7C00, 0x3E0, 0x1F, 0x8000, 16),
            26 => pixel_format_ctor_rgba(format, 0xF00, 0xF0, 0xF, 0xF000, 16),
            _ => pixel_format_ctor_rgba(format, 0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000, 32),
        };
    } else if signed <= FOURCC_DXT1 as i32 {
        match d3d_format {
            FOURCC_DXT1 => pixel_format_ctor_format(format, 6),
            60 => pixel_format_ctor_bump_luma(format, 0xFF, 0xFF00, 0, 16),
            61 => pixel_format_ctor_bump_luma(format, 0x1F, 0x3E0, 0xFC00, 16),
            62 => pixel_format_ctor_bump_luma(format, 0xFF, 0xFF00, 0xFF_0000, 32),
            _ => pixel_format_ctor_rgba(format, 0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000, 32),
        };
    } else {
        match d3d_format {
            FOURCC_DXT3 => pixel_format_ctor_format(format, 7),
            FOURCC_DXT5 => pixel_format_ctor_format(format, 8),
            _ => pixel_format_ctor_rgba(format, 0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000, 32),
        };
    }
}

/// The end of the paths of `Create` that fail: deletes the renderer and returns 0.
unsafe fn abandon(this: usize) -> usize {
    delete_object(this);
    0
}

/// One try of `Create` with the device choice the renderer has. `backbuffer_count` is the
/// register the game code keeps the count in: after a `CreateDevice` it has the count of
/// the present parameters from before the call. True when the device exists.
#[allow(clippy::too_many_arguments)]
unsafe fn try_create_device(
    this: usize,
    width: u32,
    height: u32,
    flags: u32,
    samples: u32,
    backbuffer_format: u32,
    depth_stencil_format: u32,
    backbuffer_count: &mut u32,
    swap_effect: u32,
    refresh_rate: u32,
    presentation_interval: u32,
) -> bool {
    let device_desc = selected_device_desc(this);
    if rd::<u32>(device_desc, DeviceDesc::caps) == 0 || device_desc == 0 {
        creation_failed(ERR_DEVICE_NOT_VALID, 0x7C2F78);
        return false;
    }
    let ok = initialize_present_params(
        this,
        width,
        height,
        flags,
        samples,
        backbuffer_format,
        depth_stencil_format,
        *backbuffer_count,
        swap_effect,
        refresh_rate,
        presentation_interval,
    );
    if ok == 0 {
        return false;
    }
    let behavior = rd::<u32>(this, R::d3dBehaviorFlags) | D3DCREATE_MULTITHREADED;
    *backbuffer_count = rd(this, R::d3dPresentParameters_BackBufferCount);
    wr(this, R::d3dBehaviorFlags, behavior);
    let d3d: usize = rd(DIRECT3D, 0);
    let result = d3d_create_device(
        d3d,
        rd(this, R::currentAdapterIndex),
        rd(this, R::d3dDeviceType),
        rd(this, R::focusWindow),
        behavior,
        this + R::d3dPresentParameters,
        this + R::d3dDevice,
    );
    if result == 0 {
        return true;
    }
    if *backbuffer_count == rd::<u32>(this, R::d3dPresentParameters_BackBufferCount) {
        return false;
    }
    // The device changed the number of back buffers: once more with that number.
    debug_print(0x7C2F18);
    let behavior = rd::<u32>(this, R::d3dBehaviorFlags) | D3DCREATE_MULTITHREADED;
    wr(this, R::d3dBehaviorFlags, behavior);
    let d3d: usize = rd(DIRECT3D, 0);
    let result = d3d_create_device(
        d3d,
        rd(this, R::currentAdapterIndex),
        rd(this, R::d3dDeviceType),
        rd(this, R::focusWindow),
        behavior,
        this + R::d3dPresentParameters,
        this + R::d3dDevice,
    );
    result == 0
}

/// The back buffer and the depth buffer of the device into `backbufferRenderTarget`, as
/// `Create` and `Recreate` do after the device exists. `make_format` makes the
/// `NiDX8PixelFormat` in a new block. `log` has the two debug texts.
///
/// When `GetDesc` fails the game code takes the width and the height from stack memory it
/// did not fill; here they are 0.
unsafe fn read_back_buffer(this: usize, make_format: unsafe fn(usize, u32), log: [usize; 2]) {
    let target = this + R::backbufferRenderTarget;
    for word in 0..RT::SIZE / 4 {
        wr::<u32>(target, word * 4, 0);
    }
    let surface_slot = this + R::backbufferRenderTarget_d3dSurfaceBackBuffer;
    device_get_render_target(rd(this, R::d3dDevice), surface_slot);
    let surface: usize = rd(surface_slot, 0);
    let mut desc = [0u32; 8];
    if surface_get_desc(surface, desc.as_mut_ptr() as usize) == 0 {
        let format = operator_new(PF::SIZE);
        if format != 0 {
            make_format(format, desc[SURFACE_DESC_FORMAT]);
        }
        wr(this, R::backbufferRenderTarget_DX8PixelFormat, format);
        debug_print1(log[0], d3dformat_to_string(desc[SURFACE_DESC_FORMAT]));
    }
    device_get_depth_stencil_surface(rd(this, R::d3dDevice), this + R::backbufferRenderTarget_d3dSurfaceDepthStencil);
    debug_print1(log[1], d3dformat_to_string(rd(this, R::d3dPresentParameters_AutoDepthStencilFormat)));
    wr(target, RT::width, desc[SURFACE_DESC_WIDTH]);
    wr(this, R::backbufferRenderTarget_height, desc[SURFACE_DESC_HEIGHT]);
    wr(this, R::currentRenderTarget, target);
}

/// The pixel format of the back buffer as `Recreate` makes it.
unsafe fn make_format_with_setter(format: usize, d3d_format: u32) {
    dx8_pixel_format_ctor(format);
    dx8_pixel_format_set(format, d3d_format);
}

/// 0x6AA3F0 `NiDX8Renderer::Create` (cdecl). Returns the renderer or 0.
///
/// `device_caps` is the first device choice (0 = pure hardware, 1 = hardware, 2 = mixed,
/// 3 = software vertex processing, 4 = reference device). A choice that fails goes to the
/// next one up to 3.
#[allow(clippy::too_many_arguments)]
pub unsafe fn create(
    width: u32,
    height: u32,
    flags: u32,
    window_device: u32,
    window_focus: u32,
    adapter: u32,
    device_caps: u32,
    backbuffer_format: u32,
    depth_stencil_format: u32,
    presentation_interval: u32,
    swap_effect: u32,
    samples: u32,
    backbuffer_count: u32,
    refresh_rate: u32,
) -> usize {
    crate::shadow::touch(&create_port::STATE);
    if rd::<usize>(DIRECT3D, 0) == 0 {
        if !load_direct3d() {
            debug_print(LOG_D3D_FAILED);
            creation_failed(ERR_NO_DIRECT3D, LOG_INVALID_DEVICE_ID);
            return 0;
        }
        debug_print(LOG_D3D_OK);
    }
    let block = operator_new(R::SIZE);
    let this = if block != 0 { renderer_ctor(block) } else { 0 };
    let block = operator_new(SystemDesc::SIZE);
    let system = if block != 0 { system_desc_ctor(block, rd(DIRECT3D, 0)) } else { 0 };
    wr(this, R::systemDesc, system);
    wr(this, R::windowDevice, window_device);
    wr(this, R::currentAdapterIndex, adapter);
    wr(this, R::focusWindow, window_focus);

    let mut choice = device_caps;
    if choice > 4 {
        creation_failed(ERR_INVALID_DEVICE_TYPE, LOG_INVALID_DEVICE_ID);
        return abandon(this);
    }
    select_device(this, choice);

    let adapter_desc: usize = rd(rd(rd::<usize>(this, R::systemDesc), SystemDesc::adapterList_storage), rd::<usize>(this, R::currentAdapterIndex) * 4);
    wr(this, R::maybe_currentAdapter, adapter_desc);
    if adapter_desc == 0 {
        creation_failed(ERR_INVALID_ADAPTER, 0x7C3044);
        return abandon(this);
    }
    let device_desc: usize = if rd::<u32>(this, R::d3dDeviceType) == D3DDEVTYPE_HAL {
        rd(adapter_desc, Adapter::HALDeviceDesc)
    } else {
        rd(adapter_desc, Adapter::REFDeviceDesc)
    };
    let device_desc = if rd::<u32>(device_desc, DeviceDesc::caps) != 0 { device_desc } else { 0 };
    wr(this, R::maybe_currentDeviceDesc, device_desc);
    if device_desc == 0 {
        creation_failed(ERR_INVALID_DEVICE_TYPE, LOG_INVALID_DEVICE_ID);
        return abandon(this);
    }

    let mut count = backbuffer_count;
    loop {
        debug_print(0x7C2FDC);
        let created = try_create_device(
            this,
            width,
            height,
            flags,
            samples,
            backbuffer_format,
            depth_stencil_format,
            &mut count,
            swap_effect,
            refresh_rate,
            presentation_interval,
        );
        if created {
            break;
        }
        if choice == 4 {
            creation_failed(ERR_NO_REFERENCE_DEVICE, 0x7C2E30);
            return abandon(this);
        }
        choice += 1;
        if choice == 4 {
            // The reference device is never a fallback.
            debug_print(0x7C2DE0);
            return abandon(this);
        }
        debug_print(0x7C2EB8);
        if choice > 4 {
            creation_failed(ERR_INVALID_DEVICE_TYPE, LOG_INVALID_DEVICE_ID);
            return abandon(this);
        }
        select_device(this, choice);
    }

    if rd::<usize>(this, R::d3dDevice) == 0 {
        creation_failed(ERR_NO_HARDWARE_DEVICE, 0x7C2D60);
        return abandon(this);
    }
    read_back_buffer(this, init_backbuffer_pixel_format, [0x7C2D28, 0x7C2CF0]);

    if validate_device_caps(this) == 0 {
        return abandon(this);
    }
    let device_desc = selected_device_desc(this);
    if rd::<u32>(device_desc, DeviceDesc::caps) == 0 || device_desc == 0 {
        creation_failed(ERR_NO_DEVICE_CAPS, 0x7C2C80);
        return abandon(this);
    }
    let caps = device_desc + DeviceDesc::caps;

    let block = operator_new(SIZE_VERTEX_BUFFER_MANAGER);
    let manager = if block != 0 { vertex_buffer_manager_ctor(block, rd(this, R::d3dDevice)) } else { 0 };
    wr(this, R::vertexBufferManager, manager);
    let block = operator_new(SIZE_INDEX_BUFFER_MANAGER);
    let manager = if block != 0 { index_buffer_manager_ctor(block, rd(this, R::d3dDevice)) } else { 0 };
    wr(this, R::indexBufferManager, manager);
    let block = operator_new(SIZE_TEXTURE_MANAGER);
    let manager = if block != 0 { texture_manager_ctor(block, this) } else { 0 };
    wr(this, R::textureManager, manager);

    // Bit 7 of the capability flags: the depth buffer has stencil. The game code pushes
    // the whole shifted word with its low byte cut to that bit.
    let shifted = rd::<u32>(this, R::capabilityFlags) >> 7;
    let stencil = (shifted & 0xFFFF_FF00) | (shifted & 1);
    let state = render_state_create(rd(this, R::d3dDevice), caps, 1, 1, stencil);
    wr(this, R::renderState, state);

    // left, right, top, bottom, near, far
    let frustum: [u32; 6] = [0xBF80_0000, 0x3F80_0000, 0x3F80_0000, 0xBF80_0000, 0x3DCC_CCCD, 0x3F80_0000];
    // left, right, top, bottom
    let port: [u32; 4] = [0, 0x3F80_0000, 0x3F80_0000, 0];
    let set_camera_data: unsafe extern "thiscall" fn(usize, usize, usize, usize, usize, usize, usize) -> u32 = transmute(virtual_at(this, 0xB8));
    set_camera_data(this, VEC_ZERO, VEC_X_AXIS, VEC_Y_AXIS, VEC_Z_AXIS, frustum.as_ptr() as usize, port.as_ptr() as usize);

    let block = operator_new(SIZE_LIGHT_MANAGER);
    let manager = if block != 0 { light_manager::ctor(block, rd(this, R::renderState), rd(this, R::d3dDevice)) } else { 0 };
    wr(this, R::lightManager, manager);

    if initialize_texture_defaults(this) == 0 {
        debug_print(0x7C2C38);
        return abandon(this);
    }
    let pipeline = texture_pipeline::create(this, caps, rd(this, R::renderState), rd(this, R::textureManager), 1);
    wr(this, R::configurableTexturePipeline, pipeline);
    if rd::<u32>(this, R::d3dPresentParameters_MultiSampleType) != 0 {
        render_state::set_multisampling(rd(this, R::renderState), 1);
    }
    this
}

// ---------------------------------------------------------------------------------------
// Recreate
// ---------------------------------------------------------------------------------------

/// Takes `data` out of the list of renderer data of `texture`.
unsafe fn unlink_renderer_data(texture: usize, data: usize) {
    let mut current: usize = rd(texture, TEXTURE_RENDERER_DATA);
    if current == 0 {
        return;
    }
    let mut next: usize = rd(current, RENDERER_DATA_NEXT);
    if current == data {
        wr(texture, TEXTURE_RENDERER_DATA, next);
        return;
    }
    while next != 0 {
        if next == data {
            wr::<u32>(current, RENDERER_DATA_NEXT, rd(next, RENDERER_DATA_NEXT));
            return;
        }
        current = next;
        next = rd(next, RENDERER_DATA_NEXT);
    }
}

/// The first half of `Recreate` for one map of rendered textures: deletes the renderer
/// data of every texture and leaves the texture in the map with the value 0.
///
/// In the map of cube maps the game code gives the position of the next node as the key
/// when it fills the node again (0x6AB1CD), not the texture. `key_is_next_position` has
/// that.
unsafe fn release_rendered_textures(map: usize, key_is_next_position: bool) {
    let mut position = hash_map_first(map);
    while position != 0 {
        let texture: usize = rd(position, Node::key);
        let data: usize = rd(position, Node::data);
        let next: usize = rd(position, Node::nextNode);
        position = if next != 0 { next } else { hash_map_first_after(map, texture) };

        unlink_renderer_data(texture, data);
        if data != 0 {
            delete_object(data);
        }

        let bucket = thiscall1(virtual_at(map, 4), map, texture as u32) as usize;
        let mut node: usize = rd(rd(map, Map::arrayBuckets), bucket * 4);
        let mut found = false;
        while node != 0 {
            if thiscall2(virtual_at(map, 8), map, texture as u32, rd(node, Node::key)) as u8 != 0 {
                thiscall1(virtual_at(map, 0x10), map, node as u32);
                let key = if key_is_next_position { position } else { texture };
                thiscall3(virtual_at(map, 0x0C), map, node as u32, key as u32, 0);
                found = true;
                break;
            }
            node = rd(node, Node::nextNode);
        }
        if !found {
            let node = operator_new(Node::SIZE);
            thiscall3(virtual_at(map, 0x0C), map, node as u32, texture as u32, 0);
            wr::<u32>(node, Node::nextNode, rd(rd(map, Map::arrayBuckets), bucket * 4));
            wr(rd::<usize>(map, Map::arrayBuckets), bucket * 4, node);
            wr(map, Map::count, rd::<u32>(map, Map::count).wrapping_add(1));
        }
    }
}

/// 0x6AAE80 `NiDX8Renderer::Recreate`: resets a lost device. Everything that lives in the
/// default pool goes first and comes back after `Reset`. Returns 0 when `Reset` fails or
/// the texture formats cannot be made.
pub unsafe fn recreate(this: usize) -> u8 {
    crate::shadow::touch(&recreate_port::STATE);
    light_manager::release_lights(rd(this, R::lightManager));
    index_buffer_manager_recreate(rd(this, R::indexBufferManager));
    vertex_buffer_manager_recreate(rd(this, R::vertexBufferManager));
    texture_manager_recreate(rd(this, R::textureManager));
    let surface: usize = rd(this, R::backbufferRenderTarget_d3dSurfaceBackBuffer);
    if surface != 0 {
        com_release(surface);
        wr::<u32>(this, R::backbufferRenderTarget_d3dSurfaceBackBuffer, 0);
    }
    let surface: usize = rd(this, R::backbufferRenderTarget_d3dSurfaceDepthStencil);
    if surface != 0 {
        com_release(surface);
        wr::<u32>(this, R::backbufferRenderTarget_d3dSurfaceDepthStencil, 0);
    }
    let textures = this + R::hashmap_maybeRenderedTextures;
    let cube_maps = this + R::hashmap_maybeRenderedCubemaps;
    release_rendered_textures(textures, false);
    release_rendered_textures(cube_maps, true);

    let format: usize = rd(this, R::backbufferRenderTarget_DX8PixelFormat);
    if format != 0 {
        operator_delete(format);
    }
    if (device_reset(rd(this, R::d3dDevice), this + R::d3dPresentParameters) as i32) < 0 {
        return 0;
    }
    read_back_buffer(this, make_format_with_setter, [0x7C319C, 0x7C3164]);
    if initialize_texture_defaults(this) == 0 {
        debug_print(0x7C3118);
        return 0;
    }

    // New renderer data for every rendered texture: virtual functions at +0xF8 and +0xFC
    // of the renderer.
    let mut position = hash_map_first(textures);
    while position != 0 {
        let texture: usize = rd(position, Node::key);
        let next: usize = rd(position, Node::nextNode);
        position = if next != 0 { next } else { hash_map_first_after(textures, texture) };
        thiscall1(virtual_at(this, 0xF8), this, texture as u32);
    }
    let mut position = [hash_map_first(cube_maps)];
    while position[0] != 0 {
        let mut texture = [0usize];
        let mut value = [0usize];
        hash_map_get_next(cube_maps, position.as_mut_ptr() as usize, texture.as_mut_ptr() as usize, value.as_mut_ptr() as usize);
        thiscall1(virtual_at(this, 0xFC), this, texture[0] as u32);
    }
    render_state_initialize(rd(this, R::renderState));
    texture_pipeline::init_pipeline_state(rd(this, R::configurableTexturePipeline));
    light_manager::reset_render_states(rd(this, R::lightManager));
    1
}

// ---------------------------------------------------------------------------------------
// Destruction
// ---------------------------------------------------------------------------------------

/// Deletes the manager in a member of the renderer and sets the member to 0.
unsafe fn delete_member(this: usize, offset: usize) {
    let object: usize = rd(this, offset);
    if object != 0 {
        delete_object(object);
        wr::<u32>(this, offset, 0);
    }
}

/// 0x6A9E10 `NiDX8Renderer::dtor`
pub unsafe fn dtor(this: usize) {
    crate::shadow::touch(&dtor_port::STATE);
    wr(this, R::vtbl, VTABLE_RENDERER);
    if rd::<usize>(this, R::d3dDevice) != 0 {
        // Nothing stays bound to the device.
        let mut stage = 0u32;
        if rd::<u32>(TEXTURE_STAGE_COUNT, 0) > 0 {
            loop {
                let state: usize = rd(this, R::renderState);
                let slot = RS::texturePerStage + stage as usize * 4;
                if rd::<u32>(state, slot) != 0 {
                    wr::<u32>(state, slot, 0);
                    device_set_texture(rd(state, RS::device), stage, 0);
                }
                stage += 1;
                if stage >= rd::<u32>(TEXTURE_STAGE_COUNT, 0) {
                    break;
                }
            }
        }
        let mut stream = 0u32;
        if rd::<u32>(STREAM_COUNT, 0) > 0 {
            loop {
                device_set_stream_source(rd(this, R::d3dDevice), stream, 0, 0);
                stream += 1;
                if stream >= rd::<u32>(STREAM_COUNT, 0) {
                    break;
                }
            }
        }
        device_set_indices(rd(this, R::d3dDevice), 0, 0);
    }

    for index in 0..FORMAT_GROUPS * FORMATS_PER_GROUP {
        let format: usize = rd(this, R::arrayArrayTextureFormats + index * 4);
        if format != 0 {
            operator_delete(format);
        }
    }

    let property_state: usize = rd(this, R::currentPropertyState);
    if property_state != 0 {
        release_reference(property_state);
        wr::<u32>(this, R::currentPropertyState, 0);
    }

    let mut batched: usize = rd(this, R::batchedObjectListHead);
    while batched != 0 {
        let next: usize = rd(batched, Batched::nextBatch);
        operator_delete(batched);
        batched = next;
    }
    wr::<u32>(this, R::batchedObjectListHead, 0);
    wr::<u32>(this, R::batchedObjectListTail, 0);

    // The geometry buffers of the shapes.
    let geom_buffers = this + R::hashGeomBuffers;
    let mut node = hash_map_first(geom_buffers);
    while node != 0 {
        let next: usize = rd(node, Node::nextNode);
        let buffer: usize = rd(node, Node::data);
        node = if next != 0 { next } else { hash_map_first_after(geom_buffers, rd(node, Node::key)) };
        if buffer != 0 {
            free_geom_buffer(buffer);
        }
    }

    // The geometry buffers of the skin partitions; a partition with one leaves the map.
    let skin_partitions = this + R::hashmap_skinPartitionsToGeomBuffers;
    let mut node = hash_map_first(skin_partitions);
    while node != 0 {
        let partition: usize = rd(node, Node::key);
        node = rd(node, Node::nextNode);
        if node == 0 {
            node = hash_map_first_after(skin_partitions, partition);
        }
        let buffer: usize = rd(partition, PARTITION_GEOM_BUFFER);
        if buffer != 0 {
            free_geom_buffer(buffer);
            wr::<u32>(partition, PARTITION_GEOM_BUFFER, 0);
            hash_map_remove(skin_partitions, partition);
        }
    }

    delete_member(this, R::configurableTexturePipeline);

    let textures = this + R::hashmap_maybeRenderedTextures;
    let cube_maps = this + R::hashmap_maybeRenderedCubemaps;
    hash_map_drain(textures);
    hash_map_drain(cube_maps);

    let surface: usize = rd(this, R::backbufferRenderTarget_d3dSurfaceBackBuffer);
    if surface != 0 {
        com_release(surface);
        wr::<u32>(this, R::backbufferRenderTarget_d3dSurfaceBackBuffer, 0);
    }
    let surface: usize = rd(this, R::backbufferRenderTarget_d3dSurfaceDepthStencil);
    if surface != 0 {
        com_release(surface);
        wr::<u32>(this, R::backbufferRenderTarget_d3dSurfaceDepthStencil, 0);
    }
    let format: usize = rd(this, R::backbufferRenderTarget_DX8PixelFormat);
    if format != 0 {
        operator_delete(format);
    }

    delete_member(this, R::vertexBufferManager);
    delete_member(this, R::indexBufferManager);
    clear_texture_data(this);
    clear_listed_renderer_data(this);
    delete_member(this, R::renderState);
    delete_member(this, R::textureManager);
    delete_member(this, R::lightManager);

    let device: usize = rd(this, R::d3dDevice);
    if device != 0 {
        com_release(device);
        wr::<u32>(this, R::d3dDevice, 0);
    }
    let system: usize = rd(this, R::systemDesc);
    if system != 0 {
        system_desc_dtor(system);
        operator_delete(system);
    }
    let module = get_module_handle_a(STR_D3D8_DLL);
    if module != 0 {
        free_library(module);
    }
    wr::<u32>(DIRECT3D_CREATE8, 0, 0);

    // The destructors of the members, last member first.
    wr(cube_maps, Map::vtbl, VTABLE_MAP_CUBE_MAPS);
    clear_hash_map(cube_maps);
    operator_delete(rd(cube_maps, Map::arrayBuckets));

    wr(textures, Map::vtbl, VTABLE_MAP_TEXTURES);
    hash_map_drain(textures);
    operator_delete(rd(textures, Map::arrayBuckets));

    wr(skin_partitions, Map::vtbl, VTABLE_MAP_SKIN_PARTITIONS);
    clear_hash_map(skin_partitions);
    operator_delete(rd(skin_partitions, Map::arrayBuckets));

    wr(geom_buffers, Map::vtbl, VTABLE_MAP_GEOM_BUFFERS);
    hash_map_drain(geom_buffers);
    operator_delete(rd(geom_buffers, Map::arrayBuckets));

    let object: usize = rd(this, R::backbufferRenderTarget_field_18);
    if object != 0 {
        ni_object_release(object);
    }
    let object: usize = rd(this, R::backbufferRenderTarget_field_C);
    if object != 0 {
        ni_object_release(object);
    }
    vector_destructor(this + R::defaultTexturePixelData, 4, FORMAT_GROUPS as u32, NI_POINTER_DTOR);
    bound_dtor(this + R::defaultBound);
    bound_dtor(this + R::worldBound);
    let object: usize = rd(this, R::effectStatePtr);
    if object != 0 {
        release_reference(object);
    }
    let object: usize = rd(this, R::propertyStatePtr);
    if object != 0 {
        release_reference(object);
    }
    ni_renderer_dtor(this);
}

/// 0x6A9D80 `NiDX8Renderer::deleting_dtor`. Bit 0 of `flags`: free the memory.
pub unsafe fn deleting_dtor(this: usize, flags: u8) -> usize {
    crate::shadow::touch(&deleting_dtor_port::STATE);
    dtor(this);
    if flags & 1 != 0 {
        operator_delete(this);
    }
    this
}

// ---------------------------------------------------------------------------------------
// videoAdapterEnum
// ---------------------------------------------------------------------------------------

/// 0x429E20 `NiDX8Renderer_static::videoAdapterEnum` (cdecl): fills the map of `TES3Game`
/// from the description of an adapter to its `NiDX8AdapterDesc`.
///
/// The system description is deleted at the end, so the map has pointers into freed
/// memory, as in the game.
pub unsafe fn video_adapter_enum(game: usize) {
    crate::shadow::touch(&video_adapter_enum_port::STATE);
    let map = game + GAME_ADAPTER_MAP;
    // erase(begin, end)
    let head: usize = rd(map, 4);
    let mut erased = [0usize];
    adapter_map_erase(map, erased.as_mut_ptr() as usize, rd(head, 0), head);

    let system = get_adapters();
    if system == 0 {
        return;
    }
    let mut index = 0u32;
    if rd::<u32>(system, SystemDesc::d3dAdapterCount) > 0 {
        loop {
            let adapter: usize = rd(rd(system, SystemDesc::adapterList_storage), index as usize * 4);
            if adapter != 0 {
                // std::string of Visual C++ 6: allocator, pointer, length, reserved. The
                // game code puts the low byte of the index where the allocator is.
                let mut name = [index & 0xFF, 0, 0, 0];
                let name = name.as_mut_ptr() as usize;
                let description = adapter + Adapter::d3dAdapterIdentifier_description;
                std_string_assign(name, description, c_string_length(description) as u32);
                // The pair for the map: the name and the value 0.
                let mut pair = [0u32; 5];
                let pair = pair.as_mut_ptr() as usize;
                std_string_copy_ctor(pair, name);
                wr::<u32>(pair, 16, 0);
                // The result of insert: the node and "inserted".
                let mut result = [0usize; 2];
                let result = result.as_mut_ptr() as usize;
                adapter_map_insert(map, result, pair);
                let mut copy = [0usize; 2];
                insert_result_copy(copy.as_mut_ptr() as usize, result, result + 4);
                let node = copy[0];
                std_string_tidy(pair, 1);
                wr(node, STD_MAP_NODE_VALUE, adapter);
                // The destructor of the name: the byte before the text counts the users.
                let text: usize = rd(name, 4);
                if text != 0 {
                    let users: u8 = rd(text - 1, 0);
                    if users == 0 || users == 0xFF {
                        operator_delete(text - 1);
                    } else {
                        wr::<u8>(text - 1, 0, users - 1);
                    }
                }
            }
            index += 1;
            if index >= rd::<u32>(system, SystemDesc::d3dAdapterCount) {
                break;
            }
        }
    }
    system_desc_dtor(system);
    operator_delete(system);
}

// ---------------------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------------------

// Loads D3D8.DLL and makes the IDirect3D8.
port_cdecl!(create_d3d_port, 0x6A99E0, "NiDX8Renderer::CreateD3D",
    fn() -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = create_d3d);

// Releases the IDirect3D8.
port_cdecl!(release_d3d_port, 0x6A9A50, "NiDX8Renderer::ReleaseD3D",
    fn() -> u32,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = release_d3d);

port_thiscall!(get_rtti_port, 0x6A9C50, "NiDX8Renderer::GetRTTI",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| (),
    rust = get_rtti);

// Destroys the device and the renderer.
port_thiscall!(deleting_dtor_port, 0x6A9D80, "NiDX8Renderer::deleting_dtor",
    fn(this, flags: u8) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = deleting_dtor);

// Destroys the device, frees D3D8.DLL.
port_thiscall!(dtor_port, 0x6A9E10, "NiDX8Renderer::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = dtor);

// Creates the device.
port_cdecl!(create_port, 0x6AA3F0, "NiDX8Renderer::Create",
    fn(width: u32, height: u32, flags: u32, window_device: u32, window_focus: u32, adapter: u32, device_caps: u32,
       backbuffer_format: u32, depth_stencil_format: u32, presentation_interval: u32, swap_effect: u32, samples: u32,
       backbuffer_count: u32, refresh_rate: u32) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = create);

// Resets the device.
port_thiscall!(recreate_port, 0x6AAE80, "NiDX8Renderer::Recreate",
    fn(this) -> u8,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = recreate);

port_thiscall!(initialize_present_params_port, 0x6AB470, "NiDX8Renderer::InitializePresentParams",
    fn(this, width: u32, height: u32, flags: u32, samples: u32, backbuffer_format: u32, depth_stencil_format: u32,
       backbuffer_count: u32, swap_effect: u32, refresh_rate: u32, presentation_interval: u32) -> u8,
    compare_eax = true,
    regions = |set| initialize_present_params_regions(set, this),
    rust = initialize_present_params);

// The default textures are engine objects with a constructor that cannot run twice, and
// an old one can be deleted through its virtual destructor.
port_thiscall!(initialize_texture_defaults_port, 0x6AB8C0, "NiDX8Renderer::initializeTextureDefaults",
    fn(this) -> u8,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = initialize_texture_defaults);

port_thiscall!(validate_device_caps_port, 0x6AB960, "NiDX8Renderer::validateDeviceCaps",
    fn(this) -> u8,
    compare_eax = true,
    regions = |set| validate_device_caps_regions(set, this),
    rust = validate_device_caps);

// See initializeTextureDefaults.
port_thiscall!(create_default_textures_port, 0x6ABA70, "NiDX8Renderer::createDefaultTextures",
    fn(this) -> u8,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = create_default_textures);

port_thiscall!(get_driver_info_port, 0x6ABBA0, "NiDX8Renderer::GetDriverInfo",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| set.add(this, R::SIZE),
    rust = get_driver_info);

port_thiscall!(probe_texture_formats_port, 0x6B05E0, "NiDX8Renderer::probeTextureFormats",
    fn(this, formats: usize, unused: u32, resource_type: u32) -> (),
    compare_eax = false,
    regions = |set| probe_texture_formats_regions(set, this, formats),
    rust = probe_texture_formats);

port_cdecl!(get_adapters_port, 0x6B0FC0, "NiDX8Renderer_static::GetAdapters",
    fn() -> usize,
    compare_eax = true,
    regions = |set| get_adapters_regions(set),
    rust = get_adapters);

port_cdecl!(d3dformat_to_string_port, 0x6B2920, "NiDX8Renderer_static::D3DFORMAT_toString",
    fn(format: u32) -> usize,
    compare_eax = true,
    regions = |set| (),
    rust = d3dformat_to_string);

// The map and the strings use the allocator of MSVCP60, which the heap tracking does not
// see, and the function can load D3D8.DLL.
port_cdecl!(video_adapter_enum_port, 0x429E20, "NiDX8Renderer_static::videoAdapterEnum",
    fn(game: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = video_adapter_enum);

pub static PORTS: &[&crate::shadow::PortState] = &[
    &create_d3d_port::STATE,
    &release_d3d_port::STATE,
    &get_rtti_port::STATE,
    &deleting_dtor_port::STATE,
    &dtor_port::STATE,
    &create_port::STATE,
    &recreate_port::STATE,
    &initialize_present_params_port::STATE,
    &initialize_texture_defaults_port::STATE,
    &validate_device_caps_port::STATE,
    &create_default_textures_port::STATE,
    &get_driver_info_port::STATE,
    &probe_texture_formats_port::STATE,
    &get_adapters_port::STATE,
    &d3dformat_to_string_port::STATE,
    &video_adapter_enum_port::STATE,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn renderer() -> Vec<u8> {
        vec![0u8; R::SIZE]
    }

    #[test]
    fn offsets_the_code_depends_on() {
        assert_eq!(R::SIZE, 0x6A0);
        assert_eq!(SystemDesc::SIZE, 0x1C);
        assert_eq!(PF::SIZE, 0x28);
        assert_eq!(NiPixelData::SIZE, 0x48);
        assert_eq!(Node::SIZE, 0x0C);
        assert_eq!(RT::SIZE, 9 * 4);
        assert_eq!((PF::extraData, PF::d3dFormat, PF::bitsPerPixel, PF::compareBits), (0x20, 0x24, 0x14, 0x18));
        assert_eq!((Adapter::HALDeviceDesc, Adapter::REFDeviceDesc), (0x44C, 0x450));
        assert_eq!(Adapter::d3dAdapterIdentifier_description, 0x204);
        assert_eq!(DeviceDesc::caps + Caps::PresentationIntervals, 0x18);
        assert_eq!((Caps::MaxTextureBlendStages, Caps::MaxSimultaneousTextures, Caps::MaxVertexBlendMatrices), (0x94, 0x98, 0xA8));
        assert_eq!((Caps::RasterCaps, Caps::TextureCaps), (0x24, 0x3C));
        assert_eq!(RS::texturePerStage, 0x1208);
        assert_eq!(RS::device, 8);
        assert_eq!(Batched::nextBatch, 0x18);
        assert_eq!((NiPixelData::pixelsAllMips, NiPixelData::mipmapOffsets, NiPixelData::bytesPerPixel), (0x2C, 0x38, 0x40));
        assert_eq!(R::arrayArrayTextureFormats, 0x3E8);
        assert_eq!(R::defaultTextureFormats, R::defaultTexturePixelData - 0x10);
        // The second group of formats, as initializeTextureDefaults reaches it.
        assert_eq!(R::depthStencilFormats - 0xC8, R::arrayArrayTextureFormats + FORMATS_PER_GROUP * 4);
        assert_eq!(R::field_28C - R::capabilityFlags, 4);
        assert_eq!(ADAPTER_IDENTIFIER_WORDS * 4, 0x42C);
    }

    #[test]
    fn format_names() {
        unsafe {
            assert_eq!(d3dformat_to_string(0), 0x7C3778);
            assert_eq!(d3dformat_to_string(21), 0x7C3768);
            assert_eq!(d3dformat_to_string(22), 0x7C3758);
            assert_eq!(d3dformat_to_string(63), 0x7C3654);
            assert_eq!(d3dformat_to_string(75), 0x7C3620);
            assert_eq!(d3dformat_to_string(80), 0x7C35E4);
            assert_eq!(d3dformat_to_string(100), 0x7C35B0);
            assert_eq!(d3dformat_to_string(101), 0x7C3590);
            assert_eq!(d3dformat_to_string(102), 0x7C35A0);
            assert_eq!(d3dformat_to_string(u32::from_le_bytes(*b"DXT1")), 0x7C3584);
            assert_eq!(d3dformat_to_string(u32::from_le_bytes(*b"DXT2")), 0x7C3578);
            assert_eq!(d3dformat_to_string(u32::from_le_bytes(*b"DXT3")), 0x7C3554);
            assert_eq!(d3dformat_to_string(u32::from_le_bytes(*b"DXT4")), 0x7C3560);
            assert_eq!(d3dformat_to_string(u32::from_le_bytes(*b"DXT5")), 0x7C3548);
            assert_eq!(d3dformat_to_string(u32::from_le_bytes(*b"YUY2")), 0x7C356C);
            assert_eq!(d3dformat_to_string(u32::from_le_bytes(*b"UYVY")), 0x7C353C);
            // Not in the list: in a gap of a table, between the tables, negative.
            for format in [1, 19, 31, 42, 53, 66, 72, 81, 99, 103, 0x3154_5845, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF] {
                assert_eq!(d3dformat_to_string(format), 0x7941B8, "{format:#x}");
            }
        }
    }

    #[test]
    fn present_parameter_mapping() {
        // NetImmerse frame buffer formats 1 to 11 are D3DFMT_R8G8B8 (20) to X4R4G4B4 (30).
        assert_eq!(frame_buffer_d3d_format(1), 20);
        assert_eq!(frame_buffer_d3d_format(3), 22);
        assert_eq!(frame_buffer_d3d_format(11), 30);
        assert_eq!(frame_buffer_d3d_format(0), 0);
        assert_eq!(frame_buffer_d3d_format(12), 0);
        assert_eq!([0, 1, 2, 16, 17].map(multi_sample_type), [0, 0, 2, 16, 0]);
        assert_eq!([0, 1, 2, 3, 4, 5].map(|s| swap_effect_value(0, s)), [1, 1, 2, 3, 4, 1]);
        assert_eq!([2, 3, 4].map(|s| swap_effect_value(1, s)), [2, 3, 4]);
        assert_eq!([2, 3, 4].map(|s| swap_effect_value(2, s)), [1, 1, 1]);
        assert_eq!([0, 60, 0xFFFF_FFFF].map(refresh_rate_value), [0, 60, 0x7FFF_FFFF]);
        assert_eq!([0, 1, 2, 3, 4, 5].map(presentation_interval_value), [0x8000_0000, 1, 2, 4, 8, 0]);
    }

    // An IDirect3D8 that answers CheckDeviceMultiSampleType.
    thread_local! {
        static MULTI_SAMPLE_CALLS: RefCell<Vec<[u32; 5]>> = const { RefCell::new(Vec::new()) };
        static MULTI_SAMPLE_RESULT: RefCell<u32> = const { RefCell::new(0) };
    }

    unsafe extern "system" fn fake_check_multi_sample(_this: usize, adapter: u32, device_type: u32, format: u32, windowed: u32, samples: u32) -> u32 {
        MULTI_SAMPLE_CALLS.with(|calls| calls.borrow_mut().push([adapter, device_type, format, windowed, samples]));
        MULTI_SAMPLE_RESULT.with(|result| *result.borrow())
    }

    #[test]
    fn present_parameters_of_a_window() {
        let mut vtable = [0usize; 16];
        vtable[0x2C / 4] = fake_check_multi_sample as *const () as usize;
        // The object is one pointer to the vtable; the slot has the address of the object.
        let object = vtable.as_ptr() as usize;
        let object_address = &object as *const usize as usize;
        let slot = &object_address as *const usize as usize;
        let mut this = renderer();
        let r = this.as_mut_ptr() as usize;
        unsafe {
            wr::<u32>(r, R::windowDevice, 0x1234);
            wr::<u32>(r, R::currentAdapterIndex, 2);
            wr::<u32>(r, R::d3dDeviceType, 1);
            // Garbage that the function must clear.
            wr::<u32>(r, R::d3dPresentParameters_Flags, 0xEEEE_EEEE);

            // 800 x 600, window (bit 2 clear), 4 samples, frame buffer format 3, depth
            // format D24S8 given, 0 back buffers, swap effect "flip", rate 75, interval 1.
            let ok = initialize_present_params_on(r, slot, 800, 600, 0, 4, 3, 75, 0, 2, 75, 1);
            assert_eq!(ok, 1);
            // Width, height, format, count, samples, swap effect, window, windowed,
            // auto depth, depth format, flags, rate, interval.
            assert_eq!(rd::<[u32; 13]>(r, R::d3dPresentParameters), [800, 600, 22, 1, 4, 1, 0x1234, 1, 1, 75, 0, 0, 0]);
            assert_eq!(MULTI_SAMPLE_CALLS.with(|calls| calls.borrow_mut().drain(..).collect::<Vec<_>>()), vec![[2, 1, 22, 1, 4]]);

            // One sample: a lockable back buffer and the swap effect that was asked. Five
            // back buffers become three. Bit 0 of the flags: no depth buffer. A rate
            // means nothing in a window.
            let ok = initialize_present_params_on(r, slot, 640, 480, 0x101, 1, 4, 80, 5, 3, 0xFFFF_FFFF, 0);
            assert_eq!(ok, 1);
            assert_eq!(rd::<[u32; 13]>(r, R::d3dPresentParameters), [640, 480, 23, 3, 0, 3, 0x1234, 1, 0, 80, 1, 0, 0]);
            assert_eq!(MULTI_SAMPLE_CALLS.with(|calls| calls.borrow_mut().drain(..).collect::<Vec<_>>()), vec![[2, 1, 23, 1, 0]]);
        }
    }

    /// A system description with one adapter and its HAL device description.
    struct Descriptions {
        _system: Vec<u32>,
        _storage: Vec<usize>,
        _adapter: Vec<u8>,
        device: Vec<u8>,
        system_address: usize,
    }

    fn descriptions() -> Descriptions {
        let mut device = vec![0u8; DeviceDesc::SIZE];
        let mut adapter = vec![0u8; Adapter::SIZE];
        let mut storage = vec![0usize; 2];
        let mut system = vec![0u32; SystemDesc::SIZE / 4];
        unsafe {
            wr(adapter.as_mut_ptr() as usize, Adapter::HALDeviceDesc, device.as_mut_ptr() as usize);
            storage[1] = adapter.as_ptr() as usize;
            wr(system.as_mut_ptr() as usize, SystemDesc::adapterList_storage, storage.as_ptr() as usize);
            // D3DCAPS8::DeviceType
            wr::<u32>(device.as_mut_ptr() as usize, DeviceDesc::caps, 1);
        }
        let system_address = system.as_ptr() as usize;
        Descriptions { _system: system, _storage: storage, _adapter: adapter, device, system_address }
    }

    #[test]
    fn capability_flags_from_the_device_caps() {
        let mut d = descriptions();
        let caps = d.device.as_mut_ptr() as usize + DeviceDesc::caps;
        let mut this = renderer();
        let r = this.as_mut_ptr() as usize;
        unsafe {
            wr(r, R::systemDesc, d.system_address);
            wr::<u32>(r, R::currentAdapterIndex, 1);
            wr::<u32>(r, R::d3dDeviceType, 1);
            wr::<u32>(caps, Caps::MaxTextureBlendStages, 8);
            wr::<u32>(caps, Caps::MaxSimultaneousTextures, 2);
            wr::<u32>(caps, Caps::MaxVertexBlendMatrices, 4);

            assert_eq!(selected_device_desc(r), d.device.as_ptr() as usize);
            // No depth buffer of the device: no stencil bit.
            wr::<u32>(r, R::d3dPresentParameters_AutoDepthStencilFormat, D3DFMT_D24S8);
            assert_eq!(validate_device_caps(r), 1);
            assert_eq!(rd::<u32>(r, R::capabilityFlags), 0x6D);
            assert_eq!(rd::<u32>(r, R::countHWBones), 4);
            assert_eq!(rd::<u8>(r, R::maybe_bMipmapCubeMaps), 0);

            wr::<u32>(r, R::d3dPresentParameters_EnableAutoDepthStencil, 1);
            for (format, flags) in [(75, 0xED), (73, 0xED), (79, 0xED), (77, 0x6D), (80, 0x6D), (71, 0x6D)] {
                wr::<u32>(r, R::d3dPresentParameters_AutoDepthStencilFormat, format);
                assert_eq!(validate_device_caps(r), 1);
                assert_eq!(rd::<u32>(r, R::capabilityFlags), flags, "format {format}");
            }

            // W buffer, square textures only, mip maps of cube maps.
            wr::<u32>(r, R::d3dPresentParameters_AutoDepthStencilFormat, 75);
            wr::<u32>(caps, Caps::RasterCaps, 0x40000);
            wr::<u32>(caps, Caps::TextureCaps, 0x1_0020);
            assert_eq!(validate_device_caps(r), 1);
            assert_eq!(rd::<u32>(r, R::capabilityFlags), 0x1EF);
            assert_eq!(rd::<u8>(r, R::maybe_bMipmapCubeMaps), 1);
        }
    }

    #[test]
    fn behavior_text() {
        let mut this = vec![0xEEu8; R::SIZE];
        let r = this.as_mut_ptr() as usize;
        let empty = b"\0";
        let texts = [b"PURE\0".as_ptr(), b" MT\0".as_ptr(), b" HWVP\0".as_ptr(), b" MIXVP\0".as_ptr(), b" SWVP\0".as_ptr()].map(|p| p as usize);
        let text = |r: usize| unsafe {
            let length = c_string_length(r + R::field_28C);
            String::from_utf8(core::slice::from_raw_parts((r + R::field_28C) as *const u8, length).to_vec()).unwrap()
        };
        unsafe {
            wr::<u32>(r, R::d3dBehaviorFlags, 0x54);
            write_behavior_text(r, empty.as_ptr() as usize, texts);
            assert_eq!(text(r), "PURE MT HWVP");
            // One byte after the end of the text and no more.
            assert_eq!(rd::<[u8; 2]>(r, R::field_28C + 12), [0, 0xEE]);
            wr::<u32>(r, R::d3dBehaviorFlags, 0x84);
            write_behavior_text(r, empty.as_ptr() as usize, texts);
            assert_eq!(text(r), " MT MIXVP");
            wr::<u32>(r, R::d3dBehaviorFlags, 0x20);
            write_behavior_text(r, empty.as_ptr() as usize, texts);
            assert_eq!(text(r), " SWVP");
            wr::<u32>(r, R::d3dBehaviorFlags, 0);
            write_behavior_text(r, empty.as_ptr() as usize, texts);
            assert_eq!(text(r), "");
        }
    }

    #[test]
    fn device_choices() {
        let mut this = renderer();
        let r = this.as_mut_ptr() as usize;
        let read = |r: usize| unsafe { (rd::<u32>(r, R::d3dDeviceType), rd::<u32>(r, R::d3dBehaviorFlags)) };
        unsafe {
            select_device(r, 0);
            assert_eq!(read(r), (1, 0x50));
            select_device(r, 1);
            assert_eq!(read(r), (1, 0x40));
            select_device(r, 2);
            assert_eq!(read(r), (1, 0x80));
            select_device(r, 3);
            assert_eq!(read(r), (1, 0x20));
            select_device(r, 4);
            assert_eq!(read(r), (2, 0x20));
        }
    }

    #[test]
    fn default_format_of_a_group() {
        let mut this = renderer();
        let r = this.as_mut_ptr() as usize;
        let slot = |group: usize, index: usize| R::arrayArrayTextureFormats + (group * FORMATS_PER_GROUP + index) * 4;
        unsafe {
            assert_eq!(default_texture_format(r, 0), 0);
            wr::<u32>(r, slot(0, 5), 0x55);
            wr::<u32>(r, slot(0, 3), 0x33);
            assert_eq!(default_texture_format(r, 0), 0x33);
            wr::<u32>(r, slot(0, 4), 0x44);
            assert_eq!(default_texture_format(r, 0), 0x44);
            wr::<u32>(r, slot(0, 0), 0x10);
            assert_eq!(default_texture_format(r, 0), 0x10);
            wr::<u32>(r, slot(0, 1), 0x11);
            assert_eq!(default_texture_format(r, 0), 0x11);
            // Slots after 5 are not looked at, and a group does not see the next one.
            wr::<u32>(r, slot(2, 8), 0x88);
            wr::<u32>(r, slot(3, 2), 0x22);
            assert_eq!(default_texture_format(r, 2), 0);
            assert_eq!(default_texture_format(r, 3), 0x22);
        }
    }

    // A hash map with the hash "key modulo bucket count".
    unsafe extern "thiscall" fn fake_hash(map: usize, key: u32) -> u32 {
        key % rd::<u32>(map, Map::bucketCount)
    }

    #[test]
    fn hash_map_walk() {
        let vtable = [0usize, fake_hash as *const () as usize];
        let mut buckets = [0usize; 4];
        // key, data, next
        let (mut a, mut b, mut c) = ([1usize, 0, 0], [5usize, 0, 0], [3usize, 0, 0]);
        let (pa, pb, pc) = (a.as_mut_ptr() as usize, b.as_mut_ptr() as usize, c.as_mut_ptr() as usize);
        unsafe { wr(pa, Node::nextNode, pb) };
        buckets[1] = pa;
        buckets[3] = pc;
        let map = [vtable.as_ptr() as usize, 3, 4, buckets.as_ptr() as usize];
        let m = map.as_ptr() as usize;
        unsafe {
            assert_eq!(hash_map_first(m), pa);
            assert_eq!(hash_map_first_after(m, 5), pc);
            assert_eq!(hash_map_first_after(m, 3), 0);
            let empty = [vtable.as_ptr() as usize, 0, 0, 0];
            assert_eq!(hash_map_first(empty.as_ptr() as usize), 0);
            let no_nodes = [0usize; 4];
            let empty = [vtable.as_ptr() as usize, 0, 4, no_nodes.as_ptr() as usize];
            assert_eq!(hash_map_first(empty.as_ptr() as usize), 0);
        }
    }

    #[test]
    fn renderer_data_leaves_the_list_of_its_texture() {
        // Renderer data: vtable, next.
        let (mut a, mut b, mut c) = ([0usize; 2], [0usize; 2], [0usize; 2]);
        let (pa, pb, pc) = (a.as_mut_ptr() as usize, b.as_mut_ptr() as usize, c.as_mut_ptr() as usize);
        let mut texture = [0usize; 9];
        let t = texture.as_mut_ptr() as usize;
        let link = |texture: usize| unsafe {
            wr(texture, TEXTURE_RENDERER_DATA, pa);
            wr(pa, RENDERER_DATA_NEXT, pb);
            wr(pb, RENDERER_DATA_NEXT, pc);
            wr::<usize>(pc, RENDERER_DATA_NEXT, 0);
        };
        unsafe {
            link(t);
            unlink_renderer_data(t, pa);
            assert_eq!(rd::<usize>(t, TEXTURE_RENDERER_DATA), pb);
            link(t);
            unlink_renderer_data(t, pb);
            assert_eq!((rd::<usize>(t, TEXTURE_RENDERER_DATA), rd::<usize>(pa, RENDERER_DATA_NEXT)), (pa, pc));
            link(t);
            unlink_renderer_data(t, pc);
            assert_eq!(rd::<usize>(pb, RENDERER_DATA_NEXT), 0);
            // Not in the list: nothing changes.
            link(t);
            unlink_renderer_data(t, 0x1234);
            assert_eq!((rd::<usize>(pa, RENDERER_DATA_NEXT), rd::<usize>(pb, RENDERER_DATA_NEXT)), (pb, pc));
            wr::<usize>(t, TEXTURE_RENDERER_DATA, 0);
            unlink_renderer_data(t, pa);
            assert_eq!(rd::<usize>(t, TEXTURE_RENDERER_DATA), 0);
        }
    }

    #[test]
    fn rtti_and_regions() {
        unsafe {
            assert_eq!(get_rtti(0), 0x7DD89C);
            let mut this = renderer();
            let r = this.as_mut_ptr() as usize;
            let mut set = Regions::new();
            validate_device_caps_regions(&mut set, r);
            assert_eq!(set.as_slice(), &[(r, R::SIZE), (ERROR_STRING, 0x100)]);
        }
    }
}
