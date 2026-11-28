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

/// The id of the open dialog whose message is `message`, as `ui.inspect` lists it.
fn alert_id(h: &mut H, message: &str) -> Option<String> {
    let listed = control(h, "ui.inspect", json!({}));
    listed["result"]["dialogs"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|d| d["fields"]["message"] == json!(message))
        .and_then(|d| d["id"].as_str().map(str::to_owned))
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
    // The alert assertions use English text; don't follow the system locale.
    app.session.prefs.edit(|p| p.interface.language = "en".into());
    app.run("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let mut h = harness(app);
    let id = open(&mut h, "image.variables.dataSets");
    set(&mut h, &id, "defs", json!([{"name": "photo", "layer": layer.0, "type": "pixelReplacement", "method": "conform"}]));
    set(&mut h, &id, "dataSets", json!([blue_row(&dir, "Blue")]));
    (dir, h, id)
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

/// Duplicate labels must not change which row the Data Sets page's Apply sends to the engine.
#[test]
fn duplicate_names_apply_the_selected_row() {
    let (dir, mut h, id) = pixel_replacement_harness();
    set(&mut h, &id, "dataSets", json!([{"name": "Blue", "values": []}, blue_row(&dir, "Blue")]));
    h.get_by_label("Next Data Set").click();
    h.run_steps(2);
    assert_eq!(inspect(&mut h, &id)["cur"], 1);
    h.get_by_label("Apply").click();
    h.run_steps(3);
    assert_eq!(read_pixel(&mut h), json!([0.0, 0.0, 1.0, 1.0]));
    assert_eq!(h.state().session.active().unwrap().doc.variables.active, Some(1));
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
    let (dir, mut h, id) = pixel_replacement_harness();
    let missing = json!([{"name": "Blue", "values": [{"variable": "photo", "kind": "pixels", "value": dir.path().join("missing.png")}]}]);
    set(&mut h, &id, "dataSets", missing);
    h.get_by_label("Apply").click();
    h.run_steps(3);
    assert!(h.state().ui.status.contains("missing.png"), "{}", h.state().ui.status);
    // The missing file's alert opens on top (#14500); OK dismisses it.
    let alert = alert_id(&mut h, "Could not apply data set because the replacement file was not found.").expect("the missing-file alert is open");
    confirm(&mut h, &alert);
    assert_eq!(alert_id(&mut h, "Could not apply data set because the replacement file was not found."), None);

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
