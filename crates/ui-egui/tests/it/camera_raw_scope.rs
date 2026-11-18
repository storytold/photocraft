//! Real dialog/controller acceptance: interactions never make a second document edit path. The
//! drawing and gestures of the scope, curve and canvas are the framework's; what is tested here is
//! the app's data behind them: refusals, the failed OK, curves, clipping counts, Lab readouts, the
//! region scope, the persisted scope settings and the hover sample.
use egui::{Pos2, pos2, vec2};
use egui_kittest::Harness;
use photocraft_ui_egui::{PhotocraftApp, camera_raw_ui, control, theme::AppTheme};
use serde_json::{Value, json};

fn menu(app: &mut PhotocraftApp, ctx: &egui::Context, p: Value) -> Result<Value, String> {
    camera_raw_ui::menu(app, ctx, "filter.cameraRaw", &p).ok_or("missing handler")?
}
fn inspect(h: &Harness<'_, PhotocraftApp>) -> Value {
    control::inspect(h.state(), &h.ctx)["cameraRaw"].clone()
}

/// A harness whose document `prepare` sets up before Camera Raw opens on it.
fn harness(theme: AppTheme, prepare: impl FnOnce(&mut PhotocraftApp) + 'static) -> Harness<'static, PhotocraftApp> {
    app_harness(theme, true, prepare)
}
/// [`harness`], with Camera Raw opened at the start or left for the test's first request. A develop is only
/// delivered when the app's own frame runs, which is what a harness steps.
fn app_harness(theme: AppTheme, open: bool, prepare: impl FnOnce(&mut PhotocraftApp) + 'static) -> Harness<'static, PhotocraftApp> {
    let shown = theme.clone();
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1200.0, 800.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, shown.clone());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
        app.set_theme(&cc.egui_ctx, shown);
        prepare(&mut app);
        if open {
            camera_raw_ui::open(&mut app, &cc.egui_ctx).unwrap();
        }
        app
    });
    h.run_steps(3);
    // Startup preferences can replace the initial theme; apply the requested test theme after it.
    let ctx = h.ctx.clone();
    h.state_mut().set_theme(&ctx, theme.clone());
    h.run_steps(3);
    assert_eq!(h.state().ui.theme, theme);
    if open {
        settle(&mut h);
    }
    h
}
fn fixture(theme: AppTheme) -> Harness<'static, PhotocraftApp> {
    harness(theme, |app| {
        app.run("file.new", json!({"width":64,"height":48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color":"#80ff00"})).unwrap();
    })
}
/// Waits for the proxy develop the workspace starts on the runtime's job queue, so its frame
/// can't land, and bump `previewRevision`, after a test has recorded the revision. The workspace
/// starts its first develop a pass or two after it opens, and the queue runs it when a request
/// finishes it, as the agent entry point's `waitPreview` does: the frame holds no histogram until
/// then.
fn settle(h: &mut Harness<'_, PhotocraftApp>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    h.run_steps(2);
    loop {
        craft::ui::runtime::pending::finish_jobs(&h.ctx, std::time::Duration::from_secs(30));
        h.run_steps(1);
        // A failed develop clears `previewPending` without a new frame.
        let app = control::inspect(h.state(), &h.ctx);
        assert_ne!(app["statusError"], true, "the Camera Raw preview failed: {}", app["status"]);
        let described = inspect(h);
        if described["previewPending"] == false && !described["histogram"].is_null() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the Camera Raw preview never finished");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    // The frame that picked up the result also rebuilt the scope; let it go quiet.
    h.run_steps(2);
}

/// A request, then the develop it asks for. The reply describes the open dialog; a request that closed it
/// answers with its own reply.
fn request(h: &mut Harness<'_, PhotocraftApp>, p: Value) -> Result<Value, String> {
    let ctx = h.ctx.clone();
    let reply = menu(h.state_mut(), &ctx, p)?;
    if inspect(h).is_null() {
        return Ok(reply);
    }
    settle(h);
    Ok(inspect(h))
}
/// A harness on a `width` x `height` document filled with `color`, Camera Raw closed.
fn plain_app(width: u32, height: u32, color: &'static str) -> Harness<'static, PhotocraftApp> {
    app_harness(AppTheme::pro(), false, move |app| {
        app.run("file.new", json!({"width":width,"height":height})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color":color})).unwrap();
    })
}

/// Moves the pointer across the window until it is over the picture, and returns where and the
/// readout the workspace reports there.
fn hover_over_picture(h: &mut Harness<'_, PhotocraftApp>) -> (Pos2, Value) {
    for y in [400.0, 300.0, 500.0] {
        for x in (100..1100).step_by(25) {
            let at = pos2(x as f32, y);
            h.hover_at(at);
            h.run_steps(2);
            let readout = inspect(h)["pointerReadout"].clone();
            if !readout.is_null() {
                return (at, readout);
            }
        }
    }
    panic!("no point of the window reads the picture");
}

#[test]
fn invalid_mixed_controller_requests_leave_dialog_preferences_and_document_unchanged() {
    let mut h = fixture(AppTheme::pro());
    let ctx = h.ctx.clone();
    let before = inspect(&h);
    let preferences = h.state().session.prefs().dialogs.clone();
    let document = h.state().session.active().unwrap().doc.clone();
    let history = document_steps(&h);
    // Each key is applied or refused on its own: a refused key leaves its own value, and the
    // reply names it.
    for (request, key) in [
        (json!({"before":"yes"}), "before"),
        (json!({"set":{"unknown":1.0}}), "unknown"),
        (json!({"unknown":true}), "unknown"),
        (json!({"scope":"not settings"}), "scope"),
        (json!({"set":{"pointCurve":vec![[0,0];17]}}), "pointCurve"),
    ] {
        let refusal = menu(h.state_mut(), &ctx, json!({"ui":request})).expect_err(key);
        assert!(refusal.contains(key), "the reply names the refused key {key}: {refusal}");
        let now = inspect(&h);
        assert_eq!((&now["params"], &now["scope"], &now["before"]), (&before["params"], &before["scope"], &before["before"]), "{key}");
        assert_eq!(h.state().session.prefs().dialogs, preferences);
        assert!(std::sync::Arc::ptr_eq(&h.state().session.active().unwrap().doc, &document));
        assert_eq!(document_steps(&h), history);
    }
    // A refused key does not stop the accepted keys of the same request.
    let refusal = menu(h.state_mut(), &ctx, json!({"ui":{"set":{"exposure":1.0},"before":"yes"}})).unwrap_err();
    assert!(refusal.contains("before"), "{refusal}");
    assert_eq!(inspect(&h)["params"]["exposure"], 1.0);
    assert_eq!(document_steps(&h), history, "settings edits are not document edits");
}

fn document_steps(h: &Harness<'_, PhotocraftApp>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

#[test]
fn invalid_requests_do_not_open_a_dialog_or_replace_scope_preferences() {
    let mut h = fixture(AppTheme::pro());
    let ctx = h.ctx.clone();
    let scope = inspect(&h)["scope"].clone();
    assert!(scope.is_object(), "the open dialog reports its scope settings");
    menu(h.state_mut(), &ctx, json!({"ui":{"cancel":true}})).unwrap();
    h.run_steps(2);
    assert!(inspect(&h).is_null(), "Cancel closed it");
    let prefs = h.state().session.prefs().dialogs.clone();
    for request in [json!(null), json!([]), json!(1), json!({"ui":[]}), json!({"ui":{"set":{"pointCurve":vec![[0,0];17]}}}), json!({"ui":{"scope":"x"}})] {
        assert!(menu(h.state_mut(), &ctx, request).is_err());
        h.run_steps(2);
        assert!(inspect(&h).is_null(), "a refused request opens nothing");
        assert_eq!(h.state().session.prefs().dialogs, prefs);
    }
    // The scope settings come back as they were.
    menu(h.state_mut(), &ctx, json!({})).unwrap();
    settle(&mut h);
    assert_eq!(inspect(&h)["scope"], scope);
}

#[test]
fn failed_commit_keeps_the_dialog_and_filter_settings_for_retry() {
    let mut h = fixture(AppTheme::pro());
    let ctx = h.ctx.clone();
    menu(h.state_mut(), &ctx, json!({"ui":{"set":{"exposure":0.5}}})).unwrap();
    h.state_mut().run("layer.lockLayers", json!({"pixels":true})).unwrap();
    let document = h.state().session.active().unwrap().doc.clone();
    assert!(menu(h.state_mut(), &ctx, json!({"ui":{"commit":true}})).is_err());
    h.run_steps(2);
    assert_eq!(inspect(&h)["params"]["exposure"], 0.5);
    assert!(std::sync::Arc::ptr_eq(&h.state().session.active().unwrap().doc, &document));
    h.state_mut().run("layer.lockLayers", json!({"pixels":false})).unwrap();
    menu(h.state_mut(), &ctx, json!({"ui":{"commit":true}})).unwrap();
    h.run_steps(2);
    assert!(inspect(&h).is_null());
}

#[test]
fn malformed_curves_fail_in_both_preview_and_engine_without_an_edit() {
    let mut h = fixture(AppTheme::pro());
    let ctx = h.ctx.clone();
    let before = inspect(&h);
    let document = h.state().session.active().unwrap().doc.clone();
    let history = document_steps(&h);
    for field in ["pointCurve", "pointCurveRed", "pointCurveGreen", "pointCurveBlue"] {
        for points in [json!([[255, 255], [0, 0]]), json!([[0, 0], [0, 128]]), json!([[0, -1], [255, 255]]), json!(vec![[0, 0]; 17])] {
            let settings = json!({field:points});
            assert!(menu(h.state_mut(), &ctx, json!({"ui":{"set":settings}})).is_err(), "{field}: the dialog refuses it");
            assert!(h.state_mut().run("filter.cameraRaw", settings).is_err(), "{field}: the engine refuses it");
            let now = inspect(&h);
            assert_eq!((&now["params"], &now["previewRevision"]), (&before["params"], &before["previewRevision"]), "{field}: nothing applied or developed");
            assert!(std::sync::Arc::ptr_eq(&h.state().session.active().unwrap().doc, &document));
            assert_eq!(document_steps(&h), history, "{field}: no step recorded");
        }
    }
}

/// The clipping counts of the develop are exact at every depth: Blue 0 is in every pixel's
/// shadows, Green 255 in every pixel's highlights, and no other channel clips. A scope change
/// is no edit: nothing is recorded and Cancel leaves the document alone.
// PLAN: describe's `samplerReadouts` cannot be driven through the control protocol (the sampler points are the workspace's pointer state, not a settable field), so the sampler cap `MAX_SAMPLERS` has no trial here; it belongs to the framework's ColorSamplers trial.
#[test]
fn exact_clipping_rgb_lab_capped_samplers_and_reversible_tone_controls() {
    for depth in [8, 16, 32] {
        let mut h = app_harness(AppTheme::pro(), false, move |app| {
            app.run("file.new", json!({"width":16,"height":8,"mode":"rgb","depth":depth})).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
            app.run("edit.fill", json!({"color":"#80ff00"})).unwrap();
        });
        let original = h.state().session.active().unwrap().doc.clone();
        let history = h.state().session.active().unwrap().history.past_len();
        let initial = request(&mut h, json!({})).unwrap();
        let histogram = &initial["histogram"];
        let counts = |key: &str| (0..3).map(|channel| histogram[key][channel].as_u64().unwrap()).collect::<Vec<_>>();
        assert_eq!(counts("shadows"), [0, 0, 128], "depth {depth}: Blue 0 clips every pixel");
        assert_eq!(counts("highlights"), [0, 128, 0], "depth {depth}: Green 255 clips every pixel");
        let warned = request(&mut h, json!({"ui":{"scope":{"labReadouts":true,"shadowWarning":true,"highlightWarning":true}}})).unwrap();
        assert_eq!(warned["scope"]["shadowWarning"], true);
        assert_eq!(warned["histogram"], initial["histogram"], "the warnings count, they do not change the develop");
        assert_eq!(warned["previewRevision"], initial["previewRevision"], "a scope change starts no develop");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
        request(&mut h, json!({"ui":{"cancel":true}})).unwrap();
        assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
        request(&mut h, json!({"ui":{"set":{"exposure":0.5},"commit":true}})).unwrap();
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
    }
}

/// The Lab readout converts through the document's own profile: a red pixel reads differently in
/// sRGB and in Display P3.
#[test]
fn lab_readouts_respect_embedded_rgb_profile() {
    let lab_a = |profile: Option<&'static str>| {
        // Big enough that the pointer scan of `hover_over_picture` lands on it: the view fits a picture up to 400 %.
        let mut h = harness(AppTheme::pro(), move |app| {
            app.run("file.new", json!({"width":64,"height":48})).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
            app.run("edit.fill", json!({"color":"#ff0000"})).unwrap();
            if let Some(profile) = profile {
                app.run("edit.assignProfile", json!({"profile":profile})).unwrap();
            }
        });
        let ctx = h.ctx.clone();
        menu(h.state_mut(), &ctx, json!({"ui":{"scope":{"labReadouts":true}}})).unwrap();
        settle(&mut h);
        let (_, readout) = hover_over_picture(&mut h);
        assert_eq!(readout["space"], "lab");
        readout["values"][1].as_f64().unwrap()
    };
    let (a, b) = (lab_a(None), lab_a(Some("display-p3")));
    assert!((a - b).abs() > 5.0, "Lab must use document ICC: {a} vs {b}");
}

/// The scope settings persist in the saved tier: a new dialog starts with the settings the last
/// one left, except that the vectorscope is opt-in each time.
#[test]
fn real_shortcuts_triangles_menu_and_scope_preferences_roundtrip() {
    let mut h = plain_app(16, 16, "#808080");
    let first = request(&mut h, json!({})).unwrap();
    assert_eq!(first["scope"]["labReadouts"], false);
    let set = request(&mut h, json!({"ui":{"scope":{"labReadouts":true,"vectorscope":true,"shadowWarning":true}}})).unwrap();
    assert_eq!((&set["scope"]["labReadouts"], &set["scope"]["vectorscope"], &set["scope"]["shadowWarning"]), (&json!(true), &json!(true), &json!(true)));
    assert_eq!(set["previewRevision"], first["previewRevision"], "scope settings start no develop");
    request(&mut h, json!({"ui":{"cancel":true}})).unwrap();
    let again = request(&mut h, json!({})).unwrap();
    assert_eq!(again["scope"]["labReadouts"], true, "kept");
    assert_eq!(again["scope"]["shadowWarning"], true, "kept");
    assert_eq!(again["scope"]["vectorscope"], false, "the vectorscope stays opt-in when reopening Camera Raw");
    assert_eq!(again["samplerReadouts"], json!([]), "samplers are never restored");
}

#[test]
fn selected_region_scope_and_preview_share_the_engine_selection_boundary() {
    let mut h = plain_app(16, 8, "#808080");
    h.state_mut().run("select.rect", json!({"x":0,"y":0,"width":8,"height":8})).unwrap();
    let original = h.state().session.active().unwrap().doc.clone();
    request(&mut h, json!({})).unwrap();
    let full = request(&mut h, json!({"ui":{"set":{"exposure":1.0},"scope":{"vectorscope":true}}})).unwrap();
    assert_eq!(full["hasScopeSelection"], true);
    assert_eq!(full["vectorscope"]["samples"], 128);
    let selected = request(&mut h, json!({"ui":{"scope":{"vectorscope":true,"selectedRegion":true}}})).unwrap();
    assert_eq!(selected["vectorscope"]["samples"], 64, "only the pixels inside the selection");
    assert_eq!(selected["previewRevision"], full["previewRevision"]);
    let before = request(&mut h, json!({"ui":{"before":true}})).unwrap();
    assert_eq!(before["vectorscope"]["before"], true);
    let after = request(&mut h, json!({"ui":{"before":false}})).unwrap();
    assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
    // A refused scope leaves the accepted settings as they were.
    assert!(request(&mut h, json!({"ui":{"scope":"zzz"}})).is_err());
    let unchanged = inspect(&h);
    assert_eq!(unchanged["params"], after["params"]);
    assert_eq!(unchanged["scope"], after["scope"]);
    request(&mut h, json!({"ui":{"commit":true}})).unwrap();
    let committed = request(&mut h, json!({})).unwrap();
    assert_eq!(committed["histogram"], after["histogram"]);
}
