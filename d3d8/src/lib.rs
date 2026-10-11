//! The `d3d8.dll` for a game without a Direct3D device.
//!
//! The game loads the `d3d8.dll` of its folder and calls `Direct3DCreate8`. This one loads
//! the nirender plugin and gives the call to it: the game gets the Direct3D 8 of the plugin,
//! which makes no device of Direct3D. See `src/own_device.rs` of the plugin.
//!
//! The file takes the place of the `d3d8.dll` of MGE XE. The `dinput8.dll` of MGE XE asks the
//! `d3d8.dll` of the game folder for `DirectInput8Create`, so this file has that function
//! too: it gives the call to the DirectInput of Windows. MGE XE also loads the script
//! extender, `MWSE.dll`, and it turns the films of the start off when `mgeXE.toml` says so;
//! this file does the two as well.

use core::ffi::c_void;

extern "system" {
    fn LoadLibraryA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn GetModuleFileNameA(module: *mut c_void, name: *mut u8, size: u32) -> u32;
    fn GetSystemDirectoryA(buffer: *mut u8, size: u32) -> u32;
    fn OutputDebugStringA(text: *const u8);
    fn VirtualProtect(address: *mut c_void, size: usize, protect: u32, old: *mut u32) -> i32;
}

/// The two places in Morrowind.exe where MGE XE turns the films of the start off
/// (`MWPatches::disableIntroMovies`): the film of the company and the check for the film of
/// the game. Each gets a short jump over the code that plays the film.
const FILM_PATCHES: [usize; 2] = [0x418EF0, 0x5FC8F7];
const FILM_JUMP: [u8; 2] = [0xEB, 0x16];

/// True when `mgeXE.toml` of the game folder has `skip_intro = true`. This is the setting of
/// MGE XE (`runtime.skip_intro`), so the game starts in the same way with either file.
unsafe fn skip_films_wanted() -> bool {
    let path = beside_program(b"mgeXE.toml");
    let Ok(text) = std::fs::read_to_string(String::from_utf8_lossy(&path).as_ref()) else { return false };
    text.lines().any(|line| {
        let line = line.split('#').next().unwrap_or("");
        let mut parts = line.splitn(2, '=');
        parts.next().map(str::trim) == Some("skip_intro") && parts.next().map(str::trim) == Some("true")
    })
}

/// Turns the films of the start off. Only in Morrowind.exe: the addresses are of that program.
unsafe fn skip_films() {
    let mut name = [0u8; 600];
    let length = GetModuleFileNameA(core::ptr::null_mut(), name.as_mut_ptr(), 512) as usize;
    if !name[..length].to_ascii_lowercase().ends_with(b"morrowind.exe") {
        return;
    }
    for address in FILM_PATCHES {
        let mut old = 0u32;
        // PAGE_EXECUTE_READWRITE
        if VirtualProtect(address as *mut c_void, 2, 0x40, &mut old) != 0 {
            core::ptr::copy_nonoverlapping(FILM_JUMP.as_ptr(), address as *mut u8, 2);
            let mut ignored = 0u32;
            VirtualProtect(address as *mut c_void, 2, old, &mut ignored);
        }
    }
}

/// The plugin, from the folder of the game.
const PLUGIN: &[u8] = b"Data Files\\MWSE\\lib\\nirender.dll\0";

/// The path of a file in the folder of the program, with a 0 at the end.
unsafe fn beside_program(file: &[u8]) -> Vec<u8> {
    let mut path = vec![0u8; 1024];
    let length = GetModuleFileNameA(core::ptr::null_mut(), path.as_mut_ptr(), 512) as usize;
    let folder = path[..length].iter().rposition(|c| *c == b'\\').map_or(0, |slash| slash + 1);
    path.truncate(folder);
    path.extend_from_slice(file);
    path
}

#[no_mangle]
pub unsafe extern "system" fn Direct3DCreate8(sdk_version: u32) -> *mut c_void {
    let plugin = LoadLibraryA(beside_program(PLUGIN).as_ptr());
    if plugin.is_null() {
        OutputDebugStringA(b"nirender d3d8.dll: no Data Files\\MWSE\\lib\\nirender.dll\n\0".as_ptr());
        return core::ptr::null_mut();
    }
    let create = GetProcAddress(plugin, b"nirender_direct3d_create8\0".as_ptr());
    if create.is_null() {
        OutputDebugStringA(b"nirender d3d8.dll: the plugin has no nirender_direct3d_create8\n\0".as_ptr());
        return core::ptr::null_mut();
    }
    let create: unsafe extern "C" fn(u32) -> *mut c_void = core::mem::transmute(create);
    create(sdk_version)
}

/// MGE XE loads the script extender when the game loads its `d3d8.dll`, and the script
/// extender then puts itself into the game. This file does the same: without the script
/// extender the plugin has no Lua code to start it.
#[no_mangle]
pub unsafe extern "system" fn DllMain(_module: *mut c_void, reason: u32, _reserved: *mut c_void) -> i32 {
    // DLL_PROCESS_ATTACH
    if reason == 1 && skip_films_wanted() {
        skip_films();
    }
    if reason == 1 && LoadLibraryA(beside_program(b"MWSE.dll\0").as_ptr()).is_null() {
        OutputDebugStringA(b"nirender d3d8.dll: no MWSE.dll in the game folder\n\0".as_ptr());
    }
    1
}

/// The plugin finds this function in the `d3d8.dll` of the game folder: the file is the one
/// of this repository. Returns the version of the file.
#[no_mangle]
pub extern "system" fn NirenderD3D8() -> u32 {
    1
}

/// E_FAIL
const FAIL: u32 = 0x8000_4005;

#[no_mangle]
pub unsafe extern "system" fn DirectInput8Create(instance: *mut c_void, version: u32, interface: *const c_void, out: *mut *mut c_void, outer: *mut c_void) -> u32 {
    let mut path = vec![0u8; 600];
    let length = GetSystemDirectoryA(path.as_mut_ptr(), 512) as usize;
    path.truncate(length);
    path.extend_from_slice(b"\\dinput8.dll\0");
    let module = LoadLibraryA(path.as_ptr());
    let create = if module.is_null() { core::ptr::null_mut() } else { GetProcAddress(module, b"DirectInput8Create\0".as_ptr()) };
    if create.is_null() {
        return FAIL;
    }
    let create: unsafe extern "system" fn(*mut c_void, u32, *const c_void, *mut *mut c_void, *mut c_void) -> u32 = core::mem::transmute(create);
    create(instance, version, interface, out, outer)
}
