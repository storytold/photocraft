//! #14076: File › Package with a missing linked file shows the error alert instead of asking for
//! the destination.

use photocraft_engine::print_cmds::PACKAGE_MISSING_LINK;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

/// An empty folder of its own.
fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-ui-pkg-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().replace('\\', "/")
}

/// A document saved as `<dir>/doc.pcraft` with a smart object linked to `<dir>/red.png`.
fn app_with_link(dir: &str) -> (PhotocraftApp, String) {
    let link = format!("{dir}/red.png");
    let mut src = photocraft_engine::Session::new();
    src.execute("file.new", json!({"width": 4, "height": 4, "background": "#ff0000"})).unwrap();
    src.execute("file.saveACopy", json!({"path": link})).unwrap();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    app.run("file.placeLinked", json!({"path": link})).unwrap();
    let path = format!("{dir}/doc.pcraft");
    app.run("file.saveACopy", json!({"path": path})).unwrap();
    app.session.active_mut().unwrap().path = Some(path);
    (app, link)
}

/// Chooses File › Package… and returns the dialog it opened: (kind, fields).
fn choose_package(app: &mut PhotocraftApp) -> (DialogKind, serde_json::Map<String, Value>) {
    let r = crate::menus::invoke(app, &egui::Context::default(), "file.package", json!({})).unwrap();
    let id = r["dialog"].as_u64().unwrap();
    let d = app.ui.dialogs.iter().find(|d| d.id == id).unwrap();
    (d.kind, d.fields.clone())
}

#[test]
fn a_missing_link_shows_the_alert_and_not_the_destination() {
    let dir = tmp("missing");
    let (mut app, link) = app_with_link(&dir);
    std::fs::remove_file(&link).unwrap();
    assert!(app.session.is_enabled("file.package"), "the menu item stays enabled");
    let (kind, fields) = choose_package(&mut app);
    assert_eq!(kind, DialogKind::Error);
    assert_eq!(fields["message"], json!(PACKAGE_MISSING_LINK));
    assert_eq!(app.ui.dialogs.len(), 1, "only the alert is open");
    assert!(!std::path::Path::new(&format!("{dir}/doc")).exists());
}

#[test]
fn links_all_there_ask_for_the_destination() {
    let dir = tmp("present");
    let (mut app, _) = app_with_link(&dir);
    let (kind, fields) = choose_package(&mut app);
    assert_eq!(kind, DialogKind::Command);
    assert_eq!(fields["__command"], json!("file.package"));
    assert_eq!(fields["dir"], json!(dir));
}
