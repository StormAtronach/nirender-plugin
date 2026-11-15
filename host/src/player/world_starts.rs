//! The times of the first world frames (W369): for each start of the world frames, the frame
//! interval after it. The helper prints them as one line in its log at each start:
//! `first world frames (N):`, N the count of all starts, and the times in ms of the last
//! [`KEPT_TIMES`] starts. The rule is that of the D300 probe and `d306_analyse.py`: a start at
//! frame F gives the interval that the start of frame F+1 measures. A start comes after each load,
//! door and menu, so the line keeps a bound.
//!
//! The world flag at the start of frame F is that of frame F-1: `begin_frame` reads `parameters`
//! before it clears them, and a world draw of the frame sets them. So frame F-1 is the first world
//! frame, and the interval at F+1 is the time of frame F, the second world frame \[I\], by reading
//! `begin_frame` and `items.rs`. The rule stays to match `d306_analyse.py` and the D306 numbers.

use std::collections::VecDeque;
use std::time::Instant;

/// The most times in the line: those of the last starts.
pub(crate) const KEPT_TIMES: usize = 32;

/// The world starts and their first frame times. [`FirstWorldFrames::frame_start`] runs at the
/// start of each frame of the player.
#[derive(Default)]
pub(crate) struct FirstWorldFrames {
    /// The frame before had a world scene.
    world: bool,
    /// The start of the frame before.
    last_start: Option<Instant>,
    /// The world frames started at the start of the frame before: this start gives its time.
    started: bool,
    /// The world starts that gave a time, all of them.
    starts: u64,
    /// The times in ms of the last [`KEPT_TIMES`] starts, in their order.
    times_ms: VecDeque<f64>,
}

impl FirstWorldFrames {
    /// Notes the start of a frame at `now`; `world` is true when the frame before had a world
    /// scene. Returns the line of the log when this start gave the time of a world start.
    pub(crate) fn frame_start(&mut self, now: Instant, world: bool) -> Option<String> {
        let interval = self.last_start.map(|before| now.saturating_duration_since(before));
        self.last_start = Some(now);
        let mut line = None;
        if std::mem::take(&mut self.started) {
            if let Some(interval) = interval {
                if self.times_ms.len() == KEPT_TIMES {
                    self.times_ms.pop_front();
                }
                self.times_ms.push_back(interval.as_secs_f64() * 1000.0);
                self.starts += 1;
                line = Some(self.line());
            }
        }
        if world && !self.world {
            self.started = true;
        }
        self.world = world;
        line
    }

    /// `first world frames (N):`, N the count of all starts, and the times of the last
    /// [`KEPT_TIMES`] starts, in ms.
    fn line(&self) -> String {
        let times: Vec<String> = self.times_ms.iter().map(|ms| format!("{ms:.1}")).collect();
        format!("first world frames ({}): {}", self.starts, times.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::FirstWorldFrames;
    use std::time::{Duration, Instant};

    /// Frames at the given ms from a start, each with its world flag: the lines that they give.
    fn lines(frames: &[(u64, bool)]) -> Vec<(usize, String)> {
        let zero = Instant::now();
        let mut starts = FirstWorldFrames::default();
        frames
            .iter()
            .enumerate()
            .filter_map(|(frame, &(ms, world))| {
                starts
                    .frame_start(zero + Duration::from_millis(ms), world)
                    .map(|line| (frame, line))
            })
            .collect()
    }

    /// A start at frame F gives the interval from the start of F to the start of F+1, as
    /// `d306_analyse.py` reads it; each later start adds its time to the line; a frame with no
    /// change of the world gives no line.
    #[test]
    fn each_world_start_gives_the_next_frame_interval() {
        let frames = [
            (0, false),
            (16, false),
            (32, true),  // the world starts at frame 2
            (845, true), // frame 3: 813 ms after frame 2
            (861, true),
            (877, false), // a load
            (900, false),
            (920, true),  // the world starts again at frame 7
            (1040, true), // frame 8: 120 ms
            (1056, true),
        ];
        assert_eq!(
            lines(&frames),
            [
                (3, "first world frames (1): 813.0".to_string()),
                (8, "first world frames (2): 813.0 120.0".to_string())
            ]
        );
    }

    /// No world gives no line. A world at the first frame is a start: no frame before it had a
    /// world. A start with no frame after it gives no line.
    #[test]
    fn no_start_gives_no_line() {
        assert!(lines(&[(0, false), (16, false), (32, false)]).is_empty());
        assert_eq!(
            lines(&[(0, true), (50, true), (66, true)]),
            [(1, "first world frames (1): 50.0".to_string())]
        );
        assert!(
            lines(&[(0, false), (16, true)]).is_empty(),
            "the frame after the start did not come"
        );
    }

    /// Many starts: the line keeps the count of all starts and the times of the last
    /// [`KEPT_TIMES`](super::KEPT_TIMES) only, so its length stays bounded.
    #[test]
    fn the_line_keeps_the_last_times_only() {
        let starts = super::KEPT_TIMES as u64 + 9;
        // Each start: a world frame of start + 1 ms, and a frame without a world. 20 ms apart.
        let mut frames = Vec::new();
        for start in 0..starts {
            let at = start * 100;
            frames.extend([(at, false), (at + 20, true), (at + 21 + start, true)]);
        }
        let all = lines(&frames);
        assert_eq!(all.len(), starts as usize);
        let last = &all.last().unwrap().1;
        let times: Vec<&str> = last.split(": ").nth(1).unwrap().split(' ').collect();
        assert!(last.starts_with(&format!("first world frames ({starts}): ")), "{last}");
        assert_eq!(times.len(), super::KEPT_TIMES, "{last}");
        // The first time kept is that of start 9 (10 ms), the last that of the last start.
        assert_eq!(times[0], "10.0", "{last}");
        assert_eq!(times[super::KEPT_TIMES - 1], format!("{:.1}", starts as f64), "{last}");
        let longest = all.iter().map(|(_, line)| line.len()).max().unwrap();
        assert_eq!(longest, last.len(), "the line does not grow past the bound");
    }
}
