//! The panel duplicates the selected action/step and keeps its new row selected.
use craft::automation::ui::actions::ActionSelection;
use egui_kittest::Harness;
use photocraft_engine::{
    Session,
    actions_cmds::{Action, ActionStep},
    atn_cmds,
};
use photocraft_ui_egui::{
    PhotocraftApp,
    control::{ControlRequest, Reply, handle},
    dialogs,
};
use serde_json::{Value, json};

/// One control request through the public channel; the reply is the request's answer.
fn control(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (request, _reply) = ControlRequest::new(method, params);
    match handle(app, ctx, &request) {
        Reply::Done(reply) => reply,
        _ => panic!("{method} answers at once"),
    }
}

#[test]
fn duplicate_menu_selects_the_new_action_and_then_the_new_step() {
    let mut app = PhotocraftApp::new(Session::new(), Default::default());
    let mut a = Action::new("Source", vec![ActionStep::new("select.all", json!({}))]);
    a.disabled.insert(0);
    a.dialogs.insert(0);
    app.session.actions.list.push(a).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Action(0));
    app.ui.actions.panel.expanded.insert("Source".into());
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(360.0, 460.0)).build_ui_state(|ui, app| dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    let ctx = h.ctx.clone();
    let fire = |h: &mut Harness<'static, PhotocraftApp>| {
        let reply = control(h.state_mut(), &ctx, "ui.menu.invoke", json!({"id": "actionsPanel.duplicate"}));
        assert_eq!(reply["ok"], true, "{reply}");
        h.run_steps(3);
    };
    fire(&mut h);
    let list = &h.state().session.actions.list;
    assert_eq!(list.actions().len(), 2);
    assert_eq!(list.action(1).unwrap().name, "Source copy");
    assert_eq!(h.state().ui.actions.panel.selected, Some(ActionSelection::Action(1)));
    assert_eq!(h.state().ui.actions.panel.expanded.iter().map(String::as_str).collect::<Vec<_>>(), ["Source", "Source copy"]);
    h.state_mut().ui.actions.panel.selected = Some(ActionSelection::Step(1, 0));
    fire(&mut h);
    assert_eq!(h.state().ui.actions.panel.selected, Some(ActionSelection::Step(1, 1)));
    let copied = h.state().session.actions.list.action(1).unwrap();
    assert_eq!(copied.steps.len(), 2);
    assert_eq!(copied.disabled.iter().copied().collect::<Vec<_>>(), [0, 1]);
    assert_eq!(copied.dialogs.iter().copied().collect::<Vec<_>>(), [0, 1]);
}

#[test]
fn duplicate_cannot_shift_the_owner_of_a_suspended_recording() {
    let mut app = PhotocraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 8, "height": 6})).unwrap();
    app.session.actions.list.push(Action::new("Menu", vec![ActionStep::new(atn_cmds::MENU_ITEM, json!({"id": "image.imageSize"}))])).unwrap();
    app.run("actions.record", json!({"name": "Owner"})).unwrap();
    let result = app.run("actions.play", json!({"action": "Menu"})).unwrap();
    let dialog = result["paused"]["dialog"].clone();
    let list = app.session.actions.list.clone();
    assert!(app.session.actions.recording.is_none());
    let error = app.run("actions.duplicate", json!({"action": "Menu"})).unwrap_err();
    assert!(error.contains("Confirm or cancel"), "{error}");
    assert_eq!(app.session.actions.list, list);
    let app_ctx = app.ctx().clone();
    let cancelled = control(&mut app, &app_ctx, "ui.dialog.cancel", json!({"dialog": dialog}));
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    assert_eq!(app.session.actions.recording.map(|(_, i)| i), Some(1));
    app.run("actions.stop", json!({})).unwrap();
}
