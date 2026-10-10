//! Integration tests of `photocraft-raw`: one test binary (one link) with a module per area.
//! A new integration test goes into `tests/it/<name>.rs` plus a `mod <name>;` line here,
//! not into a new top-level `tests/*.rs` file (each of those links a binary of its own).

mod dng;
mod hostile;
mod raf;
mod raw_corpus;
mod vendor_codecs;
mod vendors;
