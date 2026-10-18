//! The A/B switches of the helper, read from the environment once. Each switch keeps the
//! rule before a change, for a comparison of the two; none of them is a setting. They
//! retire at checkpoint 4 of the takeover (the plan, section 2).

use std::sync::OnceLock;

/// The order of the retained draws of a view.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Order {
    /// Front to back by the bucket's nearest draw, the blends after (the rule since step 3).
    Default,
    /// `MGE_XE_ORDER=opaque`: the opaque buckets before the others.
    Opaque,
    /// `MGE_XE_ORDER=buckets`: the order before step 3, bucket by bucket.
    Buckets,
}

/// The switches, as the environment of the process had them at the first read.
#[derive(Clone, Debug)]
pub(crate) struct Switches {
    /// `MGE_XE_REPLAY_DEBUG=1`: a line for each draw a replay issues, and the values of the
    /// world's start.
    pub(crate) replay_debug: bool,
    /// `NIRENDER_FLAT_SHADING=0`: the flat shade bit of a material is not drawn.
    pub(crate) flat_shading: bool,
    /// `MGE_XE_CORES=0`: the threads are left to the scheduler.
    pub(crate) cores_to_scheduler: bool,
    /// `MGE_XE_CLUSTERS=1` or `0`: the clusters of the lights on or off, over the setting.
    pub(crate) clusters: Option<bool>,
    /// `MGE_XE_DRAW_DUMP=<geometry handle, hex>`: the words of each draw of that geometry.
    pub(crate) draw_dump: u32,
    /// `MGE_XE_GPU_TIME=1`: time stamps of the parts of the frame (about 0.4 ms a frame).
    pub(crate) gpu_time: bool,
    /// `MGE_XE_RUNS_DUMP=1`: the runs of the table of each frame.
    pub(crate) runs_dump: bool,
    /// `MGE_XE_MERGE=0`: the blends of the engine's path stay in place (the rule before F5).
    pub(crate) merge_blends: bool,
    /// `MGE_XE_ORDER`: the order of the retained draws.
    pub(crate) order: Order,
    /// `MGE_XE_INDIRECT=0`: one draw call for each retained draw, no indirect runs.
    pub(crate) indirect: bool,
    /// `MGE_XE_RETAINED=0`: the table draws nothing (the engine's path draws everything).
    pub(crate) retained: bool,
    /// `MGE_XE_ARRAY_CAP=<n>`: the room of the texture arrays at most n (at least 256), for
    /// the test of a full room (the owner's review of CP4, point 3). Not a setting.
    pub(crate) array_cap: Option<u32>,
    /// `MGE_XE_RECORD_LIMIT=<n>`: at most n draw records (at least 1024), for the test of the
    /// guard of the device's limit (the owner's review of CP4, point 5). Not a setting.
    pub(crate) record_limit: Option<usize>,
}

impl Switches {
    fn read() -> Switches {
        let var = |name: &str| std::env::var(name).ok();
        let on_unless_0 = |name: &str| var(name).is_none_or(|v| v != "0");
        let is = |name: &str, value: &str| var(name).is_some_and(|v| v == value);
        Switches {
            replay_debug: is("MGE_XE_REPLAY_DEBUG", "1"),
            flat_shading: on_unless_0("NIRENDER_FLAT_SHADING"),
            cores_to_scheduler: is("MGE_XE_CORES", "0"),
            clusters: match var("MGE_XE_CLUSTERS").as_deref() {
                Some("1") => Some(true),
                Some("0") => Some(false),
                _ => None,
            },
            draw_dump: var("MGE_XE_DRAW_DUMP").and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok()).unwrap_or(0),
            gpu_time: is("MGE_XE_GPU_TIME", "1"),
            runs_dump: is("MGE_XE_RUNS_DUMP", "1"),
            merge_blends: on_unless_0("MGE_XE_MERGE"),
            order: match var("MGE_XE_ORDER").as_deref() {
                Some("opaque") => Order::Opaque,
                Some("buckets") => Order::Buckets,
                _ => Order::Default,
            },
            indirect: on_unless_0("MGE_XE_INDIRECT"),
            retained: on_unless_0("MGE_XE_RETAINED"),
            array_cap: var("MGE_XE_ARRAY_CAP").and_then(|v| v.parse::<u32>().ok()).map(|n| n.max(256)),
            record_limit: var("MGE_XE_RECORD_LIMIT").and_then(|v| v.parse::<usize>().ok()).map(|n| n.max(1024)),
        }
    }
}

/// The switches of the process, read at the first call.
pub(crate) fn switches() -> &'static Switches {
    static SWITCHES: OnceLock<Switches> = OnceLock::new();
    SWITCHES.get_or_init(Switches::read)
}
