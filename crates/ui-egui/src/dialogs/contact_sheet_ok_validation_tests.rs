//! #13692: a fresh Contact Sheet II dialog has no folder chosen, and OK without one shows the
//! reference application's alert over the dialog, which stays open with its values and runs
//! nothing. A chosen folder still makes the sheet.

use craft::ui::dialog::{DialogId, HostedAlert, StackDialog};
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use serde_json::{Value, json};

use super::contact_sheet_layout_tests::{H, field, harness, image_folder, opened, set};
use crate::PhotocraftApp;

/// The open alerts: their ids and messages.
fn alerts(h: &H) -> Vec<(DialogId, String)> {
    let dialogs = &h.state().framework.dialogs;
    dialogs
        .open_dialogs()
        .into_iter()
        .filter_map(|d| {
            let alert = dialogs.get_by_id::<HostedAlert>(d.id)?;
            let message = alert.json_fields()?.get("message").and_then(Value::as_str)?.to_owned();
            Some((d.id, message))
        })
        .collect()
}

fn messages(h: &H) -> Vec<String> {
    alerts(h).into_iter().map(|(_, m)| m).collect()
}

fn is_open(h: &H, id: DialogId) -> bool {
    h.state().framework.dialogs.open_dialogs().iter().any(|d| d.id == id)
}

/// OK as the default button files it, with the refusal of the held OK as the error.
fn ok(h: &mut H, id: DialogId) -> Result<(), String> {
    let answer = crate::dialogs::testing::confirm(h.state_mut(), id);
    h.run_steps(4);
    answer
}

/// The dialog's values the user can edit.
fn values(h: &H, id: DialogId) -> Vec<Value> {
    ["input", "mode", "columns", "units", "width"].map(|key| field(h, id, key)).into()
}

#[test]
fn a_fresh_dialog_has_no_folder_chosen() {
    let (h, id) = opened();
    assert_eq!(field(&h, id, "input"), json!(""), "not the working folder");
    let rows = super::rows();
    let input = rows.iter().find(|p| p.key == "input").expect("an Input row");
    assert_eq!(input.kind.default_value().and_then(|v| serde_json::to_value(v).ok()), Some(json!("")));

    // Nor the folder of the saved document in front.
    let mut session = photocraft_engine::Session::new();
    session.add_document(Document::with_background("a", Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::WHITE), Some("/tmp/photos/a.psd".into()));
    let mut app = PhotocraftApp::new(session, crate::Services::default());
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, super::COMMAND, json!({})).unwrap();
    let id = app.framework.dialogs.open_dialogs()[0].id;
    let h = harness(app);
    assert_eq!(field(&h, id, "input"), json!(""));
}

#[test]
fn ok_without_a_folder_shows_the_alert_and_keeps_the_dialog_open() {
    let (mut h, id) = opened();
    set(&mut h, id, "mode", json!("gray"));
    set(&mut h, id, "columns", json!(3));
    let before = values(&h, id);
    let documents = h.state().session.documents().len();

    assert!(ok(&mut h, id).is_err(), "OK is refused");
    assert_eq!(messages(&h), ["Select an Image Folder"]);
    assert!(is_open(&h, id), "the dialog stays open");
    assert_eq!(values(&h, id), before, "with every value");
    assert_eq!(h.state().session.documents().len(), documents, "no sheet was made");

    // The alert's OK closes the alert only.
    let [(alert, _)] = alerts(&h).try_into().unwrap_or_else(|a: Vec<_>| panic!("one alert: {a:?}"));
    crate::dialogs::testing::confirm(h.state_mut(), alert).expect("the alert's OK");
    h.run_steps(4);
    assert!(alerts(&h).is_empty());
    assert!(is_open(&h, id));
    assert_eq!(values(&h, id), before);

    // A confirm repeated from automation (`ui.dialog.confirm`) is refused again and keeps one alert.
    for _ in 0..2 {
        assert!(ok(&mut h, id).is_err());
    }
    assert_eq!(messages(&h), ["Select an Image Folder"]);
    assert_eq!(values(&h, id), before);
    assert_eq!(h.state().session.documents().len(), documents);
}

/// The folder set in Input is what OK runs on: the sheet is made and the dialog closes.
#[test]
fn ok_with_a_folder_makes_the_sheet() {
    let (mut h, id) = opened();
    let dir = image_folder();
    set(&mut h, id, "input", json!(dir.path().to_string_lossy()));
    set(&mut h, id, "caption", json!(false));
    ok(&mut h, id).expect("OK is accepted");
    assert!(alerts(&h).is_empty(), "{:?}", alerts(&h));
    assert!(!is_open(&h, id), "OK closes the dialog");
    assert_eq!(h.state().session.documents().len(), 1, "one sheet");
}

/// Not measured in the reference: a chosen folder that holds no images, or that is gone, still
/// reaches the command and gets its error, as before.
#[test]
fn a_folder_without_images_or_gone_still_gets_the_command_error() {
    let empty = tempfile::tempdir().unwrap();
    let gone = empty.path().join("gone");
    for dir in [empty.path().to_path_buf(), gone] {
        let (mut h, id) = opened();
        set(&mut h, id, "input", json!(dir.to_string_lossy()));
        ok(&mut h, id).expect("OK reaches the command");
        assert!(alerts(&h).iter().all(|(_, m)| m != "Select an Image Folder"), "{}: no folder alert", dir.display());
        assert!(!is_open(&h, id), "{}: the dialog closed", dir.display());
        assert!(h.state().session.documents().is_empty(), "{}: no sheet", dir.display());
    }
}

/// The engine command still returns an error, not a panic, without a source.
#[test]
fn the_command_still_errors_without_input() {
    let base = json!({"units": "pixels", "width": 400, "height": 300, "resolution": 72});
    for (case, input) in [("missing", None), ("empty", Some(json!(""))), ("empty list", Some(json!([])))] {
        let mut params = base.clone();
        if let Some(input) = input {
            params["input"] = input;
        }
        let mut session = photocraft_engine::Session::new();
        assert!(session.execute(super::COMMAND, params).is_err(), "{case}");
        assert!(session.documents().is_empty(), "{case}");
    }
}
