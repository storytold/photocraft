use craft::automation::actions::{Action, ActionStep};
use craft::automation::ui::actions::ActionSelection;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use photocraft_engine::actions_cmds::ActionsExt;
use serde_json::{Value, json};

use super::*;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 120, "height": 80, "name": "web"})).unwrap();
    app
}

fn set(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId, field: &str, value: Value) {
    app.framework.dialogs.set_json_field(ctx, id, field, value).unwrap_or_else(|e| panic!("{field}: {e}"));
}

/// Actions in sets A and B: Create Droplet starts on the selected action's set and saves that action's steps, not
/// the first action's.
#[test]
fn create_droplet_saves_the_selected_actions_set_and_steps() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.pcdroplet").to_string_lossy().into_owned();
    app.run("file.new", json!({"width": 2, "height": 1})).unwrap();
    for (set_name, name, step) in [("A", "Retouch", "image.adjustments.invert"), ("B", "Turn", "image.imageRotation.90cw")] {
        let mut action = Action::new(name, vec![ActionStep::new(step, json!({}))]);
        action.set = Some(set_name.into());
        app.session.actions_mut().list.push(action).unwrap();
    }
    app.ui.actions.panel.selected = Some(ActionSelection::Action(1));
    let id = open_create_droplet(&mut app).unwrap();
    assert!(app.framework.dialogs.set_json_field(&ctx, id, "set", json!("Z")).is_err(), "the set choice offers only the actions' sets");
    set(&mut app, &ctx, id, "set", json!("A"));
    set(&mut app, &ctx, id, "set", json!("B"));
    set(&mut app, &ctx, id, "path", json!(path));
    set(&mut app, &ctx, id, "output", json!(dir.path().join("out").to_string_lossy()));
    let hosted = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).unwrap();
    assert_eq!(hosted.fields().text("set"), Some("B"));
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["action"]["steps"], json!([["image.imageRotation.90cw", {}]]));
}

/// The droplet contains the form's chosen action; cancelling writes nothing.
#[test]
fn create_droplet_uses_the_action_selected_in_the_form() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.png").to_string_lossy().into_owned();
    let output = dir.path().join("out").to_string_lossy().into_owned();
    let path = dir.path().join("chosen.pcdroplet").to_string_lossy().into_owned();
    app.run("file.new", json!({"width": 2, "height": 1})).unwrap();
    app.run("file.saveACopy", json!({"path": input})).unwrap();
    for (name, step) in [("Invert", "image.adjustments.invert"), ("Rotate", "image.imageRotation.90cw")] {
        app.session.actions_mut().list.push(Action::new(name, vec![ActionStep::new(step, json!({}))])).unwrap();
    }
    app.ui.actions.panel.selected = Some(ActionSelection::Action(0));
    let id = open_create_droplet(&mut app).unwrap();
    assert!(app.framework.dialogs.set_json_field(&ctx, id, "action", json!("Missing")).is_err(), "the action choice offers the listed actions");
    set(&mut app, &ctx, id, "action", json!("Rotate"));
    set(&mut app, &ctx, id, "path", json!(path));
    set(&mut app, &ctx, id, "output", json!(output));
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["action"]["steps"], json!([["image.imageRotation.90cw", {}]]));
    let result = photocraft_engine::Session::new().execute("file.automate.runDroplet", json!({"droplet": path, "input": [input]})).unwrap();
    assert_eq!(result["errors"], json!([]));
    let image = photocraft_io::import("source.png", &std::fs::read(result["files"][0].as_str().unwrap()).unwrap()).unwrap().document;
    assert_eq!((image.size.width, image.size.height), (1, 2));

    let cancelled = dir.path().join("cancelled.pcdroplet");
    let id = open_create_droplet(&mut app).unwrap();
    set(&mut app, &ctx, id, "path", json!(cancelled.to_string_lossy()));
    assert!(app.framework.dialogs.cancel_by_id(&ctx, id));
    assert!(!cancelled.exists());
}
