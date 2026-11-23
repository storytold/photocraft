//! The environment of the cargo processes xtask starts.
//!
//! `cargo xtask` is `cargo run -p xtask`, so xtask runs with the variables Cargo sets for the
//! binary it runs: `CARGO_MANIFEST_DIR`, `CARGO_PKG_NAME`, `CARGO_PKG_VERSION_*` and the rest, all
//! describing xtask. A cargo started from xtask passes them on to every build script it runs. Cargo
//! compares a build script's `rerun-if-env-changed` variables with its own environment, so a script
//! that declares one of them (ring declares `CARGO_PKG_NAME` and the version parts) reruns, and
//! everything that depends on it rebuilds, each time one build starts inside xtask and the next
//! outside it (a CI script, the cache warm-up, a developer's own `cargo test`).

use std::ffi::OsStr;
use std::process::Command;

/// Whether `name` is a variable Cargo sets for the binary `cargo run` starts (rather than one it
/// reads from the environment of whoever started it). `CARGO` stays: it names the cargo that ran
/// xtask, which is the one xtask starts.
pub fn set_by_cargo_run(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else { return false };
    matches!(name, "CARGO_MANIFEST_DIR" | "CARGO_MANIFEST_PATH" | "CARGO_CRATE_NAME" | "CARGO_BIN_NAME" | "CARGO_PRIMARY_PACKAGE" | "OUT_DIR")
        || name.starts_with("CARGO_PKG_")
        || name.starts_with("CARGO_BIN_EXE_")
}

/// Removes from `cmd` every variable of the current environment that Cargo set for this binary,
/// so the cargo it starts sees the environment xtask itself was started with.
pub fn without_cargo_run_vars(cmd: &mut Command) -> &mut Command {
    for (name, _) in std::env::vars_os() {
        if set_by_cargo_run(&name) {
            cmd.env_remove(name);
        }
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_only_the_variables_cargo_run_sets() {
        for name in [
            "CARGO_MANIFEST_DIR",
            "CARGO_MANIFEST_PATH",
            "CARGO_PKG_NAME",
            "CARGO_PKG_VERSION_PRE",
            "CARGO_CRATE_NAME",
            "CARGO_BIN_NAME",
            "CARGO_PRIMARY_PACKAGE",
            "CARGO_BIN_EXE_photocraft",
            "OUT_DIR",
        ] {
            assert!(set_by_cargo_run(OsStr::new(name)), "{name}");
        }
        // What the developer or CI set: build settings, the target dir, the cargo to run.
        for name in ["CARGO", "CARGO_HOME", "CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "CARGO_PROFILE_DEV_DEBUG", "RUSTFLAGS", "PATH", "PKG_CONFIG_PATH"] {
            assert!(!set_by_cargo_run(OsStr::new(name)), "{name}");
        }
    }

    // A test process runs with the variables Cargo sets for test binaries, CARGO_PKG_NAME among them.
    #[test]
    fn a_child_cargo_does_not_inherit_them() {
        assert!(std::env::var_os("CARGO_PKG_NAME").is_some(), "cargo sets CARGO_PKG_NAME for tests");
        let mut cmd = Command::new("true");
        without_cargo_run_vars(&mut cmd);
        let removed: Vec<_> = cmd.get_envs().filter(|(_, value)| value.is_none()).map(|(name, _)| name.to_owned()).collect();
        assert!(removed.iter().any(|name| name == "CARGO_PKG_NAME"), "{removed:?}");
        assert!(removed.iter().all(|name| set_by_cargo_run(name)), "{removed:?}");
    }
}
