//! #13657: Artboards to PDF with a destination folder that doesn't exist keeps its dialog open with
//! its values; a finished export says it was successful, with the PDF's path.

use craft::ui::dialog::{DialogId, HostedAlert, StackDialog};
use egui_kittest::Harness;
use serde_json::{Value, json};

use crate::PhotocraftApp;

const COMMAND: &str = "file.export.artboardsToPdf";

type H = Harness<'static, Box<PhotocraftApp>>;

/// A 64 × 64 document with one artboard.
fn harness() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [0, 0, 64, 64]})).unwrap();
    app.sync_views();
    let h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h
}

/// Opens the dialog from its menu entry with File set to `path`.
fn open(h: &mut H, path: &str) -> DialogId {
    let ctx = h.ctx.clone();
    let reply = crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    let want = reply["dialog"].as_str().unwrap().to_owned();
    let dialog = h.state().framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|d| d.value().to_string() == want).unwrap();
    h.run_steps(4);
    h.state_mut().framework.dialogs.set_json_field(&ctx, dialog, "path", json!(path)).unwrap();
    h.run_steps(4);
    dialog
}

fn confirm(h: &mut H, dialog: DialogId) {
    let ctx = h.ctx.clone();
    let _ = crate::dialogs::confirm(h.state_mut(), &ctx, dialog);
    h.run_steps(4);
}

fn is_open(h: &H, dialog: DialogId) -> bool {
    h.state().framework.dialogs.open_dialogs().into_iter().any(|d| d.id == dialog)
}

/// The open alert's message, if one is up.
fn alert_message(h: &H) -> Option<String> {
    h.state().framework.dialogs.get::<HostedAlert>()?.json_fields()?.get("message").and_then(Value::as_str).map(str::to_owned)
}

#[test]
fn a_missing_destination_keeps_the_dialog_open() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("nodir").join("boards.pdf").to_string_lossy().into_owned();
    let mut h = harness();
    let d = open(&mut h, &path);
    confirm(&mut h, d);
    assert!(is_open(&h, d), "the dialog stays open");
    assert_eq!(alert_message(&h), None, "nothing was exported");
    assert!(!root.path().join("nodir").exists(), "no folder is created");
}

#[test]
fn a_finished_export_says_it_was_successful_with_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("boards.pdf").to_string_lossy().into_owned();
    let mut h = harness();
    let d = open(&mut h, &path);
    confirm(&mut h, d);
    assert!(!is_open(&h, d), "the dialog closes");
    assert!(std::fs::read(&path).unwrap().starts_with(b"%PDF"));
    assert_eq!(alert_message(&h), Some(format!("Artboards To PDF was successful.\n{path}")));
}
