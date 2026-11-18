use super::*;

/// A 160 × 100 document: warm on the left (x < 80), cool on the right, with the top-left
/// 20 × 20 selected.
fn app_with_selection() -> PhotocraftApp {
    app_sized(160, 100)
}

fn app_sized(w: i32, h: i32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": w, "height": h, "depth": 8})).unwrap();
    app.session
        .edit("paint", |doc, _| {
            let s = doc.layers[0].surface_mut().unwrap();
            s.fill_rect(Rect::new(0, 0, w / 2, h), &[0.85, 0.55, 0.25, 1.0]);
            s.fill_rect(Rect::new(w / 2, 0, w, h), &[0.15, 0.35, 0.75, 1.0]);
            Ok(())
        })
        .unwrap();
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    app.sync_views();
    app
}

/// Sends `v` to the open workspace the way `ui.menu.invoke`'s `{ui: …}` does; the reply merges the workspace's
/// fields with the app facts.
fn try_ui(app: &mut PhotocraftApp, v: Value) -> Result<Value, String> {
    let ctx = app.ctx().clone();
    crate::control::dialogs::workspace_ui(app, &ctx, "select.selectAndMask", &v)
}

fn ui(app: &mut PhotocraftApp, v: Value) -> Value {
    try_ui(app, v).unwrap()
}

/// The workspace's fields and app facts as the control reply gives them.
fn fields(app: &mut PhotocraftApp) -> Value {
    ui(app, json!({}))
}

fn selected(app: &mut PhotocraftApp) -> u64 {
    ui(app, json!({"waitPreview": true}))["selectedProxyPixels"].as_u64().unwrap()
}

fn cov(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    app.session.active().unwrap().doc.selection.as_ref().map_or(0.0, |s| s.sample_channel(x, y, 0))
}

/// The source the open workspace shows.
fn src(app: &PhotocraftApp) -> Arc<PcSelectMaskSource> {
    source(app).expect("Select and Mask is open")
}

/// Lets the workspace's source follow the document, as the app does each pass.
fn follow(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    take(app, &ctx);
}

/// A layer mask without a selection (or the targeted mask) is refined and output to the mask.
#[test]
fn a_layer_mask_opens_on_the_mask() {
    let mut app = app_with_selection();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    // Revealing the selection uses it up (#8381), leaving the mask alone.
    app.session.execute("layer.layerMask.revealSelection", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_none());
    open(&mut app).unwrap();
    assert_eq!(describe(&app).unwrap()["settings"]["output"].as_str(), Some("layerMask"));
    assert_eq!(selected(&mut app), 400, "the mask is what is shown");
}

/// OK runs `select.refineEdge` with the edits, strokes and settings: one history step, the same
/// result as the command with those params, and the workspace closes.
#[test]
fn ok_is_one_step_and_matches_the_command() {
    let mut app = app_with_selection();
    let mut direct = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"radius": 2, "stroke": {"points": [[120.0, 50.0]]}}));
    ui(&mut app, json!({"tool": "brush", "stroke": {"points": [[40.0, 60.0], [60.0, 60.0]]}}));
    let params = commit_params(&app).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    ui(&mut app, json!({"commit": true}));
    assert!(!app.dialog_open(), "OK closes");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Select and Mask"));
    direct.session.execute(crate::select_mask_view::COMMAND, params).unwrap();
    for (x, y) in [(5, 5), (120, 50), (50, 60), (50, 80), (79, 50)] {
        assert_eq!(cov(&app, x, y), cov(&direct, x, y), "({x}, {y})");
    }
    assert!(cov(&app, 120, 50) > 0.5 && cov(&app, 50, 60) > 0.5 && cov(&app, 50, 80) < 0.5);
}

/// Remember Settings keeps the settings for the next time, written on OK; without it the workspace opens on
/// Photoshop's defaults.
#[test]
fn remember_settings() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"remember": true, "radius": 7, "view": "overlay", "overlayColor": [0, 0, 255], "commit": true}));
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    open(&mut app).unwrap();
    let s = describe(&app).unwrap()["settings"].clone();
    assert_eq!((s["radius"].as_f64(), s["view"].as_str(), s["overlayColor"].clone()), (Some(7.0), Some("overlay"), json!([0, 0, 255])));
    ui(&mut app, json!({"remember": false, "commit": true}));
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    open(&mut app).unwrap();
    let s = describe(&app).unwrap()["settings"].clone();
    assert_eq!((s["radius"].as_f64(), s["view"].as_str()), (Some(0.0), Some("onionSkin")));
}

/// High Quality Preview runs on the document at full size; otherwise on a copy at most 1600 px.
#[test]
fn high_quality_preview_is_full_size() {
    let mut app = app_sized(3300, 60);
    open(&mut app).unwrap();
    assert_eq!(describe(&app).unwrap()["proxyScale"].as_u64(), Some(3));
    let reduced = selected(&mut app);
    assert!((40..=60).contains(&reduced), "{reduced} reduced px of the 20 × 20 selection");
    assert_eq!(ui(&mut app, json!({"highQualityPreview": true}))["proxyScale"].as_u64(), Some(1));
    assert_eq!(selected(&mut app), 400);
}

/// Edits scale with the reduced copy: points, rectangles and sizes by 1 / k.
#[test]
fn edits_scale_to_the_reduced_copy() {
    let e = json!({"tool": "quick", "points": [[30.0, 60.0]], "size": 30.0, "rect": [3, 6, 9, 12], "point": [15, 3]});
    assert_eq!(scale_edit(&e, 3), json!({"tool": "quick", "points": [[10.0, 20.0]], "size": 10.0, "rect": [1.0, 2.0, 3.0, 4.0], "point": [5.0, 1.0]}));
    assert_eq!(scale_edit(&json!({"size": 2.0}), 4)["size"], json!(1.0), "never below a pixel");
    assert_eq!(scale_edit(&json!("x"), 2), json!("x"));
}

/// Queues one pointer event and runs frames, so the tool host delivers it to the workspace's canvas.
fn pointer(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, event: Value) {
    call(h.state_mut(), "ui.pointer", json!({"events": [event]}));
    h.run_steps(3);
}

/// With Real-Time Refinement the preview request carries the stroke being drawn; without it, it does not.
#[test]
fn real_time_refinement_previews_the_stroke_being_drawn() {
    for real_time in [true, false] {
        let mut app = app_with_selection();
        open(&mut app).unwrap();
        let mut h = egui_kittest::Harness::builder()
            .with_context(app.ctx().clone())
            .build_ui_state(|ui, app: &mut PhotocraftApp| app.framework.dialogs.run(ui), app);
        h.run_steps(3);
        ui(h.state_mut(), json!({"realTimeRefinement": real_time}));
        selected(h.state_mut());
        pointer(&mut h, json!({"kind": "down", "x": 120, "y": 50, "pressure": 1.0}));
        pointer(&mut h, json!({"kind": "move", "x": 130, "y": 50, "pressure": 1.0}));
        selected(h.state_mut());
        let during = src(h.state()).last_request().expect("a preview was requested").session.edits.len();
        assert_eq!(during, usize::from(real_time), "real time {real_time}: the stroke so far in the request");
        pointer(&mut h, json!({"kind": "up", "x": 130, "y": 50}));
        selected(h.state_mut());
        assert_eq!(fields(h.state_mut())["edits"].as_u64(), Some(1));
        assert_eq!(src(h.state()).last_request().unwrap().session.edits.len(), 1, "the released stroke is in the request");
    }
}

/// Opening again while the workspace is open keeps its work and says why.
#[test]
fn opening_again_keeps_the_open_workspace() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"stroke": {"points": [[120.0, 50.0], [130.0, 50.0]]}}));
    let ctx = app.ctx().clone();
    let e = crate::menus::invoke(&mut app, &ctx, "select.selectAndMask", json!({}));
    assert!(e.is_err_and(|e| e.contains("already open")));
    assert_eq!(fields(&mut app)["edits"].as_u64(), Some(1), "the edit survives");
}

/// The preview keeps the selection after the edits it applied and starts the next from there:
/// after undo it starts over.
#[test]
fn the_preview_reuses_the_edits_it_applied() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"stroke": {"points": [[120.0, 50.0]]}}));
    selected(&mut app);
    assert_eq!(src(&app).cached_edits(), Some(1));
    ui(&mut app, json!({"tool": "brush", "stroke": {"points": [[40.0, 60.0]]}}));
    let both = selected(&mut app);
    assert_eq!(src(&app).cached_edits(), Some(2));
    ui(&mut app, json!({"undo": true}));
    let one = selected(&mut app);
    assert!(one < both);
    assert_eq!(src(&app).cached_edits(), Some(1));
}

/// The selection the edits made on an older document is not reused: a Quick Selection stroke
/// on a repainted image previews what OK would commit.
#[test]
fn the_preview_recomputes_the_edits_after_the_document_changes() {
    let warm = |app: &mut PhotocraftApp| {
        app.session
            .edit("paint", |doc, _| {
                doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(0, 0, 160, 100), &[0.85, 0.55, 0.25, 1.0]);
                Ok(())
            })
            .unwrap();
    };
    let stroke = json!({"stroke": {"points": [[120.0, 50.0], [130.0, 50.0]]}});
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, stroke.clone());
    let two_tone = selected(&mut app);
    // The pixels change under the workspace (a control-channel command) before it redraws.
    warm(&mut app);
    follow(&mut app);
    let after = selected(&mut app);
    let mut fresh = app_with_selection();
    warm(&mut fresh);
    open(&mut fresh).unwrap();
    ui(&mut fresh, stroke);
    let expected = selected(&mut fresh);
    assert_ne!(two_tone, expected, "the repaint changes what the stroke selects");
    assert_eq!(after, expected);
    assert_eq!(src(&app).cached_revision(), app.session.active().map(|s| s.revision));
}

/// A request with a bad field is refused for that field: the reply names it and its value stays.
#[test]
fn a_refused_request_changes_nothing() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    let before = fields(&mut app);
    for (bad, key) in [
        (json!({"mode": "invalid"}), "mode"),
        (json!({"stroke": {"points": "x"}}), "stroke"),
        (json!({"stroke": {"points": [[1.0]]}}), "stroke"),
    ] {
        let err = try_ui(&mut app, bad.clone()).expect_err(&bad.to_string());
        assert!(err.contains(key), "{bad}: the refusal names the key: {err}");
        let now = fields(&mut app);
        assert_eq!(now[key], before[key], "{bad}: the refused key keeps its value");
        assert_eq!((now["edits"].as_u64(), now["strokes"].as_u64()), (Some(0), Some(0)), "{bad}");
    }
}

/// With nothing selected OK refines nothing, so Show Edge shows no Refine Edge Brush stroke.
#[test]
fn show_edge_shows_no_stroke_without_a_selection() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"clearSelection": true, "showEdge": true}));
    let empty = selected(&mut app);
    ui(&mut app, json!({"tool": "refineEdge", "stroke": {"points": [[40.0, 20.0], [40.0, 80.0]]}}));
    assert_eq!(selected(&mut app), empty);
    ui(&mut app, json!({"commit": true}));
    assert_eq!(cov(&app, 40, 50), 0.0, "and OK selects nothing there");
}

/// A preview of pixels the document no longer has is not shown: after a document edit the old view is gone
/// until the preview of the new pixels lands.
#[test]
fn a_preview_of_an_old_revision_is_dropped() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    selected(&mut app);
    assert_eq!(describe(&app).unwrap()["previewReady"].as_bool(), Some(true));
    app.session.execute("select.all", json!({})).unwrap();
    follow(&mut app);
    // PLAN: assumed `previewReady` is false while the shown picture is of an older revision
    assert_eq!(describe(&app).unwrap()["previewReady"].as_bool(), Some(false), "the old preview is not shown");
    assert_eq!(selected(&mut app), 160 * 100, "the new preview shows the new selection");
}

fn call(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    let (req, _rx) = crate::control::ControlRequest::new(method, params);
    let ctx = app.ctx().clone();
    match crate::control::handle(app, &ctx, &req) {
        crate::control::Reply::Done(v) => v,
        _ => panic!("{method}: expected an immediate reply"),
    }
}

/// A second document with its own selection, the first one active again.
fn two_documents() -> PhotocraftApp {
    let mut app = app_with_selection();
    app.session.execute("file.new", json!({"width": 160, "height": 100, "depth": 8})).unwrap();
    app.session.execute("select.rect", json!({"x": 100, "y": 50, "width": 30, "height": 30})).unwrap();
    assert!(app.session.set_active(0));
    app.sync_views();
    app
}

fn fingerprint(app: &PhotocraftApp, i: usize) -> (u64, Option<f32>, Option<f32>) {
    let st = &app.session.documents()[i];
    let sel = st.doc.selection.as_ref();
    (st.revision, sel.map(|s| s.sample_channel(10, 10, 0)), sel.map(|s| s.sample_channel(120, 50, 0)))
}

/// The #8896 review: a document switch and OK queued on the control channel, with no frame
/// between them, must not apply the first document's edits to the second.
#[test]
fn ok_after_a_document_switch_changes_neither_document() {
    let mut app = two_documents();
    open(&mut app).unwrap();
    ui(&mut app, json!({"stroke": {"points": [[120.0, 50.0], [130.0, 50.0]]}}));
    let before = (fingerprint(&app, 0), fingerprint(&app, 1));
    let switched = call(&mut app, "engine.execute", json!({"command": "document.activate", "params": {"document": 1}}));
    assert_eq!(switched["ok"], false, "external edits respect the modal gate");
    assert_eq!(app.session.active_index(), Some(0));
    assert!(app.session.set_active(1), "simulate a trusted native target change before the next frame");
    let ctx = app.ctx().clone();
    crate::distort_ui::drop_foreign(&mut app, &ctx);
    assert!(!app.dialog_open(), "the stale workspace is closed");
    assert!(try_ui(&mut app, json!({"commit": true})).is_err(), "the stale OK is refused");
    assert_eq!((fingerprint(&app, 0), fingerprint(&app, 1)), before, "neither document changed");
}

/// Control requests on another document are refused, and none of them edits that document.
#[test]
fn control_and_preview_on_another_document_are_refused() {
    for route in ["control", "preview"] {
        let mut app = two_documents();
        open(&mut app).unwrap();
        let before = fingerprint(&app, 1);
        assert!(app.session.set_active(1));
        let request = match route {
            "control" => json!({"stroke": {"points": [[120.0, 50.0], [130.0, 50.0]]}}),
            _ => json!({"waitPreview": true}),
        };
        assert!(try_ui(&mut app, request).is_err(), "{route}");
        assert!(!app.dialog_open(), "{route}: the stale workspace is closed");
        assert_eq!(fingerprint(&app, 1), before, "{route}: the other document is untouched");
    }
}

/// Selecting another layer in the same document (a shortcut, or the control channel between
/// requests) must not send a layer-mask OK to that layer.
#[test]
fn ok_after_a_layer_switch_leaves_the_new_layer_alone() {
    let mut app = app_with_selection();
    let first = app.session.active().unwrap().active_layer.unwrap();
    let other = app.run("layer.new.layer", json!({"name": "Other"})).unwrap()["layer"].as_u64().map(LayerId).unwrap();
    app.run("layer.select", json!({"layer": first.0})).unwrap();
    open(&mut app).unwrap();
    ui(&mut app, json!({"output": "layerMask"}));
    let revision = app.session.active().unwrap().revision;
    let blocked = call(&mut app, "engine.execute", json!({"command": "layer.select", "params": {"layer": other.0}}));
    assert_eq!(blocked["ok"], false, "external edits respect the modal gate");
    app.session.execute("layer.select", json!({"layer":other.0})).unwrap();
    let revision_after_select = app.session.active().unwrap().revision;
    assert!(try_ui(&mut app, json!({"commit": true})).is_err(), "the stale OK is refused");
    let st = app.session.active().unwrap();
    assert!(st.doc.layer(other).unwrap().mask.is_none(), "no mask on the newly selected layer");
    assert!(st.doc.layer(first).unwrap().mask.is_none(), "nor on the first");
    assert!(st.doc.selection.is_some(), "the selection stands");
    assert!(revision_after_select >= revision && st.revision == revision_after_select, "nothing was edited after the switch");
    assert!(!app.dialog_open());
}

/// A stale workspace is closed with a reason that is shown in the status bar and returned to the
/// control channel (#8896).
#[test]
fn the_reason_a_stale_workspace_closed_is_reported() {
    let document = "Select and Mask was closed: its document is no longer the active one";
    let layer = "Select and Mask was closed: another layer was selected";
    for (reason, english) in [("document", document), ("layer", layer)] {
        let mut app = if reason == "document" { two_documents() } else { app_with_selection() };
        let first = app.session.active().unwrap().active_layer.unwrap();
        let other = (reason == "layer").then(|| app.run("layer.new.layer", json!({"name": "Other"})).unwrap()["layer"].as_u64().map(LayerId).unwrap());
        app.run("layer.select", json!({"layer": first.0})).unwrap();
        open(&mut app).unwrap();
        match other {
            Some(other) => drop(app.session.execute("layer.select", json!({"layer": other.0}))),
            None => assert!(app.session.set_active(1)),
        }
        let err = try_ui(&mut app, json!({"commit": true})).expect_err(&format!("{reason}: the stale OK is refused"));
        assert_eq!(err, english, "{reason}: the error");
        assert_eq!(app.ui.status, english, "{reason}: the status");
    }
}

/// Selected pixels are counted tile by tile, so a sparse selection on a huge canvas costs nothing.
#[test]
fn sparse_large_preview_counts_its_selected_pixels() {
    let area = Rect::new(0, 0, 100_000, 100_000);
    let mut selection = Surface::new(photocraft_color::PixelFormat::GRAY8);
    selection.write_pixel(99_999, 99_999, &[1.0]);
    selection.write_pixel(2, 3, &[0.5]);
    selection.write_pixel(-1, -1, &[1.0]);
    assert_eq!(selected_pixels(Some(&selection), area), 2);
    let small = Rect::new(0, 0, 10, 10);
    assert_eq!(selected_pixels(Some(&selection), small), 1);

    let mut inverted = Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[1.0]);
    inverted.write_pixel(2, 3, &[0.0]);
    let smaller = Rect::new(0, 0, 1000, 1000);
    assert_eq!(selected_pixels(Some(&inverted), smaller), 999_999);
    assert_eq!(selected_pixels(None, area), 0);
}

/// Until the preview at the new scale arrives, the ants keep the reduction of the preview they outline.
#[test]
fn the_outline_keeps_its_previews_scale_while_high_quality_is_toggled() {
    let mut app = app_sized(3300, 60);
    app.session.execute("select.rect", json!({"x": 30, "y": 15, "width": 30, "height": 30})).unwrap();
    open(&mut app).unwrap();
    ui(&mut app, json!({"view": "marchingAnts", "waitPreview": true}));
    // PLAN: assumed (dialogs-w41 readers) `ants_k()` is the reduction of the ants shown, 0 for none
    assert_eq!(src(&app).ants_k(), 3);
    ui(&mut app, json!({"highQualityPreview": true}));
    assert_eq!(src(&app).ants_k(), 3, "the new scale shows once its preview lands");
    ui(&mut app, json!({"waitPreview": true}));
    assert_eq!(src(&app).ants_k(), 1);
}

/// A document edit while the workspace is open drops the old preview's ants with its image.
#[test]
fn a_document_edit_clears_the_old_ants() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"view": "marchingAnts", "waitPreview": true}));
    assert_ne!(src(&app).ants_k(), 0, "the preview has ants");
    app.session
        .edit("paint", |doc, _| {
            doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(0, 0, 4, 4), &[0.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    follow(&mut app);
    assert_eq!(src(&app).ants_k(), 0);
}
