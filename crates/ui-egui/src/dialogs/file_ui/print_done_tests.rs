//! #11655: the Print dialog answers Cancel, Done and Print at all times; Done keeps the chosen
//! settings, closes, and prints nothing, and while an action records it is the action's step.
//! Print reopens with them, and they are the settings Print One Copy sends. Driven through the
//! real framework dialog chrome, as `custom_paper_tests` does; the dialog host is crate-private,
//! so these are unit tests of the app.
use craft::print::ui::HostedPrint;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::FieldValue;
use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::{Map, Value, json};

use super::open_print;
use crate::PhotocraftApp;

/// The dialog's values, to read and write as the user's edits do.
fn print_fields(app: &mut PhotocraftApp, id: DialogId) -> &mut craft::ui::patterns::form_dialogs::DialogFields {
    app.framework.dialogs.get_mut_by_id::<HostedPrint>(id).expect("the Print dialog is open").fields_mut()
}

/// Sets each member of the object `values` as the dialog's field of that name.
fn set_fields(app: &mut PhotocraftApp, id: DialogId, values: Value) {
    let fields = print_fields(app, id);
    for (key, value) in values.as_object().expect("an object") {
        fields.set(key.clone(), serde_json::from_value::<FieldValue>(value.clone()).expect("a field value"));
    }
}

fn app() -> (Box<PhotocraftApp>, DialogId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    // Print lists this printer and does not ask the spooler; a recorded Done must not ask either.
    app.session.printer_source =
        Some(std::sync::Arc::new(|| vec![craft::print::Printer { name: "Test printer".into(), default: true, two_sided: None, description: String::new() }]));
    app.run("file.new", json!({"width": 120, "height": 80, "name": "Done recording"})).unwrap();
    let id = photocraft_engine::print_cmds::with_spooler(|| open_print(&mut app));
    // Every UI path in these fixtures is safe even if an unexpected Enter confirms the dialog.
    set_fields(&mut app, id, json!({"dryRun": true}));
    (app, id)
}

/// One pass of the dialogs, then the app's result takers, as the app's frame runs them.
fn show(ui: &mut egui::Ui, app: &mut Box<PhotocraftApp>) {
    crate::dialogs::run(app, ui);
    let ctx = ui.ctx().clone();
    super::take(app, &ctx);
}

fn harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder()
        // The paper menu lists the Japanese paper names; no installed face is guaranteed to draw them.
        .allow_missing_glyphs()
        .with_size(vec2(1000.0, 900.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(show, app);
    h.run_steps(4);
    h
}

fn shown(h: &Harness<'_, Box<PhotocraftApp>>, text: &str) -> bool {
    // A form caption and its control both expose the same accessible name.
    h.query_all_by_label(text).next().is_some() || h.query_all_by_value(text).next().is_some()
}

fn click(h: &mut Harness<'_, Box<PhotocraftApp>>, label: &str) {
    if let Some(node) = h.query_by_label(label) {
        node.click();
    } else {
        h.get_by_value(label).click();
    }
    h.run_steps(3);
}

fn open(h: &Harness<'_, Box<PhotocraftApp>>, id: DialogId) -> bool {
    h.state().framework.dialogs.get_by_id::<HostedPrint>(id).is_some()
}

fn fields(h: &Harness<'_, Box<PhotocraftApp>>, id: DialogId) -> Map<String, Value> {
    let open = h.state().framework.dialogs.open_dialogs();
    open.into_iter().find(|d| d.id == id).and_then(|d| d.fields).and_then(|v| v.as_object().cloned()).expect("the Print dialog's values")
}

/// Runs a command on the app's session from the harness (the session needs `&mut`).
fn run(h: &mut Harness<'_, Box<PhotocraftApp>>, id: &str, params: Value) -> Value {
    h.state_mut().session.execute(id, params).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// The reference application's button row is Cancel, Done and Print at all times, recording or not;
/// Print stays the dialog's default answer.
#[test]
fn done_is_always_beside_print_and_cancel() {
    let (app, id) = app();
    let mut h = harness(app);
    assert!(shown(&h, "Print"), "Print is the dialog's answer");
    assert!(shown(&h, "Cancel"), "and Cancel is there");
    assert!(shown(&h, "Done"), "Done is there with nothing recording");
    run(&mut h, "actions.record", json!({"name": "Print setup"}));
    h.run_steps(3);
    assert!(shown(&h, "Done"), "Done is there while an action records");
    assert!(shown(&h, "Print"), "and Print is still there beside it");
    run(&mut h, "actions.stop", json!({}));
    h.run_steps(3);
    assert!(shown(&h, "Done"), "and stays when the recording stops");
    assert!(open(&h, id), "the dialog stays open throughout");
}

/// The click the ticket asks for: set the settings, Done. The dialog closes with the settings kept
/// as the action's step; no job is sent, no file written and the document is untouched.
#[test]
fn done_keeps_the_settings_as_a_step_and_prints_nothing() {
    let (mut app, id) = app();
    app.session.execute("actions.record", json!({"name": "Print setup"})).expect("record");
    set_fields(&mut app, id, json!({"paper": "a4", "scale": 150, "registrationMarks": true, "colorHandling": "noColorManagement"}));
    // Copies belong to the printer panel's settings, not the sections: set them as its spinner does.
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, "copies", json!(2)).expect("the panel takes copies 2");
    let (revision, steps) = (app.session.active().unwrap().revision, app.session.active().unwrap().history.past_len());
    let mut h = harness(app);
    assert_eq!(fields(&h, id)["paper"], json!("a4"), "the dialog holds the paper the user chose");
    // The hosted panel only names its default printer once the lister's fresh list has arrived,
    // as the file_ui tests step for; Done must carry the choice the user is looking at.
    for _ in 0..10 {
        if fields(&h, id)["printer"] != json!("") {
            break;
        }
        h.run_steps(3);
    }
    click(&mut h, "Done");
    assert!(!open(&h, id), "Done closes the dialog");
    let kept = h.state().session.file_menu.last_print.clone();
    assert!(kept.is_some(), "Done stored the settings");
    // The step joins the action when the recording stops, as every recorded command does.
    let stopped = run(&mut h, "actions.stop", json!({}));
    assert_eq!(stopped["steps"], json!(1), "Done recorded exactly one step: {stopped}");
    let action = run(&mut h, "actions.get", json!({"action": "Print setup"}));
    assert_eq!(action["steps"][0][0], json!("file.printSettings"), "{action}");
    assert_eq!(action["steps"][0][1]["paper"], json!("a4"), "the step carries the paper");
    assert_eq!(action["steps"][0][1]["scale"], json!(150), "the step carries the scale");
    assert_eq!(action["steps"][0][1]["registrationMarks"], json!(true), "the step carries the marks");
    let kept = kept.expect("Print opens with these settings again");
    assert_eq!(
        (kept["paper"].clone(), kept["scale"].clone(), kept["copies"].clone(), kept["printer"].clone()),
        (json!("a4"), json!(150), json!(2), json!("Test printer")),
        "Done kept paper, scale, copies and the printer panel's choice"
    );
    for key in ["output", "dryRun", "send", "printable"] {
        assert!(kept.get(key).is_none(), "how one job went out is not a setting: {kept}");
    }
    assert!(h.state().session.jobs().is_empty(), "nothing was spooled");
    assert_eq!(h.state().session.active().unwrap().revision, revision, "the document is unchanged");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "and Done adds no history step");
    // The reporter's workflow: Print One Copy is what sends the recorded settings (#11655). The
    // page those stored settings lay out is the A4 at the recorded scale, whatever paper the
    // driver gave. (The engine's own tests send `last_print` through `file.printOneCopy`.)
    let doc = h.state().session.active().unwrap().doc.clone();
    let want = photocraft_engine::print_cmds::layout(&doc, &kept, "file.print").expect("the stored settings lay out a page");
    assert_eq!(want.paper, photocraft_engine::print_cmds::paper_size("a4").expect("a4 is a paper"), "the stored paper is the page Print One Copy sends");
    assert_eq!(want.scale, 1.5, "at the recorded scale");
    // Print opens with them again.
    let again = open_print(h.state_mut());
    set_fields(h.state_mut(), again, json!({"dryRun": true}));
    h.run_steps(4);
    assert_eq!(fields(&h, again)["paper"], json!("a4"), "a reopened Print shows the kept paper");
    assert_eq!(fields(&h, again)["scale"], json!(150.0), "and the kept scale, reseeded as the dialog's own number");
}

/// Done with no action recording still closes the dialog and keeps the chosen settings, without
/// printing, recording or touching the document: Print opens with them again and Print One Copy
/// sends them.
#[test]
fn done_without_recording_keeps_the_settings_and_prints_nothing() {
    let (mut app, id) = app();
    assert!(app.session.actions.recording.is_none(), "nothing is recording");
    set_fields(&mut app, id, json!({"paper": "a4", "scale": 150}));
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, "copies", json!(2)).expect("the panel takes copies 2");
    let (revision, steps) = (app.session.active().unwrap().revision, app.session.active().unwrap().history.past_len());
    let mut h = harness(app);
    click(&mut h, "Done");
    assert!(!open(&h, id), "Done closes the dialog");
    let kept = h.state().session.file_menu.last_print.clone().expect("Done kept the settings");
    assert_eq!((kept["paper"].clone(), kept["scale"].clone(), kept["copies"].clone()), (json!("a4"), json!(150), json!(2)));
    assert!(h.state().session.jobs().is_empty(), "nothing was spooled");
    assert_eq!(h.state().session.active().unwrap().revision, revision, "the document is unchanged");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "and Done adds no history step");
    assert!(h.state().session.actions.list.actions().is_empty(), "no action was created");
    let doc = h.state().session.active().unwrap().doc.clone();
    let want = photocraft_engine::print_cmds::layout(&doc, &kept, "file.print").expect("the kept settings lay out a page");
    assert_eq!(want.scale, 1.5, "Print One Copy sends them at the kept scale");
}

/// Cancel records nothing: the settings the user was looking at stay in the dialog, not the action.
#[test]
fn cancel_while_recording_keeps_nothing() {
    let (mut app, id) = app();
    app.session.execute("actions.record", json!({"name": "Print setup"})).expect("record");
    set_fields(&mut app, id, json!({"paper": "a3"}));
    let mut h = harness(app);
    click(&mut h, "Cancel");
    assert!(!open(&h, id), "Cancel closes the dialog");
    let stopped = run(&mut h, "actions.stop", json!({}));
    assert_eq!(stopped["steps"], json!(0), "Cancel records no step: {stopped}");
    let action = run(&mut h, "actions.get", json!({"action": "Print setup"}));
    assert_eq!(action["steps"], json!([]), "Cancel records no step: {action}");
    assert!(h.state().session.file_menu.last_print.is_none(), "and stores no settings");
}

/// A dialog an agent confirms still prints: OK is the Print button's answer, Done is only its
/// button, so automation cannot record settings where a print was judged for.
#[test]
fn automation_confirm_answers_print_never_done() {
    let (mut app, id) = app();
    let dir = tempfile::tempdir().unwrap();
    set_fields(&mut app, id, json!({"output": dir.path().join("preview.pdf").to_string_lossy()}));
    app.session.execute("actions.record", json!({"name": "Print setup"})).expect("record");
    // Let the dialog run its passes until its printer panel settles, so OK can answer. (Under a
    // full parallel run a busy spooler can make the first listing attempts come back empty.)
    let mut h = harness(app);
    let ctx = h.ctx.clone();
    let mut confirm = crate::dialogs::confirm(h.state_mut(), &ctx, id);
    for _ in 0..10 {
        if matches!(&confirm, Err(e) if e.contains("cannot be confirmed")) {
            h.run_steps(3);
            confirm = crate::dialogs::confirm(h.state_mut(), &ctx, id);
        } else {
            break;
        }
    }
    confirm.expect("OK answers as Print once the panel settles");
    super::take(h.state_mut(), &ctx);
    let stopped = run(&mut h, "actions.stop", json!({}));
    assert_eq!(stopped["steps"], json!(1), "one step, the print itself: {stopped}");
    let action = run(&mut h, "actions.get", json!({"action": "Print setup"}));
    assert_eq!(action["steps"][0][0], json!("file.print"), "the answer printed a preview, not a step of settings: {action}");
}
