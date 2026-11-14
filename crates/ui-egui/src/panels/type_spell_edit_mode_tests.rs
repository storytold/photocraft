//! #14186: Edit > Check Spelling... opened while editing type checks only the words of that layer
//! that start at or after the caret, or that lie inside the selection, with Check All Layers greyed
//! and unticked; the text stays in edit mode after Done. A Change keeps the following words, the
//! caret and the selection in place. `edit.checkSpelling` `list` takes the range as `range`.

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::spell_enabled_ui_tests::{app, open_edit_menu};
use super::spell_order_ui_tests::words;
use crate::PhotocraftApp;

/// A type layer with `text` over one with its own typo, edited as a double click on its Layers
/// thumbnail does (`type.editText`), which selects all of it.
fn editing(text: &str) -> Harness<'static, PhotocraftApp> {
    let mut app = app(true);
    let id = app.run("type.create", json!({"x": 10, "y": 60, "text": text})).unwrap()["layer"].clone();
    app.run("type.create", json!({"x": 10, "y": 160, "text": "zzapp other"})).unwrap();
    app.run("layer.select", json!({"layer": id})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
        |ui, app| {
            let ctx = ui.ctx().clone();
            // The shortcuts (the Type tool's keys among them) wait for the app's fonts.
            if ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::shortcuts::handle(app, &ctx);
            }
            crate::menus::menu_bar(app, ui);
            super::windows(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "type.editText", json!({})).unwrap();
    h.run_steps(2);
    h
}

/// `from` (Home or End), then `n` presses of `step` (with ⇧ when `select`).
fn keys(h: &mut Harness<'static, PhotocraftApp>, from: Key, step: Key, n: usize, select: bool) {
    h.key_press(from);
    h.run_steps(1);
    for _ in 0..n {
        h.key_press_modifiers(if select { Modifiers::SHIFT } else { Modifiers::NONE }, step);
        h.run_steps(1);
    }
}

/// Home, then `n` presses of → (with ⇧ when `select`).
fn place(h: &mut Harness<'static, PhotocraftApp>, n: usize, select: bool) {
    keys(h, Key::Home, Key::ArrowRight, n, select);
}

/// The text edit's anchor and caret.
fn caret(app: &PhotocraftApp) -> (usize, usize) {
    let ed = app.ui.text_edit.as_ref().expect("the type is still being edited");
    (ed.anchor, ed.caret)
}

fn check_spelling(h: &mut Harness<'static, PhotocraftApp>) {
    assert!(open_edit_menu(h));
    h.get_by_label_contains("Check Spelling").click();
    h.run_steps(4);
}

fn click(h: &mut Harness<'static, PhotocraftApp>, button: &str) {
    h.get_by_label(button).click();
    h.run_steps(4);
}

/// Sets Change To as typing into the field would.
fn change_to(h: &mut Harness<'static, PhotocraftApp>, to: &str) {
    h.state_mut().ui.type_panels.spell.as_mut().expect("the Check Spelling dialog is open").change_to = to.into();
}

fn text(app: &PhotocraftApp) -> String {
    crate::type_tool::target_text(app).map_or(String::new(), |t| t.text.clone())
}

fn no_error(app: &PhotocraftApp) {
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

#[test]
fn with_a_caret_it_checks_from_the_caret_to_the_end_of_that_layer() {
    let mut h = editing("Teh quick brwn fox");
    let layer = h.state().ui.text_edit.as_ref().map(|ed| ed.layer);
    place(&mut h, 9, false);
    assert_eq!(caret(h.state()), (9, 9), "the caret is after \"quick\"");
    check_spelling(&mut h);
    assert_eq!(words(h.state()), ["brwn"], "not \"Teh\" before the caret, nor the other layer's \"zzapp\"");

    let all_layers = h.get_by_label("Check All Layers");
    assert!(all_layers.accesskit_node().is_disabled(), "Check All Layers is greyed");
    assert_eq!(all_layers.accesskit_node().toggled(), Some(egui::accesskit::Toggled::False), "and unticked");

    click(&mut h, "Ignore");
    assert!(h.query_by_label("Spell check complete.").is_some(), "no wrap to the start of the text");
    click(&mut h, "Done");
    assert!(h.state().ui.type_panels.spell.is_none());
    assert_eq!(h.state().ui.text_edit.as_ref().map(|ed| ed.layer), layer, "the layer stays in edit mode");
}

#[test]
fn a_word_the_caret_is_in_is_not_checked() {
    let mut h = editing("Teh quick brwn fox");
    place(&mut h, 11, false);
    check_spelling(&mut h);
    assert!(words(h.state()).is_empty(), "\"brwn\" starts before the caret: {:?}", words(h.state()));
    no_error(h.state());
}

#[test]
fn a_caret_at_the_end_of_the_text_checks_nothing() {
    let mut h = editing("Teh quick brwn fox");
    keys(&mut h, Key::End, Key::ArrowRight, 0, false);
    assert_eq!(caret(h.state()), (18, 18));
    check_spelling(&mut h);
    assert!(words(h.state()).is_empty(), "{:?}", words(h.state()));
    assert!(h.query_by_label("Spell check complete.").is_some());
    no_error(h.state());
}

#[test]
fn with_a_selection_it_checks_only_the_words_inside_it() {
    let mut h = editing("Teh quick brwn fox");
    place(&mut h, 6, true);
    assert_eq!(caret(h.state()), (0, 6), "\"Teh qu\" is selected");
    check_spelling(&mut h);
    assert_eq!(words(h.state()), ["Teh"], "not \"brwn\" after the selection");
    assert!(h.get_by_label("Check All Layers").accesskit_node().is_disabled());

    let mut h = editing("Teh quick brwn fox");
    place(&mut h, 12, true);
    check_spelling(&mut h);
    assert_eq!(words(h.state()), ["Teh"], "\"Teh quick br\" is selected: \"brwn\" is only partly inside");
}

#[test]
fn a_selection_made_right_to_left_is_checked_the_same() {
    let mut h = editing("Teh quick brwn fox");
    keys(&mut h, Key::End, Key::ArrowLeft, 8, true);
    assert_eq!(caret(h.state()), (18, 10), "\"brwn fox\" is selected from the end");
    check_spelling(&mut h);
    assert_eq!(words(h.state()), ["brwn"]);
}

/// A Change longer than the word moves the end of the selection with it: "brwn", selected, still
/// comes up after it, and the selection still ends after "brwn".
#[test]
fn a_change_keeps_the_rest_of_the_selection_in_the_check() {
    let mut h = editing("Teh brwn fox qwzx");
    place(&mut h, 8, true);
    check_spelling(&mut h);
    assert_eq!(words(h.state()), ["Teh", "brwn"]);
    change_to(&mut h, "The really");
    click(&mut h, "Change");
    assert_eq!(text(h.state()), "The really brwn fox qwzx");
    assert_eq!(words(h.state()), ["brwn"], "\"brwn\" is still in the selection, \"qwzx\" is not");
    assert_eq!(caret(h.state()), (0, 15), "the selection moved with the text");
}

/// Change All replaces the word in the whole layer, as without a text edit; the check then goes
/// on from the same place in the shorter text, and the caret moves with it.
#[test]
fn change_all_before_the_caret_moves_the_start_of_the_check() {
    let mut h = editing("Teh xq Teh brwn");
    place(&mut h, 7, false);
    check_spelling(&mut h);
    assert_eq!(words(h.state()), ["Teh", "brwn"], "from the second \"Teh\"; \"xq\" is before the caret");
    change_to(&mut h, "a");
    click(&mut h, "Change All");
    assert_eq!(text(h.state()), "a xq a brwn");
    assert_eq!(words(h.state()), ["brwn"], "still not \"xq\"");
    assert_eq!(caret(h.state()), (5, 5), "the caret is still before the second word");
    no_error(h.state());
}

/// The text gets shorter while the dialog is open: the next list keeps to what is left.
#[test]
fn a_shorter_text_keeps_the_check_in_range() {
    let mut h = editing("Teh brwn qwzx fox");
    place(&mut h, 0, false);
    check_spelling(&mut h);
    assert_eq!(words(h.state()), ["Teh", "brwn", "qwzx"]);
    let layer = h.state().ui.text_edit.as_ref().map(|ed| ed.layer);
    h.state_mut().run("type.insertText", json!({"layer": layer, "text": "!", "range": [13, 17]})).unwrap();
    assert_eq!(text(h.state()), "Teh brwn qwzx!");
    click(&mut h, "Ignore All");
    no_error(h.state());
    assert_eq!(words(h.state()), ["brwn", "qwzx"]);
}

/// A recorded Check Spelling step played while editing type checks as recorded (all layers), not
/// the text edit's range, which only the menu uses.
#[test]
fn a_played_step_keeps_its_recorded_scope() {
    let mut h = editing("Teh quick brwn fox");
    place(&mut h, 9, false);
    let mut action = photocraft_engine::actions_cmds::Action::new("Spell", vec![("edit.checkSpelling".into(), json!({"allLayers": true}))]);
    action.dialogs.insert(0);
    h.state_mut().session.actions.list.push(action);
    let played = crate::actions::play(h.state_mut(), &json!({"action": "Spell"})).unwrap();
    assert_eq!(played["paused"]["dialog"], super::SPELLING, "{played}");
    let spell = h.state().ui.type_panels.spell.as_ref().expect("the Check Spelling dialog is open");
    assert!(spell.all_layers, "Check All Layers as recorded");
    assert_eq!(words(h.state()), ["Teh", "brwn", "zzapp"]);
}

/// "start..end word" for each word `list` returns.
fn listed(app: &mut PhotocraftApp, p: Value) -> Vec<String> {
    let r = app.session.execute("edit.checkSpelling", p).unwrap();
    r["misspellings"].as_array().unwrap().iter().map(|m| format!("{}..{} {}", m["start"], m["end"], m["word"].as_str().unwrap())).collect()
}

#[test]
fn list_with_a_range_lists_only_the_words_inside_it() {
    let mut app = app(true);
    // "naïve" (not in the dictionary) and the snowman make byte offsets differ from characters.
    let id = app.run("type.create", json!({"x": 10, "y": 60, "text": "naïve ☃ Teh quick brwn fox"})).unwrap()["layer"].clone();
    assert_eq!(listed(&mut app, json!({"layer": id})), ["0..5 naïve", "8..11 Teh", "18..22 brwn"]);
    for (range, want) in [
        ([17, 26], vec!["18..22 brwn"]),
        ([8, 14], vec!["8..11 Teh"]),
        ([8, 11], vec!["8..11 Teh"]),
        ([0, 5], vec!["0..5 naïve"]),
        ([9, 10], vec![]),
        ([9, 26], vec!["18..22 brwn"]),
        ([14, 19], vec![]),
        ([4, 6], vec![]),
        ([11, 18], vec![]),
        ([26, 26], vec![]),
        ([0, 26], vec!["0..5 naïve", "8..11 Teh", "18..22 brwn"]),
    ] {
        assert_eq!(listed(&mut app, json!({"layer": id, "range": range})), want, "{range:?}");
    }
    assert_eq!(listed(&mut app, json!({"layer": id, "range": null})).len(), 3, "a null range is no range");
    let none = app.session.execute("edit.checkSpelling", json!({"layer": id, "suggestions": 0})).unwrap();
    assert_eq!(none["misspellings"][0]["suggestions"], json!([]), "no suggestions asked for");
}

#[test]
fn a_bad_range_is_an_error() {
    let mut app = app(true);
    let id = app.run("type.create", json!({"x": 10, "y": 60, "text": "Teh end"})).unwrap()["layer"].clone();
    let pixels = app.run("layer.new.layer", json!({"name": "Pixels"})).unwrap()["layer"].clone();
    for p in [
        json!({"layer": id, "range": [3, 1]}),
        json!({"layer": id, "range": [0, 8]}),
        json!({"layer": id, "range": [0, u64::MAX]}),
        json!({"layer": id, "range": [0]}),
        json!({"layer": id, "range": [0, -1]}),
        json!({"layer": id, "range": "0..3"}),
        json!({"range": [0, 3]}),
        json!({"layers": [id], "range": [0, 3]}),
        json!({"layer": pixels, "range": [0, 0]}),
    ] {
        assert!(app.session.execute("edit.checkSpelling", p.clone()).is_err(), "{p}");
    }
    assert_eq!(listed(&mut app, json!({"layer": id, "range": [0, 7]})), ["0..3 Teh"]);
}
