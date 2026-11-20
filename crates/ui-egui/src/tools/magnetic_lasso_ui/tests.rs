//! The Magnetic Lasso through `tool_event` (mouse and `ui.pointer` alike): the `PcEdges` adapter and
//! the commit mapping.

use craft::behavior::{Cursor, PaintedCursor};
use craft::ui::primitives::ToolIcon;
use craft::ui::runtime::{cursor, windows::WindowHost};
use egui::Modifiers;
use photocraft_geom::Rect;
use serde_json::json;

use super::inspect;
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::guide_cursor_tests::{harness, hover};
use crate::state::Tool;

const SQUARE: Rect = Rect::new(50, 40, 130, 110);

/// A black 80×70 square on a white 200×150 document, the Magnetic Lasso, 100 % zoom.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.sync_views();
    app.session
        .edit("paint", |doc, _| {
            let bg = doc.layers[0].surface_mut().unwrap();
            bg.fill_rect(Rect::new(0, 0, 200, 150), &[1.0, 1.0, 1.0, 1.0]);
            bg.fill_rect(SQUARE, &[0.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::MagneticLasso;
    // Frequency 100 (a point about every 24 px with the button up, #11687): the 80 x 70 square is small.
    app.ui.tool_options.magnetic.frequency = 100.0;
    app
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64, m: Modifiers) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x, y }, m);
}

/// The pointer moves (button up) along the polyline `pts` in 2 px steps, like a hand on a mouse.
fn hover_along(app: &mut PhotocraftApp, pts: &[[f64; 2]]) {
    for w in pts.windows(2) {
        let len = (w[0][0] - w[1][0]).hypot(w[0][1] - w[1][1]);
        let n = (len / 2.0).ceil().max(1.0) as usize;
        for k in 1..=n {
            let t = k as f64 / n as f64;
            let (x, y) = (w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t);
            tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, Modifiers::NONE);
        }
    }
}

/// Around the square, 3 px outside its edges, back to just past the start.
const AROUND: [[f64; 2]; 6] = [[52.0, 37.0], [133.0, 37.0], [133.0, 113.0], [47.0, 113.0], [47.0, 37.0], [52.0, 37.0]];

fn coverage(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    app.session.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))
}

fn selected_area(app: &PhotocraftApp) -> f32 {
    (0..150).flat_map(|y| (0..200).map(move |x| (x, y))).map(|(x, y)| coverage(app, x, y)).sum()
}

fn last_step(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.entries().last().cloned()
}

#[test]
fn tracing_around_a_shape_selects_it() {
    let mut app = app();
    click(&mut app, AROUND[0][0], AROUND[0][1], Modifiers::NONE);
    assert!(app.canvas_tools.selection.outline_open());
    hover_along(&mut app, &AROUND[..5]);
    // Points fastened by themselves on the way (about every 24 px at Frequency 100).
    let v = inspect(&app);
    assert!(v["anchors"].as_array().is_some_and(|a| a.len() >= 5), "{v}");
    // No selection until the border closes, and nothing in the history.
    assert_eq!(last_step(&app).as_deref(), Some("paint"));
    // Back at the first point, a click closes the border along the edges.
    hover_along(&mut app, &AROUND[4..]);
    click(&mut app, AROUND[5][0], AROUND[5][1], Modifiers::NONE);
    assert!(!app.canvas_tools.selection.outline_open());
    let area = selected_area(&app);
    assert!((area - 5600.0).abs() < 120.0, "the square's 80 × 70: {area}");
    assert_eq!((coverage(&app, 90, 75), coverage(&app, 47, 75), coverage(&app, 90, 113)), (1.0, 0.0, 0.0));
    assert_eq!(last_step(&app).as_deref(), Some("Magnetic Lasso"));
}

#[test]
fn agents_see_the_border_in_progress() {
    let mut app = app();
    click(&mut app, 52.0, 37.0, Modifiers::NONE);
    hover_along(&mut app, &[[52.0, 37.0], [133.0, 37.0]]);
    let v = crate::control::inspect(&app, app.ctx());
    let m = &v["magnetic"];
    assert_eq!(m["mode"], json!("replace"));
    assert!(m["anchors"].as_array().is_some_and(|a| !a.is_empty()), "{m}");
    assert!(m["path"].as_array().is_some_and(|p| p.len() >= 2), "{m}");
    assert!(m["live"].as_array().is_some_and(|p| !p.is_empty()), "{m}");
    assert_eq!(v["tool"], json!("MagneticLasso"));
}

#[test]
fn a_document_edit_while_drawing_is_traced_on() {
    let mut app = app();
    click(&mut app, 52.0, 37.0, Modifiers::NONE);
    hover_along(&mut app, &[[52.0, 37.0], [90.0, 37.0]]);
    // The square grows 10 px taller upwards mid-border: the border follows the new edge.
    app.session
        .edit("grow", |doc, _| {
            doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(50, 30, 130, 40), &[0.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    hover_along(&mut app, &[[90.0, 37.0], [100.0, 27.0], [125.0, 27.0]]);
    let v = inspect(&app);
    let end = v["live"].as_array().and_then(|l| l.last()).expect("a live segment");
    let y = end[1].as_f64().unwrap();
    assert!((y - 30.0).abs() < 0.01, "on the new edge: {end}");
}

/// The live Cursors preference reaches the Magnetic Lasso through the app's canvas (#15466).
#[test]
fn the_magnetic_lasso_cursor_follows_other_cursors_and_caps_lock() {
    let mut h = harness(Tool::MagneticLasso);
    h.state_mut().ui.tool_options.magnetic.width = 10.0;
    let host = WindowHost::of(&h.ctx);
    let lasso = ToolIcon::MagneticLasso.icon();

    for (preference, caps_lock, zoom, radius) in [
        ("standard", false, 1.0, None),
        ("precise", false, 1.0, Some(10.0)),
        ("precise", true, 1.0, None),
        ("standard", true, 1.0, Some(10.0)),
        ("standard", true, 2.0, Some(20.0)),
        ("standard", false, 2.0, None),
    ] {
        h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": preference})).unwrap();
        let view = &mut h.state_mut().ui.views[0];
        view.zoom = zoom;
        view.clear_pending();
        host.set_caps_lock(Some(caps_lock));
        // The fixture's vertical guide is at x = 200. Hover clear of it.
        hover(&mut h, 100.0, 100.0, Modifiers::NONE);

        let shown = cursor::current(&h.ctx);
        let matches = match (radius, &shown) {
            (Some(expected), Some(Cursor::Painted(PaintedCursor::Circle { radius, .. }))) => (*radius - expected).abs() < 1e-3,
            (None, Some(Cursor::Painted(PaintedCursor::Tool { icon, .. }))) => *icon == lasso,
            _ => false,
        };
        assert!(matches, "{preference}, Caps Lock {caps_lock}, zoom {zoom}: expected radius {radius:?} or a lasso icon, got {shown:?}");
    }
}
