//! The time of the parts of the work, in ticks of the processor's counter.
//!
//! The accounting reads the time many times for each draw and each record
//! (`Counts::times`), so it uses `rdtsc` and not [`Instant::now`], which costs much more. The
//! counter runs at a constant rate on current processors. The rate is not asked of the
//! system: it is the ratio of the ticks to [`Instant`] since the first read, so a report made
//! seconds later has it to better than 1 in 10000.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The ticks and the instant of the first read.
static START: OnceLock<(u64, Instant)> = OnceLock::new();

/// The counter now.
#[inline]
pub fn now() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: `rdtsc` reads a counter; it has no side effect and needs no feature check
        // on x86-64.
        unsafe { core::arch::x86_64::_rdtsc() }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let (_, instant) = *start();
        instant.elapsed().as_nanos() as u64
    }
}

fn start() -> &'static (u64, Instant) {
    START.get_or_init(|| {
        #[cfg(target_arch = "x86_64")]
        let ticks = unsafe { core::arch::x86_64::_rdtsc() };
        #[cfg(not(target_arch = "x86_64"))]
        let ticks = 0;
        (ticks, Instant::now())
    })
}

/// A moment, for the time of a part: `elapsed` is the ticks since it.
#[derive(Clone, Copy)]
pub struct Clock(u64);

impl Clock {
    #[inline]
    pub fn now() -> Clock {
        start();
        Clock(now())
    }

    #[inline]
    pub fn elapsed(&self) -> u64 {
        now().saturating_sub(self.0)
    }
}

/// The ticks of a second: the ratio of the counter to [`Instant`] since the first read. Within
/// 20 ms of the first read it waits for the rest of those 20 ms, so that the ratio has four
/// digits.
fn rate() -> f64 {
    let (ticks0, instant0) = *start();
    loop {
        let elapsed = instant0.elapsed();
        if elapsed >= Duration::from_millis(20) {
            return (now() - ticks0) as f64 / elapsed.as_secs_f64();
        }
        std::thread::sleep(Duration::from_millis(20) - elapsed);
    }
}

/// Ticks as a duration.
pub fn duration(ticks: u64) -> Duration {
    Duration::from_secs_f64(ticks as f64 / rate())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ticks_of_a_sleep_match_the_instant() {
        let clock = Clock::now();
        let instant = Instant::now();
        std::thread::sleep(Duration::from_millis(30));
        let ticks = duration(clock.elapsed());
        let real = instant.elapsed();
        let error = ticks.abs_diff(real);
        assert!(error < Duration::from_millis(2), "ticks {ticks:?}, instant {real:?}");
    }

    #[test]
    fn a_clock_does_not_go_back() {
        let clock = Clock::now();
        assert!(clock.elapsed() <= clock.elapsed());
    }
}
