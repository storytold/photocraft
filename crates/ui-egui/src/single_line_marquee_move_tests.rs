//! Single Row / Single Column Marquee inside the selection (#11392): a drag that starts inside
//! moves the selection, as the other marquees do; a drag from outside and a click without moving
//! still select one line where the pointer is, and Shift (Add) still draws.

use egui::Modifiers;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// An 800 × 600 document with two lines selected: rows 150 and 250, or columns 200 and 300.
fn two_lines(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = tool;
    let row = tool == Tool::SingleRowMarquee;
    for (i, at) in if row { [150, 250] } else { [200, 300] }.into_iter().enumerate() {
        let (x, y, w, h) = if row { (0, at, 800, 1) } else { (at, 0, 1, 600) };
        let mode = if i == 0 { "replace" } else { "add" };
        app.run("select.rect", json!({"x": x, "y": y, "width": w, "height": h, "mode": mode, "line": if row { "row" } else { "column" }})).unwrap();
    }
    app
}

fn drag(app: &mut PhotocraftApp, from: (f64, f64), to: (f64, f64), m: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from.0, y: from.1, pressure: 1.0 }, m);
    for t in [0.25, 0.5, 0.75, 1.0] {
        let (x, y) = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
    }
    tool_event(app, ToolEvent::Up { x: to.0, y: to.1 }, m);
}

fn bounds(app: &PhotocraftApp) -> Option<Rect> {
    app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds())
}

/// Rows (or columns) of the selection that hold anything, within the document.
fn lines(app: &PhotocraftApp, row: bool) -> Vec<i32> {
    let sel = app.session.active().unwrap().doc.selection.clone().unwrap();
    (0..if row { 600 } else { 800 }).filter(|&i| if row { sel.pixel(400, i)[0] > 0.0 } else { sel.pixel(i, 300)[0] > 0.0 }).collect()
}

fn steps(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries().to_vec()
}

/// What the Rectangular Marquee's drag inside the selection records and keeps past the canvas
/// edge, for the same move.
fn rect_marquee_move(sel: Rect, by: (f64, f64)) -> (String, Option<Rect>) {
    let mut app = two_lines(Tool::SingleRowMarquee);
    app.run("select.rect", json!({"x": sel.x0, "y": sel.y0, "width": sel.width(), "height": sel.height()})).unwrap();
    app.ui.tool = Tool::RectMarquee;
    let from = (f64::from(sel.x0) + 0.5, f64::from(sel.y0) + 0.5);
    drag(&mut app, from, (from.0 + by.0, from.1 + by.1), Modifiers::NONE);
    (steps(&app).last().cloned().unwrap_or_default(), bounds(&app))
}

#[test]
fn a_drag_from_inside_the_rows_moves_them() {
    let mut app = two_lines(Tool::SingleRowMarquee);
    assert_eq!(lines(&app, true), [150, 250]);
    let before = steps(&app).len();
    drag(&mut app, (300.0, 250.5), (400.0, 320.5), Modifiers::NONE);
    assert_eq!(lines(&app, true), [220, 320], "both rows moved down 70");
    assert_eq!(steps(&app).len(), before + 1, "one step");
    let (step, moved) = rect_marquee_move(Rect::new(0, 150, 800, 251), (100.0, 70.0));
    assert_eq!(steps(&app).last(), Some(&step), "named as the Rectangular Marquee's move");
    let b = bounds(&app).unwrap();
    assert_eq!((b.x0, b.y0, b.y1), (100, 220, 321), "{b:?}");
    assert_eq!(b.x1, moved.unwrap().x1, "past the canvas edge, as the Rectangular Marquee's move keeps it");
}

#[test]
fn a_drag_from_inside_the_columns_moves_them() {
    let mut app = two_lines(Tool::SingleColumnMarquee);
    assert_eq!(lines(&app, false), [200, 300]);
    let before = steps(&app).len();
    drag(&mut app, (300.5, 250.0), (400.5, 320.0), Modifiers::NONE);
    assert_eq!(lines(&app, false), [300, 400], "both columns moved right 100");
    assert_eq!(steps(&app).len(), before + 1, "one step");
    let (step, moved) = rect_marquee_move(Rect::new(200, 0, 301, 600), (100.0, 70.0));
    assert_eq!(steps(&app).last(), Some(&step));
    let b = bounds(&app).unwrap();
    assert_eq!((b.x0, b.x1, b.y0), (300, 401, 70), "{b:?}");
    assert_eq!(b.y1, moved.unwrap().y1, "past the canvas edge, as the Rectangular Marquee's move keeps it");
}

#[test]
fn a_drag_from_outside_places_one_new_line_at_the_release() {
    for (tool, from, to, want) in
        [(Tool::SingleRowMarquee, (100.0, 100.0), (100.0, 200.0), vec![200]), (Tool::SingleColumnMarquee, (100.0, 100.0), (150.0, 100.0), vec![150])]
    {
        let mut app = two_lines(tool);
        drag(&mut app, from, to, Modifiers::NONE);
        let row = tool == Tool::SingleRowMarquee;
        assert_eq!(lines(&app, row), want, "{tool:?}");
        let name = if row { "Single Row Marquee" } else { "Single Column Marquee" };
        assert_eq!(steps(&app).last().map(String::as_str), Some(name), "{tool:?}");
    }
}

#[test]
fn a_click_inside_without_moving_selects_the_line_under_the_pointer() {
    for (tool, at, want) in [(Tool::SingleRowMarquee, (300.0, 250.5), vec![250]), (Tool::SingleColumnMarquee, (300.5, 250.0), vec![300])] {
        let mut app = two_lines(tool);
        drag(&mut app, at, at, Modifiers::NONE);
        let row = tool == Tool::SingleRowMarquee;
        assert_eq!(lines(&app, row), want, "{tool:?}: one line, never deselected");
        let name = if row { "Single Row Marquee" } else { "Single Column Marquee" };
        assert_eq!(steps(&app).last().map(String::as_str), Some(name), "{tool:?}");
    }
}

#[test]
fn a_shift_drag_from_inside_still_adds_a_line() {
    let mut app = two_lines(Tool::SingleRowMarquee);
    drag(&mut app, (300.0, 250.5), (400.0, 320.5), Modifiers::SHIFT);
    assert_eq!(lines(&app, true), [150, 250, 320], "Shift picks Add: a new row at the release");
    let mut app = two_lines(Tool::SingleColumnMarquee);
    drag(&mut app, (300.5, 250.0), (400.5, 320.0), Modifiers::SHIFT);
    assert_eq!(lines(&app, false), [200, 300, 400]);
}
