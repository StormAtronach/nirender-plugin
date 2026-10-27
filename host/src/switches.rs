//! The diagnostic and test switches of the helper, read from the environment once: none of
//! them is a setting.

use std::sync::OnceLock;

use crate::wgpu;

/// The switches, as the environment of the process had them at the first read.
#[derive(Clone, Debug)]
pub(crate) struct Switches {
    /// `MGE_XE_REPLAY_DEBUG=1`: a line for each draw a replay issues, and the values of the
    /// world's start.
    pub(crate) replay_debug: bool,
    /// `MGE_XE_CORES=0`: the threads are left to the scheduler.
    pub(crate) cores_to_scheduler: bool,
    /// `MGE_XE_CLUSTERS=1` or `0`: the clusters of the lights on or off, over the setting.
    pub(crate) clusters: Option<bool>,
    /// `MGE_XE_LIGHTS=world`: a lit draw of the world takes the point lights of the world
    /// that reach its pixel (the clusters of `WORLD_LIGHTS`), not the lights the engine chose
    /// for it. `engine` (the default until the A/B reaches parity): the lights the engine chose.
    pub(crate) world_lights: bool,
    /// `MGE_XE_DRAW_DUMP=<geometry handle, hex>`: the words of each draw of that geometry.
    pub(crate) draw_dump: u32,
    /// `MGE_XE_GPU_TIME=1`: time stamps of the parts of the frame (they cost time).
    pub(crate) gpu_time: bool,
    /// `MGE_XE_RUNS_DUMP=1`: the runs of the table of each frame.
    pub(crate) runs_dump: bool,
    /// `MGE_XE_INDIRECT=0`: one draw call for each retained draw, no indirect runs: it rules
    /// the indirect runs out when a picture is wrong, and goes around a driver's fault in them.
    pub(crate) indirect: bool,
    /// `MGE_XE_RETAINED=0`: the table draws nothing. With a capture of the plugin's mode
    /// `dual` (and the engine's walk), the engine's own draws of the same capture: the
    /// reference of a debug toggle.
    pub(crate) retained: bool,
    /// `MGE_XE_ARRAY_CAP=<n>`: the room of the texture arrays at most n (at least 256), for
    /// the test of a full room.
    pub(crate) array_cap: Option<u32>,
    /// `MGE_XE_RECORD_LIMIT=<n>`: at most n draw records (at least 1024), for the test of the
    /// guard of the device's limit.
    pub(crate) record_limit: Option<usize>,
    /// `MGE_XE_SAMPLES=<n>`: the samples for each pixel of the targets of the frame (1, 2, 4
    /// or 8), over the setting of the extension: for an A/B of the multisampling.
    pub(crate) samples: Option<u32>,
    /// `MGE_XE_DEPTH_FORMAT=24`: the depth of the frame in `Depth24PlusStencil8` even when the
    /// adapter has `Depth32FloatStencil8`: for an A/B of the format and the test of its fallback.
    pub(crate) depth24: bool,
    /// `MGE_XE_DEPTH_REVERSED=0`: the float depth of the frame keeps the plain convention (near
    /// 0, far 1): for an A/B of the reversed depth (`depth::reversed`).
    pub(crate) depth_plain: bool,
    /// `MGE_XE_PARTICLE_QUADS=cpu`: the quads of the particle systems made on the CPU, as
    /// before the compute pass of `particle_quads`: for its A/B.
    pub(crate) particle_quads_cpu: bool,
    /// `MGE_XE_INSTANCE_UPLOAD=all`: the retained segment of the instances goes up whole when
    /// more than 64 slots changed in a frame, as before the copy of `retained_upload`: for its
    /// A/B.
    pub(crate) instance_upload_all: bool,
    /// `MGE_XE_LAZY_PASS=0`: a pass on the game's targets begins when it is asked for, and a
    /// clear does not carry into the next pass, as before `Player::begin_pass`: for its A/B.
    pub(crate) lazy_pass: bool,
    /// `MGE_XE_PASS_GROUPS=0`: the game's draws before the world and after the post chain bind
    /// the whole group of the texture arrays, as before `Renderer::small_array_group`: for its
    /// A/B.
    pub(crate) pass_groups: bool,
    /// `MGE_XE_FRAME_LATENCY=<n>` (1 to 3): the frames that may wait for the GPU, over the
    /// setting of the extension ([`crate::Extension::frame_latency`]): for an A/B of the
    /// mouse's lag.
    pub(crate) frame_latency: Option<u32>,
    /// `MGE_XE_PRESENT=immediate|mailbox|fifo|auto`: the present mode of the window, over
    /// `AutoNoVsync`, for a test.
    pub(crate) present: Option<wgpu::PresentMode>,
    /// `MGE_XE_WINDOW=game`: the surface goes on the window of the game itself, not on a
    /// child window of it: for an A/B of the mouse's lag.
    pub(crate) game_window: bool,
    /// `MGE_XE_BOX_SECONDS=<n>`: a message box of this program closes itself after n seconds
    /// (the harness mode of the boxes, [`crate::gpu::message_box`]); 0 or unset: the box waits
    /// for the click. The plugin and MGE XE's d3d8.dll read the same variable for their boxes.
    pub(crate) box_seconds: u32,
}

impl Switches {
    fn read() -> Switches {
        let var = |name: &str| std::env::var(name).ok();
        let on_unless_0 = |name: &str| var(name).is_none_or(|v| v != "0");
        let is = |name: &str, value: &str| var(name).is_some_and(|v| v == value);
        Switches {
            replay_debug: is("MGE_XE_REPLAY_DEBUG", "1"),
            cores_to_scheduler: is("MGE_XE_CORES", "0"),
            clusters: match var("MGE_XE_CLUSTERS").as_deref() {
                Some("1") => Some(true),
                Some("0") => Some(false),
                _ => None,
            },
            world_lights: is("MGE_XE_LIGHTS", "world"),
            draw_dump: var("MGE_XE_DRAW_DUMP").and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok()).unwrap_or(0),
            gpu_time: is("MGE_XE_GPU_TIME", "1"),
            runs_dump: is("MGE_XE_RUNS_DUMP", "1"),
            indirect: on_unless_0("MGE_XE_INDIRECT"),
            retained: on_unless_0("MGE_XE_RETAINED"),
            array_cap: var("MGE_XE_ARRAY_CAP").and_then(|v| v.parse::<u32>().ok()).map(|n| n.max(256)),
            record_limit: var("MGE_XE_RECORD_LIMIT").and_then(|v| v.parse::<usize>().ok()).map(|n| n.max(1024)),
            samples: var("MGE_XE_SAMPLES").and_then(|v| v.parse::<u32>().ok()).map(|n| n.clamp(1, 8)),
            depth24: is("MGE_XE_DEPTH_FORMAT", "24"),
            depth_plain: is("MGE_XE_DEPTH_REVERSED", "0"),
            particle_quads_cpu: is("MGE_XE_PARTICLE_QUADS", "cpu"),
            instance_upload_all: is("MGE_XE_INSTANCE_UPLOAD", "all"),
            lazy_pass: on_unless_0("MGE_XE_LAZY_PASS"),
            pass_groups: on_unless_0("MGE_XE_PASS_GROUPS"),
            frame_latency: var("MGE_XE_FRAME_LATENCY").and_then(|v| v.parse::<u32>().ok()).map(|n| n.clamp(1, 3)),
            present: match var("MGE_XE_PRESENT").as_deref() {
                Some("immediate") => Some(wgpu::PresentMode::Immediate),
                Some("mailbox") => Some(wgpu::PresentMode::Mailbox),
                Some("fifo") => Some(wgpu::PresentMode::Fifo),
                Some("auto") => Some(wgpu::PresentMode::AutoNoVsync),
                _ => None,
            },
            game_window: is("MGE_XE_WINDOW", "game"),
            // At most an hour: the timeout of the box is in ms, and a larger value overflows it.
            box_seconds: var("MGE_XE_BOX_SECONDS").and_then(|v| v.trim().parse::<u32>().ok()).map_or(0, |n| n.min(3600)),
        }
    }
}

/// The switches of the process, read at the first call.
pub(crate) fn switches() -> &'static Switches {
    static SWITCHES: OnceLock<Switches> = OnceLock::new();
    SWITCHES.get_or_init(Switches::read)
}
