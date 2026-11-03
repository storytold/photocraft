use super::{AtomicFs, RealFs, sync_file, temp_name};
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const MAX_CANDIDATES: u32 = 100_000;

/// Write to a free filename, adding numeric suffixes on collisions without overwriting files.
pub fn write_unique(path: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    write_unique_with(&RealFs, path, bytes, &|from: &Path, to: &Path| std::fs::hard_link(from, to), &write_and_sync)
        .map_err(|e| io::Error::new(e.kind(), format!("cannot export {}: {e}", path.display())))
}

struct Staged<'a> {
    fs: &'a dyn AtomicFs,
    path: PathBuf,
    cleanup_pending: bool,
}
impl Drop for Staged<'_> {
    /// Clean up private staging on early returns.
    fn drop(&mut self) {
        if self.cleanup_pending {
            let _ = self.fs.remove(&self.path);
        }
    }
}

/// Write fallback bytes and flush them with the platform sync policy.
fn write_and_sync(output: &mut File, bytes: &[u8]) -> io::Result<()> {
    output.write_all(bytes)?;
    sync_file(output)
}

/// Publish without replacement, with injectable link and write operations for failure tests.
fn write_unique_with(
    fs: &dyn AtomicFs,
    path: &Path,
    bytes: &[u8],
    link: &dyn Fn(&Path, &Path) -> io::Result<()>,
    write: &dyn Fn(&mut File, &[u8]) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let last_component = path.as_os_str().as_encoded_bytes().rsplit(|&b| std::path::is_separator(char::from(b))).next().unwrap_or_default();
    if last_component.is_empty() || last_component == b"." || last_component == b".." {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Quick Export needs a filename"));
    }
    let stem = path.file_stem().filter(|s| !s.is_empty()).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Quick Export needs a filename"))?;
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut staged = None;
    for _ in 0..8 {
        let tmp = dir.join(temp_name("export"));
        match fs.write_new(&tmp, bytes) {
            Ok(()) => {
                staged = Some(Staged { fs, path: tmp, cleanup_pending: true });
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                let _ = fs.remove(&tmp);
                return Err(e);
            }
        }
    }
    let mut staged = staged.ok_or_else(|| io::Error::other("could not create a Quick Export staging file"))?;
    for suffix in 0..MAX_CANDIDATES {
        let candidate = if suffix == 0 {
            path.to_path_buf()
        } else {
            let mut name = stem.to_os_string();
            name.push(format!("_{suffix}"));
            if let Some(ext) = path.extension() {
                name.push(".");
                name.push(ext);
            }
            path.with_file_name(name)
        };
        let linked = if staged.cleanup_pending {
            match link(&staged.path, &candidate) {
                Ok(()) => true,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(_) => {
                    fs.remove(&staged.path)?;
                    staged.cleanup_pending = false;
                    false
                }
            }
        } else {
            false
        };
        if !linked {
            let mut output = match std::fs::OpenOptions::new().write(true).create_new(true).open(&candidate) {
                Ok(file) => file,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            };
            if let Err(e) = write(&mut output, bytes) {
                // The pathname may now refer to another process's file.
                return Err(io::Error::other(format!("could not write {}: {e}; output may be incomplete and was not removed", candidate.display())));
            }
        }
        if staged.cleanup_pending {
            fs.remove(&staged.path).map_err(|e| io::Error::other(format!("wrote {}, but staging cleanup failed: {e}", candidate.display())))?;
            staged.cleanup_pending = false;
        }
        let synced = fs.sync_dir(dir);
        #[cfg(not(windows))]
        let synced = super::dir_synced(synced);
        synced.map_err(|e| io::Error::other(format!("wrote {}, but could not confirm durability: {e}", candidate.display())))?;
        return Ok(candidate);
    }
    Err(io::Error::other("Quick Export exhausted 100000 filename candidates"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(temp_name(tag));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn assert_no_temp(dir: &Path) {
        assert!(!std::fs::read_dir(dir).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
    }

    /// Fill the first collision gap without changing existing files on either publication path.
    #[test]
    fn exclusive_publication_fills_gaps_with_and_without_hard_links() {
        for fallback in [false, true] {
            let dir = TempDir::new("unique");
            for name in ["写真.edit_2.png", "写真.edit_2_1.png", "写真.edit_2_3.png"] {
                std::fs::write(dir.0.join(name), b"original").unwrap();
            }
            let link = |a: &Path, b: &Path| if fallback { Err(io::ErrorKind::Unsupported.into()) } else { std::fs::hard_link(a, b) };
            let out = write_unique_with(&RealFs, &dir.0.join("写真.edit_2.png"), b"export", &link, &write_and_sync).unwrap();
            assert_eq!(out, dir.0.join("写真.edit_2_2.png"));
            assert_eq!(std::fs::read(out).unwrap(), b"export");
            assert_eq!(std::fs::read(dir.0.join("写真.edit_2.png")).unwrap(), b"original");
            assert_no_temp(&dir.0);
        }
    }

    /// Release staging before fallback creation, including a competing claim on the base name.
    #[test]
    fn fallback_releases_staging_before_creating_output_and_retries_collisions() {
        struct FallbackFs {
            target: PathBuf,
            collide: bool,
            removals: Cell<usize>,
            removed_before_output: Cell<bool>,
        }
        impl AtomicFs for FallbackFs {
            fn write_new(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
                RealFs.write_new(path, bytes)
            }
            fn rename(&self, _: &Path, _: &Path) -> io::Result<()> {
                panic!("exclusive publication must never replace")
            }
            /// Optionally claim the base name at staging removal to simulate a competing export.
            fn remove(&self, path: &Path) -> io::Result<()> {
                self.removed_before_output.set(!self.target.exists());
                RealFs.remove(path)?;
                self.removals.set(self.removals.get() + 1);
                if self.collide {
                    std::fs::write(&self.target, b"original")?;
                }
                Ok(())
            }
            fn sync_dir(&self, dir: &Path) -> io::Result<()> {
                RealFs.sync_dir(dir)
            }
            fn copy_metadata(&self, _: &Path, _: &Path) -> io::Result<()> {
                panic!("existing candidate metadata must not be copied")
            }
        }
        for collide in [false, true] {
            let dir = TempDir::new("unique-fallback");
            let fs = FallbackFs { target: dir.0.join("photo.png"), collide, removals: Cell::new(0), removed_before_output: Cell::new(false) };
            let links = Cell::new(0);
            let link = |from: &Path, _: &Path| {
                assert!(from.exists(), "a removed staging file must not be linked again");
                links.set(links.get() + 1);
                Err(io::ErrorKind::Unsupported.into())
            };
            let out = write_unique_with(&fs, &fs.target, b"export", &link, &write_and_sync).unwrap();
            assert!(fs.removed_before_output.get(), "staging must be released before creating the output");
            assert_eq!(links.get(), 1);
            assert_eq!(fs.removals.get(), 1);
            assert_eq!(out, dir.0.join(if collide { "photo_1.png" } else { "photo.png" }));
            assert_eq!(std::fs::read(out).unwrap(), b"export");
            if collide {
                assert_eq!(std::fs::read(&fs.target).unwrap(), b"original");
            }
            assert_no_temp(&dir.0);
        }
    }

    /// Keep concurrent exports distinct and complete with and without hard-link support.
    #[test]
    fn concurrent_exports_publish_distinct_complete_files() {
        for fallback in [false, true] {
            let dir = TempDir::new("concurrent-unique");
            let handles: Vec<_> = (0..8u8)
                .map(|n| {
                    let path = dir.0.join("photo.png");
                    std::thread::spawn(move || {
                        let link = |a: &Path, b: &Path| if fallback { Err(io::ErrorKind::Unsupported.into()) } else { std::fs::hard_link(a, b) };
                        let bytes = vec![n; 4096];
                        let out = write_unique_with(&RealFs, &path, &bytes, &link, &write_and_sync).unwrap();
                        assert_eq!(std::fs::read(&out).unwrap(), bytes);
                        out
                    })
                })
                .collect();
            let names: std::collections::HashSet<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
            assert_eq!(names.len(), 8);
            assert_no_temp(&dir.0);
        }
    }

    struct FailingFs {
        stage: bool,
        removals: Cell<usize>,
    }
    impl AtomicFs for FailingFs {
        /// Optionally fail after staging creation so error cleanup is observable.
        fn write_new(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
            RealFs.write_new(path, bytes)?;
            if self.stage { Err(io::ErrorKind::StorageFull.into()) } else { Ok(()) }
        }
        fn rename(&self, _: &Path, _: &Path) -> io::Result<()> {
            panic!("exclusive publication must never replace")
        }
        fn remove(&self, path: &Path) -> io::Result<()> {
            self.removals.set(self.removals.get() + 1);
            RealFs.remove(path)
        }
        fn sync_dir(&self, _: &Path) -> io::Result<()> {
            Err(io::ErrorKind::AlreadyExists.into())
        }
        fn copy_metadata(&self, _: &Path, _: &Path) -> io::Result<()> {
            panic!("existing candidate metadata must not be copied")
        }
    }

    /// Clean staging once and avoid another export after publication succeeds.
    #[test]
    fn failures_cleanup_staging_and_never_retry_after_publication() {
        let dir = TempDir::new("unique-errors");
        let path = dir.0.join("photo.png");
        let link = |a: &Path, b: &Path| std::fs::hard_link(a, b);
        let failed_stage = FailingFs { stage: true, removals: Cell::new(0) };
        assert!(write_unique_with(&failed_stage, &path, b"bytes", &link, &write_and_sync).is_err());
        assert_eq!(failed_stage.removals.get(), 1);
        assert!(!path.exists());
        assert_no_temp(&dir.0);
        let failed_sync = FailingFs { stage: false, removals: Cell::new(0) };
        let error = write_unique_with(&failed_sync, &path, b"bytes", &link, &write_and_sync).unwrap_err();
        assert_eq!(failed_sync.removals.get(), 1);
        assert!(error.to_string().contains("could not confirm durability"));
        assert_eq!(std::fs::read(&path).unwrap(), b"bytes");
        assert!(!dir.0.join("photo_1.png").exists());
        assert_no_temp(&dir.0);
    }

    /// Reject invalid destinations without creating directories or changing existing files.
    #[test]
    fn invalid_paths_and_long_names_fail_without_touching_existing_files() {
        let dir = TempDir::new("unique-invalid");
        let file = dir.0.join("file");
        std::fs::write(&file, b"original").unwrap();
        for invalid in ["folder/", "folder/.", "folder/.."] {
            let error = write_unique(&dir.0.join(invalid), b"bytes").unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        }
        #[cfg(windows)]
        for invalid in ["folder\\", "folder\\.", "folder\\.."] {
            assert_eq!(write_unique(&dir.0.join(invalid), b"bytes").unwrap_err().kind(), io::ErrorKind::InvalidInput);
        }
        assert!(!dir.0.join("folder").exists());
        assert!(write_unique(&file.join("photo.png"), b"bytes").is_err());
        assert!(write_unique(&dir.0.join(format!("{}.png", "x".repeat(300))), b"bytes").is_err());
        assert_eq!(std::fs::read(file).unwrap(), b"original");
        assert_no_temp(&dir.0);
    }

    /// Keep partial outputs and replacement files when writes or syncs fail, without retrying.
    #[test]
    fn fallback_failures_preserve_partial_outputs_and_replacements_without_retrying() {
        for replace in [false, true] {
            for sync_failure in [false, true] {
                let dir = TempDir::new("unique-failed-output");
                let path = dir.0.join("photo.png");
                let moved = dir.0.join("moved.png");
                let attempts = Cell::new(0);
                let link = |_: &Path, _: &Path| Err(io::ErrorKind::Unsupported.into());
                let write = |output: &mut File, bytes: &[u8]| {
                    attempts.set(attempts.get() + 1);
                    output.write_all(if sync_failure { bytes } else { b"exp" })?;
                    if replace {
                        std::fs::rename(&path, &moved)?;
                        std::fs::write(&path, b"replacement")?;
                    }
                    Err(if sync_failure { io::ErrorKind::AlreadyExists } else { io::ErrorKind::StorageFull }.into())
                };
                let error = write_unique_with(&RealFs, &path, b"export", &link, &write).unwrap_err();
                assert_eq!(attempts.get(), 1);
                assert_eq!(error.kind(), io::ErrorKind::Other);
                assert!(error.to_string().contains(&path.display().to_string()));
                assert!(error.to_string().contains("output may be incomplete and was not removed"));
                let expected: &[u8] = if sync_failure { b"export" } else { b"exp" };
                if replace {
                    assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
                    assert_eq!(std::fs::read(&moved).unwrap(), expected);
                } else {
                    assert_eq!(std::fs::read(&path).unwrap(), expected);
                }
                assert!(!dir.0.join("photo_1.png").exists());
                assert_no_temp(&dir.0);
            }
        }
    }

    /// Treat dangling symlinks as occupied names while preserving non-Unicode filename bytes.
    #[cfg(unix)]
    #[test]
    fn dangling_symlink_is_a_collision_and_non_unicode_paths_are_preserved() {
        use std::os::unix::{ffi::OsStrExt, fs::symlink};
        let dir = TempDir::new("unique-symlink");
        let path = dir.0.join(std::ffi::OsStr::from_bytes(b"photo_\xff.png"));
        symlink(dir.0.join("missing"), &path).unwrap();
        let out = write_unique(&path, b"bytes").unwrap();
        assert_eq!(out.file_name().unwrap().as_bytes(), b"photo_\xff_1.png");
        assert!(std::fs::symlink_metadata(path).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read(out).unwrap(), b"bytes");
    }
}
