use super::*;
use egui_kittest::kittest::Queryable;
use photocraft_color::{PixelFormat, SampleType};
use photocraft_raster::Surface;
use std::sync::Arc;

const ID: &str = "local.skinsmooth.skin-smooth";
const MENU: &str = "plugin.filter.local.skinsmooth.skin-smooth";

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("plugin.install", json!({"path":concat!(env!("CARGO_MANIFEST_DIR"), "/../plugins/tests/fixtures/skin-smooth.wasm")})).unwrap();
    app.run("file.new", json!({"width":32,"height":24,"background":"#b17d62"})).unwrap();
    Arc::make_mut(&mut app.session.active_mut().unwrap().doc).icc_profile = Some(photocraft_cms::builtin::Builtin::Srgb.profile().to_bytes());
    crate::menus::invoke(&mut app, &egui::Context::default(), MENU, json!({})).unwrap();
    assert!(owns(&app.ui.dialogs[0].fields));
    app
}

fn field(app: &mut PhotocraftApp, key: &str, value: Value) {
    app.ui.dialogs[0].fields.insert(key.into(), value);
}

#[test]
fn duplicate_windows_cannot_replace_the_canonical_mask_request() {
    let mut app = app();
    app.background_jobs = false;
    field(&mut app, "__skinMask", json!(true));
    let doc = app.session.active().unwrap().doc.clone();
    let id = app.ui.alloc_id();
    app.ui.windows.push(crate::state::DocWindow { id, document: 0, view: app.ui.views[0].clone(), open: true, floating: false, focused: true, position: None });
    let duplicate = egui::ViewportId::from_hash_of(("docwin", id));
    let ctx = egui::Context::default();
    let paint = |app: &mut PhotocraftApp, viewport_id, rect| {
        let mut shown = false;
        let mut input = egui::RawInput { viewport_id, ..Default::default() };
        input.viewports.entry(viewport_id).or_default();
        ctx.run_ui(input, |ui| {
            shown = overlay(app, ui.ctx(), doc.id, rect).is_some();
        })
        .textures_delta
        .clear();
        shown
    };
    assert!(paint(&mut app, egui::ViewportId::ROOT, doc.bounds()));
    let key = app.skin_smooth_mask.as_ref().unwrap().key.clone();
    let other = Rect::new(8, 4, 24, 20);
    assert!(!paint(&mut app, duplicate, other));
    assert_eq!(app.skin_smooth_mask.as_ref().unwrap().key, key);
    let window_id = egui::Id::unique("canvas-doc-window");
    ctx.data_mut(|d| d.insert_temp(window_id, duplicate));
    assert!(!paint(&mut app, egui::ViewportId::ROOT, other), "embedded duplicates share ROOT but cannot own the mask");
    ctx.data_mut(|d| d.remove::<egui::ViewportId>(window_id));
    // Undocking transfers ownership, independent of focus; closing the window restores it.
    app.ui.windows[0].floating = true;
    app.ui.windows[0].focused = false;
    assert!(!paint(&mut app, egui::ViewportId::ROOT, doc.bounds()));
    assert!(paint(&mut app, duplicate, other));
    ctx.data_mut(|d| d.insert_temp(window_id, duplicate));
    assert!(paint(&mut app, egui::ViewportId::ROOT, other), "an embedded floating window must still show its mask");
    ctx.data_mut(|d| d.remove::<egui::ViewportId>(window_id));
    app.ui.windows[0].open = false;
    assert!(paint(&mut app, egui::ViewportId::ROOT, doc.bounds()));
    assert!(!paint(&mut app, duplicate, other));
}

#[test]
fn sample_swatch_shows_encoded_source_rgb_without_a_second_gamma_conversion() {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    let fields = json!({POSITIVE:[[128.0f32/255.0,64.0f32/255.0,32.0f32/255.0]]});
    let mut output = ctx.run_ui(Default::default(), |ui| swatches(ui, fields.as_object().unwrap(), POSITIVE, "Include", "None"));
    output.textures_delta.clear();
    assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == Color32::from_rgb(128,64,32))));
}

#[test]
fn proxy_mask_clips_partial_pixels_to_the_document_edge() {
    let mut app = app();
    Arc::make_mut(&mut app.session.active_mut().unwrap().doc).size = photocraft_geom::Size::new(2501, 1201);
    if let Some(view) = app.ui.views.first_mut() {
        view.zoom = 0.25;
    }
    let mut fields = app.ui.dialogs[0].fields.clone();
    bind(&app, &mut fields);
    app.ui.dialogs[0].fields = fields;
    field(&mut app, "__skinMask", json!(true));
    let doc = app.session.active().unwrap().doc.clone();
    let (_, area, uv) = overlay(&mut app, &egui::Context::default(), doc.id, doc.bounds()).unwrap();
    assert_eq!(area, doc.bounds());
    assert!(uv.max.x <= 1.0 && uv.max.y <= 1.0);
}

#[test]
fn control_pointer_samples_and_does_not_paint_through_the_dialog() {
    let mut app = app();
    field(&mut app, "__skinTool", json!("sample"));
    let original = app.session.active().unwrap().doc.clone();
    let (request, _) = crate::control::ControlRequest::new("ui.pointer", json!({"events":[{"kind":"down","x":2,"y":3},{"kind":"up","x":2,"y":3}]}));
    crate::control::handle(&mut app, &egui::Context::default(), &request);
    assert_eq!(colors(&app.ui.dialogs[0].fields, POSITIVE).len(), 1);
    assert!(Arc::ptr_eq(&original, &app.session.active().unwrap().doc));
}

#[test]
fn background_mask_discards_stale_samples_and_cancel_releases_it() {
    let mut app = app();
    app.background_jobs = true;
    let doc = app.session.active().unwrap().doc.clone();
    field(&mut app, "__skinMask", json!(true));
    let ctx = egui::Context::default();
    assert!(overlay(&mut app, &ctx, doc.id, doc.bounds()).is_none());
    // An old request may be wanted again after its cancellation: retry it, don't cache failure.
    let other = FilterPreviewKey {
        doc: doc.id,
        revision: 0,
        dialog: 0,
        active: None,
        target: ChannelTarget::Composite,
        command: "different-mask-request".into(),
        params: Value::Null,
        k: 1,
    };
    let worker = &mut app.skin_smooth_mask.as_mut().unwrap().worker;
    worker.supersede(&other, 0.0);
    worker.supersede(&other, 200.0);
    assert!(worker.cancelled());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while overlay(&mut app, &ctx, doc.id, doc.bounds()).is_none() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    field(&mut app, NEGATIVE, json!([[0.7, 0.5, 0.4]]));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while overlay(&mut app, &ctx, doc.id, doc.bounds()).is_none() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(app.skin_smooth_mask.as_ref().unwrap().key.as_ref().unwrap().params["values"][NEGATIVE], json!([[0.7, 0.5, 0.4]]));
    let id = app.ui.dialogs[0].id;
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert!(app.skin_smooth_mask.is_none());
    assert!(Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
}

#[test]
fn eyedroppers_read_original_layer_under_mask_and_never_modify_document() {
    let mut app = app();
    let doc = app.session.active().unwrap().doc.clone();
    let revision = app.session.active().unwrap().revision;
    let original = doc.layers[0].surface().unwrap().pixel(1, 1);
    field(&mut app, "__skinTool", json!("sample"));
    field(&mut app, "__skinMask", json!(true));
    field(&mut app, "__preview", json!(false));
    assert!(overlay(&mut app, &egui::Context::default(), doc.id, doc.bounds()).is_some());
    assert!(pick(&mut app, [1.0, 1.0], egui::Modifiers::NONE));
    assert_eq!(colors(&app.ui.dialogs[0].fields, POSITIVE), vec![<[f32; 3]>::try_from(&original[..3]).unwrap()]);
    assert!(pick(&mut app, [1.0, 1.0], egui::Modifiers::SHIFT));
    assert_eq!(colors(&app.ui.dialogs[0].fields, POSITIVE).len(), 1, "duplicate picks never grow the range strength");
    assert!(pick(&mut app, [1.0, 1.0], egui::Modifiers::ALT));
    assert_eq!(colors(&app.ui.dialogs[0].fields, NEGATIVE).len(), 1);
    assert!(pick(&mut app, [1.0, 1.0], egui::Modifiers::NONE));
    assert!(colors(&app.ui.dialogs[0].fields, NEGATIVE).is_empty(), "plain eyedropper replaces both groups");
    assert!(!pick(&mut app, [f64::NAN, 1.0], egui::Modifiers::NONE));
    assert!(!pick(&mut app, [-1.0, 1.0], egui::Modifiers::NONE));
    assert!(!pick(&mut app, [32.0, 1.0], egui::Modifiers::NONE));
    assert!(Arc::ptr_eq(&doc, &app.session.active().unwrap().doc));
    assert_eq!(revision, app.session.active().unwrap().revision);
}

#[test]
fn mask_matches_selection_alpha_offset_and_filter_weights_at_every_depth() {
    let plugin = photocraft_plugins::Plugin::load(include_bytes!("../../../../plugins/tests/fixtures/skin-smooth.wasm"), Default::default()).unwrap();
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut app = app();
        let st = app.session.active_mut().unwrap();
        let doc = Arc::make_mut(&mut st.doc);
        let mut surface = Surface::new(PixelFormat { mode: ColorMode::Rgb, sample, alpha: true });
        let extent = Rect::new(5, 3, 25, 18);
        surface.fill_rect(extent, &[0.7, 0.48, 0.36, 0.5]);
        doc.layers[0].content = LayerContent::Raster(surface);
        let mut selection = Surface::new(PixelFormat::GRAY8);
        selection.fill_rect(Rect::new(8, 7, 20, 20), &[0.4]);
        doc.selection = Some(selection);
        let doc = st.doc.clone();
        let layer = st.active_layer.unwrap();
        let mut fields = app.ui.dialogs[0].fields.clone();
        fields.insert("skinOnly".into(), json!(false));
        let mask = mask_buffer(&doc, layer, &fields, doc.bounds(), 1, &|| false).unwrap();
        let source = doc.layer(layer).unwrap().surface().unwrap();
        let alpha = source.pixel(10, 10)[3];
        let selected = doc.selection.as_ref().unwrap().pixel(10, 10)[0];
        assert!((mask.get(10, 10)[0] - alpha * selected).abs() < 1e-6);
        assert_eq!(mask.get(0, 0), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(mask.get(6, 4)[0], 0.0);
        let rgb = <[f32; 3]>::try_from(&source.pixel(10, 10)[..3]).unwrap();
        fields.insert("skinOnly".into(), json!(true));
        fields.insert(POSITIVE.into(), json!([rgb]));
        fields.insert(NEGATIVE.into(), json!([rgb]));
        let mask = mask_buffer(&doc, layer, &fields, doc.bounds(), 1, &|| false).unwrap();
        assert_eq!(mask.get(10, 10)[0], 0.0);
        let out = plugin.apply(source, doc.bounds(), doc.selection.as_ref(), &json!({"positiveColors":[rgb],"negativeColors":[rgb],"brightness":100})).unwrap();
        assert_eq!(out.read_region(doc.bounds()), source.read_region(doc.bounds()), "excluded pixels remain unchanged at {sample:?}");
        assert!(mask_buffer(&doc, layer, &fields, doc.bounds(), 1, &|| true).is_none());
    }
}

#[test]
fn view_switches_invalidate_mask_samples_and_presets_do_not_store_samples() {
    let mut app = app();
    field(&mut app, POSITIVE, json!([[0.7, 0.5, 0.4]]));
    field(&mut app, NEGATIVE, json!([[0.2, 0.4, 0.8]]));
    field(&mut app, "__skinMask", json!(true));
    app.run("plugin.preset.save", json!({"id":ID,"name":"Portrait","params":crate::filter_dialog::params_of(&app.ui.dialogs[0].fields)})).unwrap();
    let saved = &app.session.prefs().dialogs[&format!("plugin.presets.{ID}")]["Portrait"];
    assert!(saved.get(POSITIVE).is_none());
    assert!(saved.get(NEGATIVE).is_none());
    let fields = crate::filter_dialog::params_of(&app.ui.dialogs[0].fields);
    assert!(fields.get("__skinMask").is_none());
    assert!(fields.get("__skinSource").is_none());
    let id = app.session.active().unwrap().doc.id;
    assert!(overlay(&mut app, &egui::Context::default(), id, Rect::new(0, 0, 32, 24)).is_some());
    app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Color(0);
    assert!(supported(&app).is_err());
    tick(&mut app);
    assert!(app.skin_smooth_mask.is_none());
    assert!(colors(&app.ui.dialogs[0].fields, POSITIVE).is_empty());
    app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Composite;
    tick(&mut app);
    app.run("file.new", json!({"width":8,"height":8})).unwrap();
    tick(&mut app);
    assert!(colors(&app.ui.dialogs[0].fields, NEGATIVE).is_empty());
    assert!(!app.ui.dialogs[0].fields.contains_key("__skinMask"));
}

#[test]
fn mask_is_independent_of_preview_and_ok_only_applies_smoothing_with_undo() {
    for preview in [false, true] {
        let mut app = app();
        let doc = app.session.active().unwrap().doc.clone();
        field(&mut app, "__preview", json!(preview));
        field(&mut app, "__skinMask", json!(true));
        field(&mut app, "brightness", json!(50));
        let ctx = egui::Context::default();
        assert!(overlay(&mut app, &ctx, doc.id, doc.bounds()).is_some());
        field(&mut app, "__skinMask", json!(false));
        assert!(overlay(&mut app, &ctx, doc.id, doc.bounds()).is_none());
        field(&mut app, "__skinMask", json!(true));
        let d = app.ui.dialogs[0].id;
        super::super::confirm(&mut app, d).unwrap();
        tick(&mut app);
        assert!(app.skin_smooth_mask.is_none());
        let applied = app.session.active().unwrap().doc.layers[0].surface().unwrap().pixel(1, 1);
        assert!(applied[0] > applied[1] && applied[1] > applied[2], "OK never commits grayscale mask pixels");
        assert_ne!(applied, doc.layers[0].surface().unwrap().pixel(1, 1));
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers[0].surface().unwrap().pixel(1, 1), doc.layers[0].surface().unwrap().pixel(1, 1));
        crate::menus::invoke(&mut app, &ctx, MENU, json!({})).unwrap();
        assert!(colors(&app.ui.dialogs[0].fields, POSITIVE).is_empty());
    }
}

#[test]
fn sampling_controls_and_fixed_footer_fit_all_themes_and_small_windows() {
    for theme in crate::theme::ThemeKind::ALL {
        for size in [vec2(800.0, 480.0), vec2(1280.0, 800.0)] {
            let mut state = app();
            state.ui.dialogs.clear();
            let mut h = egui_kittest::Harness::builder().with_size(size).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), state);
            PhotocraftApp::setup_context(&h.ctx, theme);
            let ctx = h.ctx.clone();
            crate::menus::invoke(h.state_mut(), &ctx, MENU, json!({})).unwrap();
            h.run_steps(12);
            let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            for label in ["New…", "Delete", "Reset preset", "Preview", "Show Mask", "OK", "Cancel"] {
                assert!(bounds.contains_rect(h.get_by_label(label).rect()), "{theme:?} {size:?}: {label}");
            }
            if size.y > 480.0 {
                for label in ["Sample skin colour", "Add skin colour (Shift)", "Subtract skin colour (Alt)", "Clear samples"] {
                    assert!(bounds.contains_rect(h.get_by_label(label).rect()), "{theme:?}: {label}");
                }
            }
            h.get_by_label("Show Mask").click();
            h.run_steps(3);
            assert_eq!(h.state().ui.dialogs[0].fields["__skinMask"], true);
            assert_eq!(h.state().ui.dialogs[0].fields["__preview"], false);
        }
    }
}

#[test]
fn non_srgb_sources_disable_sampling_and_mask() {
    let mut app = app();
    field(&mut app, "__skinTool", json!("sample"));
    let layer = app.session.active().unwrap().active_layer.unwrap();
    for profile in [Some(photocraft_cms::builtin::Builtin::DisplayP3.profile().to_bytes()), Some(Arc::new(vec![0; 128]))] {
        Arc::make_mut(&mut app.session.active_mut().unwrap().doc).icc_profile = profile;
        assert!(supported(&app).is_err());
        assert!(!pick(&mut app, [2.0, 3.0], egui::Modifiers::default()));
        let doc = &app.session.active().unwrap().doc;
        assert!(mask_buffer(doc, layer, &app.ui.dialogs[0].fields, doc.bounds(), 1, &|| false).is_none());
    }
}

#[test]
fn untagged_skin_sampling_and_mask_follow_the_host_default_profile() {
    let mut app = app();
    Arc::make_mut(&mut app.session.active_mut().unwrap().doc).icc_profile = None;
    field(&mut app, "__skinTool", json!("sample"));
    assert!(supported(&app).is_ok());
    assert!(pick(&mut app, [2.0, 3.0], egui::Modifiers::default()));
    let state = app.session.active().unwrap();
    let mask = mask_buffer(&state.doc, state.active_layer.unwrap(), &app.ui.dialogs[0].fields, state.doc.bounds(), 1, &|| false).unwrap();
    assert!(mask.get(2, 3)[0] > 0.0);
}
