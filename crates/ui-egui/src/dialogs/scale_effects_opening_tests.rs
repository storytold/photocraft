//! #14994: Layer Style › Scale Effects opens at 100 % every time, also after OK at another
//! value, and keeps Preview as the dialog last showed it, closed by OK or Cancel alike. Preview
//! is on in a fresh session; Alt+Reset puts Scale back to 100 and leaves Preview alone. Other
//! schema dialogs keep remembering their values and open with Preview on.
//!
//! Each test runs on its own thread, so each starts from a fresh session's Preview state.

use serde_json::{Map, Value, json};

use super::{body, open, reset_fields};
use crate::PhotocraftApp;

const CMD: &str = "layer.layerStyle.scaleEffects";

/// A document whose active layer has a Drop Shadow, so Scale Effects can run.
fn styled() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Shape"})).unwrap();
    app.run("layer.layerStyle.dropShadow", json!({"distance": 10, "size": 4})).unwrap();
    app
}

/// Draw a dialog body once, as a live dialog does every frame.
fn drawn(f: &mut Map<String, Value>) {
    let mut out = egui::Context::default().run_ui(Default::default(), |ui| body(ui, f));
    out.textures_delta.clear();
}

/// Open `command`'s dialog and draw its first frame: the fields it shows.
fn opened(app: &mut PhotocraftApp, command: &str) -> (u64, Map<String, Value>) {
    let id = open(app, command).unwrap();
    let mut f = app.ui.dialog_mut(id).unwrap().fields.clone();
    drawn(&mut f);
    app.ui.dialog_mut(id).unwrap().fields = f.clone();
    (id, f)
}

/// Set `key` in the open dialog, as the user would, and draw the frame that shows it.
fn set(app: &mut PhotocraftApp, id: u64, key: &str, v: Value) {
    let mut f = app.ui.dialog_mut(id).unwrap().fields.clone();
    f.insert(key.into(), v);
    drawn(&mut f);
    app.ui.dialog_mut(id).unwrap().fields = f;
}

#[test]
fn scale_opens_at_100_also_after_ok_at_150() {
    let mut app = styled();
    let (id, f) = opened(&mut app, CMD);
    assert_eq!(f["scale"].as_f64(), Some(100.0), "a first opening reads 100");
    set(&mut app, id, "scale", json!(150.0));
    crate::dialogs::confirm(&mut app, id).unwrap();
    let (_id, f) = opened(&mut app, CMD);
    assert_eq!(f["scale"].as_f64(), Some(100.0), "after OK at 150 the next opening reads 100");
    assert!(!app.session.prefs().dialogs.contains_key(CMD), "Scale Effects keeps no last-used value");
}

#[test]
fn a_scale_saved_by_an_earlier_version_is_not_restored() {
    let mut app = styled();
    app.session.prefs.edit(|p| p.dialogs.insert(CMD.into(), json!({"scale": 150.0})));
    let (_id, f) = opened(&mut app, CMD);
    assert_eq!(f["scale"].as_f64(), Some(100.0));
}

#[test]
fn preview_is_on_in_a_fresh_session_and_follows_the_last_close() {
    let mut app = styled();
    let (id, f) = opened(&mut app, CMD);
    assert_eq!(f["__preview"], json!(true), "Preview is on in a fresh session");

    // Off, then Cancel: the next opening shows Preview off.
    set(&mut app, id, "__preview", json!(false));
    crate::dialogs::cancel(&mut app, id).unwrap();
    let (id, f) = opened(&mut app, CMD);
    assert_eq!(f["__preview"], json!(false), "closed by Cancel with Preview off");

    // Still off, OK: stays off.
    set(&mut app, id, "scale", json!(50.0));
    crate::dialogs::confirm(&mut app, id).unwrap();
    let (id, f) = opened(&mut app, CMD);
    assert_eq!(f["__preview"], json!(false), "closed by OK with Preview off");

    // Switched back on, then Cancel: on again.
    set(&mut app, id, "__preview", json!(true));
    crate::dialogs::cancel(&mut app, id).unwrap();
    let (id, f) = opened(&mut app, CMD);
    assert_eq!(f["__preview"], json!(true), "switched back on");

    // Off, then OK: off.
    set(&mut app, id, "__preview", json!(false));
    crate::dialogs::confirm(&mut app, id).unwrap();
    let (_id, f) = opened(&mut app, CMD);
    assert_eq!(f["__preview"], json!(false), "closed by OK after switching Preview off");
}

#[test]
fn alt_reset_puts_scale_back_to_100_and_leaves_preview() {
    let mut app = styled();
    let (id, _) = opened(&mut app, CMD);
    set(&mut app, id, "scale", json!(300.0));
    set(&mut app, id, "__preview", json!(false));
    let mut f = app.ui.dialog_mut(id).unwrap().fields.clone();
    reset_fields(&app, &mut f);
    drawn(&mut f);
    assert_eq!(f["scale"].as_f64(), Some(100.0));
    assert_eq!(f["__preview"], json!(false), "Reset leaves Preview as it is");
}

#[test]
fn other_dialogs_keep_remembering_and_open_with_preview_on() {
    let mut app = styled();
    // Scale Effects' Preview off does not reach other dialogs.
    let (id, _) = opened(&mut app, CMD);
    set(&mut app, id, "__preview", json!(false));
    crate::dialogs::cancel(&mut app, id).unwrap();

    let blur = "filter.blur.gaussianBlur";
    let (id, f) = opened(&mut app, blur);
    assert_eq!(f["__preview"], json!(true), "Gaussian Blur opens with Preview on");
    set(&mut app, id, "radius", json!(7.0));
    set(&mut app, id, "__preview", json!(false));
    crate::dialogs::confirm(&mut app, id).unwrap();
    let (_id, f) = opened(&mut app, blur);
    assert_eq!(f["radius"].as_f64(), Some(7.0), "Gaussian Blur still remembers its radius");
    assert_eq!(f["__preview"], json!(true), "and opens with Preview on, as today");
}
