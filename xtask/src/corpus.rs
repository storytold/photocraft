//! `cargo xtask corpus [...]` (fetch test corpora) and `cargo xtask test-corpus [...]` (fetch,
//! then run the corpus tests). Pins: `corpus_pins.rs`. Docs: `docs/development.md` › Test corpora.

use std::path::PathBuf;
use std::process::Command;

use crate::corpus_pins::{self, AG_PSD_COMMIT, HEIC_RS_COMMIT, OPENEXR_COMMIT, PHOTOCRAFT_CORPUS_COMMIT, PILLOW_HEIF_COMMIT, PNGSUITE_URL, PSD_TOOLS_COMMIT};
use crate::pinned::{PinnedCorpus, USER_AGENT};
use crate::{cargo, root, run};

/// Crates with corpus tests (behind their `corpus` feature).
pub const CORPUS_CRATES: &[&str] = &["photocraft-psd", "photocraft-codecs", "photocraft-io", "photocraft-engine"];
/// Opt-in corpus crates: fetched and tested only with `test-corpus --pixls`.
pub const PIXLS_CRATES: &[&str] = &["photocraft-raw"];

/// Corpus crates with a `heif` feature: test-corpus enables it so the HEIF corpus tests run.
const HEIF_CRATES: &[&str] = &["photocraft-codecs", "photocraft-io"];

/// Paths whose changes make `test-corpus --changed` run (the file-format and rendering crates).
const CRITICAL: &[&str] = &[
    "crates/psd/",
    "crates/io/",
    "crates/codecs/",
    "crates/heif/",
    "crates/compose/",
    "crates/gpu/",
    "crates/text/",
    "crates/format/",
    "crates/affinity/",
    "crates/raw/",
];
/// `cargo xtask corpus [--all | --pngsuite | --download | --psd | --psd-tools | --heif | --exr | --pixls | --affinity | --photoshop [--local]] [--update-manifest]`
pub fn cmd(args: &[&str]) -> Result<(), String> {
    let update = args.contains(&"--update-manifest");
    let mut did = false;
    if args.contains(&"--all") {
        return fetch_all(args.contains(&"--local"), update);
    }
    if args.contains(&"--pngsuite") || args.contains(&"--download") {
        fetch_pngsuite()?;
        did = true;
    }
    if args.contains(&"--psd") {
        corpus_pins::PSD_MIXED.fetch(update)?;
        did = true;
    }
    if args.contains(&"--psd-tools") {
        corpus_pins::PSD_TOOLS.fetch(update)?;
        did = true;
    }
    if args.contains(&"--heif") {
        corpus_pins::HEIF.fetch(update)?;
        did = true;
    }
    if args.contains(&"--pixls") {
        crate::pinned::fetch_pixls(update)?;
        did = true;
    }
    if args.contains(&"--exr") {
        corpus_pins::EXR.fetch(update)?;
        did = true;
    }
    if args.contains(&"--affinity") {
        corpus_pins::AFFINITY.fetch(update)?;
        did = true;
    }
    if args.contains(&"--photoshop") {
        if args.contains(&"--local") {
            corpus_pins::PHOTOSHOP.fetch_local(&local_clone()?, update)?;
        } else {
            corpus_pins::PHOTOSHOP.fetch(update)?;
        }
        did = true;
    }
    if !did {
        list();
    }
    Ok(())
}

/// The authoring clone of photocraft-corpus: `$PHOTOCRAFT_CORPUS_REPO`, else `../photocraft-corpus`.
fn local_clone() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("PHOTOCRAFT_CORPUS_REPO").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    let sibling = root().parent().map(|p| p.join("photocraft-corpus"));
    sibling.filter(|s| s.join("photoshop").is_dir()).ok_or_else(|| {
        "no local photocraft-corpus clone: expected ../photocraft-corpus next to this checkout, or set PHOTOCRAFT_CORPUS_REPO=<path>".to_string()
    })
}

/// Fetches every corpus that is missing or stale (verified ones are left alone). With `local`,
/// `corpus/photoshop` is copied from the authoring clone of photocraft-corpus instead. With
/// `update`, every pinned corpus is refetched and its manifest rewritten (`--update-manifest`).
pub fn fetch_all(local: bool, update: bool) -> Result<(), String> {
    fetch_pngsuite()?;
    fetch_pinned(local, update, fetch_one)?;
    println!("all corpora present and verified under {}", root().join("corpus").display());
    Ok(())
}

/// Calls `fetch(corpus, from_clone, update)` for every pinned corpus; `from_clone` is set for
/// `corpus/photoshop` when `local`.
fn fetch_pinned(local: bool, update: bool, mut fetch: impl FnMut(&PinnedCorpus, bool, bool) -> Result<(), String>) -> Result<(), String> {
    for c in corpus_pins::ALL {
        fetch(c, local && std::ptr::eq(*c, &corpus_pins::PHOTOSHOP), update)?;
    }
    Ok(())
}

fn fetch_one(c: &PinnedCorpus, from_clone: bool, update: bool) -> Result<(), String> {
    if from_clone { c.fetch_local(&local_clone()?, update) } else { c.fetch(update) }
}

fn status(present: bool) -> &'static str {
    if present { "present" } else { "missing" }
}

fn list() {
    let corpus = root().join("corpus");
    let png = corpus.join("pngsuite");
    let png_ok = pngsuite_count(&png) > 0;
    println!(
        "Test corpora live under {} (gitignored, never committed). The corpus tests are opt-in
(cargo feature `corpus`) and fail, not skip, when a corpus is missing. CI always runs them.

  cargo xtask corpus --all      fetch every corpus below that is missing or stale (sha256-verified)
  cargo xtask test-corpus       fetch, then run every corpus test (--release --features corpus)

  corpus/photoshop/  [{}] our Photoshop-authored oracles (MIT OR Apache-2.0), from
                     https://github.com/storytold/photocraft-corpus at {PHOTOCRAFT_CORPUS_COMMIT}
                     manifest xtask/photoshop-corpus.sha256. Fetch: --photoshop
                     (--photoshop --local copies from ../photocraft-corpus or $PHOTOCRAFT_CORPUS_REPO)
  corpus/psd/        [{}] 170 small psd-tools + ag-psd files (MIT), psd-tools@{} and
                     ag-psd@{}, manifest xtask/psd-corpus.sha256. Fetch: --psd
  corpus/psd-tools/  [{}] the full psd-tools test set (MIT) at {PSD_TOOLS_COMMIT}
                     manifest xtask/psd-tools-corpus.sha256. Fetch: --psd-tools
  corpus/heif/       [{}] a few HEIC/HEIF files from heic-rs@{} (MIT OR Apache-2.0) and
                     pillow-heif@{} (BSD-3-Clause), manifest xtask/heif-corpus.sha256. Fetch: --heif
  corpus/exr/        [{}] the deep OpenEXR test images (BSD-3-Clause), openexr@{}
                     manifest xtask/exr-corpus.sha256. Fetch: --exr
  corpus/affinity/   [{}] public Affinity documents and #1606 samples (CC0, MIT), with PNG references;
                     manifest xtask/affinity-corpus.sha256. Fetch: --affinity
  corpus/pngsuite/   [{}] PngSuite (public domain), {PNGSUITE_URL}. Fetch: --pngsuite
  corpus/pixls/      [opt-in] real camera raws from raw.pixls.us (public domain), one per decode
                     path plus the known-unsupported packed ORF. Fetch: --pixls (opt-in)

Pins: xtask/src/corpus_pins.rs. Moving one: change it, then --<name> --update-manifest.",
        corpus.display(),
        status(corpus_pins::PHOTOSHOP.is_current()),
        status(corpus_pins::PSD_MIXED.is_current()),
        &PSD_TOOLS_COMMIT[..12],
        &AG_PSD_COMMIT[..12],
        status(corpus_pins::PSD_TOOLS.is_current()),
        status(corpus_pins::HEIF.is_current()),
        &HEIC_RS_COMMIT[..12],
        &PILLOW_HEIF_COMMIT[..12],
        status(corpus_pins::EXR.is_current()),
        &OPENEXR_COMMIT[..12],
        status(corpus_pins::AFFINITY.is_current()),
        status(png_ok),
    );
}

fn pngsuite_count(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir).map(|d| d.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "png")).count()).unwrap_or(0)
}

/// PngSuite: 175 PNGs from a fixed release archive (skipped when already present).
fn fetch_pngsuite() -> Result<(), String> {
    let corpus = root().join("corpus");
    let dest = corpus.join("pngsuite");
    let have = pngsuite_count(&dest);
    if have >= 175 {
        println!("pngsuite: {have} PNG files in {} already", dest.display());
        return Ok(());
    }
    std::fs::create_dir_all(&dest).map_err(|e| format!("create {}: {e}", dest.display()))?;
    let tgz = corpus.join("PngSuite-2017jul19.tgz");
    let mut curl = Command::new("curl");
    curl.args(["-fsSL", "--retry", "3", "-A", USER_AGENT, "-o"]).arg(&tgz).arg(PNGSUITE_URL);
    run(curl, &format!("curl {PNGSUITE_URL}"))?;
    let mut tar = Command::new("tar");
    tar.arg("-xzf").arg(&tgz).arg("-C").arg(&dest);
    run(tar, "tar -xzf PngSuite-2017jul19.tgz")?;
    let _ = std::fs::remove_file(&tgz);
    let n = pngsuite_count(&dest);
    println!("pngsuite: {n} PNG files in {}", dest.display());
    if n == 0 { Err("no PNG files extracted".into()) } else { Ok(()) }
}

/// Files changed relative to `origin/main` (committed, staged or not).
fn changed_files() -> Result<Vec<String>, String> {
    let out = Command::new("git").current_dir(root()).args(["diff", "--name-only", "origin/main"]).output().map_err(|e| format!("git diff: {e}"))?;
    if !out.status.success() {
        return Err(format!("git diff origin/main failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect())
}

/// `cargo xtask test-corpus [-p <crate>]... [--changed] [--local] [-- <test args>]`
pub fn test_cmd(args: &[&str]) -> Result<(), String> {
    let (ours, passthrough) = match args.iter().position(|a| *a == "--") {
        Some(i) => (&args[..i], &args[i + 1..]),
        None => (args, &[][..]),
    };
    let mut raw_changed = false;
    if ours.contains(&"--changed") {
        let changed = changed_files()?;
        let hits: Vec<&String> = changed.iter().filter(|f| CRITICAL.iter().any(|c| f.starts_with(c))).collect();
        if hits.is_empty() {
            println!("test-corpus --changed: nothing under {} changed since origin/main; skipping", CRITICAL.join(", "));
            return Ok(());
        }
        println!("test-corpus --changed: {} critical files changed (e.g. {}); running the corpus tests", hits.len(), hits[0]);
        raw_changed = hits.iter().any(|f| f.starts_with("crates/raw/"));
    }
    let mut crates: Vec<String> = Vec::new();
    let mut it = ours.iter();
    while let Some(a) = it.next() {
        match *a {
            "-p" | "--package" => {
                let name = it.next().ok_or("-p needs a crate name")?;
                let full = if name.starts_with("photocraft-") { (*name).to_string() } else { format!("photocraft-{name}") };
                if !CORPUS_CRATES.contains(&full.as_str()) && !PIXLS_CRATES.contains(&full.as_str()) {
                    return Err(format!("{full} has no corpus tests (crates: {} or the opt-in {})", CORPUS_CRATES.join(", "), PIXLS_CRATES.join(", ")));
                }
                crates.push(full);
            }
            "--changed" | "--local" | "--pixls" => {}
            other => return Err(format!("test-corpus: unknown argument `{other}`")),
        }
    }
    if crates.is_empty() {
        crates = CORPUS_CRATES.iter().map(|s| (*s).to_string()).collect();
    }
    // `--pixls` opts in (and fetches), and a raw change under --changed does too: the raw
    // corpus tests are not part of the default set until a maintainer decides otherwise.
    let wants_pixls = ours.contains(&"--pixls") || crates.iter().any(|c| PIXLS_CRATES.contains(&c.as_str())) || raw_changed;
    if wants_pixls && !crates.iter().any(|c| c == "photocraft-raw") {
        crates.push("photocraft-raw".to_string());
    }
    if wants_pixls {
        crate::pinned::fetch_pixls(false)?;
    }
    fetch_all(ours.contains(&"--local"), false)?;
    let mut c = cargo();
    c.args(["test", "--release", "--lib", "--tests"]);
    for k in &crates {
        c.args(["-p", k]);
    }
    let mut features: Vec<String> = crates.iter().map(|k| format!("{k}/corpus")).collect();
    features.extend(crates.iter().filter(|k| HEIF_CRATES.contains(&k.as_str())).map(|k| format!("{k}/heif")));
    c.args(["--features", &features.join(",")]);
    if !passthrough.is_empty() {
        c.arg("--").args(passthrough);
    }
    run(c, &format!("cargo test --release --features corpus,heif ({})", crates.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The flags each pinned corpus is fetched with by `fetch_all(local, update)`.
    fn calls(local: bool, update: bool) -> Vec<(&'static str, bool)> {
        let mut seen = Vec::new();
        let r = fetch_pinned(local, update, |c, _, u| {
            seen.push((c.name, u));
            Ok(())
        });
        assert_eq!(r, Ok(()));
        seen
    }

    #[test]
    fn all_with_update_manifest_updates_every_pinned_corpus() {
        for local in [false, true] {
            let seen = calls(local, true);
            assert_eq!(seen.len(), corpus_pins::ALL.len());
            assert!(seen.iter().all(|(_, u)| *u), "--all --update-manifest dropped the flag: {seen:?}");
        }
    }

    #[test]
    fn all_without_update_manifest_only_verifies() {
        assert!(calls(false, false).iter().all(|(_, u)| !*u));
    }
}
