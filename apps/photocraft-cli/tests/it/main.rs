//! The crate's integration tests, linked into one test binary.

mod cli;
mod man_and_completions;
#[path = "../../../../xtask/src/psd_write_oracle.rs"]
mod oracle_runner;
mod orientation;
mod psd_write_oracle;
#[path = "../../../../xtask/src/sha256.rs"]
mod sha256;

/// Nextest archives relocate binaries; plain Cargo keeps the compile-time path valid.
fn binary() -> std::ffi::OsString {
    std::env::var_os("NEXTEST_BIN_EXE_photocraft_cli").unwrap_or_else(|| env!("CARGO_BIN_EXE_photocraft-cli").into())
}
