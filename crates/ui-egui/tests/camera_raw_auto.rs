use egui_kittest::{Harness, kittest::Queryable};
use photocraft_ui_egui::{PhotocraftApp, camera_raw_ui, control, theme::ThemeKind};
use serde_json::{Value, json};

fn fixture() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":64,"height":32,"depth":16})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("fixture", |doc, id| {
            let surf = doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
            let fmt = surf.format();
            for y in 0..32 {
                for x in 0..64 {
                    let v = 0.06 + x as f32 / 64.0 * 0.22;
                    surf.write_pixel(x, y, &photocraft_raster::from_rgba(&fmt, [v, v * 0.9, v * 0.85, 1.0]));
                }
            }
            Ok(())
        })
        .unwrap();
    app
}

fn invoke(app: &mut PhotocraftApp, ctx: &egui::Context, ui: Value) -> Value {
    camera_raw_ui::menu(app, ctx, "filter.cameraRaw", &json!({"ui":ui})).unwrap().unwrap()
}

#[test]
fn auto_updates_sliders_and_preview_preserves_white_balance_and_commits_once() {
    let ctx = egui::Context::default();
    let mut app = fixture();
    let original = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    let before = invoke(&mut app, &ctx, json!({"set":{"temperature":23,"tint":-11,"texture":12}}));
    let after = invoke(&mut app, &ctx, json!({"auto":true}));
    assert!(after["params"]["exposure"].as_f64().unwrap() > 0.0);
    for key in ["temperature", "tint", "texture"] {
        assert_eq!(after["params"][key], before["params"][key]);
    }
    assert_ne!(after["histogram"]["red"], before["histogram"]["red"]);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    let repeat = invoke(&mut app, &ctx, json!({"auto":true}));
    assert_eq!(after["params"], repeat["params"]);
    assert_eq!(after["previewRevision"], repeat["previewRevision"]);
    assert!(camera_raw_ui::menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"auto":"yes"}})).unwrap().is_err());
    assert!(camera_raw_ui::menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"auto":true,"view":{"unknown":true}}})).unwrap().is_err());
    assert_eq!(control::inspect(&app, &ctx)["cameraRaw"]["params"], after["params"]);
    invoke(&mut app, &ctx, json!({"cancel":true}));
    assert!(std::sync::Arc::ptr_eq(&original, &app.session.active().unwrap().doc));
    invoke(&mut app, &ctx, json!({"auto":true,"commit":true}));
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert!(std::sync::Arc::ptr_eq(&original, &app.session.active().unwrap().doc));
}

#[test]
fn auto_button_uses_the_same_command_as_automation() {
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let mut app = fixture();
        app.session.prefs.edit(|p| p.interface.language = "en".into());
        camera_raw_ui::open(&mut app, &cc.egui_ctx).unwrap();
        app
    });
    h.run_steps(4);
    let steps = h.state().session.active().unwrap().history.past_len();
    h.get_by_label("Auto").click();
    h.run_steps(4);
    let state = control::inspect(h.state(), &h.ctx);
    assert!(state["cameraRaw"]["params"]["exposure"].as_f64().unwrap() > 0.0);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps);
}

#[test]
fn auto_edits_an_existing_smart_filter_without_adding_another_filter() {
    let ctx = egui::Context::default();
    let mut app = fixture();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("filter.cameraRaw", json!({"temperature":15,"tint":-4,"texture":9})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    camera_raw_ui::open_smart_filter(&mut app, &ctx, layer, 0).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    let state = invoke(&mut app, &ctx, json!({"auto":true}));
    assert_eq!(state["params"]["temperature"], 15.0);
    assert_eq!(state["params"]["tint"], -4.0);
    assert_eq!(state["params"]["texture"], 9.0);
    assert!(state["params"]["exposure"].as_f64().unwrap() > 0.0);
    invoke(&mut app, &ctx, json!({"commit":true}));
    let doc = &app.session.active().unwrap().doc;
    let photocraft_doc::LayerContent::Smart(sm) = &doc.layer(layer).unwrap().content else { panic!("not a smart object") };
    assert_eq!(sm.smart_filters.len(), 1);
    assert_eq!(sm.smart_filters[0].params["exposure"], state["params"]["exposure"]);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
}
