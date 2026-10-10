//! The Lua side: `include("nirender")` returns the table built in `luaopen_nirender`.

use core::ffi::{c_char, c_void};
use core::sync::atomic::Ordering;
use std::ffi::CString;

use crate::gen::layouts::NiDX8Renderer;
use crate::ports;
use crate::shadow::{self, Mode};
use crate::{census, device, engine, frame, hook, x87};

type LuaState = *mut c_void;
type LuaFn = unsafe extern "C" fn(LuaState) -> i32;

#[link(name = "lua51", kind = "raw-dylib")]
extern "C" {
    fn lua_createtable(l: LuaState, narr: i32, nrec: i32);
    fn lua_pushcclosure(l: LuaState, f: LuaFn, n: i32);
    fn lua_setfield(l: LuaState, index: i32, key: *const c_char);
    fn lua_pushlstring(l: LuaState, s: *const c_char, len: usize);
    fn lua_pushboolean(l: LuaState, b: i32);
    fn lua_pushnumber(l: LuaState, n: f64);
    fn lua_pushnil(l: LuaState);
    fn lua_tonumber(l: LuaState, index: i32) -> f64;
    fn lua_toboolean(l: LuaState, index: i32) -> i32;
    fn lua_tolstring(l: LuaState, index: i32, len: *mut usize) -> *const c_char;
}

unsafe fn push_str(l: LuaState, text: &str) {
    lua_pushlstring(l, text.as_ptr() as *const c_char, text.len());
}

unsafe fn arg_str(l: LuaState, index: i32) -> Option<String> {
    let mut len = 0usize;
    let ptr = lua_tolstring(l, index, &mut len);
    if ptr.is_null() {
        None
    } else {
        Some(String::from_utf8_lossy(core::slice::from_raw_parts(ptr as *const u8, len)).into_owned())
    }
}

unsafe fn set_fn(l: LuaState, name: &str, f: LuaFn) {
    let key = CString::new(name).unwrap();
    lua_pushcclosure(l, f, 0);
    lua_setfield(l, -2, key.as_ptr());
}

/// `nirender.install([logPath])` -> ok, message. Puts the recorder on the device and the
/// detours on the engine functions. Every port starts in mode "off".
unsafe extern "C" fn install(l: LuaState) -> i32 {
    if let Some(path) = arg_str(l, 1) {
        crate::log::open(&path);
    }
    let (ok, message) = match install_all() {
        Ok(message) => (true, message),
        Err(message) => (false, message),
    };
    crate::log!("install: {}", message);
    lua_pushboolean(l, ok as i32);
    push_str(l, &message);
    2
}

/// `data_IDirect3D8`: the `IDirect3D8` of the game, 0 until `NiDX8Renderer::CreateD3D` ran.
const DIRECT3D: usize = 0x7DD8C0;
const CREATE_D3D: usize = 0x6A99E0;

unsafe fn after_create_d3d() {
    device::attach_factory(crate::mem::rd(DIRECT3D, 0));
}

/// The census: marks the ports of functions another mod changed in a way the port does not
/// follow. With `hooked` the entries hold our own jumps, which are not a change.
unsafe fn census_pass(hooked: bool) -> usize {
    let changes = census::changes();
    let mut changed: Vec<usize> = Vec::new();
    for change in &changes {
        let port = ports::all().find(|p| p.addr == change.func.ea);
        if hooked && change.offset < 16 && port.map_or(false, |p| p.installed()) {
            continue;
        }
        changed.push(change.func.ea);
        let live = core::slice::from_raw_parts(change.func.ea as *const u8, change.func.size);
        let kind = crate::site::classify(change.func, live, change.offset, change.len);
        let served = matches!(kind, crate::site::Kind::RedirectedCall(site) if ports::serves_site(site))
            || ports::follows(change.func.ea + change.offset, change.len);
        let mut note = String::new();
        if let Some(port) = port {
            if served {
                note = " (the port follows the change)".into();
            } else {
                if !port.locked() && port.mode() != Mode::Off {
                    note = " (PORT LOCKED NOW; it ran as Rust before)".into();
                } else {
                    note = " (PORT LOCKED: it stays game code here)".into();
                }
                port.locked.store(true, Ordering::Relaxed);
                port.set_mode(Mode::Off);
            }
        }
        crate::log!("census: {}{}", census::describe(change), note);
    }
    changed.dedup();
    crate::log!("census: {} of {} functions differ from the database", changed.len(), crate::gen::funcs::FUNCS.len());
    changed.len()
}

unsafe fn install_all() -> Result<String, String> {
    if ports::all().any(|p| p.installed()) {
        return attach_now();
    }
    shadow::set_main_thread();
    crate::log!("x87 control word {:#06x}", x87::control_word());

    // Before the detours: they change the entry bytes themselves.
    let changed = census_pass(false);

    match crate::alloc::install() {
        Ok(()) => crate::log!("allocator hooks on: calls that allocate are compared"),
        Err(error) => crate::log!("no allocator hooks ({error}): calls that allocate are not compared"),
    }
    device::set_frame_hook(frame::on_present);

    let mut failed = 0;
    for port in ports::all() {
        match hook::install(port.addr, (port.detour)(), port.stack_args, if port.cdecl { hook::Cleanup::Caller } else { hook::Cleanup::Callee }) {
            Ok(trampoline) => port.orig.store(trampoline, Ordering::Relaxed),
            Err(error) => {
                failed += 1;
                crate::log!("no detour on {}: {}", port.name, error);
            }
        }
        if port.addr == CREATE_D3D {
            port.after.store(after_create_d3d as *const () as usize, Ordering::Relaxed);
        }
    }
    // The game may have its IDirect3D8, or even its device, already.
    after_create_d3d();
    let attached = attach_now()?;
    Ok(format!(
        "{} ports installed, {} failed, {} functions differ from the database; {}",
        ports::all().count() - failed,
        failed,
        changed,
        attached
    ))
}

/// Puts the recorder on the device of the renderer, when the game has one.
unsafe fn attach_now() -> Result<String, String> {
    let renderer = engine::renderer();
    if renderer == 0 {
        return Ok("the game has no renderer yet".into());
    }
    let device_ptr: usize = crate::mem::rd(renderer, NiDX8Renderer::d3dDevice);
    device::attach(device_ptr)?;
    Ok(format!("renderer {renderer:#x}, device {device_ptr:#x} with the recorder"))
}

/// `nirender.recheck()` -> number of changed functions. The census again, for mods that
/// change the game code after this plugin loaded. Ports of such functions get locked.
unsafe extern "C" fn recheck(l: LuaState) -> i32 {
    let attached = attach_now();
    crate::log!("recheck: {:?}", attached);
    lua_pushnumber(l, census_pass(true) as f64);
    1
}

/// `nirender.dumpState(path)` -> ok. The renderer objects, for the comparison of two runs.
unsafe extern "C" fn dump_state(l: LuaState) -> i32 {
    let ok = match arg_str(l, 1) {
        Some(path) => crate::dump::state(&path).is_ok(),
        None => false,
    };
    lua_pushboolean(l, ok as i32);
    1
}

/// `nirender.dumpFrame(path)` -> started. The device calls of the next frame go to the file;
/// `frameTestResult()` tells when it is written.
unsafe extern "C" fn dump_frame(l: LuaState) -> i32 {
    let started = match arg_str(l, 1) {
        Some(path) => frame::request_dump(path),
        None => false,
    };
    lua_pushboolean(l, started as i32);
    1
}

/// `nirender.setMode(pattern, mode)` -> number of ports set. `pattern` is "*" or a part of
/// the engine name; `mode` is "off", "shadow" or "live".
unsafe extern "C" fn set_mode(l: LuaState) -> i32 {
    let pattern = arg_str(l, 1).unwrap_or_default();
    let mode = arg_str(l, 2).and_then(|m| Mode::parse(&m));
    let mut count = 0;
    if let Some(mode) = mode {
        for port in ports::all() {
            if port.installed() && (pattern == "*" || port.name.contains(pattern.as_str())) {
                port.set_mode(mode);
                count += 1;
            }
        }
        crate::log!("mode {:?} for '{}': {} ports", mode, pattern, count);
    }
    lua_pushnumber(l, count as f64);
    1
}

/// One line per port: name, mode, calls compared, calls that differed, calls not compared,
/// calls that ran as Rust. Tab separated, for the log and for tools/status.py.
pub fn report() -> String {
    let mut text = String::from("port\taddr\tinstalled\tmode\tshadow_calls\tmismatches\tskipped\tlive_calls\tlocked\trust_runs\n");
    for port in ports::all() {
        text.push_str(&format!(
            "{}\t{:#x}\t{}\t{:?}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            port.name,
            port.addr,
            port.installed(),
            port.mode(),
            port.shadow_calls.load(Ordering::Relaxed),
            port.mismatches.load(Ordering::Relaxed),
            port.skipped.load(Ordering::Relaxed),
            port.live_calls.load(Ordering::Relaxed),
            port.locked(),
            port.rust_runs.load(Ordering::Relaxed),
        ));
    }
    text
}

/// `nirender.report()` -> the table as text. Also written to the log.
unsafe extern "C" fn report_lua(l: LuaState) -> i32 {
    let text = report();
    crate::log!("report:\n{}", text);
    push_str(l, &text);
    1
}

/// `nirender.frameTest(kind)` -> started. `kind` is "control" or "live".
unsafe extern "C" fn frame_test(l: LuaState) -> i32 {
    let kind = match arg_str(l, 1).as_deref() {
        Some("live") => frame::Kind::Live,
        _ => frame::Kind::Control,
    };
    lua_pushboolean(l, frame::request(kind) as i32);
    1
}

/// `nirender.frameTestResult()` -> the result line, or nil while the test runs.
unsafe extern "C" fn frame_test_result(l: LuaState) -> i32 {
    match frame::take_result() {
        Some(text) => push_str(l, &text),
        None => lua_pushnil(l),
    }
    1
}

/// `nirender.recordBuffers(on)`: also record what the engine writes into vertex and index buffers.
unsafe extern "C" fn record_buffers(l: LuaState) -> i32 {
    let on = lua_toboolean(l, 1) != 0;
    crate::resource::enable(on);
    crate::log!("buffer recording {}", if on { "on" } else { "off" });
    0
}

/// `nirender.fault(n)`: puts fault `n` into the Rust code (see `engine::FAULT`). 0 removes it.
unsafe extern "C" fn fault(l: LuaState) -> i32 {
    let n = lua_tonumber(l, 1) as u32;
    engine::FAULT.store(n, Ordering::Relaxed);
    crate::log!("fault {n} put into the Rust code on purpose");
    0
}

/// `nirender.renderer()` -> the address of the game's renderer, or 0 before it exists.
unsafe extern "C" fn renderer(l: LuaState) -> i32 {
    lua_pushnumber(l, engine::renderer() as f64);
    1
}

/// `nirender.fpuControlWord()` -> the x87 control word of the calling thread.
unsafe extern "C" fn fpu_control_word(l: LuaState) -> i32 {
    lua_pushnumber(l, x87::control_word() as f64);
    1
}

#[no_mangle]
pub unsafe extern "C" fn luaopen_nirender(l: LuaState) -> i32 {
    lua_createtable(l, 0, 8);
    set_fn(l, "install", install);
    set_fn(l, "setMode", set_mode);
    set_fn(l, "report", report_lua);
    set_fn(l, "frameTest", frame_test);
    set_fn(l, "frameTestResult", frame_test_result);
    set_fn(l, "fpuControlWord", fpu_control_word);
    set_fn(l, "recordBuffers", record_buffers);
    set_fn(l, "fault", fault);
    set_fn(l, "renderer", renderer);
    set_fn(l, "recheck", recheck);
    set_fn(l, "dumpState", dump_state);
    set_fn(l, "dumpFrame", dump_frame);
    push_str(l, env!("CARGO_PKG_VERSION"));
    lua_setfield(l, -2, b"version\0".as_ptr() as *const c_char);
    lua_pushnumber(l, ports::all().count() as f64);
    lua_setfield(l, -2, b"ports\0".as_ptr() as *const c_char);
    1
}
