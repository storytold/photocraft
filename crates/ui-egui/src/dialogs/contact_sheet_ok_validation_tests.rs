//! #13692: a fresh Contact Sheet II dialog has no folder chosen, and OK without one shows the
//! reference application's alert over the dialog, which stays open with its values and runs
//! nothing. A chosen folder still makes the sheet.

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use serde_json::{Map, Value, json};

use super::contact_sheet_layout_tests::{H, fields, harness, harness_with, image_folder, pick, type_into};
use crate::PhotocraftApp;
use crate::state::DialogKind;

/// The message of every open alert.
fn alerts(h: &H) -> Vec<&str> {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message").and_then(Value::as_str)).collect()
}

/// The Contact Sheet II dialog's fields, if it is open.
fn sheet_dialog(h: &H) -> Option<&Map<String, Value>> {
    h.state().ui.dialogs.iter().find(|d| super::owns(&d.fields)).map(|d| &d.fields)
}

/// The dialog's Input field: what it holds and what it shows while empty.
fn input_field(h: &H) -> (String, String) {
    let node = h.get_by_role_and_label(Role::TextInput, "Input");
    let node = node.accesskit_node();
    (node.value().unwrap_or_default().to_string(), node.placeholder().unwrap_or_default().to_string())
}

fn click_ok(h: &mut H) {
    h.get_by_role_and_label(Role::Button, "OK").click();
    h.run_steps(4);
}

#[test]
fn a_fresh_dialog_has_no_folder_chosen() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let h = harness();
        assert_eq!(fields(&h)["input"], "", "not the working folder");
        assert_eq!(input_field(&h), (String::new(), "[Select Image Folder]".to_string()));

        // Nor the folder of the saved document in front.
        let mut session = photocraft_engine::Session::new();
        session.add_document(Document::with_background("a", Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::WHITE), Some("/tmp/photos/a.psd".into()));
        let h = harness_with(PhotocraftApp::new(session, crate::Services::default()));
        assert_eq!(fields(&h)["input"], "");
        assert_eq!(input_field(&h), (String::new(), "[Select Image Folder]".to_string()));
    });
}

#[test]
fn ok_without_a_folder_shows_the_alert_and_keeps_the_dialog_open() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        pick(&mut h, "Mode", "Grayscale");
        type_into(&mut h, "Columns", "3");
        let before = fields(&h).clone();
        assert_eq!((&before["mode"], &before["columns"]), (&json!("gray"), &json!(3)));

        let ok = h.get_by_role_and_label(Role::Button, "OK").rect();
        click_ok(&mut h);
        assert_eq!(alerts(&h), ["Select an Image Folder"]);
        assert!(h.query_by_label("Select an Image Folder").is_some(), "the alert is on screen");
        assert_eq!(sheet_dialog(&h), Some(&before), "the dialog stays open with every value");
        assert!(h.state().session.documents().is_empty(), "no sheet was made");

        // The alert's OK closes the alert only.
        let alert_ok = h.get_all_by_role_and_label(Role::Button, "OK").find(|b| b.rect() != ok).expect("the alert has an OK button");
        alert_ok.click();
        h.run_steps(4);
        assert!(alerts(&h).is_empty());
        assert_eq!(sheet_dialog(&h), Some(&before));
        assert_eq!(input_field(&h).0, "");

        // A confirm repeated from automation (`ui.dialog.confirm`) gets the error and keeps one alert.
        let id = h.state().ui.dialogs.iter().find(|d| super::owns(&d.fields)).map(|d| d.id).unwrap();
        for _ in 0..2 {
            assert_eq!(crate::dialogs::confirm(h.state_mut(), id), Err("Select an Image Folder".to_string()));
        }
        h.run_steps(2);
        assert_eq!(alerts(&h), ["Select an Image Folder"]);
        assert_eq!(sheet_dialog(&h), Some(&before));
        assert!(h.state().session.documents().is_empty());
    });
}

/// The folder typed into Input is what OK runs on: the sheet is made and the dialog closes.
#[test]
fn ok_with_a_folder_makes_the_sheet() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        let dir = image_folder("ok-validation");
        h.get_by_role_and_label(Role::TextInput, "Input").click();
        h.run_steps(2);
        h.get_by_role_and_label(Role::TextInput, "Input").type_text(&dir.to_string_lossy());
        h.run_steps(3);
        assert_eq!(fields(&h)["input"], json!(dir.to_string_lossy()));
        click_ok(&mut h);
        std::fs::remove_dir_all(&dir).ok();
        assert!(alerts(&h).is_empty(), "{:?}", alerts(&h));
        assert!(h.state().ui.dialogs.is_empty(), "OK closes the dialog");
        assert_eq!(h.state().session.documents().len(), 1, "one sheet");
    });
}

/// Not measured in the reference: a chosen folder that holds no images, or that is gone, still
/// reaches the command and gets its error, as before.
#[test]
fn a_folder_without_images_or_gone_still_gets_the_command_error() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let empty = std::env::temp_dir().join(format!("pc-contact-sheet-empty-{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        let gone = empty.join("gone");
        let gone_error = gone.to_string_lossy().into_owned();
        for (dir, error) in [(&empty, "no images to place"), (&gone, gone_error.as_str())] {
            let mut h = harness();
            h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("input".into(), json!(dir.to_string_lossy()));
            h.run_steps(2);
            click_ok(&mut h);
            assert!(alerts(&h).is_empty(), "{}: no alert", dir.display());
            let status = &h.state().ui.status;
            assert!(!status.is_empty() && status.contains(error), "{}: the command's error is reported, got {status:?}", dir.display());
            assert!(h.state().session.documents().is_empty());
        }
        std::fs::remove_dir_all(&empty).ok();
    });
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
