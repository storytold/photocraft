use serde_json::json;

use super::*;

fn app(dpi: f64, depth: u64) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 20, "mode": "grayscale", "depth": depth, "resolution": dpi})).unwrap();
    app
}

fn open(app: &mut PhotocraftApp) -> craft::ui::dialog::DialogId {
    let id = crate::filter_dialog::open(app, COMMAND).expect("the Bitmap dialog opens");
    // The dialog is drawn once before its fields are set and answered, as the app's frame draws it.
    crate::canvas::testing::frame(app);
    id
}

/// OK, then the frames that take its answer and run the command.
fn confirm(app: &mut PhotocraftApp, id: craft::ui::dialog::DialogId) {
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(app, &ctx, id).unwrap();
    for _ in 0..3 {
        crate::canvas::testing::frame(app);
    }
}

#[test]
fn ok_resamples_to_the_output_resolution() {
    for depth in [8, 16] {
        let mut app = app(72.0, depth);
        let ctx = app.ctx().clone();
        let id = open(&mut app);
        app.framework.dialogs.set_json_field(&ctx, id, "method", json!("threshold")).unwrap();
        app.framework.dialogs.set_json_field(&ctx, id, "resolution", json!(144)).unwrap();
        confirm(&mut app, id);
        let doc = &app.session.active().unwrap().doc;
        assert_eq!((doc.size.width, doc.size.height, doc.resolution_dpi, doc.mode), (80, 40, 144.0, photocraft_color::ColorMode::Bitmap), "{depth}");
    }
}

#[test]
fn ok_with_custom_pattern_and_halftone_screen_converts() {
    for method in ["customPattern", "halftone"] {
        let mut app = app(72.0, 8);
        let ctx = app.ctx().clone();
        let id = open(&mut app);
        let set = |app: &mut PhotocraftApp, field: &str, value| app.framework.dialogs.set_json_field(&ctx, id, field, value).unwrap();
        set(&mut app, "method", json!(method));
        if method == "customPattern" {
            let pattern = photocraft_engine::pattern_cmds::resolve(&app.session, "").expect("a pattern").id.to_string();
            set(&mut app, "pattern", json!(pattern));
        } else {
            set(&mut app, "frequency", json!(60));
            set(&mut app, "angle", json!(30));
            set(&mut app, "shape", json!("line"));
        }
        confirm(&mut app, id);
        let doc = &app.session.active().unwrap().doc;
        assert_eq!((doc.size.width, doc.mode), (40, photocraft_color::ColorMode::Bitmap), "{method}");
    }
}

#[test]
fn the_preview_keeps_the_canvas_size() {
    let app = app(72.0, 8);
    let st = app.session.active().unwrap();
    let p = json!({"method": "threshold", "resolution": 144});
    let shown = crate::filter_dialog::preview_document(&st.doc, st.active_layer, COMMAND, &p, 1).unwrap();
    assert_eq!((shown.size.width, shown.size.height), (40, 20));
    assert_eq!(shown.mode, photocraft_color::ColorMode::Bitmap);
}
