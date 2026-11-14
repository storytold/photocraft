//! #14178: when Check Spelling has handled the last unknown word of its scope, a "Spell check
//! complete." alert with one OK button comes up over the dialog, and OK closes both, as Done does.
//! With no unknown word in the scope only the alert shows.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_engine::actions_cmds::Action;
use serde_json::{Value, json};

use super::tests::app_with_text;
use crate::PhotocraftApp;
use crate::canvas_tool_menu::count_tests::call;
use crate::state::DialogKind;

const COMPLETE: &str = "Spell check complete.";

/// The type panels' windows and the dialogs over them, drawn in the frame's order.
fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(
        |ui, app| {
            // The fonts are set up after the harness's first frame.
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                super::windows(app, ui.ctx());
                crate::dialogs::show(app, ui.ctx());
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    h
}

/// Edit > Check Spelling... on a document whose one type layer says `text`.
fn checking(text: &str) -> Harness<'static, PhotocraftApp> {
    let mut app = app_with_text(text);
    super::menu(&mut app, "edit.checkSpelling", &json!({})).unwrap().unwrap();
    harness(app)
}

fn press(h: &mut Harness<'static, PhotocraftApp>, button: &str) {
    h.get_by_label(button).click();
    h.run_steps(4);
}

/// The messages of the open alerts.
fn alerts(app: &PhotocraftApp) -> Vec<&str> {
    app.ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message").and_then(Value::as_str)).collect()
}

/// Whether the Check Spelling dialog is on screen.
fn dialog_shown(h: &Harness<'static, PhotocraftApp>) -> bool {
    h.query_by_label("Done").is_some()
}

/// The text of the document's type layer.
fn text(app: &PhotocraftApp) -> String {
    let doc = &app.session.active().unwrap().doc;
    let texts: Vec<String> = doc
        .walk()
        .iter()
        .filter_map(|(_, _, l)| match &l.content {
            photocraft_doc::LayerContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), 1, "{texts:?}");
    texts[0].clone()
}

fn layer_names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.walk().iter().map(|(_, _, l)| l.name.clone()).collect()
}

/// An action whose Check Spelling step (dialog on) is followed by a New Layer "after", played up
/// to its pause at Check Spelling.
fn paused_action(text: &str) -> PhotocraftApp {
    let mut app = app_with_text(text);
    let mut a = Action::new("Spell", vec![("edit.checkSpelling".into(), json!({"allLayers": false})), ("layer.new.layer".into(), json!({"name": "after"}))]);
    a.dialogs.insert(0);
    app.session.actions.list.push(a);
    let v = crate::actions::play(&mut app, &json!({"action": "Spell"})).unwrap();
    assert_eq!(v["paused"]["dialog"], super::SPELLING, "{v}");
    app
}

fn afters(app: &PhotocraftApp) -> usize {
    layer_names(app).iter().filter(|n| *n == "after").count()
}

/// The names of the active document's history states.
fn history(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries()
}

#[test]
fn ok_on_the_alert_after_the_last_word_closes_it_and_the_dialog() {
    let mut h = checking("Teh xjqvzkwp");
    press(&mut h, "Change");
    assert!(alerts(h.state()).is_empty(), "a word is left: no alert yet");
    press(&mut h, "Ignore");
    assert_eq!(alerts(h.state()), [COMPLETE]);
    assert!(h.query_by_label(COMPLETE).is_some(), "the alert is on screen");
    assert!(dialog_shown(&h), "over the dialog");
    press(&mut h, "OK");
    assert!(h.state().ui.dialogs.is_empty(), "OK closed the alert");
    assert!(h.state().ui.type_panels.spell.is_none(), "and Check Spelling");
    assert!(!dialog_shown(&h));
    assert_eq!(text(h.state()), "The xjqvzkwp", "the change is kept");
}

/// OK keeps every change of the run, Change and Change All alike, and adds or takes away no
/// history state.
#[test]
fn ok_keeps_every_change() {
    let mut h = checking("Teh brwon brwon xjqvzkwp");
    press(&mut h, "Change");
    press(&mut h, "Change All");
    press(&mut h, "Ignore");
    assert_eq!(alerts(h.state()), [COMPLETE]);
    let (changed, states) = (text(h.state()), history(h.state()));
    assert!(!changed.contains("Teh") && !changed.contains("brwon") && changed.ends_with(" xjqvzkwp"), "{changed}");
    press(&mut h, "OK");
    assert!(h.state().ui.type_panels.spell.is_none());
    assert_eq!(text(h.state()), changed);
    assert_eq!(history(h.state()), states);
}

#[test]
fn with_no_unknown_word_only_the_alert_shows() {
    let mut h = checking("The cat");
    assert_eq!(alerts(h.state()), [COMPLETE]);
    assert!(h.query_by_label(COMPLETE).is_some());
    for _ in 0..4 {
        h.step();
        assert!(!dialog_shown(&h), "no Check Spelling dialog in any frame");
    }
    press(&mut h, "OK");
    assert!(h.state().ui.dialogs.is_empty());
    assert!(h.state().ui.type_panels.spell.is_none());
}

/// Esc and Enter close the alert as on every other alert, and so end Check Spelling as OK does.
#[test]
fn escape_and_enter_on_the_alert_are_ok() {
    for key in [egui::Key::Escape, egui::Key::Enter] {
        let mut h = checking("Teh cat");
        press(&mut h, "Change");
        assert_eq!(alerts(h.state()), [COMPLETE]);
        h.key_press(key);
        h.run_steps(4);
        assert!(h.state().ui.dialogs.is_empty(), "{key:?}");
        assert!(h.state().ui.type_panels.spell.is_none(), "{key:?}");
        assert_eq!(text(h.state()), "The cat");
    }
}

/// OK is Done for an action paused at its Check Spelling step: the action goes on.
#[test]
fn ok_continues_an_action_paused_at_check_spelling() {
    for words in ["Teh cat", "The cat"] {
        let mut h = harness(paused_action(words));
        if words == "Teh cat" {
            assert!(alerts(h.state()).is_empty(), "{words}");
            press(&mut h, "Change");
        } else {
            assert!(!dialog_shown(&h), "only the alert for {words}");
        }
        assert_eq!(alerts(h.state()), [COMPLETE], "{words}");
        assert_eq!(afters(h.state()), 0, "the action waits");
        press(&mut h, "OK");
        assert!(h.state().ui.type_panels.spell.is_none(), "{words}");
        assert_eq!(afters(h.state()), 1, "the action went on once after {words}: {}", h.state().ui.status);
        assert_eq!(text(h.state()), "The cat");
    }
}

/// Check Spelling opened again while its alert is up for a paused action: the old alert closes,
/// the action keeps waiting, and the new run's OK continues it once.
#[test]
fn reopening_with_the_alert_up_keeps_the_waiting_action() {
    let mut app = paused_action("The cat");
    let token = app.ui.type_panels.spell.as_ref().and_then(|d| d.resume);
    let old = app.ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).map(|d| d.id).expect("the alert is up");
    super::menu(&mut app, "edit.checkSpelling", &json!({})).unwrap().unwrap();
    assert!(app.ui.dialogs.iter().all(|d| d.id != old), "the old alert closed");
    assert_eq!(alerts(&app), [COMPLETE], "one alert");
    assert_eq!(app.ui.type_panels.spell.as_ref().and_then(|d| d.resume), token);
    assert_eq!(afters(&app), 0, "the action did not go on");
    let mut h = harness(app);
    press(&mut h, "OK");
    assert_eq!(afters(h.state()), 1);
}

/// OK while an action is recorded records one Check Spelling step, as Done does.
#[test]
fn ok_records_the_check_spelling_step() {
    let mut app = app_with_text("Teh cat");
    app.run("actions.record", json!({"name": "Spell"})).unwrap();
    super::menu(&mut app, "edit.checkSpelling", &json!({})).unwrap().unwrap();
    let mut h = harness(app);
    press(&mut h, "Change");
    press(&mut h, "OK");
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let action = &h.state().session.actions.list[0];
    assert_eq!(action.steps, vec![("edit.checkSpelling".to_string(), json!({"allLayers": true}))]);
    assert_eq!(action.dialogs.iter().copied().collect::<Vec<_>>(), [0]);
}

/// The control channel sees the alert in `ui.inspect`, and `ui.dialog.confirm` on it is Done
/// from the next frame.
#[test]
fn the_alert_is_driven_through_the_control_channel() {
    let mut app = app_with_text("The cat");
    let ctx = egui::Context::default();
    call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "edit.checkSpelling"}));
    let state = call(&mut app, &ctx, "ui.inspect", json!({}));
    let dialogs = state["result"]["dialogs"].as_array().unwrap();
    assert_eq!(dialogs.len(), 1, "{dialogs:?}");
    assert_eq!(dialogs[0]["fields"]["message"], COMPLETE);
    let v = call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog": dialogs[0]["id"]}));
    assert_eq!(v["ok"], true, "{v}");
    assert!(app.ui.dialogs.is_empty());
    let mut frame = ctx.run_ui(Default::default(), |ui| super::windows(&mut app, ui.ctx()));
    frame.textures_delta.clear();
    assert!(app.ui.type_panels.spell.is_none(), "confirming the alert closed Check Spelling");
}
