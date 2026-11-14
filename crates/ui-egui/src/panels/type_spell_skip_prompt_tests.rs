//! #14175: Check Spelling checks no hidden or Lock All type layer. When it reaches one it asks
//! first: Skip (the default, Enter) goes on with the next layer, Skip All also passes every later
//! layer of that kind, and Cancel (Esc) ends the check, keeping the changes made. A hidden or
//! locked first layer is asked about before the dialog opens. Headless `list` reports these layers
//! in `skipped`, with `at`, the index of the next misspelling, where the dialog asks.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_engine::actions_cmds::Action;
use serde_json::{Value, json};

use super::spell_enabled_ui_tests::{app, harness, open_edit_menu};
use super::spell_per_layer_tests::{click, document, open, text_of, to_come};
use crate::PhotocraftApp;

const COMPLETE: &str = "Spell check complete.";

fn hidden(name: &str) -> String {
    format!("The text layer “{name}” is hidden and cannot be spell checked.  Skip hidden layer and continue spell checking?")
}

fn locked(name: &str) -> String {
    format!("The text layer “{name}” is locked and cannot be spell checked.  Skip locked layer and continue spell checking?")
}

/// The prompt on screen, if one is.
fn prompt(h: &Harness<'static, PhotocraftApp>) -> Option<String> {
    h.query_by(|n| n.role() == egui::accesskit::Role::Label && n.value().is_some_and(|v| v.starts_with("The text layer")))
        .and_then(|n| n.accesskit_node().value())
}

fn dialog_shown(h: &Harness<'static, PhotocraftApp>) -> bool {
    h.query_by_label("Done").is_some()
}

fn complete_shown(h: &Harness<'static, PhotocraftApp>) -> bool {
    h.query_by_label(COMPLETE).is_some()
}

fn ignore(h: &mut Harness<'static, PhotocraftApp>, times: usize) {
    for _ in 0..times {
        click(h, "Ignore");
    }
}

fn key(h: &mut Harness<'static, PhotocraftApp>, key: egui::Key) {
    h.key_press(key);
    h.run_steps(4);
}

/// "T1 typos" (active), "T2 hidden" and "T3 locked", bottom to top, each with typos.
fn typos_in_every_layer() -> PhotocraftApp {
    let mut app = app(true);
    for (i, (name, text)) in [("T1 typos", "Teh quick brwn fox"), ("T2 hidden", "secnd hidden"), ("T3 locked", "lockd text")].into_iter().enumerate() {
        let id = app.run("type.create", json!({"x": 10, "y": 60 + 60 * i, "text": text, "name": name})).unwrap()["layer"].clone();
        if name.contains("hidden") {
            app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        } else if name.contains("locked") {
            app.run("layer.setProps", json!({"layer": id, "locked": true})).unwrap();
        }
    }
    app.run("layer.select", json!({"layer": "T1 typos"})).unwrap();
    app
}

/// `edit.checkSpelling` `list` with `params` added: the words, and (layer name, reason, at) of each
/// skipped layer.
fn list(app: &mut PhotocraftApp, params: Value) -> (Vec<String>, Vec<(String, String, u64)>) {
    let mut p = json!({"action": "list"});
    p.as_object_mut().unwrap().extend(params.as_object().unwrap().clone());
    let r = app.session.execute("edit.checkSpelling", p).unwrap();
    let doc = &app.session.active().unwrap().doc;
    let name = |v: &Value| doc.layer(photocraft_doc::LayerId(v.as_u64().unwrap())).unwrap().name.clone();
    let words = r["misspellings"].as_array().unwrap().iter().map(|m| format!("{}: {}", name(&m["layer"]), m["word"].as_str().unwrap())).collect();
    let skipped =
        r["skipped"].as_array().unwrap().iter().map(|k| (name(&k["layer"]), k["reason"].as_str().unwrap().to_string(), k["at"].as_u64().unwrap())).collect();
    (words, skipped)
}

fn skip(name: &str, reason: &str, at: u64) -> (String, String, u64) {
    (name.to_string(), reason.to_string(), at)
}

fn layer_id(app: &PhotocraftApp, name: &str) -> u64 {
    app.session.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.name == name).unwrap().2.id.0
}

// ------------------------------------------------------------------ headless `list`

/// From T3: T4 and T5 come before T6's word, T9 and T10 after T8's two words and before T1's.
#[test]
fn list_reports_hidden_and_locked_layers_where_the_check_reaches_them() {
    let mut app = document("T3 clean");
    let (words, skipped) = list(&mut app, json!({}));
    assert_eq!(words.len(), 7, "paragrph, Hallo, Weltt, brwn, Teh, Teh, brwn: {words:?}");
    assert_eq!(skipped, [skip("T4 hidden", "hidden", 0), skip("T5 locked", "locked", 0), skip("T9 hidden2", "hidden", 5), skip("T10 locked2", "locked", 5)]);
}

#[test]
fn words_in_hidden_and_locked_layers_are_not_listed_or_changed() {
    let mut app = typos_in_every_layer();
    let (words, skipped) = list(&mut app, json!({}));
    assert_eq!(words, ["T1 typos: Teh", "T1 typos: brwn"]);
    assert_eq!(skipped, [skip("T2 hidden", "hidden", 2), skip("T3 locked", "locked", 2)]);
    for word in ["secnd", "lockd"] {
        let r = app.session.execute("edit.checkSpelling", json!({"action": "changeAll", "word": word, "replace": "x"})).unwrap();
        assert_eq!(r["changed"], 0, "headless Change All: {word}");
    }
    assert_eq!(text_of(&app, "T2 hidden"), "secnd hidden");
}

/// Check All Layers off (`allLayers: false` or `layer`) on a hidden or Lock All layer.
#[test]
fn one_hidden_or_locked_layer_lists_nothing_and_is_reported() {
    let mut app = typos_in_every_layer();
    for (name, reason) in [("T2 hidden", "hidden"), ("T3 locked", "locked")] {
        let id = layer_id(&app, name);
        app.run("layer.select", json!({"layer": id})).unwrap();
        assert_eq!(list(&mut app, json!({"allLayers": false})), (vec![], vec![skip(name, reason, 0)]));
        assert_eq!(list(&mut app, json!({"layer": id})), (vec![], vec![skip(name, reason, 0)]));
    }
}

/// Selected layers (#14184): only the selected hidden and locked ones are reported.
#[test]
fn selected_layers_report_only_their_hidden_and_locked_ones() {
    let mut app = document("T1 typos");
    let ids: Vec<u64> = ["T5 locked", "T4 hidden", "T1 typos"].iter().map(|n| layer_id(&app, n)).collect();
    let (words, skipped) = list(&mut app, json!({"layers": ids, "from": ids[2]}));
    assert_eq!(words, ["T1 typos: Teh", "T1 typos: brwn"]);
    assert_eq!(skipped, [skip("T4 hidden", "hidden", 2), skip("T5 locked", "locked", 2)]);
}

#[test]
fn without_hidden_or_locked_layers_skipped_is_empty() {
    let mut app = app(true);
    app.run("type.create", json!({"x": 10, "y": 60, "text": "Teh end"})).unwrap();
    assert_eq!(app.session.execute("edit.checkSpelling", json!({"action": "list"})).unwrap()["skipped"], json!([]));
}

/// Not measured, kept as today: a type layer in a hidden group and one with only its pixels
/// locked are checked.
#[test]
fn a_layer_in_a_hidden_group_and_a_partly_locked_layer_are_still_checked() {
    let mut app = app(true);
    let pixels = app.run("type.create", json!({"x": 10, "y": 60, "text": "Teh", "name": "pixels"})).unwrap()["layer"].clone();
    app.run("layer.setProps", json!({"layer": pixels, "locks": {"pixels": true}})).unwrap();
    app.run("type.create", json!({"x": 10, "y": 120, "text": "brwn", "name": "grouped"})).unwrap();
    app.run("layer.groupLayers", json!({})).unwrap();
    app.run("layer.setProps", json!({"visible": false})).unwrap();
    app.run("layer.select", json!({"layer": "pixels"})).unwrap();
    assert_eq!(list(&mut app, json!({})), (vec!["pixels: Teh".to_string(), "grouped: brwn".to_string()], vec![]));
}

// ------------------------------------------------------------------ the dialog

/// From T1: T1's two words, then T4's and T5's prompts before T6's word. Skip passes only that
/// layer: T9 hidden2 is asked about too.
#[test]
fn the_check_asks_at_a_hidden_then_a_locked_layer_and_skip_goes_on() {
    let mut h = open(document("T1 typos"));
    assert_eq!(prompt(&h), None);
    assert_eq!(to_come(h.state())[0], "T1 typos: Teh");
    ignore(&mut h, 2);
    assert_eq!(prompt(&h), Some(hidden("T4 hidden")));
    assert!(dialog_shown(&h), "over the dialog");
    assert!(!complete_shown(&h) && h.query_by_label("paragrph").is_none(), "the dialog shows neither the end nor the next word");
    assert_eq!(h.get_by_label("Don't show again").accesskit_node().toggled(), Some(egui::accesskit::Toggled::False));
    click(&mut h, "Skip");
    assert_eq!(prompt(&h), Some(locked("T5 locked")));
    click(&mut h, "Skip");
    assert_eq!(prompt(&h), None);
    assert_eq!(to_come(h.state())[0], "T6 para: paragrph");
    // paragrph, Hallo, Weltt, brwn, Teh.
    ignore(&mut h, 5);
    assert_eq!(prompt(&h), Some(hidden("T9 hidden2")));
    assert!(!complete_shown(&h), "not at the end yet");
    click(&mut h, "Skip");
    assert_eq!(prompt(&h), Some(locked("T10 locked2")));
}

/// Left to right: "Don't show again", then Skip, Cancel, Skip All.
#[test]
fn the_buttons_read_skip_cancel_skip_all_after_the_box() {
    let h = open(document("T9 hidden2"));
    let x = |label: &str| h.get_by_label(label).rect().center().x;
    assert!(x("Don't show again") < x("Skip") && x("Skip") < x("Cancel") && x("Cancel") < x("Skip All"));
}

/// Skip All on the hidden prompt still asks at the locked layer; later hidden layers pass silently.
#[test]
fn skip_all_passes_later_layers_of_its_kind_and_still_asks_for_the_other() {
    let mut h = open(document("T1 typos"));
    ignore(&mut h, 2);
    click(&mut h, "Skip All");
    assert_eq!(prompt(&h), Some(locked("T5 locked")));
    click(&mut h, "Skip");
    ignore(&mut h, 5);
    assert_eq!(prompt(&h), Some(locked("T10 locked2")), "T9 hidden2 is passed");
    assert!(!complete_shown(&h), "no end of the check under the prompt");
    click(&mut h, "Skip All");
    assert_eq!(prompt(&h), None);
    assert!(dialog_shown(&h) && complete_shown(&h));
    assert!(to_come(h.state()).is_empty());
}

#[test]
fn skip_all_on_both_kinds_passes_the_second_hidden_and_locked_silently() {
    let mut h = open(document("T1 typos"));
    ignore(&mut h, 2);
    click(&mut h, "Skip All");
    click(&mut h, "Skip All");
    assert_eq!(prompt(&h), None);
    for _ in 0..5 {
        click(&mut h, "Ignore");
        assert_eq!(prompt(&h), None);
    }
    assert!(complete_shown(&h));
}

#[test]
fn cancel_ends_the_check_and_keeps_the_changes_made() {
    let mut h = open(document("T1 typos"));
    click(&mut h, "Change");
    click(&mut h, "Ignore");
    assert_eq!(prompt(&h), Some(hidden("T4 hidden")));
    click(&mut h, "Cancel");
    assert!(h.state().ui.type_panels.spell.is_none(), "the dialog is closed");
    assert!(!dialog_shown(&h) && prompt(&h).is_none());
    assert!(h.state().ui.dialogs.is_empty(), "no other alert");
    assert_eq!(text_of(h.state(), "T1 typos"), "The quick brwn fox");
}

/// Cancel at the last prompt (T10) shows no "Spell check complete.".
#[test]
fn cancel_at_the_end_shows_no_end_of_the_check() {
    let mut h = open(document("T1 typos"));
    ignore(&mut h, 2);
    click(&mut h, "Skip");
    click(&mut h, "Skip");
    ignore(&mut h, 5);
    click(&mut h, "Skip");
    assert_eq!(prompt(&h), Some(locked("T10 locked2")));
    assert!(!complete_shown(&h));
    click(&mut h, "Cancel");
    assert!(!complete_shown(&h) && !dialog_shown(&h));
    assert!(h.state().ui.dialogs.is_empty());
}

/// Enter is Skip; Esc is Cancel and keeps the changes.
#[test]
fn enter_skips_and_escape_cancels() {
    let mut h = open(document("T1 typos"));
    click(&mut h, "Change");
    click(&mut h, "Ignore");
    assert_eq!(prompt(&h), Some(hidden("T4 hidden")));
    key(&mut h, egui::Key::Enter);
    assert_eq!(prompt(&h), Some(locked("T5 locked")));
    key(&mut h, egui::Key::Escape);
    assert!(h.state().ui.type_panels.spell.is_none() && prompt(&h).is_none());
    assert_eq!(text_of(h.state(), "T1 typos"), "The quick brwn fox");
}

/// Started on T9 hidden2: its prompt and T10 locked2's come before the dialog opens.
#[test]
fn a_hidden_first_layer_is_asked_about_before_the_dialog_opens() {
    let mut h = open(document("T9 hidden2"));
    assert_eq!(prompt(&h), Some(hidden("T9 hidden2")));
    assert!(!dialog_shown(&h), "the dialog is not open yet");
    click(&mut h, "Skip");
    assert_eq!(prompt(&h), Some(locked("T10 locked2")));
    assert!(!dialog_shown(&h));
    click(&mut h, "Skip");
    assert_eq!(prompt(&h), None);
    assert!(dialog_shown(&h));
    assert_eq!(to_come(h.state())[0], "T1 typos: Teh");
}

/// Only a hidden and a locked type layer: both prompts, then the dialog with nothing to check.
#[test]
fn with_only_hidden_and_locked_layers_the_prompts_come_then_the_empty_dialog() {
    let mut app = typos_in_every_layer();
    app.run("layer.delete", json!({})).unwrap();
    app.run("layer.select", json!({"layer": "T2 hidden"})).unwrap();
    let mut h = open(app);
    assert_eq!(prompt(&h), Some(hidden("T2 hidden")));
    click(&mut h, "Skip");
    assert_eq!(prompt(&h), Some(locked("T3 locked")));
    assert!(!dialog_shown(&h));
    click(&mut h, "Skip");
    assert!(dialog_shown(&h) && complete_shown(&h));
}

/// Started on the hidden layer that has typos: after the prompts only T1's words come up.
#[test]
fn a_hidden_layers_typo_never_comes_up() {
    let mut app = typos_in_every_layer();
    app.run("layer.select", json!({"layer": "T2 hidden"})).unwrap();
    let mut h = open(app);
    click(&mut h, "Skip");
    click(&mut h, "Skip");
    assert_eq!(to_come(h.state()), ["T1 typos: Teh", "T1 typos: brwn"]);
}

/// Changing the last word brings up the prompt of the layer after it.
#[test]
fn changing_the_last_word_brings_up_the_prompt_after_it() {
    let mut h = open(typos_in_every_layer());
    click(&mut h, "Ignore");
    assert_eq!(prompt(&h), None);
    click(&mut h, "Change");
    assert_eq!(prompt(&h), Some(hidden("T2 hidden")));
}

/// Ignore All (#14176) and Change All drop words before the passed layer: its prompt still comes
/// right after T8.
#[test]
fn ignore_all_and_change_all_keep_the_prompt_after_the_layer_it_comes_after() {
    for button in ["Ignore All", "Change All"] {
        let mut h = open(document("T8 repeat2"));
        click(&mut h, button);
        assert_eq!(to_come(h.state())[0], "T8 repeat2: Teh", "{button}");
        assert_eq!(prompt(&h), None, "{button}");
        click(&mut h, "Ignore");
        assert_eq!(prompt(&h), Some(hidden("T9 hidden2")), "{button}");
    }
}

/// A playback paused at Check Spelling: Cancel keeps the correction and stops the action.
#[test]
fn cancel_stops_an_action_paused_at_check_spelling_and_keeps_the_change() {
    let mut app = typos_in_every_layer();
    let mut action =
        Action::new("Spell", vec![("edit.checkSpelling".into(), json!({"allLayers": true})), ("layer.new.layer".into(), json!({"name": "after"}))]);
    action.dialogs.insert(0);
    app.session.actions.list.push(action);
    let mut h = harness(app);
    let v = crate::actions::play(h.state_mut(), &json!({"action": "Spell"})).unwrap();
    assert_eq!(v["paused"]["dialog"], crate::type_panels_ui::SPELLING, "{v}");
    h.run_steps(4);
    click(&mut h, "Change");
    click(&mut h, "Ignore");
    assert_eq!(prompt(&h), Some(hidden("T2 hidden")));
    click(&mut h, "Cancel");
    let app = h.state();
    assert!(app.ui.type_panels.spell.is_none());
    assert_eq!(text_of(app, "T1 typos"), "The quick brwn fox", "the change stays");
    assert!(!app.session.active().unwrap().doc.walk().iter().any(|(_, _, l)| l.name == "after"), "the rest of the action did not run");
    assert_eq!(app.session.actions.playing, 0, "the playback is over");
    assert_eq!(app.ui.status, "Action playback stopped");
}

/// What `ui.menu.invoke` returns tells an agent that a prompt waits.
#[test]
fn opening_the_dialog_returns_the_skipped_layers() {
    let mut app = document("T9 hidden2");
    let r = super::menu(&mut app, "edit.checkSpelling", &json!({})).unwrap().unwrap();
    let reasons: Vec<&str> = r["skipped"].as_array().unwrap().iter().map(|k| k["reason"].as_str().unwrap()).collect();
    assert_eq!(reasons, ["hidden", "locked", "hidden", "locked"]);
    assert_eq!(r["skipped"][0]["at"], 0);
}

/// Not measured: a ticked "Don't show again" changes nothing for now.
#[test]
fn a_ticked_dont_show_again_still_asks_next_time() {
    let mut h = open(document("T9 hidden2"));
    click(&mut h, "Don't show again");
    click(&mut h, "Cancel");
    assert!(open_edit_menu(&mut h));
    h.get_by_label_contains("Check Spelling").click();
    h.run_steps(4);
    assert_eq!(prompt(&h), Some(hidden("T9 hidden2")));
}
