//! On-canvas corner radius widgets for live rectangles (TOOL-214-12), as in Photoshop: with Path
//! Selection, the Rectangle tool, or the Move tool showing its transform controls, every selected
//! shape layer that is a live rectangle shows a round widget inside each corner, at the centre of
//! its arc. Dragging one rounds every corner of every selected rectangle to the same radius;
//! ⌥/Alt changes that corner only, on each of them. The drag previews the new outlines and
//! commits one `shape.edit` with `radii` per changed layer on release, as one history step; a
//! click without movement changes nothing.

use egui::{Color32, Modifiers, Pos2, Stroke};
use photocraft_doc::vector::LiveShape;
use photocraft_doc::{DocId, LayerContent, LayerId};

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

/// A widget sits at least this many screen points inside its corner, so a sharp corner's widget
/// can still be grabbed.
const MIN_INSET: f64 = 10.0;

/// A selected live rectangle: its layer, `[x, y, w, h]` with a positive size, and its radii.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    layer: u64,
    rect: [f64; 4],
    radii: [f64; 4],
}

/// A corner radius drag in progress.
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    doc: DocId,
    revision: u64,
    tool: Tool,
    selection: Vec<LayerId>,
    /// Every selected live rectangle, as at the press.
    targets: Vec<Target>,
    /// The rectangle whose widget was pressed (an index into `targets`) and its corner,
    /// `[tl, tr, br, bl]` order.
    grabbed: usize,
    corner: usize,
    /// Every corner (no ⌥) or the dragged one only.
    all: bool,
    /// Where the press was.
    start: [f64; 2],
    /// Each target's radii so far.
    now: Vec<[f64; 4]>,
}

/// Do the widgets show with the current tool?
fn shows(app: &PhotocraftApp) -> bool {
    let tool = app.active_tool();
    app.ui.transform.is_none()
        && !app.ui.mask_target
        && !app.ui.vector_mask_target
        && (!crate::vector_ui::is_shape_tool(tool) || app.ui.tool_options.shape.mode == "shape")
        && match tool {
            Tool::PathSelection | Tool::Rectangle => true,
            Tool::Move => app.ui.tool_options.move_show_transform,
            _ => false,
        }
}

/// Is this radius system active for the canvas tool?
pub(crate) fn supports_tool(app: &PhotocraftApp, tool: Tool) -> bool {
    app.active_tool() == tool && shows(app)
}

pub(crate) fn dragging(app: &PhotocraftApp) -> bool {
    app.ui.corner_radius.is_some()
}

/// Drop a preview when its document, selection, tool or edit context changes.
pub(crate) fn cancel_stale(app: &mut PhotocraftApp) -> bool {
    let stale = app.ui.corner_radius.as_ref().is_some_and(|d| {
        app.session.active().is_none_or(|st| {
            st.doc.id != d.doc
                || st.revision != d.revision
                || app.active_tool() != d.tool
                || st.selected_layers() != d.selection
                || !shows(app)
                || targets(app).iter().map(|t| t.layer).collect::<Vec<_>>() != d.targets.iter().map(|t| t.layer).collect::<Vec<_>>()
        })
    });
    if stale {
        app.ui.corner_radius = None;
    }
    stale
}

/// A temporary navigation tool owns the pointer and cancels any radius preview immediately.
pub(crate) fn end_if_navigation(app: &mut PhotocraftApp, tool: Tool) -> bool {
    matches!(tool, Tool::Hand | Tool::Zoom | Tool::RotateView) && app.ui.corner_radius.take().is_some()
}

/// The selected layers that are live rectangles, front first.
fn targets(app: &PhotocraftApp) -> Vec<Target> {
    if !shows(app) {
        return Vec::new();
    }
    let Some(st) = app.session.active() else { return Vec::new() };
    let mut out: Vec<Target> = st
        .selected_layers()
        .into_iter()
        .filter_map(|id| {
            let l = st.doc.layer(id)?;
            let locks = st.doc.effective_locks(id);
            if locks.all || locks.pixels || locks.position {
                return None;
            }
            let LayerContent::Shape(sh) = &l.content else { return None };
            let Some(LiveShape::Rect { rect: [x, y, w, h], radii }) = sh.live else { return None };
            let (x, w) = if w < 0.0 { (x + w, -w) } else { (x, w) };
            let (y, h) = if h < 0.0 { (y + h, -h) } else { (y, h) };
            (w > 0.0 && h > 0.0 && [x, y, w, h].iter().chain(&radii).all(|v| v.is_finite())).then_some(Target { layer: l.id.0, rect: [x, y, w, h], radii })
        })
        .collect();
    // `selected_layers` lists bottom to top; the topmost widget wins a press.
    out.reverse();
    out
}

/// The largest radius a corner of `rect` takes.
fn max_radius(rect: [f64; 4]) -> f64 {
    rect[2].min(rect[3]) / 2.0
}

/// Corner `i` of `rect` and the unit-step direction pointing inside from it.
fn corner(rect: [f64; 4], i: usize) -> ([f64; 2], [f64; 2]) {
    let [x, y, w, h] = rect;
    match i {
        0 => ([x, y], [1.0, 1.0]),
        1 => ([x + w, y], [-1.0, 1.0]),
        2 => ([x + w, y + h], [-1.0, -1.0]),
        _ => ([x, y + h], [1.0, -1.0]),
    }
}

/// Where corner `i`'s widget is drawn: the centre of its arc, or a few screen points inside a
/// sharper corner.
fn widget(rect: [f64; 4], radii: [f64; 4], i: usize, zoom: f64) -> [f64; 2] {
    let (c, d) = corner(rect, i);
    let r = radii.get(i).copied().unwrap_or(0.0).max(MIN_INSET / zoom).min(max_radius(rect));
    [c[0] + d[0] * r, c[1] + d[1] * r]
}

/// The radius the pressed corner `i` asks for when the pointer has moved from `start` to `p`: its
/// radius at the press plus the move projected onto the corner's diagonal (inward rounds more),
/// at least 0. Relative, so a click on a sharp corner's inset widget changes nothing. Each
/// rectangle then caps it at half its own shorter side.
fn radius_after(rect: [f64; 4], i: usize, from: f64, start: [f64; 2], p: [f64; 2]) -> f64 {
    let (_, d) = corner(rect, i);
    let r = from + ((p[0] - start[0]) * d[0] + (p[1] - start[1]) * d[1]) / 2.0;
    if r.is_finite() { r.max(0.0) } else { from }
}

fn zoom(app: &PhotocraftApp) -> f64 {
    f64::from(app.point_zoom().max(0.01))
}

/// The widget of `targets` within grabbing distance of document point `p`: the rectangle (an
/// index into `targets`) and its corner, the nearest when several are close.
fn hit(app: &PhotocraftApp, targets: &[Target], p: [f64; 2]) -> Option<(usize, usize)> {
    let z = zoom(app);
    let aspect = f64::from(app.ui.view.display_aspect());
    targets
        .iter()
        .enumerate()
        .flat_map(|(t, g)| {
            (0..4).map(move |i| {
                let w = widget(g.rect, g.radii, i, z);
                (t, i, ((w[0] - p[0]) * z * aspect).hypot((w[1] - p[1]) * z))
            })
        })
        .filter(|(.., d)| *d <= 7.0)
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(t, i, _)| (t, i))
}

/// Is a widget under document point `p`, or one being dragged? The canvas then shows the corner
/// radius cursor.
pub fn hovers(app: &PhotocraftApp, p: [f64; 2]) -> bool {
    app.ui.corner_radius.is_some() || hit(app, &targets(app), p).is_some()
}

pub(crate) fn cursor(app: &PhotocraftApp, p: [f64; 2]) -> Option<egui::CursorIcon> {
    hovers(app, p).then_some(egui::CursorIcon::PointingHand)
}

/// Canvas pointer events for the corner widgets; false when the event is not theirs.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: Modifiers) -> bool {
    if cancel_stale(app) {
        return !matches!(ev, ToolEvent::Down { .. });
    }
    match ev {
        ToolEvent::Down { x, y, .. } => {
            if mods.command {
                return false;
            }
            let targets = targets(app);
            let Some((grabbed, corner)) = hit(app, &targets, [x, y]) else { return false };
            let Some(st) = app.session.active() else { return false };
            let now = targets.iter().map(|t| t.radii).collect();
            app.ui.corner_radius = Some(Drag {
                doc: st.doc.id,
                revision: st.revision,
                tool: app.active_tool(),
                selection: st.selected_layers(),
                targets,
                grabbed,
                corner,
                all: !mods.alt,
                start: [x, y],
                now,
            });
            // The press may have started the Move tool's snapping, which a radius drag has none of.
            app.prefs_rt.snap = None;
            app.prefs_rt.snap_lines.clear();
            app.move_mods = Default::default();
            true
        }
        ToolEvent::Move { x, y, .. } => {
            let z = zoom(app);
            let aspect = f64::from(app.ui.view.display_aspect());
            let Some(d) = app.ui.corner_radius.as_mut() else { return false };
            let screen_dx = (x - d.start[0]) * z * aspect;
            let screen_dy = (y - d.start[1]) * z;
            if screen_dx.hypot(screen_dy) < 0.5 {
                for (target, now) in d.targets.iter().zip(&mut d.now) {
                    *now = target.radii;
                }
                return true;
            }
            let Some(g) = d.targets.get(d.grabbed) else { return true };
            let from = g.radii.get(d.corner).copied().unwrap_or(0.0).clamp(0.0, max_radius(g.rect));
            let r = radius_after(g.rect, d.corner, from, d.start, [x, y]);
            // Every selected rectangle takes the same radius, within its own size.
            for (t, now) in d.targets.iter().zip(d.now.iter_mut()) {
                let r = r.min(max_radius(t.rect));
                if d.all {
                    *now = [r; 4];
                } else {
                    *now = t.radii;
                    if let Some(v) = now.get_mut(d.corner) {
                        *v = r;
                    }
                }
            }
            true
        }
        ToolEvent::Up { x, y } => {
            if app.ui.corner_radius.is_none() {
                return false;
            }
            pointer(app, ToolEvent::Move { x, y, pressure: 1.0 }, mods);
            let Some(d) = app.ui.corner_radius.take() else { return true };
            // One history step for every layer changed.
            let key = format!("corner-radius-{}", app.ui.alloc_id());
            for (t, now) in d.targets.iter().zip(&d.now) {
                if *now != t.radii
                    && let Err(e) = app.run("shape.edit", serde_json::json!({"layer": t.layer, "radii": now, "coalesce": key}))
                {
                    app.ui.status = e;
                    app.ui.status_error = true;
                    break;
                }
            }
            true
        }
    }
}

/// Escape cancels the preview without changing document history.
pub(crate) fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    cancel_stale(app);
    if app.ui.corner_radius.is_some() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, egui::Key::Escape)) {
        app.ui.corner_radius = None;
        return true;
    }
    false
}

/// The radius the dragged corner shows in the readout.
fn dragged_radius(d: &Drag) -> Option<f64> {
    d.now.get(d.grabbed).and_then(|r| r.get(d.corner)).copied()
}

/// The widgets of every selected live rectangle; while one is dragged, the new outlines and the
/// radius beside the pointer (`R: 25.0 px`), as in Photoshop.
pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, to_scr: &dyn Fn([f64; 2]) -> Pos2, accent: Color32) {
    let drag = app.ui.corner_radius.as_ref();
    if let Some(r) = drag.and_then(dragged_radius)
        && let Some(at) = painter.ctx().input(|i| i.pointer.hover_pos())
    {
        crate::canvas::draw_value_readout(app, painter.ctx(), "corner-radius-readout", at, ["R:"], [format!("{r:.1} px")]);
    }
    let z = zoom(app);
    for t in targets(app) {
        let now = drag.and_then(|d| d.targets.iter().position(|g| g.layer == t.layer).and_then(|i| d.now.get(i)));
        let radii = now.copied().unwrap_or(t.radii);
        if now.is_some() {
            let path = photocraft_vector::shapes::rounded_rect(t.rect[0], t.rect[1], t.rect[2], t.rect[3], radii);
            crate::vector_ui::draw_outline(painter, &path, to_scr, accent, crate::shape_modes::path_display(app, painter.ctx()).1);
        }
        for i in 0..4 {
            let p = to_scr(widget(t.rect, radii, i, z));
            painter.circle(p, 5.0, Color32::WHITE, Stroke::new(1.5, accent));
            painter.circle_filled(p, 2.5, accent);
        }
    }
}

#[cfg(test)]
#[path = "corner_radius_tests.rs"]
mod tests;
