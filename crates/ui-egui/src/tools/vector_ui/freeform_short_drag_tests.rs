//! #12831: a Freeform Pen drag of 2 px draws nothing, as a click does, as in Photoshop. Drags of
//! 3 to 10 px (not measured) keep making a path, and the measured 400 px stroke still does.

use egui::Modifiers;
use photocraft_doc::vector::Path;

use super::freeform_close_tolerance_tests::app;
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};

/// A drag in document px from `from` to `to` in one move. Returns the Work Path and the number of
/// History steps it added.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) -> (Option<Path>, usize) {
    let before = app.session.active().unwrap().history.past_len();
    let mods = Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    if from != to {
        tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    }
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
    let st = app.session.active().unwrap();
    (st.doc.work_path.clone(), st.history.past_len() - before)
}

#[test]
fn at_100_percent_a_click_and_a_2_px_drag_draw_nothing() {
    // 2 px plus the f32 error a real drag gets from the screen-to-document mapping.
    for to in [[200.0, 200.0], [202.0, 200.0], [200.0, 198.0], [201.0, 201.0], [202.0 + 5e-5, 200.0]] {
        let (path, steps) = drag(&mut app(1.0), [200.0, 200.0], to);
        assert!(path.is_none(), "to {to:?}: no Work Path");
        assert_eq!(steps, 0, "to {to:?}: no History step");
    }
}

#[test]
fn the_straight_400_px_stroke_still_makes_its_2_anchor_path() {
    let mut app = app(1.0);
    let (path, steps) = drag(&mut app, [100.0, 400.0], [500.0, 400.0]);
    let subpaths = path.unwrap().subpaths;
    assert_eq!((subpaths.len(), subpaths[0].knots.len(), steps), (1, 2, 1));
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Freeform Pen"));
}

/// Not measured: drags from 3 to 10 px keep today's path.
#[test]
fn drags_of_3_to_10_px_still_make_a_path() {
    for len in 3..=10 {
        let (path, steps) = drag(&mut app(1.0), [200.0, 200.0], [200.0 + f64::from(len), 200.0]);
        assert_eq!(path.map(|p| p.subpaths[0].knots.len()), Some(2), "{len} px");
        assert_eq!(steps, 1, "{len} px");
    }
}

/// Not measured: at other zooms only a drag within 2 px both on screen and in the document is
/// dropped.
#[test]
fn at_other_zooms_only_a_drag_short_on_screen_and_in_the_document_is_dropped() {
    for (zoom, len, drawn) in [(2.0, 1.0, false), (2.0, 2.0, true), (0.5, 2.0, false), (0.5, 3.0, true)] {
        let (path, steps) = drag(&mut app(zoom), [200.0, 200.0], [200.0 + len, 200.0]);
        assert_eq!((path.is_some(), steps), (drawn, usize::from(drawn)), "{len} document px at {zoom}×");
    }
}

/// Not measured: on a Retina screen (2 pixels per point) at 100 % the 2 px is in document px.
#[test]
fn on_a_retina_screen_at_100_percent_a_2_px_drag_draws_nothing() {
    for (len, drawn) in [(2.0, false), (3.0, true)] {
        let mut app = app(1.0);
        app.ppp = 2.0;
        let (path, steps) = drag(&mut app, [200.0, 200.0], [200.0 + len, 200.0]);
        assert_eq!((path.is_some(), steps), (drawn, usize::from(drawn)), "{len} px");
    }
}

/// Not measured: ⌥-clicked straight segments of 1 px keep today's anchors.
#[test]
fn a_short_trace_of_alt_clicked_straight_segments_still_makes_its_anchors() {
    let mut app = app(1.0);
    for x in [200.0, 201.0] {
        tool_event(&mut app, ToolEvent::Down { x, y: 200.0, pressure: 1.0 }, Modifiers::ALT);
        tool_event(&mut app, ToolEvent::Up { x, y: 200.0 }, Modifiers::ALT);
    }
    // Releasing ⌥ between clicks finishes the trace.
    tool_event(&mut app, ToolEvent::Move { x: 201.0, y: 200.0, pressure: 1.0 }, Modifiers::NONE);
    let st = app.session.active().unwrap();
    let knots = st.doc.work_path.as_ref().map(|p| p.subpaths[0].knots.len());
    assert_eq!((knots, st.history.undo_label()), (Some(2), Some("Freeform Pen")));
}
