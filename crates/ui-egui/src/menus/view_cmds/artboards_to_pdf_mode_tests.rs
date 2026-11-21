//! #13649: the Artboards to PDF dialog has the radio pair Multi-Page Document / Document Per
//! Artboard, Multi-Page Document selected on first open, and the choice reaches the command.
use crate::PhotocraftApp;
use craft::ui::dialog::DialogId;
use egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

const COMMAND: &str = "file.export.artboardsToPdf";

type H = Harness<'static, Box<PhotocraftApp>>;

/// A document named `abpdf-test` with two artboards.
fn harness() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"name": "abpdf-test", "width": 400, "height": 200})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [0, 0, 200, 150], "name": "Red Board"})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [250, 0, 100, 100], "name": "Blue Board"})).unwrap();
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
    // Every board, not only the selected one: Export Selected Artboards opens ticked while a board is selected (#13647).
    h.state_mut().framework.dialogs.set_json_field(&ctx, dialog, "artboards", serde_json::Value::Null).unwrap();
    h.run_steps(4);
    dialog
}

fn confirm(h: &mut H, dialog: DialogId) {
    let ctx = h.ctx.clone();
    crate::dialogs::confirm(h.state_mut(), &ctx, dialog).unwrap();
    h.run_steps(4);
}

fn selected(h: &H, label: &str) -> bool {
    h.get_by_role_and_label(Role::RadioButton, label).accesskit_node().toggled() == Some(Toggled::True)
}

fn listing(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

#[test]
fn multi_page_document_is_selected_on_first_open() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("abpdf-test.pdf").to_string_lossy().into_owned();
    let mut h = harness();
    let d = open(&mut h, &path);
    assert!(selected(&h, "Multi-Page Document"));
    assert!(!selected(&h, "Document Per Artboard"));
    confirm(&mut h, d);
    assert_eq!(listing(t.path()), ["abpdf-test.pdf"]);
}

#[test]
fn document_per_artboard_writes_a_file_per_board() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("abpdf-test.pdf").to_string_lossy().into_owned();
    let mut h = harness();
    let d = open(&mut h, &path);
    h.get_by_role_and_label(Role::RadioButton, "Document Per Artboard").click();
    h.run_steps(2);
    assert!(selected(&h, "Document Per Artboard"));
    assert!(!selected(&h, "Multi-Page Document"));
    confirm(&mut h, d);
    assert_eq!(listing(t.path()), ["abpdf-testBlue Board.pdf", "abpdf-testRed Board.pdf"]);
}
