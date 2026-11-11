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
    wgpu::Limits {
        max_sampled_textures_per_shader_stage: SAMPLED_TEXTURES,
        ..wgpu::Limits::default()
    }
}

/// `MGE_XE_OLD_GPU=least|downlevel`: the adapter taken as an old graphics card, for a test of
/// the path of old hardware on any machine. The request and the probe see the features and the
/// limits of the profile in the place of the adapter's (each limit at most the adapter's), and
/// the device gets them, so that wgpu's validation refuses anything above them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OldGpu {
    /// `least`: the limits that the request asks for ([`fixed_limits`]: wgpu's defaults with
    /// 32 sampled textures in a stage; buffers of 256 MiB and storage bindings of 128 MiB),
    /// with textures of 16384, as every Direct3D 12 adapter of wgpu has (the atlas of MGE XE's
    /// distant statics is 16384 wide). A real adapter of tier C (on Direct3D 12, resource
    /// binding tier 2, NVIDIA Kepler; wgpu 30 offers no tier 1 adapter) is above it in each
    /// limit.
    Least,
    /// `downlevel`: wgpu's downlevel limits (textures of 2048, 4 storage buffers in a stage),
    /// a class of adapter below the backends of the renderer: the request names the lacks.
    Downlevel,
}

impl OldGpu {
    pub fn name(self) -> &'static str {
        match self {
            OldGpu::Least => "least",
            OldGpu::Downlevel => "downlevel",
        }
    }

    /// The limits of the profile, at most the adapter's.
    fn limits(self, adapter: &wgpu::Limits) -> wgpu::Limits {
        let profile = match self {
            OldGpu::Least => wgpu::Limits {
                max_texture_dimension_2d: 16384,
                ..fixed_limits()
            },
            OldGpu::Downlevel => wgpu::Limits::downlevel_defaults(),
        };
        profile.or_worse_values_from(adapter)
    }

    /// The features of the profile: the BC compression and the line mode, which every Direct3D
    /// 12 adapter of wgpu has, and a Vulkan adapter with `fillModeNonSolid`. No binding arrays
    /// (texture tier C), no float depth, no time stamps, no first instance or count of indirect
    /// draws.
    fn features(self) -> wgpu::Features {
        wgpu::Features::TEXTURE_COMPRESSION_BC | wgpu::Features::POLYGON_MODE_LINE
    }
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
    assert!(
        out.contains(arrays),
        "the binding arrays of group 1 are not in the world shader"
    );
    let mut fixed = String::new();
    for stage in 0..STAGES {
        fixed += &format!("@group(1) @binding({stage}) var t{stage}: texture_2d<f32>;\n");
    }
    for stage in 0..STAGES {
        fixed += &format!("@group(1) @binding({}) var s{stage}: sampler;\n", STAGES + stage);
    }
    out = out.replacen(arrays, &fixed, 1);
    for stage in 0..STAGES {
        let indexed =
            format!("textures[draws[record].stages[{stage}].slots.x], samplers[draws[record].stages[{stage}].slots.y]");
        assert_eq!(
            out.matches(&indexed).count(),
            1,
            "stage {stage} does not sample the arrays once"
        );
        out = out.replacen(&indexed, &format!("t{stage}, s{stage}"), 1);
    }
    assert!(
        !out.contains("textures[") && !out.contains("samplers["),
        "the world shader reads the arrays in another place"
    );
    out
}

/// The source of the world shader with the text of an extension (its sky and draw parts):
/// `shader.wgsl`, the depth functions after its `enable` directives, the cone of a spot light
/// ([`crate::spot::WGSL`], for the shader and the extension), then the extension's text with
/// the record of the draw read as `draws[record]` and sent through the functions of the shader.
pub fn world_shader_source(extension_text: &str) -> String {
    let extension = retained::rewrite_extension_text(&extension_text.replace("draw.", "draws[record]."));
    format!(
        "{}\n{}\n{}\n{extension}",
        include_str!("shader.wgsl"),
        depth::wgsl(),
        crate::spot::WGSL
    )
}

/// The line of the world shader that names its debug view (shader.wgsl).
const DEBUG_VIEW_LINE: &str = "const DEBUG_VIEW: u32 = 0u;";

/// The text of the world shader's variant ([`crate::debug_lines::world_variant`]): the text
/// with the variant's id on its one `DEBUG_VIEW` line. None when the text does not have that
/// line exactly once: a missed line would give a variant equal to normal play, with no error.
pub fn world_variant_source(source: &str, variant: u32) -> Option<String> {
    if source.matches(DEBUG_VIEW_LINE).count() != 1 {
        return None;
    }
    Some(source.replacen(DEBUG_VIEW_LINE, &format!("const DEBUG_VIEW: u32 = {variant}u;"), 1))
}

fn array_features() -> wgpu::Features {
    wgpu::Features::TEXTURE_BINDING_ARRAY
        | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
        | wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY
}

/// A backend, an adapter and a device that have what the renderer needs, with the choices
/// made from what the adapter has.
pub(crate) struct Request {
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
    /// The features and the limits of the adapter as the request took them: with
    /// `MGE_XE_OLD_GPU`, those of the profile ([`OldGpu`]).
    pub features: wgpu::Features,
    pub limits: wgpu::Limits,
}

impl Request {
    /// The lines of the log about the request, as the live start prints them.
    pub(crate) fn report(&self) {
        println!("adapter: {}", adapter_line(&self.info));
        if self.surface.is_some() {
            println!("window: the present modes of the surface are {:?}", self.present_modes);
        }
        println!(
            "depth: {:?}, {} (the adapter {} Depth32FloatStencil8{})",
            self.depth_format,
            if depth::reversed() { "reversed" } else { "plain" },
            if self.features.contains(wgpu::Features::DEPTH32FLOAT_STENCIL8) {
                "has"
            } else {
                "lacks"
            },
            if switches().depth24 { "; MGE_XE_DEPTH_FORMAT=24" } else { "" }
        );
        println!("multisampling: {} samples for each pixel of the frame", self.samples);
        if let Some(old) = switches().old_gpu {
            println!(
                "old graphics card: MGE_XE_OLD_GPU={}: the features the BC compression and the line mode; textures up to {}, {} sampled textures in a stage, {} storage buffers in a stage, storage bindings of {} MiB",
                old.name(),
                self.limits.max_texture_dimension_2d,
                self.limits.max_sampled_textures_per_shader_stage,
                self.limits.max_storage_buffers_per_shader_stage,
                self.limits.max_storage_buffer_binding_size >> 20
            );
        }
        match self.tier {
            TextureTier::A => println!(
                "texture tier: a (binding arrays: room for {} textures and {} samplers)",
                self.array_cap, self.sampler_cap
            ),
            TextureTier::C => println!(
                "texture tier: c (a group of {STAGES} textures and {STAGES} samplers for each set; no binding arrays)"
            ),
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
        write!(
            f,
            "{}: {}: {}",
            backend_name(self.backend),
            self.adapter.as_deref().unwrap_or("no adapter"),
            self.reason
        )
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
    format!(
        "{} ({:?}, driver {} {})",
        info.name, info.backend, info.driver, info.driver_info
    )
}

/// Asks the backends in their order for an adapter and a device with what the renderer
/// needs; the first that serves is the answer. With a window, the surface is made on it in
/// the same try, since a surface belongs to the instance of its backend. When no backend
/// serves, every refusal comes back in the order of the tries.
pub(crate) fn request(
    backends: wgpu::Backends,
    window: Option<WindowTarget>,
    wanted_format: wgpu::TextureFormat,
    texture_slots: u32,
    samples: u32,
) -> Result<Request, Vec<Refusal>> {
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
fn try_backend(
    backend: wgpu::Backends,
    window: Option<&WindowTarget>,
    wanted_format: wgpu::TextureFormat,
    texture_slots: u32,
    samples: u32,
) -> Result<Request, Refusal> {
    let refuse = |adapter: Option<String>, reason: String| Refusal {
        backend,
        adapter,
        reason,
    };
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
        Some(WindowTarget::Own(window)) => Some(
            instance
                .create_surface(window.clone())
                .map_err(|error| refuse(None, format!("no surface on the window: {error}")))?,
        ),
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

    // The needs, each with its number. `MGE_XE_OLD_GPU` takes the adapter as an old card.
    let (features, limits) = match switches().old_gpu {
        Some(old) => (adapter.features() & old.features(), old.limits(&adapter.limits())),
        None => (adapter.features(), adapter.limits()),
    };
    let mut lacks = Vec::new();
    // The arrays' lacks: a refusal for tier A forced; else they give tier C.
    let mut array_lacks = Vec::new();
    if !features.contains(array_features()) {
        let missing: Vec<&str> = [
            (wgpu::Features::TEXTURE_BINDING_ARRAY, "TEXTURE_BINDING_ARRAY"),
            (
                wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
                "SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING",
            ),
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
    let wanted = switches().array_cap.unwrap_or(if texture_slots == 0 {
        AUTO_TEXTURE_SLOTS
    } else {
        texture_slots.max(MIN_TEXTURE_SLOTS)
    });
    let array_cap = limits
        .max_binding_array_elements_per_shader_stage
        .saturating_sub(SAMPLER_CAP)
        .min(wanted);
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
    let (array_cap, sampler_cap) = if tier == TextureTier::A {
        (array_cap, sampler_cap)
    } else {
        (0, 0)
    };
    if !features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC) {
        lacks.push("the BC texture compression (TEXTURE_COMPRESSION_BC)".to_string());
    }
    if limits.max_sampled_textures_per_shader_stage < SAMPLED_TEXTURES {
        lacks.push(format!(
            "sampled textures in one stage: has {}, needs {SAMPLED_TEXTURES}",
            limits.max_sampled_textures_per_shader_stage
        ));
    }
    // The other limits that the request asks for whatever the adapter (a device without them
    // is refused at its request).
    fixed_limits().check_limits_with_fail_fn(&limits, false, |name, needs, has| {
        if name != "max_sampled_textures_per_shader_stage" {
            lacks.push(format!("{name}: has {has}, needs {needs}"));
        }
    });
    if !lacks.is_empty() {
        return Err(refuse(name, lacks_text(backend, &lacks)));
    }

    // The choices from what the adapter has.
    let mut target_format = wanted_format;
    let mut surface_usages = wgpu::TextureUsages::empty();
    let mut present_modes = Vec::new();
    if let Some(surface) = surface.as_ref() {
        // The game gives colours that are ready for the screen: no sRGB format.
        let capabilities = surface.get_capabilities(&adapter);
        if !capabilities.formats.contains(&target_format) {
            target_format = capabilities
                .formats
                .iter()
                .copied()
                .find(|format| !format.is_srgb())
                .unwrap_or(capabilities.formats[0]);
        }
        surface_usages = capabilities.usages;
        present_modes = capabilities.present_modes;
    }
    // The format of the depth: a float depth with a stencil when the adapter has it. A
    // float depth is reversed: near 1, far 0 (`depth`). A fixed-point depth gains nothing
    // from it.
    let float_depth = features.contains(wgpu::Features::DEPTH32FLOAT_STENCIL8) && !switches().depth24;
    let depth_format = if float_depth {
        wgpu::TextureFormat::Depth32FloatStencil8
    } else {
        DEPTH_FORMAT
    };
    depth::set_reversed(float_depth);
    depth::set_one_buffer(switches().one_depth);
    if switches().one_depth && !depth::one_buffer() {
        println!("depth: MGE_XE_ONE_DEPTH needs the reversed float depth; the land keeps its own depth");
    }
    // The samples. The adapter's flags for a format hold only on a device that has
    // TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES (the request asks for it below when the adapter
    // has it); any other device takes the counts that the WebGPU spec guarantees.
    let adapter_formats = features.contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);
    let asked = samples;
    let samples = choose_samples(asked, |count| {
        if !adapter_formats {
            return GUARANTEED_SAMPLES.contains(&count);
        }
        let color = adapter.get_texture_format_features(target_format).flags;
        let depth = adapter.get_texture_format_features(depth_format).flags;
        color.sample_count_supported(count)
            && color.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE)
            && depth.sample_count_supported(count)
    });
    println!(
        "samples: asked {asked}, using {samples}{}",
        samples_reason(asked, samples, adapter_formats)
    );

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
                    | wgpu::Features::MULTI_DRAW_INDIRECT_COUNT
                    | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES))
            | if float_depth {
                wgpu::Features::DEPTH32FLOAT_STENCIL8
            } else {
                wgpu::Features::empty()
            },
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

    Ok(Request {
        info,
        surface,
        device,
        queue,
        target_format,
        surface_usages,
        present_modes,
        depth_format,
        samples,
        array_cap,
        sampler_cap,
        tier,
        features,
        limits,
    })
}

/// The counts of samples that the WebGPU spec guarantees for the formats of the frame. A
/// device without TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES refuses any other count.
const GUARANTEED_SAMPLES: [u32; 2] = [1, 4];

/// The largest count of samples up to `asked` that `supported` accepts: `asked` is cut to 1 to
/// 8 and up to a power of two, then halved until `supported` says yes. 1 always stands.
fn choose_samples(asked: u32, supported: impl Fn(u32) -> bool) -> u32 {
    let mut samples = asked.clamp(1, 8).next_power_of_two();
    while samples > 1 && !supported(samples) {
        samples /= 2;
    }
    samples
}

/// The words that go after the chosen count in the log line: why it is lower than the count
/// that was asked for, or nothing when it is the count asked for.
fn samples_reason(asked: u32, chosen: u32, adapter_formats: bool) -> String {
    let wanted = asked.clamp(1, 8).next_power_of_two();
    if chosen == asked {
        String::new()
    } else if chosen == wanted {
        format!(" (asked counts are cut to a power of two from 1 to {MOST_SAMPLES})")
    } else if adapter_formats {
        format!(" ({wanted} is not in the adapter's flags for the colour and depth formats)")
    } else {
        format!(" ({wanted} is not guaranteed: the device has no adapter-specific format features, so only 1 and 4 stand)")
    }
}

/// The most samples for each pixel that the renderer takes.
const MOST_SAMPLES: u32 = 8;

/// The lacks that the refusal names: the first [`LACKS_NAMED`], then their count. The probe's
/// lines and the box of d3d8.dll have a limit of length; all the lacks go into a line of their
/// own, which does not start with `probe: ` (d3d8.dll leaves such lines out of mgeXE.log).
fn lacks_text(backend: wgpu::Backends, lacks: &[String]) -> String {
    if lacks.len() <= LACKS_NAMED {
        return format!("lacks {}", lacks.join("; "));
    }
    println!("{}: all the lacks: {}", backend_name(backend), lacks.join("; "));
    format!(
        "lacks {}; and {} more",
        lacks[..LACKS_NAMED].join("; "),
        lacks.len() - LACKS_NAMED
    )
}

/// The lacks that a refusal names before their count ([`lacks_text`]).
const LACKS_NAMED: usize = 3;

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
                    "probe: {}: {}: ok; texture tier: {}{}; binding arrays {} textures and {} samplers (tier a needs {ARRAY_TEXTURES} and {ARRAY_SAMPLERS}), BC yes, sampled textures {} (needs {SAMPLED_TEXTURES}), depth {:?}",
                    backend_name(backend),
                    adapter_line(&request.info),
                    request.tier.name(),
                    switches().old_gpu.map(|old| format!(" (MGE_XE_OLD_GPU={})", old.name())).unwrap_or_default(),
                    request.array_cap,
                    request.sampler_cap,
                    request.limits.max_sampled_textures_per_shader_stage,
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
            println!(
                "probe: none: {}",
                refusals.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
            );
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
    let reasons: Vec<String> = refusals.iter().map(|refusal| format!("gpu: {refusal}")).collect();
    let why: Vec<String> = refusals
        .iter()
        .map(|refusal| {
            format!(
                "{} ({}) {}.",
                refusal.adapter.as_deref().unwrap_or("No adapter"),
                backend_name(refusal.backend),
                refusal.reason
            )
        })
        .collect();
    let text = format!(
        "The wgpu renderer of MGE XE cannot use this graphics device.\n{}\nClose this box: the game closes. Set [runtime] renderer_wgpu = false in mgeXE.toml for Direct3D 9.\nnirender-host.log in the folder of the game has the details.",
        why.join("\n")
    );
    gpu::refuse(crate::stream_format::EXIT_GPU, &reasons.join("\n"), &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_samples_are_the_largest_count_that_stands() {
        let all = |_: u32| true;
        let guaranteed = |count: u32| GUARANTEED_SAMPLES.contains(&count);
        // An adapter that has every count: the asked count, cut to a power of two from 1 to 8.
        assert_eq!(
            [0, 1, 2, 3, 4, 8, 16].map(|asked| choose_samples(asked, all)),
            [1, 1, 2, 4, 4, 8, 8]
        );
        // A device held to the guaranteed counts (no adapter-specific format features): 8 and 2 step down.
        assert_eq!(
            [1, 2, 4, 8, 16].map(|asked| choose_samples(asked, guaranteed)),
            [1, 1, 4, 4, 4]
        );
        // No count above 1 stands: 1 stands whatever is asked.
        assert_eq!(choose_samples(8, |_| false), 1);
    }

    #[test]
    fn the_reason_of_a_lower_count_names_its_cause() {
        assert_eq!(samples_reason(8, 8, true), "");
        assert_eq!(samples_reason(1, 1, false), "");
        assert!(samples_reason(8, 4, false).contains("not guaranteed"));
        assert!(samples_reason(8, 4, true).contains("adapter's flags"));
        assert!(samples_reason(3, 4, true).contains("power of two"));
    }

    #[test]
    fn the_backends_are_tried_in_their_order() {
        assert_eq!(order(wgpu::Backends::PRIMARY), [wgpu::Backends::VULKAN, wgpu::Backends::DX12]);
        assert_eq!(order(wgpu::Backends::VULKAN), [wgpu::Backends::VULKAN, wgpu::Backends::DX12]);
        assert_eq!(order(wgpu::Backends::DX12), [wgpu::Backends::DX12, wgpu::Backends::VULKAN]);
        assert_eq!(order(wgpu::Backends::GL), [wgpu::Backends::GL]);
    }

    #[test]
    fn a_refusal_names_the_backend_the_adapter_and_the_lack() {
        let refusal = Refusal {
            backend: wgpu::Backends::DX12,
            adapter: Some("A card (Dx12, driver x 1.0)".to_string()),
            reason: "lacks sampled textures in one stage: has 16, needs 32".to_string(),
        };
        assert_eq!(
            refusal.to_string(),
            "dx12: A card (Dx12, driver x 1.0): lacks sampled textures in one stage: has 16, needs 32"
        );
        let none = Refusal {
            backend: wgpu::Backends::VULKAN,
            adapter: None,
            reason: "no graphics adapter: x".to_string(),
        };
        assert_eq!(none.to_string(), "vulkan: no adapter: no graphics adapter: x");
    }

    #[test]
    fn the_fixed_limits_carry_the_sampled_textures_and_the_features_the_arrays() {
        assert_eq!(fixed_limits().max_sampled_textures_per_shader_stage, SAMPLED_TEXTURES);
        assert_eq!(fixed_limits().max_bind_groups, wgpu::Limits::default().max_bind_groups);
        assert!(required_features_of(TextureTier::A)
            .contains(wgpu::Features::TEXTURE_BINDING_ARRAY | wgpu::Features::TEXTURE_COMPRESSION_BC));
        // Every adapter: tier C, without the arrays.
        assert_eq!(required_features(), wgpu::Features::TEXTURE_COMPRESSION_BC);
        assert!(!required_features().contains(wgpu::Features::DEPTH32FLOAT_STENCIL8));
    }

    #[test]
    fn a_refusal_names_three_lacks_and_counts_the_rest() {
        let lacks: Vec<String> = (1..=5).map(|n| format!("lack {n}")).collect();
        assert_eq!(
            lacks_text(wgpu::Backends::VULKAN, &lacks),
            "lacks lack 1; lack 2; lack 3; and 2 more"
        );
        assert_eq!(
            lacks_text(wgpu::Backends::VULKAN, &lacks[..3]),
            "lacks lack 1; lack 2; lack 3"
        );
    }

    #[test]
    fn the_old_gpu_profiles_cut_the_adapter_and_the_least_one_passes() {
        let large = wgpu::Limits {
            max_texture_dimension_2d: 32768,
            max_storage_buffer_binding_size: 1 << 32,
            max_sampled_textures_per_shader_stage: 1 << 20,
            ..wgpu::Limits::default()
        };
        let least = OldGpu::Least.limits(&large);
        assert_eq!(least.max_texture_dimension_2d, 16384);
        assert_eq!(least.max_storage_buffer_binding_size, 128 << 20);
        assert_eq!(least.max_sampled_textures_per_shader_stage, SAMPLED_TEXTURES);
        assert!(
            fixed_limits().check_limits(&least),
            "the request does not take its least adapter"
        );
        // A profile is never above the adapter.
        let small = wgpu::Limits {
            max_texture_dimension_2d: 4096,
            ..fixed_limits()
        };
        assert_eq!(OldGpu::Least.limits(&small).max_texture_dimension_2d, 4096);
        // The downlevel profile lacks some of what the request asks for, by name.
        let mut lacks = Vec::new();
        fixed_limits().check_limits_with_fail_fn(&OldGpu::Downlevel.limits(&large), false, |name, _, _| lacks.push(name));
        assert!(
            lacks.contains(&"max_storage_buffers_per_shader_stage") && lacks.contains(&"max_texture_dimension_2d"),
            "{lacks:?}"
        );
        assert_eq!(
            OldGpu::Least.features(),
            wgpu::Features::TEXTURE_COMPRESSION_BC | wgpu::Features::POLYGON_MODE_LINE
        );
    }

    #[test]
    fn the_world_shader_of_tier_c_is_valid_wgsl() {
        let source = tier_c_shader_source(&world_shader_source(&crate::extension::extension_shaders(None)));
        crate::assert_wgsl("tier C", &source);
        // The null test: the check fails on a broken line.
        let broken = source.replacen("var t0: texture_2d<f32>;", "var t0: texture_2d<f32>", 1);
        assert_ne!(broken, source);
        assert!(std::panic::catch_unwind(|| crate::assert_wgsl("tier C, broken", &broken)).is_err());
    }

    #[test]
    fn tier_c_samples_six_fixed_pairs_and_no_array() {
        let source = tier_c_shader_source(&world_shader_source(
            "fn extension_shade(in: VertexOut) -> vec4f { return draw.fog_color; }\n",
        ));
        assert!(!source.contains("binding_array") && !source.contains("textures[") && !source.contains("samplers["));
        for stage in 0..STAGES {
            assert!(source.contains(&format!("@group(1) @binding({stage}) var t{stage}: texture_2d<f32>;")));
            assert!(source.contains(&format!("@group(1) @binding({}) var s{stage}: sampler;", STAGES + stage)));
            assert!(
                source.contains(&format!("textureSample(t{stage}, s{stage}, ")),
                "stage {stage}"
            );
        }
    }

    /// The world shader of tier C on a device with only tier C's features (no binding arrays):
    /// the module is made without an error. Needs a GPU:
    /// `cargo test -p nirender-host -- --ignored tier_c_shader`.
    #[test]
    #[ignore = "needs a GPU"]
    fn tier_c_shader_validates_on_a_device_without_the_arrays() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("an adapter");
        let descriptor = wgpu::DeviceDescriptor {
            required_features: required_features_of(TextureTier::C),
            required_limits: fixed_limits(),
            ..Default::default()
        };
        let (device, _queue) = pollster::block_on(adapter.request_device(&descriptor)).expect("a device");
        let source = tier_c_shader_source(&world_shader_source(&crate::extension::extension_shaders(None)));
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let _module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tier c"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{error:?}");
    }

    #[test]
    fn the_world_shader_source_joins_the_extension_after_the_depth_functions() {
        let source = world_shader_source("fn extension_shade(in: VertexOut) -> vec4f { return draw.fog_color; }\n");
        assert!(source.starts_with("// The binding arrays"), "{}", &source[..40]);
        assert!(
            source.contains("return draw_fog_color();"),
            "the record read is not sent through the shader's function"
        );
        assert!(source.find("fn extension_shade").unwrap() > source.find("fn view_distance").unwrap_or(0));
    }

    #[test]
    fn the_world_shader_is_valid_wgsl() {
        crate::assert_wgsl(
            "shader.wgsl",
            &world_shader_source(&crate::extension::extension_shaders(None)),
        );
    }

    /// A variant's text is the world shader's text with the variant's id on the one
    /// `DEBUG_VIEW` line, and it validates; a text without the line, or with it twice, has no
    /// variant (a missed line would give a variant equal to normal play).
    #[test]
    fn a_world_variant_changes_the_one_debug_view_line() {
        use crate::debug_lines::view_id;
        let world = world_shader_source(&crate::extension::extension_shaders(None));
        assert_eq!(world.matches(DEBUG_VIEW_LINE).count(), 1);
        for variant in [view_id::LIGHTS, view_id::SHADOW_LAYERS] {
            let source = world_variant_source(&world, variant).expect("the line is there once");
            let line = format!("const DEBUG_VIEW: u32 = {variant}u;");
            assert_ne!(source, world);
            assert_eq!(source.matches(&line).count(), 1);
            assert_eq!(source.replacen(&line, DEBUG_VIEW_LINE, 1), world);
            crate::assert_wgsl(&format!("world variant {variant}"), &source);
        }
        // The text of tier C keeps the line.
        assert!(world_variant_source(&tier_c_shader_source(&world), view_id::LIGHTS).is_some());
        assert_eq!(world_variant_source(&world.replace(DEBUG_VIEW_LINE, ""), 1), None);
        assert_eq!(world_variant_source(&format!("{world}\n{DEBUG_VIEW_LINE}\n"), 1), None);
    }

    /// The cone of a spot light has one home: the world shader has the text of spot.rs once,
    /// before the extension's text, and `shader.wgsl` calls it and has no cone of its own.
    #[test]
    fn the_world_shader_takes_the_one_spot_cone() {
        let source = world_shader_source("fn extension_shade(in: VertexOut) -> vec4f { return draw.fog_color; }\n");
        assert_eq!(source.matches(crate::spot::WGSL).count(), 1);
        assert_eq!(source.matches("fn spot_cone").count(), 1);
        assert!(source.find(crate::spot::WGSL).unwrap() < source.find("fn extension_shade").unwrap());
        let own = include_str!("shader.wgsl");
        assert!(
            own.contains("spot_cone(light, direction)") && !own.contains("fn spot_cone") && !own.contains("light.cone.x")
        );
    }
}
