//! Sandboxed WebAssembly plug-ins for PhotoCraft.
//!
//! A plug-in is a WebAssembly module that implements the PhotoCraft plug-in ABI: **v1** (`kind:
//! "filter"`, documented in `docs/plugins.md`) transforms pixels, and **v2** adds **panels**
//! (`kind: "panel"`): a live instance the host sends events to and which answers with operations
//! for the host to carry out. Both run in [`wasmi`], a pure-Rust interpreter, with:
//!
//! - **no host imports**: no WASI, file system, network, clock or randomness. A module that
//!   imports anything is rejected;
//! - an **instruction budget** (fuel) per call, metered in slices so a wall-clock deadline is
//!   also enforced (native builds), so infinite loops end with an error;
//! - a **linear-memory cap**, a **recursion-depth cap** and wasmi's strict module limits;
//! - a **fresh instance per band** for a filter, so no state leaks between calls, and **one
//!   persistent instance per panel** ([`Instance`]), owning the plug-in's state machine.
//!
//! Every failure (malformed module, trap, exhausted budget, bad output) is an [`Error`]; nothing
//! a module does can panic or hang the host.
//!
//! Pixels are handed over as interleaved, normalised `f32` samples in the document's own colour
//! model (bit-depth and colour-mode agnostic), in horizontal bands so the copy stays bounded.
//! [`Plugin::apply`] runs a filter on a [`photocraft_raster::Surface`], blending by the selection.
//! Image bytes never cross into the interpreter at all: a panel passes [`blobs`] ids around, and
//! the host does the encoding.
//!
//! A panel never touches the document *itself* — no import reaches one: it returns
//! [`ops::Operation`]s, which the host validates against the manifest's [`Capabilities`] and either
//! performs or refuses. `doc.command` is how a panel really edits the document (the allow-listed
//! engine commands, each an ordinary edit with its own undo step); `doc.capture`, `doc.commit` and
//! `http.request` are refused in this build.
//!
//! The process-wide [`registry`] holds the installed plug-ins.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod apply;
pub mod blobs;
pub mod instance;
pub mod manifest;
pub mod ops;
pub mod registry;
mod runtime;
pub mod view;

pub use apply::format_code;
pub use instance::Instance;
pub use manifest::{Area, Capabilities, DocumentCap, Kind, Manifest, Network, Panel, ParamSpec, Unit};
pub use runtime::{ABI_PANEL, ABI_VERSION, ABI_VERSION_MIN, Limits, Plugin};

/// A plug-in error. Every way a module can misbehave ends here, never in a panic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("not a valid plug-in module: {0}")]
    Module(String),
    #[error("plug-in ABI: {0}")]
    Abi(String),
    #[error("plug-in manifest: {0}")]
    Manifest(String),
    #[error("plug-in trapped: {0}")]
    Trap(String),
    #[error("plug-in exceeded its {0}")]
    Limit(String),
    #[error("plug-in parameters: {0}")]
    Params(String),
    #[error("plug-in operation: {0}")]
    Ops(String),
    #[error("plug-in failed: {0}")]
    Failed(String),
    #[error("no plug-in {0:?} is installed")]
    NotFound(String),
    #[error("{0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, Error>;
