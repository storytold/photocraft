//! Edit › Perspective Warp on the canvas.
//!
//! **Layout** mode: drag on the image to draw a plane, drag its corners onto the image's
//! perspective lines; a corner dropped near another plane's corner snaps to it, which links the
//! planes along that edge. **Warp** mode: drag corners (linked corners move together) and the
//! image follows; the options bar straightens near-vertical / near-horizontal edges. ↩ / Commit
//! runs `edit.perspectiveWarp` with the planes. The preview is the layer texture on a 40×40 mesh
//! pushed through the same continuous multi-plane map the engine uses.

use std::sync::Arc;

use egui::{Color32, Pos2, Stroke, TextureHandle, pos2};
use photocraft_algo::perspective::{PerspectiveMap, Plane, Straighten, linked_corners, linked_edge_count, plane_point, straighten, weld_adjacent_edges};
use photocraft_doc::{DocId, Document, LayerId};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PerspMode {
    Layout,
    Warp,
}

enum Drag {
    NewPlane {
        start: [f64; 2],
        cur: [f64; 2],
    },
    /// Linked corners being moved, and the pointer offset from the corner.
    Corner {
        group: Vec<(usize, usize)>,
        offset: [f64; 2],
    },
}

pub struct PerspSession {
    pub key: u64,
    /// Document this session belongs to (#5115).
    pub document: DocId,
    pub layer: LayerId,
    command: String,
    bounds: Rect,
    pub planes: Vec<Plane>,
    pub mode: PerspMode,
    drag: Option<Drag>,
    texture: TextureHandle,
    opacity: f32,
    pub preview_doc: Arc<Document>,
    /// Preview mesh: screen-independent destination points for a 128×128 grid over `bounds`.
    grid: Vec<[f64; 2]>,
}

const GRID: usize = 128;

impl PerspSession {
    pub fn describe(&self) -> Value {
        json!({"document": self.document.0, "layer": self.layer.0, "mode": self.mode, "planes": self.planes, "linkedEdges": linked_edge_count(&self.planes)})
    }

    fn update_grid(&mut self) {
        let b = self.bounds;
        let map = if self.mode == PerspMode::Warp { PerspectiveMap::new(&self.planes) } else { None };
        self.grid.clear();
        for j in 0..=GRID {
            for i in 0..=GRID {
                let x = f64::from(b.x0) + f64::from(b.width()) * i as f64 / GRID as f64;
                let y = f64::from(b.y0) + f64::from(b.height()) * j as f64 / GRID as f64;
                let (u, v) = map.as_ref().map_or((x, y), |m| m.map(x, y));
                self.grid.push([u, v]);
            }
        }
    }

    fn corners(&self, p: usize) -> &[[f64; 2]; 4] {
        if self.mode == PerspMode::Layout { &self.planes[p].src } else { &self.planes[p].dst }
    }

    fn corner_at(&self, p: [f64; 2], tol: f64) -> Option<(usize, usize)> {
        let mut best = None;
        let mut bd = tol;
        for pi in 0..self.planes.len() {
            for (ci, c) in self.corners(pi).iter().enumerate() {
                let d = (c[0] - p[0]).hypot(c[1] - p[1]);
                if d <= bd {
                    bd = d;
                    best = Some((pi, ci));
                }
            }
        }
        best
    }

    fn snap(&self, p: [f64; 2], tol: f64, skip: &[(usize, usize)]) -> [f64; 2] {
        for (pi, pl) in self.planes.iter().enumerate() {
            for (ci, c) in pl.src.iter().enumerate() {
                if !skip.contains(&(pi, ci)) && (c[0] - p[0]).hypot(c[1] - p[1]) <= tol {
                    return *c;
                }
            }
        }
        p
    }

    fn set_corner(&mut self, group: &[(usize, usize)], p: [f64; 2]) {
        let layout = self.mode == PerspMode::Layout;
        for &(pi, ci) in group {
            let Some(pl) = self.planes.get_mut(pi) else { continue };
            if layout && let Some(c) = pl.src.get_mut(ci) {
                *c = p;
            }
            if let Some(c) = pl.dst.get_mut(ci) {
                *c = p;
            }
        }
    }

    /// Mouse edits are an atomic geometry transaction. An invalid candidate must never
    /// make the preview silently fall back to the unwarped (identity) grid.
    fn set_corner_if_valid(&mut self, group: &[(usize, usize)], p: [f64; 2]) -> bool {
        let previous = self.planes.clone();
        self.set_corner(group, p);
        if validate_planes(&self.planes).is_err() {
            self.planes = previous;
            false
        } else {
            true
        }
    }
}

/// A preview belongs to the document that supplied its pixels, not necessarily the active tab.
pub(crate) fn bound_to_active_document(app: &PhotocraftApp, s: &PerspSession) -> bool {
    app.session.active().is_some_and(|state| state.doc.id == s.preview_doc.id)
}

pub fn begin(app: &mut PhotocraftApp, ctx: &egui::Context, command: &str) -> Result<(), String> {
    if let Some(why) = app.session.disabled_reason(command) {
        return Err(why);
    }
    if app.ui.transform.is_some() {
        return Err("finish Free Transform first".into());
    }
    let (layer, surface, _) = crate::distort_ui::active_pixels(app)?;
    let st = app.session.active().ok_or("no document")?;
    let l = st.doc.layer(layer).ok_or("no layer")?;
    let bounds = surface.content_bounds();
    if bounds.is_empty() {
        return Err("Could not use Perspective Warp: the layer is empty".into());
    }
    let opacity = l.opacity * l.fill_opacity;
    let preview_doc = crate::distort_ui::without_layer(&st.doc, layer);
    let key = app.ui.alloc_id();
    let texture = ctx.load_texture(format!("persp-{key}"), crate::distort_ui::surface_image(&surface, bounds, 2048), egui::TextureOptions::LINEAR);
    let mut s = PerspSession {
        key,
        document: st.doc.id,
        layer,
        command: command.to_string(),
        bounds,
        planes: Vec::new(),
        mode: PerspMode::Layout,
        drag: None,
        texture,
        opacity,
        preview_doc,
        grid: Vec::new(),
    };
    s.update_grid();
    app.distort.perspective = Some(s);
    app.ui.status = tl!("Perspective Warp: draw planes along the image's perspective, then switch to Warp").into();
    Ok(())
}

/// Apply the pending perspective transaction. A command error must not be reported as a
/// successful commit to an agent, and the editable preview survives failure.
pub fn commit(app: &mut PhotocraftApp) -> Result<Value, String> {
    if app.distort.perspective.as_ref().is_some_and(|s| !bound_to_active_document(app, s)) {
        let error = tl!("Perspective Warp preview belongs to another document").to_string();
        app.ui.status = error.clone();
        return Err(error);
    }
    let Some(s) = app.distort.perspective.take() else {
        return Err(tl!("Perspective Warp is not active").into());
    };
    if s.planes.is_empty() {
        return Ok(json!({"committed": false, "reason": "noPlanes"}));
    }
    if let Err(error) = validate_planes(&s.planes) {
        app.ui.status = error.clone();
        app.distort.perspective = Some(s);
        return Err(error);
    }
    if PerspectiveMap::is_identity(&s.planes) {
        return Ok(json!({"committed": false, "reason": "identity"}));
    }
    match app.run(&s.command, json!({"planes": s.planes.clone(), "layer": s.layer.0})) {
        Ok(result) => Ok(json!({"committed": true, "result": result})),
        Err(error) => {
            app.ui.status = crate::i18n::fmt(tl!("Perspective Warp: {error}"), &[("error", &error)]);
            app.distort.perspective = Some(s);
            Err(error)
        }
    }
}

/// Validate an agent-supplied preview before mutating the UI state.
fn validate_planes(planes: &[Plane]) -> Result<(), String> {
    if planes.iter().flat_map(|p| p.src.iter().chain(p.dst.iter())).any(|point| point.iter().any(|v| !v.is_finite())) {
        return Err(tl!("plane coordinates must be finite").into());
    }
    if !planes.is_empty() && PerspectiveMap::new(planes).is_none() {
        return Err(tl!("a plane is degenerate (its corners must form a quad)").into());
    }
    Ok(())
}

fn set_mode(s: &mut PerspSession, m: PerspMode) {
    s.mode = m;
    if m == PerspMode::Warp {
        // Linked corners share one position from here on.
        photocraft_algo::perspective::unify(&mut s.planes);
    }
    s.update_grid();
}

/// Preserve the last valid layout and its preview when straighten would collapse a quad.
fn straighten_if_valid(s: &mut PerspSession, mode: Straighten) -> Result<(), String> {
    let previous = s.planes.clone();
    straighten(&mut s.planes, mode);
    if let Err(error) = validate_planes(&s.planes) {
        s.planes = previous;
        return Err(error);
    }
    s.update_grid();
    Ok(())
}

/// Control channel: `edit.perspectiveWarp {"ui": {...}}` while the mode is active.
pub fn control(app: &mut PhotocraftApp, ui: &Value) -> Result<Value, String> {
    if ui.get("commit").and_then(Value::as_bool) == Some(true) {
        return commit(app);
    }
    if ui.get("cancel").and_then(Value::as_bool) == Some(true) {
        app.distort.perspective = None;
        return Ok(json!({"cancelled": true}));
    }
    // A stale preview belongs to its original document even if an agent switches tabs.
    if app.distort.perspective.as_ref().is_some_and(|s| !bound_to_active_document(app, s)) {
        return Err(tl!("Perspective Warp preview belongs to another document").into());
    }
    // Parse every requested field before changing the session. A malformed request should
    // never partly switch modes, discard a corner drag or silently produce an identity preview.
    let requested_mode = match ui.get("mode") {
        None => None,
        Some(Value::String(mode)) => Some(match mode.as_str() {
            "layout" => PerspMode::Layout,
            "warp" => PerspMode::Warp,
            _ => return Err("mode must be layout|warp".into()),
        }),
        Some(_) => return Err("mode must be layout|warp".into()),
    };
    let replacement: Option<Vec<Plane>> = ui.get("planes").map(|p| serde_json::from_value(p.clone()).map_err(|e| format!("bad planes: {e}"))).transpose()?;
    let requested_straighten = match ui.get("straighten") {
        None => None,
        Some(Value::String(st)) => Some(Straighten::parse(st).ok_or("straighten: horizontal|vertical|auto")?),
        Some(_) => return Err("straighten: horizontal|vertical|auto".into()),
    };
    let requested_weld = match ui.get("linkEdges") {
        None | Some(Value::Bool(false)) => None,
        Some(Value::Bool(true)) => Some(8.0),
        Some(Value::Object(obj)) => {
            let tolerance = obj.get("tolerance").and_then(Value::as_f64).ok_or("linkEdges tolerance must be a number in 0..32")?;
            if !tolerance.is_finite() || !(0.0..=32.0).contains(&tolerance) {
                return Err("linkEdges tolerance must be finite and within 0..32 document pixels".into());
            }
            Some(tolerance)
        }
        Some(_) => return Err("linkEdges: true|false|{\"tolerance\":0..32}".into()),
    };
    let s = app.distort.perspective.as_mut().ok_or(tl!("Perspective Warp is not active"))?;
    let new_mode = requested_mode.unwrap_or(s.mode);
    if requested_weld.is_some() && new_mode != PerspMode::Layout {
        return Err("linkEdges is available only in Layout mode".into());
    }
    let replaced_planes = replacement.is_some();
    let mut new_planes = replacement.unwrap_or_else(|| s.planes.clone());
    let welded_edges = match requested_weld {
        Some(tolerance) => weld_adjacent_edges(&mut new_planes, tolerance).map_err(str::to_string)?,
        None => 0,
    };
    if new_mode == PerspMode::Warp && (requested_mode == Some(PerspMode::Warp) || replaced_planes) {
        photocraft_algo::perspective::unify(&mut new_planes);
    }
    if let Some(straighten_mode) = requested_straighten {
        straighten(&mut new_planes, straighten_mode);
    }
    validate_planes(&new_planes)?;
    s.mode = new_mode;
    s.planes = new_planes;
    if replaced_planes || requested_mode.is_some() || requested_weld.is_some() {
        // A corner drag holds indices into the previous plane list (#952).
        s.drag = None;
    }
    s.update_grid();
    let mut reply = s.describe();
    if requested_weld.is_some() {
        reply["weldedEdges"] = json!(welded_edges);
    }
    Ok(reply)
}

pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, _mods: egui::Modifiers) {
    let tol = crate::distort_ui::tolerance(app);
    let Some(s) = app.distort.perspective.as_mut() else { return };
    let mut rejected = false;
    match ev {
        ToolEvent::Down { x, y, .. } => {
            let p = [x, y];
            if let Some((pi, ci)) = s.corner_at(p, tol) {
                let c = s.corners(pi)[ci];
                // Move every corner linked to this one.
                let group = linked_corners(&s.planes).into_iter().find(|g| g.contains(&(pi, ci))).unwrap_or_else(|| vec![(pi, ci)]);
                s.drag = Some(Drag::Corner { group, offset: [c[0] - x, c[1] - y] });
            } else if s.mode == PerspMode::Layout {
                let start = s.snap(p, tol, &[]);
                s.drag = Some(Drag::NewPlane { start, cur: p });
            }
        }
        ToolEvent::Move { x, y, .. } => match &mut s.drag {
            Some(Drag::NewPlane { cur, .. }) => *cur = [x, y],
            Some(Drag::Corner { group, offset }) => {
                let group = group.clone();
                let mut q = [x + offset[0], y + offset[1]];
                if s.mode == PerspMode::Layout {
                    q = s.snap(q, tol, &group);
                }
                if s.set_corner_if_valid(&group, q) {
                    if s.mode == PerspMode::Warp {
                        s.update_grid();
                    }
                } else {
                    rejected = true;
                }
            }
            None => {}
        },
        ToolEvent::Up { x, y } => {
            match s.drag.take() {
                Some(Drag::NewPlane { start, .. }) => {
                    let end = s.snap([x, y], tol, &[]);
                    let (x0, y0, x1, y1) = (start[0].min(end[0]), start[1].min(end[1]), start[0].max(end[0]), start[1].max(end[1]));
                    if x1 - x0 > 4.0 && y1 - y0 > 4.0 {
                        // Keep the snapped corners exact.
                        let mut q = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
                        for c in q.iter_mut() {
                            *c = s.snap(*c, tol, &[]);
                        }
                        // Snapping may collapse multiple corners onto a single source point.
                        // Validate the whole candidate list before replacing the active layout.
                        let mut candidate = s.planes.clone();
                        candidate.push(Plane::identity(q));
                        if validate_planes(&candidate).is_ok() {
                            s.planes = candidate;
                        } else {
                            rejected = true;
                        }
                    }
                }
                Some(Drag::Corner { group, offset }) => {
                    let mut q = [x + offset[0], y + offset[1]];
                    if s.mode == PerspMode::Layout {
                        q = s.snap(q, tol, &group);
                    }
                    if !s.set_corner_if_valid(&group, q) {
                        rejected = true;
                    }
                }
                None => {}
            }
            s.update_grid();
        }
    }
    if rejected {
        app.ui.status = tl!("Perspective Warp: invalid plane; drag ignored").into();
    }
}

/// Shared by the checkmark and Enter: never drop an interactive commit error.
pub fn commit_from_ui(app: &mut PhotocraftApp) {
    match commit(app) {
        Err(error) => {
            app.ui.status = crate::i18n::fmt(tl!("Perspective Warp: {error}"), &[("error", &error)]);
        }
        Ok(result) if result["committed"] == false => {
            app.ui.status = tl!("Perspective Warp: no changes to apply").into();
        }
        Ok(_) => {}
    }
}

/// The warped pixels: image content, drawn under the grid and guides like the document so a
/// guide stays visible across the layer being warped (#7781). The planes and pins are [`draw`].
pub fn draw_content(s: &PerspSession, painter: &egui::Painter, xf: &ViewXform) {
    let scr = |p: [f64; 2]| xf.to_screen(p[0] as f32, p[1] as f32);
    let mut mesh = egui::Mesh::with_texture(s.texture.id());
    let tint = Color32::from_white_alpha((s.opacity.clamp(0.0, 1.0) * 255.0) as u8);
    for j in 0..=GRID {
        for i in 0..=GRID {
            let p = s.grid[j * (GRID + 1) + i];
            mesh.vertices.push(egui::epaint::Vertex { pos: scr(p), uv: pos2(i as f32 / GRID as f32, j as f32 / GRID as f32), color: tint });
        }
    }
    for j in 0..GRID {
        for i in 0..GRID {
            let a = (j * (GRID + 1) + i) as u32;
            mesh.add_triangle(a, a + 1, a + GRID as u32 + 2);
            mesh.add_triangle(a, a + GRID as u32 + 2, a + GRID as u32 + 1);
        }
    }
    painter.add(mesh);
}

/// The planes, their grids and pins over the warped pixels ([`draw_content`]) and the guides.
pub fn draw(s: &PerspSession, painter: &egui::Painter, xf: &ViewXform) {
    let scr = |p: [f64; 2]| xf.to_screen(p[0] as f32, p[1] as f32);
    let accent = crate::theme::Tokens::get(painter.ctx()).accent;
    let warp = s.mode == PerspMode::Warp;
    for p in &s.planes {
        // Plane grid at thirds (like Photoshop's), then the outline.
        let thin = Stroke::new(0.75, Color32::from_rgba_unmultiplied(255, 255, 255, 150));
        for k in 1..3 {
            let t = k as f64 / 3.0;
            painter.line_segment([scr(plane_point(p, t, 0.0, warp)), scr(plane_point(p, t, 1.0, warp))], thin);
            painter.line_segment([scr(plane_point(p, 0.0, t, warp)), scr(plane_point(p, 1.0, t, warp))], thin);
        }
        let q = if warp { &p.dst } else { &p.src };
        let pts: Vec<Pos2> = q.iter().map(|c| scr(*c)).collect();
        painter.add(egui::Shape::closed_line(pts.clone(), Stroke::new(1.5, accent)));
        for c in pts {
            painter.circle_filled(c, 4.5, Color32::WHITE);
            painter.circle_stroke(c, 4.5, Stroke::new(1.25, accent));
        }
    }
    if let Some(Drag::NewPlane { start, cur }) = &s.drag {
        let r = egui::Rect::from_two_pos(scr(*start), scr(*cur));
        painter.rect_stroke(r, 0.0, Stroke::new(1.0, accent), egui::StrokeKind::Inside);
    }
}

pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(s) = app.distort.perspective.as_mut() else { return };
    if crate::widgets::pill_tab(ui, "Layout", s.mode == PerspMode::Layout).clicked() {
        set_mode(s, PerspMode::Layout);
    }
    if crate::widgets::pill_tab(ui, "Warp", s.mode == PerspMode::Warp).clicked() && !s.planes.is_empty() {
        set_mode(s, PerspMode::Warp);
    }
    crate::widgets::vline(ui, 22.0);
    if s.mode == PerspMode::Warp {
        for (label, tip, m) in [
            (tl!("Straighten"), tl!("Automatically straighten near-vertical line segments"), Straighten::Vertical),
            (tl!("Level"), tl!("Automatically level near-horizontal line segments"), Straighten::Horizontal),
            (tl!("Both"), tl!("Automatically straighten and level"), Straighten::Auto),
        ] {
            if ui.button(tl!(&label)).on_hover_text(tip).clicked()
                && let Err(error) = straighten_if_valid(s, m)
            {
                app.ui.status = crate::i18n::fmt(tl!("Perspective Warp: {error}"), &[("error", &error)]);
            }
        }
    } else {
        ui.label(crate::i18n::trn(crate::i18n::current(), s.planes.len() as u64, "{n} plane", "{n} planes"));
        if ui.button(tl!("Link Edges")).on_hover_text(tl!("Join matching neighbouring edges within 8 document pixels; preserves other planes")).clicked() {
            match weld_adjacent_edges(&mut s.planes, 8.0) {
                Ok(count) => {
                    s.drag = None;
                    s.update_grid();
                    let count = count.to_string();
                    app.ui.status = crate::i18n::fmt(tl!("Edges linked: {count}"), &[("count", &count)]);
                }
                Err(error) => app.ui.status = crate::i18n::fmt(tl!("Perspective Warp: {error}"), &[("error", error)]),
            }
        }
        let linked = linked_edge_count(&s.planes).to_string();
        ui.label(crate::i18n::fmt(tl!("Edges linked: {count}"), &[("count", &linked)]));
        if ui.button(tl!("Remove Planes")).clicked() {
            s.planes.clear();
            s.update_grid();
        }
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if crate::widgets::primary_button(ui, "✓", 32.0)
            .on_hover_text(crate::i18n::fmt(tl!("Commit Perspective Warp ({key})"), &[("key", &crate::shortcuts::pretty("Enter"))]))
            .clicked()
        {
            commit_from_ui(app);
        } else if crate::widgets::secondary_button(ui, "⊘", 32.0).on_hover_text(tl!("Cancel (Esc)")).clicked() {
            app.distort.perspective = None;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_snaps_planes_then_warp_and_commit() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width": 200, "height": 120, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session
            .edit("paint", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 10, 190, 110), &[0.2, 0.6, 0.9, 1.0]);
                Ok(())
            })
            .unwrap();
        app.sync_views();
        crate::distort_ui::menu(&mut app, &ctx, "edit.perspectiveWarp", &json!({})).unwrap().unwrap();
        let ev = |app: &mut PhotocraftApp, e| crate::distort_ui::pointer(app, e, egui::Modifiers::NONE);
        // Two planes sharing an edge (the second starts at the first one's corner and snaps).
        ev(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Move { x: 100.0, y: 100.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Up { x: 100.0, y: 100.0 });
        ev(&mut app, ToolEvent::Down { x: 120.0, y: 30.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Up { x: 180.0, y: 101.0 });
        // Drop its left corners next to the first plane's right corners: they snap.
        ev(&mut app, ToolEvent::Down { x: 120.0, y: 30.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Up { x: 102.0, y: 21.0 });
        ev(&mut app, ToolEvent::Down { x: 120.0, y: 101.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Up { x: 101.0, y: 99.0 });
        let s = app.distort.perspective.as_ref().unwrap();
        assert_eq!(s.planes.len(), 2);
        assert_eq!(s.planes[1].src[0], [100.0, 20.0], "snapped to the first plane's corner");
        assert_eq!(s.planes[1].src[3], [100.0, 100.0]);
        control(&mut app, &json!({"mode": "warp"})).unwrap();
        // Dragging the shared top corner moves both planes.
        ev(&mut app, ToolEvent::Down { x: 100.0, y: 20.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Up { x: 100.0, y: 5.0 });
        let s = app.distort.perspective.as_ref().unwrap();
        assert_eq!(s.planes[0].dst[1], [100.0, 5.0]);
        assert_eq!(s.planes[1].dst[0], [100.0, 5.0]);
        assert_eq!(s.planes[0].src[1], [100.0, 20.0], "warp mode keeps the layout");
        let before = app.session.active().unwrap().history.past_len();
        assert_eq!(commit(&mut app).unwrap()["committed"], json!(true));
        assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
    }

    #[test]
    fn replacing_the_planes_mid_corner_drag_ends_the_drag() {
        // #952: the drag kept (plane, corner) indices into the old list, so the next move or
        // release indexed the new, shorter one and panicked.
        for release in [false, true] {
            let ctx = egui::Context::default();
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.session.execute("file.new", json!({"width": 100, "height": 80, "depth": 8})).unwrap();
            app.session.execute("layer.new.layer", json!({})).unwrap();
            app.session
                .edit("paint", |doc, active| {
                    doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(20, 30, 80, 50), &[1.0, 0.0, 0.0, 1.0]);
                    Ok(())
                })
                .unwrap();
            app.sync_views();
            crate::distort_ui::menu(&mut app, &ctx, "edit.perspectiveWarp", &json!({})).unwrap().unwrap();
            let ev = |app: &mut PhotocraftApp, e| crate::distort_ui::pointer(app, e, egui::Modifiers::NONE);
            ev(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 });
            ev(&mut app, ToolEvent::Move { x: 90.0, y: 70.0, pressure: 1.0 });
            ev(&mut app, ToolEvent::Up { x: 90.0, y: 70.0 });
            // Press a corner, then replace the planes while it's held.
            ev(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 });
            assert!(matches!(app.distort.perspective.as_ref().unwrap().drag, Some(Drag::Corner { .. })));
            control(&mut app, &json!({"planes": []})).unwrap();
            if release {
                ev(&mut app, ToolEvent::Up { x: 30.0, y: 30.0 });
            } else {
                ev(&mut app, ToolEvent::Move { x: 30.0, y: 30.0, pressure: 1.0 });
            }
            let s = app.distort.perspective.as_ref().unwrap();
            assert!(s.planes.is_empty() && s.drag.is_none(), "release {release}");
        }
    }
}

#[cfg(test)]
mod agent_control_tests {
    use super::*;

    fn open_session() -> PhotocraftApp {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width": 64, "height": 64, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session
            .edit("paint", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(4, 4, 60, 60), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        app.sync_views();
        begin(&mut app, &ctx, "edit.perspectiveWarp").unwrap();
        app
    }

    fn valid_plane() -> Plane {
        Plane { src: [[10.0, 10.0], [50.0, 10.0], [50.0, 50.0], [10.0, 50.0]], dst: [[12.0, 10.0], [48.0, 8.0], [50.0, 48.0], [12.0, 52.0]] }
    }

    #[test]
    fn agent_can_link_nearby_complete_edges_without_mutating_on_bad_requests() {
        let mut app = open_session();
        let a = Plane::identity([[8.0, 8.0], [31.0, 8.0], [31.0, 48.0], [8.0, 48.0]]);
        let b = Plane::identity([[35.0, 8.0], [58.0, 8.0], [58.0, 48.0], [35.0, 48.0]]);
        let before = control(&mut app, &json!({"planes": [a, b]})).unwrap();
        assert_eq!(before["linkedEdges"], 0);
        assert!(control(&mut app, &json!({"linkEdges": {"tolerance": "bad"}})).is_err());
        assert_eq!(app.distort.perspective.as_ref().unwrap().describe(), before, "bad input must not partially weld");

        let result = control(&mut app, &json!({"linkEdges": true})).unwrap();
        assert_eq!(result["weldedEdges"], 1);
        assert_eq!(result["linkedEdges"], 1);
        assert_eq!(result["planes"][0]["src"][1], json!([33.0, 8.0]));
        assert_eq!(result["planes"][1]["src"][0], json!([33.0, 8.0]));
        let before = app.distort.perspective.as_ref().unwrap().describe();
        assert!(control(&mut app, &json!({"mode": "warp", "linkEdges": true})).unwrap_err().contains("Layout"));
        assert_eq!(app.distort.perspective.as_ref().unwrap().describe(), before);
        assert_eq!(control(&mut app, &json!({"mode": "warp"})).unwrap()["linkedEdges"], 1);
    }

    #[test]
    fn invalid_agent_input_is_atomic() {
        let mut app = open_session();
        let before = app.distort.perspective.as_ref().unwrap().describe();
        let bad = Plane { src: [[1.0, 1.0]; 4], dst: [[2.0, 2.0]; 4] };
        assert!(control(&mut app, &json!({"mode": "warp", "planes": [bad]})).unwrap_err().contains("degenerate"));
        assert_eq!(app.distort.perspective.as_ref().unwrap().describe(), before);
        assert!(control(&mut app, &json!({"mode": "unknown"})).unwrap_err().contains("mode"));
        assert_eq!(app.distort.perspective.as_ref().unwrap().describe(), before);
        assert!(control(&mut app, &json!({"mode": 42})).unwrap_err().contains("mode"));
        assert_eq!(app.distort.perspective.as_ref().unwrap().describe(), before);
    }

    #[test]
    fn agent_commit_does_not_claim_success_on_noop_or_invalid_geometry() {
        let mut app = open_session();
        assert_eq!(control(&mut app, &json!({"commit": true})).unwrap()["committed"], json!(false));
        assert!(app.distort.perspective.is_none());

        let mut app = open_session();
        control(&mut app, &json!({"mode": "warp", "planes": [valid_plane()]})).unwrap();
        app.distort.perspective.as_mut().unwrap().planes[0].dst = [[4.0, 4.0]; 4];
        let before_history = app.session.active().unwrap().history.past_len();
        assert!(control(&mut app, &json!({"commit": true})).unwrap_err().contains("degenerate"));
        assert!(app.distort.perspective.is_some(), "an invalid commit must not lose the preview");
        assert_eq!(app.session.active().unwrap().history.past_len(), before_history);
    }

    #[test]
    fn successful_agent_commit_is_one_history_step() {
        let mut app = open_session();
        control(&mut app, &json!({"mode": "warp", "planes": [valid_plane()]})).unwrap();
        let before_history = app.session.active().unwrap().history.past_len();
        let reply = control(&mut app, &json!({"commit": true})).unwrap();
        assert_eq!(reply["committed"], json!(true));
        assert!(app.distort.perspective.is_none());
        assert_eq!(app.session.active().unwrap().history.past_len(), before_history + 1);
    }

    #[test]
    fn engine_failure_is_reported_and_preserves_preview() {
        let mut app = open_session();
        control(&mut app, &json!({"mode": "warp", "planes": [valid_plane()]})).unwrap();
        // An unknown engine command simulates an error after geometry validation.
        app.distort.perspective.as_mut().unwrap().command = "unrecognized.perspective.command".into();
        let before_history = app.session.active().unwrap().history.past_len();
        let error = control(&mut app, &json!({"commit": true})).unwrap_err();
        assert!(!error.is_empty(), "engine error must reach the agent");
        assert!(app.distort.perspective.is_some(), "a failed engine command must retain the preview");
        assert_eq!(app.session.active().unwrap().history.past_len(), before_history);
    }

    /// Agent calls cannot edit another document's preview. The current UI also
    /// cancels a stale session at sync_views, so returning to the original tab
    /// must not resurrect an already-cancelled perspective transaction.
    #[test]
    fn agent_actions_stay_bound_to_original_document() {
        let mut app = open_session();
        control(&mut app, &json!({"mode": "warp", "planes": [valid_plane()]})).unwrap();
        let preview = app.distort.perspective.as_ref().unwrap().describe();
        let original = app.session.active().unwrap().doc.id;
        let original_history = app.session.active().unwrap().history.past_len();
        app.session.execute("file.new", json!({"width": 72, "height": 72, "depth": 8})).unwrap();
        assert_ne!(app.session.active().unwrap().doc.id, original);
        let other_history = app.session.active().unwrap().history.past_len();
        assert!(
            crate::distort_ui::display_doc(&app, app.session.active_index().unwrap()).is_none(),
            "a preview must never be drawn over an unrelated document"
        );
        assert!(control(&mut app, &json!({"mode": "layout"})).is_err());
        assert_eq!(app.distort.perspective.as_ref().unwrap().describe(), preview);
        assert!(control(&mut app, &json!({"commit": true})).is_err());
        assert_eq!(app.session.active().unwrap().history.past_len(), other_history);

        app.sync_views();
        assert!(app.distort.perspective.is_none(), "switching documents cancels the stale preview");
        assert_eq!(app.session.active().unwrap().history.past_len(), other_history);
        assert!(app.session.set_active(0));
        assert!(control(&mut app, &json!({"commit": true})).is_err(), "a cancelled preview must not be committed");
        assert_eq!(app.session.active().unwrap().history.past_len(), original_history);
    }

    #[test]
    fn invalid_identity_plane_does_not_discard_preview() {
        let mut app = open_session();
        let collapsed = Plane { src: [[10.0, 10.0]; 4], dst: [[10.0, 10.0]; 4] };
        app.distort.perspective.as_mut().unwrap().planes.push(collapsed);
        let before_history = app.session.active().unwrap().history.past_len();
        assert!(control(&mut app, &json!({"commit": true})).unwrap_err().contains("degenerate"));
        assert!(app.distort.perspective.is_some(), "a degenerate identity is an error, not an accepted no-op");
        assert_eq!(app.session.active().unwrap().history.past_len(), before_history);
    }

    #[test]
    fn nonfinite_agent_plane_is_rejected() {
        let mut bad = valid_plane();
        bad.dst[0][0] = f64::NAN;
        assert!(validate_planes(&[bad]).unwrap_err().contains("finite"));
    }

    /// Review #5173: an invalid interactive Warp gesture must not turn the preview
    /// into the identity grid or silently accept a self-crossing quadrilateral.
    #[test]
    fn interactive_warp_rejects_crossing_corner_and_preserves_mesh() {
        let mut app = open_session();
        control(&mut app, &json!({"mode": "warp", "planes": [valid_plane()]})).unwrap();
        let start = [12.0, 10.0];
        let invalid = [70.0, 70.0];
        let mut crossed = valid_plane();
        crossed.dst[0] = invalid;
        assert!(validate_planes(&[crossed]).is_err(), "test target must be invalid");
        let (before_planes, before_mesh) = {
            let session = app.distort.perspective.as_ref().unwrap();
            (session.describe(), session.grid.clone())
        };
        let past = app.session.active().unwrap().history.past_len();
        pointer(&mut app, ToolEvent::Down { x: start[0], y: start[1], pressure: 1.0 }, egui::Modifiers::NONE);
        pointer(&mut app, ToolEvent::Move { x: invalid[0], y: invalid[1], pressure: 1.0 }, egui::Modifiers::NONE);
        assert_eq!(app.distort.perspective.as_ref().unwrap().describe(), before_planes, "invalid move must not mutate planes");
        assert_eq!(app.distort.perspective.as_ref().unwrap().grid, before_mesh, "invalid move must keep the last valid grid");
        pointer(&mut app, ToolEvent::Up { x: invalid[0], y: invalid[1] }, egui::Modifiers::NONE);
        assert_eq!(app.distort.perspective.as_ref().unwrap().describe(), before_planes, "invalid release must not mutate planes");
        assert_eq!(app.distort.perspective.as_ref().unwrap().grid, before_mesh, "invalid release must not reset preview");
        assert_eq!(app.session.active().unwrap().history.past_len(), past, "drag must not commit history");
    }

    #[test]
    fn interactive_warp_accepts_valid_corner_edit() {
        let mut app = open_session();
        control(&mut app, &json!({"mode": "warp", "planes": [valid_plane()]})).unwrap();
        pointer(&mut app, ToolEvent::Down { x: 12.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
        pointer(&mut app, ToolEvent::Move { x: 16.0, y: 14.0, pressure: 1.0 }, egui::Modifiers::NONE);
        pointer(&mut app, ToolEvent::Up { x: 16.0, y: 14.0 }, egui::Modifiers::NONE);
        let session = app.distort.perspective.as_ref().unwrap();
        assert_eq!(session.planes[0].dst[0], [16.0, 14.0]);
        assert!(PerspectiveMap::new(&session.planes).is_some());
    }

    #[test]
    fn enter_shows_invalid_warp_commit_error_and_keeps_editable_preview() {
        let mut app = open_session();
        control(&mut app, &json!({"mode": "warp", "planes": [valid_plane()]})).unwrap();
        app.distort.perspective.as_mut().unwrap().planes[0].dst = [[9.0, 9.0]; 4];
        let history = app.session.active().unwrap().history.past_len();
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput {
            events: vec![egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
            ..Default::default()
        });
        assert!(crate::distort_ui::keys(&mut app, &ctx));
        ctx.end_pass().textures_delta.clear();
        assert!(app.ui.status.contains("degenerate"), "Enter must display the failure: {}", app.ui.status);
        assert!(app.distort.perspective.is_some());
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
    }

    #[test]
    fn checkmark_handler_displays_engine_failure_and_keeps_preview() {
        let mut app = open_session();
        control(&mut app, &json!({"mode": "warp", "planes": [valid_plane()]})).unwrap();
        app.distort.perspective.as_mut().unwrap().command = "unrecognized.perspective.command".into();
        let before = app.session.active().unwrap().history.past_len();
        commit_from_ui(&mut app);
        assert!(app.ui.status.contains("Perspective Warp:"));
        assert!(app.distort.perspective.is_some(), "checkmark error must preserve preview");
        assert_eq!(app.session.active().unwrap().history.past_len(), before);
    }

    #[test]
    fn invalid_interactive_commit_surfaces_error_and_keeps_preview() {
        let mut app = open_session();
        control(&mut app, &json!({"mode": "warp", "planes": [valid_plane()]})).unwrap();
        // Direct mutation simulates an outdated document/invalid preview while the user
        // presses Enter or the checkmark, irrespective of the mouse guard.
        app.distort.perspective.as_mut().unwrap().planes[0].dst = [[9.0, 9.0]; 4];
        let before_history = app.session.active().unwrap().history.past_len();
        assert!(commit(&mut app).is_err());
        assert!(app.ui.status.contains("degenerate"), "interactive user needs the specific failure in the status: {}", app.ui.status);
        assert!(app.distort.perspective.is_some());
        assert_eq!(app.session.active().unwrap().history.past_len(), before_history);
    }

    /// Reviewer #8498: snapping two new corners to one existing corner must not
    /// leave a degenerate plane in the editable session.
    #[test]
    fn snapped_degenerate_new_plane_is_rejected() {
        let mut app = open_session();
        let a = Plane::identity([[10.0, 10.0], [50.0, 10.0], [50.0, 50.0], [10.0, 50.0]]);
        control(&mut app, &json!({"planes": [a]})).unwrap();
        assert!(crate::distort_ui::tolerance(&app) > 5.4);
        let (before, grid) = {
            let s = app.distort.perspective.as_ref().unwrap();
            (s.describe(), s.grid.clone())
        };
        let h = app.session.active().unwrap().history.past_len();
        pointer(&mut app, ToolEvent::Down { x: 45.0, y: 70.0, pressure: 1.0 }, egui::Modifiers::NONE);
        pointer(&mut app, ToolEvent::Up { x: 55.0, y: 48.0 }, egui::Modifiers::NONE);
        let s = app.distort.perspective.as_ref().unwrap();
        assert_eq!(s.describe(), before, "a snapped collapse must not append a plane");
        assert_eq!(s.grid, grid, "a rejected plane cannot poison the preview mesh");
        assert_eq!(app.session.active().unwrap().history.past_len(), h);
        assert!(app.ui.status.contains("invalid plane"));
        pointer(&mut app, ToolEvent::Down { x: 70.0, y: 15.0, pressure: 1.0 }, egui::Modifiers::NONE);
        pointer(&mut app, ToolEvent::Up { x: 90.0, y: 55.0 }, egui::Modifiers::NONE);
        let s = app.distort.perspective.as_ref().unwrap();
        assert_eq!(s.planes.len(), 2, "a subsequent valid plane should still work");
        assert!(validate_planes(&s.planes).is_ok());
    }

    #[test]
    fn straighten_that_would_collapse_a_valid_plane_rolls_back() {
        let mut app = open_session();
        let plane = Plane { src: [[10.0, 10.0], [50.0, 10.0], [50.0, 50.0], [10.0, 50.0]], dst: [[11.0, -3.0], [-1.0, -4.0], [5.0, 2.0], [11.0, 6.0]] };
        assert!(validate_planes(std::slice::from_ref(&plane)).is_ok());
        let mut formerly_mutated = vec![plane.clone()];
        straighten(&mut formerly_mutated, Straighten::Horizontal);
        assert!(validate_planes(&formerly_mutated).is_err(), "the old unguarded straighten corrupts valid geometry");

        control(&mut app, &json!({"planes": [plane], "mode": "warp"})).unwrap();
        let (before, mesh) = {
            let s = app.distort.perspective.as_ref().unwrap();
            (s.describe(), s.grid.clone())
        };
        let history = app.session.active().unwrap().history.past_len();
        let s = app.distort.perspective.as_mut().unwrap();
        assert!(straighten_if_valid(s, Straighten::Horizontal).is_err());
        assert_eq!(s.describe(), before, "straighten failure must restore original quads");
        assert_eq!(s.grid, mesh, "straighten failure must preserve last valid preview");
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
    }

    #[test]
    fn linked_edge_count_status_uses_formatted_localized_template() {
        let label = crate::i18n::fmt(tl!("Edges linked: {count}"), &[("count", "2")]);
        assert!(label.contains('2'));
        assert!(!label.contains("{count}"));
    }
}
