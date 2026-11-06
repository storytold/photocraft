//! Exclusive publication and rename for files in one directory.
//!
//! Prefer a native exclusive rename, with atomic hard-link publication when that operation is
//! unavailable. The caller handles failures between separate file operations. Never replace
//! an exclusive operation with an existence check followed by an overwriting rename.

use std::{io, path::Path};

/// Publication failure distinguishes an unchanged destination from a published file whose cleanup or directory flush failed.
#[derive(Debug)]
pub struct PublishNewError {
    pub published: bool,
    pub error: io::Error,
}

impl std::fmt::Display for PublishNewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.published { write!(f, "file published; completion unconfirmed: {}", self.error) } else { std::fmt::Display::fmt(&self.error, f) }
    }
}
impl std::error::Error for PublishNewError {}

/// Publish a complete, flushed file only if its destination is still absent.
/// Temporary files are created exclusively beside the destination; only our own temp is removed.
pub fn write_noreplace(path: &Path, bytes: &[u8]) -> Result<(), PublishNewError> {
    use crate::atomic::{AtomicFs, RealFs};
    use std::{
        io::Write,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let before = |error| PublishNewError { published: false, error };
    let dir = path.parent().ok_or_else(|| before(io::Error::new(io::ErrorKind::InvalidInput, "missing publication directory")))?;
    // A bare filename still needs a real directory path for the final flush.
    let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
    let mut owned = None;
    for _ in 0..32 {
        let temp = dir.join(format!(".pc-new-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => {
                owned = Some((temp, file));
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(before(error)),
        }
    }
    let (temp, mut file) = owned.ok_or_else(|| before(io::Error::new(io::ErrorKind::AlreadyExists, "no unused publication temp name")))?;
    let written = file.write_all(bytes).and_then(|_| crate::atomic::sync_file(&file));
    drop(file);
    let published = written
        .map_err(before)
        .and_then(|()| move_noreplace(&temp, path, platform, |from, to| std::fs::hard_link(from, to), |path| std::fs::remove_file(path)));
    if published.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    finish_publication(published, || RealFs.sync_dir(dir))
}

/// Rename within one directory, refusing an existing destination atomically.
/// Unsupported rename primitives use a hard link followed by unlink, never an overwriting
/// rename or cross-volume copy. If unlink fails, both names retain the same bytes and this
/// returns an error, so the caller reports retirement as incomplete.
/// The caller must flush the directory before reporting crash-durable migration completion;
/// the existing `atomic::AtomicFs::sync_dir` provides that separately fallible operation.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    if from == to || from.parent() != to.parent() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "no-replace retirement requires distinct names in one directory"));
    }
    move_noreplace(from, to, platform, |from, to| std::fs::hard_link(from, to), |path| std::fs::remove_file(path)).map_err(|failure| failure.error)
}

/// Use fallback only when the exclusive rename primitive itself is unsupported.
fn unsupported_rename(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::Unsupported {
        return true;
    }
    #[cfg(unix)]
    if matches!(error.raw_os_error(), Some(code) if code == rustix::io::Errno::INVAL.raw_os_error() || code == rustix::io::Errno::NOSYS.raw_os_error()) {
        return true;
    }
    false
}

/// Linking claims an absent destination atomically; a later unlink failure is already published.
fn move_noreplace(
    from: &Path,
    to: &Path,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
    link: impl FnOnce(&Path, &Path) -> io::Result<()>,
    remove: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), PublishNewError> {
    match rename(from, to) {
        Ok(()) => Ok(()),
        Err(error) if unsupported_rename(&error) => {
            link(from, to).map_err(|error| PublishNewError { published: false, error })?;
            remove(from).map_err(|error| PublishNewError { published: true, error })
        }
        Err(error) => Err(PublishNewError { published: false, error }),
    }
}

/// A successful name change is durable only after the separately fallible directory flush.
fn finish_publication(result: Result<(), PublishNewError>, sync: impl FnOnce() -> io::Result<()>) -> Result<(), PublishNewError> {
    match result {
        Ok(()) => sync().map_err(|error| PublishNewError { published: true, error }),
        Err(failure) => {
            if failure.published {
                let _ = sync();
            }
            Err(failure)
        }
    }
}

#[cfg(windows)]
fn platform(from: &Path, to: &Path) -> io::Result<()> {
    let from = from.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "legacy file path is not valid Unicode"))?;
    let to = to.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "backup path is not valid Unicode"))?;
    // MoveFile (unlike MoveFileEx with REPLACE_EXISTING) fails if the destination exists.
    // The same-directory restriction above also excludes its cross-volume copy behavior.
    winsafe::MoveFile(from, to).map_err(|error| io::Error::from_raw_os_error(error.raw() as i32))
}

#[cfg(all(unix, any(target_os = "linux", target_os = "android", target_os = "redox", target_vendor = "apple")))]
fn platform(from: &Path, to: &Path) -> io::Result<()> {
    // rustix maps NOREPLACE to Linux renameat2 or Apple's RENAME_EXCL, without unsafe FFI here.
    rustix::fs::renameat_with(rustix::fs::CWD, from, rustix::fs::CWD, to, rustix::fs::RenameFlags::NOREPLACE).map_err(io::Error::from)
}

#[cfg(not(any(windows, all(unix, any(target_os = "linux", target_os = "android", target_os = "redox", target_vendor = "apple")))))]
fn platform(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "this platform has no supported no-replace retirement primitive"))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    struct Dir(PathBuf);
    impl Dir {
        fn new() -> io::Result<Self> {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("pc-noreplace-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            // Never clean a preexisting directory to make the fixture pass.
            std::fs::create_dir(&path)?;
            Ok(Self(path))
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn retirement_moves_the_original_bytes_without_overwriting_a_backup() {
        let dir = Dir::new().unwrap();
        let from = dir.0.join("legacy");
        let to = dir.0.join("retained");
        std::fs::write(&from, b"exact original bytes").unwrap();
        std::fs::write(&to, b"existing backup").unwrap();
        assert_eq!(rename_noreplace(&from, &to).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&from).unwrap(), b"exact original bytes");
        assert_eq!(std::fs::read(&to).unwrap(), b"existing backup");
        std::fs::remove_file(&to).unwrap();
        rename_noreplace(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(std::fs::read(&to).unwrap(), b"exact original bytes");
    }

    #[test]
    fn retirement_refuses_same_name_and_missing_source_without_changing_existing_data() {
        let dir = Dir::new().unwrap();
        let from = dir.0.join("legacy");
        let to = dir.0.join("retained");
        std::fs::write(&to, b"existing backup").unwrap();
        assert!(rename_noreplace(&from, &to).is_err());
        assert_eq!(std::fs::read(&to).unwrap(), b"existing backup");
        assert_eq!(rename_noreplace(&to, &to).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(std::fs::read(&to).unwrap(), b"existing backup");
    }

    #[test]
    fn publication_keeps_an_existing_destination_and_leaves_no_owned_temp() {
        let dir = Dir::new().unwrap();
        let path = dir.0.join("actions.atn");
        std::fs::write(&path, b"existing ATN").unwrap();
        let error = write_noreplace(&path, b"replacement").unwrap_err();
        assert!(!error.published);
        assert_eq!(std::fs::read(&path).unwrap(), b"existing ATN");
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[test]
    fn publication_creates_complete_bytes_then_removes_its_temp_name() {
        let dir = Dir::new().unwrap();
        let path = dir.0.join("actions.atn");
        write_noreplace(&path, b"complete ATN").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"complete ATN");
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    /// An existing destination survives both the fallback and the native primitive unchanged.
    #[test]
    fn fallback_is_exclusive_and_moves_complete_bytes() {
        let dir = Dir::new().unwrap();
        let (from, to) = (dir.0.join("legacy"), dir.0.join("backup"));
        std::fs::write(&from, b"original").unwrap();
        std::fs::write(&to, b"existing").unwrap();
        let unsupported = |_: &Path, _: &Path| Err(io::Error::from(io::ErrorKind::Unsupported));
        let failed = move_noreplace(&from, &to, unsupported, |from, to| std::fs::hard_link(from, to), |path| std::fs::remove_file(path)).unwrap_err();
        assert!(!failed.published);
        assert_eq!(std::fs::read(&to).unwrap(), b"existing");
        assert_eq!(std::fs::read(&from).unwrap(), b"original");
        std::fs::remove_file(&to).unwrap();
        move_noreplace(&from, &to, unsupported, |from, to| std::fs::hard_link(from, to), |path| std::fs::remove_file(path)).unwrap();
        assert!(!from.exists());
        assert_eq!(std::fs::read(&to).unwrap(), b"original");
    }

    /// A published destination cannot be called unchanged when source cleanup or fsync fails.
    #[test]
    fn fallback_unlink_and_directory_flush_failures_report_publication() {
        let dir = Dir::new().unwrap();
        let (from, to) = (dir.0.join("legacy"), dir.0.join("backup"));
        std::fs::write(&from, b"original").unwrap();
        let error = move_noreplace(
            &from,
            &to,
            |_, _| Err(io::ErrorKind::Unsupported.into()),
            |from, to| std::fs::hard_link(from, to),
            |_| Err(io::ErrorKind::PermissionDenied.into()),
        )
        .unwrap_err();
        assert!(error.published);
        assert_eq!(std::fs::read(&from).unwrap(), b"original");
        assert_eq!(std::fs::read(&to).unwrap(), b"original");
        let error = finish_publication(Err(error), || Ok(())).unwrap_err();
        assert!(error.published);
        assert_eq!(error.error.kind(), io::ErrorKind::PermissionDenied);
        let error = finish_publication(Ok(()), || Err(io::ErrorKind::Other.into())).unwrap_err();
        assert!(error.published);
        assert_eq!(std::fs::read(&to).unwrap(), b"original");
    }

    /// Ordinary rename failures must not trigger a second filesystem mutation route.
    #[test]
    fn ordinary_rename_errors_do_not_attempt_fallback() {
        for kind in [io::ErrorKind::AlreadyExists, io::ErrorKind::PermissionDenied, io::ErrorKind::NotFound] {
            let result = move_noreplace(
                Path::new("from"),
                Path::new("to"),
                |_, _| Err(kind.into()),
                |_, _| panic!("ordinary failure must not link"),
                |_| panic!("ordinary failure must not unlink"),
            );
            let error = result.unwrap_err();
            assert!(!error.published);
            assert_eq!(error.error.kind(), kind);
        }
    }
}
