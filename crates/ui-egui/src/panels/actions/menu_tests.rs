//! The panel menu's rows and states, and the New Set… and Insert Stop… dialogs they open.
use super::*;
use craft::automation::ui::actions::{ActionSelection, ActionsPanelCommands as C};
use craft::ui::dialog::DialogId;
use craft::ui::menu::PanelRow;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use craft::ui::runtime::commands::{CommandRef, CommandSet as _, CommandState};
use photocraft_engine::actions_cmds::ActionStep;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app
}

/// The commands of the menu's rows, `None` for a separator.
fn rows(app: &PhotocraftApp) -> Vec<Option<C>> {
    menu::panel_rows(app)
        .into_iter()
        .map(|row| match row {
            PanelRow::Command(command) => Some(C::from_ref(&command).expect("an Actions row")),
            _ => None,
        })
        .collect()
}

/// The state the menu gives `command`.
fn state(app: &PhotocraftApp, command: C) -> CommandState {
    menu::states(app).into_iter().find(|(row, _)| C::from_ref(row) == Some(command)).map(|(_, state)| state).expect("the command has a row")
}

fn enabled(app: &PhotocraftApp, command: C) -> bool {
    state(app, command) == CommandState::new().enabled(true)
}

/// Fires the row of `command` as the menu does.
fn fire(app: &mut PhotocraftApp, command: C) {
    let app_ctx = app.ctx().clone();
    assert!(menu::run_fired(app, &app_ctx, &CommandRef::from(command)));
}

/// One control request through the app's public channel.
fn control(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    let (request, _reply) = crate::control::ControlRequest::new(method, params);
    let app_ctx = app.ctx().clone();
    match crate::control::handle(app, &app_ctx, &request) {
        crate::control::Reply::Done(v) => v,
        _ => panic!("{method} answers at once"),
    }
}

/// The dialog the last fired row opened.
fn top_dialog(app: &PhotocraftApp) -> DialogId {
    app.framework.dialogs.stack().top().expect("a dialog is open")
}

/// Sets one field of the hosted dialog `id`.
fn set(app: &mut PhotocraftApp, id: DialogId, field: &str, value: Value) {
    let reply = control(app, "ui.dialog.set", json!({"dialog": id, "field": field, "value": value}));
    assert_eq!(reply["ok"], true, "{field}: {reply}");
}

fn ok(app: &mut PhotocraftApp, id: DialogId) {
    let app_ctx = app.ctx().clone();
    crate::dialogs::confirm(app, &app_ctx, id).unwrap();
}

#[test]
fn the_panel_menu_offers_the_hosts_rows_with_the_apps_state() {
    let mut app = app();
    // Rows: no Set Options…, Batch… or Button Mode; no separator at an end or twice in a row.
    let shown = rows(&app);
    for left_out in [C::SetOptions, C::Batch, C::ButtonMode] {
        assert!(!shown.contains(&Some(left_out)), "{left_out:?} is not offered");
    }
    assert!(shown.contains(&Some(C::StartRecording)) && !shown.contains(&Some(C::StopRecording)));
    assert!(shown.first().is_some_and(|row| row.is_some()) && shown.last().is_some_and(|row| row.is_some()));
    assert!(shown.windows(2).all(|pair| pair[0].is_some() || pair[1].is_some()), "no two separators in a row");

    // Save Actions…: off with nothing to save, on for a selected empty set.
    assert!(!enabled(&app, C::Save));
    app.run("actions.newSet", json!({"name": "Empty"})).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Set("Empty".into()));
    assert!(enabled(&app, C::Save));

    // Insert Path: on only when the selected path is a saved path.
    app.session.actions.list.push(Action::new("A", vec![ActionStep::new("select.all", json!({})), ActionStep::new("select.inverse", json!({}))])).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Action(0));
    app.ui.selected_path = Some("Curve".into());
    assert!(!enabled(&app, C::InsertPath), "no path of that name");
    app.run("path.set", json!({"name": "Curve", "path": {"subpaths": [{"closed": false, "knots": [{"anchor": [1, 2], "out": [4, 5]}, [9, 8]]}]}})).unwrap();
    assert!(enabled(&app, C::InsertPath));

    // Allow Tool Recording is checked from the session's setting.
    for on in [true, false] {
        app.run("actions.allowToolRecording", json!({"enabled": on})).unwrap();
        assert_eq!(state(&app, C::ToolRecording), CommandState::new().checked(on));
    }

    // Playback Options… waits while a paced play waits.
    app.run("actions.setPlayback", json!({"performance": "step", "seconds": 1, "singleState": true})).unwrap();
    assert!(enabled(&app, C::PlaybackOptions));
    play(&mut app, &json!({"action": "A"})).unwrap();
    assert!(playback::pending(&app));
    assert!(!enabled(&app, C::PlaybackOptions));
    assert!(playback::stop(&mut app));
    assert!(enabled(&app, C::PlaybackOptions));

    // Start Recording is replaced by Stop Recording while recording.
    app.run("actions.record", json!({"name": "Rec"})).unwrap();
    let shown = rows(&app);
    assert!(shown.contains(&Some(C::StopRecording)) && !shown.contains(&Some(C::StartRecording)));
    assert!(!enabled(&app, C::NewSet));
    app.run("actions.stop", json!({})).unwrap();

    // Play with a selected set plays the set through the one event mapping.
    let mut member = Action::new("Member", vec![ActionStep::new("layer.new.layer", json!({"name": "FromSet"}))]);
    member.set = Some("Group".into());
    app.session.actions.list.push(member).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Set("Group".into()));
    app.run("actions.setPlayback", json!({"performance": "accelerated"})).unwrap();
    fire(&mut app, C::Play);
    assert!(app.session.active().unwrap().doc.walk().iter().any(|(_, _, layer)| layer.name == "FromSet"), "the set's action ran");
}

#[test]
fn new_set_names_the_first_free_set_and_new_action_records_into_it() {
    let mut app = app();
    app.run("actions.newSet", json!({"name": "Set 1"})).unwrap();
    fire(&mut app, C::NewSet);
    let id = top_dialog(&app);
    // The dialog seeds its fields with its first drawn pass.
    crate::smart_filter_dialog::tests::pass(&mut app);
    assert_eq!(app.framework.dialogs.get_by_id::<HostedParamDialog>(id).and_then(|d| d.fields().text("name").map(str::to_owned)).as_deref(), Some("Set 2"));
    set(&mut app, id, "name", json!("Web"));
    ok(&mut app, id);
    assert_eq!(app.ui.actions.panel.selected, Some(ActionSelection::Set("Web".into())));
    assert!(!app.dialog_open());
    fire(&mut app, C::NewAction);
    app.run("actions.stop", json!({})).unwrap();
    let recorded = app.session.actions.list.actions().last().expect("New Action made an action");
    assert_eq!(recorded.set.as_deref(), Some("Web"));
}

#[test]
fn insert_stop_adds_a_stop_after_the_selected_step() {
    let mut app = app();
    app.session.actions.list.push(Action::new("A", vec![ActionStep::new("select.all", json!({})), ActionStep::new("select.inverse", json!({}))])).unwrap();
    fire(&mut app, C::InsertStop);
    assert!(!app.dialog_open(), "Insert Stop… needs a selected action");
    app.ui.actions.panel.selected = Some(ActionSelection::Step(0, 0));
    fire(&mut app, C::InsertStop);
    let id = top_dialog(&app);
    set(&mut app, id, "message", json!("Check the mask"));
    set(&mut app, id, "allowContinue", json!(true));
    ok(&mut app, id);
    let action = app.session.actions.list.action(0).unwrap();
    assert_eq!(action.steps[1], ActionStep::new(photocraft_engine::atn_cmds::STOP_MESSAGE, json!({"message": "Check the mask", "allowContinue": true})));
    assert!(action.dialogs.contains(&1), "an inserted Stop has its dialog toggle on");
    assert_eq!(app.ui.actions.panel.selected, Some(ActionSelection::Step(0, 1)));
    assert!(app.ui.actions.panel.expanded.contains("A"));
    // With the action selected and no step, the stop goes last.
    app.ui.actions.panel.selected = Some(ActionSelection::Action(0));
    fire(&mut app, C::InsertStop);
    let id = top_dialog(&app);
    set(&mut app, id, "message", json!("Last"));
    ok(&mut app, id);
    let steps = &app.session.actions.list.action(0).unwrap().steps;
    assert_eq!(steps.len(), 4);
    assert_eq!(steps[3].params["message"], "Last");
    assert_eq!(app.ui.actions.panel.selected, Some(ActionSelection::Step(0, 3)));
    // Cancel inserts nothing.
    fire(&mut app, C::InsertStop);
    let id = top_dialog(&app);
    let app_ctx = app.ctx().clone();
    crate::dialogs::cancel(&mut app, &app_ctx, id).unwrap();
    assert_eq!(app.session.actions.list.action(0).unwrap().steps.len(), 4);
}
