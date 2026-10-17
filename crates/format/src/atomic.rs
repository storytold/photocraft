//! Crash-safe file replacement: the one way PhotoCraft writes a document to disk.
//!
//! [`atomic_write`] never truncates the destination. It writes a temporary file in the **same
//! directory**, flushes it to stable storage (`sync_all`), renames it over the destination and,
//! on Unix, fsyncs the directory so the rename itself survives a power loss. If anything fails,
//! the destination keeps its previous bytes and the temporary file is removed.
//!
//! - **Windows:** antivirus scanners and the search indexer briefly hold files open, which makes
//!   the replacing rename fail with `PermissionDenied`. The rename is retried with exponential
//!   backoff before giving up. (`std::fs::rename` uses `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`,
//!   i.e. replace semantics.)
//! - **Cross-device (`EXDEV`):** cannot happen while the temporary file sits beside the target;
//!   if it does anyway, the save fails with a clear error. It never falls back to writing the
//!   destination in place.
//! - **Unwritable folder:** the save fails with a clear error instead of writing in place.
//! - **Network shares (macOS):** `sync_all` asks for a full flush to the disk's platters
//!   (`F_FULLFSYNC`), which SMB shares and some virtual file systems refuse while a plain `fsync`
//!   works. [`sync_file`] falls back to the plain one there (#1336).
//! - **Orphans:** after a successful save, [`sweep_stale_temps`] removes temp files an earlier
//!   interrupted save of the same file left beside it, once their owner process is gone (#6604).
//!
//! On the web there is no file system: writes go through the platform services (downloads), so
//! this module is never reached there; on `wasm32` every call returns the `std` "unsupported"
//! error.
//!
//! [`atomic_write_with`] takes an [`AtomicFs`] so tests can inject failures at each step.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The file-system steps [`atomic_write_with`] performs. [`RealFs`] is the real thing; tests
/// wrap it to inject failures.
pub trait AtomicFs {
    /// Create `tmp` (it must not exist), write all of `bytes` and flush it ([`sync_file`]).
    fn write_new(&self, tmp: &Path, bytes: &[u8]) -> io::Result<()>;
    /// [`write_new`](Self::write_new), reporting the fraction of `bytes` written so far to
    /// `progress` (a save's progress bar, #3389). By default it reports nothing.
    fn write_new_progress(&self, tmp: &Path, bytes: &[u8], progress: &dyn Fn(f32)) -> io::Result<()> {
        let _ = progress;
        self.write_new(tmp, bytes)
    }
    /// Rename `from` over `to`, replacing it.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Remove a (temporary) file.
    fn remove(&self, path: &Path) -> io::Result<()>;
    /// Flush a directory entry change (Unix); a no-op elsewhere.
    fn sync_dir(&self, dir: &Path) -> io::Result<()>;
    /// Copy the destination's permissions onto the temporary file, so replacing a file keeps its
    /// mode. Best effort.
    fn copy_permissions(&self, from: &Path, to: &Path) -> io::Result<()>;
}

/// The real file system.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealFs;

/// How much [`RealFs::write_new_progress`] writes between progress reports.
const WRITE_CHUNK: usize = 8 << 20;

impl AtomicFs for RealFs {
    fn write_new(&self, tmp: &Path, bytes: &[u8]) -> io::Result<()> {
        self.write_new_progress(tmp, bytes, &|_| {})
    }
    fn write_new_progress(&self, tmp: &Path, bytes: &[u8], progress: &dyn Fn(f32)) -> io::Result<()> {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(tmp)?;
        let mut written = 0usize;
        for chunk in bytes.chunks(WRITE_CHUNK) {
            f.write_all(chunk)?;
            written = written.saturating_add(chunk.len());
            progress(written as f32 / bytes.len().max(1) as f32);
        }
        sync_file(&f)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }
    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        #[cfg(unix)]
        {
            sync_file(&std::fs::File::open(dir)?)
        }
        #[cfg(not(unix))]
        {
            let _ = dir;
            Ok(())
        }
    }
    fn copy_permissions(&self, from: &Path, to: &Path) -> io::Result<()> {
        let perms = std::fs::metadata(from)?.permissions();
        std::fs::set_permissions(to, perms)
    }
}

/// Flush `file` to stable storage (`sync_all`). On Apple platforms that is a full flush
/// (`F_FULLFSYNC`); where the file system can't do one (SMB shares refuse it with `ENOTSUP`,
/// #1336; Tart's shared folders with `ENOTTY`) this falls back to a plain `fsync`, as Apple's own
/// `removefile` does. Any other error is returned as it is.
pub fn sync_file(file: &std::fs::File) -> io::Result<()> {
    #[cfg(target_vendor = "apple")]
    {
        or_plain_fsync(file.sync_all(), || rustix::fs::fsync(file).map_err(io::Error::from))
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        file.sync_all()
    }
}

/// `full`, or `fsync()` when `full` failed only because the file system can't do a full flush.
pub fn or_plain_fsync(full: io::Result<()>, fsync: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    match full {
        Err(e) if full_flush_unsupported(&e) => fsync(),
        other => other,
    }
}

/// Whether `e` is a file system refusing a full flush (`F_FULLFSYNC`): `ENOTSUP` (45, SMB) or
/// `ENOTTY` (25) on Apple platforms. Never elsewhere: there `sync_all` is a plain `fsync`.
fn full_flush_unsupported(e: &io::Error) -> bool {
    const ENOTTY: i32 = 25;
    const ENOTSUP: i32 = 45;
    cfg!(target_vendor = "apple") && matches!(e.raw_os_error(), Some(ENOTSUP | ENOTTY))
}

/// How hard to retry a rename that fails with `PermissionDenied`.
#[derive(Debug, Clone, Copy)]
pub struct RenameRetry {
    /// Retries after the first attempt (0 = no retry).
    pub attempts: u32,
    /// Delay before the first retry; doubles each time.
    pub first_delay_ms: u64,
}

impl RenameRetry {
    /// The platform default: retry on Windows (about 1.3 s in total), never elsewhere, where
    /// `PermissionDenied` is a real permission problem that waiting won't fix.
    pub fn platform() -> Self {
        if cfg!(windows) { Self { attempts: 7, first_delay_ms: 10 } } else { Self { attempts: 0, first_delay_ms: 0 } }
    }
}

/// Atomically replace (or create) `path` with `bytes`. See the module docs.
///
/// Errors keep their [`io::ErrorKind`] and say what happened and that the original file is
/// unchanged.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_write_with(&RealFs, RenameRetry::platform(), path, bytes)
}

/// [`atomic_write`] reporting the fraction of `bytes` written to `progress` as it goes (a save's
/// progress bar, #3389); the flush and rename follow the last report.
pub fn atomic_write_progress(path: &Path, bytes: &[u8], progress: &dyn Fn(f32)) -> io::Result<()> {
    atomic_write_with_progress(&RealFs, RenameRetry::platform(), path, bytes, progress)
}

/// [`atomic_write`] through an explicit file system and retry policy (the failure-injection seam).
pub fn atomic_write_with(fs: &dyn AtomicFs, retry: RenameRetry, path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_write_with_progress(fs, retry, path, bytes, &|_| {})
}

/// [`atomic_write_with`] reporting write progress (see [`atomic_write_progress`]).
pub fn atomic_write_with_progress(fs: &dyn AtomicFs, retry: RenameRetry, path: &Path, bytes: &[u8], progress: &dyn Fn(f32)) -> io::Result<()> {
    let target = resolve_symlink(path);
    let name = target.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{}: not a file path", path.display())))?;
    let dir = match target.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    std::fs::create_dir_all(&dir).map_err(|e| context(e, format!("cannot create the folder {}", dir.display())))?;

    // Create a fresh temporary file next to the target (same directory, so the rename never
    // crosses a device). The name is unique per process and call; retry a few on collisions.
    let mut tmp = None;
    for _ in 0..8 {
        let candidate = dir.join(temp_name(&name.to_string_lossy()));
        match fs.write_new_progress(&candidate, bytes, progress) {
            Ok(()) => {
                tmp = Some(candidate);
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                // A partial temp file may exist; the destination was never touched.
                let _ = fs.remove(&candidate);
                return Err(write_error(e, path, &dir));
            }
        }
    }
    let tmp = tmp.ok_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, format!("{}: could not create a temporary file", dir.display())))?;

    // Keep the existing file's mode (e.g. 0600) instead of the process default.
    if target.exists() {
        let _ = fs.copy_permissions(&target, &tmp);
    }

    if let Err(e) = retry_rename(retry, || fs.rename(&tmp, &target)) {
        let _ = fs.remove(&tmp);
        return Err(rename_error(e, path));
    }
    // The new bytes are in place; a failed directory fsync can't be undone and must not report
    // the save as failed, so it is best effort.
    let _ = fs.sync_dir(&dir);
    // The save succeeded: clear any temp files an earlier interrupted save of this file left (#6604).
    sweep_stale_temps(&target);
    Ok(())
}

/// Run `rename`, retrying with backoff while it fails with `PermissionDenied` (see
/// [`RenameRetry`]). For callers that rename through their own handles (capability directories).
pub fn retry_rename(retry: RenameRetry, mut rename: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    let mut delay = retry.first_delay_ms;
    let mut left = retry.attempts;
    loop {
        match rename() {
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied && left > 0 => {
                left -= 1;
                sleep_ms(delay);
                delay = delay.saturating_mul(2);
            }
            r => return r,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn sleep_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}
#[cfg(target_arch = "wasm32")]
fn sleep_ms(_ms: u64) {}

/// Write through a symlink to the file it points at, instead of replacing the link.
fn resolve_symlink(path: &Path) -> PathBuf {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
        _ => path.to_path_buf(),
    }
}

/// A unique hidden temporary file name to write beside `file` before renaming over it.
pub fn temp_name(file: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    // Keep the name short: a long file name plus the suffix must stay under NAME_MAX.
    let stem: String = file.chars().take(64).collect();
    format!(".{stem}.{}-{n}.pcsave.tmp", std::process::id())
}

/// How long a foreign temporary file must have been untouched before a save may remove it where
/// no PID probe exists (Windows). Far longer than any save takes, so another instance's
/// in-progress write is never mistaken for an orphan (#6604).
#[cfg(not(unix))]
const STALE_TEMP_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Best-effort removal of the temporary files an interrupted save left beside `path` (#6604).
///
/// Removes only regular files named exactly like this file's atomic-write temps (see
/// [`temp_name`]) whose owner process is gone: on Unix the PID in the name is probed with the
/// null signal; elsewhere (no PID probe without a new dependency) only files untouched for
/// [`STALE_TEMP_AGE`] go. This process's own temps are never touched (a concurrent save may be
/// mid-write), nor are symlinks, directories or names that do not match. Errors are ignored:
/// cleanup must never fail a save.
pub fn sweep_stale_temps(path: &Path) {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return };
    let Some(dir) = path.parent() else { return };
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    // `temp_name` truncates the file name to 64 chars and prefixes a dot.
    let stem: String = name.chars().take(64).collect();
    let prefix = format!(".{stem}.");
    for e in rd.flatten() {
        let entry_name = e.file_name();
        let Some(rest) = entry_name.to_str().and_then(|n| n.strip_prefix(&prefix)).and_then(|n| n.strip_suffix(".pcsave.tmp")) else { continue };
        // `{pid}-{n}`: anything else (a longer stem sharing this prefix, a hand-made name) stays.
        let Some((pid, seq)) = rest.split_once('-') else { continue };
        let Ok(pid) = pid.parse::<u32>() else { continue };
        if seq.is_empty() || !seq.bytes().all(|b| b.is_ascii_digit()) || !e.file_type().is_ok_and(|ft| ft.is_file()) {
            continue;
        }
        let tmp = e.path();
        if temp_is_stale(pid, &tmp) {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

/// Whether a temp file owned by `pid` may be removed while saving beside it.
fn temp_is_stale(pid: u32, path: &Path) -> bool {
    if pid == std::process::id() {
        return false;
    }
    #[cfg(unix)]
    {
        let _ = path;
        !pid_is_running(pid)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        std::fs::symlink_metadata(path).ok().and_then(|m| m.modified().ok()).and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age >= STALE_TEMP_AGE)
    }
}

/// Whether process `pid` exists right now. `kill(pid, 0)` only checks: success or `EPERM` means
/// the process exists (it may belong to another user), `ESRCH` means it is gone. A PID that is
/// not a valid `pid_t` is treated as running: keeping a file we cannot judge is the safe call.
#[cfg(unix)]
fn pid_is_running(pid: u32) -> bool {
    let Some(p) = i32::try_from(pid).ok().and_then(rustix::process::Pid::from_raw) else { return true };
    match rustix::process::test_kill_process(p) {
        Ok(()) => true,
        Err(e) => e == rustix::io::Errno::PERM,
    }
}

fn context(e: io::Error, msg: String) -> io::Error {
    io::Error::new(e.kind(), format!("{msg}: {e}"))
}

const UNCHANGED: &str = "the original file was not changed";

fn write_error(e: io::Error, path: &Path, dir: &Path) -> io::Error {
    let msg = match e.kind() {
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
            format!("cannot save {}: the folder {} is not writable ({e}); {UNCHANGED}", path.display(), dir.display())
        }
        io::ErrorKind::StorageFull => format!("cannot save {}: the disk is full ({e}); {UNCHANGED}", path.display()),
        _ => format!("cannot save {}: {e}; {UNCHANGED}", path.display()),
    };
    io::Error::new(e.kind(), msg)
}

fn rename_error(e: io::Error, path: &Path) -> io::Error {
    let msg = if is_cross_device(&e) {
        format!("cannot save {}: the temporary file is on a different volume than the destination ({e}); {UNCHANGED}", path.display())
    } else if e.kind() == io::ErrorKind::PermissionDenied {
        format!("cannot save {}: the file is locked or read-only ({e}); {UNCHANGED}", path.display())
    } else {
        format!("cannot save {}: {e}; {UNCHANGED}", path.display())
    };
    io::Error::new(e.kind(), msg)
}

fn is_cross_device(e: &io::Error) -> bool {
    // EXDEV is 18 on Linux, macOS and the BSDs; ERROR_NOT_SAME_DEVICE is 17 on Windows.
    e.kind() == io::ErrorKind::CrossesDevices || (cfg!(unix) && e.raw_os_error() == Some(18)) || (cfg!(windows) && e.raw_os_error() == Some(17))
}

#[cfg(test)]
mod tests;
