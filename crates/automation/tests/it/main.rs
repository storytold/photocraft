//! Integration tests of `photocraft-automation`: one test binary (one link) with a module per area.
//! A new integration test goes into `tests/it/<name>.rs` plus a `mod <name>;` line here,
//! not into a new top-level `tests/*.rs` file (each of those links a binary of its own).

mod agent_tasks;
mod inactive_save;
mod layered_save_dirty;
mod mcp;
mod mcp_wire;
