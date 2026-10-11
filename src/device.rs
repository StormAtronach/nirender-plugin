//! The recorder on the `IDirect3DDevice8` the engine draws with.
//!
//! `attach` replaces the vtable pointer of the device object with a table of wrappers. A
//! wrapper can record the call and can skip it. A recording that skips calls is "dry". The
//! Rust function runs dry for a comparison, so it draws nothing.
//!
//! A record in the log has the slot, the argument count, the arguments, the payload length in
//! words and the payload. The payload is the bytes behind a pointer argument. Two logs are
//! equal when the words are equal.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub const SLOTS: usize = 97;
/// The copy goes past the interface, because the device class can have more virtual functions.
const TABLE_LEN: usize = 160;

pub const SLOT_PRESENT: usize = 15;
const SLOT_CREATE_VERTEX_BUFFER: usize = 23;
const SLOT_CREATE_INDEX_BUFFER: usize = 24;
const SLOT_SET_STREAM_SOURCE: usize = 83;
const SLOT_SET_INDICES: usize = 85;

/// The number of stack arguments of each method, without `this`.
pub const ARGS: [u8; SLOTS] = [
    2, 0, 0, 0, 0, 1, 1, 1, 1, 1, // 0 QueryInterface .. 9 GetCreationParameters
    3, 3, 1, 2, 1, 4, 3, 1, 2, 1, // 10 SetCursorProperties .. 19 GetGammaRamp
    7, 8, 6, 5, 5, 6, 5, 4, 5, 2, // 20 CreateTexture .. 29 UpdateTexture
    1, 2, 1, 1, 0, 0, 6, 2, 2, 2, // 30 GetFrontBuffer .. 39 MultiplyTransform
    1, 1, 1, 1, 2, 2, 2, 2, 2, 2, // 40 SetViewport .. 49 GetClipPlane
    2, 2, 0, 1, 1, 1, 1, 2, 1, 1, // 50 SetRenderState .. 59 GetClipStatus
    2, 2, 3, 3, 1, 3, 2, 2, 1, 1, // 60 GetTexture .. 69 GetCurrentTexturePalette
    3, 5, 4, 8, 5, 4, 1, 1, 1, 3, // 70 DrawPrimitive .. 79 SetVertexShaderConstant
    3, 3, 3, 3, 3, 2, 2, 2, 1, 1, // 80 GetVertexShaderConstant .. 89 GetPixelShader
    1, 3, 3, 3, 3, 3, 1, // 90 DeletePixelShader .. 96 DeletePatch
];

pub const NAMES: [&str; SLOTS] = [
    "QueryInterface", "AddRef", "Release", "TestCooperativeLevel", "GetAvailableTextureMem",
    "ResourceManagerDiscardBytes", "GetDirect3D", "GetDeviceCaps", "GetDisplayMode", "GetCreationParameters",
    "SetCursorProperties", "SetCursorPosition", "ShowCursor", "CreateAdditionalSwapChain", "Reset",
    "Present", "GetBackBuffer", "GetRasterStatus", "SetGammaRamp", "GetGammaRamp",
    "CreateTexture", "CreateVolumeTexture", "CreateCubeTexture", "CreateVertexBuffer", "CreateIndexBuffer",
    "CreateRenderTarget", "CreateDepthStencilSurface", "CreateImageSurface", "CopyRects", "UpdateTexture",
    "GetFrontBuffer", "SetRenderTarget", "GetRenderTarget", "GetDepthStencilSurface", "BeginScene",
    "EndScene", "Clear", "SetTransform", "GetTransform", "MultiplyTransform",
    "SetViewport", "GetViewport", "SetMaterial", "GetMaterial", "SetLight",
    "GetLight", "LightEnable", "GetLightEnable", "SetClipPlane", "GetClipPlane",
    "SetRenderState", "GetRenderState", "BeginStateBlock", "EndStateBlock", "ApplyStateBlock",
    "CaptureStateBlock", "DeleteStateBlock", "CreateStateBlock", "SetClipStatus", "GetClipStatus",
    "GetTexture", "SetTexture", "GetTextureStageState", "SetTextureStageState", "ValidateDevice",
    "GetInfo", "SetPaletteEntries", "GetPaletteEntries", "SetCurrentTexturePalette", "GetCurrentTexturePalette",
    "DrawPrimitive", "DrawIndexedPrimitive", "DrawPrimitiveUP", "DrawIndexedPrimitiveUP", "ProcessVertices",
    "CreateVertexShader", "SetVertexShader", "GetVertexShader", "DeleteVertexShader", "SetVertexShaderConstant",
    "GetVertexShaderConstant", "GetVertexShaderDeclaration", "GetVertexShaderFunction", "SetStreamSource", "GetStreamSource",
    "SetIndices", "GetIndices", "CreatePixelShader", "SetPixelShader", "GetPixelShader",
    "DeletePixelShader", "SetPixelShaderConstant", "GetPixelShaderConstant", "GetPixelShaderFunction", "DrawRectPatch",
    "DrawTriPatch", "DeletePatch",
];

/// A dry run does not pass these calls on. They change device state or draw, and they return
/// no data.
pub fn dry_skips(slot: usize) -> bool {
    matches!(
        slot,
        15 | 18 | 31 | 34 | 35 | 36 | 37 | 39 | 40 | 42 | 44 | 46 | 48 | 50 | 54 | 58 | 61 | 63 | 66 | 68 | 70 | 71 | 72 | 73 | 76
            | 79 | 83 | 85 | 88 | 91 | 94 | 95
    )
}

/// The number of vertices that a primitive count gives.
pub fn vertex_count(primitive_type: u32, primitives: u32) -> u32 {
    match primitive_type {
        1 => primitives,         // point list
        2 => primitives * 2,     // line list
        3 => primitives + 1,     // line strip
        4 => primitives * 3,     // triangle list
        5 | 6 => primitives + 2, // triangle strip, fan
        _ => 0,
    }
}

/// The bytes behind the pointer argument of a call, as (address, length). The result is
/// (0, 0) when the call has none.
pub fn payload(slot: usize, a: &[u32]) -> (usize, usize) {
    let (ptr, len) = match slot {
        18 => (a[1], 0x600),                                // SetGammaRamp: D3DGAMMARAMP
        36 => (a[1], a[0] * 16),                            // Clear: D3DRECT[count]
        37 | 39 => (a[1], 64),                              // SetTransform, MultiplyTransform: D3DMATRIX
        40 => (a[0], 24),                                   // SetViewport: D3DVIEWPORT8
        42 => (a[0], 68),                                   // SetMaterial: D3DMATERIAL8
        44 => (a[1], 104),                                  // SetLight: D3DLIGHT8
        48 => (a[1], 16),                                   // SetClipPlane: float[4]
        72 => (a[2], vertex_count(a[0], a[1]) * a[3]),      // DrawPrimitiveUP: vertices
        79 | 91 => (a[1], a[2] * 16),                       // Set*ShaderConstant: float[4][count]
        _ => (0, 0),
    };
    if ptr == 0 || len == 0 {
        (0, 0)
    } else {
        (ptr as usize, len as usize)
    }
}

/// The arguments that are addresses of the storage of the caller. Such an address is the
/// place for a result or the place of the payload data. Bit n is argument n. The log holds 0
/// for them. The Rust function and the engine function keep such storage in different places.
pub fn address_args(slot: usize) -> u32 {
    match slot {
        0 | 13 | 44 | 45 | 47 | 49 | 51 | 57 | 60 | 65 | 67 | 80 | 87 | 92 => 0b10,
        6 | 7 | 8 | 9 | 17 | 19 | 32 | 33 | 40 | 41 | 42 | 43 | 53 | 59 | 64 | 69 | 77 | 89 => 0b1,
        16 | 62 | 72 => 0b100,
        18 | 36 | 37 | 38 | 39 | 48 | 79 | 91 => 0b10,
        20 => 1 << 6,
        21 => 1 << 7,
        22 | 25 => 1 << 5,
        23 | 24 | 26 => 1 << 4,
        27 => 1 << 3,
        75 => 0b100,
        81 | 82 | 84 | 93 => 0b110,
        86 => 0b11,
        _ => 0,
    }
}

fn makes_buffer(slot: usize) -> bool {
    slot == SLOT_CREATE_VERTEX_BUFFER || slot == SLOT_CREATE_INDEX_BUFFER
}

/// The methods that make a device resource and return it through their last argument.
pub fn creates(slot: usize) -> bool {
    (20..=27).contains(&slot)
}

/// A resource that the Rust run of a comparison made.
struct Created {
    slot: usize,
    args: [u32; 8],
    result: u32,
    object: usize,
}

struct Replay {
    /// The engine run takes the resources of the Rust run and does not make its own.
    on: bool,
    created: Vec<Created>,
    taken: usize,
    mismatch: bool,
}

static mut REPLAY: Replay = Replay { on: false, created: Vec::new(), taken: 0, mismatch: false };

/// Starts the engine run of a comparison. The run is recorded and real. It gets the
/// resources that the Rust run made, in the same order.
pub unsafe fn begin_engine_run() {
    (*core::ptr::addr_of_mut!(LOG)).clear();
    let replay = &mut *core::ptr::addr_of_mut!(REPLAY);
    replay.on = true;
    replay.taken = 0;
    replay.mismatch = false;
    DRY.store(false, Ordering::Relaxed);
    RECORDING.store(true, Ordering::Relaxed);
}

/// Ends a comparison. Returns the differences in the resources made, as text. The text is
/// empty when the engine run made the same resources as the Rust run.
pub unsafe fn end_engine_run() -> String {
    let replay = &mut *core::ptr::addr_of_mut!(REPLAY);
    replay.on = false;
    let mut text = String::new();
    if replay.mismatch {
        text.push_str(" the engine made a resource the rust run did not;");
    }
    if replay.taken < replay.created.len() {
        text.push_str(&format!(" rust made {} resources, the engine {};", replay.created.len(), replay.taken));
        for created in &replay.created[replay.taken..] {
            if created.object != 0 {
                // No object holds the resource. This releases the reference that the creation gave.
                let vtable = *(created.object as *const usize);
                let release: unsafe extern "system" fn(usize) -> u32 = core::mem::transmute(*((vtable + 8) as *const usize));
                release(created.object);
            }
        }
    }
    replay.created.clear();
    replay.taken = 0;
    text
}

static mut ORIG: [usize; TABLE_LEN + 1] = [0; TABLE_LEN + 1];
static mut OURS: [usize; TABLE_LEN + 1] = [0; TABLE_LEN + 1];

static RECORDING: AtomicBool = AtomicBool::new(false);
static DRY: AtomicBool = AtomicBool::new(false);
/// The calls that the device makes on itself during a call are not calls of the game.
static DEPTH: AtomicU32 = AtomicU32::new(0);
static mut LOG: Vec<u32> = Vec::new();
static mut FRAME_HOOK: Option<unsafe fn()> = None;

/// Starts the recording of a frame. Every call goes to the device.
pub unsafe fn begin_frame() {
    (*core::ptr::addr_of_mut!(LOG)).clear();
    DRY.store(false, Ordering::Relaxed);
    RECORDING.store(true, Ordering::Relaxed);
}

/// Starts the Rust run of a comparison. The run is dry: the calls that set state or draw do
/// not reach the device.
pub unsafe fn begin_rust_run() {
    (*core::ptr::addr_of_mut!(LOG)).clear();
    let replay = &mut *core::ptr::addr_of_mut!(REPLAY);
    replay.created.clear();
    replay.taken = 0;
    DRY.store(true, Ordering::Relaxed);
    RECORDING.store(true, Ordering::Relaxed);
}

/// Ends the recording and moves the log into `into`.
pub unsafe fn end(into: &mut Vec<u32>) {
    RECORDING.store(false, Ordering::Relaxed);
    DRY.store(false, Ordering::Relaxed);
    into.clear();
    core::mem::swap(into, &mut *core::ptr::addr_of_mut!(LOG));
}

/// A record that is not a device call. It holds the data of a buffer at Unlock. The log gets
/// only the calls that the game makes. It does not get what the device does inside one of
/// its own calls.
pub unsafe fn log_extra(slot: usize, args: &[u32], payload_bytes: &[u8]) {
    if RECORDING.load(Ordering::Relaxed) && DEPTH.load(Ordering::Relaxed) == 0 {
        append_record(&mut *core::ptr::addr_of_mut!(LOG), slot, args, payload_bytes);
    }
}

pub fn recording() -> bool {
    RECORDING.load(Ordering::Relaxed)
}

/// The recorder calls `hook` at each Present that the game makes, before the call goes on.
pub unsafe fn set_frame_hook(hook: unsafe fn()) {
    FRAME_HOOK = Some(hook);
}

pub fn append_record(log: &mut Vec<u32>, slot: usize, args: &[u32], payload_bytes: &[u8]) {
    log.push(slot as u32);
    log.push(args.len() as u32);
    log.extend_from_slice(args);
    let words = (payload_bytes.len() + 3) / 4;
    log.push(words as u32);
    let start = log.len();
    log.resize(start + words, 0);
    // This is safe. `words` u32 values cover the bytes, and the tail of the last word stays zero.
    unsafe {
        core::ptr::copy_nonoverlapping(payload_bytes.as_ptr(), log.as_mut_ptr().add(start) as *mut u8, payload_bytes.len());
    }
}

/// One record of a log. The result is `None` at the end or when the log is not complete.
pub struct Record<'a> {
    pub slot: usize,
    pub args: &'a [u32],
    pub payload: &'a [u32],
}

pub fn read_record(log: &[u32], at: usize) -> Option<(Record<'_>, usize)> {
    let slot = *log.get(at)? as usize;
    let argc = *log.get(at + 1)? as usize;
    let args = log.get(at + 2..at + 2 + argc)?;
    let words = *log.get(at + 2 + argc)? as usize;
    let payload = log.get(at + 3 + argc..at + 3 + argc + words)?;
    Some((Record { slot, args, payload }, at + 3 + argc + words))
}

pub fn count_records(log: &[u32]) -> usize {
    let (mut at, mut n) = (0, 0);
    while let Some((_, next)) = read_record(log, at) {
        at = next;
        n += 1;
    }
    n
}

/// The name of a log slot: a device method, or the record of a buffer at Unlock.
pub fn slot_name(slot: usize) -> &'static str {
    match slot {
        crate::resource::SLOT_BUFFER_UNLOCK => "Buffer::Unlock",
        slot => NAMES.get(slot).copied().unwrap_or("?"),
    }
}

pub fn describe(record: &Record<'_>) -> String {
    let name = slot_name(record.slot);
    let args: Vec<String> = record.args.iter().map(|a| format!("{a:#x}")).collect();
    let mut text = format!("{}({})", name, args.join(", "));
    if !record.payload.is_empty() {
        let words: Vec<String> = record.payload.iter().take(24).map(|w| format!("{w:08x}")).collect();
        text.push_str(&format!(" data[{}]: {}", record.payload.len(), words.join(" ")));
    }
    text
}

/// The first place two logs differ, as text, or `None` when they are equal.
pub fn first_difference(a: &[u32], b: &[u32]) -> Option<String> {
    if a == b {
        return None;
    }
    let (mut at_a, mut at_b, mut index) = (0, 0, 0);
    loop {
        match (read_record(a, at_a), read_record(b, at_b)) {
            (Some((ra, next_a)), Some((rb, next_b))) => {
                if a[at_a..next_a] != b[at_b..next_b] {
                    return Some(format!("call {index}: rust {} | engine {}", describe(&ra), describe(&rb)));
                }
                at_a = next_a;
                at_b = next_b;
                index += 1;
            }
            (Some((ra, _)), None) => return Some(format!("call {index}: rust {} | engine has no more calls", describe(&ra))),
            (None, Some((rb, _))) => return Some(format!("call {index}: rust has no more calls | engine {}", describe(&rb))),
            (None, None) => return Some("logs differ outside any record".into()),
        }
    }
}

unsafe fn call_original(slot: usize, this: usize, a: &[u32]) -> u32 {
    use core::mem::transmute as t;
    let f = ORIG[slot + 1];
    match a.len() {
        0 => t::<usize, unsafe extern "system" fn(usize) -> u32>(f)(this),
        1 => t::<usize, unsafe extern "system" fn(usize, u32) -> u32>(f)(this, a[0]),
        2 => t::<usize, unsafe extern "system" fn(usize, u32, u32) -> u32>(f)(this, a[0], a[1]),
        3 => t::<usize, unsafe extern "system" fn(usize, u32, u32, u32) -> u32>(f)(this, a[0], a[1], a[2]),
        4 => t::<usize, unsafe extern "system" fn(usize, u32, u32, u32, u32) -> u32>(f)(this, a[0], a[1], a[2], a[3]),
        5 => t::<usize, unsafe extern "system" fn(usize, u32, u32, u32, u32, u32) -> u32>(f)(this, a[0], a[1], a[2], a[3], a[4]),
        6 => t::<usize, unsafe extern "system" fn(usize, u32, u32, u32, u32, u32, u32) -> u32>(f)(this, a[0], a[1], a[2], a[3], a[4], a[5]),
        7 => t::<usize, unsafe extern "system" fn(usize, u32, u32, u32, u32, u32, u32, u32) -> u32>(f)(
            this, a[0], a[1], a[2], a[3], a[4], a[5], a[6],
        ),
        _ => t::<usize, unsafe extern "system" fn(usize, u32, u32, u32, u32, u32, u32, u32, u32) -> u32>(f)(
            this, a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7],
        ),
    }
}

/// Calls the method of the device. The calls that the device makes on itself during this call
/// are not calls of the game, so the depth is not 0 for them.
unsafe fn forward(slot: usize, this: usize, args: &[u32]) -> u32 {
    DEPTH.store(DEPTH.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
    let result = call_original(slot, this, args);
    DEPTH.store(DEPTH.load(Ordering::Relaxed) - 1, Ordering::Relaxed);
    result
}

/// Puts a call of the game into the log. Returns the result when the call does not go to the
/// device from here: a dry run skips it, or the call makes a resource of a comparison.
unsafe fn record(slot: usize, this: usize, args: &[u32]) -> Option<u32> {
    let (ptr, len) = payload(slot, args);
    let bytes: &[u8] = if len == 0 { &[] } else { core::slice::from_raw_parts(ptr as *const u8, len) };
    let mut logged = [0u32; 8];
    let mask = address_args(slot);
    for (i, arg) in args.iter().enumerate() {
        logged[i] = if mask & (1 << i) != 0 { 0 } else { *arg };
    }
    append_record(&mut *core::ptr::addr_of_mut!(LOG), slot, &logged[..args.len()], bytes);

    let dry = DRY.load(Ordering::Relaxed);
    if dry && dry_skips(slot) {
        return Some(0);
    }
    if !creates(slot) {
        return None;
    }
    let replay = &mut *core::ptr::addr_of_mut!(REPLAY);
    let out = args[args.len() - 1] as *mut usize;
    if replay.on {
        // The engine run gets the resource that the Rust run made.
        match replay.created.get(replay.taken) {
            Some(created) if created.slot == slot && created.args == logged => {
                replay.taken += 1;
                *out = created.object;
                return Some(created.result);
            }
            _ => replay.mismatch = true,
        }
        None
    } else if dry {
        // The Rust run makes the resource for real.
        let result = forward(slot, this, args);
        let object = if (result as i32) >= 0 { *out } else { 0 };
        if makes_buffer(slot) {
            crate::resource::wrap(object);
        }
        replay.created.push(Created { slot, args: logged, result, object });
        Some(result)
    } else {
        None
    }
}

const SLOT_TEST_COOPERATIVE_LEVEL: usize = crate::d3d::DEVICE_TEST_COOPERATIVE_LEVEL / 4;
/// A test asked for a reset of the device. See `device_call`.
pub static FORCE_RESET: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

unsafe fn device_call(slot: usize, this: usize, args: &[u32]) -> u32 {
    if DEPTH.load(Ordering::Relaxed) != 0 {
        return call_original(slot, this, args);
    }
    // A test asked for a reset: the next `TestCooperativeLevel` of the game says that the
    // device was lost and can be reset. `BeginPaint` then calls `Recreate`.
    if slot == SLOT_TEST_COOPERATIVE_LEVEL && !DRY.load(Ordering::Relaxed) && FORCE_RESET.swap(false, Ordering::Relaxed) {
        crate::log!("TestCooperativeLevel: D3DERR_DEVICENOTRESET on purpose");
        return crate::d3d::D3DERR_DEVICENOTRESET;
    }
    // A call that the host of the render link takes does not go to the device.
    if !DRY.load(Ordering::Relaxed) {
        if let Some(result) = crate::link_client::intercept(slot, args) {
            return result;
        }
    }
    if CALLER_CENSUS.load(Ordering::Relaxed) {
        crate::callers::count(slot, CALLER.load(Ordering::Relaxed) as usize);
    }
    // The Present of a dry run is not a frame of the game.
    if slot == SLOT_PRESENT && !DRY.load(Ordering::Relaxed) {
        crate::perf::on_present();
        if let Some(hook) = FRAME_HOOK {
            hook();
        }
    }
    if RECORDING.load(Ordering::Relaxed) {
        if let Some(result) = record(slot, this, args) {
            return result;
        }
    }
    // A buffer that the game binds gets the recorder. The time of its creation is not important.
    match slot {
        SLOT_SET_STREAM_SOURCE => crate::resource::wrap(args[1] as usize),
        SLOT_SET_INDICES => crate::resource::wrap(args[0] as usize),
        _ => {}
    }
    let result = forward(slot, this, args);
    if makes_buffer(slot) && (result as i32) >= 0 {
        crate::resource::wrap(*(args[args.len() - 1] as *const usize));
    }
    result
}

macro_rules! wrappers {
    ($( $slot:literal => $name:ident ( $($a:ident),* ); )*) => {
        $(
            unsafe extern "system" fn $name(this: usize $(, $a: u32)*) -> u32 {
                device_call($slot, this, &[$($a),*])
            }
        )*
        fn wrapper_table() -> [usize; SLOTS] {
            [$( $name as *const () as usize ),*]
        }
    };
}

wrappers! {
    0 => w0(a, b); 1 => w1(); 2 => w2(); 3 => w3(); 4 => w4(); 5 => w5(a); 6 => w6(a); 7 => w7(a); 8 => w8(a); 9 => w9(a);
    10 => w10(a, b, c); 11 => w11(a, b, c); 12 => w12(a); 13 => w13(a, b); 14 => w14(a); 15 => w15(a, b, c, d);
    16 => w16(a, b, c); 17 => w17(a); 18 => w18(a, b); 19 => w19(a);
    20 => w20(a, b, c, d, e, f, g); 21 => w21(a, b, c, d, e, f, g, h); 22 => w22(a, b, c, d, e, f); 23 => w23(a, b, c, d, e);
    24 => w24(a, b, c, d, e); 25 => w25(a, b, c, d, e, f); 26 => w26(a, b, c, d, e); 27 => w27(a, b, c, d);
    28 => w28(a, b, c, d, e); 29 => w29(a, b);
    30 => w30(a); 31 => w31(a, b); 32 => w32(a); 33 => w33(a); 34 => w34(); 35 => w35(); 36 => w36(a, b, c, d, e, f);
    37 => w37(a, b); 38 => w38(a, b); 39 => w39(a, b);
    40 => w40(a); 41 => w41(a); 42 => w42(a); 43 => w43(a); 44 => w44(a, b); 45 => w45(a, b); 46 => w46(a, b); 47 => w47(a, b);
    48 => w48(a, b); 49 => w49(a, b);
    50 => w50(a, b); 51 => w51(a, b); 52 => w52(); 53 => w53(a); 54 => w54(a); 55 => w55(a); 56 => w56(a); 57 => w57(a, b);
    58 => w58(a); 59 => w59(a);
    60 => w60(a, b); 61 => w61(a, b); 62 => w62(a, b, c); 63 => w63(a, b, c); 64 => w64(a); 65 => w65(a, b, c); 66 => w66(a, b);
    67 => w67(a, b); 68 => w68(a); 69 => w69(a);
    70 => w70(a, b, c); 71 => w71(a, b, c, d, e); 72 => w72(a, b, c, d); 73 => w73(a, b, c, d, e, f, g, h); 74 => w74(a, b, c, d, e);
    75 => w75(a, b, c, d); 76 => w76(a); 77 => w77(a); 78 => w78(a); 79 => w79(a, b, c);
    80 => w80(a, b, c); 81 => w81(a, b, c); 82 => w82(a, b, c); 83 => w83(a, b, c); 84 => w84(a, b, c); 85 => w85(a, b);
    86 => w86(a, b); 87 => w87(a, b); 88 => w88(a); 89 => w89(a);
    90 => w90(a); 91 => w91(a, b, c); 92 => w92(a, b, c); 93 => w93(a, b, c); 94 => w94(a, b, c); 95 => w95(a, b, c); 96 => w96(a);
}

/// The return address of the device call that is in progress. The entry code of each wrapper
/// writes it. See `entry_code`.
static CALLER: AtomicU32 = AtomicU32::new(0);
/// The wrappers count the callers of the device. See `callers.rs`.
static CALLER_CENSUS: AtomicBool = AtomicBool::new(false);

pub fn set_caller_census(on: bool) {
    CALLER_CENSUS.store(on, Ordering::Relaxed);
}

/// The bytes of the entry code of one wrapper, for the address `at`. A Rust function cannot
/// read its return address, so this code runs first:
///
/// ```text
/// mov eax, [esp]      ; the return address of the device call
/// mov [CALLER], eax
/// jmp wrapper
/// ```
///
/// EAX has no argument of a `__stdcall` method.
fn entry_code(at: usize, wrapper: usize) -> [u8; ENTRY_SIZE] {
    let mut code = [0x8B, 0x04, 0x24, 0xA3, 0, 0, 0, 0, 0xE9, 0, 0, 0, 0];
    code[4..8].copy_from_slice(&(core::ptr::addr_of!(CALLER) as u32).to_le_bytes());
    let relative = (wrapper as u32).wrapping_sub(at as u32 + ENTRY_SIZE as u32);
    code[9..13].copy_from_slice(&relative.to_le_bytes());
    code
}

const ENTRY_SIZE: usize = 13;

/// Makes the entry code of each wrapper. Returns the address of the first one, or 0.
unsafe fn make_entries() -> usize {
    let block = crate::win::VirtualAlloc(
        core::ptr::null_mut(),
        SLOTS * ENTRY_SIZE,
        crate::win::MEM_COMMIT_RESERVE,
        crate::win::PAGE_EXECUTE_READWRITE,
    ) as usize;
    if block != 0 {
        for (slot, wrapper) in wrapper_table().iter().enumerate() {
            let at = block + slot * ENTRY_SIZE;
            core::ptr::copy_nonoverlapping(entry_code(at, *wrapper).as_ptr(), at as *mut u8, ENTRY_SIZE);
        }
    }
    block
}

/// Puts the recorder on the device object at `device`. A second call for the same device is
/// safe.
pub unsafe fn attach(device: usize) -> Result<(), String> {
    if device == 0 {
        return Err("no device".into());
    }
    let ours = core::ptr::addr_of_mut!(OURS) as *mut usize;
    let orig = core::ptr::addr_of_mut!(ORIG) as *mut usize;
    let vtable = *(device as *const usize);
    if vtable == ours.add(1) as usize {
        return Ok(());
    }
    // The copy starts one word before the table. The compiler keeps a pointer to the type
    // information there.
    core::ptr::copy_nonoverlapping((vtable - 4) as *const usize, orig, TABLE_LEN + 1);
    core::ptr::copy_nonoverlapping(orig, ours, TABLE_LEN + 1);
    static ENTRIES: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    if ENTRIES.load(Ordering::Relaxed) == 0 {
        ENTRIES.store(make_entries(), Ordering::Relaxed);
    }
    let entries = ENTRIES.load(Ordering::Relaxed);
    for (slot, wrapper) in wrapper_table().iter().enumerate() {
        // Without the entry code the wrapper works, and the census has no caller.
        *ours.add(slot + 1) = if entries != 0 { entries + slot * ENTRY_SIZE } else { *wrapper };
    }
    *(device as *mut usize) = ours.add(1) as usize;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_code_jumps_to_the_wrapper() {
        let code = entry_code(0x1000, 0x5000);
        assert_eq!(&code[..4], &[0x8B, 0x04, 0x24, 0xA3]);
        assert_eq!(code[8], 0xE9);
        let relative = u32::from_le_bytes([code[9], code[10], code[11], code[12]]);
        assert_eq!(0x1000 + ENTRY_SIZE as u32 + relative, 0x5000);
    }

    #[test]
    fn argument_table_matches_the_wrappers() {
        // A wrapper with the wrong argument count unbalances the stack of every caller.
        let text = include_str!("device.rs");
        let block = text.split("wrappers! {").nth(1).unwrap().split("\n}").next().unwrap();
        let mut seen = 0;
        for entry in block.split(';') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            let slot: usize = entry.split("=>").next().unwrap().trim().parse().unwrap();
            let inside = entry.split('(').nth(1).unwrap().trim_end_matches(')');
            let argc = inside.split(',').filter(|s| !s.trim().is_empty()).count();
            assert_eq!(argc, ARGS[slot] as usize, "slot {slot} {}", NAMES[slot]);
            assert_eq!(slot, seen);
            seen += 1;
        }
        assert_eq!(seen, SLOTS);
    }

    #[test]
    fn known_vtable_offsets() {
        // The game code uses SetRenderState at offset 0xC8 and DrawIndexedPrimitive at offset 0x11C.
        assert_eq!(NAMES[0xC8 / 4], "SetRenderState");
        assert_eq!(NAMES[0x11C / 4], "DrawIndexedPrimitive");
        assert_eq!(NAMES[SLOT_PRESENT], "Present");
        assert_eq!(NAMES[0xFC / 4], "SetTextureStageState");
        assert_eq!(NAMES[0x94 / 4], "SetTransform");
        assert_eq!(NAMES[96], "DeletePatch");
    }

    #[test]
    fn records_round_trip_and_compare() {
        let mut a = Vec::new();
        append_record(&mut a, 50, &[7, 1], &[]);
        append_record(&mut a, 37, &[256, 0x1234], &[1, 2, 3, 4, 5]);
        assert_eq!(count_records(&a), 2);
        let (first, next) = read_record(&a, 0).unwrap();
        assert_eq!((first.slot, first.args), (50, &[7u32, 1][..]));
        let (second, end) = read_record(&a, next).unwrap();
        assert_eq!(second.payload, &[0x04030201, 5][..]);
        assert_eq!(end, a.len());

        let mut b = Vec::new();
        append_record(&mut b, 50, &[7, 1], &[]);
        append_record(&mut b, 37, &[256, 0x1234], &[1, 2, 3, 4, 6]);
        assert!(first_difference(&a, &a.clone()).is_none());
        let text = first_difference(&a, &b).unwrap();
        assert!(text.starts_with("call 1: rust SetTransform"), "{text}");
        b.truncate(4);
        assert!(first_difference(&a, &b).unwrap().contains("engine has no more calls"));
    }

    #[test]
    fn payload_sizes() {
        assert_eq!(payload(37, &[256, 0x1000]), (0x1000, 64));
        assert_eq!(payload(37, &[256, 0]), (0, 0));
        assert_eq!(payload(36, &[0, 0, 7, 0, 0, 0]), (0, 0));
        assert_eq!(payload(72, &[4, 2, 0x2000, 20]), (0x2000, 120));
        assert_eq!(payload(50, &[7, 1]), (0, 0));
    }

    #[test]
    fn address_arguments_are_real_arguments() {
        for slot in 0..SLOTS {
            let mask = address_args(slot);
            assert!(mask >> ARGS[slot] == 0, "slot {slot} {} masks an argument it does not have", NAMES[slot]);
            if creates(slot) {
                assert_eq!(mask, 1 << (ARGS[slot] - 1), "slot {slot} {}: the result is the last argument", NAMES[slot]);
            }
            // Every argument with a payload is an address of the caller's storage.
            let mut args = [0x1000u32; 8];
            args[0] = 4;
            let (ptr, _) = payload(slot, &args[..ARGS[slot] as usize]);
            if ptr != 0 {
                assert!(mask != 0, "slot {slot} {} has a payload but logs its address", NAMES[slot]);
            }
        }
    }
}
