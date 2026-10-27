//! A Direct3D 8 of the plugin: the game gets its `IDirect3D8` and its `IDirect3DDevice8` from
//! here, and no Direct3D device is in the process.
//!
//! The `d3d8.dll` of this repository is a small file in the game folder. Its
//! `Direct3DCreate8` loads this plugin and calls `nirender_direct3d_create8`. The objects that
//! the game then gets answer the questions of the start (capabilities, formats, display
//! modes) as the Direct3D of the testing computer did, and they make resources with memory of
//! their own: the objects of `own_host`.
//!
//! This device draws nothing itself. The picture comes from the ports: they send state
//! packets through the render link, the plugin is the host of the link, and the stream goes
//! to the host program, which `CreateDevice` starts. A state call or a draw call that does
//! not come from a port does not reach the helper. Only `SetTransform` (the view and the
//! projection) and `DrawPrimitiveUP` (the films) go into the stream.
//!
//! The device has the recorder of `device` on it, as a device of Direct3D has. A call comes
//! here in two ways: from the recorder when the link does not take the call, and from
//! `link_client::intercept` when the plugin is the host and the call has no call of the link.

use core::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering};

use crate::checked::Open;
use crate::own_host::Kept;
use crate::sync::{DeviceLock, LockCell};
use crate::link::*;

const OK: u32 = 0;
const NO_INTERFACE: u32 = 0x8000_4002;
const INVALID_CALL: u32 = 0x8876_086C;
const NOT_AVAILABLE: u32 = 0x8876_086A;

#[link(name = "user32")]
extern "system" {
    fn EnumDisplaySettingsA(device: *const u8, mode: u32, out: *mut u8) -> i32;
    fn MonitorFromWindow(window: usize, flags: u32) -> usize;
}

extern "system" {
    fn GetModuleHandleExA(flags: u32, address: *const core::ffi::c_void, module: *mut usize) -> i32;
    fn GetModuleFileNameA(module: usize, name: *mut u8, size: u32) -> u32;
    fn GetCurrentProcessId() -> u32;
    fn CreateProcessA(
        application: *const u8,
        command_line: *mut u8,
        process_attributes: *const u8,
        thread_attributes: *const u8,
        inherit: i32,
        flags: u32,
        environment: *const u8,
        directory: *const u8,
        startup: *const u8,
        information: *mut usize,
    ) -> i32;
    fn CloseHandle(handle: isize) -> i32;
    fn WaitForSingleObject(handle: isize, milliseconds: u32) -> u32;
    fn TerminateProcess(process: isize, code: u32) -> i32;
    fn GetExitCodeProcess(process: isize, code: *mut u32) -> i32;
}

/// The process of the host program that `start_stream` started, or 0, and the name of its
/// file.
static HOST_PROCESS: AtomicIsize = AtomicIsize::new(0);
static HOST_NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
/// The window of the game that the device draws into, from the start of the stream, or 0:
/// a stop box closes it when the box closes (`failure.rs`).
static GAME_WINDOW: AtomicUsize = AtomicUsize::new(0);

/// The window of the game, or 0 before the device is made.
pub fn game_window() -> usize {
    GAME_WINDOW.load(Ordering::Relaxed)
}

/// The game showed a frame. When the host program has ended, no new picture goes into the
/// window: the plugin logs the exit code and starts the failure path (`failure`: the
/// autosave, then the box), unless the host program ended on a refusal of its own, which it
/// named in its own box. The host program ends after the game at the end of a session, so
/// this finds a crash, its watchdog, `end_host` or a kill only. A device that the host
/// program lost does not end it: the host program says so in a message (`capture.rs`).
/// Returns true when the host program has ended: the stream then ends.
pub unsafe fn check_host() -> bool {
    let process = HOST_PROCESS.load(Ordering::Relaxed);
    // WAIT_OBJECT_0: the process has ended.
    if process == 0 || WaitForSingleObject(process, 0) != 0 {
        return false;
    }
    HOST_PROCESS.store(0, Ordering::Relaxed);
    let mut code = 0u32;
    GetExitCodeProcess(process, &mut code);
    CloseHandle(process);
    crate::log!("own device: the host program ended with the exit code {code:#x}; the window gets no new picture");
    // The host program ended on a refusal of its own (a stream of another build, no device):
    // it named the reason in its log and in its own message box.
    if crate::stream_format::helper_said_why(code) {
        crate::log!("own device: the exit code is a refusal of the host program, which showed its message (nirender-host.log has the reason)");
        return true;
    }
    crate::failure::found(crate::failure::Cause::of_exit_code(code));
    true
}

/// Ends the host program with the exit code `code`, when it runs: for a host program that
/// reads no record (the ring wait of `capture.rs`). `check_host` then finds the code.
///
/// # Safety
///
/// The process handle of `HOST_PROCESS` is one that `start_stream` made and `check_host` has
/// not closed: the handle stays open until `check_host` finds the end.
pub unsafe fn end_host(code: u32) {
    let process = HOST_PROCESS.load(Ordering::Relaxed);
    if process != 0 && TerminateProcess(process, code) != 0 {
        crate::log!("own device: the host program is ended with the exit code {code}");
    }
}

/// The D3DCAPS8 of the device of the testing computer (MGE XE on DXVK), from
/// `nirender.dumpDevice`, so that the game makes the same choices as there. One change:
/// `MaxActiveLights` (word 40, byte 0xA0) is 32, not 8. The engine's renderer does not read
/// this word. `updateLights` (0x6BB6C0) enables every light of the effect state. The only cap
/// is 7 lights of a node's own list (`NiNode::PushLocalEffects` 0x6C8F00), 32 with
/// `distant_land.expanded_light_limit`.
const CAPS: [u32; 53] = [
    0x00000001, 0x00000000, 0x00020000, 0x200a0000, 0x00000420, 0x8000000f, 0x00000000, 0x001bbef0, 0x00000ef2, 0x00f37191,
    0x000000ff, 0x00001fff, 0x00001fff, 0x000000ff, 0x00084208, 0x0001ecc5, 0x03030700, 0x03030300, 0x03030300, 0x0000003f,
    0x0000003f, 0x0000001f, 0x00004000, 0x00004000, 0x00000800, 0x00002000, 0x00004000, 0x00000010, 0x501502f9, 0xccbebc20,
    0xccbebc20, 0x4cbebc20, 0x4cbebc20, 0x00000000, 0x000000ff, 0x00180008, 0x03feffff, 0x00000008, 0x00000008, 0x0000003b,
    0x00000020, 0x00000008, 0x00000004, 0x00000000, 0x46000000, 0x00ffffff, 0x00ffffff, 0x00000010, 0x000000ff, 0x00000000,
    0x00000100, 0x00000000, 0x477fe000,
];

/// D3DFMT_X8R8G8B8, the format of the display and of the back buffer.
const DISPLAY_FORMAT: u32 = 22;
const DXT: [u32; 5] = [0x3154_5844, 0x3254_5844, 0x3354_5844, 0x3454_5844, 0x3554_5844];
/// The formats that the Direct3D of the testing computer accepted for a texture that the
/// game draws into, and the ones it accepted in addition for a texture. The game does not
/// start with a shorter list.
const TARGET_FORMATS: [u32; 13] = [0x15, 0x16, 0x17, 0x18, 0x19, 0x1c, 0x32, 0x33, 0x3c, 0x3d, 0x3e, 0x3f, 0x40];
const TEXTURE_ONLY_FORMATS: [u32; 3] = [0x1a, 0x1e, 0x34];
/// D3DFMT_D24S8, D3DFMT_D24X8 and D3DFMT_D16.
const DEPTH_FORMATS: [u32; 3] = [75, 77, 80];

struct State {
    /// The D3DPRESENT_PARAMETERS of `CreateDevice` or of the last `Reset`.
    parameters: [u32; 13],
    focus_window: u32,
    behavior: u32,
    viewport: [u32; 6],
    render_states: [u32; 256],
    stage_states: [[u32; 32]; 8],
    textures: [Kept; 8],
    /// The transforms by their D3DTRANSFORMSTATETYPE: the view 2, the projection 3, the
    /// textures 16 to 23, the worlds from 256.
    transforms: Vec<(u32, [f32; 16])>,
    vertex_shader: u32,
    stream: (Kept, u32),
    indices: (Kept, u32),
    render_target: Kept,
    /// The display modes: width, height, refresh rate.
    modes: Vec<[u32; 3]>,
    current_mode: [u32; 3],
}

static STATE: LockCell<Option<State>> = LockCell::new(None);
static PRESENT: AtomicBool = AtomicBool::new(false);
static DEVICE: AtomicUsize = AtomicUsize::new(0);

#[track_caller]
fn state<'a>(lock: &'a DeviceLock) -> Open<'a, State> {
    STATE.get(lock).map(|state| state.get_or_insert_with(|| State {
        parameters: [0; 13],
        focus_window: 0,
        behavior: 0,
        viewport: [0, 0, 640, 480, 0, 1.0f32.to_bits()],
        render_states: [0; 256],
        stage_states: [[0; 32]; 8],
        textures: [Kept::default(); 8],
        transforms: Vec::new(),
        vertex_shader: 0,
        stream: (Kept::default(), 0),
        indices: (Kept::default(), 0),
        render_target: Kept::default(),
        modes: Vec::new(),
        current_mode: [640, 480, 60],
    }))
}

/// True when the game got its Direct3D from this module.
#[inline]
pub fn present() -> bool {
    PRESENT.load(Ordering::Relaxed)
}

/// The size of the back buffer.
pub unsafe fn size() -> (u32, u32) {
    let lock = crate::sync::device_lock();
    let s = state(&lock);
    (s.parameters[0].max(1), s.parameters[1].max(1))
}

/// A COM object of this module: the method table and the reference count.
#[repr(C)]
struct Object {
    table: *const usize,
    references: u32,
}

/// A method table with one word before it and room after it: the recorder of `device`
/// copies 160 words of a device table and the word before them.
struct Table(core::cell::UnsafeCell<[usize; 162]>);
unsafe impl Sync for Table {}

static DIRECT3D_TABLE: Table = Table(core::cell::UnsafeCell::new([0; 162]));
static DEVICE_TABLE: Table = Table(core::cell::UnsafeCell::new([0; 162]));

unsafe fn new_object(table: &'static Table, methods: &[usize]) -> usize {
    let words = &mut *table.0.get();
    // A method past the interface must not be a jump to 0.
    words.fill(d3d_nothing as *const () as usize);
    words[1..1 + methods.len()].copy_from_slice(methods);
    Box::into_raw(Box::new(Object { table: words.as_ptr().add(1), references: 1 })) as usize
}

unsafe fn put(out: u32, words: &[u32]) -> u32 {
    if out == 0 {
        return INVALID_CALL;
    }
    core::ptr::copy_nonoverlapping(words.as_ptr(), out as usize as *mut u32, words.len());
    OK
}

// ---- IDirect3D8

unsafe extern "system" fn d3d_nothing(_this: usize) -> u32 {
    OK
}

unsafe extern "system" fn d3d_query_interface(_this: usize, _id: u32, out: *mut usize) -> u32 {
    if !out.is_null() {
        *out = 0;
    }
    NO_INTERFACE
}

unsafe extern "system" fn d3d_add_reference(this: *mut Object) -> u32 {
    (*this).references += 1;
    (*this).references
}

/// The objects of this module stay for the life of the process.
unsafe extern "system" fn d3d_release(this: *mut Object) -> u32 {
    (*this).references = (*this).references.saturating_sub(1).max(1);
    (*this).references
}

unsafe extern "system" fn d3d_register_software_device(_this: usize, _function: u32) -> u32 {
    OK
}

unsafe extern "system" fn d3d_adapter_count(_this: usize) -> u32 {
    1
}

/// A D3DADAPTER_IDENTIFIER8 has 1068 bytes: the driver and the description, 512 bytes each.
unsafe extern "system" fn d3d_adapter_identifier(_this: usize, _adapter: u32, _flags: u32, out: *mut u8) -> u32 {
    if out.is_null() {
        return INVALID_CALL;
    }
    core::ptr::write_bytes(out, 0, 1068);
    for (at, text) in [(0usize, &b"nirender.dll"[..]), (512, &b"nirender render link"[..])] {
        core::ptr::copy_nonoverlapping(text.as_ptr(), out.add(at), text.len());
    }
    OK
}

/// Reads the display modes of the computer with 32 bits for each pixel.
unsafe fn read_modes(s: &mut State) {
    if !s.modes.is_empty() {
        return;
    }
    // A DEVMODEA has 156 bytes: its size at 36, the bits for each pixel at 104, the width at
    // 108, the height at 112 and the refresh rate at 120.
    let mut mode = [0u8; 256];
    let read = |mode: &[u8], at: usize| u32::from_le_bytes([mode[at], mode[at + 1], mode[at + 2], mode[at + 3]]);
    mode[36..38].copy_from_slice(&156u16.to_le_bytes());
    // ENUM_CURRENT_SETTINGS
    if EnumDisplaySettingsA(core::ptr::null(), 0xFFFF_FFFF, mode.as_mut_ptr()) != 0 {
        s.current_mode = [read(&mode, 108), read(&mode, 112), read(&mode, 120)];
    }
    let mut index = 0;
    while index < 4096 && EnumDisplaySettingsA(core::ptr::null(), index, mode.as_mut_ptr()) != 0 {
        let entry = [read(&mode, 108), read(&mode, 112), read(&mode, 120)];
        if read(&mode, 104) == 32 && !s.modes.contains(&entry) {
            s.modes.push(entry);
        }
        index += 1;
    }
    if s.modes.is_empty() {
        s.modes.push(s.current_mode);
    }
}

unsafe extern "system" fn d3d_adapter_mode_count(_this: usize, _adapter: u32) -> u32 {
    let lock = crate::sync::device_lock();
    let mut s = state(&lock);
    read_modes(&mut s);
    s.modes.len() as u32
}

/// A D3DDISPLAYMODE: width, height, refresh rate, format.
unsafe extern "system" fn d3d_enum_adapter_modes(_this: usize, _adapter: u32, index: u32, out: u32) -> u32 {
    let lock = crate::sync::device_lock();
    let mut s = state(&lock);
    read_modes(&mut s);
    match s.modes.get(index as usize) {
        Some(mode) => put(out, &[mode[0], mode[1], mode[2], DISPLAY_FORMAT]),
        None => INVALID_CALL,
    }
}

unsafe extern "system" fn d3d_adapter_display_mode(_this: usize, _adapter: u32, out: u32) -> u32 {
    let lock = crate::sync::device_lock();
    let mut s = state(&lock);
    read_modes(&mut s);
    put(out, &[s.current_mode[0], s.current_mode[1], s.current_mode[2], DISPLAY_FORMAT])
}

/// Only the hardware device, D3DDEVTYPE_HAL.
unsafe extern "system" fn d3d_check_device_type(_this: usize, _adapter: u32, kind: u32, display: u32, back: u32, _windowed: u32) -> u32 {
    if kind == 1 && (display == 21 || display == 22) && (back == 21 || back == 22) {
        OK
    } else {
        NOT_AVAILABLE
    }
}

/// Usage: 1 is a target of draws and 2 is a depth buffer.
unsafe extern "system" fn d3d_check_device_format(_this: usize, _adapter: u32, _kind: u32, _display: u32, usage: u32, _resource: u32, format: u32) -> u32 {
    let accepted = if usage & 2 != 0 {
        DEPTH_FORMATS.contains(&format)
    } else if usage & 1 != 0 {
        TARGET_FORMATS.contains(&format)
    } else {
        TARGET_FORMATS.contains(&format) || TEXTURE_ONLY_FORMATS.contains(&format) || DXT.contains(&format)
    };
    if accepted {
        OK
    } else {
        NOT_AVAILABLE
    }
}

/// The host program draws without multisampling.
unsafe extern "system" fn d3d_check_multisample(_this: usize, _adapter: u32, _kind: u32, _format: u32, _windowed: u32, samples: u32) -> u32 {
    if samples <= 1 {
        OK
    } else {
        NOT_AVAILABLE
    }
}

unsafe extern "system" fn d3d_check_depth_match(_this: usize, _adapter: u32, _kind: u32, _display: u32, _target: u32, depth: u32) -> u32 {
    if DEPTH_FORMATS.contains(&depth) {
        OK
    } else {
        NOT_AVAILABLE
    }
}

unsafe extern "system" fn d3d_device_caps(_this: usize, _adapter: u32, _kind: u32, out: u32) -> u32 {
    put(out, &CAPS)
}

/// MONITOR_DEFAULTTOPRIMARY
unsafe extern "system" fn d3d_adapter_monitor(_this: usize, _adapter: u32) -> u32 {
    MonitorFromWindow(0, 1) as u32
}

unsafe extern "system" fn d3d_create_device(this: *mut Object, _adapter: u32, _kind: u32, window: u32, behavior: u32, parameters: *mut u32, out: *mut usize) -> u32 {
    if parameters.is_null() || out.is_null() {
        return INVALID_CALL;
    }
    let lock = crate::sync::device_lock();
    let mut s = state(&lock);
    read_modes(&mut s);
    accept_parameters(&mut s, parameters);
    s.focus_window = window;
    s.behavior = behavior;
    let device = new_object(&DEVICE_TABLE, &method_table());
    DEVICE.store(device, Ordering::Relaxed);
    d3d_add_reference(this);
    crate::log!(
        "own device: made, {} x {}, window {:#x}, behavior {behavior:#x}",
        s.parameters[0],
        s.parameters[1],
        device_window(&s)
    );
    start_stream(&s);
    *out = device;
    OK
}

/// Takes the D3DPRESENT_PARAMETERS of the game: width, height, format, back buffer count,
/// multisample type, swap effect, window, windowed, depth buffer, depth format, flags,
/// refresh rate, interval. A size or a format of 0 is that of the display, and the game
/// reads the values back.
unsafe fn accept_parameters(s: &mut State, parameters: *mut u32) {
    let given = core::slice::from_raw_parts_mut(parameters, 13);
    if given[0] == 0 || given[1] == 0 {
        given[0] = s.current_mode[0];
        given[1] = s.current_mode[1];
    }
    if given[2] == 0 {
        given[2] = DISPLAY_FORMAT;
    }
    given[3] = given[3].max(1);
    s.parameters.copy_from_slice(given);
    s.viewport = [0, 0, given[0], given[1], 0, 1.0f32.to_bits()];
}

unsafe fn device_window(s: &State) -> u32 {
    if s.parameters[6] != 0 {
        s.parameters[6]
    } else {
        s.focus_window
    }
}

/// Starts the stream and the host program that draws it in the window of the game. The
/// host program is `nirender-host.exe`: the file that the environment variable
/// NIRENDER_HOST_EXE names, or the one in the folder of this plugin.
unsafe fn start_stream(s: &State) {
    crate::own_host::activate();
    GAME_WINDOW.store(device_window(s) as usize, Ordering::Relaxed);
    let name = format!("nirender-{}", GetCurrentProcessId());
    let megabytes = std::env::var("NIRENDER_RING_MB").ok().and_then(|text| text.parse().ok()).unwrap_or(64);
    if let Err(problem) = crate::capture::start_direct(&name, megabytes, s.parameters[0], s.parameters[1], device_window(s)) {
        crate::log!("own device: no stream: {problem}");
        return;
    }
    let program = std::env::var("NIRENDER_HOST_EXE").ok().filter(|path| std::path::Path::new(path).is_file()).or_else(|| {
        let mut module = 0usize;
        let mut path = [0u8; 600];
        // GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
        GetModuleHandleExA(4, start_stream as *const core::ffi::c_void, &mut module);
        let length = GetModuleFileNameA(module, path.as_mut_ptr(), 512) as usize;
        let folder = std::path::Path::new(core::str::from_utf8(&path[..length]).ok()?).parent()?.join("nirender-host.exe");
        folder.is_file().then(|| folder.to_string_lossy().into_owned())
    });
    let Some(program) = program else {
        crate::log!("own device: no nirender-host.exe; the stream {name} waits for a host program");
        return;
    };
    let mut command = format!("\"{program}\" --live {name} --embed");
    if let Ok(backend) = std::env::var("NIRENDER_BACKEND") {
        command.push_str(&format!(" --backend {backend}"));
    }
    // More arguments for the host program, for a test.
    if let Ok(more) = std::env::var("NIRENDER_HOST_ARGS") {
        command.push_str(&format!(" {more}"));
    }
    // The text of the host program goes into a file: the one that NIRENDER_HOST_LOG names, or
    // nirender-host.log in the folder of the game.
    let log = std::env::var("NIRENDER_HOST_LOG").unwrap_or_else(|_| "nirender-host.log".into());
    let log = std::path::absolute(&log).map(|path| path.to_string_lossy().into_owned()).unwrap_or(log);
    command.push_str(&format!(" --log \"{log}\""));
    let mut line = command.clone().into_bytes();
    line.push(0);
    // A STARTUPINFOA has 68 bytes and starts with its size.
    let mut startup = [0u8; 68];
    startup[0] = 68;
    let mut information = [0usize; 4];
    let started = CreateProcessA(
        core::ptr::null(),
        line.as_mut_ptr(),
        core::ptr::null(),
        core::ptr::null(),
        0,
        0,
        core::ptr::null(),
        core::ptr::null(),
        startup.as_ptr(),
        information.as_mut_ptr(),
    );
    if started != 0 {
        // The process stays open: `check_host` asks it whether it has ended.
        HOST_PROCESS.store(information[0] as isize, Ordering::Relaxed);
        let file = std::path::Path::new(&program).file_name().map(|name| name.to_string_lossy().into_owned());
        let _ = HOST_NAME.set(file.unwrap_or(program.clone()));
        CloseHandle(information[1] as isize);
    }
    crate::log!("own device: host program {}: {command}", if started != 0 { "started" } else { "NOT started" });
}

/// Puts data of another program into the stream, for its part in the renderer: `id` is a
/// number that the two parts agree on. With the bit `capture::EXTENSION_KEPT` the data is a
/// state: the stream and each capture file start with its last record. The `d3d8.dll` of MGE
/// XE calls it.
#[no_mangle]
pub unsafe extern "C" fn nirender_stream_extension(id: u32, data: *const u8, length: u32) {
    if !data.is_null() && length != 0 && length < (1 << 20) {
        crate::capture::extension(id, core::slice::from_raw_parts(data, length as usize));
    }
}

/// The `Direct3DCreate8` of the plugin. The `d3d8.dll` of this repository calls it.
#[no_mangle]
pub unsafe extern "C" fn nirender_direct3d_create8(_sdk_version: u32) -> usize {
    if crate::install::refuse_without_ports() {
        crate::log!("own device: the game asked for a Direct3D 8; refused (see above): it gets the Direct3D 9 of MGE XE");
        return 0;
    }
    PRESENT.store(true, Ordering::Relaxed);
    let methods: [usize; 16] = [
        d3d_query_interface as *const () as usize,
        d3d_add_reference as *const () as usize,
        d3d_release as *const () as usize,
        d3d_register_software_device as *const () as usize,
        d3d_adapter_count as *const () as usize,
        d3d_adapter_identifier as *const () as usize,
        d3d_adapter_mode_count as *const () as usize,
        d3d_enum_adapter_modes as *const () as usize,
        d3d_adapter_display_mode as *const () as usize,
        d3d_check_device_type as *const () as usize,
        d3d_check_device_format as *const () as usize,
        d3d_check_multisample as *const () as usize,
        d3d_check_depth_match as *const () as usize,
        d3d_device_caps as *const () as usize,
        d3d_adapter_monitor as *const () as usize,
        d3d_create_device as *const () as usize,
    ];
    static DIRECT3D: AtomicUsize = AtomicUsize::new(0);
    let mut object = DIRECT3D.load(Ordering::Relaxed);
    if object == 0 {
        object = new_object(&DIRECT3D_TABLE, &methods);
        DIRECT3D.store(object, Ordering::Relaxed);
        crate::log!("own device: the game asked for a Direct3D 8; it gets the one of the plugin");
    } else {
        d3d_add_reference(object as *mut Object);
    }
    object
}

// ---- IDirect3DDevice8

/// Adds a reference to a resource through its method table (`AddRef`, slot 1).
unsafe fn resource_add_reference(object: u32) {
    if object != 0 {
        let table: usize = *(object as usize as *const usize);
        let add: unsafe extern "system" fn(u32) -> u32 = core::mem::transmute(*((table + 4) as *const usize));
        add(object);
    }
}

const IDENTITY: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// Does a call of the device. Returns [`None`] for a method that needs nothing here.
///
/// `a` are the arguments without the device. The numbers are the places of the methods in
/// `IDirect3DDevice8`; `device::NAMES` has the names.
pub unsafe fn answer(slot: usize, a: &[u32]) -> Option<u32> {
    use crate::own_host::direct_call as call;
    let lock = crate::sync::device_lock();
    let mut s = state(&lock);
    let device = DEVICE.load(Ordering::Relaxed) as *mut Object;
    Some(match slot {
        0 => {
            // QueryInterface
            if a[1] != 0 {
                *(a[1] as usize as *mut usize) = 0;
            }
            NO_INTERFACE
        }
        1 => d3d_add_reference(device),
        2 => d3d_release(device),
        // GetAvailableTextureMem: a number of bytes, not a result.
        4 => 0x4000_0000,
        6 => {
            // GetDirect3D
            let direct3d = nirender_direct3d_create8(0);
            put(a[0], &[direct3d as u32])
        }
        7 => put(a[0], &CAPS),
        8 => put(a[0], &[s.current_mode[0], s.current_mode[1], s.current_mode[2], DISPLAY_FORMAT]),
        // GetCreationParameters: adapter, device type, focus window, behavior.
        9 => put(a[0], &[0, 1, s.focus_window, s.behavior]),
        // CreateAdditionalSwapChain, CreateVolumeTexture, CreateCubeTexture
        13 | 21 | 22 => INVALID_CALL,
        14 => {
            // Reset
            if a[0] == 0 {
                return Some(INVALID_CALL);
            }
            accept_parameters(&mut s, a[0] as usize as *mut u32);
            crate::log!("own device: reset, {} x {}", s.parameters[0], s.parameters[1]);
            OK
        }
        15 => call(CALL_PRESENT, a),
        // GetBackBuffer(index, type, address)
        16 => call(CALL_GET_RENDER_TARGET, &a[2..3]),
        // GetRasterStatus: not in a vertical blank, line 0.
        17 => put(a[0], &[0, 0]),
        18 => {
            // SetGammaRamp(flags, ramp): the helper applies it to the picture of the window.
            if a[1] != 0 {
                crate::capture::gamma_ramp(core::slice::from_raw_parts(a[1] as usize as *const u8, crate::capture::GAMMA_RAMP_BYTES));
            }
            OK
        }
        19 => {
            // GetGammaRamp(address)
            if a[0] != 0 {
                let ramp = crate::capture::gamma_ramp_now();
                core::ptr::copy_nonoverlapping(ramp.as_ptr(), a[0] as usize as *mut u8, ramp.len());
            }
            OK
        }
        20 => call(CALL_CREATE_TEXTURE, a),
        23 => call(CALL_CREATE_VERTEX_BUFFER, a),
        24 => call(CALL_CREATE_INDEX_BUFFER, a),
        // CreateRenderTarget(width, height, format, multisample, lockable, address)
        25 => call(CALL_CREATE_IMAGE_SURFACE, &[a[0], a[1], a[2], a[5]]),
        26 => call(CALL_CREATE_DEPTH_BUFFER, a),
        27 => call(CALL_CREATE_IMAGE_SURFACE, a),
        28 => call(CALL_COPY_PIXELS, a),
        31 => {
            s.render_target = Kept::of(a[0]);
            call(CALL_RENDER_TARGET, a)
        }
        32 => {
            // GetRenderTarget
            let target = s.render_target.live();
            if target != 0 {
                resource_add_reference(target);
                put(a[0], &[target])
            } else {
                call(CALL_GET_RENDER_TARGET, a)
            }
        }
        33 => call(CALL_GET_DEPTH_BUFFER, a),
        34 => call(CALL_BEGIN_SCENE, a),
        35 => call(CALL_END_SCENE, a),
        36 => call(CALL_CLEAR, a),
        37 => {
            // SetTransform(state, matrix)
            if a[1] == 0 {
                return Some(INVALID_CALL);
            }
            let matrix: [f32; 16] = *(a[1] as usize as *const [f32; 16]);
            match s.transforms.iter_mut().find(|(state, _)| *state == a[0]) {
                Some(entry) => entry.1 = matrix,
                None => s.transforms.push((a[0], matrix)),
            }
            crate::capture::command(COMMAND_TRANSFORM, a[0], a[1], 0);
            OK
        }
        38 => {
            // GetTransform
            let matrix = s.transforms.iter().find(|(state, _)| *state == a[0]).map_or(IDENTITY, |entry| entry.1);
            if a[1] == 0 {
                return Some(INVALID_CALL);
            }
            *(a[1] as usize as *mut [f32; 16]) = matrix;
            OK
        }
        40 => {
            // SetViewport
            if a[0] == 0 {
                return Some(INVALID_CALL);
            }
            s.viewport = *(a[0] as usize as *const [u32; 6]);
            call(CALL_VIEWPORT, a)
        }
        41 => put(a[0], &s.viewport),
        // GetMaterial: 17 floats.
        43 => put(a[0], &[0; 17]),
        // GetLight: a D3DLIGHT8 has 26 words.
        45 => put(a[1], &[0; 26]),
        // GetLightEnable
        47 => put(a[1], &[0]),
        // GetClipPlane
        49 => put(a[1], &[0; 4]),
        50 => {
            // SetRenderState
            if let Some(value) = s.render_states.get_mut(a[0] as usize) {
                *value = a[1];
            }
            crate::capture::command(COMMAND_RENDER_STATE, a[0], a[1], 0);
            OK
        }
        51 => put(a[1], &[s.render_states.get(a[0] as usize).copied().unwrap_or(0)]),
        // CreateStateBlock(type, address of the token)
        57 => put(a[1], &[1]),
        // GetClipStatus
        59 => put(a[0], &[0, 0]),
        60 => {
            // GetTexture(stage, address)
            // A handle this device kept, as a getter returns it: the handle while its object
            // lives (`Kept::live`). The device holds no count on what it keeps (unlike Direct3D
            // 8), so the object can have been released since, and its address taken by
            // another. A count added through a released handle writes into freed memory.
            let texture = s.textures.get(a[0] as usize).map_or(0, |kept| kept.live());
            resource_add_reference(texture);
            put(a[1], &[texture])
        }
        61 => {
            if let Some(texture) = s.textures.get_mut(a[0] as usize) {
                *texture = Kept::of(a[1]);
            }
            OK
        }
        62 => put(a[2], &[s.stage_states.get(a[0] as usize).and_then(|stage| stage.get(a[1] as usize)).copied().unwrap_or(0)]),
        63 => {
            // SetTextureStageState(stage, state, value)
            if let Some(value) = s.stage_states.get_mut(a[0] as usize).and_then(|stage| stage.get_mut(a[1] as usize)) {
                *value = a[2];
            }
            crate::capture::command(COMMAND_STAGE_STATE, a[0], a[1], a[2]);
            OK
        }
        // ValidateDevice: one pass.
        64 => put(a[0], &[1]),
        // GetInfo, GetPaletteEntries, GetCurrentTexturePalette: not in this device.
        65 | 67 | 69 => INVALID_CALL,
        70 => {
            crate::capture::command(COMMAND_DRAW, a.as_ptr() as u32, 0, 0);
            OK
        }
        71 => {
            crate::capture::command(COMMAND_DRAW_INDEXED, a.as_ptr() as u32, 0, 0);
            OK
        }
        72 => {
            // DrawPrimitiveUP(type, primitives, vertices, bytes of a vertex): the films of
            // the game.
            let length = crate::device::vertex_count(a[0], a[1]) * a[3];
            if a[2] != 0 && length != 0 {
                let vertices = core::slice::from_raw_parts(a[2] as usize as *const u8, length as usize);
                crate::capture::draw_up(s.textures[0].live(), s.vertex_shader, a[3], a[0], a[1], vertices);
            }
            OK
        }
        // ProcessVertices, CreateVertexShader, CreatePixelShader: no shaders and no software
        // vertex work in this device.
        74 | 75 | 87 => INVALID_CALL,
        76 => {
            s.vertex_shader = a[0];
            OK
        }
        77 => put(a[0], &[s.vertex_shader]),
        // The read of a shader constant, of a shader declaration or of a shader function.
        80 | 81 | 82 | 92 | 93 => INVALID_CALL,
        83 => {
            // SetStreamSource(stream, buffer, stride)
            if a[0] == 0 {
                s.stream = (Kept::of(a[1]), a[2]);
                #[cfg(feature = "rig")]
                crate::resource::wrap(a[1] as usize);
            }
            OK
        }
        84 => {
            let stream = s.stream.0.live();
            resource_add_reference(stream);
            put(a[1], &[stream]);
            put(a[2], &[s.stream.1])
        }
        85 => {
            s.indices = (Kept::of(a[0]), a[1]);
            #[cfg(feature = "rig")]
            crate::resource::wrap(a[0] as usize);
            OK
        }
        86 => {
            let indices = s.indices.0.live();
            resource_add_reference(indices);
            put(a[0], &[indices]);
            put(a[1], &[s.indices.1])
        }
        89 => put(a[0], &[0]),
        _ => return None,
    })
}

/// A call that came to the device itself: the link did not take it.
unsafe fn base_call(slot: usize, _this: usize, args: &[u32]) -> u32 {
    answer(slot, args).unwrap_or(OK)
}

macro_rules! methods {
    ($( $slot:literal => $name:ident ( $($a:ident),* ); )*) => {
        $(
            unsafe extern "system" fn $name(this: usize $(, $a: u32)*) -> u32 {
                base_call($slot, this, &[$($a),*])
            }
        )*
        fn method_table() -> [usize; crate::device::SLOTS] {
            [$( $name as *const () as usize ),*]
        }
    };
}

methods! {
    0 => m0(a, b); 1 => m1(); 2 => m2(); 3 => m3(); 4 => m4(); 5 => m5(a); 6 => m6(a); 7 => m7(a); 8 => m8(a); 9 => m9(a);
    10 => m10(a, b, c); 11 => m11(a, b, c); 12 => m12(a); 13 => m13(a, b); 14 => m14(a); 15 => m15(a, b, c, d);
    16 => m16(a, b, c); 17 => m17(a); 18 => m18(a, b); 19 => m19(a);
    20 => m20(a, b, c, d, e, f, g); 21 => m21(a, b, c, d, e, f, g, h); 22 => m22(a, b, c, d, e, f); 23 => m23(a, b, c, d, e);
    24 => m24(a, b, c, d, e); 25 => m25(a, b, c, d, e, f); 26 => m26(a, b, c, d, e); 27 => m27(a, b, c, d);
    28 => m28(a, b, c, d, e); 29 => m29(a, b);
    30 => m30(a); 31 => m31(a, b); 32 => m32(a); 33 => m33(a); 34 => m34(); 35 => m35(); 36 => m36(a, b, c, d, e, f);
    37 => m37(a, b); 38 => m38(a, b); 39 => m39(a, b);
    40 => m40(a); 41 => m41(a); 42 => m42(a); 43 => m43(a); 44 => m44(a, b); 45 => m45(a, b); 46 => m46(a, b); 47 => m47(a, b);
    48 => m48(a, b); 49 => m49(a, b);
    50 => m50(a, b); 51 => m51(a, b); 52 => m52(); 53 => m53(a); 54 => m54(a); 55 => m55(a); 56 => m56(a); 57 => m57(a, b);
    58 => m58(a); 59 => m59(a);
    60 => m60(a, b); 61 => m61(a, b); 62 => m62(a, b, c); 63 => m63(a, b, c); 64 => m64(a); 65 => m65(a, b, c); 66 => m66(a, b);
    67 => m67(a, b); 68 => m68(a); 69 => m69(a);
    70 => m70(a, b, c); 71 => m71(a, b, c, d, e); 72 => m72(a, b, c, d); 73 => m73(a, b, c, d, e, f, g, h); 74 => m74(a, b, c, d, e);
    75 => m75(a, b, c, d); 76 => m76(a); 77 => m77(a); 78 => m78(a); 79 => m79(a, b, c);
    80 => m80(a, b, c); 81 => m81(a, b, c); 82 => m82(a, b, c); 83 => m83(a, b, c); 84 => m84(a, b, c); 85 => m85(a, b);
    86 => m86(a, b); 87 => m87(a, b); 88 => m88(a); 89 => m89(a);
    90 => m90(a); 91 => m91(a, b, c); 92 => m92(a, b, c); 93 => m93(a, b, c); 94 => m94(a, b, c); 95 => m95(a, b, c); 96 => m96(a);
}
