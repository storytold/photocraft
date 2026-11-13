//! Installing and removing model files. This is the only network access in the crate, and it
//! happens only when a user asks for a download (Settings › AI Models, `photocraft-cli models
//! fetch`); inference never touches the network.
//!
//! Bytes stream into a `.part` file beside the model and are checked against the published size
//! and SHA-256 before an atomic rename, so the loader never sees a partial or corrupt file.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::store::{Model, models_dir};
use crate::{Cancel, MlError};

/// Download `model` into [`models_dir`], reporting `progress(done, total)` bytes and stopping with
/// [`MlError::Cancelled`] once `cancel` says so. An installed, verified copy is kept as it is.
/// Blocking: call it from a worker thread.
pub fn download(model: Model, progress: &dyn Fn(u64, u64), cancel: Cancel<'_>) -> Result<PathBuf, MlError> {
    let dir = models_dir().ok_or_else(|| MlError::Download("cannot locate the models folder; set PHOTOCRAFT_MODELS_DIR".into()))?;
    std::fs::create_dir_all(&dir).map_err(|e| MlError::Download(format!("create {}: {e}", dir.display())))?;
    let _lock = lock_model(&dir, model)?;
    let path = dir.join(model.file_name());
    if let Ok(mut file) = File::open(&path)
        && verify(&mut file, model.size_bytes(), model.sha256()).is_ok()
    {
        progress(model.size_bytes(), model.size_bytes());
        return Ok(path);
    }
    // The lock is held, so any `.part` file is left over from a download that died.
    clear_stale_part(&path)?;
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .user_agent(concat!("photocraft/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(30))
        // Per connect/read/write, so a stalled server fails (and Cancel is seen) within this.
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| MlError::Download(format!("create download client: {e}")))?;
    let response = client.get(model.url()).send().and_then(reqwest::blocking::Response::error_for_status).map_err(|e| MlError::Download(e.to_string()))?;
    install_from(response, &path, model.size_bytes(), model.sha256(), progress, cancel)?;
    Ok(path)
}

/// Delete the installed file of `model` (and any leftover partial download). Removing a model
/// that is not installed is not an error.
pub fn remove(model: Model) -> Result<(), MlError> {
    let dir = models_dir().ok_or_else(|| MlError::Download("cannot locate the models folder".into()))?;
    remove_in(&dir, model)
}

fn remove_in(dir: &Path, model: Model) -> Result<(), MlError> {
    let _lock = lock_model(dir, model)?;
    for path in [dir.join(model.file_name()), part_path(&dir.join(model.file_name()))] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(MlError::Download(format!("remove {}: {e}", path.display()))),
        }
    }
    Ok(())
}

/// Exclusive, cross-process hold on one model's files for a download or removal, so a removal
/// can't unlink another worker's partial file and two fetches can't publish each other's bytes.
/// Released when the returned file is dropped (or the process exits).
fn lock_model(dir: &Path, model: Model) -> Result<File, MlError> {
    let path = dir.join(format!("{}.lock", model.file_name()));
    let file =
        OpenOptions::new().create(true).truncate(false).write(true).open(&path).map_err(|e| MlError::Download(format!("open {}: {e}", path.display())))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => {
            Err(MlError::Download(format!("the {} model is being downloaded or removed elsewhere; try again when that finishes", model.label())))
        }
        Err(std::fs::TryLockError::Error(e)) => Err(MlError::Download(format!("lock {}: {e}", path.display()))),
    }
}

/// Delete the partial download next to `path`, if any. Missing is fine.
fn clear_stale_part(path: &Path) -> Result<(), MlError> {
    let part = part_path(path);
    match std::fs::remove_file(&part) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(MlError::Download(format!("remove {}: {e}", part.display()))),
    }
}

fn part_path(path: &Path) -> PathBuf {
    path.with_extension("onnx.part")
}

/// Check the bytes on disk, including same-size corruption.
pub(crate) fn verify(file: &mut File, size: u64, sha256: &str) -> Result<(), MlError> {
    let actual = file.metadata().map_err(|e| MlError::Download(e.to_string()))?.len();
    if actual != size {
        return Err(MlError::Download(format!("size mismatch: expected {size} bytes, got {actual}")));
    }
    file.seek(SeekFrom::Start(0)).map_err(|e| MlError::Download(e.to_string()))?;
    let mut hash = Sha256::new();
    std::io::copy(file, &mut hash).map_err(|e| MlError::Download(e.to_string()))?;
    let actual = format!("{:x}", hash.finalize());
    if actual != sha256 {
        return Err(MlError::Download(format!("SHA-256 mismatch: expected {sha256}, got {actual}")));
    }
    Ok(())
}

/// Stream `reader` into `path` through a `.part` file, verify it and publish it. Nothing is left
/// behind on failure or cancel, and an existing `path` is only replaced by a verified file.
pub(crate) fn install_from(
    mut reader: impl Read,
    path: &Path,
    size: u64,
    sha256: &str,
    progress: &dyn Fn(u64, u64),
    cancel: Cancel<'_>,
) -> Result<(), MlError> {
    let part = part_path(path);
    // Exclusive creation keeps two downloads of one model from corrupting each other.
    let mut file = OpenOptions::new().read(true).write(true).create_new(true).open(&part).map_err(|e| {
        let hint = if e.kind() == std::io::ErrorKind::AlreadyExists { " (another download is running, or remove the stale .part file)" } else { "" };
        MlError::Download(format!("create {}: {e}{hint}", part.display()))
    })?;
    let result = (|| {
        let mut buf = vec![0u8; 1 << 16];
        let mut done = 0u64;
        progress(0, size);
        loop {
            if cancel() {
                return Err(MlError::Cancelled);
            }
            let n = reader.read(&mut buf).map_err(|e| MlError::Download(e.to_string()))?;
            if n == 0 {
                break;
            }
            done = done.saturating_add(n as u64);
            // Bound disk use even if a server ignores the published size.
            if done > size {
                return Err(MlError::Download(format!("the download is larger than the published {size} bytes")));
            }
            file.write_all(buf.get(..n).unwrap_or_default()).map_err(|e| MlError::Download(e.to_string()))?;
            progress(done, size);
        }
        file.sync_all().map_err(|e| MlError::Download(e.to_string()))?;
        verify(&mut file, size, sha256)?;
        if cancel() {
            return Err(MlError::Cancelled);
        }
        drop(file);
        std::fs::rename(&part, path).map_err(|e| MlError::Download(format!("install {}: {e}", path.display())))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU64, Ordering};

    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> std::io::Result<Self> {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("photocraft-ml-install-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir(&path)?;
            Ok(Self(path))
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn never() -> bool {
        false
    }

    #[test]
    fn verifies_size_and_hash() -> Result<(), Box<dyn std::error::Error>> {
        let t = Temp::new()?;
        let path = t.0.join("m.onnx");
        std::fs::write(&path, b"abc")?;
        let mut f = File::open(&path)?;
        verify(&mut f, 3, ABC_SHA256)?;
        assert!(verify(&mut f, 4, ABC_SHA256).is_err());
        assert!(verify(&mut f, 3, &"0".repeat(64)).is_err());
        Ok(())
    }

    #[test]
    fn publishes_only_verified_downloads_and_reports_progress() -> Result<(), Box<dyn std::error::Error>> {
        let t = Temp::new()?;
        let path = t.0.join("m.onnx");
        std::fs::write(&path, b"old")?;
        for bytes in [&b"ab"[..], &b"abcd"[..], &b"bad"[..]] {
            assert!(install_from(bytes, &path, 3, ABC_SHA256, &|_, _| {}, &never).is_err());
            assert_eq!(std::fs::read(&path)?, b"old");
            assert!(!part_path(&path).exists());
        }
        let last = Cell::new((0, 0));
        install_from(&b"abc"[..], &path, 3, ABC_SHA256, &|d, t| last.set((d, t)), &never)?;
        assert_eq!(std::fs::read(&path)?, b"abc");
        assert_eq!(last.get(), (3, 3));
        assert!(!part_path(&path).exists());
        Ok(())
    }

    #[test]
    fn cancel_leaves_nothing_behind() -> Result<(), Box<dyn std::error::Error>> {
        let t = Temp::new()?;
        let path = t.0.join("m.onnx");
        assert_eq!(install_from(&b"abc"[..], &path, 3, ABC_SHA256, &|_, _| {}, &|| true), Err(MlError::Cancelled));
        assert!(!path.exists());
        assert!(!part_path(&path).exists());
        Ok(())
    }

    #[test]
    fn refuses_a_running_download_of_the_same_model() -> Result<(), Box<dyn std::error::Error>> {
        let t = Temp::new()?;
        let path = t.0.join("m.onnx");
        std::fs::write(part_path(&path), b"other")?;
        assert!(install_from(&b"abc"[..], &path, 3, ABC_SHA256, &|_, _| {}, &never).is_err());
        assert_eq!(std::fs::read(part_path(&path))?, b"other");
        Ok(())
    }

    #[test]
    fn a_held_model_lock_blocks_removal_and_a_second_holder() -> Result<(), Box<dyn std::error::Error>> {
        let t = Temp::new()?;
        let model = Model::Lama;
        std::fs::write(t.0.join(model.file_name()), b"x")?;
        let held = lock_model(&t.0, model)?;
        assert!(lock_model(&t.0, model).is_err());
        assert!(remove_in(&t.0, model).is_err());
        assert!(t.0.join(model.file_name()).exists(), "removal must wait for the download");
        // Another model is independent.
        drop(lock_model(&t.0, Model::BirefnetLite)?);
        drop(held);
        remove_in(&t.0, model)?;
        assert!(!t.0.join(model.file_name()).exists());
        Ok(())
    }

    #[test]
    fn remove_deletes_the_model_and_a_partial_download() -> Result<(), Box<dyn std::error::Error>> {
        let t = Temp::new()?;
        let model = Model::Lama;
        let path = t.0.join(model.file_name());
        std::fs::write(&path, b"x")?;
        std::fs::write(part_path(&path), b"y")?;
        remove_in(&t.0, model)?;
        assert!(!path.exists() && !part_path(&path).exists());
        // Not installed: still fine.
        remove_in(&t.0, model)?;
        Ok(())
    }

    #[test]
    fn a_stale_partial_download_does_not_block_a_retry() -> Result<(), Box<dyn std::error::Error>> {
        let t = Temp::new()?;
        let path = t.0.join("m.onnx");
        // Left by a download that was killed before it could clean up.
        std::fs::write(part_path(&path), b"stale")?;
        clear_stale_part(&path)?;
        install_from(&b"abc"[..], &path, 3, ABC_SHA256, &|_, _| {}, &never)?;
        assert_eq!(std::fs::read(&path)?, b"abc");
        // Nothing to clear is fine.
        clear_stale_part(&path)?;
        Ok(())
    }
}
