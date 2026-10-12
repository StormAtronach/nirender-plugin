//! The client of the render link. It tells MGE XE the scene and the object of the device
//! calls that follow. `link.rs` has the contract.
//!
//! The packets come from `before` functions on the ports. They do not depend on the mode of
//! a port: the game code and the Rust code send the same packets.
//!
//! The addresses of the game objects come from
//! `moreFPS/docs/engineering-notes/renderer/scene-phases-and-draw-classification.md`.

use core::ffi::c_void;
use core::mem::size_of;
use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::engine::{vcall0, WORLD_CONTROLLER};
use crate::d3d::{
    D3DRS_ALPHABLENDENABLE, D3DRS_ALPHAFUNC, D3DRS_ALPHAREF, D3DRS_ALPHATESTENABLE, D3DRS_AMBIENT, D3DRS_CULLMODE,
    D3DRS_DESTBLEND, D3DRS_DIFFUSEMATERIALSOURCE, D3DRS_EMISSIVEMATERIALSOURCE, D3DRS_FOGENABLE, D3DRS_LIGHTING,
    D3DRS_SRCBLEND, D3DRS_VERTEXBLEND, D3DRS_ZWRITEENABLE,
};
use crate::gen::layouts::{
    NiDX8LightManager as LM, NiDX8LightManager_LightEntry as LE, NiDX8MaterialManager_Entry as ME, NiDX8RenderState as RS,
    NiAVObject as AV, NiCamera as CAM, NiDX8Renderer as R, NiDynamicEffectState as DES,
    NiDynamicEffectState_InternalList as DL, NiLight as L, NiMaterialProperty as MP, NiPropertyState as PS,
};
use crate::d3d::{D3DTS_TEXTURE0, D3DTS_WORLDMATRIX0};
use crate::link::*;
use crate::mem::rd;
use crate::shadow::BeforeFn;
use crate::{ports, win};

// ---- addresses of the renderer functions

const SET_CAMERA_DATA: usize = 0x6AC620;
const BEGIN_PAINT: usize = 0x6AC520;
const END_PAINT: usize = 0x6AC590;
const CLEAR_BUFFER: usize = 0x6ABE60;
const SET_RENDER_TARGET: usize = 0x6ABD20;
const SWAP_BUFFERS: usize = 0x6ABE40;
const RENDER_SHAPE: usize = 0x6ACEF0;
const RENDER_TRI_STRIPS: usize = 0x6ACFC0;
const RENDER_POINTS: usize = 0x6AD0A0;
const RENDER_LINES: usize = 0x6AD890;
const RENDER_SCREEN_POLY: usize = 0x6ADD20;

// ---- game objects

/// `*(DataHandler**)`
const DATA_HANDLER: usize = 0x7C67E0;
/// `DataHandler::sgSunlight`: the directional light of the sun, and of an interior cell.
const DATA_HANDLER_SUN_LIGHT: usize = 0x98;
/// `*(NiCamera**)`: the camera of the race menu head.
const RACE_MENU_CAMERA: usize = 0x7D5838;

// Offsets in the world controller.
const WC_SPLASH_CAMERA: usize = 0x108;
const WC_WORLD_ROOT: usize = 0x12C;
const WC_WORLD_CAMERA: usize = 0x134;
const WC_ARM_CAMERA: usize = 0x160;
const WC_MENU_CAMERA: usize = 0x18C;
const WC_PREVIEW_CAMERA: usize = 0x1B8;
const WC_MAP_CAMERA: usize = 0x23C;
const WC_SHADOW_CAMERA: usize = 0x2C0;
const WC_SHADOW_MANAGER: usize = 0x2DC;
const WC_WEATHER: usize = 0x58;

// Offsets in the shadow manager.
const SM_SHADOW_VOLUME: usize = 0x60;
const SM_SHADOW_OVERLAY: usize = 0x64;
const SM_SCENE_OBJECTS: usize = 0x68;

// Offsets in the data handler.
const DH_OBJECT_ROOT: usize = 0x8C;
const DH_LANDSCAPE_ROOT: usize = 0x94;
const DH_WATER_CONTROLLER: usize = 0xB4EC;

// Offsets in the water controller.
const WATER_NV_LAKE: usize = 0x0;
const WATER_RIPPLES: usize = 0xB0;
const WATER_PLANE: usize = 0xB4;

// Offsets in the weather controller.
const W_SUN_VIS: usize = 0x0;
const W_SUN_BASE: usize = 0x4;
const W_SUN_GLARE: usize = 0x8;
const W_SECUNDA: usize = 0x44;
const W_MASSER: usize = 0x48;
const W_SKY_ROOT: usize = 0x4C;
const W_NIGHT_SKY: usize = 0x50;
const W_SKY_DOME: usize = 0x54;
const W_CLOUDS: usize = 0x58;
const W_RAIN: usize = 0x5C;
const W_SNOW: usize = 0x60;
const W_STORM: usize = 0x6C;

// Offsets in a moon.
const MOON_ROOT: usize = 0x0;
const MOON_NODE: usize = 0x4;
const MOON_SHADOW_NODE: usize = 0x8;
const MOON_SHADOW_SHAPE: usize = 0x10;

/// The offset of the translation of `worldTransform` in a `NiAVObject`. `NiCamera::Click`
/// passes the address of this member of the camera to `SetCameraData`.
const AV_WORLD_TRANSLATION: usize = 0x64;
/// The offset of `worldTransform` in a `NiAVObject`. `NiTriShape::Display` passes the address
/// of this member to `RenderShape`.
const AV_WORLD_TRANSFORM: usize = 0x40;
/// `NiLight::GetType` in the vtable.
const LIGHT_VTBL_GET_TYPE: usize = 0x94;
/// The game does not use a light with a dimmer below this value (0x74F2EC).
const MIN_DIMMER: f32 = 0.01;
/// Bit 0 of the flags of a `NiAVObject`: the application culled the object.
const FLAG_APP_CULLED: u16 = 1;
/// The longest parent chain that the walk follows.
const MAX_DEPTH: usize = 64;
const MAX_LIGHTS: usize = 64;

/// The objects that the tests compare with. `begin_paint` reads them one time for a scene.
#[derive(Clone, Copy)]
struct Roots {
    scene_kind: u32,
    render_target: u32,
    sky_root: usize,
    sky_dome: usize,
    night_sky: usize,
    clouds: usize,
    sun_vis: usize,
    sun_base: usize,
    sun_glare: usize,
    moon_roots: [usize; 4],
    moon_shadows: [usize; 4],
    particles: [usize; 3],
    water_ripples: usize,
    water_plane: usize,
    landscape_root: usize,
    object_root: usize,
    shadow_volume: usize,
    shadow_overlay: usize,
}

const NO_ROOTS: Roots = Roots {
    scene_kind: SCENE_OTHER,
    render_target: 0,
    sky_root: 0,
    sky_dome: 0,
    night_sky: 0,
    clouds: 0,
    sun_vis: 0,
    sun_base: 0,
    sun_glare: 0,
    moon_roots: [0; 4],
    moon_shadows: [0; 4],
    particles: [0; 3],
    water_ripples: 0,
    water_plane: 0,
    landscape_root: 0,
    object_root: 0,
    shadow_volume: 0,
    shadow_overlay: 0,
};

/// The table of the host. 0 until `connect` has a host.
static HOST: AtomicUsize = AtomicUsize::new(0);
/// The camera of the scene packet that `set_camera_data` sent, until `begin_paint` sees it.
static SCENE_SENT_FOR: AtomicUsize = AtomicUsize::new(0);
// Only the render thread reads and writes these two.
static mut ROOTS: Roots = NO_ROOTS;
/// The number of draw packets of each class since the last read of the counters.
static mut CLASS_COUNTS: [u32; CLASS_COUNT] = [0; CLASS_COUNT];
const CLASS_COUNT: usize = 18;
/// The lights of the state packet in progress. Only the render thread uses it.
static mut STATE_LIGHTS: [StateLightV1; MAX_STATE_LIGHTS] = [StateLightV1 { device_index: 0, d3d_light: [0; 26] }; MAX_STATE_LIGHTS];
const MAX_STATE_LIGHTS: usize = 64;
/// The world matrices, the stage textures and the texture transforms that the Rust code sent
/// to the device last. The renderer of the game keeps no record of them. Only the render
/// thread uses them.
static mut WORLDS: [[f32; 16]; 4] = [[0.0; 16]; 4];
/// The transforms that the copies have: bit n is world matrix n, bit 8 + n is the texture
/// transform of stage n.
static mut TRANSFORMS_KNOWN: u32 = 0;
static mut TEXTURES: [u32; STATE_STAGES as usize] = [0; STATE_STAGES as usize];
static mut TEXTURE_TRANSFORMS: [[f32; 16]; STATE_STAGES as usize] = [[0.0; 16]; STATE_STAGES as usize];

/// Keeps a world matrix or a texture transform that goes to the device.
#[inline]
pub unsafe fn note_transform(state: u32, matrix: usize) {
    let to = if (D3DTS_WORLDMATRIX0..D3DTS_WORLDMATRIX0 + 4).contains(&state) {
        *addr_of_mut!(TRANSFORMS_KNOWN) |= 1 << (state - D3DTS_WORLDMATRIX0);
        // The host knows the space of this matrix only now, during the call of the game.
        let bit = 1 << (state - D3DTS_WORLDMATRIX0);
        let relative = crate::camera_space::pending()
            || host().is_some_and(|host| host.capabilities & CAP_WORLD_SPACE != 0 && (host.world_space)() != 0);
        let mask = &mut *addr_of_mut!(WORLD_RELATIVE);
        *mask = if relative { *mask | bit } else { *mask & !bit };
        addr_of_mut!(WORLDS[(state - D3DTS_WORLDMATRIX0) as usize])
    } else if (D3DTS_TEXTURE0..D3DTS_TEXTURE0 + STATE_STAGES).contains(&state) {
        *addr_of_mut!(TRANSFORMS_KNOWN) |= 0x100 << (state - D3DTS_TEXTURE0);
        addr_of_mut!(TEXTURE_TRANSFORMS[(state - D3DTS_TEXTURE0) as usize])
    } else {
        return;
    };
    core::ptr::copy_nonoverlapping(matrix as *const f32, to as *mut f32, 16);
}

/// The space of the world matrices in `WORLDS`: bit n is 1 when matrix n is relative to the
/// camera.
static mut WORLD_RELATIVE: u32 = 0;

/// True when the copy of world matrix `index` is relative to the camera.
pub unsafe fn world_relative(index: u32) -> bool {
    *addr_of!(WORLD_RELATIVE) >> index & 1 != 0
}

/// The copy of world matrix `index`, as bit patterns.
pub unsafe fn world_matrix(index: u32) -> [u32; 16] {
    (*addr_of!(WORLDS))[index as usize].map(f32::to_bits)
}

/// The facts that the host takes from the packets: the answer of the last `use_facts`.
static USED_FACTS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// True when the host leaves the camera-relative world matrices to this plugin.
pub fn client_camera_space() -> bool {
    USED_FACTS.load(Ordering::Relaxed) >> FACT_CLIENT_CAMERA_SPACE & 1 != 0
}

/// Asks the host if the scene in progress is in camera space. On true, `origin` has the
/// origin that the host uses.
pub unsafe fn host_camera_space(origin: &mut [f64; 3]) -> bool {
    match host() {
        Some(host) if host.capabilities & CAP_CAMERA_SPACE != 0 => (host.camera_space)(origin.as_mut_ptr()) != 0,
        _ => false,
    }
}

/// The stages whose texture is in `TEXTURES`: bit n is stage n.
static mut TEXTURES_KNOWN: u32 = 0;
/// The buffers that the Rust code sent to the device last: vertex buffer, stride, vertex
/// format, index buffer, base vertex index.
static mut BUFFERS: [u32; 5] = [0; 5];
/// The renderer of the last state packet.
static RENDERER: AtomicUsize = AtomicUsize::new(0);
/// The device kinds that the host takes from the packets: bits of `FACT_DEVICE_*`. The Rust
/// code makes no D3D8 call for them.
static SUPPRESS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
const DEVICE_FACTS: u32 =
    1 << FACT_DEVICE_RENDER_STATES | 1 << FACT_DEVICE_STAGE_STATES | 1 << FACT_DEVICE_TEXTURES | 1 << FACT_DEVICE_BUFFERS
        | 1 << FACT_DEVICE_TRANSFORMS | 1 << FACT_DEVICE_LIGHTS;
/// The render states that a state packet has, as `D3DRS_*`: ZENABLE, ZWRITEENABLE,
/// ALPHATESTENABLE, SRCBLEND, DESTBLEND, CULLMODE, ZFUNC, ALPHAREF, ALPHAFUNC,
/// ALPHABLENDENABLE, FOGENABLE, the eight stencil states, LIGHTING, AMBIENT,
/// DIFFUSEMATERIALSOURCE, EMISSIVEMATERIALSOURCE, VERTEXBLEND.
const PACKET_RENDER_STATES: [u32; 24] = [7, 14, 15, 19, 20, 22, 23, 24, 25, 27, 28, 52, 53, 54, 55, 56, 57, 58, 59, 137, 139, 145, 148, 151];
/// The sampler states of a state packet, in the order of the contract: `D3DTSS_*`.
const SAMPLER_STATES: [u32; STATE_SAMPLER_STATES as usize] = [13, 14, 16, 17, 18];

/// The D3D8 calls that the Rust code did not make, for each device kind: render states,
/// stage states, textures, buffers, transforms, lights.
static mut SKIPPED: [u32; 6] = [0; 6];

/// True when the host has this device kind. It counts the call that the caller then skips.
#[inline]
fn skip(fact: u32, in_packet: bool) -> bool {
    let skipped = in_packet && SUPPRESS.load(Ordering::Relaxed) >> fact & 1 != 0;
    if skipped {
        unsafe { (*addr_of_mut!(SKIPPED))[(fact - FACT_DEVICE_RENDER_STATES) as usize] += 1 };
    }
    skipped
}

/// `DataHandler::currentInteriorCell`: the interior cell that the player is in, or 0.
const CURRENT_INTERIOR_CELL: usize = 0xAC;

/// True when the player is in an interior cell.
unsafe fn in_interior() -> bool {
    let data_handler: usize = rd(DATA_HANDLER, 0);
    data_handler != 0 && rd::<usize>(data_handler, CURRENT_INTERIOR_CELL) != 0
}

/// The two functions of the `d3d8.dll` of MGE XE for a game without a Direct3D device
/// (`MGE_StreamPresent`, `MGE_StreamTransform`), or 0. MGE XE runs its logic of a frame that
/// is not drawing in them: zoom, field of view, screen shake, crosshair. Its proxy device
/// ran that logic at `Present` and at `SetTransform`.
static STREAM_PRESENT: AtomicUsize = AtomicUsize::new(0);
static STREAM_TRANSFORM: AtomicUsize = AtomicUsize::new(0);

/// Finds the functions of MGE XE. The `d3d8.dll` of this repository has none.
unsafe fn find_stream_functions() {
    let module = host_module();
    if module == 0 {
        return;
    }
    let present = win::GetProcAddress(module as *mut c_void, b"MGE_StreamPresent\0".as_ptr());
    let transform = win::GetProcAddress(module as *mut c_void, b"MGE_StreamTransform\0".as_ptr());
    STREAM_PRESENT.store(present as usize, Ordering::Relaxed);
    STREAM_TRANSFORM.store(transform as usize, Ordering::Relaxed);
    if !present.is_null() {
        crate::log!("link: the frame logic of MGE XE runs from the stream");
    }
}

/// The game showed a frame: MGE XE runs its logic of a frame.
pub unsafe fn stream_present() {
    let function = STREAM_PRESENT.load(Ordering::Relaxed);
    if function != 0 && crate::shadow::on_main_thread() {
        let (width, height) = crate::own_device::size();
        core::mem::transmute::<usize, unsafe extern "C" fn(u32, u32)>(function)(width, height);
    }
}

/// Sends a state command to the host. Returns false when the host takes no commands.
#[inline]
unsafe fn command(command: u32, a: u32, b: u32, c: u32) -> bool {
    match host() {
        Some(host) if host.capabilities & CAP_COMMANDS != 0 => {
            // MGE XE can change the view and the projection of the game: its camera effects
            // and its zoom. The change is for the stream only; the matrix of the game stays.
            let function = STREAM_TRANSFORM.load(Ordering::Relaxed);
            let changed = (function != 0 && command == COMMAND_TRANSFORM && (a == 2 || a == 3) && b != 0).then(|| {
                let mut matrix = *(b as usize as *const [f32; 16]);
                let back_buffer = crate::own_host::target_is_back_buffer() as u32;
                core::mem::transmute::<usize, unsafe extern "C" fn(u32, *mut f32, u32)>(function)(a, matrix.as_mut_ptr(), back_buffer);
                matrix
            });
            let b = changed.as_ref().map_or(b, |matrix| matrix.as_ptr() as u32);
            (host.command)(command, a, b, c);
            crate::capture::command(command, a, b, c);
            true
        }
        _ => false,
    }
}

/// The calls of the Rust code on D3D8 objects that are not the device: reference counts,
/// buffers, textures, surfaces. The index is the place in `OBJECT_CALL_NAMES`.
static mut OBJECT_CALLS: [u32; 10] = [0; 10];
const OBJECT_CALL_NAMES: [&str; 10] = [
    "AddRef",
    "Release",
    "buffer.Lock",
    "buffer.Unlock",
    "buffer.GetDesc",
    "texture.GetSurfaceLevel",
    "cube.GetCubeMapSurface",
    "surface.GetDesc",
    "surface.LockRect",
    "surface.UnlockRect",
];

/// One call on a resource. `index` is the place in `OBJECT_CALL_NAMES`, and the call of the
/// link is `CALL_ADD_REF + index`. Returns the result when the host made the call, and
/// `None` when the caller must make the D3D8 call. Only the D3D8 calls are counted.
#[inline]
pub unsafe fn object_call(index: usize, args: &[u32]) -> Option<u32> {
    let result = kind_call(KIND_OBJECTS, CALL_ADD_REF + index as u32, args);
    if result.is_none() {
        (*addr_of_mut!(OBJECT_CALLS))[index] += 1;
    }
    result
}

/// The kinds `KIND_*` that the host has: the answer of the last `use_kinds`.
static KINDS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
/// The frame and scene calls that went to the host and not to the device.
static mut FRAME_CALLS: u32 = 0;

/// Tells the host which kinds `KIND_*` it has. Returns the mask that it uses, or `None`
/// when the host has no such kinds.
pub unsafe fn use_kinds(mask: u32) -> Option<u32> {
    let host = host()?;
    if host.capabilities & CAP_CALLS == 0 {
        return None;
    }
    let used = (host.use_kinds)(mask);
    KINDS.store(used, Ordering::Relaxed);
    Some(used)
}

/// Makes a frame or scene call on the host. Returns `None` when the caller must make the
/// D3D8 call.
#[inline]
pub unsafe fn frame_call(call: u32, args: &[u32]) -> Option<u32> {
    kind_call(KIND_FRAME, call, args)
}

/// The pixels of a surface of the host with 4 bytes for each pixel: width, height, format and
/// the rows without the bytes after each row. The capture uses it; the call is not in the
/// stream.
pub unsafe fn surface_pixels(surface: u32) -> Option<(u32, u32, u32, Vec<u8>)> {
    let host = host()?;
    // D3DSURFACE_DESC: format, type, usage, pool, size, multisample type, width, height.
    let mut description = [0u32; 8];
    if (host.call)(CALL_SURFACE_DESCRIPTION, [surface, description.as_mut_ptr() as u32].as_ptr()) as i32 != 0 {
        return None;
    }
    let (format, width, height) = (description[0], description[6], description[7]);
    // D3DFMT_A8R8G8B8 is 21 and D3DFMT_X8R8G8B8 is 22.
    if !(format == 21 || format == 22) || width == 0 || height == 0 {
        return None;
    }
    // D3DLOCKED_RECT: pitch, bits. D3DLOCK_READONLY is 0x10.
    let mut locked = [0u32; 2];
    if (host.call)(CALL_SURFACE_LOCK, [surface, locked.as_mut_ptr() as u32, 0, 0x10].as_ptr()) as i32 != 0 {
        return None;
    }
    let row = width as usize * 4;
    let mut pixels = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        pixels.extend_from_slice(core::slice::from_raw_parts((locked[1] as usize + y * locked[0] as usize) as *const u8, row));
    }
    (host.call)(CALL_SURFACE_UNLOCK, [surface].as_ptr());
    Some((width, height, format, pixels))
}

/// Makes a call of the target kind on the host. Returns `None` when the caller must make
/// the D3D8 call.
#[inline]
pub unsafe fn target_call(call: u32, args: &[u32]) -> Option<u32> {
    kind_call(KIND_TARGETS, call, args)
}

/// Makes a buffer on the host. Returns `None` when the caller must make the D3D8 call.
#[inline]
pub unsafe fn buffer_call(call: u32, args: &[u32]) -> Option<u32> {
    kind_call(KIND_BUFFERS, call, args)
}

#[inline]
unsafe fn kind_call(kind: u32, call: u32, args: &[u32]) -> Option<u32> {
    if KINDS.load(Ordering::Relaxed) & kind == 0 || crate::shadow::leg() == crate::shadow::Leg::Rust {
        return None;
    }
    let host = host()?;
    if kind == KIND_FRAME {
        *addr_of_mut!(FRAME_CALLS) += 1;
        if call == CALL_PRESENT {
            // This Present does not go through the device, where the frame timer is.
            crate::perf::on_present();
        }
    }
    let result = (host.call)(call, args.as_ptr());
    crate::capture::call(call, args, result);
    Some(result)
}

/// True when the host has this device kind and takes commands.
#[inline]
unsafe fn commands_for(fact: u32) -> bool {
    SUPPRESS.load(Ordering::Relaxed) >> fact & 1 != 0 && host().is_some_and(|host| host.capabilities & CAP_COMMANDS != 0)
}

/// Sends a draw with the index buffer to the host. Returns false when the caller must make
/// the D3D8 call.
#[inline]
pub unsafe fn draw_indexed(primitive_type: u32, min_index: u32, vertices: u32, start_index: u32, primitives: u32) -> bool {
    if !commands_for(FACT_DEVICE_BUFFERS) {
        return false;
    }
    let words = [primitive_type, min_index, vertices, start_index, primitives];
    skip(FACT_DEVICE_BUFFERS, true);
    command(COMMAND_DRAW_INDEXED, words.as_ptr() as u32, 0, 0)
}

/// Sends a draw without an index buffer to the host. Returns false when the caller must
/// make the D3D8 call.
#[inline]
pub unsafe fn draw(primitive_type: u32, start_vertex: u32, primitives: u32) -> bool {
    if !commands_for(FACT_DEVICE_BUFFERS) {
        return false;
    }
    let words = [primitive_type, start_vertex, primitives];
    skip(FACT_DEVICE_BUFFERS, true);
    command(COMMAND_DRAW, words.as_ptr() as u32, 0, 0)
}

// The draw functions of the Morrowind Code Patch get the device and its vtable in registers.
// They call the device through that vtable: the draw, and for a draw with alpha also render
// states and stage states. The four functions below are the entries of a vtable that the
// plugin gives them when the host draws. Each one sends a command, and makes the D3D8 call
// only when the host does not take that command.

unsafe extern "system" fn vtable_draw_indexed(device: usize, primitive_type: u32, min_index: u32, vertices: u32, start_index: u32, primitives: u32) -> u32 {
    if draw_indexed(primitive_type, min_index, vertices, start_index, primitives) {
        return 0;
    }
    let f: unsafe extern "system" fn(usize, u32, u32, u32, u32, u32) -> u32 =
        core::mem::transmute(crate::engine::virtual_at(device, crate::d3d::DEVICE_DRAW_INDEXED_PRIMITIVE));
    f(device, primitive_type, min_index, vertices, start_index, primitives)
}

unsafe extern "system" fn vtable_draw(device: usize, primitive_type: u32, start_vertex: u32, primitives: u32) -> u32 {
    if draw(primitive_type, start_vertex, primitives) {
        return 0;
    }
    let f: unsafe extern "system" fn(usize, u32, u32, u32) -> u32 =
        core::mem::transmute(crate::engine::virtual_at(device, crate::d3d::DEVICE_DRAW_PRIMITIVE));
    f(device, primitive_type, start_vertex, primitives)
}

/// A render state of the Code Patch is for the draw that follows at once, so it goes to the
/// host now and not with the next state packet.
unsafe extern "system" fn vtable_set_render_state(device: usize, state: u32, value: u32) -> u32 {
    if commands_for(FACT_DEVICE_RENDER_STATES) && command(COMMAND_RENDER_STATE, state, value, 0) {
        return 0;
    }
    let f: unsafe extern "system" fn(usize, u32, u32) -> u32 =
        core::mem::transmute(crate::engine::virtual_at(device, crate::d3d::DEVICE_SET_RENDER_STATE));
    f(device, state, value)
}

unsafe extern "system" fn vtable_set_stage_state(device: usize, stage: u32, state: u32, value: u32) -> u32 {
    if commands_for(FACT_DEVICE_STAGE_STATES) && command(COMMAND_STAGE_STATE, stage, state, value) {
        return 0;
    }
    let f: unsafe extern "system" fn(usize, u32, u32, u32) -> u32 =
        core::mem::transmute(crate::engine::virtual_at(device, crate::d3d::DEVICE_SET_TEXTURE_STAGE_STATE));
    f(device, stage, state, value)
}

/// The number of methods of `IDirect3DDevice8`.
const DEVICE_METHODS: usize = 97;
static mut DRAW_VTABLE: [usize; DEVICE_METHODS] = [0; DEVICE_METHODS];

/// The vtable to give to a draw function of the Code Patch: `vtable`, the one of the device,
/// or a copy with the four entries above when the host draws.
pub unsafe fn draw_vtable(vtable: usize) -> usize {
    if !commands_for(FACT_DEVICE_BUFFERS) {
        return vtable;
    }
    let copy = &mut *addr_of_mut!(DRAW_VTABLE);
    core::ptr::copy_nonoverlapping(vtable as *const usize, copy.as_mut_ptr(), DEVICE_METHODS);
    copy[crate::d3d::DEVICE_DRAW_INDEXED_PRIMITIVE / 4] = vtable_draw_indexed as *const () as usize;
    copy[crate::d3d::DEVICE_DRAW_PRIMITIVE / 4] = vtable_draw as *const () as usize;
    copy[crate::d3d::DEVICE_SET_RENDER_STATE / 4] = vtable_set_render_state as *const () as usize;
    copy[crate::d3d::DEVICE_SET_TEXTURE_STAGE_STATE / 4] = vtable_set_stage_state as *const () as usize;
    copy.as_ptr() as usize
}

/// A device call of code that is not a port of this plugin: game code outside the renderer,
/// a game function that another mod changed, a patch. The hook of the device gives each
/// call here first. `slot` is the number of the method and `args` are its arguments without
/// the device. Returns the result when the call went to the host, and `None` when it must
/// go to the device. A state call goes to the host at once, as a command: such code has no
/// state packet.
pub unsafe fn intercept(slot: usize, args: &[u32]) -> Option<u32> {
    use crate::d3d::*;
    if KINDS.load(Ordering::Relaxed) == 0 && SUPPRESS.load(Ordering::Relaxed) == 0 {
        return None;
    }
    /// `IDirect3DDevice8::CreateDepthStencilSurface`
    const DEVICE_CREATE_DEPTH_STENCIL_SURFACE: usize = 0x68;
    let sent = |done: bool| if done { Some(0) } else { None };
    let routed = match slot * 4 {
        DEVICE_TEST_COOPERATIVE_LEVEL => frame_call(CALL_TEST_DEVICE, args),
        DEVICE_PRESENT => frame_call(CALL_PRESENT, args),
        DEVICE_BEGIN_SCENE => frame_call(CALL_BEGIN_SCENE, args),
        DEVICE_END_SCENE => frame_call(CALL_END_SCENE, args),
        DEVICE_CLEAR => frame_call(CALL_CLEAR, args),
        DEVICE_SET_VIEWPORT => frame_call(CALL_VIEWPORT, args),
        DEVICE_SET_RENDER_TARGET => frame_call(CALL_RENDER_TARGET, args),
        DEVICE_CREATE_VERTEX_BUFFER => buffer_call(CALL_CREATE_VERTEX_BUFFER, args),
        DEVICE_CREATE_INDEX_BUFFER => buffer_call(CALL_CREATE_INDEX_BUFFER, args),
        DEVICE_CREATE_TEXTURE => target_call(CALL_CREATE_TEXTURE, args),
        DEVICE_CREATE_DEPTH_STENCIL_SURFACE => target_call(CALL_CREATE_DEPTH_BUFFER, args),
        DEVICE_CREATE_IMAGE_SURFACE => target_call(CALL_CREATE_IMAGE_SURFACE, args),
        DEVICE_COPY_RECTS => target_call(CALL_COPY_PIXELS, args),
        DEVICE_GET_RENDER_TARGET => target_call(CALL_GET_RENDER_TARGET, args),
        DEVICE_GET_DEPTH_STENCIL_SURFACE => target_call(CALL_GET_DEPTH_BUFFER, args),
        DEVICE_RESOURCE_MANAGER_DISCARD_BYTES => target_call(CALL_DISCARD_BYTES, args),
        DEVICE_SET_RENDER_STATE if commands_for(FACT_DEVICE_RENDER_STATES) => sent(command(COMMAND_RENDER_STATE, args[0], args[1], 0)),
        DEVICE_SET_TEXTURE_STAGE_STATE if commands_for(FACT_DEVICE_STAGE_STATES) => sent(command(COMMAND_STAGE_STATE, args[0], args[1], args[2])),
        DEVICE_SET_TRANSFORM if commands_for(FACT_DEVICE_TRANSFORMS) => sent(command(COMMAND_TRANSFORM, args[0], args[1], 0)),
        DEVICE_DRAW_INDEXED_PRIMITIVE if commands_for(FACT_DEVICE_BUFFERS) => sent(command(COMMAND_DRAW_INDEXED, args.as_ptr() as u32, 0, 0)),
        DEVICE_DRAW_PRIMITIVE if commands_for(FACT_DEVICE_BUFFERS) => sent(command(COMMAND_DRAW, args.as_ptr() as u32, 0, 0)),
        _ => None,
    };
    /// `IDirect3DDevice8::GetBackBuffer`
    const DEVICE_GET_BACK_BUFFER: usize = 0x40;
    // `GetBackBuffer(index, type, address)`: with the own host, the back buffer is the
    // surface that the host gives as the first render target.
    let routed = if routed.is_none() && crate::own_host::active() && slot * 4 == DEVICE_GET_BACK_BUFFER && args.len() >= 3 {
        target_call(CALL_GET_RENDER_TARGET, &args[2..3])
    } else {
        routed
    };
    if routed.is_none() && crate::own_host::active() {
        // `GetRenderState(state, address)`: the Code Patch asks for the blend state before a
        // draw into a rendered texture. The answer is the value in the record of the game.
        if slot * 4 == DEVICE_GET_RENDER_STATE && args.len() >= 2 && args[1] != 0 {
            let renderer = crate::engine::renderer();
            if renderer != 0 && args[0] < RENDER_STATES_IN_RECORD {
                let render_state: usize = rd(renderer, R::renderState);
                let value: u32 = rd(render_state, RS::renderStateSettings + args[0] as usize * 8);
                *(args[1] as usize as *mut u32) = if value == STATE_UNKNOWN { 0 } else { value };
                return Some(0);
            }
        }
        // `GetTextureStageState(stage, state, address)`: the Code Patch keeps the alpha
        // operation of stage 0 and puts it back after its draws.
        const DEVICE_GET_TEXTURE_STAGE_STATE: usize = 0xF8;
        if slot * 4 == DEVICE_GET_TEXTURE_STAGE_STATE && args.len() >= 3 && args[2] != 0 && args[0] < 8 && args[1] < 32 {
            let renderer = crate::engine::renderer();
            if renderer != 0 {
                let render_state: usize = rd(renderer, R::renderState);
                let value: u32 = rd(render_state, RS::maybe_textureStageStateSettings + (args[1] + args[0] * 32) as usize * 8);
                *(args[2] as usize as *mut u32) = if value == STATE_UNKNOWN { 0 } else { value };
                return Some(0);
            }
        }
        // The device of the plugin answers what it can.
        if crate::own_device::present() {
            if let Some(result) = crate::own_device::answer(slot, args) {
                return Some(result);
            }
        }
        // No device is behind the link. The call ends here.
        crate::own_host::take(slot);
        return Some(0);
    }
    routed
}

/// True when the caller must not make the D3D8 call for this render state: the host sets it
/// from the packets, or the host got it as a command here.
#[inline]
pub unsafe fn skip_render_state(state: u32, value: u32) -> bool {
    skip(FACT_DEVICE_RENDER_STATES, PACKET_RENDER_STATES.contains(&state))
        || (skip(FACT_DEVICE_RENDER_STATES, true) && command(COMMAND_RENDER_STATE, state, value, 0))
}

/// True when the host sets the material from the packets. The material is a part of the
/// render state kind.
#[inline]
pub fn skip_material() -> bool {
    skip(FACT_DEVICE_RENDER_STATES, true)
}

/// True when the caller must not make the D3D8 call for this texture stage state: the host
/// sets it from the packets, or the host got it as a command here.
#[inline]
pub unsafe fn skip_stage_state(stage: u32, state: u32, value: u32) -> bool {
    skip(FACT_DEVICE_STAGE_STATES, STAGE_STATES.contains(&state) || SAMPLER_STATES.contains(&state))
        || (skip(FACT_DEVICE_STAGE_STATES, true) && command(COMMAND_STAGE_STATE, stage, state, value))
}

/// True when the host sets the texture of this stage from the packets.
#[inline]
pub fn skip_texture(stage: u32) -> bool {
    skip(FACT_DEVICE_TEXTURES, stage < STATE_STAGES)
}

/// True when the caller must not make the D3D8 call for this transform. The host sets a
/// world matrix of the vertex blend and a texture transform from the packets. It gets the
/// view and the projection as a command here.
#[inline]
pub unsafe fn skip_transform(state: u32, matrix: usize) -> bool {
    const D3DTS_VIEW: u32 = 2;
    const D3DTS_PROJECTION: u32 = 3;
    skip(
        FACT_DEVICE_TRANSFORMS,
        (D3DTS_WORLDMATRIX0..D3DTS_WORLDMATRIX0 + 4).contains(&state) || (D3DTS_TEXTURE0..D3DTS_TEXTURE0 + STATE_STAGES).contains(&state),
    ) || ((state == D3DTS_VIEW || state == D3DTS_PROJECTION)
        && skip(FACT_DEVICE_TRANSFORMS, true)
        && command(COMMAND_TRANSFORM, state, matrix as u32, 0))
}

/// True when the host sets the lights from the packets.
#[inline]
pub fn skip_lights() -> bool {
    skip(FACT_DEVICE_LIGHTS, true)
}

/// True when the host sets the buffers and the vertex format from the packets.
#[inline]
pub fn skip_buffers() -> bool {
    skip(FACT_DEVICE_BUFFERS, true)
}

#[inline]
pub unsafe fn note_stream_source(buffer: usize, stride: u32) {
    let buffers = &mut *addr_of_mut!(BUFFERS);
    buffers[0] = buffer as u32;
    buffers[1] = stride;
}

#[inline]
pub unsafe fn note_vertex_shader(handle: u32) {
    (*addr_of_mut!(BUFFERS))[2] = handle;
}

#[inline]
pub unsafe fn note_indices(buffer: usize, base_vertex: u32) {
    let buffers = &mut *addr_of_mut!(BUFFERS);
    buffers[3] = buffer as u32;
    buffers[4] = base_vertex;
}

/// Sends to the device the state that the game believes the device has, for the kinds in
/// `kinds`. The Rust code made no call for them while the host had them.
/// The record of the game has 258 render states.
const RENDER_STATES_IN_RECORD: u32 = 256;

/// Each render state that the game set, with its value, from the record of the game. A
/// stream starts with them: the game sends a state only when its value changes.
pub unsafe fn render_states() -> Vec<(u32, u32)> {
    let renderer = crate::engine::renderer();
    if renderer == 0 {
        return Vec::new();
    }
    let render_state: usize = rd(renderer, R::renderState);
    (0..RENDER_STATES_IN_RECORD)
        .map(|state| (state, rd::<u32>(render_state, RS::renderStateSettings + state as usize * 8)))
        .filter(|(_, value)| *value != STATE_UNKNOWN)
        .collect()
}

unsafe fn resync(kinds: u32) {
    let renderer = RENDERER.load(Ordering::Relaxed);
    if renderer == 0 {
        return;
    }
    let device: usize = rd(renderer, R::d3dDevice);
    let render_state: usize = rd(renderer, R::renderState);
    if kinds >> FACT_DEVICE_RENDER_STATES & 1 != 0 {
        for state in PACKET_RENDER_STATES {
            let value: u32 = rd(render_state, RS::renderStateSettings + state as usize * 8);
            if value != STATE_UNKNOWN {
                crate::d3d::device_set_render_state(device, state, value);
            }
        }
    }
    if kinds >> FACT_DEVICE_STAGE_STATES & 1 != 0 {
        for stage in 0..STATE_STAGES {
            for state in STAGE_STATES.into_iter().chain(SAMPLER_STATES) {
                let value: u32 = rd(render_state, RS::maybe_textureStageStateSettings + (state + stage * 32) as usize * 8);
                if value != STATE_UNKNOWN {
                    crate::d3d::device_set_texture_stage_state(device, stage, state, value);
                }
            }
        }
    }
    if kinds >> FACT_DEVICE_TEXTURES & 1 != 0 {
        for stage in 0..STATE_STAGES {
            if *addr_of!(TEXTURES_KNOWN) >> stage & 1 != 0 {
                crate::d3d::device_set_texture(device, stage, (*addr_of!(TEXTURES))[stage as usize] as usize);
            }
        }
    }
    if kinds >> FACT_DEVICE_TRANSFORMS & 1 != 0 {
        // Not the world matrices: the game sends them again for each draw.
        let known = *addr_of!(TRANSFORMS_KNOWN);
        for stage in 0..STATE_STAGES {
            if known >> (8 + stage) & 1 != 0 {
                crate::d3d::device_set_transform(device, D3DTS_TEXTURE0 + stage, addr_of!(TEXTURE_TRANSFORMS[stage as usize]) as usize);
            }
        }
    }
    if kinds >> FACT_DEVICE_LIGHTS & 1 != 0 {
        // The host put all lights off. The lights of the game go on again, the oldest first:
        // the list of the game has the newest first.
        let manager: usize = rd(renderer, R::lightManager);
        let mut entries = [0usize; MAX_STATE_LIGHTS];
        let mut count = 0;
        let mut item: usize = rd(manager, LM::deviceLights_head);
        while item != 0 && count < MAX_STATE_LIGHTS {
            let light: usize = rd(item, ITEM_DATA);
            item = rd(item, ITEM_NEXT);
            let entry = if light != 0 { crate::ports::hash_map::find_value(manager + LM::lightEntries, rd(light, L::index)) } else { 0 };
            if entry != 0 {
                entries[count] = entry;
                count += 1;
            }
        }
        for &entry in entries[..count].iter().rev() {
            let index: u32 = rd(entry, LE::lightIndex);
            crate::d3d::device_set_light(device, index, entry + LE::d3dLight);
            crate::d3d::device_light_enable(device, index, 1);
        }
    }
    if kinds >> FACT_DEVICE_BUFFERS & 1 != 0 {
        let buffers = *addr_of!(BUFFERS);
        if buffers[0] != 0 {
            crate::d3d::device_set_stream_source(device, 0, buffers[0] as usize, buffers[1]);
            crate::d3d::device_set_vertex_shader(device, rd(render_state, RS::currentVertexShader));
        }
        if buffers[3] != 0 {
            crate::d3d::device_set_indices(device, buffers[3] as usize, buffers[4]);
        }
    }
}

/// The number of `SetRenderState` and `SetTextureStageState` calls of the Rust code, for
/// each state. It shows which states the game changes during a frame.
static mut RENDER_STATE_SETS: [u32; 256] = [0; 256];
static mut STAGE_STATE_SETS: [u32; 32] = [0; 32];

#[inline]
pub unsafe fn note_render_state(state: u32) {
    if state < 256 {
        *addr_of_mut!(RENDER_STATE_SETS[state as usize]) += 1;
    }
}

#[inline]
pub unsafe fn note_stage_state(state: u32) {
    if state < 32 {
        *addr_of_mut!(STAGE_STATE_SETS[state as usize]) += 1;
    }
}

/// Keeps the texture of a stage that goes to the device.
#[inline]
pub unsafe fn note_texture(stage: u32, texture: usize) {
    if stage < STATE_STAGES {
        *addr_of_mut!(TEXTURES[stage as usize]) = texture as u32;
        *addr_of_mut!(TEXTURES_KNOWN) |= 1 << stage;
    }
}
/// The texture stage states of a state packet, in the order of the contract: `D3DTSS_*`.
const STAGE_STATES: [u32; STATE_STAGE_STATES as usize] = [1, 2, 3, 4, 5, 6, 26, 27, 28, 11, 24, 7, 8, 9, 10, 22, 23];
/// A list item of the game: data, next, previous.
const ITEM_DATA: usize = 0x0;
const ITEM_NEXT: usize = 0x4;
static mut LIGHTS: [LightV1; MAX_LIGHTS] = [LightV1 { light: 0, device_index: 0, light_type: 0, radius: 0.0 }; MAX_LIGHTS];

/// Makes `table` the host of the link. `own_host` uses this: the plugin answers the calls.
pub unsafe fn set_host(table: &'static HostV1) {
    HOST.store(table as *const HostV1 as usize, Ordering::Relaxed);
}

unsafe fn host() -> Option<&'static HostV1> {
    (HOST.load(Ordering::Relaxed) as *const HostV1).as_ref()
}

/// Reads a pointer member. A null `base` gives 0.
unsafe fn ptr_at(base: usize, offset: usize) -> usize {
    if base == 0 {
        0
    } else {
        rd(base, offset)
    }
}

unsafe fn app_culled(object: usize) -> bool {
    object != 0 && rd::<u16>(object, AV::flags) & FLAG_APP_CULLED != 0
}

/// The reflection camera: water controller, NV lake, the render-to-texture object, its camera.
unsafe fn reflection_camera(water_controller: usize) -> usize {
    let lake = ptr_at(water_controller, WATER_NV_LAKE);
    let sky_render = ptr_at(lake, 0x4);
    ptr_at(sky_render, 0x4)
}

unsafe fn scene_kind(camera: usize, world_controller: usize, water_controller: usize) -> u32 {
    if camera == 0 || world_controller == 0 {
        return SCENE_OTHER;
    }
    let is = |offset: usize| camera == rd::<usize>(world_controller, offset);
    if is(WC_WORLD_CAMERA) {
        let root: usize = rd(camera, CAM::spScene);
        if root != 0 && root == ptr_at(water_controller, WATER_PLANE) {
            SCENE_WATER
        } else if root != rd::<usize>(world_controller, WC_WORLD_ROOT) {
            SCENE_OTHER
        } else if app_culled(root) {
            SCENE_SORTED_ALPHA
        } else {
            SCENE_WORLD
        }
    } else if is(WC_SHADOW_CAMERA) {
        let scene_objects = ptr_at(rd(world_controller, WC_SHADOW_MANAGER), SM_SCENE_OBJECTS);
        if app_culled(scene_objects) {
            SCENE_SHADOW_VOLUME
        } else {
            SCENE_SHADOW_CASTERS
        }
    } else if is(WC_ARM_CAMERA) {
        SCENE_FIRST_PERSON
    } else if is(WC_MENU_CAMERA) {
        SCENE_UI
    } else if is(WC_SPLASH_CAMERA) {
        SCENE_SPLASH
    } else if is(WC_PREVIEW_CAMERA) {
        SCENE_CHARACTER_PREVIEW
    } else if is(WC_MAP_CAMERA) {
        SCENE_LOCAL_MAP
    } else if camera == rd::<usize>(RACE_MENU_CAMERA, 0) {
        SCENE_RACE_MENU_HEAD
    } else if camera == reflection_camera(water_controller) {
        SCENE_WATER_REFLECTION
    } else {
        SCENE_OTHER
    }
}

/// `SetCameraData(this, location, direction, up, right, frustum, port)`. `NiCamera::Click`
/// calls it immediately before `BeginPaint`. The scene packet goes out here, because this
/// function sends the view transform and MGE XE decides on the scene at that call.
unsafe fn set_camera_data(args: &[usize]) {
    let camera = args[1] - AV_WORLD_TRANSLATION;
    send_scene(camera, crate::camera_space::scene_begin(args[1]));
    SCENE_SENT_FOR.store(camera, Ordering::Relaxed);
}

/// `BeginPaint(this, camera)`
unsafe fn begin_paint(args: &[usize]) {
    // A caller other than `NiCamera::Click` did not go through `SetCameraData`.
    if SCENE_SENT_FOR.swap(0, Ordering::Relaxed) != args[1] {
        send_scene(args[1], crate::camera_space::scene_begin(0));
    }
}

unsafe fn send_scene(camera: usize, origin: Option<[f64; 3]>) {
    let Some(host) = host() else { return };
    let world_controller: usize = rd(WORLD_CONTROLLER, 0);
    let data_handler: usize = rd(DATA_HANDLER, 0);
    let water_controller = ptr_at(data_handler, DH_WATER_CONTROLLER);
    let weather = ptr_at(world_controller, WC_WEATHER);
    let shadow_manager = ptr_at(world_controller, WC_SHADOW_MANAGER);
    let masser = ptr_at(weather, W_MASSER);
    let secunda = ptr_at(weather, W_SECUNDA);

    let roots = &mut *addr_of_mut!(ROOTS);
    *roots = Roots {
        scene_kind: scene_kind(camera, world_controller, water_controller),
        render_target: roots.render_target,
        sky_root: ptr_at(weather, W_SKY_ROOT),
        sky_dome: ptr_at(weather, W_SKY_DOME),
        night_sky: ptr_at(weather, W_NIGHT_SKY),
        clouds: ptr_at(weather, W_CLOUDS),
        sun_vis: ptr_at(weather, W_SUN_VIS),
        sun_base: ptr_at(weather, W_SUN_BASE),
        sun_glare: ptr_at(weather, W_SUN_GLARE),
        moon_roots: [
            ptr_at(masser, MOON_ROOT),
            ptr_at(secunda, MOON_ROOT),
            ptr_at(masser, MOON_NODE),
            ptr_at(secunda, MOON_NODE),
        ],
        // The shadow shape of each moon, and the node above it.
        moon_shadows: [
            ptr_at(masser, MOON_SHADOW_SHAPE),
            ptr_at(secunda, MOON_SHADOW_SHAPE),
            ptr_at(masser, MOON_SHADOW_NODE),
            ptr_at(secunda, MOON_SHADOW_NODE),
        ],
        particles: [ptr_at(weather, W_RAIN), ptr_at(weather, W_SNOW), ptr_at(weather, W_STORM)],
        water_ripples: ptr_at(water_controller, WATER_RIPPLES),
        water_plane: ptr_at(water_controller, WATER_PLANE),
        landscape_root: ptr_at(data_handler, DH_LANDSCAPE_ROOT),
        object_root: ptr_at(data_handler, DH_OBJECT_ROOT),
        shadow_volume: ptr_at(shadow_manager, SM_SHADOW_VOLUME),
        shadow_overlay: ptr_at(shadow_manager, SM_SHADOW_OVERLAY),
    };
    let scene = SceneV1 {
        struct_size: size_of::<SceneV1>() as u32,
        kind: roots.scene_kind,
        camera: camera as u32,
        scene_root: ptr_at(camera, CAM::spScene) as u32,
        render_target: roots.render_target,
        flags: if origin.is_some() { SCENE_HAS_CAMERA_ORIGIN } else { 0 } | if in_interior() { SCENE_INTERIOR } else { 0 },
        camera_origin: origin.unwrap_or_default(),
    };
    (host.scene_begin)(&scene);
    crate::capture::scene_begin(&scene);
}

/// Runs after `EndPaint`.
unsafe fn end_paint() {
    if let Some(host) = host() {
        (host.scene_end)();
        crate::capture::scene_end();
    }
}

/// `ClearBuffer(this, rect, flags)`
unsafe fn clear_buffer(args: &[usize]) {
    if let Some(host) = host() {
        (host.frame_event)(EVENT_CLEAR, args[2] as u32, 0);
        crate::capture::frame_event(EVENT_CLEAR, args[2] as u32, 0);
    }
}

/// `SetRenderTarget(this, texture)`
unsafe fn set_render_target(args: &[usize]) {
    (*addr_of_mut!(ROOTS)).render_target = args[1] as u32;
    if let Some(host) = host() {
        (host.frame_event)(EVENT_SET_TARGET, args[1] as u32, 0);
        crate::capture::frame_event(EVENT_SET_TARGET, args[1] as u32, 0);
    }
}

/// `SwapBuffers(this)`
unsafe fn swap_buffers(_args: &[usize]) {
    crate::camera_space::frame_end();
    if let Some(host) = host() {
        (host.frame_event)(EVENT_SWAP, 0, 0);
        crate::capture::frame_event(EVENT_SWAP, 0, 0);
    }
}

/// The class of a geometry: a compare of the geometry and of each parent with the known
/// objects. The scene kind gives the class when the walk finds none.
unsafe fn object_class(geometry: usize, roots: &Roots) -> u32 {
    match roots.scene_kind {
        SCENE_UI | SCENE_SPLASH | SCENE_RACE_MENU_HEAD => return CLASS_UI,
        _ => {}
    }
    if roots.moon_shadows.contains(&geometry) {
        return CLASS_SKY_MOON_SHADOW;
    }
    let mut node = geometry;
    for _ in 0..MAX_DEPTH {
        if node == 0 {
            break;
        }
        let class = if node == roots.sky_dome {
            CLASS_SKY_DOME
        } else if node == roots.night_sky {
            CLASS_SKY_NIGHT
        } else if node == roots.clouds {
            CLASS_SKY_CLOUDS
        } else if node == roots.sun_vis || node == roots.sun_base {
            CLASS_SKY_SUN
        } else if roots.moon_shadows.contains(&node) {
            CLASS_SKY_MOON_SHADOW
        } else if roots.moon_roots.contains(&node) {
            CLASS_SKY_MOON
        } else if node == roots.sky_root {
            CLASS_SKY_OTHER
        } else if node == roots.sun_glare {
            CLASS_SUN_GLARE
        } else if roots.particles.contains(&node) {
            CLASS_WEATHER_PARTICLES
        } else if node == roots.water_ripples {
            CLASS_WATER_RIPPLE
        } else if node == roots.water_plane {
            CLASS_WATER_PLANE
        } else if node == roots.landscape_root {
            CLASS_LANDSCAPE
        } else if node == roots.object_root {
            CLASS_OBJECT
        } else if node == roots.shadow_volume {
            CLASS_SHADOW_VOLUME
        } else if node == roots.shadow_overlay {
            CLASS_SHADOW_OVERLAY
        } else {
            node = rd(node, AV::parentNode);
            continue;
        };
        return class;
    }
    if roots.scene_kind == SCENE_FIRST_PERSON {
        CLASS_FIRST_PERSON
    } else {
        CLASS_UNKNOWN
    }
}

/// 0x4D2F40 `game_dynamicLightTest`, cdecl with 6 arguments, and its first instruction.
const DYNAMIC_LIGHT_TEST: usize = 0x4D2F40;
const DYNAMIC_LIGHT_TEST_HEAD: [u8; 6] = [0x64, 0xA1, 0x00, 0x00, 0x00, 0x00];
static mut DYNAMIC_LIGHT_TEST_ORIGINAL: usize = 0;
/// The lights that `game_dynamicLightTest` got, with the radius that it got for each: the
/// address of the light and the radius. The slot comes from the address.
const GAME_LIGHT_SLOTS: usize = 2048;
static mut GAME_LIGHTS: [(u32, f32); GAME_LIGHT_SLOTS] = [(0, 0.0); GAME_LIGHT_SLOTS];

fn game_light_slot(light: usize) -> usize {
    (light >> 4) % GAME_LIGHT_SLOTS
}

/// The detour of `game_dynamicLightTest`: a note of the light, then the game function.
unsafe extern "C" fn dynamic_light_test_detour(light: usize, node: usize, radius: u32, flags: u32, is_land: u32, high_priority: u32) {
    if light != 0 {
        (*addr_of_mut!(GAME_LIGHTS))[game_light_slot(light)] = (light as u32, radius as f32);
    }
    let original: unsafe extern "C" fn(usize, usize, u32, u32, u32, u32) = core::mem::transmute(*addr_of!(DYNAMIC_LIGHT_TEST_ORIGINAL));
    original(light, node, radius, flags, is_land, high_priority)
}

/// The radius that the game keeps in a point light, or 0 when it is not known.
///
/// `game_dynamicLightTest` writes the radius into the three parts of the specular colour.
/// A light that is part of a mesh does not go through that function, and its specular
/// colour is a colour. Thus the value counts only for a light that the function got, and
/// only while the colour is the radius that the function got.
unsafe fn game_light_radius(light: usize) -> f32 {
    let (noted, radius) = (*addr_of!(GAME_LIGHTS))[game_light_slot(light)];
    let specular: [f32; 3] = rd(light, L::specularColor_r);
    if noted == light as u32 && specular == [radius; 3] {
        radius
    } else {
        0.0
    }
}

/// Fills `LIGHTS` with the lights of an effect state that the light manager sends to the
/// device. Returns the number of lights.
unsafe fn collect_lights(effect_state: usize) -> usize {
    let lights = &mut *addr_of_mut!(LIGHTS);
    let data_handler: usize = rd(DATA_HANDLER, 0);
    let sun_light: usize = if data_handler != 0 { rd(data_handler, DATA_HANDLER_SUN_LIGHT) } else { 0 };
    let mut count = 0;
    let mut iter = ptr_at(effect_state, DES::lights);
    while iter != 0 && count < MAX_LIGHTS {
        let light: usize = rd(iter, DL::effect);
        iter = rd(iter, DL::next);
        if light == 0 || rd::<u8>(light, L::enabled) == 0 || rd::<f32>(light, L::dimmer) < MIN_DIMMER {
            continue;
        }
        let mut light_type = vcall0(light, LIGHT_VTBL_GET_TYPE);
        if light_type == LIGHT_DIRECTIONAL && light == sun_light {
            light_type = LIGHT_SUN;
        }
        lights[count] = LightV1 {
            light: light as u32,
            device_index: rd(light, L::index),
            light_type,
            radius: if light_type == LIGHT_POINT { game_light_radius(light) } else { 0.0 },
        };
        count += 1;
    }
    count
}

unsafe fn send_draw(kind: u32, this: usize, geometry: usize, geometry_data: usize, skin: usize) {
    let Some(host) = host() else { return };
    let roots = &*addr_of!(ROOTS);
    let mut flags = 0;
    if skin != 0 {
        flags |= DRAW_SKINNED;
    }
    // The accumulator is null while the sorted queue draws its objects.
    let world_camera = matches!(roots.scene_kind, SCENE_WORLD | SCENE_SORTED_ALPHA | SCENE_WATER);
    if world_camera && rd::<usize>(this, R::accumulator) == 0 {
        flags |= DRAW_FROM_SORTED_QUEUE;
    }
    let property_state: usize = rd(this, R::currentPropertyState);
    let effect_state: usize = rd(this, R::currentEffectState);
    let material = ptr_at(property_state, PS::propMaterial);
    let light_count = collect_lights(effect_state);
    let draw = DrawV1 {
        struct_size: size_of::<DrawV1>() as u32,
        kind,
        object_class: if geometry != 0 { object_class(geometry, roots) } else { CLASS_UNKNOWN },
        flags,
        geometry: geometry as u32,
        geometry_data: geometry_data as u32,
        skin_instance: skin as u32,
        property_state: property_state as u32,
        effect_state: effect_state as u32,
        material_shininess: if material != 0 { rd(material, MP::shininess) } else { 0.0 },
        light_count: light_count as u32,
        lights: addr_of!(LIGHTS) as *const LightV1,
    };
    let counts = &mut *addr_of_mut!(CLASS_COUNTS);
    if let Some(count) = counts.get_mut(draw.object_class as usize) {
        *count += 1;
    }
    (host.draw)(&draw);
    let geometry_handle = crate::geometry_stream::note(kind, geometry_data, skin);
    let material_handle = crate::material_stream::note(this, property_state, effect_state);
    crate::shape_stream::note_handles(geometry_handle, material_handle);
    crate::capture::object(&draw, geometry_handle, material_handle);
}

/// Sends the state of the D3D8 draw call that follows. The draw ports call it. `renderer` is
/// the `NiDX8Renderer` and `geom_buffer` the `NiGeometryBufferData` of the draw.
///
/// The packet goes out only when the Rust code draws for real: not in the Rust run of a
/// comparison, where the device is dry and the game code makes the draw after it.
pub unsafe fn draw_state(
    renderer: usize,
    primitive_type: u32,
    vertex_count: u32,
    start_index: u32,
    primitive_count: u32,
    skinned: bool,
    later_pass: bool,
) {
    let Some(host) = host() else { return };
    if host.capabilities & CAP_DRAW_STATE == 0 || crate::shadow::leg() == crate::shadow::Leg::Rust {
        return;
    }
    // The render state object of the game keeps the value of each state that it sent to the
    // device. The Rust ports write this record, so it is the state of this draw.
    RENDERER.store(renderer, Ordering::Relaxed);
    let render_state: usize = rd(renderer, R::renderState);
    let rs = |state: u32| -> u32 { rd(render_state, RS::renderStateSettings + state as usize * 8) };
    let buffers = *addr_of!(BUFFERS);
    let mut samplers = [[0u32; STATE_SAMPLER_STATES as usize]; STATE_STAGES as usize];
    for (stage, values) in samplers.iter_mut().enumerate() {
        for (value, state) in values.iter_mut().zip(SAMPLER_STATES) {
            *value = rd(render_state, RS::maybe_textureStageStateSettings + (state as usize + stage * 32) * 8);
        }
    }
    let mut material = [0.0f32; 17];
    let entry: usize = rd(render_state, RS::materialManager_currentMaterialEntry);
    if entry != 0 {
        core::ptr::copy_nonoverlapping((entry + ME::d3dMaterial) as *const f32, material.as_mut_ptr(), 17);
    }
    // The record of the texture stage states is in the same object: 32 states for a stage.
    let mut stages = [[0u32; STATE_STAGE_STATES as usize]; STATE_STAGES as usize];
    for (stage, values) in stages.iter_mut().enumerate() {
        for (value, state) in values.iter_mut().zip(STAGE_STATES) {
            *value = rd(render_state, RS::maybe_textureStageStateSettings + (state as usize + stage * 32) * 8);
        }
    }
    // The light manager has a list of the lights that are on, and an entry with the
    // D3DLIGHT8 for each light that it sent.
    let lights = &mut *addr_of_mut!(STATE_LIGHTS);
    let mut light_count = 0;
    let manager: usize = rd(renderer, R::lightManager);
    let mut item: usize = rd(manager, LM::deviceLights_head);
    while item != 0 && light_count < MAX_STATE_LIGHTS {
        let light: usize = rd(item, ITEM_DATA);
        item = rd(item, ITEM_NEXT);
        let entry = if light != 0 { crate::ports::hash_map::find_value(manager + LM::lightEntries, rd(light, L::index)) } else { 0 };
        if entry == 0 {
            continue;
        }
        lights[light_count].device_index = rd(entry, LE::lightIndex);
        core::ptr::copy_nonoverlapping((entry + LE::d3dLight) as *const u32, lights[light_count].d3d_light.as_mut_ptr(), 26);
        light_count += 1;
    }
    let mut state = DrawStateV1 {
        light_count: light_count as u32,
        lights: addr_of!(STATE_LIGHTS) as *const StateLightV1,
        stages,
        depth_write: rs(D3DRS_ZWRITEENABLE),
        cull_mode: rs(D3DRS_CULLMODE),
        vertex_blend: rs(D3DRS_VERTEXBLEND),
        blend_enable: rs(D3DRS_ALPHABLENDENABLE),
        source_blend: rs(D3DRS_SRCBLEND),
        destination_blend: rs(D3DRS_DESTBLEND),
        alpha_test_enable: rs(D3DRS_ALPHATESTENABLE),
        alpha_function: rs(D3DRS_ALPHAFUNC),
        alpha_reference: rs(D3DRS_ALPHAREF),
        lighting: rs(D3DRS_LIGHTING),
        fog_enable: rs(D3DRS_FOGENABLE),
        diffuse_material_source: rs(D3DRS_DIFFUSEMATERIALSOURCE),
        emissive_material_source: rs(D3DRS_EMISSIVEMATERIALSOURCE),
        ambient: rs(D3DRS_AMBIENT),
        material,
        struct_size: size_of::<DrawStateV1>() as u32,
        flags: if skinned { STATE_SKINNED } else { 0 } | if later_pass { STATE_LATER_PASS } else { 0 },
        primitive_type,
        base_vertex_index: buffers[4],
        min_index: 0,
        vertex_count,
        start_index,
        primitive_count,
        vertex_buffer: buffers[0],
        vertex_stride: buffers[1],
        // The game keeps the vertex format that it sent and sends no equal value again.
        vertex_format: rd(render_state, RS::currentVertexShader),
        index_buffer: buffers[3],
        depth_enable: rs(7),
        depth_function: rs(23),
        stencil: [rs(52), rs(53), rs(54), rs(55), rs(56), rs(57), rs(58), rs(59)],
        samplers,
        world_relative_mask: *addr_of!(WORLD_RELATIVE),
        world: *addr_of!(WORLDS[0]),
        blend_worlds: [*addr_of!(WORLDS[1]), *addr_of!(WORLDS[2]), *addr_of!(WORLDS[3])],
        textures: *addr_of!(TEXTURES),
        texture_transforms: *addr_of!(TEXTURE_TRANSFORMS),
    };
    if !skinned {
        core::ptr::copy_nonoverlapping((renderer + R::currentWorldMatrix) as *const f32, state.world.as_mut_ptr(), 16);
    }
    (host.draw_state)(&state);
    crate::capture::draw_state(&state);
}

/// The size of a state light.
pub const STATE_LIGHT_SIZE: usize = size_of::<StateLightV1>();

/// The lights of the light manager of the renderer as state lights, as `draw_state` reads
/// them: the bytes of the records.
pub unsafe fn state_lights(renderer: usize) -> &'static [u8] {
    let lights = &mut *addr_of_mut!(STATE_LIGHTS);
    let mut light_count = 0;
    let manager: usize = rd(renderer, R::lightManager);
    let mut item: usize = rd(manager, LM::deviceLights_head);
    while item != 0 && light_count < MAX_STATE_LIGHTS {
        let light: usize = rd(item, ITEM_DATA);
        item = rd(item, ITEM_NEXT);
        let entry = if light != 0 { crate::ports::hash_map::find_value(manager + LM::lightEntries, rd(light, L::index)) } else { 0 };
        if entry == 0 {
            continue;
        }
        lights[light_count].device_index = rd(entry, LE::lightIndex);
        core::ptr::copy_nonoverlapping((entry + LE::d3dLight) as *const u32, lights[light_count].d3d_light.as_mut_ptr(), 26);
        light_count += 1;
    }
    core::slice::from_raw_parts(addr_of!(STATE_LIGHTS) as *const u8, light_count * STATE_LIGHT_SIZE)
}

/// True when the world matrix of the renderer is relative to the camera.
pub unsafe fn world_is_relative() -> bool {
    *addr_of!(WORLD_RELATIVE) & 1 != 0
}

/// `RenderShape(this, geometryData, skinInstance, transform, bound)`
unsafe fn render_shape(args: &[usize]) {
    crate::camera_space::geometry_begin(args[3]);
    send_draw(DRAW_SHAPE, args[0], args[3] - AV_WORLD_TRANSFORM, args[1], args[2]);
}

/// `RenderTriStrips(this, geometryData, skinInstance, transform, bound)`
unsafe fn render_tri_strips(args: &[usize]) {
    crate::camera_space::geometry_begin(args[3]);
    send_draw(DRAW_STRIPS, args[0], args[3] - AV_WORLD_TRANSFORM, args[1], args[2]);
}

// `NiParticles::Display` (0x6DD3E0) and `NiLines::Display` (0x6DA850) give the renderer the
// address of their own world transform, as the shapes do: the object of the draw is at
// that address less the offset of the transform, and its class comes from its parents.

/// `RenderPoints(this, geometryData, transform, bound)`
unsafe fn render_points(args: &[usize]) {
    crate::camera_space::geometry_begin(args[2]);
    send_draw(DRAW_POINTS, args[0], args[2] - AV_WORLD_TRANSFORM, args[1], 0);
}

/// `RenderLines(this, linesData, transform, bound)`
unsafe fn render_lines(args: &[usize]) {
    crate::camera_space::geometry_begin(args[2]);
    send_draw(DRAW_LINES, args[0], args[2] - AV_WORLD_TRANSFORM, args[1], 0);
}

/// `RenderScreenPoly(this, ...)`
unsafe fn render_screen_poly(args: &[usize]) {
    crate::camera_space::geometry_begin(0);
    send_draw(DRAW_SCREEN_POLYGON, args[0], 0, 0, 0);
}

/// The `d3d8.dll` that is in the folder of the game. The Direct3D 8 of Windows has the same
/// file name and is also in the process, because MGE XE loads it.
unsafe fn host_module() -> usize {
    let mut path = [0u16; 600];
    let len = win::GetModuleFileNameW(core::ptr::null_mut(), path.as_mut_ptr(), 512) as usize;
    let Some(slash) = path[..len].iter().rposition(|&c| c == b'\\' as u16) else { return 0 };
    let name: Vec<u16> = "d3d8.dll\0".encode_utf16().collect();
    path[slash + 1..slash + 1 + name.len()].copy_from_slice(&name);
    win::GetModuleHandleW(path.as_ptr()) as usize
}

/// True when the game gets its Direct3D 8 from the plugin. This is true before the game asks
/// for it. The `d3d8.dll` of the game folder says so with its function `NirenderD3D8`, which
/// gives a number that is not 0: the file is the one of this repository, or it is MGE XE
/// with its switch `runtime.renderer_wgpu` on.
pub unsafe fn own_d3d8() -> bool {
    let module = host_module();
    if module == 0 {
        return false;
    }
    let export = win::GetProcAddress(module as *mut c_void, b"NirenderD3D8\0".as_ptr());
    !export.is_null() && core::mem::transmute::<*mut c_void, unsafe extern "system" fn() -> u32>(export)() != 0
}

/// Connects to the host and puts the packet functions on the ports. The ports must have
/// their detours. Returns what happened, as text.
pub unsafe fn connect() -> Result<String, String> {
    if HOST.load(Ordering::Relaxed) != 0 {
        return Ok("connected before".to_string());
    }
    let table: *const HostV1 = if crate::own_device::present() || own_d3d8() {
        // The device of the game is the one of the plugin. The plugin is the host too.
        crate::own_host::table()
    } else {
        let module = host_module();
        if module == 0 {
            return Err("no d3d8.dll of the game folder is in the process".to_string());
        }
        let export = win::GetProcAddress(module as *mut c_void, CONNECT_EXPORT.as_ptr());
        if export.is_null() {
            return Err("the d3d8.dll has no MGE_RenderLinkConnect".to_string());
        }
        let connect: ConnectFn = core::mem::transmute(export);
        connect(VERSION)
    };
    let Some(host) = table.as_ref() else {
        return Err(format!("the host refused version {VERSION}"));
    };
    if *addr_of!(DYNAMIC_LIGHT_TEST_ORIGINAL) == 0 {
        let detour = dynamic_light_test_detour as *const () as usize;
        *addr_of_mut!(DYNAMIC_LIGHT_TEST_ORIGINAL) =
            crate::hook::install_raw(DYNAMIC_LIGHT_TEST, &DYNAMIC_LIGHT_TEST_HEAD, &[], detour, 6, crate::hook::Cleanup::Caller)?;
    }
    // The members up to `read_counters` are in each version of the table.
    if (host.struct_size as usize) < core::mem::offset_of!(HostV1, use_facts) {
        return Err(format!("the table of the host has {} bytes", host.struct_size));
    }

    let before: [(usize, BeforeFn); 10] = [
        (SET_CAMERA_DATA, set_camera_data),
        (BEGIN_PAINT, begin_paint),
        (CLEAR_BUFFER, clear_buffer),
        (SET_RENDER_TARGET, set_render_target),
        (SWAP_BUFFERS, swap_buffers),
        (RENDER_SHAPE, render_shape),
        (RENDER_TRI_STRIPS, render_tri_strips),
        (RENDER_POINTS, render_points),
        (RENDER_LINES, render_lines),
        (RENDER_SCREEN_POLY, render_screen_poly),
    ];
    let mut missing = Vec::new();
    for (addr, function) in before {
        match ports::all().find(|p| p.addr == addr && p.installed()) {
            Some(port) => port.before.store(function as usize, Ordering::Relaxed),
            None => missing.push(format!("{addr:#x}")),
        }
    }
    match ports::all().find(|p| p.addr == END_PAINT && p.installed()) {
        Some(port) => port.after.store(end_paint as *const () as usize, Ordering::Relaxed),
        None => missing.push(format!("{END_PAINT:#x}")),
    }
    HOST.store(table as usize, Ordering::Relaxed);
    if own_d3d8() {
        find_stream_functions();
    }
    Ok(format!(
        "host version {}, capabilities {:#x}; functions without a detour: {}",
        host.version,
        host.capabilities,
        if missing.is_empty() { "none".to_string() } else { missing.join(" ") }
    ))
}

/// Tells the host which facts to take from the packets. Bit n of `mask` is the fact `FACT_`
/// n. Returns the mask that the host uses, or `None` when the host cannot use facts.
pub unsafe fn use_facts(mask: u32) -> Option<u32> {
    let host = host()?;
    if host.capabilities & CAP_USE_FACTS == 0 || (host.struct_size as usize) < size_of::<HostV1>() {
        return None;
    }
    let used = (host.use_facts)(mask);
    USED_FACTS.store(used, Ordering::Relaxed);
    if used >> FACT_CLIENT_CAMERA_SPACE & 1 != 0 {
        if let Err(error) = crate::camera_space::install() {
            crate::log!("camera space: {error}");
        }
    }
    let before = SUPPRESS.swap(used & DEVICE_FACTS, Ordering::Relaxed);
    if before & !used != 0 {
        resync(before & !used);
    }
    Some(used)
}

/// The plugin makes the source textures through the link. See `ports/texture_link.rs`.
static LINK_TEXTURES: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Sets the use of the link for new source textures. Returns false when the host cannot
/// make textures.
pub unsafe fn set_link_textures(on: bool) -> bool {
    let able = host().is_some_and(|host| host.capabilities & CAP_TEXTURES != 0 && host.struct_size as usize >= size_of::<HostV1>());
    LINK_TEXTURES.store(on && able, Ordering::Relaxed);
    able
}

/// True when a new source texture must come from the link. Not in the Rust half of a shadow
/// test: that half must not make a device object.
pub fn link_textures() -> bool {
    LINK_TEXTURES.load(Ordering::Relaxed) && crate::shadow::leg() != crate::shadow::Leg::Rust
}

/// Asks the host for a texture with these pixels. Returns the handle, or 0.
pub unsafe fn texture_create(texture: &TextureV1) -> usize {
    match host() {
        Some(host) => {
            let handle = (host.texture_create)(texture) as usize;
            crate::capture::texture(texture, handle);
            handle
        }
        None => 0,
    }
}

/// The counters of the host, or `None` without a host.
pub unsafe fn counters(reset: bool) -> Option<CountersV1> {
    let host = host()?;
    let mut out = CountersV1 { struct_size: size_of::<CountersV1>() as u32, ..Default::default() };
    (host.read_counters)(&mut out, reset as u32);
    Some(out)
}

/// The counters as text: one line for each fact that the host compared.
pub unsafe fn counters_text(reset: bool) -> String {
    let Some(c) = counters(reset) else {
        return "no host".to_string();
    };
    let mut text = format!("scenes {} draws {} draws_without_packet {}\n", c.scenes, c.draws, c.draws_without_packet);
    for (index, name) in FACT_NAMES.iter().enumerate() {
        let fact = c.facts[index];
        text.push_str(&format!(
            "fact {}: both_yes {} both_no {} host_only {} packet_only {}\n",
            name.replace(' ', "_"),
            fact.both_yes,
            fact.both_no,
            fact.host_only,
            fact.packet_only
        ));
    }
    // The index is `CLASS_*` of the contract.
    let counts = &mut *addr_of_mut!(CLASS_COUNTS);
    text.push_str("classes:");
    for count in counts.iter() {
        text.push_str(&format!(" {count}"));
    }
    text.push('\n');
    if reset {
        *counts = [0; CLASS_COUNT];
    }
    text.push_str("object_calls:");
    for (name, count) in OBJECT_CALL_NAMES.iter().zip((*addr_of_mut!(OBJECT_CALLS)).iter_mut()) {
        if *count != 0 {
            text.push_str(&format!(" {name}={count}"));
        }
        if reset {
            *count = 0;
        }
    }
    text.push(0xA as char);
    text.push_str(&format!("frame_calls_to_host: {}", *addr_of!(FRAME_CALLS)));
    text.push(0xA as char);
    if reset {
        *addr_of_mut!(FRAME_CALLS) = 0;
    }
    let skipped = &mut *addr_of_mut!(SKIPPED);
    text.push_str(&format!(
        "calls_not_made: render_states {} stage_states {} textures {} buffers {} transforms {} lights {}",
        skipped[0], skipped[1], skipped[2], skipped[3], skipped[4], skipped[5]
    ));
    text.push(0xA as char);
    if reset {
        *skipped = [0; 6];
    }
    // The states that the Rust code asked for, as `state=calls`.
    text.push_str(&crate::camera_space::report(reset));
    text.push(0xA as char);
    let render: &mut [u32] = &mut *addr_of_mut!(RENDER_STATE_SETS);
    let stage: &mut [u32] = &mut *addr_of_mut!(STAGE_STATE_SETS);
    let sets = [("render_states:", render), ("stage_states:", stage)];
    for (label, sets) in sets {
        text.push_str(label);
        for (state, count) in sets.iter_mut().enumerate() {
            if *count != 0 {
                text.push_str(&format!(" {state}={count}"));
            }
            if reset {
                *count = 0;
            }
        }
        text.push(0xA as char);
    }
    text
}
