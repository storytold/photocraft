//! #12090: Alt+drag with the Vertical Type Mask tool opens the Paragraph Text Size dialog instead
//! of committing the box. Without a selection the dialog prefills twice the dragged size (the box
//! is drawn from the centre, as with the marquee) and OK centres the box on the drag start; with a
//! selection it prefills the dragged size and OK puts the box's top-left at the drag start; Cancel
//! creates nothing and adds no History state. The neighbours of that route keep today's behaviour,
//! pinned below: a plain drag still makes the mask box at once, Alt+click without a drag is still
//! a point mask, the Horizontal Type Mask tool keeps the plain drag (not measured), a drag up, to
//! the left, or up and to the right of its start keeps today's box (not measured), ⌥ is read at the
//! press, and on the centred route fields edited in the dialog keep the box centred on the drag
//! start.
//!
//! The gestures go through the canvas pass (`canvas::testing`) the way the released Type tool does
//! on the hosted framework, and the dialog is driven the way a control answer drives a hosted
//! parameter dialog (as in the Paragraph Text Size tests of #12044, the dialog this reuses).

use craft::ui::dialog::DialogId;
use egui::Modifiers;
use photocraft_doc::LayerContent;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::testing::{ToolEvent, frame, tool_event};
use crate::state::Tool;

/// A 400 × 200 document with the Vertical Type Mask tool ready.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::VerticalTypeMask;
    app
}

/// The measured "with a selection" state: a rectangle away from both drags below.
fn with_selection(app: &mut PhotocraftApp) {
    app.run("select.rect", json!({"x": 10.0, "y": 150.0, "width": 40.0, "height": 20.0})).unwrap();
    app.sync_views();
}

/// Press at `from` with `down` held, drag to `to` and release with `up` held, as the canvas pass
/// sees it. ⌥ can change mid-gesture: the Alt+drag rule reads it from the press.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], down: Modifiers, up: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, down);
    if from != to {
        tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, up);
    }
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, up);
}

fn layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

/// The active document's History entries: "Cancel adds no history state" is measured here.
fn history_len(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

/// Asserts that the gesture opened exactly one dialog and that it is the Paragraph Text Size
/// family's (#12044's dialog, reused here); returns its id and fields.
fn only_dialog(app: &PhotocraftApp) -> (DialogId, Value) {
    let open = app.framework.dialogs.open_dialogs();
    assert_eq!(open.len(), 1, "the Alt+drag opens one dialog");
    let d = &open[0];
    assert!(d.key.as_deref().is_some_and(crate::paragraph_text_size::owns), "it is the Paragraph Text Size dialog, got key {:?}", d.key);
    (d.id, d.fields.clone().unwrap_or_default())
}

/// Set dialog fields without confirming them, the way a control `ui.dialog.set` does.
fn set(app: &mut PhotocraftApp, id: DialogId, fields: [(&str, Value); 2]) {
    let ctx = app.ctx().clone();
    for (field, value) in fields {
        for (field, value) in crate::control::dialogs::map_request(app, id, field, value) {
            app.framework.dialogs.set_json_field(&ctx, id, &field, value).unwrap();
        }
    }
}

fn text(app: &PhotocraftApp, id: u64) -> photocraft_doc::TextLayer {
    let st = app.session.active().unwrap();
    let layer = st.doc.layer(photocraft_doc::LayerId(id)).unwrap();
    let LayerContent::Text(t) = &layer.content else { panic!("not a type layer") };
    t.clone()
}

/// The paragraph box of `id` in document pixels: its top-left through the layer transform, then
/// its width and height.
fn box_rect(app: &PhotocraftApp, id: u64) -> [f64; 4] {
    let t = text(app, id);
    let photocraft_doc::text::TextShape::Box { x, y, width, height } = t.shape else { panic!("not paragraph text") };
    [t.transform.m[4] + f64::from(x), t.transform.m[5] + f64::from(y), f64::from(width), f64::from(height)]
}

/// #12090 measured, no selection: Alt+drag from (200, 100) to (250, 125) opens the Paragraph Text
/// Size dialog prefilled twice the dragged size; the drag itself creates nothing.
#[test]
fn alt_drag_without_a_selection_opens_the_dialog_at_twice_the_dragged_size() {
    let mut app = app();
    let before = layers(&app);
    drag(&mut app, [200.0, 100.0], [250.0, 125.0], Modifiers::ALT, Modifiers::ALT);
    let (_id, f) = only_dialog(&app);
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(100.0), Some(50.0)), "twice the 50 × 25 drag");
    assert_eq!(layers(&app), before, "the drag itself creates nothing");
    assert!(app.ui.text_edit.is_none(), "and starts no edit session");
}

/// #12090 measured, no selection: OK makes a vertical paragraph mask whose box is centred on the
/// drag start, in one History state, with the working layer hidden as a plain mask drag leaves it.
#[test]
fn confirming_centres_the_box_on_the_drag_start() {
    let mut app = app();
    drag(&mut app, [200.0, 100.0], [250.0, 125.0], Modifiers::ALT, Modifiers::ALT);
    let (d, _) = only_dialog(&app);
    let steps = history_len(&app);
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(&mut app, &ctx, d).unwrap();
    assert!(app.framework.dialogs.open_dialogs().is_empty(), "OK closes the dialog");
    assert_eq!(history_len(&app), steps + 1, "OK adds one History state, as the drag's create does");
    let ed = app.ui.text_edit.as_ref().expect("OK starts the mask session, as the drag does");
    assert!(ed.mask, "the session is a Type Mask");
    assert_eq!(box_rect(&app, ed.layer), [150.0, 75.0, 100.0, 50.0], "centred on the drag start (200, 100)");
    let t = text(&app, ed.layer);
    assert_eq!(t.orientation, photocraft_doc::text::Orientation::Vertical, "vertical type");
    assert!(
        !app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(ed.layer)).unwrap().visible,
        "the mask's working layer is hidden, as a plain mask drag leaves it"
    );
}

/// #12090 centred route, field EDIT (the module's claim, `paragraph_text_size.rs`: "fields edited
/// in the dialog keep the box centred on the drag start"): entering 80 × 40 in the prefilled dialog
/// moves the top-left so the box stays centred on the drag start.
#[test]
fn editing_the_fields_keeps_the_box_centred_on_the_drag_start() {
    let mut app = app();
    drag(&mut app, [200.0, 100.0], [250.0, 125.0], Modifiers::ALT, Modifiers::ALT);
    let (d, f) = only_dialog(&app);
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(100.0), Some(50.0)), "the prefilled size, twice the drag");
    set(&mut app, d, [("width", json!(80.0)), ("height", json!(40.0))]);
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(&mut app, &ctx, d).unwrap();
    let ed = app.ui.text_edit.as_ref().expect("OK starts the mask session");
    assert!(ed.mask, "the session is a Type Mask");
    assert_eq!(box_rect(&app, ed.layer), [160.0, 80.0, 80.0, 40.0], "the typed 80 × 40, still centred on the drag start (200, 100)");
}

/// #12090 measured, with a selection: the dialog opens prefilled with the plain drag size.
#[test]
fn alt_drag_with_a_selection_opens_the_dialog_at_the_dragged_size() {
    let mut app = app();
    with_selection(&mut app);
    let before = layers(&app);
    drag(&mut app, [100.0, 50.0], [200.0, 150.0], Modifiers::ALT, Modifiers::ALT);
    let (_id, f) = only_dialog(&app);
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(100.0), Some(100.0)), "the plain drag size, not twice it");
    assert_eq!(layers(&app), before, "the drag itself creates nothing");
}

/// #12090 measured, with a selection: entering 120 / 80 and OK gives a 120 × 80 box whose
/// top-left is the drag start.
#[test]
fn confirming_with_a_selection_puts_the_top_left_at_the_drag_start() {
    let mut app = app();
    with_selection(&mut app);
    drag(&mut app, [100.0, 50.0], [200.0, 150.0], Modifiers::ALT, Modifiers::ALT);
    let (d, _) = only_dialog(&app);
    set(&mut app, d, [("width", json!(120.0)), ("height", json!(80.0))]);
    let steps = history_len(&app);
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(&mut app, &ctx, d).unwrap();
    assert_eq!(history_len(&app), steps + 1, "OK adds one History state");
    let ed = app.ui.text_edit.as_ref().expect("OK starts the mask session");
    assert!(ed.mask, "the session is a Type Mask");
    assert_eq!(box_rect(&app, ed.layer), [100.0, 50.0, 120.0, 80.0], "top-left at the drag start, the typed size");
}

/// #12090 Done when: Cancel makes nothing and adds no History state.
#[test]
fn cancelling_creates_nothing_and_adds_no_history_state() {
    let mut app = app();
    drag(&mut app, [200.0, 100.0], [250.0, 125.0], Modifiers::ALT, Modifiers::ALT);
    let (id, _) = only_dialog(&app);
    let before = layers(&app);
    let steps = history_len(&app);
    let ctx = app.ctx().clone();
    app.framework.dialogs.cancel_by_id(&ctx, id);
    frame(&mut app);
    assert!(app.framework.dialogs.open_dialogs().is_empty(), "Cancel closes the dialog");
    assert_eq!(layers(&app), before, "Cancel creates nothing");
    assert_eq!(history_len(&app), steps, "and adds no History state");
    assert!(app.ui.text_edit.is_none(), "and starts no edit session");
}

/// Unchanged (guard test, passes before and after): a plain drag with the Vertical Type Mask tool
/// still makes the box at once, without a dialog.
#[test]
fn a_plain_drag_still_makes_the_mask_box_at_once() {
    let mut app = app();
    drag(&mut app, [100.0, 50.0], [200.0, 150.0], Modifiers::NONE, Modifiers::NONE);
    assert!(app.framework.dialogs.open_dialogs().is_empty(), "a plain drag opens no dialog");
    let ed = app.ui.text_edit.as_ref().expect("the drag starts the mask at once");
    assert!(ed.mask, "the session is a Type Mask");
    assert_eq!(box_rect(&app, ed.layer), [100.0, 50.0, 100.0, 100.0]);
}

/// Not measured (Alt+click without a drag), so today's behaviour, pinned: a point mask.
#[test]
fn an_alt_click_without_a_drag_still_starts_a_point_mask() {
    let mut app = app();
    drag(&mut app, [50.0, 100.0], [50.0, 100.0], Modifiers::ALT, Modifiers::ALT);
    assert!(app.framework.dialogs.open_dialogs().is_empty(), "a click opens no dialog");
    let ed = app.ui.text_edit.as_ref().expect("the click starts a point mask");
    assert!(ed.mask, "the session is a Type Mask");
    let t = text(&app, ed.layer);
    assert!(matches!(t.shape, photocraft_doc::text::TextShape::Point), "point text");
    assert_eq!([t.transform.m[4], t.transform.m[5]], [50.0, 100.0]);
}

/// Guard: pins that the ticket measured the Vertical Type Mask tool only, so the Horizontal
/// Type Mask tool keeps today's gesture — an Alt+drag there commits the plain box at once.
#[test]
fn an_alt_drag_with_the_horizontal_type_mask_tool_still_makes_the_box_directly() {
    let mut app = app();
    app.ui.tool = Tool::HorizontalTypeMask;
    drag(&mut app, [100.0, 50.0], [200.0, 150.0], Modifiers::ALT, Modifiers::ALT);
    assert!(app.framework.dialogs.open_dialogs().is_empty(), "the Horizontal Type Mask tool opens no dialog");
    let ed = app.ui.text_edit.as_ref().expect("the drag starts the mask at once");
    assert_eq!(text(&app, ed.layer).orientation, photocraft_doc::text::Orientation::Horizontal, "horizontal, as today");
    assert_eq!(box_rect(&app, ed.layer), [100.0, 50.0, 100.0, 100.0], "the plain drag's box");
}

/// Not measured (a drag up or to the left of the start), so today's rule whole: the box is the
/// rectangle between the two points, committed at once, with no dialog. Up and left in two tests,
/// as the ticket names both.
#[test]
fn an_upward_alt_drag_keeps_todays_plain_box() {
    let mut app = app();
    drag(&mut app, [200.0, 100.0], [150.0, 50.0], Modifiers::ALT, Modifiers::ALT);
    assert!(app.framework.dialogs.open_dialogs().is_empty(), "an up-and-left ⌥-drag opens no dialog");
    let ed = app.ui.text_edit.as_ref().expect("the drag starts the mask at once");
    assert_eq!(box_rect(&app, ed.layer), [150.0, 50.0, 50.0, 50.0], "the rectangle between the two points");
}

#[test]
fn a_leftward_alt_drag_keeps_todays_plain_box() {
    let mut app = app();
    drag(&mut app, [200.0, 50.0], [150.0, 100.0], Modifiers::ALT, Modifiers::ALT);
    assert!(app.framework.dialogs.open_dialogs().is_empty(), "a leftward ⌥-drag opens no dialog");
    let ed = app.ui.text_edit.as_ref().expect("the drag starts the mask at once");
    assert_eq!(box_rect(&app, ed.layer), [150.0, 50.0, 50.0, 50.0], "the rectangle between the two points");
}

/// Guard for the SECOND condition of the gate (`pointer.rs`: `end[0] >= start[0] && end[1] >= start[1]`):
/// a drag up and to the right passes the first condition and fails the second, so it is not the
/// measured down-right configuration and keeps today's plain box, with no dialog. Green before and
/// after this PR on purpose: it pins the axis the gate turns on.
#[test]
fn an_up_right_alt_drag_keeps_todays_plain_box() {
    let mut app = app();
    drag(&mut app, [200.0, 100.0], [250.0, 50.0], Modifiers::ALT, Modifiers::ALT);
    assert!(app.framework.dialogs.open_dialogs().is_empty(), "an up-right ⌥-drag opens no dialog");
    let ed = app.ui.text_edit.as_ref().expect("the drag starts the mask at once");
    assert_eq!(box_rect(&app, ed.layer), [200.0, 50.0, 50.0, 50.0], "the rectangle between the two points");
}

/// Guard: the ⌥ latch the dialog route already has (#12044) — ⌥ pressed only as the button comes
/// up opens nothing, the mask drag stays today's plain box.
#[test]
fn alt_pressed_mid_drag_makes_the_plain_mask_box_instead() {
    let mut app = app();
    drag(&mut app, [100.0, 50.0], [200.0, 150.0], Modifiers::NONE, Modifiers::ALT);
    assert!(app.framework.dialogs.open_dialogs().is_empty(), "⌥ pressed mid-drag opens no dialog");
    let ed = app.ui.text_edit.as_ref().expect("the drag starts the mask at once");
    assert_eq!(box_rect(&app, ed.layer), [100.0, 50.0, 100.0, 100.0], "the plain drag's box");
}

/// The latch the other way: ⌥ held at the press and let go mid-drag still opens the dialog (the
/// rule reads the press, as the Type tool's route does).
#[test]
fn alt_released_mid_drag_still_opens_the_dialog() {
    let mut app = app();
    let before = layers(&app);
    drag(&mut app, [200.0, 100.0], [250.0, 125.0], Modifiers::ALT, Modifiers::NONE);
    let (_id, f) = only_dialog(&app);
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(100.0), Some(50.0)), "twice the 50 × 25 drag");
    assert_eq!(layers(&app), before, "the drag itself still creates nothing");
}
