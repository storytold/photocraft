//! The brush feedback the Canvas body shows after the displays: the stroke's footprint trail, the
//! Clone Source crosshair and overlay, the smoothing string, the Object Finder highlight, the ⇧
//! line preview and the brush-resize preview. Each part reads its own state and shows nothing when
//! its input is absent, so a pass without a stroke, marker, finder, ⇧ or resize costs only the
//! checks. The module is `brush_feedback` because `crate::tool_feedback` is the canvas's own.

use craft::behavior::geometry::Homography;
use craft::behavior::kurbo::Point;
use craft::ui::canvas::RasterDisplay;
use craft::ui::canvas_controls::StraightLinePreview;
use craft::ui::canvas_controls::brush::SourceMarker;
use craft::ui::contract::Show as _;

use crate::PhotocraftApp;
use crate::state::Tool;

/// Shows the brush feedback over the canvas body. `rect` is the body's rect and `pointer` its
/// hover position this pass; the parts that follow the pointer show only on the active document's
/// main canvas. Called once per pass after the displays, in the order the parts stack.
pub(crate) fn show(ui: &mut egui::Ui, app: &mut PhotocraftApp, rect: egui::Rect, pointer: Option<egui::Pos2>) {
    crate::stroke_trail::show(ui, app, rect);
    show_source_marker(ui, app, rect);
    let at = pointer.filter(|p| rect.contains(*p)).and_then(|p| crate::canvas::document_display::doc_point_at(app, p));
    // The Clone Source panel's X/Y read the pairing at the last hover point (#13124): a new point
    // repaints so the panel shows it.
    if let Some(at) = at
        && crate::preset_panels::remember_clone_cursor(app, [at.x, at.y])
    {
        ui.ctx().request_repaint();
    }
    show_clone_overlay(ui, app, at, rect);
    show_smoothing_string(ui, app, rect);
    crate::object_finder::show(ui, app, at);
    if let Some(at) = at {
        let shift = craft::ui::runtime::pointer::mods(ui.ctx()).shift;
        let tool = app.ui.tool;
        crate::stroke_constraint::show_line_preview(ui, app, at, tool, shift);
    }
    app.brush_resize.show_preview(ui, app, rect);
}

/// The Clone Source crosshair, while a Clone Stamp or Healing Brush stroke paints.
fn show_source_marker(ui: &mut egui::Ui, app: &PhotocraftApp, rect: egui::Rect) {
    let Some(at) = crate::canvas::paint_tool::stroking(app).and_then(|s| s.points.last()).map(|p| [p[0], p[1]]) else { return };
    let Some(p) = crate::retouch_ui::source_marker_point(app, Some(at)) else { return };
    SourceMarker::new(tl!("Clone source"), Point::new(p[0], p[1])).show_at(ui, rect);
}

/// The Clone Source overlay under the brush: the source's pixels at the hover point, shown through
/// a mapped raster display. It hides while the brush is being resized and while Alt is held (Alt
/// picks a new source).
fn show_clone_overlay(ui: &mut egui::Ui, app: &mut PhotocraftApp, at: Option<Point>, rect: egui::Rect) {
    let Some(at) = at else { return };
    if !matches!(app.ui.tool, Tool::CloneStamp | Tool::Healing) {
        return;
    }
    if app.brush_resize.active() || craft::ui::runtime::pointer::mods(ui.ctx()).alt {
        return;
    }
    let Some(source) = crate::canvas::strokes::clone_source(app, [at.x, at.y]) else { return };
    let corners = source.corners.map(|c| Point::new(c[0], c[1]));
    let Some(frame) = Homography::unit_to_quad(corners) else { return };
    RasterDisplay::new(tl!("Clone source overlay"), &source).mapped(frame, 1).show_at(ui, rect);
}

/// The smoothing string: the line from the brush end to the pointer's end with a dot at the brush
/// end, while a stroke's live preview has one.
fn show_smoothing_string(ui: &mut egui::Ui, app: &PhotocraftApp, rect: egui::Rect) {
    let Some(idx) = app.session.active_index() else { return };
    let Some((brush_end, pointer_end)) = crate::canvas::strokes::live_stroke(app, idx).and_then(|l| l.smoothing_string()) else { return };
    StraightLinePreview::new(tl!("Smoothing"), Point::new(brush_end[0], brush_end[1]), Point::new(pointer_end[0], pointer_end[1])).dot().show_at(ui, rect);
}
