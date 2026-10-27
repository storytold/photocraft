//! Edit › Transform › Warp's Cylinder in Warp mode (#4112): the preset that wraps the contents
//! round a horizontal cylinder, for the label on a can, a jar or a bottle. Photoshop keeps it as
//! seven values rather than a mesh ([`photocraft_geom::warp::cylinder_mesh`]): the left and right
//! silhouettes, the top and bottom edges, the depth of the upper and lower ellipse (in units of
//! the half axis) and how far the contents wrap round (0..1). This is its editor: the handles set
//! those values, the options bar's fields read and type them, and the warp stays a cylinder
//! through the commit, so a smart object keeps it as `warpCylinder` in a PSD.
//!
//! The gestures follow Photoshop's by observation (never its code): the four corners move a
//! silhouette and an edge together, the middle of a silhouette moves it alone, the middle of the
//! top and bottom edge sets that ellipse's depth, and a drag across the surface turns the
//! contents round the cylinder (the wrap). A cylinder has no splits and no grid.

use super::warp::surface_at;
use super::*;

/// What a press on the cylinder grabbed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CylinderHandle {
    /// A corner, clockwise from the top-left: its silhouette and its edge follow the pointer.
    Corner(usize),
    /// The middle of the left (0) or right (1) silhouette: that silhouette follows sideways.
    Side(usize),
    /// The middle of the top (0) or bottom (1) edge: that ellipse's depth follows up and down.
    Depth(usize),
    /// Anywhere else on the cylinder: a drag across it turns the contents round it.
    Surface,
}

/// The depth of both ellipses the Warp menu's Cylinder starts with, in units of the half axis.
const DEFAULT_DEPTH: f64 = 0.15;

/// How far round the Warp menu's Cylinder starts wrapped: the middle of Photoshop's range.
const DEFAULT_WRAP: f64 = 0.5;

/// The silhouettes never cross: the right one stays this far (document px) right of the left.
const MIN_WIDTH: f64 = 1.0;

/// An ellipse is at most as deep as the half axis, either way up.
const DEPTH_LIMIT: f64 = 1.0;

/// The cylinder the Warp menu's Cylinder starts with over the box `b`: the silhouettes on the
/// box's sides, the top edge on its top and the front of the lower ellipse reaching its bottom,
/// [`DEFAULT_DEPTH`] deep (less on a box so wide that the bulge would take more than a quarter
/// of its height) and [`DEFAULT_WRAP`] wrapped. `None` for an empty box.
pub(super) fn cylinder_default(b: [f64; 4]) -> Option<Warp> {
    let (a, h) = ((b[2] - b[0]) / 2.0, b[3] - b[1]);
    if !(a > 0.0 && h > 0.0) {
        return None;
    }
    let depth = DEFAULT_DEPTH.min(0.25 * h / a);
    Warp::cylinder(&[b[0], b[3] - depth * a, b[2], b[1], depth, depth, DEFAULT_WRAP], b)
}

/// The seven values of `w` when it is a cylinder: left, bottom, right, top (document px), the
/// top and bottom ellipses' depth (half-axis units, down is positive) and the wrap (0..1).
pub(super) fn cylinder_values(w: &Warp) -> Option<[f64; 7]> {
    if w.style != WarpStyle::Cylinder {
        return None;
    }
    w.values.as_slice().try_into().ok()
}

/// Where the handles are, in document space: the corners, the middle of each silhouette and the
/// front of each ellipse (the middle of the top and bottom edge, where the mesh puts it).
pub(super) fn cylinder_handles(v: &[f64; 7]) -> [([f64; 2], CylinderHandle); 8] {
    let [left, bottom, right, top, top_depth, bottom_depth, _] = *v;
    let a = (right - left) / 2.0;
    let cx = (left + right) / 2.0;
    let cy = (top + bottom) / 2.0;
    [
        ([left, top], CylinderHandle::Corner(0)),
        ([right, top], CylinderHandle::Corner(1)),
        ([right, bottom], CylinderHandle::Corner(2)),
        ([left, bottom], CylinderHandle::Corner(3)),
        ([left, cy], CylinderHandle::Side(0)),
        ([right, cy], CylinderHandle::Side(1)),
        ([cx, top + top_depth * a], CylinderHandle::Depth(0)),
        ([cx, bottom + bottom_depth * a], CylinderHandle::Depth(1)),
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

/// The values after `handle` is dragged by `d` from the press, where they were `v0`. A corner or
/// a side takes its silhouette (and the corner its edge) along; a depth handle follows the
/// pointer up and down; dragging the surface by the cylinder's width turns the contents the whole
/// way round. The silhouettes keep their order and the depths stay within the half axis.
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
        CylinderHandle::Side(0) => v[0] += d[0],
        CylinderHandle::Side(_) => v[2] += d[0],
        CylinderHandle::Depth(0) => v[4] = (v0[4] + d[1] / a).clamp(-DEPTH_LIMIT, DEPTH_LIMIT),
        CylinderHandle::Depth(_) => v[5] = (v0[5] + d[1] / a).clamp(-DEPTH_LIMIT, DEPTH_LIMIT),
        CylinderHandle::Surface => v[6] = (v0[6] + d[0] / (2.0 * a)).clamp(0.0, 1.0),
    }
    // The silhouette being moved stops just short of the other one.
    if v[2] - v[0] < MIN_WIDTH {
        if matches!(handle, CylinderHandle::Corner(1 | 2) | CylinderHandle::Side(1)) {
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
/// view turns it), a hand on the surface (a drag turns the contents), the plain arrow off it.
pub(super) fn cylinder_cursor(w: &Warp, p: [f64; 2], tol: f64, xf: &ViewXform) -> CursorIcon {
    let Some(v) = cylinder_values(w) else { return CursorIcon::Default };
    let quad = [[v[0], v[3]], [v[2], v[3]], [v[2], v[1]], [v[0], v[1]]];
    match cylinder_hit(w, p, tol) {
        Some(CylinderHandle::Corner(i)) => crate::tool_cursor::corner_resize(&quad, i, xf),
        Some(CylinderHandle::Side(_)) => crate::tool_cursor::resize([1.0, 0.0], xf),
        Some(CylinderHandle::Depth(_)) => crate::tool_cursor::resize([0.0, 1.0], xf),
        Some(CylinderHandle::Surface) => CursorIcon::Grab,
        None => CursorIcon::Default,
    }
}

/// The cylinder's handles over its grid: squares at the corners and the sides, circles at the
/// front of each ellipse; the one being dragged is filled with the grid's colour.
pub(super) fn draw_cylinder(painter: &egui::Painter, w: &Warp, scr: impl Fn([f64; 2]) -> Pos2, pv: &TransformPreview, line: Stroke) {
    let Some(v) = cylinder_values(w) else { return };
    let dragged = match &pv.warp_drag {
        Some(WarpDrag::Cylinder { handle, .. }) => Some(*handle),
        _ => None,
    };
    for (q, h) in cylinder_handles(&v) {
        let p = scr(q);
        let fill = if dragged == Some(h) { line.color } else { Color32::WHITE };
        if matches!(h, CylinderHandle::Depth(_)) {
            painter.circle_filled(p, 3.5, fill);
            painter.circle_stroke(p, 3.5, line);
        } else {
            let r = egui::Rect::from_center_size(p, vec2(7.0, 7.0));
            painter.rect_filled(r, 0.0, fill);
            painter.rect_stroke(r, 0.0, line, egui::StrokeKind::Inside);
        }
    }
}
