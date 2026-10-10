//! # photocraft-adobe-assets
//!
//! Readers and writers for Adobe preset and asset files, at the format level: each module keeps
//! what a file stores, so unmodified files write back byte for byte, and leaves interpreting it
//! to the caller.
//!
//! * [`atn`]: Photoshop action files (`.atn`).
//!
//! The crate is standalone (no workspace dependencies). Steps keep their ActionDescriptors as
//! raw bytes, found with a structural walk; `photocraft_psd::Descriptor::from_bytes` decodes them.
//!
//! Implemented clean-room from files saved by Photoshop; each module documents what was observed
//! and what is only accepted. Parsers never panic on malformed input: counts are capped and
//! checked before allocating, and descriptor nesting is bounded.
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod atn;
pub mod error;
mod io;

pub use error::{AssetError, Result};
