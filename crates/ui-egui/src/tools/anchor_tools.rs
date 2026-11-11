//! Add Anchor Point and Delete Anchor Point tools (P group with the Pen, #1044), as in Photoshop.
//! They edit the targeted path only (#12762, #12705): the saved path selected in the Paths panel,
//! the targeted vector mask or the active shape layer's path (not while that layer is hidden), or the work
//! path. A top-level shape layer that is not active is left alone, unlike Direct Selection; one inside a group is still edited as before.
//!
//! * Add Anchor Point: hovering a segment highlights it and shows where the anchor would go; a
//!   click inserts it there (`path.addAnchor`). A curve is split without changing its shape.
//! * Delete Anchor Point: hovering an anchor highlights it; a click removes it
//!   (`path.deleteAnchor`), joining its neighbours.
//! * ⌥ swaps the two tools and ⌘/Ctrl is Direct Selection (`direct_select`) while held.
//! * The Pen does both with Auto Add/Delete on (its options bar, on by default, #4144): a click on
//!   a segment adds, a click on an anchor deletes, except an open subpath's end points; ⇧ turns it
//!   off while held. It edits the active path, or while a path is being drawn that path (#6140),
//!   and drawing carries on from its last anchor.
//!
//! Each click is one command and one history step; a click on nothing it can edit does nothing.

use egui::{Color32, Modifiers, Pos2, Stroke};
use photocraft_doc::vector::Path;
use photocraft_doc::{LayerContent, LayerId, Locks};
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
    let (r, path, hit) = crate::direct_select::hit_active(app, p)?;
    // The anchor tools ignore a click on a shape with only its own position lock set (#12730, #12825).
    if let PathRef::Layer(id) = r
        && let Some(st) = app.session.active()
        && let Some(layer) = st.doc.layer(LayerId(id))
        && matches!(layer.content, LayerContent::Shape(_))
        && layer.locks == (Locks { position: true, ..Locks::default() })
        && let Some(at) = st.doc.path_of(layer.id)
        && at.split_last().is_some_and(|(_, ancestors)| !st.doc.locks_at(ancestors).any())
    {
        return None;
    }
    match (tool, hit) {
        (Tool::AddAnchorPoint, Hit::Segment(..)) | (Tool::DeleteAnchorPoint, Hit::Anchor(_)) => Some((r, path, hit)),
        _ => None,
    }
}

/// The Pen's Auto Add/Delete (#4144): what a Pen click at `p` does to the active path instead of
/// starting a new one, as the anchor tool that does it (Add Anchor Point on a segment, Delete
/// Anchor Point on an anchor), the path and the hit. While a path is being drawn it edits that
/// path only (#6140): a click anywhere else places the next anchor. `None` when the option is off,
/// ⇧ or ⌘ is held, an anchor's handles are being pulled out, or the click hits nothing it edits.
/// With ⌥ it takes only a straight segment of the active path while no path is being drawn
/// (#12899): the press adds an anchor there and the drag pulls out its handles
/// (`pen_alt_segment_press`).
/// The end points of an open subpath are left to the Pen, as in Photoshop: on the path being
/// drawn, the first closes it and the last breaks its handle.
pub fn pen_auto(app: &PhotocraftApp, tool: Tool, mods: Modifiers, p: [f64; 2]) -> Option<(Tool, PathRef, Path, Hit)> {
    if tool != Tool::Pen || !app.ui.tool_options.pen_auto_add_delete || mods.shift || mods.command {
        return None;
    }
    let tol = 6.0 / f64::from(app.point_zoom().max(0.01));
    let at = Point::new(p[0], p[1]);
    let (r, path, hit) = match app.ui.pen.as_ref() {
        // ⌥ while drawing stays the last anchor's handle grab and break.
        Some(pen) if pen.dragging || pen.curvature || mods.alt => return None,
        Some(pen) => {
            let path = crate::vector_ui::pen_path(pen);
            let hit = edit::hit(&path, at, tol, &[])?;
            (PathRef::Pen, path, hit)
        }
        None => crate::direct_select::active_paths(app).into_iter().find_map(|(r, path)| edit::hit(&path, at, tol, &[]).map(|h| (r, path, h)))?,
    };
    let tool = match hit {
        Hit::Segment(k, _) if mods.alt && !straight(&path, k) => return None,
        Hit::Segment(..) => Tool::AddAnchorPoint,
        // ⌥ on an anchor is Convert Point (`direct_select`).
        Hit::Anchor(_) if mods.alt => return None,
        Hit::Anchor([s, k]) => {
            let sp = path.subpaths.get(s)?;
            if !sp.closed && (k == 0 || k + 1 == sp.knots.len()) {
                return None;
            }
            Tool::DeleteAnchorPoint
        }
        Hit::Handle(..) => return None,
    };
    Some((tool, r, path, hit))
}

/// ⌥/Alt-press with the Pen on a straight segment of the active path while no path is being drawn
/// (#12899), as in Photoshop: an anchor is added where the segment was pressed (`path.addAnchor`)
/// and the drag pulls out its handles, the outgoing one under the pointer and the incoming one
/// opposite it (`path.convertPoint` on release, through Direct Selection's drag). Both are one
/// History step, Add Anchor Point. False when the press is not that, so ⌥ stays Convert Point.
pub fn pen_alt_segment_press(app: &mut PhotocraftApp, mods: Modifiers, p: [f64; 2]) -> bool {
    use crate::direct_select::{Drag, Edit, Owner, active_paths};
    if !mods.alt {
        return false;
    }
    let Some((_, target, _, Hit::Segment([s, k], t))) = pen_auto(app, app.ui.tool, mods, p) else { return false };
    let key = format!("pen-alt-add-{}", app.ui.alloc_id());
    let mut params = serde_json::json!({"subpath": s, "knot": k, "t": t, "coalesce": key});
    if let (Some(o), Some(tp)) = (params.as_object_mut(), target.params(app).as_object()) {
        o.extend(tp.clone());
    }
    let added = match app.run("path.addAnchor", params) {
        Ok(v) => v,
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
            return true;
        }
    };
    let n = |f: &str| added["anchor"][f].as_u64().and_then(|v| usize::try_from(v).ok());
    let (Some(s), Some(k), Some(base)) = (n("subpath"), n("knot"), active_paths(app).into_iter().find(|(r, _)| *r == target).map(|(_, path)| path)) else {
        return true;
    };
    app.ui.direct_selection =
        DirectSelection { target: Some(target), anchors: vec![[s, k]], coalesce: Some(key), owner: Owner::current(app), ..Default::default() };
    app.ui.direct_selection.drag = Some(Drag::Edit { target, base, edit: Edit::Convert([s, k]), start: p, end: p, preview: None });
    true
}

/// Is the segment leaving knot `k` (`[subpath, knot]`) straight: both of its handles retracted?
fn straight(path: &Path, [s, k]: [usize; 2]) -> bool {
    let on = |a: Point, b: Point| (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9;
    path.subpaths.get(s).and_then(|sp| sp.segments().get(k).copied()).is_some_and(|seg| on(seg[0], seg[1]) && on(seg[2], seg[3]))
}

/// Canvas pointer events for the anchor tools, and a Pen press that Auto Add/Delete takes; false
/// when they aren't theirs.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: Modifiers) -> bool {
    if let ToolEvent::Down { x, y, .. } = ev
        && let Some((_, r, path, hit)) = pen_auto(app, app.ui.tool, mods, [x, y])
    {
        // The release finds no Pen drag to finish, so the Pen draws nothing.
        if r == PathRef::Pen {
            edit_pen(app, path, hit);
        } else {
            apply(app, r, hit);
        }
        return true;
    }
    let Some(tool) = effective(app.ui.tool, mods) else { return false };
    if let ToolEvent::Down { x, y, .. } = ev {
        click(app, tool, [x, y]);
    }
    true
}

fn click(app: &mut PhotocraftApp, tool: Tool, p: [f64; 2]) {
    let Some((r, path, hit)) = target(app, tool, p) else { return };
    if r == PathRef::Pen {
        edit_pen(app, path, hit);
        return;
    }
    apply(app, r, hit);
}

/// Add or delete an anchor of the Pen path being drawn (`path`, from `pen_path`). It isn't in the
/// document yet, so it takes the edit directly; drawing carries on from its last anchor.
fn edit_pen(app: &mut PhotocraftApp, mut path: Path, hit: Hit) {
    let edited = match hit {
        Hit::Segment(k, t) => edit::add_anchor(&mut path, k, t).is_ok(),
        Hit::Anchor(k) => edit::delete_anchor(&mut path, k).is_ok(),
        Hit::Handle(..) => false,
    };
    if edited {
        app.ui.direct_selection = DirectSelection { target: Some(PathRef::Pen), ..Default::default() };
        crate::vector_ui::pen_replace(app, &path);
    }
}

/// Add an anchor where `hit` is on a segment of `r`, or delete the anchor it is on: one command.
fn apply(app: &mut PhotocraftApp, r: PathRef, hit: Hit) {
    let (id, mut params) = match hit {
        Hit::Segment([s, k], t) => ("path.addAnchor", serde_json::json!({"subpath": s, "knot": k, "t": t})),
        Hit::Anchor([s, k]) => ("path.deleteAnchor", serde_json::json!({"subpath": s, "knot": k})),
        Hit::Handle(..) => return,
    };
    // A hit right at a segment's end is an anchor (anchors win the hit test), so `t` is inside.
    if let (Some(o), Some(t)) = (params.as_object_mut(), r.params(app).as_object()) {
        o.extend(t.clone());
    }
    match app.run(id, params) {
        // The anchors shift: a Direct Selection on this path would point at the wrong ones.
        Ok(_) if app.ui.direct_selection.target.as_ref() == Some(&r) => app.ui.direct_selection = DirectSelection { target: Some(r), ..Default::default() },
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
    draw_hit(painter, to_scr, accent, &path, hit);
}

/// The same feedback for the Pen when Auto Add/Delete would act on a click.
pub fn draw_pen_hover(app: &PhotocraftApp, painter: &egui::Painter, to_scr: &dyn Fn([f64; 2]) -> Pos2, accent: Color32, mods: Modifiers) {
    let Some(p) = app.hover_doc else { return };
    if let Some((_, _, path, hit)) = pen_auto(app, app.ui.tool, mods, p) {
        draw_hit(painter, to_scr, accent, &path, hit);
    }
}

fn draw_hit(painter: &egui::Painter, to_scr: &dyn Fn([f64; 2]) -> Pos2, accent: Color32, path: &Path, hit: Hit) {
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
mod add_anchor_lock_tests;
#[cfg(test)]
mod add_anchor_targeted_path_tests;
#[cfg(test)]
mod delete_anchor_lock_tests;
#[cfg(test)]
mod delete_anchor_targeted_path_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod add_anchor_delete_key_tests;

#[cfg(test)]
mod delete_anchor_escape_tests;
