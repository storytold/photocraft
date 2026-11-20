use super::*;
use crate::control::dialogs::workspace_ui;
use craft::imaging::ui::liquify::{BackdropMode, BackdropUse, LiquifySettings};
use craft::ui::runtime::memory::{saved_value, set_saved_value};

const KEY: &str = "filter.liquify";

/// The hosted Liquify workspace's value.
fn ws(app: &mut PhotocraftApp) -> &mut LiquifyWorkspace {
    app.framework.dialogs.get_mut::<Hosted>().expect("the Liquify workspace is open").workspace_mut()
}

/// The app's side of the open Liquify workspace.
fn la(app: &PhotocraftApp) -> &LiquifyApp {
    app.liquify.as_ref().expect("the Liquify app state exists")
}

/// The app's field, proxy and pictures (`take` writes them under the same lock).
fn eng(app: &PhotocraftApp) -> MutexGuard<'_, Engine> {
    lock(&la(app).engine)
}

fn is_open(app: &PhotocraftApp) -> bool {
    app.framework.dialogs.is_open::<Hosted>()
}

/// Runs one frame: the workspace's pass reports the events it queued, and the app applies them to its field.
fn settle(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    crate::canvas::testing::frame(app);
}

/// The open workspace's facts (`describe`).
fn facts(app: &PhotocraftApp) -> Value {
    describe(app).expect("the Liquify workspace is open")
}

/// A control request to the Liquify workspace, then the events it produced.
fn ctl(app: &mut PhotocraftApp, v: Value) -> Result<Value, String> {
    let ctx = app.ctx().clone();
    let r = workspace_ui(app, &ctx, KEY, &v);
    settle(app, &ctx);
    r
}

/// Opens the workspace and runs its first frame: the pass reads the remembered settings and asks for the backdrop.
fn open_ws(app: &mut PhotocraftApp) -> Result<(), String> {
    let ctx = app.ctx().clone();
    open(app, &ctx)?;
    crate::canvas::testing::frame(app);
    Ok(())
}

/// The workspace's stroke of `tool` with the default brush at `size`.
fn stroke_with(tool: StepTool, size: f64, points: &[[f64; 2]]) -> StepStroke {
    let mut brush = LiquifySettings::default().brush;
    brush.size = size;
    StepStroke::new(tool, &brush, points.iter().map(|p| [p[0], p[1], 1.0]).collect())
}

/// The engine's form of `stroke`, as `filter.liquify` takes it.
fn engine_of(stroke: &StepStroke) -> LiquifyStroke {
    engine_stroke(stroke).expect("a painting tool")
}

/// Records `stroke` in the workspace and lets the app apply it.
fn perform(app: &mut PhotocraftApp, stroke: StepStroke) {
    let ctx = app.ctx().clone();
    ws(app).perform(LiquifyStep::Stroke(stroke));
    settle(app, &ctx);
}

fn undo(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    ws(app).undo();
    settle(app, &ctx);
}

fn redo(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    ws(app).redo();
    settle(app, &ctx);
}

fn strokes(app: &PhotocraftApp) -> u64 {
    facts(app)["strokes"].as_u64().unwrap()
}

fn redo_len(app: &PhotocraftApp) -> u64 {
    facts(app)["redo"].as_u64().unwrap()
}

/// A three-point stroke through the middle of the test layer, for the tests that need a visible warp.
fn sweep(app: &mut PhotocraftApp, y: f64) {
    perform(app, stroke_with(StepTool::ForwardWarp, 50.0, &[[40.0, y], [60.0, y + 2.0], [80.0, y]]));
}

fn app_with_layer() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 120, "height": 80, "depth": 8})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            let s = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            for y in 10..70 {
                for x in 10..110 {
                    let v = ((x / 5 + y / 5) % 2) as f32;
                    s.write_pixel(x, y, &[v, 0.3, 1.0 - v, 1.0]);
                }
            }
            Ok(())
        })
        .unwrap();
    app.sync_views();
    app
}

#[test]
fn native_component_liquify_preserves_proxy_detail_and_captured_commit_targets() {
    for mode in ["rgb", "cmyk", "lab"] {
        for depth in [8, 16, 32] {
            for alpha in [true, false] {
                let fixture = crate::distort_ui::component_tests::app(mode, depth);
                let mut app = if alpha { fixture } else { crate::distort_ui::component_tests::without_alpha(fixture) };
                open_ws(&mut app).unwrap();
                let layer = la(&app).layer;
                let (before, bounds) = {
                    let e = eng(&app);
                    (e.source.clone(), e.canvas)
                };
                {
                    let e = eng(&app);
                    let mut field = LiquifyField::new(bounds, 8.0);
                    field.d.fill([2.0, 0.0]);
                    let mut out = vec![[0; 4]; 64 * 48];
                    assert!(render_components(&before, bounds, &field, 1.0, &e.target, bounds, bounds, &mut out));
                    for y in 0..48 {
                        for x in 0..64 {
                            let mut native = before.pixel(x, y);
                            for k in [0, 2] {
                                native[k] = before.sample_channel((x + 2).min(63), y, k);
                            }
                            let rgba = photocraft_raster::to_rgba(&before.format(), &native);
                            let expected = [rgba[0] * rgba[3], rgba[1] * rgba[3], rgba[2] * rgba[3], rgba[3]].map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
                            assert!(out[(y * 64 + x) as usize].iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1), "{mode}@{depth}");
                        }
                    }
                }
                perform(&mut app, stroke_with(StepTool::ForwardWarp, 30.0, &[[24.0, 24.0], [34.0, 24.0]]));
                {
                    let e = eng(&app);
                    let mut detail = vec![[0; 4]; 64 * 48];
                    assert!(render_components(&before, bounds, &e.field, 1.0, &e.target, bounds, bounds, &mut detail));
                    assert_eq!(e.out, detail, "proxy at 100% equals the detail path");
                }
                app.run("channel.target", json!({"channel":{"color":1}})).unwrap();
                ctl(&mut app, json!({"commit": true})).unwrap();
                let after = app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap();
                crate::distort_ui::component_tests::protected(&before, after, bounds);
                assert_ne!(after.read_region(bounds), before.read_region(bounds));
            }
        }
    }
}

#[test]
fn a_zoomed_read_covers_only_the_asked_region_and_a_stroke_reports_its_damage() {
    let mut app = app_with_layer();
    open_ws(&mut app).unwrap();
    let read = |app: &PhotocraftApp| {
        let mut image = ColorImage::filled([30, 20], Color32::TRANSPARENT);
        eng(app).read(pixel_rect(Rect::new(45, 30, 75, 50)), 1, &mut image);
        image.pixels
    };
    let before = read(&app);
    let revision = eng(&app).revision;
    assert_eq!(before.len(), 30 * 20, "one texel per document pixel of the region");

    perform(&mut app, stroke_with(StepTool::ForwardWarp, 24.0, &[[50.0, 40.0], [58.0, 40.0]]));
    assert_ne!(read(&app), before, "a brush update changes what the region reads");
    let damage = eng(&app).damage(revision).expect("the ring still holds the stroke");
    assert!(damage.x1 - damage.x0 < 120 && damage.y1 - damage.y0 < 80, "the stroke damages part of the picture, not all of it");
}

/// #7609: what each Show Backdrop › Use shows, and that a remembered Use opens on All Layers.
#[test]
fn backdrop_use_and_mode() {
    // All Layers is the document with the liquified layer in it; one layer is that layer alone.
    let app = app_with_layer();
    let st = app.session.active().unwrap();
    let (layer, background) = (st.active_layer.unwrap(), st.doc.layers[0].id);
    let at = |doc: &Document| photocraft_compose::render(doc, Rect::new(10, 10, 11, 11)).px[0];
    let own = at(&st.doc);
    assert_eq!(at(&backdrop_doc(&st.doc, None)), own, "All Layers");
    assert_eq!(at(&backdrop_doc(&st.doc, Some(layer.0))), own, "the liquified layer alone");
    assert_ne!(at(&backdrop_doc(&st.doc, Some(background.0))), own, "the Background alone");
    assert_eq!(at(&backdrop_doc(&st.doc, Some(9999))), own, "a layer that's gone: All Layers");
    // A hidden layer shows when it is the one used.
    let mut hidden = (*st.doc).clone();
    hidden.layer_mut(layer).unwrap().visible = false;
    assert_eq!(at(&backdrop_doc(&hidden, Some(layer.0))), own, "the hidden liquified layer, alone");
    // A remembered Use is a layer of another document: Liquify opens on All Layers.
    let mut app = app_with_layer();
    let ctx = app.ctx().clone();
    let mut remembered = LiquifySettings::default();
    remembered.backdrop = BackdropUse::Layer(42);
    remembered.backdrop_mode = BackdropMode::Blend;
    set_saved_value(&ctx, &REMEMBERED, Some(remembered));
    open_ws(&mut app).unwrap();
    let s = ws(&mut app).settings();
    assert!(matches!(s.backdrop, BackdropUse::AllLayers), "the layer is not kept");
    assert!(matches!(s.backdrop_mode, BackdropMode::Blend), "the mode is kept");
}

/// #418: the brush settings survive Cancel, OK and Esc, and come back the next time Liquify opens.
#[test]
fn brush_settings_are_remembered_between_uses() {
    let mut app = app_with_layer();
    let ctx = app.ctx().clone();
    open_ws(&mut app).unwrap();
    let fresh = ws(&mut app).settings().clone();
    assert_eq!((fresh.brush.density, fresh.tool), (50.0, StepTool::ForwardWarp), "first use: the defaults");
    let set = json!({"tool": "twirlCw", "size": 37, "density": 85, "pressure": 60, "rate": 30, "showMesh": true, "meshSize": "large", "showBackdrop": true, "backdropOpacity": 25});
    ctl(&mut app, set).unwrap();
    let changed = serde_json::to_value(ws(&mut app).settings()).unwrap();
    assert_ne!(changed, serde_json::to_value(&fresh).unwrap());
    ctl(&mut app, json!({"cancel": true})).unwrap();
    assert!(!is_open(&app));
    open_ws(&mut app).unwrap();
    assert_eq!(serde_json::to_value(ws(&mut app).settings()).unwrap(), changed, "Cancel keeps the settings");
    // OK and Esc keep the latest settings too.
    ctl(&mut app, json!({"density": 90})).unwrap();
    ctl(&mut app, json!({"commit": true})).unwrap();
    open_ws(&mut app).unwrap();
    assert_eq!(ws(&mut app).settings().brush.density, 90.0);
    ctl(&mut app, json!({"density": 70})).unwrap();
    ctl(&mut app, json!({"cancel": true})).unwrap();
    assert_eq!(saved_value::<Option<LiquifySettings>>(&ctx, &REMEMBERED).expect("stored").brush.density, 70.0);
}

/// Follow-up to #1494 / #1484: a reversed twirl replays the same after undo and redo.
#[test]
fn alt_reversed_twirl_survives_replay_undo_and_redo() {
    let mut app = app_with_layer();
    open_ws(&mut app).unwrap();
    perform(&mut app, stroke_with(StepTool::TwirlCounterclockwise, 36.0, &[[53.0, 40.0], [58.0, 41.0]]));
    let first = eng(&app).field.clone();
    perform(&mut app, stroke_with(StepTool::TwirlClockwise, 36.0, &[[70.0, 40.0], [75.0, 42.0]]));
    let both = eng(&app).field.clone();
    assert_ne!(first, both);
    assert_eq!(strokes(&app), 2);

    undo(&mut app);
    assert_eq!(strokes(&app), 1);
    assert_eq!(redo_len(&app), 1);
    assert_eq!(eng(&app).field, first, "undo replays the reversed twirl alone");

    redo(&mut app);
    assert_eq!(strokes(&app), 2);
    assert_eq!(redo_len(&app), 0);
    assert_eq!(eng(&app).field, both, "redo replays both strokes the same");
}

#[test]
fn dialog_strokes_match_the_engine_and_commit_once() {
    let mut app = app_with_layer();
    open_ws(&mut app).unwrap();
    let before_pixels = layer_pixels(&app);
    let warp = stroke_with(StepTool::ForwardWarp, 40.0, &[[40.0, 40.0], [48.0, 41.0], [56.0, 40.0]]);
    perform(&mut app, warp.clone());
    perform(&mut app, stroke_with(StepTool::Bloat, 40.0, &[[80.0, 40.0], [80.0, 40.0]]));
    assert_eq!(strokes(&app), 2);
    assert_eq!(redo_len(&app), 0);
    assert!(eng(&app).field.max_displacement() > 3.0);
    undo(&mut app);
    assert_eq!((strokes(&app), redo_len(&app)), (1, 1));
    redo(&mut app);
    assert_eq!((strokes(&app), redo_len(&app)), (2, 0));
    undo(&mut app);
    let cell = eng(&app).cell;
    let before = app.session.active().unwrap().history.past_len();
    ctl(&mut app, json!({"commit": true})).unwrap();
    assert!(!is_open(&app));
    assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
    // The pixels are the command's for the one stroke left.
    let mut reference = app_with_layer();
    let layer = reference.session.active().unwrap().active_layer.unwrap();
    reference.run("filter.liquify", json!({"strokes": [serde_json::to_value(engine_of(&warp)).unwrap()], "meshSize": cell, "layer": layer.0})).unwrap();
    assert_eq!(layer_pixels(&app), layer_pixels(&reference));
    assert_ne!(layer_pixels(&app), before_pixels);
}

#[test]
fn undo_rerenders_the_proxy_before_the_next_texture_upload() {
    let mut app = app_with_layer();
    open_ws(&mut app).unwrap();
    perform(&mut app, stroke_with(StepTool::ForwardWarp, 40.0, &[[35.0, 40.0], [55.0, 40.0]]));
    perform(&mut app, stroke_with(StepTool::ForwardWarp, 40.0, &[[70.0, 40.0], [90.0, 40.0]]));

    let before = eng(&app).out.clone();
    undo(&mut app);
    assert_eq!(strokes(&app), 1);
    let d = eng(&app);
    assert_ne!(d.out, before, "undo must update the pixels shown by the existing texture");

    let mut expected = vec![[0u8; 4]; d.proxy.w * d.proxy.h];
    d.proxy.render(&d.field, [0, 0, d.proxy.w, d.proxy.h], &mut expected);
    assert_eq!(d.out, expected, "dirty upload source must match the rebuilt field");
}

/// A freeze lasso reaches the app's freeze mask, and Alt (amount 0) thaws it; undo replays the first lasso.
#[test]
fn freeze_lasso_records_a_stroke_and_clears_redo() {
    let mut app = app_with_layer();
    open_ws(&mut app).unwrap();
    let lasso = |r: [f64; 4], amount: f64| {
        let mut s = stroke_with(StepTool::FreezeLasso, 30.0, &[[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]], [r[0], r[1]]]);
        s.amount = Some(amount);
        s
    };
    perform(&mut app, lasso([20.0, 20.0, 80.0, 60.0], 1.0));
    assert_eq!(strokes(&app), 1);
    assert_eq!(eng(&app).field.freeze_at(50.0, 40.0), 1.0, "frozen inside");
    assert_eq!(eng(&app).field.freeze_at(5.0, 5.0), 0.0, "not outside");
    perform(&mut app, lasso([40.0, 30.0, 60.0, 50.0], 0.0));
    assert_eq!(eng(&app).field.freeze_at(50.0, 40.0), 0.0, "Alt thaws");
    assert_eq!(eng(&app).field.freeze_at(25.0, 25.0), 1.0);
    undo(&mut app);
    assert_eq!(eng(&app).field.freeze_at(50.0, 40.0), 1.0, "undo replays the first lasso");
    assert_eq!(redo_len(&app), 1);
    perform(&mut app, lasso([0.0, 0.0, 10.0, 10.0], 1.0));
    assert_eq!(redo_len(&app), 0, "a new lasso clears redo");
}

/// A smart object with a Liquify smart filter (one stroke, mesh size 3) and a Gaussian Blur
/// above it.
fn smart_liquify_app() -> (Box<PhotocraftApp>, LayerId) {
    let mut app = app_with_layer();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let stroke = json!({"tool": "forwardWarp", "size": 40, "points": [[40.0, 40.0, 1.0], [48.0, 41.0, 1.0], [56.0, 40.0, 1.0]]});
    app.run("filter.liquify", json!({"strokes": [stroke], "meshSize": 3.0})).unwrap();
    app.run("filter.blur.gaussianBlur", json!({"radius": 2.0})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    (app, layer)
}

fn smart(app: &PhotocraftApp, layer: LayerId) -> photocraft_doc::SmartObject {
    match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
        photocraft_doc::LayerContent::Smart(sm) => sm.clone(),
        _ => panic!("not a smart object"),
    }
}

fn drag(app: &mut PhotocraftApp, from: f64, to: f64) {
    perform(app, stroke_with(StepTool::ForwardWarp, 40.0, &[[from, 40.0], [to, 40.0]]));
}

#[test]
fn smart_filter_double_click_edits_the_recorded_liquify() {
    let (mut app, layer) = smart_liquify_app();
    let ctx = app.ctx().clone();
    let steps = app.session.active().unwrap().history.past_len();
    let sm = smart(&app, layer);
    let recorded = sm.smart_filters[0].params["strokes"].as_array().unwrap().len();
    assert_eq!(recorded, 1);

    crate::smart_ui::open_editor(&mut app, &ctx, layer.0, &sm, 0);
    let ids: Vec<_> = app.framework.dialogs.stack().ids().collect();
    assert_eq!(ids.len(), 1, "the Liquify workspace only, no generic parameter form");
    assert_eq!(crate::distort_ui::modal_workspace(&app), Some(KEY));
    assert_eq!((la(&app).smart_filter, eng(&app).cell), (Some(0), 3.0));
    assert_eq!(strokes(&app), 1, "the recorded stroke is loaded");
    assert!(eng(&app).field.max_displacement() > 0.0, "and replayed into the field");
    // The preview starts from the pixels below the filter (no liquify, no blur).
    let below = photocraft_engine::smart_cmds::render_below_filter(&app.session.active().unwrap().doc, &sm, 0).unwrap().unwrap();
    let e = eng(&app);
    assert_eq!(e.source.read_region(e.canvas), below.read_region(e.canvas));
    drop(e);

    drag(&mut app, 70.0, 84.0);
    ctl(&mut app, json!({"commit": true})).unwrap();
    assert!(!is_open(&app));
    let sm = smart(&app, layer);
    assert_eq!(sm.smart_filters.len(), 2, "edited in place, not added");
    assert_eq!(sm.smart_filters[0].command, photocraft_engine::distort_cmds::LIQUIFY);
    assert_eq!(sm.smart_filters[0].params["strokes"].as_array().map(Vec::len), Some(2));
    assert_eq!(sm.smart_filters[0].params["meshSize"], json!(3.0));
    assert_eq!(sm.smart_filters[1].command, "filter.blur.gaussianBlur");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(smart(&app, layer).smart_filters[0].params["strokes"].as_array().map(Vec::len), Some(1));
}

#[test]
fn smart_filter_cancel_keeps_and_restore_all_clears_the_recorded_strokes() {
    let (mut app, layer) = smart_liquify_app();
    let ctx = app.ctx().clone();
    let steps = app.session.active().unwrap().history.past_len();
    let params = smart(&app, layer).smart_filters[0].params.clone();

    open_smart_filter(&mut app, &ctx, layer, 0).unwrap();
    drag(&mut app, 70.0, 84.0);
    ctl(&mut app, json!({"cancel": true})).unwrap();
    assert_eq!(smart(&app, layer).smart_filters[0].params, params, "Cancel leaves the filter as it was");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);

    // OK after Restore All keeps the filter, with no strokes.
    open_smart_filter(&mut app, &ctx, layer, 0).unwrap();
    ctl(&mut app, json!({"restoreAll": true})).unwrap();
    ctl(&mut app, json!({"commit": true})).unwrap();
    let sm = smart(&app, layer);
    assert_eq!(sm.smart_filters.len(), 2);
    assert_eq!(sm.smart_filters[0].params["strokes"], json!([]));
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
}

#[test]
fn open_smart_filter_rejects_other_filters_and_layers() {
    let (mut app, layer) = smart_liquify_app();
    let ctx = app.ctx().clone();
    assert!(open_smart_filter(&mut app, &ctx, layer, 1).is_err(), "the Gaussian Blur");
    assert!(open_smart_filter(&mut app, &ctx, layer, 9).is_err(), "no such filter");
    let mut plain = app_with_layer();
    let pixels = plain.session.active().unwrap().active_layer.unwrap();
    assert!(open_smart_filter(&mut plain, &ctx, pixels, 0).is_err(), "not a smart object");
    assert!(app.liquify.is_none() && plain.liquify.is_none());
}

/// OK on a smart filter that is gone: the engine's reason shows in the status line, the workspace stays open
/// (the held OK is answered with `Err`) and nothing is recorded.
#[test]
fn smart_filter_edit_that_cannot_be_applied_reports_the_error() {
    let (mut app, layer) = smart_liquify_app();
    let ctx = app.ctx().clone();
    open_smart_filter(&mut app, &ctx, layer, 0).unwrap();
    drag(&mut app, 70.0, 84.0);
    app.session.execute("layer.smartFilter.clearSmartFilters", json!({"layer": layer.0})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    let _ = ctl(&mut app, json!({"commit": true}));
    assert!(is_open(&app), "a refused OK leaves the workspace open");
    let status = app.ui.status.clone();
    assert!(status.contains("no smart filters"), "{status}");
    assert!(smart(&app, layer).smart_filters.is_empty());
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}

/// OK after the layer was deleted: the same, for a plain layer.
#[test]
fn liquify_that_cannot_be_applied_reports_the_error() {
    let mut app = app_with_layer();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    open_ws(&mut app).unwrap();
    drag(&mut app, 40.0, 56.0);
    assert_eq!(strokes(&app), 1);
    app.session.execute("layer.delete", json!({"layer": layer.0})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    let _ = ctl(&mut app, json!({"commit": true}));
    assert!(is_open(&app), "a refused OK leaves the workspace open");
    assert!(!app.ui.status.is_empty());
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}

/// Opens the workspace `key` with `open`, checks it is the one on the host, and cancels it.
fn opens_and_cancels(app: &mut PhotocraftApp, key: &str, what: &str, open: impl FnOnce(&mut PhotocraftApp) -> Result<(), String>) {
    open(app).unwrap_or_else(|e| panic!("{what}: {e}"));
    assert_eq!(crate::distort_ui::modal_workspace(app), Some(key), "{what}");
    let ctx = app.ctx().clone();
    workspace_ui(app, &ctx, key, &json!({"cancel": true})).unwrap_or_else(|e| panic!("{what}: cancel: {e}"));
    assert_eq!(crate::distort_ui::modal_workspace(app), None, "{what} is cancelled");
}

/// Liquify, Adaptive Wide Angle and the Filter Gallery open on the layer when no colour set is
/// targeted: with its mask, an alpha channel or the Quick Mask targeted. OK sends the command no
/// `target`, so it routes the targeted surface as the command without a target does.
#[test]
fn workspaces_open_on_the_layer_with_a_mask_channel_or_quick_mask_targeted() {
    let setup = |case: &str| {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
        app.run("select.rect", json!({"x": 20, "y": 16, "width": 16, "height": 16})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#0000ff"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        match case {
            "mask" => drop(app.run("layer.layerMask.revealAll", json!({})).unwrap()),
            "alpha" => {
                app.run("channel.new", json!({"fill": "black"})).unwrap();
                app.run("channel.target", json!({"channel": 0})).unwrap();
            }
            _ => drop(app.run("select.editInQuickMaskMode", json!({"on": true})).unwrap()),
        }
        app.sync_views();
        app
    };
    let surfaces = |app: &PhotocraftApp| {
        let st = app.session.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
        (
            l.surface().cloned(),
            l.mask.as_ref().map(|m| m.surface.clone()),
            st.doc.channels.iter().map(|c| c.surface.clone()).collect::<Vec<_>>(),
            st.doc.quick_mask.as_ref().map(|c| c.surface.clone()),
        )
    };
    for case in ["mask", "alpha", "quickMask"] {
        let mut app = setup(case);
        let ctx = app.ctx().clone();
        assert!(crate::canvas::paint_target(&app).get("colors").is_none(), "{case}");
        opens_and_cancels(&mut app, "filter.adaptiveWideAngle", &format!("{case}: Adaptive Wide Angle"), |app| crate::wide_angle_ui::open(app, &ctx));
        opens_and_cancels(&mut app, "filter.filterGallery", &format!("{case}: Filter Gallery"), crate::gallery_ui::open);
        open_ws(&mut app).unwrap_or_else(|e| panic!("{case}: Liquify: {e}"));
        assert_eq!(crate::distort_ui::modal_workspace(&app), Some(KEY), "{case}: Liquify");
        let stroke = stroke_with(StepTool::ForwardWarp, 30.0, &[[20.0, 24.0], [36.0, 24.0]]);
        perform(&mut app, stroke.clone());
        let cell = eng(&app).cell;
        ctl(&mut app, json!({"commit": true})).unwrap();
        let mut before = setup(case);
        let layer = before.session.active().unwrap().active_layer.unwrap();
        before.run("filter.liquify", json!({"strokes": [serde_json::to_value(engine_of(&stroke)).unwrap()], "meshSize": cell, "layer": layer.0})).unwrap();
        assert!(surfaces(&app) == surfaces(&before), "{case}: OK edits what the command without a target edits");
        assert!(surfaces(&app) != surfaces(&setup(case)), "{case}: the stroke changed something");
    }
}

/// A smart object's targeted filter mask: the workspaces open on the smart object, and Liquify's
/// OK adds a smart filter, as the filter mask doesn't take it.
#[test]
fn workspaces_open_on_a_smart_object_with_its_filter_mask_targeted() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.ui.filter_mask_target = Some(layer.0);
    app.sync_views();
    assert!(app.filter_mask_targeted());
    let ctx = app.ctx().clone();
    opens_and_cancels(&mut app, "filter.adaptiveWideAngle", "Adaptive Wide Angle", |app| crate::wide_angle_ui::open(app, &ctx));
    opens_and_cancels(&mut app, "filter.filterGallery", "Filter Gallery", crate::gallery_ui::open);
    open_ws(&mut app).unwrap();
    perform(&mut app, stroke_with(StepTool::ForwardWarp, 30.0, &[[20.0, 24.0], [36.0, 24.0]]));
    ctl(&mut app, json!({"commit": true})).unwrap();
    let st = app.session.active().unwrap();
    let Some(photocraft_doc::LayerContent::Smart(sm)) = st.doc.layer(layer).map(|l| &l.content) else { panic!("not a smart object") };
    assert_eq!(sm.smart_filters.iter().map(|f| f.command.as_str()).collect::<Vec<_>>(), ["filter.blur.gaussianBlur", photocraft_engine::distort_cmds::LIQUIFY]);
}

/// Preview off shows the image unwarped while strokes still change the field (#7905). Each opening
/// starts with it on.
#[test]
fn preview_off_shows_the_original_and_p_toggles_it() {
    let mut app = app_with_layer();
    open_ws(&mut app).unwrap();
    assert!(facts(&app)["preview"] == json!(true), "Liquify opens with Preview on");
    let mut original = {
        let d = eng(&app);
        vec![[0u8; 4]; d.proxy.w * d.proxy.h]
    };
    {
        let d = eng(&app);
        d.proxy.render(&LiquifyField::new(d.canvas, d.cell), [0, 0, d.proxy.w, d.proxy.h], &mut original);
    }
    warp_stroke(&mut app, 40.0);
    assert_ne!(eng(&app).out, original, "the warp shows");

    let r = ctl(&mut app, json!({"preview": false})).unwrap();
    assert_eq!(r["preview"], json!(false));
    assert_eq!(eng(&app).out, original, "Preview off: the image as it was");
    warp_stroke(&mut app, 30.0);
    assert_eq!(strokes(&app), 2);
    assert_eq!(eng(&app).out, original, "still unwarped while Preview is off");

    ctl(&mut app, json!({"preview": true})).unwrap();
    let d = eng(&app);
    let mut shown = vec![[0u8; 4]; d.proxy.w * d.proxy.h];
    d.proxy.render(&d.field, [0, 0, d.proxy.w, d.proxy.h], &mut shown);
    assert_eq!(d.out, shown, "Preview on: the warp of both strokes");
}

/// A forward-warp stroke across the test layer, at row `y`.
fn warp_stroke(app: &mut PhotocraftApp, y: f64) {
    perform(app, stroke_with(StepTool::ForwardWarp, 40.0, &[[35.0, y], [60.0, y]]));
}

/// The active layer's pixels over the whole document.
fn layer_pixels(app: &PhotocraftApp) -> Vec<f32> {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().read_region(Rect::new(0, 0, 120, 80))
}

/// With Preview off, a stroke changes the displacement field but the display stays the original
/// image (#7905).
#[test]
fn a_stroke_with_preview_off_changes_the_field_but_not_the_display() {
    let mut app = app_with_layer();
    open_ws(&mut app).unwrap();
    ctl(&mut app, json!({"preview": false})).unwrap();
    let (field_before, shown_before) = {
        let e = eng(&app);
        (e.field.clone(), e.out.clone())
    };
    assert_eq!(eng(&app).field.max_displacement(), 0.0);
    warp_stroke(&mut app, 40.0);
    let d = eng(&app);
    assert_ne!(d.field, field_before, "the stroke changes the field");
    assert!(d.field.max_displacement() > 3.0);
    assert_eq!(d.out, shown_before, "the display does not change");
}

/// OK with Preview off applies the warp: the layer ends up exactly as it does when the same stroke
/// is committed with Preview on.
#[test]
fn ok_with_preview_off_applies_the_warp() {
    let commit_with = |preview: bool| {
        let mut app = app_with_layer();
        let before = layer_pixels(&app);
        open_ws(&mut app).unwrap();
        ctl(&mut app, json!({"preview": preview})).unwrap();
        warp_stroke(&mut app, 40.0);
        let past = app.session.active().unwrap().history.past_len();
        ctl(&mut app, json!({"commit": true})).unwrap();
        assert!(!is_open(&app));
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1, "one Liquify step");
        (before, layer_pixels(&app))
    };
    let (before, unpreviewed) = commit_with(false);
    assert_ne!(unpreviewed, before, "OK with Preview off warps the layer");
    assert_eq!(unpreviewed, commit_with(true).1, "the same warp as with Preview on");
}

/// Counts the proxy pixels outside the selection (document x >= 60) that differ from `original`, and those inside
/// that do: the preview shows Liquify only inside the selection.
fn assert_masked_to_left_half(d: &Engine, original: &[[u8; 4]]) {
    let doc_x = |i: usize| d.proxy.bounds.x0 as f64 + ((i % d.proxy.w) as f64 + 0.5) * d.proxy.scale;
    let (mut outside, mut inside_changed) = (0, 0);
    for (i, (a, b)) in d.out.iter().zip(original).enumerate() {
        if doc_x(i) >= 60.0 {
            outside += 1;
            assert_eq!(a, b, "proxy pixel {i} (x {}) is outside the selection", doc_x(i));
        } else if a != b {
            inside_changed += 1;
        }
    }
    assert!(outside > 0 && inside_changed > 0, "outside {outside}, changed inside {inside_changed}");
}

/// The picture's region x 30..90, y 20..60 as the display reads it, one texel per document pixel.
fn read_view(app: &PhotocraftApp) -> Vec<Color32> {
    let mut image = ColorImage::filled([60, 40], Color32::TRANSPARENT);
    eng(app).read(pixel_rect(Rect::new(30, 20, 90, 60)), 1, &mut image);
    image.pixels
}

/// The same for what the display reads (`read_view`), against `before` taken before the stroke.
fn assert_view_masked(after: &[Color32], before: &[Color32]) {
    let (mut outside, mut inside_changed) = (0, 0);
    for (i, (a, b)) in after.iter().zip(before).enumerate() {
        let x = 30 + (i % 60) as i32;
        if x >= 60 {
            outside += 1;
            assert_eq!(a, b, "view pixel at x {x} is outside the selection");
        } else if a != b {
            inside_changed += 1;
        }
    }
    assert!(outside > 0 && inside_changed > 0, "view: outside {outside}, changed inside {inside_changed}");
}

/// A targeted colour channel's preview renders through its own path, and the selection applies
/// to that path too (#9541): unselected pixels stay as they were.
#[test]
fn a_targeted_channels_preview_shows_liquify_only_inside_the_selection() {
    let mut app = app_with_layer();
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 60, "height": 80})).unwrap();
    app.run("channel.target", json!({"channel": {"color": 0}})).unwrap();
    open_ws(&mut app).unwrap();
    assert!(eng(&app).native_proxy.is_some());
    let view_before = read_view(&app);
    sweep(&mut app, 40.0);
    {
        let mut d = eng(&app);
        d.dirty = Some([0, 0, d.proxy.w, d.proxy.h]);
        d.render_dirty();
        let mut original = vec![[0u8; 4]; d.proxy.w * d.proxy.h];
        d.proxy.render(&LiquifyField::new(d.canvas, d.cell), [0, 0, d.proxy.w, d.proxy.h], &mut original);
        assert_masked_to_left_half(&d, &original);
    }
    assert_view_masked(&read_view(&app), &view_before);
}

/// #9541: with a selection, the preview shows what OK applies: unselected pixels stay as they were,
/// in the proxy and in what the display reads.
#[test]
fn the_preview_shows_liquify_only_inside_the_selection() {
    let mut app = app_with_layer();
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 60, "height": 80})).unwrap();
    open_ws(&mut app).unwrap();
    let view_before = read_view(&app);
    sweep(&mut app, 40.0);
    {
        let d = eng(&app);
        assert!(d.field.max_displacement() > 3.0);
        let mut original = vec![[0u8; 4]; d.proxy.w * d.proxy.h];
        d.proxy.render(&LiquifyField::new(d.canvas, d.cell), [0, 0, d.proxy.w, d.proxy.h], &mut original);
        assert_masked_to_left_half(&d, &original);
    }
    assert_view_masked(&read_view(&app), &view_before);
}

/// On a smart object with a filter mask, Liquify is shown through that mask, as the engine applies
/// it (`add_smart_filter` keeps the mask), not through a different selection. Where the mask hides
/// it the placed pixels show, not the filters below the Liquify being edited.
#[test]
fn the_preview_on_a_smart_object_follows_its_filter_mask() {
    let mut app = app_with_layer();
    let ctx = app.ctx().clone();
    app.session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    // The first smart filter, run on the right half, makes that half the filter mask.
    app.session.execute("select.rect", json!({"x": 60, "y": 0, "width": 60, "height": 80})).unwrap();
    app.session.execute("filter.blur.gaussianBlur", json!({"radius": 0.5})).unwrap();
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 60, "height": 80})).unwrap();
    open_ws(&mut app).unwrap();
    sweep(&mut app, 40.0);
    // Reopened as a smart filter above the blur, it edits the blurred pixels.
    ctl(&mut app, json!({"commit": true})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    open_smart_filter(&mut app, &ctx, layer, 1).unwrap();
    sweep(&mut app, 30.0);
    let st = app.session.active().unwrap();
    let Some(photocraft_doc::LayerContent::Smart(sm)) = st.doc.layer(st.active_layer.unwrap()).map(|l| &l.content) else { panic!("not a smart object") };
    let placed = photocraft_engine::smart_cmds::render_below_filter(&st.doc, sm, 0).unwrap().unwrap();
    let d = eng(&app);
    let placed = ProxyImage::new(&placed, d.canvas, PROXY_SIDE);
    let mut original = vec![[0u8; 4]; d.proxy.w * d.proxy.h];
    placed.render(&LiquifyField::new(d.canvas, d.cell), [0, 0, d.proxy.w, d.proxy.h], &mut original);
    // Inside the mask it edits the blurred pixels: the blur alone must not count as the Liquify.
    let mut input = vec![[0u8; 4]; d.proxy.w * d.proxy.h];
    d.proxy.render(&LiquifyField::new(d.canvas, d.cell), [0, 0, d.proxy.w, d.proxy.h], &mut input);
    let doc_x = |i: usize| d.proxy.bounds.x0 as f64 + ((i % d.proxy.w) as f64 + 0.5) * d.proxy.scale;
    let (mut masked, mut shown_changed) = (0, 0);
    for (i, (a, (b, c))) in d.out.iter().zip(original.iter().zip(&input)).enumerate() {
        if doc_x(i) < 60.0 {
            masked += 1;
            assert_eq!(a, b, "proxy pixel {i} (x {}) is outside the filter mask", doc_x(i));
        } else if a != c {
            shown_changed += 1;
        }
    }
    assert!(masked > 0 && shown_changed > 0, "masked {masked}, changed inside the mask {shown_changed}");
}
