//! # photocraft-s3
//!
//! `.pcraft` bundles on S3-compatible object storage (AWS S3, Cloudflare R2, MinIO), as a
//! [`photocraft_format::BundleStore`] (#3163). Open with
//! [`load_bytes_source`](photocraft_format::load_bytes_source), save with
//! [`PcraftWriter::save_to`](photocraft_format::PcraftWriter::save_to):
//!
//! ```no_run
//! # fn main() -> photocraft_format::Result<()> {
//! use photocraft_format::{LoadOptions, PcraftWriter, SaveOptions, load_bytes_source};
//! use photocraft_s3::{Credentials, S3Config, S3Store};
//!
//! let config = S3Config {
//!     endpoint: "http://127.0.0.1:9000".into(),
//!     region: "us-east-1".into(),
//!     bucket: "designs".into(),
//!     credentials: Credentials { access_key_id: "…".into(), secret_access_key: "…".into(), session_token: None },
//!     virtual_hosted: false,
//! };
//! let mut store = S3Store::new(config, "pool/", "docs/poster/")?;
//! let doc = load_bytes_source(&store, &LoadOptions::default())?;
//! PcraftWriter::new().save_to(&doc, &mut store, &SaveOptions::default())?;
//! # Ok(())
//! # }
//! ```
//!
//! The bundle is stored folder-style: each document's `manifest.json` is its head, and the
//! content-addressed tiles and blobs sit in a pool that versions and documents share. See
//! [`S3Store`] for the layout and how concurrent saves are kept from overwriting each other.
//!
//! Native only, at L6: no network code goes into the core or wasm crates. On wasm32 the
//! crate is empty; a browser build reaches the same layout through short-lived signed URLs its
//! host server hands out (README).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(not(target_arch = "wasm32"))]
mod client;
#[cfg(not(target_arch = "wasm32"))]
mod sign;
#[cfg(not(target_arch = "wasm32"))]
mod store;

#[cfg(not(target_arch = "wasm32"))]
pub use client::{Credentials, S3Config};
#[cfg(not(target_arch = "wasm32"))]
pub use store::{DEFAULT_PARALLEL_UPLOADS, HeadChanged, S3Store, is_conflict};
