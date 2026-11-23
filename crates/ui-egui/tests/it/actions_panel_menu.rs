//! Real panel controls compose the remaining commands with consent and retained selection. The trials that read
//! the dialog host are unit tests in `panels/actions/panel_menu_tests.rs`.
use craft::automation::ui::actions::{ActionSelection, ActionsPanelCommands};
use craft::ui::runtime::commands::CommandRef;
use egui_kittest::Harness;
use photocraft_engine::{
    Session,
    actions_cmds::{Action, ActionStep},
    actions_edit_cmds,
};
use photocraft_ui_egui::{
    PhotocraftApp,
    control::{ControlRequest, Reply, handle},
    dialogs,
};
use serde_json::{Value, json};

/// A real dialog host over one selected action in a set.
fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(Session::new(), Default::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width":16,"height":12})).unwrap();
    let mut a = Action::new("A", vec![]);
    a.set = Some("S".into());
    app.session.actions.list.push(a).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Action(0));
    app.ui.actions.panel.expanded.insert("A".into());
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(1100.0, 900.0)).build_ui_state(|ui, app| dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    h
}

/// One control request through the public channel; the reply is the request's answer.
fn control(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (request, _reply) = ControlRequest::new(method, params);
    match handle(app, ctx, &request) {
        Reply::Done(reply) => reply,
        _ => panic!("{method} answers at once"),
    }
}

/// Fire the panel menu row of `command` as a click on it would.
fn fire(h: &mut Harness<'static, Box<PhotocraftApp>>, command: ActionsPanelCommands) -> Value {
    let ctx = h.ctx.clone();
    let reply = control(h.state_mut(), &ctx, "ui.menu.invoke", json!({"id": CommandRef::from(command).id}));
    h.run_steps(3);
    reply
}

/// The steps of action `A`.
fn steps(app: &PhotocraftApp) -> Vec<ActionStep> {
    app.session.actions.list.action(0).unwrap().steps.clone()
}

/// The checkbox changes recording policy rather than disabling actual painting.
#[test]
fn allow_tool_recording_checkbox_records_only_enabled_strokes() {
    let mut h = harness();
    h.state_mut().run("actions.record", json!({"action":"A"})).unwrap();
    for enabled in [true, false] {
        fire(&mut h, ActionsPanelCommands::ToolRecording);
        assert_eq!(h.state().session.recording.record_tools, enabled);
        h.state_mut().run("paint.stroke", json!({"points":[[2,3],[10,3]],"size":2,"color":if enabled {"#ff0000"} else {"#00ff00"}})).unwrap();
    }
    h.state_mut().run("actions.stop", json!({})).unwrap();
    assert_eq!(steps(h.state()).len(), 1);
    assert_eq!(steps(h.state())[0].params["color"], "#ff0000");
    assert_eq!(h.state().session.journal.iter().filter(|s| s.0 == "paint.stroke").count(), 2);
}

/// Folder selection dispatches set duplication and keeps the new folder selected, empty ones included.
#[test]
fn duplicate_selected_set_keeps_folder_and_disclosure_ownership() {
    let mut h = harness();
    h.state_mut().ui.actions.panel.selected = Some(ActionSelection::Set("S".into()));
    fire(&mut h, ActionsPanelCommands::Duplicate);
    assert_eq!(h.state().ui.actions.panel.selected, Some(ActionSelection::Set("S copy".into())));
    assert_eq!(actions_edit_cmds::set_names(&h.state().session), ["S", "S copy"]);
    assert_eq!(h.state().ui.actions.panel.expanded.iter().collect::<Vec<_>>(), ["A"], "the copied action starts closed");
    h.state_mut().run("actions.newSet", json!({"name":"Empty"})).unwrap();
    h.state_mut().ui.actions.panel.selected = Some(ActionSelection::Set("Empty".into()));
    fire(&mut h, ActionsPanelCommands::Duplicate);
    assert_eq!(h.state().ui.actions.panel.selected, Some(ActionSelection::Set("Empty copy".into())));
}

/// The selected path is snapshotted without applying it to the document.
#[test]
fn insert_path_uses_actual_selected_saved_path() {
    let mut h = harness();
    assert!(h.state_mut().run("actions.insertPath", json!({"action":"A"})).is_err());
    h.state_mut().run("path.set", json!({"name":"Curve","path":{"subpaths":[{"closed":false,"knots":[{"anchor":[1,2],"out":[4,5]},[9,8]]}]}})).unwrap();
    h.state_mut().ui.selected_path = Some("Curve".into());
    h.run_steps(3);
    let revision = h.state().session.active().unwrap().revision;
    fire(&mut h, ActionsPanelCommands::InsertPath);
    assert_eq!(steps(h.state())[0].command, "path.set");
    assert_eq!(steps(h.state())[0].params["path"]["subpaths"][0]["knots"][0]["out"], json!([4.0, 5.0]));
    assert_eq!(h.state().session.active().unwrap().revision, revision);
}
