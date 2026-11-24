//! The load watch (W640 step 1a; the Architect's design `designs/w634-load-readiness-2026-11-24.md`,
//! section 2). With `MGE_XE_LOAD_WATCH=1`, the log gets one line for each frame of the helper
//! around a load, for the measurement of the first seconds after the load. The switch is off by
//! default. Without it, nothing of this module runs.
//!
//! The window: a frame with no world (the loading screen, or the menu before a load) opens it.
//! It stays open until [`WINDOW_AFTER`] after the last frame with no world. With
//! `distant_land.wait_on_load`, MGE XE holds the loading screen until its ARRIVED, so the window
//! ends about 20 s after ARRIVED. A frame with no world in play (a menu over a kept picture)
//! opens the window again [I].
//!
//! The line comes at the end of the frame, after its present:
//! `load watch: frame N, world yes|no, interval A ms, busy B ms, gpu C ms in k frames read;
//! uber-shader draws with a stage key: ...; pipelines given back: ...; the list: ...`.
//!
//! - N: the renderer's count of the frames with this one. It is the `frame` of the scene that
//!   [`crate::Extension::after_frame`] gets; a scene in the frame ([`crate::Extension::frame`])
//!   has N - 1. A line that the extension prints during the frame comes before the frame's line
//!   in the log.
//! - interval: from the end of the frame before to the end of this one. busy: the timed parts of
//!   the helper in the frame (the busy time of the game's pacing).
//! - gpu: with `MGE_XE_GPU_TIME=1`, the GPU's frame times that the frame read back, and their
//!   count. They are of earlier frames: the read-back is a few frames late. Without the switch,
//!   "gpu off".
//! - the draws of the frame that the uber-shader drew with a stage key, by cause
//!   ([`crate::stage_shaders::UberDraws`]).
//! - the pipelines that the stage thread gave back in the frame, by origin, with the thread's
//!   time for them ([`crate::stage_shaders::Received`]).
//! - the list: its pipelines not sent to the thread yet, and those sent and not given back.
//!
//! The end of a window gives one more line: `load watch: the window ends at frame N`.

use std::time::{Duration, Instant};

use crate::stage_shaders::{Received, UberDraws};

/// The time that the window stays open after the last frame with no world: the 10 s after a
/// load and 10 s more as the steady state (the design, section 2, step 1b).
pub(crate) const WINDOW_AFTER: Duration = Duration::from_secs(20);

/// The facts of a frame at its end, for [`LoadWatch::frame_end`].
pub(crate) struct FrameFacts {
    /// The renderer's count of the frames with this one.
    pub(crate) frame: u64,
    /// The frame drew the world.
    pub(crate) world: bool,
    /// The timed parts of the helper in the frame, in ms.
    pub(crate) busy_ms: f64,
    /// With `MGE_XE_GPU_TIME=1`: the sum of the GPU's frame times read back in the session, in
    /// ms, and their count.
    pub(crate) gpu: Option<(f64, u32)>,
    /// The pipelines that the stage thread gave back in the frame.
    pub(crate) received: Received,
    /// The draws on the uber-shader of the session.
    pub(crate) draws: UberDraws,
    /// The list's pipelines not sent, and sent and not given back.
    pub(crate) list_left: (usize, usize),
}

/// The state of the watch between two frames.
#[derive(Default)]
pub(crate) struct LoadWatch {
    /// The end of the frame before.
    last_end: Option<Instant>,
    /// The end of the open window.
    open_until: Option<Instant>,
    /// The counts of the session at the end of the frame before.
    draws: UberDraws,
    gpu: (f64, u32),
}

impl LoadWatch {
    /// The lines of the frame that ends at `now`: none outside the window, the frame's line in
    /// it, and the line of the window's end after the last frame of a window.
    pub(crate) fn frame_end(&mut self, now: Instant, facts: &FrameFacts) -> Vec<String> {
        let interval = self.last_end.map(|before| now.saturating_duration_since(before));
        self.last_end = Some(now);
        let draws = draws_since(&self.draws, &facts.draws);
        self.draws = facts.draws;
        let gpu = facts.gpu.map(|(sum, count)| {
            let read = (sum - self.gpu.0, count.wrapping_sub(self.gpu.1));
            self.gpu = (sum, count);
            read
        });
        if !facts.world {
            self.open_until = Some(now + WINDOW_AFTER);
        }
        let Some(until) = self.open_until else {
            return Vec::new();
        };
        if now > until {
            self.open_until = None;
            return vec![format!("load watch: the window ends at frame {}", facts.frame)];
        }
        vec![frame_line(facts, interval, gpu, &draws)]
    }
}

/// The draws of each cause between two counts of the session.
fn draws_since(before: &UberDraws, now: &UberDraws) -> UberDraws {
    UberDraws {
        not_made: now.not_made.saturating_sub(before.not_made),
        failed: now.failed.saturating_sub(before.failed),
        refused: now.refused.saturating_sub(before.refused),
        past_cap: now.past_cap.saturating_sub(before.past_cap),
        no_key: now.no_key.saturating_sub(before.no_key),
    }
}

/// The line of a frame in the window.
fn frame_line(facts: &FrameFacts, interval: Option<Duration>, gpu: Option<(f64, u32)>, draws: &UberDraws) -> String {
    let ms = |duration: Duration| duration.as_secs_f64() * 1000.0;
    let interval = interval.map_or("none".to_string(), |interval| format!("{:.2} ms", ms(interval)));
    let gpu = gpu.map_or("gpu off".to_string(), |(sum, count)| {
        format!("gpu {sum:.3} ms in {count} frames read")
    });
    let received = &facts.received;
    format!(
        "load watch: frame {}, world {}, interval {interval}, busy {:.2} ms, {gpu}; uber-shader draws with a stage key: {} not made, {} after a failed pipeline, {} after a failed rewrite, {} past the cap, {} with no key; pipelines given back: {} of the uber set, {} of the list, {} of first use, {} failed, in {:.2} ms of the thread; the list: {} not sent, {} sent and not given back",
        facts.frame,
        if facts.world { "yes" } else { "no" },
        facts.busy_ms,
        draws.not_made,
        draws.failed,
        draws.refused,
        draws.past_cap,
        draws.no_key,
        received.uber_set,
        received.list,
        received.first_use,
        received.failed,
        ms(received.thread),
        facts.list_left.0,
        facts.list_left.1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(frame: u64, world: bool, not_made: u64, gpu: Option<(f64, u32)>) -> FrameFacts {
        FrameFacts {
            frame,
            world,
            busy_ms: 2.5,
            gpu,
            received: Received {
                list: 2,
                thread: Duration::from_millis(7),
                ..Received::default()
            },
            draws: UberDraws {
                not_made,
                ..UberDraws::default()
            },
            list_left: (5, 3),
        }
    }

    /// No line before a frame with no world; a line for each frame from it to 20 s after the
    /// last such frame; then the line of the end, and none after it until the next such frame.
    #[test]
    fn the_window_opens_at_a_frame_with_no_world_and_ends_20_s_after_the_last() {
        let zero = Instant::now();
        let at = |ms: u64| zero + Duration::from_millis(ms);
        let mut watch = LoadWatch::default();
        assert!(watch.frame_end(at(0), &facts(1, true, 0, None)).is_empty());
        let first = watch.frame_end(at(16), &facts(2, false, 0, None));
        assert_eq!(first.len(), 1);
        assert!(
            first[0].starts_with("load watch: frame 2, world no, interval 16.00 ms, busy 2.50 ms, gpu off;"),
            "{}",
            first[0]
        );
        // The world comes back; the window stays open until 20 s after frame 3, the last with no world.
        assert_eq!(watch.frame_end(at(32), &facts(3, false, 0, None)).len(), 1);
        assert_eq!(watch.frame_end(at(48), &facts(4, true, 0, None)).len(), 1);
        assert_eq!(watch.frame_end(at(20_032), &facts(5, true, 0, None)).len(), 1);
        assert_eq!(
            watch.frame_end(at(20_048), &facts(6, true, 0, None)),
            ["load watch: the window ends at frame 6"]
        );
        assert!(watch.frame_end(at(20_064), &facts(7, true, 0, None)).is_empty());
        assert_eq!(watch.frame_end(at(30_000), &facts(8, false, 0, None)).len(), 1);
    }

    /// The draws and the GPU times are those of the frame: the counts of the session less those
    /// of the frame before, also of a frame outside the window.
    #[test]
    fn the_counts_are_those_of_the_frame() {
        let zero = Instant::now();
        let mut watch = LoadWatch::default();
        assert!(watch.frame_end(zero, &facts(1, true, 10, Some((30.0, 10)))).is_empty());
        let line = watch.frame_end(zero + Duration::from_millis(20), &facts(2, false, 14, Some((34.5, 12))));
        assert_eq!(
            line,
            ["load watch: frame 2, world no, interval 20.00 ms, busy 2.50 ms, gpu 4.500 ms in 2 frames read; uber-shader draws with a stage key: 4 not made, 0 after a failed pipeline, 0 after a failed rewrite, 0 past the cap, 0 with no key; pipelines given back: 0 of the uber set, 2 of the list, 0 of first use, 0 failed, in 7.00 ms of the thread; the list: 5 not sent, 3 sent and not given back"]
        );
    }

    /// The first frame of the session has no interval.
    #[test]
    fn the_first_frame_has_no_interval() {
        let mut watch = LoadWatch::default();
        let line = watch.frame_end(Instant::now(), &facts(1, false, 0, None));
        assert!(
            line[0].starts_with("load watch: frame 1, world no, interval none, "),
            "{}",
            line[0]
        );
    }
}
