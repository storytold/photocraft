//! Integration tests of `photocraft-codecs`: one test binary (one link) with a module per area.
//! A new integration test goes into `tests/it/<name>.rs` plus a `mod <name>;` line here,
//! not into a new top-level `tests/*.rs` file (each of those links a binary of its own).

mod common;

mod camera_raw;
mod decode_warnings;
mod deep_exr;
mod detect_caps;
mod exr_multipart;
mod fidelity;
mod heif;
mod limits;
mod malformed;
mod metadata;
mod orientation;
mod png_jpeg;
mod resolution;
mod roundtrip;
mod tiff_bench;
mod tiff_pages;
mod tiff_photoshop;
mod tiff_regressions;
mod webp_lossy;
