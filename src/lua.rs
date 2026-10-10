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

unsafe fn install_all() -> Result<String, String> {
    if ports::all().any(|p| p.installed()) {
        return Ok("already installed".into());
    }
    let renderer = engine::renderer();
    if renderer == 0 {
        return Err("the game has no renderer yet".into());
    }
    let device_ptr: usize = crate::mem::rd(renderer, NiDX8Renderer::d3dDevice);
    shadow::set_main_thread();
    crate::log!("renderer {renderer:#x}, device {device_ptr:#x}, x87 control word {:#06x}", x87::control_word());

    // Before the detours: they change the entry bytes themselves.
    let changes = census::changes();
    let mut changed: Vec<usize> = changes.iter().map(|c| c.func.ea).collect();
    changed.dedup();
    crate::log!("census: {} of {} functions differ from the database", changed.len(), crate::gen::funcs::FUNCS.len());
    for change in &changes {
        let ported = ports::all().any(|p| p.addr == change.func.ea);
        crate::log!("census: {}{}", census::describe(change), if ported { " (PORTED)" } else { "" });
    }

    match crate::alloc::install() {
        Ok(()) => crate::log!("allocator hooks on: calls that allocate are compared"),
        Err(error) => crate::log!("no allocator hooks ({error}): calls that allocate are not compared"),
    }
    device::attach(device_ptr)?;
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
    }
    Ok(format!("{} ports installed, {} failed, {} functions differ from the database", ports::all().count() - failed, failed, changed.len()))
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
    let mut text = String::from("port\taddr\tinstalled\tmode\tshadow_calls\tmismatches\tskipped\tlive_calls\n");
    for port in ports::all() {
        text.push_str(&format!(
            "{}\t{:#x}\t{}\t{:?}\t{}\t{}\t{}\t{}\n",
            port.name,
            port.addr,
            port.installed(),
            port.mode(),
            port.shadow_calls.load(Ordering::Relaxed),
            port.mismatches.load(Ordering::Relaxed),
            port.skipped.load(Ordering::Relaxed),
            port.live_calls.load(Ordering::Relaxed),
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
    push_str(l, env!("CARGO_PKG_VERSION"));
    lua_setfield(l, -2, b"version\0".as_ptr() as *const c_char);
    lua_pushnumber(l, ports::all().count() as f64);
    lua_setfield(l, -2, b"ports\0".as_ptr() as *const c_char);
    1
}
