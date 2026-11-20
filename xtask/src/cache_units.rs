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

/// Records xtask's own units: `cargo xtask` builds it before it can record anything, and an
/// unrecorded unit is trimmed from the entry and rebuilt next run. The build is fresh, so this only
/// prints the messages.
pub fn record_xtask() -> Result<(), String> {
    if file().is_none() {
        return Ok(());
    }
    let mut c = crate::cargo();
    c.args(["build", "-p", "xtask", "--message-format", "json-render-diagnostics"]);
    run(c, "cargo build -p xtask --message-format json-render-diagnostics")
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
    fn cargo_messages_are_recorded_and_test_output_passes_through() {
        let input = "{\"reason\":\"compiler-artifact\",\"fresh\":true}\nrunning 3 tests\ntest a ... ok\n{\"reason\":\"build-finished\",\"success\":true}";
        let (mut record, mut out) = (Vec::new(), Vec::new());
        route(input.as_bytes(), &mut record, &mut out).unwrap();
        assert_eq!(String::from_utf8(record).unwrap(), "{\"reason\":\"compiler-artifact\",\"fresh\":true}\n{\"reason\":\"build-finished\",\"success\":true}\n");
        assert_eq!(String::from_utf8(out).unwrap(), "running 3 tests\ntest a ... ok\n");
    }
}
