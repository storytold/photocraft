//! The depth model's folder and download, through `craft-update`'s resumable, checksum-verified
//! downloader. Only the two public weight files are requested, with a generic User-Agent, and
//! only after the user asked (`depth.downloadModel`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use photocraft_depth::WEIGHTS;
use photocraft_engine::depth_cmds::{FetchEvent, FetchFn};

/// Where the weights live for this process (see [`photocraft_automation::app_dirs::resolve_models`]).
pub fn models_dir() -> Option<PathBuf> {
    photocraft_automation::app_dirs::resolve_models(crate::app_dirs::current(), |k| std::env::var_os(k))
}

pub fn service() -> FetchFn {
    Box::new(|dir, stop| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("depth model download".into())
            .spawn(move || {
                let result = download(&dir, &stop, &tx);
                if let Err(e) = &result {
                    log::warn!("Depth model download: {e}");
                }
                let _ = tx.send(FetchEvent::Done(result));
            })
            .map_err(|e| format!("Could not start the download: {e}"))?;
        Ok(rx)
    })
}

fn download(dir: &std::path::Path, flag: &AtomicBool, tx: &Sender<FetchEvent>) -> Result<(), String> {
    let http = craft_update::Http::new("PhotoCraft/Depth")?;
    let stop = craft_update::Stop::default();
    let total: u64 = WEIGHTS.iter().map(|w| w.size).sum();
    let mut finished = 0u64;
    for w in &WEIGHTS {
        let mut progress = |t: craft_update::http::Transfer| {
            // The engine's flag asks to stop; the downloader checks `stop` between reads.
            if flag.load(Ordering::SeqCst) {
                stop.cancel();
            }
            let (craft_update::http::Transfer::Bytes { done, .. } | craft_update::http::Transfer::Reconnecting { done, .. }) = t;
            let _ = tx.send(FetchEvent::Progress { done: finished.saturating_add(done.min(w.size)), total });
        };
        if flag.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        http.download(w.url, &dir.join(w.name), w.size, w.sha256, &stop, &mut progress)?;
        finished = finished.saturating_add(w.size);
    }
    Ok(())
}
