//! The Lua interface. `include("akulakhan")` returns the table that `luaopen_akulakhan` builds
//! (the DLL is akulakhan.dll). The mod's main.lua registers the table as the module
//! `nirender` too, so `include("nirender")` works after it ran.

use core::ffi::{c_char, c_void};
#[cfg(feature = "rig")]
use core::sync::atomic::Ordering;
use std::ffi::CString;

use crate::main_thread::{MainCell, MainThread};
use crate::ports;
use crate::shadow::Mode;
use crate::{engine, install, x87};

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
    fn lua_gettop(l: LuaState) -> i32;
    fn lua_tolstring(l: LuaState, index: i32, len: *mut usize) -> *const c_char;
    fn lua_rawseti(l: LuaState, index: i32, n: i32);
    fn lua_pushlightuserdata(l: LuaState, p: *mut c_void);
    fn lua_touserdata(l: LuaState, index: i32) -> *mut c_void;
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

/// Registers `f` behind `main_thread_gate`: the function and its name are the closure's two
/// upvalues.
unsafe fn set_fn(l: LuaState, name: &str, f: LuaFn) {
    let key = CString::new(name).unwrap();
    lua_pushlightuserdata(l, f as *mut c_void);
    push_str(l, name);
    lua_pushcclosure(l, main_thread_gate, 2);
    lua_setfield(l, -2, key.as_ptr());
}

/// The pseudo-indexes of a C closure's upvalues (LuaJIT: `LUA_GLOBALSINDEX - i`).
const UPVALUE_FUNCTION: i32 = -10002 - 1;
const UPVALUE_NAME: i32 = -10002 - 2;

/// Every `nirender.*` function runs on the main thread only. MWSE fires some Lua events on the
/// cell worker (`referenceSceneNodeCreated`), so a mod can call this table there; the plugin's
/// state is the main thread's. Off the main thread
/// `changed` is queued for the main thread's next frame, and every other function does
/// nothing and returns no value (the first call of each name is logged).
unsafe extern "C" fn main_thread_gate(l: LuaState) -> i32 {
    let f: LuaFn = core::mem::transmute(lua_touserdata(l, UPVALUE_FUNCTION));
    // Behind this check a function claims the proof of the main thread
    // (`MainThread::claim`) when it opens a table.
    if crate::shadow::on_main_thread() {
        return f(l);
    }
    if f as usize == changed as LuaFn as usize {
        crate::instance_stream::queue_change(lua_tonumber(l, 1) as u32 as usize);
        return 0;
    }
    let name = arg_str(l, UPVALUE_NAME).unwrap_or_default();
    static REFUSED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    if let Ok(mut refused) = REFUSED.lock() {
        if !refused.contains(&name) {
            crate::log!("lua: nirender.{name} called on thread {} (not the main thread): refused", crate::win::GetCurrentThreadId());
            refused.push(name);
        }
    }
    0
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

/// `nirender.skinSource([name])` sets where the bone influences of a skinned shape come from:
/// "skindata" (the weights of the model) or "partitions" (the skin partitions of the game, as
/// for Direct3D 8). The geometries are packed again. Returns the source before. Without a
/// name it only returns the source.
unsafe extern "C" fn skin_source(l: LuaState) -> i32 {
    let name = arg_str(l, 1);
    let before = match name.as_deref() {
        Some("partitions") => crate::geometry_stream::set_skin_from_partitions(&MainThread::claim(), true),
        Some("skindata") => crate::geometry_stream::set_skin_from_partitions(&MainThread::claim(), false),
        _ => crate::geometry_stream::skin_from_partitions(),
    };
    push_str(l, if before { "partitions" } else { "skindata" });
    1
}

/// `nirender.refusalOptions()` returns a list of `{ id = <feature>, name = <option> }`: the
/// options of the Morrowind Code Patch whose changes locked a port. A refusal with an empty
/// list has another cause.
unsafe extern "C" fn refusal_options(l: LuaState) -> i32 {
    let options = install::locked_by_options();
    lua_createtable(l, options.len() as i32, 0);
    for (index, &feature) in options.iter().enumerate() {
        lua_createtable(l, 0, 2);
        lua_pushnumber(l, feature as f64);
        lua_setfield(l, -2, b"id\0".as_ptr() as *const c_char);
        push_str(l, ports::code_patch_option(feature));
        lua_setfield(l, -2, b"name\0".as_ptr() as *const c_char);
        lua_rawseti(l, -2, index as i32 + 1);
    }
    1
}

/// `nirender.capture(path, warmupFrames, frames)` starts a capture of the render link stream
/// into a file. See `capture`. Returns true, or false and the problem.
unsafe extern "C" fn capture(l: LuaState) -> i32 {
    let path = arg_str(l, 1).unwrap_or_default();
    match crate::capture::start(&MainThread::claim(), &path, lua_tonumber(l, 2) as u32, lua_tonumber(l, 3) as u32) {
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

/// `nirender.streamShot(path)` asks the host program of the stream for a picture file of the
/// frame in progress.
unsafe extern "C" fn stream_shot(l: LuaState) -> i32 {
    crate::capture::shot(&arg_str(l, 1).unwrap_or_default());
    0
}

/// `nirender.streamMark(text)` puts a mark into the stream: the host program prints its
/// counts since the last mark, with the text.
unsafe extern "C" fn stream_mark(l: LuaState) -> i32 {
    crate::capture::mark(&arg_str(l, 1).unwrap_or_default());
    0
}

/// `nirender.loadingDone()`: the game has its data (the "initialized" event).
unsafe extern "C" fn loading_done(_: LuaState) -> i32 {
    crate::capture::loading_done();
    0
}

/// `nirender.overlay([on])` turns the debug overlay of the host program on or off; without
/// a value, to the other state. Returns the state.
unsafe extern "C" fn overlay(l: LuaState) -> i32 {
    let value = if lua_gettop(l) >= 1 { Some(lua_toboolean(l, 1) != 0) } else { None };
    lua_pushboolean(l, crate::capture::overlay(value) as i32);
    1
}

/// `nirender.tune(command)` sends a command for the parameters that the renderer shows in
/// its debug overlay, as a text: `set <name> <value>`, `reset <name>`, or `reset all`. The
/// command goes into the stream as data of an extension, with the number 5; a renderer
/// without parameters does nothing with it. The parameters window of the MGE XE options
/// sends these from its sliders (the row "water (renderer)" of its shader list).
unsafe extern "C" fn tune(l: LuaState) -> i32 {
    const TUNE: u32 = 5;
    crate::capture::extension(TUNE, arg_str(l, 1).unwrap_or_default().as_bytes());
    0
}

/// `nirender.portTimes(n)` returns the `n` ports with the most time of the main thread since
/// the last call, as one text: name, calls, ms. The time of a port has the ports that it
/// calls in it, so the numbers do not add up.
unsafe extern "C" fn port_times(l: LuaState) -> i32 {
    // The live port calls are timed from the first call on (the times are off by default).
    crate::shadow::set_timing(true);
    let t = &MainThread::claim();
    static LAST_CALLS: MainCell<Vec<u64>> = MainCell::new(Vec::new());
    let mut last = LAST_CALLS.get(t);
    let ports: Vec<_> = ports::all().collect();
    last.resize(ports.len(), 0);
    let mut rows: Vec<(f64, u64, &str)> = ports
        .iter()
        .enumerate()
        .map(|(i, port)| {
            let calls = port.live_calls.get();
            let row = (crate::shadow::stamps_to_ms(port.live_stamps.take()), calls - last[i], port.name);
            last[i] = calls;
            row
        })
        .collect();
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    let n = (lua_tonumber(l, 1) as usize).max(1);
    let text: Vec<String> = rows.iter().take(n).filter(|row| row.1 != 0).map(|(ms, calls, name)| format!("{name} {calls} {ms:.1}")).collect();
    push_str(l, &text.join("; "));
    1
}


/// `nirender.gameScreenshot()` makes the game take its own screenshot, as its screenshot
/// key does: `WorldControllerRenderTarget::takeScreenshot` 0x42E620 of
/// `WorldController::characterRenderTarget`. The game draws the scene again, reads the back
/// buffer and writes a BMP file into its folder.
unsafe extern "C" fn game_screenshot(_l: LuaState) -> i32 {
    const TAKE_SCREENSHOT: usize = 0x42E620;
    const CHARACTER_RENDER_TARGET: usize = crate::gen::layouts::WorldController::characterRenderTarget;
    let world_controller: usize = crate::mem::rd(crate::engine::WORLD_CONTROLLER, 0);
    if world_controller != 0 {
        crate::engine::thiscall0(TAKE_SCREENSHOT, world_controller + CHARACTER_RENDER_TARGET);
    }
    0
}

/// `nirender.dumpDevice(path)` writes what the Direct3D 8 of the game says about itself into
/// a text file: the capabilities of the device, the display mode, the modes of the adapter,
/// and the formats that it accepts. The device of the plugin gives the same answers.
unsafe extern "C" fn dump_device(l: LuaState) -> i32 {
    use core::mem::transmute;
    use std::fmt::Write;
    let path = arg_str(l, 1).unwrap_or_default();
    let renderer = crate::engine::renderer();
    if renderer == 0 {
        lua_pushboolean(l, 0);
        return 1;
    }
    use crate::gen::layouts::NiDX8Renderer as R;
    let device: usize = crate::mem::rd(renderer, R::d3dDevice);
    let method = |object: usize, index: usize| crate::engine::virtual_at(object, index * 4);
    let hex = |bytes: &[u32]| bytes.iter().map(|w| format!("{w:08x}")).collect::<Vec<_>>().join(" ");
    let mut text = String::new();
    // D3DCAPS8 has 212 bytes.
    let mut caps = [0u32; 53];
    transmute::<usize, unsafe extern "system" fn(usize, *mut u32) -> u32>(method(device, 7))(device, caps.as_mut_ptr());
    let _ = writeln!(text, "caps {}", hex(&caps));
    let parameters: [u32; 13] = crate::mem::rd(renderer, R::d3dPresentParameters);
    let _ = writeln!(text, "present {}", hex(&parameters));
    let _ = writeln!(text, "behavior {:08x} type {}", crate::mem::rd::<u32>(renderer, R::d3dBehaviorFlags), crate::mem::rd::<u32>(renderer, R::d3dDeviceType));
    let mut direct3d = 0usize;
    transmute::<usize, unsafe extern "system" fn(usize, *mut usize) -> u32>(method(device, 6))(device, &mut direct3d);
    if direct3d != 0 {
        let mut mode = [0u32; 4];
        transmute::<usize, unsafe extern "system" fn(usize, u32, *mut u32) -> u32>(method(direct3d, 8))(direct3d, 0, mode.as_mut_ptr());
        let _ = writeln!(text, "display {}", hex(&mode));
        let count = transmute::<usize, unsafe extern "system" fn(usize, u32) -> u32>(method(direct3d, 6))(direct3d, 0);
        for i in 0..count.min(400) {
            transmute::<usize, unsafe extern "system" fn(usize, u32, u32, *mut u32) -> u32>(method(direct3d, 7))(direct3d, 0, i, mode.as_mut_ptr());
            let _ = writeln!(text, "mode {} {} {} {}", mode[0], mode[1], mode[2], mode[3]);
        }
        // CheckDeviceFormat(adapter, device type, adapter format, usage, resource type, format).
        let check: unsafe extern "system" fn(usize, u32, u32, u32, u32, u32, u32) -> u32 = transmute(method(direct3d, 10));
        let formats: Vec<u32> = (20..=30).chain(40..=41).chain(50..=52).chain(60..=64).chain(70..=80).chain([0x3154_5844, 0x3254_5844, 0x3354_5844, 0x3454_5844, 0x3554_5844]).collect();
        for (usage, kind) in [(0u32, 3u32), (1, 3), (2, 1), (2, 3), (0, 5), (1, 5), (1, 1), (0, 1)] {
            let accepted: Vec<String> = formats.iter().filter(|f| check(direct3d, 0, 1, 22, usage, kind, **f) == 0).map(|f| format!("{f:x}")).collect();
            let _ = writeln!(text, "formats usage {usage} type {kind}: {}", accepted.join(" "));
        }
        // CheckDepthStencilMatch(adapter, device type, adapter format, target format, depth format).
        let matches: unsafe extern "system" fn(usize, u32, u32, u32, u32, u32) -> u32 = transmute(method(direct3d, 12));
        for target in [21u32, 22, 23] {
            let accepted: Vec<String> = (70..=80u32).filter(|f| matches(direct3d, 0, 1, 22, target, *f) == 0).map(|f| f.to_string()).collect();
            let _ = writeln!(text, "depth for target {target}: {}", accepted.join(" "));
        }
        // D3DADAPTER_IDENTIFIER8 has 1068 bytes: the driver and the description first.
        let mut identifier = [0u8; 1068];
        transmute::<usize, unsafe extern "system" fn(usize, u32, u32, *mut u8) -> u32>(method(direct3d, 5))(direct3d, 0, 2, identifier.as_mut_ptr());
        let name = |from: usize| String::from_utf8_lossy(&identifier[from..from + 512]).trim_end_matches('\0').to_string();
        let _ = writeln!(text, "driver {}", name(0));
        let _ = writeln!(text, "description {}", name(512));
        let rest: Vec<String> = identifier[1024..].iter().map(|b| format!("{b:02x}")).collect();
        let _ = writeln!(text, "identifier rest {}", rest.join(""));
        transmute::<usize, unsafe extern "system" fn(usize) -> u32>(method(direct3d, 2))(direct3d);
    }
    lua_pushboolean(l, std::fs::write(&path, text).is_ok() as i32);
    1
}

/// `nirender.ownDevice()` returns true when the game gets its Direct3D 8 from the plugin: the
/// `d3d8.dll` of the game folder is the one of this repository, and no Direct3D device is in
/// the process.
unsafe extern "C" fn own_device(l: LuaState) -> i32 {
    lua_pushboolean(l, (crate::own_device::present() || crate::link_client::own_d3d8()) as i32);
    1
}

/// `nirender.threadCensus()` returns the calls that came into the plugin from a thread that is
/// not the main thread, as text: one line for each kind of call, with its count.
unsafe extern "C" fn thread_census(l: LuaState) -> i32 {
    push_str(l, &crate::sync::census_text());
    1
}

/// `nirender.captureStop()` ends the capture or the stream.
unsafe extern "C" fn capture_stop(_l: LuaState) -> i32 {
    crate::capture::stop();
    0
}

/// `nirender.captureFrames(n)` asks a capture file that holds (`nirender.capture(path, 0, 0)`
/// on a stream) for `n` frames more. Returns true when there is such a file.
unsafe extern "C" fn capture_frames(l: LuaState) -> i32 {
    lua_pushboolean(l, crate::capture::tee_frames(&MainThread::claim(), lua_tonumber(l, 1) as u32) as i32);
    1
}

/// `nirender.captureEnd()` ends a capture file that holds; the stream goes on.
unsafe extern "C" fn capture_end(_l: LuaState) -> i32 {
    crate::capture::tee_end();
    0
}

/// `nirender.nifShading([on])`: with a boolean, the shape's own shade, dither and specular
/// properties (from its NIF) go into the material record instead of the engine's
/// globals (experimental, off by default; a changed record is sent again). Returns the
/// state.
unsafe extern "C" fn nif_shading(l: LuaState) -> i32 {
    if lua_gettop(l) >= 1 {
        crate::material_stream::set_nif_shading(lua_toboolean(l, 1) != 0);
    }
    lua_pushboolean(l, crate::material_stream::nif_shading() as i32);
    1
}

/// `nirender.skyDraws([on])`: with a boolean, the draws of the sky instances go as the table's
/// `INSTANCE_DRAW` records (on, the default) or as the engine's shape records (off). Returns the
/// state before.
unsafe extern "C" fn sky_draws(l: LuaState) -> i32 {
    let before = crate::instance_stream::sky_draws();
    if lua_gettop(l) >= 1 {
        crate::instance_stream::set_sky_draws(lua_toboolean(l, 1) != 0);
    }
    lua_pushboolean(l, before as i32);
    1
}

/// `nirender.particleDraws([on])`: with a boolean, the draws of the particle instances go as
/// `INSTANCE_PARTICLES` and `INSTANCE_DRAW` records (on, the default when the table takes
/// particles) or as the engine's shape records (off). Returns the state before.
unsafe extern "C" fn particle_draws(l: LuaState) -> i32 {
    let before = crate::instance_stream::particle_draws();
    if lua_gettop(l) >= 1 {
        crate::instance_stream::set_particle_draws(lua_toboolean(l, 1) != 0);
    }
    lua_pushboolean(l, before as i32);
    1
}

/// `nirender.animationPolicy([name])`: with a name (`view`, `shadows`, `all`) sets the
/// policy of what animates when the plugin's table draws the world. Returns
/// the name of the policy in force.
unsafe extern "C" fn animation_policy(l: LuaState) -> i32 {
    if lua_gettop(l) >= 1 {
        let mut length = 0usize;
        let text = lua_tolstring(l, 1, &mut length);
        if !text.is_null() {
            let name = core::str::from_utf8_unchecked(core::slice::from_raw_parts(text as *const u8, length));
            crate::animation::set_policy_name(name);
        }
    }
    push_str(l, crate::animation::policy_name());
    1
}

/// `nirender.instances()`: the report of the retained instances, as the log gets it at the
/// end of a run: the counts of the retained, the rejected by reason, the draws not taken.
unsafe extern "C" fn instances_report(l: LuaState) -> i32 {
    let t = &MainThread::claim();
    push_str(l, &crate::instance_stream::report(t));
    1
}

/// `nirender.captureStatus()` returns the phase of the capture ("off", "warmup", "record",
/// "done"), its records and its bytes, as one text.
unsafe extern "C" fn capture_status(l: LuaState) -> i32 {
    push_str(l, &crate::capture::status());
    1
}

/// `nirender.changed(address)`: `notifier.lua` reports the address of an object a Lua mod
/// changed. See `instance_stream::note_change`.
unsafe extern "C" fn changed(l: LuaState) -> i32 {
    crate::instance_stream::note_change(&MainThread::claim(), lua_tonumber(l, 1) as u32 as usize);
    0
}

/// `nirender.noteEvery(on)`: note the material of every retained shape at every display;
/// returns the state before; without an argument the state.
unsafe extern "C" fn note_every(l: LuaState) -> i32 {
    let before = crate::instance_stream::note_every();
    if lua_gettop(l) >= 1 {
        crate::instance_stream::set_note_every(lua_toboolean(l, 1) != 0);
    }
    lua_pushboolean(l, before as i32);
    1
}

/// `nirender.displayCost()`: the skipped displays since the last call, the notes among
/// them and their time in ms, as text; the counters reset.
unsafe extern "C" fn display_cost(l: LuaState) -> i32 {
    let (calls, notes, ms) = crate::instance_stream::display_cost();
    push_str(l, &format!("{calls} {notes} {ms:.3}"));
    1
}

/// `nirender.skinTable(on)`: the table draws the skinned shapes (the actors, the default) or
/// the engine's display does; returns the state before; without an argument the state.
unsafe extern "C" fn skin_table(l: LuaState) -> i32 {
    let t = &MainThread::claim();
    let before = crate::instance_stream::skin_table(t);
    if lua_gettop(l) >= 1 {
        let on = lua_toboolean(l, 1) != 0;
        crate::instance_stream::set_skin_table(t, on);
        crate::log!("skin table: {} (was {})", on, before);
    }
    lua_pushboolean(l, before as i32);
    1
}

/// `nirender.worldLights()`: a table with a line for each light of the world
/// (`world_lights::describe`), for a check in a live session.
unsafe extern "C" fn world_lights(l: LuaState) -> i32 {
    let lines = crate::world_lights::describe();
    lua_createtable(l, lines.len() as i32, 0);
    for (i, line) in lines.iter().enumerate() {
        push_str(l, line);
        lua_rawseti(l, -2, i as i32 + 1);
    }
    1
}

/// `nirender.retained(mode)` sets the mode of the retained instances ("off", "dual" or
/// "gpu") and returns the mode before. Without an argument it returns the mode.
unsafe extern "C" fn retained(l: LuaState) -> i32 {
    let t = &MainThread::claim();
    let before = crate::instance_stream::mode(t);
    if let Some(mode) = arg_str(l, 1).and_then(|m| crate::instance_stream::Mode::parse(&m)) {
        crate::instance_stream::set_mode(t, mode);
        crate::log!("retained: {} (was {})", mode.name(), before.name());
    }
    push_str(l, before.name());
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
#[cfg(feature = "rig")]
unsafe extern "C" fn dump_state(l: LuaState) -> i32 {
    let ok = match arg_str(l, 1) {
        Some(path) => crate::dump::state(&path).is_ok(),
        None => false,
    };
    lua_pushboolean(l, ok as i32);
    1
}

/// `nirender.dumpFrame(path)` returns started. The device calls of the next frame go to the
/// file. [`frameTestResult()`] tells when the file is complete.
#[cfg(feature = "rig")]
unsafe extern "C" fn dump_frame(l: LuaState) -> i32 {
    let started = match arg_str(l, 1) {
        Some(path) => crate::frame::request_dump(path),
        None => false,
    };
    lua_pushboolean(l, started as i32);
    1
}

/// `nirender.setMode(pattern, mode)` returns the number of ports set. `pattern` is "*" or a
/// part of the engine name. `mode` is "off" or "live", and "shadow" with the feature `rig`.
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
#[cfg(feature = "rig")]
unsafe extern "C" fn frame_test(l: LuaState) -> i32 {
    use crate::frame;
    let kind = match arg_str(l, 1).as_deref() {
        Some("live") => frame::Kind::Live,
        _ => frame::Kind::Control,
    };
    lua_pushboolean(l, frame::request(kind) as i32);
    1
}

/// `nirender.frameTestResult()` returns the result line, or nil while the test runs.
#[cfg(feature = "rig")]
unsafe extern "C" fn frame_test_result(l: LuaState) -> i32 {
    match crate::frame::take_result() {
        Some(text) => push_str(l, &text),
        None => lua_pushnil(l),
    }
    1
}

/// `nirender.recordBuffers(on)` also records the data that the game writes into vertex and
/// index buffers.
#[cfg(feature = "rig")]
unsafe extern "C" fn record_buffers(l: LuaState) -> i32 {
    let on = lua_toboolean(l, 1) != 0;
    crate::resource::enable(on);
    crate::log!("buffer recording {}", if on { "on" } else { "off" });
    0
}

/// `nirender.fault(n)` puts fault `n` into the Rust code. See [`engine::FAULT`]. 0 removes it.
#[cfg(feature = "rig")]
unsafe extern "C" fn fault(l: LuaState) -> i32 {
    let n = lua_tonumber(l, 1) as u32;
    engine::FAULT.store(n, Ordering::Relaxed);
    crate::log!("fault {n} put into the Rust code on purpose");
    0
}

/// `nirender.forceReset()` makes the next [`TestCooperativeLevel`] of the game say that the
/// device can be reset. The game then makes its renderer again: [`NiDX8Renderer::Recreate`].
#[cfg(feature = "rig")]
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
#[cfg(feature = "rig")]
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
#[cfg(feature = "rig")]
unsafe extern "C" fn live_calls(l: LuaState) -> i32 {
    lua_pushnumber(l, ports::all().map(|p| p.live_calls.get()).sum::<u64>() as f64);
    1
}

/// `nirender.fpuControlWord()` returns the x87 control word of the calling thread.
unsafe extern "C" fn fpu_control_word(l: LuaState) -> i32 {
    lua_pushnumber(l, x87::control_word() as f64);
    1
}

/// `nirender.link()` returns ok and a message. It connects the ports to the host of the render
/// link in the plugin (`own_host`). After that the ports tell the host the scene and the
/// object of the device calls. Call `install` first.
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

/// `nirender.linkCounters([reset])` returns the counters of the host as text: for each fact,
/// how many times the packet and the inference of the host were equal or different.
unsafe extern "C" fn link_counters(l: LuaState) -> i32 {
    let text = crate::link_client::counters_text(&MainThread::claim(), lua_toboolean(l, 1) != 0);
    crate::log!("link counters:\n{}", text);
    push_str(l, &text);
    1
}

/// `nirender.linkUseFacts(mask)` returns the mask that the host uses, or nil when the host
/// cannot use facts. Bit n of `mask` is fact n of the contract. The host takes these facts
/// from the packets and not from its inference.
unsafe extern "C" fn link_use_facts(l: LuaState) -> i32 {
    match crate::link_client::use_facts(&MainThread::claim(), lua_tonumber(l, 1) as u32) {
        Some(used) => {
            crate::log!("link: the host uses the facts of mask {used:#x}");
            lua_pushnumber(l, used as f64);
        }
        None => lua_pushnil(l),
    }
    1
}

/// `nirender.linkTextures(on)` sets the use of the link for new source textures. Returns
/// false when the host cannot make textures.
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

/// `nirender.linkUseKinds(mask)` tells the host which more device kinds it has. Bit 0 is the
/// frame and scene calls. Returns the mask that the host uses, or nil.
unsafe extern "C" fn link_use_kinds(l: LuaState) -> i32 {
    match crate::link_client::use_kinds(lua_tonumber(l, 1) as u32) {
        Some(used) => lua_pushnumber(l, used as f64),
        None => lua_pushnil(l),
    }
    1
}

/// `nirender.deviceCallers(true)` starts the census of the callers of the device.
/// `nirender.deviceCallers(false)` stops it and returns the counts as text. See `callers.rs`.
#[cfg(feature = "rig")]
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
pub unsafe extern "C" fn luaopen_akulakhan(l: LuaState) -> i32 {
    crate::shadow::note_main_thread_if_unknown();
    lua_createtable(l, 0, 20);
    set_fn(l, "install", install);
    set_fn(l, "refusalOptions", refusal_options);
    set_fn(l, "skinSource", skin_source);
    set_fn(l, "setMode", set_mode);
    set_fn(l, "report", report_lua);
    set_fn(l, "fpuControlWord", fpu_control_word);
    set_fn(l, "capture", capture);
    set_fn(l, "captureStatus", capture_status);
    set_fn(l, "instances", instances_report);
    set_fn(l, "nifShading", nif_shading);
    set_fn(l, "skyDraws", sky_draws);
    set_fn(l, "particleDraws", particle_draws);
    set_fn(l, "animationPolicy", animation_policy);
    set_fn(l, "skinTable", skin_table);
    set_fn(l, "noteEvery", note_every);
    set_fn(l, "changed", changed);
    set_fn(l, "displayCost", display_cost);
    set_fn(l, "retained", retained);
    set_fn(l, "worldLights", world_lights);
    set_fn(l, "captureFrames", capture_frames);
    set_fn(l, "captureEnd", capture_end);
    set_fn(l, "stream", stream);
    set_fn(l, "captureStop", capture_stop);
    set_fn(l, "streamShot", stream_shot);
    set_fn(l, "streamMark", stream_mark);
    set_fn(l, "overlay", overlay);
    set_fn(l, "tune", tune);
    set_fn(l, "loadingDone", loading_done);
    set_fn(l, "portTimes", port_times);
    set_fn(l, "dumpDevice", dump_device);
    set_fn(l, "ownDevice", own_device);
    set_fn(l, "threadCensus", thread_census);
    set_fn(l, "gameScreenshot", game_screenshot);
    set_fn(l, "renderer", renderer);
    set_fn(l, "recheck", recheck);
    set_fn(l, "link", link);
    set_fn(l, "linkCounters", link_counters);
    set_fn(l, "linkUseFacts", link_use_facts);
    set_fn(l, "linkTextures", link_textures);
    set_fn(l, "linkUseKinds", link_use_kinds);
    set_fn(l, "cameraSpaceCompare", camera_space_compare);
    // The functions of the comparison rig.
    #[cfg(feature = "rig")]
    {
        set_fn(l, "frameTest", frame_test);
        set_fn(l, "frameTestResult", frame_test_result);
        set_fn(l, "recordBuffers", record_buffers);
        set_fn(l, "fault", fault);
        set_fn(l, "forceReset", force_reset);
        set_fn(l, "liveCalls", live_calls);
        set_fn(l, "timing", timing);
        set_fn(l, "dumpState", dump_state);
        set_fn(l, "dumpFrame", dump_frame);
        set_fn(l, "deviceCallers", device_callers);
    }
    push_str(l, env!("CARGO_PKG_VERSION"));
    lua_setfield(l, -2, b"version\0".as_ptr() as *const c_char);
    lua_pushnumber(l, ports::all().count() as f64);
    lua_setfield(l, -2, b"ports\0".as_ptr() as *const c_char);
    1
}
