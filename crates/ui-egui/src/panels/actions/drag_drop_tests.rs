//! #12373: an Actions panel row dragged onto the trash is deleted, onto Create new action duplicated.
use super::*;
use egui_kittest::{Harness, kittest::Queryable};

fn names(app: &PhotocraftApp) -> Vec<String> {
    app.session.actions.list.iter().map(|a| a.name.clone()).collect()
}

fn steps(app: &PhotocraftApp, action: usize) -> Vec<String> {
    app.session.actions.list[action].steps.iter().map(|s| s.0.clone()).collect()
}

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.actions.list.push(Action {
        name: "First".into(),
        steps: vec![("view.zoomOut".into(), json!({})), ("view.fitOnScreen".into(), json!({})), ("view.actualPixels".into(), json!({}))],
        ..Default::default()
    });
    app.session.actions.list.push(Action { name: "Second".into(), steps: vec![("view.zoomIn".into(), json!({}))], ..Default::default() });
    app.ui.actions.expanded = vec![true, false];
    let mut h = Harness::builder().with_size(vec2(320.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(4);
    h
}

fn drag(h: &mut Harness<'_, PhotocraftApp>, from: egui::Pos2, to: egui::Pos2) {
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerMoved(from + vec2(10.0, 0.0)));
    h.run_steps(2);
    h.event(egui::Event::PointerMoved(to));
    h.run_steps(2);
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
}

fn center(h: &Harness<'_, PhotocraftApp>, label: &str) -> egui::Pos2 {
    h.get_by_label(label).rect().center()
}

#[test]
fn step_dragged_onto_trash_deletes_only_that_step() {
    let mut h = harness();
    let (from, to) = (center(&h, "Fit on Screen"), center(&h, "Delete"));
    drag(&mut h, from, to);
    assert_eq!(names(h.state()), ["First", "Second"]);
    assert_eq!(steps(h.state(), 0), ["view.zoomOut", "view.actualPixels"]);
}

#[test]
fn action_dragged_onto_trash_is_deleted_even_when_another_is_selected() {
    let mut h = harness();
    h.state_mut().ui.actions.select(1, None);
    h.run_steps(2);
    let (from, to) = (center(&h, "First"), center(&h, "Delete"));
    drag(&mut h, from, to);
    assert_eq!(names(h.state()), ["Second"]);
    assert_eq!(h.state().ui.actions.selected, Some(0));
    assert_eq!(h.state().ui.actions.expanded.len(), 1);
}

#[test]
fn action_dragged_onto_plus_adds_a_copy_after_it() {
    let mut h = harness();
    let (from, to) = (center(&h, "First"), center(&h, "Create new action"));
    drag(&mut h, from, to);
    assert_eq!(names(h.state()).len(), 3);
    assert_eq!(names(h.state())[0], "First");
    assert_eq!(names(h.state())[2], "Second");
    assert_eq!(steps(h.state(), 1), steps(h.state(), 0));
    assert!(h.state().session.actions.recording.is_none());
}

#[test]
fn step_dragged_onto_plus_duplicates_the_step_in_place() {
    let mut h = harness();
    let (from, to) = (center(&h, "Fit on Screen"), center(&h, "Create new action"));
    drag(&mut h, from, to);
    assert_eq!(names(h.state()), ["First", "Second"]);
    assert_eq!(steps(h.state(), 0), ["view.zoomOut", "view.fitOnScreen", "view.fitOnScreen", "view.actualPixels"]);
}

#[test]
fn drag_released_over_empty_space_changes_nothing() {
    let mut h = harness();
    let from = center(&h, "Fit on Screen");
    drag(&mut h, from, pos2(160.0, 300.0));
    assert_eq!(names(h.state()), ["First", "Second"]);
    assert_eq!(steps(h.state(), 0).len(), 3);
    assert_eq!(steps(h.state(), 1).len(), 1);
}

#[test]
fn action_dropped_on_trash_while_recording_is_not_deleted() {
    let mut h = harness();
    h.state_mut().run("actions.record", json!({"name": "Rec"})).unwrap();
    h.run_steps(3);
    let (from, to) = (center(&h, "Second"), center(&h, "Delete"));
    drag(&mut h, from, to);
    assert!(names(h.state()).contains(&"Second".to_string()));
    assert!(names(h.state()).contains(&"First".to_string()));
}
