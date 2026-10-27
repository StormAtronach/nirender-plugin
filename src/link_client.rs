//! The client of the render link. It tells the host (`own_host`) the scene and the object of the device
//! calls that follow. `link.rs` has the contract.
//!
//! The packets come from `before` functions on the ports. They do not depend on the mode of
//! a port: the game code and the Rust code send the same packets.
//!
//! The addresses of the game objects come from
//! `moreFPS/docs/engineering-notes/renderer/scene-phases-and-draw-classification.md`.

use core::ffi::c_void;
use core::mem::size_of;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

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
use crate::gen::layouts::{
    DataHandler, InterfaceNiLight, Moon, NVSkyRenderToTexture, ShadowManager, TES3Game, TES3WaterNVLake, WaterController,
    WeatherController, WorldController,
};
use crate::d3d::{D3DTS_TEXTURE0, D3DTS_WORLDMATRIX0};
use crate::link::*;
use crate::checked::Open;
use crate::main_thread::{MainCell, MainThread};
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
/// The directional light of the sun, and of an interior cell.
const DATA_HANDLER_SUN_LIGHT: usize = DataHandler::sgSunlight;
/// `*(NiCamera**)`: the camera of the race menu head.
const RACE_MENU_CAMERA: usize = 0x7D5838;

// The world controller.
const WC_SPLASH_CAMERA: usize = WorldController::splashscreenCamera_cameraData_camera;
const WC_WORLD_ROOT: usize = WorldController::worldCamera_sgRoot;
const WC_WORLD_CAMERA: usize = WorldController::worldCamera_cameraData_camera;
const WC_ARM_CAMERA: usize = WorldController::armCamera_cameraData_camera;
const WC_MENU_CAMERA: usize = WorldController::menuCamera_cameraData_camera;
const WC_PREVIEW_CAMERA: usize = WorldController::characterRenderTarget_cameraData_camera;
const WC_MAP_CAMERA: usize = WorldController::mapRenderTarget_cameraData_camera;
const WC_SHADOW_CAMERA: usize = WorldController::shadowCamera_cameraData_camera;
const WC_SHADOW_MANAGER: usize = WorldController::shadowManager;
const WC_WEATHER: usize = WorldController::weatherController;

// The shadow manager.
const SM_SHADOW_VOLUME: usize = ShadowManager::sgShadowVolume;
const SM_SHADOW_OVERLAY: usize = ShadowManager::sgShadowOverlay;
const SM_SCENE_OBJECTS: usize = ShadowManager::sgSceneObjects;

// The data handler.
const DH_OBJECT_ROOT: usize = DataHandler::sgWorldObjectRoot;
const DH_LANDSCAPE_ROOT: usize = DataHandler::sgWorldLandscapeRoot;
const DH_WATER_CONTROLLER: usize = DataHandler::waterController;

// The water controller.
const WATER_NV_LAKE: usize = WaterController::waterNVLake;
const WATER_RIPPLES: usize = WaterController::sgRippleNode;
const WATER_PLANE: usize = WaterController::sgWaterPlane;

// The weather controller.
const W_SUN_VIS: usize = WeatherController::sgSunVis;
const W_SUN_BASE: usize = WeatherController::sgSunBase;
const W_SUN_GLARE: usize = WeatherController::sgSunGlare;
const W_SECUNDA: usize = WeatherController::secundaMoonData;
const W_MASSER: usize = WeatherController::masserMoonData;
const W_SKY_ROOT: usize = WeatherController::sgSkyRoot;
const W_NIGHT_SKY: usize = WeatherController::sgSkyNight;
const W_SKY_DOME: usize = WeatherController::sgSkyAtmosphere;
const W_CLOUDS: usize = WeatherController::sgSkyClouds;
const W_RAIN: usize = WeatherController::sgRain;
const W_SNOW: usize = WeatherController::sgSnowRoot;
const W_STORM: usize = WeatherController::sgStormRoot;

// A moon.
const MOON_ROOT: usize = Moon::sgMoonRoot;
const MOON_NODE: usize = Moon::sgMoonNode;
const MOON_SHADOW_NODE: usize = Moon::sgShadowNode;
const MOON_SHADOW_SHAPE: usize = Moon::sgShadowTriShape;

/// The translation of `worldTransform` in a `NiAVObject`. `NiCamera::Click` passes the address
/// of this member of the camera to `SetCameraData`.
const AV_WORLD_TRANSLATION: usize = AV::worldTransform_translation;
/// `worldTransform` in a `NiAVObject`. `NiTriShape::Display` passes the address of this member
/// to `RenderShape`.
const AV_WORLD_TRANSFORM: usize = AV::worldTransform;
const LIGHT_VTBL_GET_TYPE: usize = InterfaceNiLight::GetType;
/// The game does not use a light with a dimmer below this value (the constant 0x74F2EC;
/// `updateLights` compares the dimmer with it at 0x6BB7F7).
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
/// The state of the link that the ports' Rust writes on the main thread: the roots of the
/// scene, the lights of the packets, and the copies of what the Rust code sent to the device
/// last (the renderer of the game keeps no record of them). It opens only for the proof of
/// the main thread (`main_thread::MainThread`); a device call of another thread is counted
/// and does not open it.
struct LinkState {
    roots: Roots,
    /// The lights of the state packet in progress.
    state_lights: [StateLightV1; MAX_STATE_LIGHTS],
    /// The lights of the draw packet in progress.
    lights: [LightV1; MAX_LIGHTS],
    worlds: [[f32; 16]; 4],
    /// The transforms that the copies have: bit n is world matrix n, bit 8 + n is the
    /// texture transform of stage n.
    transforms_known: u32,
    /// The texture of each stage, with the serial of its object (`own_host::Kept`).
    textures: [crate::own_host::Kept; STATE_STAGES as usize],
    texture_transforms: [[f32; 16]; STATE_STAGES as usize],
    /// The space of the world matrices in `worlds`: bit n is 1 when matrix n is relative
    /// to the camera.
    world_relative: u32,
    /// The stages whose texture is in `textures`: bit n is stage n.
    textures_known: u32,
    /// The buffers that the Rust code sent to the device last: vertex buffer, stride,
    /// vertex format, index buffer, base vertex index.
    buffers: [u32; 5],
    /// The vertex buffer and the index buffer of `buffers`, with the serials of their objects.
    kept_buffers: [crate::own_host::Kept; 2],
    /// The copy of the device's vtable with the draw functions of the link (`draw_vtable`).
    draw_vtable: [usize; DEVICE_METHODS],
}

static STATE: MainCell<LinkState> = MainCell::new(LinkState {
    roots: NO_ROOTS,
    state_lights: [StateLightV1 { device_index: 0, d3d_light: [0; 26] }; MAX_STATE_LIGHTS],
    lights: [LightV1 { light: 0, device_index: 0, light_type: 0, radius: 0.0 }; MAX_LIGHTS],
    worlds: [[0.0; 16]; 4],
    transforms_known: 0,
    textures: [crate::own_host::Kept::NONE; STATE_STAGES as usize],
    texture_transforms: [[0.0; 16]; STATE_STAGES as usize],
    world_relative: 0,
    textures_known: 0,
    buffers: [0; 5],
    kept_buffers: [crate::own_host::Kept::NONE; 2],
    draw_vtable: [0; DEVICE_METHODS],
});

/// The state, for a holder of the proof of the main thread.
#[track_caller]
#[inline(always)]
fn state(t: &MainThread) -> Open<'_, LinkState> {
    STATE.get(t)
}

/// The kind of the scene of the Click in progress (`begin_paint` reads it from the camera).
pub fn scene_kind_now(t: &MainThread) -> u32 {
    state(t).roots.scene_kind
}

/// The weather's sky root as `begin_paint` read it for the Click in progress, or 0.
pub fn sky_root_now(t: &MainThread) -> usize {
    state(t).roots.sky_root
}

/// A count that only the main thread adds to, without a bus lock (as `shadow::Counter`),
/// and that a report reads and resets.
struct Count(core::sync::atomic::AtomicU32);

impl Count {
    const fn new() -> Count {
        Count(core::sync::atomic::AtomicU32::new(0))
    }
    #[inline(always)]
    fn add(&self) {
        self.0.store(self.0.load(Ordering::Relaxed).wrapping_add(1), Ordering::Relaxed);
    }
    fn get(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }
    fn reset(&self) {
        self.0.store(0, Ordering::Relaxed);
    }
}

/// The number of draw packets of each class since the last read of the counters.
static CLASS_COUNTS: [Count; CLASS_COUNT] = [const { Count::new() }; CLASS_COUNT];
const CLASS_COUNT: usize = 18;
const MAX_STATE_LIGHTS: usize = 64;

/// Keeps a world matrix or a texture transform that goes to the device.
#[inline]
pub unsafe fn note_transform(state: u32, matrix: usize) {
    // The device wrapper of a port: the main thread, or a call of another thread that the
    // census counts and the state does not take.
    let Some(t) = MainThread::here() else {
        crate::sync::off_thread("link", "transform");
        return;
    };
    let mut s = self::state(&t);
    let to = if (D3DTS_WORLDMATRIX0..D3DTS_WORLDMATRIX0 + 4).contains(&state) {
        s.transforms_known |= 1 << (state - D3DTS_WORLDMATRIX0);
        // The host knows the space of this matrix only now, during the call of the game.
        let bit = 1 << (state - D3DTS_WORLDMATRIX0);
        let relative = crate::camera_space::pending(&t)
            || host().is_some_and(|host| host.capabilities & CAP_WORLD_SPACE != 0 && (host.world_space)() != 0);
        s.world_relative = if relative { s.world_relative | bit } else { s.world_relative & !bit };
        &mut s.worlds[(state - D3DTS_WORLDMATRIX0) as usize]
    } else if (D3DTS_TEXTURE0..D3DTS_TEXTURE0 + STATE_STAGES).contains(&state) {
        s.transforms_known |= 0x100 << (state - D3DTS_TEXTURE0);
        &mut s.texture_transforms[(state - D3DTS_TEXTURE0) as usize]
    } else {
        return;
    };
    core::ptr::copy_nonoverlapping(matrix as *const f32, to.as_mut_ptr(), 16);
}

/// True when the copy of world matrix `index` is relative to the camera.
pub fn world_relative(t: &MainThread, index: u32) -> bool {
    state(t).world_relative >> index & 1 != 0
}

/// The copy of world matrix `index`, as bit patterns.
pub fn world_matrix(t: &MainThread, index: u32) -> [u32; 16] {
    state(t).worlds[index as usize].map(f32::to_bits)
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

/// The renderer of the last state packet.
static RENDERER: AtomicUsize = AtomicUsize::new(0);
/// The displays of the water plane in the scene in progress, those seen in all, and those
/// skipped in all.
static WATER_PLANE_DISPLAYS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static WATER_PLANE_SEEN: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static WATER_PLANE_SKIPPED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
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
static SKIPPED: [Count; 6] = [const { Count::new() }; 6];

/// True when the host has this device kind. It counts the call that the caller then skips.
#[inline]
fn skip(fact: u32, in_packet: bool) -> bool {
    let skipped = in_packet && SUPPRESS.load(Ordering::Relaxed) >> fact & 1 != 0;
    if skipped {
        SKIPPED[(fact - FACT_DEVICE_RENDER_STATES) as usize].add();
    }
    skipped
}

/// The interior cell that the player is in, or 0.
const CURRENT_INTERIOR_CELL: usize = DataHandler::currentInteriorCell;

/// True when the player is in an interior cell.
unsafe fn in_interior() -> bool {
    let data_handler: usize = rd(DATA_HANDLER, 0);
    data_handler != 0 && rd::<usize>(data_handler, CURRENT_INTERIOR_CELL) != 0
}

/// `*(TES3Game**)`; `TES3Game::wireframeProperty` (+0xB0), the one wireframe property of the
/// world (MWSE `TES3Game.h`). The console command ToggleWireframe turns it on and off
/// (`MenuController::setWireframeEnabled` in MWSE).
const TES3_GAME: usize = 0x7C6CDC;
const GAME_WIREFRAME_PROPERTY: usize = TES3Game::sgWireframeProperty;
/// A u16; bit 0 of a wireframe property is "on".
const PROPERTY_FLAGS: usize = crate::gen::layouts::NiProperty::flags;

/// True when the game draws the world in wireframe.
unsafe fn game_wireframe() -> bool {
    let game: usize = rd(TES3_GAME, 0);
    let property: usize = if game != 0 { rd(game, GAME_WIREFRAME_PROPERTY) } else { 0 };
    property != 0 && rd::<u16>(property, PROPERTY_FLAGS) & 1 != 0
}

/// True when the game hides the sky: `WeatherController` 0x440870 culls the sky root for an
/// interior cell that does not behave like an exterior, and for the console command
/// ToggleSky; 0x440820 shows it again. Without a sky root there is no sky either.
unsafe fn sky_hidden(sky_root: usize) -> bool {
    sky_root == 0 || app_culled(sky_root)
}

/// The two functions of the `d3d8.dll` of MGE XE for a game without a Direct3D device
/// (`MGE_StreamPresent`, `MGE_StreamTransform`), or 0. MGE XE runs in them the logic of a
/// frame that does not draw: zoom, field of view, screen shake, crosshair. With a Direct3D
/// device, its proxy device runs that logic at `Present` and at `SetTransform`.
static STREAM_PRESENT: AtomicUsize = AtomicUsize::new(0);
static STREAM_TRANSFORM: AtomicUsize = AtomicUsize::new(0);
/// `MGE_StreamMessage` of MGE XE: a message of the extension in the host program for MGE XE
/// (`Extension::messages` of the crate), or 0.
static STREAM_MESSAGE: AtomicUsize = AtomicUsize::new(0);
/// `MGE_StreamSceneBegin` of MGE XE (the contract, `mge_render_link.h`): the start of a
/// camera's Click with the scene packet, or 0. MGE XE takes its main view and its eye from it
/// and from the camera object it names, not from the view matrix.
static STREAM_SCENE_BEGIN: AtomicUsize = AtomicUsize::new(0);

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
    let message = win::GetProcAddress(module as *mut c_void, b"MGE_StreamMessage\0".as_ptr());
    STREAM_MESSAGE.store(message as usize, Ordering::Relaxed);
    let scene_begin = win::GetProcAddress(module as *mut c_void, b"MGE_StreamSceneBegin\0".as_ptr());
    STREAM_SCENE_BEGIN.store(scene_begin as usize, Ordering::Relaxed);
    if !present.is_null() {
        crate::log!("link: the frame logic of MGE XE runs from the stream");
    }
}

/// The game showed a frame and MGE XE has not yet run its logic of that frame.
static PRESENT_DUE: AtomicBool = AtomicBool::new(false);
/// The Present calls whose frame logic waited for a later Present, because the device lock
/// was held at the end of the Present.
static PRESENT_LATE: AtomicU32 = AtomicU32::new(0);

/// The capture recorded a Present: MGE XE runs its logic of the frame at `after_present`.
pub fn present_due() {
    PRESENT_DUE.store(true, Ordering::Release);
}

/// Takes the due Present when this thread does not hold the device lock. With the lock held
/// the Present stays due, and the next call without the lock takes it.
fn take_present_due() -> bool {
    if crate::sync::device_lock_held_here() {
        return false;
    }
    PRESENT_DUE.swap(false, Ordering::AcqRel)
}

/// The end of a Present, on the main thread: MGE XE runs its logic of the frame, then the
/// capture runs a pending purge. Both stall the cell worker, and the worker takes the device
/// lock for its device calls, so both run only with the lock free (the rule of
/// `sync::device_lock`). The callers: `kind_call` after the Present's capture call, and the
/// device's `Present` after its lock is free. Under a lock that a caller further out holds,
/// the frame logic waits for the next Present.
pub unsafe fn after_present() {
    if !crate::shadow::on_main_thread() {
        return;
    }
    crate::map_tiles::poll();
    if !take_present_due() {
        if PRESENT_DUE.load(Ordering::Acquire) && PRESENT_LATE.fetch_add(1, Ordering::Relaxed) == 0 {
            crate::log!("link: the frame logic of MGE XE waits for the next Present: the device lock is held at the end of a Present");
        }
        return;
    }
    stream_present();
    crate::capture::run_pending_purge();
}

/// MGE XE runs its logic of a frame. Only through `after_present`, without the device lock.
unsafe fn stream_present() {
    let function = STREAM_PRESENT.load(Ordering::Relaxed);
    if function != 0 {
        let (width, height) = crate::own_device::size();
        core::mem::transmute::<usize, unsafe extern "C" fn(u32, u32)>(function)(width, height);
    }
}

/// Gives MGE XE a message of its extension in the host program.
pub unsafe fn stream_message(id: u32, bytes: &[u8]) {
    let function = STREAM_MESSAGE.load(Ordering::Relaxed);
    if function != 0 && crate::shadow::on_main_thread() {
        core::mem::transmute::<usize, unsafe extern "C" fn(u32, *const u8, u32)>(function)(id, bytes.as_ptr(), bytes.len() as u32);
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
/// buffers, textures, surfaces. The index is the place in [`OBJECT_CALL_NAMES`].
static OBJECT_CALLS: [core::sync::atomic::AtomicU32; 10] = [const { core::sync::atomic::AtomicU32::new(0) }; 10];
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
/// [`None`] when the caller must make the D3D8 call. Only the D3D8 calls are counted.
#[inline]
pub unsafe fn object_call(index: usize, args: &[u32]) -> Option<u32> {
    let result = kind_call(KIND_OBJECTS, CALL_ADD_REF + index as u32, args);
    if result.is_none() {
        OBJECT_CALLS[index].fetch_add(1, Ordering::Relaxed);
    }
    result
}

/// The kinds `KIND_*` that the host has: the answer of the last `use_kinds`.
static KINDS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
/// The frame and scene calls that went to the host and not to the device.
static FRAME_CALLS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Tells the host which kinds `KIND_*` it has. Returns the mask that it uses, or [`None`]
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

/// Makes a frame or scene call on the host. Returns [`None`] when the caller must make the
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

/// Makes a call of the target kind on the host. Returns [`None`] when the caller must make
/// the D3D8 call.
#[inline]
pub unsafe fn target_call(call: u32, args: &[u32]) -> Option<u32> {
    kind_call(KIND_TARGETS, call, args)
}

/// Makes a buffer on the host. Returns [`None`] when the caller must make the D3D8 call.
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
        FRAME_CALLS.fetch_add(1, Ordering::Relaxed);
        // This Present does not go through the device, where the frame timer of the rig is.
        #[cfg(feature = "rig")]
        if call == CALL_PRESENT {
            crate::perf::on_present();
        }
    }
    let result = (host.call)(call, args.as_ptr());
    crate::capture::call(call, args, result);
    if kind == KIND_FRAME && call == CALL_PRESENT {
        after_present();
    }
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

/// The vtable to give to a draw function of the Code Patch: `vtable`, the one of the device,
/// or a copy with the four entries above when the host draws.
pub unsafe fn draw_vtable(t: &MainThread, vtable: usize) -> usize {
    if !commands_for(FACT_DEVICE_BUFFERS) {
        return vtable;
    }
    let copy = &mut state(t).draw_vtable;
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
/// the device. Returns the result when the call went to the host, and [`None`] when it must
/// go to the device. A state call goes to the host at once, as a command: such code has no
/// state packet.
pub unsafe fn intercept(slot: usize, args: &[u32]) -> Option<u32> {
    use crate::d3d::*;
    if KINDS.load(Ordering::Relaxed) == 0 && SUPPRESS.load(Ordering::Relaxed) == 0 {
        return None;
    }
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
    let Some(t) = MainThread::here() else {
        crate::sync::off_thread("link", "stream source");
        return;
    };
    let mut s = state(&t);
    s.buffers[0] = buffer as u32;
    s.buffers[1] = stride;
    s.kept_buffers[0] = crate::own_host::Kept::of(buffer as u32);
}

#[inline]
pub unsafe fn note_vertex_shader(handle: u32) {
    let Some(t) = MainThread::here() else {
        crate::sync::off_thread("link", "vertex shader");
        return;
    };
    state(&t).buffers[2] = handle;
}

#[inline]
pub unsafe fn note_indices(buffer: usize, base_vertex: u32) {
    let Some(t) = MainThread::here() else {
        crate::sync::off_thread("link", "indices");
        return;
    };
    let mut s = state(&t);
    s.buffers[3] = buffer as u32;
    s.buffers[4] = base_vertex;
    s.kept_buffers[1] = crate::own_host::Kept::of(buffer as u32);
}

/// The number of render-state settings in the game's record (`NiDX8RenderState`, 0x1F8 to
/// 0x9F8). The pixel and vertex shader settings follow them.
const RENDER_STATES_IN_RECORD: u32 = 256;

/// Sends to the device the state that the game believes the device has, for the kinds in
/// `kinds`. The Rust code made no call for them while the host had them.
unsafe fn resync(t: &MainThread, kinds: u32) {
    let renderer = RENDERER.load(Ordering::Relaxed);
    if renderer == 0 {
        return;
    }
    // A copy of the kept state: the device calls below note their values into the state
    // again (`note_texture`, `note_transform`, `note_stream_source`, ...), so no guard of the
    // state may be live across them.
    let (textures_known, textures, transforms_known, texture_transforms, buffers, kept_buffers) = {
        let s = state(t);
        (s.textures_known, s.textures, s.transforms_known, s.texture_transforms, s.buffers, s.kept_buffers)
    };
    let mut textures_gone = 0u32;
    let (mut vertex_buffer_gone, mut index_buffer_gone) = (false, false);
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
    // The kept texture and buffer handles go to the device again only when the own host
    // still has the same objects alive: while the host took these kinds nothing held them,
    // and the game or the cell worker can have released them, and a later object can have
    // their address (`own_host::Kept`). A handle that is gone is forgotten; the game's next
    // draw sets what it needs.
    let live = |kept: crate::own_host::Kept| if crate::own_host::active() { kept.live() } else { 0 };
    if kinds >> FACT_DEVICE_TEXTURES & 1 != 0 {
        for stage in 0..STATE_STAGES {
            if textures_known >> stage & 1 != 0 {
                let texture = live(textures[stage as usize]);
                if texture != 0 {
                    crate::d3d::device_set_texture(device, stage, texture as usize);
                } else {
                    textures_gone |= 1 << stage;
                }
            }
        }
    }
    if kinds >> FACT_DEVICE_TRANSFORMS & 1 != 0 {
        // Not the world matrices: the game sends them again for each draw.
        for stage in 0..STATE_STAGES {
            if transforms_known >> (8 + stage) & 1 != 0 {
                crate::d3d::device_set_transform(device, D3DTS_TEXTURE0 + stage, texture_transforms[stage as usize].as_ptr() as usize);
            }
        }
    }
    if kinds >> FACT_DEVICE_LIGHTS & 1 != 0 {
        // The host put all lights off. The lights of the game go on again, the oldest first:
        // the list of the game has the newest first.
        let manager: usize = rd(renderer, R::lightManager);
        let mut entries = [0usize; MAX_STATE_LIGHTS];
        let mut count = 0;
        let _lock = crate::sync::engine_lock(manager + LM::criticalSection);
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
        if buffers[0] != 0 && live(kept_buffers[0]) == buffers[0] {
            crate::d3d::device_set_stream_source(device, 0, buffers[0] as usize, buffers[1]);
            crate::d3d::device_set_vertex_shader(device, rd(render_state, RS::currentVertexShader));
        } else {
            vertex_buffer_gone = true;
        }
        if buffers[3] != 0 && live(kept_buffers[1]) == buffers[3] {
            crate::d3d::device_set_indices(device, buffers[3] as usize, buffers[4]);
        } else {
            index_buffer_gone = true;
        }
    }
    // The handles that are gone are forgotten, after the device calls.
    let mut s = state(t);
    s.textures_known &= !textures_gone;
    if vertex_buffer_gone {
        s.buffers[0] = 0;
    }
    if index_buffer_gone {
        s.buffers[3] = 0;
    }
}

/// The number of `SetRenderState` and `SetTextureStageState` calls of the Rust code, for
/// each state. It shows which states the game changes during a frame.
static RENDER_STATE_SETS: [Count; 256] = [const { Count::new() }; 256];
static STAGE_STATE_SETS: [Count; 32] = [const { Count::new() }; 32];

#[inline]
pub fn note_render_state(state: u32) {
    if state < 256 {
        RENDER_STATE_SETS[state as usize].add();
    }
}

#[inline]
pub fn note_stage_state(state: u32) {
    if state < 32 {
        STAGE_STATE_SETS[state as usize].add();
    }
}

/// Keeps the texture of a stage that goes to the device.
#[inline]
pub unsafe fn note_texture(stage: u32, texture: usize) {
    if stage < STATE_STAGES {
        let Some(t) = MainThread::here() else {
            crate::sync::off_thread("link", "texture");
            return;
        };
        let mut s = state(&t);
        s.textures[stage as usize] = crate::own_host::Kept::of(texture as u32);
        s.textures_known |= 1 << stage;
    }
}
/// The texture stage states of a state packet, in the order of the contract: `D3DTSS_*`.
const STAGE_STATES: [u32; STATE_STAGE_STATES as usize] = [1, 2, 3, 4, 5, 6, 26, 27, 28, 11, 24, 7, 8, 9, 10, 22, 23];
/// A list item of the game: data, next, previous.
const ITEM_DATA: usize = crate::gen::layouts::TListItem::data;
const ITEM_NEXT: usize = crate::gen::layouts::TListItem::next;

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
    let sky_render = ptr_at(lake, TES3WaterNVLake::nvSkyRenderToTexture);
    ptr_at(sky_render, NVSkyRenderToTexture::camera)
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
        } else {
            // On wgpu no module splits the world camera's Click (MGE XE's patchCameraClick is
            // the Direct3D 9 proxy's), so the root's app-culled flag names no scene kind: a
            // module or a console command can set it in a world scene.
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
    // A `before` function of a port: the main thread (`shadow::run_before`).
    let t = &MainThread::claim();
    crate::ports::base::note_main_accumulator(t, args[0]);
    let camera = args[1] - AV_WORLD_TRANSLATION;
    send_scene(camera, crate::camera_space::scene_begin(t, args[1]));
    SCENE_SENT_FOR.store(camera, Ordering::Relaxed);
}

/// `BeginPaint(this, camera)`
unsafe fn begin_paint(args: &[usize]) {
    // A `before` function of a port: the main thread (`shadow::run_before`).
    let t = &MainThread::claim();
    // A caller other than `NiCamera::Click` did not go through `SetCameraData`.
    if SCENE_SENT_FOR.swap(0, Ordering::Relaxed) != args[1] {
        send_scene(args[1], crate::camera_space::scene_begin(t, 0));
    }
}

unsafe fn send_scene(camera: usize, origin: Option<[f64; 3]>) {
    let Some(host) = host() else { return };
    // The scene of a frame starts on the main thread: a call from another thread is a
    // fault of the game and is counted.
    let Some(t) = MainThread::here() else {
        crate::sync::off_thread("link", "scene");
        return;
    };
    let t = &t;
    let world_controller: usize = rd(WORLD_CONTROLLER, 0);
    let data_handler: usize = rd(DATA_HANDLER, 0);
    let water_controller = ptr_at(data_handler, DH_WATER_CONTROLLER);
    let weather = ptr_at(world_controller, WC_WEATHER);
    let shadow_manager = ptr_at(world_controller, WC_SHADOW_MANAGER);
    let masser = ptr_at(weather, W_MASSER);
    let secunda = ptr_at(weather, W_SECUNDA);

    WATER_PLANE_DISPLAYS.store(0, Ordering::Relaxed);
    let roots = &mut state(t).roots;
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
        flags: if origin.is_some() { SCENE_HAS_CAMERA_ORIGIN } else { 0 }
            | if in_interior() { SCENE_INTERIOR } else { 0 }
            | if sky_hidden(roots.sky_root) { SCENE_SKY_HIDDEN } else { 0 }
            | if renderer_on_back_buffer() { SCENE_BACK_BUFFER } else { 0 }
            | if game_wireframe() { SCENE_WIREFRAME } else { 0 },
        camera_origin: origin.unwrap_or_default(),
    };
    (host.scene_begin)(&scene);
    crate::capture::scene_begin(&scene);
    crate::instance_stream::scene_begin(t, roots.scene_kind, camera);
    // MGE XE's frame logic takes the same packet (the contract's `MGE_StreamSceneBegin`).
    let function = STREAM_SCENE_BEGIN.load(Ordering::Relaxed);
    if function != 0 {
        core::mem::transmute::<usize, unsafe extern "C" fn(*const SceneV1)>(function)(&scene);
    }
}

/// The device's target is the renderer's back buffer.
unsafe fn renderer_on_back_buffer() -> bool {
    let renderer = crate::engine::renderer();
    renderer != 0 && rd::<usize>(renderer, R::currentRenderTarget) == renderer + R::backbufferRenderTarget
}

/// Runs after `EndPaint`.
unsafe fn end_paint() {
    if let Some(host) = host() {
        (host.scene_end)();
        // An `after` function of a port: the main thread (`shadow::dispatch`).
        let t = &MainThread::claim();
        crate::instance_stream::scene_end(t);
        let flags = if crate::ports::base::main_queue_open(t) { SCENE_END_QUEUE_OPEN } else { 0 };
        crate::capture::scene_end(flags);
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
    // A `before` function of a port: the main thread (`shadow::run_before`).
    state(&MainThread::claim()).roots.render_target = args[1] as u32;
    if let Some(host) = host() {
        (host.frame_event)(EVENT_SET_TARGET, args[1] as u32, 0);
        crate::capture::frame_event(EVENT_SET_TARGET, args[1] as u32, 0);
    }
}

/// `SwapBuffers(this)`
unsafe fn swap_buffers(_args: &[usize]) {
    // A `before` function of a port: the main thread (`shadow::run_before`).
    let t = &MainThread::claim();
    crate::camera_space::frame_end(t);
    if let Some(host) = host() {
        (host.frame_event)(EVENT_SWAP, 0, 0);
        crate::capture::frame_event(EVENT_SWAP, 0, 0);
    }
}

/// A display of the game's water plane that the engine skips: a shape under the water
/// plane's root, displayed in the water scene, while the helper draws the water itself. A
/// draw of the sorted queue in that scene has another root. The first display of a scene
/// goes through, so that the helper's item for the water keeps its place; the others make
/// no record and no draw. The queue stays active in the water walk (`StartAccumulating`
/// 0x6F9170 takes the same camera again, 0x41C654). On the pixel shader path the water
/// plane has the no-sorter bit (0x51C597) and draws in place; on the plain path, with the
/// eye below the plane, it is a sorted blend (0x51C5EB). \[I\] This rule was not checked
/// against the second case.
pub unsafe fn water_plane_display_skipped(t: &MainThread, shape: usize) -> bool {
    let roots = &state(t).roots;
    if roots.scene_kind != SCENE_WATER || object_class(shape, roots) != CLASS_WATER_PLANE {
        return false;
    }
    WATER_PLANE_SEEN.fetch_add(1, Ordering::Relaxed);
    if WATER_PLANE_DISPLAYS.fetch_add(1, Ordering::Relaxed) == 0 || crate::capture::helper_takes() & (1 << CLASS_WATER_PLANE) == 0 {
        return false;
    }
    WATER_PLANE_SKIPPED.fetch_add(1, Ordering::Relaxed);
    true
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

/// The radius of a point light: the radius the engine passed to its attach pass when it placed
/// the light (`light_stream::radius_of`, from MWSE's event). Else the light's specular colour,
/// where `game_dynamicLightTest` (0x4D3039) writes the attach radius it was given (0x4D3150
/// and `game_getLightRadiusFromSpecular` 0x4D1B80 read it back): without a module that owns the
/// light attach radius it is the light's radius, but MGE XE makes it the end of its fade and
/// 160 more. A light that a mod made in Lua has the radius the mod set, 0 when it set none.
/// The light manager's device values carry the specular colour times the dimmer: they are not
/// the radius.
pub unsafe fn point_light_radius(light: usize) -> f32 {
    if let Some(t) = MainThread::here() {
        if let Some(radius) = crate::light_stream::radius_of(&t, rd(light, L::index)) {
            return radius;
        }
    }
    rd(light, L::specularColor_r)
}

/// Fills `LIGHTS` with the lights of an effect state that the light manager sends to the
/// device. Returns the number of lights.
unsafe fn collect_lights(t: &MainThread, effect_state: usize) -> usize {
    let lights = &mut state(t).lights;
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
            radius: if light_type == LIGHT_POINT { point_light_radius(light) } else { 0.0 },
        };
        count += 1;
    }
    count
}

unsafe fn send_draw(kind: u32, this: usize, geometry: usize, geometry_data: usize, skin: usize) {
    // A draw comes from the ports' Rust on the main thread; a call from another thread is
    // a fault of the game and is counted, and the draw is not sent.
    let Some(t) = MainThread::here() else {
        crate::sync::off_thread("link", "draw");
        return;
    };
    let t = &t;
    let Some(host) = host() else { return };
    let roots = state(t).roots;
    let roots = &roots;
    let mut flags = 0;
    if skin != 0 {
        flags |= DRAW_SKINNED;
    }
    // A draw of the flush of the Click's sorted queue. A draw with no accumulator outside that
    // flush is a draw in place (a sort adjust node in the modes off and sub-sort).
    let world_camera = matches!(roots.scene_kind, SCENE_WORLD | SCENE_SORTED_ALPHA | SCENE_WATER);
    if world_camera && crate::ports::base::in_main_flush(t) {
        flags |= DRAW_FROM_SORTED_QUEUE;
    }
    if kind == DRAW_POINTS {
        crate::shape_stream::note_points_scene(t, world_camera, roots.scene_kind, flags, geometry);
    }
    let property_state: usize = rd(this, R::currentPropertyState);
    let effect_state: usize = rd(this, R::currentEffectState);
    // A retained instance (`instance_stream`): the draw needs no notes and no object
    // record, its handles are known.
    crate::instance_stream::begin_draw(t);
    // A shape of the world, skinned or not: a draw of the world scene, or a blended shape
    // drawn from the sorted queue in a scene of the world camera. The table takes the
    // blends, and the first draw of a blended shape is the queue's. A particle system goes the
    // same way when the table takes particles (the default; `NIRENDER_PARTICLES=off` turns it off).
    let points = kind == DRAW_POINTS && crate::instance_stream::particle_table_on();
    let plain_world_shape = (matches!(kind, DRAW_SHAPE | DRAW_STRIPS) || points) && (roots.scene_kind == SCENE_WORLD || (world_camera && flags & DRAW_FROM_SORTED_QUEUE != 0)) && crate::capture::recording();
    if plain_world_shape {
        let mut class = CLASS_UNKNOWN;
        if let Some((geometry_handle, material_handle)) = crate::instance_stream::seen(t, geometry, kind, &mut class) {
            crate::shape_stream::note_handles(t, geometry_handle, material_handle, kind, class, geometry_data);
            return;
        }
    }
    let material = ptr_at(property_state, PS::propMaterial);
    let light_count = collect_lights(t, effect_state);
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
        lights: state(t).lights.as_ptr(),
    };
    if let Some(count) = CLASS_COUNTS.get(draw.object_class as usize) {
        count.add();
    }
    (host.draw)(&draw);
    // The record of a skinned shape has the bone influences from the skin data. When it takes
    // them from the skin partitions (see `geometry_stream::needs_partitions`), which the game
    // makes at the first draw of a skin, after this point, they are made here first.
    let skin_data: usize = if skin != 0 { rd(skin, crate::gen::layouts::NiSkinInstance::skinData) } else { 0 };
    if skin_data != 0 && geometry_data != 0 && crate::capture::recording() && crate::geometry_stream::needs_partitions(skin_data) {
        crate::ports::draw::ensure_skin_partitions(this, geometry_data, skin);
    }
    // A particle instance's geometry is the key of its particles (the serial of the data,
    // `geometry_stream::stream_handle`), as its shape record and its `INSTANCE_PARTICLES`
    // records name it.
    let geometry_handle = if !points {
        crate::geometry_stream::note(t, kind, geometry_data, skin, geometry)
    } else if geometry_data != 0 {
        crate::geometry_stream::stream_handle(t, geometry_data as u32, rd(geometry_data, crate::gen::layouts::NiGeometryData::uniqueID))
    } else {
        0
    };
    let material_handle = crate::material_stream::note(t, this, property_state, effect_state);
    crate::shape_stream::note_handles(t, geometry_handle, material_handle, kind, draw.object_class, geometry_data);
    crate::capture::object(&draw, geometry_handle, material_handle);
    crate::instance_stream::consider(t, geometry, plain_world_shape && (points || crate::instance_stream::class_retained(t, draw.object_class, skin != 0)), skin != 0, points, draw.object_class, geometry_handle, material_handle, property_state, effect_state);
    if (CLASS_SKY_DOME..=CLASS_SUN_GLARE).contains(&draw.object_class) && crate::instance_stream::sky_census_on() {
        crate::instance_stream::sky_census(t, "sky", geometry, draw.object_class, plain_world_shape, geometry_handle, material_handle, property_state, effect_state);
    }
    if draw.object_class == CLASS_WEATHER_PARTICLES && crate::instance_stream::weather_census_on() {
        crate::instance_stream::sky_census(t, "weather", geometry, draw.object_class, plain_world_shape, geometry_handle, material_handle, property_state, effect_state);
    }
}

/// Sends the state of the D3D8 draw call that follows. The draw ports call it. `renderer` is
/// the `NiDX8Renderer` and `geom_buffer` the `NiGeometryBufferData` of the draw.
///
/// The packet goes out only when the Rust code draws for real: not in the Rust run of a
/// comparison, where the device is dry and the game code makes the draw after it.
pub unsafe fn draw_state(
    t: &MainThread,
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
    let mut s = state(t);
    let buffers = s.buffers;
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
    let lights = &mut s.state_lights;
    let mut light_count = 0;
    let manager: usize = rd(renderer, R::lightManager);
    let _lock = crate::sync::engine_lock(manager + LM::criticalSection);
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
        lights: s.state_lights.as_ptr(),
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
        world_relative_mask: s.world_relative,
        world: s.worlds[0],
        blend_worlds: [s.worlds[1], s.worlds[2], s.worlds[3]],
        textures: s.textures.map(|kept| kept.handle),
        texture_transforms: s.texture_transforms,
    };
    if !skinned {
        core::ptr::copy_nonoverlapping((renderer + R::currentWorldMatrix) as *const f32, state.world.as_mut_ptr(), 16);
    }
    (host.draw_state)(&state);
}

/// The size of a state light.
pub const STATE_LIGHT_SIZE: usize = size_of::<StateLightV1>();

/// The lights of the light manager of the renderer as state lights, as `draw_state` reads
/// them: the bytes of the records.
pub unsafe fn state_lights<'a>(t: &'a MainThread, renderer: usize) -> &'a [u8] {
    let lights = &mut state(t).state_lights;
    let mut light_count = 0;
    let manager: usize = rd(renderer, R::lightManager);
    let _lock = crate::sync::engine_lock(manager + LM::criticalSection);
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
    core::slice::from_raw_parts(state(t).state_lights.as_ptr() as *const u8, light_count * STATE_LIGHT_SIZE)
}

/// True when the world matrix of the renderer is relative to the camera.
pub fn world_is_relative(t: &MainThread) -> bool {
    state(t).world_relative & 1 != 0
}

/// `RenderShape(this, geometryData, skinInstance, transform, bound)`
unsafe fn render_shape(args: &[usize]) {
    // A `before` function of a port: the main thread (`shadow::run_before`).
    let t = &MainThread::claim();
    crate::camera_space::geometry_begin(t, args[3]);
    send_draw(DRAW_SHAPE, args[0], args[3] - AV_WORLD_TRANSFORM, args[1], args[2]);
}

/// `RenderTriStrips(this, geometryData, skinInstance, transform, bound)`
unsafe fn render_tri_strips(args: &[usize]) {
    // A `before` function of a port: the main thread (`shadow::run_before`).
    let t = &MainThread::claim();
    crate::camera_space::geometry_begin(t, args[3]);
    send_draw(DRAW_STRIPS, args[0], args[3] - AV_WORLD_TRANSFORM, args[1], args[2]);
}

// `NiParticles::Display` (0x6DD3E0) and `NiLines::Display` (0x6DA850) give the renderer the
// address of their own world transform, as the shapes do: the object of the draw is at
// that address less the offset of the transform, and its class comes from its parents.

/// `RenderPoints(this, geometryData, transform, bound)`
unsafe fn render_points(args: &[usize]) {
    // A `before` function of a port: the main thread (`shadow::run_before`).
    let t = &MainThread::claim();
    crate::camera_space::geometry_begin(t, args[2]);
    send_draw(DRAW_POINTS, args[0], args[2] - AV_WORLD_TRANSFORM, args[1], 0);
}

/// `RenderLines(this, linesData, transform, bound)`
unsafe fn render_lines(args: &[usize]) {
    // A `before` function of a port: the main thread (`shadow::run_before`).
    let t = &MainThread::claim();
    crate::camera_space::geometry_begin(t, args[2]);
    send_draw(DRAW_LINES, args[0], args[2] - AV_WORLD_TRANSFORM, args[1], 0);
}

/// `RenderScreenPoly(this, ...)`
unsafe fn render_screen_poly(args: &[usize]) {
    // A `before` function of a port: the main thread (`shadow::run_before`).
    let t = &MainThread::claim();
    crate::camera_space::geometry_begin(t, 0);
    send_draw(DRAW_SCREEN_POLYGON, args[0], 0, 0, 0);
}

/// The `d3d8.dll` that is in the folder of the game. The Direct3D 8 of Windows has the same
/// file name and is also in the process, because MGE XE loads it.
pub(crate) unsafe fn host_module() -> usize {
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
    // The plugin is the host of the link when the device of the game is its own. With a
    // Direct3D device of MGE XE in the process there is no host.
    if !(crate::own_device::present() || own_d3d8()) {
        return Err("the game has a Direct3D device of MGE XE: no host of the render link".to_string());
    }
    let table: *const HostV1 = crate::own_host::table();
    let Some(host) = table.as_ref() else {
        return Err(format!("the host refused version {VERSION}"));
    };
    crate::map_tiles::install();
    crate::map_fog::install();
    crate::world_walk::install();
    crate::queue_flush::install();
    crate::world_lights::install();
    crate::pacing::install();
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
/// n. Returns the mask that the host uses, or [`None`] when the host cannot use facts.
pub unsafe fn use_facts(t: &MainThread, mask: u32) -> Option<u32> {
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
        resync(t, before & !used);
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

/// The counters of the host, or [`None`] without a host.
pub unsafe fn counters(reset: bool) -> Option<CountersV1> {
    let host = host()?;
    let mut out = CountersV1 { struct_size: size_of::<CountersV1>() as u32, ..Default::default() };
    (host.read_counters)(&mut out, reset as u32);
    Some(out)
}

/// The counters as text: one line for each fact that the host compared.
pub unsafe fn counters_text(t: &MainThread, reset: bool) -> String {
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
    text.push_str("classes:");
    for count in CLASS_COUNTS.iter() {
        text.push_str(&format!(" {}", count.get()));
        if reset {
            count.reset();
        }
    }
    text.push('\n');
    text.push_str(&format!("water plane displays: {} seen, {} skipped\n", WATER_PLANE_SEEN.load(Ordering::Relaxed), WATER_PLANE_SKIPPED.load(Ordering::Relaxed)));
    text.push_str("object_calls:");
    for (name, count) in OBJECT_CALL_NAMES.iter().zip(OBJECT_CALLS.iter()) {
        let value = if reset { count.swap(0, Ordering::Relaxed) } else { count.load(Ordering::Relaxed) };
        if value != 0 {
            text.push_str(&format!(" {name}={value}"));
        }
    }
    text.push(0xA as char);
    let frame_calls = if reset { FRAME_CALLS.swap(0, Ordering::Relaxed) } else { FRAME_CALLS.load(Ordering::Relaxed) };
    text.push_str(&format!("frame_calls_to_host: {frame_calls}"));
    text.push(0xA as char);
    let skipped: Vec<u32> = SKIPPED.iter().map(Count::get).collect();
    text.push_str(&format!(
        "calls_not_made: render_states {} stage_states {} textures {} buffers {} transforms {} lights {}",
        skipped[0], skipped[1], skipped[2], skipped[3], skipped[4], skipped[5]
    ));
    text.push(0xA as char);
    if reset {
        SKIPPED.iter().for_each(Count::reset);
    }
    // The states that the Rust code asked for, as `state=calls`.
    text.push_str(&crate::camera_space::report(t, reset));
    text.push(0xA as char);
    let sets: [(&str, &[Count]); 2] = [("render_states:", &RENDER_STATE_SETS), ("stage_states:", &STAGE_STATE_SETS)];
    for (label, sets) in sets {
        text.push_str(label);
        for (state, count) in sets.iter().enumerate() {
            let value = count.get();
            if value != 0 {
                text.push_str(&format!(" {state}={value}"));
            }
            if reset {
                count.reset();
            }
        }
        text.push(0xA as char);
    }
    text
}

#[cfg(test)]
mod present_tests {
    use super::*;

    /// The rule of [`sync::device_lock`]: MGE XE's logic of the frame stalls the cell worker,
    /// so a due Present is not taken while this thread holds the device lock.
    #[test]
    fn a_due_present_waits_while_the_device_lock_is_held() {
        present_due();
        {
            let _lock = crate::sync::device_lock();
            assert!(!take_present_due());
            let _nested = crate::sync::device_lock();
            assert!(!take_present_due());
        }
        assert!(take_present_due());
    }
}

#[cfg(test)]
mod resync_tests {
    use super::*;
    use core::sync::atomic::AtomicU32;

    /// The calls of `SetTransform` of the fake device, and the state of the last one.
    static SET_TRANSFORMS: AtomicU32 = AtomicU32::new(0);
    static LAST_TRANSFORM: AtomicU32 = AtomicU32::new(0);

    unsafe extern "system" fn set_transform(_device: usize, state: u32, _matrix: usize) -> u32 {
        SET_TRANSFORMS.fetch_add(1, Ordering::Relaxed);
        LAST_TRANSFORM.store(state, Ordering::Relaxed);
        0
    }

    // The host of the test: it takes the facts that it is asked for, and nothing else.
    unsafe extern "C" fn use_facts_of_host(mask: u32) -> u32 {
        mask
    }
    unsafe extern "C" fn scene_begin(_scene: *const SceneV1) {}
    unsafe extern "C" fn no_arguments() {}
    unsafe extern "C" fn frame_event(_kind: u32, _a: u32, _b: u32) {}
    unsafe extern "C" fn draw(_draw: *const DrawV1) {}
    unsafe extern "C" fn read_counters(_out: *mut CountersV1, _reset: u32) {}
    unsafe extern "C" fn draw_state(_state: *const DrawStateV1) {}
    unsafe extern "C" fn texture_create(_texture: *const TextureV1) -> u32 {
        0
    }
    unsafe extern "C" fn none() -> u32 {
        0
    }
    unsafe extern "C" fn camera_space(_origin: *mut f64) -> u32 {
        0
    }
    unsafe extern "C" fn command(_command: u32, _a: u32, _b: u32, _c: u32) {}
    unsafe extern "C" fn call(_call: u32, _args: *const u32) -> u32 {
        0
    }
    unsafe extern "C" fn use_kinds(_mask: u32) -> u32 {
        0
    }

    /// The fakes of the test in the statics of the link, and the test's thread as the main
    /// thread. The drop puts all back, also when an assert of the test fails: else the fakes
    /// stay for the later tests of the process, and a later thread with the same id would
    /// count as the main thread.
    struct Fakes {
        main_thread_before: u32,
    }

    impl Fakes {
        fn install(renderer: usize) -> Fakes {
            let main_thread_before = crate::shadow::main_thread_for_test();
            HOST.store(&HOST_OF_TEST as *const HostV1 as usize, Ordering::Relaxed);
            RENDERER.store(renderer, Ordering::Relaxed);
            Fakes { main_thread_before }
        }
    }

    impl Drop for Fakes {
        fn drop(&mut self) {
            HOST.store(0, Ordering::Relaxed);
            RENDERER.store(0, Ordering::Relaxed);
            SUPPRESS.store(0, Ordering::Relaxed);
            USED_FACTS.store(0, Ordering::Relaxed);
            crate::shadow::restore_main_thread(self.main_thread_before);
        }
    }

    static HOST_OF_TEST: HostV1 = HostV1 {
        struct_size: size_of::<HostV1>() as u32,
        version: 1,
        capabilities: CAP_USE_FACTS,
        reserved: 0,
        scene_begin,
        scene_end: no_arguments,
        frame_event,
        draw,
        read_counters,
        use_facts: use_facts_of_host,
        draw_state,
        texture_create,
        world_space: none,
        camera_space,
        command,
        call,
        use_kinds,
    };

    /// The path of 662621d: a mask that clears a device fact that the host had makes
    /// `use_facts` call `resync`, whose `device_set_transform` notes the transform into the
    /// link state (`note_transform`). With a guard of the state live across the device calls,
    /// that note opens the state a second time, and the refusal of a second open panics
    /// ("is opened again at"). The renderer and the device are fakes: the renderer holds
    /// the device at its `d3dDevice` field, and the device's vtable has `SetTransform` only.
    #[test]
    fn resync_makes_its_device_calls_with_no_guard_of_the_state() {
        // The fake device: its first word is its vtable, whose slot `DEVICE_SET_TRANSFORM` is
        // the fake's function. The other slots are 0: the test reaches no other call.
        let mut vtable = vec![0usize; crate::d3d::DEVICE_SET_TRANSFORM / size_of::<usize>() + 1];
        vtable[crate::d3d::DEVICE_SET_TRANSFORM / size_of::<usize>()] = set_transform as unsafe extern "system" fn(usize, u32, usize) -> u32 as usize;
        let device = Box::new(vtable.as_ptr() as usize);
        let device = &*device as *const usize as usize;
        // The fake renderer: zeros, with the device at its field. The render state is 0: the
        // transforms of `resync` do not read it.
        let mut renderer = vec![0u8; R::renderState + 64];
        // SAFETY: the field is inside the buffer, and the buffer lives for the test.
        unsafe { (renderer.as_mut_ptr().add(R::d3dDevice) as *mut usize).write_unaligned(device) };

        // This thread is the main thread of the test: the notes of the device calls open the
        // state only there. The guard comes after the fakes, so it drops before them.
        let _fakes = Fakes::install(renderer.as_ptr() as usize);
        let t = MainThread::for_test();
        let identity: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        // SAFETY: the matrix is 16 floats that live for the calls; the fakes above stand in
        // for the host, the renderer and the device for the whole test.
        unsafe {
            // The game set a texture transform: the state keeps it.
            note_transform(D3DTS_TEXTURE0, identity.as_ptr() as usize);
            // The host takes the transforms, then gives them back: `resync` sends the kept
            // texture transform to the device.
            assert_eq!(use_facts(&t, 1 << FACT_DEVICE_TRANSFORMS), Some(1 << FACT_DEVICE_TRANSFORMS));
            assert_eq!(use_facts(&t, 0), Some(0));
        }

        assert_eq!(SET_TRANSFORMS.load(Ordering::Relaxed), 1, "resync made one SetTransform for the kept texture transform");
        assert_eq!(LAST_TRANSFORM.load(Ordering::Relaxed), D3DTS_TEXTURE0);
        // The state is closed again: a later open does not panic.
        assert_eq!(state(&t).transforms_known & 0x100, 0x100);
    }
}
