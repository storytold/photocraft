//! No wall-clock timer outside `Stopwatch` (issue #334).
//!
//! `std::time::Instant::now()` panics with "time not implemented on this platform" on
//! wasm32-unknown-unknown, the target of the web build. `cargo xtask wasm` only *checks* that
//! target, so a stray `Instant` compiles cleanly and panics at run time — which is what Liquify
//! did when you pressed OK in the browser. This walks the engine's production source and
//! requires each `Instant::now` to sit directly under `#[cfg(not(target_arch = "wasm32"))]`,
//! the way `photo_cmds::Stopwatch` does it; use that instead of timing anything yourself.
//!
//! Scoped to the engine: the other wasm-safe crates wrap `std::time` in a helper of their own
//! (`gpu::web_time_now`, `plugins::Deadline`, `ui_egui::now_ms`) behind a `#[cfg]` *block*,
//! which a line-level scan cannot tell apart from an unguarded call.

use std::path::{Path, PathBuf};

/// Production source of a file: everything before its unit-test module.
fn production(text: &str) -> &str {
    let cut = ["#[cfg(test)]\nmod ", "#[cfg(test)]\r\nmod ", "#[cfg(test)]\npub mod ", "#[cfg(test)]\r\npub mod "]
        .into_iter()
        .filter_map(|m| text.find(m))
        .min()
        .unwrap_or(text.len());
    &text[..cut]
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if p.is_dir() {
            if !matches!(name.as_str(), "tests" | "examples" | "benches" | "target") {
                rust_sources(&p, out);
            }
        } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
            out.push(p);
        }
    }
}

/// The line numbers of the `Instant::now` calls in `text` that the line above does not opt out
/// of wasm. Only the neighbouring line is inspected: the engine gates its one timer with a
/// plain attribute (`#[cfg(not(target_arch = "wasm32"))]` in `Stopwatch::start`), and anything
/// looser (a `#[cfg]` several lines up, a helper) is exactly the mistake this test is for.
fn ungated_instant_lines(text: &str) -> Vec<usize> {
    text.match_indices("Instant::now")
        .filter(|(i, _)| {
            // `.rev().skip(1)`: the last line of `text[..i]` is the call's own line, up to it.
            let prev = text[..*i].lines().rev().skip(1).find(|l| !l.trim().is_empty());
            prev.is_none_or(|l| l.trim() != "#[cfg(not(target_arch = \"wasm32\"))]")
        })
        .map(|(i, _)| text[..i].lines().count())
        .collect()
}

#[test]
fn no_ungated_instant_in_the_engine() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_sources(&src, &mut files);
    assert!(files.len() > 50, "found only {} source files under {}", files.len(), src.display());

    let mut bad = Vec::new();
    for p in &files {
        let Ok(text) = std::fs::read_to_string(p) else { continue };
        for line in ungated_instant_lines(production(&text)) {
            bad.push(format!("{}:{line}", p.display()));
        }
    }
    assert!(
        bad.is_empty(),
        "Instant::now() panics on wasm32; gate it with #[cfg(not(target_arch = \"wasm32\"))] or use Stopwatch (crates/engine/src/photo_cmds.rs):\n{}",
        bad.join("\n")
    );
}
