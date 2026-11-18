use craft::ui::patterns::form_dialogs::HostedParamDialog;
use serde_json::json;

use super::*;

/// A 40 × 30 layer: the top half at 0.4 gray, the bottom half at 0.5, with the left half selected.
fn setup() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 15})).unwrap();
    app.run("edit.fill", json!({"color": [0.4, 0.4, 0.4]})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 15, "width": 40, "height": 15})).unwrap();
    app.run("edit.fill", json!({"color": [0.5, 0.5, 0.5]})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 30})).unwrap();
    app
}

fn gray(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)[0]
}

#[test]
fn entire_image_based_on_the_selected_area_maps_the_whole_layer() {
    let mut app = setup();
    let ctx = app.ctx().clone();
    let id = open(&mut app);
    app.framework.dialogs.set_json_field(&ctx, id, KEY, json!(ENTIRE)).unwrap();
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert!(gray(&app, 30, 1) < 0.05 && gray(&app, 30, 20) > 0.95, "outside the selection too");
}

/// Each change refuses OK with the dialog open and nothing equalized; undoing it lets OK run.
fn disturb(app: &mut PhotocraftApp, case: usize) {
    match case {
        0 => {
            app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
            app.run("edit.fill", json!({"color": [0.4, 0.4, 0.4]})).unwrap();
        }
        1 => {
            app.run("select.deselect", json!({})).unwrap();
        }
        2 => {
            app.run("layer.new.layer", json!({})).unwrap();
            app.run("edit.fill", json!({"color": [0.4, 0.4, 0.4]})).unwrap();
        }
        _ => {
            app.run("select.rect", json!({"x": 20, "y": 0, "width": 20, "height": 30})).unwrap();
        }
    }
}

fn restore(app: &mut PhotocraftApp, case: usize, layer: &serde_json::Value) {
    match case {
        0 => assert!(app.session.set_active(0)),
        2 => {
            app.run("layer.select", json!({ "layer": layer })).unwrap();
        }
        _ => {
            app.run("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 30})).unwrap();
        }
    }
}

#[test]
fn ok_refuses_while_the_document_layer_or_selection_differ() {
    for case in 0..4 {
        let mut app = setup();
        let ctx = app.ctx().clone();
        let layer = json!(app.session.active().unwrap().active_layer.unwrap().0);
        let id = open(&mut app);
        disturb(&mut app, case);
        assert!(crate::dialogs::confirm(&mut app, &ctx, id).is_err(), "case {case}");
        assert!(app.framework.dialogs.is_open::<HostedParamDialog>(), "case {case}: the dialog stays open");
        assert!(app.session.set_active(0) || case != 0);
        assert!((gray(&app, 1, 1) - 0.4).abs() < 0.01 && (gray(&app, 30, 20) - 0.5).abs() < 0.01 || case == 2 || case == 0, "case {case}: nothing equalized");
        restore(&mut app, case, &layer);
        crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
        assert!(gray(&app, 1, 1) < 0.05, "case {case}: OK runs once the dialog's state is back");
    }
}
