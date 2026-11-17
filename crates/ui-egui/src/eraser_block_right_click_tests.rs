//! Right-click routing on the canvas with the Eraser: the mode menu, the Brush Preset picker or an erase, by the
//! Eraser's mode, Shift and the right-click preference.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::canvas_tool_menu::MenuKind;
use crate::state::Tool;

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Menu,
    Picker,
    Erase,
}

fn app(mode: &str, erase_preference: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    // The erase stroke commits in the click's own pass, so its journal entry is there when the click returns.
    app.background_jobs = false;
    app.run("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    app.ui.tool = Tool::Eraser;
    app.ui.tool_options.eraser_mode = mode.into();
    if erase_preference {
        app.run("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": "erase"})).unwrap();
    }
    app.sync_views();
    app
}

/// A secondary click at the document centre, with Shift held when asked; what the click led to.
fn right_click(mode: &str, shift: bool, erase_preference: bool) -> Outcome {
    let mut app = app(mode, erase_preference);
    let journal = app.session.journal.len();
    let mods = if shift { Modifiers::SHIFT } else { Modifiers::NONE };
    testing::secondary_click(&mut app, [32.0, 32.0], mods);
    if app.ui.canvas_tool_menu.as_ref().is_some_and(|m| matches!(m.kind, MenuKind::BrushMode { .. })) {
        Outcome::Menu
    } else if crate::brush_picker::opened_at(&app).is_some() {
        Outcome::Picker
    } else {
        // An erase is a stroke command whose parameters carry `erase: true`: `paint.stroke` in Brush mode,
        // `paint.pencil` in Pencil and Block mode (`strokes.rs`, as production).
        let erased = app.session.journal[journal..].iter().any(|(id, p)| (id == "paint.stroke" || id == "paint.pencil") && p["erase"] == json!(true));
        assert!(erased, "a right-click that opened nothing erases: {mode}, shift {shift}, erase preference {erase_preference}");
        Outcome::Erase
    }
}

#[test]
fn eraser_right_click_routes_by_mode_shift_and_preference() {
    use Outcome::{Erase, Menu, Picker};
    let table = [
        ("block", false, false, Menu),
        ("brush", false, false, Picker),
        ("pencil", false, false, Picker),
        ("brush", true, false, Menu),
        ("pencil", true, false, Menu),
        ("block", true, false, Menu),
        ("block", false, true, Erase),
    ];
    for (mode, shift, erase_preference, want) in table {
        assert_eq!(right_click(mode, shift, erase_preference), want, "{mode}, shift {shift}, erase preference {erase_preference}");
    }
}
