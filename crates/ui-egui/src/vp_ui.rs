//! Filter › Vanishing Point on the canvas (dialog mode).
//!
//! **Create Plane** (C): four clicks set a plane's corners, in any order. **Edit Plane** (V):
//! drag a corner node, or an edge node to slide that edge along the plane's own perspective;
//! click a plane to select it, Delete removes it. Ctrl/Cmd-dragging an edge node tears off a
//! perpendicular plane; dragging the torn plane's far edge node sets its depth and Alt-dragging
//! it its angle (it stays a torn-off plane, so it follows its parent). The plane's grid is drawn
//! at the Grid Size and,
//! like Photoshop's, in the accent colour when the plane is valid, yellow when its perspective is
//! doubtful and red when it is invalid ([`VpPlane::check`]). With an image on the clipboard the
//! selected plane shows it pasted in perspective; drag it to move it on the plane. **Stamp** (S):
//! Alt-click a plane to set the source, then paint to clone in perspective (Diameter, Hardness,
//! Opacity, Aligned); the canvas shows the strokes as they are painted. ↩ / OK runs
//! `filter.vanishingPoint` with the planes, the paste and the strokes, so the whole session is
//! one history step and an action or agent can replay it exactly.

use std::sync::Arc;

use egui::{Color32, Pos2, Stroke, TextureHandle, pos2};
use photocraft_algo::transform::Homography;
use photocraft_algo::vanishing::{Edge, PlaneCheck, Scene, VpPlane, clone_dabs_from, clone_source};
use photocraft_doc::{DocId, Document, Layer, LayerContent, LayerId};
use photocraft_engine::vp_cmds::{CloneSpec, MAX_CLONE_DABS, MAX_CLONE_STROKES, VP, copy_changed, over, paste_onto, visible_image};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::edit_history::{EditHistory, Step};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VpTool {
    EditPlane,
    CreatePlane,
    Stamp,
}

/// A plane torn off an edge of an earlier plane: the engine's
/// `{"from", "edge", "angle", "depth"}`, so it follows its parent when that is edited.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct Torn {
    pub from: usize,
    pub edge: Edge,
    /// Degrees between the planes (90 = perpendicular).
    pub angle: f64,
    /// Extent as a multiple of the parent's adjacent side.
    pub depth: f64,
}

/// The Stamp tool's options.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StampOptions {
    /// Brush diameter in px where a stroke starts.
    pub diameter: f64,
    /// 0..=100.
    pub hardness: f64,
    /// 0..=100.
    pub opacity: f64,
    /// Later strokes keep the first stroke's source offset instead of restarting at the source.
    pub aligned: bool,
}

/// Edge node `i` (0 top, 1 right, 2 bottom, 3 left) as the engine's edge.
const EDGES: [Edge; 4] = [Edge::Top, Edge::Right, Edge::Bottom, Edge::Left];

fn edge_index(e: Edge) -> usize {
    match e {
        Edge::Top => 0,
        Edge::Right => 1,
        Edge::Bottom => 2,
        Edge::Left => 3,
    }
}

enum Drag {
    /// A corner node, and the pointer offset from it.
    Corner { plane: usize, corner: usize, offset: [f64; 2] },
    /// An edge node (0 top, 1 right, 2 bottom, 3 left): the edge slides along the plane's
    /// perspective as it was at the press.
    Edge { plane: usize, edge: usize, h: Homography, inv: Homography },
    /// The pasted image, and the pointer offset from its top-left in plane coordinates.
    Paste { offset: [f64; 2] },
    /// The far edge node of torn-off plane `plane` (or of one being torn off): the drag sets
    /// its depth, or with `rotate` (Alt) its angle, once it has left `start`.
    Torn { plane: usize, rotate: bool, start: [f64; 2], moved: bool },
    /// Painting the last of the clone strokes.
    Stamp,
}

/// The clone strokes painted onto the target layer, shown on the canvas while stamping.
struct StampPreview {
    /// The session's `rev`, New Layer and paste it was painted with (any changing repaints it).
    rev: u64,
    new_layer: bool,
    /// The paste (plane, top-left, width) baked in under the strokes, as OK pastes before it clones.
    paste: Option<(usize, [f64; 2], f64)>,
    /// The paste landed in `out` (it can fail, or miss the plane).
    baked: bool,
    /// On a new layer: the visible image the strokes sample, with the strokes so far.
    work: Option<Surface>,
    /// The sampled image before the current stroke, retained across frame batches.
    source: Option<(usize, Surface)>,
    /// The target layer's pixels as painted so far.
    out: Surface,
    /// Strokes painted: those before `.0`, and the first `.1` points of stroke `.0`.
    done: (usize, usize),
    /// Preview keys shown, each with the area changed since the previous one (the first since
    /// the document).
    keys: Vec<(u64, Rect)>,
    shown: Arc<Document>,
}

/// The clipboard image shown pasted onto the plane.
struct Clip {
    /// The clipboard pixels and their content bounds, which the preview pastes like OK does.
    surface: Surface,
    rect: Rect,
    texture: TextureHandle,
    width: f64,
    height: f64,
}

pub struct VpSession {
    /// Document this session belongs to (#5115).
    pub document: DocId,
    pub layer: LayerId,
    canvas: Rect,
    /// The planes' corners; torn-off planes' are derived from their parents ([`Self::resolve`]).
    pub planes: Vec<VpPlane>,
    /// Per plane: how it was torn off, or `None` for a drawn plane (always the first one).
    pub torn: Vec<Option<Torn>>,
    pub tool: VpTool,
    /// Corners clicked so far with Create Plane.
    pending: Vec<[f64; 2]>,
    pub selected: Option<usize>,
    /// Grid cell size in px at the plane's nearest edge.
    pub grid_size: f64,
    /// Paste the clipboard onto the selected plane on OK.
    pub paste: bool,
    /// Top-left of the pasted image in plane coordinates (0..1).
    pub paste_at: [f64; 2],
    /// Width of the pasted image as a fraction of the plane's width.
    pub paste_width: f64,
    pub new_layer: bool,
    clip: Option<Clip>,
    drag: Option<Drag>,
    pub stamp: StampOptions,
    /// Stamp source (Alt-click), image px.
    pub source: Option<[f64; 2]>,
    /// Clone strokes painted so far, sent as `"clone"` on OK.
    pub strokes: Vec<CloneSpec>,
    /// The first stroke's (source, start) since the source was set: Aligned strokes keep its offset.
    aligned_ref: Option<([f64; 2], [f64; 2])>,
    /// Bumped whenever the planes or the strokes change other than by painting on.
    rev: u64,
    /// The document as the session began (the preview paints over it).
    base: Arc<Document>,
    doc_revision: u64,
    preview: Option<StampPreview>,
    history: EditHistory<VpEdit>,
}

/// What ⌘Z steps through: the planes (and how they were torn off), where the paste sits on
/// them, and the clone source and strokes.
#[derive(Clone, PartialEq)]
struct VpEdit {
    planes: Vec<VpPlane>,
    torn: Vec<Option<Torn>>,
    paste_at: [f64; 2],
    paste_width: f64,
    source: Option<[f64; 2]>,
    strokes: Vec<CloneSpec>,
    aligned_ref: Option<([f64; 2], [f64; 2])>,
}

/// Most planes a session holds (the control channel can't make it allocate without bound).
const MAX_PLANES: usize = 64;
/// Most grid lines drawn per direction.
const MAX_LINES: usize = 200;
/// Preview keys of the stamp preview: `PREVIEW_KEY + id` (see `canvas::display_doc`).
const PREVIEW_KEY: u64 = 1 << 42;

fn union(a: Rect, b: Rect) -> Rect {
    if a.is_empty() {
        b
    } else if b.is_empty() {
        a
    } else {
        a.union(&b)
    }
}

impl VpSession {
    pub fn describe(&self) -> Value {
        let center = self.center();
        let planes: Vec<Value> = self
            .planes
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let mut v = json!({"corners": p.corners, "check": p.check(center)});
                if let Some(t) = self.torn.get(i).copied().flatten() {
                    v["tornOff"] = json!(t);
                }
                v
            })
            .collect();
        json!({
            "document": self.document.0,
            "layer": self.layer.0,
            "tool": self.tool,
            "planes": planes,
            "stamp": self.stamp,
            "source": self.source,
            "strokes": self.strokes.len(),
            "dabs": self.dabs(),
            "pending": self.pending,
            "selected": self.selected,
            "gridSize": self.grid_size,
            "paste": self.paste && self.clip.is_some(),
            "pasteAt": self.paste_at,
            "pasteWidth": self.paste_width,
            "newLayer": self.new_layer,
        })
    }

    fn center(&self) -> [f64; 2] {
        let c = self.canvas;
        [f64::from(c.x0 + c.x1) / 2.0, f64::from(c.y0 + c.y1) / 2.0]
    }

    /// The planes with the shared camera the engine derives from the first one.
    fn scene(&self) -> Option<Scene> {
        let first = self.planes.first()?.clone();
        let fallback = f64::from(self.canvas.width().max(self.canvas.height()));
        let mut sc = Scene::new(first, self.center(), None, fallback);
        sc.planes.extend(self.planes.iter().skip(1).cloned());
        Some(sc)
    }

    /// Plane `i`'s unit square → image map and metric side lengths.
    fn frame(&self, i: usize) -> Option<(Homography, f64, f64)> {
        if i >= self.planes.len() {
            return None;
        }
        let (h, lu, lv) = self.scene()?.frame(i)?;
        (lu.is_finite() && lv.is_finite() && lu > 1e-12 && lv > 1e-12).then_some((h, lu, lv))
    }

    /// The plane the paste lands on.
    fn paste_plane(&self) -> Option<usize> {
        if !self.paste || self.clip.is_none() {
            return None;
        }
        self.selected.filter(|i| *i < self.planes.len()).or_else(|| (!self.planes.is_empty()).then_some(0))
    }

    /// What OK pastes: the plane, the top-left and the width.
    fn paste_key(&self) -> Option<(usize, [f64; 2], f64)> {
        self.paste_plane().map(|i| (i, self.paste_at, self.paste_width))
    }

    /// The stamp preview is current and already shows the paste under the strokes.
    fn preview_has_paste(&self) -> bool {
        self.preview.as_ref().is_some_and(|p| p.baked && p.rev == self.rev && p.paste == self.paste_key())
    }

    /// The paste is dragged while the stamp preview holds it in its old place.
    fn moving_baked_paste(&self) -> bool {
        matches!(self.drag, Some(Drag::Paste { .. })) && self.preview.as_ref().is_some_and(|p| p.baked)
    }

    /// [`draw_content`] draws the paste overlay: not when the stamp preview shows the paste, and
    /// not while a plane drag freezes that preview (as it freezes the strokes) until release.
    fn paste_overlay(&self) -> bool {
        let frozen = matches!(self.drag, Some(Drag::Corner { .. } | Drag::Edge { .. } | Drag::Torn { .. })) && self.preview.as_ref().is_some_and(|p| p.baked);
        !self.preview_has_paste() && !frozen
    }

    /// Size of the pasted image in plane coordinates (the engine keeps its metric aspect).
    fn paste_extent(&self, lu: f64, lv: f64) -> Option<[f64; 2]> {
        let c = self.clip.as_ref()?;
        Some([self.paste_width, self.paste_width * lu / lv * c.height / c.width.max(1.0)])
    }

    /// The command's params: the planes (torn-off ones as such), the paste and the clone
    /// strokes when there are any.
    pub fn params(&self) -> Value {
        let planes: Vec<Value> = self
            .planes
            .iter()
            .enumerate()
            .map(|(i, p)| match self.torn.get(i).copied().flatten() {
                Some(t) => json!({"from": t.from, "edge": t.edge, "angle": t.angle, "depth": t.depth}),
                None => json!({"corners": p.corners}),
            })
            .collect();
        let mut p = json!({"planes": planes});
        if let Some(i) = self.paste_plane() {
            p["paste"] = json!([{"plane": i, "at": self.paste_at, "width": self.paste_width}]);
        }
        if !self.strokes.is_empty() {
            let clone: Vec<Value> = self
                .strokes
                .iter()
                .map(|c| json!({"source": c.source, "points": c.points, "size": c.size, "hardness": c.hardness * 100.0, "opacity": c.opacity * 100.0}))
                .collect();
            p["clone"] = json!(clone);
        }
        if self.new_layer {
            p["newLayer"] = json!(true);
        }
        p
    }

    fn dabs(&self) -> usize {
        self.strokes.iter().map(|c| c.points.len()).sum()
    }

    /// Recomputes the torn-off planes' corners from their parents, in order (the way the engine
    /// builds the scene), so they follow every edit. A plane that can't be torn off any more
    /// (it would lie behind the camera) collapses to an invalid one.
    fn resolve(&mut self) {
        self.rev += 1;
        self.torn.resize(self.planes.len(), None);
        let Some(first) = self.planes.first().cloned() else { return };
        let fallback = f64::from(self.canvas.width().max(self.canvas.height()));
        let mut sc = Scene::new(first, self.center(), None, fallback);
        for i in 1..self.planes.len() {
            match self.torn.get(i).copied().flatten().filter(|t| t.from < i) {
                Some(t) => {
                    if sc.tear_off(t.from, t.edge, t.angle, t.depth).is_none() {
                        sc.planes.push(VpPlane { corners: [[0.0; 2]; 4] });
                    }
                }
                None => sc.planes.push(self.planes[i].clone()),
            }
        }
        self.planes = sc.planes;
    }

    fn push_plane(&mut self, plane: VpPlane, torn: Option<Torn>) {
        self.torn.resize(self.planes.len(), None);
        self.planes.push(plane);
        self.torn.push(torn);
        self.selected = Some(self.planes.len() - 1);
        self.resolve();
    }

    fn add_corner(&mut self, p: [f64; 2]) {
        if self.planes.len() >= MAX_PLANES {
            return;
        }
        self.pending.push(p);
        if let Ok(corners) = <[[f64; 2]; 4]>::try_from(self.pending.as_slice()) {
            self.pending.clear();
            self.push_plane(VpPlane { corners }.normalized(), None);
            // Photoshop hands over to Edit Plane once a plane is closed.
            self.tool = VpTool::EditPlane;
        }
    }

    fn edit(&self) -> VpEdit {
        VpEdit {
            planes: self.planes.clone(),
            torn: self.torn.clone(),
            paste_at: self.paste_at,
            paste_width: self.paste_width,
            source: self.source,
            strokes: self.strokes.clone(),
            aligned_ref: self.aligned_ref,
        }
    }

    fn settle(&mut self, busy: bool) {
        let now = self.edit();
        self.history.settle(&now, busy || self.drag.is_some());
    }

    pub(crate) fn step(&mut self, step: Step) {
        let mut now = self.edit();
        if self.history.step(&mut now, step) {
            let VpEdit { planes, torn, paste_at, paste_width, source, strokes, aligned_ref } = now;
            (self.planes, self.torn, self.paste_at, self.paste_width) = (planes, torn, paste_at, paste_width);
            (self.source, self.strokes, self.aligned_ref) = (source, strokes, aligned_ref);
            self.drag = None;
            self.pending.clear();
            self.selected = self.selected.filter(|i| *i < self.planes.len());
            // The stamp preview repaints from scratch.
            self.rev += 1;
        }
    }

    /// Deletes the selected plane; planes torn off it keep their place as drawn planes.
    fn delete_selected(&mut self) {
        if let Some(i) = self.selected.take().filter(|i| *i < self.planes.len()) {
            self.torn.resize(self.planes.len(), None);
            self.planes.remove(i);
            self.torn.remove(i);
            for t in self.torn.iter_mut() {
                match t {
                    Some(c) if c.from == i => *t = None,
                    Some(c) if c.from > i => c.from -= 1,
                    _ => {}
                }
            }
            self.drag = None;
            self.resolve();
        }
    }

    /// Starts tearing a plane off edge `edge` of plane `from` at the press `p`.
    fn tear_off(&mut self, from: usize, edge: usize, p: [f64; 2]) {
        if self.planes.len() >= MAX_PLANES {
            return;
        }
        let Some(&e) = EDGES.get(edge) else { return };
        self.push_plane(VpPlane { corners: [[0.0; 2]; 4] }, Some(Torn { from, edge: e, angle: 90.0, depth: 0.5 }));
        // As after closing a plane, Edit Plane takes over.
        self.tool = VpTool::EditPlane;
        self.pending.clear();
        self.drag = Some(Drag::Torn { plane: self.planes.len() - 1, rotate: false, start: p, moved: false });
    }

    /// Fits torn plane `i`'s depth (or with `rotate` its angle) so its far edge's middle is as
    /// near `p` as it can get.
    fn fit_torn(&mut self, i: usize, rotate: bool, p: [f64; 2]) {
        let Some(t) = self.torn.get(i).copied().flatten() else { return };
        let Some(first) = self.planes.first().cloned() else { return };
        if t.from >= i || i >= self.planes.len() {
            return;
        }
        let fallback = f64::from(self.canvas.width().max(self.canvas.height()));
        let mut sc = Scene::new(first, self.center(), None, fallback);
        sc.planes.extend(self.planes.iter().take(i).skip(1).cloned());
        let e = edge_index(t.edge);
        let mut miss = |angle: f64, depth: f64| -> f64 {
            if sc.tear_off(t.from, t.edge, angle, depth).is_none() {
                return f64::MAX;
            }
            let Some(c) = sc.planes.pop().map(|pl| pl.corners) else { return f64::MAX };
            let (a, b) = (c[e], c[(e + 1) % 4]);
            let d = ((a[0] + b[0]) / 2.0 - p[0]).hypot((a[1] + b[1]) / 2.0 - p[1]);
            if d.is_finite() { d } else { f64::MAX }
        };
        let best = |values: &mut dyn Iterator<Item = f64>, miss: &mut dyn FnMut(f64) -> f64| {
            values.map(|v| (miss(v), v)).filter(|m| m.0 < f64::MAX).min_by(|a, b| a.0.total_cmp(&b.0)).map(|m| m.1)
        };
        let (angle, depth) = if rotate {
            let Some(a) = best(&mut (1..180).map(f64::from), &mut |a| miss(a, t.depth)) else { return };
            let a = best(&mut (-10..=10).map(|k| a + f64::from(k) / 10.0), &mut |a| miss(a, t.depth)).unwrap_or(a);
            (a.clamp(1.0, 179.0), t.depth)
        } else {
            let Some(d) = best(&mut (0..=160).map(|k| 0.02 * 1.06f64.powi(k)), &mut |d| miss(t.angle, d)) else { return };
            let d = best(&mut (-10..=10).map(|k| d * (1.0 + f64::from(k) * 0.006)), &mut |d| miss(t.angle, d)).unwrap_or(d);
            (t.angle, d.clamp(0.02, 1000.0))
        };
        if let Some(Some(t)) = self.torn.get_mut(i) {
            t.angle = angle;
            t.depth = depth;
        }
        self.resolve();
    }

    /// Stamp press: Alt sets the source, otherwise a stroke starts. Returns a status message.
    fn stamp_press(&mut self, p: [f64; 2], alt: bool) -> Option<&'static str> {
        let sc = self.scene()?;
        let on_plane = |q: [f64; 2]| sc.plane_at(q).is_some();
        if alt {
            if !on_plane(p) {
                return Some(tl!("Alt-click a plane to set the clone source"));
            }
            self.source = Some(p);
            self.aligned_ref = None;
            return None;
        }
        let Some(source) = self.source else { return Some(tl!("Alt-click a plane to set the clone source")) };
        if !on_plane(p) {
            return None;
        }
        if self.strokes.len() >= MAX_CLONE_STROKES || self.dabs() >= MAX_CLONE_DABS {
            return Some(tl!("Too many clone strokes: apply Vanishing Point first"));
        }
        let src = match self.aligned_ref.filter(|_| self.stamp.aligned) {
            Some((s0, a0)) => clone_source(&sc, s0, a0, p).filter(|s| on_plane(*s)).unwrap_or(source),
            None => source,
        };
        if self.aligned_ref.is_none() {
            self.aligned_ref = Some((src, p));
        }
        let o = self.stamp;
        self.strokes.push(CloneSpec { source: src, points: vec![p], size: o.diameter, hardness: o.hardness / 100.0, opacity: o.opacity / 100.0 });
        self.drag = Some(Drag::Stamp);
        None
    }

    /// Extends the current stroke to `p` with dabs a tenth of the brush apart.
    fn stamp_to(&mut self, p: [f64; 2]) {
        let room = MAX_CLONE_DABS.saturating_sub(self.dabs());
        let Some(c) = self.strokes.last_mut() else { return };
        let Some(last) = c.points.last().copied() else { return };
        let step = (c.size * 0.1).max(1.0);
        let d = (p[0] - last[0]).hypot(p[1] - last[1]);
        if !d.is_finite() || d < step {
            return;
        }
        let n = ((d / step) as usize).min(room);
        c.points.extend((1..=n).map(|k| {
            let t = k as f64 * step / d;
            [last[0] + (p[0] - last[0]) * t, last[1] + (p[1] - last[1]) * t]
        }));
    }

    /// Paints what the preview doesn't show yet (all of it after the planes changed). `key`
    /// names a new preview. False when nothing changed.
    fn paint_preview(&mut self, key: impl FnOnce() -> u64) -> bool {
        if self.strokes.is_empty() {
            let had = self.preview.is_some();
            self.preview = None;
            return had;
        }
        let Some(sc) = self.scene() else {
            // No planes left: nothing to show the strokes on.
            return self.preview.take().is_some();
        };
        let paste = self.paste_key();
        let canvas = self.canvas;
        let stale = self.preview.as_ref().is_none_or(|p| p.rev != self.rev || p.new_layer != self.new_layer || p.paste != paste);
        let mut area = Rect::EMPTY;
        let mut baked = false;
        if stale {
            let Some(layer) = self.base.layer(self.layer).and_then(Layer::surface) else {
                // The layer is gone (or no longer pixels): nothing to show the strokes on.
                return self.preview.take().is_some();
            };
            let mut out = if self.new_layer { Surface::new(layer.format()) } else { layer.clone() };
            // OK pastes first and the strokes then paint over (and sample) the pasted pixels.
            if let (Some((i, at, width)), Some(c)) = (paste, self.clip.as_ref())
                && self.planes.get(i).is_some_and(|p| p.check(self.center()) != PlaneCheck::Invalid)
                && let Ok(r) = paste_onto(&sc, &mut out, i, &c.surface, c.rect, at, width)
            {
                area = r.intersect(&canvas);
                baked = !r.is_empty();
            }
            let work = self.new_layer.then(|| {
                let mut w = visible_image(&self.base);
                over(&mut w, &out);
                w
            });
            // The old preview's area changes back too.
            let keys = self.preview.take().map(|p| p.keys).unwrap_or_default();
            area = keys.iter().fold(area, |a, k| union(a, k.1));
            self.preview = Some(StampPreview {
                rev: self.rev,
                new_layer: self.new_layer,
                paste,
                baked,
                work,
                source: None,
                out,
                done: (0, 0),
                keys,
                shown: self.base.clone(),
            });
        }
        let Some(pv) = self.preview.as_mut() else { return false };
        let (from_stroke, from_point) = pv.done;
        for (si, c) in self.strokes.iter().enumerate().skip(from_stroke) {
            let skip = if si == from_stroke { from_point } else { 0 };
            let (Some(anchor), Some(points)) = (c.points.first(), c.points.get(skip..)) else { continue };
            if points.is_empty() {
                continue;
            }
            if pv.source.as_ref().is_none_or(|(stroke, _)| *stroke != si) {
                pv.source = Some((si, pv.work.as_ref().unwrap_or(&pv.out).clone()));
            }
            let Some((_, source)) = pv.source.as_ref() else { continue };
            let (hard, opacity) = (c.hardness, c.opacity as f32);
            let changed = match pv.work.as_mut() {
                Some(work) => {
                    let before = work.clone();
                    let (_, a) = clone_dabs_from(&sc, work, source, c.source, *anchor, points, c.size, hard, opacity, Some(canvas));
                    // As OK does: only the canvas (the dabs' boxes can span far off it).
                    let a = a.intersect(&canvas);
                    copy_changed(&mut pv.out, &before, work, a);
                    a
                }
                None => clone_dabs_from(&sc, &mut pv.out, source, c.source, *anchor, points, c.size, hard, opacity, Some(canvas)).1,
            };
            area = union(area, changed);
        }
        pv.done = (self.strokes.len() - 1, self.strokes.last().map_or(0, |c| c.points.len()));
        if area.is_empty() && !stale {
            return false;
        }
        let mut d = (*self.base).clone();
        if self.new_layer {
            let mut l = Layer::raster(d.next_layer_name("Vanishing Point"), d.pixel_format());
            l.content = LayerContent::Raster(pv.out.clone());
            d.insert_above(Some(self.layer), l);
        } else if let Some(LayerContent::Raster(s)) = d.layer_mut(self.layer).map(|l| &mut l.content) {
            *s = pv.out.clone();
        }
        pv.shown = Arc::new(d);
        pv.keys.push((key(), area));
        true
    }

    /// The document with the strokes painted, and its preview key. Not while the paste is
    /// dragged out of its baked-in place: the overlay follows the pointer instead, and the
    /// strokes repaint over it on release.
    pub(crate) fn shown(&self) -> Option<(Arc<Document>, u64)> {
        let p = self.preview.as_ref()?;
        if self.moving_baked_paste() {
            return None;
        }
        Some((p.shown.clone(), PREVIEW_KEY + p.keys.last()?.0))
    }

    /// [`Self::shown`], only on the document at the revision the strokes were painted on.
    pub(crate) fn shown_on(&self, doc: DocId, revision: u64) -> Option<(Arc<Document>, u64)> {
        if self.document != doc || self.doc_revision != revision {
            return None;
        }
        self.shown()
    }

    /// The document changed under the mode (a menu command, the Layers panel): paint the
    /// strokes over it as it is now, as OK will.
    fn rebase(&mut self, doc: &Arc<Document>, revision: u64) {
        if doc.id != self.document || revision == self.doc_revision {
            return;
        }
        if Arc::ptr_eq(doc, &self.base) {
            // A view change (layer selection, channel view): the same pixels, keep the preview.
            self.doc_revision = revision;
            return;
        }
        self.base = doc.clone();
        self.doc_revision = revision;
        self.canvas = doc.bounds();
        self.preview = None;
    }

    /// Image positions of the edge nodes of plane `i`.
    fn edge_nodes(&self, i: usize) -> Option<[[f64; 2]; 4]> {
        let h = self.planes.get(i)?.homography()?;
        let at = |u: f64, v: f64| {
            let (x, y) = h.apply(u, v);
            [x, y]
        };
        Some([at(0.5, 0.0), at(1.0, 0.5), at(0.5, 1.0), at(0.0, 0.5)])
    }

    fn press(&mut self, p: [f64; 2], tol: f64, mods: egui::Modifiers) -> Option<&'static str> {
        if self.tool == VpTool::Stamp {
            return self.stamp_press(p, mods.alt);
        }
        let near = |c: [f64; 2]| (c[0] - p[0]).hypot(c[1] - p[1]) <= tol;
        // The selected plane's nodes win over the others'.
        let order: Vec<usize> = self.selected.into_iter().chain((0..self.planes.len()).rev()).filter(|i| *i < self.planes.len()).collect();
        // Ctrl/Cmd on an edge node tears off a perpendicular plane (with either plane tool).
        if mods.command
            && let Some((i, e)) = order.iter().find_map(|&i| Some((i, self.edge_nodes(i)?.iter().position(|c| near(*c))?)))
        {
            self.tear_off(i, e, p);
            return None;
        }
        if self.tool == VpTool::CreatePlane {
            self.add_corner(p);
            return None;
        }
        for &i in &order {
            // A torn-off plane follows its parent: only its far edge node drags (depth, or
            // with Alt the angle).
            if let Some(t) = self.torn.get(i).copied().flatten() {
                if self.edge_nodes(i).and_then(|n| n.get(edge_index(t.edge)).copied()).is_some_and(near) {
                    self.selected = Some(i);
                    self.drag = Some(Drag::Torn { plane: i, rotate: mods.alt, start: p, moved: false });
                    return None;
                }
                continue;
            }
            if let Some(ci) = self.planes[i].corners.iter().position(|c| near(*c)) {
                let c = self.planes[i].corners[ci];
                self.selected = Some(i);
                self.drag = Some(Drag::Corner { plane: i, corner: ci, offset: [c[0] - p[0], c[1] - p[1]] });
                return None;
            }
            if let Some(e) = self.edge_nodes(i).and_then(|n| n.iter().position(|c| near(*c)))
                && let Some(h) = self.planes[i].homography()
                && let Some(inv) = h.inverse()
            {
                self.selected = Some(i);
                self.drag = Some(Drag::Edge { plane: i, edge: e, h, inv });
                return None;
            }
        }
        if let Some(i) = self.paste_plane()
            && let Some((h, lu, lv)) = self.frame(i)
            && let Some(ext) = self.paste_extent(lu, lv)
            && let Some(inv) = h.inverse()
        {
            let (u, v) = inv.apply(p[0], p[1]);
            let (a, b) = (self.paste_at, [self.paste_at[0] + ext[0], self.paste_at[1] + ext[1]]);
            if (a[0]..=b[0]).contains(&u) && (a[1]..=b[1]).contains(&v) {
                self.drag = Some(Drag::Paste { offset: [a[0] - u, a[1] - v] });
                return None;
            }
        }
        self.selected = self.scene().and_then(|sc| sc.plane_at(p));
        None
    }

    fn drag_to(&mut self, p: [f64; 2]) {
        match &mut self.drag {
            Some(Drag::Corner { plane, corner, offset }) => {
                if let Some(c) = self.planes.get_mut(*plane).and_then(|pl| pl.corners.get_mut(*corner)) {
                    *c = [p[0] + offset[0], p[1] + offset[1]];
                }
                self.resolve();
            }
            Some(Drag::Torn { plane, rotate, start, moved }) => {
                // A click without a drag leaves the plane as it is.
                *moved |= (p[0] - start[0]).hypot(p[1] - start[1]) > 2.0;
                if *moved {
                    let (i, r) = (*plane, *rotate);
                    self.fit_torn(i, r, p);
                }
            }
            Some(Drag::Stamp) => self.stamp_to(p),
            Some(Drag::Edge { plane, edge, h, inv }) => {
                let (u, v) = inv.apply(p[0], p[1]);
                // The plane's (u, v) rectangle after the move.
                let (u0, v0, u1, v1) = match edge {
                    0 => (0.0, v.min(0.99), 1.0, 1.0),
                    1 => (0.0, 0.0, u.max(0.01), 1.0),
                    2 => (0.0, 0.0, 1.0, v.max(0.01)),
                    _ => (u.min(0.99), 0.0, 1.0, 1.0),
                };
                if let Some(q) = slid_quad(h, u0, v0, u1, v1)
                    && let Some(pl) = self.planes.get_mut(*plane)
                {
                    pl.corners = q;
                }
                self.resolve();
            }
            Some(Drag::Paste { offset }) => {
                let offset = *offset;
                if let Some(i) = self.paste_plane()
                    && let Some(inv) = self.planes.get(i).and_then(VpPlane::homography).and_then(|h| h.inverse())
                {
                    let (u, v) = inv.apply(p[0], p[1]);
                    let at = [(u + offset[0]).clamp(-1.0, 2.0), (v + offset[1]).clamp(-1.0, 2.0)];
                    if at.iter().all(|x| x.is_finite()) {
                        self.paste_at = at;
                    }
                }
            }
            None => {}
        }
    }
}

/// Corners of the image of `[u0, u1] × [v0, v1]` under `h`, or `None` when part of it would lie
/// on or beyond the plane's horizon (where the edge can't follow the perspective).
fn slid_quad(h: &Homography, u0: f64, v0: f64, u1: f64, v1: f64) -> Option<[[f64; 2]; 4]> {
    let m = &h.0;
    let w = |u: f64, v: f64| m[6] * u + m[7] * v + m[8];
    let side = w(0.5, 0.5).signum();
    let uv = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
    if uv.iter().any(|c| w(c[0], c[1]) * side <= 1e-9 * w(0.5, 0.5).abs()) {
        return None;
    }
    let q = uv.map(|c| {
        let (x, y) = h.apply(c[0], c[1]);
        [x, y]
    });
    (VpPlane { corners: q }.check([0.0, 0.0]) != PlaneCheck::Invalid).then_some(q)
}

/// Starts Vanishing Point on the active pixel layer.
pub fn begin(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<(), String> {
    if app.ui.transform.is_some() {
        return Err("finish Free Transform first".into());
    }
    let st = app.session.active().ok_or("no document")?;
    let layer = st.active_layer.ok_or("no active layer")?;
    match st.doc.layer(layer).map(|l| &l.content) {
        Some(LayerContent::Raster(_)) => {}
        Some(other) => return Err(format!("Vanishing Point needs a pixel layer (active layer is {} {} layer)", other.article(), other.kind_name())),
        None => return Err("no active layer".into()),
    }
    let canvas = st.doc.bounds();
    let (base, doc_revision) = (st.doc.clone(), st.revision);
    let key = app.ui.alloc_id();
    let clip = app.session.clipboard.as_ref().and_then(|c| {
        let r = c.surface.content_bounds();
        (!r.is_empty()).then(|| Clip {
            surface: c.surface.clone(),
            rect: r,
            texture: ctx.load_texture(format!("vp-paste-{key}"), crate::distort_ui::surface_image(&c.surface, r, 1024), egui::TextureOptions::LINEAR),
            width: f64::from(r.width()),
            height: f64::from(r.height()),
        })
    });
    app.distort.vanishing = Some(VpSession {
        document: st.doc.id,
        layer,
        canvas,
        planes: Vec::new(),
        torn: Vec::new(),
        tool: VpTool::CreatePlane,
        pending: Vec::new(),
        selected: None,
        grid_size: 100.0,
        paste: clip.is_some(),
        paste_at: [0.25, 0.25],
        paste_width: 0.5,
        new_layer: false,
        clip,
        drag: None,
        stamp: StampOptions { diameter: 40.0, hardness: 50.0, opacity: 100.0, aligned: true },
        source: None,
        strokes: Vec::new(),
        aligned_ref: None,
        rev: 0,
        base,
        doc_revision,
        preview: None,
        history: EditHistory::new(&VpEdit {
            planes: Vec::new(),
            torn: Vec::new(),
            paste_at: [0.25, 0.25],
            paste_width: 0.5,
            source: None,
            strokes: Vec::new(),
            aligned_ref: None,
        }),
    });
    app.ui.status = tl!("Vanishing Point: click four corners to create a plane").into();
    Ok(())
}

/// OK: runs `filter.vanishingPoint` with the planes (and the paste and the clone strokes). An
/// invalid plane keeps the mode open; with nothing to paste or clone the mode just closes.
pub fn commit(app: &mut PhotocraftApp) -> Result<Value, String> {
    let Some(s) = app.distort.vanishing.as_ref() else { return Ok(Value::Null) };
    let center = s.center();
    if s.planes.iter().any(|p| p.check(center) == PlaneCheck::Invalid) {
        let msg = tl!("Fix the red plane first: its corners must form a convex quad");
        app.ui.status = msg.into();
        return Err(msg.into());
    }
    let mut params = s.params();
    // The layer the mode was opened on, when another one was picked meanwhile (the mode isn't
    // modal). Left out otherwise, so a recorded action stays portable.
    if let Some(st) = app.session.active()
        && st.active_layer != Some(s.layer)
        && st.doc.layer(s.layer).is_some()
    {
        params["layer"] = json!(s.layer.0);
    }
    let session = app.distort.vanishing.take();
    if params.get("paste").is_none() && params.get("clone").is_none() {
        return Ok(json!({"committed": false}));
    }
    match app.run(VP, params) {
        Ok(r) => Ok(json!({"committed": true, "result": r})),
        Err(e) => {
            // Keep the planes, source and strokes so the user can fix the cause and retry.
            app.distort.vanishing = session;
            Err(e)
        }
    }
}

/// Control channel: `filter.vanishingPoint {"ui": {...}}` while the mode is active.
pub fn control(app: &mut PhotocraftApp, ui: &Value) -> Result<Value, String> {
    if ui.get("commit").and_then(Value::as_bool) == Some(true) {
        return commit(app);
    }
    if ui.get("cancel").and_then(Value::as_bool) == Some(true) {
        app.distort.vanishing = None;
        return Ok(json!({"cancelled": true}));
    }
    let s = app.distort.vanishing.as_mut().ok_or(tl!("Vanishing Point is not active"))?;
    if let Some(t) = ui.get("tool").and_then(Value::as_str) {
        s.tool = match t {
            "edit" | "editPlane" => VpTool::EditPlane,
            "create" | "createPlane" => VpTool::CreatePlane,
            "stamp" => VpTool::Stamp,
            other => return Err(format!("tool: edit|create|stamp, not `{other}`")),
        };
        s.pending.clear();
    }
    if let Some(p) = ui.get("planes") {
        let (planes, torn) = parse_planes(p)?;
        s.planes = planes;
        s.torn = torn;
        // A drag holds indices into the old list.
        s.drag = None;
        s.selected = (!s.planes.is_empty()).then(|| s.planes.len() - 1);
        s.resolve();
    }
    if let Some(o) = ui.get("stamp") {
        let num = |k: &str, lo: f64, hi: f64| o.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| v.clamp(lo, hi));
        let st = &mut s.stamp;
        st.diameter = num("diameter", 1.0, 2000.0).unwrap_or(st.diameter);
        st.hardness = num("hardness", 0.0, 100.0).unwrap_or(st.hardness);
        st.opacity = num("opacity", 0.0, 100.0).unwrap_or(st.opacity);
        st.aligned = o.get("aligned").and_then(Value::as_bool).unwrap_or(st.aligned);
    }
    if let Some(src) = ui.get("source") {
        s.source = match src {
            Value::Null => None,
            v => Some(pt(v).ok_or("source: [x, y]")?),
        };
        s.aligned_ref = None;
    }
    if let Some(c) = ui.get("clone") {
        s.strokes = photocraft_engine::vp_cmds::clone_specs(&json!({"clone": c})).map_err(|e| e.to_string())?;
        s.drag = None;
        s.rev += 1;
    }
    if let Some(i) = ui.get("select").and_then(Value::as_u64) {
        s.selected = usize::try_from(i).ok().filter(|i| *i < s.planes.len());
    }
    if let Some(g) = ui.get("gridSize").and_then(Value::as_f64).filter(|g| g.is_finite()) {
        s.grid_size = g.clamp(2.0, 2000.0);
    }
    if let Some(b) = ui.get("paste").and_then(Value::as_bool) {
        s.paste = b;
    }
    if let Some(at) = ui.get("pasteAt").and_then(|v| Some([v.get(0)?.as_f64()?, v.get(1)?.as_f64()?])) {
        s.paste_at = at.map(|x| if x.is_finite() { x.clamp(-1.0, 2.0) } else { 0.25 });
    }
    if let Some(w) = ui.get("pasteWidth").and_then(Value::as_f64).filter(|w| w.is_finite()) {
        s.paste_width = w.clamp(0.01, 10.0);
    }
    if let Some(b) = ui.get("newLayer").and_then(Value::as_bool) {
        s.new_layer = b;
    }
    s.settle(false);
    sync_preview(app);
    Ok(app.distort.vanishing.as_ref().map(VpSession::describe).unwrap_or(Value::Null))
}

fn pt(v: &Value) -> Option<[f64; 2]> {
    let p = [v.get(0)?.as_f64()?, v.get(1)?.as_f64()?];
    p.iter().all(|x| x.is_finite() && x.abs() < 1e7).then_some(p)
}

/// The control channel's planes, in the command's form: `{"corners": [[x, y]×4]}` or
/// `{"from": earlier plane, "edge", "angle": 90, "depth": 1}`.
fn parse_planes(v: &Value) -> Result<(Vec<VpPlane>, Vec<Option<Torn>>), String> {
    let list = v.as_array().ok_or("planes: a list")?;
    if list.len() > MAX_PLANES {
        return Err(format!("at most {MAX_PLANES} planes"));
    }
    let (mut planes, mut torn) = (Vec::new(), Vec::new());
    for (i, p) in list.iter().enumerate() {
        if let Some(c) = p.get("corners") {
            let c = c.as_array().filter(|c| c.len() == 4).ok_or(format!("plane {i}: four corners"))?;
            let corners = [0, 1, 2, 3].map(|k| c.get(k).and_then(pt));
            let [Some(a), Some(b), Some(c), Some(d)] = corners else { return Err(format!("plane {i}: corners are [x, y]")) };
            planes.push(VpPlane { corners: [a, b, c, d] });
            torn.push(None);
            continue;
        }
        let from = p
            .get("from")
            .and_then(Value::as_u64)
            .and_then(|f| usize::try_from(f).ok())
            .filter(|f| *f < i)
            .ok_or(format!("plane {i}: `corners` or `from` (an earlier plane)"))?;
        let edge = match p.get("edge").and_then(Value::as_str).unwrap_or("top") {
            "top" => Edge::Top,
            "right" => Edge::Right,
            "bottom" => Edge::Bottom,
            "left" => Edge::Left,
            e => return Err(format!("plane {i}: unknown edge `{e}`")),
        };
        let angle = p.get("angle").and_then(Value::as_f64).unwrap_or(90.0);
        let depth = p.get("depth").and_then(Value::as_f64).unwrap_or(1.0);
        if !(0.0..=360.0).contains(&angle) || !(depth > 0.0 && depth <= 1000.0) {
            return Err(format!("plane {i}: angle 0..360, depth above 0 and at most 1000"));
        }
        planes.push(VpPlane { corners: [[0.0; 2]; 4] });
        torn.push(Some(Torn { from, edge, angle, depth }));
    }
    Ok((planes, torn))
}

/// Brings the stamp preview up to date with the strokes (and repaints it after the planes
/// changed, once no plane is being dragged).
pub(crate) fn sync_preview(app: &mut PhotocraftApp) {
    let Some(s) = app.distort.vanishing.as_mut() else { return };
    if let Some(st) = app.session.active() {
        s.rebase(&st.doc, st.revision);
    }
    if matches!(s.drag, Some(Drag::Corner { .. } | Drag::Edge { .. } | Drag::Torn { .. } | Drag::Paste { .. })) {
        return;
    }
    let ui = &mut app.ui;
    s.paint_preview(|| ui.alloc_id());
}

/// What changed between stamp preview (or document) key `seen` and `now` of document `doc` at
/// `revision`: the areas the strokes painted. `None` when either key isn't the session's.
pub(crate) fn damage(app: &PhotocraftApp, doc: DocId, revision: u64, seen: u64, now: u64) -> Option<Rect> {
    let s = app.distort.vanishing.as_ref().filter(|s| s.base.id == doc && s.doc_revision == revision)?;
    let keys = &s.preview.as_ref()?.keys;
    let at = |k: u64| if k == 0 { Some(0) } else { keys.iter().position(|e| PREVIEW_KEY + e.0 == k).map(|i| i + 1) };
    let (a, b) = (at(seen)?, at(now)?);
    if a == b {
        return None;
    }
    Some(keys.get(a.min(b)..a.max(b))?.iter().fold(Rect::EMPTY, |r, k| union(r, k.1)))
}

pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) {
    let tol = crate::distort_ui::tolerance(app);
    let Some(s) = app.distort.vanishing.as_mut() else { return };
    match ev {
        ToolEvent::Down { x, y, .. } => {
            if let Some(msg) = s.press([x, y], tol, mods) {
                app.ui.status = msg.into();
            }
        }
        ToolEvent::Move { x, y, .. } => s.drag_to([x, y]),
        ToolEvent::Up { x, y } => {
            s.drag_to([x, y]);
            s.drag = None;
        }
    }
    s.settle(false);
    sync_preview(app);
}

/// Keys while the mode is active (↩ and Esc are handled by `distort_ui::keys`).
pub fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) {
    use egui::{Key, Modifiers};
    if ctx.memory(|m| m.focused().is_some()) {
        return;
    }
    let Some(s) = app.distort.vanishing.as_mut() else { return };
    let key = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
    if key(Key::V) {
        s.tool = VpTool::EditPlane;
        s.pending.clear();
    } else if key(Key::C) {
        s.tool = VpTool::CreatePlane;
    } else if key(Key::S) {
        s.tool = VpTool::Stamp;
        s.pending.clear();
    } else if key(Key::Delete) || key(Key::Backspace) {
        s.delete_selected();
        s.settle(false);
    }
    sync_preview(app);
}

/// Esc: drops a half-clicked plane first, then leaves the mode.
pub fn escape(app: &mut PhotocraftApp) {
    match app.distort.vanishing.as_mut() {
        Some(s) if !s.pending.is_empty() => s.pending.clear(),
        _ => app.distort.vanishing = None,
    }
}

/// The paste on its plane: image content, drawn under the grid and guides like the document so a
/// guide stays visible across it (#7781). The planes and their grids are [`draw`]. A current
/// stamp preview already shows the paste under its strokes.
pub fn draw_content(s: &VpSession, painter: &egui::Painter, xf: &ViewXform) {
    let scr = |p: [f64; 2]| xf.to_screen(p[0] as f32, p[1] as f32);
    if s.paste_overlay()
        && let Some(i) = s.paste_plane()
        && s.planes.get(i).is_some_and(|p| p.check(s.center()) != PlaneCheck::Invalid)
        && let (Some((h, lu, lv)), Some(c)) = (s.frame(i), s.clip.as_ref())
        && let Some(ext) = s.paste_extent(lu, lv)
    {
        draw_paste(painter, &scr, &h, s.paste_at, ext, &c.texture);
    }
}

/// The planes, their grids and nodes over the paste ([`draw_content`]) and the guides.
pub fn draw(s: &VpSession, painter: &egui::Painter, xf: &ViewXform) {
    let scr = |p: [f64; 2]| xf.to_screen(p[0] as f32, p[1] as f32);
    let tokens = crate::theme::Tokens::get(painter.ctx());
    let center = s.center();
    for (i, p) in s.planes.iter().enumerate() {
        let check = p.check(center);
        let colour = match check {
            PlaneCheck::Valid => tokens.accent,
            PlaneCheck::Questionable => tokens.warning,
            PlaneCheck::Invalid => tokens.danger,
        };
        let selected = s.selected == Some(i);
        // An invalid plane has no perspective to grid: just its red outline, as in Photoshop.
        if check != PlaneCheck::Invalid
            && let Some((h, lu, lv)) = s.frame(i)
        {
            let thin = Stroke::new(0.75, colour.gamma_multiply(0.6));
            let at = |u: f64, v: f64| {
                let (x, y) = h.apply(u, v);
                scr([x, y])
            };
            for u in grid_steps(p, lu, lv, s.grid_size, true) {
                painter.line_segment([at(u, 0.0), at(u, 1.0)], thin);
            }
            for v in grid_steps(p, lu, lv, s.grid_size, false) {
                painter.line_segment([at(0.0, v), at(1.0, v)], thin);
            }
        }
        let pts: Vec<Pos2> = p.corners.iter().map(|c| scr(*c)).collect();
        painter.add(egui::Shape::closed_line(pts.clone(), Stroke::new(if selected { 2.0 } else { 1.25 }, colour)));
        if selected {
            let nodes = pts.into_iter().chain(s.edge_nodes(i).into_iter().flatten().map(scr));
            for c in nodes {
                let r = egui::Rect::from_center_size(c, egui::vec2(7.0, 7.0));
                painter.rect_filled(r, 0.0, Color32::WHITE);
                painter.rect_stroke(r, 0.0, Stroke::new(1.25, colour), egui::StrokeKind::Inside);
            }
        }
    }
    // Create Plane: the corners so far and a rubber band to the pointer.
    if !s.pending.is_empty() {
        let mut pts: Vec<Pos2> = s.pending.iter().map(|c| scr(*c)).collect();
        if let Some(hover) = painter.ctx().pointer_hover_pos() {
            pts.push(hover);
        }
        painter.add(egui::Shape::line(pts.clone(), Stroke::new(1.25, tokens.accent)));
        for c in pts.iter().take(s.pending.len()) {
            painter.rect_filled(egui::Rect::from_center_size(*c, egui::vec2(6.0, 6.0)), 0.0, tokens.accent);
        }
    }
    if s.tool == VpTool::Stamp {
        draw_stamp(s, painter, xf, &scr);
    }
}

/// Stamp: the source as a crosshair and the brush under the pointer as the circle it paints on
/// the plane, foreshortened.
fn draw_stamp(s: &VpSession, painter: &egui::Painter, xf: &ViewXform, scr: &dyn Fn([f64; 2]) -> Pos2) {
    let outline = |pts: &[Pos2]| {
        painter.add(egui::Shape::closed_line(pts.to_vec(), Stroke::new(2.5, Color32::from_black_alpha(160))));
        painter.add(egui::Shape::closed_line(pts.to_vec(), Stroke::new(1.0, Color32::WHITE)));
    };
    if let Some(src) = s.source {
        let c = scr(src);
        for (a, b) in [(egui::vec2(-7.0, 0.0), egui::vec2(7.0, 0.0)), (egui::vec2(0.0, -7.0), egui::vec2(0.0, 7.0))] {
            painter.line_segment([c + a, c + b], Stroke::new(2.5, Color32::from_black_alpha(160)));
            painter.line_segment([c + a, c + b], Stroke::new(1.0, Color32::WHITE));
        }
    }
    let Some(hover) = painter.ctx().pointer_hover_pos() else { return };
    let p = xf.to_doc(hover);
    let Some(sc) = s.scene() else { return };
    let Some(i) = sc.plane_at(p) else { return };
    let Some((h, lu, lv)) = s.frame(i) else { return };
    let Some(inv) = h.inverse() else { return };
    let metric = |q: [f64; 2]| {
        let (u, v) = inv.apply(q[0], q[1]);
        [u * lu, v * lv]
    };
    let (m, probe) = (metric(p), metric([p[0] + s.stamp.diameter / 2.0, p[1]]));
    let r = (probe[0] - m[0]).hypot(probe[1] - m[1]);
    if !r.is_finite() || r <= 0.0 {
        return;
    }
    let ring: Vec<Pos2> = (0..48)
        .map(|k| {
            let a = f64::from(k) / 48.0 * std::f64::consts::TAU;
            let (x, y) = h.apply((m[0] + r * a.cos()) / lu, (m[1] + r * a.sin()) / lv);
            scr([x, y])
        })
        .collect();
    if ring.iter().all(|q| q.is_finite()) {
        outline(&ring);
    }
}

/// Plane coordinates (0..1, exclusive) of the grid lines across `u` (or `v`) for cells of
/// `size` px at the plane's nearest edge, square in the plane's metric.
fn grid_steps(p: &VpPlane, lu: f64, lv: f64, size: f64, along_u: bool) -> Vec<f64> {
    let q = &p.corners;
    let d = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    // Image px per metric unit at the nearest (longest-looking) edge.
    let k = [d(q[0], q[1]) / lu, d(q[3], q[2]) / lu, d(q[0], q[3]) / lv, d(q[1], q[2]) / lv].into_iter().fold(0.0, f64::max);
    let cell = size.max(1.0) / k;
    let len = if along_u { lu } else { lv };
    if !cell.is_finite() || cell <= 0.0 || !len.is_finite() {
        return Vec::new();
    }
    let step = cell / len;
    (1..MAX_LINES).map(|j| j as f64 * step).take_while(|t| *t < 1.0 - 1e-9).collect()
}

/// The clipboard image on the plane: a mesh through the plane's homography, cells outside the
/// plane left out (the engine clips the paste to the plane).
fn draw_paste(painter: &egui::Painter, scr: &dyn Fn([f64; 2]) -> Pos2, h: &Homography, at: [f64; 2], ext: [f64; 2], tex: &TextureHandle) {
    const N: usize = 24;
    let mut mesh = egui::Mesh::with_texture(tex.id());
    let tint = Color32::from_white_alpha(230);
    for j in 0..=N {
        for i in 0..=N {
            let (s, t) = (i as f64 / N as f64, j as f64 / N as f64);
            let (x, y) = h.apply(at[0] + s * ext[0], at[1] + t * ext[1]);
            mesh.vertices.push(egui::epaint::Vertex { pos: scr([x, y]), uv: pos2(s as f32, t as f32), color: tint });
        }
    }
    for j in 0..N {
        for i in 0..N {
            let (u, v) = (at[0] + (i as f64 + 0.5) / N as f64 * ext[0], at[1] + (j as f64 + 0.5) / N as f64 * ext[1]);
            if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
                continue;
            }
            let a = (j * (N + 1) + i) as u32;
            mesh.add_triangle(a, a + 1, a + N as u32 + 2);
            mesh.add_triangle(a, a + N as u32 + 2, a + N as u32 + 1);
        }
    }
    painter.add(mesh);
}

pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    // New Layer toggled last frame repaints the stamp preview.
    sync_preview(app);
    let Some(s) = app.distort.vanishing.as_mut() else { return };
    if crate::widgets::pill_tab(ui, tl!("Edit Plane"), s.tool == VpTool::EditPlane).on_hover_text("V").clicked() {
        s.tool = VpTool::EditPlane;
        s.pending.clear();
    }
    if crate::widgets::pill_tab(ui, tl!("Create Plane"), s.tool == VpTool::CreatePlane).on_hover_text("C").clicked() {
        s.tool = VpTool::CreatePlane;
    }
    if crate::widgets::pill_tab(ui, tl!("Stamp"), s.tool == VpTool::Stamp).on_hover_text("S").clicked() {
        s.tool = VpTool::Stamp;
        s.pending.clear();
    }
    crate::widgets::vline(ui, 22.0);
    if s.tool == VpTool::Stamp {
        let st = &mut s.stamp;
        for (label, value, range, unit) in [
            (tl!("Diameter:"), &mut st.diameter, 1.0..=2000.0, "px"),
            (tl!("Hardness:"), &mut st.hardness, 0.0..=100.0, "%"),
            (tl!("Opacity:"), &mut st.opacity, 0.0..=100.0, "%"),
        ] {
            ui.label(label);
            let mut v = *value as f32;
            if crate::widgets::value_field(ui, &mut v, range, unit, 56.0).changed() {
                *value = f64::from(v);
            }
        }
        crate::widgets::checkbox(ui, &mut st.aligned, tl!("Aligned"));
        crate::widgets::vline(ui, 22.0);
    }
    ui.label(tl!("Grid Size:"));
    let mut g = s.grid_size as f32;
    if crate::widgets::value_field(ui, &mut g, 2.0..=2000.0, "px", 56.0).accessible_name(tl!("Grid Size:")).changed() {
        s.grid_size = f64::from(g);
    }
    if s.clip.is_some() {
        crate::widgets::vline(ui, 22.0);
        crate::widgets::checkbox(ui, &mut s.paste, tl!("Paste"));
        if s.paste {
            ui.label(tl!("Width:"));
            let mut w = (s.paste_width * 100.0) as f32;
            if crate::widgets::value_field(ui, &mut w, 1.0..=1000.0, "%", 56.0).accessible_name(tl!("Width:")).changed() {
                s.paste_width = f64::from(w) / 100.0;
            }
        }
    }
    if s.clip.is_some() || !s.strokes.is_empty() || s.tool == VpTool::Stamp {
        crate::widgets::checkbox(ui, &mut s.new_layer, tl!("New Layer"));
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if crate::widgets::primary_button(ui, "✓", 32.0)
            .on_hover_text(crate::i18n::fmt(tl!("Apply Vanishing Point ({key})"), &[("key", &crate::shortcuts::pretty("Enter"))]))
            .clicked()
        {
            let _ = commit(app);
        } else if crate::widgets::secondary_button(ui, "⊘", 32.0).on_hover_text(tl!("Cancel (Esc)")).clicked() {
            app.distort.vanishing = None;
        }
    });
    if let Some(s) = app.distort.vanishing.as_mut() {
        s.settle(ui.input(|i| i.pointer.any_down()));
    }
    // A paste Width or Paste change shows in this frame's canvas, the paste under the strokes.
    sync_preview(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_layer() -> (egui::Context, PhotocraftApp) {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width": 240, "height": 160, "depth": 16})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session.execute("edit.fill", json!({"color": "#4080c0"})).unwrap();
        app.sync_views();
        (ctx, app)
    }

    #[test]
    fn four_clicks_make_a_plane_and_ok_pastes_in_one_step() {
        let (ctx, mut app) = app_with_layer();
        // Something to paste: a red square on the clipboard.
        app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
        app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.session.execute("edit.copy", json!({})).unwrap();
        app.session.execute("select.deselect", json!({})).unwrap();
        crate::distort_ui::menu(&mut app, &ctx, VP, &json!({})).unwrap().unwrap();
        let click = |app: &mut PhotocraftApp, x: f64, y: f64| {
            crate::distort_ui::pointer(app, ToolEvent::Down { x, y, pressure: 1.0 }, egui::Modifiers::NONE);
            crate::distort_ui::pointer(app, ToolEvent::Up { x, y }, egui::Modifiers::NONE);
        };
        // A floor receding to the top, clicked counter-clockwise from the bottom-left.
        for (x, y) in [(20.0, 150.0), (220.0, 150.0), (170.0, 60.0), (70.0, 60.0)] {
            click(&mut app, x, y);
        }
        let s = app.distort.vanishing.as_ref().unwrap();
        assert_eq!(s.planes.len(), 1);
        assert_eq!(s.planes[0].corners, [[70.0, 60.0], [170.0, 60.0], [220.0, 150.0], [20.0, 150.0]], "clockwise from the top-left");
        assert_eq!(s.tool, VpTool::EditPlane);
        assert_eq!(s.describe()["planes"][0]["check"], "valid");
        // Edit Plane: drag the top edge node up; the edge slides along the perspective.
        let top = s.edge_nodes(0).unwrap()[0];
        crate::distort_ui::pointer(&mut app, ToolEvent::Down { x: top[0], y: top[1], pressure: 1.0 }, egui::Modifiers::NONE);
        crate::distort_ui::pointer(&mut app, ToolEvent::Up { x: top[0], y: top[1] - 15.0 }, egui::Modifiers::NONE);
        let c = app.distort.vanishing.as_ref().unwrap().planes[0].corners;
        assert!(c[0][1] < 60.0 && c[1][1] < 60.0 && c[1][0] - c[0][0] < 100.0, "taller and narrower at the far edge: {c:?}");
        assert_eq!(c[2], [220.0, 150.0]);
        let before = app.session.active().unwrap().history.past_len();
        commit(&mut app).unwrap();
        assert!(app.distort.vanishing.is_none());
        assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
        // The paste landed on the plane: red at its middle, blue outside it.
        let st = app.session.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        let h = VpPlane { corners: c }.homography().unwrap();
        let (x, y) = h.apply(0.5, 0.35);
        assert!(l.rgba(x as i32, y as i32)[0] > 0.9, "{:?}", l.rgba(x as i32, y as i32));
        assert!(l.rgba(230, 20)[0] < 0.5, "outside the plane");
    }

    #[test]
    fn ctrl_drag_tears_off_a_wall_and_the_stamp_clones_in_one_step() {
        let (ctx, mut app) = app_with_layer();
        // A green mark near the floor's front edge to clone.
        app.session.execute("select.rect", json!({"x": 100, "y": 135, "width": 40, "height": 8})).unwrap();
        app.session.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
        app.session.execute("select.deselect", json!({})).unwrap();
        crate::distort_ui::menu(&mut app, &ctx, VP, &json!({})).unwrap().unwrap();
        control(&mut app, &json!({"planes": [{"corners": [[70, 60], [170, 60], [220, 150], [20, 150]]}]})).unwrap();
        let ev = |app: &mut PhotocraftApp, e: ToolEvent, m: egui::Modifiers| crate::distort_ui::pointer(app, e, m);
        let drag = |app: &mut PhotocraftApp, a: [f64; 2], b: [f64; 2], m: egui::Modifiers| {
            ev(app, ToolEvent::Down { x: a[0], y: a[1], pressure: 1.0 }, m);
            ev(app, ToolEvent::Move { x: (a[0] + b[0]) / 2.0, y: (a[1] + b[1]) / 2.0, pressure: 1.0 }, m);
            ev(app, ToolEvent::Up { x: b[0], y: b[1] }, m);
        };
        fn vp(app: &PhotocraftApp) -> &VpSession {
            app.distort.vanishing.as_ref().unwrap()
        }

        // Ctrl/Cmd-drag the top edge node up: a wall torn off the floor's far edge, 90° to it.
        let top = vp(&app).edge_nodes(0).unwrap()[0];
        drag(&mut app, top, [top[0], top[1] - 35.0], egui::Modifiers::COMMAND);
        let s = vp(&app);
        assert_eq!(s.planes.len(), 2);
        let t = s.torn[1].unwrap();
        assert_eq!((t.from, t.edge, t.angle), (0, Edge::Top, 90.0));
        let c = s.planes[1].corners;
        assert!((c[0][1] + c[1][1]) / 2.0 < 40.0, "the far edge followed the pointer up: {c:?}");
        assert_ne!(s.describe()["planes"][1]["check"], "invalid");
        assert_eq!(s.params()["planes"][1], json!({"from": 0, "edge": "top", "angle": 90.0, "depth": t.depth}));
        // Alt-dragging its far edge node turns it; editing the floor carries it along.
        let far = s.edge_nodes(1).unwrap()[0];
        drag(&mut app, far, [far[0], far[1] + 12.0], egui::Modifiers::ALT);
        assert_ne!(vp(&app).torn[1].unwrap().angle, 90.0);
        let before = vp(&app).planes[1].corners;
        drag(&mut app, [220.0, 150.0], [228.0, 150.0], egui::Modifiers::NONE);
        assert_ne!(vp(&app).planes[1].corners, before);
        assert!(vp(&app).torn[1].is_some());

        // Stamp: painting needs a source first; Alt-click sets it on the floor's green mark.
        control(&mut app, &json!({"tool": "stamp", "stamp": {"diameter": 12, "hardness": 80}})).unwrap();
        drag(&mut app, [120.0, 100.0], [120.0, 100.0], egui::Modifiers::NONE);
        assert!(vp(&app).strokes.is_empty() && app.ui.status.contains("Alt-click"), "{}", app.ui.status);
        drag(&mut app, [120.0, 139.0], [120.0, 139.0], egui::Modifiers::ALT);
        assert_eq!(vp(&app).source, Some([120.0, 139.0]));
        drag(&mut app, [118.0, 100.0], [126.0, 100.0], egui::Modifiers::NONE);
        assert_eq!(vp(&app).strokes.len(), 1);
        assert!(vp(&app).strokes[0].points.len() >= 3, "dabs along the drag");
        // The canvas shows the stroke; the document is untouched until OK.
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let green = |d: &Document| d.layer(layer).unwrap().surface().unwrap().rgba(122, 100)[1];
        let (shown, key) = crate::distort_ui::display_doc(&app, 0).expect("a stamp preview");
        assert!(green(&shown) > 0.8, "{}", green(&shown));
        let st = app.session.active().unwrap();
        assert!(green(&st.doc) < 0.6);
        let area = damage(&app, st.doc.id, st.revision, 0, key).unwrap();
        assert!(area.contains(122, 100) && !area.contains(20, 20), "only the stroke recomposites: {area:?}");

        let before = app.session.active().unwrap().history.past_len();
        let r = commit(&mut app).unwrap();
        assert_eq!(r["committed"], true, "{r}");
        let st = app.session.active().unwrap();
        assert_eq!(st.history.past_len(), before + 1, "one history step for the whole session");
        assert!(green(&st.doc) > 0.8, "{}", green(&st.doc));
    }

    #[test]
    fn undo_steps_back_through_clone_strokes_and_tear_offs() {
        let (ctx, mut app) = app_with_layer();
        crate::distort_ui::menu(&mut app, &ctx, VP, &json!({})).unwrap().unwrap();
        control(&mut app, &json!({"planes": [{"corners": [[70, 60], [170, 60], [220, 150], [20, 150]]}]})).unwrap();
        let drag = |app: &mut PhotocraftApp, a: [f64; 2], b: [f64; 2], m: egui::Modifiers| {
            crate::distort_ui::pointer(app, ToolEvent::Down { x: a[0], y: a[1], pressure: 1.0 }, m);
            crate::distort_ui::pointer(app, ToolEvent::Move { x: b[0], y: b[1], pressure: 1.0 }, m);
            crate::distort_ui::pointer(app, ToolEvent::Up { x: b[0], y: b[1] }, m);
        };
        let step = |app: &mut PhotocraftApp, st: Step| {
            app.distort.vanishing.as_mut().unwrap().step(st);
            sync_preview(app);
        };
        let top = app.distort.vanishing.as_ref().unwrap().edge_nodes(0).unwrap()[0];
        drag(&mut app, top, [top[0], top[1] - 35.0], egui::Modifiers::COMMAND);
        control(&mut app, &json!({"tool": "stamp", "source": [120, 139]})).unwrap();
        drag(&mut app, [118.0, 100.0], [126.0, 100.0], egui::Modifiers::NONE);
        assert_eq!(app.distort.vanishing.as_ref().unwrap().strokes.len(), 1);
        assert!(crate::distort_ui::display_doc(&app, 0).is_some());

        // ⌘Z takes the stroke back (and its preview), then the source, then the torn-off wall.
        step(&mut app, Step::Undo);
        assert!(app.distort.vanishing.as_ref().unwrap().strokes.is_empty());
        assert!(crate::distort_ui::display_doc(&app, 0).is_none(), "no stamp preview left");
        step(&mut app, Step::Undo);
        assert_eq!(app.distort.vanishing.as_ref().unwrap().source, None);
        step(&mut app, Step::Undo);
        let s = app.distort.vanishing.as_ref().unwrap();
        assert_eq!((s.planes.len(), s.torn.len()), (1, 1));
        // ⇧⌘Z brings them back.
        for _ in 0..3 {
            step(&mut app, Step::Redo);
        }
        let s = app.distort.vanishing.as_ref().unwrap();
        assert_eq!((s.planes.len(), s.strokes.len()), (2, 1));
        assert!(s.torn[1].is_some());
        assert!(crate::distort_ui::display_doc(&app, 0).is_some(), "the stroke shows again");
    }
    #[test]
    fn overlapping_stamp_preview_matches_commit_across_batches() {
        for new_layer in [false, true] {
            let (ctx, mut app) = app_with_layer();
            app.session
                .edit("pattern", |doc, active| {
                    let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                    for x in 0..240 {
                        surf.fill_rect(Rect::new(x, 0, x + 1, 160), &[x as f32 / 240.0, (x % 7) as f32 / 6.0, 0.3, 1.0]);
                    }
                    Ok(())
                })
                .unwrap();
            crate::distort_ui::menu(&mut app, &ctx, VP, &json!({})).unwrap().unwrap();
            control(&mut app, &json!({"planes": [{"corners": [[10, 10], [230, 10], [230, 150], [10, 150]]}]})).unwrap();
            let points = [[80.0, 80.0], [84.0, 80.0], [88.0, 80.0], [92.0, 80.0]];
            let s = app.distort.vanishing.as_mut().unwrap();
            s.new_layer = new_layer;
            s.strokes.push(CloneSpec { source: [76.0, 80.0], points: points[..2].to_vec(), size: 12.0, hardness: 0.8, opacity: 1.0 });
            assert!(s.paint_preview(|| 1));
            s.strokes[0].points.extend_from_slice(&points[2..]);
            assert!(s.paint_preview(|| 2));
            let (shown, _) = s.shown().unwrap();
            let expected = photocraft_compose::flatten(&shown).px;
            assert_ne!(expected, photocraft_compose::flatten(&app.session.active().unwrap().doc).px);
            commit(&mut app).unwrap();
            assert_eq!(photocraft_compose::flatten(&app.session.active().unwrap().doc).px, expected, "newLayer={new_layer}");
        }
    }

    /// A red square on the clipboard (copied from the layer's top-left corner).
    fn copy_red_square(app: &mut PhotocraftApp) {
        app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap();
        app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.session.execute("edit.copy", json!({})).unwrap();
        app.session.execute("select.deselect", json!({})).unwrap();
    }

    #[test]
    fn stamp_preview_paints_over_the_paste_like_ok() {
        for new_layer in [false, true] {
            let (ctx, mut app) = app_with_layer();
            copy_red_square(&mut app);
            crate::distort_ui::menu(&mut app, &ctx, VP, &json!({})).unwrap().unwrap();
            control(&mut app, &json!({"planes": [{"corners": [[10, 10], [230, 10], [230, 150], [10, 150]]}], "newLayer": new_layer})).unwrap();
            let s = app.distort.vanishing.as_ref().unwrap();
            assert!(s.paste && s.paste_plane() == Some(0), "the clipboard pastes by default");
            // The paste covers x 65..175 from y 45 down. One stroke clones it onto the base
            // layer, the other paints across its right edge.
            control(
                &mut app,
                &json!({"clone": [
                    {"source": [100, 60], "points": [[40, 120], [48, 120], [56, 120]], "size": 16, "hardness": 80},
                    {"source": [40, 130], "points": [[150, 60], [170, 60], [190, 60]], "size": 16, "hardness": 80},
                ]}),
            )
            .unwrap();
            let s = app.distort.vanishing.as_ref().unwrap();
            assert!(s.preview_has_paste(), "the preview shows the paste, so the overlay isn't drawn over the strokes");
            let (shown, _) = s.shown().unwrap();
            let expected = photocraft_compose::flatten(&shown).px;
            commit(&mut app).unwrap();
            assert_eq!(photocraft_compose::flatten(&app.session.active().unwrap().doc).px, expected, "newLayer={new_layer}");
        }
    }

    #[test]
    fn moving_the_paste_repaints_the_stamp_preview() {
        let (ctx, mut app) = app_with_layer();
        copy_red_square(&mut app);
        crate::distort_ui::menu(&mut app, &ctx, VP, &json!({})).unwrap().unwrap();
        control(&mut app, &json!({"planes": [{"corners": [[10, 10], [230, 10], [230, 150], [10, 150]]}]})).unwrap();
        control(&mut app, &json!({"clone": [{"source": [200, 130], "points": [[30, 130]], "size": 8}]})).unwrap();
        let red = |app: &PhotocraftApp, x: i32, y: i32| {
            let (shown, _) = app.distort.vanishing.as_ref().unwrap().shown().unwrap();
            let l = app.session.active().unwrap().active_layer.unwrap();
            shown.layer(l).unwrap().surface().unwrap().rgba(x, y)[0]
        };
        assert!(red(&app, 100, 60) > 0.9);
        control(&mut app, &json!({"pasteAt": [0.0, 0.5]})).unwrap();
        assert!(red(&app, 100, 60) < 0.5, "the paste left its old place");
        assert!(red(&app, 30, 100) > 0.9, "and shows at the new one");
        control(&mut app, &json!({"paste": false})).unwrap();
        assert!(red(&app, 30, 100) < 0.5, "paste off: gone from the preview");
        assert!(!app.distort.vanishing.as_ref().unwrap().preview_has_paste());
    }

    #[test]
    fn dragging_the_paste_shows_it_once() {
        let (ctx, mut app) = app_with_layer();
        copy_red_square(&mut app);
        crate::distort_ui::menu(&mut app, &ctx, VP, &json!({})).unwrap().unwrap();
        control(&mut app, &json!({"planes": [{"corners": [[10, 10], [230, 10], [230, 150], [10, 150]]}], "tool": "edit"})).unwrap();
        control(&mut app, &json!({"clone": [{"source": [200, 130], "points": [[30, 130]], "size": 8}]})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let red = |app: &PhotocraftApp, x: i32, y: i32| {
            let (shown, _) = crate::distort_ui::display_doc(app, 0)?;
            Some(shown.layer(layer).unwrap().surface().unwrap().rgba(x, y)[0])
        };
        assert!(red(&app, 100, 60).unwrap() > 0.9, "the paste is in the preview");
        let ev = |app: &mut PhotocraftApp, e: ToolEvent| crate::distort_ui::pointer(app, e, egui::Modifiers::NONE);
        ev(&mut app, ToolEvent::Down { x: 100.0, y: 60.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Move { x: 60.0, y: 100.0, pressure: 1.0 });
        // Mid-drag: no copy left behind at the old place, only the overlay at the pointer.
        assert!(red(&app, 100, 60).is_none_or(|r| r < 0.5), "{:?}", red(&app, 100, 60));
        assert!(app.distort.vanishing.as_ref().unwrap().paste_overlay(), "the overlay draws the moving paste");
        ev(&mut app, ToolEvent::Up { x: 60.0, y: 100.0 });
        assert!(red(&app, 100, 60).unwrap() < 0.5);
        assert!(red(&app, 60, 100).unwrap() > 0.9, "released: the preview pastes at the new place");
        assert!(!app.distort.vanishing.as_ref().unwrap().paste_overlay());
        // A plane corner drag freezes the preview, paste included, until release: one copy.
        ev(&mut app, ToolEvent::Down { x: 230.0, y: 150.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Move { x: 236.0, y: 156.0, pressure: 1.0 });
        let s = app.distort.vanishing.as_ref().unwrap();
        assert!(matches!(s.drag, Some(Drag::Corner { .. })));
        assert!(red(&app, 60, 100).unwrap() > 0.9 && !s.paste_overlay(), "frozen preview, no overlay on top");
        ev(&mut app, ToolEvent::Up { x: 236.0, y: 156.0 });
        let s = app.distort.vanishing.as_ref().unwrap();
        assert!(s.preview_has_paste() && !s.paste_overlay());
    }

    #[test]
    fn new_layer_stamp_preview_stays_on_the_canvas() {
        let (ctx, mut app) = app_with_layer();
        // A red stripe where the first dab samples, so cloning it changes pixels.
        app.session.execute("select.rect", json!({"x": 130, "y": 70, "width": 20, "height": 20})).unwrap();
        app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.session.execute("select.deselect", json!({})).unwrap();
        crate::distort_ui::menu(&mut app, &ctx, VP, &json!({})).unwrap().unwrap();
        control(&mut app, &json!({"planes": [{"corners": [[10, 10], [230, 10], [230, 150], [10, 150]]}], "newLayer": true})).unwrap();
        // The second dab lies left of the canvas and samples the canvas 100 px to its right.
        control(&mut app, &json!({"clone": [{"source": [140, 80], "points": [[40, 80], [-30, 80]], "size": 16}]})).unwrap();
        let s = app.distort.vanishing.as_ref().unwrap();
        let out = s.preview.as_ref().unwrap().out.content_bounds();
        assert!(!out.is_empty() && s.canvas.contains_rect(&out), "{out:?}");
        let st = app.session.active().unwrap();
        let area = damage(&app, st.doc.id, st.revision, 0, s.shown().unwrap().1).unwrap();
        assert!(s.canvas.contains_rect(&area), "{area:?}");
    }

    #[test]
    fn deleting_the_last_plane_drops_the_stamp_preview() {
        let (ctx, mut app) = app_with_layer();
        crate::distort_ui::menu(&mut app, &ctx, VP, &json!({})).unwrap().unwrap();
        control(&mut app, &json!({"planes": [{"corners": [[70, 60], [170, 60], [220, 150], [20, 150]]}]})).unwrap();
        control(&mut app, &json!({"clone": [{"source": [120, 139], "points": [[118, 100], [126, 100]], "size": 12}]})).unwrap();
        assert!(crate::distort_ui::display_doc(&app, 0).is_some());
        let s = app.distort.vanishing.as_mut().unwrap();
        s.selected = Some(0);
        s.delete_selected();
        sync_preview(&mut app);
        assert!(app.distort.vanishing.as_ref().unwrap().planes.is_empty());
        assert!(crate::distort_ui::display_doc(&app, 0).is_none(), "no strokes shown without a plane");
    }

    /// Opens the mode on the active layer of `app_with_layer` and stamps one stroke.
    fn stamp_a_stroke(app: &mut PhotocraftApp, ctx: &egui::Context) {
        crate::distort_ui::menu(app, ctx, VP, &json!({})).unwrap().unwrap();
        control(app, &json!({"planes": [{"corners": [[10, 10], [230, 10], [230, 150], [10, 150]]}]})).unwrap();
        control(app, &json!({"clone": [{"source": [60, 80], "points": [[120, 80], [126, 80]], "size": 12}]})).unwrap();
    }

    #[test]
    fn stamp_preview_is_not_shown_on_another_document() {
        let (ctx, mut app) = app_with_layer();
        stamp_a_stroke(&mut app, &ctx);
        assert!(crate::distort_ui::display_doc(&app, 0).is_some());
        app.session.execute("file.new", json!({"width": 32, "height": 32, "depth": 8})).unwrap();
        let active = app.session.active_index().unwrap();
        // The session stays until drop_foreign; the preview must not follow this document (#5115).
        assert!(app.distort.vanishing.is_some());
        assert!(crate::distort_ui::display_doc(&app, active).is_none());
    }

    #[test]
    fn ok_paints_the_layer_the_mode_was_opened_on() {
        for new_layer in [false, true] {
            let (ctx, mut app) = app_with_layer();
            let layer = app.session.active().unwrap().active_layer.unwrap();
            app.session
                .edit("stripe", |doc, active| {
                    doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(50, 70, 70, 90), &[1.0, 0.0, 0.0, 1.0]);
                    Ok(())
                })
                .unwrap();
            stamp_a_stroke(&mut app, &ctx);
            app.distort.vanishing.as_mut().unwrap().new_layer = new_layer;
            sync_preview(&mut app);
            // Another layer is picked while the mode is open (it isn't modal).
            app.session.execute("layer.new.layer", json!({"name": "other"})).unwrap();
            let other = app.session.active().unwrap().active_layer.unwrap();
            sync_preview(&mut app);
            let expected = photocraft_compose::flatten(&crate::distort_ui::display_doc(&app, 0).unwrap().0).px;
            commit(&mut app).unwrap();
            let d = &app.session.active().unwrap().doc;
            assert!(d.layer(other).unwrap().surface().unwrap().content_bounds().is_empty(), "newLayer={new_layer}");
            if !new_layer {
                assert!(d.layer(layer).unwrap().surface().unwrap().rgba(120, 80)[0] > 0.9, "the stroke lands on the session's layer");
                // And it becomes active, so Fade and a colour-channel target follow it.
                assert_eq!(app.session.active().unwrap().active_layer, Some(layer));
            }
            assert_eq!(photocraft_compose::flatten(d).px, expected, "newLayer={new_layer}");
        }
    }

    #[test]
    fn an_edit_under_the_mode_shows_on_the_canvas() {
        let (ctx, mut app) = app_with_layer();
        app.session
            .edit("stripe", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(50, 70, 70, 90), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        stamp_a_stroke(&mut app, &ctx);
        let before = crate::distort_ui::display_doc(&app, 0).unwrap();
        app.session
            .edit("edit", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(200, 20, 220, 40), &[0.0, 1.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        // Not the stale preview, even before the mode catches up.
        assert!(crate::distort_ui::display_doc(&app, 0).is_none_or(|(d, _)| !Arc::ptr_eq(&d, &before.0)));
        // The canvas catches up before it draws: the strokes over the edit, no frame without them.
        let (shown, key) = crate::canvas::display_doc(&mut app, 0);
        assert_ne!(key, before.1);
        let st = app.session.active().unwrap();
        let layer = st.active_layer.unwrap();
        let px = shown.layer(layer).unwrap().surface().unwrap();
        assert!(px.rgba(210, 30)[1] > 0.9, "the edit shows: {:?}", px.rgba(210, 30));
        assert!(px.rgba(120, 80) != st.doc.layer(layer).unwrap().surface().unwrap().rgba(120, 80), "and so does the stroke");
        let expected = photocraft_compose::flatten(&shown).px;
        commit(&mut app).unwrap();
        assert_eq!(photocraft_compose::flatten(&app.session.active().unwrap().doc).px, expected);
    }

    #[test]
    fn selecting_another_layer_keeps_the_stamp_preview() {
        let (ctx, mut app) = app_with_layer();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        app.session.execute("layer.new.layer", json!({"name": "other"})).unwrap();
        app.session.execute("layer.select", json!({"layer": layer.0})).unwrap();
        app.sync_views();
        stamp_a_stroke(&mut app, &ctx);
        let before = crate::canvas::display_doc(&mut app, 0);
        let (doc, revision) = app.session.active().map(|st| (st.doc.clone(), st.revision)).unwrap();
        app.session.execute("layer.select", json!({"layer": "other"})).unwrap();
        let st = app.session.active().unwrap();
        assert!(st.revision != revision && Arc::ptr_eq(&st.doc, &doc), "a selection change leaves the pixels alone");
        // Same preview, same key: nothing to repaint or recomposite.
        let after = crate::canvas::display_doc(&mut app, 0);
        assert!(Arc::ptr_eq(&after.0, &before.0));
        assert_eq!(after.1, before.1);
    }
}
