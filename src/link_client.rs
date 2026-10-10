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
use crate::gen::layouts::{
    NiGeometryBufferData as GB,
    NiAVObject as AV, NiCamera as CAM, NiDX8Renderer as R, NiDynamicEffectState as DES,
    NiDynamicEffectState_InternalList as DL, NiLight as L, NiMaterialProperty as MP, NiPropertyState as PS,
};
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
static mut LIGHTS: [LightV1; MAX_LIGHTS] = [LightV1 { light: 0, device_index: 0, light_type: 0, radius: 0.0 }; MAX_LIGHTS];

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
    send_scene(camera);
    SCENE_SENT_FOR.store(camera, Ordering::Relaxed);
}

/// `BeginPaint(this, camera)`
unsafe fn begin_paint(args: &[usize]) {
    // A caller other than `NiCamera::Click` did not go through `SetCameraData`.
    if SCENE_SENT_FOR.swap(0, Ordering::Relaxed) != args[1] {
        send_scene(args[1]);
    }
}

unsafe fn send_scene(camera: usize) {
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
        reserved: 0,
    };
    (host.scene_begin)(&scene);
}

/// Runs after `EndPaint`.
unsafe fn end_paint() {
    if let Some(host) = host() {
        (host.scene_end)();
    }
}

/// `ClearBuffer(this, rect, flags)`
unsafe fn clear_buffer(args: &[usize]) {
    if let Some(host) = host() {
        (host.frame_event)(EVENT_CLEAR, args[2] as u32, 0);
    }
}

/// `SetRenderTarget(this, texture)`
unsafe fn set_render_target(args: &[usize]) {
    (*addr_of_mut!(ROOTS)).render_target = args[1] as u32;
    if let Some(host) = host() {
        (host.frame_event)(EVENT_SET_TARGET, args[1] as u32, 0);
    }
}

/// `SwapBuffers(this)`
unsafe fn swap_buffers(_args: &[usize]) {
    if let Some(host) = host() {
        (host.frame_event)(EVENT_SWAP, 0, 0);
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

/// Fills `LIGHTS` with the lights of an effect state that the light manager sends to the
/// device. Returns the number of lights.
unsafe fn collect_lights(effect_state: usize) -> usize {
    let lights = &mut *addr_of_mut!(LIGHTS);
    let mut count = 0;
    let mut iter = ptr_at(effect_state, DES::lights);
    while iter != 0 && count < MAX_LIGHTS {
        let light: usize = rd(iter, DL::effect);
        iter = rd(iter, DL::next);
        if light == 0 || rd::<u8>(light, L::enabled) == 0 || rd::<f32>(light, L::dimmer) < MIN_DIMMER {
            continue;
        }
        let light_type = vcall0(light, LIGHT_VTBL_GET_TYPE);
        lights[count] = LightV1 {
            light: light as u32,
            device_index: rd(light, L::index),
            light_type,
            // `game_dynamicLightTest` (0x4D2F40) keeps the radius of the light record in the
            // specular colour.
            radius: if light_type == LIGHT_POINT { rd(light, L::specularColor_r) } else { 0.0 },
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
}

/// Sends the state of the D3D8 draw call that follows. The draw ports call it. `renderer` is
/// the `NiDX8Renderer` and `geom_buffer` the `NiGeometryBufferData` of the draw.
///
/// The packet goes out only when the Rust code draws for real: not in the Rust run of a
/// comparison, where the device is dry and the game code makes the draw after it.
pub unsafe fn draw_state(
    renderer: usize,
    geom_buffer: usize,
    primitive_type: u32,
    vertex_count: u32,
    start_index: u32,
    primitive_count: u32,
    skinned: bool,
) {
    let Some(host) = host() else { return };
    if host.capabilities & CAP_DRAW_STATE == 0 || crate::shadow::leg() == crate::shadow::Leg::Rust {
        return;
    }
    let mut state = DrawStateV1 {
        struct_size: size_of::<DrawStateV1>() as u32,
        flags: if skinned { STATE_SKINNED } else { 0 },
        primitive_type,
        base_vertex_index: 0,
        min_index: 0,
        vertex_count,
        start_index,
        primitive_count,
        vertex_buffer: rd(geom_buffer, GB::d3dVertexBuffer),
        vertex_stride: rd(geom_buffer, GB::vertexStride),
        vertex_format: rd(geom_buffer, GB::fvf),
        index_buffer: rd(geom_buffer, GB::indexBuffer),
        world: [0.0; 16],
    };
    core::ptr::copy_nonoverlapping((renderer + R::currentWorldMatrix) as *const f32, state.world.as_mut_ptr(), 16);
    (host.draw_state)(&state);
}

/// `RenderShape(this, geometryData, skinInstance, transform, bound)`
unsafe fn render_shape(args: &[usize]) {
    send_draw(DRAW_SHAPE, args[0], args[3] - AV_WORLD_TRANSFORM, args[1], args[2]);
}

/// `RenderTriStrips(this, geometryData, skinInstance, transform, bound)`
unsafe fn render_tri_strips(args: &[usize]) {
    send_draw(DRAW_STRIPS, args[0], args[3] - AV_WORLD_TRANSFORM, args[1], args[2]);
}

// The three functions below send no geometry: it is not known that their callers pass the
// transform of the object itself.

/// `RenderPoints(this, geometryData, transform, bound)`
unsafe fn render_points(args: &[usize]) {
    send_draw(DRAW_POINTS, args[0], 0, args[1], 0);
}

/// `RenderLines(this, linesData, transform, bound)`
unsafe fn render_lines(args: &[usize]) {
    send_draw(DRAW_LINES, args[0], 0, args[1], 0);
}

/// `RenderScreenPoly(this, ...)`
unsafe fn render_screen_poly(args: &[usize]) {
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

/// Connects to the host and puts the packet functions on the ports. The ports must have
/// their detours. Returns what happened, as text.
pub unsafe fn connect() -> Result<String, String> {
    if HOST.load(Ordering::Relaxed) != 0 {
        return Ok("connected before".to_string());
    }
    let module = host_module();
    if module == 0 {
        return Err("no d3d8.dll of the game folder is in the process".to_string());
    }
    let export = win::GetProcAddress(module as *mut c_void, CONNECT_EXPORT.as_ptr());
    if export.is_null() {
        return Err("the d3d8.dll has no MGE_RenderLinkConnect".to_string());
    }
    let connect: ConnectFn = core::mem::transmute(export);
    let table = connect(VERSION);
    let Some(host) = table.as_ref() else {
        return Err(format!("the host refused version {VERSION}"));
    };
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
    Some((host.use_facts)(mask))
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
    text
}
