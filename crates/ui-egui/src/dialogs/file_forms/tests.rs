use craft::automation::actions::{Action, ActionStep};
use craft::automation::ui::actions::ActionSelection;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog};
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

/// A selected Rotate action, as the Actions panel keeps one.
fn push_rotate(app: &mut PhotocraftApp) {
    app.run("file.new", json!({"width": 2, "height": 1})).unwrap();
    app.session.actions.list.push(Action::new("Rotate", vec![ActionStep::new("image.imageRotation.90cw", json!({}))])).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Action(0));
}

/// The Create Droplet form holds the File Naming rows (defaults Document Name + extension), the
/// Starting Serial # and the Compatibility flags (#13712).
#[test]
fn create_droplet_form_holds_the_naming_rows_serial_and_compatibility() {
    let mut app = app();
    push_rotate(&mut app);
    let id = open_create_droplet(&mut app).unwrap();
    let hosted = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("the form is open");
    let naming = serde_json::to_value(hosted.fields().get("namingComponents").expect("the naming rows")).expect("a field value");
    assert_eq!(naming, json!([{"component": "name"}, {"component": "extension"}]), "defaults: Document Name + extension, four None rows store nothing");
    assert_eq!(hosted.fields().int("startingSerial"), Some(1), "Starting Serial # defaults to 1");
    let compat = serde_json::to_value(hosted.fields().get("compatibility").expect("the compatibility flags")).expect("a field value");
    assert_eq!(compat, json!({"macintosh": false, "windows": cfg!(windows), "unix": false}), "Windows checked and greyed on Windows, the others off");
}

/// OK runs the command with the form's naming rows, serial and flags, and the droplet stores them.
#[test]
fn create_droplet_stores_the_form_naming_rows_in_the_droplet() {
    let mut app = app();
    let ctx = app.ctx().clone();
    push_rotate(&mut app);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("named.pcdroplet").to_string_lossy().into_owned();
    let id = open_create_droplet(&mut app).unwrap();
    set(&mut app, &ctx, id, "path", json!(path));
    set(&mut app, &ctx, id, "output", json!(dir.path().join("out").to_string_lossy()));
    set(&mut app, &ctx, id, "namingComponents", json!([{"component": "serial3"}, {"text": "_x"}, {"component": "extension"}]));
    set(&mut app, &ctx, id, "startingSerial", json!(7));
    set(&mut app, &ctx, id, "compatibility", json!({"macintosh": false, "windows": true, "unix": false}));
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["options"]["namingComponents"], json!([{"component": "serial3"}, {"text": "_x"}, {"component": "extension"}]), "{saved}");
    assert_eq!(saved["options"]["startingSerial"], 7, "{saved}");
    assert_eq!(saved["options"]["compatibility"], json!({"macintosh": false, "windows": true, "unix": false}), "{saved}");
}

/// The Example line previews the sample document through the engine's own batch naming: the
/// default rows give `MyFile.gif`; 3 Digit Serial + text `_x` + EXTENSION with Starting Serial 7
/// give `007_x.GIF`; with no rows stored it shows nothing and the run keeps input names.
#[test]
fn the_example_line_previews_the_sample_document_from_the_current_rows() {
    let mut fields = DialogFields::new();
    fields.set("namingComponents", FieldValue::Json(json!([{"component": "name"}, {"component": "extension"}])));
    assert_eq!(naming_example(&fields), "MyFile.gif", "the default Example line");
    fields.set("namingComponents", FieldValue::Json(json!([{"component": "serial3"}, {"text": "_x"}, {"component": "EXTENSION"}])));
    fields.set("startingSerial", FieldValue::Int(7));
    assert_eq!(naming_example(&fields), "007_x.GIF", "the measured rows and serial");
    fields.remove("namingComponents");
    assert_eq!(naming_example(&fields), "", "no rows, no preview");
}

/// The row model: a component row shows its entry's label in the field, a typed row jumps its
/// dropdown to None, and rows are read back into exactly six.
#[test]
fn naming_rows_round_trip_components_and_typed_text() {
    let mut fields = DialogFields::new();
    fields.set("namingComponents", FieldValue::Json(json!([{"component": "serial3"}, {"text": "_x"}, {"component": "extension"}])));
    let rows = naming_rows(&fields);
    assert_eq!(rows.len(), 6, "six rows are shown");
    assert_eq!((rows[0].rule.as_str(), rows[0].text.as_str()), ("serial3", "3 Digit Serial Number"), "a component row shows the entry's label");
    assert_eq!((rows[1].rule.as_str(), rows[1].text.as_str()), ("none", "_x"), "a text row reads as None plus its text");
    assert_eq!((rows[5].rule.as_str(), rows[5].text.as_str()), ("none", ""), "rows past the stored components are empty");
    let mut rows = rows;
    rows[0].text = "3 Digit Serial NumberZ".into();
    rows[0].rule = "none".into();
    assert_eq!(
        Value::Array(naming_components(&rows).expect("named rows store components")),
        json!([{"text": "3 Digit Serial NumberZ"}, {"text": "_x"}, {"component": "extension"}]),
        "typed text stores as a literal component"
    );
    let empty = vec![NamingRow { rule: "none".into(), text: String::new() }; 6];
    assert_eq!(naming_components(&empty), None, "all-None rows store no naming");
}

/// Independent-review MF2: picking `None` in a rule row clears that row instead of leaving the
/// old rule's LABEL in the field, which would store as literal text: the droplet keeps only the
/// extension row, not a `Document Name` text component.
#[test]
fn picking_none_in_a_rule_row_leaves_no_label_text_behind() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let mut app = app();
    app.session.prefs.edit(|p| p.interface.language = "en".into());
    crate::i18n::sync_context(app.ctx(), "en");
    push_rotate(&mut app);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cleared.pcdroplet").to_string_lossy().into_owned();
    let id = open_create_droplet(&mut app).unwrap();
    let ctx = app.ctx().clone();
    set(&mut app, &ctx, id, "path", json!(path));
    set(&mut app, &ctx, id, "output", json!(dir.path().join("out").to_string_lossy()));
    let mut h = egui_kittest::Harness::builder()
        .with_context(ctx)
        // Tall enough that the rule dropdown's popup shows all 19 entries: `None` sits last and
        // an offscreen entry is not in the accesskit tree at all.
        .with_size(egui::vec2(760.0, 1400.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    h.run_steps(4);
    h.query_all_by_role(Role::ComboBox).find(|node| node.value().as_deref() == Some("Document Name")).expect("row 1's rule dropdown").click();
    h.run_steps(2);
    h.get_by_label("None").click();
    h.run_steps(3);
    let hosted = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("the form is still open");
    let naming = serde_json::to_value(hosted.fields().get("namingComponents").expect("the naming rows")).expect("a field value");
    assert_eq!(naming, json!([{"component": "extension"}]), "the row whose rule was set to None stores nothing, not its label");
}

/// Starting Serial # takes 0..9999; a typed 12345, Tab-ed out, takes the closest value 9999 and
/// the reference alert says so (#13712). The framework words the alert with the dialog's title.
#[test]
fn starting_serial_past_the_range_takes_the_closest_value_and_alerts() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let mut app = app();
    // The alert words its range in English with the host's number grouping; pin "en" and compare
    // the sentence without that separator.
    app.session.prefs.edit(|p| p.interface.language = "en".into());
    crate::i18n::sync_context(app.ctx(), "en");
    push_rotate(&mut app);
    let id = open_create_droplet(&mut app).unwrap();
    let mut h = egui_kittest::Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(egui::vec2(760.0, 700.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    h.run_steps(4);
    h.get_by_role_and_label(Role::SpinButton, "Starting Serial #").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("12345".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    let hosted = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(id).expect("the form is still open");
    assert_eq!(hosted.fields().int("startingSerial"), Some(9999), "the closest value is inserted");
    let shown: Vec<String> =
        h.state().framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_string)).collect();
    // The framework words the alert with the host's number grouping ("9.999" or "9,999"); the
    // reference sentence itself is compared without that separator.
    let flat = |m: &str| m.replace(['.', ','], "");
    let want = flat("An integer between 0 and 9999 is required. Closest value inserted.");
    assert!(shown.iter().any(|m| flat(m) == want), "{shown:?}");
}
