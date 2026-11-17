//! Running a script in its own process, so that no script can take the app down.
//!
//! The engine has no interrupt hook and no heap limit, and some inputs overflow the native stack
//! or exhaust memory before any in-process check can run. A run therefore executes in a script
//! host: the same executable started with [`SCRIPT_HOST_FLAG`], which calls [`serve`] before
//! anything else. The app side, [`IsolatedJob`], has the same shape as the in-process job
//! (`spawn`, `poll`, `reply`, `cancel`, `is_finished`, with the same events and replies), so an
//! interface switches by changing the type it holds.
//!
//! The app enforces the limits; nothing the script does inside the host can lift them:
//! - **Time:** the app's watchdog kills the host once the run has executed longer than its time
//!   limit. The clock stops while a dialog is open.
//! - **Memory:** see [`limits`] for each platform.
//! - **Requests:** the host is untrusted. Its frames are size-capped, must arrive in order, and
//!   may only ask for the commands in [`protocol::COMMANDS`] with exactly their parameters;
//!   anything else stops the run as a [`FailureKind::HostViolation`]. Its commands always carry
//!   the run's own history key.
//! - **Lifetime:** one host per run, kept across the replays that follow dialogs. A host that
//!   ends without a result is classified from what the app knows ([`FailureKind`]): stopped for
//!   time, for memory (stopped by the app, a failed allocation, or a peak near the limit), or
//!   crashed with its exit status, stack overflows and access violations named as such. A request
//!   still queued once the host's output has ended, it sent its result, it exited or the app
//!   stopped it is dropped. A request the interface already took while the host was running is
//!   one the run validly made, and the interface may still carry it out; nothing of the run
//!   arrives after [`Event::Finished`]. The run's history step needs no closing: its key is the
//!   run's alone, so no later edit joins it.
//!
//! The web build has no processes. Its isolation runs the same [`protocol`] messages over
//! `postMessage` to a dedicated Worker that holds the engine, behind the same job interface: the
//! Worker cannot block for an answer, so a command or dialog ends the attempt there and the next
//! attempt replays with the answer, as dialogs already do; the time limit calls the Worker's
//! `terminate()`, and the Worker's memory is a WebAssembly memory with a fixed maximum. WebAssembly
//! builds abort on panic, so the in-process panic guard in [`crate::Runner::step`] does nothing
//! there: the Worker boundary is what turns an engine panic into a failed run.
//!
//! Until that supervisor exists the web build runs scripts in-process only, and it has no
//! watchdog: the in-process time budget is checked on host calls, so a pure-compute loop that
//! makes none (for example `for (;;) new Date()`) runs past it. The Worker is the follow-up that
//! bounds such loops there.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(not(target_arch = "wasm32"))]
mod host;
#[cfg(not(target_arch = "wasm32"))]
pub mod limits;
pub mod protocol;
#[cfg(not(target_arch = "wasm32"))]
mod supervisor;

#[cfg(not(target_arch = "wasm32"))]
pub use host::{STACK, serve};
#[cfg(not(target_arch = "wasm32"))]
pub use supervisor::{
    CANCELLED, Crash, CrashCause, DEFAULT_MEMORY_LIMIT, Event, Failure, FailureKind, HelperCommand, IsolateOptions, IsolatedJob, MIN_MEMORY_LIMIT, Reply,
};

/// The first argument that makes the executable a script host.
pub const SCRIPT_HOST_FLAG: &str = "--script-host";

/// Whether this process was started as a script host. Check it first thing in `main`, before
/// any window, logging or single-instance handling, and then exit with [`serve`]'s status.
pub fn script_host_requested() -> bool {
    std::env::args_os().nth(1).is_some_and(|a| a == SCRIPT_HOST_FLAG)
}
