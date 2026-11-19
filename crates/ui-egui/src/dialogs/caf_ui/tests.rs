use craft::imaging::ui::fill::FillOp;
use craft::ui::canvas::PixelRect;
use photocraft_doc::LayerId;
use serde_json::{Value, json};

use super::preview::{compute_preview, plan_params};
use super::*;
use crate::control::dialogs::workspace_ui;

/// A textured layer (a checker of two colours over a gradient) with a 20 px square selected.
fn app_with_selection() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 200, "height": 160, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            let s = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            for y in 0..160 {
                for x in 0..200 {
                    let v = ((x / 6 + y / 6) % 2) as f32 * 0.5 + x as f32 / 400.0;
                    s.write_pixel(x, y, &[v, 0.3, 1.0 - v, 1.0]);
                }
            }
            // A red blemish to remove.
            for y in 72..88 {
                for x in 92..108 {
                    s.write_pixel(x, y, &[1.0, 0.0, 0.0, 1.0]);
                }
            }
            Ok(())
        })
        .unwrap();
    app.session.execute("select.rect", json!({"x": 90, "y": 70, "width": 20, "height": 20})).unwrap();
    app.sync_views();
    app
}

fn px(app: &PhotocraftApp, layer: LayerId, x: i32, y: i32) -> Vec<f32> {
    app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().pixel(x, y)
}

/// The app's own context, which the dialog host and the remembered values run on.
fn ctx(app: &PhotocraftApp) -> egui::Context {
    app.ctx().clone()
}

/// One pass of the host with the app's taker, as a frame runs them.
fn pass(app: &mut PhotocraftApp) {
    let ctx = ctx(app);
    crate::canvas::testing::frame(app);
    take(app, &ctx);
}

/// A `{ui: …}` request to the open workspace, then the passes that report what it queued.
fn ui(app: &mut PhotocraftApp, request: Value) -> Result<Value, String> {
    let ctx = ctx(app);
    let r = workspace_ui(app, &ctx, CMD, &request);
    pass(app);
    pass(app);
    r
}

fn facts(app: &PhotocraftApp) -> Value {
    describe(app).expect("the workspace is open")
}

fn sampled(app: &PhotocraftApp) -> u64 {
    facts(app)["sampledProxyPixels"].as_u64().unwrap()
}

/// Runs passes until `done` holds of the facts (the preview job runs on another thread).
fn settle(app: &mut PhotocraftApp, done: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..600 {
        pass(app);
        // The preview job waits on the framework's queue until a request finishes it, as `waitPreview` does.
        craft::ui::runtime::pending::finish_jobs(&ctx(app), std::time::Duration::from_secs(30));
        pass(app);
        let d = facts(app);
        if done(&d) {
            return d;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the workspace did not settle: {}", facts(app));
}

fn close_all(app: &mut PhotocraftApp) {
    for _ in 0..20 {
        if app.caf.is_none() {
            return;
        }
        pass(app);
    }
}

fn hosted(app: &mut PhotocraftApp) -> &mut Hosted {
    let id = app.caf.as_ref().unwrap().id;
    app.framework.dialogs.get_mut_by_id::<Hosted>(id).unwrap()
}

#[test]
fn brush_strokes_change_the_sampling_area_and_undo() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    pass(&mut app);
    ui(&mut app, json!({"sampling": "rectangular"})).unwrap();
    let all = sampled(&app);
    assert_eq!(all, 200 * 160 - 400, "Rectangular: the whole window but the fill area");
    // Subtracting (the default): a 10 px stroke takes about 10 × 60 px away.
    ui(&mut app, json!({"stroke": {"points": [[20.0, 20.0], [80.0, 20.0]], "size": 10}})).unwrap();
    let less = sampled(&app);
    assert!((all - less).abs_diff(10 * 60 + 79) < 120, "{all} → {less}");
    // Undo brings it back; redo takes it again.
    ui(&mut app, json!({"undo": true})).unwrap();
    assert_eq!(sampled(&app), all);
    ui(&mut app, json!({"redo": true})).unwrap();
    assert_eq!(sampled(&app), less);
    // Custom starts empty and the brush adds; Alt reverses it.
    ui(&mut app, json!({"sampling": "custom"})).unwrap();
    let d = facts(&app);
    assert_eq!((d["sampledProxyPixels"].as_u64(), d["brushMode"].as_str(), d["strokes"].as_u64()), (Some(0), Some("add"), Some(0)));
    ui(&mut app, json!({"stroke": {"points": [[20.0, 120.0], [60.0, 120.0]], "size": 8}})).unwrap();
    assert!(sampled(&app) > 300);
    ui(&mut app, json!({"stroke": {"points": [[20.0, 120.0], [60.0, 120.0]], "size": 8, "alt": true}})).unwrap();
    assert_eq!(sampled(&app), 0);
    // Undoing the change of option restores Rectangular with its stroke.
    for _ in 0..3 {
        ui(&mut app, json!({"undo": true})).unwrap();
    }
    let d = facts(&app);
    assert_eq!((d["settings"]["sampling"].as_str(), d["strokes"].as_u64(), d["brushMode"].as_str()), (Some("rectangular"), Some(1), Some("subtract")));
}

#[test]
fn the_lasso_edits_the_fill_area_and_the_window_follows() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"lasso": {"op": "add", "points": [[130.0, 70.0], [150.0, 70.0], [150.0, 90.0], [130.0, 90.0]]}})).unwrap();
    assert_eq!(facts(&app)["fillArea"], json!(800));
    ui(&mut app, json!({"expand": 2})).unwrap();
    assert!(facts(&app)["fillArea"].as_u64().unwrap() > 900);
    ui(&mut app, json!({"undo": true, "lassoMode": "subtract"})).unwrap();
    let d = facts(&app);
    assert_eq!((d["fillArea"].as_u64(), d["lassoMode"].as_str()), (Some(800), Some("subtract")));
    // Subtracting everything is refused: the fill area can't be empty.
    ui(&mut app, json!({"lasso": {"op": "subtract", "points": [[0.0, 0.0], [200.0, 0.0], [200.0, 160.0], [0.0, 160.0]]}})).unwrap();
    let d = facts(&app);
    assert_eq!(d["fillArea"], json!(800));
    assert!(d["error"].as_str().is_some_and(|e| e.contains("empty")), "{d}");
    // The reset button goes back to the selection.
    ui(&mut app, json!({"reset": "fillArea"})).unwrap();
    assert_eq!(facts(&app)["fillArea"], json!(400));
}

/// A sampling area the Sampling Brush erased entirely previews as an error naming the
/// problem, not as the black fill it would make; undoing the stroke brings the preview back.
#[test]
fn an_erased_sampling_area_previews_as_an_error() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"sampling": "rectangular"})).unwrap();
    settle(&mut app, |d| d["previewReady"] == json!(true) && d["previewError"].is_null());
    // Subtracting (the default) with a brush wider than the 200 × 160 window.
    ui(&mut app, json!({"stroke": {"points": [[100.0, 80.0]], "size": 600}})).unwrap();
    let d = settle(&mut app, |d| !d["previewError"].is_null());
    assert_eq!(d["sampledProxyPixels"].as_u64(), Some(0), "{d}");
    assert!(d["previewError"].as_str().is_some_and(|e| e.contains("nothing to sample from")), "{d}");
    let runs = d["previewRuns"].as_u64().unwrap();
    ui(&mut app, json!({"undo": true})).unwrap();
    let d = settle(&mut app, |d| d["previewError"].is_null() && d["previewRuns"].as_u64().unwrap() > runs);
    assert!(d["previewError"].is_null(), "{d}");
}

/// A fill the engine refuses reaches the workspace as the held OK's `Err`: the engine's reason shows, no history
/// step is added, and once the lock is lifted OK fills once and closes.
#[test]
fn a_failed_ok_keeps_the_workspace_open() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    app.session.execute("layer.setProps", json!({"locks": {"pixels": true}})).unwrap();
    let steps = app.session.active().unwrap().history.past_len().max(steps);
    let refusal = ui(&mut app, json!({"commit": true})).unwrap_err();
    assert!(refusal.contains("locked"), "the refused OK replies with the engine's reason: {refusal}");
    assert!(app.caf.is_some(), "OK keeps the workspace open when the fill fails");
    assert!(facts(&app)["error"].as_str().is_some_and(|e| e.contains("locked")), "{}", facts(&app));
    assert!(app.ui.status_error);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "a refused fill adds no step");
    app.session.execute("layer.setProps", json!({"locks": {"pixels": false}})).unwrap();
    let steps = app.session.active().unwrap().history.past_len().max(steps);
    ui(&mut app, json!({"commit": true})).unwrap();
    close_all(&mut app);
    assert!(app.caf.is_none(), "OK closes once the fill runs");
    assert!(app.session.active().unwrap().history.past_len() > steps, "and fills");
}

/// Undo or Redo of a sampling stroke changes what OK would fill, so it clears the error a failed OK left, as a new
/// stroke does; otherwise the stale error would stay drawn over the recovered preview.
#[test]
fn undo_and_redo_of_a_stroke_clear_a_failed_oks_error() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"stroke": {"points": [[20.0, 20.0], [60.0, 20.0]], "size": 10}})).unwrap();
    app.session.execute("layer.setProps", json!({"locks": {"pixels": true}})).unwrap();
    assert!(ui(&mut app, json!({"commit": true})).is_err(), "the engine refuses the locked layer");
    assert!(facts(&app)["error"].is_string(), "the failed OK reports in the panel");
    ui(&mut app, json!({"undo": true})).unwrap();
    assert!(facts(&app)["error"].is_null(), "Undo of the stroke clears it");
    assert!(ui(&mut app, json!({"commit": true})).is_err(), "the engine refuses the locked layer again");
    assert!(facts(&app)["error"].is_string());
    ui(&mut app, json!({"redo": true})).unwrap();
    assert!(facts(&app)["error"].is_null(), "Redo of the stroke clears it too");
}

/// The preview picture's pixel at document point `(x, y)`.
fn preview_pixel(app: &PhotocraftApp, x: i32, y: i32) -> egui::Color32 {
    let mut out = egui::ColorImage::new([1, 1], vec![egui::Color32::TRANSPARENT]);
    app.caf.as_ref().unwrap().picture.read(PixelRect::new(x, y, x + 1, y + 1), 1, &mut out);
    out.pixels[0]
}

#[test]
fn apply_fills_and_stays_open_ok_closes_cancel_keeps_what_was_applied() {
    let mut app = app_with_selection();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    open(&mut app).unwrap();
    // The preview is what Apply writes.
    let before = facts(&app)["previewRevision"].as_u64().unwrap();
    ui(&mut app, json!({"output": "current", "colorAdaptation": "none"})).unwrap();
    settle(&mut app, |d| d["previewRevision"].as_u64().unwrap() > before && d["previewError"].is_null());
    let shown = preview_pixel(&app, 100, 80);
    ui(&mut app, json!({"apply": true})).unwrap();
    assert!(app.caf.is_some(), "Apply keeps the workspace open");
    let after = px(&app, layer, 100, 80);
    assert!(after[0] < 0.9 || after[1] > 0.1, "the blemish is filled: {after:?}");
    assert!((f32::from(shown.r()) / 255.0 - after[0]).abs() < 0.01 && (f32::from(shown.b()) / 255.0 - after[2]).abs() < 0.01, "preview {shown:?} vs {after:?}");
    let steps = app.session.active().unwrap().history.past_len();
    ui(&mut app, json!({"cancel": true})).unwrap();
    close_all(&mut app);
    assert!(app.caf.is_none() && !app.dialog_open());
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "Cancel keeps the applied fill");
    // OK with New Layer: a new layer holds the fill; the original stays.
    app.session.execute("edit.undo", json!({})).unwrap();
    open(&mut app).unwrap();
    ui(&mut app, json!({"output": "new", "commit": true})).unwrap();
    close_all(&mut app);
    assert!(app.caf.is_none());
    assert_eq!(px(&app, layer, 100, 80)[0], 1.0, "the original layer keeps its blemish");
    let top = app.session.active().unwrap().active_layer.unwrap();
    assert_ne!(top, layer);
    assert!(px(&app, top, 100, 80)[3] > 0.99, "the new layer holds the fill");
}

/// The engine's sampling area over the plan's rectangle (what the fill samples) and its fill area.
fn engine_source(app: &PhotocraftApp) -> (Rect, Vec<bool>, Vec<bool>) {
    let c = app.caf.as_ref().unwrap();
    let id = c.id;
    let command = app.framework.dialogs.get_by_id::<Hosted>(id).unwrap().workspace().command();
    let plan = photocraft_engine::edit_menu_cmds::content_aware_plan(&c.doc, c.layer, &plan_params(&command, c.layer), CMD).unwrap();
    (plan.rect, plan.source, plan.hole)
}

/// Each stroke is painted into the proxy once, over its own pixels, and the overlay is exactly what the engine
/// samples (at one proxy px per document px). A second `take` in the same frame applies nothing again.
#[test]
fn strokes_are_painted_once_and_the_overlay_is_the_engines_sampling_area() {
    use craft::imaging::ui::fill::SamplingStroke as Stroke;
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"sampling": "rectangular"})).unwrap();
    let start = std::time::Instant::now();
    for i in 0..400u32 {
        // Lines across the document, most subtracting, every fifth adding.
        let y = 2.0 + f64::from(i % 40) * 4.0;
        let pts: Vec<[f64; 2]> = (0..50).map(|j| [f64::from(j) * 4.0 + f64::from(i % 3), y + f64::from(j % 2)]).collect();
        let mode = if i % 5 == 0 { EditMode::Add } else { EditMode::Subtract };
        hosted(&mut app).workspace_mut().perform(FillOp::Stroke(Stroke::new(mode, f64::from(6 + (i % 4) * 3), pts))).unwrap();
    }
    pass(&mut app);
    let painted = start.elapsed();
    let once = app.caf.as_ref().unwrap().mask.clone();
    let ctx = ctx(&app);
    take(&mut app, &ctx);
    assert!(app.caf.as_ref().unwrap().mask == once, "a second pass of the frame applies nothing again");
    assert!(painted.as_secs_f64() < 20.0, "400 strokes {painted:?}");
    let c = app.caf.as_ref().unwrap();
    assert_eq!((c.k, c.known.strokes.len()), (1, 400));
    let (rect, source, hole) = engine_source(&app);
    let c = app.caf.as_ref().unwrap();
    assert_eq!(rect, c.canvas, "Rectangular's window is the whole 200 × 160 canvas");
    let differ = (0..source.len()).filter(|&i| (c.mask[i] && !c.hole_px[i]) != (source[i] && !hole[i])).count();
    assert_eq!(differ, 0, "proxy pixels that differ from the engine's sampling area");
    assert!(sampled(&app) > 1000);
}

/// `edit.contentAwareFill {"ui": …}` addresses the workspace: with it closed that is an error, not a fill with the
/// default settings (an agent polling the state after an Esc used to add a fill per poll).
#[test]
fn ui_params_with_the_workspace_closed_are_an_error_not_a_fill() {
    let mut app = app_with_selection();
    let ctx = ctx(&app);
    let steps = app.session.active().unwrap().history.past_len();
    for request in [json!({}), json!({"cancel": true}), json!({"sampling": "rectangular"})] {
        let r = workspace_ui(&mut app, &ctx, CMD, &request);
        assert!(r.as_ref().is_err_and(|e| e.contains("not open")), "{r:?}");
    }
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "nothing was filled");
    assert!(!app.dialog_open() && app.caf.is_none());
}

/// The preview texture fits the device: a plan rectangle wider than the texture limit is box-averaged to fit, and
/// is still drawn over the plan's rectangle.
#[test]
fn the_preview_texture_respects_the_device_limit() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 2200, "height": 40, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            let s = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            s.fill_rect(Rect::new(0, 0, 2200, 40), &[0.2, 0.6, 0.4, 1.0]);
            s.fill_rect(Rect::new(1090, 10, 1110, 30), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.session.execute("select.rect", json!({"x": 1090, "y": 10, "width": 20, "height": 20})).unwrap();
    app.sync_views();
    open(&mut app).unwrap();
    // Custom sampling with a stroke across the whole width: the plan's rectangle is 2200 px wide.
    {
        use craft::imaging::ui::fill::{SamplingMode, SamplingStroke as Stroke};
        let ws = hosted(&mut app).workspace_mut();
        ws.perform(FillOp::Sampling(SamplingMode::Custom)).unwrap();
        ws.perform(FillOp::Stroke(Stroke::new(EditMode::Add, 8.0, vec![[0.0, 5.0], [2200.0, 5.0]]))).unwrap();
    }
    let c = app.caf.as_ref().unwrap();
    let command = app.framework.dialogs.get_by_id::<Hosted>(c.id).unwrap().workspace().command();
    let params = plan_params(&command, c.layer);
    for (limit, want) in [(2048usize, 1100usize), (4096, 2200)] {
        let (rect, img, factor) = compute_preview(&c.doc, c.layer, &params, &photocraft_raster::Interrupt::NONE, limit).unwrap();
        assert_eq!((rect.width(), img.size[0]), (2200, want), "limit {limit}");
        assert_eq!(factor as usize, 2200 / want);
        assert!(img.size.iter().all(|s| *s <= limit), "limit {limit}: {:?}", img.size);
        let (sum, n) = img.pixels.iter().fold((0u32, 0u32), |(s, n), p| (s + u32::from(p.g()), n + 1));
        assert!(sum / n > 100, "limit {limit}: the reduced preview keeps the image ({})", sum / n);
    }
}

/// A preview that fails stands for that request: the error is reported and the same preview is not started
/// again on every pass.
#[test]
fn a_failing_preview_is_reported_once_not_restarted() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    settle(&mut app, |d| d["previewReady"] == json!(true));
    // The selection goes away under the workspace: there is nothing to fill.
    app.session.execute("select.deselect", json!({})).unwrap();
    // The replan refuses the empty selection before a new preview request exists, and says so as the panel's error.
    let d = settle(&mut app, |d| d["error"].as_str().is_some_and(|e| e.contains("selection")));
    let runs = d["previewRuns"].as_u64().unwrap();
    for _ in 0..40 {
        pass(&mut app);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(facts(&app)["previewRuns"].as_u64(), Some(runs), "the failed request is not started again");
}

/// The source's image is the document, not the layer alone: after Apply to a new layer the fill (on that layer)
/// covers the blemish.
#[test]
fn the_view_shows_the_document_after_an_apply_to_a_new_layer() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    let red = |app: &PhotocraftApp| {
        let mut out = egui::ColorImage::new([1, 1], vec![egui::Color32::TRANSPARENT]);
        let source = app.framework.dialogs.get_by_id::<Hosted>(app.caf.as_ref().unwrap().id).unwrap().source().clone();
        source.image().read(PixelRect::new(100, 80, 101, 81), 1, &mut out);
        let p = out.pixels[0];
        p.r() > 200 && p.g() < 60
    };
    assert!(red(&app), "the blemish shows before the fill");
    ui(&mut app, json!({"output": "new", "colorAdaptation": "none", "apply": true})).unwrap();
    for _ in 0..5 {
        pass(&mut app);
    }
    assert!(!red(&app), "the fill on the new layer shows");
}

/// The overlay colour swatch opens a colour picker child over the workspace, and the picker's OK reaches the
/// workspace without losing its strokes.
#[test]
fn keys_wait_while_the_overlay_colour_picker_is_open() {
    use egui_kittest::kittest::Queryable;
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"sampling": "rectangular"})).unwrap();
    ui(&mut app, json!({"stroke": {"points": [[20.0, 20.0], [80.0, 20.0]], "size": 10}})).unwrap();
    let less = sampled(&app);
    let app_ctx = ctx(&app);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).with_context(app_ctx).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            app.framework.dialogs.run(ui);
            let ctx = ui.ctx().clone();
            take(app, &ctx);
        },
        app,
    );
    h.run_steps(4);
    // The Sampling Area Overlay section starts closed; its swatch is named "Color".
    h.query_all_by_label("Sampling Area Overlay").next().expect("the section header").click();
    h.run_steps(4);
    h.get_by_label("Color").click();
    h.run_steps(4);
    assert!(crate::color_picker_ui::top(h.state()).is_some(), "the swatch opened the colour picker over the workspace");
    let picker = crate::color_picker_ui::top(h.state()).expect("the colour picker is open");
    crate::color_picker_ui::testing::set_color(h.state_mut(), picker, "#0000ff");
    crate::dialogs::testing::confirm(h.state_mut(), picker).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().caf.as_ref().map(|_| sampled(h.state())), Some(less), "the strokes survive the picker");
    let c = h.state().caf.as_ref().unwrap();
    let settings = h.state().framework.dialogs.get_by_id::<Hosted>(c.id).unwrap().workspace().settings().clone();
    assert_eq!(settings.color, [0, 0, 255]);
}

/// Runs passes until a preview job started after `runs` has finished and `done` holds of the facts.
fn settle_after(app: &mut PhotocraftApp, runs: u64, done: impl Fn(&Value) -> bool) -> Value {
    settle(app, |d| d["previewRuns"].as_u64().unwrap() > runs && done(d))
}

fn runs(app: &PhotocraftApp) -> u64 {
    facts(app)["previewRuns"].as_u64().unwrap()
}

fn hint(d: &Value) -> bool {
    d["previewHint"] == json!(CUSTOM_HINT)
}

/// Picking Custom before anything is painted is how Photoshop starts it: the preview shows the image unfilled and a
/// hint to paint with the Sampling Brush, not the fill's "custom sampling needs `area`…" refusal (#10621). An erasing
/// stroke paints nothing in an empty area: still the hint. The first added stroke previews the fill.
#[test]
fn custom_sampling_with_nothing_painted_hints_instead_of_failing() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    settle(&mut app, |d| d["previewReady"] == json!(true));
    let before = runs(&app);
    ui(&mut app, json!({"sampling": "custom"})).unwrap();
    let d = settle_after(&mut app, before, hint);
    assert!(d["previewError"].is_null() && d["error"].is_null(), "{d}");
    assert_eq!(d["previewReady"], json!(false), "the image shows unfilled, not the earlier fill: {d}");
    let before = runs(&app);
    ui(&mut app, json!({"stroke": {"points": [[20.0, 120.0]], "size": 8, "alt": true}})).unwrap();
    let d = settle_after(&mut app, before, |_| true);
    assert!(hint(&d) && d["previewError"].is_null(), "{d}");
    ui(&mut app, json!({"stroke": {"points": [[20.0, 120.0], [60.0, 120.0]], "size": 8}})).unwrap();
    let d = settle(&mut app, |d| d["previewReady"] == json!(true));
    assert!(d["previewHint"].is_null() && d["previewError"].is_null(), "the fill previews: {d}");
}

/// The texts the last frame painted.
fn painted(app: &PhotocraftApp) -> Vec<String> {
    app.canvas_tools.frames.as_ref().map(|f| f.texts().iter().map(|t| t.text.clone()).collect()).unwrap_or_default()
}

/// Custom with nothing painted is not a failed preview in the workspace either: the framework draws no red "Preview
/// failed" notice over the image, which shows unfilled (#10621, FredApps on #10624).
#[test]
fn custom_with_nothing_painted_shows_no_failed_preview_in_the_workspace() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    settle(&mut app, |d| d["previewReady"] == json!(true));
    let before = runs(&app);
    ui(&mut app, json!({"sampling": "custom"})).unwrap();
    settle_after(&mut app, before, hint);
    pass(&mut app);
    let texts = painted(&app);
    assert!(texts.iter().any(|t| t.contains("Sampling")), "the workspace is on screen: {texts:?}");
    assert!(!texts.iter().any(|t| t.contains("Preview failed")), "no failed-preview notice: {texts:?}");
}

/// Erasing everything the Sampling Brush added in Custom leaves the area as empty as before anything was painted: the
/// same hint, the image unfilled, and Apply and OK say so too, without running the fill and without an error in the
/// status bar or the app's panel error (#10621).
#[test]
fn custom_sampling_erased_again_hints_and_apply_and_ok_do_not_run_the_fill() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    ui(&mut app, json!({"sampling": "custom"})).unwrap();
    ui(&mut app, json!({"apply": true})).unwrap();
    assert_eq!(app.ui.status, CUSTOM_HINT, "Apply with nothing painted");
    assert!(!app.ui.status_error);
    ui(&mut app, json!({"stroke": {"points": [[20.0, 120.0], [60.0, 120.0]], "size": 8}})).unwrap();
    settle(&mut app, |d| d["previewReady"] == json!(true));
    ui(&mut app, json!({"stroke": {"points": [[20.0, 120.0], [60.0, 120.0]], "size": 8, "alt": true}})).unwrap();
    let d = settle(&mut app, hint);
    assert_eq!(d["sampledProxyPixels"].as_u64(), Some(0), "{d}");
    assert!(d["previewError"].is_null() && d["previewReady"] == json!(false), "{d}");
    app.ui.status.clear();
    ui(&mut app, json!({"apply": true})).unwrap();
    assert_eq!(app.ui.status, CUSTOM_HINT, "Apply");
    assert_eq!(ui(&mut app, json!({"commit": true})).unwrap_err(), format!("`commit`: {CUSTOM_HINT}"), "OK answers with the hint");
    close_all(&mut app);
    assert!(app.caf.is_some(), "OK keeps the workspace open");
    assert!(facts(&app)["error"].is_null() && !app.ui.status_error, "{}", facts(&app));
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "nothing was filled");
    // Painting again previews the fill.
    ui(&mut app, json!({"stroke": {"points": [[20.0, 120.0], [60.0, 120.0]], "size": 8}})).unwrap();
    let d = settle(&mut app, |d| d["previewReady"] == json!(true));
    assert!(d["previewHint"].is_null() && d["previewError"].is_null() && d["error"].is_null(), "{d}");
}

/// Apply and OK with nothing to sample in Custom put the hint in the status bar in the UI's language, not in the
/// error colour, and an earlier Apply's error gives way to it (#10621).
#[test]
fn custom_apply_and_ok_with_nothing_painted_show_the_translated_hint_not_an_error() {
    let ja = crate::i18n::Lang::from_code("ja").unwrap();
    // The catalogs live in craft-i18n: the hint shows in Japanese once it is translated there, and in English until
    // then; either way it is the hint, not an error.
    let hint = crate::i18n::tr(ja, CUSTOM_HINT).to_string();
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"sampling": "custom"})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    for ok in [false, true] {
        app.caf.as_mut().unwrap().error = Some("an earlier Apply's error".into());
        app.ui.status_error = true;
        let what = if ok { "OK" } else { "Apply" };
        let request = if ok { json!({"commit": true}) } else { json!({"apply": true}) };
        let _ = crate::i18n::with_language(ja, || ui(&mut app, request));
        close_all(&mut app);
        assert!(app.caf.is_some(), "{what} keeps the workspace open");
        assert!(facts(&app)["error"].is_null(), "{what}: no panel error: {}", facts(&app));
        assert_eq!(app.ui.status, hint, "{what}");
        assert!(!app.ui.status_error, "{what}: not an error");
    }
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "nothing was filled");
}

/// Custom's hint follows the fill area as well as the strokes: a lasso or Expand that covers everything painted
/// brings it back, and one that uncovers a stroke (Contract, or undoing the lasso) takes it away, without anything
/// being painted again.
#[test]
fn custom_hint_follows_fill_area_changes_under_the_strokes() {
    let mut app = app_with_selection();
    open(&mut app).unwrap();
    ui(&mut app, json!({"sampling": "custom"})).unwrap();
    settle(&mut app, hint);
    // A stroke inside the fill area (90..110) samples nothing; Contract uncovers it.
    let before = runs(&app);
    ui(&mut app, json!({"stroke": {"points": [[92.0, 76.0], [92.0, 84.0]], "size": 2}})).unwrap();
    let d = settle_after(&mut app, before, |_| true);
    assert!(hint(&d), "{d}");
    ui(&mut app, json!({"contract": 5})).unwrap();
    let d = settle(&mut app, |d| !hint(d));
    assert!(d["error"].is_null(), "contracted off the stroke: {d}");
    ui(&mut app, json!({"undo": true})).unwrap();
    settle(&mut app, hint);
    // A stroke just right of it samples; a lasso over it covers it, Expand too.
    ui(&mut app, json!({"undo": true})).unwrap();
    assert_eq!(facts(&app)["strokes"].as_u64(), Some(0));
    ui(&mut app, json!({"stroke": {"points": [[112.0, 76.0], [112.0, 84.0]], "size": 2}})).unwrap();
    settle(&mut app, |d| !hint(d));
    ui(&mut app, json!({"lasso": {"op": "add", "points": [[105.0, 70.0], [125.0, 70.0], [125.0, 90.0], [105.0, 90.0]]}})).unwrap();
    let d = settle(&mut app, hint);
    assert!(d["error"].is_null(), "the lasso covers the stroke: {d}");
    assert_eq!(d["strokes"].as_u64(), Some(1), "the stroke is kept: {d}");
    ui(&mut app, json!({"undo": true})).unwrap();
    settle(&mut app, |d| !hint(d));
    ui(&mut app, json!({"expand": 4})).unwrap();
    let d = settle(&mut app, hint);
    assert!(d["error"].is_null(), "Expand covers the stroke: {d}");
    assert_eq!(ui(&mut app, json!({"commit": true})).unwrap_err(), format!("`commit`: {CUSTOM_HINT}"), "OK answers with the hint");
}

/// Custom's "nothing to sample" follows what the fill samples, the full-resolution area, not the proxy (2 document px
/// per proxy px here): a 1 px stroke on the fill area's left edge keeps a column outside it that the proxy, sampling
/// every other column, doesn't see, and one just past the canvas's right edge marks the proxy's last column (half of
/// it off the canvas) while the fill has nothing there.
#[test]
fn custom_hint_follows_the_full_resolution_area_not_the_proxy() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 2201, "height": 40, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            let s = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            s.fill_rect(Rect::new(0, 0, 2201, 40), &[0.2, 0.6, 0.4, 1.0]);
            s.fill_rect(Rect::new(1091, 10, 1111, 30), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.session.execute("select.rect", json!({"x": 1091, "y": 10, "width": 20, "height": 20})).unwrap();
    app.sync_views();
    open(&mut app).unwrap();
    assert_eq!(app.caf.as_ref().unwrap().k, 2);
    ui(&mut app, json!({"sampling": "custom"})).unwrap();
    settle(&mut app, hint);
    let steps = app.session.active().unwrap().history.past_len();
    // Column 1090 is sampled; the proxy's column 545 (document column 1091) is in the fill area.
    ui(&mut app, json!({"stroke": {"points": [[1091.0, 15.0], [1091.0, 25.0]], "size": 1}})).unwrap();
    let d = settle(&mut app, |d| !hint(d));
    assert_eq!(d["sampledProxyPixels"].as_u64(), Some(0), "{d}");
    let (_, source, hole) = engine_source(&app);
    assert!(source.iter().zip(&hole).any(|(s, h)| *s && !*h), "the fill samples column 1090");
    ui(&mut app, json!({"apply": true})).unwrap();
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "Apply fills");
    // Past the canvas: the proxy's column 1100 (document column 2201) is marked, the fill samples nothing.
    ui(&mut app, json!({"cancel": true})).unwrap();
    close_all(&mut app);
    open(&mut app).unwrap();
    ui(&mut app, json!({"sampling": "custom"})).unwrap();
    let before = runs(&app);
    ui(&mut app, json!({"stroke": {"points": [[2202.0, 5.0], [2202.0, 35.0]], "size": 1}})).unwrap();
    let d = settle_after(&mut app, before, |_| true);
    assert!(d["sampledProxyPixels"].as_u64().unwrap() > 0, "{d}");
    assert!(hint(&d), "{d}");
    assert_eq!(ui(&mut app, json!({"commit": true})).unwrap_err(), format!("`commit`: {CUSTOM_HINT}"), "OK answers with the hint");
}

/// A big Sampling Brush stroke kept inside the fill area samples nothing: the hint stays. One that reaches past the
/// fill area samples the pixels outside it.
#[test]
fn custom_stroke_inside_the_fill_area_keeps_the_hint() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 300, "height": 200, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 300, 200), &[0.2, 0.6, 0.4, 1.0]);
            Ok(())
        })
        .unwrap();
    app.session.execute("select.rect", json!({"x": 50, "y": 40, "width": 200, "height": 120})).unwrap();
    app.sync_views();
    open(&mut app).unwrap();
    ui(&mut app, json!({"sampling": "custom"})).unwrap();
    let before = runs(&app);
    ui(&mut app, json!({"stroke": {"points": [[120.0, 100.0], [180.0, 100.0]], "size": 100}})).unwrap();
    let d = settle_after(&mut app, before, |_| true);
    assert!(hint(&d), "{d}");
    // 10 px past the fill area's right edge.
    ui(&mut app, json!({"stroke": {"points": [[160.0, 100.0], [210.0, 100.0]], "size": 100}})).unwrap();
    let d = settle(&mut app, |d| !hint(d));
    assert!(d["previewError"].is_null(), "{d}");
}
