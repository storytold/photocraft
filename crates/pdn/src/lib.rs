//! Reader for Paint.NET (.pdn) documents.
//!
//! Layouts follow the PDN3 format:
//! - 4-byte ASCII signature `PDN3`
//! - 3-byte little-endian header length (24-bit integer)
//! - UTF-8 XML document header with canvas dimensions and layer count
//! - 2-byte indicator `0x00, 0x01`
//! - .NET Remoting Binary Format (NRBF) object stream containing the document model
//! - Deferred compressed pixel chunks (GZIP or uncompressed) for each layer
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod chunks;
mod error;
mod header;
pub mod model;
pub mod nrbf;
#[cfg(any(test, feature = "synth"))]
pub mod synth;

pub use error::Error;
pub use header::{HeaderInfo, is_pdn};
pub use model::{BlendMode, Document, Layer, Limits, read, read_with_limits};

#[cfg(test)]
mod tests;
