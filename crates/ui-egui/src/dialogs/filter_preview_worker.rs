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

// #11560 R2: a real producer's unpolled native receipt must retire on Cancel before reuse.
// This tests cancellation/receipt ownership, not algorithm preemption or OS-thread joining.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod issue_11560_native_lifecycle {
    use super::*;
    use serde_json::json;

    #[test]
    fn cancel_unpolled_native_mini_then_reopen_other_command_without_publication() {
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.background_jobs = false;
        app.run("file.new", json!({"width":64,"height":48,"depth":8})).unwrap();
        app.session
            .edit("Native lifecycle edge fixture", |doc, active| {
                let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                for x in 0..64 {
                    let v = 0.25 + x as f32 / 256.0 + if x >= 32 { 0.15 } else { 0.0 } + if x % 3 == 0 { 0.04 } else { 0.0 };
                    surface.fill_rect(photocraft_geom::Rect::new(x, 0, x + 1, 48), &[v, v, v, 1.0]);
                }
                Ok(())
            })
            .unwrap();
        let original = app.session.active().unwrap().clone();
        let pixels = photocraft_compose::flatten(&original.doc).into_px();
        let history = |st: &photocraft_engine::DocState| {
            let rows: Vec<_> = st
                .history
                .row_ids()
                .enumerate()
                .map(|(i, id)| {
                    let doc = st.history.row_doc(i, &st.doc).unwrap();
                    json!({"id":id,"pixels":photocraft_compose::flatten(&doc).into_px()})
                })
                .collect();
            json!({"rows":rows,"entries":st.history.entries(),"redo":st.history.redo_labels().collect::<Vec<_>>(),
                "past":st.history.past_len(),"undo_label":st.history.undo_label(),"redo_label":st.history.redo_label(),
                "can_undo":st.history.can_undo(),"can_redo":st.history.can_redo(),
                "brush_source":st.history.brush_source(),"visibility":st.history.current_visibility()})
        };
        let original_history = history(&original);
        let ctx = egui::Context::default();
        let first = crate::filter_dialog::open(&mut app, "filter.sharpen.unsharpMask").unwrap();
        let fields = &mut app.ui.dialog_mut(first).unwrap().fields;
        fields.insert("__preview".into(), json!(false));
        fields.insert("amount".into(), json!(60));
        fields.insert("radius".into(), json!(2.0));
        fields.insert("threshold".into(), json!(0));
        app.background_jobs = true;

        // The actual canvas producer spawns the actual filter closure. No Running, channel,
        // cache or Computed is injected. A tiny computation may finish before Cancel; its
        // receipt is still owned until poll, so no timing race is needed to select this case.
        discard_closed(&mut app);
        assert!(crate::canvas::ensure_filter_preview(&mut app, 0, &ctx).is_none());
        let pending = app.filter_preview_worker.running.as_ref().expect("real producer retained a native job before its first poll");
        let old_key = pending.key.clone();
        let old_job = pending.job.clone();
        assert_eq!(old_key.dialog, first);
        assert_eq!(old_key.command, "filter.sharpen.unsharpMask");
        assert!(!old_job.cancelled());
        assert!(app.filter_preview.is_none(), "starting a native job cannot publish it inline");
        crate::dialogs::cancel(&mut app, first).unwrap();
        discard_closed(&mut app);
        let cancellation_requested = old_job.cancelled();
        // Exercise the logic cleanup route alone: a hidden/closed canvas need not draw for
        // the native receipt to be retired. !busy means channel ownership ended, not join.
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.filter_preview_worker.busy() && Instant::now() < deadline {
            discard_closed(&mut app);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(cancellation_requested, "Cancel cleanup must signal the real production JobCtx");
        assert!(!app.filter_preview_worker.busy(), "closed native receipt did not retire");
        assert!(app.filter_preview_worker.poll().is_none(), "retired result cannot be delivered again");
        assert!(app.filter_preview.is_none(), "cleanup cannot publish the closed dialog's completion");
        assert!(app.ui.dialogs.iter().all(|d| d.id != first));
        let st = app.session.active().unwrap();
        assert!(Arc::ptr_eq(&st.doc, &original.doc));
        assert_eq!(photocraft_compose::flatten(&st.doc).px, pixels);
        assert_eq!(st.revision, original.revision);
        assert_eq!((st.active_layer, &st.selected_layers), (original.active_layer, &original.selected_layers));
        assert_eq!(history(st), original_history);

        // Reuse the same Worker for a different mini-owning command, still unchecked.
        // An independent Session command supplies the pixels, not the preview helper.
        let command = "filter.sharpen.smartSharpen";
        let second = crate::filter_dialog::open(&mut app, command).unwrap();
        assert_ne!(first, second);
        let fields = &mut app.ui.dialog_mut(second).unwrap().fields;
        fields.insert("__preview".into(), json!(false));
        fields.insert("amount".into(), json!(180));
        fields.insert("radius".into(), json!(2.0));
        fields.insert("reduceNoise".into(), json!(0));
        let params = crate::filter_dialog::params_of(fields);
        let mut oracle = photocraft_engine::Session::new();
        oracle.color.settings = app.session.color.settings.clone();
        oracle.add_document(original.doc.as_ref().clone(), None);
        *oracle.active_mut().unwrap() = original.clone();
        oracle.execute(command, params.clone()).unwrap();
        let expected = photocraft_compose::flatten(&oracle.active().unwrap().doc).into_px();
        assert_ne!(expected, pixels, "a non-vacuous sharpening control");
        discard_closed(&mut app);
        assert!(crate::canvas::ensure_filter_preview(&mut app, 0, &ctx).is_none());
        let pending = app.filter_preview_worker.running.as_ref().expect("the reused real producer starts a native job");
        let new_key = pending.key.clone();
        let new_job = pending.job.clone();
        assert_eq!(new_key.dialog, second);
        assert_eq!(new_key.command, command);
        assert_eq!(new_key.params, params);
        assert_ne!(new_key, old_key);
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.filter_preview_worker.busy() && Instant::now() < deadline {
            discard_closed(&mut app);
            assert!(crate::canvas::ensure_filter_preview(&mut app, 0, &ctx).is_none(), "native mini demand must not substitute the unchecked canvas");
            assert!(!new_job.cancelled(), "old cancellation must not leak into the demanded reopened job");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!app.filter_preview_worker.busy(), "reopened native job did not complete");
        let shown = app.filter_preview.as_ref().expect("the reopened native result is published");
        assert_eq!(shown.key, new_key);
        assert_eq!(photocraft_compose::flatten(shown.result.as_ref().unwrap()).px, expected);
        assert!(!app.ui.status_error, "{}", app.ui.status);
        crate::dialogs::cancel(&mut app, second).unwrap();
        discard_closed(&mut app);
        assert!(app.filter_preview.is_none());
        assert!(!app.filter_preview_worker.busy());
        let st = app.session.active().unwrap();
        assert!(Arc::ptr_eq(&st.doc, &original.doc));
        assert_eq!(photocraft_compose::flatten(&st.doc).px, pixels);
        assert_eq!(st.revision, original.revision);
        assert_eq!((st.active_layer, &st.selected_layers), (original.active_layer, &original.selected_layers));
        assert_eq!(history(st), original_history);
    }
}
