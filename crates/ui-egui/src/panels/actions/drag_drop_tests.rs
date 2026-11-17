//! #12373: an Actions panel row dropped on the trash is deleted, on Create new action duplicated.
//! The panel reports the drop as `Delete` or `Duplicate`; these tests run those events.
use super::*;
use craft::automation::actions::ActionStep;

fn names(app: &PhotocraftApp) -> Vec<String> {
    app.session.actions.list.actions().iter().map(|a| a.name.clone()).collect()
}

fn steps(app: &PhotocraftApp, action: usize) -> Vec<String> {
    app.session.actions.list.action(action).unwrap().steps.iter().map(|s| s.command.clone()).collect()
}

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let first = ["view.zoomOut", "view.fitOnScreen", "view.actualPixels"].map(|id| ActionStep::new(id, json!({}))).to_vec();
    app.session.actions.list.push(Action { name: "First".into(), steps: first, ..Default::default() }).unwrap();
    let second = Action { name: "Second".into(), steps: vec![ActionStep::new("view.zoomIn", json!({}))], ..Default::default() };
    app.session.actions.list.push(second).unwrap();
    app
}

#[test]
fn step_dragged_onto_trash_deletes_only_that_step() {
    let mut app = app();
    apply_actions_events(&mut app, vec![ActionsEvent::Delete(ActionSelection::Step(0, 1))]);
    assert_eq!(names(&app), ["First", "Second"]);
    assert_eq!(steps(&app, 0), ["view.zoomOut", "view.actualPixels"]);
    assert_eq!(steps(&app, 1).len(), 1);
}

#[test]
fn action_dragged_onto_trash_is_deleted_even_when_another_is_selected() {
    let mut app = app();
    app.ui.actions.panel.selected = Some(ActionSelection::Action(0));
    apply_actions_events(&mut app, vec![ActionsEvent::Delete(ActionSelection::Action(1))]);
    assert_eq!(names(&app), ["First"]);
    assert_eq!(app.ui.actions.panel.selected, None);
}

#[test]
fn action_dragged_onto_plus_adds_a_copy_after_it() {
    let mut app = app();
    app.ui.actions.panel.expanded.insert("First".into());
    apply_actions_events(&mut app, vec![ActionsEvent::Duplicate(ActionSelection::Action(0))]);
    let names = names(&app);
    assert_eq!(names.len(), 3);
    assert_eq!((names[0].as_str(), names[2].as_str()), ("First", "Second"));
    assert_eq!(steps(&app, 1), steps(&app, 0));
    assert_eq!(app.ui.actions.panel.selected, Some(ActionSelection::Action(1)));
    assert!(app.ui.actions.panel.expanded.contains(&names[1]), "the copy is open as its source was");
    assert!(app.session.actions.recording.is_none());
}

#[test]
fn step_dragged_onto_plus_duplicates_the_step_in_place() {
    let mut app = app();
    apply_actions_events(&mut app, vec![ActionsEvent::Duplicate(ActionSelection::Step(0, 1))]);
    assert_eq!(names(&app), ["First", "Second"]);
    assert_eq!(steps(&app, 0), ["view.zoomOut", "view.fitOnScreen", "view.fitOnScreen", "view.actualPixels"]);
    assert_eq!(app.ui.actions.panel.selected, Some(ActionSelection::Step(0, 2)));
}
