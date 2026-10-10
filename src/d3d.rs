//! Direct3D 8 calls through the vtable of the COM object, as the game code makes them.
//!
//! Each function reads the vtable pointer and the slot on each call. It does not keep the
//! function pointer, because the recorder replaces the vtable. Each function is `__stdcall`
//! with the object as the first argument. It returns the `HRESULT`, or the count for
//! `AddRef` and `Release`.
//!
//! An argument that is an address is `usize`. Each other argument is `u32`.


use core::mem::transmute;

use crate::engine::{fault, virtual_at};
use crate::mem::rd;

// ---- vtable slots, as offsets in bytes

/// The slot of `IUnknown::AddRef`.
pub const COM_ADD_REF: usize = 0x04;
/// The slot of `IUnknown::Release`.
pub const COM_RELEASE: usize = 0x08;
/// The slot of `IDirect3D8::GetAdapterCount`.
pub const D3D_GET_ADAPTER_COUNT: usize = 0x10;
/// The slot of `IDirect3D8::GetAdapterIdentifier`.
pub const D3D_GET_ADAPTER_IDENTIFIER: usize = 0x14;
/// The slot of `IDirect3D8::GetAdapterModeCount`.
pub const D3D_GET_ADAPTER_MODE_COUNT: usize = 0x18;
/// The slot of `IDirect3D8::EnumAdapterModes`.
pub const D3D_ENUM_ADAPTER_MODES: usize = 0x1C;
/// The slot of `IDirect3D8::GetAdapterDisplayMode`.
pub const D3D_GET_ADAPTER_DISPLAY_MODE: usize = 0x20;
/// The slot of `IDirect3D8::CheckDeviceType`.
pub const D3D_CHECK_DEVICE_TYPE: usize = 0x24;
/// The slot of `IDirect3D8::CheckDeviceFormat`.
pub const D3D_CHECK_DEVICE_FORMAT: usize = 0x28;
/// The slot of `IDirect3D8::CheckDeviceMultiSampleType`.
pub const D3D_CHECK_DEVICE_MULTI_SAMPLE_TYPE: usize = 0x2C;
/// The slot of `IDirect3D8::CheckDepthStencilMatch`.
pub const D3D_CHECK_DEPTH_STENCIL_MATCH: usize = 0x30;
/// The slot of `IDirect3D8::GetDeviceCaps`.
pub const D3D_GET_DEVICE_CAPS: usize = 0x34;
/// The slot of `IDirect3D8::CreateDevice`.
pub const D3D_CREATE_DEVICE: usize = 0x3C;
/// The slot of `IDirect3DDevice8::AddRef`.
pub const DEVICE_ADD_REF: usize = 0x04;
/// The slot of `IDirect3DDevice8::Release`.
pub const DEVICE_RELEASE: usize = 0x08;
/// The slot of `IDirect3DDevice8::TestCooperativeLevel`.
pub const DEVICE_TEST_COOPERATIVE_LEVEL: usize = 0x0C;
/// The slot of `IDirect3DDevice8::ResourceManagerDiscardBytes`.
pub const DEVICE_RESOURCE_MANAGER_DISCARD_BYTES: usize = 0x14;
/// The slot of `IDirect3DDevice8::Reset`.
pub const DEVICE_RESET: usize = 0x38;
/// The slot of `IDirect3DDevice8::Present`.
pub const DEVICE_PRESENT: usize = 0x3C;
/// The slot of `IDirect3DDevice8::CreateTexture`.
pub const DEVICE_CREATE_TEXTURE: usize = 0x50;
/// The slot of `IDirect3DDevice8::CreateCubeTexture`.
pub const DEVICE_CREATE_CUBE_TEXTURE: usize = 0x58;
/// The slot of `IDirect3DDevice8::CreateVertexBuffer`.
pub const DEVICE_CREATE_VERTEX_BUFFER: usize = 0x5C;
/// The slot of `IDirect3DDevice8::CreateIndexBuffer`.
pub const DEVICE_CREATE_INDEX_BUFFER: usize = 0x60;
/// The slot of `IDirect3DDevice8::CreateImageSurface`.
pub const DEVICE_CREATE_IMAGE_SURFACE: usize = 0x6C;
/// The slot of `IDirect3DDevice8::CopyRects`.
pub const DEVICE_COPY_RECTS: usize = 0x70;
/// The slot of `IDirect3DDevice8::SetRenderTarget`.
pub const DEVICE_SET_RENDER_TARGET: usize = 0x7C;
/// The slot of `IDirect3DDevice8::GetRenderTarget`.
pub const DEVICE_GET_RENDER_TARGET: usize = 0x80;
/// The slot of `IDirect3DDevice8::GetDepthStencilSurface`.
pub const DEVICE_GET_DEPTH_STENCIL_SURFACE: usize = 0x84;
/// The slot of `IDirect3DDevice8::BeginScene`.
pub const DEVICE_BEGIN_SCENE: usize = 0x88;
/// The slot of `IDirect3DDevice8::EndScene`.
pub const DEVICE_END_SCENE: usize = 0x8C;
/// The slot of `IDirect3DDevice8::Clear`.
pub const DEVICE_CLEAR: usize = 0x90;
/// The slot of `IDirect3DDevice8::SetTransform`.
pub const DEVICE_SET_TRANSFORM: usize = 0x94;
/// The slot of `IDirect3DDevice8::SetViewport`.
pub const DEVICE_SET_VIEWPORT: usize = 0xA0;
/// The slot of `IDirect3DDevice8::SetLight`.
pub const DEVICE_SET_LIGHT: usize = 0xB0;
/// The slot of `IDirect3DDevice8::LightEnable`.
pub const DEVICE_LIGHT_ENABLE: usize = 0xB8;
/// The slot of `IDirect3DDevice8::SetRenderState`.
pub const DEVICE_SET_RENDER_STATE: usize = 0xC8;
/// The slot of `IDirect3DDevice8::SetTexture`.
pub const DEVICE_SET_TEXTURE: usize = 0xF4;
/// The slot of `IDirect3DDevice8::SetTextureStageState`.
pub const DEVICE_SET_TEXTURE_STAGE_STATE: usize = 0xFC;
/// The slot of `IDirect3DDevice8::DrawPrimitive`.
pub const DEVICE_DRAW_PRIMITIVE: usize = 0x118;
/// The slot of `IDirect3DDevice8::DrawIndexedPrimitive`.
pub const DEVICE_DRAW_INDEXED_PRIMITIVE: usize = 0x11C;
/// The slot of `IDirect3DDevice8::SetVertexShader`.
pub const DEVICE_SET_VERTEX_SHADER: usize = 0x130;
/// The slot of `IDirect3DDevice8::SetStreamSource`.
pub const DEVICE_SET_STREAM_SOURCE: usize = 0x14C;
/// The slot of `IDirect3DDevice8::SetIndices`.
pub const DEVICE_SET_INDICES: usize = 0x154;
/// The slot of `IDirect3DDevice8::SetPixelShader`.
pub const DEVICE_SET_PIXEL_SHADER: usize = 0x160;
/// The slot of `IDirect3DVertexBuffer8::Lock`.
pub const BUFFER_LOCK: usize = 0x2C;
/// The slot of `IDirect3DVertexBuffer8::Unlock`.
pub const BUFFER_UNLOCK: usize = 0x30;
/// The slot of `IDirect3DVertexBuffer8::GetDesc`.
pub const BUFFER_GET_DESC: usize = 0x34;
/// The slot of `IDirect3DTexture8::GetSurfaceLevel`.
pub const TEXTURE_GET_SURFACE_LEVEL: usize = 0x3C;
/// The slot of `IDirect3DCubeTexture8::GetCubeMapSurface`.
pub const CUBE_TEXTURE_GET_CUBE_MAP_SURFACE: usize = 0x3C;
/// The slot of `IDirect3DSurface8::GetDesc`.
pub const SURFACE_GET_DESC: usize = 0x20;
/// The slot of `IDirect3DSurface8::LockRect`.
pub const SURFACE_LOCK_RECT: usize = 0x24;
/// The slot of `IDirect3DSurface8::UnlockRect`.
pub const SURFACE_UNLOCK_RECT: usize = 0x28;

// ---- calls

/// `IUnknown::AddRef` through the vtable of the object, slot 0x04.
/// It is for each COM object: a device, a buffer, a texture or a surface.
#[inline(always)]
pub unsafe fn com_add_ref(object: usize) -> u32 {
    if let Some(result) = crate::link_client::object_call(0, &[object as u32]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize) -> u32 = transmute(virtual_at(object, COM_ADD_REF));
    f(object)
}

/// `IUnknown::Release` through the vtable of the object, slot 0x08.
/// It is for each COM object: a device, a buffer, a texture or a surface.
#[inline(always)]
pub unsafe fn com_release(object: usize) -> u32 {
    if let Some(result) = crate::link_client::object_call(1, &[object as u32]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize) -> u32 = transmute(virtual_at(object, COM_RELEASE));
    f(object)
}

/// `IDirect3D8::GetAdapterCount` through the vtable of the object, slot 0x10.
#[inline(always)]
pub unsafe fn d3d_get_adapter_count(d3d: usize) -> u32 {
    let f: unsafe extern "system" fn(usize) -> u32 = transmute(virtual_at(d3d, D3D_GET_ADAPTER_COUNT));
    f(d3d)
}

/// `IDirect3D8::GetAdapterIdentifier` through the vtable of the object, slot 0x14.
#[inline(always)]
pub unsafe fn d3d_get_adapter_identifier(d3d: usize, adapter: u32, flags: u32, identifier: usize) -> u32 {
    let f: unsafe extern "system" fn(usize, u32, u32, usize) -> u32 = transmute(virtual_at(d3d, D3D_GET_ADAPTER_IDENTIFIER));
    f(d3d, adapter, flags, identifier)
}

/// `IDirect3D8::GetAdapterModeCount` through the vtable of the object, slot 0x18.
#[inline(always)]
pub unsafe fn d3d_get_adapter_mode_count(d3d: usize, adapter: u32) -> u32 {
    let f: unsafe extern "system" fn(usize, u32) -> u32 = transmute(virtual_at(d3d, D3D_GET_ADAPTER_MODE_COUNT));
    f(d3d, adapter)
}

/// `IDirect3D8::EnumAdapterModes` through the vtable of the object, slot 0x1C.
#[inline(always)]
pub unsafe fn d3d_enum_adapter_modes(d3d: usize, adapter: u32, mode: u32, out: usize) -> u32 {
    let f: unsafe extern "system" fn(usize, u32, u32, usize) -> u32 = transmute(virtual_at(d3d, D3D_ENUM_ADAPTER_MODES));
    f(d3d, adapter, mode, out)
}

/// `IDirect3D8::GetAdapterDisplayMode` through the vtable of the object, slot 0x20.
#[inline(always)]
pub unsafe fn d3d_get_adapter_display_mode(d3d: usize, adapter: u32, out: usize) -> u32 {
    let f: unsafe extern "system" fn(usize, u32, usize) -> u32 = transmute(virtual_at(d3d, D3D_GET_ADAPTER_DISPLAY_MODE));
    f(d3d, adapter, out)
}

/// `IDirect3D8::CheckDeviceType` through the vtable of the object, slot 0x24.
#[inline(always)]
pub unsafe fn d3d_check_device_type(d3d: usize, adapter: u32, device_type: u32, display_format: u32, back_buffer_format: u32, windowed: u32) -> u32 {
    let f: unsafe extern "system" fn(usize, u32, u32, u32, u32, u32) -> u32 = transmute(virtual_at(d3d, D3D_CHECK_DEVICE_TYPE));
    f(d3d, adapter, device_type, display_format, back_buffer_format, windowed)
}

/// `IDirect3D8::CheckDeviceFormat` through the vtable of the object, slot 0x28.
#[inline(always)]
pub unsafe fn d3d_check_device_format(d3d: usize, adapter: u32, device_type: u32, adapter_format: u32, usage: u32, resource_type: u32, format: u32) -> u32 {
    let f: unsafe extern "system" fn(usize, u32, u32, u32, u32, u32, u32) -> u32 = transmute(virtual_at(d3d, D3D_CHECK_DEVICE_FORMAT));
    f(d3d, adapter, device_type, adapter_format, usage, resource_type, format)
}

/// `IDirect3D8::CheckDeviceMultiSampleType` through the vtable of the object, slot 0x2C.
#[inline(always)]
pub unsafe fn d3d_check_device_multi_sample_type(d3d: usize, adapter: u32, device_type: u32, format: u32, windowed: u32, samples: u32) -> u32 {
    let f: unsafe extern "system" fn(usize, u32, u32, u32, u32, u32) -> u32 = transmute(virtual_at(d3d, D3D_CHECK_DEVICE_MULTI_SAMPLE_TYPE));
    f(d3d, adapter, device_type, format, windowed, samples)
}

/// `IDirect3D8::CheckDepthStencilMatch` through the vtable of the object, slot 0x30.
#[inline(always)]
pub unsafe fn d3d_check_depth_stencil_match(d3d: usize, adapter: u32, device_type: u32, adapter_format: u32, target_format: u32, depth_format: u32) -> u32 {
    let f: unsafe extern "system" fn(usize, u32, u32, u32, u32, u32) -> u32 = transmute(virtual_at(d3d, D3D_CHECK_DEPTH_STENCIL_MATCH));
    f(d3d, adapter, device_type, adapter_format, target_format, depth_format)
}

/// `IDirect3D8::GetDeviceCaps` through the vtable of the object, slot 0x34.
#[inline(always)]
pub unsafe fn d3d_get_device_caps(d3d: usize, adapter: u32, device_type: u32, caps: usize) -> u32 {
    let f: unsafe extern "system" fn(usize, u32, u32, usize) -> u32 = transmute(virtual_at(d3d, D3D_GET_DEVICE_CAPS));
    f(d3d, adapter, device_type, caps)
}

/// `IDirect3D8::CreateDevice` through the vtable of the object, slot 0x3C.
#[inline(always)]
pub unsafe fn d3d_create_device(d3d: usize, adapter: u32, device_type: u32, focus_window: u32, behavior_flags: u32, parameters: usize, device: usize) -> u32 {
    let f: unsafe extern "system" fn(usize, u32, u32, u32, u32, usize, usize) -> u32 = transmute(virtual_at(d3d, D3D_CREATE_DEVICE));
    f(d3d, adapter, device_type, focus_window, behavior_flags, parameters, device)
}

/// `IDirect3DDevice8::AddRef` through the vtable of the object, slot 0x04.
#[inline(always)]
pub unsafe fn device_add_ref(device: usize) -> u32 {
    let f: unsafe extern "system" fn(usize) -> u32 = transmute(virtual_at(device, DEVICE_ADD_REF));
    f(device)
}

/// `IDirect3DDevice8::Release` through the vtable of the object, slot 0x08.
#[inline(always)]
pub unsafe fn device_release(device: usize) -> u32 {
    let f: unsafe extern "system" fn(usize) -> u32 = transmute(virtual_at(device, DEVICE_RELEASE));
    f(device)
}

/// `IDirect3DDevice8::TestCooperativeLevel` through the vtable of the object, slot 0x0C.
#[inline(always)]
pub unsafe fn device_test_cooperative_level(device: usize) -> u32 {
    if let Some(result) = crate::link_client::frame_call(crate::link::CALL_TEST_DEVICE, &[]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize) -> u32 = transmute(virtual_at(device, DEVICE_TEST_COOPERATIVE_LEVEL));
    f(device)
}

/// `IDirect3DDevice8::ResourceManagerDiscardBytes` through the vtable of the object, slot 0x14.
#[inline(always)]
pub unsafe fn device_resource_manager_discard_bytes(device: usize, bytes: u32) -> u32 {
    if let Some(result) = crate::link_client::target_call(crate::link::CALL_DISCARD_BYTES, &[bytes]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize, u32) -> u32 = transmute(virtual_at(device, DEVICE_RESOURCE_MANAGER_DISCARD_BYTES));
    f(device, bytes)
}

/// `IDirect3DDevice8::Reset` through the vtable of the object, slot 0x38.
#[inline(always)]
pub unsafe fn device_reset(device: usize, parameters: usize) -> u32 {
    let f: unsafe extern "system" fn(usize, usize) -> u32 = transmute(virtual_at(device, DEVICE_RESET));
    f(device, parameters)
}

/// `IDirect3DDevice8::Present` through the vtable of the object, slot 0x3C.
#[inline(always)]
pub unsafe fn device_present(device: usize, source_rect: usize, dest_rect: usize, window: u32, dirty_region: usize) -> u32 {
    let args = [source_rect as u32, dest_rect as u32, window, dirty_region as u32];
    if let Some(result) = crate::link_client::frame_call(crate::link::CALL_PRESENT, &args) {
        return result;
    }
    let f: unsafe extern "system" fn(usize, usize, usize, u32, usize) -> u32 = transmute(virtual_at(device, DEVICE_PRESENT));
    f(device, source_rect, dest_rect, window, dirty_region)
}

/// `IDirect3DDevice8::CreateTexture` through the vtable of the object, slot 0x50.
#[inline(always)]
pub unsafe fn device_create_texture(device: usize, width: u32, height: u32, levels: u32, usage: u32, format: u32, pool: u32, out: usize) -> u32 {
    let args = [width, height, levels, usage, format, pool, out as u32];
    if let Some(result) = crate::link_client::target_call(crate::link::CALL_CREATE_TEXTURE, &args) {
        return result;
    }
    let f: unsafe extern "system" fn(usize, u32, u32, u32, u32, u32, u32, usize) -> u32 = transmute(virtual_at(device, DEVICE_CREATE_TEXTURE));
    f(device, width, height, levels, usage, format, pool, out)
}

/// `IDirect3DDevice8::CreateCubeTexture` through the vtable of the object, slot 0x58.
#[inline(always)]
pub unsafe fn device_create_cube_texture(device: usize, edge: u32, levels: u32, usage: u32, format: u32, pool: u32, out: usize) -> u32 {
    let f: unsafe extern "system" fn(usize, u32, u32, u32, u32, u32, usize) -> u32 = transmute(virtual_at(device, DEVICE_CREATE_CUBE_TEXTURE));
    f(device, edge, levels, usage, format, pool, out)
}

/// `IDirect3DDevice8::CreateVertexBuffer` through the vtable of the object, slot 0x5C.
#[inline(always)]
pub unsafe fn device_create_vertex_buffer(device: usize, length: u32, usage: u32, fvf: u32, pool: u32, out: usize) -> u32 {
    if let Some(result) = crate::link_client::buffer_call(crate::link::CALL_CREATE_VERTEX_BUFFER, &[length, usage, fvf, pool, out as u32]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize, u32, u32, u32, u32, usize) -> u32 = transmute(virtual_at(device, DEVICE_CREATE_VERTEX_BUFFER));
    f(device, length, usage, fvf, pool, out)
}

/// `IDirect3DDevice8::CreateIndexBuffer` through the vtable of the object, slot 0x60.
#[inline(always)]
pub unsafe fn device_create_index_buffer(device: usize, length: u32, usage: u32, format: u32, pool: u32, out: usize) -> u32 {
    if let Some(result) = crate::link_client::buffer_call(crate::link::CALL_CREATE_INDEX_BUFFER, &[length, usage, format, pool, out as u32]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize, u32, u32, u32, u32, usize) -> u32 = transmute(virtual_at(device, DEVICE_CREATE_INDEX_BUFFER));
    f(device, length, usage, format, pool, out)
}

/// `IDirect3DDevice8::CreateImageSurface` through the vtable of the object, slot 0x6C.
#[inline(always)]
pub unsafe fn device_create_image_surface(device: usize, width: u32, height: u32, format: u32, out: usize) -> u32 {
    if let Some(result) = crate::link_client::target_call(crate::link::CALL_CREATE_IMAGE_SURFACE, &[width, height, format, out as u32]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize, u32, u32, u32, usize) -> u32 = transmute(virtual_at(device, DEVICE_CREATE_IMAGE_SURFACE));
    f(device, width, height, format, out)
}

/// `IDirect3DDevice8::CopyRects` through the vtable of the object, slot 0x70.
#[inline(always)]
pub unsafe fn device_copy_rects(device: usize, source: usize, rects: usize, count: u32, dest: usize, points: usize) -> u32 {
    let args = [source as u32, rects as u32, count, dest as u32, points as u32];
    if let Some(result) = crate::link_client::target_call(crate::link::CALL_COPY_PIXELS, &args) {
        return result;
    }
    let f: unsafe extern "system" fn(usize, usize, usize, u32, usize, usize) -> u32 = transmute(virtual_at(device, DEVICE_COPY_RECTS));
    f(device, source, rects, count, dest, points)
}

/// `IDirect3DDevice8::SetRenderTarget` through the vtable of the object, slot 0x7C.
#[inline(always)]
pub unsafe fn device_set_render_target(device: usize, target: usize, depth_stencil: usize) -> u32 {
    if let Some(result) = crate::link_client::frame_call(crate::link::CALL_RENDER_TARGET, &[target as u32, depth_stencil as u32]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize, usize, usize) -> u32 = transmute(virtual_at(device, DEVICE_SET_RENDER_TARGET));
    f(device, target, depth_stencil)
}

/// `IDirect3DDevice8::GetRenderTarget` through the vtable of the object, slot 0x80.
#[inline(always)]
pub unsafe fn device_get_render_target(device: usize, out: usize) -> u32 {
    if let Some(result) = crate::link_client::target_call(crate::link::CALL_GET_RENDER_TARGET, &[out as u32]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize, usize) -> u32 = transmute(virtual_at(device, DEVICE_GET_RENDER_TARGET));
    f(device, out)
}

/// `IDirect3DDevice8::GetDepthStencilSurface` through the vtable of the object, slot 0x84.
#[inline(always)]
pub unsafe fn device_get_depth_stencil_surface(device: usize, out: usize) -> u32 {
    if let Some(result) = crate::link_client::target_call(crate::link::CALL_GET_DEPTH_BUFFER, &[out as u32]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize, usize) -> u32 = transmute(virtual_at(device, DEVICE_GET_DEPTH_STENCIL_SURFACE));
    f(device, out)
}

/// `IDirect3DDevice8::BeginScene` through the vtable of the object, slot 0x88.
#[inline(always)]
pub unsafe fn device_begin_scene(device: usize) -> u32 {
    if let Some(result) = crate::link_client::frame_call(crate::link::CALL_BEGIN_SCENE, &[]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize) -> u32 = transmute(virtual_at(device, DEVICE_BEGIN_SCENE));
    f(device)
}

/// `IDirect3DDevice8::EndScene` through the vtable of the object, slot 0x8C.
#[inline(always)]
pub unsafe fn device_end_scene(device: usize) -> u32 {
    if let Some(result) = crate::link_client::frame_call(crate::link::CALL_END_SCENE, &[]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize) -> u32 = transmute(virtual_at(device, DEVICE_END_SCENE));
    f(device)
}

/// `IDirect3DDevice8::Clear` through the vtable of the object, slot 0x90.
/// `z` is the bits of the float.
#[inline(always)]
pub unsafe fn device_clear(device: usize, count: u32, rects: usize, flags: u32, color: u32, z: u32, stencil: u32) -> u32 {
    if let Some(result) = crate::link_client::frame_call(crate::link::CALL_CLEAR, &[count, rects as u32, flags, color, z, stencil]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize, u32, usize, u32, u32, u32, u32) -> u32 = transmute(virtual_at(device, DEVICE_CLEAR));
    f(device, count, rects, flags, color, z, stencil)
}

/// `IDirect3DDevice8::SetTransform` through the vtable of the object, slot 0x94.
#[inline(always)]
pub unsafe fn device_set_transform(device: usize, state: u32, matrix: usize) -> u32 {
    crate::link_client::note_transform(state, matrix);
    if crate::link_client::skip_transform(state, matrix) {
        return 0;
    }
    let f: unsafe extern "system" fn(usize, u32, usize) -> u32 = transmute(virtual_at(device, DEVICE_SET_TRANSFORM));
    f(device, state, matrix)
}

/// `IDirect3DDevice8::SetViewport` through the vtable of the object, slot 0xA0.
#[inline(always)]
pub unsafe fn device_set_viewport(device: usize, viewport: usize) -> u32 {
    if let Some(result) = crate::link_client::frame_call(crate::link::CALL_VIEWPORT, &[viewport as u32]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize, usize) -> u32 = transmute(virtual_at(device, DEVICE_SET_VIEWPORT));
    f(device, viewport)
}

/// `IDirect3DDevice8::SetLight` through the vtable of the object, slot 0xB0.
#[inline(always)]
pub unsafe fn device_set_light(device: usize, index: u32, light: usize) -> u32 {
    if crate::link_client::skip_lights() {
        return 0;
    }
    let f: unsafe extern "system" fn(usize, u32, usize) -> u32 = transmute(virtual_at(device, DEVICE_SET_LIGHT));
    f(device, index, light)
}

/// `IDirect3DDevice8::LightEnable` through the vtable of the object, slot 0xB8.
#[inline(always)]
pub unsafe fn device_light_enable(device: usize, index: u32, enable: u32) -> u32 {
    if crate::link_client::skip_lights() {
        return 0;
    }
    let f: unsafe extern "system" fn(usize, u32, u32) -> u32 = transmute(virtual_at(device, DEVICE_LIGHT_ENABLE));
    f(device, index, enable)
}

/// `IDirect3DDevice8::SetRenderState` through the vtable of the device, as the game code calls it.
#[inline(always)]
pub unsafe fn device_set_render_state(device: usize, state: u32, value: u32) -> u32 {
    // For D3DRS_ZFUNC, D3DCMP_LESSEQUAL (4) becomes D3DCMP_LESS (2).
    let value = if state == 23 && value == 4 && fault() == 1 { 2 } else { value };
    crate::link_client::note_render_state(state);
    if crate::link_client::skip_render_state(state, value) {
        return 0;
    }
    let vtable: usize = rd(device, 0);
    let f: unsafe extern "system" fn(usize, u32, u32) -> u32 = transmute(rd::<usize>(vtable, DEVICE_SET_RENDER_STATE));
    f(device, state, value)
}

/// `IDirect3DDevice8::SetTexture` through the vtable of the object, slot 0xF4.
#[inline(always)]
pub unsafe fn device_set_texture(device: usize, stage: u32, texture: usize) -> u32 {
    crate::link_client::note_texture(stage, texture);
    if crate::link_client::skip_texture(stage) {
        return 0;
    }
    let f: unsafe extern "system" fn(usize, u32, usize) -> u32 = transmute(virtual_at(device, DEVICE_SET_TEXTURE));
    f(device, stage, texture)
}

/// `IDirect3DDevice8::SetTextureStageState` through the vtable of the device.
#[inline(always)]
pub unsafe fn device_set_texture_stage_state(device: usize, stage: u32, state: u32, value: u32) -> u32 {
    crate::link_client::note_stage_state(state);
    if crate::link_client::skip_stage_state(stage, state, value) {
        return 0;
    }
    let vtable: usize = rd(device, 0);
    let f: unsafe extern "system" fn(usize, u32, u32, u32) -> u32 = transmute(rd::<usize>(vtable, DEVICE_SET_TEXTURE_STAGE_STATE));
    f(device, stage, state, value)
}

/// `IDirect3DDevice8::DrawPrimitive` through the vtable of the object, slot 0x118.
#[inline(always)]
pub unsafe fn device_draw_primitive(device: usize, primitive_type: u32, start_vertex: u32, primitives: u32) -> u32 {
    if crate::link_client::draw(primitive_type, start_vertex, primitives) {
        return 0;
    }
    let f: unsafe extern "system" fn(usize, u32, u32, u32) -> u32 = transmute(virtual_at(device, DEVICE_DRAW_PRIMITIVE));
    f(device, primitive_type, start_vertex, primitives)
}

/// `IDirect3DDevice8::DrawIndexedPrimitive` through the vtable of the object, slot 0x11C.
#[inline(always)]
pub unsafe fn device_draw_indexed_primitive(device: usize, primitive_type: u32, min_index: u32, vertices: u32, start_index: u32, primitives: u32) -> u32 {
    if crate::link_client::draw_indexed(primitive_type, min_index, vertices, start_index, primitives) {
        return 0;
    }
    let f: unsafe extern "system" fn(usize, u32, u32, u32, u32, u32) -> u32 = transmute(virtual_at(device, DEVICE_DRAW_INDEXED_PRIMITIVE));
    f(device, primitive_type, min_index, vertices, start_index, primitives)
}

/// `IDirect3DDevice8::SetVertexShader` through the vtable of the object, slot 0x130.
#[inline(always)]
pub unsafe fn device_set_vertex_shader(device: usize, handle: u32) -> u32 {
    crate::link_client::note_vertex_shader(handle);
    if crate::link_client::skip_buffers() {
        return 0;
    }
    let f: unsafe extern "system" fn(usize, u32) -> u32 = transmute(virtual_at(device, DEVICE_SET_VERTEX_SHADER));
    f(device, handle)
}

/// `IDirect3DDevice8::SetStreamSource` through the vtable of the object, slot 0x14C.
#[inline(always)]
pub unsafe fn device_set_stream_source(device: usize, stream: u32, buffer: usize, stride: u32) -> u32 {
    if stream == 0 {
        crate::link_client::note_stream_source(buffer, stride);
        if crate::link_client::skip_buffers() {
            return 0;
        }
    }
    let f: unsafe extern "system" fn(usize, u32, usize, u32) -> u32 = transmute(virtual_at(device, DEVICE_SET_STREAM_SOURCE));
    f(device, stream, buffer, stride)
}

/// `IDirect3DDevice8::SetIndices` through the vtable of the object, slot 0x154.
#[inline(always)]
pub unsafe fn device_set_indices(device: usize, buffer: usize, base_vertex: u32) -> u32 {
    crate::link_client::note_indices(buffer, base_vertex);
    if crate::link_client::skip_buffers() {
        return 0;
    }
    let f: unsafe extern "system" fn(usize, usize, u32) -> u32 = transmute(virtual_at(device, DEVICE_SET_INDICES));
    f(device, buffer, base_vertex)
}

/// `IDirect3DDevice8::SetPixelShader` through the vtable of the object, slot 0x160.
#[inline(always)]
pub unsafe fn device_set_pixel_shader(device: usize, handle: u32) -> u32 {
    let f: unsafe extern "system" fn(usize, u32) -> u32 = transmute(virtual_at(device, DEVICE_SET_PIXEL_SHADER));
    f(device, handle)
}

/// `IDirect3DVertexBuffer8::Lock` through the vtable of the object, slot 0x2C.
/// `IDirect3DIndexBuffer8::Lock` has the same slot and the same arguments.
#[inline(always)]
pub unsafe fn buffer_lock(buffer: usize, offset: u32, size: u32, out: usize, flags: u32) -> u32 {
    if let Some(result) = crate::link_client::object_call(2, &[buffer as u32, offset, size, out as u32, flags]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize, u32, u32, usize, u32) -> u32 = transmute(virtual_at(buffer, BUFFER_LOCK));
    f(buffer, offset, size, out, flags)
}

/// `IDirect3DVertexBuffer8::Unlock` through the vtable of the object, slot 0x30.
/// `IDirect3DIndexBuffer8::Unlock` has the same slot.
#[inline(always)]
pub unsafe fn buffer_unlock(buffer: usize) -> u32 {
    if let Some(result) = crate::link_client::object_call(3, &[buffer as u32]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize) -> u32 = transmute(virtual_at(buffer, BUFFER_UNLOCK));
    f(buffer)
}

/// `IDirect3DVertexBuffer8::GetDesc` through the vtable of the object, slot 0x34.
/// `IDirect3DIndexBuffer8::GetDesc` has the same slot.
#[inline(always)]
pub unsafe fn buffer_get_desc(buffer: usize, desc: usize) -> u32 {
    if let Some(result) = crate::link_client::object_call(4, &[buffer as u32, desc as u32]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize, usize) -> u32 = transmute(virtual_at(buffer, BUFFER_GET_DESC));
    f(buffer, desc)
}

/// `IDirect3DTexture8::GetSurfaceLevel` through the vtable of the object, slot 0x3C.
#[inline(always)]
pub unsafe fn texture_get_surface_level(texture: usize, level: u32, out: usize) -> u32 {
    if let Some(result) = crate::link_client::object_call(5, &[texture as u32, level, out as u32]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize, u32, usize) -> u32 = transmute(virtual_at(texture, TEXTURE_GET_SURFACE_LEVEL));
    f(texture, level, out)
}

/// `IDirect3DCubeTexture8::GetCubeMapSurface` through the vtable of the object, slot 0x3C.
/// The slot is the slot of `IDirect3DTexture8::GetSurfaceLevel`. The arguments are different.
#[inline(always)]
pub unsafe fn cube_texture_get_cube_map_surface(texture: usize, face: u32, level: u32, out: usize) -> u32 {
    if let Some(result) = crate::link_client::object_call(6, &[texture as u32, face, level, out as u32]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize, u32, u32, usize) -> u32 = transmute(virtual_at(texture, CUBE_TEXTURE_GET_CUBE_MAP_SURFACE));
    f(texture, face, level, out)
}

/// `IDirect3DSurface8::GetDesc` through the vtable of the object, slot 0x20.
#[inline(always)]
pub unsafe fn surface_get_desc(surface: usize, desc: usize) -> u32 {
    if let Some(result) = crate::link_client::object_call(7, &[surface as u32, desc as u32]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize, usize) -> u32 = transmute(virtual_at(surface, SURFACE_GET_DESC));
    f(surface, desc)
}

/// `IDirect3DSurface8::LockRect` through the vtable of the object, slot 0x24.
#[inline(always)]
pub unsafe fn surface_lock_rect(surface: usize, locked: usize, rect: usize, flags: u32) -> u32 {
    if let Some(result) = crate::link_client::object_call(8, &[surface as u32, locked as u32, rect as u32, flags]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize, usize, usize, u32) -> u32 = transmute(virtual_at(surface, SURFACE_LOCK_RECT));
    f(surface, locked, rect, flags)
}

/// `IDirect3DSurface8::UnlockRect` through the vtable of the object, slot 0x28.
#[inline(always)]
pub unsafe fn surface_unlock_rect(surface: usize) -> u32 {
    if let Some(result) = crate::link_client::object_call(9, &[surface as u32]) {
        return result;
    }
    let f: unsafe extern "system" fn(usize) -> u32 = transmute(virtual_at(surface, SURFACE_UNLOCK_RECT));
    f(surface)
}

// ---- constants of d3d8types.h and d3d8.h that the ports use

// D3DRS: the render states.
/// `D3DRS_ZENABLE`
pub const D3DRS_ZENABLE: u32 = 7;
/// `D3DRS_FILLMODE`
pub const D3DRS_FILLMODE: u32 = 8;
/// `D3DRS_SHADEMODE`
pub const D3DRS_SHADEMODE: u32 = 9;
/// `D3DRS_ZWRITEENABLE`
pub const D3DRS_ZWRITEENABLE: u32 = 14;
/// `D3DRS_ALPHATESTENABLE`
pub const D3DRS_ALPHATESTENABLE: u32 = 15;
/// `D3DRS_SRCBLEND`
pub const D3DRS_SRCBLEND: u32 = 19;
/// `D3DRS_DESTBLEND`
pub const D3DRS_DESTBLEND: u32 = 20;
/// `D3DRS_CULLMODE`
pub const D3DRS_CULLMODE: u32 = 22;
/// `D3DRS_ZFUNC`
pub const D3DRS_ZFUNC: u32 = 23;
/// `D3DRS_ALPHAREF`
pub const D3DRS_ALPHAREF: u32 = 24;
/// `D3DRS_ALPHAFUNC`
pub const D3DRS_ALPHAFUNC: u32 = 25;
/// `D3DRS_DITHERENABLE`
pub const D3DRS_DITHERENABLE: u32 = 26;
/// `D3DRS_ALPHABLENDENABLE`
pub const D3DRS_ALPHABLENDENABLE: u32 = 27;
/// `D3DRS_FOGENABLE`
pub const D3DRS_FOGENABLE: u32 = 28;
/// `D3DRS_SPECULARENABLE`
pub const D3DRS_SPECULARENABLE: u32 = 29;
/// `D3DRS_FOGCOLOR`
pub const D3DRS_FOGCOLOR: u32 = 34;
/// `D3DRS_FOGTABLEMODE`
pub const D3DRS_FOGTABLEMODE: u32 = 35;
/// `D3DRS_FOGSTART`
pub const D3DRS_FOGSTART: u32 = 36;
/// `D3DRS_FOGEND`
pub const D3DRS_FOGEND: u32 = 37;
/// `D3DRS_RANGEFOGENABLE`
pub const D3DRS_RANGEFOGENABLE: u32 = 48;
/// `D3DRS_STENCILENABLE`
pub const D3DRS_STENCILENABLE: u32 = 52;
/// `D3DRS_STENCILFAIL`
pub const D3DRS_STENCILFAIL: u32 = 53;
/// `D3DRS_STENCILZFAIL`
pub const D3DRS_STENCILZFAIL: u32 = 54;
/// `D3DRS_STENCILPASS`
pub const D3DRS_STENCILPASS: u32 = 55;
/// `D3DRS_STENCILFUNC`
pub const D3DRS_STENCILFUNC: u32 = 56;
/// `D3DRS_STENCILREF`
pub const D3DRS_STENCILREF: u32 = 57;
/// `D3DRS_STENCILMASK`
pub const D3DRS_STENCILMASK: u32 = 58;
/// `D3DRS_TEXTUREFACTOR`
pub const D3DRS_TEXTUREFACTOR: u32 = 60;
/// `D3DRS_WRAP0`
pub const D3DRS_WRAP0: u32 = 128;
/// `D3DRS_WRAP1`
pub const D3DRS_WRAP1: u32 = 129;
/// `D3DRS_LIGHTING`
pub const D3DRS_LIGHTING: u32 = 137;
/// `D3DRS_AMBIENT`
pub const D3DRS_AMBIENT: u32 = 139;
/// `D3DRS_FOGVERTEXMODE`
pub const D3DRS_FOGVERTEXMODE: u32 = 140;
/// `D3DRS_DIFFUSEMATERIALSOURCE`
pub const D3DRS_DIFFUSEMATERIALSOURCE: u32 = 145;
/// `D3DRS_AMBIENTMATERIALSOURCE`
pub const D3DRS_AMBIENTMATERIALSOURCE: u32 = 147;
/// `D3DRS_EMISSIVEMATERIALSOURCE`
pub const D3DRS_EMISSIVEMATERIALSOURCE: u32 = 148;
/// `D3DRS_VERTEXBLEND`
pub const D3DRS_VERTEXBLEND: u32 = 151;
/// `D3DRS_MULTISAMPLEANTIALIAS`
pub const D3DRS_MULTISAMPLEANTIALIAS: u32 = 161;

// D3DTSS: the texture stage states.
/// `D3DTSS_COLOROP`
pub const D3DTSS_COLOROP: u32 = 1;
/// `D3DTSS_COLORARG1`
pub const D3DTSS_COLORARG1: u32 = 2;
/// `D3DTSS_COLORARG2`
pub const D3DTSS_COLORARG2: u32 = 3;
/// `D3DTSS_ALPHAOP`
pub const D3DTSS_ALPHAOP: u32 = 4;
/// `D3DTSS_ALPHAARG1`
pub const D3DTSS_ALPHAARG1: u32 = 5;
/// `D3DTSS_ALPHAARG2`
pub const D3DTSS_ALPHAARG2: u32 = 6;
/// `D3DTSS_BUMPENVMAT00`
pub const D3DTSS_BUMPENVMAT00: u32 = 7;
/// `D3DTSS_BUMPENVMAT01`
pub const D3DTSS_BUMPENVMAT01: u32 = 8;
/// `D3DTSS_BUMPENVMAT10`
pub const D3DTSS_BUMPENVMAT10: u32 = 9;
/// `D3DTSS_BUMPENVMAT11`
pub const D3DTSS_BUMPENVMAT11: u32 = 10;
/// `D3DTSS_TEXCOORDINDEX`
pub const D3DTSS_TEXCOORDINDEX: u32 = 11;
/// `D3DTSS_ADDRESSU`
pub const D3DTSS_ADDRESSU: u32 = 13;
/// `D3DTSS_ADDRESSV`
pub const D3DTSS_ADDRESSV: u32 = 14;
/// `D3DTSS_MAGFILTER`
pub const D3DTSS_MAGFILTER: u32 = 16;
/// `D3DTSS_MINFILTER`
pub const D3DTSS_MINFILTER: u32 = 17;
/// `D3DTSS_MIPFILTER`
pub const D3DTSS_MIPFILTER: u32 = 18;
/// `D3DTSS_BUMPENVLSCALE`
pub const D3DTSS_BUMPENVLSCALE: u32 = 22;
/// `D3DTSS_BUMPENVLOFFSET`
pub const D3DTSS_BUMPENVLOFFSET: u32 = 23;
/// `D3DTSS_TEXTURETRANSFORMFLAGS`
pub const D3DTSS_TEXTURETRANSFORMFLAGS: u32 = 24;
/// `D3DTSS_TCI_CAMERASPACEPOSITION`
pub const D3DTSS_TCI_CAMERASPACEPOSITION: u32 = 0x20000;

// D3DTS: the transform states.
/// `D3DTS_VIEW`
pub const D3DTS_VIEW: u32 = 2;
/// `D3DTS_PROJECTION`
pub const D3DTS_PROJECTION: u32 = 3;
/// `D3DTS_TEXTURE0`
pub const D3DTS_TEXTURE0: u32 = 16;
/// `D3DTS_WORLDMATRIX0`
pub const D3DTS_WORLDMATRIX0: u32 = 256;

// D3DPT: the primitive types.
/// `D3DPT_TRIANGLELIST`
pub const D3DPT_TRIANGLELIST: u32 = 4;
/// `D3DPT_TRIANGLESTRIP`
pub const D3DPT_TRIANGLESTRIP: u32 = 5;
/// `D3DPT_TRIANGLEFAN`
pub const D3DPT_TRIANGLEFAN: u32 = 6;

// D3DFVF: the bits of a flexible vertex format.
/// `D3DFVF_XYZ`
pub const D3DFVF_XYZ: u32 = 0x002;
/// `D3DFVF_NORMAL`
pub const D3DFVF_NORMAL: u32 = 0x010;
/// `D3DFVF_DIFFUSE`
pub const D3DFVF_DIFFUSE: u32 = 0x040;
/// `D3DFVF_TEX1`
pub const D3DFVF_TEX1: u32 = 0x100;

// D3DFMT: the formats. A DXT format is its FOURCC.
/// `D3DFMT_R8G8B8`
pub const D3DFMT_R8G8B8: u32 = 20;
/// `D3DFMT_A8R8G8B8`
pub const D3DFMT_A8R8G8B8: u32 = 21;
/// `D3DFMT_X8R8G8B8`
pub const D3DFMT_X8R8G8B8: u32 = 22;
/// `D3DFMT_R5G6B5`
pub const D3DFMT_R5G6B5: u32 = 23;
/// `D3DFMT_X1R5G5B5`
pub const D3DFMT_X1R5G5B5: u32 = 24;
/// `D3DFMT_A1R5G5B5`
pub const D3DFMT_A1R5G5B5: u32 = 25;
/// `D3DFMT_A4R4G4B4`
pub const D3DFMT_A4R4G4B4: u32 = 26;
/// `D3DFMT_P8`
pub const D3DFMT_P8: u32 = 41;
/// `D3DFMT_V8U8`
pub const D3DFMT_V8U8: u32 = 60;
/// `D3DFMT_L6V5U5`
pub const D3DFMT_L6V5U5: u32 = 61;
/// `D3DFMT_X8L8V8U8`
pub const D3DFMT_X8L8V8U8: u32 = 62;
/// `D3DFMT_D15S1`
pub const D3DFMT_D15S1: u32 = 73;
/// `D3DFMT_D24S8`
pub const D3DFMT_D24S8: u32 = 75;
/// `D3DFMT_D24X4S4`
pub const D3DFMT_D24X4S4: u32 = 79;
/// `D3DFMT_VERTEXDATA`
pub const D3DFMT_VERTEXDATA: u32 = 100;
/// `D3DFMT_INDEX16`
pub const D3DFMT_INDEX16: u32 = 101;
/// `D3DFMT_DXT1`
pub const D3DFMT_DXT1: u32 = 0x3154_5844;
/// `D3DFMT_DXT2`
pub const D3DFMT_DXT2: u32 = 0x3254_5844;
/// `D3DFMT_DXT3`
pub const D3DFMT_DXT3: u32 = 0x3354_5844;
/// `D3DFMT_DXT4`
pub const D3DFMT_DXT4: u32 = 0x3454_5844;
/// `D3DFMT_DXT5`
pub const D3DFMT_DXT5: u32 = 0x3554_5844;
/// `D3DFMT_YUY2`
pub const D3DFMT_YUY2: u32 = 0x3259_5559;
/// `D3DFMT_UYVY`
pub const D3DFMT_UYVY: u32 = 0x5956_5955;

// D3DERR: the results that the ports test.
/// `D3DERR_DEVICELOST`
pub const D3DERR_DEVICELOST: u32 = 0x8876_0868;
/// `D3DERR_DEVICENOTRESET`
pub const D3DERR_DEVICENOTRESET: u32 = 0x8876_0869;

// Other enumerations.
/// `D3DPOOL_DEFAULT`
pub const D3DPOOL_DEFAULT: u32 = 0;
/// `D3DPOOL_MANAGED`
pub const D3DPOOL_MANAGED: u32 = 1;
/// `D3DUSAGE_RENDERTARGET`
pub const D3DUSAGE_RENDERTARGET: u32 = 1;
/// `D3DLOCK_READONLY`
pub const D3DLOCK_READONLY: u32 = 0x10;
/// `D3DDEVTYPE_HAL`
pub const D3DDEVTYPE_HAL: u32 = 1;
/// `D3DDEVTYPE_REF`
pub const D3DDEVTYPE_REF: u32 = 2;
/// `D3DCREATE_MULTITHREADED`
pub const D3DCREATE_MULTITHREADED: u32 = 4;
/// `D3DRTYPE_INDEXBUFFER`
pub const D3DRTYPE_INDEXBUFFER: u32 = 7;
/// `D3DCMP_ALWAYS`
pub const D3DCMP_ALWAYS: u32 = 8;
/// `D3DFOG_LINEAR`
pub const D3DFOG_LINEAR: u32 = 3;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::NAMES;
    use std::cell::RefCell;

    thread_local! {
        /// The object and the arguments of each call that a fake method got.
        static CALLS: RefCell<Vec<Vec<u32>>> = const { RefCell::new(Vec::new()) };
    }

    fn note(values: &[u32]) -> u32 {
        CALLS.with(|calls| calls.borrow_mut().push(values.to_vec()));
        0x8000_0000 | (values.len() as u32 - 1)
    }

    extern "system" fn m0(this: usize) -> u32 {
        note(&[this as u32])
    }
    extern "system" fn m1(this: usize, a: u32) -> u32 {
        note(&[this as u32, a])
    }
    extern "system" fn m2(this: usize, a: u32, b: u32) -> u32 {
        note(&[this as u32, a, b])
    }
    extern "system" fn m3(this: usize, a: u32, b: u32, c: u32) -> u32 {
        note(&[this as u32, a, b, c])
    }
    extern "system" fn m4(this: usize, a: u32, b: u32, c: u32, d: u32) -> u32 {
        note(&[this as u32, a, b, c, d])
    }
    extern "system" fn m5(this: usize, a: u32, b: u32, c: u32, d: u32, e: u32) -> u32 {
        note(&[this as u32, a, b, c, d, e])
    }
    extern "system" fn m6(this: usize, a: u32, b: u32, c: u32, d: u32, e: u32, f: u32) -> u32 {
        note(&[this as u32, a, b, c, d, e, f])
    }
    extern "system" fn m7(this: usize, a: u32, b: u32, c: u32, d: u32, e: u32, f: u32, g: u32) -> u32 {
        note(&[this as u32, a, b, c, d, e, f, g])
    }

    /// The method of a second vtable.
    extern "system" fn other(_this: usize, _a: u32, _b: u32) -> u32 {
        0x55
    }

    /// The address of the fake method with `count` arguments after the object.
    fn method(count: usize) -> usize {
        match count {
            0 => m0 as *const () as usize,
            1 => m1 as *const () as usize,
            2 => m2 as *const () as usize,
            3 => m3 as *const () as usize,
            4 => m4 as *const () as usize,
            5 => m5 as *const () as usize,
            6 => m6 as *const () as usize,
            _ => m7 as *const () as usize,
        }
    }

    /// Makes an object with a vtable that has one method only, at `slot`. Each other slot is 0,
    /// thus a call through a wrong slot is a fault. The test does `call` and compares the
    /// arguments that the method got with 1, 2, 3 and so on.
    fn check(slot: usize, count: usize, call: impl FnOnce(usize) -> u32) {
        let mut vtable = [0usize; 97];
        vtable[slot / 4] = method(count);
        let object = [vtable.as_ptr() as usize];
        let o = object.as_ptr() as usize;
        CALLS.with(|calls| calls.borrow_mut().clear());
        assert_eq!(call(o), 0x8000_0000 | count as u32, "the result at slot {slot:#x}");
        let mut expected = vec![o as u32];
        expected.extend(1..=count as u32);
        assert_eq!(CALLS.with(|calls| calls.borrow().clone()), [expected], "the arguments at slot {slot:#x}");
    }

    #[test]
    fn com_and_direct3d_calls_use_their_slots() {
        unsafe {
            check(COM_ADD_REF, 0, |o| com_add_ref(o));
            check(COM_RELEASE, 0, |o| com_release(o));
            check(D3D_GET_ADAPTER_COUNT, 0, |o| d3d_get_adapter_count(o));
            check(D3D_GET_ADAPTER_IDENTIFIER, 3, |o| d3d_get_adapter_identifier(o, 1, 2, 3));
            check(D3D_GET_ADAPTER_MODE_COUNT, 1, |o| d3d_get_adapter_mode_count(o, 1));
            check(D3D_ENUM_ADAPTER_MODES, 3, |o| d3d_enum_adapter_modes(o, 1, 2, 3));
            check(D3D_GET_ADAPTER_DISPLAY_MODE, 2, |o| d3d_get_adapter_display_mode(o, 1, 2));
            check(D3D_CHECK_DEVICE_TYPE, 5, |o| d3d_check_device_type(o, 1, 2, 3, 4, 5));
            check(D3D_CHECK_DEVICE_FORMAT, 6, |o| d3d_check_device_format(o, 1, 2, 3, 4, 5, 6));
            check(D3D_CHECK_DEVICE_MULTI_SAMPLE_TYPE, 5, |o| d3d_check_device_multi_sample_type(o, 1, 2, 3, 4, 5));
            check(D3D_CHECK_DEPTH_STENCIL_MATCH, 5, |o| d3d_check_depth_stencil_match(o, 1, 2, 3, 4, 5));
            check(D3D_GET_DEVICE_CAPS, 3, |o| d3d_get_device_caps(o, 1, 2, 3));
            check(D3D_CREATE_DEVICE, 6, |o| d3d_create_device(o, 1, 2, 3, 4, 5, 6));
        }
    }

    #[test]
    fn device_calls_use_their_slots() {
        unsafe {
            check(DEVICE_ADD_REF, 0, |o| device_add_ref(o));
            check(DEVICE_RELEASE, 0, |o| device_release(o));
            check(DEVICE_TEST_COOPERATIVE_LEVEL, 0, |o| device_test_cooperative_level(o));
            check(DEVICE_RESOURCE_MANAGER_DISCARD_BYTES, 1, |o| device_resource_manager_discard_bytes(o, 1));
            check(DEVICE_RESET, 1, |o| device_reset(o, 1));
            check(DEVICE_PRESENT, 4, |o| device_present(o, 1, 2, 3, 4));
            check(DEVICE_CREATE_TEXTURE, 7, |o| device_create_texture(o, 1, 2, 3, 4, 5, 6, 7));
            check(DEVICE_CREATE_CUBE_TEXTURE, 6, |o| device_create_cube_texture(o, 1, 2, 3, 4, 5, 6));
            check(DEVICE_CREATE_VERTEX_BUFFER, 5, |o| device_create_vertex_buffer(o, 1, 2, 3, 4, 5));
            check(DEVICE_CREATE_INDEX_BUFFER, 5, |o| device_create_index_buffer(o, 1, 2, 3, 4, 5));
            check(DEVICE_CREATE_IMAGE_SURFACE, 4, |o| device_create_image_surface(o, 1, 2, 3, 4));
            check(DEVICE_COPY_RECTS, 5, |o| device_copy_rects(o, 1, 2, 3, 4, 5));
            check(DEVICE_SET_RENDER_TARGET, 2, |o| device_set_render_target(o, 1, 2));
            check(DEVICE_GET_RENDER_TARGET, 1, |o| device_get_render_target(o, 1));
            check(DEVICE_GET_DEPTH_STENCIL_SURFACE, 1, |o| device_get_depth_stencil_surface(o, 1));
            check(DEVICE_BEGIN_SCENE, 0, |o| device_begin_scene(o));
            check(DEVICE_END_SCENE, 0, |o| device_end_scene(o));
            check(DEVICE_CLEAR, 6, |o| device_clear(o, 1, 2, 3, 4, 5, 6));
            check(DEVICE_SET_TRANSFORM, 2, |o| device_set_transform(o, 1, 2));
            check(DEVICE_SET_VIEWPORT, 1, |o| device_set_viewport(o, 1));
            check(DEVICE_SET_LIGHT, 2, |o| device_set_light(o, 1, 2));
            check(DEVICE_LIGHT_ENABLE, 2, |o| device_light_enable(o, 1, 2));
            check(DEVICE_SET_RENDER_STATE, 2, |o| device_set_render_state(o, 1, 2));
            check(DEVICE_SET_TEXTURE, 2, |o| device_set_texture(o, 1, 2));
            check(DEVICE_SET_TEXTURE_STAGE_STATE, 3, |o| device_set_texture_stage_state(o, 1, 2, 3));
            check(DEVICE_DRAW_PRIMITIVE, 3, |o| device_draw_primitive(o, 1, 2, 3));
            check(DEVICE_DRAW_INDEXED_PRIMITIVE, 5, |o| device_draw_indexed_primitive(o, 1, 2, 3, 4, 5));
            check(DEVICE_SET_VERTEX_SHADER, 1, |o| device_set_vertex_shader(o, 1));
            check(DEVICE_SET_STREAM_SOURCE, 3, |o| device_set_stream_source(o, 1, 2, 3));
            check(DEVICE_SET_INDICES, 2, |o| device_set_indices(o, 1, 2));
            check(DEVICE_SET_PIXEL_SHADER, 1, |o| device_set_pixel_shader(o, 1));
        }
    }

    #[test]
    fn buffer_texture_and_surface_calls_use_their_slots() {
        unsafe {
            check(BUFFER_LOCK, 4, |o| buffer_lock(o, 1, 2, 3, 4));
            check(BUFFER_UNLOCK, 0, |o| buffer_unlock(o));
            check(BUFFER_GET_DESC, 1, |o| buffer_get_desc(o, 1));
            check(TEXTURE_GET_SURFACE_LEVEL, 2, |o| texture_get_surface_level(o, 1, 2));
            check(CUBE_TEXTURE_GET_CUBE_MAP_SURFACE, 3, |o| cube_texture_get_cube_map_surface(o, 1, 2, 3));
            check(SURFACE_GET_DESC, 1, |o| surface_get_desc(o, 1));
            check(SURFACE_LOCK_RECT, 3, |o| surface_lock_rect(o, 1, 2, 3));
            check(SURFACE_UNLOCK_RECT, 0, |o| surface_unlock_rect(o));
        }
    }

    #[test]
    fn device_slots_agree_with_the_method_table_of_the_recorder() {
        assert_eq!(NAMES[DEVICE_ADD_REF / 4], "AddRef");
        assert_eq!(NAMES[DEVICE_RELEASE / 4], "Release");
        assert_eq!(NAMES[DEVICE_TEST_COOPERATIVE_LEVEL / 4], "TestCooperativeLevel");
        assert_eq!(NAMES[DEVICE_RESOURCE_MANAGER_DISCARD_BYTES / 4], "ResourceManagerDiscardBytes");
        assert_eq!(NAMES[DEVICE_RESET / 4], "Reset");
        assert_eq!(NAMES[DEVICE_PRESENT / 4], "Present");
        assert_eq!(NAMES[DEVICE_CREATE_TEXTURE / 4], "CreateTexture");
        assert_eq!(NAMES[DEVICE_CREATE_CUBE_TEXTURE / 4], "CreateCubeTexture");
        assert_eq!(NAMES[DEVICE_CREATE_VERTEX_BUFFER / 4], "CreateVertexBuffer");
        assert_eq!(NAMES[DEVICE_CREATE_INDEX_BUFFER / 4], "CreateIndexBuffer");
        assert_eq!(NAMES[DEVICE_CREATE_IMAGE_SURFACE / 4], "CreateImageSurface");
        assert_eq!(NAMES[DEVICE_COPY_RECTS / 4], "CopyRects");
        assert_eq!(NAMES[DEVICE_SET_RENDER_TARGET / 4], "SetRenderTarget");
        assert_eq!(NAMES[DEVICE_GET_RENDER_TARGET / 4], "GetRenderTarget");
        assert_eq!(NAMES[DEVICE_GET_DEPTH_STENCIL_SURFACE / 4], "GetDepthStencilSurface");
        assert_eq!(NAMES[DEVICE_BEGIN_SCENE / 4], "BeginScene");
        assert_eq!(NAMES[DEVICE_END_SCENE / 4], "EndScene");
        assert_eq!(NAMES[DEVICE_CLEAR / 4], "Clear");
        assert_eq!(NAMES[DEVICE_SET_TRANSFORM / 4], "SetTransform");
        assert_eq!(NAMES[DEVICE_SET_VIEWPORT / 4], "SetViewport");
        assert_eq!(NAMES[DEVICE_SET_LIGHT / 4], "SetLight");
        assert_eq!(NAMES[DEVICE_LIGHT_ENABLE / 4], "LightEnable");
        assert_eq!(NAMES[DEVICE_SET_RENDER_STATE / 4], "SetRenderState");
        assert_eq!(NAMES[DEVICE_SET_TEXTURE / 4], "SetTexture");
        assert_eq!(NAMES[DEVICE_SET_TEXTURE_STAGE_STATE / 4], "SetTextureStageState");
        assert_eq!(NAMES[DEVICE_DRAW_PRIMITIVE / 4], "DrawPrimitive");
        assert_eq!(NAMES[DEVICE_DRAW_INDEXED_PRIMITIVE / 4], "DrawIndexedPrimitive");
        assert_eq!(NAMES[DEVICE_SET_VERTEX_SHADER / 4], "SetVertexShader");
        assert_eq!(NAMES[DEVICE_SET_STREAM_SOURCE / 4], "SetStreamSource");
        assert_eq!(NAMES[DEVICE_SET_INDICES / 4], "SetIndices");
        assert_eq!(NAMES[DEVICE_SET_PIXEL_SHADER / 4], "SetPixelShader");
    }

    #[test]
    fn a_call_reads_the_vtable_again_after_a_change() {
        let mut first = [0usize; 97];
        first[DEVICE_SET_TEXTURE / 4] = method(2);
        let mut second = [0usize; 97];
        second[DEVICE_SET_TEXTURE / 4] = other as *const () as usize;
        let mut object = [first.as_ptr() as usize];
        let o = object.as_mut_ptr() as usize;
        unsafe {
            assert_eq!(device_set_texture(o, 0, 0), 0x8000_0002);
            // The recorder puts another vtable into the device.
            crate::mem::wr(o, 0, second.as_ptr() as usize);
            assert_eq!(device_set_texture(o, 0, 0), 0x55);
        }
    }

    #[test]
    fn the_deliberate_fault_changes_the_z_compare_function_only() {
        let mut vtable = [0usize; 97];
        vtable[DEVICE_SET_RENDER_STATE / 4] = method(2);
        let object = [vtable.as_ptr() as usize];
        let o = object.as_ptr() as usize;
        CALLS.with(|calls| calls.borrow_mut().clear());
        unsafe {
            device_set_render_state(o, D3DRS_ZFUNC, 4);
            device_set_render_state(o, D3DRS_ALPHAFUNC, 4);
        }
        // The fault is off in the unit tests, thus the two values go to the device as they are.
        assert_eq!(fault(), 0);
        assert_eq!(CALLS.with(|calls| calls.borrow().clone()), [[o as u32, 23, 4], [o as u32, 25, 4]]);
    }
}
