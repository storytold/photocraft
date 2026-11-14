//! Alt+drag with the Type tool and the Paragraph Text Size dialog it opens (#12044): the dialog is
//! measured (prefilled with the dragged size, decimals kept, OK creates the box at the drag start,
//! Cancel creates nothing), and the two unmeasured neighbours of that route are checked to keep
//! today's behaviour: a plain drag still makes the box directly, and Alt+click is still point text.

use egui::Modifiers;
use photocraft_doc::LayerContent;
use serde_json::{Value, json};

use super::*;
use crate::canvas::{Drag, finish_gesture};
use crate::state::Tool;
use crate::type_tool::PLACEHOLDER;

/// A 400 × 200 document with the Type tool ready.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Type;
    app
}

/// Release a Type-tool gesture pressed at `start` and released at `end` with `mods` held, the way
/// the canvas does (`canvas::gesture` hands the gesture's modifiers to the Type tool).
fn drag(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2], mods: Modifiers) {
    let d = Drag::new(Tool::Type, start, vec![[start[0], start[1], 1.0], [end[0], end[1], 1.0]], mods, false);
    finish_gesture(app, d);
}

fn layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

/// The dialog the gesture opened, and only it.
fn only_dialog(app: &PhotocraftApp) -> u64 {
    assert_eq!(app.ui.dialogs.len(), 1, "the Alt+drag opens one dialog");
    let d = &app.ui.dialogs[0];
    assert!(owns(&d.fields), "it is the Paragraph Text Size dialog");
    d.id
}

/// Set dialog fields without confirming them.
fn set(app: &mut PhotocraftApp, id: u64, fields: Value) {
    let d = app.ui.dialog_mut(id).unwrap();
    for (k, v) in fields.as_object().unwrap() {
        d.fields.insert(k.clone(), v.clone());
    }
}

fn text(app: &PhotocraftApp, id: u64) -> photocraft_doc::TextLayer {
    let st = app.session.active().unwrap();
    let layer = st.doc.layer(photocraft_doc::LayerId(id)).unwrap();
    let LayerContent::Text(t) = &layer.content else { panic!("not a type layer") };
    t.clone()
}

/// The paragraph box of `id` in document pixels: its top-left through the layer transform, then its
/// width and height.
fn box_rect(app: &PhotocraftApp, id: u64) -> [f64; 4] {
    let t = text(app, id);
    let photocraft_doc::text::TextShape::Box { x, y, width, height } = t.shape else { panic!("not paragraph text") };
    [t.transform.m[4] + f64::from(x), t.transform.m[5] + f64::from(y), f64::from(width), f64::from(height)]
}

/// #12044: Alt+drag opens the modal dialog instead of creating the text, prefilled with the size
/// that was dragged — including its decimals.
#[test]
fn alt_drag_opens_the_dialog_prefilled_with_the_dragged_size() {
    let mut app = app();
    let before = layers(&app);
    drag(&mut app, [10.0, 10.0], [109.5, 60.0], Modifiers::ALT);
    let id = only_dialog(&app);
    assert_eq!(crate::dialogs::title(&app.ui.dialogs[0]), "Paragraph Text Size");
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(99.5), Some(50.0)), "the dragged size, decimals kept");
    assert_eq!(f["__command"].as_str(), Some("type.create"));
    assert_eq!(layers(&app), before, "the drag itself creates nothing");
    assert!(app.ui.text_edit.is_none(), "and starts no edit session");
}

/// OK creates the paragraph layer with the box the dialog holds, top-left at the drag's start, and
/// the placeholder selected — as a plain drag of that box does.
#[test]
fn confirming_creates_the_paragraph_box_at_the_drag_start() {
    let mut app = app();
    drag(&mut app, [10.0, 10.0], [109.5, 60.0], Modifiers::ALT);
    let d = only_dialog(&app);
    set(&mut app, d, json!({"width": 250.0, "height": 120.0}));
    let r = crate::dialogs::confirm(&mut app, d).unwrap();
    let id = r["layer"].as_u64().unwrap();
    assert!(app.ui.dialogs.is_empty(), "OK closes the dialog");
    assert_eq!(box_rect(&app, id), [10.0, 10.0, 250.0, 120.0], "the dialog's box at the drag's start");
    let ed = app.ui.text_edit.as_ref().expect("OK starts the edit session, as the drag does");
    assert_eq!((ed.layer, ed.anchor, ed.caret), (id, 0, PLACEHOLDER.chars().count()), "placeholder text, all of it selected");
    assert_eq!(text(&app, id).text, PLACEHOLDER);
}

/// Cancel closes the dialog and creates nothing: no layer, no edit session.
#[test]
fn cancelling_creates_nothing() {
    use egui_kittest::{Harness, kittest::Queryable};

    let app = app();
    // The harness first, so its fonts are installed before the dialog is drawn.
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.run_steps(3);
    let before = layers(h.state());
    drag(h.state_mut(), [10.0, 10.0], [109.5, 60.0], Modifiers::ALT);
    h.run_steps(3);
    only_dialog(h.state());
    // The dialog is up with the fields the prefill test pinned; what this test owns is that the
    // rendered Cancel button closes it and creates nothing.
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty(), "Cancel closes the dialog");
    assert_eq!(layers(h.state()), before, "Cancel creates nothing");
    assert!(h.state().ui.text_edit.is_none());
}

/// The dialog belongs to the document the Alt+drag was made in: when another document is active at
/// OK, nothing is created in it (or anywhere), and the dialog closes.
#[test]
fn confirming_after_another_document_became_active_creates_nothing() {
    let mut app = app();
    drag(&mut app, [10.0, 10.0], [109.5, 60.0], Modifiers::ALT);
    let d = only_dialog(&app);
    let first = app.session.active_index().unwrap();
    let first_layers = layers(&app);
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.sync_views();
    assert_ne!(app.session.active_index(), Some(first), "the second document is active");
    let second_layers = layers(&app);
    let r = crate::dialogs::confirm(&mut app, d);
    assert!(r.is_err(), "OK in another document is refused: {r:?}");
    assert!(app.ui.dialogs.is_empty(), "the dialog closes");
    assert_eq!(layers(&app), second_layers, "nothing lands in the active document");
    assert!(app.session.set_active(first));
    assert_eq!(layers(&app), first_layers, "nor in the original one");
    assert!(app.ui.text_edit.is_none());
}
/// Not measured by the ticket, so unchanged: a drag without Alt still commits the box directly.
#[test]
fn a_plain_drag_still_makes_the_box_directly() {
    let mut app = app();
    drag(&mut app, [10.0, 10.0], [110.0, 60.0], Modifiers::NONE);
    assert!(!app.ui.dialogs.iter().any(|d| owns(&d.fields)), "a plain drag opens no dialog");
    let ed = app.ui.text_edit.as_ref().expect("the drag creates the text at once");
    assert_eq!(box_rect(&app, ed.layer), [10.0, 10.0, 100.0, 50.0]);
}

/// Not measured by the ticket, so unchanged: Alt+click (no drag) stays point text.
#[test]
fn an_alt_click_is_still_point_text() {
    let mut app = app();
    drag(&mut app, [50.0, 100.0], [50.0, 100.0], Modifiers::ALT);
    assert!(!app.ui.dialogs.iter().any(|d| owns(&d.fields)), "a click opens no dialog");
    let ed = app.ui.text_edit.as_ref().expect("the click creates point text");
    let t = text(&app, ed.layer);
    assert!(matches!(t.shape, photocraft_doc::text::TextShape::Point), "point text");
    assert_eq!([t.transform.m[4], t.transform.m[5]], [50.0, 100.0]);
}
