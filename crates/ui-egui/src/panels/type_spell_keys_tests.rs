//! #14179: Enter and Escape close the Check Spelling dialog like Done, keeping the changes made so
//! far; neither key reaches the canvas or the tool shortcuts while the dialog is open.

use super::type_panels_ui::{menu, windows};
use crate::PhotocraftApp;
use egui::vec2;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use std::cell::Cell;
use std::rc::Rc;

fn app_with_text(text: &str) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 120, "background": "white"})).unwrap();
    app.run("type.create", json!({"x": 10, "y": 60, "text": text})).unwrap();
    app.sync_views();
    app
}

fn layer_text(app: &PhotocraftApp) -> String {
    let s = app.session.active().unwrap();
    let id = s.active_layer.unwrap();
    let photocraft_doc::LayerContent::Text(t) = &s.doc.layer(id).unwrap().content else { panic!("not text") };
    t.text.clone()
}

/// Opens the dialog on `text` and builds a harness drawing it. `seen` records whether an Enter or
/// Escape key press was still visible to the rest of the frame (the canvas, the shortcuts).
fn harness(text: &str, seen: Rc<Cell<(bool, bool)>>) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut app = app_with_text(text);
    menu(&mut app, "edit.checkSpelling", &json!({})).unwrap().unwrap();
    // The fonts are set up after the harness's first frame: the window appears after it.
    let spell = app.ui.type_panels.spell.take();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
        move |ui, app| {
            windows(app, ui.ctx());
            let keys = ui.ctx().input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
            if keys.0 || keys.1 {
                let old = seen.get();
                seen.set((old.0 || keys.0, old.1 || keys.1));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.state_mut().ui.type_panels.spell = spell;
    h.run_steps(3);
    h
}

fn change_first_word(app: &mut PhotocraftApp) {
    let item = app.ui.type_panels.spell.as_ref().unwrap().items[0].clone();
    let change = json!({"action": "change", "layer": item["layer"], "start": item["start"], "end": item["end"], "word": "Teh", "replace": "The"});
    crate::actions::spelling_change(app, change).unwrap();
}

#[test]
fn escape_closes_the_dialog_and_keeps_the_changes_made() {
    let seen = Rc::new(Cell::new((false, false)));
    let mut h = harness("Teh cat. Wrold", seen.clone());
    assert!(h.state().ui.type_panels.spell.is_some());
    // Change the first word the way the Change button does, refresh the list, then press Escape.
    change_first_word(h.state_mut());
    h.state_mut().ui.type_panels.spell.as_mut().unwrap().index = 0;
    h.run_steps(2);
    assert!(layer_text(h.state()).starts_with("The cat"), "the change was made: {}", layer_text(h.state()));
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().ui.type_panels.spell.is_none(), "Escape closed Check Spelling");
    assert!(layer_text(h.state()).starts_with("The cat"), "and the change stays: {}", layer_text(h.state()));
    assert!(!seen.get().1, "Escape did not reach the rest of the frame");
}

#[test]
fn enter_in_change_to_runs_done_and_does_not_change_the_word() {
    let seen = Rc::new(Cell::new((false, false)));
    let mut h = harness("Teh cat", seen.clone());
    let before = layer_text(h.state());
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Change To:").click();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().ui.type_panels.spell.is_none(), "Enter closed Check Spelling");
    assert_eq!(layer_text(h.state()), before, "the current word was not changed");
    assert!(!seen.get().0, "Enter did not reach the canvas or the tool shortcuts");
}

#[test]
fn enter_with_the_focus_elsewhere_runs_done_too() {
    let seen = Rc::new(Cell::new((false, false)));
    let mut h = harness("Teh cat", seen.clone());
    let before = layer_text(h.state());
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().ui.type_panels.spell.is_none());
    assert_eq!(layer_text(h.state()), before);
    assert!(!seen.get().0);
}

#[test]
fn the_keys_do_nothing_when_the_dialog_is_closed() {
    let seen = Rc::new(Cell::new((false, false)));
    let mut h = harness("Teh cat", seen.clone());
    h.state_mut().ui.type_panels.spell = None;
    h.key_press(egui::Key::Escape);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(seen.get().0 && seen.get().1, "with no dialog the keys reach the frame as before");
}
