use super::*;
use craft::ui::dialog::DialogId;
use serde_json::{Map, Value, json};

/// The id of the dialog on top of the host's stack: the one a test just opened.
fn top_dialog(app: &PhotocraftApp) -> DialogId {
    app.framework.dialogs.stack().top().expect("a dialog is open")
}

/// The open parameter dialog.
fn hosted(app: &PhotocraftApp) -> &HostedParamDialog {
    app.framework.dialogs.get::<HostedParamDialog>().expect("a parameter dialog is open")
}

/// The open dialog's fields as the JSON map the filter helpers read.
fn fields(app: &PhotocraftApp) -> Map<String, Value> {
    let Value::Object(fields) = serde_json::to_value(hosted(app).fields()).expect("fields serialize") else { panic!("fields are an object") };
    fields
}

/// Opens the dialog of `command` and shows it once: a hosted form loads its values when it first shows.
fn open_filter(app: &mut PhotocraftApp, command: &str) {
    crate::filter_dialog::open(app, command).unwrap_or_else(|error| panic!("{command} has a dialog: {error:?}"));
    crate::dialogs::testing::show(app);
}

/// Sets `name` of the open dialog through the host, as a script or a click on its row does.
fn set_field(app: &mut PhotocraftApp, ctx: &egui::Context, name: &str, value: Value) {
    let id = top_dialog(app);
    app.framework.dialogs.set_json_field(ctx, id, name, value).unwrap_or_else(|error| panic!("the dialog takes {name}: {error}"));
}

/// Sets every field of `values` (an object) of the open dialog.
fn set_fields(app: &mut PhotocraftApp, ctx: &egui::Context, values: &Value) {
    for (name, value) in values.as_object().expect("an object of fields") {
        set_field(app, ctx, name, value.clone());
    }
}

/// A tile that fails to read and recovers changes the pixels without a new revision or new dialog values: the
/// preview is computed again, so it does not keep what it read during the error (`crate::scratch_reads`).
#[test]
fn the_preview_is_computed_again_after_a_scratch_read_error_transition() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    open_filter(&mut app, "filter.stylize.solarize");
    let ctx = app.ctx().clone();
    let result = |app: &PhotocraftApp| app.filter_preview.as_ref().and_then(|preview| preview.result.clone()).expect("a preview result");
    ensure_filter_preview(&mut app, 0, &ctx).expect("a preview");
    let before = result(&app);
    ensure_filter_preview(&mut app, 0, &ctx).expect("the same preview");
    assert!(std::sync::Arc::ptr_eq(&result(&app), &before), "nothing changed: the same result");
    let (revision, values) = (app.session.active().unwrap().revision, fields(&app));
    crate::scratch_reads::tests::simulate_read_error_transition();
    ensure_filter_preview(&mut app, 0, &ctx).expect("a preview");
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert_eq!(fields(&app), values, "the dialog values did not change");
    assert!(!std::sync::Arc::ptr_eq(&result(&app), &before), "the preview is computed again");
}

/// #7490: the inline (web) preview path composites and uploads each changed preview once, as
/// the background worker does. It used to flatten and upload it twice and record it twice,
/// so the frame's refresh time counted the preview twice.
#[test]
fn inline_filter_preview_is_composited_and_recorded_once() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width": 320, "height": 240})).unwrap();
    open_filter(&mut app, "filter.stylize.solarize");
    app.perf.stage = Default::default();
    let ctx = app.ctx().clone();
    let (_, _, size) = ensure_filter_preview(&mut app, 0, &ctx).expect("a preview");
    assert_eq!(size, [320, 240]);
    assert_eq!(app.perf.last_refresh, "filter-preview");
    assert_eq!(app.perf.last_refresh_px, 320 * 240);
    assert_eq!(app.perf.stage.composite_ms + app.perf.stage.upload_ms, app.perf.composite_ms + app.perf.upload_ms, "one record per preview");
    // Unchanged request: nothing is recomputed or recorded again.
    ensure_filter_preview(&mut app, 0, &ctx).expect("the same preview");
    assert_eq!(app.perf.stage.composite_ms + app.perf.stage.upload_ms, app.perf.composite_ms + app.perf.upload_ms);
}

/// #6326: a filter's live preview is never coarser than the screen. On a document big enough
/// for a reduced proxy, the view at 100 % previews at full resolution, and only a view zoomed
/// out far enough uses the proxy. Through `ensure_filter_preview`, so the factor the canvas
/// actually requests is the one under test.
#[test]
fn filter_preview_at_100_percent_is_full_resolution() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width": 2400, "height": 2200})).unwrap();
    assert_eq!(crate::proxy::preview_factor(&app.session.active().unwrap().doc, true), 2, "big enough for a proxy");
    open_filter(&mut app, "filter.stylize.solarize");
    let ctx = app.ctx().clone();
    let preview_at = |app: &mut PhotocraftApp, zoom: f32| {
        app.ui.views.get_mut(0).expect("the new document's view").zoom = zoom;
        assert_eq!(app.current_zoom(), zoom);
        let (k, _, size) = ensure_filter_preview(app, 0, &ctx).expect("a preview");
        (k, size)
    };
    assert_eq!(preview_at(&mut app, 1.0), (1, [2400, 2200]), "100 %: every pixel");
    assert_eq!(preview_at(&mut app, 0.25), (2, [1200, 1100]), "zoomed out: the document's proxy");
    // Low Resolution Previews off: full resolution at any zoom, exactly as before #6326.
    app.run("prefs.set", json!({"path": "performance.lowResolutionPreviews", "value": false})).unwrap();
    assert_eq!(preview_at(&mut app, 0.25), (1, [2400, 2200]), "the preference still decides");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn every_filter_previews_off_the_ui_thread_and_a_newer_value_cancels_the_old() {
    // Dragging Gaussian Blur's radius to 1000 px froze the window: the preview ran inline in
    // the frame, and every slider step waited for the previous blur to finish.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.background_jobs = true;
    open_filter(&mut app, "filter.blur.gaussianBlur");
    let ctx = app.ctx().clone();
    let wait_for = |app: &mut PhotocraftApp, radius: f64| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.filter_preview.as_ref().is_none_or(|p| p.key.params["radius"].as_f64() != Some(radius)) {
            assert!(std::time::Instant::now() < deadline, "no preview for radius {radius}");
            ensure_filter_preview(app, 0, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    };
    // The first frame hands the blur to the worker and returns.
    assert!(ensure_filter_preview(&mut app, 0, &ctx).is_none());
    assert!(app.filter_preview_worker.busy(), "the preview computes on the worker");
    let first = fields(&app)["radius"].as_f64().unwrap();
    wait_for(&mut app, first);
    assert!(app.filter_preview.as_ref().unwrap().result.is_some());

    // A preview for old values occupies the worker until it is cancelled.
    let dialog = top_dialog(&app);
    let st = app.session.active().unwrap();
    let stale = crate::filter_dialog::FilterPreviewKey {
        doc: st.doc.id,
        revision: st.revision,
        sources: Vec::new(),
        dialog,
        open: app.framework.dialogs.open_serial(dialog).unwrap_or(0),
        active: st.active_layer,
        target: st.channel_view.target,
        command: "filter.blur.gaussianBlur".into(),
        params: json!({"radius": 999.0}),
        k: 1,
        tool_colors: (app.session.tools.foreground, app.session.tools.background),
        scratch: crate::scratch_reads::generation(),
    };
    let (told, cancelled) = std::sync::mpsc::channel();
    app.filter_preview_worker
        .start(stale, ctx.clone(), move |job| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !job.cancelled() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let _ = told.send(job.cancelled());
            crate::dialogs::filter_preview_worker::Computed { result: None, ms: 0.0 }
        })
        .unwrap();
    // The slider moves on and stops: once the value settles, the stale preview is cancelled
    // instead of being waited out.
    set_field(&mut app, &ctx, "radius", json!(12.0));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let was_cancelled = loop {
        ensure_filter_preview(&mut app, 0, &ctx);
        if let Ok(c) = cancelled.try_recv() {
            break c;
        }
        assert!(std::time::Instant::now() < deadline, "the stale preview was never cancelled");
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    assert!(was_cancelled, "the stale preview stopped because it was cancelled, not by timing out");
    wait_for(&mut app, 12.0);
    assert!(app.filter_preview.as_ref().unwrap().result.is_some());
}

// Starts a worker job that computes a real preview for the dialog as it stands, but only
// finishes when the returned sender is signalled. Gating it here keeps these tests off the
// clock: `supersede` cancels a running job once the values have been still for SETTLE_MS, so a
// test that relies on a real blur beating a 150 ms timer is a test that fails on a loaded box.
#[cfg(not(target_arch = "wasm32"))]
fn gated_blur_preview(app: &mut PhotocraftApp, ctx: &egui::Context) -> (std::sync::mpsc::Sender<()>, f64) {
    let dialog = top_dialog(app);
    let st = app.session.active().expect("a document is active");
    let command = "filter.blur.gaussianBlur";
    let params = app.with_mask_target(command, crate::filter_dialog::params_of(&fields(app)));
    assert_eq!(
        crate::filter_dialog::preview_factor(command, &params, 1),
        1,
        "a 64x48 document previews at full resolution, and these tests rely on k staying 1"
    );
    let key = crate::filter_dialog::FilterPreviewKey {
        doc: st.doc.id,
        revision: st.revision,
        sources: Vec::new(),
        dialog,
        open: app.framework.dialogs.open_serial(dialog).unwrap_or(0),
        active: st.active_layer,
        target: st.channel_view.target,
        command: command.into(),
        params,
        k: 1,
        tool_colors: (app.session.tools.foreground, app.session.tools.background),
        scratch: crate::scratch_reads::generation(),
    };
    let doc = app.session.documents()[0].doc.clone();
    let documents: Vec<_> = app.session.documents().iter().map(|d| d.doc.clone()).collect();
    let radius = key.params["radius"].as_f64().expect("the dialog seeds a radius");
    let (release, wait) = std::sync::mpsc::channel::<()>();
    app.filter_preview_worker
        .start(key.clone(), ctx.clone(), move |_| {
            wait.recv_timeout(std::time::Duration::from_secs(60)).expect("the test releases the job");
            let result = crate::filter_dialog::preview_document_for_target(
                &doc,
                key.active,
                &key.command,
                &key.params,
                key.k,
                None,
                key.target,
                &documents,
                &Default::default(),
                Some(key.tool_colors),
            )
            .map(|d| {
                let buffer = photocraft_compose::flatten(&d);
                (std::sync::Arc::new(d), buffer)
            });
            crate::dialogs::filter_preview_worker::Computed { result, ms: 0.0 }
        })
        .expect("the worker starts");
    (release, radius)
}

// The worker hands its result to the channel only after `compute` returns, so a signal sent
// from inside `compute` would still race that send. These tests drive the canvas until the
// state they need appears instead of assuming a timing.
#[cfg(not(target_arch = "wasm32"))]
fn drive_until(app: &mut PhotocraftApp, ctx: &egui::Context, what: &str, done: impl Fn(&PhotocraftApp) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !done(app) {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
        ensure_filter_preview(app, 0, ctx);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_held_preview_frame_stays_on_the_canvas_when_the_proxy_factor_changes() {
    // #10474: Minimum and Maximum take their proxy factor from the radius, so a radius drag across
    // a whole pixel changes `FilterPreviewKey::k`. The frame for the old factor stays on the
    // canvas, drawn at its own factor, until the one for the new factor lands.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 2400, "height": 2200})).unwrap();
    app.background_jobs = true;
    app.ui.views.get_mut(0).expect("the new document's view").zoom = 0.25;
    open_filter(&mut app, "filter.other.maximum");
    let ctx = app.ctx().clone();
    set_field(&mut app, &ctx, "radius", json!(6.0));
    drive_until(&mut app, &ctx, "the first frame", |a| a.filter_preview.as_ref().is_some_and(|p| p.result.is_some()));
    assert_eq!(app.filter_preview.as_ref().map(|p| p.key.k), Some(2), "6 px on a reduced document previews on the proxy");

    // 1.5 px previews at full resolution: a new request at a new factor, computed on the worker.
    set_field(&mut app, &ctx, "radius", json!(1.5));
    let (k, _, size) = ensure_filter_preview(&mut app, 0, &ctx).expect("the held frame stays on the canvas");
    assert_eq!((k, size), (2, [1200, 1100]), "drawn at its own factor");
    drive_until(&mut app, &ctx, "the full-resolution frame", |a| a.filter_preview.as_ref().is_some_and(|p| p.key.k == 1 && p.result.is_some()));
    let (k, _, size) = ensure_filter_preview(&mut app, 0, &ctx).expect("the new frame");
    assert_eq!((k, size), (1, [2400, 2200]));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn indexed_matte_preview_matches_commit_and_refreshes_tool_colors_inline_and_on_worker() {
    for background in [false, true] {
        for depth in [8, 16, 32] {
            for matte in ["foreground", "background"] {
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                app.background_jobs = background;
                app.run("file.new", json!({"width": 64, "height": 48, "depth": depth, "background": "transparent"})).unwrap();
                app.run("edit.assignProfile", json!({"profile": "adobe-rgb-compat"})).unwrap();
                app.run("edit.colorSettings", json!({"workingRgb": "display-p3"})).unwrap();
                app.session
                    .edit("Translucent edge", |doc, active| {
                        let format = doc.pixel_format();
                        doc.layer_mut(active.unwrap())
                            .unwrap()
                            .surface_mut()
                            .unwrap()
                            .fill_rect(photocraft_geom::Rect::new(0, 0, 32, 48), &photocraft_raster::from_rgba(&format, [1.0, 0.0, 0.0, 0.5]));
                        Ok(())
                    })
                    .unwrap();
                let source = app.session.active().unwrap().doc.clone();
                let params = json!({"palette": "exact", "forced": "none", "dither": "none", "matte": matte});
                open_filter(&mut app, "image.mode.indexedColor");
                let ctx = app.ctx().clone();
                set_fields(&mut app, &ctx, &params);
                let mut previous = None;
                for (foreground, background_color) in [("#3366cc", "#cc6633"), ("#26bf59", "#bf26a6")] {
                    app.session.execute("tools.setColors", json!({"foreground": foreground, "background": background_color})).unwrap();
                    let colors = (app.session.tools.foreground, app.session.tools.background);
                    let shown = ensure_filter_preview(&mut app, 0, &ctx);
                    if background {
                        assert!(shown.is_none(), "old tool colors must not remain visible");
                    }
                    drive_until(&mut app, &ctx, "the current matte colors", |app| {
                        app.filter_preview.as_ref().is_some_and(|p| p.key.tool_colors == colors && p.result.is_some())
                    });
                    let preview = app.filter_preview.as_ref().unwrap().result.as_ref().unwrap();
                    let mut applied = photocraft_engine::Session::new();
                    applied.color.settings = app.session.color.settings.clone();
                    applied.tools.foreground = colors.0;
                    applied.tools.background = colors.1;
                    applied.add_document((*source).clone(), None);
                    applied.execute("image.mode.indexedColor", params.clone()).unwrap();
                    let committed = &applied.active().unwrap().doc;
                    assert_eq!(preview.icc_profile, source.icc_profile);
                    assert_eq!(preview.color_table, committed.color_table);
                    let actual = preview.layers[0].surface().unwrap().read_region(preview.bounds());
                    let expected = committed.layers[0].surface().unwrap().read_region(committed.bounds());
                    assert_eq!(actual, expected, "{matte}, {depth}-bit, worker={background}");
                    if let Some(previous) = previous {
                        assert_ne!(actual, previous, "changing the selected tool color refreshes the pixels");
                    }
                    previous = Some(actual);
                    assert_eq!(app.session.active().unwrap().doc.mode, photocraft_doc::ColorMode::Rgb, "preview does not commit");
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn cross_scale_indexed_matte_preview_rejects_old_tool_colors_from_cached_and_worker_results() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.background_jobs = true;
    app.run("file.new", json!({"width": 2400, "height": 2200, "depth": 8, "background": "transparent"})).unwrap();
    app.run("edit.assignProfile", json!({"profile": "adobe-rgb-compat"})).unwrap();
    app.run("edit.colorSettings", json!({"workingRgb": "display-p3"})).unwrap();
    app.session
        .edit("Translucent edge", |doc, active| {
            let format = doc.pixel_format();
            doc.layer_mut(active.unwrap())
                .unwrap()
                .surface_mut()
                .unwrap()
                .fill_rect(photocraft_geom::Rect::new(0, 0, 32, 48), &photocraft_raster::from_rgba(&format, [1.0, 0.0, 0.0, 0.5]));
            Ok(())
        })
        .unwrap();
    app.ui.views.get_mut(0).expect("the new document's view").zoom = 0.25;
    let params = json!({"palette": "exact", "forced": "none", "dither": "none", "matte": "foreground"});
    open_filter(&mut app, "image.mode.indexedColor");
    let ctx = app.ctx().clone();
    set_fields(&mut app, &ctx, &params);
    app.session.execute("tools.setColors", json!({"foreground": "#3366cc", "background": "#cc6633"})).unwrap();
    drive_until(&mut app, &ctx, "the first indexed preview", |a| a.filter_preview.as_ref().is_some_and(|p| p.result.is_some()));
    let old = app.filter_preview.as_ref().expect("the first preview").key.clone();
    assert_eq!(old.k, 2, "the zoomed-out large document uses the reduced preview");

    app.session.execute("tools.setColors", json!({"foreground": "#26bf59", "background": "#bf26a6"})).unwrap();
    app.ui.views.get_mut(0).expect("the new document's view").zoom = 1.0;
    let new_colors = (app.session.tools.foreground, app.session.tools.background);
    let new = crate::filter_dialog::FilterPreviewKey { k: 1, tool_colors: new_colors, ..old.clone() };
    assert_ne!(old.tool_colors, new.tool_colors);
    assert_ne!(old.k, new.k, "the cache and request use different preview factors");

    // Keep the current worker pending so the only possible frame is the cached old-color one.
    let (release_new, wait_new) = std::sync::mpsc::channel();
    app.filter_preview_worker
        .start(new.clone(), ctx.clone(), move |_| {
            wait_new.recv_timeout(std::time::Duration::from_secs(60)).expect("the test releases the job");
            crate::dialogs::filter_preview_worker::Computed { result: None, ms: 0.0 }
        })
        .expect("the worker starts");
    assert!(ensure_filter_preview(&mut app, 0, &ctx).is_none(), "the cached old-color frame is not painted while the new worker runs");
    release_new.send(()).expect("released");
    drive_until(&mut app, &ctx, "the new worker to finish", |a| !a.filter_preview_worker.busy());
    assert_eq!(app.filter_preview.as_ref().expect("the current request").key, new);
    assert!(app.filter_preview.as_ref().unwrap().result.is_none());

    // Start an old-color worker while its request is current, then move the dialog to a new
    // cross-scale request before the old result lands. Its real preview must not replace the
    // current request through the loose worker-result gate.
    app.session.execute("tools.setColors", json!({"foreground": "#3366cc", "background": "#cc6633"})).unwrap();
    app.ui.views.get_mut(0).expect("the new document's view").zoom = 0.25;
    let old_worker = old.clone();
    let doc = app.session.documents()[0].doc.clone();
    let documents: Vec<_> = app.session.documents().iter().map(|d| d.doc.clone()).collect();
    let color = app.session.color.settings.clone();
    let (release_old, wait_old) = std::sync::mpsc::channel();
    app.filter_preview_worker
        .start(old_worker.clone(), ctx.clone(), move |_| {
            wait_old.recv_timeout(std::time::Duration::from_secs(60)).expect("the test releases the job");
            let result = crate::filter_dialog::preview_document_for_target(
                &doc,
                old_worker.active,
                &old_worker.command,
                &old_worker.params,
                old_worker.k,
                None,
                old_worker.target,
                &documents,
                &color,
                Some(old_worker.tool_colors),
            )
            .map(|doc| {
                let buffer = photocraft_compose::flatten(&doc);
                (std::sync::Arc::new(doc), buffer)
            });
            crate::dialogs::filter_preview_worker::Computed { result, ms: 0.0 }
        })
        .expect("the worker starts");
    assert!(ensure_filter_preview(&mut app, 0, &ctx).is_none(), "the old cached request is not current");

    app.session.execute("tools.setColors", json!({"foreground": "#26bf59", "background": "#bf26a6"})).unwrap();
    app.ui.views.get_mut(0).expect("the new document's view").zoom = 1.0;
    set_field(&mut app, &ctx, "matte", json!("background"));
    let mut new_late = new.clone();
    new_late.params.as_object_mut().unwrap().insert("matte".into(), json!("background"));
    assert!(ensure_filter_preview(&mut app, 0, &ctx).is_none(), "the current request has no result while the old worker runs");
    release_old.send(()).expect("released");
    drive_until(&mut app, &ctx, "the stale old-color worker to finish", |a| {
        if let Some(shown) = &a.filter_preview {
            assert_eq!(shown.key.tool_colors, new.tool_colors, "the stale old-color worker result was not painted");
            assert_ne!(shown.key.k, old.k, "the stale old-scale worker result was not painted");
        }
        a.filter_preview.as_ref().is_some_and(|p| p.key == new_late && p.result.is_some()) && !a.filter_preview_worker.busy()
    });
    assert_eq!(app.filter_preview.as_ref().expect("the current request").key, new_late);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn match_color_preview_rejects_cached_and_late_frames_from_an_old_source_layer_or_revision() {
    use crate::dialogs::filter_preview_worker::Computed;

    let command = "image.adjustments.matchColor";
    for change_layer in [true, false] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
        app.run("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
        let red = app.session.active().unwrap().active_layer.unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#cc4422"})).unwrap();
        app.run("layer.new.layer", json!({"name": "Blue"})).unwrap();
        let blue = app.session.active().unwrap().active_layer.unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#2266cc"})).unwrap();
        assert!(app.session.set_active(0));
        open_filter(&mut app, command);
        let ctx = app.ctx().clone();
        set_fields(&mut app, &ctx, &json!({"source": "1", "sourceLayer": red}));
        assert!(ensure_filter_preview(&mut app, 0, &ctx).is_some(), "the first source preview reaches the canvas");
        let preview = app.filter_preview.as_ref().unwrap();
        let old = preview.key.clone();
        let doc = preview.result.as_ref().unwrap().clone();
        let mut slider = old.clone();
        slider.params["fade"] = json!(25.0);
        assert!(same_target(&app, &slider), "a slider on the same source layer keeps the last frame");

        app.background_jobs = true;
        let (release, wait) = std::sync::mpsc::channel();
        assert!(
            app.filter_preview_worker
                .start(old.clone(), ctx.clone(), move |_| {
                    wait.recv_timeout(std::time::Duration::from_secs(60)).expect("the test releases the job");
                    let buffer = photocraft_compose::flatten(&doc);
                    Computed { result: Some((doc, buffer)), ms: 0.0 }
                })
                .unwrap()
        );
        let current_layer = if change_layer {
            set_field(&mut app, &ctx, "sourceLayer", json!(blue));
            blue
        } else {
            assert!(app.session.set_active(1));
            app.session.select_layer(red).unwrap();
            app.session.execute("edit.fill", json!({"contents": "color", "color": "#448822"})).unwrap();
            assert!(app.session.set_active(0));
            red
        };
        let source = &app.session.documents()[1];
        let sources = vec![(source.doc.id, source.revision)];
        assert!(ensure_filter_preview(&mut app, 0, &ctx).is_none(), "the cached old preview is not displayed");
        app.filter_preview = None;
        release.send(()).unwrap();
        drive_until(&mut app, &ctx, "a current Match Color preview", |app| {
            let Some(preview) = &app.filter_preview else { return false };
            assert_eq!(preview.key.params["sourceLayer"], json!(current_layer), "a late worker published the wrong layer");
            assert_eq!(preview.key.sources, sources, "a late worker published the wrong source revision");
            preview.result.is_some() && !app.filter_preview_worker.busy()
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_preview_computed_mid_drag_reaches_the_canvas_and_then_converges() {
    // A drag used to paint nothing at all: every completion was dropped for carrying stale
    // parameters, so the canvas only ever showed an effect once the pointer had come to rest.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.background_jobs = true;
    open_filter(&mut app, "filter.blur.gaussianBlur");
    let ctx = app.ctx().clone();
    let (release, first) = gated_blur_preview(&mut app, &ctx);

    // The pointer moves on before the frame lands.
    let moved = first + 4.0;
    set_field(&mut app, &ctx, "radius", json!(moved));
    release.send(()).expect("released");
    drive_until(&mut app, &ctx, "the mid-drag frame", |a| a.filter_preview.is_some());

    // The finished frame is painted even though the dialog has moved on.
    let shown = app.filter_preview.as_ref().expect("the mid-drag frame was published");
    assert_eq!(shown.key.params["radius"].as_f64(), Some(first), "the frame is the one the worker finished");
    assert!(shown.result.is_some());
    assert_eq!(fields(&app)["radius"].as_f64(), Some(moved), "the slider kept moving");
    // The display gate lets it through, so the canvas has something to draw: this is what
    // re-adding `params` to that gate would break.
    assert!(ensure_filter_preview(&mut app, 0, &ctx).is_some(), "the canvas draws the mid-drag frame");

    // And the preview still converges on the value the dialog holds. `fresh` has to stay an
    // exact match for that: relaxed, the canvas would believe it already had the current
    // request after the first frame of a drag and never compute again.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.filter_preview.as_ref().is_none_or(|p| p.key.params["radius"].as_f64() != Some(moved)) {
        assert!(std::time::Instant::now() < deadline, "the preview never converged on the current radius");
        ensure_filter_preview(&mut app, 0, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_mid_drag_frame_does_not_replace_a_frame_that_is_already_exact() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.background_jobs = true;
    open_filter(&mut app, "filter.blur.gaussianBlur");
    let ctx = app.ctx().clone();

    // A frame for the radius the dialog holds right now reaches the canvas. This also proves
    // the hand-built key is the one the canvas builds: it can only be published by the exact
    // arm, which requires `finished == request`.
    let (release, first) = gated_blur_preview(&mut app, &ctx);
    release.send(()).expect("released");
    drive_until(&mut app, &ctx, "the first frame", |a| a.filter_preview.is_some());
    assert_eq!(app.filter_preview.as_ref().expect("published").key.params["radius"].as_f64(), Some(first));

    // The slider moves away and a frame for the new radius is computed...
    let moved = first + 4.0;
    set_field(&mut app, &ctx, "radius", json!(moved));
    let (release_moved, _) = gated_blur_preview(&mut app, &ctx);
    // ...and back to `first` before that frame lands.
    set_field(&mut app, &ctx, "radius", json!(first));
    release_moved.send(()).expect("released");

    // The frame for `moved` arrives late. It belongs to this preview, but the frame on screen
    // is already exact for the dialog, so it must never be shown in place of it. Checked on
    // every pass: a wrong frame published and then overwritten before the loop ends would
    // otherwise never be seen.
    drive_until(&mut app, &ctx, "the late frame to arrive", |a| {
        if let Some(shown) = &a.filter_preview {
            assert_ne!(shown.key.params["radius"].as_f64(), Some(moved), "a late frame replaced an exact one");
        }
        !a.filter_preview_worker.busy()
    });
    let shown = app.filter_preview.as_ref().expect("a frame is on screen");
    assert_eq!(shown.key.params["radius"].as_f64(), Some(first), "a late frame replaced an exact one");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_mid_drag_empty_result_leaves_the_preview_alone() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.background_jobs = true;
    open_filter(&mut app, "filter.blur.gaussianBlur");
    let ctx = app.ctx().clone();

    let (release, first) = gated_blur_preview(&mut app, &ctx);
    release.send(()).expect("released");
    drive_until(&mut app, &ctx, "the first frame", |a| a.filter_preview.is_some());
    assert!(app.filter_preview.as_ref().expect("published").result.is_some());

    // The slider moves and a job for the new radius is started, then the slider moves again
    // before that job lands: its result is stale by the time it arrives.
    let moved = first + 4.0;
    set_field(&mut app, &ctx, "radius", json!(moved));
    let dialog = top_dialog(&app);
    let st = app.session.active().expect("a document is active");
    let key = crate::filter_dialog::FilterPreviewKey {
        doc: st.doc.id,
        revision: st.revision,
        sources: Vec::new(),
        dialog,
        open: app.framework.dialogs.open_serial(dialog).unwrap_or(0),
        active: st.active_layer,
        target: st.channel_view.target,
        command: "filter.blur.gaussianBlur".into(),
        params: json!({"radius": moved}),
        k: 1,
        tool_colors: (app.session.tools.foreground, app.session.tools.background),
        scratch: crate::scratch_reads::generation(),
    };
    let (release_empty, wait) = std::sync::mpsc::channel::<()>();
    app.filter_preview_worker
        .start(key, ctx.clone(), move |_| {
            wait.recv_timeout(std::time::Duration::from_secs(60)).expect("released");
            crate::dialogs::filter_preview_worker::Computed { result: None, ms: 0.0 }
        })
        .expect("the worker starts");
    set_field(&mut app, &ctx, "radius", json!(moved + 4.0));
    release_empty.send(()).expect("released");

    // Storing an empty result here would drop the canvas back to the unfiltered layer for a
    // frame. The exact request may still store one; a stale one may not. Checked on every
    // pass, because the reopened preview that follows would otherwise repair the symptom.
    drive_until(&mut app, &ctx, "the empty frame to arrive", |a| {
        if let Some(shown) = &a.filter_preview {
            assert!(shown.result.is_some(), "an empty mid-drag result blanked the canvas");
        }
        !a.filter_preview_worker.busy()
    });
    assert!(app.filter_preview.as_ref().expect("a preview is still shown").result.is_some(), "an empty mid-drag result blanked the canvas");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_reopened_dialog_never_paints_the_previous_dialogs_preview() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.background_jobs = true;
    open_filter(&mut app, "filter.blur.gaussianBlur");
    let ctx = app.ctx().clone();
    let first_dialog = top_dialog(&app);
    let first_open = app.framework.dialogs.open_serial(first_dialog);
    let (release, _) = gated_blur_preview(&mut app, &ctx);

    // Close that dialog and open another one for the same layer and the same command. Only the
    // dialog id differs, which is exactly what the preview session has to notice.
    assert!(app.framework.dialogs.cancel_by_id(&ctx, first_dialog), "the first dialog closes");
    open_filter(&mut app, "filter.blur.gaussianBlur");
    let second_dialog = top_dialog(&app);
    let (first_open, second_open) =
        (first_open.expect("the first dialog was open"), app.framework.dialogs.open_serial(second_dialog).expect("the second dialog is open"));
    assert_ne!(second_open, first_open, "a reopened dialog is a new open, though its id repeats");
    release.send(()).expect("released");

    // The first dialog's frame arrives late and must not be painted under the second one.
    // `discard_closed` usually reclaims it a frame earlier, which is why this drives the gate
    // directly: the id check is the part that has to hold when that cleanup misses. The
    // invariant is checked on every pass, not only at the end, because a wrongly published
    // frame can still be replaced by the reopened dialog's own preview before we stop.
    drive_until(&mut app, &ctx, "the first dialog's frame to arrive", |a| {
        if let Some(preview) = &a.filter_preview {
            assert_ne!(preview.key.open, first_open, "a reopened dialog painted the previous dialog's preview");
        }
        !a.filter_preview_worker.busy()
    });
}
