//! Alt+drag with the Type tool and the Paragraph Text Size dialog it opens (#12044): the dialog is
//! measured (prefilled with the dragged size, decimals kept, OK creates the box at the drag start
//! and adds one History state, Cancel creates nothing and adds none), and the neighbours of that
//! route are checked to keep today's behaviour: a plain drag still makes the box directly, Alt+click
//! is still point text, the vertical Type tool keeps its plain drag, ⌥ is read at the press so it can
//! come up or go down mid-drag, a second drag while the dialog is open is swallowed by that dialog,
//! the layer keeps the tool, the type options and the placeholder preference that were active when
//! the dialog opened, and a name the dialog does not show as a field is refused.
//!
//! The gestures go through the canvas pass (`canvas::testing`), the way the released Type tool does
//! on the hosted framework: a press with ⌥ held, a move, a release. The dialog is driven the way a
//! control answer drives a hosted parameter dialog: fields through `set_json_field`, OK through
//! `dialogs::confirm`, Cancel through the host.

use craft::ui::dialog::DialogId;
use egui::Modifiers;
use photocraft_doc::LayerContent;
use serde_json::{Value, json};

use super::*;
use crate::canvas::testing::{ToolEvent, frame, tool_event};
use crate::state::Tool;
use crate::type_tool::PLACEHOLDER;

/// A 400 × 200 document with the Type tool ready.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Type;
    app
}

/// Press at `from` with `down` held, drag to `to` and release with `up` held, as the canvas pass
/// sees it (the move carries `up`, the state just before the release). ⌥ can change mid-gesture:
/// the Alt+drag rule reads it from the press, and these two modifiers are how a test says so.
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

/// The active document's History entries: the ticket's "adds no history state" is measured here.
fn history_len(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

/// The open hosted dialogs: id, key and fields. Asserts that the Alt+drag opened one dialog and
/// that it is the Paragraph Text Size family's.
fn only_dialog(app: &PhotocraftApp) -> (DialogId, Value) {
    let open = app.framework.dialogs.open_dialogs();
    assert_eq!(open.len(), 1, "the Alt+drag opens one dialog");
    let d = &open[0];
    assert_eq!(d.key.as_deref(), Some(KEY), "it is the Paragraph Text Size dialog");
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

/// The paragraph box of `id` in document pixels: its top-left through the layer transform, then its
/// width and height.
fn box_rect(app: &PhotocraftApp, id: u64) -> [f64; 4] {
    let t = text(app, id);
    let photocraft_doc::text::TextShape::Box { x, y, width, height } = t.shape else { panic!("not paragraph text") };
    [t.transform.m[4] + f64::from(x), t.transform.m[5] + f64::from(y), f64::from(width), f64::from(height)]
}

/// #12044: Alt+drag opens the parameter dialog instead of creating the text, prefilled with the
/// size that was dragged — including its decimals.
#[test]
fn alt_drag_opens_the_dialog_prefilled_with_the_dragged_size() {
    let mut app = app();
    let before = layers(&app);
    drag(&mut app, [10.0, 10.0], [109.5, 60.0], Modifiers::ALT, Modifiers::ALT);
    let (_id, f) = only_dialog(&app);
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(99.5), Some(50.0)), "the dragged size, decimals kept");
    assert_eq!(layers(&app), before, "the drag itself creates nothing");
    assert!(app.ui.text_edit.is_none(), "and starts no edit session");
}

/// OK creates the paragraph layer with the box the dialog holds, top-left at the drag's start, and
/// the placeholder selected — as a plain drag of that box does.
#[test]
fn confirming_creates_the_paragraph_box_at_the_drag_start() {
    let mut app = app();
    drag(&mut app, [10.0, 10.0], [109.5, 60.0], Modifiers::ALT, Modifiers::ALT);
    let (d, _) = only_dialog(&app);
    set(&mut app, d, [("width", json!(250.0)), ("height", json!(120.0))]);
    let steps = history_len(&app);
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(&mut app, &ctx, d).unwrap();
    assert!(app.framework.dialogs.open_dialogs().is_empty(), "OK closes the dialog");
    assert_eq!(history_len(&app), steps + 1, "OK adds one History state, as the drag's create does");
    let ed = app.ui.text_edit.as_ref().expect("OK starts the edit session, as the drag does");
    assert_eq!(box_rect(&app, ed.layer), [10.0, 10.0, 250.0, 120.0], "the dialog's box at the drag's start");
    assert_eq!((ed.anchor, ed.caret), (0, PLACEHOLDER.chars().count()), "placeholder text, all of it selected");
    assert_eq!(text(&app, ed.layer).text, PLACEHOLDER);
}

/// Cancel closes the dialog and creates nothing: no layer, no edit session. Cancel goes through
/// the host, the way `ui.dialog.cancel` and the rendered Cancel button both do.
#[test]
fn cancelling_creates_nothing() {
    let mut app = app();
    drag(&mut app, [10.0, 10.0], [109.5, 60.0], Modifiers::ALT, Modifiers::ALT);
    let (id, _) = only_dialog(&app);
    let before = layers(&app);
    let steps = history_len(&app);
    let ctx = app.ctx().clone();
    app.framework.dialogs.cancel_by_id(&ctx, id);
    frame(&mut app);
    assert!(app.framework.dialogs.open_dialogs().is_empty(), "Cancel closes the dialog");
    assert_eq!(layers(&app), before, "Cancel creates nothing");
    assert_eq!(history_len(&app), steps, "and adds no History state");
    assert!(app.ui.text_edit.is_none());
}

/// The dialog belongs to the document the Alt+drag was made in: when another document is active at
/// OK, nothing is created in it (or anywhere), and the dialog closes with the reason in the status.
#[test]
fn confirming_after_another_document_became_active_creates_nothing() {
    let mut app = app();
    drag(&mut app, [10.0, 10.0], [109.5, 60.0], Modifiers::ALT, Modifiers::ALT);
    let (d, _) = only_dialog(&app);
    let first = app.session.active_index().unwrap();
    let first_layers = layers(&app);
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.sync_views();
    assert_ne!(app.session.active_index(), Some(first), "the second document is active");
    let second_layers = layers(&app);
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(&mut app, &ctx, d).unwrap();
    assert!(app.framework.dialogs.open_dialogs().is_empty(), "the dialog closes");
    assert!(app.ui.status_error, "OK in another document says why: {}", app.ui.status);
    assert_eq!(layers(&app), second_layers, "nothing lands in the active document");
    assert!(app.session.set_active(first));
    assert_eq!(layers(&app), first_layers, "nor in the original one");
    assert!(app.ui.text_edit.is_none());
}

/// Not measured by the ticket, so unchanged: a drag without Alt still commits the box directly.
#[test]
fn a_plain_drag_still_makes_the_box_directly() {
    let mut app = app();
    drag(&mut app, [10.0, 10.0], [110.0, 60.0], Modifiers::NONE, Modifiers::NONE);
    assert!(app.framework.dialogs.open_dialogs().is_empty(), "a plain drag opens no dialog");
    let ed = app.ui.text_edit.as_ref().expect("the drag creates the text at once");
    assert_eq!(box_rect(&app, ed.layer), [10.0, 10.0, 100.0, 50.0]);
}

/// Not measured by the ticket, so unchanged: Alt+click (no drag) stays point text.
#[test]
fn an_alt_click_is_still_point_text() {
    let mut app = app();
    drag(&mut app, [50.0, 100.0], [50.0, 100.0], Modifiers::ALT, Modifiers::ALT);
    assert!(app.framework.dialogs.open_dialogs().is_empty(), "a click opens no dialog");
    let ed = app.ui.text_edit.as_ref().expect("the click creates point text");
    let t = text(&app, ed.layer);
    assert!(matches!(t.shape, photocraft_doc::text::TextShape::Point), "point text");
    assert_eq!([t.transform.m[4], t.transform.m[5]], [50.0, 100.0]);
}

/// Guard (pattern 8, the order of checks): ⌥ is read at the press of the gesture, so letting it up
/// mid-drag does not take the dialog away. Pinning the chosen latch, so it passes with the rule read
/// at the release too only if that rule were in the code: with the release read instead, this drag
/// has no ⌥ left at the release and no dialog opens - seen red.
#[test]
fn alt_released_mid_drag_still_opens_the_dialog() {
    let mut app = app();
    let before = layers(&app);
    drag(&mut app, [10.0, 10.0], [109.5, 60.0], Modifiers::ALT, Modifiers::NONE);
    let (_id, f) = only_dialog(&app);
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(99.5), Some(50.0)), "the dragged size, decimals kept");
    assert_eq!(layers(&app), before, "the drag itself still creates nothing");
    assert!(app.ui.text_edit.is_none(), "and still starts no edit session");
}

/// The same latch the other way, guard as well: ⌥ pressed only as the button comes up opens nothing,
/// the gesture stays today's plain drag (rounded to the pixel, as it has always been).
#[test]
fn alt_pressed_mid_drag_makes_the_plain_box_instead() {
    let mut app = app();
    drag(&mut app, [10.0, 10.0], [109.5, 60.0], Modifiers::NONE, Modifiers::ALT);
    assert!(app.framework.dialogs.open_dialogs().is_empty(), "⌥ pressed mid-drag opens no dialog");
    let ed = app.ui.text_edit.as_ref().expect("the drag creates the text at once");
    assert_eq!(box_rect(&app, ed.layer), [10.0, 10.0, 100.0, 50.0], "the plain drag's rounded box");
}

/// Guard: the dialog the first drag opened stays the only one, and a second canvas gesture while it
/// is open reaches neither the rule nor `type.create`: nothing is created, nothing is edited.
#[test]
fn a_second_drag_while_the_dialog_is_open_creates_nothing() {
    let mut app = app();
    drag(&mut app, [10.0, 10.0], [109.5, 60.0], Modifiers::ALT, Modifiers::ALT);
    let (_id, _) = only_dialog(&app);
    let before = layers(&app);
    drag(&mut app, [200.0, 20.0], [260.0, 70.0], Modifiers::ALT, Modifiers::ALT);
    assert_eq!(app.framework.dialogs.open_dialogs().len(), 1, "the second drag opens no second dialog");
    assert_eq!(layers(&app), before, "and creates nothing");
    assert!(app.ui.text_edit.is_none(), "and starts no edit session");
}

/// Guard (pattern 4, scope): the ticket measured the horizontal Type tool alone, so the vertical one
/// keeps today's gesture - an Alt+drag there still commits the box at once, with no dialog.
#[test]
fn an_alt_drag_with_the_vertical_type_tool_still_makes_the_box_directly() {
    let mut app = app();
    app.ui.tool = Tool::VerticalType;
    drag(&mut app, [10.0, 10.0], [110.0, 60.0], Modifiers::ALT, Modifiers::ALT);
    assert!(app.framework.dialogs.open_dialogs().is_empty(), "the vertical Type tool opens no dialog");
    let ed = app.ui.text_edit.as_ref().expect("the drag creates the text at once");
    assert_eq!(box_rect(&app, ed.layer), [10.0, 10.0, 100.0, 50.0], "the plain drag's box");
}

/// The point size of the new layer's first run, as `type.create` stored it.
fn size_pt(app: &PhotocraftApp, id: u64) -> f32 {
    text(app, id).char_runs().into_iter().next().expect("the new layer has one run").style.size_pt
}

/// #12044 review: the reference dialog is modal, so nothing can change the tool or its options
/// between the Alt+drag and OK. The dialog therefore snapshots the tool, the type options and the
/// placeholder preference when it opens, and OK creates from that snapshot: a control that switches
/// to the vertical Type tool and changes the size while the dialog is open does not leak into the
/// new layer, which stays horizontal at the size the drag had.
#[test]
fn the_layer_keeps_the_tool_and_options_active_at_the_drag() {
    let mut app = app();
    app.ui.tool_options.type_font = "Drag Font".into();
    app.ui.tool_options.type_style = "Regular".into();
    app.ui.tool_options.type_size = 24.0;
    drag(&mut app, [10.0, 10.0], [109.5, 60.0], Modifiers::ALT, Modifiers::ALT);
    let (d, _) = only_dialog(&app);
    // Change the settings the dialog is about to read, while it is open.
    app.ui.tool = Tool::VerticalType;
    app.ui.tool_options.type_font = "Later Font".into();
    app.ui.tool_options.type_size = 72.0;
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(&mut app, &ctx, d).unwrap();
    let ed = app.ui.text_edit.as_ref().expect("OK starts the edit session, as the drag does");
    let t = text(&app, ed.layer);
    assert_eq!(t.orientation, photocraft_doc::text::Orientation::Horizontal, "the tool held at the drag, not the one at OK");
    assert_eq!(size_pt(&app, ed.layer), 24.0, "the size held at the drag, not the one at OK");
    assert_eq!(t.char_runs()[0].style.font_family, "Drag Font", "and the family held at the drag");
    assert_eq!(box_rect(&app, ed.layer), [10.0, 10.0, 99.5, 50.0], "still the box the drag drew");
}

/// The same snapshot covers the placeholder preference: turning "Fill new type layers with
/// placeholder text" off while the dialog is open does not leave the new layer empty.
#[test]
fn the_layer_keeps_the_placeholder_preference_active_at_the_drag() {
    let mut app = app();
    drag(&mut app, [10.0, 10.0], [109.5, 60.0], Modifiers::ALT, Modifiers::ALT);
    let (d, _) = only_dialog(&app);
    app.session.edit_prefs(|p| p.type_.fill_new_type_layers_with_placeholder = false);
    let ctx = app.ctx().clone();
    crate::dialogs::confirm(&mut app, &ctx, d).unwrap();
    let ed = app.ui.text_edit.as_ref().expect("OK starts the edit session, as the drag does");
    assert_eq!(text(&app, ed.layer).text, PLACEHOLDER, "the preference held at the drag, not the one at OK");
    assert_eq!((ed.anchor, ed.caret), (0, PLACEHOLDER.chars().count()), "and its placeholder stays selected");
}

/// Not changed by the snapshot: a plain drag reads the tool and its options at the moment it ends.
/// Pinned so the snapshot stays inside the dialog's route.
#[test]
fn a_plain_drag_reads_the_settings_at_its_own_release() {
    let mut app = app();
    app.ui.tool = Tool::VerticalType;
    app.ui.tool_options.type_font = "Drag Font".into();
    app.ui.tool_options.type_size = 24.0;
    drag(&mut app, [10.0, 10.0], [110.0, 60.0], Modifiers::NONE, Modifiers::NONE);
    let ed = app.ui.text_edit.as_ref().expect("the drag creates the text at once");
    let t = text(&app, ed.layer);
    assert_eq!(t.orientation, photocraft_doc::text::Orientation::Vertical, "the tool in hand at the release");
    assert_eq!(size_pt(&app, ed.layer), 24.0, "and the options in hand at the release");
    assert_eq!(t.char_runs()[0].style.font_family, "Drag Font");
}

/// The other unchanged neighbour: a click reads the live settings too, including the placeholder
/// preference as it is at that moment.
#[test]
fn a_click_reads_the_settings_at_its_own_release() {
    let mut app = app();
    app.session.edit_prefs(|p| p.type_.fill_new_type_layers_with_placeholder = false);
    app.ui.tool_options.type_size = 30.0;
    drag(&mut app, [50.0, 100.0], [50.0, 100.0], Modifiers::NONE, Modifiers::NONE);
    let ed = app.ui.text_edit.as_ref().expect("the click creates point text");
    let t = text(&app, ed.layer);
    assert!(matches!(t.shape, photocraft_doc::text::TextShape::Point), "point text");
    assert_eq!(t.text, "", "the preference as it stands at the click");
    assert_eq!(size_pt(&app, ed.layer), 30.0, "and the options as they stand at the click");
}

/// Guard (pattern 9, no guessing from a name): the drag's top-left, the document it was made in and
/// the settings snapshot are the family's captured value, not fields - a control `set_json_field`
/// for a name the dialog does not show is refused, so neither the geometry nor the tool and its
/// options can be pushed in through the field map.
#[test]
fn a_name_the_dialog_does_not_show_as_a_field_is_refused() {
    let mut app = app();
    drag(&mut app, [10.0, 10.0], [109.5, 60.0], Modifiers::ALT, Modifiers::ALT);
    let (id, _) = only_dialog(&app);
    let ctx = app.ctx().clone();
    for field in ["at", "document", "settings", "box", "text"] {
        assert!(app.framework.dialogs.set_json_field(&ctx, id, field, json!(1.0)).is_err(), "`{field}` is not a field of this dialog");
    }
    assert_eq!(only_dialog(&app).1["width"].as_f64(), Some(99.5), "and the two real fields keep the dragged size");
}
