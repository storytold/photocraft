//! #11789: a right-click with the Note tool, or on a note with the Move tool, opens the notes menu:
//! New Note · Import Notes… ― Open Note · Close Note · Delete Note · Delete All Notes, greyed as
//! measured for off a note, on a note, and on the note open in the Notes panel.

use serde_json::{Value, json};

use super::*;
use crate::control::{ControlRequest, Outcome, handle};

const IMPORT: &str = "file.import.notes";

/// A 200 × 100 document with two notes, at 20,20 and 120,50, and the Note tool.
fn app() -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("notes.add", json!({"x": 20, "y": 20, "text": "one"})).unwrap();
    app.run("notes.add", json!({"x": 120, "y": 50, "text": "two"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Note;
    (app, egui::Context::default())
}

fn right_click(app: &mut PhotocraftApp, ctx: &egui::Context, tool: Tool, at: [f64; 2]) -> CanvasToolMenu {
    app.ui.canvas_tool_menu = None;
    assert!(open_at(app, ctx, tool, [10.0, 10.0], Some(at)), "{tool:?} at {at:?}: a menu opens");
    app.ui.canvas_tool_menu.clone().unwrap()
}

/// The rows' ids and whether each is enabled, separators as "―".
fn state(app: &PhotocraftApp, menu: &CanvasToolMenu) -> Vec<(String, bool)> {
    rows(menu).iter().map(|r| r.map_or(("―".to_string(), false), |(_, id)| (id.to_string(), entry_enabled(app, menu, id)))).collect()
}

fn want(on: [bool; 6]) -> Vec<(String, bool)> {
    let ids = [NOTE_NEW, IMPORT, "―", NOTE_OPEN, NOTE_CLOSE, NOTE_DELETE, NOTE_DELETE_ALL];
    let mut enabled = on.iter().copied();
    ids.iter().map(|id| (id.to_string(), if *id == "―" { false } else { enabled.next().unwrap() })).collect()
}

fn notes(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.notes.len()
}

#[test]
fn the_rows_are_in_the_reference_order() {
    let (mut app, ctx) = app();
    let menu = right_click(&mut app, &ctx, Tool::Note, [80.0, 80.0]);
    let labels: Vec<&str> = rows(&menu).iter().map(|r| r.map_or("―", |(l, _)| l)).collect();
    assert_eq!(labels, ["New Note", "Import Notes…", "―", "Open Note", "Close Note", "Delete Note", "Delete All Notes"]);
}

#[test]
fn note_tool_off_a_note_and_on_the_pasteboard() {
    for at in [[80.0, 80.0], [-30.0, 40.0]] {
        let (mut app, ctx) = app();
        assert!(!pasteboard_at(&app, Tool::Note, at) || at[0] >= 0.0, "the pasteboard gets the notes menu, not its colour menu");
        let menu = right_click(&mut app, &ctx, Tool::Note, at);
        // New · Import ― Open (grey) · Close (grey) · Delete (grey) · Delete All.
        assert_eq!(state(&app, &menu), want([true, true, false, false, false, true]), "at {at:?}");
    }
}

#[test]
fn note_tool_on_a_note() {
    let (mut app, ctx) = app();
    let menu = right_click(&mut app, &ctx, Tool::Note, [24.0, 25.0]);
    assert_eq!(menu.notes.as_ref().unwrap().note, Some(0));
    // New (grey) · Import (grey) ― Open · Close (grey) · Delete · Delete All.
    assert_eq!(state(&app, &menu), want([false, false, true, false, true, true]));
}

#[test]
fn another_tool_on_the_note_open_in_the_panel() {
    let (mut app, ctx) = app();
    crate::analysis_ui::open_note(&mut app, 1);
    let menu = right_click(&mut app, &ctx, Tool::Move, [124.0, 55.0]);
    // The same menu, with Open Note grey and Close Note enabled.
    assert_eq!(state(&app, &menu), want([false, false, false, true, true, true]));
    let steps = app.session.active().unwrap().history.past_len();
    choose(&mut app, &ctx, NOTE_CLOSE);
    assert!(!app.ui.analysis.notes, "Close Note closes the Notes panel");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "and adds no history step");
}

#[test]
fn another_tool_off_a_note_keeps_its_own_menu() {
    let (mut app, ctx) = app();
    app.ui.canvas_tool_menu = None;
    assert!(!open_at(&mut app, &ctx, Tool::Move, [10.0, 10.0], Some([80.0, 80.0])), "the Move tool has no canvas menu off a note");
    let menu = right_click(&mut app, &ctx, Tool::RectMarquee, [80.0, 80.0]);
    assert!(menu.notes.is_none() && rows(&menu) != NOTES_MENU, "the marquee keeps its selection menu");
}

#[test]
fn new_note_lands_on_the_right_clicked_point() {
    let (mut app, ctx) = app();
    right_click(&mut app, &ctx, Tool::Note, [80.0, 70.0]);
    choose(&mut app, &ctx, NOTE_NEW);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.notes.len(), 3);
    assert_eq!(st.doc.notes[2].position, [80.0, 70.0]);
    assert_eq!(st.history.undo_label(), Some("New Note"));
}

#[test]
fn open_and_delete_act_on_the_right_clicked_note() {
    let (mut app, ctx) = app();
    right_click(&mut app, &ctx, Tool::Note, [124.0, 55.0]);
    choose(&mut app, &ctx, NOTE_OPEN);
    assert_eq!((app.ui.analysis.note_selected, app.ui.analysis.notes), (Some(1), true), "Open Note selects it and opens the Notes panel");
    // Delete Note asks first ("Delete this note?", #11788); OK deletes that note.
    right_click(&mut app, &ctx, Tool::Note, [24.0, 25.0]);
    choose(&mut app, &ctx, NOTE_DELETE);
    let id = app.ui.dialogs.iter().find(|d| crate::analysis_ui::owns_delete_note(&d.fields)).map(|d| d.id).expect("the alert");
    assert_eq!(notes(&app), 2, "nothing deleted before the answer");
    crate::dialogs::confirm(&mut app, id).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.notes.len(), 1);
    assert_eq!(st.doc.notes[0].position, [120.0, 50.0], "the right-clicked note went");
}

#[test]
fn delete_all_notes_deletes_every_note() {
    let (mut app, ctx) = app();
    right_click(&mut app, &ctx, Tool::Note, [80.0, 80.0]);
    choose(&mut app, &ctx, NOTE_DELETE_ALL);
    assert_eq!(notes(&app), 0);
    let menu = right_click(&mut app, &ctx, Tool::Note, [80.0, 80.0]);
    assert!(!entry_enabled(&app, &menu, NOTE_DELETE_ALL), "nothing left to delete");
}

fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (req, _rx) = ControlRequest::new(method, params);
    match handle(app, ctx, &req) {
        Outcome::Done(v) => v,
        _ => panic!("{method}: expected an immediate reply"),
    }
}

#[test]
fn over_the_control_channel_the_rows_are_listed_and_chosen() {
    let (mut app, ctx) = app();
    let r =
        call(&mut app, &ctx, "ui.pointer", json!({"button": "secondary", "events": [{"kind": "down", "x": 80, "y": 80}, {"kind": "up", "x": 80, "y": 80}]}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(notes(&app), 2, "the right-click adds no note");
    let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
    let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).cloned().unwrap();
    let listed: Vec<(String, bool)> =
        entries.iter().map(|e| (e["id"].as_str().map_or_else(|| "―".to_string(), str::to_string), e["enabled"].as_bool().unwrap_or(false))).collect();
    assert_eq!(listed, want([true, true, false, false, false, true]));
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id": NOTE_NEW}))["ok"], true);
    assert_eq!(notes(&app), 3);
}

#[test]
fn tools_not_measured_on_a_note_keep_their_own_menus() {
    // Measured on a note: the Note and Move tools. The Pen keeps its path menu, a marquee its
    // selection menu, the Brush and the Type tool their own right-click.
    for tool in [Tool::Pen, Tool::RectMarquee, Tool::Brush, Tool::Type] {
        let (mut app, ctx) = app();
        app.ui.canvas_tool_menu = None;
        open_at(&mut app, &ctx, tool, [10.0, 10.0], Some([24.0, 25.0]));
        assert!(app.ui.canvas_tool_menu.as_ref().is_none_or(|m| m.notes.is_none()), "{tool:?} on a note: not the notes menu");
    }
}

#[test]
fn no_notes_menu_without_a_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.tool = Tool::Note;
    let ctx = egui::Context::default();
    assert!(!open_at(&mut app, &ctx, Tool::Note, [10.0, 10.0], Some([20.0, 20.0])), "no document: no menu, so no Import Notes… or New Note to choose");
    assert!(app.ui.canvas_tool_menu.is_none());
}

#[test]
fn a_menu_whose_note_moved_acts_on_no_other_note() {
    let (mut shifted, ctx) = app();
    let menu = right_click(&mut shifted, &ctx, Tool::Note, [24.0, 25.0]);
    // Note 0 goes while the menu is open (another client, say): note 0 is now "two".
    shifted.run("notes.delete", json!({"index": 0})).unwrap();
    assert!(!entry_enabled(&shifted, &menu, NOTE_OPEN) && !entry_enabled(&shifted, &menu, NOTE_DELETE), "Open and Delete grey: their note is gone");
    choose(&mut shifted, &ctx, NOTE_DELETE);
    assert!(!shifted.ui.dialogs.iter().any(|d| crate::analysis_ui::owns_delete_note(&d.fields)), "no alert to delete another note");
    // Out of range: the menu on note 1, then note 0 goes.
    let (mut app, ctx) = app();
    let menu = right_click(&mut app, &ctx, Tool::Note, [124.0, 55.0]);
    app.run("notes.delete", json!({"index": 0})).unwrap();
    assert!(!entry_enabled(&app, &menu, NOTE_OPEN), "Open Note greys for a note that is gone");
}

#[test]
fn a_rejected_delete_all_keeps_the_selected_note() {
    let (mut app, ctx) = app();
    crate::analysis_ui::open_note(&mut app, 1);
    right_click(&mut app, &ctx, Tool::Note, [80.0, 80.0]);
    app.session.authorize = Some(|id, _| if id == "notes.delete" { Err(photocraft_engine::EngineError::Other("test rejection".into())) } else { Ok(()) });
    choose(&mut app, &ctx, NOTE_DELETE_ALL);
    assert_eq!(notes(&app), 2, "nothing deleted");
    assert_eq!(app.ui.analysis.note_selected, Some(1), "the Notes panel keeps its note");
}

#[test]
fn a_menu_for_another_document_acts_on_none_of_this_ones_notes() {
    let (mut app, ctx) = app();
    let menu = right_click(&mut app, &ctx, Tool::Note, [80.0, 80.0]);
    // Another document comes to the front while the menu is open (over the control channel, say).
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("notes.add", json!({"x": 20, "y": 20, "text": "three"})).unwrap();
    app.sync_views();
    assert_eq!(state(&app, &menu), want([false; 6]), "every row greys");
    choose(&mut app, &ctx, NOTE_DELETE_ALL);
    assert_eq!(notes(&app), 1, "Delete All Notes deletes nothing here");
    app.ui.canvas_tool_menu = Some(menu);
    choose(&mut app, &ctx, NOTE_NEW);
    assert_eq!(notes(&app), 1, "New Note adds nothing here");
}

// The Move tool's right-click goes to layer picking: on a note, a plain right-click opens the
// notes menu before it (#11789). A miss, the pasteboard and ⌘/Ctrl+right-click keep today's
// menus, as do the other tools.

/// A Move right-click at document point `at` over `ui.pointer`, with the flags in `modifiers`.
fn pointer_right_click(app: &mut PhotocraftApp, ctx: &egui::Context, at: [f64; 2], modifiers: Value) {
    app.ui.canvas_tool_menu = None;
    app.ui.layer_menu = None;
    let events = json!([{"kind": "down", "x": at[0], "y": at[1]}, {"kind": "up", "x": at[0], "y": at[1]}]);
    let r = call(app, ctx, "ui.pointer", json!({"button": "secondary", "modifiers": modifiers, "events": events}));
    assert_eq!(r["ok"], true, "{r}");
}

fn notes_menu_on(app: &PhotocraftApp) -> Option<Option<usize>> {
    app.ui.canvas_tool_menu.as_ref().and_then(|m| m.notes.as_ref()).map(|n| n.note)
}

#[test]
fn ui_pointer_move_right_click_on_a_note_opens_the_notes_menu() {
    let (mut app, ctx) = app();
    app.ui.tool = Tool::Move;
    pointer_right_click(&mut app, &ctx, [124.0, 55.0], json!({}));
    assert_eq!(notes_menu_on(&app), Some(Some(1)), "the notes menu, on note 1");
    assert!(app.ui.layer_menu.is_none(), "no layer list over it");
}

#[test]
fn ui_pointer_move_right_click_off_the_notes_lists_the_layers() {
    let (mut app, ctx) = app();
    app.ui.tool = Tool::Move;
    pointer_right_click(&mut app, &ctx, [80.0, 80.0], json!({}));
    assert!(app.ui.layer_menu.is_some(), "layer picking, as before");
    assert_eq!(notes_menu_on(&app), None);
}

#[test]
fn ui_pointer_command_right_click_on_a_note_lists_the_layers() {
    for tool in [Tool::Move, Tool::Note] {
        let (mut app, ctx) = app();
        app.ui.tool = tool;
        pointer_right_click(&mut app, &ctx, [124.0, 55.0], json!({"command": true}));
        assert!(app.ui.layer_menu.is_some(), "{tool:?}: ⌘/Ctrl+right-click on a note keeps layer picking");
        assert_eq!(notes_menu_on(&app), None, "{tool:?}");
    }
}

/// The app drawing its document area, so a right-click goes through the canvas.
fn canvas_harness(tool: Tool) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let (mut a, _) = app();
    a.ui.tool = tool;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        a,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
    h.run_steps(4);
    h
}

/// A right-click on the canvas at document point `at`, moved `nudge` screen pixels into the note icon.
fn canvas_right_click(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, at: [f32; 2], nudge: [f32; 2], modifiers: egui::Modifiers) {
    h.state_mut().ui.canvas_tool_menu = None;
    h.state_mut().ui.layer_menu = None;
    let p = crate::canvas::ViewXform::active(h.state()).unwrap().to_screen(at[0], at[1]) + egui::vec2(nudge[0], nudge[1]);
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Secondary, pressed: true, modifiers });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Secondary, pressed: false, modifiers });
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(2);
}

#[test]
fn canvas_move_right_click_on_a_note_opens_the_notes_menu() {
    let mut h = canvas_harness(Tool::Move);
    canvas_right_click(&mut h, [120.0, 50.0], [4.0, 5.0], egui::Modifiers::NONE);
    assert_eq!(notes_menu_on(h.state()), Some(Some(1)), "the notes menu, on note 1");
    assert!(h.state().ui.layer_menu.is_none(), "no layer list over it");
}

#[test]
fn canvas_move_right_click_off_the_notes_keeps_layer_picking_and_the_pasteboard_menu() {
    let mut h = canvas_harness(Tool::Move);
    canvas_right_click(&mut h, [80.0, 80.0], [0.0, 0.0], egui::Modifiers::NONE);
    assert!(h.state().ui.layer_menu.is_some(), "layer picking, as before");
    assert_eq!(notes_menu_on(h.state()), None);
    canvas_right_click(&mut h, [-20.0, 40.0], [0.0, 0.0], egui::Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.pasteboard), "the pasteboard colour menu, as before");
}

#[test]
fn canvas_command_right_click_on_a_note_keeps_layer_picking() {
    for tool in [Tool::Move, Tool::Note] {
        let mut h = canvas_harness(tool);
        canvas_right_click(&mut h, [120.0, 50.0], [4.0, 5.0], egui::Modifiers::COMMAND);
        assert!(h.state().ui.layer_menu.is_some(), "{tool:?}: ⌘/Ctrl+right-click on a note keeps layer picking");
        assert_eq!(notes_menu_on(h.state()), None, "{tool:?}");
    }
}
