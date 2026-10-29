use super::curvature::curvature_refresh;
use super::path_ops::{active_type_path, targeted_vector_mask, work_path_shown_in};
use super::pen::{closing, over_first_anchor, shift_45};
use egui::{Color32, Pos2, Rect, Stroke, pos2, vec2};
use photocraft_doc::vector::Path;
use photocraft_doc::{Document, LayerContent};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::Tool;
use crate::theme::Tokens;

// ---------------------------------------------------------------------------------------------
// Drawing

pub(super) fn bezier(p0: [f64; 2], c0: [f64; 2], c1: [f64; 2], p1: [f64; 2], t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    let f = |a: f64, b: f64, c: f64, d: f64| u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d;
    [f(p0[0], c0[0], c1[0], p1[0]), f(p0[1], c0[1], c1[1], p1[1])]
}

/// `q` lies on the chord from `a` to `d` (within a thousandth of a pixel).
fn on_chord(a: [f64; 2], q: [f64; 2], d: [f64; 2]) -> bool {
    let (dx, dy, vx, vy) = (d[0] - a[0], d[1] - a[1], q[0] - a[0], q[1] - a[1]);
    let len = dx.hypot(dy);
    if len < 1e-9 {
        return vx.hypot(vy) < 1e-3;
    }
    let t = (vx * dx + vy * dy) / (len * len);
    (vx * dy - vy * dx).abs() / len < 1e-3 && (-1e-9..=1.0 + 1e-9).contains(&t)
}

/// How far (in screen pixels) a drawn chord may stray from the true curve.
const CURVE_TOLERANCE: f64 = 0.3;

/// Pieces per segment at most: 0.3 px holds for curves bending up to about 1.4 million px on screen.
pub(super) const MAX_CURVE_PIECES: usize = 1024;

/// Screen points along one cubic segment (both ends included). The control points are mapped to
/// the screen first (`xf` is affine, so the curve maps exactly), and the number of chords follows
/// how much the curve bends there: a straight segment is one chord, and a curve zoomed in stays
/// smooth instead of showing a fixed 16 chords (#4084). Uses the bound
/// `max|B''| / 8n² <= tol` with `max|B''| <= 6·max(|p0−2c0+c1|, |c0−2c1+p1|)`.
pub(crate) fn curve_points(p0: [f64; 2], c0: [f64; 2], c1: [f64; 2], p1: [f64; 2], xf: &dyn Fn([f64; 2]) -> Pos2) -> Vec<Pos2> {
    let [a, b, c, d] = [p0, c0, c1, p1].map(|q| {
        let s = xf(q);
        [f64::from(s.x), f64::from(s.y)]
    });
    let second = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| (p[0] - 2.0 * q[0] + r[0]).hypot(p[1] - 2.0 * q[1] + r[1]);
    let n = (0.75 * second(a, b, c).max(second(b, c, d)) / CURVE_TOLERANCE).sqrt().ceil();
    // A straight segment (controls on the chord, like the Pen's retracted handles) is one chord;
    // NaN (a non-finite point) falls back to one too.
    let n = if n.is_finite() && !(on_chord(a, b, d) && on_chord(a, c, d)) { (n as usize).clamp(1, MAX_CURVE_PIECES) } else { 1 };
    (0..=n)
        .map(|k| {
            let q = bezier(a, b, c, d, k as f64 / n as f64);
            pos2(q[0] as f32, q[1] as f32)
        })
        .collect()
}

/// Screen polylines for a path (one per subpath).
pub(crate) fn path_lines(path: &Path, xf: &dyn Fn([f64; 2]) -> Pos2) -> Vec<(Vec<Pos2>, bool)> {
    path.subpaths
        .iter()
        .map(|s| {
            let n = s.knots.len();
            let mut pts = Vec::new();
            let segs = if s.closed { n } else { n.saturating_sub(1) };
            for i in 0..segs {
                let (a, b) = (&s.knots[i], &s.knots[(i + 1) % n]);
                pts.extend(curve_points([a.anchor.x, a.anchor.y], [a.out_ctrl.x, a.out_ctrl.y], [b.in_ctrl.x, b.in_ctrl.y], [b.anchor.x, b.anchor.y], xf));
            }
            if pts.is_empty()
                && let Some(k) = s.knots.first()
            {
                pts.push(xf([k.anchor.x, k.anchor.y]));
            }
            (pts, s.closed)
        })
        .collect()
}

/// Path Options › Color: the colours a path can be drawn in, by id. "default" is the theme's
/// accent (#9713).
pub(crate) const PATH_COLORS: &[(&str, &str, Option<[u8; 3]>)] = &[
    ("default", "Default", None),
    ("lightBlue", "Light Blue", Some([0x4c, 0xb4, 0xff])),
    ("red", "Red", Some([0xff, 0x2a, 0x2a])),
    ("green", "Green", Some([0x22, 0xc5, 0x3a])),
    ("mediumBlue", "Medium Blue", Some([0x1f, 0x4f, 0xe0])),
    ("yellow", "Yellow", Some([0xff, 0xd8, 0x00])),
    ("magenta", "Magenta", Some([0xf0, 0x1e, 0xd2])),
    ("cyan", "Cyan", Some([0x00, 0xe0, 0xe0])),
    ("lightGray", "Light Gray", Some([0xc0, 0xc0, 0xc0])),
    ("black", "Black", Some([0x00, 0x00, 0x00])),
];

/// Path Options › Thickness range (screen px).
pub(crate) const PATH_THICKNESS: std::ops::RangeInclusive<f32> = 0.5..=3.0;
/// Path Options › Anchor Size range; 3 draws the 6 px squares, a pixel more or less per step.
pub(crate) const ANCHOR_SIZES: std::ops::RangeInclusive<u32> = 1..=7;

/// How paths, their anchors and direction handles are drawn: the Pen options' Path Options
/// (#9713). The default look is a 1 px accent line with 6 px anchor squares.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathLook {
    pub color: Color32,
    /// The outline's width (screen px).
    pub width: f32,
    /// An anchor square's side (screen px).
    pub anchor: f32,
    /// Ring the first anchor of each open subpath, where the path begins.
    pub mark_start: bool,
}

impl PathLook {
    /// The default look in `accent`, for overlays Path Options doesn't style.
    pub(crate) fn plain(accent: Color32) -> Self {
        Self { color: accent, width: 1.0, anchor: 6.0, mark_start: false }
    }

    /// The look Path Options sets; its Default colour is `accent`. Out-of-range or non-finite
    /// settings (a hand-edited settings file) fall back to the nearest valid ones.
    pub(crate) fn of(app: &PhotocraftApp, accent: Color32) -> Self {
        let o = &app.ui.tool_options;
        let color = PATH_COLORS.iter().find(|c| c.0 == o.path_color).and_then(|c| c.2).map_or(accent, |[r, g, b]| Color32::from_rgb(r, g, b));
        let width = if o.path_thickness.is_finite() { o.path_thickness.clamp(*PATH_THICKNESS.start(), *PATH_THICKNESS.end()) } else { 1.0 };
        let size = o.anchor_size.clamp(*ANCHOR_SIZES.start(), *ANCHOR_SIZES.end());
        Self { color, width, anchor: 6.0 + size as f32 - 3.0, mark_start: o.path_mark_start }
    }
}

/// A path's outline in the look's colour and width.
pub(crate) fn draw_outline(painter: &egui::Painter, p: &Path, to_scr: &dyn Fn([f64; 2]) -> Pos2, look: &PathLook) {
    for (pts, closed) in path_lines(p, to_scr) {
        if closed {
            painter.add(egui::Shape::closed_line(pts, Stroke::new(look.width, look.color)));
        } else {
            painter.add(egui::Shape::line(pts, Stroke::new(look.width, look.color)));
        }
    }
}

/// An anchor point's square: filled with the path colour when selected, else hollow (white).
pub(crate) fn draw_anchor(painter: &egui::Painter, c: Pos2, selected: bool, look: &PathLook) {
    let r = Rect::from_center_size(c, vec2(look.anchor, look.anchor));
    painter.rect_filled(r, 0.0, if selected { look.color } else { Color32::WHITE });
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, look.color), egui::StrokeKind::Inside);
}

/// A direction handle: the line from its anchor and the round handle end.
pub(crate) fn draw_handle(painter: &egui::Painter, anchor: Pos2, handle: Pos2, look: &PathLook) {
    painter.line_segment([anchor, handle], Stroke::new(1.0, look.color));
    painter.circle_filled(handle, look.anchor / 2.0, look.color);
}

/// Path Options › Highlight Start Point: a ring around the first anchor of each open subpath, so
/// the end a path was begun at can be told from the one it was finished at (#9713). Closed
/// subpaths have no start to find.
pub(crate) fn draw_start_marks(painter: &egui::Painter, p: &Path, to_scr: &dyn Fn([f64; 2]) -> Pos2, look: &PathLook) {
    if !look.mark_start {
        return;
    }
    for k in p.subpaths.iter().filter(|s| !s.closed && s.knots.len() > 1).filter_map(|s| s.knots.first()) {
        draw_start_ring(painter, to_scr([k.anchor.x, k.anchor.y]), look);
    }
}

/// The ring [`draw_start_marks`] draws around a start anchor at `c`.
fn draw_start_ring(painter: &egui::Painter, c: Pos2, look: &PathLook) {
    painter.circle_stroke(c, look.anchor * 0.5 + 3.5, Stroke::new(1.5, look.color));
}

/// Does `tool` draw the anchor squares of the paths it can edit? Path Selection does, and so do
/// the Pen tools: as in Photoshop, a path finished with Enter or by closing it keeps its anchors
/// on screen, so it can still be edited (⌘/Ctrl-drag, Auto Add/Delete) (#6884, #4716).
pub(crate) fn shows_anchors(tool: Tool) -> bool {
    // Add/Delete Anchor Point and Convert Point draw Direct Selection's overlay instead.
    matches!(tool, Tool::PathSelection | Tool::Pen | Tool::FreeformPen | Tool::CurvaturePen)
}

/// The saved path selected in the Paths panel: Photoshop's target path, drawn on the canvas with
/// any tool (#6550). `None` when the panel selects the work path, a layer's path or nothing.
pub(crate) fn selected_saved_path(app: &PhotocraftApp) -> Option<&Path> {
    let sel = app.ui.selected_path.as_deref().filter(|s| !matches!(*s, "work" | "layer"))?;
    app.session.active()?.doc.paths.iter().find(|p| p.name == sel).map(|p| &p.path)
}

/// Work path / active shape path / selected saved path outlines, anchors, and the pen path in
/// progress.
pub fn draw_overlay(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, doc: &Document) {
    let tool = app.ui.tool;
    let accent = Tokens::get(painter.ctx()).accent;
    let look = PathLook::of(app, accent);
    let to_scr = |q: [f64; 2]| xf.to_screen(q[0] as f32, q[1] as f32);
    let draw_path = |p: &Path, anchors: bool| {
        draw_outline(painter, p, &to_scr, &look);
        if anchors {
            for k in p.subpaths.iter().flat_map(|s| &s.knots) {
                draw_anchor(painter, to_scr([k.anchor.x, k.anchor.y]), false, &look);
            }
        }
        // The start ring shows on an outline without anchor squares too (a shape tool's path).
        draw_start_marks(painter, p, &to_scr, &look);
    };
    // View › Show › Target Path (under Extras) hides the paths; the Pen's path in progress stays.
    // Free Transform Path draws the path through its box instead (transform_tool).
    let paths = app.ui.view.shows(app.ui.view.show.target_path) && app.ui.transform.as_ref().is_none_or(|t| t.path.is_none());
    if app.session.active().is_some_and(|st| st.doc.id == doc.id) && crate::direct_select::shows(app, painter.ctx().input(|i| i.modifiers)) {
        // Direct Selection (or the Pen with ⌘/Ctrl held) draws the paths it edits (#790).
        crate::direct_select::draw_overlay(app, painter, &to_scr, &look, paths);
        if paths {
            // The selected saved path shows too, though Direct Selection doesn't edit it yet.
            if let Some(p) = selected_saved_path(app) {
                draw_path(p, true);
            }
            crate::anchor_tools::draw_hover(app, painter, &to_scr, &look, painter.ctx().input(|i| i.modifiers));
        }
    } else if paths {
        // The shape's or vector mask's path stays visible as an outline with any tool. Path
        // Selection and the Pen tools add anchor squares; Direct Selection has its own overlay
        // above. The Work Path is drawn only while it is the path selected in the Paths panel or
        // a path tool is active, as in Photoshop: a Work Path left unselected (one a PSD arrived
        // with, or after Ctrl-click Load as Selection) would otherwise cover the marching ants (#6551).
        let anchors = shows_anchors(tool);
        if let Some(wp) = doc.work_path.as_ref().filter(|_| work_path_shown_in(app, doc))
            && (anchors || app.ui.selected_path.as_deref() == Some("work"))
        {
            draw_path(wp, anchors);
        }
        // The saved path selected in the Paths panel is the target path (#6550).
        if let Some(p) = selected_saved_path(app) {
            draw_path(p, anchors);
        }
        let st = app.session.active();
        if let Some(l) = st.and_then(|s| s.active_layer.and_then(|id| s.doc.layer(id)))
            && let LayerContent::Shape(sh) = &l.content
        {
            draw_path(&sh.path, anchors);
        }
        // A targeted vector mask shows its path with any tool (#196).
        if let Some((_, p)) = targeted_vector_mask(app) {
            draw_path(&p, anchors);
        }
        // So does type on a path's Type Path, which the Pen's Auto Add/Delete edits (#9114).
        if let Some((_, p)) = active_type_path(app) {
            draw_path(&p, anchors);
        }
        crate::corner_radius::draw(app, painter, &to_scr, accent);
    }
    // Pen path in progress, with handles of the last knot and a rubber band to the pointer. While
    // ⌘/Ctrl drags it, Direct Selection draws its preview instead.
    let edited = matches!(&app.ui.direct_selection.drag, Some(crate::direct_select::Drag::Edit { target: crate::direct_select::PathRef::Pen, .. }));
    // Only on the document it is drawn on: a view of another document never shows it (#5373).
    if let Some(pen) = app.ui.pen.as_ref().filter(|_| !edited).filter(|_| app.ui.pen_doc.is_none_or(|d| d == doc.id)) {
        let mut pts = Vec::new();
        // A close-drag shows the first segment bending with the handle being pulled out.
        let mut knots = pen.knots.clone();
        if closing(pen)
            && let Some(&[a, _, out]) = pen.knots.last()
            && out != a
            && let Some(first) = knots.first_mut()
        {
            first[2] = out;
        }
        for w in knots.windows(2) {
            pts.extend(curve_points(w[0][0], w[0][2], w[1][1], w[1][0], &to_scr));
        }
        painter.add(egui::Shape::line(pts, Stroke::new(look.width + 0.5, look.color)));
        // No rubber band where a click adds or deletes an anchor of this path instead (#6140).
        let auto = app.hover_doc.is_some_and(|h| crate::anchor_tools::pen_auto(app, tool, painter.ctx().input(|i| i.modifiers), h).is_some());
        if let (Some(last), Some(h)) = (pen.knots.last(), app.hover_doc)
            && !pen.dragging
            && pen.moving.is_none()
            && !auto
        {
            if pen.curvature && pen.knots.len() >= 2 {
                // The Curvature Pen previews the curve the next click makes through the pointer.
                let mut next = pen.clone();
                next.knots.push([h; 3]);
                curvature_refresh(&mut next, false);
                if let [.., a, b] = next.knots.as_slice() {
                    let pts = curve_points(a[0], a[2], b[1], b[0], &to_scr);
                    painter.add(egui::Shape::dashed_line(&pts, Stroke::new(1.0, look.color), 4.0, 3.0));
                }
            } else {
                // The rubber band ends where a ⇧-click would put the next anchor.
                let shift = painter.ctx().input(|i| i.modifiers.shift);
                let h = if pen.curvature { h } else { shift_45(last[0], h, shift) };
                painter.add(egui::Shape::dashed_line(&[to_scr(last[0]), to_scr(h)], Stroke::new(1.0, look.color), 4.0, 3.0));
            }
        }
        for (i, k) in pen.knots.iter().enumerate() {
            let c = to_scr(k[0]);
            let last = i + 1 == pen.knots.len();
            if last && k[2] != k[0] && !pen.curvature {
                for hnd in [k[1], k[2]] {
                    draw_handle(painter, c, to_scr(hnd), &look);
                }
            }
            draw_anchor(painter, c, last, &look);
        }
        if look.mark_start
            && pen.knots.len() > 1
            && let Some(first) = pen.knots.first()
        {
            draw_start_ring(painter, to_scr(first[0]), &look);
        }
        // Hovering the first anchor: Photoshop's close cue, a small ring beside the pointer.
        let tol = 6.0 / f64::from(app.point_zoom().max(0.01));
        if let Some(h) = app.hover_doc
            && over_first_anchor(pen, h, tol)
            && let Some(first) = pen.knots.first()
        {
            let c = to_scr(first[0]);
            painter.rect_filled(Rect::from_center_size(c, vec2(look.anchor + 2.0, look.anchor + 2.0)), 0.0, look.color);
            painter.circle_stroke(c + vec2(11.0, 11.0), 3.5, Stroke::new(1.5, look.color));
        }
    }
    // The segment the Pen's Auto Add/Delete would add an anchor to, or the anchor it would delete,
    // over the path being drawn. While drawing, the other paths can be hidden: it edits none of them.
    if paths || app.ui.pen.is_some() {
        crate::anchor_tools::draw_pen_hover(app, painter, &to_scr, &look, painter.ctx().input(|i| i.modifiers));
    }
}
