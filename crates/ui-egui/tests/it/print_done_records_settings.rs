//! #11655: while an action records, the Print dialog answers with **Done** beside Print: it keeps
//! the chosen settings as the action's step, closes, and prints nothing. Print One Copy sends them.
//! Driven through the real dialog chrome, as `custom_print_paper.rs` does.
use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_ui_egui::{PhotocraftApp, file_ui::open_print};
use serde_json::{Map, Value, json};

fn app() -> (PhotocraftApp, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), photocraft_ui_egui::Services::default());
    // Print lists this printer and does not ask the spooler; a recorded Done must not ask either.
    app.session.printer_source = Some(std::sync::Arc::new(Vec::new));
    app.run("file.new", json!({"width": 120, "height": 80, "name": "Done recording"})).unwrap();
    let id = photocraft_engine::print_cmds::with_spooler(|| open_print(&mut app));
    (app, id)
}

fn show(ui: &mut egui::Ui, app: &mut PhotocraftApp) {
    if ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
        photocraft_ui_egui::dialogs::show(app, ui.ctx());
    }
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1000.0, 900.0)).with_max_steps(64).build_ui_state(show, app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

fn shown(h: &Harness<'_, PhotocraftApp>, text: &str) -> bool {
    // A form caption and its control both expose the same accessible name.
    h.query_all_by_label(text).next().is_some() || h.query_all_by_value(text).next().is_some()
}

fn click(h: &mut Harness<'_, PhotocraftApp>, label: &str) {
    if let Some(node) = h.query_by_label(label) {
        node.click();
    } else {
        h.get_by_value(label).click();
    }
    h.run_steps(3);
}

fn set_fields(h: &mut Harness<'_, PhotocraftApp>, id: u64, fields: Value) {
    h.state_mut().ui.dialog_mut(id).expect("the dialog is open").fields.extend(fields.as_object().expect("an object").clone());
    h.run_steps(2);
}

fn fields(h: &Harness<'_, PhotocraftApp>, id: u64) -> Map<String, Value> {
    h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone()
}

/// Runs a command on the app's session from the harness (the session needs `&mut`).
fn run(h: &mut Harness<'_, PhotocraftApp>, id: &str, params: Value) -> Value {
    h.state_mut().session.execute(id, params).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Photoshop shows Done only while an action is recording; PhotoCraft follows it, and Print stays
/// the dialog's default answer either way.
#[test]
fn done_shows_while_an_action_records_and_not_otherwise() {
    let (app, id) = app();
    let mut h = harness(app);
    assert!(shown(&h, "Print"), "Print is the dialog's answer");
    assert!(!shown(&h, "Done"), "no Done with nothing recording");
    h.state_mut().session.execute("actions.record", json!({"name": "Print setup"})).expect("record");
    h.run_steps(3);
    assert!(shown(&h, "Done"), "Done appears while an action records");
    assert!(shown(&h, "Print"), "and Print is still there beside it");
    h.state_mut().session.execute("actions.stop", json!({})).expect("stop");
    h.run_steps(3);
    assert!(!shown(&h, "Done"), "and goes when the recording stops");
    assert!(h.state().ui.dialogs.iter().any(|d| d.id == id), "the dialog stays open throughout");
    // A dialog an agent confirms still prints: the Done answer is a marker it may not add.
    assert!(photocraft_ui_egui::dialogs::is_route_marker("__recordPrintSettings"), "Done is a route marker");
}

/// The click the ticket asks for: set the settings, Done. The dialog closes with the settings kept
/// as the action's step; no job is sent, no file written and the document is untouched.
#[test]
fn done_keeps_the_settings_as_a_step_and_prints_nothing() {
    let (mut app, id) = app();
    app.session.execute("actions.record", json!({"name": "Print setup"})).expect("record");
    let mut h = harness(app);
    set_fields(&mut h, id, json!({"paper": "a4", "scale": 150, "copies": 2, "registrationMarks": true, "colorHandling": "noColorManagement"}));
    assert_eq!(fields(&h, id)["paper"], json!("a4"), "the dialog holds the paper the user chose");
    let (revision, steps) = (h.state().session.active().unwrap().revision, h.state().session.active().unwrap().history.past_len());
    click(&mut h, "Done");
    assert!(h.state().ui.dialogs.is_empty(), "Done closes the dialog");
    let status = h.state().ui.status.clone();
    let kept = h.state().session.file_menu.last_print.clone();
    assert!(kept.is_some(), "Done stored the settings (status was {status:?})");
    // The step joins the action when the recording stops, as every recorded command does.
    let stopped = run(&mut h, "actions.stop", json!({}));
    assert_eq!(stopped["steps"], json!(1), "Done recorded exactly one step: {stopped}");
    let action = run(&mut h, "actions.get", json!({"action": "Print setup"}));
    assert_eq!(action["steps"][0][0], json!("file.printSettings"), "{action}");
    assert_eq!(action["steps"][0][1]["paper"], json!("a4"), "the step carries the paper");
    assert_eq!(action["steps"][0][1]["scale"], json!(150.0), "the step carries the scale");
    assert_eq!(action["steps"][0][1]["registrationMarks"], json!(true), "the step carries the marks");
    let kept = kept.expect("Print opens with these settings again");
    assert_eq!((kept["paper"].clone(), kept["scale"].clone(), kept["copies"].clone()), (json!("a4"), json!(150.0), json!(2.0)));
    assert!(kept.get("output").is_none(), "the dialog's private fields are not settings: {kept}");
    assert!(kept.get("__print").is_none(), "the dialog's private fields are not settings: {kept}");
    assert!(h.state().session.jobs().is_empty(), "nothing was spooled");
    assert_eq!(h.state().session.active().unwrap().revision, revision, "the document is unchanged");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "and Done adds no history step");
    // The reporter's workflow: Print One Copy is what sends the recorded settings (#11655). The
    // page it prints is the one the stored settings lay out, whatever paper the driver gave.
    let one = run(&mut h, "file.printOneCopy", json!({"dryRun": true}));
    let doc = h.state().session.active().unwrap().doc.clone();
    let want = photocraft_engine::print_cmds::layout(&doc, &kept, "file.print").expect("the recorded settings lay out a page");
    assert_eq!(
        (one["paper"].clone(), one["scale"].clone(), one["copies"].clone()),
        (json!([want.paper.0, want.paper.1]), json!(150.0), json!(1)),
        "Print One Copy repeats the recorded settings, one copy"
    );
}

/// Cancel records nothing: the settings the user was looking at stay in the dialog, not the action.
#[test]
fn cancel_while_recording_keeps_nothing() {
    let (mut app, id) = app();
    app.session.execute("actions.record", json!({"name": "Print setup"})).expect("record");
    let mut h = harness(app);
    set_fields(&mut h, id, json!({"paper": "a3"}));
    click(&mut h, "Cancel");
    assert!(h.state().ui.dialogs.is_empty(), "Cancel closes the dialog");
    let stopped = run(&mut h, "actions.stop", json!({}));
    assert_eq!(stopped["steps"], json!(0), "Cancel records no step: {stopped}");
    let action = run(&mut h, "actions.get", json!({"action": "Print setup"}));
    assert_eq!(action["steps"], json!([]), "Cancel records no step: {action}");
    assert!(h.state().session.file_menu.last_print.is_none(), "and stores no settings");
}
