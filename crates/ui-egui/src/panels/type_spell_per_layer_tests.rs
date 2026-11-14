//! #14176: with Check All Layers on, the Check Spelling dialog's Change All and Ignore All act on
//! the current word's layer only; the same word in another layer is flagged when the check gets
//! there. Headless `changeAll` without `layer` still changes every checked layer.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::spell_enabled_ui_tests::{app, harness, open_edit_menu};
use crate::PhotocraftApp;

/// The issue's document, type layers bottom to top, then an empty pixel layer on top. The issue
/// gives no text for the hidden and locked layers; theirs has no typos, so they list nothing.
const LAYERS: [(&str, &str); 10] = [
    ("T1 typos", "Teh quick brwn fox"),
    ("T2 repeat", "the the end"),
    ("T3 clean", "hello world"),
    ("T4 hidden", "hidden"),
    ("T5 locked", "locked"),
    ("T6 para", "paragrph text here"),
    ("T7 german", "Hallo Weltt"),
    ("T8 repeat2", "brwn dog Teh"),
    ("T9 hidden2", "hidden"),
    ("T10 locked2", "locked"),
];

/// The issue's document with `active` the active layer.
fn document(active: &str) -> PhotocraftApp {
    let mut app = app(false);
    app.run("file.new", json!({"width": 800, "height": 500, "background": "white"})).unwrap();
    for (i, (name, text)) in LAYERS.into_iter().enumerate() {
        let y = 30.0 + 40.0 * i as f64;
        let mut params = json!({"x": 10, "y": y, "text": text, "size": 12, "name": name});
        if name == "T6 para" {
            params = json!({"box": [10, y - 20.0, 400, 36], "text": text, "size": 12, "name": name});
        }
        let id = app.run("type.create", params).unwrap()["layer"].clone();
        if name.contains("hidden") {
            app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        } else if name.contains("locked") {
            app.run("layer.setProps", json!({"layer": id, "locked": true})).unwrap();
        }
    }
    app.run("layer.new.layer", json!({"name": "Empty"})).unwrap();
    app.run("layer.select", json!({"layer": active})).unwrap();
    app.sync_views();
    app
}

/// Opens Edit > Check Spelling... from the menu bar.
fn open(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = harness(app);
    assert!(open_edit_menu(&mut h));
    h.get_by_label_contains("Check Spelling").click();
    h.run_steps(4);
    h
}

fn click(h: &mut Harness<'static, PhotocraftApp>, button: &str) {
    h.get_by_label(button).click();
    h.run_steps(4);
}

/// The words still to come, from the current one on, as "layer: word".
fn to_come(app: &PhotocraftApp) -> Vec<String> {
    let spell = app.ui.type_panels.spell.as_ref().expect("the Check Spelling dialog is open");
    let doc = &app.session.active().unwrap().doc;
    let name = |i: &serde_json::Value| i["layer"].as_u64().and_then(|id| doc.layer(photocraft_doc::LayerId(id))).map_or(String::new(), |l| l.name.clone());
    spell.items[spell.index.min(spell.items.len())..].iter().map(|i| format!("{}: {}", name(i), i["word"].as_str().unwrap_or(""))).collect()
}

fn text_of(app: &PhotocraftApp, name: &str) -> String {
    let doc = &app.session.active().unwrap().doc;
    let layer = doc.walk().into_iter().map(|(_, _, l)| l).find(|l| l.name == name).expect("the layer exists");
    match &layer.content {
        photocraft_doc::LayerContent::Text(t) => t.text.clone(),
        _ => String::new(),
    }
}

/// From T8 up, then wrapped to T1: T8's two typos, then T1's, T6's and T7's.
const FROM_T8: [&str; 7] =
    ["T8 repeat2: brwn", "T8 repeat2: Teh", "T1 typos: Teh", "T1 typos: brwn", "T6 para: paragrph", "T7 german: Hallo", "T7 german: Weltt"];

#[test]
fn change_all_on_brwn_in_t8_changes_t8_only_and_t1_is_still_flagged() {
    let mut h = open(document("T8 repeat2"));
    assert_eq!(to_come(h.state()), FROM_T8);
    assert_eq!(h.state().ui.type_panels.spell.as_ref().map(|d| d.change_to.clone()).as_deref(), Some("brown"));
    click(&mut h, "Change All");
    assert_eq!(text_of(h.state(), "T8 repeat2"), "brown dog Teh");
    assert_eq!(text_of(h.state(), "T1 typos"), "Teh quick brwn fox", "T1 is not changed");
    assert_eq!(to_come(h.state()), FROM_T8[1..], "T1's \"brwn\" still comes up");
}

#[test]
fn ignore_all_on_teh_in_t8_still_flags_teh_in_t1() {
    let mut h = open(document("T8 repeat2"));
    click(&mut h, "Ignore");
    assert_eq!(to_come(h.state())[0], "T8 repeat2: Teh");
    click(&mut h, "Ignore All");
    assert_eq!(to_come(h.state()), FROM_T8[2..], "T1's \"Teh\" still comes up");
}

/// Bottom `bottom` and top "Teh two", the bottom one active.
fn two_layers(bottom: &str) -> PhotocraftApp {
    let mut app = app(true);
    let bottom = app.run("type.create", json!({"x": 10, "y": 60, "text": bottom, "name": "bottom"})).unwrap()["layer"].clone();
    app.run("type.create", json!({"x": 10, "y": 120, "text": "Teh two", "name": "top"})).unwrap();
    app.run("layer.select", json!({"layer": bottom})).unwrap();
    app
}

const TEH: &str = "Teh one teh Tehs Teh";

#[test]
fn change_all_changes_every_whole_word_case_sensitive_match_in_the_layer() {
    let mut h = open(two_layers(TEH));
    assert_eq!(to_come(h.state()), ["bottom: Teh", "bottom: teh", "bottom: Tehs", "bottom: Teh", "top: Teh"]);
    click(&mut h, "Change All");
    assert_eq!(text_of(h.state(), "bottom"), "The one teh Tehs The");
    assert_eq!(text_of(h.state(), "top"), "Teh two");
    assert_eq!(to_come(h.state()), ["bottom: teh", "bottom: Tehs", "top: Teh"]);
}

/// Ignore All skips the rest of the layer (ignoring case, as today) and nothing else, also after a
/// Change refreshes the list.
#[test]
fn ignore_all_skips_the_word_for_the_rest_of_the_layer_only() {
    let mut h = open(two_layers(TEH));
    click(&mut h, "Ignore All");
    assert_eq!(to_come(h.state()), ["bottom: Tehs", "top: Teh"]);
    click(&mut h, "Change");
    assert_eq!(to_come(h.state()), ["top: Teh"]);
}

/// Ignore All compares words as the dictionary does: typographic apostrophes and `'s` aside.
#[test]
fn ignore_all_also_skips_the_possessive_forms_in_the_layer() {
    let mut h = open(two_layers("Teh Teh's teh\u{2019}s"));
    assert_eq!(to_come(h.state()), ["bottom: Teh", "bottom: Teh's", "bottom: teh\u{2019}s", "top: Teh"]);
    click(&mut h, "Ignore All");
    assert_eq!(to_come(h.state()), ["top: Teh"]);
}

/// Words passed with Ignore that Change All replaces or Ignore All drops do not move the dialog
/// past the next word.
#[test]
fn change_all_after_ignore_goes_on_with_the_next_word() {
    let mut h = open(two_layers(TEH));
    for _ in 0..3 {
        click(&mut h, "Ignore");
    }
    assert_eq!(to_come(h.state()), ["bottom: Teh", "top: Teh"]);
    click(&mut h, "Change All");
    assert_eq!(text_of(h.state(), "bottom"), "The one teh Tehs The");
    assert_eq!(to_come(h.state()), ["top: Teh"]);
}

#[test]
fn ignore_all_after_ignore_goes_on_with_the_next_word() {
    let mut h = open(two_layers(TEH));
    click(&mut h, "Ignore");
    assert_eq!(to_come(h.state())[0], "bottom: teh");
    click(&mut h, "Ignore All");
    assert_eq!(to_come(h.state()), ["bottom: Tehs", "top: Teh"]);
}

/// Pins today: Ignore All is forgotten when the dialog closes, so the next run flags the word again.
#[test]
fn ignore_all_is_forgotten_in_the_next_run() {
    let mut h = open(document("T8 repeat2"));
    click(&mut h, "Ignore All");
    assert_eq!(to_come(h.state())[0], "T8 repeat2: Teh");
    click(&mut h, "Done");
    assert!(h.state().ui.type_panels.spell.is_none());
    assert!(open_edit_menu(&mut h));
    h.get_by_label_contains("Check Spelling").click();
    h.run_steps(4);
    assert_eq!(to_come(h.state()), FROM_T8);
}

/// Pins today: headless `changeAll` without `layer` changes the word in every checked layer.
#[test]
fn headless_change_all_without_a_layer_changes_every_layer() {
    let mut app = document("T8 repeat2");
    let r = app.run("edit.checkSpelling", json!({"action": "changeAll", "word": "brwn", "replace": "brown"})).unwrap();
    assert_eq!((r["changed"].clone(), r["layers"].clone()), (json!(2), json!(2)));
    assert_eq!(text_of(&app, "T1 typos"), "Teh quick brown fox");
    assert_eq!(text_of(&app, "T8 repeat2"), "brown dog Teh");
}
