//! Build-script helper shared by the `photocraft` and `photocraft-cli` binaries: the git commit
//! of the source checkout, so a plain `cargo build` from git reports `0.8.2 (dev build, 6f8505421)`
//! (#5247). Release builds set `PHOTOCRAFT_BUILD_SHA` instead and are left alone; without git or
//! outside a checkout of this workspace (a source tarball) nothing is set and the build stays
//! `(dev build)`. The binaries pass the value to `photocraft_engine::build_info::set_checkout`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs `git <args>` in `dir`; its trimmed output when it succeeds.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}

/// Emits `PHOTOCRAFT_CHECKOUT_SHA` for the crate whose build script calls it.
pub fn emit() {
    println!("cargo:rerun-if-env-changed=PHOTOCRAFT_BUILD_SHA");
    if std::env::var("PHOTOCRAFT_BUILD_SHA").is_ok_and(|s| !s.trim().is_empty()) {
        return;
    }
    let Some(manifest) = std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from) else { return };
    // apps/<crate> → the workspace root. A tarball unpacked inside some other repository must not
    // report that repository's commit.
    let Some(root) = manifest.parent().and_then(Path::parent).and_then(|r| r.canonicalize().ok()) else { return };
    let Some(top) = git(&root, &["rev-parse", "--show-toplevel"]).and_then(|t| PathBuf::from(t).canonicalize().ok()) else { return };
    if top != root {
        return;
    }
    // Watch before asking for the commit: on an unborn branch `rev-parse HEAD` fails, and the
    // first commit must still re-run this script.
    for path in watch_paths(&root) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let Some(sha) = git(&root, &["rev-parse", "HEAD"]) else { return };
    println!("cargo:rustc-env=PHOTOCRAFT_CHECKOUT_SHA={sha}");
}

/// The git files whose change means HEAD moved: a commit or checkout changes HEAD (detached) or the
/// branch's ref. Cargo treats a watched path that does not exist as always changed (it re-runs the
/// build script and recompiles the binary on every build), so only existing paths are returned.
pub(crate) fn watch_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(dir) = git(root, &["rev-parse", "--absolute-git-dir"]) {
        let head = Path::new(&dir).join("HEAD");
        paths.extend(watched(&head, &head));
    }
    let Some(common) = git(root, &["rev-parse", "--path-format=absolute", "--git-common-dir"]) else { return paths };
    let common = Path::new(&common);
    if git(root, &["rev-parse", "--show-ref-format"]).as_deref() == Some("reftable") {
        // Every ref update in a reftable repository rewrites the table stack.
        let tables = common.join("reftable").join("tables.list");
        paths.extend(watched(&tables, &tables));
        return paths;
    }
    if let Some(head_ref) = git(root, &["symbolic-ref", "-q", "HEAD"]) {
        // A packed or unborn branch has no loose ref file; the first commit on it creates one,
        // which changes the nearest existing directory above it under `refs/`.
        paths.extend(watched(&common.join(&head_ref), &common.join("refs")));
    }
    let packed = common.join("packed-refs");
    paths.extend(watched(&packed, &packed));
    paths
}

/// The path Cargo should watch for `path`: `path` itself when it exists, else its nearest existing
/// ancestor that is still inside `stop`; `None` when neither exists.
pub(crate) fn watched(path: &Path, stop: &Path) -> Option<PathBuf> {
    path.ancestors().take_while(|p| p.starts_with(stop)).find(|p| p.exists()).map(Path::to_path_buf)
}
