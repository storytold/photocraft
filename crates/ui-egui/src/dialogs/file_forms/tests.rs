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

/// An app with one saved action, selected in the Actions panel: what Create Droplet asks for.
fn droplet_app() -> Box<PhotocraftApp> {
    let mut app = app();
    app.session.actions.list.push(Action::new("Invert", vec![ActionStep::new("image.adjustments.invert", json!({}))])).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Action(0));
    app
}

/// #13713: the form carries the reference application's Errors dropdown, opening on Stop for Errors
/// and offering only its two entries.
#[test]
fn create_droplet_opens_on_stop_for_errors_offering_the_two_reference_choices() {
    let mut app = droplet_app();
    let ctx = app.ctx().clone();
    let id = open_create_droplet(&mut app).unwrap();
    let hosted = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).unwrap();
    assert_eq!(hosted.fields().text("errors"), Some("stop"), "the dropdown opens on Stop for Errors");
    assert_eq!(hosted.fields().text("errorLog"), Some(""), "no log is chosen at first");
    set(&mut app, &ctx, id, "errors", json!("log"));
    assert!(app.framework.dialogs.set_json_field(&ctx, id, "errors", json!("quiet")).is_err(), "only the two reference choices are offered");
    assert_eq!(app.framework.dialogs.get_by_id::<HostedParamDialog>(id).unwrap().fields().text("errors"), Some("log"), "the choice took");
}

/// #13713: OK stores the choice in the droplet: Stop for Errors as `stopOnError`, and Log Errors to
/// File with the path Save As… chose.
#[test]
fn create_droplet_ok_stores_the_errors_choice_in_the_droplet() {
    let mut app = droplet_app();
    let ctx = app.ctx().clone();
    let dir = tempfile::tempdir().unwrap();
    let saved_at = |app: &mut PhotocraftApp, name: &str, setup: &dyn Fn(&mut PhotocraftApp, &egui::Context, DialogId)| -> Value {
        let path = dir.path().join(name).to_string_lossy().into_owned();
        let id = open_create_droplet(app).unwrap();
        set(app, &ctx, id, "path", json!(path));
        setup(app, &ctx, id);
        crate::dialogs::confirm(app, &ctx, id).unwrap();
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap()
    };
    let v = saved_at(&mut app, "stop.pcdroplet", &|_, _, _| {});
    assert_eq!(v["options"]["stopOnError"], json!(true), "the form's default Stop for Errors is stored");
    assert!(v["options"].get("errorLog").is_none(), "Stop for Errors stores no log path");
    let log = dir.path().join("log").join("droplog.txt").to_string_lossy().into_owned();
    let v = saved_at(&mut app, "log.pcdroplet", &|app, ctx, id| {
        set(app, ctx, id, "errors", json!("log"));
        set(app, ctx, id, "errorLog", json!(log));
    });
    assert_eq!(v["options"]["errorLog"], json!(log), "the chosen log path rides in the droplet");
    assert_eq!(v["options"]["stopOnError"], json!(false), "a logging droplet goes on after a failure");
}

/// #13713: Save As… is greyed while Errors shows Stop for Errors and goes live with Log Errors to
/// File; the Output row's Browse stays live either way.
#[test]
fn create_droplet_greys_save_as_until_log_errors_to_file_is_picked() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    let app = droplet_app();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 800.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    open_create_droplet(h.state_mut()).unwrap();
    h.run_steps(4);
    // The rows in form order: Output's Browse…, then Save As…'s.
    let browses = |h: &Harness<'_, Box<PhotocraftApp>>| h.query_all_by_label("Browse…").map(|node| node.accesskit_node().is_disabled()).collect::<Vec<bool>>();
    assert_eq!(browses(&h), vec![false, true], "Save As… is greyed with Stop for Errors, Output stays live");
    let pick = |h: &mut Harness<'_, Box<PhotocraftApp>>, caption: &str| {
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Errors").click();
        h.run_steps(2);
        h.get_by_label(caption).click();
        h.run_steps(2);
    };
    pick(&mut h, "Log Errors to File");
    assert_eq!(browses(&h), vec![false, false], "Save As… opens the log's save dialog with Log Errors to File");
    pick(&mut h, "Stop for Errors");
    assert_eq!(browses(&h), vec![false, true], "and is greyed again back on Stop for Errors");
}
