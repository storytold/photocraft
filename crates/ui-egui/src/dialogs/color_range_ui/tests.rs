use super::*;
use craft::imaging::ui::color_range::SampleTool;
use craft::keymap::Mods;
use egui::Modifiers;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{Color, ColorMode, SampleType, Size};
use photocraft_geom::Rect as GRect;

/// 40 × 30: red block (0..20, 0..15), blue block (20..40, 0..15), black and white below.
fn session_with_doc() -> photocraft_engine::Session {
    let mut doc = Document::with_background("cr", Size::new(40, 30), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let bg = doc.layers[0].surface_mut().unwrap();
    bg.fill_rect(GRect::new(0, 0, 20, 15), &[1.0, 0.0, 0.0, 1.0]);
    bg.fill_rect(GRect::new(20, 0, 40, 15), &[0.0, 0.0, 1.0, 1.0]);
    bg.fill_rect(GRect::new(0, 15, 20, 30), &[0.0, 0.0, 0.0, 1.0]);
    let mut s = photocraft_engine::Session::new();
    s.add_document(doc, None);
    s
}

fn app_with_doc() -> Box<PhotocraftApp> {
    PhotocraftApp::new(session_with_doc(), crate::Services::default())
}

fn coverage(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    app.session.active().unwrap().doc.selection.as_ref().map_or(0.0, |s| s.sample_channel(x, y, 0))
}

/// The app's UI on its own context: the menu bar and the dialog host, as a frame orders them.
fn harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).with_context(app.ctx().clone()).build_ui_state(
        |ui, app| {
            crate::menus::menu_bar(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(2);
    h
}

fn hosted(app: &PhotocraftApp) -> &HostedColorRange {
    app.framework.dialogs.get::<HostedColorRange>().expect("Color Range dialog open")
}

fn dialog_id(app: &PhotocraftApp) -> DialogId {
    app.framework.dialogs.stack().top().expect("Color Range dialog open")
}

/// Sets the dialog's field `key` through the host, as a script or a click on its control does.
fn set_field(app: &mut PhotocraftApp, key: &str, v: Value) {
    let id = dialog_id(app);
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, key, v).unwrap_or_else(|e| panic!("the dialog takes `{key}`: {e}"));
}

fn set(h: &mut Harness<'static, Box<PhotocraftApp>>, key: &str, v: Value) {
    set_field(h.state_mut(), key, v);
    h.run_steps(3);
}

/// OK through the host, then the frames in which the dialog's answer is taken.
fn ok(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    let id = dialog_id(h.state());
    let _ = crate::dialogs::testing::confirm(h.state_mut(), id);
    h.run_steps(3);
}

fn press_release(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    // The dialog opens centred over the canvas and refits the view; the press needs a point it neither covers nor moves.
    crate::canvas::testing::clear_of_dialogs(app, at);
    crate::canvas::testing::press(app, at, mods);
    crate::canvas::testing::release(app, at, mods);
}

#[test]
fn preview_and_eyedropper_keep_the_original_selection_boundary() {
    let mut app = app_with_doc();
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 30})).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    for params in [json!({"select": "sampledColors", "color": "#ff0000", "fuzziness": 0}), json!({"select": "reds"})] {
        let mask = proxy_mask(&app, &doc, 1, &params).unwrap();
        assert_eq!(mask[5 * 40 + 5], 1.0, "red inside the selection stays visible");
        assert_eq!(mask[5 * 40 + 25], 0.0, "the blue outside the selection stays excluded");
    }
    for params in [json!({"select": "sampledColors", "color": "#ffffff", "invert": true}), json!({"select": "outOfGamut", "invert": true})] {
        let mask = proxy_mask(&app, &doc, 1, &params).unwrap();
        assert_eq!(mask[5 * 40 + 25], 0.0, "even inverted previews stay within the selection");
    }

    // As in Photoshop, the eyedropper samples anywhere; only the result is scoped (#5244).
    open(&mut app).unwrap();
    press_release(&mut app, [25.5, 5.5], Modifiers::NONE);
    assert_eq!(hosted(&app).value().samples.add, vec![[25.0, 5.0]], "a colour outside the selection is sampled");
    let mask = proxy_mask(&app, &doc, 1, &json!({"select": "sampledColors", "points": [[25, 5]], "fuzziness": 0})).unwrap();
    assert_eq!(mask[5 * 40 + 25], 0.0, "but the preview stays within the selection");
    press_release(&mut app, [5.5, 5.5], Modifiers::NONE);
    assert_eq!(hosted(&app).value().samples.add, vec![[5.0, 5.0]]);
}

#[test]
fn preview_cache_refreshes_after_foreground_and_proof_changes() {
    let mut app = app_with_doc();
    let p = json!({"select": "outOfGamut"});
    let mut previous = query_key(&app, &p);
    app.session.tools.foreground = [0.2, 0.3, 0.4, 1.0];
    let key = query_key(&app, &p);
    assert_ne!(key, previous);
    previous = key;
    for setup in [
        json!({"profile": "srgb"}),
        json!({"profile": "linear-srgb"}),
        json!({"profile": "linear-srgb", "intent": "perceptual"}),
        json!({"profile": "linear-srgb", "intent": "perceptual", "bpc": false}),
        json!({"profile": "linear-srgb", "intent": "perceptual", "bpc": false, "simulatePaper": true}),
    ] {
        app.run("view.proofSetup", setup).unwrap();
        let key = query_key(&app, &p);
        assert_ne!(key, previous, "changed proof settings must invalidate the preview");
        assert_eq!(key, query_key(&app, &p), "an unchanged ICC profile must keep its cache key");
        previous = key;
    }
    app.run("view.gamutWarning", json!({"on": true, "threshold": 8})).unwrap();
    assert_ne!(previous, query_key(&app, &p));
    previous = query_key(&app, &p);
    app.run("view.gamutWarning", json!({"on": true, "threshold": 16})).unwrap();
    assert_ne!(previous, query_key(&app, &p));
    assert_ne!(query_key(&app, &p), query_key(&app, &json!({"select": "outOfGamut", "invert": true})));
}

#[test]
fn eyedropper_hover_reuses_preview_textures_without_sampling() {
    let mut app = app_with_doc();
    open(&mut app).unwrap();
    let ctx = app.ctx().clone();
    // The first pass shows the canvas and builds the dialog's pictures.
    crate::canvas::testing::move_to(&mut app, [4.5, 15.5], Modifiers::NONE);
    set_field(&mut app, "selectionPreview", json!(PreviewMode::QuickMask.id()));
    crate::canvas::testing::move_to(&mut app, [4.5, 15.5], Modifiers::NONE);
    let cached = |app: &mut PhotocraftApp| {
        let revision = selection_preview(app, &ctx).expect("Quick Mask shows a preview").mask.revision();
        let p = app.color_range.as_ref().unwrap();
        (p.mask_key, Arc::as_ptr(p.mask.as_ref().unwrap()) as *const () as usize, Arc::as_ptr(&p.proxy) as usize, revision)
    };
    let before = cached(&mut app);
    let st = app.session.active().unwrap();
    let document = (st.revision, st.history.past_len());
    for i in 0..12 {
        crate::canvas::testing::move_to(&mut app, [4.0 + i as f64 * 0.5, 15.0], Modifiers::NONE);
        assert_eq!(cached(&mut app), before, "hover must reuse the mask pictures and proxy");
    }
    assert!(hosted(&app).value().samples.add.is_empty(), "hover must not pick a colour");
    let st = app.session.active().unwrap();
    assert_eq!((st.revision, st.history.past_len()), document);
}

#[test]
fn ok_applies_one_undo_step() {
    let mut h = harness(app_with_doc());
    // The prior selection covers the left red square: Color Range is scoped to it (#3313).
    h.state_mut().run("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 30})).unwrap();
    let past = h.state().session.active().unwrap().history.past_len();
    open(h.state_mut()).unwrap();
    h.run_steps(4);
    set(&mut h, "select", json!("reds"));
    ok(&mut h);
    assert!(!h.state().framework.dialogs.is_open::<HostedColorRange>(), "OK closes the dialog");
    assert!(h.state().color_range.is_none(), "and drops the preview cache");
    let app = h.state();
    assert_eq!((coverage(app, 5, 5), coverage(app, 30, 5), coverage(app, 2, 22)), (1.0, 0.0, 0.0));
    assert_eq!(app.session.active().unwrap().history.past_len(), past + 1, "one history step");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Color Range"));
    h.state_mut().run("edit.undo", json!({})).unwrap();
    let app = h.state();
    assert_eq!((coverage(app, 5, 5), coverage(app, 2, 22), coverage(app, 30, 5)), (1.0, 1.0, 0.0), "undo restores the previous selection");
}

#[test]
fn preview_mode_is_view_state_and_cancel_preserves_selection() {
    let mut app = app_with_doc();
    app.run("select.rect", json!({"x": 0, "y": 20, "width": 5, "height": 5})).unwrap();
    let original = app.session.active().unwrap();
    let (past, revision) = (original.history.past_len(), original.revision);
    let id = open(&mut app).unwrap();
    let ctx = app.ctx().clone();
    for mode in [PreviewMode::None, PreviewMode::Grayscale, PreviewMode::BlackMatte, PreviewMode::WhiteMatte, PreviewMode::QuickMask] {
        set_field(&mut app, "selectionPreview", json!(mode.id()));
        assert!(params(hosted(&app).value()).get("preview").is_none(), "the mode is not a command parameter");
        match selection_preview(&mut app, &ctx) {
            Some(p) => assert_eq!(p.mode, mode),
            None => assert!(matches!(mode, PreviewMode::None), "{mode:?} shows a preview"),
        }
        let st = app.session.active().unwrap();
        assert_eq!((st.revision, st.history.past_len()), (revision, past));
        assert_eq!(coverage(&app, 2, 22), 1.0);
    }
    app.framework.dialogs.cancel_by_id(&ctx, id);
    take(&mut app, &ctx);
    assert!(selection_preview(&mut app, &ctx).is_none(), "closing removes the preview");
    assert!(app.color_range.is_none());
    assert_eq!(coverage(&app, 2, 22), 1.0);
}

/// A sample between thumbnail pixels must use the original pixel, not its neighbour.
#[test]
fn reduced_preview_matches_the_original_eyedropper_sample() {
    let mut doc = Document::with_background("fine detail", Size::new(400, 4), ColorMode::Rgb, SampleType::U8, Color::rgb(1.0, 0.0, 0.0));
    let surface = doc.layers[0].surface_mut().unwrap();
    surface.fill_rect(GRect::new(200, 0, 400, 4), &[0.0, 0.0, 1.0, 1.0]);
    surface.fill_rect(GRect::new(101, 0, 102, 4), &[0.0, 0.0, 1.0, 1.0]);
    let mut session = photocraft_engine::Session::new();
    session.add_document(doc, None);
    let mut app = PhotocraftApp::new(session, crate::Services::default());
    open(&mut app).unwrap();
    press_release(&mut app, [101.5, 1.5], Modifiers::NONE);
    set_field(&mut app, "fuzziness", json!(0.0));
    let ctx = app.ctx().clone();
    take(&mut app, &ctx);
    let k = app.color_range.as_ref().expect("the preview is cached").k;
    assert_eq!(k, 2, "400 px at 200 pt");
    let value = hosted(&app).value().clone();
    assert_eq!(value.samples.add, vec![[101.0, 1.0]]);
    let original = app.session.active().unwrap().doc.clone();
    let proxy = crate::proxy::proxy_document(&original, k);
    let mut old = photocraft_engine::Session::new();
    old.add_document(proxy.clone(), None);
    old.execute(COMMAND, json!({"points": [[50, 0]], "fuzziness": 0})).unwrap();
    assert_eq!(
        old.active().unwrap().doc.selection.as_ref().unwrap().sample_channel(150, 0, 0),
        0.0,
        "sampling the thumbnail instead selects the red neighbour"
    );
    let shown = proxy_mask(&app, &proxy, k, &params(&value)).unwrap();
    app.run(COMMAND, params(&value)).unwrap();
    assert_eq!(coverage(&app, 300, 0), 1.0);
    assert_eq!(shown[150], coverage(&app, 300, 0), "preview must show the same blue pixels as OK");
    assert_eq!(shown[0], coverage(&app, 0, 0), "preview must leave the red background unselected");
}

/// `sampleAllLayers: false` judges the active layer in the preview, as OK does.
#[test]
fn preview_honours_sample_all_layers_like_ok() {
    let mut app = app_with_doc();
    let doc = app.session.active().unwrap().doc.clone();
    let mut top = photocraft_doc::Layer::new("blue cover", photocraft_doc::LayerContent::Raster(doc.layers[0].surface().unwrap().clone()));
    top.surface_mut().unwrap().fill_rect(GRect::new(0, 0, 40, 30), &[0.0, 0.0, 1.0, 1.0]);
    app.session
        .edit("layers", |doc, _| {
            doc.layers.push(top);
            Ok(())
        })
        .unwrap();
    for all_layers in [false, true] {
        let p = json!({"select": "sampledColors", "points": [[5, 5]], "fuzziness": 0, "sampleAllLayers": all_layers});
        let proxy = crate::proxy::proxy_document(&app.session.active().unwrap().doc, 1);
        let shown = proxy_mask(&app, &proxy, 1, &p).unwrap();
        app.run(COMMAND, p).unwrap();
        for (x, y) in [(5, 5), (30, 5), (5, 20), (30, 20)] {
            assert_eq!(shown[y * 40 + x], coverage(&app, x as i32, y as i32), "all_layers={all_layers} at ({x}, {y})");
        }
    }
}

#[test]
fn params_send_only_what_the_mode_uses() {
    let app = app_with_doc();
    let size = [40, 30];
    let mut v = ColorRangeValue::default();
    assert_eq!(params(&v), json!({"select": "sampledColors", "invert": false, "fuzziness": 40.0}));
    // Localized needs a picked point.
    v.localized = true;
    assert!(params(&v).get("localized").is_none());
    let shift = Mods { shift: true, ..Mods::default() };
    let alt = Mods { alt: true, ..Mods::default() };
    v.samples.pick([3.0, 4.0], size, SampleTool::Sample, Mods::default());
    v.samples.pick([99.0, -4.0], size, SampleTool::Sample, shift);
    v.samples.pick([1.0, 1.0], size, SampleTool::Sample, alt);
    assert_eq!(
        params(&v),
        json!({"select": "sampledColors", "invert": false, "fuzziness": 40.0, "points": [[3.0, 4.0], [39.0, 0.0]], "subtractPoints": [[1.0, 1.0]], "order": ["+", "+", "-"], "localized": true, "range": 100.0})
    );
    v.select = ColorRangeSelect::Midtones;
    v.midtones_low = 160.0;
    assert_eq!(params(&v), json!({"select": "midtones", "invert": false, "fuzziness": 20.0, "tonalRange": [150.0, 160.0]}));
    v.select = ColorRangeSelect::Highlights;
    assert_eq!(params(&v)["tonalRange"], json!(190.0));
    v.select = ColorRangeSelect::SkinTones;
    assert_eq!(params(&v), json!({"select": "skinTones", "invert": false, "fuzziness": 40.0}));
    v.select = ColorRangeSelect::Cyans;
    assert_eq!(params(&v), json!({"select": "cyans", "invert": false}));
    // Every Select entry is accepted by the engine.
    let mut app = app;
    for select in [
        ColorRangeSelect::Sampled,
        ColorRangeSelect::Reds,
        ColorRangeSelect::Yellows,
        ColorRangeSelect::Greens,
        ColorRangeSelect::Cyans,
        ColorRangeSelect::Blues,
        ColorRangeSelect::Magentas,
        ColorRangeSelect::Highlights,
        ColorRangeSelect::Midtones,
        ColorRangeSelect::Shadows,
        ColorRangeSelect::SkinTones,
        ColorRangeSelect::OutOfGamut,
    ] {
        v.select = select;
        app.run(COMMAND, params(&v)).unwrap();
    }
}

/// #145: the whole app without a GPU (no wgpu render state: the CPU canvas, as on a Linux
/// machine whose adapter can't run the GPU canvas), on a small window. Select › Color Range…
/// from the menu opens the dialog and draws both previews.
#[test]
fn opens_and_previews_in_the_full_app_without_a_gpu() {
    let mut h = Harness::builder().with_size(egui::vec2(1024.0, 600.0)).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        assert!(cc.wgpu_render_state.is_none(), "this test covers the no-GPU path");
        PhotocraftApp::with_context(cc.egui_ctx.clone(), session_with_doc(), crate::Services::default())
    });
    h.run_steps(6);
    assert!(h.state().gpu.is_none(), "the CPU canvas");
    h.get_all_by_label("Select").next().expect("the Select menu").click();
    h.run_steps(3);
    h.get_by_label("Color Range…").click();
    h.run_steps(6);
    assert!(h.state().framework.dialogs.is_open::<HostedColorRange>());
    assert!(h.query_by_label("Color Range preview").is_some());
    let p = h.state().color_range.as_ref().expect("the preview is cached");
    assert!(p.mask.is_some() && p.error.is_none(), "the selection preview is drawn: {:?}", p.error);
    assert!(p.image.is_some() && p.error.is_none(), "the image preview is drawn: {:?}", p.error);
    assert!(!hosted(h.state()).shows_image());
    set_field(h.state_mut(), "view", json!("image"));
    h.run_steps(3);
    assert!(hosted(h.state()).shows_image());
    set_field(h.state_mut(), "select", json!("reds"));
    let id = dialog_id(h.state());
    crate::dialogs::testing::confirm(h.state_mut(), id).unwrap();
    h.run_steps(3);
    assert_eq!(coverage(h.state(), 5, 5), 1.0);
}

#[test]
fn a_preview_failure_is_shown_not_silent() {
    let mut app = app_with_doc();
    open(&mut app).unwrap();
    let ctx = app.ctx().clone();
    take(&mut app, &ctx);
    assert!(app.color_range.as_ref().unwrap().error.is_none());
    // The engine refuses these params: the failure reaches the dialog's preview instead of leaving it blank.
    refresh(&mut app, &json!({"select": "noSuchMode"}));
    assert!(app.color_range.as_ref().unwrap().error.is_some(), "the failure is recorded");
    refresh(&mut app, &json!({"select": "reds"}));
    assert!(app.color_range.as_ref().unwrap().error.is_none(), "and cleared once the preview works again");
}
