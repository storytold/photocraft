//! #13680: Conditional Mode Change opens with RGB Color to Grayscale and then with the sources and
//! target of the last run that converted a document, also after a restart. Cancel and a run that
//! converted nothing keep the settings from before. Drawn with the real dialog window.

use std::cell::RefCell;
use std::rc::Rc;

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_color::ColorMode;
use serde_json::{Value, json};

use super::COMMAND;
use super::tests::{LEFT, RIGHT, checked, click, field, mode, open_in, pick_target, steps};
use crate::PhotocraftApp;

/// A 40 x 20 document in `mode`, in an app whose preferences load from `prefs` on its first frame
/// and are saved to `saved` by the frames after it.
fn app_in(mode: &str, prefs: Option<String>, saved: &Rc<RefCell<String>>) -> PhotocraftApp {
    let store = saved.clone();
    let services = crate::Services {
        load_prefs: prefs.map(|text| Box::new(move || Some(text.clone())) as crate::services::LoadTextFn),
        save_prefs: Some(Box::new(move |text| {
            *store.borrow_mut() = text.to_string();
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    crate::prefs_ui::tick(&mut app, &egui::Context::default());
    app.run("file.new", json!({"width": 40, "height": 20})).unwrap();
    if mode != "rgb" {
        app.run(&format!("image.mode.{mode}"), json!({})).unwrap();
    }
    app
}

/// A fresh app (no stored settings) with a document in `mode` and the dialog open.
fn opened_on(mode: &str) -> (Harness<'static, PhotocraftApp>, u64) {
    open_in(app_in(mode, None, &Rc::default()))
}

/// The ticked sources and the target the Mode: list shows.
fn shown(h: &Harness<'_, PhotocraftApp>) -> (Vec<&'static str>, String) {
    let ticked = LEFT.into_iter().chain(RIGHT).filter(|l| checked(h, l)).collect();
    (ticked, h.get_by_role(Role::ComboBox).accesskit_node().value().unwrap_or_default().to_string())
}

/// Open the dialog again from the File menu, in the same window; its dialog was closed.
fn reopen(h: &mut Harness<'_, PhotocraftApp>) -> u64 {
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(3);
    id
}

/// Set a field of dialog `id` over the control channel, as an agent would.
fn set(h: &mut Harness<'_, PhotocraftApp>, id: u64, key: &str, value: Value) {
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::ControlRequest::new("ui.dialog.set", json!({"dialog": id, "field": key, "value": value}));
    let crate::control::Outcome::Done(reply) = crate::control::handle(h.state_mut(), &ctx, &req) else { panic!("ui.dialog.set replies at once") };
    assert_eq!(reply["ok"], json!(true), "ui.dialog.set {key}: {reply}");
    h.run_steps(3);
}

/// What the preferences hold for the dialog.
fn stored(h: &Harness<'_, PhotocraftApp>) -> Option<Value> {
    h.state().session.prefs().dialogs.get(COMMAND).cloned()
}

/// Tick exactly `sources`, pick `target` and press OK.
fn run(h: &mut Harness<'_, PhotocraftApp>, sources: &[&str], target: &str) {
    click(h, Role::Button, "None");
    for s in sources {
        click(h, Role::CheckBox, s);
    }
    pick_target(h, target);
    click(h, Role::Button, "OK");
    assert!(h.state().ui.dialogs.is_empty(), "OK closes the dialog");
}

#[test]
fn the_first_open_is_rgb_color_to_grayscale_whatever_the_document_mode() {
    for doc in ["rgb", "grayscale", "lab", "cmyk"] {
        let (h, id) = opened_on(doc);
        assert_eq!(shown(&h), (vec!["RGB Color"], "Grayscale".to_string()), "on a {doc} document");
        assert_eq!((field(&h, id, "from"), field(&h, id, "to")), (json!(["rgb"]), json!("grayscale")), "on a {doc} document");
    }
    // OK on it converts an RGB document to Grayscale.
    let (mut h, _) = opened_on("rgb");
    click(&mut h, Role::Button, "OK");
    assert_eq!(mode(&h), ColorMode::Grayscale);
}

#[test]
fn a_run_that_converted_opens_next_time_with_its_sources_and_target() {
    let (mut h, _) = opened_on("grayscale");
    run(&mut h, &["Grayscale"], "RGB Color");
    assert_eq!(mode(&h), ColorMode::Rgb);
    reopen(&mut h);
    assert_eq!(shown(&h), (vec!["Grayscale"], "RGB Color".to_string()), "Grayscale to RGB Color is kept");
    click(&mut h, Role::Button, "All");
    pick_target(&mut h, "CMYK Color");
    click(&mut h, Role::Button, "OK");
    assert_eq!(mode(&h), ColorMode::Cmyk);
    reopen(&mut h);
    let every: Vec<&str> = LEFT.into_iter().chain(RIGHT).collect();
    assert_eq!(shown(&h), (every, "CMYK Color".to_string()), "All to CMYK Color is kept");
}

/// An RGB document converted by RGB Color to Lab Color, with the dialog open again.
fn after_rgb_to_lab() -> Harness<'static, PhotocraftApp> {
    let (mut h, _) = opened_on("rgb");
    run(&mut h, &["RGB Color"], "Lab Color");
    assert_eq!(mode(&h), ColorMode::Lab);
    reopen(&mut h);
    h
}

#[test]
fn cancel_escape_and_runs_that_converted_nothing_keep_the_settings_from_before() {
    let before = (vec!["RGB Color"], "Lab Color".to_string());
    let mut h = after_rgb_to_lab();
    click(&mut h, Role::Button, "All");
    pick_target(&mut h, "CMYK Color");
    click(&mut h, Role::Button, "Cancel");
    assert!(h.state().ui.dialogs.is_empty(), "Cancel closes the dialog");
    reopen(&mut h);
    assert_eq!(shown(&h), before, "after Cancel");

    click(&mut h, Role::Button, "None");
    click(&mut h, Role::CheckBox, "Grayscale");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty(), "Escape closes the dialog");
    reopen(&mut h);
    assert_eq!(shown(&h), before, "after Escape");

    // The Lab document is not ticked: OK converts nothing.
    let history = steps(&h);
    run(&mut h, &["Grayscale"], "RGB Color");
    assert_eq!((mode(&h), steps(&h)), (ColorMode::Lab, history), "nothing converted");
    reopen(&mut h);
    assert_eq!(shown(&h), before, "after a run that converted nothing");

    // Already in the target mode: nothing converts either.
    run(&mut h, &["Lab Color"], "Lab Color");
    assert_eq!((mode(&h), steps(&h)), (ColorMode::Lab, history), "already Lab Color");
    reopen(&mut h);
    assert_eq!(shown(&h), before, "after a run already in the target mode");
}

/// A failed run, and a run whose `from` the dialog cannot show, store nothing.
#[test]
fn a_failed_run_and_a_run_with_values_the_dialog_cannot_show_store_nothing() {
    let mut h = after_rgb_to_lab();
    let before = stored(&h);
    assert_eq!(before, Some(json!({"from": ["rgb"], "to": "lab"})));
    let id = h.state().ui.dialogs[0].id;
    set(&mut h, id, "from", json!(["lab"]));
    set(&mut h, id, "to", json!("xyz"));
    let history = steps(&h);
    let result = crate::dialogs::confirm(h.state_mut(), id);
    assert!(result.is_err(), "the command refuses an unknown target: {result:?}");
    h.run_steps(3);
    assert_eq!((mode(&h), steps(&h)), (ColorMode::Lab, history), "nothing converted");
    assert_eq!(stored(&h), before, "a failed run stores nothing");

    // `from` as a number runs as "any" in the command and converts, but is not stored.
    assert!(h.state().ui.dialogs.is_empty(), "OK closed the dialog");
    let id = reopen(&mut h);
    set(&mut h, id, "from", json!(5));
    set(&mut h, id, "to", json!("rgb"));
    click(&mut h, Role::Button, "OK");
    assert_eq!(mode(&h), ColorMode::Rgb, "the run converted");
    assert_eq!(stored(&h), before, "a from the dialog cannot show is not stored");
}

#[test]
fn the_settings_of_a_converting_run_survive_a_restart() {
    let saved = Rc::new(RefCell::new(String::new()));
    let (mut h, _) = open_in(app_in("rgb", None, &saved));
    run(&mut h, &["Grayscale", "RGB Color"], "Lab Color");
    assert_eq!(mode(&h), ColorMode::Lab);
    let ctx = h.ctx.clone();
    crate::prefs_ui::tick(h.state_mut(), &ctx);
    let text = saved.borrow().clone();
    let stored: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("the preferences were saved ({e})"));
    assert_eq!(stored["dialogs"][COMMAND], json!({"from": ["grayscale", "rgb"], "to": "lab"}), "written to the store");
    // A restart reads them back, on a document of another mode.
    let (h, _) = open_in(app_in("grayscale", Some(text), &Rc::default()));
    assert_eq!(shown(&h), (vec!["Grayscale", "RGB Color"], "Lab Color".to_string()));
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
        let (h, _) = open_in(app_in("rgb", Some(file.to_string()), &Rc::default()));
        assert_eq!(shown(&h), (vec!["RGB Color"], "Grayscale".to_string()), "{what}: {}", file["dialogs"].get(COMMAND).unwrap_or(&Value::Null));
    }
}
