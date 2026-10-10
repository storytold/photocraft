//! Integration tests of `photocraft-format`: one test binary (one link) with a module per area.
//! A new integration test goes into `tests/it/<name>.rs` plus a `mod <name>;` line here,
//! not into a new top-level `tests/*.rs` file (each of those links a binary of its own).

mod common;

mod analysis;
mod autosave;
mod corruption;
mod layer_labels;
mod roundtrip;
mod smart_contents;
