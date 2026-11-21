//! Filter › Liquify…: the framework's Liquify workspace, hosted on the app's dialog host.
//!
//! The workspace owns the session (steps, undo, settings) and every control. This module owns the
//! pixels: a CPU proxy (the layer box-downsampled once to at most 1600 px, `ProxyImage`) that is
//! re-rendered through the displacement field only where a dab changed it, shown to the workspace
//! through [`PcLiquifySource`]. Above 100 % the display asks for full-resolution regions, which are
//! rendered from the layer and the field on demand. OK answers the recorded steps, and
//! `filter.liquify` replays them on the document, so it gets the same field as the preview.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use craft::behavior::PixelSize;
use craft::behavior::kurbo::Point;
use craft::imaging::ui::liquify::{
    BackdropUse, LiquifyBrush, LiquifyDone, LiquifyEvent, LiquifyGlobal, LiquifyMesh, LiquifyOpen, LiquifySettings, LiquifySource, LiquifyStep,
    LiquifyStroke as StepStroke, LiquifyTool as StepTool, LiquifyWorkspace, MeshSize, StrokeEvent,
};
use craft::ui::canvas::{PixelRect, RasterSource};
use craft::ui::dialog::{DialogId, HostedWorkspace};
use craft::ui::primitives::Revision;
use craft::ui::runtime::memory::{SaveKey, saved_value, set_saved_value};
use egui::{Color32, ColorImage};
use photocraft_algo::liquify::{LiquifyField, LiquifyStroke, LiquifyTool, ProxyImage, auto_cell};
use photocraft_doc::{DocId, Document, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// Longest side of the preview proxy (px).
const PROXY_SIDE: usize = 1600;

/// The saved slot of the brush and view settings; the workspace writes it on every change.
const REMEMBERED: SaveKey = SaveKey::new("photocraft.filter.liquify");

/// Changed rectangles the picture remembers for `damage`; a display older than this reads everything.
const DAMAGE_RING: usize = 64;

type Hosted = HostedWorkspace<LiquifyWorkspace>;

/// Native component sampling keeps the other components and opacity at the destination. Work
/// stays bounded by the dirty proxy/viewport rectangle; a dab never scans the full source.
#[allow(clippy::too_many_arguments)]
fn render_components(
    source: &Surface,
    source_bounds: Rect,
    field: &LiquifyField,
    scale: f64,
    target: &Value,
    bounds: Rect,
    dirty: Rect,
    out: &mut [[u8; 4]],
) -> bool {
    let Ok(Some(colors)) = photocraft_engine::color_channels::explicit(&json!({"target":target}), source.format().mode.color_channels()) else { return false };
    let width = bounds.width() as usize;
    if source_bounds.is_empty() || bounds.is_empty() || !scale.is_finite() || scale <= 0.0 || width.checked_mul(bounds.height() as usize) != Some(out.len()) {
        return false;
    }
    let fmt = source.format();
    let mut native = vec![0.0; fmt.channels()];
    let dirty = dirty.intersect(&bounds);
    for y in dirty.y0..dirty.y1 {
        for x in dirty.x0..dirty.x1 {
            let px = f64::from(field.bounds.x0) + (f64::from(x.saturating_sub(source_bounds.x0)) + 0.5) * scale;
            let py = f64::from(field.bounds.y0) + (f64::from(y.saturating_sub(source_bounds.y0)) + 0.5) * scale;
            let displacement = field.sample(px, py);
            let sx = (f64::from(x) + displacement[0] / scale).clamp(f64::from(source_bounds.x0), f64::from(source_bounds.x1 - 1));
            let sy = (f64::from(y) + displacement[1] / scale).clamp(f64::from(source_bounds.y0), f64::from(source_bounds.y1 - 1));
            let (x0, y0) = (sx.floor() as i32, sy.floor() as i32);
            let (fx, fy) = ((sx - f64::from(x0)) as f32, (sy - f64::from(y0)) as f32);
            let taps = [
                (x0, y0, (1.0 - fx) * (1.0 - fy)),
                (x0.saturating_add(1).min(source_bounds.x1 - 1), y0, fx * (1.0 - fy)),
                (x0, y0.saturating_add(1).min(source_bounds.y1 - 1), (1.0 - fx) * fy),
                (x0.saturating_add(1).min(source_bounds.x1 - 1), y0.saturating_add(1).min(source_bounds.y1 - 1), fx * fy),
            ];
            let alpha_index = fmt.mode.color_channels();
            let alpha = |tx, ty| if fmt.alpha { source.sample_channel(tx, ty, alpha_index) } else { 1.0 };
            let warped_alpha: f32 = taps.iter().map(|(tx, ty, w)| alpha(*tx, *ty) * w).sum();
            for (channel, value) in native.iter_mut().enumerate() {
                *value = if colors.contains(channel) {
                    if warped_alpha > 0.0 {
                        taps.iter().map(|(tx, ty, w)| source.sample_channel(*tx, *ty, channel) * alpha(*tx, *ty) * w).sum::<f32>() / warped_alpha
                    } else {
                        0.0
                    }
                } else {
                    source.sample_channel(x, y, channel)
                };
            }
            let rgba = photocraft_raster::to_rgba(&fmt, &native);
            let a = rgba[3].clamp(0.0, 1.0);
            let encoded = [rgba[0] * a, rgba[1] * a, rgba[2] * a, a].map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
            let index = (y - bounds.y0) as usize * width + (x - bounds.x0) as usize;
            if let Some(pixel) = out.get_mut(index) {
                *pixel = encoded;
            }
        }
    }
    true
}

/// The tools that exist on both sides: the workspace's and the engine's. The one table both
/// directions read.
const TOOLS: [(StepTool, LiquifyTool); 11] = [
    (StepTool::ForwardWarp, LiquifyTool::ForwardWarp),
    (StepTool::Reconstruct, LiquifyTool::Reconstruct),
    (StepTool::Smooth, LiquifyTool::Smooth),
    (StepTool::TwirlClockwise, LiquifyTool::TwirlCw),
    (StepTool::TwirlCounterclockwise, LiquifyTool::TwirlCcw),
    (StepTool::Pucker, LiquifyTool::Pucker),
    (StepTool::Bloat, LiquifyTool::Bloat),
    (StepTool::PushLeft, LiquifyTool::PushLeft),
    (StepTool::Freeze, LiquifyTool::Freeze),
    (StepTool::Thaw, LiquifyTool::Thaw),
    (StepTool::FreezeLasso, LiquifyTool::LassoMask),
];

/// The whole-field operations and the engine tools that run them.
const GLOBALS: [(LiquifyTool, fn() -> LiquifyGlobal); 3] = [
    (LiquifyTool::FreezeAll, || LiquifyGlobal::MaskAll),
    (LiquifyTool::ThawAll, || LiquifyGlobal::MaskNone),
    (LiquifyTool::InvertFreeze, || LiquifyGlobal::InvertAll),
];

/// The engine's stroke for a workspace stroke; `None` for a tool that paints nothing (Hand, Zoom).
fn engine_stroke(s: &StepStroke) -> Option<LiquifyStroke> {
    let tool = TOOLS.iter().find(|(f, _)| *f == s.tool).map(|(_, a)| *a)?;
    Some(LiquifyStroke {
        tool,
        size: s.size,
        density: s.density,
        pressure: s.pressure,
        rate: s.rate,
        points: s.points.iter().map(|p| p.to_vec()).collect(),
        amount: s.amount,
    })
}

/// The engine's stroke for a whole-field operation.
fn global_stroke(g: &LiquifyGlobal) -> Option<LiquifyStroke> {
    match g {
        LiquifyGlobal::Reconstruct { amount, .. } => {
            let mut s = LiquifyStroke::new(LiquifyTool::ReconstructAll, 1.0);
            s.amount = Some(*amount);
            Some(s)
        }
        other => {
            let tool = GLOBALS.iter().find(|(_, make)| make() == *other).map(|(t, _)| *t)?;
            Some(LiquifyStroke::new(tool, 1.0))
        }
    }
}

/// The engine's strokes for recorded steps, in replay order.
fn strokes_of(steps: &[LiquifyStep]) -> Vec<LiquifyStroke> {
    steps
        .iter()
        .filter_map(|step| match step {
            LiquifyStep::Stroke(s) => engine_stroke(s),
            LiquifyStep::Global(g) => global_stroke(g),
            _ => None,
        })
        .collect()
}

/// The recorded step for an engine stroke (a smart filter's parameters opened again).
fn step_of(s: &LiquifyStroke) -> Option<LiquifyStep> {
    if s.tool == LiquifyTool::ReconstructAll {
        return Some(LiquifyStep::Global(LiquifyGlobal::reconstruct(s.amount.unwrap_or(100.0))));
    }
    if let Some((_, make)) = GLOBALS.iter().find(|(t, _)| *t == s.tool) {
        return Some(LiquifyStep::Global(make()));
    }
    let tool = TOOLS.iter().find(|(_, a)| *a == s.tool).map(|(f, _)| *f)?;
    let mut brush = LiquifyBrush::default();
    brush.size = s.size;
    brush.density = s.density;
    brush.pressure = s.pressure;
    brush.rate = s.rate;
    let points = s.points.iter().map(|p| [p.first().copied().unwrap_or(0.0), p.get(1).copied().unwrap_or(0.0), p.get(2).copied().unwrap_or(1.0)]).collect();
    let mut stroke = StepStroke::new(tool, &brush, points);
    stroke.amount = s.amount;
    Some(LiquifyStep::Stroke(stroke))
}

/// Whether a stroke of `tool` changes the freeze mask.
fn edits_mask(tool: LiquifyTool) -> bool {
    matches!(tool, LiquifyTool::Freeze | LiquifyTool::Thaw | LiquifyTool::LassoMask)
}

/// The selection a Liquify preview is shown through, as the engine applies it on OK
/// (`distort_cmds::mix_by_selection`): the result over the original by its coverage. On a smart
/// object with a filter mask it is that mask, which the engine shows every filter through.
struct PreviewSelection {
    mask: photocraft_doc::LayerMask,
    /// Coverage at each proxy pixel (row-major, the proxy's size).
    proxy: Vec<f32>,
    /// What shows where the mask hides the result, when that isn't the dialog's source: a filter
    /// mask shows the smart object's placed pixels, without any filter (`filter_mask::mix`). With
    /// its proxy, the same size as the dialog's.
    base: Option<(Surface, ProxyImage)>,
}

impl PreviewSelection {
    /// `sel` with its coverage sampled at `proxy`'s pixels. Kept even when every proxy pixel is
    /// selected: a gap narrower than a proxy pixel still shows in the source-resolution crop.
    fn new(mask: photocraft_doc::LayerMask, proxy: &ProxyImage, base: Option<Surface>) -> Self {
        let (bx, by, s) = (f64::from(proxy.bounds.x0), f64::from(proxy.bounds.y0), proxy.scale);
        let mut cov = Vec::with_capacity(proxy.w * proxy.h);
        for y in 0..proxy.h {
            let dy = (by + (y as f64 + 0.5) * s).floor() as i32;
            for x in 0..proxy.w {
                let dx = (bx + (x as f64 + 0.5) * s).floor() as i32;
                cov.push(mask.value(dx, dy).clamp(0.0, 1.0));
            }
        }
        let base = base.map(|b| {
            let p = ProxyImage::new(&b, proxy.bounds, PROXY_SIDE);
            (b, p)
        });
        PreviewSelection { mask, proxy: cov, base }
    }
}

/// `new` over `old` (premultiplied RGBA8) by coverage `c`.
fn mix(new: [u8; 4], old: [f32; 4], c: f32) -> [u8; 4] {
    std::array::from_fn(|k| (f32::from(new[k]) * c + old[k].clamp(0.0, 1.0) * 255.0 * (1.0 - c) + 0.5).clamp(0.0, 255.0) as u8)
}

fn union(a: Option<[usize; 4]>, b: [usize; 4]) -> Option<[usize; 4]> {
    if b[0] >= b[2] || b[1] >= b[3] {
        return a;
    }
    Some(match a {
        Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
        None => b,
    })
}

fn pixel_rect(r: Rect) -> PixelRect {
    PixelRect::new(r.x0, r.y0, r.x1, r.y1)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The stroke in progress: its engine form, the last point applied and how many points of the
/// workspace's stroke have been applied.
struct Running {
    stroke: LiquifyStroke,
    last: [f64; 3],
    applied: usize,
}

/// The field, the proxy and the warped proxy picture. The workspace's source reads it from the
/// render thread while `take` writes it, so it sits behind one mutex.
struct Engine {
    canvas: Rect,
    cell: f64,
    field: LiquifyField,
    proxy: ProxyImage,
    source: Surface,
    target: Value,
    native_proxy: Option<Surface>,
    /// The proxy through the field and the selection, premultiplied.
    out: Vec<[u8; 4]>,
    /// Proxy pixels to re-render (x0, y0, x1, y1).
    dirty: Option<[usize; 4]>,
    /// The undisplaced field drawn while Preview is off; it exists only then.
    identity: Option<LiquifyField>,
    preview: bool,
    selection: Option<PreviewSelection>,
    running: Option<Running>,
    /// Counts every change of the picture.
    revision: u64,
    /// Changes at or before this revision are no longer in `ring`.
    floor: u64,
    /// The document rectangles of the latest dabs, with the revision each produced.
    ring: Vec<(u64, PixelRect)>,
    /// Counts the changes of the freeze mask.
    mask_revision: u64,
}

impl Engine {
    /// Marks the proxy pixels under `doc_rect` stale and records the change for `damage`.
    fn mark(&mut self, doc_rect: Rect, mask: bool) {
        if doc_rect.is_empty() {
            return;
        }
        self.dirty = union(self.dirty, self.proxy.proxy_rect(doc_rect));
        self.mask_revision += u64::from(mask);
        self.revision += 1;
        self.ring.push((self.revision, pixel_rect(doc_rect)));
        if self.ring.len() > DAMAGE_RING {
            self.floor = self.ring.remove(0).0;
        }
    }

    /// Everything changed: a display of any earlier revision reads the whole picture.
    fn invalidate(&mut self) {
        self.revision += 1;
        self.floor = self.revision;
        self.ring.clear();
    }

    /// Marks `doc_rect` and renders it.
    fn settle(&mut self, doc_rect: Rect, mask: bool) {
        self.mark(doc_rect, mask);
        self.render_dirty();
    }

    /// Re-renders the dirty proxy pixels into `out`.
    fn render_dirty(&mut self) {
        if let Some(r) = self.dirty.take() {
            self.render_proxy(r);
        }
    }

    /// Renders proxy pixels `r` through the field, then through the selection.
    fn render_proxy(&mut self, r: [usize; 4]) {
        let field = self.identity.as_ref().filter(|_| !self.preview).unwrap_or(&self.field);
        if let Some(source) = &self.native_proxy {
            let bounds = Rect::new(0, 0, self.proxy.w as i32, self.proxy.h as i32);
            let dirty = Rect::new(r[0] as i32, r[1] as i32, r[2] as i32, r[3] as i32);
            render_components(source, bounds, field, self.proxy.scale, &self.target, bounds, dirty, &mut self.out);
        } else {
            self.proxy.render(field, r, &mut self.out);
        }
        let Some(sel) = &self.selection else { return };
        let w = self.proxy.w;
        for y in r[1]..r[3].min(self.proxy.h) {
            for x in r[0]..r[2].min(w) {
                let i = y * w + x;
                let old = match &sel.base {
                    Some((_, p)) => p.px.get(i),
                    None => self.proxy.px.get(i),
                };
                if let (Some(c), Some(old), Some(new)) = (sel.proxy.get(i), old, self.out.get_mut(i))
                    && *c < 1.0
                {
                    *new = mix(*new, *old, *c);
                }
            }
        }
    }

    /// Mixes the source-resolution preview's `dirty` pixels (document px, within `bounds`) in `buf`
    /// over the original by the selection.
    fn select_view(&self, buf: &mut [[u8; 4]], bounds: Rect, dirty: Rect) {
        let Some(sel) = &self.selection else { return };
        let (w, dw) = (bounds.width() as usize, dirty.width() as usize);
        // One row of originals at a time: the crop can be millions of pixels.
        let mut old = vec![[0.0f32; 4]; dw];
        for y in dirty.y0..dirty.y1 {
            sel.base.as_ref().map_or(&self.source, |(b, _)| b).read_rgba_into(Rect::new(dirty.x0, y, dirty.x1, y + 1), &mut old);
            for x in dirty.x0..dirty.x1 {
                let c = sel.mask.value(x, y).clamp(0.0, 1.0);
                if c >= 1.0 {
                    continue;
                }
                let at = ((y - bounds.y0) as usize) * w + (x - bounds.x0) as usize;
                if let (Some(new), Some(o)) = (buf.get_mut(at), old.get((x - dirty.x0) as usize)) {
                    // Straight to premultiplied, as the preview's pixels are.
                    let o = [o[0] * o[3], o[1] * o[3], o[2] * o[3], o[3]];
                    *new = mix(*new, o, c);
                }
            }
        }
    }

    /// Renders the whole proxy.
    fn upload(&mut self) {
        self.dirty = Some([0, 0, self.proxy.w, self.proxy.h]);
        self.render_dirty();
        self.invalidate();
    }

    /// Turns Preview on or off: the whole preview is redrawn with, or without, the warp.
    fn set_preview(&mut self, on: bool) {
        if on == self.preview {
            return;
        }
        self.identity = (!on).then(|| LiquifyField::new(self.canvas, self.cell));
        self.preview = on;
        self.upload();
    }

    /// Rebuilds the field from `strokes`, after undo, redo, Restore All and a cancelled stroke.
    fn rebuild(&mut self, strokes: &[LiquifyStroke]) {
        self.running = None;
        self.field = LiquifyField::from_strokes(self.canvas, self.cell, strokes);
        self.mask_revision += 1;
        self.upload();
    }

    /// Applies a whole-field operation.
    fn global(&mut self, g: &LiquifyGlobal) {
        let Some(s) = global_stroke(g) else { return };
        self.running = None;
        let d = self.field.apply_stroke(&s);
        self.settle(d, true);
    }

    /// Applies a stroke event to the field. The points are the ones the workspace recorded, so OK
    /// replays the same.
    fn stroke(&mut self, event: StrokeEvent) {
        match event {
            StrokeEvent::Begin(s) => {
                let (Some(stroke), Some(&p)) = (engine_stroke(&s), s.points.first()) else { return };
                let d = self.field.stroke_begin(&stroke, p);
                self.settle(d, edits_mask(stroke.tool));
                self.running = Some(Running { stroke, last: p, applied: 1 });
            }
            StrokeEvent::Update(s) => self.extend(&s, false),
            StrokeEvent::Commit(s) => self.extend(&s, true),
            // The Replay that follows a cancel rebuilds the field.
            _ => self.running = None,
        }
    }

    /// Continues the running stroke by the points of `s` it has not applied. `Update` carries only
    /// new points, `Commit` the whole stroke. A commit with no stroke running is a lasso polygon.
    fn extend(&mut self, s: &StepStroke, commit: bool) {
        let Some(mut run) = self.running.take() else {
            if commit && let Some(stroke) = engine_stroke(s) {
                let d = self.field.apply_stroke(&stroke);
                self.settle(d, edits_mask(stroke.tool));
            }
            return;
        };
        let skip = if commit { run.applied } else { 0 };
        let stationary = run.stroke.tool.is_stationary();
        for &p in s.points.iter().skip(skip) {
            let d =
                if stationary { self.field.stroke_segment(&run.stroke, run.last, p) } else { self.field.stroke_segment_live(&run.stroke, run.last, p, 0.0) };
            self.mark(d, edits_mask(run.stroke.tool));
            run.last = p;
            run.applied += 1;
        }
        self.render_dirty();
        if !commit {
            self.running = Some(run);
        }
    }

    /// The field the picture is drawn through.
    fn shown_field(&self) -> &LiquifyField {
        self.identity.as_ref().filter(|_| !self.preview).unwrap_or(&self.field)
    }

    /// The picture's size.
    fn size(&self) -> PixelSize {
        PixelSize::new(self.canvas.width(), self.canvas.height())
    }

    /// The part of the picture that changed after revision `since`; `None` when it cannot tell.
    fn damage(&self, since: u64) -> Option<PixelRect> {
        if since < self.floor {
            return None;
        }
        Some(self.ring.iter().filter(|(r, _)| *r > since).fold(PixelRect::default(), |acc, (_, r)| acc.union(r)))
    }

    /// Writes `region` of the picture into `out`, one texel per `factor` pixels. A factor at or
    /// above the proxy's scale reads the proxy; a finer one renders the region at full resolution.
    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        let factor = factor.max(1) as i32;
        let [ow, oh] = out.size;
        let scale = self.proxy.scale;
        let coarse = factor >= (scale.floor() as i32).max(1);
        let render = Rect::new(region.x0, region.y0, region.x1, region.y1).intersect(&self.canvas);
        let mut detail = Vec::new();
        if !coarse && !render.is_empty() {
            let mut buf = vec![[0u8; 4]; render.width() as usize * render.height() as usize];
            let field = self.shown_field();
            let rendered = if self.native_proxy.is_some() {
                render_components(&self.source, self.canvas, field, 1.0, &self.target, render, render, &mut buf)
            } else {
                let mut scratch = Vec::new();
                ProxyImage::render_full_resolution(&self.source, field, render, render, &mut buf, &mut scratch)
            };
            if rendered {
                self.select_view(&mut buf, render, render);
                detail = buf;
            }
        }
        let (pw, ph) = (self.proxy.w as i32, self.proxy.h as i32);
        for oy in 0..oh {
            let y = region.y0 + oy as i32 * factor;
            for ox in 0..ow {
                let x = region.x0 + ox as i32 * factor;
                let p = if !detail.is_empty() {
                    if x >= render.x0 && x < render.x1 && y >= render.y0 && y < render.y1 {
                        detail[(y - render.y0) as usize * render.width() as usize + (x - render.x0) as usize]
                    } else {
                        [0; 4]
                    }
                } else if pw == 0 || ph == 0 {
                    [0; 4]
                } else {
                    let (yc, xc) = (y + factor / 2, x + factor / 2);
                    let py = (f64::from(yc - self.proxy.bounds.y0) / scale).floor().clamp(0.0, f64::from(ph - 1)) as usize;
                    let px = (f64::from(xc - self.proxy.bounds.x0) / scale).floor().clamp(0.0, f64::from(pw - 1)) as usize;
                    self.out.get(py * self.proxy.w + px).copied().unwrap_or([0; 4])
                };
                out.pixels[oy * ow + ox] = Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3]);
            }
        }
    }

    /// The displaced mesh Show Mesh draws at `size`.
    fn mesh(&self, size: MeshSize) -> LiquifyMesh {
        let cells = match size {
            MeshSize::Small => 64.0,
            MeshSize::Large => 24.0,
            _ => 40.0,
        };
        let c = self.canvas;
        let step = f64::from(c.width().max(c.height())) / cells;
        let columns = (f64::from(c.width()) / step).ceil() as usize + 1;
        let rows = (f64::from(c.height()) / step).ceil() as usize + 1;
        let field = self.shown_field();
        let mut points = Vec::with_capacity(columns * rows);
        for j in 0..rows {
            for i in 0..columns {
                let (x, y) = (f64::from(c.x0) + i as f64 * step, f64::from(c.y0) + j as f64 * step);
                let v = field.sample(x, y);
                points.push(Point::new(x + v[0], y + v[1]));
            }
        }
        LiquifyMesh::new(columns, rows, points)
    }
}

/// The backdrop the workspace shows, rendered by the app.
struct Backdrop {
    image: Option<ColorImage>,
    canvas: Rect,
    revision: u64,
}

/// The warped proxy as the workspace's picture.
struct PictureView(Arc<Mutex<Engine>>);

impl RasterSource for PictureView {
    fn size(&self) -> PixelSize {
        lock(&self.0).size()
    }

    fn revision(&self) -> Revision {
        Revision::new(lock(&self.0).revision)
    }

    fn damage(&self, since: Revision) -> Option<PixelRect> {
        lock(&self.0).damage(since.get())
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        lock(&self.0).read(region, factor, out);
    }
}

/// The freeze mask: one texel per field node, white with the frozen amount as alpha.
struct MaskView(Arc<Mutex<Engine>>);

impl RasterSource for MaskView {
    fn size(&self) -> PixelSize {
        let e = lock(&self.0);
        PixelSize::new(e.field.w as u32, e.field.h as u32)
    }

    fn revision(&self) -> Revision {
        Revision::new(lock(&self.0).mask_revision)
    }

    fn damage(&self, _since: Revision) -> Option<PixelRect> {
        None
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        let e = lock(&self.0);
        let factor = factor.max(1) as i32;
        let [ow, oh] = out.size;
        for oy in 0..oh {
            let y = region.y0 + oy as i32 * factor;
            for ox in 0..ow {
                let x = region.x0 + ox as i32 * factor;
                let inside = x >= 0 && y >= 0 && (x as usize) < e.field.w && (y as usize) < e.field.h;
                let a = if inside { e.field.freeze.get(y as usize * e.field.w + x as usize).copied().unwrap_or(0.0) } else { 0.0 };
                out.pixels[oy * ow + ox] = Color32::from_white_alpha((a.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
    }
}

/// The backdrop image over the document's area.
struct BackdropView(Arc<Mutex<Backdrop>>);

impl RasterSource for BackdropView {
    fn size(&self) -> PixelSize {
        let b = lock(&self.0);
        PixelSize::new(b.canvas.width(), b.canvas.height())
    }

    fn revision(&self) -> Revision {
        Revision::new(lock(&self.0).revision)
    }

    fn damage(&self, _since: Revision) -> Option<PixelRect> {
        None
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        let b = lock(&self.0);
        let factor = factor.max(1) as i32;
        let [ow, oh] = out.size;
        let Some(img) = &b.image else { return };
        let [iw, ih] = img.size;
        let (cw, ch) = (b.canvas.width().max(1) as i64, b.canvas.height().max(1) as i64);
        for oy in 0..oh {
            let y = region.y0 + oy as i32 * factor;
            for ox in 0..ow {
                let x = region.x0 + ox as i32 * factor;
                let inside = x >= b.canvas.x0 && x < b.canvas.x1 && y >= b.canvas.y0 && y < b.canvas.y1;
                out.pixels[oy * ow + ox] = if inside {
                    let ix = ((i64::from(x - b.canvas.x0) * iw as i64) / cw).clamp(0, iw as i64 - 1) as usize;
                    let iy = ((i64::from(y - b.canvas.y0) * ih as i64) / ch).clamp(0, ih as i64 - 1) as usize;
                    img.pixels[iy * iw + ix]
                } else {
                    Color32::TRANSPARENT
                };
            }
        }
    }
}

/// What PhotoCraft gives the Liquify workspace to show: the warped proxy, the freeze mask, the
/// displaced mesh, the backdrop and the layer names.
pub struct PcLiquifySource {
    picture: PictureView,
    mask: MaskView,
    mask_texel: u32,
    backdrop: Option<BackdropView>,
    layers: Vec<(u64, String)>,
}

impl LiquifySource for PcLiquifySource {
    fn picture(&self) -> &dyn RasterSource {
        &self.picture
    }

    fn freeze_mask(&self) -> Option<(&dyn RasterSource, u32)> {
        Some((&self.mask, self.mask_texel))
    }

    fn mesh(&self, size: MeshSize) -> Option<LiquifyMesh> {
        Some(lock(&self.picture.0).mesh(size))
    }

    fn backdrop(&self) -> Option<&dyn RasterSource> {
        self.backdrop.as_ref().map(|b| b as &dyn RasterSource)
    }

    fn layers(&self) -> Vec<(u64, String)> {
        self.layers.clone()
    }
}

/// The app's side of the open Liquify workspace: the document and layer it edits, the hosted
/// dialog's id, and the pixels it shows.
pub struct LiquifyApp {
    /// Document the workspace belongs to (#5115).
    pub document: DocId,
    /// The layer being liquified.
    pub layer: LayerId,
    /// The hosted workspace's id on the dialog host.
    pub(crate) id: DialogId,
    /// The smart filter being edited: OK updates it instead of adding a new Liquify filter.
    smart_filter: Option<usize>,
    engine: Arc<Mutex<Engine>>,
    backdrop: Arc<Mutex<Backdrop>>,
    layers: Vec<(u64, String)>,
}

impl LiquifyApp {
    /// A source over the current state, for `set_source`.
    fn source(&self) -> PcLiquifySource {
        let has_backdrop = lock(&self.backdrop).image.is_some();
        PcLiquifySource {
            picture: PictureView(self.engine.clone()),
            mask: MaskView(self.engine.clone()),
            mask_texel: lock(&self.engine).cell.round().max(1.0) as u32,
            backdrop: has_backdrop.then(|| BackdropView(self.backdrop.clone())),
            layers: self.layers.clone(),
        }
    }
}

/// Opens the workspace on the active layer.
pub fn open(app: &mut PhotocraftApp, _ctx: &egui::Context) -> Result<DialogId, String> {
    let (layer, mut surf, _) = crate::distort_ui::active_filter_target(app)?;
    let doc = &app.session.active().ok_or("no document")?.doc;
    let canvas = doc.bounds();
    // A smart object's pixels are its filters mixed through its filter mask; OK adds the Liquify
    // above the filters themselves, so that is what it edits.
    if let Some(photocraft_doc::LayerContent::Smart(sm)) = doc.layer(layer).map(|l| &l.content)
        && sm.filter_mask.is_some()
        && let Ok(Some(filtered)) = photocraft_engine::smart_cmds::render_below_filter(doc, sm, sm.smart_filters.len())
    {
        surf = filtered;
    }
    open_on(app, layer, surf, Vec::new(), auto_cell(canvas), None)
}

/// Re-opens Liquify smart filter `index` of smart object `layer` with its recorded strokes and
/// mesh size, previewing on the pixels below it; OK updates that filter
/// (`layer.smartFilter.setParams`).
pub fn open_smart_filter(app: &mut PhotocraftApp, _ctx: &egui::Context, layer: LayerId, index: usize) -> Result<DialogId, String> {
    use photocraft_engine::distort_cmds::{LIQUIFY, liquify_params};
    let st = app.session.active().ok_or("no document")?;
    let Some(photocraft_doc::LayerContent::Smart(sm)) = st.doc.layer(layer).map(|l| &l.content) else {
        return Err("the layer is not a smart object".into());
    };
    let filter = sm.smart_filters.get(index).ok_or("no such smart filter")?;
    if filter.command != LIQUIFY {
        return Err("the smart filter is not a Liquify filter".into());
    }
    let (strokes, cell) = liquify_params(LIQUIFY, &filter.params, st.doc.bounds()).map_err(|e| e.to_string())?;
    let surf = photocraft_engine::smart_cmds::render_below_filter(&st.doc, sm, index)
        .map_err(|e| e.to_string())?
        .ok_or("the smart object's contents are unavailable (missing linked file?)")?;
    open_on(app, layer, surf, strokes, cell, Some(index))
}

fn open_on(
    app: &mut PhotocraftApp,
    layer: LayerId,
    surf: Surface,
    strokes: Vec<LiquifyStroke>,
    cell: f64,
    smart_filter: Option<usize>,
) -> Result<DialogId, String> {
    let st = app.session.active().ok_or("no document")?;
    let canvas = st.doc.bounds();
    let name = st.doc.layer(layer).map(|l| l.name.clone()).unwrap_or_default();
    let proxy = ProxyImage::new(&surf, canvas, PROXY_SIDE);
    let target = if smart_filter.is_some() { json!("pixels") } else { crate::distort_ui::colour_set_target(app) };
    let colors = photocraft_engine::color_channels::explicit(&json!({"target":target}), surf.format().mode.color_channels()).map_err(|e| e.to_string())?;
    let native_proxy = colors.map(|_| crate::gallery_ui::reduce(&surf, canvas, proxy.scale as u32));
    let out = vec![[0u8; 4]; proxy.w * proxy.h];
    // A smart object's filter mask shows every filter through it, a new Liquify too, whatever is
    // selected (`smart_cmds::add_smart_filter`), over the placed pixels without any filter.
    // Otherwise a new Liquify goes through the selection; a smart filter being edited has none.
    let selection = match st.doc.layer(layer).map(|l| &l.content) {
        Some(photocraft_doc::LayerContent::Smart(sm)) if sm.filter_mask.is_some() => sm.filter_mask.clone().map(|mask| {
            let placed = photocraft_engine::smart_cmds::render_below_filter(&st.doc, sm, 0).ok().flatten();
            PreviewSelection::new(mask, &proxy, placed)
        }),
        _ => st
            .doc
            .selection
            .clone()
            .filter(|_| smart_filter.is_none())
            .map(|sel| PreviewSelection::new(photocraft_doc::LayerMask { surface: sel, ..photocraft_doc::LayerMask::reveal_all() }, &proxy, None)),
    };
    // Show Backdrop › Use: All Layers or one layer, bottom to top as Photoshop lists them.
    let layers: Vec<(u64, String)> = st.doc.walk().into_iter().map(|(_, _, l)| (l.id.0, l.name.clone())).collect();
    // A remembered Use that names a layer this document lacks (another document's) opens on All Layers.
    let ctx = app.ctx().clone();
    if let Some(mut stored) = saved_value::<Option<LiquifySettings>>(&ctx, &REMEMBERED)
        && let BackdropUse::Layer(id) = stored.backdrop
        && !layers.iter().any(|(layer, _)| *layer == id)
    {
        stored.backdrop = BackdropUse::AllLayers;
        set_saved_value(&ctx, &REMEMBERED, Some(stored));
    }
    let document = st.doc.id;
    let steps: Vec<LiquifyStep> = strokes.iter().filter_map(step_of).collect();
    let size = [canvas.width(), canvas.height()];
    let field = LiquifyField::from_strokes(canvas, cell, &strokes);
    let mut engine = Engine {
        canvas,
        cell,
        field,
        proxy,
        source: surf,
        target,
        native_proxy,
        out,
        dirty: None,
        identity: None,
        preview: true,
        selection,
        running: None,
        revision: 0,
        floor: 0,
        ring: Vec::new(),
        mask_revision: 0,
    };
    engine.upload();
    let open = LiquifyOpen::new(size, name)
        .steps(steps)
        .navigation(crate::canvas::document_display::navigation(app))
        .checkerboard(crate::canvas::document_display::checkerboard(app).unwrap_or_default());
    let mut state = LiquifyApp {
        document,
        layer,
        id: DialogId::NULL,
        smart_filter,
        engine: Arc::new(Mutex::new(engine)),
        backdrop: Arc::new(Mutex::new(Backdrop { image: None, canvas, revision: 0 })),
        layers,
    };
    let source: Arc<dyn LiquifySource> = Arc::new(state.source());
    // OK is held: a failed apply keeps the workspace open with the reason (`take`).
    let hosted = HostedWorkspace::new(LiquifyWorkspace::new(open).remember(REMEMBERED), source).held_ok();
    let id = app.framework.dialogs.open(hosted);
    state.id = id;
    app.liquify = Some(state);
    Ok(id)
}

/// Applies what the workspace reported since the last call to the field, loads a backdrop it
/// asked for, and answers a held OK. Called once per frame after the dialog host ran.
pub(crate) fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(id) = app.liquify.as_ref().map(|l| l.id) else { return };
    let Some(hosted) = app.framework.dialogs.get_mut_by_id::<Hosted>(id) else {
        // Cancelled, or finished by the answer to OK.
        app.liquify = None;
        return;
    };
    let events = hosted.take_events();
    let done = hosted.take_ok();
    let mut wanted = None;
    if !events.is_empty()
        && let Some(l) = app.liquify.as_ref()
    {
        let mut engine = lock(&l.engine);
        for event in events {
            match event {
                LiquifyEvent::Stroke(e) => engine.stroke(e),
                LiquifyEvent::Global(g) => engine.global(&g),
                LiquifyEvent::Replay(steps) => engine.rebuild(&strokes_of(&steps)),
                LiquifyEvent::BackdropWanted(of) => wanted = Some(of),
                LiquifyEvent::Preview(on) => engine.set_preview(on),
                _ => {}
            }
        }
        ctx.request_repaint();
    }
    if let Some(of) = wanted {
        let canvas = app.liquify.as_ref().map(|l| lock(&l.engine).canvas);
        if let Some(canvas) = canvas {
            let image = load_backdrop(app, of, canvas);
            if let Some(l) = app.liquify.as_ref() {
                {
                    let mut b = lock(&l.backdrop);
                    b.image = image;
                    b.revision += 1;
                }
                let source: Arc<dyn LiquifySource> = Arc::new(l.source());
                if let Some(hosted) = app.framework.dialogs.get_mut_by_id::<Hosted>(id) {
                    hosted.set_source(source);
                }
            }
        }
    }
    if let Some(done) = done {
        let answer = commit(app, &done);
        if let Some(hosted) = app.framework.dialogs.get_mut_by_id::<Hosted>(id) {
            hosted.answer_ok(answer.map_err(Into::into));
        }
        ctx.request_repaint();
    }
}

/// OK: runs `filter.liquify` with the recorded strokes (one history step). When the workspace
/// edits a smart filter, OK updates that filter instead (also when Restore All left no strokes).
/// `app.run` shows a failed command's error in the status bar, and `take` hands it to the
/// workspace.
fn commit(app: &mut PhotocraftApp, done: &LiquifyDone) -> Result<(), String> {
    let l = app.liquify.as_ref().ok_or("Liquify is not open")?;
    let (layer, smart_filter) = (l.layer, l.smart_filter);
    let (cell, target) = {
        let e = lock(&l.engine);
        (e.cell, e.target.clone())
    };
    let strokes = strokes_of(&done.steps);
    if let Some(index) = smart_filter {
        let params = json!({"strokes": strokes, "meshSize": cell});
        return app.run("layer.smartFilter.setParams", json!({"layer": layer.0, "index": index, "params": params})).map(|_| ());
    }
    if strokes.is_empty() {
        return Ok(());
    }
    let p = crate::distort_ui::with_filter_target(json!({"strokes": strokes, "meshSize": cell, "layer": layer.0}), &target);
    app.run("filter.liquify", p).map(|_| ())
}

/// Records a Forward Warp stroke through `points` (x, y, pressure) in the open workspace, as a drag with the
/// default brush would, and lets the app apply it. For tests of other panels that need a stroke to commit.
#[cfg(test)]
pub(crate) fn record_stroke(app: &mut PhotocraftApp, points: &[[f64; 3]]) {
    let ctx = app.ctx().clone();
    let brush = craft::imaging::ui::liquify::LiquifySettings::default().brush;
    let stroke = StepStroke::new(StepTool::ForwardWarp, &brush, points.to_vec());
    app.framework.dialogs.get_mut::<Hosted>().expect("the Liquify workspace is open").workspace_mut().perform(LiquifyStep::Stroke(stroke));
    take(app, &ctx);
}

/// The reply of `filter.liquify` while the workspace is open: the app's facts, with the steps
/// so far and the redo depth from the workspace.
pub(crate) fn describe(app: &PhotocraftApp) -> Option<Value> {
    let l = app.liquify.as_ref()?;
    let strokes = app.framework.dialogs.get_by_id::<Hosted>(l.id).map_or(0, |h| h.workspace().session().len());
    let redo = app.framework.dialogs.get_by_id::<Hosted>(l.id).map_or(0, |h| h.workspace().redo_len());
    let e = lock(&l.engine);
    Some(json!({
        "document": l.document.0,
        "layer": l.layer.0,
        "strokes": strokes,
        "redo": redo,
        "meshSize": e.cell,
        // The workspace's own checkbox: the engine's follows at the next pass.
        "preview": app.framework.dialogs.get_by_id::<Hosted>(l.id).map_or(e.preview, |h| h.workspace().preview()),
        "maxDisplacement": e.field.max_displacement(),
        "proxy": [e.proxy.w, e.proxy.h],
        "smartFilter": l.smart_filter,
    }))
}

/// The backdrop for Show Backdrop › Use `of`: the document as it is (All Layers, the liquified
/// layer's own pixels included, as in Photoshop) or that one layer alone. A layer that is gone
/// shows All Layers.
fn load_backdrop(app: &PhotocraftApp, of: Option<u64>, canvas: Rect) -> Option<ColorImage> {
    let st = app.session.active()?;
    let buf = photocraft_compose::render(&backdrop_doc(&st.doc, of), canvas);
    let mut s = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    let flat: Vec<f32> = buf.px.iter().flat_map(|p| *p).collect();
    s.write_region(canvas, &flat);
    Some(crate::distort_ui::surface_image(&s, canvas, PROXY_SIDE))
}

/// The document [`load_backdrop`] renders for Use `of`.
fn backdrop_doc(doc: &Document, of: Option<u64>) -> std::borrow::Cow<'_, Document> {
    match of.map(LayerId).filter(|id| doc.layer(*id).is_some()) {
        Some(id) => {
            let mut d = doc.clone();
            crate::transform_tool::solo_layer(&mut d.layers, id);
            // A hidden layer, or one in a hidden group, shows too: Use lists every layer.
            reveal(&mut d.layers, id);
            std::borrow::Cow::Owned(d)
        }
        None => std::borrow::Cow::Borrowed(doc),
    }
}

/// Makes layer `id` and the groups holding it visible; whether it was found.
fn reveal(layers: &mut [photocraft_doc::Layer], id: LayerId) -> bool {
    layers.iter_mut().any(|l| {
        let found = l.id == id
            || match &mut l.content {
                photocraft_doc::LayerContent::Group(g) => reveal(&mut g.children, id),
                _ => false,
            };
        if found {
            l.visible = true;
        }
        found
    })
}

#[cfg(test)]
mod tests;
