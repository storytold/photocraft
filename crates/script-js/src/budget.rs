//! The run's time budget: a cooperative timeout checked on host calls.
//!
//! In-process it is not a hard limit: a long loop with no host calls overshoots it. The script
//! host ([`crate::isolate`]) adds the hard limit.

use std::sync::Arc;
use std::time::{Duration, Instant};

/// Decides whether a run may continue. Charged before every host call, after each one but a
/// dialog, once per promise job and once as the run starts.
pub trait Budget {
    /// `false` stops the run with a time-limit error. Only called while the run executes
    /// (between `resume` and `pause`).
    fn charge(&mut self) -> bool;
    /// The run stopped executing, e.g. at a dialog. Time until `resume` is not charged.
    fn pause(&mut self) {}
    /// The run executes again.
    fn resume(&mut self) {}
}

/// No limit.
#[derive(Debug, Default, Clone, Copy)]
pub struct Unlimited;

impl Budget for Unlimited {
    fn charge(&mut self) -> bool {
        true
    }
}

/// A monotonic time source for [`WallClock`].
pub trait Now: Send + Sync {
    /// Time since an arbitrary fixed start.
    fn now(&self) -> Duration;
}

/// The system's monotonic clock.
#[derive(Debug)]
pub struct SystemNow(Instant);

impl Default for SystemNow {
    fn default() -> Self {
        Self(Instant::now())
    }
}

impl Now for SystemNow {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

/// A wall-clock limit on the time the run spends executing. Time while the run waits at a dialog
/// does not count.
#[derive(Clone)]
pub struct WallClock {
    spent: Duration,
    running_since: Option<Duration>,
    limit: Duration,
    clock: Arc<dyn Now>,
}

impl std::fmt::Debug for WallClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WallClock").field("spent", &self.spent).field("running", &self.running_since.is_some()).field("limit", &self.limit).finish()
    }
}

impl WallClock {
    /// A limit on the system clock.
    pub fn new(limit: Duration) -> Self {
        Self::with_clock(limit, Arc::new(SystemNow::default()))
    }

    /// A limit on `clock` (tests pass a clock they move by hand).
    pub fn with_clock(limit: Duration, clock: Arc<dyn Now>) -> Self {
        Self { spent: Duration::ZERO, running_since: None, limit, clock }
    }

    fn elapsed(&self) -> Duration {
        self.spent + self.running_since.map_or(Duration::ZERO, |t| self.clock.now().saturating_sub(t))
    }
}

impl Budget for WallClock {
    fn charge(&mut self) -> bool {
        // Charged while paused (a caller's bug), it counts the time up to the pause only.
        self.elapsed() <= self.limit
    }

    fn pause(&mut self) {
        if let Some(t) = self.running_since.take() {
            self.spent += self.clock.now().saturating_sub(t);
        }
    }

    fn resume(&mut self) {
        if self.running_since.is_none() {
            self.running_since = Some(self.clock.now());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Manual(Mutex<Duration>);

    impl Manual {
        fn advance(&self, by: Duration) {
            *self.0.lock().unwrap() += by;
        }
    }

    impl Now for Manual {
        fn now(&self) -> Duration {
            *self.0.lock().unwrap()
        }
    }

    #[test]
    fn paused_time_is_not_charged() {
        let clock = Arc::new(Manual::default());
        let mut b = WallClock::with_clock(Duration::from_millis(50), clock.clone());
        b.resume();
        clock.advance(Duration::from_millis(30));
        assert!(b.charge());
        b.pause();
        clock.advance(Duration::from_secs(10));
        b.resume();
        assert!(b.charge(), "time at the pause is not counted");
        clock.advance(Duration::from_millis(30));
        assert!(!b.charge(), "30 + 30 ms of running time is over 50 ms");
    }
}
