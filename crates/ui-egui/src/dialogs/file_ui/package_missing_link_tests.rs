//! #14076: File › Package with a missing linked file shows the error alert instead of asking for
//! the destination.

use craft::ui::dialog::{HostedAlert, StackDialog as _};
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use photocraft_engine::print_cmds::PACKAGE_MISSING_LINK;
use serde_json::json;

use crate::PhotocraftApp;

/// An empty folder of its own.
fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-ui-pkg-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().replace('\\', "/")
}

/// A document saved as `<dir>/doc.pcraft` with a smart object linked to `<dir>/red.png`.
fn app_with_link(dir: &str) -> (Box<PhotocraftApp>, String) {
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

/// Chooses File › Package… and returns the id of the dialog it opened.
fn choose_package(app: &mut PhotocraftApp) -> u64 {
    let ctx = app.ctx().clone();
    let r = crate::menus::invoke(app, &ctx, "file.package", json!({})).unwrap();
    r["dialog"].as_u64().expect("a dialog opened")
}

#[test]
fn a_missing_link_shows_the_alert_and_not_the_destination() {
    let dir = tmp("missing");
    let (mut app, link) = app_with_link(&dir);
    std::fs::remove_file(&link).unwrap();
    assert!(app.session.is_enabled("file.package"), "the menu item stays enabled");
    let id = choose_package(&mut app);
    let alert = app.framework.dialogs.get::<HostedAlert>().expect("the alert is open");
    assert_eq!(alert.id().value(), id);
    assert_eq!(alert.json_fields().unwrap()["message"], json!(PACKAGE_MISSING_LINK));
    assert!(app.framework.dialogs.get::<HostedParamDialog>().is_none(), "the destination is not asked for");
    assert!(!std::path::Path::new(&format!("{dir}/doc")).exists());
}

#[test]
fn links_all_there_ask_for_the_destination() {
    let dir = tmp("present");
    let (mut app, _) = app_with_link(&dir);
    let id = choose_package(&mut app);
    assert!(app.framework.dialogs.get::<HostedAlert>().is_none(), "no alert");
    let form = app.framework.dialogs.get::<HostedParamDialog>().expect("the Package form is open");
    assert_eq!(form.id().value(), id);
    assert_eq!(form.fields().text("dir"), Some(dir.as_str()));
}
