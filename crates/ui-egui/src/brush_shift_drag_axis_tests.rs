//! #11889: ⇧-drag with the Brush is constrained to horizontal or vertical only, never to a 45°
//! diagonal. The measured reference behaviour (issue text, coordinates copied verbatim):
//!
//! - ⇧-drag 100 px right and 40 px down from 100,100: a horizontal line.
//! - ⇧-drag mostly vertical: a vertical line.
//! - ⇧-drag at exactly 45° (100,100 → 200,200): only the start dab, no diagonal.
//! - ⇧-click still draws a line from the last painted point in any direction.
//! - The Freeform Pen's and the Gradient tool's ⇧ 45° snap are unchanged.
//!
//! The bounds below are PhotoCraft's own: the default 20 px round tip covers 24 px in each
//! direction (start dab 88,88 → 112,112), so a line from 100,100 to 200,100 bounds 88,88 →
//! 212,112 -- the same flat band the reference application's 89,89 → 110,110 start dab shows.

use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// A fresh transparent document with the default brush and no smoothing, as the issue measured.
fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 320, "height": 320, "background": "transparent"})).unwrap();
    app.session.tools.brush.smoothing.amount = 0.0;
    app.ui.tool = tool;
    app
}

/// ⇧-drag from `from` through `mids` to `to`, holding ⇧ the whole way.
fn shift_drag(app: &mut PhotocraftApp, from: [f64; 2], mids: &[[f64; 2]], to: [f64; 2]) {
    let shift = egui::Modifiers::SHIFT;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, shift);
    for p in mids.iter().chain(std::iter::once(&to)) {
        tool_event(app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, shift);
    }
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, shift);
}

fn bounds(app: &PhotocraftApp) -> Rect {
    app.session.documents()[0].doc.layers[0].surface().unwrap().content_bounds()
}

fn alpha(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    app.session.documents()[0].doc.layers[0].surface().unwrap().rgba(x, y)[3]
}

fn points(app: &PhotocraftApp) -> Vec<[f64; 2]> {
    let p = app.session.journal.iter().rev().find(|(id, _)| id == "paint.stroke").map(|(_, p)| p.clone()).unwrap();
    p["points"].as_array().unwrap().iter().map(|q| [q[0].as_f64().unwrap(), q[1].as_f64().unwrap()]).collect()
}

/// The measured reference drag "100 px right and 40 px down" paints a horizontal line.
#[test]
fn shift_drag_mostly_horizontal_paints_a_horizontal_line() {
    let mut app = app(Tool::Brush);
    shift_drag(&mut app, [100.0, 100.0], &[[140.0, 116.0]], [200.0, 140.0]);
    assert!(points(&app).iter().all(|p| p[1] == 100.0), "every point on y = 100: {:?}", points(&app));
    assert_eq!(bounds(&app), Rect::new(88, 88, 212, 112), "a horizontal band, 24 px tall");
    assert!(alpha(&app, 170, 100) > 0.9, "painted along the horizontal line");
    assert_eq!(alpha(&app, 170, 140), 0.0, "nothing where the pointer went diagonally");
}

/// The case the issue reports: 100,100 to 200,200 paints only the start dab -- no diagonal.
#[test]
fn shift_drag_at_45_degrees_paints_no_diagonal() {
    let mut app = app(Tool::Brush);
    shift_drag(&mut app, [100.0, 100.0], &[[150.0, 150.0]], [200.0, 200.0]);
    assert_eq!(points(&app), vec![[100.0, 100.0]], "the tie never decides an axis, so only the start dab");
    assert_eq!(bounds(&app), Rect::new(88, 88, 112, 112), "only the start dab, not the 124x124 diagonal of today's build");
    for k in [120, 150, 180, 200] {
        assert_eq!(alpha(&app, k, k), 0.0, "the diagonal at {k},{k} stayed empty");
    }
    assert_eq!(app.session.documents()[0].history.past_len(), 1, "one Brush Tool state");
}

/// A drag that starts on the diagonal and then breaks the tie picks up the dominant axis.
#[test]
fn a_tie_broken_mid_drag_finishes_on_the_dominant_axis() {
    let mut app = app(Tool::Brush);
    shift_drag(&mut app, [100.0, 100.0], &[[150.0, 150.0], [250.0, 160.0]], [260.0, 170.0]);
    assert!(points(&app).iter().all(|p| p[1] == 100.0), "horizontal once the tie broke: {:?}", points(&app));
    assert_eq!(bounds(&app), Rect::new(88, 88, 272, 112));
}

/// ⇧-click after a stroke still paints a straight line from the last painted point in any
/// direction, including a 45° one (Done-when bullet 3).
#[test]
fn shift_click_still_connects_in_any_direction() {
    let mut app = app(Tool::Brush);
    let none = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 40.0, y: 40.0, pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Up { x: 40.0, y: 40.0 }, none);
    let shift = egui::Modifiers::SHIFT;
    let before = app.session.documents()[0].history.past_len();
    tool_event(&mut app, ToolEvent::Down { x: 200.0, y: 200.0, pressure: 1.0 }, shift);
    tool_event(&mut app, ToolEvent::Up { x: 200.0, y: 200.0 }, shift);
    assert_eq!(points(&app), vec![[40.0, 40.0], [200.0, 200.0]], "a 45° connecting line");
    assert_eq!(app.session.documents()[0].history.past_len(), before + 1, "one state per segment");
    assert!(alpha(&app, 120, 120) > 0.9, "painted along the diagonal");
    // A non-axis-aligned segment too.
    tool_event(&mut app, ToolEvent::Down { x: 260.0, y: 120.0, pressure: 1.0 }, shift);
    tool_event(&mut app, ToolEvent::Up { x: 260.0, y: 120.0 }, shift);
    assert_eq!(points(&app), vec![[200.0, 200.0], [260.0, 120.0]]);
    assert!(alpha(&app, 230, 160) > 0.9);
}
