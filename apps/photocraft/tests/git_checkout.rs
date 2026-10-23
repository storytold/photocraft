//! The build-script helper `git_checkout.rs` (#5247): Cargo re-runs a build script on every build
//! when a `rerun-if-changed` path is missing, so only existing paths may be watched.

#[allow(dead_code)]
#[path = "../git_checkout.rs"]
mod git_checkout;

use git_checkout::{watch_paths, watched};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("photocraft-git-checkout-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn an_existing_path_is_watched_itself() {
    let git = scratch("exists");
    let packed = git.join("packed-refs");
    fs::write(&packed, "").expect("write");
    assert_eq!(watched(&packed, &packed), Some(packed.clone()));
    let _ = fs::remove_dir_all(&git);
}

#[test]
fn a_missing_file_with_no_room_to_climb_is_not_watched() {
    let git = scratch("missing");
    let packed = git.join("packed-refs");
    assert_eq!(watched(&packed, &packed), None);
    let _ = fs::remove_dir_all(&git);
}

#[test]
fn a_packed_branch_watches_the_nearest_existing_ref_directory() {
    let git = scratch("packed-branch");
    let refs = git.join("refs");
    fs::create_dir_all(refs.join("heads")).expect("refs/heads");
    // `refs/heads/fix/5247` with no `fix/` directory yet: watch `refs/heads`.
    assert_eq!(watched(&refs.join("heads/fix/5247"), &refs), Some(refs.join("heads")));
    // Once the loose ref exists it is watched directly.
    fs::create_dir_all(refs.join("heads/fix")).expect("refs/heads/fix");
    fs::write(refs.join("heads/fix/5247"), "abc\n").expect("loose ref");
    assert_eq!(watched(&refs.join("heads/fix/5247"), &refs), Some(refs.join("heads/fix/5247")));
    let _ = fs::remove_dir_all(&git);
}

#[test]
fn the_search_never_leaves_the_stop_directory() {
    let git = scratch("stop");
    // `refs/` itself is missing: its parent (the git dir) exists but is outside `stop`.
    assert_eq!(watched(&git.join("refs/heads/main"), &git.join("refs")), None);
    let _ = fs::remove_dir_all(&git);
}

/// `git init` in a fresh scratch directory with `args`; `None` when this git cannot do it.
fn init(name: &str, args: &[&str]) -> Option<PathBuf> {
    let dir = scratch(name);
    let ok = Command::new("git").arg("init").arg("-q").args(args).arg(&dir).status().is_ok_and(|s| s.success());
    if ok {
        dir.canonicalize().ok()
    } else {
        let _ = fs::remove_dir_all(&dir);
        None
    }
}

fn git_dir(root: &Path) -> PathBuf {
    root.join(".git")
}

/// `watch_paths(root)` canonicalised, so git's `C:/…` output compares equal to the scratch path on
/// Windows and `/var` to `/private/var` on macOS. Panics if any watched path is missing.
fn watched_in(root: &Path) -> Vec<PathBuf> {
    watch_paths(root).iter().map(|p| p.canonicalize().unwrap_or_else(|e| panic!("{} is watched but missing: {e}", p.display()))).collect()
}

#[test]
fn an_unborn_branch_still_watches_head_and_the_ref_directory() {
    // Before the first commit `rev-parse HEAD` fails; the first commit creates refs/heads/main,
    // which must re-run the build script.
    let Some(root) = init("unborn", &["-b", "main"]) else {
        eprintln!("git init unavailable, skipped");
        return;
    };
    let paths = watched_in(&root);
    let git = git_dir(&root);
    assert!(paths.contains(&git.join("HEAD")), "{paths:?}");
    assert!(paths.iter().any(|p| p.starts_with(git.join("refs"))), "{paths:?}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_reftable_checkout_watches_the_table_stack() {
    // Branch updates in a reftable repository rewrite reftable/tables.list, not a loose ref or
    // packed-refs. Needs git 2.45 or newer; older git cannot create the repository.
    let Some(root) = init("reftable", &["-b", "main", "--ref-format=reftable"]) else {
        eprintln!("git without reftable support, skipped");
        return;
    };
    let paths = watched_in(&root);
    let git = git_dir(&root);
    assert!(paths.contains(&git.join("reftable").join("tables.list")), "{paths:?}");
    assert!(paths.contains(&git.join("HEAD")), "{paths:?}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_files_checkout_watches_the_branch_ref_and_not_a_table_stack() {
    let Some(root) = init("files", &["-b", "main"]) else {
        eprintln!("git init unavailable, skipped");
        return;
    };
    fs::write(git_dir(&root).join("packed-refs"), "").expect("packed-refs");
    let paths = watched_in(&root);
    let git = git_dir(&root);
    assert!(paths.contains(&git.join("packed-refs")), "{paths:?}");
    assert!(!paths.iter().any(|p| p.starts_with(git.join("reftable"))), "{paths:?}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn outside_a_repository_nothing_is_watched() {
    let dir = scratch("no-repo");
    // GIT_CEILING_DIRECTORIES is not set here, so a scratch dir inside a checkout would find it;
    // the system temp dir is not inside one on CI or on a developer machine.
    if Command::new("git").arg("-C").arg(&dir).args(["rev-parse", "--git-dir"]).output().is_ok_and(|o| o.status.success()) {
        eprintln!("temp dir is inside a git repository, skipped");
    } else {
        assert!(watched_in(&dir).is_empty());
    }
    let _ = fs::remove_dir_all(&dir);
}
