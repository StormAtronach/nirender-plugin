//! The native ABI of MWSE (`MWSENativeApi.h` of the MWSE repository): the table through which
//! the plugin gets engine events, overrides and facts from MWSE, instead of hooks of its own.
//!
//! `tools/check_mwse_native.py` compares the numbers here with the header.

use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::win;

pub const NATIVE_API_VERSION: u32 = 1;

pub const NATIVE_OK: u32 = 0;
pub const NATIVE_UNKNOWN_ID: u32 = 1;
pub const NATIVE_TAKEN: u32 = 2;
pub const NATIVE_BYTES_CHANGED: u32 = 3;
pub const NATIVE_TOO_LATE: u32 = 4;
pub const NATIVE_WRONG_THREAD: u32 = 5;
pub const NATIVE_BAD_ARGUMENT: u32 = 6;

pub const NATIVE_EVENT_CAMERA_TRANSFORMS_UPDATED: u32 = 1;
pub const NATIVE_EVENT_UI_CONFIGURED: u32 = 2;
pub const NATIVE_EVENT_GAME_IDS_RESOLVED: u32 = 3;
pub const NATIVE_EVENT_ACCUMULATOR_FINISHED: u32 = 4;
pub const NATIVE_OVERRIDE_DYNAMIC_LIGHT_TEST: u32 = 1;
pub const NATIVE_OVERRIDE_CAMERA_ROOT_CULL_SHOW: u32 = 2;
pub const NATIVE_OVERRIDE_MAP_TILE_READ: u32 = 3;
pub const NATIVE_OVERRIDE_TEXTURE_PURGE: u32 = 4;
pub const NATIVE_OVERRIDE_MAP_CELL_PAINT: u32 = 5;
pub const NATIVE_OVERRIDE_SAVE_PICTURE: u32 = 6;
pub const NATIVE_OVERRIDE_RENDER_TARGET_LOCK: u32 = 7;
pub const NATIVE_OVERRIDE_RENDER_TARGET_UNLOCK: u32 = 8;
pub const NATIVE_OVERRIDE_FOG_RENDER: u32 = 9;
pub const NATIVE_OVERRIDE_FOG_BLOB: u32 = 10;
pub const NATIVE_OVERRIDE_LIGHT_ATTACH_RADIUS: u32 = 11;
pub const NATIVE_OVERRIDE_FRAME_TIMER: u32 = 12;
pub const NATIVE_REQUEST_UI_SCALE: u32 = 1;
pub const NATIVE_REQUEST_LIGHT_LIMIT: u32 = 2;
pub const NATIVE_REQUEST_RIPPLES: u32 = 3;
pub const NATIVE_REQUEST_INTRO_MOVIES: u32 = 4;
pub const NATIVE_REQUEST_LIGHT_PARTICLES_FIX: u32 = 5;
pub const NATIVE_REQUEST_SUN_GLARE: u32 = 6;
pub const NATIVE_QUERY_UI_SCALE: u32 = 1;

/// The name of the plugin in MWSE.log.
const MODULE: &[u8] = b"akulakhan\0";

pub type Listener = unsafe extern "C" fn(user: *mut c_void, packet: *const c_void);

#[repr(C)]
pub struct CameraTransformsUpdatedV1 {
    pub struct_size: u32,
    pub player_animation_controller: u32,
}

#[repr(C)]
pub struct AccumulatorFinishedV1 {
    pub struct_size: u32,
    pub accumulator: u32,
    pub camera: u32,
}

#[repr(C)]
pub struct UIScaleV1 {
    pub struct_size: u32,
    pub scale: f32,
    pub view_width: i32,
    pub view_height: i32,
    pub pick_ray_in_view_size: u32,
}

#[repr(C)]
pub struct ApiV1 {
    pub struct_size: u32,
    pub version: u32,
    pub mwse_version: *const u8,
    pub subscribe: unsafe extern "C" fn(module: *const u8, event_id: u32, listener: Listener, user: *mut c_void, priority: i32, handle: *mut u32) -> u32,
    pub unsubscribe: unsafe extern "C" fn(handle: u32),
    pub claim: unsafe extern "C" fn(module: *const u8, override_id: u32, function: *const c_void, engine_function: *mut *const c_void, handle: *mut u32) -> u32,
    pub release: unsafe extern "C" fn(handle: u32),
    pub request: unsafe extern "C" fn(module: *const u8, request_id: u32, packet: *const c_void) -> u32,
    pub query: unsafe extern "C" fn(query_id: u32, packet: *mut c_void) -> u32,
}

/// The table, or 0 before the first look, or 1 when MWSE has none.
static TABLE: AtomicUsize = AtomicUsize::new(0);

/// The table of MWSE, or `None` when the MWSE.dll of the game has no native ABI of version 1.
pub fn api() -> Option<&'static ApiV1> {
    let mut table = TABLE.load(Ordering::Relaxed);
    if table == 0 {
        table = unsafe { find_table() };
        TABLE.store(table, Ordering::Relaxed);
    }
    if table <= 1 {
        return None;
    }
    let api = unsafe { &*(table as *const ApiV1) };
    if (api.struct_size as usize) < core::mem::size_of::<ApiV1>() {
        return None;
    }
    Some(api)
}

unsafe fn find_table() -> usize {
    let name: Vec<u16> = "MWSE.dll\0".encode_utf16().collect();
    let module = win::GetModuleHandleW(name.as_ptr());
    if module.is_null() {
        return 1;
    }
    let export = win::GetProcAddress(module, b"MWSE_GetNativeApi\0".as_ptr());
    if export.is_null() {
        return 1;
    }
    let get: unsafe extern "C" fn(u32) -> *const ApiV1 = core::mem::transmute(export);
    let table = get(NATIVE_API_VERSION);
    if table.is_null() { 1 } else { table as usize }
}

/// The words of MWSE's answer.
pub fn status_text(status: u32) -> &'static str {
    match status {
        NATIVE_OK => "ok",
        NATIVE_UNKNOWN_ID => "this MWSE does not have it",
        NATIVE_TAKEN => "another module owns it (see MWSE.log)",
        NATIVE_BYTES_CHANGED => "the engine code is not what MWSE expects (see MWSE.log)",
        NATIVE_TOO_LATE => "too late",
        NATIVE_WRONG_THREAD => "not on the main thread",
        NATIVE_BAD_ARGUMENT => "a bad argument",
        _ => "an unknown answer",
    }
}

/// The text of a missing table, for a log line.
pub const NO_TABLE: &str = "the MWSE.dll of the game has no native ABI (MWSE_GetNativeApi version 1): it needs a newer MWSE";

/// Subscribes `listener` to an event of MWSE.
pub unsafe fn subscribe(event_id: u32, listener: Listener) -> Result<u32, String> {
    let api = api().ok_or_else(|| NO_TABLE.to_string())?;
    let mut handle = 0u32;
    let status = (api.subscribe)(MODULE.as_ptr(), event_id, listener, core::ptr::null_mut(), 0, &mut handle);
    if status == NATIVE_OK {
        Ok(handle)
    } else {
        Err(format!("MWSE event {event_id}: {}", status_text(status)))
    }
}

/// An override of MWSE that the plugin owns: the engine function, and the handle to give it
/// back.
#[derive(Clone, Copy)]
pub struct Claim {
    pub engine: usize,
    pub handle: u32,
}

/// Makes the plugin the owner of an override of MWSE.
pub unsafe fn claim(override_id: u32, function: usize) -> Result<Claim, String> {
    let api = api().ok_or_else(|| NO_TABLE.to_string())?;
    let mut handle = 0u32;
    let mut engine: *const c_void = core::ptr::null();
    let status = (api.claim)(MODULE.as_ptr(), override_id, function as *const c_void, &mut engine, &mut handle);
    if status == NATIVE_OK && !engine.is_null() {
        Ok(Claim { engine: engine as usize, handle })
    } else {
        Err(format!("MWSE override {override_id}: {}", status_text(status)))
    }
}

/// Makes the plugin the owner of several overrides of MWSE, all or none: a refusal gives the
/// ones before it back. Returns the claims in the order of `overrides`.
pub unsafe fn claim_all(overrides: &[(u32, usize)]) -> Result<Vec<Claim>, String> {
    let mut claims = Vec::with_capacity(overrides.len());
    for &(id, function) in overrides {
        match claim(id, function) {
            Ok(claim) => claims.push(claim),
            Err(error) => {
                for claim in claims {
                    release(claim);
                }
                return Err(error);
            }
        }
    }
    Ok(claims)
}

/// Gives an override back to the engine function.
pub unsafe fn release(claim: Claim) {
    if let Some(api) = api() {
        (api.release)(claim.handle);
    }
}

/// The scale of the UI from MWSE, or `None` when MWSE has no native ABI.
pub unsafe fn ui_scale() -> Option<UIScaleV1> {
    let api = api()?;
    let mut packet = UIScaleV1 { struct_size: core::mem::size_of::<UIScaleV1>() as u32, scale: 1.0, view_width: 0, view_height: 0, pick_ray_in_view_size: 0 };
    ((api.query)(NATIVE_QUERY_UI_SCALE, &mut packet as *mut UIScaleV1 as *mut c_void) == NATIVE_OK).then_some(packet)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_packets_have_the_sizes_of_the_header() {
        assert_eq!(core::mem::size_of::<CameraTransformsUpdatedV1>(), 8);
        assert_eq!(core::mem::size_of::<UIScaleV1>(), 20);
    }
}
