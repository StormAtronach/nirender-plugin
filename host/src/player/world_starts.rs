//! The times of the first world frames (W369): for each start of the world frames, the frame
//! interval after it. The helper prints them as one line in its log, "first world frames:" and a
//! time in ms for each start so far, by the rule of the D300 probe and `d306_analyse.py`: a start
//! at frame F gives the interval that the start of frame F+1 measures.
//!
//! The world flag at the start of frame F is that of frame F-1: `begin_frame` reads `parameters`
//! before it clears them, and a world draw of the frame sets them. So frame F-1 is the first world
//! frame, and the interval at F+1 is the time of frame F, the second world frame \[I\], by reading
//! `begin_frame` and `items.rs`. The rule stays to match `d306_analyse.py` and the D306 numbers.

use std::time::Instant;

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
    /// The times in ms, one for each world start, in their order.
    times_ms: Vec<f64>,
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
                self.times_ms.push(interval.as_secs_f64() * 1000.0);
                line = Some(self.line());
            }
        }
        if world && !self.world {
            self.started = true;
        }
        self.world = world;
        line
    }

    /// "first world frames:" and the time of each world start so far, in ms.
    fn line(&self) -> String {
        let times: Vec<String> = self.times_ms.iter().map(|ms| format!("{ms:.1}")).collect();
        format!("first world frames: {}", times.join(" "))
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
                (3, "first world frames: 813.0".to_string()),
                (8, "first world frames: 813.0 120.0".to_string())
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
            [(1, "first world frames: 50.0".to_string())]
        );
        assert!(
            lines(&[(0, false), (16, true)]).is_empty(),
            "the frame after the start did not come"
        );
    }
}
