//! Script runner for `.js` / `.jsx` files.
//!
//! A run executes in a [`boa_engine::Context`]. Every effect that leaves the engine (a session
//! command, a line of output, a clock read, a random number, a modal dialog) goes through a
//! [`tape::Tape`]. A modal dialog that has no answer yet pauses the run: the attempt is aborted
//! with an error scripts cannot catch, the UI collects the answer, and the next attempt replays
//! the tape in a fresh context. Recorded host calls are answered from the tape and never
//! re-applied, so every effect happens exactly once.
//!
//! The time budget ([`budget`]) is a cooperative timeout, not a hard limit or a security
//! boundary: it is checked on host calls, and on Boa 0.22 a long loop with no host calls can
//! overshoot it (the loop iteration limit counts per call frame).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod abort;
pub mod budget;
pub mod commands;
mod descriptor;
mod dom;
pub mod files;
pub mod grants;
pub mod host;
pub mod isolate;
mod jobs;
mod natives;
pub mod preprocess;
mod runner;
mod scan;
pub mod scriptui;
pub mod session_host;
pub mod tape;
mod unit_value;
mod value;

pub use budget::{Budget, Now, SystemNow, Unlimited, WallClock};
pub use host::{Host, HostError};
/// Most promise reactions a script may have waiting at once.
pub use jobs::MAX_QUEUED as MAX_QUEUED_REACTIONS;
pub use runner::{Modal, ModalKind, RunOptions, Runner, Step};

/// `app.name` when the run has no override.
pub const APP_NAME: &str = "PhotoCraft";
/// Compatibility level existing scripts check to decide which features are available.
/// This is the scripting interface version, not PhotoCraft's release version. `app.name` stays "PhotoCraft".
pub const APP_VERSION: &str = "15.0.0";
