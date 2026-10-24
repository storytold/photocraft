//! Cartography-audit test-suite matrix (Module 8.1b) from the Rust test runner's own listing.
//!
//! Single-file, std-only Rust: build with `rustc -O --edition 2021 testlist.rs -o <bin>`.
//!
//! Usage: testlist <test-list.txt> <repo-root>
//!   test-list.txt  produced by measure.sh: for each package a line
//!                  `### PACKAGE <name> <dir>` followed by the combined stdout+stderr of
//!                  `cargo test -p <name> -- --list --format terse`
//!                  (and optionally the same for `-- --list --ignored`, after a line
//!                  `### IGNORED <name> <dir>`).
//!
//! Counts come from the runner (`<name>: test` lines), never from attribute grepping.
//! Each suite is placed in a layer by evidence found in its source:
//!   acceptance   launches a built app binary (CARGO_BIN_EXE_*), drives the control
//!                socket (TcpStream), or renders the full egui shell headlessly
//!   integration  GPU device (wgpu / photocraft_gpu), real files or corpora
//!                (std::fs, tempfile, corpus, include_bytes!), other crates' I/O
//!   unit         everything else (in-crate `#[cfg(test)]` suites and doc-tests default here)

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

struct Suite {
    package: String,
    kind: String,
    source: String,
    count: u64,
    ignored: u64,
    layer: &'static str,
    signal: String,
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (Some(list), Some(root)) = (a.get(1), a.get(2)) else {
        eprintln!("usage: testlist <test-list.txt> <repo-root>");
        std::process::exit(2);
    };
    match run(list, Path::new(root)) {
        Ok(s) => print!("{s}"),
        Err(e) => {
            eprintln!("testlist: {e}");
            std::process::exit(1);
        }
    }
}

fn run(list: &str, root: &Path) -> Result<String, String> {
    let text = fs::read_to_string(list).map_err(|e| format!("read {list}: {e}"))?;
    let mut suites: BTreeMap<(String, String), Suite> = BTreeMap::new();
    let (mut pkg, mut dir, mut ignored_mode) = (String::new(), String::new(), false);
    let mut cur: Option<(String, String)> = None;
    let mut build_failures: Vec<String> = Vec::new();

    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("### PACKAGE ").or_else(|| t.strip_prefix("### IGNORED ")) {
            ignored_mode = t.starts_with("### IGNORED");
            let mut it = rest.split_whitespace();
            pkg = it.next().unwrap_or("").to_string();
            dir = it.next().unwrap_or("").to_string();
            cur = None;
            continue;
        }
        if t.starts_with("error: could not compile") || t.starts_with("error: failed to") {
            build_failures.push(format!("{pkg}: {t}"));
        }
        // "Running unittests src/lib.rs (target/...)" | "Running tests/foo.rs (target/...)"
        if let Some(rest) = t.strip_prefix("Running ") {
            let rest = rest.strip_prefix("unittests ").unwrap_or(rest);
            let src = rest.split(" (").next().unwrap_or(rest).to_string();
            let kind = if t.contains("unittests") { "in-crate" } else { "test target" };
            cur = Some(open(&mut suites, &pkg, &dir, kind, &src, root));
            continue;
        }
        if let Some(rest) = t.strip_prefix("Doc-tests ") {
            cur = Some(open(&mut suites, &pkg, &dir, "doc-tests", rest, root));
            continue;
        }
        if t.ends_with(": test") {
            if let Some(k) = &cur {
                if let Some(s) = suites.get_mut(k) {
                    if ignored_mode { s.ignored += 1 } else { s.count += 1 }
                }
            }
        }
    }

    let mut out = String::new();
    let _ = writeln!(out, "### Test-suite matrix (counts from `cargo test -- --list`)\n");
    let _ = writeln!(out, "| Package | Suite | Kind | Tests | of which #[ignore] | Layer | Deciding signal |\n|---|---|---|--:|--:|---|---|");
    let mut by_layer: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    for s in suites.values() {
        if s.count == 0 && s.ignored == 0 {
            continue;
        }
        let _ = writeln!(out, "| {} | {} | {} | {} | {} | {} | {} |", s.package, s.source, s.kind, s.count, s.ignored, s.layer, s.signal);
        let e = by_layer.entry(s.layer).or_default();
        e.0 += 1;
        e.1 += s.count;
    }
    let total: u64 = by_layer.values().map(|v| v.1).sum();
    let _ = writeln!(out, "\n### Pyramid roll-up\n\n| Layer | Suites | Tests | Share |\n|---|--:|--:|--:|");
    for layer in ["unit", "integration", "acceptance"] {
        let (n, c) = by_layer.get(layer).copied().unwrap_or_default();
        let share = if total > 0 { 100.0 * c as f64 / total as f64 } else { 0.0 };
        let _ = writeln!(out, "| {layer} | {n} | {c} | {share:.1}% |");
    }
    let _ = writeln!(out, "| **Total** | | **{total}** | |");
    if !build_failures.is_empty() {
        let _ = writeln!(out, "\n> Packages whose tests did not build (counts missing - report this):");
        for f in &build_failures {
            let _ = writeln!(out, "> - {f}");
        }
    }
    Ok(out)
}

fn open(
    suites: &mut BTreeMap<(String, String), Suite>,
    pkg: &str,
    dir: &str,
    kind: &str,
    src: &str,
    root: &Path,
) -> (String, String) {
    let key = (pkg.to_string(), format!("{kind}:{src}"));
    suites.entry(key.clone()).or_insert_with(|| {
        let (layer, signal) = classify(kind, &root.join(dir).join(src), dir);
        Suite {
            package: pkg.to_string(),
            kind: kind.to_string(),
            source: src.to_string(),
            count: 0,
            ignored: 0,
            layer,
            signal,
        }
    });
    key
}

fn classify(kind: &str, path: &Path, dir: &str) -> (&'static str, String) {
    if kind == "doc-tests" {
        return ("unit", "rustdoc examples".into());
    }
    if kind == "in-crate" {
        return ("unit", "in-crate #[cfg(test)] modules".into());
    }
    let Ok(src) = fs::read_to_string(path) else {
        return ("unit", format!("source not readable: {}", path.display()));
    };
    let has = |needle: &str| src.contains(needle);
    let acceptance = [
        ("CARGO_BIN_EXE_", "spawns the built app binary"),
        ("TcpStream", "drives the TCP control channel"),
        ("egui_kittest", "renders the egui shell headlessly"),
        ("--control", "launches the app with --control"),
    ];
    for (needle, why) in acceptance {
        if has(needle) {
            return ("acceptance", format!("{why} (`{needle}`)"));
        }
    }
    let integration = [
        ("wgpu::", "real GPU device"),
        ("photocraft_gpu", "real GPU compositor"),
        ("corpus", "real-file corpus"),
        ("include_bytes!", "embedded real files"),
        ("tempfile", "filesystem round trip"),
        ("std::fs", "filesystem I/O"),
        ("fs::read", "filesystem I/O"),
        ("fs::write", "filesystem I/O"),
        ("photocraft_io", "document <-> file I/O crate"),
        ("std::process::Command", "spawns a process"),
    ];
    for (needle, why) in integration {
        if has(needle) {
            return ("integration", format!("{why} (`{needle}`)"));
        }
    }
    let _ = dir;
    ("unit", "pure in-memory API calls (no I/O, GPU or process signal)".into())
}
