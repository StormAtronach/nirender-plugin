//! The Win32 calls that the plugin needs.
#![allow(non_snake_case)]

use core::ffi::c_void;

pub const PAGE_EXECUTE_READWRITE: u32 = 0x40;
pub const MEM_COMMIT_RESERVE: u32 = 0x3000;

#[link(name = "kernel32")]
extern "system" {
    pub fn VirtualProtect(addr: *mut c_void, size: usize, new_protect: u32, old_protect: *mut u32) -> i32;
    pub fn VirtualAlloc(addr: *mut c_void, size: usize, alloc_type: u32, protect: u32) -> *mut c_void;
    pub fn FlushInstructionCache(process: *mut c_void, addr: *const c_void, size: usize) -> i32;
    pub fn GetCurrentProcess() -> *mut c_void;
    pub fn GetCurrentThreadId() -> u32;
    pub fn GetModuleFileNameW(module: *mut c_void, name: *mut u16, size: u32) -> u32;
    pub fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    pub fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

/// Writes `bytes` over code at `addr`.
pub unsafe fn patch(addr: usize, bytes: &[u8]) -> bool {
    let mut old = 0u32;
    if VirtualProtect(addr as *mut c_void, bytes.len(), PAGE_EXECUTE_READWRITE, &mut old) == 0 {
        return false;
    }
    core::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, bytes.len());
    let mut ignored = 0u32;
    VirtualProtect(addr as *mut c_void, bytes.len(), old, &mut ignored);
    FlushInstructionCache(GetCurrentProcess(), addr as *const c_void, bytes.len());
    true
}
