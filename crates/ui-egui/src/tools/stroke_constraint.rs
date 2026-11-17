//! ⇧ with painting tools and the Gradient tool (#178), as in Photoshop:
//!
//! - ⇧ while dragging a painting stroke keeps it on a straight line at 0°, 45° or 90° from where
//!   the constraint began (the stroke's start, or the point where ⇧ went down mid-stroke). The
//!   direction locks once the pointer has moved a few screen points.
//! - ⇧ while dragging with the **Brush** only ever goes horizontal or vertical (#11889): the axis
//!   that dominates the movement is chosen, and a drag that stays exactly on the 45° diagonal
//!   paints nothing but its start dab. The other painting tools and the Freeform Pen keep the
//!   45° snap above.
//! - ⇧-click after a stroke paints a straight line from the end of the previous stroke to the
//!   click. The line is part of the new stroke, so it is one `paint.stroke` and one undo step.
//!   The endpoint remembers pen pressure so size and opacity interpolate between the strokes.
//! - The Gradient tool's ⇧ snaps the gradient angle to 45° increments.
//!
//! Points go through the normal drag/`LiveStroke`/commit path, so the live preview, pen dynamics
//! and Smoothing treat them like any other stroke points.

use craft::behavior::kurbo::{Point, Rect};
use craft::ui::canvas_controls::StraightLinePreview;
use craft::ui::canvas_controls::brush::TipCell;
use craft::ui::contract::Show as _;

use crate::state::Tool;

/// Screen points the pointer must travel before a ⇧-constrained stroke locks its direction.
const LOCK_DISTANCE: f64 = 3.0;

/// A ⇧ constraint in progress: its origin and, once locked, the unit direction of the line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis {
    pub origin: [f64; 2],
    pub dir: Option<[f64; 2]>,
}

/// Tools whose drag ⇧ constrains: every painting tool, the Gradient tool, and the Freeform Pen
/// (Photoshop's freehand path segment with ⇧, #7171).
pub fn applies(tool: Tool) -> bool {
    tool.is_brushlike() || tool == Tool::Gradient || tool == Tool::FreeformPen
}

/// Tools that ⇧-click connects to the previous stroke with a straight line.
/// The Art History Brush paints a separate cluster at the new click instead (#12627).
pub fn connects(tool: Tool) -> bool {
    tool.is_brushlike() && tool != Tool::ArtHistoryBrush
}

/// How a ⇧-constrained drag locks its direction. One shared lock (`constrain`) serves every
/// tool, so the difference between them is just this mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snap {
    /// The nearest multiple of 45° (#178).
    Degrees45,
    /// Horizontal or vertical only, from the axis that dominates the movement (#11889).
    Axis,
}

/// The ⇧ drag lock `tool` uses. The Brush takes the reference application's horizontal/vertical
/// lock (#11889). The Freeform Pen keeps its 45° snap (#7304); the other painting tools keep
/// #178's behaviour, which has not been measured against the reference.
pub fn snap_mode(tool: Tool) -> Snap {
    if tool == Tool::Brush { Snap::Axis } else { Snap::Degrees45 }
}

/// The unit direction from `origin` towards `p`, snapped to the nearest multiple of `step`
/// radians; None when the points coincide (or aren't finite).
pub fn snapped_dir(origin: [f64; 2], p: [f64; 2], step: f64) -> Option<[f64; 2]> {
    let (dx, dy) = (p[0] - origin[0], p[1] - origin[1]);
    if !(dx.is_finite() && dy.is_finite()) || dx.hypot(dy) < 1e-9 || step <= 0.0 {
        return None;
    }
    let a = (dy.atan2(dx) / step).round() * step;
    Some([a.cos(), a.sin()])
}

/// `p` projected onto the line through `origin` along unit `dir`.
pub fn project(origin: [f64; 2], dir: [f64; 2], p: [f64; 2]) -> [f64; 2] {
    let t = (p[0] - origin[0]) * dir[0] + (p[1] - origin[1]) * dir[1];
    [origin[0] + dir[0] * t, origin[1] + dir[1] * t]
}

/// The direction a ⇧-constrained drag locks to, in `mode`. None means "not decided yet": the
/// pointer has not moved, or `Snap::Axis` met an exact 45° tie.
fn lock_dir(mode: Snap, origin: [f64; 2], p: [f64; 2]) -> Option<[f64; 2]> {
    match mode {
        Snap::Degrees45 => snapped_dir(origin, p, std::f64::consts::FRAC_PI_4),
        Snap::Axis => axis_dir(origin, p),
    }
}

/// The horizontal or vertical unit direction of `origin` → `p`: the axis that dominates the
/// movement (#11889). An exact 45° tie decides neither axis, so it returns None and the drag stays
/// on its start point -- as in the reference application, where a 45° ⇧-drag paints only its start
/// dab. The lock is retried on the next point, so any later movement breaks the tie.
fn axis_dir(origin: [f64; 2], p: [f64; 2]) -> Option<[f64; 2]> {
    let (dx, dy) = (p[0] - origin[0], p[1] - origin[1]);
    if !(dx.is_finite() && dy.is_finite()) || dx.hypot(dy) < 1e-9 {
        return None;
    }
    if dx.abs() == dy.abs() {
        return None;
    }
    Some(if dx.abs() > dy.abs() { [1.0, 0.0] } else { [0.0, 1.0] })
}

/// `p` moved onto the nearest 45° ray from `origin` (the Gradient tool's ⇧).
pub fn snap45(origin: [f64; 2], p: [f64; 2]) -> [f64; 2] {
    match snapped_dir(origin, p, std::f64::consts::FRAC_PI_4) {
        Some(d) => project(origin, d, p),
        None => p,
    }
}

/// Constrain the pointer at `p` for a drag of `tool` that started at `start` and has reached
/// `last`. `axis` carries the stroke's lock between events; `zoom` converts the lock distance
/// to document pixels. The lock follows the tool's [`snap_mode`]: horizontal/vertical for the
/// Brush, a multiple of 45° for the other ⇧-constrained drags. Without ⇧ the point passes
/// through and the lock is dropped.
pub fn constrain(tool: Tool, axis: &mut Option<Axis>, start: [f64; 2], last: [f64; 2], p: [f64; 2], shift: bool, zoom: f32) -> [f64; 2] {
    if !shift || !applies(tool) {
        *axis = None;
        return p;
    }
    if tool == Tool::Gradient {
        return snap45(start, p);
    }
    let mode = snap_mode(tool);
    let a = axis.get_or_insert(Axis { origin: last, dir: None });
    if a.dir.is_none() {
        let reach = LOCK_DISTANCE / f64::from(zoom.max(1e-3));
        if (p[0] - a.origin[0]).hypot(p[1] - a.origin[1]) < reach {
            return a.origin;
        }
        a.dir = lock_dir(mode, a.origin, p);
    }
    match a.dir {
        Some(d) => project(a.origin, d, p),
        None => a.origin,
    }
}

/// Where the ⇧-click line preview starts (#257): the end of the last stroke on the active
/// document, while ⇧ is held with a painting tool and nothing is being painted.
pub fn line_preview_start(app: &crate::PhotocraftApp, tool: Tool, shift: bool) -> Option<[f64; 2]> {
    if !shift || !connects(tool) || crate::canvas::tools::in_gesture(app) {
        return None;
    }
    let active = app.session.active()?.doc.id;
    app.last_stroke_end.filter(|(doc, _)| *doc == active).map(|(_, p)| [p[0], p[1]])
}

/// Photoshop's rubber band: while ⇧ is held after a stroke, a thin line from where that stroke
/// ended to the pointer (`pointer`, document units) shows the line a click will paint, with the
/// brush footprint at the pointer (shown here only when the painting cursor doesn't already show
/// the tip). Nothing shows without a start (see [`line_preview_start`]) or with a non-finite one.
pub(crate) fn show_line_preview(ui: &mut egui::Ui, app: &crate::PhotocraftApp, pointer: Point, tool: Tool, shift: bool) {
    let Some(from) = line_preview_start(app, tool, shift) else { return };
    if !(from[0].is_finite() && from[1].is_finite()) {
        return;
    }
    StraightLinePreview::new(tl!("Line preview"), Point::new(from[0], from[1]), pointer).show(ui);
    use photocraft_engine::prefs::PaintingCursor;
    if matches!(app.session.prefs().cursors.painting, PaintingCursor::Standard | PaintingCursor::Precise) {
        let half = f64::from(app.session.tools.brush.size) / 2.0;
        let cell = Rect::new(pointer.x - half, pointer.y - half, pointer.x + half, pointer.y + half);
        TipCell::new(cell, true).show(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PhotocraftApp;
    use crate::canvas::{ToolEvent, tool_event};
    use serde_json::json;

    fn close(a: [f64; 2], b: [f64; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
    }

    #[test]
    fn snapping_angles() {
        let o = [10.0, 10.0];
        // Gradient: 45° increments.
        assert!(close(snap45(o, [50.0, 13.0]), [50.0, 10.0]));
        assert!(close(snap45(o, [12.0, -40.0]), [10.0, -40.0]));
        let d = snap45(o, [40.0, 37.0]);
        assert!((d[0] - d[1]).abs() < 1e-9 && d[0] > 30.0, "45°: {d:?}");
        assert!(close(snap45(o, o), o));
        assert!(snap45(o, [f64::NAN, 1.0])[0].is_nan(), "non-finite input passes through");
        // Painting: the direction locks once the pointer has moved a few points.
        let mut axis = None;
        let p = |q, ax: &mut Option<Axis>| constrain(Tool::Brush, ax, o, o, q, true, 1.0);
        assert!(close(p([11.0, 11.0], &mut axis), o), "inside the lock distance");
        assert!(close(p([30.0, 12.0], &mut axis), [30.0, 10.0]), "horizontal");
        // Locked: wandering towards 45° stays on the horizontal line.
        assert!(close(p([60.0, 45.0], &mut axis), [60.0, 10.0]));
        // Releasing ⇧ frees the stroke and drops the lock.
        assert!(close(constrain(Tool::Brush, &mut axis, o, o, [70.0, 50.0], false, 1.0), [70.0, 50.0]) && axis.is_none());
        // Vertical lock.
        let mut axis = None;
        assert!(close(p([9.0, 60.0], &mut axis), [10.0, 60.0]));
        // The Brush never locks to a diagonal: this slightly-more-vertical movement goes vertical.
        let mut axis = None;
        assert!(close(p([40.0, 43.0], &mut axis), [10.0, 43.0]), "the Brush's near-diagonal goes vertical");
        // Every other ⇧-constrained drag keeps the 45° lock (#178, and #7304 for the Freeform Pen).
        for tool in [Tool::Pencil, Tool::Eraser, Tool::FreeformPen] {
            let mut axis = None;
            let q = constrain(tool, &mut axis, o, o, [40.0, 43.0], true, 1.0);
            assert!((q[0] - 10.0 - (q[1] - 10.0)).abs() < 1e-9, "{tool:?} should lock to 45°: {q:?}");
        }
        // An exact 45° tie decides no axis, so the Brush holds at its start; a later point breaks
        // the tie (#11889).
        let mut axis = None;
        assert!(close(p([100.0, 100.0], &mut axis), o), "exact tie stays on the start point");
        assert!(close(p([160.0, 140.0], &mut axis), [160.0, 10.0]), "a later point breaks the tie");
        // Other tools pass through.
        assert!(close(constrain(Tool::Move, &mut None, o, o, [30.0, 12.0], true, 1.0), [30.0, 12.0]));
    }

    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 120, "height": 120, "background": "transparent"})).unwrap();
        app.run("tools.setBrush", json!({"brush": {"size": 4, "hardness": 1.0}})).unwrap();
        app.session.tools.brush.smoothing.amount = 0.0;
        app
    }

    fn last_stroke_points(app: &PhotocraftApp) -> Vec<[f64; 2]> {
        let p = app.session.journal.iter().rev().find(|(id, _)| id == "paint.stroke").map(|(_, p)| p.clone()).unwrap();
        p["points"].as_array().unwrap().iter().map(|q| [q[0].as_f64().unwrap(), q[1].as_f64().unwrap()]).collect()
    }

    fn alpha(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
        app.session.documents()[0].doc.layers[0].surface().unwrap().rgba(x, y)[3]
    }

    #[test]
    fn shift_click_draws_a_line_from_the_last_stroke_in_one_step() {
        let mut app = app();
        app.ui.tool = Tool::Brush;
        let none = egui::Modifiers::NONE;
        // A first (free) stroke ending at (20, 20).
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, none);
        tool_event(&mut app, ToolEvent::Move { x: 20.0, y: 20.0, pressure: 1.0 }, none);
        tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 20.0 }, none);
        let before = app.session.documents()[0].history.past_len();
        // ⇧-click at (100, 100): a straight line from (20, 20).
        let shift = egui::Modifiers::SHIFT;
        tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 100.0, pressure: 1.0 }, shift);
        tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 100.0 }, shift);
        assert_eq!(last_stroke_points(&app), vec![[20.0, 20.0], [100.0, 100.0]]);
        assert_eq!(app.session.documents()[0].history.past_len(), before + 1, "one undo step per line");
        assert!(alpha(&app, 60, 60) > 0.9 && alpha(&app, 60, 70) == 0.0);
        // And the next ⇧-click continues from there.
        tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 20.0, pressure: 1.0 }, shift);
        tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 20.0 }, shift);
        assert_eq!(last_stroke_points(&app), vec![[100.0, 100.0], [100.0, 20.0]]);
        assert!(alpha(&app, 100, 60) > 0.9);
        // Undo removes just the last line.
        app.session.undo();
        assert!(alpha(&app, 100, 60) == 0.0 && alpha(&app, 60, 60) > 0.9);
        // The line start survives the undo (as in Photoshop). With smoothing on, the line still
        // runs from (100, 20) exactly to the click.
        app.session.tools.brush.smoothing.amount = 0.6;
        tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 100.0, pressure: 1.0 }, shift);
        tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 100.0 }, shift);
        assert!(alpha(&app, 20, 100) > 0.9 && alpha(&app, 40, 80) > 0.9 && alpha(&app, 90, 30) > 0.9);
    }

    #[test]
    fn shift_click_line_ends_at_the_click_with_catch_up_on_stroke_end_off() {
        // Measured in Photoshop (#11887): smoothing doesn't apply to a ⇧-click line, whatever
        // Smoothing and Catch-up on Stroke End are set to.
        let mut app = app();
        app.ui.tool = Tool::Brush;
        app.session.tools.brush.smoothing.amount = 0.6;
        app.session.tools.brush.smoothing.catch_up_on_end = false;
        let none = egui::Modifiers::NONE;
        // A plain two-point drag at this smoothing ends short of the release point.
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 60.0, pressure: 1.0 }, none);
        tool_event(&mut app, ToolEvent::Move { x: 100.0, y: 60.0, pressure: 1.0 }, none);
        tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 60.0 }, none);
        assert!(alpha(&app, 100, 60) == 0.0, "a smoothed drag lags");
        // ⇧-click: the line runs from the drag's release point all the way to the click.
        let shift = egui::Modifiers::SHIFT;
        tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 110.0, pressure: 1.0 }, shift);
        tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 110.0 }, shift);
        assert_eq!(last_stroke_points(&app), vec![[100.0, 60.0], [100.0, 110.0]]);
        assert!(alpha(&app, 100, 62) > 0.9 && alpha(&app, 100, 85) > 0.9 && alpha(&app, 100, 110) > 0.9);
        // The options bar's Smoothing is unchanged.
        assert_eq!(app.session.tools.brush.smoothing.amount, 0.6);
    }

    #[test]
    fn retouch_shift_click_line_ends_at_the_click_with_catch_up_on_stroke_end_off() {
        // The retouching tools build their brush in the engine (`retouch_cmds::stroke_brush`, or
        // `resolve_brush` for the Background Eraser), so their lines need the override too (#11887).
        for tool in [Tool::Burn, Tool::BackgroundEraser] {
            let mut app = app();
            app.run("edit.fill", json!({"color": "#808080"})).unwrap();
            let px = |app: &PhotocraftApp, x, y| app.session.documents()[0].doc.layers[0].surface().unwrap().rgba(x, y);
            let gray = px(&app, 100, 60);
            app.ui.tool = tool;
            // The tool's own brush, which the press would swap in.
            crate::paint_mouse::sync_tool_brush(&mut app);
            app.session.tools.brush.smoothing.amount = 0.6;
            app.session.tools.brush.smoothing.catch_up_on_end = false;
            let none = egui::Modifiers::NONE;
            tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 60.0, pressure: 1.0 }, none);
            tool_event(&mut app, ToolEvent::Move { x: 100.0, y: 60.0, pressure: 1.0 }, none);
            tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 60.0 }, none);
            assert_eq!(px(&app, 100, 60), gray, "{tool:?}: a smoothed drag lags");
            let shift = egui::Modifiers::SHIFT;
            tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 110.0, pressure: 1.0 }, shift);
            tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 110.0 }, shift);
            for y in [62, 85, 110] {
                assert_ne!(px(&app, 100, y), gray, "{tool:?}: the line reaches (100, {y})");
            }
            assert_eq!(app.session.tools.brush.smoothing.amount, 0.6);
        }
    }

    #[test]
    fn shift_click_interpolates_from_previous_strokes_endpoint_pressure() {
        let mut app = app();
        app.ui.tool = Tool::Brush;
        app.session.tools.brush.pressure_size = true;
        app.session.tools.brush.pressure_opacity = true;
        let none = egui::Modifiers::NONE;
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 0.2 }, none);
        tool_event(&mut app, ToolEvent::Move { x: 20.0, y: 20.0, pressure: 0.35 }, none);
        tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 20.0 }, none);
        let before = app.session.documents()[0].history.past_len();

        let shift = egui::Modifiers::SHIFT;
        tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 100.0, pressure: 0.85 }, shift);
        tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 100.0 }, shift);
        let (_, stroke) = app.session.journal.iter().rev().find(|(id, _)| id == "paint.stroke").expect("connecting stroke committed");
        let points = stroke["points"].as_array().expect("stroke points array");
        assert_eq!(points.len(), 2, "one connecting segment, one stroke");
        assert_eq!(last_stroke_points(&app), vec![[20.0, 20.0], [100.0, 100.0]]);
        let pressure = |i: usize| points[i][2].as_f64().expect("pen pressure in stroke point");
        assert!((pressure(0) - 0.35).abs() < 1e-6, "line starts at previous pen pressure, not the new click's");
        assert!((pressure(1) - 0.85).abs() < 1e-6, "line ends at the new click's pen pressure");
        assert_eq!(app.session.documents()[0].history.past_len(), before + 1, "connecting line is one undo step");

        // Another Shift-click uses the just-completed endpoint, not the first stroke's.
        tool_event(&mut app, ToolEvent::Down { x: 80.0, y: 30.0, pressure: 0.55 }, shift);
        tool_event(&mut app, ToolEvent::Up { x: 80.0, y: 30.0 }, shift);
        let (_, next) = app.session.journal.iter().rev().find(|(id, _)| id == "paint.stroke").expect("next connecting stroke");
        let next = next["points"].as_array().expect("next stroke points");
        assert!((next[0][2].as_f64().unwrap() - 0.85).abs() < 1e-6);
        assert!((next[1][2].as_f64().unwrap() - 0.55).abs() < 1e-6);
    }

    #[test]
    fn shift_click_without_a_previous_stroke_is_a_dab() {
        let mut app = app();
        app.ui.tool = Tool::Brush;
        let shift = egui::Modifiers::SHIFT;
        tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, shift);
        tool_event(&mut app, ToolEvent::Up { x: 50.0, y: 50.0 }, shift);
        assert_eq!(last_stroke_points(&app), vec![[50.0, 50.0]]);
        // A new document doesn't connect to another document's stroke.
        app.run("file.new", json!({"width": 60, "height": 60, "background": "transparent"})).unwrap();
        tool_event(&mut app, ToolEvent::Down { x: 5.0, y: 5.0, pressure: 1.0 }, shift);
        tool_event(&mut app, ToolEvent::Up { x: 5.0, y: 5.0 }, shift);
        assert_eq!(last_stroke_points(&app), vec![[5.0, 5.0]]);
    }

    #[test]
    fn line_preview_needs_shift_a_previous_stroke_and_no_stroke_in_progress() {
        let mut app = app();
        app.ui.tool = Tool::Brush;
        assert_eq!(line_preview_start(&app, Tool::Brush, true), None, "no previous stroke");
        let none = egui::Modifiers::NONE;
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, none);
        tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 20.0, pressure: 1.0 }, none);
        assert_eq!(line_preview_start(&app, Tool::Brush, true), None, "while painting");
        tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 20.0 }, none);
        assert_eq!(line_preview_start(&app, Tool::Brush, true), Some([30.0, 20.0]));
        assert_eq!(line_preview_start(&app, Tool::Brush, false), None, "⇧ released");
        assert_eq!(line_preview_start(&app, Tool::Eraser, true), Some([30.0, 20.0]), "any painting tool");
        assert_eq!(line_preview_start(&app, Tool::Move, true), None);
        // Another document has no line start.
        app.run("file.new", json!({"width": 60, "height": 60})).unwrap();
        assert_eq!(line_preview_start(&app, Tool::Brush, true), None);
    }
}
