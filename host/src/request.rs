//! The request of the graphics adapter and the device, with its reason when it fails; the
//! probe that asks the same without a window; and what a check of a shader without a GPU
//! takes from the request.
//!
//! The live start, a replay and `numidium.exe --probe` call one function, [`request`], so
//! that the probe cannot say yes where the live start says no. The needs are what the
//! renderer cannot do without: an adapter, the binding arrays of textures with their room,
//! the BC compression and the sampled textures of a stage ([`required_features`],
//! [`ARRAY_TEXTURES`], [`ARRAY_SAMPLERS`], [`SAMPLED_TEXTURES`]). The rest of the request
//! takes what the adapter has, with a fallback (the float depth, the samples, the line mode,
//! the indirect draws). A refusal names the backend, the adapter and the lack with its
//! number. The backends are tried in an order: with `auto`, Vulkan then Direct3D 12; a named
//! backend first, then the other, with a line. The exit code of a refusal is `EXIT_GPU` of
//! `stream_format.rs` in
//! the live mode and in the probe (the failure-path page of the project, section 4).

use crate::data::{AUTO_TEXTURE_SLOTS, DEPTH_FORMAT, MIN_TEXTURE_SLOTS, SAMPLER_CAP, STAGES};
use crate::stream_format::EXIT_GPU;
use crate::switches::switches;
use crate::{depth, gpu, retained, WindowTarget};

/// The sampled textures of one stage the renderer requires: a post shader of MGE XE reads up
/// to 22 (10 of its own, the picture, the depth, the waves, the outputs of its passes), and a
/// shader of a mod may read more. One fixed number, not the adapter's: an adapter with
/// fewer fails in the game whatever a check says.
pub const SAMPLED_TEXTURES: u32 = 32;
/// The room for textures in the binding arrays of a frame that the renderer requires.
pub const ARRAY_TEXTURES: u32 = MIN_TEXTURE_SLOTS;
/// The room for samplers in the binding arrays of a frame that the renderer requires.
pub const ARRAY_SAMPLERS: u32 = 8;

/// The features the request requires of every adapter: those of tier C, which has no
/// binding arrays. The optional ones of the request (the line mode, the timestamps, the
/// indirect draws, the float depth) are taken when the adapter has them; they map to no
/// capability of a shader.
pub fn required_features() -> wgpu::Features {
    required_features_of(TextureTier::C)
}

/// The features that the request requires for a tier: the arrays only for tier A.
pub fn required_features_of(tier: TextureTier) -> wgpu::Features {
    match tier {
        TextureTier::A => wgpu::Features::TEXTURE_COMPRESSION_BC | array_features(),
        TextureTier::C => wgpu::Features::TEXTURE_COMPRESSION_BC,
    }
}

/// The limits of the request that do not depend on the adapter: what a check of a shader
/// without a GPU compares. The request takes the sizes (the largest texture, the buffers,
/// the room of the arrays) from the adapter.
pub fn fixed_limits() -> wgpu::Limits {
    wgpu::Limits { max_sampled_textures_per_shader_stage: SAMPLED_TEXTURES, ..wgpu::Limits::default() }
}

/// How the world shader takes the textures of a draw (milestone 12, the texture binding tiers:
/// the design `notes/designs-2026-10/texture-tiers-design.md` of the Numidium project).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextureTier {
    /// The binding arrays of the frame: one group for the draws of a pass; a draw's record names
    /// the slot of each stage's texture and sampler. The adapter needs `array_features` and
    /// the room of [`ARRAY_TEXTURES`] and [`ARRAY_SAMPLERS`].
    A,
    /// No binding arrays: a group of the 6 textures and the 6 samplers of the stages for each
    /// set (`Renderer::stage_group`), bound before each draw or call of the table that takes
    /// another set. For an adapter without the arrays.
    C,
}

impl TextureTier {
    pub fn name(self) -> &'static str {
        match self {
            TextureTier::A => "a",
            TextureTier::C => "c",
        }
    }
}

/// The world shader of tier C: the source of tier A with the two binding arrays of group 1
/// replaced by 6 textures (`t0` to `t5`, bindings 0 to 5) and 6 samplers (`s0` to `s5`,
/// bindings 6 to 11), and each stage sampling its own pair. Every stage of a draw samples
/// its pair: an unused stage has the white texture and a plain sampler in its group.
pub fn tier_c_shader_source(source: &str) -> String {
    let mut out = source.replacen("enable wgpu_binding_array;\n", "", 1);
    let arrays = "@group(1) @binding(0) var textures: binding_array<texture_2d<f32>>;\n@group(1) @binding(1) var samplers: binding_array<sampler>;\n";
    assert!(out.contains(arrays), "the binding arrays of group 1 are not in the world shader");
    let mut fixed = String::new();
    for stage in 0..STAGES {
        fixed += &format!("@group(1) @binding({stage}) var t{stage}: texture_2d<f32>;\n");
    }
    for stage in 0..STAGES {
        fixed += &format!("@group(1) @binding({}) var s{stage}: sampler;\n", STAGES + stage);
    }
    out = out.replacen(arrays, &fixed, 1);
    for stage in 0..STAGES {
        let indexed = format!("textures[draws[record].stages[{stage}].slots.x], samplers[draws[record].stages[{stage}].slots.y]");
        assert_eq!(out.matches(&indexed).count(), 1, "stage {stage} does not sample the arrays once");
        out = out.replacen(&indexed, &format!("t{stage}, s{stage}"), 1);
    }
    assert!(!out.contains("textures[") && !out.contains("samplers["), "the world shader reads the arrays in another place");
    out
}

/// The source of the world shader with the text of an extension (its sky and draw parts):
/// `shader.wgsl`, the depth functions after its `enable` directives, then the extension's
/// text with the record of the draw read as `draws[record]` and sent through the functions
/// of the shader.
pub fn world_shader_source(extension_text: &str) -> String {
    let extension = retained::rewrite_extension_text(&extension_text.replace("draw.", "draws[record]."));
    format!("{}\n{}\n{extension}", include_str!("shader.wgsl"), depth::wgsl())
}

fn array_features() -> wgpu::Features {
    wgpu::Features::TEXTURE_BINDING_ARRAY | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING | wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY
}

/// A backend, an adapter and a device that have what the renderer needs, with the choices
/// made from what the adapter has.
pub(crate) struct Request {
    pub adapter: wgpu::Adapter,
    pub info: wgpu::AdapterInfo,
    pub surface: Option<wgpu::Surface<'static>>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// The format of the target: the wanted one, or a format of the surface without sRGB.
    pub target_format: wgpu::TextureFormat,
    pub surface_usages: wgpu::TextureUsages,
    pub present_modes: Vec<wgpu::PresentMode>,
    pub depth_format: wgpu::TextureFormat,
    /// The samples of each pixel of the frame: the largest count up to the wanted one that
    /// the adapter has for the colour and the depth format.
    pub samples: u32,
    /// The room of the texture array and of the sampler array of a frame.
    pub array_cap: u32,
    pub sampler_cap: u32,
    /// How the world shader takes the textures ([`TextureTier`]).
    pub tier: TextureTier,
}

impl Request {
    /// The lines of the log about the request, as the live start prints them.
    pub(crate) fn report(&self) {
        println!("adapter: {}", adapter_line(&self.info));
        if self.surface.is_some() {
            println!("window: the present modes of the surface are {:?}", self.present_modes);
        }
        println!(
            "depth: {:?}, {} (the adapter {} Depth32FloatStencil8{}{})",
            self.depth_format,
            if depth::reversed() { "reversed" } else { "plain" },
            if self.adapter.features().contains(wgpu::Features::DEPTH32FLOAT_STENCIL8) { "has" } else { "lacks" },
            if switches().depth24 { "; MGE_XE_DEPTH_FORMAT=24" } else { "" },
            if switches().depth_plain { "; MGE_XE_DEPTH_REVERSED=0" } else { "" }
        );
        println!("multisampling: {} samples for each pixel of the frame", self.samples);
        match self.tier {
            TextureTier::A => println!("texture tier: a (binding arrays: room for {} textures and {} samplers)", self.array_cap, self.sampler_cap),
            TextureTier::C => println!("texture tier: c (a group of {STAGES} textures and {STAGES} samplers for each set; no binding arrays)"),
        }
    }
}

/// Why a backend cannot serve: the adapter when there is one, and the lack with its number.
#[derive(Debug, Clone)]
pub struct Refusal {
    pub backend: wgpu::Backends,
    pub adapter: Option<String>,
    pub reason: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}: {}: {}", backend_name(self.backend), self.adapter.as_deref().unwrap_or("no adapter"), self.reason)
    }
}

/// The backends in the order they are tried: a named one first, then the other; `auto`
/// (the primary backends) is Vulkan, then Direct3D 12.
fn order(backends: wgpu::Backends) -> Vec<wgpu::Backends> {
    if backends == wgpu::Backends::DX12 {
        vec![wgpu::Backends::DX12, wgpu::Backends::VULKAN]
    } else if backends == wgpu::Backends::VULKAN || backends.contains(wgpu::Backends::VULKAN | wgpu::Backends::DX12) {
        vec![wgpu::Backends::VULKAN, wgpu::Backends::DX12]
    } else {
        vec![backends]
    }
}

fn backend_name(backend: wgpu::Backends) -> &'static str {
    match backend {
        wgpu::Backends::VULKAN => "vulkan",
        wgpu::Backends::DX12 => "dx12",
        _ => "other",
    }
}

fn adapter_line(info: &wgpu::AdapterInfo) -> String {
    format!("{} ({:?}, driver {} {})", info.name, info.backend, info.driver, info.driver_info)
}

/// Asks the backends in their order for an adapter and a device with what the renderer
/// needs; the first that serves is the answer. With a window, the surface is made on it in
/// the same try, since a surface belongs to the instance of its backend. When no backend
/// serves, every refusal comes back in the order of the tries.
pub(crate) fn request(backends: wgpu::Backends, window: Option<WindowTarget>, wanted_format: wgpu::TextureFormat, texture_slots: u32, samples: u32) -> Result<Request, Vec<Refusal>> {
    let mut refusals = Vec::new();
    for backend in order(backends) {
        match try_backend(backend, window.as_ref(), wanted_format, texture_slots, samples) {
            Ok(request) => {
                for refusal in &refusals {
                    println!("gpu: {refusal}; {} serves instead", backend_name(backend));
                }
                return Ok(request);
            }
            Err(refusal) => refusals.push(refusal),
        }
    }
    Err(refusals)
}

/// One backend: the instance, the surface of the window, the adapter, the needs, the device.
fn try_backend(backend: wgpu::Backends, window: Option<&WindowTarget>, wanted_format: wgpu::TextureFormat, texture_slots: u32, samples: u32) -> Result<Request, Refusal> {
    let refuse = |adapter: Option<String>, reason: String| Refusal { backend, adapter, reason };
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = backend;
    // The WGPU_* variables of the environment can change the flags, for a test (for
    // example WGPU_VALIDATION_INDIRECT_CALL=0).
    // The arguments of the indirect draws are this program's own: no validation of them
    // by a compute pass (`WGPU_VALIDATION_INDIRECT_CALL=1` puts it back).
    descriptor.flags.remove(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL);
    descriptor.flags = descriptor.flags.with_env();
    // The new shader compiler of Direct3D, a part of this program: the old one is slow.
    descriptor.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::StaticDxc;
    let instance = wgpu::Instance::new(descriptor);

    let surface = match window {
        None => None,
        Some(WindowTarget::Own(window)) => Some(instance.create_surface(window.clone()).map_err(|error| refuse(None, format!("no surface on the window: {error}")))?),
        // The game's window lives until the game ends, and this program ends with it.
        Some(WindowTarget::Game(handle)) => {
            use winit::raw_window_handle::{RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle};
            extern "system" {
                fn GetWindowLongPtrW(window: isize, index: i32) -> isize;
            }
            // Vulkan wants the module of the window (GWLP_HINSTANCE).
            let mut window = Win32WindowHandle::new(*handle);
            // SAFETY: the handle is the game's window, which outlives this program; the
            // display handle of Windows needs no object.
            let surface = unsafe {
                window.hinstance = std::num::NonZeroIsize::new(GetWindowLongPtrW(handle.get(), -6));
                instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(RawDisplayHandle::Windows(WindowsDisplayHandle::new())),
                    raw_window_handle: RawWindowHandle::Win32(window),
                })
            };
            Some(surface.map_err(|error| refuse(None, format!("no surface on the window of the game: {error}")))?)
        }
    };

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: surface.as_ref(),
        force_fallback_adapter: false,
        ..Default::default()
    }))
    .map_err(|error| refuse(None, format!("no graphics adapter: {error}")))?;
    let info = adapter.get_info();
    let name = Some(adapter_line(&info));

    // The needs, each with its number.
    let features = adapter.features();
    let limits = adapter.limits();
    let mut lacks = Vec::new();
    // The arrays' lacks: a refusal for tier A forced; else they give tier C.
    let mut array_lacks = Vec::new();
    if !features.contains(array_features()) {
        let missing: Vec<&str> = [
            (wgpu::Features::TEXTURE_BINDING_ARRAY, "TEXTURE_BINDING_ARRAY"),
            (wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING, "SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING"),
            (wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY, "PARTIALLY_BOUND_BINDING_ARRAY"),
        ]
        .iter()
        .filter(|(feature, _)| !features.contains(*feature))
        .map(|(_, name)| *name)
        .collect();
        array_lacks.push(format!("the binding arrays of textures ({})", missing.join(", ")));
    }
    // The room of the texture arrays: the setting of the extension (0 automatic), the test
    // switch MGE_XE_ARRAY_CAP over it, at most the adapter's room. The samplers of the
    // array count in the elements of the arrays too.
    let wanted = switches().array_cap.unwrap_or(if texture_slots == 0 { AUTO_TEXTURE_SLOTS } else { texture_slots.max(MIN_TEXTURE_SLOTS) });
    let array_cap = limits.max_binding_array_elements_per_shader_stage.saturating_sub(SAMPLER_CAP).min(wanted);
    let sampler_cap = limits.max_binding_array_sampler_elements_per_shader_stage.min(SAMPLER_CAP);
    if array_cap < ARRAY_TEXTURES || sampler_cap < ARRAY_SAMPLERS {
        array_lacks.push(format!("room in the binding arrays: has {array_cap} textures and {sampler_cap} samplers, needs {ARRAY_TEXTURES} and {ARRAY_SAMPLERS}"));
    }
    // `MGE_XE_TEXTURE_TIER=a|c` forces a tier; else tier A when the adapter has the arrays.
    let tier = match switches().texture_tier {
        Some(TextureTier::A) => {
            lacks.append(&mut array_lacks);
            TextureTier::A
        }
        Some(TextureTier::C) => TextureTier::C,
        None if array_lacks.is_empty() => TextureTier::A,
        None => TextureTier::C,
    };
    let (array_cap, sampler_cap) = if tier == TextureTier::A { (array_cap, sampler_cap) } else { (0, 0) };
    if !features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC) {
        lacks.push("the BC texture compression (TEXTURE_COMPRESSION_BC)".to_string());
    }
    if limits.max_sampled_textures_per_shader_stage < SAMPLED_TEXTURES {
        lacks.push(format!("sampled textures in one stage: has {}, needs {SAMPLED_TEXTURES}", limits.max_sampled_textures_per_shader_stage));
    }
    if !lacks.is_empty() {
        return Err(refuse(name, format!("lacks {}", lacks.join("; "))));
    }

    // The choices from what the adapter has.
    let mut target_format = wanted_format;
    let mut surface_usages = wgpu::TextureUsages::empty();
    let mut present_modes = Vec::new();
    if let Some(surface) = surface.as_ref() {
        // The game gives colours that are ready for the screen: no sRGB format.
        let capabilities = surface.get_capabilities(&adapter);
        if !capabilities.formats.contains(&target_format) {
            target_format = capabilities.formats.iter().copied().find(|format| !format.is_srgb()).unwrap_or(capabilities.formats[0]);
        }
        surface_usages = capabilities.usages;
        present_modes = capabilities.present_modes;
    }
    // The format of the depth: a float depth with a stencil when the adapter has it. A
    // float depth is reversed: near 1, far 0 (`depth`). A fixed-point depth gains nothing
    // from it.
    let float_depth = features.contains(wgpu::Features::DEPTH32FLOAT_STENCIL8) && !switches().depth24;
    let depth_format = if float_depth { wgpu::TextureFormat::Depth32FloatStencil8 } else { DEPTH_FORMAT };
    depth::set_reversed(float_depth && !switches().depth_plain);
    depth::set_one_buffer(switches().one_depth);
    if switches().one_depth && !depth::one_buffer() {
        println!("depth: MGE_XE_ONE_DEPTH needs the reversed float depth; the land keeps its own depth");
    }
    let mut samples = samples.clamp(1, 8).next_power_of_two();
    while samples > 1 {
        let color = adapter.get_texture_format_features(target_format).flags;
        let depth = adapter.get_texture_format_features(depth_format).flags;
        if color.sample_count_supported(samples) && color.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE) && depth.sample_count_supported(samples) {
            break;
        }
        samples /= 2;
    }

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("nirender"),
        // The draws of an extension from the GPU (the distant statics of MGE XE) need the
        // first instance and the count of indirect draws, when the adapter has them.
        required_features: required_features_of(tier)
            | (features
                & (wgpu::Features::POLYGON_MODE_LINE
                    | wgpu::Features::TIMESTAMP_QUERY
                    | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
                    | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES
                    | wgpu::Features::INDIRECT_FIRST_INSTANCE
                    | wgpu::Features::MULTI_DRAW_INDIRECT_COUNT))
            | if float_depth { wgpu::Features::DEPTH32FLOAT_STENCIL8 } else { wgpu::Features::empty() },
        // The largest texture that the adapter takes, up to 16384: an atlas of the distant
        // land of MGE XE has that size. One buffer for the geometry of many meshes of an
        // extension (the pool of the distant statics of MGE XE). The records of the draws
        // are one storage binding: the adapter's room, not the default of 128 MiB
        // (`record_limit` guards it).
        required_limits: wgpu::Limits {
            max_texture_dimension_2d: limits.max_texture_dimension_2d.min(16384),
            max_buffer_size: limits.max_buffer_size,
            max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
            // Tier C asks for no room in binding arrays.
            max_binding_array_elements_per_shader_stage: array_cap + sampler_cap,
            max_binding_array_sampler_elements_per_shader_stage: sampler_cap,
            ..fixed_limits()
        },
        memory_hints: wgpu::MemoryHints::default(),
        ..Default::default()
    }))
    .map_err(|error| refuse(name.clone(), format!("no device: {error}")))?;

    Ok(Request { adapter, info, surface, device, queue, target_format, surface_usages, present_modes, depth_format, samples, array_cap, sampler_cap, tier })
}

/// `numidium.exe --probe`: asks the backends in their order, without a window, up to the
/// first that serves, as the live start does, and prints one line for each asked, the
/// answer, and the time it took. The last line before the exit is `probe: use <backend>`
/// or `probe: none: <the refusals>`; the exit code is 0 or `EXIT_GPU`. The lines go to
/// the standard output, which d3d8.dll reads through a pipe. One backend takes about 0.3 s
/// on the owner's machine, both about 0.6 s: the first that serves ends the probe.
pub fn probe(backends: wgpu::Backends, texture_slots: u32) -> i32 {
    use std::io::Write;
    let started = std::time::Instant::now();
    let mut usable = None;
    let mut refusals = Vec::new();
    for backend in order(backends) {
        match try_backend(backend, None, wgpu::TextureFormat::Bgra8Unorm, texture_slots, 1) {
            Ok(request) => {
                println!(
                    "probe: {}: {}: ok; texture tier: {}; binding arrays {} textures and {} samplers (tier a needs {ARRAY_TEXTURES} and {ARRAY_SAMPLERS}), BC yes, sampled textures {} (needs {SAMPLED_TEXTURES}), depth {:?}",
                    backend_name(backend),
                    adapter_line(&request.info),
                    request.tier.name(),
                    request.array_cap,
                    request.sampler_cap,
                    request.adapter.limits().max_sampled_textures_per_shader_stage,
                    request.depth_format
                );
                usable = Some(backend);
                break;
            }
            Err(refusal) => {
                println!("probe: {refusal}");
                refusals.push(refusal);
            }
        }
    }
    let code = match usable {
        Some(backend) => {
            println!("probe: use {}", backend_name(backend));
            0
        }
        None => {
            println!("probe: none: {}", refusals.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "));
            EXIT_GPU as i32
        }
    };
    println!("probe: {:.2} s", started.elapsed().as_secs_f64());
    let _ = std::io::stdout().flush();
    code
}

/// The box of the live start when no backend serves (the failure-path page, section 5, the
/// code 20), and the log lines of the refusals; then the program ends with the code.
pub(crate) fn refuse_live(refusals: &[Refusal]) -> ! {
    for refusal in refusals {
        println!("gpu: {refusal}");
    }
    let why: Vec<String> = refusals.iter().map(|refusal| format!("{} ({}) {}.", refusal.adapter.as_deref().unwrap_or("No adapter"), backend_name(refusal.backend), refusal.reason)).collect();
    let text = format!(
        "The wgpu renderer of MGE XE cannot use this graphics device.\n{}\nClose this box: the game closes. Set [runtime] renderer_wgpu = false in mgeXE.toml for Direct3D 9.\nnirender-host.log in the folder of the game has the details.",
        why.join("\n")
    );
    gpu::refuse(crate::stream_format::EXIT_GPU, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backends_are_tried_in_their_order() {
        assert_eq!(order(wgpu::Backends::PRIMARY), [wgpu::Backends::VULKAN, wgpu::Backends::DX12]);
        assert_eq!(order(wgpu::Backends::VULKAN), [wgpu::Backends::VULKAN, wgpu::Backends::DX12]);
        assert_eq!(order(wgpu::Backends::DX12), [wgpu::Backends::DX12, wgpu::Backends::VULKAN]);
        assert_eq!(order(wgpu::Backends::GL), [wgpu::Backends::GL]);
    }

    #[test]
    fn a_refusal_names_the_backend_the_adapter_and_the_lack() {
        let refusal = Refusal { backend: wgpu::Backends::DX12, adapter: Some("A card (Dx12, driver x 1.0)".to_string()), reason: "lacks sampled textures in one stage: has 16, needs 32".to_string() };
        assert_eq!(refusal.to_string(), "dx12: A card (Dx12, driver x 1.0): lacks sampled textures in one stage: has 16, needs 32");
        let none = Refusal { backend: wgpu::Backends::VULKAN, adapter: None, reason: "no graphics adapter: x".to_string() };
        assert_eq!(none.to_string(), "vulkan: no adapter: no graphics adapter: x");
    }

    #[test]
    fn the_fixed_limits_carry_the_sampled_textures_and_the_features_the_arrays() {
        assert_eq!(fixed_limits().max_sampled_textures_per_shader_stage, SAMPLED_TEXTURES);
        assert_eq!(fixed_limits().max_bind_groups, wgpu::Limits::default().max_bind_groups);
        assert!(required_features_of(TextureTier::A).contains(wgpu::Features::TEXTURE_BINDING_ARRAY | wgpu::Features::TEXTURE_COMPRESSION_BC));
        // Every adapter: tier C, without the arrays.
        assert_eq!(required_features(), wgpu::Features::TEXTURE_COMPRESSION_BC);
        assert!(!required_features().contains(wgpu::Features::DEPTH32FLOAT_STENCIL8));
    }

    #[test]
    fn tier_c_samples_six_fixed_pairs_and_no_array() {
        let source = tier_c_shader_source(&world_shader_source("fn extension_shade(in: VertexOut) -> vec4f { return draw.fog_color; }\n"));
        assert!(!source.contains("binding_array") && !source.contains("textures[") && !source.contains("samplers["));
        for stage in 0..STAGES {
            assert!(source.contains(&format!("@group(1) @binding({stage}) var t{stage}: texture_2d<f32>;")));
            assert!(source.contains(&format!("@group(1) @binding({}) var s{stage}: sampler;", STAGES + stage)));
            assert!(source.contains(&format!("textureSample(t{stage}, s{stage}, ")), "stage {stage}");
        }
    }

    /// The world shader of tier C on a device with only tier C's features (no binding arrays):
    /// the module is made without an error. Needs a GPU:
    /// `cargo test -p nirender-host -- --ignored tier_c_shader`.
    #[test]
    #[ignore = "needs a GPU"]
    fn tier_c_shader_validates_on_a_device_without_the_arrays() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("an adapter");
        let descriptor = wgpu::DeviceDescriptor { required_features: required_features_of(TextureTier::C), required_limits: fixed_limits(), ..Default::default() };
        let (device, _queue) = pollster::block_on(adapter.request_device(&descriptor)).expect("a device");
        let source = tier_c_shader_source(&world_shader_source(&crate::extension::extension_shaders(None)));
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let _module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("tier c"), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{error:?}");
    }

    #[test]
    fn the_world_shader_source_joins_the_extension_after_the_depth_functions() {
        let source = world_shader_source("fn extension_shade(in: VertexOut) -> vec4f { return draw.fog_color; }\n");
        assert!(source.starts_with("// The binding arrays"), "{}", &source[..40]);
        assert!(source.contains("return draw_fog_color();"), "the record read is not sent through the shader's function");
        assert!(source.find("fn extension_shade").unwrap() > source.find("fn view_distance").unwrap_or(0));
    }
}
