//! #14181: in Check Spelling, Change To holds the first suggestion, or the unknown word itself
//! when it has none, and each time a word comes up (the dialog opening, Ignore, Ignore All,
//! Change, the next Check Spelling step of an action) it has the keyboard focus with all its text
//! selected, so typing replaces it at once.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::tests::app_with_text;
use crate::PhotocraftApp;

/// The type panels' windows with Edit > Check Spelling... opened from the menu on `text`.
fn harness(text: &str) -> Harness<'static, PhotocraftApp> {
    let mut app = app_with_text(text);
    super::menu(&mut app, "edit.checkSpelling", &json!({})).unwrap().unwrap();
    harness_with(app)
}

/// The type panels' windows of `app`, whose Check Spelling dialog is open.
fn harness_with(mut app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    // The fonts are set up after the harness's first frame: the window appears after it.
    let spell = app.ui.type_panels.spell.take();
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| super::windows(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.state_mut().ui.type_panels.spell = spell;
    h.run_steps(3);
    h
}

/// The word Not in Dictionary shows and what Change To holds on screen.
fn shown(h: &Harness<'static, PhotocraftApp>) -> (String, String) {
    let value = |label| h.get_by_role_and_label(egui::accesskit::Role::TextInput, label).accesskit_node().value().unwrap_or_default();
    (value("Not in Dictionary:"), value("Change To:"))
}

/// Change To has the focus and typing `text` replaces all of it.
fn typing_replaces_change_to(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    let field = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Change To:");
    assert!(field.accesskit_node().is_focused(), "Change To has the keyboard focus");
    h.event(egui::Event::Text(text.into()));
    h.run_steps(2);
    assert_eq!(shown(h).1, text, "typing replaced the whole of Change To");
    assert_eq!(h.state().ui.type_panels.spell.as_ref().unwrap().change_to, text);
}

fn press(h: &mut Harness<'static, PhotocraftApp>, button: &str) {
    h.get_by_label(button).click();
    h.run_steps(3);
}

#[test]
fn a_word_without_suggestions_shows_itself_in_change_to() {
    let mut h = harness("xjqvzkwp");
    let item = h.state().ui.type_panels.spell.as_ref().unwrap().items[0].clone();
    assert_eq!(item["suggestions"], json!([]), "the dictionary has nothing for it");
    assert_eq!(shown(&h), ("xjqvzkwp".into(), "xjqvzkwp".into()));
    typing_replaces_change_to(&mut h, "quick");
}

#[test]
fn a_word_with_suggestions_shows_the_first_in_change_to() {
    let mut h = harness("Teh cat");
    let item = h.state().ui.type_panels.spell.as_ref().unwrap().items[0].clone();
    assert_eq!(shown(&h), ("Teh".into(), item["suggestions"][0].as_str().unwrap().into()));
    assert_eq!(shown(&h).1, "The");
    typing_replaces_change_to(&mut h, "Ten");
}

#[test]
fn ignore_moves_on_with_change_to_focused_and_selected() {
    let mut h = harness("Teh xjqvzkwp brwn");
    press(&mut h, "Ignore");
    assert_eq!(shown(&h), ("xjqvzkwp".into(), "xjqvzkwp".into()), "the next word, without suggestions, shows itself");
    typing_replaces_change_to(&mut h, "fox");
    press(&mut h, "Ignore");
    assert_eq!(shown(&h).0, "brwn");
    assert_eq!(shown(&h).1, h.state().ui.type_panels.spell.as_ref().unwrap().items[2]["suggestions"][0].as_str().unwrap());
    typing_replaces_change_to(&mut h, "brown");
}

#[test]
fn ignore_all_moves_on_with_change_to_focused_and_selected() {
    let mut h = harness("Teh cat Teh brwn");
    press(&mut h, "Ignore All");
    let next = h.state().ui.type_panels.spell.as_ref().unwrap().items[0].clone();
    assert_eq!(shown(&h), ("brwn".into(), next["suggestions"][0].as_str().unwrap().into()), "both Teh are skipped");
    typing_replaces_change_to(&mut h, "brown");
}

#[test]
fn change_moves_on_with_change_to_focused_and_selected() {
    let mut h = harness("Teh xjqvzkwp");
    press(&mut h, "Change");
    assert_eq!(shown(&h), ("xjqvzkwp".into(), "xjqvzkwp".into()));
    typing_replaces_change_to(&mut h, "fox");
}

#[test]
fn reopening_on_the_same_word_focuses_and_selects_change_to_again() {
    let mut h = harness("xjqvzkwp");
    typing_replaces_change_to(&mut h, "quick");
    press(&mut h, "Done");
    assert!(h.state().ui.type_panels.spell.is_none());
    h.run_steps(2);
    super::menu(h.state_mut(), "edit.checkSpelling", &json!({})).unwrap().unwrap();
    h.run_steps(3);
    assert_eq!(shown(&h), ("xjqvzkwp".into(), "xjqvzkwp".into()));
    typing_replaces_change_to(&mut h, "fox");
}

/// An action with two Check Spelling steps, both with their dialog on: Done on the first opens the
/// second at once (in the same pass, on the same word), and Change To has the focus again.
#[test]
fn the_next_check_spelling_step_of_an_action_focuses_and_selects_change_to() {
    let mut app = app_with_text("xjqvzkwp");
    let step = ("edit.checkSpelling".to_string(), json!({"allLayers": true}));
    let mut action = photocraft_engine::actions_cmds::Action::new("Spell twice", vec![step.clone(), step]);
    action.dialogs.extend([0, 1]);
    app.session.actions.list.push(action);
    let paused = crate::actions::play(&mut app, &json!({"action": "Spell twice"})).unwrap();
    assert_eq!(paused["paused"]["dialog"], super::SPELLING, "{paused}");
    let mut h = harness_with(app);
    typing_replaces_change_to(&mut h, "quick");
    press(&mut h, "Done");
    assert!(h.state().ui.type_panels.spell.is_some(), "the second step opened its dialog");
    assert_eq!(shown(&h), ("xjqvzkwp".into(), "xjqvzkwp".into()));
    typing_replaces_change_to(&mut h, "fox");
}
