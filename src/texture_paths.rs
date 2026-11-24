//! The cost of the two paths of a freed device texture (W454). Path A: a texture's renderer data
//! destroyed on the main thread raises the generation of the instance tables
//! (`instance_stream::textures_changed`), and the next display of every retained instance notes
//! its handles. Path B: the cell worker's freed textures (`material_stream::drain_freed_textures`)
//! send `MATERIAL_GONE` for their materials and mark only the instances of those materials for a
//! note. Each path keeps its calls, the displays that it alone made due, their time with the
//! time of the drain itself, and the largest time in one frame with that frame's number (the
//! frames of the instance tables). The time of a frame also goes to `nirender.displayCost()`, so
//! that a batch that reads it in each frame has the cost of each path in its own frames.

use core::sync::atomic::{AtomicI64, AtomicU64, Ordering::Relaxed};

use crate::main_thread::{MainCell, MainThread};

/// Path A: a raise of the generation. Path B: a freed texture of the cell worker.
pub const RAISE: usize = 0;
pub const FREED: usize = 1;
const PATHS: usize = 2;

/// The counts and the times of one path.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PathTimes {
    /// Path A: the raises of the generation. Path B: the drains that found a freed texture.
    pub calls: u64,
    /// The displays that only this path made due, and that noted their handles.
    pub notes: u64,
    /// The time of the notes and of the drain, in ticks of the performance counter.
    pub ticks: i64,
    /// The ticks of the frame `frame` so far.
    frame_ticks: i64,
    frame: u32,
    /// The largest time of one frame, and that frame.
    pub max_ticks: i64,
    pub max_frame: u32,
}

impl PathTimes {
    /// Adds `ticks` of the path in `frame`.
    pub fn add(&mut self, frame: u32, ticks: i64) {
        if frame != self.frame {
            self.frame = frame;
            self.frame_ticks = 0;
        }
        self.ticks += ticks;
        self.frame_ticks += ticks;
        if self.frame_ticks > self.max_ticks {
            self.max_ticks = self.frame_ticks;
            self.max_frame = frame;
        }
    }
}

static TIMES: MainCell<[PathTimes; PATHS]> = MainCell::new(
    [PathTimes {
        calls: 0,
        notes: 0,
        ticks: 0,
        frame_ticks: 0,
        frame: 0,
        max_ticks: 0,
        max_frame: 0,
    }; PATHS],
);

/// The ticks of each path and the raises since the last `take_frame` (`display_cost`).
static FRAME_TICKS: [AtomicI64; PATHS] = [AtomicI64::new(0), AtomicI64::new(0)];
static FRAME_RAISES: AtomicU64 = AtomicU64::new(0);

/// A raise of the generation (path A).
pub fn raise(mt: &MainThread) {
    TIMES.get(mt)[RAISE].calls += 1;
    FRAME_RAISES.fetch_add(1, Relaxed);
}

/// A drain that found a freed texture (path B), and its time.
pub fn drain(mt: &MainThread, frame: u32, ticks: i64) {
    let mut times = TIMES.get(mt);
    times[FREED].calls += 1;
    times[FREED].add(frame, ticks);
    FRAME_TICKS[FREED].fetch_add(ticks, Relaxed);
}

/// A display that only `path` made due noted its handles in `ticks`.
pub fn note(mt: &MainThread, path: usize, frame: u32, ticks: i64) {
    let mut times = TIMES.get(mt);
    times[path].notes += 1;
    times[path].add(frame, ticks);
    FRAME_TICKS[path].fetch_add(ticks, Relaxed);
}

/// The ms of path A and of path B, and the raises, since the last call; the counters reset.
pub fn take_frame() -> (f64, f64, u64) {
    let ms = |ticks: i64| ticks as f64 * 1000.0 / crate::capture::ticks_per_second() as f64;
    (
        ms(FRAME_TICKS[RAISE].swap(0, Relaxed)),
        ms(FRAME_TICKS[FREED].swap(0, Relaxed)),
        FRAME_RAISES.swap(0, Relaxed),
    )
}

/// The line of the report (`instance_stream::report`).
pub fn report_line(mt: &MainThread) -> String {
    let times = *TIMES.get(mt);
    let ms = |ticks: i64| ticks as f64 * 1000.0 / crate::capture::ticks_per_second() as f64;
    let [a, b] = times;
    format!(
        "texture paths: A (a raise of the generation): {} raises, {} displays noted, {:.3} ms, the largest frame {:.3} ms (frame {}); B (a freed texture of the cell worker): {} drains, {} displays noted, {:.3} ms with the drains, the largest frame {:.3} ms (frame {})\n",
        a.calls,
        a.notes,
        ms(a.ticks),
        ms(a.max_ticks),
        a.max_frame,
        b.calls,
        b.notes,
        ms(b.ticks),
        ms(b.max_ticks),
        b.max_frame
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_keeps_its_total_and_its_largest_frame() {
        let mut times = PathTimes::default();
        // Frame 3: 2 + 5 = 7; frame 4: 6; frame 9: 1 + 1 + 1 = 3; frame 3 again later: 1 only.
        for (frame, ticks) in [(3, 2), (3, 5), (4, 6), (9, 1), (9, 1), (9, 1), (3, 1)] {
            times.add(frame, ticks);
        }
        assert_eq!(times.ticks, 17);
        assert_eq!((times.max_ticks, times.max_frame), (7, 3));
        // A later frame that passes the largest takes its place.
        times.add(10, 4);
        times.add(10, 4);
        assert_eq!((times.max_ticks, times.max_frame), (8, 10));
    }

    #[test]
    fn a_path_with_no_calls_is_all_zero() {
        let times = PathTimes::default();
        assert_eq!(
            (times.calls, times.notes, times.ticks, times.max_ticks, times.max_frame),
            (0, 0, 0, 0, 0)
        );
    }

    #[test]
    fn a_frame_of_no_time_is_not_the_largest() {
        let mut times = PathTimes::default();
        times.add(5, 0);
        assert_eq!((times.max_ticks, times.max_frame), (0, 0));
    }
}
