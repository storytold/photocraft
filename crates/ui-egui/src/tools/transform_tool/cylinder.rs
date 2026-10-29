//! Edit › Transform › Warp's Cylinder in Warp mode (#4112): the preset that wraps the contents
//! round a horizontal cylinder, for the label on a can, a jar or a bottle. Photoshop keeps it as
//! seven values rather than a mesh ([`photocraft_geom::warp::cylinder_mesh`]): the left and right
//! silhouettes, the top and bottom edges, the depth of the upper and lower ellipse (in units of
//! the half axis) and how far the contents wrap round (0..1). This is its editor: five handles
//! set those values, and the warp stays a cylinder through the commit, so a smart object keeps
//! it as `warpCylinder` in a PSD.
//!
//! The handles and gestures follow the reference application by observation (never its code),
//! as a maintainer compared them (#9798): a square at the top-right and the bottom-left corner,
//! each moving its silhouette and its edge while the opposite corner stays put; a handle at the
//! front of each curve for that ellipse's depth; a wrap handle near the middle, dragged sideways;
//! and a drag anywhere else on the cylinder moves the whole of it. The options bar keeps Bend, H
//! and V greyed, as for a custom warp, and a cylinder has no splits and no grid.

use super::warp::surface_at;
use super::*;

/// What a press on the cylinder grabbed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CylinderHandle {
    /// A corner, clockwise from the top-left; the reference shows the top-right (1) and the
    /// bottom-left (3). Its silhouette and its edge follow the pointer, the opposite corner stays.
    Corner(usize),
    /// The front of the upper (0) or lower (1) curve: that ellipse's depth follows up and down.
    Depth(usize),
    /// The wrap handle near the middle: dragged sideways, it turns the contents round the cylinder.
    Wrap,
    /// Anywhere else on the cylinder: the whole cylinder moves.
    Surface,
}

/// The depth of both ellipses the Warp menu's Cylinder starts with, in units of the half axis:
/// about a quarter of the width, as the reference starts.
const DEFAULT_DEPTH: f64 = 0.5;

/// How far round the Warp menu's Cylinder starts wrapped: the middle of the range.
const DEFAULT_WRAP: f64 = 0.5;

/// The wrap handle's travel either side of the centre, in units of the half axis: the whole wrap
/// range fits within the middle half of the cylinder.
const WRAP_TRAVEL: f64 = 0.5;

/// The silhouettes never cross: the right one stays this far (document px) right of the left.
const MIN_WIDTH: f64 = 1.0;

/// An ellipse is at most as deep as the half axis, either way up.
const DEPTH_LIMIT: f64 = 1.0;

/// The cylinder the Warp menu's Cylinder starts with over the box `b`: its four corners on the
/// box (so the lower curve hangs below it), both curves [`DEFAULT_DEPTH`] deep and
/// [`DEFAULT_WRAP`] wrapped. `None` for an empty box.
pub(super) fn cylinder_default(b: [f64; 4]) -> Option<Warp> {
    if !(b[2] > b[0] && b[3] > b[1]) {
        return None;
    }
    Warp::cylinder(&[b[0], b[3], b[2], b[1], DEFAULT_DEPTH, DEFAULT_DEPTH, DEFAULT_WRAP], b)
}

/// The seven values of `w` when it is a cylinder: left, bottom, right, top (document px), the
/// top and bottom ellipses' depth (half-axis units, down is positive) and the wrap (0..1).
pub(super) fn cylinder_values(w: &Warp) -> Option<[f64; 7]> {
    if w.style != WarpStyle::Cylinder {
        return None;
    }
    w.values.as_slice().try_into().ok()
}

/// Where the five handles are, in document space: the top-right and bottom-left corners, the
/// front of each curve (the middle of the top and bottom edge, where the mesh puts it) and the
/// wrap handle, on the centre line between the curves' fronts, [`WRAP_TRAVEL`] left of the
/// centre at wrap 0 and as far right at wrap 1.
pub(super) fn cylinder_handles(v: &[f64; 7]) -> [([f64; 2], CylinderHandle); 5] {
    let [left, bottom, right, top, top_depth, bottom_depth, wrap] = *v;
    let a = (right - left) / 2.0;
    let cx = (left + right) / 2.0;
    let (top_front, bottom_front) = (top + top_depth * a, bottom + bottom_depth * a);
    [
        ([right, top], CylinderHandle::Corner(1)),
        ([left, bottom], CylinderHandle::Corner(3)),
        ([cx, top_front], CylinderHandle::Depth(0)),
        ([cx, bottom_front], CylinderHandle::Depth(1)),
        ([cx + (wrap.clamp(0.0, 1.0) - 0.5) * a * WRAP_TRAVEL, (top_front + bottom_front) / 2.0], CylinderHandle::Wrap),
    ]
}

/// The nearest handle within `tol` of `p`, else the surface when `p` is on the cylinder, else
/// `None` (off it: a press there grabs nothing).
pub(super) fn cylinder_hit(w: &Warp, p: [f64; 2], tol: f64) -> Option<CylinderHandle> {
    let v = cylinder_values(w)?;
    let d = |q: [f64; 2]| (q[0] - p[0]).hypot(q[1] - p[1]);
    let nearest = cylinder_handles(&v).into_iter().map(|(q, h)| (d(q), h)).filter(|(dist, _)| *dist < tol).min_by(|a, b| a.0.total_cmp(&b.0));
    match nearest {
        Some((_, h)) => Some(h),
        None => surface_at(w.mesh.as_ref()?, p).map(|_| CylinderHandle::Surface),
    }
}

/// The values after `handle` is dragged by `d` from the press, where they were `v0`. A corner
/// takes its silhouette and its edge along (the opposite corner stays put); a depth handle
/// follows the pointer up and down; the wrap handle follows it sideways over its travel; the
/// surface moves the whole cylinder. The silhouettes keep their order and the depths stay within
/// the half axis.
pub(super) fn cylinder_dragged(v0: [f64; 7], handle: CylinderHandle, d: [f64; 2]) -> [f64; 7] {
    let mut v = v0;
    let a = ((v0[2] - v0[0]) / 2.0).max(MIN_WIDTH / 2.0);
    match handle {
        CylinderHandle::Corner(i) => {
            if i == 0 || i == 3 {
                v[0] += d[0];
            } else {
                v[2] += d[0];
            }
            if i < 2 {
                v[3] += d[1];
            } else {
                v[1] += d[1];
            }
        }
        CylinderHandle::Depth(0) => v[4] = (v0[4] + d[1] / a).clamp(-DEPTH_LIMIT, DEPTH_LIMIT),
        CylinderHandle::Depth(_) => v[5] = (v0[5] + d[1] / a).clamp(-DEPTH_LIMIT, DEPTH_LIMIT),
        CylinderHandle::Wrap => v[6] = (v0[6] + d[0] / (a * WRAP_TRAVEL)).clamp(0.0, 1.0),
        CylinderHandle::Surface => {
            v[0] += d[0];
            v[2] += d[0];
            v[1] += d[1];
            v[3] += d[1];
        }
    }
    // The silhouette being moved stops just short of the other one.
    if v[2] - v[0] < MIN_WIDTH {
        if matches!(handle, CylinderHandle::Corner(1 | 2)) {
            v[2] = v[0] + MIN_WIDTH;
        } else {
            v[0] = v[2] - MIN_WIDTH;
        }
    }
    v
}

/// Pointer input on a cylinder in Warp mode: a press grabs a handle or the surface
/// ([`cylinder_hit`]), and the drag sets the values ([`cylinder_dragged`]) and rebuilds the mesh
/// from them on every move, so the preview follows. A press off the cylinder grabs nothing.
pub(super) fn cylinder_pointer(app: &mut PhotocraftApp, ev: ToolEvent, tol: f64) {
    let (Some(t), Some(pv)) = (app.ui.transform.as_mut(), app.transform_preview.as_mut()) else { return };
    let Some(w) = t.warp.as_mut() else { return };
    match ev {
        ToolEvent::Down { x, y, .. } => {
            pv.warp_drag =
                cylinder_values(w).and_then(|values0| cylinder_hit(w, [x, y], tol).map(|handle| WarpDrag::Cylinder { handle, start: [x, y], values0 }));
        }
        ToolEvent::Move { x, y, .. } | ToolEvent::Up { x, y } => {
            if let Some(WarpDrag::Cylinder { handle, start, values0 }) = &pv.warp_drag
                && let Some(c) = Warp::cylinder(&cylinder_dragged(*values0, *handle, [x - start[0], y - start[1]]), w.bounds)
            {
                *w = c;
            }
            if matches!(ev, ToolEvent::Up { .. }) {
                pv.warp_drag = None;
            }
        }
    }
}

/// The cursor over document point `p` of the cylinder `w`: a resize arrow on a handle (as the
/// view turns it), the move cursor on the surface, the plain arrow off it.
pub(super) fn cylinder_cursor(w: &Warp, p: [f64; 2], tol: f64, xf: &ViewXform) -> CursorIcon {
    let Some(v) = cylinder_values(w) else { return CursorIcon::Default };
    let quad = [[v[0], v[3]], [v[2], v[3]], [v[2], v[1]], [v[0], v[1]]];
    match cylinder_hit(w, p, tol) {
        Some(CylinderHandle::Corner(i)) => crate::tool_cursor::corner_resize(&quad, i, xf),
        Some(CylinderHandle::Depth(_)) => crate::tool_cursor::resize([0.0, 1.0], xf),
        Some(CylinderHandle::Wrap) => crate::tool_cursor::resize([1.0, 0.0], xf),
        Some(CylinderHandle::Surface) => CursorIcon::Move,
        None => CursorIcon::Default,
    }
}

/// The cylinder's handles over its curves: squares at the two corners, circles at the front of
/// each curve, a diamond for the wrap; the one being dragged is filled with the lines' colour.
pub(super) fn draw_cylinder(painter: &egui::Painter, w: &Warp, scr: impl Fn([f64; 2]) -> Pos2, pv: &TransformPreview, line: Stroke) {
    let Some(v) = cylinder_values(w) else { return };
    let dragged = match &pv.warp_drag {
        Some(WarpDrag::Cylinder { handle, .. }) => Some(*handle),
        _ => None,
    };
    for (q, h) in cylinder_handles(&v) {
        let p = scr(q);
        let fill = if dragged == Some(h) { line.color } else { Color32::WHITE };
        match h {
            CylinderHandle::Depth(_) => {
                painter.circle_filled(p, 3.5, fill);
                painter.circle_stroke(p, 3.5, line);
            }
            CylinderHandle::Wrap => {
                let pts = vec![p - vec2(0.0, 5.0), p + vec2(5.0, 0.0), p + vec2(0.0, 5.0), p - vec2(5.0, 0.0)];
                painter.add(egui::Shape::convex_polygon(pts, fill, line));
            }
            _ => {
                let r = egui::Rect::from_center_size(p, vec2(7.0, 7.0));
                painter.rect_filled(r, 0.0, fill);
                painter.rect_stroke(r, 0.0, line, egui::StrokeKind::Inside);
            }
        }
    }
}
