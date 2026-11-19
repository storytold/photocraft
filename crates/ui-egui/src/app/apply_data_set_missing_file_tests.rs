//! #14500: Apply Data Set with a missing replacement file closes the dialog and shows the
//! reference application's alert, which names no file; the status bar keeps the file's name.

use super::*;
use serde_json::json;

const MISSING: &str = "Could not apply data set because the replacement file was not found.";

/// An app with a raster layer bound to a `photo` pixel variable and one data set `Two` whose
/// image is `missing.png` in a new, empty folder. The folder lives as long as the returned guard,
/// and is removed also when an assertion fails.
fn app() -> (Box<PhotocraftApp>, tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("missing.png");
    assert!(!png.exists());
    let mut app = PhotocraftApp::new(Session::new(), Services::default());
    app.session.execute("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    let id = app.session.execute("layer.new.layer", json!({})).unwrap()["layer"].clone();
    app.session.execute("image.variables.define", json!({"defs": [{"name": "photo", "layer": id, "type": "pixelReplacement"}]})).unwrap();
    app.session
        .execute(
            "image.variables.dataSets",
            json!({"dataSets": [{"name": "Two", "values": [{"variable": "photo", "kind": "pixels", "value": png.to_string_lossy()}]}]}),
        )
        .unwrap();
    app.sync_views();
    (app, dir, png)
}

/// The messages of the open framework alerts.
fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(String::from)).collect()
}

#[test]
fn ok_in_the_apply_dialog_closes_it_and_shows_the_alert() {
    let (mut app, _dir, png) = app();
    crate::menus::invoke(&mut app, &egui::Context::default(), "image.applyDataSet", json!({})).unwrap();
    let dialog = app.top_dialog().expect("the Apply Data Set dialog is open");
    // The hosted dialog reports the refusal through the status bar and the alert, not as OK's result.
    let _ = crate::dialogs::testing::confirm(&mut app, dialog);
    assert!(app.framework.dialogs.get_dyn(dialog).is_none(), "the Apply Data Set dialog closed");
    assert_eq!(alerts(&app), [MISSING]);
    assert!(app.ui.status.contains(&*png.to_string_lossy()), "the status names the file: {}", app.ui.status);
}

#[test]
fn the_command_shows_the_alert_without_the_file_name() {
    let (mut app, _dir, png) = app();
    let e = app.run("image.applyDataSet", json!({"name": "Two"})).unwrap_err();
    assert!(e.starts_with(MISSING) && e.contains(&*png.to_string_lossy()), "{e}");
    assert_eq!(alerts(&app), [MISSING]);
}
