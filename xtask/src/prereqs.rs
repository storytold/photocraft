//! `cargo xtask doctor`: the single source of truth for what a PhotoCraft development machine
//! needs — tools, rustup targets/components, data checkouts and system libraries — each with the
//! exact command to fix it.
//!
//! `docs/prerequisites.md` is generated from this list and CI runs `doctor --check`, so the
//! instructions cannot drift from the code (the same pattern as `parity` and `scorecard`). Gates
//! call [`require`] before running, so a missing prerequisite fails with the fix rather than a
//! cryptic toolchain error.
//!
//! `Required` entries are needed for the core loop and the finish gates; `Optional` ones are for
//! specific workflows (editor integration, docs, packaging, release, on-demand data). Entries are
//! scoped to a [`Platform`] so a machine is only checked for what it can actually use. Tools that
//! CI downloads on the fly (nfpm, actionlint, zsync) are deliberately not listed: they are not
//! developer prerequisites.

use std::path::Path;
use std::process::{Command, Stdio};

use serde::Serialize;

/// Whether a prerequisite is needed for the normal workflow, or only for a specific one.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Required,
    Optional,
}

/// The platform a prerequisite applies to (`Any` = everywhere).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Any,
    Linux,
    Macos,
    Windows,
}

/// How a prerequisite is detected.
#[derive(Clone, Copy)]
enum Probe {
    /// A rustup target (`rustup target list --installed`).
    RustTarget(&'static str),
    /// A rustup component.
    RustComponent(&'static str),
    /// A program on `PATH`.
    Program(&'static str),
    /// A command that exits zero when the prerequisite is present (e.g. `pkg-config --exists`).
    Runs(&'static [&'static str]),
    /// A directory below the checkout root.
    Dir(&'static str),
    /// A sibling directory next to the checkout (e.g. `../craft-fonts`).
    SiblingDir(&'static str),
}

/// One prerequisite: what it is, why, how to detect it, and how to fix it.
pub struct Prereq {
    name: &'static str,
    kind: Kind,
    /// Grouping for the generated page (and future console output).
    group: &'static str,
    summary: &'static str,
    /// The command shown to a human; `None` for things that ship with the toolchain/OS.
    fix: Option<&'static str>,
    /// The argv `--fix` runs, or `None` when the fix needs a human (e.g. `sudo apt-get`).
    auto: Option<&'static [&'static str]>,
    probe: Probe,
    platform: Platform,
}

/// A Debian/Ubuntu system package's `pkg-config` name → the apt package.
macro_rules! linux_lib {
    ($name:literal, $pc:literal, $summary:literal) => {
        Prereq {
            name: $name,
            kind: Kind::Required,
            group: "Linux system libraries",
            summary: $summary,
            fix: Some(concat!("sudo apt-get install -y ", $name)),
            auto: None,
            probe: Probe::Runs(&["pkg-config", "--exists", $pc]),
            platform: Platform::Linux,
        }
    };
}

/// Every prerequisite, the single source of truth. Add a row here (and `doctor --markdown`) when a
/// gate or workflow needs something new.
const PREREQS: &[Prereq] = &[
    // --- core tools --------------------------------------------------------
    Prereq {
        name: "git",
        kind: Kind::Required,
        group: "Core tools",
        summary: "provenance: history, diff, corpus pins",
        fix: None,
        auto: None,
        probe: Probe::Program("git"),
        platform: Platform::Any,
    },
    Prereq {
        name: "cargo",
        kind: Kind::Required,
        group: "Core tools",
        summary: "builds and every `cargo xtask` command",
        fix: None,
        auto: None,
        probe: Probe::Program("cargo"),
        platform: Platform::Any,
    },
    // --- rust toolchain ----------------------------------------------------
    Prereq {
        name: "clippy",
        kind: Kind::Required,
        group: "Rust toolchain",
        summary: "the `cargo xtask ci` clippy step",
        fix: Some("rustup component add clippy"),
        auto: Some(&["rustup", "component", "add", "clippy"]),
        probe: Probe::RustComponent("clippy"),
        platform: Platform::Any,
    },
    Prereq {
        name: "rustfmt",
        kind: Kind::Required,
        group: "Rust toolchain",
        summary: "the `cargo xtask ci` format step",
        fix: Some("rustup component add rustfmt"),
        auto: Some(&["rustup", "component", "add", "rustfmt"]),
        probe: Probe::RustComponent("rustfmt"),
        platform: Platform::Any,
    },
    Prereq {
        name: "wasm32-unknown-unknown",
        kind: Kind::Required,
        group: "Rust toolchain",
        summary: "the `cargo xtask wasm` gate: L0-L6 must build for the web",
        fix: Some("rustup target add wasm32-unknown-unknown"),
        auto: Some(&["rustup", "target", "add", "wasm32-unknown-unknown"]),
        probe: Probe::RustTarget("wasm32-unknown-unknown"),
        platform: Platform::Any,
    },
    Prereq {
        name: "rust-analyzer",
        kind: Kind::Optional,
        group: "Rust toolchain",
        summary: "editor and MCP semantic navigation",
        fix: Some("rustup component add rust-analyzer"),
        auto: Some(&["rustup", "component", "add", "rust-analyzer"]),
        probe: Probe::RustComponent("rust-analyzer"),
        platform: Platform::Any,
    },
    // --- linux system libraries -------------------------------------------
    Prereq {
        name: "pkg-config",
        kind: Kind::Required,
        group: "Linux system libraries",
        summary: "build scripts that probe system libraries",
        fix: Some("sudo apt-get install -y pkg-config"),
        auto: None,
        probe: Probe::Program("pkg-config"),
        platform: Platform::Linux,
    },
    linux_lib!("libxkbcommon-dev", "xkbcommon", "keyboard input (egui/winit)"),
    linux_lib!("libwayland-dev", "wayland-client", "Wayland backend"),
    linux_lib!("libx11-dev", "x11", "X11 backend"),
    linux_lib!("libxrandr-dev", "xrandr", "X11 monitor info"),
    linux_lib!("libxi-dev", "xi", "X11 input"),
    linux_lib!("libgl1-mesa-dev", "gl", "OpenGL"),
    linux_lib!("libgtk-3-dev", "gtk+-3.0", "native dialogs (rfd)"),
    // --- other-platform build tools ---------------------------------------
    Prereq {
        name: "Xcode Command Line Tools",
        kind: Kind::Required,
        group: "Platform build tools",
        summary: "the macOS C toolchain and linker",
        fix: Some("xcode-select --install"),
        auto: None,
        probe: Probe::Runs(&["xcode-select", "-p"]),
        platform: Platform::Macos,
    },
    Prereq {
        name: "MSVC build tools",
        kind: Kind::Required,
        group: "Platform build tools",
        summary: "Visual Studio Build Tools (Desktop development with C++)",
        fix: Some("install Visual Studio Build Tools with the \"Desktop development with C++\" workload"),
        auto: None,
        probe: Probe::Runs(&["where", "cl"]),
        platform: Platform::Windows,
    },
    // --- editors, docs and scripts ----------------------------------------
    Prereq {
        name: "mdbook",
        kind: Kind::Optional,
        group: "Editors, docs and scripts",
        summary: "the `mdbook build book` docs gate",
        fix: Some("cargo install mdbook --version 0.4.52 --locked"),
        auto: Some(&["cargo", "install", "mdbook", "--version", "0.4.52", "--locked"]),
        probe: Probe::Program("mdbook"),
        platform: Platform::Any,
    },
    Prereq {
        name: "python3",
        kind: Kind::Optional,
        group: "Editors, docs and scripts",
        summary: "repo scripts (corpus fixtures, contributor stats, packaging metadata)",
        fix: None,
        auto: None,
        probe: Probe::Program("python3"),
        platform: Platform::Any,
    },
    // --- data and build inputs --------------------------------------------
    Prereq {
        name: "corpus/",
        kind: Kind::Optional,
        group: "Data and build inputs",
        summary: "the `cargo xtask test-corpus` gate (fetched on demand; never committed)",
        fix: Some("cargo xtask corpus --all"),
        auto: Some(&["cargo", "xtask", "corpus", "--all"]),
        probe: Probe::Dir("corpus"),
        platform: Platform::Any,
    },
    Prereq {
        name: "../craft-fonts",
        kind: Kind::Optional,
        group: "Data and build inputs",
        summary: "optional fonts build input (runs the Japanese font tests)",
        fix: Some("git clone https://github.com/storytold/craft-fonts ../craft-fonts"),
        auto: Some(&["git", "clone", "https://github.com/storytold/craft-fonts", "../craft-fonts"]),
        probe: Probe::SiblingDir("craft-fonts"),
        platform: Platform::Any,
    },
    Prereq {
        name: "curl",
        kind: Kind::Optional,
        group: "Data and build inputs",
        summary: "`cargo xtask corpus` downloads the pinned corpora with it",
        fix: None,
        auto: None,
        probe: Probe::Program("curl"),
        platform: Platform::Any,
    },
    Prereq {
        name: "tar",
        kind: Kind::Optional,
        group: "Data and build inputs",
        summary: "`cargo xtask corpus` unpacks the pinned corpora with it",
        fix: None,
        auto: None,
        probe: Probe::Program("tar"),
        platform: Platform::Any,
    },
    // --- web and fuzzing ---------------------------------------------------
    Prereq {
        name: "trunk",
        kind: Kind::Optional,
        group: "Web and fuzzing",
        summary: "the browser build (`apps/photocraft-web`, `packaging/web`)",
        fix: Some("cargo install trunk --locked"),
        auto: Some(&["cargo", "install", "trunk", "--locked"]),
        probe: Probe::Program("trunk"),
        platform: Platform::Any,
    },
    Prereq {
        name: "cargo-fuzz",
        kind: Kind::Optional,
        group: "Web and fuzzing",
        summary: "the manual fuzz job (needs a nightly toolchain)",
        fix: Some("cargo install cargo-fuzz --locked"),
        auto: Some(&["cargo", "install", "cargo-fuzz", "--locked"]),
        probe: Probe::Program("cargo-fuzz"),
        platform: Platform::Any,
    },
    // --- packaging and release --------------------------------------------
    Prereq {
        name: "shellcheck",
        kind: Kind::Optional,
        group: "Packaging and release",
        summary: "the packaging shell-script lint",
        fix: Some("sudo apt-get install -y shellcheck"),
        auto: None,
        probe: Probe::Program("shellcheck"),
        platform: Platform::Any,
    },
    Prereq {
        name: "xmllint",
        kind: Kind::Optional,
        group: "Packaging and release",
        summary: "the packaging XML lint (libxml2-utils)",
        fix: Some("sudo apt-get install -y libxml2-utils"),
        auto: None,
        probe: Probe::Program("xmllint"),
        platform: Platform::Any,
    },
    Prereq {
        name: "appstreamcli",
        kind: Kind::Optional,
        group: "Packaging and release",
        summary: "validates the AppStream metadata",
        fix: Some("sudo apt-get install -y appstream"),
        auto: None,
        probe: Probe::Program("appstreamcli"),
        platform: Platform::Any,
    },
    Prereq {
        name: "desktop-file-validate",
        kind: Kind::Optional,
        group: "Packaging and release",
        summary: "validates the .desktop entry (desktop-file-utils)",
        fix: Some("sudo apt-get install -y desktop-file-utils"),
        auto: None,
        probe: Probe::Program("desktop-file-validate"),
        platform: Platform::Any,
    },
    Prereq {
        name: "resvg",
        kind: Kind::Optional,
        group: "Packaging and release",
        summary: "renders the app icons (`packaging/icons.sh`)",
        fix: Some("cargo install resvg --locked"),
        auto: Some(&["cargo", "install", "resvg", "--locked"]),
        probe: Probe::Program("resvg"),
        platform: Platform::Any,
    },
    Prereq {
        name: "flatpak",
        kind: Kind::Optional,
        group: "Packaging and release",
        summary: "builds the Flatpak bundle",
        fix: Some("sudo apt-get install -y flatpak"),
        auto: None,
        probe: Probe::Program("flatpak"),
        platform: Platform::Linux,
    },
    Prereq {
        name: "flatpak-builder",
        kind: Kind::Optional,
        group: "Packaging and release",
        summary: "builds the Flatpak bundle",
        fix: Some("sudo apt-get install -y flatpak-builder"),
        auto: None,
        probe: Probe::Program("flatpak-builder"),
        platform: Platform::Linux,
    },
    Prereq {
        name: "librsvg2-common",
        kind: Kind::Optional,
        group: "Packaging and release",
        summary: "the SVG pixbuf loader AppStream icon rendering needs",
        fix: Some("sudo apt-get install -y librsvg2-common"),
        auto: None,
        probe: Probe::Runs(&["pkg-config", "--exists", "librsvg-2.0"]),
        platform: Platform::Linux,
    },
    Prereq {
        name: "wix",
        kind: Kind::Optional,
        group: "Packaging and release",
        summary: "builds the Windows MSI (WiX v5)",
        fix: Some("dotnet tool install --global wix --version 5.0.2"),
        auto: None,
        probe: Probe::Program("wix"),
        platform: Platform::Windows,
    },
];

/// The generated docs page.
pub const OUT: &str = "docs/prerequisites.md";

fn kind_str(k: Kind) -> &'static str {
    match k {
        Kind::Required => "required",
        Kind::Optional => "optional",
    }
}

fn platform_str(p: Platform) -> &'static str {
    match p {
        Platform::Any => "any",
        Platform::Linux => "linux",
        Platform::Macos => "macos",
        Platform::Windows => "windows",
    }
}

fn current_platform() -> Platform {
    if cfg!(target_os = "linux") {
        Platform::Linux
    } else if cfg!(target_os = "macos") {
        Platform::Macos
    } else if cfg!(target_os = "windows") {
        Platform::Windows
    } else {
        Platform::Any
    }
}

/// Whether `p` applies to the machine `doctor` is running on.
fn applicable(p: &Prereq) -> bool {
    p.platform == Platform::Any || p.platform == current_platform()
}

fn present(root: &Path, p: &Prereq) -> bool {
    if !applicable(p) {
        return true; // not this platform's problem
    }
    match p.probe {
        Probe::RustTarget(t) => rustup_has("target", t),
        Probe::RustComponent(c) => rustup_has("component", c),
        Probe::Program(bin) => on_path(bin),
        Probe::Runs(argv) => runs(argv),
        Probe::Dir(d) => root.join(d).is_dir(),
        Probe::SiblingDir(d) => root.parent().is_some_and(|p| p.join(d).is_dir()),
    }
}

fn rustup_has(kind: &str, name: &str) -> bool {
    let Ok(out) = Command::new("rustup").args([kind, "list", "--installed"]).output() else {
        return false;
    };
    let prefix = format!("{name}-");
    out.status.success()
        && String::from_utf8_lossy(&out.stdout).lines().any(|l| {
            let l = l.trim();
            l == name || l.starts_with(&prefix)
        })
}

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(bin).is_file()))
}

fn runs(argv: &[&str]) -> bool {
    let Some((prog, rest)) = argv.split_first() else { return false };
    Command::new(prog).args(rest).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// One checked prerequisite (a [`Prereq`] plus this machine's result).
#[derive(Serialize)]
struct Row {
    name: &'static str,
    kind: &'static str,
    group: &'static str,
    platform: &'static str,
    /// Whether this prerequisite applies to the platform `doctor` is running on.
    applicable: bool,
    summary: &'static str,
    fix: Option<&'static str>,
    /// Present (or not applicable, which is treated as satisfied).
    present: bool,
}

fn check_all(root: &Path) -> Vec<Row> {
    PREREQS
        .iter()
        .map(|p| Row {
            name: p.name,
            kind: kind_str(p.kind),
            group: p.group,
            platform: platform_str(p.platform),
            applicable: applicable(p),
            summary: p.summary,
            fix: p.fix,
            present: present(root, p),
        })
        .collect()
}

/// The named prerequisites, checked. Gates call this before running so a missing target or
/// component fails with what to do, not a cryptic toolchain error.
pub fn require(root: &Path, names: &[&str]) -> Result<(), String> {
    let mut missing: Vec<(&'static str, Option<&'static str>)> = Vec::new();
    for want in names {
        let Some(p) = PREREQS.iter().find(|p| p.name == *want) else {
            return Err(format!("doctor: unknown prerequisite `{want}`"));
        };
        if !present(root, p) {
            missing.push((p.name, p.fix));
        }
    }
    if missing.is_empty() { Ok(()) } else { Err(missing_message(&missing)) }
}

/// The actionable error for missing prerequisites, shared with [`require`] so its wording is
/// tested without depending on the machine.
fn missing_message(missing: &[(&str, Option<&str>)]) -> String {
    let list = missing.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ");
    let fixes = missing.iter().filter_map(|(_, f)| *f).map(|f| format!("`{f}`")).collect::<Vec<_>>();
    let hint = if fixes.is_empty() { String::new() } else { format!(" ({})", fixes.join("; ")) };
    format!("missing prerequisite(s): {list} — run `cargo xtask doctor --fix`{hint}")
}

/// The generated docs page. Deterministic: it never contains machine state, so `--check` is
/// portable across machines and CI jobs.
fn render_markdown() -> String {
    let mut s = String::from(
        "# Prerequisites\n\n\
         <!-- Generated by `cargo xtask doctor` from `xtask/src/prereqs.rs`; do not edit by hand. -->\n\n\
         CI runs `cargo xtask doctor --check`, so this page cannot drift from the code. Run\n\
         `cargo xtask doctor` to check your machine, `cargo xtask doctor --fix` to install what can be\n\
         installed automatically, or `cargo xtask doctor --json` for machine-readable output.\n\n\
         **Required** entries are needed for the core loop and the finish gates; **optional** ones are\n\
         for specific workflows. `cargo xtask ci` and `cargo xtask wasm` fail with the fix when a\n\
         required prerequisite is missing.\n",
    );
    let mut groups: Vec<&str> = Vec::new();
    for p in PREREQS {
        if !groups.contains(&p.group) {
            groups.push(p.group);
        }
    }
    for g in groups {
        s.push_str(&format!("\n## {g}\n\n| Prerequisite | Needed for | Install / fix |\n|---|---|---|\n"));
        for p in PREREQS.iter().filter(|p| p.group == g) {
            let needed = match p.kind {
                Kind::Required => "required",
                Kind::Optional => "optional",
            };
            let scope = if p.platform == Platform::Any { String::new() } else { format!(", {}", platform_str(p.platform)) };
            let fix = p.fix.map_or_else(|| "—".to_string(), |f| format!("`{f}`"));
            s.push_str(&format!("| `{}` | {} ({needed}{scope}) | {fix} |\n", p.name, p.summary));
        }
    }
    s
}

/// `cargo xtask doctor [--check] [--json] [--fix] [--markdown]`.
pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    const KNOWN: &[&str] = &["--check", "--json", "--fix", "--markdown", "--all"];
    if let Some(bad) = args.iter().find(|a| !KNOWN.contains(a)) {
        return Err(format!("doctor: unknown option `{bad}` (options: {})", KNOWN.join(", ")));
    }
    let rows = check_all(root);

    if args.contains(&"--json") {
        let text = serde_json::to_string_pretty(&rows).map_err(|e| format!("doctor: json: {e}"))?;
        println!("{text}");
        return Ok(());
    }

    let markdown = render_markdown();
    if args.contains(&"--markdown") {
        let out = root.join(OUT);
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
        std::fs::write(&out, &markdown).map_err(|e| format!("write {}: {e}", out.display()))?;
        println!("wrote {OUT}");
        return Ok(());
    }

    if args.contains(&"--fix") {
        return fix(root, &rows, args.contains(&"--all"));
    }

    let mut missing_required = 0;
    let mut group = "";
    for r in &rows {
        if r.group != group {
            group = r.group;
            println!("\n{group}:");
        }
        let mark = if !r.applicable {
            "n/a    "
        } else if r.present {
            "ok     "
        } else if r.kind == "required" {
            missing_required += 1;
            "MISSING"
        } else {
            "missing"
        };
        println!("  {mark}  {:<28} {}", r.name, r.summary);
        if r.applicable
            && !r.present
            && let Some(f) = r.fix
        {
            println!("               fix: {f}");
        }
    }

    if args.contains(&"--check") {
        // Docs freshness only, like `parity --check` / `scorecard --check`: portable across
        // machines and CI jobs. Whether a prerequisite is installed is not a property of the repo.
        let committed = std::fs::read_to_string(root.join(OUT)).map_err(|e| format!("read {OUT}: {e} (run `cargo xtask doctor --markdown`)"))?;
        if committed.replace("\r\n", "\n") != markdown {
            return Err(format!("{OUT} is stale — run `cargo xtask doctor --markdown` and commit it"));
        }
        println!("\n{OUT} is up to date.");
        return Ok(());
    }

    if missing_required > 0 {
        println!("\n{missing_required} required prerequisite(s) missing — run `cargo xtask doctor --fix`.");
    } else {
        println!("\nAll required prerequisites present.");
    }
    Ok(())
}

/// Install the missing prerequisites `--fix` can automate. By default only `Required` ones (the
/// core loop and gates); `all` also installs the optional workflow tooling.
fn fix(root: &Path, rows: &[Row], all: bool) -> Result<(), String> {
    let mut ran = 0;
    let mut skipped_optional = 0;
    for (p, r) in PREREQS.iter().zip(rows) {
        if r.present {
            continue;
        }
        if p.kind == Kind::Optional && !all {
            skipped_optional += 1;
            continue;
        }
        match p.auto {
            Some(argv) => {
                let Some((prog, rest)) = argv.split_first() else { continue };
                eprintln!("$ {}   # {}", argv.join(" "), p.name);
                if !Command::new(prog).args(rest).current_dir(root).status().is_ok_and(|s| s.success()) {
                    return Err(format!("doctor --fix: `{}` failed", argv.join(" ")));
                }
                ran += 1;
            }
            None => {
                if let Some(f) = p.fix {
                    eprintln!("doctor: `{}` needs a manual fix: {f}", p.name);
                }
            }
        }
    }
    if ran == 0 {
        println!("nothing to install; any `manual fix` lines above need a human");
    }
    if skipped_optional > 0 && !all {
        println!("{skipped_optional} optional prerequisite(s) not installed — `cargo xtask doctor --fix --all` for those, or see `cargo xtask doctor`.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_covers_the_known_prerequisites() {
        let names: Vec<&str> = PREREQS.iter().map(|p| p.name).collect();
        for want in [
            "git",
            "cargo",
            "clippy",
            "rustfmt",
            "wasm32-unknown-unknown",
            "pkg-config",
            "libgtk-3-dev",
            "mdbook",
            "corpus/",
            "../craft-fonts",
            "trunk",
            "cargo-fuzz",
            "flatpak-builder",
            "wix",
            "MSVC build tools",
        ] {
            assert!(names.contains(&want), "{want} missing from the manifest");
        }
    }

    #[test]
    fn every_prerequisite_is_well_formed() {
        let mut seen = std::collections::BTreeSet::new();
        for p in PREREQS {
            assert!(!p.name.is_empty() && !p.group.is_empty() && !p.summary.is_empty());
            assert!(seen.insert(p.name), "duplicate prerequisite `{}`", p.name);
            // An auto-fix is always a shown command too.
            assert!(p.auto.is_none() || p.fix.is_some(), "{} has an auto-fix but no shown command", p.name);
        }
    }

    #[test]
    fn require_names_the_fix_for_a_missing_prerequisite() {
        let msg = missing_message(&[("wasm32-unknown-unknown", Some("rustup target add wasm32-unknown-unknown"))]);
        assert!(msg.contains("wasm32-unknown-unknown"), "{msg}");
        assert!(msg.contains("cargo xtask doctor --fix"), "{msg}");
        assert!(msg.contains("rustup target add wasm32-unknown-unknown"), "{msg}");
    }

    #[test]
    fn require_rejects_an_unknown_name_and_accepts_a_present_one() {
        let root = crate::root();
        assert!(require(&root, &["not-a-prerequisite"]).is_err());
        // `cargo` is running this test, so it is present.
        assert!(require(&root, &["cargo"]).is_ok());
    }

    #[test]
    fn non_applicable_platforms_are_skipped() {
        // A Windows-only prerequisite is "present" (not applicable) on a Linux test runner.
        let win = PREREQS.iter().find(|p| p.platform == Platform::Windows).expect("a windows prerequisite");
        assert_eq!(present(Path::new("/repo"), win), !cfg!(target_os = "windows"));
    }

    #[test]
    fn markdown_is_deterministic_and_lists_every_prerequisite() {
        let a = render_markdown();
        assert_eq!(a, render_markdown(), "rendering is deterministic");
        for p in PREREQS {
            assert!(a.contains(p.name), "{} missing from {OUT}", p.name);
        }
        // No machine state on the page, or `--check` could not be portable.
        assert!(!a.contains("MISSING"), "machine state leaked into {OUT}");
        assert!(!a.contains("ok     "), "machine state leaked into {OUT}");
    }

    #[test]
    fn committed_docs_are_current() {
        let root = crate::root();
        let committed = std::fs::read_to_string(root.join(OUT)).unwrap_or_default().replace("\r\n", "\n");
        assert_eq!(committed, render_markdown(), "{OUT} is stale: run `cargo xtask doctor --markdown`");
    }
}
