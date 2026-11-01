use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::state::DialogKind;
use serde_json::{Map, json};

fn app_with_variables_dialog(cur: u64) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), photocraft_ui_egui::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let state = json!({
        "defs": [{
            "name": "v1",
            "layer": 0,
            "type": "visibility",
            "method": "fit",
            "align": "center",
            "clip": false
        }],
        "dataSets": [{"name": "A", "values": []}]
    });
    let mut fields = Map::new();
    fields.insert("__variables".into(), state);
    fields.insert("__page".into(), json!("dataSets"));
    fields.insert("__cur".into(), json!(cur));
    app.ui.open_dialog(DialogKind::Command, fields);
    app
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            photocraft_ui_egui::dialogs::show(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, photocraft_ui_egui::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    h
}

#[test]
fn out_of_range_data_set_index_does_not_overflow_during_render() {
    let mut h = harness(app_with_variables_dialog(u64::MAX));
    h.run_steps(2);
}

#[test]
fn out_of_range_data_set_index_is_clamped_before_delete() {
    let mut h = harness(app_with_variables_dialog(999));
    h.get_by_label("Delete").click();
    h.run_steps(2);
}

/// A real image file and unsaved dialog edits exercise the same path as manual entry.
fn pixel_replacement_harness() -> (tempfile::TempDir, Harness<'static, PhotocraftApp>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("blue.png");
    let mut source = photocraft_engine::Session::new();
    source.execute("file.new", json!({"width": 8, "height": 8, "background": "#0000ff"})).unwrap();
    let png = photocraft_io::export(&source.active().unwrap().doc, "png", &photocraft_io::ExportOptions::default()).unwrap().bytes;
    std::fs::write(&path, png).unwrap();

    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), photocraft_ui_egui::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    photocraft_ui_egui::menus::invoke(&mut app, &egui::Context::default(), "image.variables.dataSets", json!({})).unwrap();
    let dialog = app.ui.dialogs.last_mut().unwrap();
    dialog.fields.insert(
        "__variables".into(),
        json!({
            "defs": [{"name": "photo", "layer": layer.0, "type": "pixelReplacement", "method": "conform"}],
            "dataSets": [{"name": "Blue", "values": [{"variable": "photo", "kind": "pixels", "value": path}]}]
        }),
    );
    (dir, harness(app))
}

/// Apply updates the canvas immediately; OK must not apply the same request again.
#[test]
fn apply_replaces_pixels_before_ok_and_keeps_the_dialog_open() {
    let (_dir, mut h) = pixel_replacement_harness();
    let id = h.state().ui.dialogs.last().unwrap().id;
    h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("__apply".into(), json!("Blue"));
    h.run_steps(3);
    assert_eq!(h.state_mut().run("document.pixel", json!({"x": 4, "y": 4})).unwrap(), json!([1.0, 1.0, 1.0, 1.0]), "editing fields alone is not Apply");
    h.get_by_label("Apply").click();
    h.run_steps(3);

    assert_eq!(h.state().ui.dialogs.last().unwrap().id, id, "Apply leaves the editor open");
    assert!(!h.state().ui.dialogs.last().unwrap().fields.contains_key("__apply"));
    let pixel = h.state_mut().run("document.pixel", json!({"x": 4, "y": 4})).unwrap();
    h.get_by_label("OK").click();
    h.run_steps(3);
    let confirmed = h.state_mut().run("document.pixel", json!({"x": 4, "y": 4})).unwrap();
    assert_eq!(confirmed, json!([0.0, 0.0, 1.0, 1.0]), "the same replacement works through OK");
    assert_eq!(pixel, confirmed, "Apply uses the edited definitions and image path immediately");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(h.state_mut().run("document.pixel", json!({"x": 4, "y": 4})).unwrap(), json!([1.0, 1.0, 1.0, 1.0]));
}

/// Duplicate labels must not change which row Apply or OK sends to the engine.
#[test]
fn duplicate_names_apply_the_selected_row_for_apply_and_ok() {
    for button in ["Apply", "OK"] {
        let (_dir, mut h) = pixel_replacement_harness();
        let fields = &mut h.state_mut().ui.dialogs.last_mut().unwrap().fields;
        let selected = fields["__variables"]["dataSets"][0].clone();
        fields["__variables"]["dataSets"] = json!([{"name": "Blue", "values": []}, selected]);
        fields.insert("__cur".into(), json!(1));
        if button == "OK" {
            fields.insert("__apply".into(), json!(1));
        }
        h.run_steps(2);
        h.get_by_label(button).click();
        h.run_steps(3);
        assert_eq!(h.state_mut().run("document.pixel", json!({"x": 4, "y": 4})).unwrap(), json!([0.0, 0.0, 1.0, 1.0]), "{button}");
        assert_eq!(h.state().session.active().unwrap().doc.variables.active, Some(1), "{button}");
    }
}

/// A failed request reports once; correcting it can be applied and kept when Cancel closes.
#[test]
fn failed_apply_can_be_corrected_without_retries_or_cancel_rollback() {
    let (dir, mut h) = pixel_replacement_harness();
    let fields = &mut h.state_mut().ui.dialogs.last_mut().unwrap().fields;
    fields["__variables"]["dataSets"][0]["values"][0]["value"] = json!(dir.path().join("missing.png"));
    h.get_by_label("Apply").click();
    h.run_steps(3);
    assert!(h.state().ui.status.contains("missing.png"), "{}", h.state().ui.status);
    assert!(!h.state().ui.dialogs.last().unwrap().fields.contains_key("__apply"));
    assert_eq!(h.state_mut().run("document.pixel", json!({"x": 4, "y": 4})).unwrap(), json!([1.0, 1.0, 1.0, 1.0]));
    h.state_mut().ui.status.clear();
    h.run_steps(3);
    assert!(h.state().ui.status.is_empty(), "a failed Apply must not retry each frame");

    let fields = &mut h.state_mut().ui.dialogs.last_mut().unwrap().fields;
    fields["__variables"]["dataSets"][0]["values"][0]["value"] = json!(dir.path().join("blue.png"));
    h.get_by_label("Apply").click();
    h.run_steps(3);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty());
    assert_eq!(h.state_mut().run("document.pixel", json!({"x": 4, "y": 4})).unwrap(), json!([0.0, 0.0, 1.0, 1.0]));
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(h.state_mut().run("document.pixel", json!({"x": 4, "y": 4})).unwrap(), json!([1.0, 1.0, 1.0, 1.0]));
}

/// `ui.dialog.set` can store any JSON in `__variables`; the editors write into its entries.
fn app_with_malformed_variables(page: &str) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), photocraft_ui_egui::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let mut fields = Map::new();
    fields.insert(
        "__variables".into(),
        json!({"defs": [42, {"name": "v1", "layer": 0, "type": "textReplacement"}], "dataSets": ["bad", {"name": "A", "values": [7, "x"]}, {"name": "B", "values": "nope"}]}),
    );
    fields.insert("__page".into(), json!(page));
    app.ui.open_dialog(DialogKind::Command, fields);
    app
}

fn type_into_first_text_field(h: &mut Harness<'static, PhotocraftApp>) {
    h.query_all_by_role(egui::accesskit::Role::TextInput).next().expect("a text field").click();
    h.run_steps(2);
    h.query_all_by_role(egui::accesskit::Role::TextInput).next().expect("a text field").type_text("z");
    h.run_steps(2);
}

#[test]
fn malformed_variable_entries_are_dropped_before_an_edit() {
    for page in ["define", "dataSets"] {
        let mut h = harness(app_with_malformed_variables(page));
        type_into_first_text_field(&mut h);
        let state = &h.state().ui.dialogs.last().unwrap().fields["__variables"];
        assert_eq!(state["defs"].as_array().unwrap().len(), 1, "{page}: {state}");
        assert_eq!(state["dataSets"].as_array().unwrap().len(), 2, "{page}: {state}");
        assert!(state["dataSets"].as_array().unwrap().iter().all(|s| s["values"].as_array().unwrap().iter().all(|v| v.is_object())), "{page}: {state}");
    }
}
