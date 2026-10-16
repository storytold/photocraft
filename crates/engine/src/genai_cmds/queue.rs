//! Generative Fill's turns at the provider (this fork): at most [`MAX_PARALLEL`] fills (and
//! Generate More requests) talk to it at once; the others wait in the order they were started.
//! A request the service turns down for its rate limit is sent again after a pause.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, PoisonError};

use photocraft_genai::{EditRequest, GenError, GeneratedImage, ImageProvider};

use crate::jobs::JobCtx;
use crate::{EngineError, Result};

/// Most fills that talk to the provider at the same time. Each sends one request per variation,
/// and the service limits how many it takes a minute.
pub const MAX_PARALLEL: usize = 5;

/// How often a waiting fill looks at its place in line and at Cancel, in milliseconds.
const POLL_MS: u64 = 100;

/// Pauses before sending a request again after the service answered that it is over its rate
/// limit (HTTP 429), in milliseconds. Then the error stands.
#[cfg(not(test))]
const BUSY_PAUSES_MS: [u64; 3] = [4_000, 8_000, 16_000];
#[cfg(test)]
const BUSY_PAUSES_MS: [u64; 3] = [5, 10, 20];

#[derive(Default)]
struct Line {
    /// Fills holding a turn.
    busy: usize,
    /// Tickets of the fills waiting, first in line first.
    waiting: VecDeque<u64>,
    next: u64,
}

/// The provider's turns, shared by every fill of a session.
pub(crate) struct Turns {
    max: usize,
    line: Mutex<Line>,
    changed: Condvar,
}

impl Default for Turns {
    fn default() -> Self {
        Turns::new(MAX_PARALLEL)
    }
}

/// A turn at the provider: given back when dropped.
pub(crate) struct Turn<'a>(&'a Turns);

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        let mut line = self.0.line.lock().unwrap_or_else(PoisonError::into_inner);
        line.busy = line.busy.saturating_sub(1);
        self.0.changed.notify_all();
    }
}

impl Turns {
    pub(crate) fn new(max: usize) -> Self {
        Turns { max: max.max(1), line: Mutex::default(), changed: Condvar::new() }
    }

    /// Wait for a turn. While waiting, `queued` holds the place in line (1 = next); it is 0
    /// once the turn comes. `Err(Cancelled)` if the job is cancelled meanwhile.
    pub(crate) fn take(&self, ctx: &JobCtx, queued: &AtomicUsize) -> Result<Turn<'_>> {
        let mut line = self.line.lock().unwrap_or_else(PoisonError::into_inner);
        let ticket = line.next;
        line.next = line.next.wrapping_add(1);
        line.waiting.push_back(ticket);
        loop {
            let place = line.waiting.iter().position(|t| *t == ticket).unwrap_or(0);
            // The web runs jobs inline, one at a time: nothing else could give a turn back.
            if place == 0 && (line.busy < self.max || cfg!(target_arch = "wasm32")) {
                line.waiting.pop_front();
                line.busy += 1;
                queued.store(0, Ordering::Relaxed);
                // The next in line may have a turn too.
                self.changed.notify_all();
                return Ok(Turn(self));
            }
            if ctx.cancelled() {
                line.waiting.retain(|t| *t != ticket);
                queued.store(0, Ordering::Relaxed);
                self.changed.notify_all();
                return Err(EngineError::Cancelled);
            }
            queued.store(place + 1, Ordering::Relaxed);
            line = wait(&self.changed, line);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn wait<'a>(changed: &Condvar, line: std::sync::MutexGuard<'a, Line>) -> std::sync::MutexGuard<'a, Line> {
    changed.wait_timeout(line, std::time::Duration::from_millis(POLL_MS)).unwrap_or_else(PoisonError::into_inner).0
}

#[cfg(target_arch = "wasm32")]
fn wait<'a>(_: &Condvar, line: std::sync::MutexGuard<'a, Line>) -> std::sync::MutexGuard<'a, Line> {
    line
}

/// `provider.edit(request)`, sent again after a pause when the service says it is over its rate
/// limit. False from `pause` (cancelled) stops early.
pub(crate) fn edit(ctx: &JobCtx, provider: &dyn ImageProvider, request: &EditRequest) -> photocraft_genai::Result<GeneratedImage> {
    let mut r = provider.edit(request);
    for ms in BUSY_PAUSES_MS {
        if !matches!(r, Err(GenError::Quota)) {
            break;
        }
        ctx.progress(0.0, "The image service is busy; trying again");
        if !pause(ctx, ms) {
            return Err(GenError::Cancelled);
        }
        r = provider.edit(request);
    }
    r
}

/// Sleep `ms`, looking at Cancel as it goes. False if cancelled.
#[cfg(not(target_arch = "wasm32"))]
fn pause(ctx: &JobCtx, ms: u64) -> bool {
    let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    while std::time::Instant::now() < end {
        if ctx.cancelled() {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(POLL_MS.min(ms)));
    }
    !ctx.cancelled()
}

#[cfg(target_arch = "wasm32")]
fn pause(ctx: &JobCtx, _ms: u64) -> bool {
    !ctx.cancelled()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn turns_go_in_order_and_never_past_the_limit() {
        let turns = Arc::new(Turns::new(2));
        let ctx = JobCtx::new();
        let q = AtomicUsize::new(0);
        let a = turns.take(&ctx, &q).unwrap();
        let _b = turns.take(&ctx, &q).unwrap();
        // A third waits until one is given back, and says where it stands meanwhile.
        let waiting = Arc::new(AtomicUsize::new(0));
        let (t, w) = (Arc::clone(&turns), Arc::clone(&waiting));
        let third = std::thread::spawn(move || {
            let ctx = JobCtx::new();
            t.take(&ctx, &w).map(|_| ()).is_ok()
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while waiting.load(Ordering::Relaxed) != 1 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(waiting.load(Ordering::Relaxed), 1, "first in line");
        assert!(!third.is_finished());
        drop(a);
        assert!(third.join().unwrap());
        assert_eq!(waiting.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_cancelled_wait_leaves_the_line() {
        let turns = Turns::new(1);
        let q = AtomicUsize::new(0);
        let _held = turns.take(&JobCtx::new(), &q).unwrap();
        let ctx = JobCtx::new();
        ctx.cancel();
        assert!(matches!(turns.take(&ctx, &q), Err(EngineError::Cancelled)));
        assert!(turns.line.lock().unwrap().waiting.is_empty());
    }
}
