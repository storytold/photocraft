//! #3820: edit custom paper through the real Print dialog, preview it, and print safely to PDF.
//! The dialog host is crate-private, so these are unit tests of the app.
use craft::print::ui::HostedPrint;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue};
use egui::{Event, Key, Modifiers, PointerButton, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_engine::print_cmds::PrintExt;
use serde_json::{Map, Value, json};

use super::open_print;
use crate::PhotocraftApp;

/// The Print dialog's values, to read and write as the user's edits do.
fn print_fields(app: &mut PhotocraftApp, id: DialogId) -> &mut DialogFields {
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
    app_remembering(None)
}

/// The Print dialog opened after a print that kept `paper` as its paper (what `file.print` remembers).
fn app_remembering(paper: Option<Value>) -> (Box<PhotocraftApp>, DialogId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    // Print lists this printer and does not ask the spooler.
    *app.session.printer_source_mut() = Some(std::sync::Arc::new(|| {
        vec![craft::print::Printer { name: "Test native printer".into(), default: true, two_sided: None, description: String::new() }]
    }));
    app.run("file.new", json!({"width": 120, "height": 80, "name": "Custom paper"})).unwrap();
    if let Some(paper) = paper {
        app.session.file_menu.last_print = Some(json!({ "paper": paper }));
    }
    let id = photocraft_engine::print_cmds::with_spooler(|| open_print(&mut app));
    assert_no_system_listing();
    // Every UI path in these fixtures is safe even if an unexpected Enter confirms the dialog.
    set_fields(&mut app, id, json!({"dryRun": true}));
    (app, id)
}

/// No test in this binary asks the system spooler for its printers.
fn assert_no_system_listing() {
    assert_eq!(photocraft_engine::print_cmds::system_printer_listings(), 0, "a test asked the system spooler for its printers");
}

/// One pass of the dialogs, then the app's result takers, as the app's frame runs them.
fn show(ui: &mut egui::Ui, app: &mut Box<PhotocraftApp>) {
    crate::dialogs::run(app, ui);
    let ctx = ui.ctx().clone();
    super::take(app, &ctx);
}

fn harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    harness_at_scale(app, 1.0)
}

/// Builds the real dialog at an explicit physical-pixels-per-egui-point ratio so pointer
/// events remain in egui points while exercising both standard and Retina display scales.
fn harness_at_scale(app: Box<PhotocraftApp>, pixels_per_point: f32) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder()
        // The paper menu lists the Japanese paper names; no installed face is guaranteed to draw them.
        .allow_missing_glyphs()
        .with_size(vec2(1000.0, 900.0))
        .with_pixels_per_point(pixels_per_point)
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(show, app);
    h.run_steps(4);
    h
}

fn click(h: &mut Harness<'_, Box<PhotocraftApp>>, label: &str) {
    if let Some(node) = h.query_by_label(label) {
        node.click();
    } else {
        h.get_by_value(label).click();
    }
    h.run_steps(3);
}

/// Scrolls the open paper list until the row `label` is shown: the list draws only the rows in view, so a row near
/// the end is not in the access tree until the wheel has brought it into view.
fn reveal(h: &mut Harness<'_, Box<PhotocraftApp>>, label: &str) {
    for _ in 0..40 {
        if h.query_by_label(label).is_some() {
            return;
        }
        let list = h.query_all_by_role(egui::accesskit::Role::ListBox).last().expect("an open list").rect();
        h.hover_at(list.center());
        h.event(Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -3.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(2);
    }
    panic!("the row {label} never came into view");
}

/// Chooses an option from the open paper menu. The Japanese presets made the list longer than
/// the dropdown shows, so it scrolls (and reopens where it was left): bring the row into view
/// before clicking it.
fn choose_paper(h: &mut Harness<'_, Box<PhotocraftApp>>, label: &str) {
    reveal(h, label);
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn shown(h: &Harness<'_, Box<PhotocraftApp>>, text: &str) -> bool {
    // A form caption and its control both expose the same accessible name.
    h.query_all_by_label(text).next().is_some() || h.query_all_by_value(text).next().is_some()
}

fn enter_length(h: &mut Harness<'_, Box<PhotocraftApp>>, label: &str, value: &str) {
    // The caption is a separate Label node with the same text; use the accessible SpinButton.
    let pos = h.get_by_role_and_label(egui::accesskit::Role::SpinButton, label).rect().center();
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(Event::Text(value.into()));
    h.run_steps(1);
    h.key_press(Key::Tab);
    h.run_steps(3);
}

fn fields(h: &Harness<'_, Box<PhotocraftApp>>, id: DialogId) -> Map<String, Value> {
    let open = h.state().framework.dialogs.open_dialogs();
    open.into_iter().find(|d| d.id == id).and_then(|d| d.fields).and_then(|v| v.as_object().cloned()).expect("the Print dialog's values")
}

#[test]
fn custom_paper_ui_edits_reach_preview_and_pdf_and_restore_when_reopened() {
    let (mut app, id) = app();
    let output = std::env::temp_dir().join(format!("pc-custom-paper-{}.pdf", std::process::id()));
    set_fields(&mut app, id, json!({"output": output.to_string_lossy(), "dryRun": true}));
    let mut h = harness(app);
    // The paper control has no label of its own: it shows the chosen preset's display name
    // ("Letter", "A4", …), while the dialog stores the engine key ("letter", "a4", …).
    click(&mut h, "Letter");
    assert!(shown(&h, "A4"), "the real preset menu must be open");
    reveal(&mut h, "Custom");
    choose_paper(&mut h, "Custom");
    assert_eq!(fields(&h, id)["paper"], json!([612.0, 792.0]), "Custom starts at the current preset dimensions");
    click(&mut h, "Centimeters");
    click(&mut h, "Inches");
    enter_length(&mut h, "Paper Width", "7.25");
    enter_length(&mut h, "Paper Height", "9.25");
    let edited = fields(&h, id);
    assert_eq!(edited["paper"], json!([522.0, 666.0]), "the engine takes points, not the displayed units");
    click(&mut h, "Custom");
    choose_paper(&mut h, "A4");
    assert_eq!(fields(&h, id)["paper"], "a4");
    assert!(!shown(&h, "Paper Width"));
    click(&mut h, "A4");
    choose_paper(&mut h, "Custom");
    assert_eq!(fields(&h, id)["paper"], edited["paper"], "returning from a preset restores the custom dimensions");
    click(&mut h, "Inches");
    click(&mut h, "Millimeters");
    assert_eq!(fields(&h, id)["paper"], edited["paper"], "changing units must not change the physical paper");
    click(&mut h, "Millimeters");
    click(&mut h, "Inches");
    assert_eq!(fields(&h, id)["paper"], edited["paper"]);
    let preview = photocraft_engine::print_cmds::layout(&h.state().session.active().unwrap().doc, &Value::Object(edited.clone()), "file.print").unwrap();
    assert_eq!(preview.paper, (522.0, 666.0));
    click(&mut h, "Portrait");
    click(&mut h, "Landscape");
    let edited = fields(&h, id);
    assert_eq!(edited["paper"], json!([522.0, 666.0]), "orientation must not rewrite the unrotated custom dimensions");
    let preview = photocraft_engine::print_cmds::layout(&h.state().session.active().unwrap().doc, &Value::Object(edited), "file.print").unwrap();
    assert_eq!(preview.paper, (666.0, 522.0));
    click(&mut h, "Print");
    let pdf = std::fs::read(&output).unwrap();
    assert!(String::from_utf8_lossy(&pdf).contains("/MediaBox [0 0 666.00 522.00]"));
    assert_eq!(h.state().session.file_menu.last_print.as_ref().unwrap()["paper"], json!([522.0, 666.0]));
    let id = open_print(h.state_mut());
    assert_no_system_listing();
    set_fields(h.state_mut(), id, json!({"dryRun": true}));
    // A new dialog draws its first pass hidden while it sizes itself, as at the harness start.
    h.run_steps(4);
    assert!(shown(&h, "Custom"), "reopening array-valued settings must show Custom");
    assert_eq!(fields(&h, id)["paper"], json!([522.0, 666.0]));
    std::fs::remove_file(output).unwrap();
}

/// #8307: the paper menu lists the Japanese papers by their customary names. Choosing はがき
/// from it stores the preset and lays the page out at its exact metric size.
#[test]
fn japanese_postcard_preset_is_chosen_from_the_paper_menu() {
    let (app, id) = app();
    let mut h = harness(app);
    click(&mut h, "Letter");
    assert!(shown(&h, "はがき (100 × 148 mm)"), "the paper menu must list the Japanese postcard");
    choose_paper(&mut h, "はがき (100 × 148 mm)");
    assert_eq!(fields(&h, id)["paper"], "postcard", "the preset is stored by its engine name");
    assert!(shown(&h, "はがき (100 × 148 mm)"), "the paper control must show the chosen preset");
    assert!(!shown(&h, "Paper Width"), "a preset is not editable custom paper");
    let layout = photocraft_engine::print_cmds::layout(&h.state().session.active().unwrap().doc, &Value::Object(fields(&h, id)), "file.print").unwrap();
    assert_eq!(layout.paper, (283.46, 419.53), "100 × 148 mm in points");
}

#[test]
fn remembered_custom_paper_is_shown_without_becoming_a_preset() {
    let (app, id) = app_remembering(Some(json!([432.0, 576.0])));
    let h = harness(app);
    assert!(shown(&h, "Custom"), "an array paper size must not display as letter");
    assert!(h.query_by_role_and_label(egui::accesskit::Role::SpinButton, "Paper Width").is_some());
    assert!(h.query_by_role_and_label(egui::accesskit::Role::SpinButton, "Paper Height").is_some());
    assert_eq!(fields(&h, id)["paper"], json!([432.0, 576.0]));
}

/// #8307: `file.print` remembers the paper name as typed, aliases included. Reopening the dialog
/// must select the canonical preset an alias names instead of leaving the Paper control unmatched.
#[test]
fn remembered_paper_alias_selects_the_canonical_preset() {
    let (app, id) = app_remembering(Some(json!("はがき")));
    let h = harness(app);
    assert!(shown(&h, "はがき (100 × 148 mm)"), "a remembered alias must show the paper it names");
    let layout = photocraft_engine::print_cmds::layout(&h.state().session.active().unwrap().doc, &Value::Object(fields(&h, id)), "file.print").unwrap();
    assert_eq!(layout.paper, (283.46, 419.53), "the remembered alias must still print as はがき");
}

#[test]
fn invalid_custom_paper_is_rejected_without_printing_or_changing_the_document() {
    let (mut app, id) = app();
    let output = std::env::temp_dir().join(format!("pc-invalid-custom-paper-{}.pdf", std::process::id()));
    set_fields(&mut app, id, json!({"output": output.to_string_lossy()}));
    let before = app.session.active().unwrap().doc.clone();
    let mut h = harness(app);
    click(&mut h, "Letter");
    choose_paper(&mut h, "Custom");
    enter_length(&mut h, "Paper Width", "0");
    let edited = fields(&h, id);
    assert!(photocraft_engine::print_cmds::layout(&h.state().session.active().unwrap().doc, &Value::Object(edited), "file.print").is_err());
    click(&mut h, "Print");
    let error = &h.state().ui.status;
    assert!(error.contains("paper is too small"), "{error}");
    assert_eq!(h.state().session.active().unwrap().doc, before);
    assert!(h.state().session.file_menu.last_print.is_none());
    assert!(!output.exists());
}
