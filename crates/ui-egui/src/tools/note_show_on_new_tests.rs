use std::cell::Cell;
use std::rc::Rc;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;
use serde_json::json;

/// Open a document with the Note tool active and no notes yet.
fn note_tool() -> (Box<PhotocraftApp>, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
    app.ui.tool = Tool::Note;
    let ctx = app.ctx().clone();
    (app, ctx)
}

/// Send a click or drag to the canvas in document coordinates.
fn gesture(app: &mut PhotocraftApp, [x, y]: [f64; 2], [end_x, end_y]: [f64; 2]) {
    crate::canvas::tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, egui::Modifiers::NONE);
    if [x, y] != [end_x, end_y] {
        crate::canvas::tool_event(app, ToolEvent::Move { x: end_x, y: end_y, pressure: 1.0 }, egui::Modifiers::NONE);
    }
    crate::canvas::tool_event(app, ToolEvent::Up { x: end_x, y: end_y }, egui::Modifiers::NONE);
}

/// A new note turns Show Notes and the Notes panel on, takes the bar's author and colour and is
/// selected; Extras off stays off; a refused creation changes nothing.
#[test]
fn a_new_note_turns_show_notes_and_the_panel_on() {
    let (mut app, ctx) = note_tool();
    crate::menus::invoke(&mut app, &ctx, "view.show.notes", json!({})).unwrap();
    app.run("notes.add", json!({"x": 20, "y": 20})).unwrap();
    assert!(!app.ui.view.show.notes && !crate::panel_docking::is_shown(&app, "notes"));
    assert!(!app.ui.view.shows(app.ui.view.show.notes));

    app.ui.analysis.note_author = "QA".into();
    app.ui.analysis.note_color = [0.25, 0.5, 1.0];
    gesture(&mut app, [130.0, 70.0], [130.0, 70.0]);

    let notes = &app.session.active().unwrap().doc.notes;
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].position, [20.0, 20.0]);
    assert_eq!(notes[1].position, [130.0, 70.0]);
    assert_eq!(notes[1].author, "QA");
    assert_eq!(notes[1].color.to_rgb(), [0.25, 0.5, 1.0]);
    assert_eq!(app.ui.analysis.note_selected, Some(1));
    assert!(crate::panel_docking::is_shown(&app, "notes") && app.ui.view.shows(app.ui.view.show.notes));

    // The menu can hide them again.
    crate::menus::invoke(&mut app, &ctx, "view.show.notes", json!({})).unwrap();
    assert!(!app.ui.view.shows(app.ui.view.show.notes));

    // With Extras off a new note turns Show Notes on and the overlay stays suppressed.
    crate::menus::invoke(&mut app, &ctx, "view.extras", json!({})).unwrap();
    assert!(!app.ui.view.show.notes && !app.ui.view.extras);
    gesture(&mut app, [30.0, 100.0], [30.0, 100.0]);
    assert_eq!(app.session.active().unwrap().doc.notes.len(), 3);
    assert!(app.ui.view.show.notes);
    assert!(!app.ui.view.extras && !app.ui.view.shows(app.ui.view.show.notes));

    refused_creation_changes_nothing();
}

/// A denied canvas creation preserves the selected note and visibility settings.
fn refused_creation_changes_nothing() {
    let (mut app, _) = note_tool();
    app.run("notes.add", json!({"x": 20, "y": 20})).unwrap();
    app.run("notes.add", json!({"x": 70, "y": 20})).unwrap();
    gesture(&mut app, [72.0, 22.0], [72.0, 22.0]);
    assert_eq!(app.ui.analysis.note_selected, Some(1));
    app.ui.view.show.notes = false;
    app.ui.view.extras = false;
    let panel_before = crate::panel_docking::is_shown(&app, "notes");

    let denials = Rc::new(Cell::new(0));
    let observed_denials = Rc::clone(&denials);
    app.services.automation_command = Some(Box::new(move |id, _| {
        if id == "notes.add" {
            observed_denials.set(observed_denials.get() + 1);
            return Err("note creation denied".into());
        }
        Ok(())
    }));
    app.automation_input = true;
    let answer =
        crate::canvas::testing::control(&mut app, "ui.pointer", json!({"events": [{"kind": "down", "x": 130, "y": 70}, {"kind": "up", "x": 130, "y": 70}]}));

    assert_eq!(answer["ok"], true);
    assert_eq!(denials.get(), 1);
    assert!(!app.ui.view.show.notes && !app.ui.view.extras);
    assert_eq!(crate::panel_docking::is_shown(&app, "notes"), panel_before);
    assert_eq!(app.ui.analysis.note_selected, Some(1));
    assert_eq!(app.session.active().unwrap().doc.notes.len(), 2);
}

/// Selecting or moving a stored note keeps Show Notes off.
#[test]
fn selecting_and_dragging_an_existing_note_preserve_hidden_notes() {
    let (mut app, ctx) = note_tool();
    app.run("notes.add", json!({"x": 30, "y": 40, "author": "QA"})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "view.show.notes", json!({})).unwrap();

    gesture(&mut app, [32.0, 42.0], [32.0, 42.0]);
    assert!(!app.ui.view.show.notes);
    assert_eq!(app.ui.analysis.note_selected, Some(0));
    assert_eq!(app.ui.analysis.note_author, "QA");

    gesture(&mut app, [32.0, 42.0], [62.0, 52.0]);
    assert!(!app.ui.view.show.notes);
    let notes = &app.session.active().unwrap().doc.notes;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].position, [60.0, 50.0]);
}
