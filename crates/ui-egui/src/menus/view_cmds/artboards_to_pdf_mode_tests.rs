//! #13649: the Artboards to PDF dialog has the radio pair Multi-Page Document / Document Per
//! Artboard, Multi-Page Document selected on first open, and the choice reaches the command.
use crate::PhotocraftApp;
use egui::accesskit::{Role, Toggled};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

const COMMAND: &str = "file.export.artboardsToPdf";

/// A document named `abpdf-test` with two artboards.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    });
    let app = h.state_mut();
    app.run("file.new", json!({"name": "abpdf-test", "width": 400, "height": 200})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [0, 0, 200, 150], "name": "Red Board"})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [250, 0, 100, 100], "name": "Blue Board"})).unwrap();
    app.sync_views();
    h
}

/// Opens the dialog from its menu entry with Path set to `path`.
fn open(h: &mut Harness<'_, PhotocraftApp>, path: &str) -> u64 {
    let ctx = h.ctx.clone();
    let dialog = crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("path".into(), json!(path));
    h.run_steps(4);
    dialog
}

fn selected(h: &Harness<'_, PhotocraftApp>, label: &str) -> bool {
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
    crate::dialogs::confirm(h.state_mut(), d).unwrap();
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
    crate::dialogs::confirm(h.state_mut(), d).unwrap();
    assert_eq!(listing(t.path()), ["abpdf-testBlue Board.pdf", "abpdf-testRed Board.pdf"]);
}
