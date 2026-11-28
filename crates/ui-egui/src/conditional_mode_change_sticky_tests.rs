//! #13680: Conditional Mode Change opens with RGB Color to Grayscale and then with the sources and
//! target of the last run that converted a document, also after a restart. Cancel and a run that
//! converted nothing keep the settings from before. Drawn with the real dialog window.

use craft::ui::dialog::DialogId;
use egui::accesskit::Role;
use photocraft_color::ColorMode;
use photocraft_engine::prefs::SavedPrefsExt;
use serde_json::{Value, json};

use super::COMMAND;
use super::tests::{H, LEFT, RIGHT, checked, click, field, is_open, mode, ok, open_dialog, open_in, set, steps};
use crate::PhotocraftApp;

/// A 40 x 20 document in `mode`, in an app on `session`.
fn app_on(session: photocraft_engine::Session, mode: &str) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(session, crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 20})).unwrap();
    if mode != "rgb" {
        app.run(&format!("image.mode.{mode}"), json!({})).unwrap();
    }
    app
}

/// A fresh app (no stored settings) with a document in `mode` and the dialog open.
fn opened_on(mode: &str) -> (H, DialogId) {
    open_in(app_on(photocraft_engine::Session::new(), mode))
}

/// An app whose preferences were loaded from `prefs`, as at start, with the dialog open.
fn restarted(prefs: &str, mode: &str) -> (H, DialogId) {
    let mut session = photocraft_engine::Session::new();
    session.load_prefs_json(prefs).unwrap();
    open_in(app_on(session, mode))
}

/// The ticked sources and the target of dialog `id`.
fn shown(h: &H, id: DialogId) -> (Vec<&'static str>, Value) {
    let ticked = LEFT.into_iter().chain(RIGHT).filter(|l| checked(h, l)).collect();
    (ticked, field(h, id, "to"))
}

/// Open the dialog again from the File menu, in the same window; its dialog was closed.
fn reopen(h: &mut H) -> DialogId {
    let id = open_dialog(h.state_mut());
    h.run_steps(3);
    id
}

/// What the preferences hold for the dialog.
fn stored(h: &H) -> Option<Value> {
    h.state().session.prefs().dialogs.get(COMMAND).cloned()
}

/// Tick exactly `sources`, pick `target` and press OK.
fn run(h: &mut H, id: DialogId, sources: &[&str], target: &str) {
    click(h, Role::Button, "None");
    for s in sources {
        click(h, Role::CheckBox, s);
    }
    set(h, id, "to", json!(target));
    ok(h, id);
    assert!(!is_open(h, id), "OK closes the dialog");
}

#[test]
fn the_first_open_is_rgb_color_to_grayscale_whatever_the_document_mode() {
    for doc in ["rgb", "grayscale", "lab", "cmyk"] {
        let (h, id) = opened_on(doc);
        assert_eq!(shown(&h, id), (vec!["RGB Color"], json!("grayscale")), "on a {doc} document");
        assert_eq!(field(&h, id, "from"), json!(["rgb"]), "on a {doc} document");
    }
    // OK on it converts an RGB document to Grayscale.
    let (mut h, id) = opened_on("rgb");
    ok(&mut h, id);
    assert_eq!(mode(&h), ColorMode::Grayscale);
}

#[test]
fn a_run_that_converted_opens_next_time_with_its_sources_and_target() {
    let (mut h, id) = opened_on("grayscale");
    run(&mut h, id, &["Grayscale"], "rgb");
    assert_eq!(mode(&h), ColorMode::Rgb);
    let id = reopen(&mut h);
    assert_eq!(shown(&h, id), (vec!["Grayscale"], json!("rgb")), "Grayscale to RGB Color is kept");
    click(&mut h, Role::Button, "All");
    set(&mut h, id, "to", json!("cmyk"));
    ok(&mut h, id);
    assert_eq!(mode(&h), ColorMode::Cmyk);
    let id = reopen(&mut h);
    let every: Vec<&str> = LEFT.into_iter().chain(RIGHT).collect();
    assert_eq!(shown(&h, id), (every, json!("cmyk")), "All to CMYK Color is kept");
}

/// An RGB document converted by RGB Color to Lab Color, with the dialog open again.
fn after_rgb_to_lab() -> (H, DialogId) {
    let (mut h, id) = opened_on("rgb");
    run(&mut h, id, &["RGB Color"], "lab");
    assert_eq!(mode(&h), ColorMode::Lab);
    let id = reopen(&mut h);
    (h, id)
}

#[test]
fn cancel_and_runs_that_converted_nothing_keep_the_settings_from_before() {
    let before = (vec!["RGB Color"], json!("lab"));
    let (mut h, id) = after_rgb_to_lab();
    click(&mut h, Role::Button, "All");
    set(&mut h, id, "to", json!("cmyk"));
    crate::dialogs::testing::cancel(h.state_mut(), id).expect("Cancel");
    h.run_steps(3);
    assert!(!is_open(&h, id), "Cancel closes the dialog");
    let id = reopen(&mut h);
    assert_eq!(shown(&h, id), before, "after Cancel");

    // The Lab document is not ticked: OK converts nothing.
    let history = steps(&h);
    run(&mut h, id, &["Grayscale"], "rgb");
    assert_eq!((mode(&h), steps(&h)), (ColorMode::Lab, history), "nothing converted");
    let id = reopen(&mut h);
    assert_eq!(shown(&h, id), before, "after a run that converted nothing");

    // Already in the target mode: nothing converts either.
    run(&mut h, id, &["Lab Color"], "lab");
    assert_eq!((mode(&h), steps(&h)), (ColorMode::Lab, history), "already Lab Color");
    let id = reopen(&mut h);
    assert_eq!(shown(&h, id), before, "after a run already in the target mode");
}

/// A target the list does not hold is refused before OK, and a run whose `from` the dialog cannot show stores nothing.
#[test]
fn values_the_dialog_cannot_show_store_nothing() {
    let (mut h, id) = after_rgb_to_lab();
    let before = stored(&h);
    assert_eq!(before, Some(json!({"from": ["rgb"], "to": "lab"})));
    let ctx = h.ctx.clone();
    assert!(h.state_mut().framework.dialogs.set_json_field(&ctx, id, "to", json!("xyz")).is_err(), "the Mode list holds no xyz");

    // `from` as a number runs as "any" in the command and converts, but is not stored.
    set(&mut h, id, "from", json!(5));
    set(&mut h, id, "to", json!("rgb"));
    ok(&mut h, id);
    assert_eq!(mode(&h), ColorMode::Rgb, "the run converted");
    assert_eq!(stored(&h), before, "a from the dialog cannot show is not stored");
}

#[test]
fn the_settings_of_a_converting_run_survive_a_restart() {
    let (mut h, id) = opened_on("rgb");
    run(&mut h, id, &["Grayscale", "RGB Color"], "lab");
    assert_eq!(mode(&h), ColorMode::Lab);
    let saved = h.state().session.prefs_to_json();
    let stored: Value = serde_json::from_str(&saved).unwrap_or_else(|e| panic!("the preferences were saved ({e})"));
    assert_eq!(stored["dialogs"][COMMAND], json!({"from": ["grayscale", "rgb"], "to": "lab"}), "written to the preferences");
    // A restart reads them back, on a document of another mode.
    let (h, id) = restarted(&saved, "grayscale");
    assert_eq!(shown(&h, id), (vec!["Grayscale", "RGB Color"], json!("lab")));
}

#[test]
fn an_older_preferences_file_loads_and_saving_then_loading_keeps_the_settings() {
    let mut session = photocraft_engine::Session::new();
    session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), json!({"from": ["cmyk", "lab"], "to": "rgb"})));
    let current = session.prefs_value();
    let mut reloaded = photocraft_engine::Session::new();
    reloaded.load_prefs_json(&current.to_string()).unwrap();
    assert_eq!(reloaded.prefs_value(), current, "save then load gives the same preferences");
    assert_eq!(reloaded.prefs().dialogs[COMMAND], json!({"from": ["cmyk", "lab"], "to": "rgb"}));

    // A file from before the stored settings (no entry, or no dialogs map at all) and one with a
    // stored entry of the wrong shape load and open with RGB Color to Grayscale.
    let mut older = current.clone();
    older["dialogs"].as_object_mut().unwrap().remove(COMMAND);
    let mut oldest = current.clone();
    oldest.as_object_mut().unwrap().remove("dialogs");
    let mut files = vec![("no entry", older), ("no dialogs map", oldest)];
    for entry in
        [json!({"from": [1, 2], "to": "lab"}), json!({"from": ["rgb"], "to": "xyz"}), json!({"from": ["rgb"], "to": 5}), json!({"from": ["xyz"], "to": "lab"})]
    {
        let mut file = current.clone();
        file["dialogs"][COMMAND] = entry;
        files.push(("corrupt entry", file));
    }
    for (what, file) in files {
        let (h, id) = restarted(&file.to_string(), "rgb");
        assert_eq!(shown(&h, id), (vec!["RGB Color"], json!("grayscale")), "{what}: {}", file["dialogs"].get(COMMAND).unwrap_or(&Value::Null));
    }
}
