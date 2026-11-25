//! Running a script in its own process, so that no script can take the app down.
//!
//! The engine has no interrupt hook and no heap limit, and some inputs overflow the native stack
//! or exhaust memory before any in-process check can run. A run therefore executes in a script
//! host: the same executable started with [`SCRIPT_HOST_FLAG`], which calls `serve` before
//! anything else. The app side, `IsolatedJob`, has the same shape as the in-process job
//! (`spawn`, `poll`, `reply`, `cancel`, `is_finished`, with the same events and replies), so an
//! interface switches by changing the type it holds. (`serve`, `IsolatedJob`, `limits`, `Event`
//! and `FailureKind` are desktop-only, so they are named here without links: the web build has
//! none of them.)
//!
//! The app enforces the limits; nothing the script does inside the host can lift them:
//! - **Time:** the app's watchdog kills the host once the run has executed longer than its time
//!   limit. The clock stops while a dialog is open.
//! - **Memory:** the host's heap is capped on every platform: an allocation past the limit fails
//!   at once. See `limits` for what each platform adds.
//! - **Requests:** the host is untrusted. Its frames are size-capped, must arrive in order, and
//!   may only ask for the commands in [`protocol::COMMANDS`] with exactly their parameters;
//!   anything else stops the run as a `FailureKind::HostViolation`. Its commands always carry
//!   the run's own history key.
//! - **Lifetime:** one host per run, kept while the script waits for dialog replies (it does
//!   not replay the script). A host that ends without a result is classified from what the app
//!   knows (`FailureKind`): stopped for time, for memory (stopped by the app, a failed
//!   allocation, or a peak near the limit), or crashed with its exit status, stack overflows and
//!   access violations named as such. A request
//!   still queued once the host's output has ended, it sent its result, it exited or the app
//!   stopped it is dropped. A request the interface already took while the host was running is
//!   one the run validly made, and the interface may still carry it out; nothing of the run
//!   arrives after `Event::Finished`. The run's history step needs no closing: its key is the
//!   run's alone, so no later edit joins it.
//!
//! The web build has no processes. Its isolation runs the same [`protocol`] messages over
//! `postMessage` to a dedicated Worker that holds the engine; [`web_job`] is the page's side,
//! checking every request as the desktop supervisor does. The Worker waits for each answer in
//! shared memory, so a run continues where it was after a command or a dialog. The page checks
//! the time limit each time it polls the run (every frame while one runs; while messages from a
//! large batch wait to be checked, by each one's arrival time) and calls the Worker's
//! `terminate()` on the first poll past it, and the Worker's heap is capped the way the script
//! host's is. A page that can't be cross-origin isolated runs no scripts.
//! WebAssembly builds abort on panic, so the in-process panic guard in [`crate::Runner::step`]
//! does nothing there: the Worker boundary is what turns an engine panic into a failed run.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod heap;
#[cfg(not(target_arch = "wasm32"))]
mod host;
#[cfg(not(target_arch = "wasm32"))]
pub mod limits;
mod link;
pub mod protocol;
#[cfg(not(target_arch = "wasm32"))]
mod supervisor;
pub mod web_job;
#[cfg(target_arch = "wasm32")]
pub mod worker;

pub use heap::{HostAllocator, host_allocator};
#[cfg(not(target_arch = "wasm32"))]
pub use host::{STACK, serve};
#[cfg(not(target_arch = "wasm32"))]
pub use supervisor::{
    COMMAND_REPLY_LIMIT, Crash, CrashCause, DEFAULT_MEMORY_LIMIT, DIALOG_REPLY_LIMIT, Event, Failure, FailureKind, HelperCommand, IsolateOptions, IsolatedJob,
    MIN_MEMORY_LIMIT, QUEUE_GRACE, Reply,
};

#[cfg(target_arch = "wasm32")]
pub use worker::{WorkerTransport, cross_origin_isolated, serve_worker};

/// The message a cancelled run ends with.
pub const CANCELLED: &str = "The script was cancelled.";

/// The first argument that makes the executable a script host.
pub const SCRIPT_HOST_FLAG: &str = "--script-host";

/// Whether this process was started as a script host. Check it first thing in `main`, before
/// any window, logging or single-instance handling, and then exit with `serve`'s status.
pub fn script_host_requested() -> bool {
    std::env::args_os().nth(1).is_some_and(|a| a == SCRIPT_HOST_FLAG)
}
