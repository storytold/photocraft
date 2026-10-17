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
    let Some(sha) = git(&root, &["rev-parse", "HEAD"]) else { return };
    // Rebuild when HEAD moves: a commit or checkout changes HEAD (detached) or the branch's ref.
    if let Some(dir) = git(&root, &["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={dir}/HEAD");
    }
    if let (Some(common), Some(head_ref)) =
        (git(&root, &["rev-parse", "--path-format=absolute", "--git-common-dir"]), git(&root, &["symbolic-ref", "-q", "HEAD"]))
    {
        println!("cargo:rerun-if-changed={common}/{head_ref}");
        println!("cargo:rerun-if-changed={common}/packed-refs");
    }
    println!("cargo:rustc-env=PHOTOCRAFT_CHECKOUT_SHA={sha}");
}
