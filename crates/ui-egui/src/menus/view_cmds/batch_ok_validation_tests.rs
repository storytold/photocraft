//! File › Automate › Batch: OK checks for a missing source folder, then a missing destination
//! folder, as the reference does (#13669). The first that fails shows its alert titled "Batch"
//! with OK over the dialog, which stays open with every value, and nothing runs. The same holds
//! for a confirm through the control channel.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Map, Value, json};

use super::{BATCH_ALERT_TITLE, NO_DESTINATION_FOLDER, NO_SOURCE_FOLDER};
use crate::PhotocraftApp;
use crate::control::{ControlRequest, Outcome, handle};
use crate::state::DialogKind;

type H = Harness<'static, PhotocraftApp>;

/// The Batch dialog, opened from its menu command over an unsaved document with one recorded
/// action (Invert), with `input` and `output` set to the given paths.
fn batch(input: &str, output: &str) -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 2, "height": 1})).unwrap();
    app.run("actions.record", json!({"name": "Invert"})).unwrap();
    app.run("image.adjustments.invert", json!({})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 800.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.automate.batch", json!({})).expect("the Batch dialog opens");
    let fields = batch_fields_mut(&mut h);
    fields.insert("input".into(), json!(input));
    fields.insert("output".into(), json!(output));
    h.run_steps(4);
    h
}

/// The open Batch dialog's id.
fn batch_id(h: &H) -> u64 {
    h.state().ui.dialogs.iter().find(|d| d.fields.get("__command").and_then(Value::as_str) == Some("file.automate.batch")).expect("the Batch dialog is open").id
}

fn batch_fields_mut(h: &mut H) -> &mut Map<String, Value> {
    let id = batch_id(h);
    &mut h.state_mut().ui.dialog_mut(id).unwrap().fields
}

fn batch_fields(h: &H) -> Map<String, Value> {
    let id = batch_id(h);
    h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone()
}

/// Every open alert as (title, message).
fn alerts(h: &H) -> Vec<(String, String)> {
    h.state()
        .ui
        .dialogs
        .iter()
        .filter(|d| d.kind == DialogKind::Error)
        .map(|d| (crate::dialogs::title(d), d.fields.get("message").and_then(Value::as_str).unwrap_or_default().to_string()))
        .collect()
}

/// The alert the reference shows for `message`.
fn batch_alert(message: &str) -> Vec<(String, String)> {
    vec![(BATCH_ALERT_TITLE.to_string(), message.to_string())]
}

/// Empties a text field of the dialog the way a user does: select all, then Backspace.
fn clear_field(h: &mut H, label: &str) {
    h.get_by_role_and_label(Role::TextInput, label).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.key_press(egui::Key::Backspace);
    h.run_steps(2);
}

/// A folder holding one 2×1 PNG, `a.png`, and the path of an output folder beside it.
fn image_folder() -> (tempfile::TempDir, String, String) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("in")).unwrap();
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 2, "height": 1})).unwrap();
    session.execute("file.saveACopy", json!({"path": dir.path().join("in/a.png").to_string_lossy()})).unwrap();
    let input = dir.path().join("in").to_string_lossy().into_owned();
    let output = dir.path().join("out").to_string_lossy().into_owned();
    (dir, input, output)
}

/// Clicks the dialog's OK; with an alert up, the alert's OK is the other one.
fn click_batch_ok(h: &mut H) -> egui::Rect {
    let ok = h.get_by_role_and_label(Role::Button, "OK").rect();
    h.get_by_role_and_label(Role::Button, "OK").click();
    h.run_steps(4);
    ok
}

#[test]
fn ok_without_a_source_folder_shows_the_batch_alert_and_keeps_the_dialog_open() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (dir, input, output) = image_folder();
        let mut h = batch(&input, &output);
        clear_field(&mut h, "Input");
        let before = batch_fields(&h);
        assert_eq!((&before["input"], &before["output"]), (&json!(""), &json!(output)));

        let ok = click_batch_ok(&mut h);
        assert_eq!(alerts(&h), batch_alert(NO_SOURCE_FOLDER));
        assert_eq!(NO_SOURCE_FOLDER, "You must choose a source folder.");
        assert!(h.query_by_label(NO_SOURCE_FOLDER).is_some(), "the alert is on screen");
        assert_eq!(batch_fields(&h), before, "the dialog stays open with every value");
        assert!(!dir.path().join("out").exists(), "nothing was written");
        assert_eq!(h.state().session.documents().len(), 1, "nothing was opened");

        // The alert's OK closes the alert only; the dialog keeps its values.
        h.get_all_by_role_and_label(Role::Button, "OK").find(|b| b.rect() != ok).expect("the alert has an OK button").click();
        h.run_steps(4);
        assert!(alerts(&h).is_empty());
        assert_eq!(batch_fields(&h), before);
    });
}

#[test]
fn ok_without_a_destination_folder_shows_the_batch_alert_and_writes_nothing() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (dir, input, output) = image_folder();
        let mut h = batch(&input, &output);
        clear_field(&mut h, "Output");
        let before = batch_fields(&h);
        assert_eq!((&before["input"], &before["output"]), (&json!(input), &json!("")));

        click_batch_ok(&mut h);
        assert_eq!(alerts(&h), batch_alert(NO_DESTINATION_FOLDER));
        assert_eq!(NO_DESTINATION_FOLDER, "You must choose a destination folder.");
        assert_eq!(batch_fields(&h), before);
        let files: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(files, ["in"], "nothing was written");
        assert_eq!(h.state().session.documents().len(), 1, "nothing was opened");
    });
}

/// The checks run in the reference's order: with both folders missing, the source one shows.
/// A path of blanks names no folder either.
#[test]
fn the_source_folder_is_checked_before_the_destination() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        for (input, output, message) in [("", "", NO_SOURCE_FOLDER), ("  ", "", NO_SOURCE_FOLDER), ("in", "  ", NO_DESTINATION_FOLDER)] {
            let mut h = batch(input, output);
            click_batch_ok(&mut h);
            assert_eq!(alerts(&h), batch_alert(message), "input {input:?}, output {output:?}");
        }
    });
}

#[test]
fn a_confirm_over_the_control_channel_gets_the_same_check() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = batch("", "");
        let ctx = h.ctx.clone();
        let id = batch_id(&h);
        let before = batch_fields(&h);
        // Repeated confirms answer the same error and keep one alert.
        for _ in 0..2 {
            let (confirm, _rx) = ControlRequest::new("ui.dialog.confirm", json!({"dialog": id}));
            let Outcome::Done(reply) = handle(h.state_mut(), &ctx, &confirm) else { panic!("ui.dialog.confirm replies at once") };
            assert_eq!(reply["ok"], false, "{reply}");
            assert!(reply.to_string().contains(NO_SOURCE_FOLDER), "{reply}");
        }
        assert_eq!(alerts(&h), batch_alert(NO_SOURCE_FOLDER));
        assert_eq!(batch_fields(&h), before);

        // With the source set over the channel, the destination is checked next.
        let (set, _rx) = ControlRequest::new("ui.dialog.set", json!({"dialog": id, "field": "input", "value": "in"}));
        assert!(matches!(handle(h.state_mut(), &ctx, &set), Outcome::Done(v) if v["ok"] == true));
        let alert = h.state().ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).unwrap().id;
        crate::dialogs::confirm(h.state_mut(), alert).unwrap();
        assert_eq!(crate::dialogs::confirm(h.state_mut(), id), Err(NO_DESTINATION_FOLDER.to_string()));
        assert_eq!(alerts(&h), batch_alert(NO_DESTINATION_FOLDER));
    });
}

/// With both folders chosen OK runs the batch and closes the dialog, without an alert.
#[test]
fn ok_with_both_folders_runs_the_batch() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let (dir, input, output) = image_folder();
        let mut h = batch(&input, &output);
        click_batch_ok(&mut h);
        assert!(alerts(&h).is_empty(), "{:?}", alerts(&h));
        assert!(h.state().ui.dialogs.is_empty(), "OK closes the dialog");
        assert!(dir.path().join("out/a.png").is_file(), "the batch wrote a.png");
    });
}

/// Opened Files reads no folder, and without an output folder it leaves the documents open and
/// changed (Destination None, as before): no check applies.
#[test]
fn opened_files_without_an_output_folder_runs_without_an_alert() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = batch("", "");
        batch_fields_mut(&mut h).insert("source".into(), json!("open"));
        h.run_steps(2);
        let pixel = |h: &H| {
            let st = h.state().session.active().unwrap();
            st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(0, 0)
        };
        let before = pixel(&h);
        let id = batch_id(&h);
        let result = crate::dialogs::confirm(h.state_mut(), id).expect("the batch runs");
        assert!(alerts(&h).is_empty(), "{:?}", alerts(&h));
        assert_eq!(result["documents"], 1, "{result}");
        assert_eq!(h.state().session.documents().len(), 1, "the document stays open");
        assert_eq!(pixel(&h)[0], 1.0 - before[0], "and the action inverted it");
    });
}

/// Other alerts keep the title "Error"; only one that names its own title shows another.
#[test]
fn an_alert_without_its_own_title_is_titled_error() {
    let mut ui = crate::state::UiState::default();
    crate::panels::options_bar::range_alert(&mut ui, "A value between 1 and 300000 is required. Closest value inserted.");
    assert_eq!(crate::dialogs::title(ui.dialogs.last().unwrap()), "Error");
}
