//! The Lua interface. `include("nirender")` returns the table that `luaopen_nirender` builds.

use core::ffi::{c_char, c_void};
use core::sync::atomic::Ordering;
use std::ffi::CString;

use crate::ports;
use crate::shadow::Mode;
use crate::{engine, frame, install, x87};

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

/// `nirender.install([logPath])` returns ok and a message. It puts the recorder on the device
/// and the detours on the engine functions. Every port starts in mode "off".
unsafe extern "C" fn install(l: LuaState) -> i32 {
    if let Some(path) = arg_str(l, 1) {
        crate::log::open(&path);
    }
    let (ok, message) = match install::install() {
        Ok(message) => (true, message),
        Err(message) => (false, message),
    };
    crate::log!("install: {}", message);
    lua_pushboolean(l, ok as i32);
    push_str(l, &message);
    2
}

/// `nirender.capture(path, warmupFrames, frames)` starts a capture of the render link stream
/// into a file. See `capture`. Returns true, or false and the problem.
unsafe extern "C" fn capture(l: LuaState) -> i32 {
    let path = arg_str(l, 1).unwrap_or_default();
    match crate::capture::start(&path, lua_tonumber(l, 2) as u32, lua_tonumber(l, 3) as u32) {
        Ok(()) => {
            lua_pushboolean(l, 1);
            1
        }
        Err(error) => {
            lua_pushboolean(l, 0);
            push_str(l, &error);
            2
        }
    }
}

/// `nirender.stream(name, megabytes, warmupFrames, ownHost)` starts a stream of the render
/// link into a ring in shared memory, for a host program. With `ownHost`, the plugin becomes
/// the host of the link: no call of the game gets to a D3D device after the warm-up. Returns
/// true, or false and the problem.
unsafe extern "C" fn stream(l: LuaState) -> i32 {
    let name = arg_str(l, 1).unwrap_or_default();
    match crate::capture::start_stream(&name, lua_tonumber(l, 2) as u32, lua_tonumber(l, 3) as u32, lua_toboolean(l, 4) != 0) {
        Ok(()) => {
            lua_pushboolean(l, 1);
            1
        }
        Err(error) => {
            lua_pushboolean(l, 0);
            push_str(l, &error);
            2
        }
    }
}

/// `nirender.captureStop()` ends the capture or the stream.
unsafe extern "C" fn capture_stop(_l: LuaState) -> i32 {
    crate::capture::stop();
    0
}

/// `nirender.captureStatus()` returns the phase of the capture ("off", "warmup", "record",
/// "done"), its records and its bytes, as one text.
unsafe extern "C" fn capture_status(l: LuaState) -> i32 {
    push_str(l, &crate::capture::status());
    1
}

/// `nirender.recheck()` returns the number of changed functions. It does the census again,
/// for mods that change the game code after this plugin loaded. It locks the ports of such
/// functions.
unsafe extern "C" fn recheck(l: LuaState) -> i32 {
    let attached = install::attach_device();
    crate::log!("recheck: {:?}", attached);
    lua_pushnumber(l, install::census(true) as f64);
    1
}

/// `nirender.dumpState(path)` returns ok. It writes the renderer objects, for the comparison
/// of two runs.
unsafe extern "C" fn dump_state(l: LuaState) -> i32 {
    let ok = match arg_str(l, 1) {
        Some(path) => crate::dump::state(&path).is_ok(),
        None => false,
    };
    lua_pushboolean(l, ok as i32);
    1
}

/// `nirender.dumpFrame(path)` returns started. The device calls of the next frame go to the
/// file. `frameTestResult()` tells when the file is complete.
unsafe extern "C" fn dump_frame(l: LuaState) -> i32 {
    let started = match arg_str(l, 1) {
        Some(path) => frame::request_dump(path),
        None => false,
    };
    lua_pushboolean(l, started as i32);
    1
}

/// `nirender.setMode(pattern, mode)` returns the number of ports set. `pattern` is "*" or a
/// part of the engine name. `mode` is "off", "shadow" or "live".
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

/// `nirender.report()` returns the table as text. It also writes the text to the log.
unsafe extern "C" fn report_lua(l: LuaState) -> i32 {
    let text = install::report();
    crate::log!("report:\n{}", text);
    push_str(l, &text);
    1
}

/// `nirender.frameTest(kind)` returns started. `kind` is "control" or "live".
unsafe extern "C" fn frame_test(l: LuaState) -> i32 {
    let kind = match arg_str(l, 1).as_deref() {
        Some("live") => frame::Kind::Live,
        _ => frame::Kind::Control,
    };
    lua_pushboolean(l, frame::request(kind) as i32);
    1
}

/// `nirender.frameTestResult()` returns the result line, or nil while the test runs.
unsafe extern "C" fn frame_test_result(l: LuaState) -> i32 {
    match frame::take_result() {
        Some(text) => push_str(l, &text),
        None => lua_pushnil(l),
    }
    1
}

/// `nirender.recordBuffers(on)` also records the data that the game writes into vertex and
/// index buffers.
unsafe extern "C" fn record_buffers(l: LuaState) -> i32 {
    let on = lua_toboolean(l, 1) != 0;
    crate::resource::enable(on);
    crate::log!("buffer recording {}", if on { "on" } else { "off" });
    0
}

/// `nirender.fault(n)` puts fault `n` into the Rust code. See `engine::FAULT`. 0 removes it.
unsafe extern "C" fn fault(l: LuaState) -> i32 {
    let n = lua_tonumber(l, 1) as u32;
    engine::FAULT.store(n, Ordering::Relaxed);
    crate::log!("fault {n} put into the Rust code on purpose");
    0
}

/// `nirender.forceReset()` makes the next `TestCooperativeLevel` of the game say that the
/// device can be reset. The game then makes its renderer again: `NiDX8Renderer::Recreate`.
unsafe extern "C" fn force_reset(_l: LuaState) -> i32 {
    crate::device::FORCE_RESET.store(true, Ordering::Relaxed);
    0
}

/// `nirender.renderer()` returns the address of the renderer of the game, or 0 before it
/// exists.
unsafe extern "C" fn renderer(l: LuaState) -> i32 {
    lua_pushnumber(l, engine::renderer() as f64);
    1
}

/// `nirender.timing(true)` starts a measurement of frame times. `nirender.timing(false)`
/// ends it and returns the frame times in microseconds as one string, separated by commas.
unsafe extern "C" fn timing(l: LuaState) -> i32 {
    if lua_toboolean(l, 1) != 0 {
        crate::perf::start();
        return 0;
    }
    let times = crate::perf::stop();
    let mut text = String::with_capacity(times.len() * 8);
    for (index, time) in times.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        text.push_str(&format!("{time:.1}"));
    }
    push_str(l, &text);
    1
}

/// `nirender.liveCalls()` returns the number of calls that ran as Rust in mode "live".
unsafe extern "C" fn live_calls(l: LuaState) -> i32 {
    lua_pushnumber(l, ports::all().map(|p| p.live_calls.get()).sum::<u64>() as f64);
    1
}

/// `nirender.fpuControlWord()` returns the x87 control word of the calling thread.
unsafe extern "C" fn fpu_control_word(l: LuaState) -> i32 {
    lua_pushnumber(l, x87::control_word() as f64);
    1
}

/// `nirender.link()` returns ok and a message. It connects to the render link of MGE XE.
/// After that the plugin tells MGE XE the scene and the object of the device calls. Call
/// `install` first.
unsafe extern "C" fn link(l: LuaState) -> i32 {
    let (ok, message) = match crate::link_client::connect() {
        Ok(message) => (true, message),
        Err(message) => (false, message),
    };
    crate::log!("link: {}", message);
    lua_pushboolean(l, ok as i32);
    push_str(l, &message);
    2
}

/// `nirender.linkCounters([reset])` returns the counters of MGE XE as text: for each fact,
/// how many times the packet and the inference of MGE XE were equal or different.
unsafe extern "C" fn link_counters(l: LuaState) -> i32 {
    let text = crate::link_client::counters_text(lua_toboolean(l, 1) != 0);
    crate::log!("link counters:\n{}", text);
    push_str(l, &text);
    1
}

/// `nirender.linkUseFacts(mask)` returns the mask that MGE XE uses, or nil when MGE XE cannot
/// use facts. Bit n of `mask` is fact n of the contract. MGE XE takes these facts from the
/// packets and not from its inference.
unsafe extern "C" fn link_use_facts(l: LuaState) -> i32 {
    match crate::link_client::use_facts(lua_tonumber(l, 1) as u32) {
        Some(used) => {
            crate::log!("link: MGE XE uses the facts of mask {used:#x}");
            lua_pushnumber(l, used as f64);
        }
        None => lua_pushnil(l),
    }
    1
}

/// `nirender.linkTextures(on)` sets the use of the link for new source textures. Returns
/// false when MGE XE cannot make textures.
unsafe extern "C" fn link_textures(l: LuaState) -> i32 {
    let able = crate::link_client::set_link_textures(lua_toboolean(l, 1) != 0);
    lua_pushboolean(l, able as i32);
    1
}

/// `nirender.cameraSpaceCompare(on)` turns the comparison of the camera-relative world
/// matrices on or off. Returns false when the hook for it did not go in.
unsafe extern "C" fn camera_space_compare(l: LuaState) -> i32 {
    let result = crate::camera_space::set_compare(lua_toboolean(l, 1) != 0);
    if let Err(error) = &result {
        crate::log!("camera space: {error}");
    }
    lua_pushboolean(l, result.is_ok() as i32);
    1
}

/// `nirender.linkUseKinds(mask)` tells MGE XE which more device kinds it has. Bit 0 is the
/// frame and scene calls. Returns the mask that MGE XE uses, or nil.
unsafe extern "C" fn link_use_kinds(l: LuaState) -> i32 {
    match crate::link_client::use_kinds(lua_tonumber(l, 1) as u32) {
        Some(used) => lua_pushnumber(l, used as f64),
        None => lua_pushnil(l),
    }
    1
}

/// `nirender.deviceCallers(true)` starts the census of the callers of the device.
/// `nirender.deviceCallers(false)` stops it and returns the counts as text. See `callers.rs`.
unsafe extern "C" fn device_callers(l: LuaState) -> i32 {
    if lua_toboolean(l, 1) != 0 {
        crate::callers::set(true);
        return 0;
    }
    crate::callers::set(false);
    let text = crate::callers::report();
    crate::log!("device callers:\n{}", text);
    push_str(l, &text);
    1
}

#[no_mangle]
pub unsafe extern "C" fn luaopen_nirender(l: LuaState) -> i32 {
    lua_createtable(l, 0, 20);
    set_fn(l, "install", install);
    set_fn(l, "setMode", set_mode);
    set_fn(l, "report", report_lua);
    set_fn(l, "frameTest", frame_test);
    set_fn(l, "frameTestResult", frame_test_result);
    set_fn(l, "fpuControlWord", fpu_control_word);
    set_fn(l, "recordBuffers", record_buffers);
    set_fn(l, "fault", fault);
    set_fn(l, "capture", capture);
    set_fn(l, "captureStatus", capture_status);
    set_fn(l, "stream", stream);
    set_fn(l, "captureStop", capture_stop);
    set_fn(l, "forceReset", force_reset);
    set_fn(l, "renderer", renderer);
    set_fn(l, "liveCalls", live_calls);
    set_fn(l, "timing", timing);
    set_fn(l, "recheck", recheck);
    set_fn(l, "dumpState", dump_state);
    set_fn(l, "dumpFrame", dump_frame);
    set_fn(l, "link", link);
    set_fn(l, "linkCounters", link_counters);
    set_fn(l, "linkUseFacts", link_use_facts);
    set_fn(l, "linkTextures", link_textures);
    set_fn(l, "linkUseKinds", link_use_kinds);
    set_fn(l, "cameraSpaceCompare", camera_space_compare);
    set_fn(l, "deviceCallers", device_callers);
    push_str(l, env!("CARGO_PKG_VERSION"));
    lua_setfield(l, -2, b"version\0".as_ptr() as *const c_char);
    lua_pushnumber(l, ports::all().count() as f64);
    lua_setfield(l, -2, b"ports\0".as_ptr() as *const c_char);
    1
}
