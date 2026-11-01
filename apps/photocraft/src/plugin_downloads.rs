//! Plugins › Browse Plugins… downloads through `craft-update`'s resumable HTTPS download, one
//! background thread per module, into `plugin-downloads` in the per-user settings folder (not the
//! shared temp folder, where another user could plant a link at the predictable partial-file path).
//! It reports progress and stops between reads when cancelled. The engine checks what arrives
//! against the catalog's size and SHA-256.

use std::io::Read;
use std::path::{Path, PathBuf};

use craft_update::Stop;
use craft_update::http::Transfer;
use photocraft_engine::plugin_store_cmds::{FetchFn, FetchResult, Request};

/// Downloads into `dir` (created owner-only); `None` when there is no settings folder.
pub fn service(dir: Option<PathBuf>) -> FetchFn {
    Box::new(move |request: Request| {
        let dir = dir.clone().ok_or("PhotoCraft has no settings folder to download plug-ins into")?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("plug-in download".into())
            .spawn(move || {
                let result = fetch(&dir, &request);
                if let Err(e) = &result {
                    log::warn!("Plug-in download {}: {e}", request.url);
                }
                let _ = tx.send(result);
            })
            .map_err(|e| format!("Could not start the download: {e}"))?;
        Ok(rx)
    })
}

fn fetch(dir: &Path, request: &Request) -> FetchResult {
    if request.progress.cancelled() {
        return Err("cancelled".into());
    }
    let http = craft_update::Http::new(&format!("PhotoCraft/{}", photocraft_engine::build_info::VERSION))?;
    // Named by checksum (64 hex digits, checked by the engine), so an interrupted download of the
    // same module resumes, and one module can't pick up another's partial file.
    if request.sha256.len() != 64 || !request.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("the catalog gives no valid checksum".into());
    }
    private_dir(dir)?;
    let dest = dir.join(format!("{}.wasm", request.sha256.to_ascii_lowercase()));
    let stop = Stop::default();
    let size = request.size as u64;
    http.download(&request.url, &dest, size, &request.sha256, &stop, &mut |t| {
        let (Transfer::Bytes { done, .. } | Transfer::Reconnecting { done, .. }) = t;
        request.progress.set_received(done);
        if request.progress.cancelled() {
            stop.cancel();
        }
    })?;
    let read = read_capped(&dest, size);
    let _ = std::fs::remove_file(&dest);
    read
}

/// Creates `dir`, readable only by this user.
fn private_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {} ({e})", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| format!("couldn't protect {} ({e})", dir.display()))?;
    }
    Ok(())
}

fn read_capped(path: &std::path::Path, size: u64) -> FetchResult {
    let file = std::fs::File::open(path).map_err(|e| format!("couldn't read the download ({e})"))?;
    let mut bytes = Vec::new();
    file.take(size.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| format!("couldn't read the download ({e})"))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_engine::plugin_store_cmds::{Progress, bundled, verify};
    use std::sync::Arc;

    /// Downloads every catalog listing and checks it is the module listed: catches a project that
    /// moved, retagged or replaced a file. Network: `cargo test -p photocraft plugin_downloads -- --ignored`.
    #[test]
    #[ignore = "downloads every catalog plug-in"]
    fn every_catalog_listing_downloads_and_verifies() {
        let mut failed = Vec::new();
        let dir = std::env::temp_dir().join(format!("photocraft-plugin-drift-{}", std::process::id()));
        for e in bundled() {
            let progress = Arc::new(Progress::default());
            let request = Request { url: e.url.clone(), size: e.size, sha256: e.sha256.clone(), progress: progress.clone() };
            match fetch(&dir, &request).and_then(|bytes| verify(e, &bytes).map(|_| ())) {
                Ok(()) => assert_eq!(progress.received(), e.size as u64, "{}: progress ends at the size", e.id),
                Err(why) => failed.push(format!("{}: {why}", e.id)),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(failed.is_empty(), "{} listings drifted:\n{}", failed.len(), failed.join("\n"));
    }

    #[test]
    fn a_cancelled_download_stops_without_an_answer() {
        let progress = Arc::new(Progress::default());
        progress.cancel();
        let e = bundled().first().unwrap();
        let request = Request { url: e.url.clone(), size: e.size, sha256: e.sha256.clone(), progress };
        assert!(fetch(Path::new("unused"), &request).is_err());
    }

    #[test]
    fn without_a_settings_folder_downloads_fail_cleanly() {
        let mut fetch = service(None);
        let e = bundled().first().unwrap();
        let request = Request { url: e.url.clone(), size: e.size, sha256: e.sha256.clone(), progress: Arc::default() };
        assert!(fetch(request).is_err());
    }
}
