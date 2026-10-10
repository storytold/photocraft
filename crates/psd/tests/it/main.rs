//! Integration tests of `photocraft-psd`: one test binary (one link) with a module per area.
//! A new integration test goes into `tests/it/<name>.rs` plus a `mod <name>;` line here,
//! not into a new top-level `tests/*.rs` file (each of those links a binary of its own).

mod builder;
mod corpus;
mod layer_block_order;
mod malformed;
mod proptests;
mod roundtrip;
mod tree;
