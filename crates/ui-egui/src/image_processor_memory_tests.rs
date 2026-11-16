//! #13905: Image Processor opens with the values of its last run (kept with the preferences);
//! Cancel and Escape keep the ones from before; a stored value that doesn't fit is ignored.
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

const IP: &str = "file.scripts.imageProcessor";

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    app
}

fn open(app: &mut PhotocraftApp) -> u64 {
    crate::menus::invoke(app, &egui::Context::default(), IP, json!({})).unwrap()["dialog"].as_u64().unwrap()
}

fn fields(app: &PhotocraftApp, id: u64) -> Map<String, Value> {
    app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// A run's worth of changes: folder source with sub-folders, JPEG quality 9, PSD and a resized
/// LZW TIFF on.
fn change(app: &mut PhotocraftApp, id: u64, input: &str, output: &str) {
    let f = &mut app.ui.dialog_mut(id).unwrap().fields;
    f.insert("source".into(), json!("folder"));
    f.insert("input".into(), json!(input));
    f.insert("output".into(), json!(output));
    f.insert("includeSubfolders".into(), json!(true));
    f.insert("includeIcc".into(), json!(false));
    f["jpeg"]["quality"] = json!(9);
    f["psd"]["enabled"] = json!(true);
    f["tiff"] = json!({"enabled": true, "lzw": true, "resize": true, "width": 800, "height": 600});
}

#[test]
fn the_next_open_shows_the_last_runs_values_even_after_a_failed_run() {
    let tmp = tempfile::tempdir().unwrap();
    let (input, output) = (tmp.path().join("no images here"), tmp.path().join("out"));
    std::fs::create_dir_all(&input).unwrap();
    let mut app = app();
    let id = open(&mut app);
    change(&mut app, id, &input.to_string_lossy(), &output.to_string_lossy());
    let ran = fields(&app, id);
    let _ = crate::dialogs::confirm(&mut app, id); // No images: the run fails, the values stay.
    let id = open(&mut app);
    assert_eq!(fields(&app, id), ran);
}

#[test]
fn cancel_keeps_the_values_from_before() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app();
    let id = open(&mut app);
    let defaults = fields(&app, id);
    change(&mut app, id, &tmp.path().to_string_lossy(), &tmp.path().join("o").to_string_lossy());
    crate::dialogs::cancel(&mut app, id).unwrap();
    let id = open(&mut app);
    assert_eq!(fields(&app, id), defaults, "nothing stored by Cancel");
    // After a run, Cancel keeps that run's values.
    change(&mut app, id, &tmp.path().to_string_lossy(), &tmp.path().join("o").to_string_lossy());
    let ran = fields(&app, id);
    let _ = crate::dialogs::confirm(&mut app, id);
    let id = open(&mut app);
    app.ui.dialog_mut(id).unwrap().fields["jpeg"]["quality"] = json!(12);
    crate::dialogs::cancel(&mut app, id).unwrap();
    let id = open(&mut app);
    assert_eq!(fields(&app, id), ran);
}

#[test]
fn kept_across_a_restart_and_a_missing_folder_is_still_shown() {
    let mut app = app();
    let id = open(&mut app);
    change(&mut app, id, "/no/such/source", "/no/such/destination");
    let ran = fields(&app, id);
    let _ = crate::dialogs::confirm(&mut app, id);
    let saved = app.session.prefs_value().to_string();
    let mut again = self::app();
    again.session.load_prefs_json(&saved).unwrap();
    let id = open(&mut again);
    assert_eq!(fields(&again, id), ran, "after a restart, missing folders included");
}

#[test]
fn stored_values_that_dont_fit_are_ignored() {
    let mut app = app();
    let id = open(&mut app);
    let defaults = fields(&app, id);
    crate::dialogs::cancel(&mut app, id).unwrap();
    let corrupt =
        json!({"source": "elsewhere", "includeSubfolders": "yes", "output": 7, "jpeg": {"quality": 40, "enabled": "on"}, "tiff": [1, 2], "psd": {"width": -3}});
    app.session.prefs.edit(|p| p.dialogs.insert(IP.into(), corrupt));
    let id = open(&mut app);
    assert_eq!(fields(&app, id), defaults);
    // Without a document a stored document source shows the folder, as today.
    app.session.prefs.edit(|p| p.dialogs.insert(IP.into(), json!({"source": "current"})));
    let mut none = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    none.session.prefs.edit(|p| p.dialogs.insert(IP.into(), json!({"source": "open"})));
    let id = open(&mut none);
    assert_eq!(fields(&none, id)["source"], json!("folder"));
}
