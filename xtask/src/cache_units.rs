//! The build units a CI build used, for the cache entry the gate saves.
//!
//! In a gate leg that saves the dependency cache, craft-common-actions' rust-cache action names a
//! file in `CRAFT_CACHE_UNITS`, and its cache-trim action deletes every unit of the target
//! directory that the file does not name before the entry is saved. Cargo prints a JSON message for
//! every unit a build compiled or found fresh; these builds append those messages to the file, so
//! the saved entry holds this build and no older builds of updated dependencies.

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// The record file, when this leg saves the cache.
pub fn file() -> Option<PathBuf> {
    std::env::var_os("CRAFT_CACHE_UNITS").filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Runs `cmd`, a cargo build already told to print Cargo's JSON messages on stdout
/// (`json-render-diagnostics`, so diagnostics stay readable on stderr), appends the messages to the
/// record and passes every other stdout line (test output) through.
pub fn run(mut cmd: Command, what: &str) -> Result<(), String> {
    let Some(file) = file() else { return crate::run(cmd, what) };
    eprintln!("$ {what}");
    let mut record = OpenOptions::new().create(true).append(true).open(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut child = cmd.stdout(Stdio::piped()).spawn().map_err(|e| format!("{what}: failed to spawn: {e}"))?;
    let stdout = child.stdout.take().expect("piped stdout");
    let routed = route(BufReader::new(stdout), &mut record, &mut std::io::stdout().lock());
    let status = child.wait().map_err(|e| format!("{what}: {e}"))?;
    routed.map_err(|e| format!("{what}: recording the build units in {}: {e}", file.display()))?;
    if status.success() { Ok(()) } else { Err(format!("{what}: exited with {status}")) }
}

/// Capture non-Cargo output while recording Cargo messages, for a command whose output is data.
pub fn capture_recorded(mut cmd: Command, what: &str, file: &std::path::Path) -> Result<Vec<u8>, String> {
    eprintln!("$ {what}");
    let mut record = OpenOptions::new().create(true).append(true).open(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut child = cmd.stdout(Stdio::piped()).spawn().map_err(|e| format!("{what}: failed to spawn: {e}"))?;
    let stdout = child.stdout.take().expect("piped stdout");
    let mut output = Vec::new();
    let routed = route(BufReader::new(stdout), &mut record, &mut output);
    let status = child.wait().map_err(|e| format!("{what}: {e}"))?;
    routed.map_err(|e| format!("{what}: reading output: {e}"))?;
    if status.success() { Ok(output) } else { Err(format!("{what}: exited with {status}")) }
}

/// Records xtask's own units: `cargo xtask` builds it before it can record anything, and an
/// unrecorded unit is trimmed from the entry and rebuilt next run. The build is fresh, so this only
/// prints the messages.
pub fn record_xtask() -> Result<(), String> {
    if file().is_none() {
        return Ok(());
    }
    let mut c = crate::cargo();
    xtask_build_args(&mut c, cfg!(feature = "framework-checks"));
    let what = crate::shown(&c);
    run(c, &what)
}

fn xtask_build_args(c: &mut Command, framework_checks: bool) {
    c.args(["build", "--locked", "-p", "xtask", "--message-format", "json-render-diagnostics"]);
    // Match the features of the running bootstrap, including `cargo xtask-gate`.
    if !framework_checks {
        c.arg("--no-default-features");
    }
}

/// Lines that are Cargo's JSON messages go to `record`, every other line to `out`.
fn route(mut input: impl BufRead, record: &mut impl Write, out: &mut impl Write) -> std::io::Result<()> {
    let mut line = Vec::new();
    while input.read_until(b'\n', &mut line)? != 0 {
        if line.starts_with(b"{\"reason\":") {
            record.write_all(&line)?;
            if !line.ends_with(b"\n") {
                record.write_all(b"\n")?;
            }
        } else {
            out.write_all(&line)?;
        }
        line.clear();
    }
    record.flush()?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xtask_record_build_matches_bootstrap_features() {
        for enabled in [false, true] {
            let mut c = Command::new("cargo");
            xtask_build_args(&mut c, enabled);
            let args: Vec<_> = c.get_args().map(|s| s.to_string_lossy().into_owned()).collect();
            assert_eq!(args.contains(&"--no-default-features".into()), !enabled);
            assert!(args.contains(&"--locked".into()));
            assert!(args.windows(2).any(|p| p == ["-p", "xtask"]));
            assert!(args.windows(2).any(|p| p == ["--message-format", "json-render-diagnostics"]));
        }
    }

    #[test]
    fn cargo_messages_are_recorded_and_test_output_passes_through() {
        let input = "{\"reason\":\"compiler-artifact\",\"fresh\":true}\nrunning 3 tests\ntest a ... ok\n{\"reason\":\"build-finished\",\"success\":true}";
        let (mut record, mut out) = (Vec::new(), Vec::new());
        route(input.as_bytes(), &mut record, &mut out).unwrap();
        assert_eq!(String::from_utf8(record).unwrap(), "{\"reason\":\"compiler-artifact\",\"fresh\":true}\n{\"reason\":\"build-finished\",\"success\":true}\n");
        assert_eq!(String::from_utf8(out).unwrap(), "running 3 tests\ntest a ... ok\n");
    }

    #[test]
    fn mixed_nextest_listing_keeps_json_separate_and_rejects_truncation() {
        let input = b"{\"reason\":\"compiler-artifact\",\"fresh\":false}\n{\"rust-suites\":{}}\n";
        let (mut record, mut listing) = (Vec::new(), Vec::new());
        route(input.as_slice(), &mut record, &mut listing).unwrap();
        assert_eq!(record, b"{\"reason\":\"compiler-artifact\",\"fresh\":false}\n");
        assert!(serde_json::from_slice::<serde_json::Value>(&listing).is_ok());
        listing.truncate(listing.len() - 3);
        assert!(serde_json::from_slice::<serde_json::Value>(&listing).is_err());
    }

    #[test]
    fn real_nextest_list_records_build_without_rebuilding() {
        if !crate::nextest_available() {
            eprintln!("cargo-nextest unavailable; skipping mixed-stream integration test");
            return;
        }
        let dir = std::env::temp_dir().join(format!("photocraft-cache-units-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"cache-unit-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "#[test] fn one() {}\n").unwrap();
        let list = || {
            let out = crate::cargo()
                .current_dir(&dir)
                .env("CARGO_TARGET_DIR", dir.join("target"))
                .env("CARGO_BUILD_BUILD_DIR", dir.join("target"))
                .env("CARGO_NET_OFFLINE", "true")
                .env("NEXTEST_PROFILE", "ci")
                .args([
                    "nextest",
                    "list",
                    "--profile",
                    "default",
                    "--list-type",
                    "binaries-only",
                    "--message-format",
                    "json",
                    "--cargo-message-format",
                    "json-render-diagnostics",
                ])
                .output()
                .unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            out
        };
        for expected_fresh in [false, true] {
            let output = list();
            let (mut record, mut listing) = (Vec::new(), Vec::new());
            route(output.stdout.as_slice(), &mut record, &mut listing).unwrap();
            let artifacts: Vec<serde_json::Value> = record
                .split(|&b| b == b'\n')
                .filter(|line| !line.is_empty())
                .map(|line| serde_json::from_slice(line).unwrap())
                .filter(|message: &serde_json::Value| message["reason"] == "compiler-artifact")
                .collect();
            assert!(!artifacts.is_empty());
            assert!(artifacts.iter().all(|message| message["fresh"].as_bool() == Some(expected_fresh)));
            let json: serde_json::Value = serde_json::from_slice(&listing).unwrap();
            assert_eq!(json["rust-binaries"].as_object().unwrap().len(), 1);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
