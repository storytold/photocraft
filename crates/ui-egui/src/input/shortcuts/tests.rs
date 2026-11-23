use craft::keymap::CommandKind;
use craft::ui::runtime::commands::AppCatalog;

use super::*;
use crate::canvas::testing;
use crate::shortcut_dispatch::tests::{app_harness, tap};

/// A tool with a shortcut of its own leaves its letter (#4086) but stays in the history: the letter
/// falls back on the last tool it still picks, not on its group's first.
#[test]
fn a_tool_key_passes_over_a_remapped_tool_to_the_last_one_it_picks() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("prefs.set", json!({"path": "shortcuts.tools.select.polygonLasso", "value": "F6"})).unwrap();
    app.ui.remember_group_tool(Tool::MagneticLasso);
    app.ui.remember_group_tool(Tool::PolygonLasso);
    let mut h = app_harness(app);
    tap(&mut h, Key::L, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::MagneticLasso);
}

/// The Pen's unfinished points: the draft in `ui.pen`, as the tool host keeps it.
fn pen_knots(app: &PhotocraftApp) -> usize {
    app.ui.pen.as_ref().map_or(0, |pen| pen.draft.knots.len())
}

/// A document with the Pen tool selected and its points placed by clicks at `points`.
fn pen_app(points: &[[f64; 2]]) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.ui.tool = Tool::Pen;
    for at in points {
        testing::click(&mut app, *at, Modifiers::NONE);
    }
    app
}

#[test]
fn pen_command_z_and_backspace_retract_points_before_document_undo() {
    let mut app = pen_app(&[]);
    app.run("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 40], "fill": "#ff0000"})).unwrap();
    let history = app.session.active().unwrap().history.past_len();
    // Away from the rectangle's anchors, which the Pen would edit.
    for at in [[100.0, 100.0], [150.0, 100.0], [150.0, 150.0]] {
        testing::click(&mut app, at, Modifiers::NONE);
    }
    assert_eq!(pen_knots(&app), 3);

    testing::key(&mut app, Key::Z, Modifiers::COMMAND);
    assert_eq!(pen_knots(&app), 2);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    testing::key(&mut app, Key::Backspace, Modifiers::NONE);
    assert_eq!(pen_knots(&app), 1);
    testing::key(&mut app, Key::Delete, Modifiers::NONE);
    assert_eq!(pen_knots(&app), 0);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    // No unfinished Pen points remain, so regular Undo can now affect the document.
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().history.past_len(), history - 1);
}

#[test]
fn pen_custom_undo_binding_and_menu_retract_pending_points() {
    let mut app = pen_app(&[[10.0, 10.0], [50.0, 10.0]]);
    // A library change older than the Pen points: Undo retracts the points first, then reaches the library.
    app.run("actions.reset", json!({"confirm": true, "mode": "append"})).unwrap();
    app.session.prefs.edit(|p| p.shortcuts.insert("edit.undo".into(), "Cmd+Shift+Y".into()));
    // Once rebound, the default key must no longer intercept pending points.
    testing::key(&mut app, Key::Z, Modifiers::COMMAND);
    assert_eq!(pen_knots(&app), 2);
    testing::key(&mut app, Key::Y, Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(pen_knots(&app), 1);
    assert!(crate::menus::is_enabled(&app, "edit.undo"));
    let undo_label = |app: &PhotocraftApp| crate::menus::menu_rows(app).into_iter().find(|row| row.command.id == "edit.undo").unwrap().label;
    assert_ne!(undo_label(&app), "Undo Reset Actions");
    let ctx = testing::ctx(&app);
    crate::menus::invoke(&mut app, &ctx, "edit.undo", json!({})).unwrap();
    assert_eq!(pen_knots(&app), 0);
    assert_eq!(undo_label(&app), "Undo Reset Actions");
}

/// Every row of the catalog builds into the runtime's catalog (no repeated id, every row in the app scope), every tool is
/// a `Tool` row of its letter's group, and no two rows share a default key unless they are tools of one group, which
/// cycle (T-M12).
#[test]
fn the_catalog_builds() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let rows = crate::shortcut_dispatch::command_rows(&app);
    AppCatalog::new(rows.clone(), 0).unwrap_or_else(|e| panic!("the catalog refuses a row: {e:?}"));
    for t in Tool::ALL {
        let id = crate::tool_shortcuts::id(t);
        let row = rows.iter().find(|r| r.command.id == id).unwrap_or_else(|| panic!("{id} has no row"));
        assert_eq!(row.kind, CommandKind::Tool { group: crate::tool_shortcuts::default(t) }, "{id}");
    }
    let mut owners: std::collections::HashMap<String, (&str, &CommandKind)> = std::collections::HashMap::new();
    for row in &rows {
        for default in &row.defaults {
            if let Some((other, kind)) = owners.insert(default.to_string(), (&row.command.id, &row.kind)) {
                assert!(
                    matches!((kind, &row.kind), (CommandKind::Tool { .. }, CommandKind::Tool { .. })),
                    "{default} is the default of both {other} and {}",
                    row.command.id
                );
            }
        }
    }
}

#[test]
fn clipboard_events_become_pixel_shortcuts_outside_text() {
    let ctx = egui::Context::default();
    let raw = |mut events: Vec<egui::Event>, modifiers| {
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        egui::RawInput { events, ..Default::default() }
    };
    let keys = |r: &egui::RawInput| -> Vec<(Key, bool, bool)> {
        r.events
            .iter()
            .filter_map(|e| if let egui::Event::Key { key, pressed, modifiers, .. } = e { Some((*key, *pressed, modifiers.shift)) } else { None })
            .collect()
    };
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let mut r = raw(vec![egui::Event::Copy, egui::Event::Cut], cmd_shift);
    clipboard_keys(&ctx, false, &mut r);
    assert_eq!(keys(&r), vec![(Key::C, true, true), (Key::X, true, true)]);
    // A text paste becomes ⌘V; its key release then adds nothing.
    let up = egui::Event::Key { key: Key::V, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::COMMAND };
    let mut r = raw(vec![egui::Event::Paste("x".into()), up.clone()], Modifiers::COMMAND);
    clipboard_keys(&ctx, false, &mut r);
    assert_eq!(keys(&r), vec![(Key::V, true, false), (Key::V, false, false)]);
    // An image on the clipboard: egui-winit sends only the release. It still pastes.
    let mut r = raw(vec![up.clone()], Modifiers::COMMAND);
    clipboard_keys(&ctx, false, &mut r);
    assert_eq!(keys(&r), vec![(Key::V, true, false), (Key::V, false, false)]);
    // In a text field the events stay text clipboard events.
    let mut r = raw(vec![egui::Event::Paste("x".into())], Modifiers::COMMAND);
    clipboard_keys(&ctx, true, &mut r);
    assert!(matches!(r.events.as_slice(), [_, egui::Event::Paste(_)]));
}

/// #2638: on the web one ⌘V brings its key press and the browser's paste event; that is one paste.
#[test]
fn web_paste_key_and_paste_event_are_one_press() {
    let ctx = egui::Context::default();
    let key = |key, pressed| egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: Modifiers::COMMAND };
    let mut r = egui::RawInput { events: vec![key(Key::V, true), egui::Event::Paste("x".into()), key(Key::X, true), egui::Event::Cut], ..Default::default() };
    clipboard_keys(&ctx, false, &mut r);
    let presses = |k| r.events.iter().filter(|e| matches!(e, egui::Event::Key { key, pressed: true, .. } if *key == k)).count();
    assert_eq!((presses(Key::V), presses(Key::X)), (1, 1));
    let mut r = egui::RawInput { events: vec![key(Key::V, false)], ..Default::default() };
    clipboard_keys(&ctx, false, &mut r);
    assert!(!r.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })), "its release doesn't paste again");
}
