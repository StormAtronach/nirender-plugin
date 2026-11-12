//! The scenes of the game by their kind (`link::SCENE_*`), for the `scenes:` line of the
//! report (W401): how many scenes of each kind, the time of the main thread from the begin of
//! each scene to its end, and the displays of retained shapes that a scene into the back
//! buffer hands back to the engine's draw ([`hand_back`]). A diagnostic: it reads the
//! processor's counter at each begin and end of a scene (about 7 scenes in a frame) and changes
//! nothing of the game's work.
//!
//! The time of a scene runs from [`begin`] to [`end`]: from the plugin's begin of the scene
//! (after MGE XE's frame logic for it) to the engine's `EndPaint`, so it holds the engine's
//! Click of that camera (the culling and the displays) and the plugin's work in it.

use crate::main_thread::{MainCell, MainThread};

/// The kinds that the counts keep: `link::SCENE_*` are 0 to 12. A larger kind counts as the
/// last one.
const KINDS: usize = 16;

/// The names of the scene kinds, by `link::SCENE_*`.
const NAMES: [&str; 13] = [
    "other",
    "world",
    "sorted alpha",
    "water",
    "shadow volumes",
    "shadow casters",
    "first person",
    "interface",
    "race menu head",
    "splash",
    "character preview",
    "local map",
    "water reflection",
];

/// The kinds that the line always names, with 0 when the run has none: the shadow camera's.
const ALWAYS: [usize; 2] = [4, 5];

struct SceneKinds {
    /// The scene in progress: its kind and the counter at its begin.
    open: Option<(usize, i64)>,
    scenes: [u64; KINDS],
    handed_back: [u64; KINDS],
    /// The counter's ticks of the frame in progress, and over the run the sum and the worst
    /// frame.
    frame_ticks: [i64; KINDS],
    sum_ticks: [i64; KINDS],
    worst_ticks: [i64; KINDS],
    frames: u64,
}

impl SceneKinds {
    const fn new() -> SceneKinds {
        SceneKinds {
            open: None,
            scenes: [0; KINDS],
            handed_back: [0; KINDS],
            frame_ticks: [0; KINDS],
            sum_ticks: [0; KINDS],
            worst_ticks: [0; KINDS],
            frames: 0,
        }
    }

    fn begin(&mut self, kind: u32, now: i64) {
        // A begin with a scene still open (no end came) closes that scene here.
        self.end(now);
        let index = index(kind);
        self.scenes[index] += 1;
        self.open = Some((index, now));
    }

    fn end(&mut self, now: i64) {
        if let Some((index, start)) = self.open.take() {
            self.frame_ticks[index] += (now - start).max(0);
        }
    }

    fn frame_end(&mut self) {
        for i in 0..KINDS {
            self.sum_ticks[i] += self.frame_ticks[i];
            self.worst_ticks[i] = self.worst_ticks[i].max(self.frame_ticks[i]);
        }
        self.frame_ticks = [0; KINDS];
        self.frames += 1;
    }

    /// `scenes: by kind for each of N frames: <kind> <n> scenes, <ms> ms a frame (worst <ms>),
    /// handed back <n>; ...`. The kinds with a scene or a display handed back, and always the
    /// kinds of [`ALWAYS`].
    fn line(&self, ticks_per_second: i64) -> String {
        let frames = self.frames.max(1) as f64;
        let ms = |ticks: i64| ticks as f64 * 1000.0 / ticks_per_second.max(1) as f64;
        let parts: Vec<String> = (0..KINDS)
            .filter(|&i| self.scenes[i] > 0 || self.handed_back[i] > 0 || ALWAYS.contains(&i))
            .map(|i| {
                format!(
                    "{} {} scenes, {:.3} ms a frame (worst {:.3}), handed back {}",
                    NAMES.get(i).copied().unwrap_or("kind 13 or more"),
                    self.scenes[i],
                    ms(self.sum_ticks[i]) / frames,
                    ms(self.worst_ticks[i]),
                    self.handed_back[i]
                )
            })
            .collect();
        format!("scenes: by kind for each of {} frames: {}", self.frames, parts.join("; "))
    }
}

fn index(kind: u32) -> usize {
    (kind as usize).min(KINDS - 1)
}

static SCENES: MainCell<SceneKinds> = MainCell::new(SceneKinds::new());

/// A scene of the game begins (`instance_stream::scene_begin`).
pub fn begin(mt: &MainThread, kind: u32) {
    SCENES.get(mt).begin(kind, crate::capture::ticks());
}

/// The scene in progress ends (`instance_stream::scene_end`).
pub fn end(mt: &MainThread) {
    SCENES.get(mt).end(crate::capture::ticks());
}

/// A display of a retained shape in a scene of `kind` into the back buffer, outside the main
/// view's list: the engine draws it (`instance_stream::skipped_display_timed`).
pub fn hand_back(mt: &MainThread, kind: u32) {
    SCENES.get(mt).handed_back[index(kind)] += 1;
}

/// The displays handed back in all the kinds.
pub fn handed_back(mt: &MainThread) -> u64 {
    SCENES.get(mt).handed_back.iter().sum()
}

/// The end of a frame (`instance_stream::frame_tick`).
pub fn frame_end(mt: &MainThread) {
    SCENES.get(mt).frame_end();
}

/// The `scenes:` line of the report.
pub fn line(mt: &MainThread) -> String {
    SCENES.get(mt).line(crate::capture::ticks_per_second())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_counts_its_time_and_its_kind() {
        let mut s = SceneKinds::new();
        s.begin(5, 1000);
        s.end(1250);
        s.begin(5, 2000);
        s.end(2050);
        s.begin(1, 3000);
        s.end(4000);
        s.frame_end();
        s.begin(5, 5000);
        s.end(5100);
        s.frame_end();
        assert_eq!(s.scenes[5], 3);
        assert_eq!(s.sum_ticks[5], 400);
        assert_eq!(s.worst_ticks[5], 300);
        assert_eq!(s.sum_ticks[1], 1000);
        assert_eq!(s.frames, 2);
    }

    #[test]
    fn a_begin_without_an_end_closes_the_open_scene() {
        let mut s = SceneKinds::new();
        s.begin(1, 0);
        s.begin(6, 400);
        s.end(500);
        s.frame_end();
        assert_eq!(s.sum_ticks[1], 400);
        assert_eq!(s.sum_ticks[6], 100);
        // An end with no scene open counts nothing.
        s.end(900);
        s.frame_end();
        assert_eq!(s.sum_ticks.iter().sum::<i64>(), 500);
    }

    #[test]
    fn the_line_names_the_shadow_kinds_with_zero_and_counts_hand_backs_by_kind() {
        let mut s = SceneKinds::new();
        s.begin(1, 0);
        s.end(1000);
        s.handed_back[6] += 2;
        s.frame_end();
        let line = s.line(1_000_000);
        assert_eq!(
            line,
            "scenes: by kind for each of 1 frames: world 1 scenes, 1.000 ms a frame (worst 1.000), handed back 0; \
             shadow volumes 0 scenes, 0.000 ms a frame (worst 0.000), handed back 0; \
             shadow casters 0 scenes, 0.000 ms a frame (worst 0.000), handed back 0; \
             first person 0 scenes, 0.000 ms a frame (worst 0.000), handed back 2"
        );
        // A kind above the list counts as the last one.
        assert_eq!(index(40), KINDS - 1);
    }
}
