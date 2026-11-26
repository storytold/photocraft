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
    // The Destination dropdown opens on None (#13711); this droplet is saved for a Folder run.
    set(&mut app, &ctx, id, "destination", json!("folder"));
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
    // #13711: the form opens on Destination None; this flow runs a droplet, so it picks Folder.
    set(&mut app, &ctx, id, "destination", json!("folder"));
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
    app.session.actions_mut().list.push(Action::new("Invert", vec![ActionStep::new("image.adjustments.invert", json!({}))])).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Action(0));
    app
}

/// #13711: Create Droplet carries the reference application's Destination dropdown: the values
/// `none`, `saveAndClose` and `folder` are offered (and nothing else), the form opens on None, and
/// the Override Action "Save As" Commands box starts off.
#[test]
fn create_droplet_opens_on_destination_none_offering_the_three_reference_values() {
    let mut app = droplet_app();
    let ctx = app.ctx().clone();
    let id = open_create_droplet(&mut app).unwrap();
    let hosted = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).unwrap();
    assert_eq!(hosted.fields().text("destination"), Some("none"), "the dropdown opens on None");
    assert_eq!(hosted.fields().bool("overrideSaveAs"), Some(false), "the Override box starts off");
    set(&mut app, &ctx, id, "destination", json!("folder"));
    set(&mut app, &ctx, id, "destination", json!("saveAndClose"));
    assert!(app.framework.dialogs.set_json_field(&ctx, id, "destination", json!("banana")).is_err(), "only the three reference values are offered");
    assert_eq!(app.framework.dialogs.get_by_id::<HostedParamDialog>(id).unwrap().fields().text("destination"), Some("saveAndClose"), "the choice took");
}

/// #13711: OK stores the Destination choice in the droplet (all three), and a ticked Override
/// Action "Save As" Commands rides with it; a call that leaves the box off stores no tick.
#[test]
fn create_droplet_ok_stores_the_destination_in_the_droplet() {
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
    let v = saved_at(&mut app, "default.pcdroplet", &|_, _, _| {});
    assert_eq!(v["options"]["destination"], json!("none"), "the form's default None is stored");
    assert!(v["options"].get("overrideSaveAs").is_none(), "an untouched box stores no tick");
    assert!(v["options"].get("output").is_none(), "a none droplet stores no output folder (#13711 review)");
    let v = saved_at(&mut app, "close.pcdroplet", &|app, ctx, id| set(app, ctx, id, "destination", json!("saveAndClose")));
    assert_eq!(v["options"]["destination"], json!("saveAndClose"));
    assert!(v["options"].get("output").is_none(), "a saveAndClose droplet stores no output folder either");
    let v = saved_at(&mut app, "folder.pcdroplet", &|app, ctx, id| {
        set(app, ctx, id, "destination", json!("folder"));
        set(app, ctx, id, "overrideSaveAs", json!(true));
    });
    assert_eq!(v["options"]["destination"], json!("folder"));
    assert!(v["options"]["output"].is_string(), "the folder run keeps its output path in the droplet");
    assert_eq!(v["options"]["overrideSaveAs"], json!(true), "the tick rides in the droplet");
}

/// #13711: with Destination None (and, as the unmeasured middle choice, Save and Close) the Output
/// row's Choose... and the Override Action "Save As" Commands box are greyed; with Folder both go
/// live again.
#[test]
fn create_droplet_greys_choose_and_override_until_folder_is_picked() {
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
    let greyed =
        |h: &Harness<'_, Box<PhotocraftApp>>, label: &str| h.get_by_role_and_label(egui::accesskit::Role::CheckBox, label).accesskit_node().is_disabled();
    let browses = |h: &Harness<'_, Box<PhotocraftApp>>| h.query_all_by_label("Browse…").map(|node| node.accesskit_node().is_disabled()).collect::<Vec<bool>>();
    assert!(greyed(&h, "Override Action \"Save As\" Commands"), "the box is greyed with Destination None");
    assert_eq!(browses(&h), vec![true], "the Output row's Choose... is greyed with Destination None");
    let pick = |h: &mut Harness<'_, Box<PhotocraftApp>>, caption: &str| {
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Destination").click();
        h.run_steps(2);
        h.get_by_label(caption).click();
        h.run_steps(2);
    };
    pick(&mut h, "Save and Close");
    assert!(greyed(&h, "Override Action \"Save As\" Commands"), "Save and Close keeps both off too (not measured to enable them)");
    assert_eq!(browses(&h), vec![true], "and its Choose... greyed");
    pick(&mut h, "Folder");
    assert!(!greyed(&h, "Override Action \"Save As\" Commands"), "the box is live with Folder");
    assert_eq!(browses(&h), vec![false], "and Choose... opens the folder picker with Folder");
}
