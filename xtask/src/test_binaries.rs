//! `cargo xtask test-binaries`: a crate's integration tests are modules of one binary.
//!
//! Every file directly under `crates/<crate>/tests/`, and every `tests/<dir>/main.rs` other than
//! `tests/it/main.rs`, is compiled and linked as its own test binary, which costs one link per
//! binary and one process start per binary under nextest. Integration tests go in
//! `crates/<crate>/tests/it/<name>.rs` with a `mod <name>;` line in `tests/it/main.rs`.
//!
//! A test binary that must stay separate is listed in [`ALLOWLIST`] with the reason.

use std::path::Path;

/// A single-file test binary that stays, with the reason.
#[derive(Debug, Clone, Copy)]
pub struct Allowed {
    /// Path relative to the workspace root, with `/` separators.
    pub file: &'static str,
    pub reason: &'static str,
}

pub const ALLOWLIST: &[Allowed] = &[
    Allowed {
        file: "crates/engine/tests/panic_hunt.rs",
        reason: "installs a process-wide panic hook and counts every panic in the process, so no other test may share it; `cargo xtask` selects it by binary id",
    },
    Allowed {
        file: "crates/engine/tests/global_state/main.rs",
        reason: "tests that assert on process-wide state other engine tests change; they need a process without the rest of the suite",
    },
    Allowed { file: "crates/tablet/tests/monitor_macos.rs", reason: "`harness = false` binary with its own `main`" },
];

/// Single-file test binaries under `root` that are not allow-listed, plus allow-list entries that
/// no longer match a file.
pub fn violations(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut seen = Vec::new();
    let mut crates: Vec<_> = std::fs::read_dir(root.join("crates")).into_iter().flatten().flatten().map(|e| e.path()).collect();
    crates.sort();
    for krate in crates {
        let mut files: Vec<_> = std::fs::read_dir(krate.join("tests")).into_iter().flatten().flatten().map(|e| e.path()).collect();
        files.sort();
        for file in files {
            let file = if file.is_dir() && file.file_name().is_some_and(|n| n != "it") { file.join("main.rs") } else { file };
            if !file.is_file() || file.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let rel = file.strip_prefix(root).unwrap_or(&file).to_string_lossy().replace('\\', "/");
            if ALLOWLIST.iter().any(|a| a.file == rel) {
                seen.push(rel);
            } else {
                found.push(format!(
                    "{rel}: a separate test binary; make it a module of crates/{}/tests/it/main.rs",
                    krate.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
        }
    }
    for a in ALLOWLIST {
        if !seen.iter().any(|s| s == a.file) {
            found.push(format!("{}: allow-list entry matches no file; delete it from xtask/src/test_binaries.rs", a.file));
        }
    }
    found
}

pub fn run(root: &Path) -> Result<(), String> {
    let found = violations(root);
    if found.is_empty() {
        println!("test-binaries: ok");
        for a in ALLOWLIST {
            println!("  kept as its own binary: {} ({})", a.file, a.reason);
        }
        return Ok(());
    }
    Err(format!("test-binaries: integration tests are modules of tests/it/main.rs:\n  {}", found.join("\n  ")))
}

#[cfg(test)]
mod tests {
    use super::violations;

    fn tree(files: &[&str]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-test-binaries-{}-{}", std::process::id(), files.len()));
        let _ = std::fs::remove_dir_all(&dir);
        for f in files {
            let p = dir.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        dir
    }

    #[test]
    fn refuses_a_new_single_file_binary_but_not_modules() {
        let root = tree(&[
            "crates/a/tests/it/main.rs",
            "crates/a/tests/it/x.rs",
            "crates/a/tests/new.rs",
            "crates/a/tests/fixtures/f.rs",
            "crates/a/tests/other/main.rs",
        ]);
        let found = violations(&root);
        let _ = std::fs::remove_dir_all(&root);
        let mine: Vec<_> = found.iter().filter(|m| m.starts_with("crates/a/")).collect();
        assert_eq!(mine.len(), 2, "{found:?}");
        assert!(mine[0].starts_with("crates/a/tests/new.rs"));
        assert!(mine[1].starts_with("crates/a/tests/other/main.rs"));
    }
}
