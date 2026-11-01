//! Exercise the same manifest reader used by build.rs, with synthetic files only.
#[path = "../build_manifest.rs"]
mod build_manifest;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

struct TempDir(PathBuf);
impl TempDir {
    fn new(parent: &Path) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = parent.join(format!("photocraft-font-manifest-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("creating synthetic fixture: {e}"),
            }
        }
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn checkout() -> TempDir {
    let dir = TempDir::new(&std::env::temp_dir());
    std::fs::create_dir_all(dir.path().join("fonts/face")).unwrap();
    std::fs::write(dir.path().join("fonts/face/A.ttf"), b"abc").unwrap();
    std::fs::write(dir.path().join("outside.txt"), b"synthetic outside data").unwrap();
    dir
}

const HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn manifest(dir: &Path, file: &str, hash: &str) {
    std::fs::write(
        dir.join("fonts/manifest.txt"),
        format!("# synthetic manifest\n\nFace | Regular | {file} | Latn,Jpan | OFL-1.1 | fonts/face/OFL.txt | {hash} | https://example.invalid/font\n"),
    )
    .unwrap();
}

#[test]
fn valid_external_checkout_and_relative_checkout() {
    let dir = checkout();
    manifest(dir.path(), "fonts/face/A.ttf", HASH);
    let src = build_manifest::craft_fonts(dir.path()).unwrap();
    assert!(src.contains("family: \"Face\""));
    assert!(src.contains("scripts: &[\"Latn\", \"Jpan\"]"));
    assert!(src.contains(&format!("include_bytes!({:?})", dir.path().join("fonts/face/A.ttf").canonicalize().unwrap().display().to_string())));
    // A relative CRAFT_FONTS_DIR remains relative to the build script's working directory.
    let cwd = std::env::current_dir().unwrap();
    let local = TempDir::new(&cwd);
    std::fs::create_dir_all(local.path().join("fonts")).unwrap();
    std::fs::write(local.path().join("fonts/manifest.txt"), "# no fonts\n").unwrap();
    assert_eq!(build_manifest::craft_fonts(local.path().strip_prefix(cwd).unwrap()).unwrap(), "");
}

#[test]
fn refuses_traversal_absolute_and_non_files() {
    let dir = checkout();
    let absolute = dir.path().join("fonts/face/A.ttf").canonicalize().unwrap().display().to_string();
    for bad in [
        "outside.txt",
        "fonts/../outside.txt",
        "fonts/face/../../outside.txt",
        "../outside.txt",
        "/outside.txt",
        "C:/outside.txt",
        "C:outside.txt",
        "\\outside.txt",
        "\\\\server\\share\\font.ttf",
        "\\\\?\\C:\\outside.txt",
        "fonts\\..\\outside.txt",
        "fonts/face/A.ttf:stream",
        "fonts/face",
        "fonts/missing.ttf",
        &absolute,
    ] {
        manifest(dir.path(), bad, HASH);
        assert!(build_manifest::craft_fonts(dir.path()).is_err(), "accepted {bad}");
    }
}

#[test]
fn checks_the_seventh_column_digest() {
    let dir = checkout();
    for bad in
        ["", "1234", "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD", "0000000000000000000000000000000000000000000000000000000000000000"]
    {
        manifest(dir.path(), "fonts/face/A.ttf", bad);
        assert!(build_manifest::craft_fonts(dir.path()).is_err(), "accepted digest {bad:?}");
    }
    manifest(dir.path(), "fonts/face/A.ttf", HASH);
    std::fs::write(dir.path().join("fonts/face/A.ttf"), b"changed").unwrap();
    assert!(build_manifest::craft_fonts(dir.path()).unwrap_err().contains("SHA-256 mismatch"));
}

#[test]
fn malformed_or_missing_manifest_is_an_error() {
    let dir = checkout();
    assert!(build_manifest::craft_fonts(dir.path()).is_err());
    std::fs::write(dir.path().join("fonts/manifest.txt"), "Face | Regular | fonts/face/A.ttf | Latn\n").unwrap();
    assert!(build_manifest::craft_fonts(dir.path()).is_err());
}

#[cfg(unix)]
fn symlink_file(from: &Path, to: &Path) {
    std::os::unix::fs::symlink(from, to).unwrap();
}
#[cfg(unix)]
fn symlink_dir(from: &Path, to: &Path) {
    std::os::unix::fs::symlink(from, to).unwrap();
}
#[cfg(windows)]
fn symlink_dir(from: &Path, to: &Path) {
    // Junctions exercise Windows reparse-point containment without requiring symlink privileges.
    let output = std::process::Command::new("cmd.exe")
        .args(["/c", "mklink", "/J"])
        .arg(to.as_os_str().to_string_lossy().replace('/', "\\"))
        .arg(from.as_os_str().to_string_lossy().replace('/', "\\"))
        .output()
        .unwrap();
    assert!(output.status.success(), "junction creation failed: {output:?}");
}

#[test]
#[cfg(unix)]
fn resolves_internal_symlinks_but_rejects_file_and_directory_escapes() {
    let dir = checkout();
    symlink_file(&dir.path().join("fonts/face/A.ttf"), &dir.path().join("fonts/inside.ttf"));
    manifest(dir.path(), "fonts/inside.ttf", HASH);
    assert!(build_manifest::craft_fonts(dir.path()).is_ok());
    symlink_file(&dir.path().join("outside.txt"), &dir.path().join("fonts/escape.ttf"));
    symlink_dir(dir.path(), &dir.path().join("fonts/escape-dir"));
    for bad in ["fonts/escape.ttf", "fonts/escape-dir/outside.txt"] {
        manifest(dir.path(), bad, HASH);
        assert!(build_manifest::craft_fonts(dir.path()).unwrap_err().contains("outside"));
    }
}

#[test]
#[cfg(unix)]
fn manifest_link_cannot_read_outside_fonts() {
    let dir = checkout();
    symlink_file(&dir.path().join("outside.txt"), &dir.path().join("fonts/manifest.txt"));
    assert!(build_manifest::craft_fonts(dir.path()).unwrap_err().contains("outside"));
}

#[test]
fn directory_links_cannot_escape_fonts() {
    let dir = checkout();
    symlink_dir(&dir.path().join("fonts/face"), &dir.path().join("fonts/inside"));
    manifest(dir.path(), "fonts/inside/A.ttf", HASH);
    assert!(build_manifest::craft_fonts(dir.path()).is_ok());
    symlink_dir(dir.path(), &dir.path().join("fonts/escape"));
    manifest(dir.path(), "fonts/escape/outside.txt", HASH);
    assert!(build_manifest::craft_fonts(dir.path()).unwrap_err().contains("outside"));
}

#[test]
fn symlinked_checkout_is_valid_but_fonts_root_cannot_redirect_trust() {
    let dir = checkout();
    let links = TempDir::new(&std::env::temp_dir());
    symlink_dir(dir.path(), &links.path().join("checkout"));
    manifest(dir.path(), "fonts/face/A.ttf", HASH);
    assert!(build_manifest::craft_fonts(&links.path().join("checkout")).is_ok());
    std::fs::remove_dir_all(dir.path().join("fonts")).unwrap();
    // A malicious fonts/ symlink must not redefine the containment root.
    symlink_dir(links.path(), &dir.path().join("fonts"));
    std::fs::write(links.path().join("manifest.txt"), "# empty\n").unwrap();
    assert!(build_manifest::craft_fonts(dir.path()).unwrap_err().contains("fonts/"));
}
