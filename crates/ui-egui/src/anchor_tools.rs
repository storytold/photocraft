//! Add Anchor Point and Delete Anchor Point tools (P group with the Pen, #1044), as in Photoshop.
//! They edit the paths Direct Selection edits, front first: the targeted vector mask or the active
//! shape layer's path, then the work path.
//!
//! * Add Anchor Point: hovering a segment highlights it and shows where the anchor would go; a
//!   click inserts it there (`path.addAnchor`). A curve is split without changing its shape.
//! * Delete Anchor Point: hovering an anchor highlights it; a click removes it
//!   (`path.deleteAnchor`), joining its neighbours.
//! * ⌥ swaps the two tools and ⌘/Ctrl is Direct Selection (`direct_select`) while held.
//!
//! Each click is one command and one history step; a click on nothing it can edit does nothing.

use egui::{Color32, Modifiers, Pos2, Stroke};
use photocraft_doc::vector::Path;
use photocraft_geom::Point;
use photocraft_vector::edit::{self, Hit};

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::direct_select::{DirectSelection, PathRef};
use crate::state::Tool;

/// The anchor tool in effect, if the current tool is one: ⌥ swaps Add and Delete.
pub fn effective(tool: Tool, mods: Modifiers) -> Option<Tool> {
    let (add, delete) = (Tool::AddAnchorPoint, Tool::DeleteAnchorPoint);
    match tool {
        Tool::AddAnchorPoint => Some(if mods.alt { delete } else { add }),
        Tool::DeleteAnchorPoint => Some(if mods.alt { add } else { delete }),
        _ => None,
    }
}

/// What a click at `p` would edit with `tool`: the frontmost path under the pointer and the
/// segment (Add) or anchor (Delete) hit on it. An anchor hit with Add, or a segment with Delete,
/// edits nothing.
fn target(app: &PhotocraftApp, tool: Tool, p: [f64; 2]) -> Option<(PathRef, Path, Hit)> {
    let tol = 6.0 / f64::from(app.point_zoom().max(0.01));
    let (r, path, hit) =
        crate::direct_select::candidates(app).into_iter().find_map(|(r, path)| edit::hit(&path, Point::new(p[0], p[1]), tol, &[]).map(|h| (r, path, h)))?;
    match (tool, hit) {
        (Tool::AddAnchorPoint, Hit::Segment(..)) | (Tool::DeleteAnchorPoint, Hit::Anchor(_)) => Some((r, path, hit)),
        _ => None,
    }
}

/// Canvas pointer events for the anchor tools; false when the current tool isn't one.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: Modifiers) -> bool {
    let Some(tool) = effective(app.ui.tool, mods) else { return false };
    if let ToolEvent::Down { x, y, .. } = ev {
        click(app, tool, [x, y]);
    }
    true
}

fn click(app: &mut PhotocraftApp, tool: Tool, p: [f64; 2]) {
    let Some((r, _, hit)) = target(app, tool, p) else { return };
    let (id, mut params) = match hit {
        Hit::Segment([s, k], t) => ("path.addAnchor", serde_json::json!({"subpath": s, "knot": k, "t": t})),
        Hit::Anchor([s, k]) => ("path.deleteAnchor", serde_json::json!({"subpath": s, "knot": k})),
        Hit::Handle(..) => return,
    };
    // A hit right at a segment's end is an anchor (anchors win the hit test), so `t` is inside.
    if let (Some(o), Some(t)) = (params.as_object_mut(), r.params().as_object()) {
        o.extend(t.clone());
    }
    match app.run(id, params) {
        // The anchors shift: a Direct Selection on this path would point at the wrong ones.
        Ok(_) if app.ui.direct_selection.target == Some(r) => app.ui.direct_selection = DirectSelection { target: Some(r), ..Default::default() },
        Ok(_) => {}
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// The hover feedback over the Direct Selection overlay: Add highlights the segment under the
/// pointer and shows the anchor a click would add; Delete highlights the anchor it would remove.
pub fn draw_hover(app: &PhotocraftApp, painter: &egui::Painter, to_scr: &dyn Fn([f64; 2]) -> Pos2, accent: Color32, mods: Modifiers) {
    let (Some(tool), Some(p)) = (effective(app.ui.tool, mods), app.hover_doc) else { return };
    if mods.command {
        return;
    }
    let Some((_, path, hit)) = target(app, tool, p) else { return };
    match hit {
        Hit::Segment([s, k], t) => {
            let Some(seg) = path.subpaths.get(s).and_then(|sp| sp.segments().get(k).copied()) else { return };
            let pts: Vec<Pos2> = (0..=48).map(|i| edit::eval(&seg, f64::from(i) / 48.0)).map(|q| to_scr([q.x, q.y])).collect();
            painter.add(egui::Shape::line(pts, Stroke::new(2.5, accent)));
            let q = edit::eval(&seg, t);
            crate::vector_ui::draw_anchor(painter, to_scr([q.x, q.y]), false, accent);
        }
        Hit::Anchor([s, k]) => {
            let Some(kn) = path.subpaths.get(s).and_then(|sp| sp.knots.get(k)) else { return };
            let c = to_scr([kn.anchor.x, kn.anchor.y]);
            painter.rect_stroke(egui::Rect::from_center_size(c, egui::vec2(11.0, 11.0)), 0.0, Stroke::new(1.5, accent), egui::StrokeKind::Inside);
            crate::vector_ui::draw_anchor(painter, c, true, accent);
        }
        Hit::Handle(..) => {}
    }
}

#[cfg(test)]
#[path = "anchor_tools_tests.rs"]
mod tests;
