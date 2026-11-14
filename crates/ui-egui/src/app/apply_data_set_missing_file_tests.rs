//! #14500: Apply Data Set with a missing replacement file closes the dialog and shows the
//! reference application's alert, which names no file; the status bar keeps the file's name.

use super::*;
use serde_json::json;

const MISSING: &str = "Could not apply data set because the replacement file was not found.";

/// An app with a raster layer bound to a `photo` pixel variable and one data set `Two` whose
/// image is `missing.png` in a folder that has no such file.
fn app(name: &str) -> (PhotocraftApp, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("pc-ui-vars-{}-{name}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let png = dir.join("missing.png");
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
    (app, png)
}

fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.ui.dialogs.iter().filter(|d| d.kind == state::DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(String::from)).collect()
}

#[test]
fn ok_in_the_apply_dialog_closes_it_and_shows_the_alert() {
    let (mut app, png) = app("ok");
    crate::menus::invoke(&mut app, &egui::Context::default(), "image.applyDataSet", json!({})).unwrap();
    let dialog = app.ui.dialogs.last().unwrap().id;
    assert!(crate::dialogs::confirm(&mut app, dialog).is_err());
    assert!(app.ui.dialogs.iter().all(|d| d.id != dialog), "the Apply Data Set dialog closed");
    assert_eq!(alerts(&app), [MISSING]);
    assert!(app.ui.status.contains(&*png.to_string_lossy()), "the status names the file: {}", app.ui.status);
    let _ = std::fs::remove_dir_all(png.parent().unwrap());
}

#[test]
fn the_command_shows_the_alert_without_the_file_name() {
    let (mut app, png) = app("cmd");
    let e = app.run("image.applyDataSet", json!({"name": "Two"})).unwrap_err();
    assert!(e.starts_with(MISSING) && e.contains(&*png.to_string_lossy()), "{e}");
    assert_eq!(alerts(&app), [MISSING]);
    let _ = std::fs::remove_dir_all(png.parent().unwrap());
}
