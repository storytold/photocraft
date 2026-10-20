//! The build-script helper `git_checkout.rs` (#5247): Cargo re-runs a build script on every build
//! when a `rerun-if-changed` path is missing, so only existing paths may be watched.

#[allow(dead_code)]
#[path = "../git_checkout.rs"]
mod git_checkout;

use git_checkout::watched;
use std::fs;
use std::path::PathBuf;

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
