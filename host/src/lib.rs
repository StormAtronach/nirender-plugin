//! The crate `nirender_wgpu`: the renderer of the records of the nirender plugin, on wgpu.
//!
//! The helper of MGE XE G7 (numidium.exe) has this crate as a part: it reads the ring that
//! the plugin writes in the game process and draws each frame into the window of the game,
//! and adds its own draws (the distant land, the shadows, the water, the post shaders)
//! through the `Extension` trait. The same crate is the program `nirender-host`, which
//! draws a capture file without the game, or the ring in a window of its own:
//!
//! ```text
//! nirender-host <capture file> <picture.png> [--backend vulkan|dx12] [--frame N] [--anisotropy N]
//! nirender-host --live <name of the shared memory> [--backend vulkan|dx12] [--frames N] [--shot picture.png]
//!               [--embed] [--window-shots]
//! ```
//!
//! The files, by concern:
//! - `extension.rs`: the `Extension` trait and the types a program that draws more sees.
//! - `data.rs`: the data of a draw as the shader sees it, the counts, the constants.
//! - `renderer.rs`: the methods of `Renderer` (the device, the pipelines, the textures, the
//!   buffers); `stream.rs`: the state of the stream between the records; `player/`: the
//!   methods of `Player` (`feed`, `items`, `draws`, `frame`, `report`); `replay.rs`: the kept
//!   world draws drawn again for an extension; `retained.rs`: the table of the plugin's
//!   instances; `engine/`: what the engine does, as pure functions; `lights.rs`, `material.rs`,
//!   `pool.rs`, `d3d.rs`, `gpu.rs`, `ticks.rs`, `switches.rs`, `overlay.rs`.
//! - `cli.rs`: the arguments and the replay of a capture file; `live.rs`: the live mode (the
//!   ring, the window); `picture.rs`: the pictures and their files; `tag.rs`: the record tags.
//!
//! This file keeps the structs of the renderer and of the player, the pass state, the kept
//! instances, and the scheduling on the cores.
//!
//! The capture comes from `nirender.capture` of the plugin (`src/capture.rs` there has the
//! format). The counts that the program prints at the end show what it does not do.

use std::collections::HashMap;
use std::rc::Rc;

use rustc_hash::FxHashMap;

mod depth_merge;
mod live;
mod multisample;
mod picture;
use picture::*;
mod cli;
use cli::*;
mod tag;
mod extension;
pub use extension::*;
mod replay;
pub use replay::*;
mod data;
pub use data::*;
mod renderer;
use renderer::*;
mod stream;
use stream::*;
pub use cli::{run, run_with};

mod d3d;
use d3d::{address, attributes, blend_factor, color, compare, decode_dxt, expand_16_bits, expands, filter, gpu_bytes, known, stencil_operation, texture_format, INPUT_FORMATS};
pub use d3d::{d3d_compare, d3d_stencil_operation};
mod lights;
use lights::{light_buffer, storage_buffer, Clusters, ExtensionLights};
mod switches;
use switches::switches;
mod particle_quads;
mod engine;
mod player;
use engine::{plan, scene};
mod retained;
use retained::{light_data, light_index_buffer, retained_light_buffer, scene_buffer, PinnedTexture, RetainedSet, SceneData, INSTANCE_BLOCK, LIGHT_INDEXES_AT_FIRST, RETAINED_AT_FIRST, RETAINED_BONES_AT_FIRST, RETAINED_LIGHTS_AT_FIRST, SCENE_SIZE};

/// The wgpu of this crate, for a program that draws with it through `Extension`.
pub use wgpu;

/// The facts of the world scene of the game at a moment, for a call of the extension.
#[derive(Clone, Copy)]
struct WorldFacts {
    target_size: (u32, u32),
    view: [f32; 16],
    projection: [f32; 16],
    interior: bool,
    wireframe: bool,
    fog_color: [f32; 4],
    fog_start: f32,
    fog_end: f32,
    sun: Option<([f32; 3], [f32; 3], [f32; 3])>,
    ambient: [f32; 3],
    /// The exact position of the camera, when the scene of the world is relative to it.
    origin: Option<[f64; 3]>,
    /// The samples for each pixel of the targets of the frame (`Renderer::samples`).
    samples: u32,
}

/// The position of the camera of a view matrix of Direct3D: minus its translation times its
/// transposed rotation. Only for a stream without the camera's origin.
fn eye_of_view(view: &[f32; 16]) -> [f64; 3] {
    let v = view.map(f64::from);
    core::array::from_fn(|i| -(v[12] * v[i * 4] + v[13] * v[i * 4 + 1] + v[14] * v[i * 4 + 2]))
}

impl WorldFacts {
    fn view<'a>(&self, device: &'a wgpu::Device, queue: &'a wgpu::Queue, commands: &'a FrameCommands, target_format: wgpu::TextureFormat) -> SceneView<'a> {
        SceneView {
            device,
            queue,
            commands,
            target_format,
            depth_format: DEPTH_FORMAT,
            target_size: self.target_size,
            samples: self.samples,
            view: self.view,
            projection: self.projection,
            eye: self.origin.unwrap_or_else(|| eye_of_view(&self.view)),
            interior: self.interior,
            wireframe: self.wireframe,
            fog_color: self.fog_color,
            fog_start: self.fog_start,
            fog_end: self.fog_end,
            sun_direction: self.sun.map(|sun| sun.0),
            sun_color: self.sun.map_or([0.0; 3], |sun| sun.1),
            sun_ambient: self.sun.map_or([0.0; 3], |sun| sun.2),
            ambient: self.ambient,
            picture: None,
            depth: None,
            before_depth: None,
            lights: None,
        }
    }
}

/// The groups 0 to 2 (frame, samplers, textures) that a pass has now: a draw sets a group only
/// when it changes. Anything else that can set groups in the pass (a new pass, an extension, a
/// screen draw) forgets them. Group 3, the slot of the draw, has a dynamic offset and is set
/// for each draw.
#[derive(Default)]
struct BoundGroups([Option<Rc<wgpu::BindGroup>>; 3]);

impl BoundGroups {
    fn set(&mut self, pass: &mut wgpu::RenderPass<'static>, index: u32, group: &Rc<wgpu::BindGroup>) {
        let slot = &mut self.0[index as usize];
        if slot.as_ref().is_some_and(|bound| Rc::ptr_eq(bound, group)) {
            return;
        }
        let offsets: &[u32] = if index == 0 { &[0] } else { &[] };
        pass.set_bind_group(index, &**group, offsets);
        *slot = Some(group.clone());
    }

    fn forget(&mut self) {
        *self = Self::default();
    }

    /// Whether this group is the one bound at the index (`None`: no group is wanted).
    fn is(&self, index: u32, group: Option<&Rc<wgpu::BindGroup>>) -> bool {
        match (self.0[index as usize].as_ref(), group) {
            (Some(bound), Some(group)) => Rc::ptr_eq(bound, group),
            (None, None) => true,
            _ => false,
        }
    }
}

/// A draw with its vertices in the record, ready for the pass.
struct ScreenDraw {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    buffer: wgpu::Buffer,
    vertices: u32,
}

/// What a frame has, in its order. `feed` collects the items and `run_items` does them.
enum Item {
    /// The game draws into this rendered texture, or into the back buffer.
    Target(Option<u32>),
    Clear(Option<wgpu::Color>, Option<f32>, Option<u32>),
    Viewport(([f32; 4], [f32; 2])),
    Draw(Rc<Draw>),
    Screen(ScreenDraw),
    WorldStart(Box<WorldFacts>),
    WorldEnd(Box<WorldFacts>),
    /// The sorted blends of the retained table and the blends of the engine's path that
    /// waited for this item: the generation names the item within the frame.
    SortedBlends(u32),
    /// A run of places of the table's blends in the engine's walk (`engine::walk`): the
    /// number of the run in the frame.
    WalkPlaces(usize),
    /// The extension draws in place of the draws of this class. The number of world draws
    /// before this item: those after it are not in the picture yet.
    Replace(u32, Box<WorldFacts>, usize),
    /// The post-processing of the extension, after the world views.
    Post(Box<WorldFacts>),
    /// The depth of the first-person scene goes into the depth of the world for the
    /// post-processing, with the projection of the first person.
    FirstPersonDepth([f32; 16]),
    /// A held shape that the extension draws in its place in the order of the game
    /// (`Extension::draw_held_in_place`), with the facts of the world there.
    HeldHere(Box<HeldDraw>, Box<WorldFacts>),
}

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

/// A retained instance of the plugin (`tag::INSTANCE`): the helper draws it from this at
/// each `INSTANCE_DRAW` record, as it draws a shape record.
#[derive(Clone, Copy)]
struct Retained {
    class: u32,
    flags: u32,
    geometry: u32,
    material: u32,
    /// The rows 0 to 2 of the world matrix.
    rotation: [f32; 12],
    /// The exact translation, for a relative draw: the helper subtracts the origin of the
    /// scene. The stored one of the engine is the translation of a draw that is not.
    exact: [f64; 3],
    stored: [f32; 3],
    colours: [f32; 14],
    /// The world bound of the shape: the centre and the radius.
    bound: [f32; 4],
}

/// Bit 32 of `light_range[3]`: the draw may take the point lights of the world in the
/// cluster of its pixel (`MGE_XE_LIGHTS=world`, `world_lights_here` in the shader). A lit
/// draw of the world of the stream and a retained slot with lights have it.
pub(crate) const WORLD_LIGHTS_BIT: u32 = 32;

/// Bit 16 of `light_range[3]` for a material with flat shading (NiShadeProperty off): the
/// shader takes the lit colour and the normal of the first vertex of each triangle, as
/// Direct3D 8 does. The plugin sends the shade of a shape only with
/// `nirender.nifShading(true)` (off by default).
pub(crate) fn flat_shading_bit(derived: &material::Derived) -> u32 {
    if derived.shade == 1 {
        16
    } else {
        0
    }
}

/// The size of a light of a shape draw: its key, then the `D3DLIGHT8` of 26 words.
const STATE_LIGHT_SIZE: usize = 108;

/// The words of the stage of a texture effect in a shape record: the coordinate index with
/// its generation, the transform flags, the texture matrix.
const EFFECT_STAGE_WORDS: usize = 18;

/// One stage of a pass of a shape draw, as the texture pass of the game sets it.
#[derive(Clone, Copy)]
struct PassStage {
    /// Operation, argument 1, argument 2 (Direct3D values). The operation 1 (disable) ends
    /// the stages of the pass.
    color: [u32; 3],
    alpha: [u32; 3],
    /// The coordinate index with its generation in the high word, and the transform flags.
    coordinate: u32,
    transform: u32,
    /// The four values of the bump matrix, then the luminance scale and offset.
    bump: [f32; 6],
    texture: u32,
    /// Address U, address V, the magnify, minify and mip filters.
    sampler: [u32; 5],
    texture_transform: [f32; 16],
}

/// The state of one pass of a shape draw, from the derived state of its material: what the
/// pipeline of the game sets for the pass, and the shape is the same for each pass.
#[derive(Clone)]
struct Pass {
    /// Blend on, source, destination.
    blend: [u32; 3],
    stages: [PassStage; 8],
    /// The alpha test in place of that of the material (on, function, reference), and the
    /// colour channels that the draw writes (D3DCOLORWRITEENABLE: red 1, green 2, blue 4,
    /// alpha 8): for the draws of the Code Patch into the picture of the character.
    alpha_test: Option<[u32; 3]>,
    color_write: u32,
}

impl Pass {
    /// `effects` is the stages of the environment map and of the fog map from the shape
    /// record (the texture pass of the game makes them from the effect and the camera of
    /// the frame); without them a stage of an effect gets the generation of its kind and no
    /// transform.
    fn new(d: &material::Derived, pass: usize, m: &material::Material, textures: &FxHashMap<u32, Rc<wgpu::TextureView>>, effects: Option<&[[u32; EFFECT_STAGE_WORDS]; 2]>) -> Pass {
        const NONE: PassStage = PassStage { color: [1, 0, 0], alpha: [1, 0, 0], coordinate: 0, transform: 0, bump: [0.0; 6], texture: 0, sampler: [0; 5], texture_transform: [0.0; 16] };
        // D3DBLEND_ONE is 2 and D3DBLEND_ZERO is 1: the blend of a pass that does not blend.
        let blend = d.blends.get(pass).copied().unwrap_or([0; 3]);
        let blend = if blend[0] != 0 { [1, blend[1], blend[2]] } else { [0, 2, 1] };
        let mut stages = [NONE; 8];
        for (stage, st) in stages.iter_mut().zip(&d.passes[pass]) {
            let effect = effects
                .and_then(|e| match st.source {
                    8 => Some(&e[0]),
                    9 => Some(&e[1]),
                    _ => None,
                })
                .filter(|words| words[0] != 0);
            let (coordinate, transform, texture_transform) = match effect {
                Some(words) => (words[0], words[1], core::array::from_fn(|k| f32::from_bits(words[2 + k]))),
                None => {
                    // D3DTSS_TCI_CAMERASPACEREFLECTIONVECTOR for the environment map and
                    // D3DTSS_TCI_CAMERASPACEPOSITION for the fog map.
                    let generated = match st.source {
                        8 => 0x30000,
                        9 => 0x20000,
                        _ => 0,
                    };
                    (st.set | generated, 0, [0.0; 16])
                }
            };
            let (clamp, filter) = match st.source {
                8 => (m.environment.clamp, m.environment.filter),
                9 => (m.fog_map.clamp, m.fog_map.filter),
                10 => (3, 1),
                map => (m.maps[map as usize].clamp, m.maps[map as usize].filter),
            };
            let mut sampler = material::sampler_states(clamp, filter);
            // A texture with one level gets no mip filter. [I] The engine's filter for a
            // texture with one level was not read.
            if textures.get(&st.texture).is_some_and(|t| t.texture().mip_level_count() <= 1) {
                sampler[4] = 0;
            }
            *stage = PassStage {
                color: st.color,
                alpha: st.alpha,
                coordinate,
                transform,
                // The stage of the bump map has the values of the material.
                bump: if st.source == 5 { [m.bump[2], m.bump[3], m.bump[4], m.bump[5], m.bump[0], m.bump[1]] } else { [0.5, 0.0, 0.0, 0.5, 1.0, 0.0] },
                texture: st.texture,
                sampler,
                texture_transform,
            };
        }
        // A pass after the first has the alpha test of its template, not that of the material.
        let alpha_test = if pass > 0 { d.alpha_tests.get(pass).copied() } else { None };
        Pass { blend, stages, alpha_test, color_write: 15 }
    }

    /// The draws of the Morrowind Code Patch for this pass into the picture of the character
    /// of the inventory (`patchDrawIndexedPrimitive`; see `shape_stream::code_patch_picture`
    /// of the plugin). A pass that does not blend is drawn as it is. A pass that blends is
    /// drawn with the alpha of the texture: the alpha test "greater than 0" (D3DCMP_GREATER
    /// is 5), the blend SRCALPHA and INVSRCALPHA (5 and 6), and the alpha of stage 0 from
    /// its first argument (D3DTOP_SELECTARG1 is 2). Then, for a material with an alpha that
    /// is not 1, it is drawn into the alpha channel only, with the blend ONE and ZERO (2
    /// and 1) and the alpha of stage 0 modulated (D3DTOP_MODULATE is 4): the alpha of the
    /// picture gets the number of the equipment slot, which the game put into the material.
    fn code_patch_picture(&self, material_alpha: f32) -> (Pass, Option<Pass>) {
        if self.blend[0] == 0 {
            return (self.clone(), None);
        }
        let draw = |blend: [u32; 3], alpha_operation: u32, color_write: u32| {
            let mut pass = self.clone();
            pass.blend = blend;
            pass.stages[0].alpha[0] = alpha_operation;
            pass.alpha_test = Some([1, 5, 0]);
            pass.color_write = color_write;
            pass
        };
        (draw([1, 5, 6], 2, 15), (material_alpha != 1.0).then(|| draw([1, 2, 1], 4, 8)))
    }
}

pub mod gpu;
pub mod material;
mod overlay;
mod pool;
mod ticks;

/// The scheduling of this program on Windows. The thread of the frames goes to the cores of
/// the highest efficiency class, and the process out of the power throttling of a background
/// process. A
/// processor with one class of cores gets no change of its cores. The priority of the
/// process stays as it is. `MGE_XE_CORES=0` leaves the scheduler alone.
#[cfg(windows)]
mod cores {
    #[repr(C)]
    struct CpuSetInformation {
        size: u32,
        kind: u32,
        id: u32,
        group: u16,
        logical_processor_index: u8,
        core_index: u8,
        last_level_cache_index: u8,
        numa_node_index: u8,
        efficiency_class: u8,
        all_flags: u8,
        reserved: u32,
        allocation_tag: u64,
    }
    #[repr(C)]
    struct PowerThrottlingState {
        version: u32,
        control_mask: u32,
        state_mask: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn GetCurrentThread() -> isize;
        fn SetProcessInformation(process: isize, class: u32, information: *const core::ffi::c_void, size: u32) -> i32;
        fn GetSystemCpuSetInformation(information: *mut CpuSetInformation, length: u32, returned: *mut u32, process: isize, flags: u32) -> i32;
        fn SetThreadSelectedCpuSets(thread: isize, ids: *const u32, count: u32) -> i32;
    }
    const PROCESS_POWER_THROTTLING: u32 = 4;
    const PROCESS_POWER_THROTTLING_EXECUTION_SPEED: u32 = 1;

    /// Returns one line for the log with what was done.
    pub fn prefer_performance_cores() -> String {
        if crate::switches().cores_to_scheduler {
            return "cores: left to the scheduler (MGE_XE_CORES=0)".into();
        }
        // SAFETY: calls of the Windows API with valid arguments; the buffer is sized by the
        // first call.
        unsafe {
            let process = GetCurrentProcess();
            let state = PowerThrottlingState { version: 1, control_mask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED, state_mask: 0 };
            let throttling = SetProcessInformation(process, PROCESS_POWER_THROTTLING, &state as *const _ as *const core::ffi::c_void, core::mem::size_of::<PowerThrottlingState>() as u32) != 0;
            let mut needed = 0u32;
            GetSystemCpuSetInformation(core::ptr::null_mut(), 0, &mut needed, process, 0);
            let count = needed as usize / core::mem::size_of::<CpuSetInformation>();
            let mut sets: Vec<CpuSetInformation> = Vec::with_capacity(count.max(1));
            let got = GetSystemCpuSetInformation(sets.as_mut_ptr(), needed, &mut needed, process, 0) != 0;
            if got {
                sets.set_len(needed as usize / core::mem::size_of::<CpuSetInformation>());
            }
            let best = sets.iter().map(|s| s.efficiency_class).max().unwrap_or(0);
            let classes = sets.iter().map(|s| s.efficiency_class).min().unwrap_or(0) != best;
            let ids: Vec<u32> = sets.iter().filter(|s| s.efficiency_class == best).map(|s| s.id).collect();
            let pinned = classes && !ids.is_empty() && SetThreadSelectedCpuSets(GetCurrentThread(), ids.as_ptr(), ids.len() as u32) != 0;
            format!(
                "cores: {} of {} logical processors in the highest efficiency class{}; power throttling off {}",
                ids.len(),
                sets.len(),
                if pinned { ", the thread of the frames on them" } else if classes { ", not pinned" } else { ", one class, not pinned" },
                if throttling { "set" } else { "not set" }
            )
        }
    }
}
#[cfg(not(windows))]
mod cores {
    pub fn prefer_performance_cores() -> String {
        "cores: left to the scheduler".into()
    }
}

struct Renderer {
    device: Rc<wgpu::Device>,
    /// The format of the picture that the pipelines draw into.
    target_format: wgpu::TextureFormat,
    /// What the picture of the window can be used for.
    surface_usages: wgpu::TextureUsages,
    /// The anisotropic filter that MGE XE forces, for a comparison with its picture. 1 for the
    /// filters that the game asks for.
    anisotropy: u16,
    /// The samples for each pixel of the targets of the frame (`Extension::samples`, cut to
    /// what the adapter has), and the pipelines of the resolves with more than one.
    samples: u32,
    multisample: Option<multisample::Multisample>,
    /// The pipeline that merges the depth of the first person into the depth of the world,
    /// made at its first use.
    depth_merge: Option<depth_merge::DepthMerge>,
    /// The device can draw the edges of triangles only.
    wireframe: bool,
    queue: wgpu::Queue,
    shader: wgpu::ShaderModule,
    /// The records of the draws of the frame (`DrawData` each; `draws` in the shader): the
    /// buffer and its room, the data of the records made so far, how many of them are on
    /// the GPU, and the copies made while the frame draws (`record_now`), which take the
    /// room from its end, so that the records of the frame can grow between the parts of a
    /// frame.
    record_buffer: Rc<wgpu::Buffer>,
    record_capacity: usize,
    /// The most records the buffer may hold: the device's limit of a storage binding.
    record_limit: usize,
    record_staging: Vec<u8>,
    record_count: usize,
    record_written: usize,
    record_extras: usize,
    /// The instances of the frame on the GPU (`upload_instances`), and the copies made while
    /// the frame draws (`append_instances`), from the end of the room.
    instance_len: usize,
    instance_extras: usize,
    /// `MGE_XE_RUNS_DUMP=1`: a line for each draw of a pass, for a study of the runs.
    runs_dump: bool,
    /// The draws of a run (one pipeline, one stencil reference, the buffers of one chunk) as
    /// one indirect call. `MGE_XE_INDIRECT=0` draws them one by
    /// one. The buffer of the arguments of the frame's draws, its room (arguments), and how
    /// many arguments are on the GPU.
    indirect: bool,
    indirect_buffer: Rc<wgpu::Buffer>,
    indirect_capacity: usize,
    indirect_written: usize,
    /// The lights of all draws of the frame, and the number of them that the buffer holds.
    light_buffer: wgpu::Buffer,
    light_capacity: usize,
    /// The buffers of the retained instances (`retained.rs`): the light indexes, their
    /// lights, the scenes of the views; the values of the world's scene; the scene slots
    /// used in the frame. The segments of the retained records and instances at the start
    /// of the record and instance buffers, and the generation of each buffer (a new buffer
    /// gets the segment again).
    light_index_buffer: wgpu::Buffer,
    light_index_capacity: usize,
    retained_light_buffer: wgpu::Buffer,
    retained_light_capacity: usize,
    scene_buffer: wgpu::Buffer,
    scene_main: SceneData,
    scene_slots_used: u32,
    retained_records: usize,
    retained_instances: usize,
    record_generation: u32,
    instance_generation: u32,
    /// The textures and the samplers with a slot of the arrays that stays from frame to
    /// frame, for the records of the retained instances. A texture slot counts the stages of
    /// the instances that use it; at 0 it is free (handle 0, the white view) and a later
    /// texture takes it, so the room does not fill with the textures of instances that went.
    pinned_textures: Vec<PinnedTexture>,
    /// The free slots, from the highest to the lowest (`pin_texture` takes the lowest).
    pinned_free: Vec<usize>,
    /// The slot of each pinned texture's handle.
    pinned_index: FxHashMap<u32, usize>,
    /// The pinned entries at the start of the arrays of the frame in progress.
    pinned_in_frame: usize,
    pinned_samplers: Vec<([u32; 5], Rc<wgpu::Sampler>)>,
    /// The pinned samplers at the start of the sampler array of the frame in progress.
    pinned_samplers_in_frame: usize,
    bone_buffer: wgpu::Buffer,
    bone_capacity: usize,
    /// The bones of the skinned retained instances (`Renderer::retained_upload`), in matrices.
    retained_bone_buffer: wgpu::Buffer,
    retained_bone_capacity: usize,
    /// The lights that came as records, by key: a `D3DLIGHT8`.
    lights_known: HashMap<u32, [u8; 104]>,
    /// The GPU time stamps, when the adapter has them.
    timing: Option<Timing>,
    pipeline_layout: wgpu::PipelineLayout,
    pipelines: FxHashMap<PipelineKey, Rc<wgpu::RenderPipeline>>,
    textures: FxHashMap<u32, Rc<wgpu::TextureView>>,
    /// The textures that the game draws into, by the handle of the texture, and the texture of
    /// each surface that the game got from a texture.
    targets: FxHashMap<u32, RenderTexture>,
    surfaces: HashMap<u32, u32>,
    /// The textures with a block format that this program made the pixels of: their size is
    /// not a multiple of 4.
    decoded: std::collections::HashSet<u32>,
    /// The textures themselves, for a write of a level later.
    texture_objects: FxHashMap<u32, (wgpu::Texture, u32)>,
    white: Rc<wgpu::TextureView>,
    /// The resources of the extension for the frame: bind group 0 of each draw.
    frame_layout: wgpu::BindGroupLayout,
    frame_uniform: wgpu::Buffer,
    frame_sampler: wgpu::Sampler,
    frame_group: Rc<wgpu::BindGroup>,
    clusters: Clusters,
    extension_lights: ExtensionLights,
    samplers: FxHashMap<[u32; 5], Rc<wgpu::Sampler>>,
    /// The textures and the samplers of the frame in binding arrays (group 1): the slot of
    /// each texture handle and sampler state of the frame, the
    /// views and the samplers by slot (slot 0: white, a plain sampler), the group of the
    /// frame, and its variants for a pass into a rendered texture of the frame (that
    /// texture's slot shows white: a texture cannot be a target and a texture of one pass).
    array_layout: wgpu::BindGroupLayout,
    array_cap: usize,
    sampler_cap: usize,
    frame_texture_slots: FxHashMap<u32, u32>,
    frame_texture_views: Vec<Rc<wgpu::TextureView>>,
    frame_sampler_slots: FxHashMap<[u32; 5], u32>,
    frame_samplers: Vec<Rc<wgpu::Sampler>>,
    array_group: Option<Rc<wgpu::BindGroup>>,
    array_variants: FxHashMap<u32, Rc<wgpu::BindGroup>>,
    /// The geometry records, by the geometry data of the game; their vertices and indexes
    /// in pools (`pool`), and the ranges of records replaced in this frame, given back at
    /// the start of the next.
    geometries: FxHashMap<u32, Geometry>,
    vertex_pool: pool::Pool,
    index_pool: pool::Pool,
    retired: Vec<pool::Range>,
    retired_indexes: Vec<pool::Range>,
    /// The index buffer of the quads of the particle systems, as the game keeps one: the
    /// number of quads it has, and the buffer. It grows to the largest system seen.
    quad_indexes: Option<(u32, Rc<wgpu::Buffer>)>,
    /// The quads of the particle systems made on the GPU (`particle_quads`), made at the
    /// first system; none with `MGE_XE_PARTICLE_QUADS=cpu`.
    particle_quads: Option<particle_quads::ParticleQuads>,
    /// The material records by handle, and the states derived from them by (handle, texture
    /// sets of the geometry).
    materials: FxHashMap<u32, material::Material>,
    derived: FxHashMap<(u32, u32), Rc<material::Derived>>,
    defaults: wgpu::Buffer,
    /// The world matrices of the instances: the segment of the retained instances, then
    /// those of the frame (one for each draw, which names its record).
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    /// For a draw in screen coordinates: the pipeline for each sample count, the layout of
    /// its group, and its sampler.
    screen_pipelines: HashMap<u32, wgpu::RenderPipeline>,
    screen_layout: Option<wgpu::BindGroupLayout>,
    screen_sampler: Option<wgpu::Sampler>,
    counts: Counts,
    /// The command buffers of the extension for the next submit.
    commands: Rc<FrameCommands>,
}

/// A blend that waits for the flush of the engine's sorted queue (`Player::frame_sorted`).
enum Waiting {
    /// A blended draw of the engine's path.
    Draw(Rc<Draw>),
    /// The place of a sorted blend of the table in the engine's queue
    /// (`tag::INSTANCE_QUEUED`): the id of the instance.
    Table(u32),
}

/// Reads the records of a stream in their order and draws the frames.
struct Player {
    renderer: Renderer,
    stream: Stream,
    /// The retained instances of the plugin, by their id.
    instances: HashMap<u32, Retained>,
    /// The draws of the instances are drawn, and a shape record that is the twin of one is
    /// not. `MGE_XE_RETAINED=0` turns it round: for a capture of the dual mode of the plugin,
    /// the picture from the shape records.
    retained_on: bool,
    /// The instances the helper draws from its table (`retained.rs`), and the ambient
    /// light and the fog of the frame for their draws (`RETAINED_SCENE`).
    retained: RetainedSet,
    retained_values: Option<(u32, Option<[u32; 4]>)>,
    /// The table's lights as the extension shaded them for the main view of this frame, and
    /// what the shade gives the scene: made before the clusters of the lights of the world
    /// (`MGE_XE_LIGHTS=world`), then taken by `draw_retained_main`.
    table_shaded: Option<(Vec<LightData>, Option<TableShade>)>,
    /// The frame in progress: its commands, its pass, and its two targets.
    encoder: Option<wgpu::CommandEncoder>,
    pass: Option<wgpu::RenderPass<'static>>,
    color: Option<Rc<wgpu::TextureView>>,
    depth: Option<Rc<wgpu::TextureView>>,
    /// The texture of `color`, for a read of its pixels. It stays after the frame.
    back_buffer: Option<wgpu::Texture>,
    /// With more than one sample: the view of the back buffer, which the pass resolves its
    /// colour into, and the multisampled colour and depth targets that `color` and `depth`
    /// view. `None` with one sample. See `multisample.rs`.
    back_view: Option<Rc<wgpu::TextureView>>,
    ms_color: Option<wgpu::Texture>,
    ms_depth: Option<wgpu::Texture>,
    /// The samples for each pixel of the targets of the pass that is open, for the pipelines.
    pass_samples: u32,
    /// A part of the frame in progress went to the GPU before its end, so the back buffer
    /// has a picture that is not complete. `last_picture` then has the last complete one.
    sent_in_part: bool,
    last_picture: Option<wgpu::Texture>,
    /// Takes the answer to a pixel read: the number of the request, the width, the height
    /// and the pixels with 4 bytes each, blue first. `None` without a game to answer.
    reply: Option<Box<dyn FnMut(u32, u32, u32, &[u8])>>,
    /// Takes the pixels of a map tile read: the ticket, the width, the height and the
    /// pixels, blue first (`player/tiles.rs`). `None` without a game to answer: then a tile
    /// is copied and not read.
    tile_out: Option<Box<dyn FnMut(u32, u32, u32, &[u8])>>,
    /// The map tile reads that wait for the GPU, in their order.
    tile_reads: Vec<player::TileRead>,
    /// The pipelines that copy a map tile, by the format of the tile.
    tile_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    /// The picture files that the game asked for: the frame in progress goes into them.
    shots: Vec<String>,
    /// The pictures of the frame from before the interface of the game, for an extension:
    /// the buffer, the length of a row, the size, the file. Saved at the end of the frame.
    early_pictures: Vec<(wgpu::Buffer, usize, u32, u32, String)>,
    /// The last `OVERLAY` record of the stream, for the live mode to take: the debug overlay
    /// on or off, with the numbers of the game process.
    overlay_record: Option<Vec<u8>>,
    /// A picture file for a frame with a draw of the film player (`--shot-film`, the
    /// `--shot-film-frame`th such draw: a film starts from black): a film holds the main loop
    /// of the game, so the game cannot ask for it in time.
    film_shot: Option<String>,
    film_shot_at: u32,
    film_draws: u32,
    /// A program that draws more than the game does.
    extension: Option<Box<dyn Extension>>,
    /// The items of the frame that are not done yet, and the world draws of the frame for
    /// `Replay`.
    items: Vec<Item>,
    world_draws: Vec<Rc<Draw>>,
    /// The draws of the world scene that are not the sky and came before the sky (see
    /// `draw`): they go into the items after the start of the world.
    held_draws: Vec<Item>,
    /// The shapes of the frame that the extension took, for `Extension::draw_held`.
    taken: Vec<HeldDraw>,
    /// The lights of all draws of the frame, in the order of the draws.
    frame_lights: Vec<LightData>,
    /// The bones of the skinned draws of the frame, for the bone buffer.
    frame_bones: Vec<[f32; 16]>,
    /// The world matrices of the instances of the frame; the first is an identity, for
    /// the draws of their own.
    frame_instances: Vec<Instance>,
    /// The collect: the answers of the extension for the frame (`None` before the world
    /// scene), the plan of the items (`plan::Plan`: the world begun and ended, the scene in
    /// progress), the sun and the ambient light of the first draw of the world, the texture
    /// that the game draws into (`None` for the back buffer), and whether the game named a
    /// rendered texture as its target.
    parameters: Option<FrameParameters>,
    plan: plan::Plan,
    /// The blended world draws of the frame that the engine sorts, and the places of the
    /// table's blends in the engine's queue: they wait for the item `SortedBlends` and keep
    /// their order of arrival; the table's blends without a place go between them by depth
    /// (the centre of the bound, the order of arrival, the draw or the place, the generation).
    frame_sorted: Vec<([f32; 3], u32, Waiting, u32)>,
    /// The near plane of the world's view of the frame, in the space of the world: the
    /// depth of the sort of the blends.
    world_near: Option<[f32; 4]>,
    /// The classes that the extension took in this frame, one bit for each.
    replaced: u32,
    /// The facts of the world at its end in this frame: the camera of the world for the call
    /// of a class that no draw brought (`replace_missing_classes`).
    world_end_facts: Option<WorldFacts>,
    /// The classes of the game that the helper draws itself, one bit for each, for the
    /// plugin: the water plane while the extension draws the water.
    /// The live loop writes it into the head of the ring after each record.
    pub(crate) takes: u32,
    /// The reach of the extension's shadow map (`FrameParameters::shadow_reach`), for the
    /// plugin's animation policy, written into the head of the ring with `takes`.
    pub(crate) shadow_reach: f32,
    /// The plane of `FrameParameters::class_plane` when its call is in the items of the
    /// frame: before the call the world draws that blend and write no depth are drawn only
    /// on its far side, and after it (`split`) only on the side of the camera.
    plane_ahead: Option<[f64; 4]>,
    split: Option<[f64; 4]>,
    /// The post-processing of this frame is in the items.
    posted: bool,
    /// The view and the projection of the last world draw of the frame: the matrices of
    /// the world for a call after the world views, when the stream has those of the user
    /// interface.
    world_matrices: Option<([f32; 16], [f32; 16])>,
    world_sun: Option<([f32; 3], [f32; 3], [f32; 3])>,
    /// The sun of the frame from the `RETAINED_SCENE` record (flag 64): the 26 words of the
    /// weather's sky light as the engine gives them to the device (`world_facts`).
    retained_sun: Option<[u8; 104]>,
    /// The first uses of the frame's sun go to the log (`world_facts`).
    world_sun_logged: u32,
    world_ambient: [f32; 3],
    target: Option<u32>,
    wants_texture: bool,
    /// The pass: its target, its size, its viewport, and the picture after
    /// `Extension::before_world` for `Extension::blend` with true when it is of this frame.
    pass_target: Option<u32>,
    target_size: (u32, u32),
    viewport: Option<([f32; 4], [f32; 2])>,
    before: Option<wgpu::Texture>,
    before_kept: bool,
    /// The picture and the depth of the frame before a call of `Extension::draw_class`.
    picture_copy: Option<wgpu::Texture>,
    depth_copy: Option<wgpu::Texture>,
    /// The depth of the game at the end of its world scene, for `Extension::post`: the game
    /// clears its depth before its later scenes. The projection of the world at that time,
    /// and true when a draw of the first-person scene went into the back buffer in this
    /// frame (its depth goes into the copy at the end of that scene).
    world_depth: Option<wgpu::Texture>,
    world_depth_kept: bool,
    world_depth_projection: Option<[f32; 16]>,
    first_person_drawn: bool,
    /// The depth of the draws of `Extension::before_world`, for `Extension::post`: the
    /// renderer clears the depth after them.
    before_depth: Option<wgpu::Texture>,
    before_depth_kept: bool,
    /// The counts at the last mark. See `tag::MARK`.
    mark_times: [u64; TIME_COUNT],
    mark_frames: u32,
    mark_draws: u32,
    mark_runs: u32,
    mark_gpu: (f64, f64, u32),
    /// `gpu::work()` and `gpu::errors()` at the last mark.
    mark_work: [u64; 4],
    mark_errors: u64,
    /// The groups that the pass of the game has now.
    bound: BoundGroups,
    /// The state of the pass that `issue` set, so that a draw sets only what changes: the
    /// pipeline, the stencil reference, the vertices and the indexes of the last draw, and
    /// whether the shared vertex buffers (the defaults, the instances) are bound.
    pass_pipeline: Option<Rc<wgpu::RenderPipeline>>,
    pass_stencil: Option<u32>,
    pass_vertices: Option<Rc<wgpu::Buffer>>,
    pass_indexes: Option<Rc<wgpu::Buffer>>,
    pass_shared: bool,
    /// The arrays of the frame are bound in this pass.
    pass_arrays: bool,
    /// The arguments of the draws of the run that is open, and of all the runs of the frame
    /// (`flush_run`).
    run_args: Vec<IndirectArgs>,
    frame_args: Vec<IndirectArgs>,
    /// The size of the back buffer of the game, from the first record. 0 when not known.
    size: (u32, u32),
    /// The size of the picture of the frame.
    frame_size: (u32, u32),
}

#[cfg(test)]
mod tests {
    use super::VertexLayout;

    #[test]
    fn layout_of_a_format() {
        // D3DFVF_XYZ | D3DFVF_NORMAL | D3DFVF_DIFFUSE | D3DFVF_TEX2: 12 + 12 + 4 + 2 * 8 bytes.
        let layout = VertexLayout::of(0x2 | 0x10 | 0x40 | 0x200, 44).unwrap();
        assert_eq!(layout, VertexLayout { stride: 44, position: 0, normal: Some(12), color: Some(24), coordinates: vec![28, 36] });
        // D3DFVF_XYZ | D3DFVF_TEX1, with no normal and no colour.
        let layout = VertexLayout::of(0x2 | 0x100, 20).unwrap();
        assert_eq!(layout, VertexLayout { stride: 20, position: 0, normal: None, color: None, coordinates: vec![12] });
        // Transformed vertices (D3DFVF_XYZRHW) and blend weights (D3DFVF_XYZB4 with the
        // indexes) have no layout for a held draw.
        assert!(VertexLayout::of(0x4 | 0x40, 20).is_none());
        assert!(VertexLayout::of(0xC | 0x1000 | 0x10, 40).is_none());
    }

    #[test]
    fn the_reach_of_a_light() {
        use super::{LIGHT_REACH_MAX, LightData, light_reach};
        let light = |cut: f32, radius: f32, range: f32| LightData { direction: [0.0, 0.0, 0.0, range], cone: [cut, 0.0, radius, 0.0], ..bytemuck::Zeroable::zeroed() };
        // The cut-off of the light for each pixel, else the radius of the game, else the range.
        assert_eq!(light_reach(&light(1.0 / 512.0, 300.0, 1000.0)), 512.0);
        assert_eq!(light_reach(&light(0.0, 300.0, 1000.0)), 300.0);
        assert_eq!(light_reach(&light(0.0, 0.0, 1000.0)), 1000.0);
        // A range without an end ends at one cell.
        assert_eq!(light_reach(&light(0.0, 0.0, f32::MAX)), LIGHT_REACH_MAX);
        assert_eq!(light_reach(&light(0.0, 0.0, f32::NAN)), LIGHT_REACH_MAX);
    }

    /// A material with an alpha test and a decal: the base pass tests as the material says,
    /// the pass of the decal blends by its alpha with no test (its template, slot 1).
    #[test]
    fn a_later_pass_has_the_alpha_test_of_its_template() {
        use super::material::{derive, Material, ALPHA_TEST, BASE, DECAL0, DEPTH_TEST, WORDS};
        let mut words = [0u32; WORDS];
        words[1] = ALPHA_TEST | DEPTH_TEST;
        // "Greater than" (4) at 128, one decal.
        (words[4], words[5], words[7]) = (4, 128, 1);
        for (map, texture) in [(BASE, 1), (DECAL0, 2)] {
            words[8 + map * 5] = texture;
            words[12 + map * 5] = 1;
        }
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let d = derive(&Material::parse(&bytes).unwrap(), 1, None);
        assert_eq!(d.passes.len(), 2);
        assert_eq!(d.alpha_tests, vec![[1, 5, 128], [0, 0, 0]]);
        assert_eq!(d.blends[1], [1, 5, 6]);
    }
}
