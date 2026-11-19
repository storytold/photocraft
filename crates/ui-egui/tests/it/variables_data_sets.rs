use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, Reply};
use serde_json::{Value, json};

type H = Harness<'static, Box<PhotocraftApp>>;

fn harness(app: Box<PhotocraftApp>) -> H {
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            photocraft_ui_egui::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

/// One control request, as a transport sends it.
fn control(h: &mut H, method: &str, params: Value) -> Value {
    let (request, _) = ControlRequest::new(method, params);
    let ctx = h.ctx.clone();
    match photocraft_ui_egui::control::handle(h.state_mut(), &ctx, &request) {
        Reply::Done(value) => value,
        _ => panic!("{method} did not answer at once"),
    }
}

fn refused(reply: &Value) -> bool {
    reply.get("error").is_some() || reply["ok"] == json!(false)
}

/// Opens a menu's dialog and returns its id.
fn open(h: &mut H, menu: &str) -> String {
    let ctx = h.ctx.clone();
    let reply = photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, menu, json!({})).unwrap();
    h.run_steps(3);
    reply["dialog"].as_str().expect("the menu opens a dialog").to_string()
}

fn set(h: &mut H, id: &str, field: &str, value: Value) -> Value {
    let reply = control(h, "ui.dialog.set", json!({"dialog": id, "field": field, "value": value}));
    h.run_steps(2);
    reply
}

/// The open dialog's fields, as `ui.inspect` lists them.
fn inspect(h: &mut H, id: &str) -> Value {
    let listed = control(h, "ui.inspect", json!({}));
    // PLAN: assumed the `ui.inspect` entry of a hosted dialog keeps its values under `fields`.
    listed["result"]["dialogs"].as_array().into_iter().flatten().find(|d| d["id"] == json!(id)).map(|d| d["fields"].clone()).expect("the dialog is listed")
}

fn confirm(h: &mut H, id: &str) {
    assert!(!refused(&control(h, "ui.dialog.confirm", json!({"dialog": id}))));
    h.run_steps(3);
}

fn cancel(h: &mut H, id: &str) {
    assert!(!refused(&control(h, "ui.dialog.cancel", json!({"dialog": id}))));
    h.run_steps(3);
}

fn read_pixel(h: &mut H) -> Value {
    h.state_mut().run("document.pixel", json!({"x": 4, "y": 4})).unwrap()
}

fn revision(h: &H) -> u64 {
    h.state().session.active().unwrap().revision
}

fn blue_row(dir: &tempfile::TempDir, name: &str) -> Value {
    json!({"name": name, "values": [{"variable": "photo", "kind": "pixels", "value": dir.path().join("blue.png")}]})
}

/// A real image file and an open Data Sets dialog exercise the same path as manual entry.
fn pixel_replacement_harness() -> (tempfile::TempDir, H, String) {
    let dir = tempfile::tempdir().unwrap();
    for (name, color) in [("blue.png", "#0000ff"), ("red.png", "#ff0000")] {
        let mut source = photocraft_engine::Session::new();
        source.execute("file.new", json!({"width": 8, "height": 8, "background": color})).unwrap();
        let png = photocraft_io::export(&source.active().unwrap().doc, "png", &photocraft_io::ExportOptions::default()).unwrap().bytes;
        std::fs::write(dir.path().join(name), png).unwrap();
    }
    #[cfg(not(target_arch = "wasm32"))]
    photocraft_raster::spill::use_test_volume_space();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), photocraft_ui_egui::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let mut h = harness(app);
    let id = open(&mut h, "image.variables.dataSets");
    set(&mut h, &id, "defs", json!([{"name": "photo", "layer": layer.0, "type": "pixelReplacement", "method": "conform"}]));
    set(&mut h, &id, "dataSets", json!([blue_row(&dir, "Blue")]));
    (dir, h, id)
}

/// OK saves the draft; the Apply Data Set dialog opens on the saved rows.
fn save_draft_and_open_apply_menu(h: &mut H, id: &str) -> String {
    confirm(h, id);
    assert!(!h.state().dialog_open());
    open(h, "image.applyDataSet")
}

#[test]
fn apply_menu_ok_requires_a_row_but_not_variable_definitions() {
    for (rows, expected) in [(json!([]), None), (json!([{"name": "Empty", "values": []}]), Some(0))] {
        #[cfg(not(target_arch = "wasm32"))]
        photocraft_raster::spill::use_test_volume_space();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), photocraft_ui_egui::Services::default());
        app.run("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
        app.run("image.variables.dataSets", json!({"dataSets": rows})).unwrap();
        let mut h = harness(app);
        let id = open(&mut h, "image.applyDataSet");
        h.state_mut().ui.status.clear();
        confirm(&mut h, &id);
        assert!(!h.state().dialog_open());
        assert_eq!(h.state().session.active().unwrap().doc.variables.active, expected);
        assert!(h.state().ui.status.is_empty(), "{}", h.state().ui.status);
    }
}

/// Apply updates the canvas immediately; OK must not apply the same request again.
#[test]
fn apply_replaces_pixels_before_ok_and_keeps_the_dialog_open() {
    let (_dir, mut h, id) = pixel_replacement_harness();
    h.run_steps(3);
    assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]), "editing fields alone is not Apply");
    h.get_by_label("Apply").click();
    h.run_steps(3);

    assert!(h.state().dialog_open(), "Apply leaves the editor open");
    let pixel = read_pixel(&mut h);
    assert_eq!(pixel, json!([0.0, 0.0, 1.0, 1.0]), "Apply uses the edited definitions and image path immediately");
    confirm(&mut h, &id);
    assert_eq!(read_pixel(&mut h), pixel, "OK does not apply again");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]), "one history entry");
}

/// Duplicate labels must not change which row Apply or OK sends to the engine.
#[test]
fn duplicate_names_apply_the_selected_row_for_apply_and_ok() {
    for button in ["Apply", "OK"] {
        let (dir, mut h, id) = pixel_replacement_harness();
        set(&mut h, &id, "dataSets", json!([{"name": "Blue", "values": []}, blue_row(&dir, "Blue")]));
        let apply = save_draft_and_open_apply_menu(&mut h, &id);
        assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]), "editing data sets only saves them");
        h.get_by_label("Next Data Set").click();
        h.run_steps(2);
        assert_eq!(inspect(&mut h, &apply)["cur"], 1);
        h.get_by_label(button).click();
        h.run_steps(3);
        assert_eq!(read_pixel(&mut h), json!([0.0, 0.0, 1.0, 1.0]), "{button}");
        assert_eq!(h.state().session.active().unwrap().doc.variables.active, Some(1), "{button}");
    }
}

#[test]
fn apply_menu_apply_then_ok_keeps_one_history_entry() {
    let (_dir, mut h, id) = pixel_replacement_harness();
    let apply = save_draft_and_open_apply_menu(&mut h, &id);
    h.get_by_label("Apply").click();
    h.run_steps(3);
    assert_eq!(read_pixel(&mut h), json!([0.0, 0.0, 1.0, 1.0]));
    let before = revision(&h);
    confirm(&mut h, &apply);
    assert_eq!(revision(&h), before);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]));
}

#[test]
fn apply_menu_ok_honors_changes_after_apply() {
    for change in ["row", "value", "document"] {
        let (dir, mut h, id) = pixel_replacement_harness();
        let red = json!(dir.path().join("red.png"));
        if change == "row" {
            let mut row = blue_row(&dir, "Blue");
            row["values"][0]["value"] = red.clone();
            set(&mut h, &id, "dataSets", json!([blue_row(&dir, "Blue"), row]));
        }
        let apply = save_draft_and_open_apply_menu(&mut h, &id);
        h.get_by_label("Apply").click();
        h.run_steps(3);
        if change == "row" {
            h.get_by_label("Next Data Set").click();
        } else if change == "value" {
            let mut row = blue_row(&dir, "Blue");
            row["values"][0]["value"] = red;
            set(&mut h, &apply, "dataSets", json!([row]));
        } else {
            h.state_mut().run("image.adjustments.invert", json!({})).unwrap();
        }
        h.run_steps(3);
        let before = if change == "document" { json!([1.0, 1.0, 0.0, 1.0]) } else { json!([0.0, 0.0, 1.0, 1.0]) };
        assert_eq!(read_pixel(&mut h), before, "{change}: only document edits change pixels before OK");
        confirm(&mut h, &apply);
        let after = if change == "document" { json!([0.0, 0.0, 1.0, 1.0]) } else { json!([1.0, 0.0, 0.0, 1.0]) };
        assert_eq!(read_pixel(&mut h), after, "{change}");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(read_pixel(&mut h), before);
    }
}

#[test]
fn apply_menu_ok_after_deleting_the_last_row_only_saves() {
    let (_dir, mut h, id) = pixel_replacement_harness();
    let apply = save_draft_and_open_apply_menu(&mut h, &id);
    h.get_by_label("Delete").click();
    h.run_steps(2);
    h.state_mut().ui.status.clear();
    confirm(&mut h, &apply);
    assert!(!h.state().dialog_open());
    assert!(h.state().ui.status.is_empty(), "{}", h.state().ui.status);
    assert!(h.state().session.active().unwrap().doc.variables.data_sets.is_empty());
    assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]));
}

#[test]
fn variables_editor_ok_only_saves_configuration() {
    for menu in ["image.variables.define", "image.variables.dataSets"] {
        let (_dir, mut h, id) = pixel_replacement_harness();
        let state = inspect(&mut h, &id);
        cancel(&mut h, &id);
        let reopened = open(&mut h, menu);
        set(&mut h, &reopened, "defs", state["defs"].clone());
        set(&mut h, &reopened, "dataSets", state["dataSets"].clone());
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert!(!h.state().dialog_open());
        assert_eq!(h.state().session.active().unwrap().doc.variables.data_sets.len(), 1, "{menu}");
        assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]), "{menu}");
    }
}

/// A failed request reports once; correcting it can be applied and kept when Cancel closes.
#[test]
fn failed_apply_can_be_corrected_without_retries_or_cancel_rollback() {
    for apply_mode in [false, true] {
        let (dir, mut h, id) = pixel_replacement_harness();
        let id = if apply_mode { save_draft_and_open_apply_menu(&mut h, &id) } else { id };
        let missing = json!([{"name": "Blue", "values": [{"variable": "photo", "kind": "pixels", "value": dir.path().join("missing.png")}]}]);
        set(&mut h, &id, "dataSets", missing);
        h.get_by_label("Apply").click();
        h.run_steps(3);
        assert!(h.state().ui.status.contains("missing.png"), "{}", h.state().ui.status);
        assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]));
        h.state_mut().ui.status.clear();
        h.run_steps(3);
        assert!(h.state().ui.status.is_empty(), "a failed Apply must not retry each frame");

        set(&mut h, &id, "dataSets", json!([blue_row(&dir, "Blue")]));
        h.get_by_label("Apply").click();
        h.run_steps(3);
        cancel(&mut h, &id);
        assert!(!h.state().dialog_open());
        assert_eq!(read_pixel(&mut h), json!([0.0, 0.0, 1.0, 1.0]));
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(read_pixel(&mut h), json!([1.0, 1.0, 1.0, 1.0]));
    }
}

/// `ui.dialog.set` takes only values the typed state holds: a malformed one is refused and the dialog keeps its state.
#[test]
fn malformed_variable_entries_are_dropped_before_an_edit() {
    for page in ["define", "dataSets"] {
        let (_dir, mut h, id) = pixel_replacement_harness();
        set(&mut h, &id, "page", json!(page));
        let before = inspect(&mut h, &id);
        for (field, bad) in [("defs", json!([42, {"name": "v1"}])), ("dataSets", json!(["bad", {"name": "B", "values": "nope"}]))] {
            assert!(refused(&set(&mut h, &id, field, bad)), "{page}: {field}");
        }
        assert_eq!(inspect(&mut h, &id), before, "{page}");
    }
}
