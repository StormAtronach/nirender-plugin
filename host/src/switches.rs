//! The diagnostic and test switches of the helper, read from the environment once: none of
//! them is a setting. The log gets one line with each switch that is set and its value at the
//! start ([`log_set`]), so that a leftover variable of a test leaves a trace, and one line for
//! each value that a read cannot take, which names the switch, the value and what is taken
//! instead. README.md of the plugin names every switch, and `tools/check_readme.py` checks it.

use core::fmt::Display;
use core::str::FromStr;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::wgpu;

/// The prefixes of the switches of the plugin and of the helper.
const PREFIXES: [&str; 3] = ["NIRENDER_", "MGE_XE_", "WGPU_"];

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
    /// `MGE_XE_ARRAY_CAP=<n>`: the room of the texture arrays at most n, for the test of a
    /// full room; below 256 the request refuses the adapter, for the test of the probe and
    /// of the live start with a GPU the renderer cannot use.
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
    /// `MGE_XE_ONE_DEPTH=1`: the distant land and the game in one depth buffer, with an
    /// infinite far plane (`depth::one_buffer`). Temporary, for the test of step 6 of the depth
    /// design.
    pub(crate) one_depth: bool,
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
    /// the setting of the extension ([`crate::Extension::vsync`]), for a test.
    pub(crate) present: Option<wgpu::PresentMode>,
    /// `MGE_XE_WINDOW=game`: the surface goes on the window of the game itself, not on a
    /// child window of it: for an A/B of the mouse's lag.
    pub(crate) game_window: bool,
    /// `NIRENDER_FORCE_GROWTH=<n>`: every n frames the retained table grows on purpose, for
    /// the test of its growth; 0 is never.
    pub(crate) force_growth: u32,
    /// `NIRENDER_GROWTH_SHOTS=<prefix>`: a picture after each forced growth, at that prefix.
    pub(crate) growth_shots: Option<String>,
    /// `MGE_XE_BOX_SECONDS=<n>`: a message box of this program closes itself after n seconds
    /// (the harness mode of the boxes, [`crate::gpu::message_box`]); 0 or unset: the box waits
    /// for the click. The plugin and MGE XE's d3d8.dll read the same variable for their boxes.
    pub(crate) box_seconds: u32,
}

impl Switches {
    fn read() -> Switches {
        use wgpu::PresentMode::{AutoNoVsync, Fifo, Immediate, Mailbox};
        let mut read = Read::default();
        let switches = Switches {
            replay_debug: read.on("MGE_XE_REPLAY_DEBUG") == Some(true),
            cores_to_scheduler: read.on("MGE_XE_CORES") == Some(false),
            clusters: read.on("MGE_XE_CLUSTERS"),
            world_lights: read.choice("MGE_XE_LIGHTS", &["world", "engine"]) == Some("world"),
            draw_dump: read.parsed("MGE_XE_DRAW_DUMP", "a geometry handle in hex", |v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok()).unwrap_or(0),
            gpu_time: read.on("MGE_XE_GPU_TIME") == Some(true),
            runs_dump: read.on("MGE_XE_RUNS_DUMP") == Some(true),
            indirect: read.on("MGE_XE_INDIRECT") != Some(false),
            retained: read.on("MGE_XE_RETAINED") != Some(false),
            array_cap: read.number::<u32>("MGE_XE_ARRAY_CAP"),
            record_limit: read.number::<usize>("MGE_XE_RECORD_LIMIT").map(|n| n.max(1024)),
            samples: read.number::<u32>("MGE_XE_SAMPLES").map(|n| n.clamp(1, 8)),
            depth24: read.choice("MGE_XE_DEPTH_FORMAT", &["24", "32"]) == Some("24"),
            depth_plain: read.on("MGE_XE_DEPTH_REVERSED") == Some(false),
            one_depth: read.on("MGE_XE_ONE_DEPTH") == Some(true),
            particle_quads_cpu: read.choice("MGE_XE_PARTICLE_QUADS", &["cpu", "gpu"]) == Some("cpu"),
            instance_upload_all: read.choice("MGE_XE_INSTANCE_UPLOAD", &["all", "changed"]) == Some("all"),
            lazy_pass: read.on("MGE_XE_LAZY_PASS") != Some(false),
            pass_groups: read.on("MGE_XE_PASS_GROUPS") != Some(false),
            frame_latency: read.number::<u32>("MGE_XE_FRAME_LATENCY").map(|n| n.clamp(1, 3)),
            present: read.choice("MGE_XE_PRESENT", &["immediate", "mailbox", "fifo", "auto"]).map(|mode| match mode {
                "immediate" => Immediate,
                "mailbox" => Mailbox,
                "fifo" => Fifo,
                _ => AutoNoVsync,
            }),
            game_window: read.choice("MGE_XE_WINDOW", &["game", "child"]) == Some("game"),
            force_growth: read.number("NIRENDER_FORCE_GROWTH").unwrap_or(0),
            growth_shots: read.text("NIRENDER_GROWTH_SHOTS"),
            // At most an hour: the timeout of the box is in ms, and a larger value overflows it.
            box_seconds: read.parsed("MGE_XE_BOX_SECONDS", "a number of seconds", |v| v.trim().parse::<u32>().ok()).map_or(0, |n| n.min(3600)),
        };
        read.report();
        switches
    }
}

/// The reads of the switches. A value that a read cannot take gives a line, printed after
/// all the reads ([`Read::report`]), so that the log has them together.
#[derive(Default)]
struct Read {
    problems: Vec<String>,
}

impl Read {
    /// The value of a switch, when it is set.
    fn text(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    /// A switch that is on (`1`) or off (`0`): `None` when it is not set, or when it has
    /// another value.
    fn on(&mut self, name: &str) -> Option<bool> {
        self.parsed(name, "1 or 0", parse_on)
    }

    /// A number: `None` when the switch is not set, or when it is not a number.
    fn number<T: FromStr>(&mut self, name: &str) -> Option<T> {
        self.parsed(name, "a number", |value| value.parse().ok())
    }

    /// One of the values in `allowed`: `None` when the switch is not set, or has another value.
    fn choice<'a>(&mut self, name: &str, allowed: &[&'a str]) -> Option<&'a str> {
        let expected = allowed.join(", ");
        self.parsed(name, &expected, |value| allowed.iter().copied().find(|allowed| *allowed == value))
    }

    /// The value of a switch through `parse`; a value that `parse` refuses gives a line that
    /// names `expected`, the values that it takes, and "not set" as what is taken.
    fn parsed<T>(&mut self, name: &str, expected: &str, parse: impl FnOnce(&str) -> Option<T>) -> Option<T> {
        self.parsed_or(name, expected, "not set", parse)
    }

    /// `parsed`, with `taken` as what the line names in place of the bad value.
    fn parsed_or<T>(&mut self, name: &str, expected: &str, taken: &str, parse: impl FnOnce(&str) -> Option<T>) -> Option<T> {
        let value = self.text(name)?;
        let parsed = parse(&value);
        if parsed.is_none() {
            self.problems.push(bad_value_line(name, &value, expected, taken));
        }
        parsed
    }

    fn report(&self) {
        for line in &self.problems {
            println!("{line}");
        }
    }
}

/// The value of a switch, when it is set: for a program that has this crate as a part and
/// reads switches of its own.
pub fn text(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// A switch that is on (`1`) or off (`0`): `None` when it is not set, or when it has another
/// value, which the log names at once.
pub fn on(name: &str) -> Option<bool> {
    read_now(|read| read.on(name))
}

/// A number; a value that is not one is named in the log at once, and `default` is taken.
pub fn number<T: FromStr + Display>(name: &str, default: T) -> T {
    read_now(|read| read.parsed_or(name, "a number", &default.to_string(), |value| value.parse().ok())).unwrap_or(default)
}

/// The path that a switch names, when it is set: a path is read as it is, not as Unicode.
pub fn path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

/// The time stamps of the parts of the frame are on (`MGE_XE_GPU_TIME=1`): one read for
/// the renderer and the extension.
pub fn gpu_time() -> bool {
    switches().gpu_time
}

/// One of the values in `allowed`: `None` when the switch is not set, or when it has another
/// value, which the log names at once with the values allowed.
pub fn choice<'a>(name: &str, allowed: &[&'a str]) -> Option<&'a str> {
    read_now(|read| read.choice(name, allowed))
}

/// The value of a switch through `parse`; a value that `parse` refuses is named in the log
/// at once with `expected`, the values that it takes.
pub fn parsed<T>(name: &str, expected: &str, parse: impl FnOnce(&str) -> Option<T>) -> Option<T> {
    read_now(|read| read.parsed(name, expected, parse))
}

/// One read, with its line about a bad value printed at once.
fn read_now<T>(read: impl FnOnce(&mut Read) -> T) -> T {
    let mut reader = Read::default();
    let value = read(&mut reader);
    reader.report();
    value
}

/// `1` is on and `0` is off.
fn parse_on(value: &str) -> Option<bool> {
    match value {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

fn bad_value_line(name: &str, value: &str, expected: &str, taken: &str) -> String {
    format!("switch {name}={value}: not {expected}; {taken}")
}

/// The switches of the process, read at the first call.
pub(crate) fn switches() -> &'static Switches {
    static SWITCHES: OnceLock<Switches> = OnceLock::new();
    SWITCHES.get_or_init(Switches::read)
}

/// One log line with every switch that is set, for the start of the log, then the lines
/// about the values that the reads cannot take. A name or a value that is not Unicode is
/// shown lossily (`vars` would panic on it).
pub(crate) fn log_set() {
    let variables = std::env::vars_os().map(|(name, value)| (name.to_string_lossy().into_owned(), value.to_string_lossy().into_owned()));
    println!("{}", set_line(variables));
    let _ = switches();
}

/// The switches among `variables`, in the order of their names.
fn set_line(variables: impl Iterator<Item = (String, String)>) -> String {
    let mut set: Vec<String> = variables.filter(|(name, _)| PREFIXES.iter().any(|prefix| name.starts_with(prefix))).map(|(name, value)| format!("{name}={value}")).collect();
    set.sort();
    if set.is_empty() {
        "switches: none set".to_string()
    } else {
        format!("switches set: {}", set.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_takes_1_and_0_only() {
        assert_eq!(parse_on("1"), Some(true));
        assert_eq!(parse_on("0"), Some(false));
        assert_eq!(parse_on("yes"), None);
    }

    /// A name of a test switch, from the prefix: a name in quotes would look like a read of
    /// the code to `tools/check_readme.py`, which wants every read in README.md.
    fn test_switch(suffix: &str) -> String {
        format!("{}TEST_SWITCH_{suffix}", PREFIXES[1])
    }

    #[test]
    fn a_read_takes_good_values_and_names_bad_ones() {
        let (on_name, number_name, choice_name, bad) = (test_switch("ON"), test_switch("NUMBER"), test_switch("CHOICE"), test_switch("BAD"));
        std::env::set_var(&on_name, "1");
        std::env::set_var(&number_name, "12");
        std::env::set_var(&choice_name, "fifo");
        std::env::set_var(&bad, "many");
        let mut read = Read::default();
        assert_eq!(read.on(&on_name), Some(true));
        assert_eq!(read.on(&test_switch("NOT_SET")), None);
        assert_eq!(read.number::<u32>(&number_name), Some(12));
        assert_eq!(read.choice(&choice_name, &["fifo", "auto"]), Some("fifo"));
        assert!(read.problems.is_empty());

        assert_eq!(read.on(&bad), None);
        assert_eq!(read.number::<u32>(&bad), None);
        assert_eq!(read.choice(&bad, &["fifo", "auto"]), None);
        assert_eq!(
            read.problems,
            [
                format!("switch {bad}=many: not 1 or 0; not set"),
                format!("switch {bad}=many: not a number; not set"),
                format!("switch {bad}=many: not fifo, auto; not set"),
            ]
        );
    }

    #[test]
    fn a_read_of_a_program_gives_the_default_for_a_bad_value() {
        let (debug, words) = (test_switch("DEBUG"), test_switch("WORDS"));
        std::env::set_var(&debug, "0.5");
        std::env::set_var(&words, "many");
        assert_eq!(number(&debug, 0.0), 0.5);
        assert_eq!(number(&words, 0.0), 0.0);
        assert_eq!(number(&test_switch("NOT_SET"), 0.0), 0.0);
        assert_eq!(on(&words), None);
        assert_eq!(choice(&words, &["cpu", "gpu"]), None);
        assert_eq!(text(&words).as_deref(), Some("many"));
        assert_eq!(path(&words).as_deref(), Some(std::path::Path::new("many")));
        assert_eq!(bad_value_line(&words, "many", "a number", "0"), format!("switch {words}=many: not a number; 0"));
    }

    #[test]
    fn the_set_line_names_the_switches_in_order() {
        let (b, a) = (format!("{}B", PREFIXES[0]), format!("{}A", PREFIXES[1]));
        let variables = [("PATH", "x"), (b.as_str(), "2"), (a.as_str(), "1"), ("HOME", "y")];
        let line = set_line(variables.iter().map(|(name, value)| (name.to_string(), value.to_string())));
        assert_eq!(line, format!("switches set: {a}=1, {b}=2"));
        assert_eq!(set_line(std::iter::empty()), "switches: none set");
    }
}
