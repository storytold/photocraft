//! #13695: Contact Sheet II opens with the values of the last OK, the folder and the resolution unit included, also
//! after a restart for what the preferences keep. Cancel and a failed run store nothing.

use craft::ui::dialog::DialogId;
use serde_json::{Value, json};

use super::contact_sheet_layout_tests::{H, field, harness, image_folder, set};
use crate::PhotocraftApp;

/// A key and a value other than its default.
type Change = (&'static str, fn() -> Value);

/// Every value the user can change.
const CHANGED: [Change; 16] = [
    ("includeSubfolders", || json!(false)),
    ("units", || json!("pixels")),
    ("width", || json!(1200.0)),
    ("height", || json!(900.0)),
    ("resolution", || json!(150.0)),
    ("resolution.unit", || json!("cm")),
    ("flatten", || json!(true)),
    ("mode", || json!("gray")),
    ("depth", || json!("16")),
    ("place", || json!("down")),
    ("columns", || json!(3)),
    ("rows", || json!(2)),
    ("rotateForBestFit", || json!(true)),
    ("autoSpacing", || json!(false)),
    // The font before the caption, which greys it.
    ("font", || json!("Courier")),
    ("caption", || json!(false)),
];

/// The dialog open from the menu over `app`.
fn open_in(mut app: Box<PhotocraftApp>) -> (H, DialogId) {
    let id = open(&mut app);
    (harness(app), id)
}

fn open(app: &mut PhotocraftApp) -> DialogId {
    let ctx = app.ctx().clone();
    let reply = crate::menus::invoke(app, &ctx, super::COMMAND, json!({})).unwrap();
    let want = reply["dialog"].as_str().map(str::to_owned).or_else(|| reply["dialog"].as_u64().map(|v| v.to_string())).expect("the reply names a dialog");
    app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|id| id.value().to_string() == want).expect("the dialog is open")
}

fn fresh() -> Box<PhotocraftApp> {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

fn reopen(h: &mut H) -> DialogId {
    let id = open(h.state_mut());
    h.run_steps(4);
    id
}

/// What the dialog shows for the [`CHANGED`] keys and the folder.
fn values(h: &H, id: DialogId) -> Vec<Value> {
    CHANGED.iter().map(|(key, _)| field(h, id, key)).chain([field(h, id, "input")]).collect()
}

/// Changes every value and OKs a run over `dir`; the run makes a sheet.
fn run_changed(h: &mut H, id: DialogId, dir: &std::path::Path) -> Vec<Value> {
    for (key, value) in CHANGED {
        set(h, id, key, value());
    }
    set(h, id, "input", json!(dir.to_string_lossy()));
    let shown = values(h, id);
    crate::dialogs::testing::confirm(h.state_mut(), id).expect("OK is accepted");
    h.run_steps(4);
    assert!(h.state().framework.dialogs.is_empty(), "OK closed the dialog");
    assert!(!h.state().session.documents().is_empty(), "the run made a sheet");
    shown
}

#[test]
fn after_ok_the_next_open_shows_every_value_of_that_run_including_the_folder() {
    let (mut h, id) = open_in(fresh());
    let dir = image_folder();
    let shown = run_changed(&mut h, id, dir.path());
    let id = reopen(&mut h);
    assert_eq!(values(&h, id), shown);
}

#[test]
fn cancel_shows_the_last_okd_values_next_time() {
    let (mut h, id) = open_in(fresh());
    let dir = image_folder();
    let shown = run_changed(&mut h, id, dir.path());
    let id = reopen(&mut h);
    set(&mut h, id, "columns", json!(9));
    set(&mut h, id, "input", json!("/elsewhere"));
    crate::dialogs::testing::cancel(h.state_mut(), id).unwrap();
    h.run_steps(3);
    let id = reopen(&mut h);
    assert_eq!(values(&h, id), shown, "Cancel stores nothing");
}

#[test]
fn cancel_before_any_ok_keeps_the_defaults() {
    let (mut h, id) = open_in(fresh());
    let defaults = values(&h, id);
    set(&mut h, id, "columns", json!(9));
    set(&mut h, id, "input", json!("/elsewhere"));
    crate::dialogs::testing::cancel(h.state_mut(), id).unwrap();
    h.run_steps(3);
    let id = reopen(&mut h);
    assert_eq!(values(&h, id), defaults);
}

/// A run the command refuses (a folder without images) keeps what the last OK'd run stored.
#[test]
fn a_failed_run_keeps_the_values_from_before() {
    let (mut h, id) = open_in(fresh());
    let dir = image_folder();
    let shown = run_changed(&mut h, id, dir.path());
    let id = reopen(&mut h);
    let empty = tempfile::tempdir().unwrap();
    set(&mut h, id, "input", json!(empty.path().to_string_lossy()));
    set(&mut h, id, "columns", json!(9));
    let _ = crate::dialogs::testing::confirm(h.state_mut(), id);
    h.run_steps(4);
    let id = reopen(&mut h);
    assert_eq!(values(&h, id), shown);
}

/// The folder and the font, which the preferences keep, survive a restart.
#[test]
fn the_folder_and_font_of_the_last_ok_survive_a_restart() {
    let (mut h, id) = open_in(fresh());
    let dir = image_folder();
    run_changed(&mut h, id, dir.path());
    let saved = h.state().session.prefs_to_json();
    let mut session = photocraft_engine::Session::new();
    session.load_prefs_json(&saved).unwrap();
    let (h, id) = open_in(PhotocraftApp::new(session, crate::Services::default()));
    assert_eq!((field(&h, id, "input"), field(&h, id, "font")), (json!(dir.path().to_string_lossy()), json!("Courier")));
}
