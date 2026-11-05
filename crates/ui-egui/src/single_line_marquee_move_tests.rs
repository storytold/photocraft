//! Single Row / Single Column Marquee drags through the real canvas (#11392): a drag that starts
//! inside the selection moves it, as the other marquees' does; one that starts outside, a click
//! inside, and a ⇧ / ⌥ press still select the line under the release.

use egui::Modifiers;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// An 800 × 600 document with snapping off, so the drags land where they are sent.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.ui.extras.snap = false;
    app
}

/// A drag with `tool` from `a` to `b`, as the canvas sends it.
fn drag(app: &mut PhotocraftApp, tool: Tool, a: [f64; 2], b: [f64; 2], m: Modifiers) {
    app.ui.tool = tool;
    tool_event(app, ToolEvent::Down { x: a[0], y: a[1], pressure: 1.0 }, m);
    let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    tool_event(app, ToolEvent::Move { x: mid[0], y: mid[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: b[0], y: b[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: b[0], y: b[1] }, m);
}

fn click(app: &mut PhotocraftApp, tool: Tool, p: [f64; 2], m: Modifiers) {
    app.ui.tool = tool;
    tool_event(app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: p[0], y: p[1] }, m);
}

fn bounds(app: &PhotocraftApp) -> Rect {
    app.session.active().unwrap().doc.selection.as_ref().map_or(Rect::EMPTY, |s| s.content_bounds())
}

fn selected(app: &PhotocraftApp, x: i32, y: i32) -> bool {
    app.session.active().unwrap().doc.selection.as_ref().is_some_and(|s| s.sample_channel(x, y, 0) > 0.5)
}

fn step(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.undo_label().map(str::to_string)
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

/// Rows 150 and 250 (New, then ⇧ adds).
fn two_rows(app: &mut PhotocraftApp) {
    click(app, Tool::SingleRowMarquee, [300.0, 250.5], Modifiers::NONE);
    click(app, Tool::SingleRowMarquee, [200.0, 150.5], Modifiers::SHIFT);
    assert_eq!(bounds(app), Rect::new(0, 150, 800, 251));
}

/// Columns 200 and 300.
fn two_columns(app: &mut PhotocraftApp) {
    click(app, Tool::SingleColumnMarquee, [300.5, 250.0], Modifiers::NONE);
    click(app, Tool::SingleColumnMarquee, [200.5, 150.0], Modifiers::SHIFT);
    assert_eq!(bounds(app), Rect::new(200, 0, 301, 600));
}

/// What the Rectangular Marquee's move of the same selection gives: where the part moved past the
/// canvas goes, and the History step's name.
fn rect_marquee_move(select: impl Fn(&mut PhotocraftApp), a: [f64; 2], b: [f64; 2]) -> (Rect, Option<String>) {
    let mut app = app();
    select(&mut app);
    drag(&mut app, Tool::RectMarquee, a, b, Modifiers::NONE);
    (bounds(&app), step(&app))
}

#[test]
fn single_row_drag_from_inside_moves_the_selection() {
    let mut app = app();
    two_rows(&mut app);
    let before = steps(&app);
    drag(&mut app, Tool::SingleRowMarquee, [300.0, 250.5], [400.0, 320.5], Modifiers::NONE);
    assert!(selected(&app, 450, 220) && selected(&app, 450, 320), "both rows moved by +100,+70");
    assert!(!selected(&app, 450, 150) && !selected(&app, 450, 250), "the old rows are gone");
    assert!(!selected(&app, 50, 220), "moved right by 100");
    assert_eq!(steps(&app), before + 1, "one step");
    assert_eq!((bounds(&app), step(&app)), rect_marquee_move(two_rows, [300.0, 250.5], [400.0, 320.5]), "as the Rectangular Marquee moves it");
}

#[test]
fn single_column_drag_from_inside_moves_the_selection() {
    let mut app = app();
    two_columns(&mut app);
    let before = steps(&app);
    drag(&mut app, Tool::SingleColumnMarquee, [300.5, 250.0], [400.5, 320.0], Modifiers::NONE);
    assert!(selected(&app, 300, 400) && selected(&app, 400, 400), "both columns moved by +100,+70");
    assert!(!selected(&app, 200, 400), "the old column 200 is gone");
    assert!(!selected(&app, 300, 30), "moved down by 70");
    assert_eq!(steps(&app), before + 1, "one step");
    assert_eq!((bounds(&app), step(&app)), rect_marquee_move(two_columns, [300.5, 250.0], [400.5, 320.0]), "as the Rectangular Marquee moves it");
}

#[test]
fn drag_from_outside_selects_the_line_at_the_release() {
    let mut app = app();
    two_rows(&mut app);
    drag(&mut app, Tool::SingleRowMarquee, [100.0, 100.5], [100.0, 200.5], Modifiers::NONE);
    assert_eq!(bounds(&app), Rect::new(0, 200, 800, 201));
    assert_eq!(step(&app).as_deref(), Some("Single Row Marquee"));

    two_columns(&mut app);
    drag(&mut app, Tool::SingleColumnMarquee, [100.5, 100.0], [150.5, 100.0], Modifiers::NONE);
    assert_eq!(bounds(&app), Rect::new(150, 0, 151, 600));
    assert_eq!(step(&app).as_deref(), Some("Single Column Marquee"));
}

#[test]
fn click_inside_selects_the_line_under_the_pointer() {
    let mut app = app();
    two_rows(&mut app);
    click(&mut app, Tool::SingleRowMarquee, [300.0, 250.5], Modifiers::NONE);
    assert_eq!(bounds(&app), Rect::new(0, 250, 800, 251), "row 250 alone, not deselected");
    assert_eq!(step(&app).as_deref(), Some("Single Row Marquee"));

    two_columns(&mut app);
    click(&mut app, Tool::SingleColumnMarquee, [200.5, 150.0], Modifiers::NONE);
    assert_eq!(bounds(&app), Rect::new(200, 0, 201, 600), "column 200 alone, not deselected");
    assert_eq!(step(&app).as_deref(), Some("Single Column Marquee"));
}

/// With snapping on, a click inside is still a click when the selection's edge is within reach of
/// the canvas edge: the snapped release moved both rows up 3 px ("Transform Selection") instead.
#[test]
fn click_inside_with_snapping_on_selects_the_line() {
    let mut app = app();
    click(&mut app, Tool::SingleRowMarquee, [300.0, 250.5], Modifiers::NONE);
    click(&mut app, Tool::SingleRowMarquee, [200.0, 3.5], Modifiers::SHIFT);
    assert_eq!(bounds(&app), Rect::new(0, 3, 800, 251));
    app.ui.extras.snap = true;
    click(&mut app, Tool::SingleRowMarquee, [300.0, 250.5], Modifiers::NONE);
    assert_eq!(bounds(&app), Rect::new(0, 250, 800, 251));
    assert_eq!(step(&app).as_deref(), Some("Single Row Marquee"));
}

/// A click whose release drifts under a pixel from the press (a zoomed-in hand jitter) is still a
/// click with snapping on: only an exactly still release ended unsnapped, so the jittered one
/// snapped the rows onto the canvas edge ("Transform Selection") and the Rectangular Marquee's
/// selection with them instead of deselecting.
#[test]
fn a_jittered_click_inside_with_snapping_on_is_still_a_click() {
    let jittered_click = |app: &mut PhotocraftApp, tool: Tool, p: [f64; 2]| {
        app.ui.tool = tool;
        let q = [p[0] + 0.2, p[1] - 0.3];
        tool_event(app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, Modifiers::NONE);
        tool_event(app, ToolEvent::Move { x: q[0], y: q[1], pressure: 1.0 }, Modifiers::NONE);
        tool_event(app, ToolEvent::Up { x: q[0], y: q[1] }, Modifiers::NONE);
    };
    let mut app = app();
    click(&mut app, Tool::SingleRowMarquee, [300.0, 250.5], Modifiers::NONE);
    click(&mut app, Tool::SingleRowMarquee, [200.0, 3.5], Modifiers::SHIFT);
    app.ui.extras.snap = true;
    jittered_click(&mut app, Tool::SingleRowMarquee, [300.0, 250.5]);
    assert_eq!(bounds(&app), Rect::new(0, 250, 800, 251), "row 250 alone, not both rows moved up 3 px");
    assert_eq!(step(&app).as_deref(), Some("Single Row Marquee"));

    app.run("select.rect", json!({"x": 3, "y": 3, "width": 100, "height": 50})).unwrap();
    jittered_click(&mut app, Tool::RectMarquee, [50.5, 30.5]);
    assert!(app.session.active().unwrap().doc.selection.is_none(), "deselected, not moved onto the edge");
}

/// With snapping on, a click inside the selection snaps the line onto a guide 2 px away, as a
/// click outside it does; the press began as a move, which kept it on the pointer.
#[test]
fn click_inside_snaps_the_line_to_a_guide() {
    let mut app = app();
    app.run("view.newGuide", json!({"orientation": "horizontal", "position": 252})).unwrap();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 302})).unwrap();
    // The selections are made unsnapped; only the click under test snaps.
    let snapped_click = |app: &mut PhotocraftApp, tool: Tool, p: [f64; 2]| {
        app.ui.extras.snap = true;
        click(app, tool, p, Modifiers::NONE);
        app.ui.extras.snap = false;
    };

    snapped_click(&mut app, Tool::SingleRowMarquee, [600.0, 250.5]);
    assert_eq!(bounds(&app), Rect::new(0, 252, 800, 253), "outside the selection: snapped to the guide");

    two_rows(&mut app);
    snapped_click(&mut app, Tool::SingleRowMarquee, [600.0, 250.5]);
    assert_eq!(bounds(&app), Rect::new(0, 252, 800, 253), "inside: snapped to the guide too");
    assert_eq!(step(&app).as_deref(), Some("Single Row Marquee"));

    two_columns(&mut app);
    snapped_click(&mut app, Tool::SingleColumnMarquee, [300.5, 500.0]);
    assert_eq!(bounds(&app), Rect::new(302, 0, 303, 600), "a column inside snaps to the guide");
    assert_eq!(step(&app).as_deref(), Some("Single Column Marquee"));

    // No guide within reach: the line under the pointer, not the selection's own edge.
    two_rows(&mut app);
    snapped_click(&mut app, Tool::SingleRowMarquee, [600.0, 150.5]);
    assert_eq!(bounds(&app), Rect::new(0, 150, 800, 151));
}

#[test]
fn combining_drag_from_inside_selects_a_line() {
    let mut app = app();
    two_rows(&mut app);
    // ⇧ adds the row at the release; nothing moves.
    drag(&mut app, Tool::SingleRowMarquee, [300.0, 250.5], [400.0, 320.5], Modifiers::SHIFT);
    assert!(selected(&app, 10, 150) && selected(&app, 10, 250) && selected(&app, 10, 320));
    // ⌥ subtracts the row under the release.
    drag(&mut app, Tool::SingleRowMarquee, [300.0, 320.5], [350.0, 250.5], Modifiers::ALT);
    assert!(selected(&app, 10, 150) && !selected(&app, 10, 250) && selected(&app, 10, 320));

    two_columns(&mut app);
    drag(&mut app, Tool::SingleColumnMarquee, [300.5, 250.0], [400.5, 320.0], Modifiers::SHIFT);
    assert!(selected(&app, 200, 10) && selected(&app, 300, 10) && selected(&app, 400, 10));
}
