//! One native preview at a time. The canvas submits the current request again after an old
//! job finishes, so slider drags cannot queue copies of every intermediate document. While a
//! slider moves, the running preview finishes, so the canvas keeps updating during the drag;
//! once the dialog's values have settled, a preview still running for older ones is cancelled
//! ([`Worker::supersede`]) so a slow one (Gaussian Blur at 1000 px) doesn't hold up the last.

use std::sync::mpsc::{Receiver, TryRecvError};

use photocraft_engine::jobs::JobCtx;

use crate::filter_dialog::FilterPreviewKey;

pub(crate) struct Computed {
    pub result: Option<(std::sync::Arc<photocraft_doc::Document>, photocraft_compose::Buffer)>,
    pub ms: f64,
}

/// How long the dialog's values must stay unchanged before a stale preview is cancelled. Shorter
/// than a pause in a drag is long; cancelling on every slider step meant no preview finished
/// until the slider stopped.
pub(crate) const SETTLE_MS: f64 = 150.0;

/// The preview being computed: its request, its result channel and its cancellation.
struct Running {
    key: FilterPreviewKey,
    rx: Receiver<Result<Computed, String>>,
    job: JobCtx,
}

#[derive(Default)]
pub(crate) struct Worker {
    running: Option<Running>,
    /// The dialog's latest request and when it was first seen (ms).
    latest: Option<(FilterPreviewKey, f64)>,
}

impl Worker {
    pub fn busy(&self) -> bool {
        self.running.is_some()
    }

    /// Never wait for a previous computation and never spawn a second concurrent preview.
    /// `compute` gets the preview's cancellation context ([`Worker::supersede`]).
    pub fn start(&mut self, key: FilterPreviewKey, ctx: egui::Context, compute: impl FnOnce(&JobCtx) -> Computed + Send + 'static) -> Result<bool, String> {
        if self.busy() {
            return Ok(false);
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let job = JobCtx::new();
        let work = job.clone();
        std::thread::Builder::new()
            .name("filter-preview".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compute(&work))).map_err(|_| "Filter preview failed".to_string());
                let _ = tx.send(result);
                ctx.request_repaint();
            })
            .map_err(|e| format!("Could not start filter preview: {e}"))?;
        self.running = Some(Running { key, rx, job });
        Ok(true)
    }

    /// The dialog wants `latest` at time `now_ms`. Once that request has been unchanged for
    /// [`SETTLE_MS`], a running preview for anything else is cancelled, so it stops at its next
    /// check and the latest can start (the caller discards its result). While the request keeps
    /// changing (a slider drag) the running preview is left to finish and be shown.
    pub fn supersede(&mut self, latest: &FilterPreviewKey, now_ms: f64) {
        let since = match &self.latest {
            Some((key, t)) if key == latest => *t,
            _ => {
                self.latest = Some((latest.clone(), now_ms));
                now_ms
            }
        };
        if now_ms - since < SETTLE_MS {
            return;
        }
        if let Some(r) = &self.running
            && &r.key != latest
        {
            r.job.cancel();
        }
    }

    pub fn poll(&mut self) -> Option<(FilterPreviewKey, Result<Computed, String>)> {
        let result = match self.running.as_ref()?.rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("Filter preview worker disconnected".to_string()),
        };
        let key = self.running.take()?.key;
        Some((key, result))
    }
}

/// Stop and release previews of closed dialogs even when their canvas is no longer being drawn.
pub(crate) fn discard_closed(app: &mut crate::PhotocraftApp) {
    let demanded = app.ui.dialogs.iter().any(|d| {
        d.fields.contains_key("__filter")
            && crate::filter_preview_ui::demanded(&d.fields)
            && d.fields.get("__smartFilter").is_none_or(|target| crate::smart_filter_dialog::validate_target(app, target).is_ok())
    });
    if !demanded {
        if let Some(r) = &app.filter_preview_worker.running {
            r.job.cancel();
        }
        let _ = app.filter_preview_worker.poll();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    mod issue_11560 {
        use super::*;

        fn request(app: &crate::PhotocraftApp, id: u64) -> FilterPreviewKey {
            let st = app.session.active().unwrap();
            let f = &app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields;
            FilterPreviewKey {
                doc: st.doc.id,
                revision: st.revision,
                dialog: id,
                active: st.active_layer,
                target: st.channel_view.target,
                command: f["__command"].as_str().unwrap().into(),
                params: crate::filter_dialog::params_of(f),
                k: 1,
            }
        }

        fn lifetime(command: &str, on: bool, closed: bool, invalid_smart: bool, should_cancel: bool) {
            let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width":64,"height":48})).unwrap();
            let id = crate::filter_dialog::open(&mut app, command).unwrap();
            let f = &mut app.ui.dialog_mut(id).unwrap().fields;
            f.insert("__preview".into(), json!(on));
            if invalid_smart {
                f.insert("__smartFilter".into(), json!({"guard":0}));
            }
            let request = request(&app, id);
            if closed {
                crate::dialogs::cancel(&mut app, id).unwrap();
            }
            let (send_job, receive_job) = std::sync::mpsc::channel();
            let (release, wait) = std::sync::mpsc::channel::<()>();
            app.filter_preview_worker
                .start(request, Default::default(), move |job| {
                    send_job.send(job.clone()).unwrap();
                    let _ = wait.recv_timeout(std::time::Duration::from_secs(5));
                    Computed { result: None, ms: 0.0 }
                })
                .unwrap();
            let job = receive_job.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
            discard_closed(&mut app);
            let cancelled = job.cancelled();
            // Always let the owned task finish before the policy assertion (including red).
            release.send(()).unwrap();
            assert!(finish(&mut app.filter_preview_worker).1.is_ok());
            assert_eq!(cancelled, should_cancel, "command={command}, on={on}, closed={closed}, invalid_smart={invalid_smart}");
            assert!(!app.filter_preview_worker.busy());
        }

        #[test]
        fn unchecked_generic_mini_work_is_not_cancelled_by_logic_cleanup() {
            lifetime("filter.sharpen.unsharpMask", false, false, false, false);
            lifetime("filter.sharpen.smartSharpen", false, false, false, false);
            lifetime("filter.blur.gaussianBlur", false, false, false, false);
        }

        #[test]
        fn checked_non_mini_work_retains_its_canvas_demand() {
            lifetime("image.rotation.arbitrary", true, false, false, false);
        }

        #[test]
        fn unchecked_non_mini_work_is_cancelled() {
            lifetime("image.rotation.arbitrary", false, false, false, true);
        }

        #[test]
        fn closed_dialog_work_is_cancelled_even_when_it_used_to_own_a_mini() {
            lifetime("filter.sharpen.unsharpMask", false, true, false, true);
        }

        #[test]
        fn invalid_smart_target_work_is_cancelled_even_with_a_mini() {
            lifetime("filter.sharpen.smartSharpen", false, false, true, true);
        }

        #[test]
        fn completed_unchecked_mini_result_is_retained_but_closed_result_is_discarded() {
            for closed in [false, true] {
                let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                app.run("file.new", json!({"width":64,"height":48})).unwrap();
                let id = crate::filter_dialog::open(&mut app, "filter.sharpen.unsharpMask").unwrap();
                app.ui.dialog_mut(id).unwrap().fields.insert("__preview".into(), json!(false));
                let request = request(&app, id);
                if closed {
                    crate::dialogs::cancel(&mut app, id).unwrap();
                }
                // A ready completion at the real worker channel boundary: no race with a
                // thread's final send, and no preview producer can repair a mistaken discard.
                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                tx.send(Ok(Computed { result: None, ms: 0.0 })).unwrap();
                app.filter_preview_worker.running = Some(Running { key: request, rx, job: JobCtx::new() });
                discard_closed(&mut app);
                let retained = app.filter_preview_worker.busy();
                if retained {
                    assert!(app.filter_preview_worker.poll().unwrap().1.is_ok());
                }
                assert_eq!(retained, !closed, "logic cleanup must leave off-state mini completions for the normal publisher");
            }
        }
    }

    fn key(dialog: u64, revision: u64, colors: u32) -> FilterPreviewKey {
        FilterPreviewKey {
            doc: photocraft_doc::DocId(1),
            revision,
            dialog,
            active: None,
            target: Default::default(),
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
                .start(key(1, 0, 8), ctx.clone(), move |_| {
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
            assert!(!worker.start(key(1, 0, colors), ctx.clone(), |_| panic!("queued an intermediate request")).unwrap());
        }
        release.send(()).unwrap();
        let (old, result) = finish(&mut worker);
        assert!(result.is_ok());
        let latest = key(1, 0, 256);
        assert_ne!(old, latest); // The canvas paints this as the drag's frame; the newest one is computed next.
        assert!(worker.start(latest.clone(), ctx, |_| Computed { result: None, ms: 1.0 }).unwrap());
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
        worker.start(key(1, 0, 64), Default::default(), |_| panic!("synthetic preview failure")).unwrap();
        assert!(finish(&mut worker).1.is_err());
        assert!(!worker.busy());
        worker.start(key(1, 0, 128), Default::default(), |_| Computed { result: None, ms: 0.0 }).unwrap();
        assert!(finish(&mut worker).1.is_ok());
    }

    #[test]
    fn a_stale_preview_is_cancelled_only_once_the_values_settle() {
        // Cancelling on every slider step meant no preview finished until the slider stopped.
        let mut worker = Worker::default();
        let (send_job, job) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel::<()>();
        worker
            .start(key(1, 0, 8), Default::default(), move |ctx| {
                send_job.send(ctx.clone()).unwrap();
                let _ = wait.recv_timeout(std::time::Duration::from_secs(5));
                Computed { result: None, ms: 0.0 }
            })
            .unwrap();
        let job = job.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        // A drag: a new value every frame, never settled; the running preview is left to finish.
        for (i, colors) in [16, 32, 64, 128].into_iter().enumerate() {
            worker.supersede(&key(1, 0, colors), 1000.0 + i as f64 * 16.0);
            assert!(!job.cancelled(), "cancelled mid-drag at {colors}");
        }
        // The same request for less than the settle time: still running.
        worker.supersede(&key(1, 0, 128), 1048.0 + SETTLE_MS - 1.0);
        assert!(!job.cancelled());
        // Settled: the stale preview is cancelled.
        worker.supersede(&key(1, 0, 128), 1048.0 + SETTLE_MS);
        assert!(job.cancelled());
        release.send(()).unwrap();
        let (finished, _) = finish(&mut worker);
        assert_eq!(finished, key(1, 0, 8));
        // A running preview for the settled request itself is never cancelled.
        let (send_job, job) = std::sync::mpsc::channel();
        worker
            .start(key(1, 0, 128), Default::default(), move |ctx| {
                send_job.send(ctx.clone()).unwrap();
                Computed { result: None, ms: 0.0 }
            })
            .unwrap();
        let job = job.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        worker.supersede(&key(1, 0, 128), 99_999.0);
        assert!(!job.cancelled());
        let _ = finish(&mut worker);
    }
}
