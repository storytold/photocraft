//! One native preview at a time. The canvas submits the current request again after an old
//! job finishes, so slider drags cannot queue copies of every intermediate document.

use std::sync::mpsc::{Receiver, TryRecvError};

use crate::filter_dialog::FilterPreviewKey;

pub(crate) struct Computed {
    pub result: Option<(std::sync::Arc<photocraft_doc::Document>, photocraft_compose::Buffer)>,
    pub ms: f64,
}

#[derive(Default)]
pub(crate) struct Worker {
    running: Option<(FilterPreviewKey, Receiver<Result<Computed, String>>)>,
}

impl Worker {
    pub fn busy(&self) -> bool {
        self.running.is_some()
    }

    /// Never wait for a previous computation and never spawn a second concurrent preview.
    pub fn start(&mut self, key: FilterPreviewKey, ctx: egui::Context, compute: impl FnOnce() -> Computed + Send + 'static) -> Result<bool, String> {
        if self.busy() {
            return Ok(false);
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("indexed-color-preview".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(compute)).map_err(|_| "Filter preview failed".to_string());
                let _ = tx.send(result);
                ctx.request_repaint();
            })
            .map_err(|e| format!("Could not start filter preview: {e}"))?;
        self.running = Some((key, rx));
        Ok(true)
    }

    pub fn poll(&mut self) -> Option<(FilterPreviewKey, Result<Computed, String>)> {
        let (_, rx) = self.running.as_ref()?;
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("Filter preview worker disconnected".to_string()),
        };
        let (key, _) = self.running.take()?;
        Some((key, result))
    }
}

/// Release results from closed dialogs even when their canvas is no longer being drawn.
pub(crate) fn discard_closed(app: &mut crate::PhotocraftApp) {
    let visible = app.ui.dialogs.iter().any(|d| {
        d.fields.get("__command").and_then(serde_json::Value::as_str) == Some("image.mode.indexedColor")
            && d.fields.get("__preview").and_then(serde_json::Value::as_bool) == Some(true)
    });
    if !visible {
        let _ = app.filter_preview_worker.poll();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(dialog: u64, revision: u64, colors: u32) -> FilterPreviewKey {
        FilterPreviewKey {
            doc: photocraft_doc::DocId(1),
            revision,
            dialog,
            active: None,
            command: "image.mode.indexedColor".into(),
            params: json!({"colors": colors}),
            k: 1,
        }
    }

    fn finish(worker: &mut Worker) -> (FilterPreviewKey, Result<Computed, String>) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(result) = worker.poll() {
                return result;
            }
            assert!(std::time::Instant::now() < deadline, "preview worker did not finish");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn blocked_preview_does_not_block_ui_or_queue_slider_requests() {
        let mut worker = Worker::default();
        let ctx = egui::Context::default();
        let (release, wait) = std::sync::mpsc::channel();
        assert!(
            worker
                .start(key(1, 0, 8), ctx.clone(), move || {
                    wait.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
                    Computed { result: None, ms: 0.0 }
                })
                .unwrap()
        );
        for colors in [16, 32, 64, 128, 256] {
            ctx.run_ui(Default::default(), |ui| {
                ui.label("The UI still draws");
            })
            .textures_delta
            .clear();
            assert!(worker.poll().is_none());
            assert!(!worker.start(key(1, 0, colors), ctx.clone(), || panic!("queued an intermediate request")).unwrap());
        }
        release.send(()).unwrap();
        let (old, result) = finish(&mut worker);
        assert!(result.is_ok());
        let latest = key(1, 0, 256);
        assert_ne!(old, latest); // The canvas must discard this result before uploading.
        assert!(worker.start(latest.clone(), ctx, || Computed { result: None, ms: 1.0 }).unwrap());
        assert_eq!(finish(&mut worker).0, latest);
    }

    #[test]
    fn request_identity_covers_dialog_revision_layer_and_params() {
        let original = key(1, 2, 64);
        assert_ne!(original, key(2, 2, 64));
        assert_ne!(original, key(1, 3, 64));
        assert_ne!(original, key(1, 2, 128));
        let mut layer = original.clone();
        layer.active = Some(photocraft_doc::LayerId(4));
        assert_ne!(original, layer);
    }

    #[test]
    fn escaped_panic_is_an_error_and_worker_can_be_reused() {
        let mut worker = Worker::default();
        worker.start(key(1, 0, 64), Default::default(), || panic!("synthetic preview failure")).unwrap();
        assert!(finish(&mut worker).1.is_err());
        assert!(!worker.busy());
        worker.start(key(1, 0, 128), Default::default(), || Computed { result: None, ms: 0.0 }).unwrap();
        assert!(finish(&mut worker).1.is_ok());
    }
}
