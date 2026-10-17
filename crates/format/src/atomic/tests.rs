use super::*;
use std::cell::Cell;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!("pc-atomic-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Self(d)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Only the target remains: no temporary file was left behind.
fn assert_no_temp(dir: &Path) {
    let leftovers: Vec<_> =
        std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".tmp")).collect();
    assert!(leftovers.is_empty(), "temporary files left behind: {leftovers:?}");
}

const ORIGINAL: &[u8] = b"the user's precious original document";

/// Real file system with failures injected at chosen steps.
#[derive(Default)]
struct Faulty {
    /// Write this many bytes, then fail like a full disk.
    fail_write_after: Option<usize>,
    /// Fail every rename with this kind.
    fail_rename: Option<io::ErrorKind>,
    /// Fail the first N renames with PermissionDenied (a transient lock), then succeed.
    locked_renames: Cell<u32>,
    renames: Cell<u32>,
}

impl AtomicFs for Faulty {
    fn write_new(&self, tmp: &Path, bytes: &[u8]) -> io::Result<()> {
        if let Some(n) = self.fail_write_after {
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(tmp)?;
            f.write_all(&bytes[..n.min(bytes.len())])?;
            return Err(io::Error::new(io::ErrorKind::StorageFull, "injected: no space left on device"));
        }
        RealFs.write_new(tmp, bytes)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.renames.set(self.renames.get() + 1);
        if let Some(kind) = self.fail_rename {
            return Err(io::Error::new(kind, "injected rename failure"));
        }
        if self.locked_renames.get() > 0 {
            self.locked_renames.set(self.locked_renames.get() - 1);
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "injected: sharing violation"));
        }
        RealFs.rename(from, to)
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        RealFs.remove(path)
    }
    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        RealFs.sync_dir(dir)
    }
    fn copy_permissions(&self, from: &Path, to: &Path) -> io::Result<()> {
        RealFs.copy_permissions(from, to)
    }
}

const NO_RETRY: RenameRetry = RenameRetry { attempts: 0, first_delay_ms: 0 };

#[test]
fn replaces_and_creates() {
    let d = TempDir::new("ok");
    let p = d.0.join("doc.psd");
    atomic_write(&p, b"first").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"first");
    atomic_write(&p, b"second, longer").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"second, longer");
    assert_no_temp(&d.0);
}

/// #3389: a large write reports how far it has got, chunk by chunk, up to 1.0, and writes the
/// same bytes as a plain atomic write.
#[test]
fn reports_write_progress() {
    let d = TempDir::new("progress");
    let p = d.0.join("big.psd");
    let bytes: Vec<u8> = (0..WRITE_CHUNK * 2 + 123).map(|i| (i % 251) as u8).collect();
    let seen = std::sync::Mutex::new(Vec::new());
    atomic_write_progress(&p, &bytes, &|f| seen.lock().unwrap().push(f)).unwrap();
    let seen = seen.into_inner().unwrap();
    assert_eq!(seen.len(), 3, "one report per chunk: {seen:?}");
    assert!(seen.windows(2).all(|w| w[0] < w[1]), "monotonic: {seen:?}");
    assert!(seen[0] > 0.0 && seen[0] < 0.6, "{seen:?}");
    assert_eq!(seen.last().copied(), Some(1.0));
    assert!(std::fs::read(&p).unwrap() == bytes, "the file holds the bytes");
    assert_no_temp(&d.0);
}

/// Writes "head", a 4-byte placeholder, `len` bytes, then patches the placeholder by seeking back.
fn patched_writer(len: usize) -> impl FnMut(&mut dyn WriteSeek) -> io::Result<()> {
    move |w| {
        w.write_all(b"head")?;
        let at = w.stream_position()?;
        w.write_all(&[0; 4])?;
        w.write_all(&vec![9u8; len])?;
        w.seek(io::SeekFrom::Start(at))?;
        w.write_all(&(len as u32).to_be_bytes())?;
        w.seek(io::SeekFrom::End(0))?;
        Ok(())
    }
}

fn patched_bytes(len: usize) -> Vec<u8> {
    let mut v = b"head".to_vec();
    v.extend_from_slice(&(len as u32).to_be_bytes());
    v.extend(std::iter::repeat_n(9u8, len));
    v
}

/// #6607: a streamed save replaces the file with what its writer wrote, seeks included, and leaves
/// no temporary file.
#[test]
fn streamed_write_replaces_and_creates() {
    let d = TempDir::new("stream");
    let p = d.0.join("doc.psd");
    atomic_write_stream(&p, &mut patched_writer(3)).unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), patched_bytes(3));
    atomic_write_stream(&p, &mut patched_writer(100_000)).unwrap();
    assert!(std::fs::read(&p).unwrap() == patched_bytes(100_000));
    assert_no_temp(&d.0);
}

/// #6607: a writer that fails halfway (a serializer error, a full disk) leaves the original as it
/// was, with the same message as a buffered save.
#[test]
fn streamed_write_failure_halfway_keeps_original() {
    let d = TempDir::new("stream-fail");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let e = atomic_write_stream(&p, &mut |w| {
        w.write_all(&[1; 70_000])?;
        Err(io::Error::new(io::ErrorKind::StorageFull, "injected: no space left on device"))
    })
    .unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::StorageFull);
    assert!(e.to_string().contains("the disk is full") && e.to_string().contains("not changed"), "{e}");
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    assert_no_temp(&d.0);
}

/// A streamed save renames like a buffered one: retried while locked, the original kept when the
/// lock outlasts the retries.
#[test]
fn streamed_write_retries_a_locked_rename() {
    let d = TempDir::new("stream-locked");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { locked_renames: Cell::new(2), ..Default::default() };
    atomic_write_stream_with(&fs, RenameRetry { attempts: 3, first_delay_ms: 1 }, &p, &mut patched_writer(10)).unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), patched_bytes(10));
    assert_eq!(fs.renames.get(), 3);
    let fs = Faulty { locked_renames: Cell::new(10), ..Default::default() };
    let e = atomic_write_stream_with(&fs, RenameRetry { attempts: 1, first_delay_ms: 1 }, &p, &mut patched_writer(20)).unwrap_err();
    assert!(e.to_string().contains("locked"), "{e}");
    assert_eq!(std::fs::read(&p).unwrap(), patched_bytes(10));
    assert_no_temp(&d.0);
}

/// A streamed save reports the furthest position over the expected length, chunk by chunk, and
/// 1.0 once the writer is done.
#[test]
fn streamed_write_reports_progress() {
    let d = TempDir::new("stream-progress");
    let p = d.0.join("big.psd");
    let len = WRITE_CHUNK * 2 + 123;
    let seen = std::sync::Mutex::new(Vec::new());
    let mut write = |w: &mut dyn WriteSeek| {
        for chunk in patched_bytes(len).chunks(WRITE_CHUNK / 4) {
            w.write_all(chunk)?;
        }
        Ok(())
    };
    atomic_write_stream_progress(&p, len as u64 + 8, &|f| seen.lock().unwrap().push(f), &mut write).unwrap();
    let seen = seen.into_inner().unwrap();
    assert_eq!(seen.len(), 3, "two chunks, then done: {seen:?}");
    assert!(seen.windows(2).all(|w| w[0] < w[1]), "monotonic: {seen:?}");
    assert_eq!(seen.last().copied(), Some(1.0));
    assert!(std::fs::read(&p).unwrap() == patched_bytes(len));
    assert_no_temp(&d.0);
}

#[test]
fn creates_missing_parent_folders() {
    let d = TempDir::new("parent");
    let p = d.0.join("a/b/out.png");
    atomic_write(&p, b"x").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"x");
}

#[test]
fn write_failure_halfway_keeps_original() {
    let d = TempDir::new("write");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { fail_write_after: Some(5), ..Default::default() };
    let e = atomic_write_with(&fs, NO_RETRY, &p, &[7u8; 4096]).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::StorageFull);
    assert!(e.to_string().contains("not changed"), "{e}");
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    assert_eq!(fs.renames.get(), 0, "never renames after a failed write");
    assert_no_temp(&d.0);
}

#[test]
fn rename_failure_keeps_original() {
    let d = TempDir::new("rename");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { fail_rename: Some(io::ErrorKind::Other), ..Default::default() };
    assert!(atomic_write_with(&fs, NO_RETRY, &p, b"new").is_err());
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    assert_no_temp(&d.0);
}

#[test]
fn cross_device_rename_errors_clearly_and_never_writes_in_place() {
    let d = TempDir::new("exdev");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { fail_rename: Some(io::ErrorKind::CrossesDevices), ..Default::default() };
    let e = atomic_write_with(&fs, NO_RETRY, &p, b"new").unwrap_err();
    assert!(e.to_string().contains("different volume"), "{e}");
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    assert_no_temp(&d.0);
}

#[test]
fn locked_target_rename_is_retried() {
    let d = TempDir::new("locked");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { locked_renames: Cell::new(2), ..Default::default() };
    atomic_write_with(&fs, RenameRetry { attempts: 3, first_delay_ms: 1 }, &p, b"new").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"new");
    assert_eq!(fs.renames.get(), 3);
    assert_no_temp(&d.0);
}

#[test]
fn lock_that_outlasts_the_retries_keeps_original() {
    let d = TempDir::new("locked2");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { locked_renames: Cell::new(10), ..Default::default() };
    let e = atomic_write_with(&fs, RenameRetry { attempts: 2, first_delay_ms: 1 }, &p, b"new").unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    assert!(e.to_string().contains("locked"), "{e}");
    assert_eq!(fs.renames.get(), 3);
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    assert_no_temp(&d.0);
}

#[cfg(unix)]
#[test]
fn read_only_folder_errors_clearly_and_never_writes_in_place() {
    use std::os::unix::fs::PermissionsExt;
    let d = TempDir::new("ro");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    // The file itself stays writable: an in-place fallback would succeed and must not happen.
    std::fs::set_permissions(&d.0, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::File::create(d.0.join("probe")).is_ok() {
        return; // running as root: permissions aren't enforced
    }
    let e = atomic_write(&p, b"new").unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    assert!(e.to_string().contains("not writable"), "{e}");
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    std::fs::set_permissions(&d.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_no_temp(&d.0);
}

#[cfg(unix)]
#[test]
fn keeps_the_original_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let d = TempDir::new("mode");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    atomic_write(&p, b"new").unwrap();
    assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
}

#[cfg(unix)]
#[test]
fn writes_through_a_symlink() {
    let d = TempDir::new("link");
    let real = d.0.join("real.psd");
    let link = d.0.join("link.psd");
    std::fs::write(&real, ORIGINAL).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    atomic_write(&link, b"new").unwrap();
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read(&real).unwrap(), b"new");
}

#[test]
fn directory_target_or_bare_root_is_an_error_not_a_panic() {
    let d = TempDir::new("dir");
    assert!(atomic_write(&d.0, b"x").is_err());
    assert!(atomic_write(Path::new("/"), b"x").is_err());
    assert!(atomic_write(Path::new(""), b"x").is_err());
}

/// A share that refuses the full flush (`F_FULLFSYNC`, #1336) still saves through a plain fsync on
/// Apple platforms; every other sync error still fails the save.
#[test]
fn a_refused_full_flush_falls_back_to_a_plain_fsync_on_apple_only() {
    let refused = |errno| Err(io::Error::from_raw_os_error(errno));
    for (errno, name) in [(45, "ENOTSUP, SMB"), (25, "ENOTTY")] {
        let fsynced = Cell::new(false);
        let r = or_plain_fsync(refused(errno), || {
            fsynced.set(true);
            Ok(())
        });
        let apple = cfg!(target_vendor = "apple");
        assert_eq!(r.is_ok(), apple, "{name}");
        assert_eq!(fsynced.get(), apple, "{name}: the plain fsync runs only on Apple platforms");
    }
    // The plain fsync's own failure is reported.
    if cfg!(target_vendor = "apple") {
        let r = or_plain_fsync(refused(45), || Err(io::Error::from_raw_os_error(5)));
        assert_eq!(r.map_err(|e| e.raw_os_error()), Err(Some(5)));
    }
    // Real errors (EIO, ENOSPC, EACCES) never fall back, and success needs no fallback.
    for errno in [5, 28, 13] {
        let r = or_plain_fsync(refused(errno), || panic!("no fallback for errno {errno}"));
        assert_eq!(r.map_err(|e| e.raw_os_error()), Err(Some(errno)));
    }
    assert!(or_plain_fsync(Ok(()), || panic!("no fallback after a full flush")).is_ok());
}

#[test]
fn sync_file_flushes_a_local_file() {
    let dir = TempDir::new("sync");
    let path = dir.0.join("f.bin");
    // Opened for writing, as the saves do: Windows can't flush a read-only handle.
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
    sync_file(&file).unwrap();
}
