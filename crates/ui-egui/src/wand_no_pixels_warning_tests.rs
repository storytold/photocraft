//! #11781: a Magic Wand click that leaves nothing selected warns "No pixels are selected.";
//! clicks that select something don't, and `select.magicWand` still just reports the result.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{DialogKind, Tool};

/// Two separate red areas (red 1 at 20–60, red 2 at 120–160) on a white background, with the
/// Magic Wand.
fn app() -> PhotocraftApp {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
    for x in [20, 120] {
        s.execute("select.rect", json!({"x": x, "y": 20, "width": 40, "height": 40})).unwrap();
        s.execute("edit.fill", json!({"color": "#dd2222"})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.ui.tool = Tool::MagicWand;
    app
}

fn click(app: &mut PhotocraftApp, x: f64, mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x, y: 40.0, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x, y: 40.0 }, mods);
}

fn warned(app: &PhotocraftApp) -> bool {
    app.ui.dialogs.iter().any(|d| d.kind == DialogKind::Error && d.fields.get("message").and_then(|m| m.as_str()) == Some(crate::wand_no_pixels::message()))
}

fn selected(app: &PhotocraftApp) -> bool {
    app.session.active().unwrap().doc.selection.is_some()
}

#[test]
fn subtracting_everything_warns_and_leaves_nothing_selected() {
    let mut app = app();
    click(&mut app, 40.0, Modifiers::NONE);
    assert!(selected(&app) && !warned(&app), "red 1 selected, no warning");
    click(&mut app, 40.0, Modifiers::ALT);
    assert!(warned(&app));
    assert!(!selected(&app));
}

#[test]
fn intersecting_with_no_overlap_warns_and_leaves_nothing_selected() {
    let mut app = app();
    click(&mut app, 40.0, Modifiers::NONE);
    click(&mut app, 140.0, Modifiers::SHIFT | Modifiers::ALT);
    assert!(warned(&app));
    assert!(!selected(&app));
}

#[test]
fn clicks_that_select_something_dont_warn() {
    let mut app = app();
    click(&mut app, 40.0, Modifiers::NONE);
    click(&mut app, 140.0, Modifiers::SHIFT);
    assert!(selected(&app) && !warned(&app), "add");
    click(&mut app, 140.0, Modifiers::ALT);
    assert!(selected(&app) && !warned(&app), "subtract part");
}

#[test]
fn the_command_reports_the_empty_result_without_a_dialog() {
    let mut app = app();
    app.run("select.magicWand", json!({"x": 40, "y": 40})).unwrap();
    let r = app.run("select.magicWand", json!({"x": 40, "y": 40, "mode": "subtract"})).unwrap();
    assert_eq!(r["selected"], json!(false));
    assert!(!warned(&app));
}

#[test]
fn options_bar_subtract_or_intersect_with_nothing_selected_selects_without_warning() {
    for bar in [2, 3] {
        let mut app = app();
        app.ui.selection_mode = bar;
        click(&mut app, 40.0, Modifiers::NONE);
        assert!(selected(&app), "bar mode {bar}: a new selection, as in the reference");
        assert!(!warned(&app), "bar mode {bar}");
    }
}

#[test]
fn a_second_empty_result_while_the_warning_is_open_adds_no_second_one() {
    let mut app = app();
    click(&mut app, 40.0, Modifiers::NONE);
    click(&mut app, 40.0, Modifiers::ALT);
    crate::wand_no_pixels::warn(&mut app);
    let n = app.ui.dialogs.iter().filter(|d| d.fields.get("message").and_then(|m| m.as_str()) == Some(crate::wand_no_pixels::message())).count();
    assert_eq!(n, 1);
}
