//! Smart selection commands: Quick Selection, Object Selection, Select Subject, Select and Mask
//! (Refine Edge) and Focus Area. The algorithms live in `photocraft_algo::segment` and
//! `photocraft_algo::matting`; these commands pick the pixels to analyse (active layer or
//! composite), run them, and store the result as the selection (or a mask / new layer).

use std::borrow::Cow;

use photocraft_algo::matting::{self, RefineParams};
use photocraft_algo::segment::{Sampler, SurfaceSampler, focus, grabcut, quick, subject};
use photocraft_algo::selection::{Region, SelectionMode, combine_region};
use photocraft_doc::{Document, Layer, LayerContent, LayerId, LayerMask};
use photocraft_geom::Rect;
use photocraft_raster::to_rgba;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::select_mask_edits;
use crate::{EngineError, Result, Session};

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
fn has_selection_or_mask(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or_else(|| "no document open".to_string())?;
    if d.doc.selection.is_some() {
        return Ok(());
    }
    if let Some(l) = d.active_layer.and_then(|id| d.doc.layer(id))
        && l.mask.is_some()
    {
        return Ok(());
    }
    Err("no selection or layer mask".into())
}

fn f(p: &Value, k: &str, d: f32) -> f32 {
    p.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}
fn b(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn mode(p: &Value, default: &str) -> SelectionMode {
    let m = p.get("mode").and_then(Value::as_str).unwrap_or(default);
    SelectionMode::parse(if m == "new" { "replace" } else { m })
}
fn bad(cmd: &str, msg: &str) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// Composite of all visible layers.
struct CompositeSampler<'a>(&'a Document);

impl Sampler for CompositeSampler<'_> {
    fn rgba(&self, r: Rect) -> Vec<[f32; 4]> {
        photocraft_compose::render(self.0, r).into_px()
    }
}

/// Runs `f` with a sampler over the active layer (or the composite with `all_layers`, or when the
/// active layer has no pixels).
fn with_sampler<R>(s: &Session, all_layers: bool, f: impl FnOnce(&dyn Sampler, &Document) -> R) -> Result<R> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    Ok(with_doc_sampler(&d.doc, d.active_layer, all_layers, f))
}

/// Runs `f` with a sampler over `layer`'s pixels in `doc` (or the composite with `all_layers`, or
/// when the layer has no pixels).
pub(crate) fn with_doc_sampler<R>(doc: &Document, layer: Option<LayerId>, all_layers: bool, f: impl FnOnce(&dyn Sampler, &Document) -> R) -> R {
    match (all_layers, layer.and_then(|id| doc.layer(id)).and_then(Layer::surface)) {
        (false, Some(surf)) => f(&SurfaceSampler(surf), doc),
        _ => f(&CompositeSampler(doc), doc),
    }
}

/// A flat, opaque image: what Quick Selection reads with a group active and Sample All Layers off.
struct FlatSampler;

impl Sampler for FlatSampler {
    fn rgba(&self, r: Rect) -> Vec<[f32; 4]> {
        vec![[0.0, 0.0, 0.0, 1.0]; r.width() as usize * r.height() as usize]
    }
}

/// A plain group: no frame, no artboard. The group kind whose clicks are measured for Quick
/// Selection (#12933) and Object Selection (#12863); artboards and frames keep today's answer.
fn plain_group(l: &Layer) -> bool {
    l.frame.is_none() && matches!(&l.content, LayerContent::Group(g) if g.artboard.is_none())
}

/// Whether `select.object` with `"point"` answers the whole-canvas area for this target
/// (#12863): a plain group active with Sample All Layers off. The Object Finder hover runs on
/// the same predicate, so its highlight never names an object the click will not commit
/// (#15472 review).
pub fn plain_group_active(doc: &Document, layer: Option<LayerId>, all_layers: bool) -> bool {
    !all_layers && layer.and_then(|id| doc.layer(id)).is_some_and(plain_group)
}

/// Runs `f` with Quick Selection's sampler: [`with_sampler`]'s, except that with a group active
/// and Sample All Layers off it reads a flat area, so a click selects only the brush disc instead
/// of the composite's shapes, as the reference application does (#12933). Artboards and frames,
/// groups too but not measured, keep reading the composite.
fn with_quick_sampler<R>(s: &Session, all_layers: bool, f: impl FnOnce(&dyn Sampler, &Document) -> R) -> Result<R> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let group = d.active_layer.and_then(|id| d.doc.layer(id)).is_some_and(plain_group);
    if group && !all_layers { Ok(f(&FlatSampler, &d.doc)) } else { with_sampler(s, all_layers, f) }
}

/// Whether the layer's own pixels are the edit target. Quick Selection's transparent-pixel drop
/// (#12919) and Object Selection's area answers (#12863) were measured there; with a targeted layer
/// mask or a single channel those clicks keep the answer from before the items (#15472 review).
fn pixels_targeted(s: &Session) -> bool {
    !s.tools.mask_targeted && s.active().is_some_and(|d| d.channel_view.target == crate::channel_cmds::ChannelTarget::Composite)
}

/// Stores `region` combined with the current selection by `m` as one history step.
/// Finding nothing never destroys work: the selection (and history) is left alone and the
/// caller gets `changed: false`, whatever the mode. (Select Subject and Object Selection run
/// heuristics that often find nothing on busy photos; clearing the selection then looks like
/// a dead button.)
fn apply(s: &mut Session, label: &str, region: Option<&Region>, m: SelectionMode) -> Result<Value> {
    if region.is_none() {
        // Nothing found: leave the selection (and history) alone.
        let selected = s.active().ok_or(EngineError::NoDocument)?.doc.selection.is_some();
        return Ok(json!({ "selected": selected, "changed": false }));
    }
    let bbox = region.map(|r| r.bbox);
    let selected = s.edit(label, |doc, _| {
        doc.selection = combine_region(doc.selection.as_ref(), region, m);
        Ok(doc.selection.is_some())
    })?;
    Ok(json!({ "selected": selected, "changed": true, "bounds": bbox.map(|r| [r.x0, r.y0, r.width() as i32, r.height() as i32]) }))
}

fn points(p: &Value) -> Vec<(f32, f32)> {
    p.get("points")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|q| Some((q.get(0)?.as_f64()? as f32, q.get(1)?.as_f64()? as f32))).collect())
        .unwrap_or_default()
}

/// Auto-Enhance for Quick Selection: a light edge refinement that flows the edge towards image
/// edges and removes blockiness.
const ENHANCE: RefineParams = RefineParams { radius: 4.0, smart_radius: true, smooth: 10.0, feather: 0.0, contrast: 25.0, shift_edge: 0.0 };

/// Enhance Edge's soft rim (#12923). The reference application's cut is not a hard cut: inside the
/// edge its coverage stays a hair under fully selected for about three pixels. Measured on a red
/// rectangle on white, the rim sits at 8-bit 253-254 (the corners a little lower) while the interior
/// stays fully selected, and the bounds do not move.
///
/// A pixel loses [`RIM_DEPTH`] of its coverage times the share of a [`RIM_SIGMA`] neighbourhood
/// around it that lies outside the cut, so the dip is deepest at the edge, fades inward, deepens
/// where two edges meet and never reaches past the cut. [`RIM_SIGMA`] is the edge radius Enhance Edge
/// already looks with, so the rim reaches as far inward as the band it snaps to; [`RIM_DEPTH`] is
/// fitted to the one configuration the issue measured (a 200 x 150 px rectangle, brush size 30, one
/// click inside). Every other shape follows the same rule, not a per-shape value.
const RIM_SIGMA: f32 = ENHANCE.radius;
const RIM_DEPTH: f32 = 0.01;
/// Pixels of halo a tile reads around itself. [`matting::gaussian_blur`] truncates its kernel at
/// three sigma, so this is exactly the reach of the neighbourhood a pixel is measured against.
/// ceil(3 * sigma) + 1; exact while [`ENHANCE`].radius stays integral.
const RIM_HALO: i32 = 3 * ENHANCE.radius as i32 + 1;

/// `cut` with the [`RIM_SIGMA`] soft rim inside its edge, as [`RIM_SIGMA`] describes. The cut is
/// walked one existing tile at a time with [`RIM_HALO`] of halo, so the float buffers cost a tile and
/// its edge band rather than the selection's bounding box. Pixels past `canvas` repeat the canvas
/// edge, so a cut that touches the document border keeps its full coverage there.
fn enhance_rim(cut: &photocraft_raster::Surface, canvas: Rect) -> std::result::Result<photocraft_raster::Surface, photocraft_raster::AllocationError> {
    let mut out = cut.clone();
    for (tc, _) in cut.tiles() {
        let tile = tc.rect().intersect(&canvas);
        if tile.is_empty() {
            continue;
        }
        let read = tile.inflate(RIM_HALO).intersect(&canvas);
        let rw = read.width() as usize;
        let cover = photocraft_algo::selection::mask_from_surface(Some(cut), read);
        // A pixel can only lose coverage if it is selected itself and something unselected lies
        // within the blur's reach. The band where those two boxes meet is all of the tile that can
        // change, and blurring that band instead of the whole tile is what keeps one click on a
        // selection that covers a large part of the document affordable.
        let (Some(selected), Some(outside)) = (touched_box(&cover, read, rw, |c| c > 0.0), touched_box(&cover, read, rw, |c| c < 1.0)) else {
            continue;
        };
        let work = outside.inflate(RIM_HALO).intersect(&selected.inflate(RIM_HALO)).intersect(&tile);
        if work.is_empty() {
            continue;
        }
        // The blur of a pixel of `work` needs [`RIM_HALO`] around it, and `read` is that much around
        // the tile, so the window is only ever clipped at the canvas - where the blur repeats the
        // canvas edge, which is what keeps a cut at the document border fully selected there.
        let win = work.inflate(RIM_HALO).intersect(&read);
        let (ww, wh) = (win.width() as usize, win.height() as usize);
        let mut patch = Vec::with_capacity(ww * wh);
        for ry in 0..wh {
            let from = ((win.y0 - read.y0) as usize + ry) * rw + (win.x0 - read.x0) as usize;
            patch.extend_from_slice(cover.get(from..from + ww).unwrap_or_default());
        }
        // What the neighbourhood is missing from fully selected is the part of the cut that is not
        // there, and that is the share of its coverage the rim takes away from a selected pixel.
        let near = matting::gaussian_blur(&patch, ww, wh, RIM_SIGMA);
        let mut data = Vec::with_capacity(work.width() as usize * work.height() as usize);
        let mut touched = false;
        // `work` lies inside `win` by construction, so every row is there; a wrong length would make
        // `try_write_region` return an error rather than write a partial band.
        for y in work.y0..work.y1 {
            let from = (y - win.y0) as usize * ww + (work.x0 - win.x0) as usize;
            let to = from + work.width() as usize;
            let (Some(row), Some(near)) = (patch.get(from..to), near.get(from..to)) else {
                continue;
            };
            for (c, near) in row.iter().zip(near) {
                // Unselected pixels are left exactly as they are: the rim only takes coverage away.
                let v = if *c <= 0.0 { 0.0 } else { (c - RIM_DEPTH * (1.0 - *near) * c).clamp(0.0, 1.0) };
                touched |= v != *c;
                data.push(v);
            }
        }
        if touched {
            out.try_write_region(work, &data)?;
        }
    }
    Ok(out)
}

/// The box of the pixels of `cover` (row-major over `area`, `rw` wide) that `holds`, or `None` when
/// no pixel does.
fn touched_box(cover: &[f32], area: Rect, rw: usize, holds: impl Fn(f32) -> bool) -> Option<Rect> {
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for (i, c) in cover.iter().enumerate() {
        if holds(*c) {
            let (y, x) = (area.y0 + (i / rw) as i32, area.x0 + (i % rw) as i32);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
    }
    (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1, y1))
}

/// The reference application's Quick Selection alert in the modes it refuses (#12934).
/// Multichannel and Duotone weren't measured and keep selecting.
pub fn quick_mode_refusal(mode: photocraft_color::ColorMode) -> Option<&'static str> {
    use photocraft_color::ColorMode;
    match mode {
        ColorMode::Indexed => {
            Some("Could not use the Quick Selection because it does not work with Indexed Color images (convert image to RGB Color mode to edit).")
        }
        ColorMode::Bitmap => Some("Could not use the Quick Selection because it does not work with Bitmap images (convert image to Grayscale mode to edit)."),
        _ => None,
    }
}

/// Expands a selection around sampled points using image gradients, optionally enhancing edges.
fn quick_selection(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some(why) = s.active().and_then(|d| quick_mode_refusal(d.doc.mode)) {
        return Err(EngineError::Other(why.into()));
    }
    let pts = points(p);
    if pts.is_empty() {
        return Err(bad("select.quick", "needs at least one point"));
    }
    let size = f(p, "size", 30.0).clamp(1.0, 5000.0);
    let m = match p.get("mode").and_then(Value::as_str).unwrap_or("add") {
        "subtract" => SelectionMode::Subtract,
        "replace" | "new" => SelectionMode::Replace,
        _ => SelectionMode::Add,
    };
    let enhance = b(p, "enhanceEdge", false);
    let all_layers = b(p, "sampleAllLayers", false);
    let region = with_quick_sampler(s, all_layers, |smp, doc| {
        let canvas = doc.bounds();
        let Some(r) = quick::quick_select(smp, canvas, &pts, size) else { return Ok(None) };
        let surface = matting::region_surface(&r);
        if !enhance {
            return Ok(Some(surface));
        }
        // Enhance Edge: snap the edge to the image, then leave the soft rim the reference measures.
        let refined = matting::refine_surface(smp, &surface, canvas, &ENHANCE, None).map_err(|e| EngineError::Other(e.to_string()))?;
        refined.map(|cut| enhance_rim(&cut, canvas)).transpose().map_err(|e| EngineError::Other(e.to_string()))
    })??;
    let Some(region) = region else { return apply(s, "Quick Selection", None, m) };
    let bounds = region.content_bounds();
    // Only for the layer's pixels as the target: a targeted layer mask or a single channel (not
    // measured) keeps today's result.
    let active = s.active().and_then(|d| d.active_layer).filter(|_| pixels_targeted(s));
    let selected = s.edit("Quick Selection", |doc, _| {
        let empty = photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8);
        let old = doc.selection.as_ref().unwrap_or(&empty);
        let area = old.content_bounds().union(&bounds).intersect(&doc.bounds());
        let out = match m {
            SelectionMode::Replace => region.clone(),
            SelectionMode::Add => photocraft_algo::selection::combine_sparse(old, &region, area, f32::max),
            SelectionMode::Subtract => photocraft_algo::selection::combine_sparse(old, &region, area, |a, b| (a - b).max(0.0)),
            SelectionMode::Intersect => photocraft_algo::selection::combine_surfaces(old, &region, area, f32::min),
        };
        // Sample All Layers off on a pixel or type layer: the stroke also drops the layer's fully
        // transparent pixels from the selection it leaves, in every mode (#12919).
        let out = match active.filter(|_| !all_layers).and_then(|id| doc.layer(id)) {
            Some(layer) => drop_transparent(layer, out),
            None => out,
        };
        doc.selection = (!out.content_bounds().is_empty()).then_some(out);
        Ok(doc.selection.is_some())
    })?;
    Ok(json!({"selected": selected, "changed": true, "bounds": [bounds.x0, bounds.y0, bounds.width(), bounds.height()]}))
}

/// `sel` without the pixels that are fully transparent on `layer`, for a pixel or type layer
/// (Quick Selection with Sample All Layers off, #12919). Other layers (smart objects, shapes,
/// fills, groups; not measured) leave it as it is.
///
/// It visits the selection's own tiles, one at a time, so a stroke costs the tiles the selection
/// holds, not its bounding box: two small pieces far apart don't make a canvas-sized RGBA buffer.
fn drop_transparent(layer: &Layer, sel: photocraft_raster::Surface) -> photocraft_raster::Surface {
    if !matches!(layer.content, LayerContent::Raster(_) | LayerContent::Text(_)) {
        return sel;
    }
    let Some(surf) = layer.surface() else { return sel };
    let coords: Vec<photocraft_geom::TileCoord> = sel.tiles().map(|(tc, _)| *tc).collect();
    let mut out = sel.clone();
    for tc in coords {
        let rect = tc.rect();
        let mask: Vec<u8> = SurfaceSampler(surf).rgba(rect).iter().map(|p| if p[3] > 0.0 { 255 } else { 0 }).collect();
        let opaque = matting::region_surface(&Region { bbox: rect, mask });
        let kept = photocraft_algo::selection::combine_surfaces(&sel, &opaque, rect, f32::min);
        match kept.tile(tc) {
            Some(tile) => out.put_tiles([(tc, std::sync::Arc::clone(tile))]),
            // `take_tiles` only allocates for a tile that is missing; this one is there.
            None => drop(out.take_tiles(rect)),
        }
    }
    out
}

fn rect_param(p: &Value) -> Option<Rect> {
    let a = p.get("rect")?.as_array()?;
    let v: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
    if v.len() < 4 {
        return None;
    }
    // Negative sizes (dragging up/left) are normalised.
    let (x0, x1) = (v[0].min(v[0] + v[2]), v[0].max(v[0] + v[2]));
    let (y0, y1) = (v[1].min(v[1] + v[3]), v[1].max(v[1] + v[3]));
    Some(Rect::new(x0.floor() as i32, y0.floor() as i32, x1.ceil() as i32, y1.ceil() as i32))
}

fn point_param(p: &Value) -> Option<(f32, f32)> {
    let a = p.get("point")?.as_array()?;
    let (x, y) = (a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32);
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

/// The layer [`with_doc_sampler`] samples for `layer` / `all_layers`: `None` = the composite.
pub fn sampled_target(doc: &Document, layer: Option<LayerId>, all_layers: bool) -> Option<LayerId> {
    layer.filter(|id| !all_layers && doc.layer(*id).and_then(Layer::surface).is_some())
}

/// Object Finder (#4087): the object Object Selection would select at document point `(x, y)`,
/// sampling `layer` (or the composite). Pure, so the UI can run it on a worker thread while the
/// pointer hovers; `select.object` with `"point"` commits the same result.
pub fn find_object(doc: &Document, layer: Option<LayerId>, all_layers: bool, x: f32, y: f32) -> Option<Region> {
    with_doc_sampler(doc, layer, all_layers, |smp, doc| grabcut::object_at(smp, doc.bounds(), x, y, grabcut::OBJECT_AT_WORK_PX))
}

/// Most objects remembered by [`ObjectFinderCache`].
pub const OBJECT_FINDER_ENTRIES: usize = 12;

/// Object Selection on a session: its Object Finder results (a session slot of this module).
pub trait SmartSelectExt {
    /// Object Selection's Object Finder results (#4087), shared by the hover preview and the
    /// click that commits it; valid while the pixels are unchanged.
    fn object_finder(&self) -> &ObjectFinderCache;
    /// The Object Finder results, to change them.
    fn object_finder_mut(&mut self) -> &mut ObjectFinderCache;
}

impl SmartSelectExt for Session {
    fn object_finder(&self) -> &ObjectFinderCache {
        self.slot_or_default::<ObjectFinderCache>()
    }
    fn object_finder_mut(&mut self) -> &mut ObjectFinderCache {
        self.slot_or_default_mut::<ObjectFinderCache>()
    }
}

/// Object Finder results for one document's pixels (#4087): hovering over an object already
/// found, or clicking it, answers at once. Keyed like [`SubjectCache`] (valid while the pixels
/// are the same, the document held weakly) plus the sampled layer; a lookup hits any cached
/// object that covers the point, so one computation serves the whole object.
#[derive(Debug, Default)]
pub struct ObjectFinderCache {
    doc: std::sync::Weak<Document>,
    doc_id: Option<photocraft_doc::DocId>,
    target: Option<LayerId>,
    regions: Vec<std::sync::Arc<Region>>,
}

impl ObjectFinderCache {
    fn valid(&self, doc: &Document, target: Option<LayerId>) -> bool {
        self.doc_id == Some(doc.id) && self.target == target && self.doc.upgrade().is_some_and(|old| crate::layer_multi_cmds::same_pixels(&old, doc))
    }

    /// A cached object of `doc` (sampling `target`) covering `(x, y)`.
    pub fn lookup(&self, doc: &Document, target: Option<LayerId>, x: f32, y: f32) -> Option<std::sync::Arc<Region>> {
        if !x.is_finite() || !y.is_finite() || !self.valid(doc, target) {
            return None;
        }
        let (px, py) = (x.floor() as i32, y.floor() as i32);
        let region = self.regions.iter().find(|r| r.at(px, py) >= 0.5)?;
        let can_seed = with_doc_sampler(doc, target, false, |sampler, doc| grabcut::can_seed_object(sampler, doc.bounds(), px, py));
        can_seed.then(|| region.clone())
    }

    /// Remembers `region`, found in `doc` sampling `target`. A different document or pixels
    /// start over; the oldest object is dropped beyond [`OBJECT_FINDER_ENTRIES`].
    pub fn insert(&mut self, doc: &std::sync::Arc<Document>, target: Option<LayerId>, region: std::sync::Arc<Region>) {
        if !self.valid(doc, target) {
            self.regions.clear();
        }
        self.doc = std::sync::Arc::downgrade(doc);
        self.doc_id = Some(doc.id);
        self.target = target;
        if self.regions.len() >= OBJECT_FINDER_ENTRIES {
            self.regions.remove(0);
        }
        self.regions.push(region);
    }

    /// Number of remembered objects.
    pub fn len(&self) -> usize {
        self.regions.len()
    }

    /// Whether nothing is remembered.
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    /// Forget the results if they were computed from the document `id` (it is closing).
    pub(crate) fn forget_doc(&mut self, id: photocraft_doc::DocId) {
        if self.doc_id == Some(id) {
            *self = Self::default();
        }
    }

    /// Follows `doc` past a selection-only edit (same pixels), so the next lookup still hits.
    fn follow(&mut self, doc: &std::sync::Arc<Document>) {
        if self.doc_id == Some(doc.id) && self.doc.upgrade().is_some_and(|old| crate::layer_multi_cmds::same_pixels(&old, doc)) {
            self.doc = std::sync::Arc::downgrade(doc);
        }
    }
}

/// Object Selection samples only the active layer by default: when that layer has no pixels in
/// `area`, say so instead of quietly finding nothing (#5246; scripts often add an empty layer
/// above the photo first).
fn check_sampled_pixels(doc: &Document, layer: Option<LayerId>, all_layers: bool, area: Rect) -> Result<()> {
    match empty_sampled_area(doc, layer, all_layers, area) {
        Some(msg) => Err(bad("select.object", &format!("{msg}; set \"sampleAllLayers\": true or select a layer with pixels"))),
        None => Ok(()),
    }
}

/// Why sampling `layer` (unless `all_layers`) can find nothing in `area`: the layer is empty, or has no pixels there.
/// `None` = there is something to sample. The click path reports it as an error, the Object Finder hover as a hint.
pub fn empty_sampled_area(doc: &Document, layer: Option<LayerId>, all_layers: bool, area: Rect) -> Option<&'static str> {
    let surf = sampled_target(doc, layer, all_layers).and_then(|id| doc.layer(id)).and_then(Layer::surface)?;
    if !surf.content_bounds().intersect(&area).is_empty() {
        return None;
    }
    Some(if surf.content_bounds().is_empty() { "the active layer is empty" } else { "the active layer has no pixels in that area" })
}

/// Where `select.object` with `"point"` answers #5246's clear message instead of #12863's
/// area answer or silence: a completely empty sampled layer, a point outside the canvas that the
/// sampled layer's pixels do not reach, or a spot with no pixels on a Smart Object or type
/// target - those layers' clicks keep their pre-#12863 answer (#15472 review). The Object Finder
/// hover hints on exactly these points, so hover and click agree in every state (#15472 review).
pub fn point_reports_sampled_pixels_error(doc: &Document, layer: Option<LayerId>, all_layers: bool, area: Rect) -> bool {
    let Some(target) = sampled_target(doc, layer, all_layers) else { return false };
    let Some(l) = doc.layer(target) else { return false };
    if matches!(l.content, LayerContent::Smart(_) | LayerContent::Text(_)) {
        return empty_sampled_area(doc, layer, all_layers, area).is_some();
    }
    // Layer pixels can hang past the canvas edge (#3043); there #5246's own content test has
    // nothing to report, and this predicate must not promise a message the checker stays silent
    // about (#15472 review).
    let Some(surf) = l.surface() else { return false };
    let content = surf.content_bounds();
    content.is_empty() || (!doc.bounds().contains(area.x0, area.y0) && content.intersect(&area).is_empty())
}

/// Hard Edge off: [`soften_edge`]'s horizontal taps in 256ths, for the pixels two and one to the
/// left, the pixel itself and the pixel to the right.
const SOFT_EDGE_ACROSS: [f64; 4] = [30.0 / 256.0, 6.0 / 256.0, 118.0 / 256.0, 102.0 / 256.0];
/// Hard Edge off: [`soften_edge`]'s vertical taps in 256ths, for the pixels two and one above,
/// the pixel itself and the pixel below.
const SOFT_EDGE_DOWN: [f64; 4] = [29.0 / 256.0, 11.0 / 256.0, 99.0 / 256.0, 117.0 / 256.0];
/// Hard Edge off: the filtered coverage that maps to 0 and the one that maps to 1 (#12855).
const SOFT_EDGE_RAMP: (f64, f64) = (0.10, 0.55);

/// Object Selection with Hard Edge off (#12855): `region` with the reference application's
/// softer rim. A four-tap filter runs across ([`SOFT_EDGE_ACROSS`]) and then down
/// ([`SOFT_EDGE_DOWN`]), and a ramp maps [`SOFT_EDGE_RAMP`] to 0..1. On the reference
/// application's test rectangle that softens both sides by one pixel, as it measures: the outer
/// pixel on the left drops to about 176 and on the top to about 197, the old rim pixel on the
/// right and bottom becomes fully selected, a new rim pixel of about 65 follows it, and then a
/// line of value 1, which sets the right and bottom bounds. The result can reach one pixel
/// further out on the left and top and two on the right and bottom. Pixels past `canvas` repeat
/// the canvas edge, so an object that touches the document border stays fully selected there.
fn soften_edge(region: &Region, canvas: Rect) -> Region {
    let src = region.bbox;
    let bbox = Rect::new(src.x0.saturating_sub(1), src.y0.saturating_sub(1), src.x1.saturating_add(2), src.y1.saturating_add(2)).intersect(&canvas);
    if bbox.is_empty() || src.is_empty() {
        return region.clone();
    }
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let at = |x: i32, y: i32| -> f64 {
        let (x, y) = (x.clamp(canvas.x0, canvas.x1 - 1), y.clamp(canvas.y0, canvas.y1 - 1));
        if !src.contains(x, y) {
            return 0.0;
        }
        let (cx, cy) = (x.abs_diff(src.x0) as usize, y.abs_diff(src.y0) as usize);
        if cx >= sw || cy >= sh {
            return 0.0;
        }
        region.mask.get(cy * sw + cx).map_or(0.0, |v| f64::from(*v) / 255.0)
    };
    let (w, h) = (bbox.width() as usize, bbox.height() as usize);
    let (lo, hi) = SOFT_EDGE_RAMP;
    // The vertical pass reads two rows above and one below, so the horizontal pass keeps four
    // rows, not the whole box: a large found object costs its 8-bit mask, not eight bytes a pixel
    // more.
    let across = |y: i32, row: &mut Vec<f64>| {
        row.clear();
        row.extend((bbox.x0..).take(w).map(|x| SOFT_EDGE_ACROSS.iter().zip(-2i32..).map(|(k, dx)| k * at(x.saturating_add(dx), y)).sum::<f64>()));
    };
    // `rows` holds the horizontal pass of rows y - 2, y - 1, y and y + 1.
    let mut rows: [Vec<f64>; 4] = std::array::from_fn(|_| Vec::with_capacity(w));
    for (dy, row) in (-2i32..).zip(rows.iter_mut().take(3)) {
        across(bbox.y0.saturating_add(dy), row);
    }
    let mut mask = Vec::with_capacity(w * h);
    for y in bbox.y0..bbox.y1 {
        across(y.saturating_add(1), &mut rows[3]);
        let filtered = (0..w).map(|i| SOFT_EDGE_DOWN.iter().zip(&rows).map(|(k, row)| k * row[i]).sum::<f64>());
        mask.extend(filtered.map(|v| (((v - lo) / (hi - lo)).clamp(0.0, 1.0) * 255.0).round() as u8));
        rows.rotate_left(1);
    }
    trimmed(Region { bbox, mask }, w)
}

/// `r` (`w` pixels wide) cut down to its selected pixels, so the reported bounds are the
/// selection's: [`soften_edge`]'s box has a spare pixel on each side that the ramp may empty, and
/// [`area_region`]'s cut mask keeps the extent its cut leaves. The kept rows and columns are
/// compacted in the buffer that is already owned, so trimming a canvas-sized mask never needs a
/// second one and never aborts on that allocation (#15472 review). Rows and columns that are empty
/// only inside the used extent stay, as they do for a found object with a hole.
fn trimmed(mut r: Region, w: usize) -> Region {
    let rows = r.mask.chunks_exact(w).count();
    let row_used = |y: usize| r.mask[y * w..(y + 1) * w].iter().any(|v| *v > 0);
    let col_used = |x: usize| r.mask.chunks_exact(w).any(|row| row[x] > 0);
    let (Some(y0), Some(y1), Some(x0), Some(x1)) =
        ((0..rows).find(|&y| row_used(y)), (0..rows).rev().find(|&y| row_used(y)), (0..w).find(|&x| col_used(x)), (0..w).rev().find(|&x| col_used(x)))
    else {
        return Region { bbox: Rect::new(r.bbox.x0, r.bbox.y0, r.bbox.x0, r.bbox.y0), mask: Vec::new() };
    };
    let at = |d: usize| i32::try_from(d).unwrap_or(i32::MAX);
    let bbox = Rect::new(
        r.bbox.x0.saturating_add(at(x0)),
        r.bbox.y0.saturating_add(at(y0)),
        r.bbox.x0.saturating_add(at(x1 + 1)),
        r.bbox.y0.saturating_add(at(y1 + 1)),
    );
    // Kept row `y` lands at row `y - y0`, and its kept columns start at `x0 >= 0`, so every copy
    // moves towards the front of the buffer and `copy_within` handles the rows it overlaps.
    let whole_buffer = y0 == 0 && x0 == 0 && y1 + 1 == rows && x1 + 1 == w && r.mask.len() == rows * w;
    if !whole_buffer {
        let (kept, mut write) = (x1 - x0 + 1, 0usize);
        for y in y0..=y1 {
            r.mask.copy_within(y * w + x0..y * w + x0 + kept, write);
            write += kept;
        }
        r.mask.truncate(write);
    }
    Region { bbox, mask: r.mask }
}

/// `select.object`'s `"hardEdge"` (default on): off, the found object gets [`soften_edge`]'s rim;
/// on, the found object itself, borrowed rather than copied.
fn object_edge<'a>(canvas: Rect, p: &Value, region: Option<&'a Region>) -> Option<Cow<'a, Region>> {
    let region = region?;
    Some(if b(p, "hardEdge", true) { Cow::Borrowed(region) } else { Cow::Owned(soften_edge(region, canvas)) })
}

/// #12863's partly-selected coverage: the reference application's model-style area answers
/// never mark a pixel fully selected; every touched pixel sits at 254 (99.6 %).
const MODEL_AREA_COVERAGE: u8 = 254;

/// The whole canvas at [`MODEL_AREA_COVERAGE`], with `cut`'s at-least-half-opaque pixels at 0
/// (the canvas minus the sampled layer's own pixels). `cut` is read in row bands, so a large
/// layer is never held whole; the mask is reserved fallibly, and a canvas whose coverage would
/// not fit in memory answers `None` (the plain nothing-found, like an out-of-memory canvas on
/// the size check). A cut answer reports [`trimmed`]'s bounds - whole empty mask rows and
/// columns at the edges go, as they do for found objects - so the reported bounds are the
/// selection's extent (#15472 review); the uncut canvas keeps the whole canvas.
fn area_region(canvas: Rect, cut: Option<&photocraft_raster::Surface>) -> Option<Region> {
    let (cw, ch) = (canvas.width() as usize, canvas.height() as usize);
    let len = cw.checked_mul(ch)?;
    let mut mask = Vec::new();
    mask.try_reserve_exact(len).ok()?;
    mask.resize(len, MODEL_AREA_COVERAGE);
    if let Some(surf) = cut {
        const BAND: i32 = 64;
        let part = surf.content_bounds().intersect(&canvas);
        let smp = SurfaceSampler(surf);
        for y0 in (part.y0..part.y1).step_by(BAND as usize) {
            let band = Rect::new(part.x0, y0, part.x1, (y0 + BAND).min(part.y1));
            let bw = band.width() as usize;
            for (i, p) in smp.rgba(band).iter().enumerate() {
                if p[3] >= 0.5 {
                    let (x, y) = (band.x0 + (i % bw) as i32, band.y0 + (i / bw) as i32);
                    if let Some(m) = mask.get_mut((y - canvas.y0) as usize * cw + (x - canvas.x0) as usize) {
                        *m = 0;
                    }
                }
            }
        }
    }
    Some(if cut.is_some() { trimmed(Region { bbox: canvas, mask }, cw) } else { Region { bbox: canvas, mask } })
}

/// Opaque 8-connected areas in the `w` x `h` scan, counting up to `stop + 1` and stopping.
/// Every index below is a fresh `0..opaque.len()` counter or checked against `opaque.len()`
/// before use, and `seen` is built to `opaque.len()`.
fn opaque_areas(opaque: &[bool], w: usize, h: usize, stop: usize) -> usize {
    let mut seen = vec![false; opaque.len()];
    let mut areas = 0;
    for start in 0..opaque.len() {
        if !opaque[start] || seen[start] {
            continue;
        }
        areas += 1;
        if areas > stop {
            return areas;
        }
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(j) = stack.pop() {
            let (jx, jy) = ((j % w) as i64, (j / w) as i64);
            for dy in -1..=1i64 {
                for dx in -1..=1i64 {
                    let (nx, ny) = (jx + dx, jy + dy);
                    if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        continue;
                    }
                    let k = (ny * w as i64 + nx) as usize;
                    if k < opaque.len() && opaque[k] && !seen[k] {
                        seen[k] = true;
                        stack.push(k);
                    }
                }
            }
        }
    }
    areas
}

/// Whether every raw pixel of `area` on `surf` is at least half opaque and identical: one flat
/// area. Identical stored samples convert to identical floats, so the exact compare is honest on
/// raw pixels - the box-averaged working scan below is not: a canvas whose bounds are not a
/// multiple of the step averages its edge boxes over different counts, and the partial sums sit
/// an ulp apart for most 8-bit colours (#15472 review). Read in row bands, giving up at the
/// first difference.
fn surface_is_flat(surf: &photocraft_raster::Surface, area: Rect) -> bool {
    const BAND: i32 = 64;
    let smp = SurfaceSampler(surf);
    let mut first: Option<[f32; 4]> = None;
    for y0 in (area.y0..area.y1).step_by(BAND as usize) {
        let band = Rect::new(area.x0, y0, area.x1, (y0 + BAND).min(area.y1));
        for p in smp.rgba(band) {
            if p[3] < 0.5 {
                return false;
            }
            match first {
                None => first = Some(p),
                Some(f) if f == p => {}
                Some(_) => return false,
            }
        }
    }
    first.is_some()
}

/// The click answers the reference application measures for flat targets (#12863): with a
/// sampled layer uniform over the whole canvas, or with a plain group active and Sample All
/// Layers off, the whole canvas; on an empty spot of a sampled layer holding exactly one
/// opaque object, the canvas with that layer's pixels cut out. A click on the opaque part of
/// its layer (its own pixel, at full resolution), a layer with several objects (measured on
/// Shapes: nothing found), and every other target (composite, adjustment, Smart Object, type)
/// answer `None` and keep the plain nothing-found - Smart Object and type are filtered by
/// content kind, because [`Layer::surface`] hands their rendered cache here and those clicks
/// keep their answer from before #12863, as do a targeted layer mask and channel, which the
/// caller skips ([`pixels_targeted`], #15472 review). Uniformity is probed on the raw pixels
/// over the canvas ([`surface_is_flat`]): a solid area floating on transparency is one object to
/// cut out, not a uniform layer; the object areas are scanned at the
/// Object Finder's working resolution, where a coarse step can merge areas that sit within one
/// step of each other, which only ever answers "several" for detail finer than the scan.
/// `target` is [`sampled_target`]'s sampled layer.
fn flat_area_answer(doc: &Document, target: Option<LayerId>, group: bool, x: f32, y: f32) -> Option<Region> {
    let canvas = doc.bounds();
    let (px, py) = (x.floor() as i32, y.floor() as i32);
    if !canvas.contains(px, py) {
        return None;
    }
    if group {
        return area_region(canvas, None);
    }
    let surf = target.and_then(|id| doc.layer(id)).filter(|l| !matches!(l.content, LayerContent::Smart(_) | LayerContent::Text(_))).and_then(Layer::surface)?;
    let content = surf.content_bounds().intersect(&canvas);
    if content.is_empty() {
        return None;
    }
    if surface_is_flat(surf, canvas) {
        return area_region(canvas, None);
    }
    let step = photocraft_algo::segment::scale_for(content, grabcut::OBJECT_AT_WORK_PX);
    let rgba = SurfaceSampler(surf).rgba_scaled(content, step);
    let (w, h) = ((content.width() as usize).div_ceil(step), (content.height() as usize).div_ceil(step));
    if w == 0 || h == 0 || rgba.len() < w * h {
        return None;
    }
    let rgba = &rgba[..w * h];
    let opaque: Vec<bool> = rgba.iter().map(|p| p[3] >= 0.5).collect();
    // The opaque-part gate reads the clicked pixel itself, not the averaged working cell: a small
    // transparent spot inside an opaque area averages to opaque there, and the click would answer
    // nothing-found where the documented answer is the canvas with the object cut out. Object
    // Finder's seed check reads the single pixel for the same reason (#13070, #15472 review);
    // pixels past the layer's content sample as transparent, so no bounds test is needed.
    if SurfaceSampler(surf).rgba(Rect::from_xywh(px, py, 1, 1)).first().is_some_and(|p| p[3] >= 0.5) {
        return None;
    }
    if opaque_areas(&opaque, w, h, 1) != 1 {
        return None;
    }
    area_region(canvas, Some(surf))
}

/// Does sampling `layer` (unless `all_layers`) find no pixel at all in `area`? Unlike
/// [`empty_sampled_area`] this also reads the pixels, so a transparent gap inside the layer's
/// content bounds counts as empty too.
fn no_sampled_pixels(doc: &Document, layer: Option<LayerId>, all_layers: bool, area: Rect) -> bool {
    let Some(surf) = sampled_target(doc, layer, all_layers).and_then(|id| doc.layer(id)).and_then(Layer::surface) else {
        return false;
    };
    let r = surf.content_bounds().intersect(&area);
    let fmt = surf.format();
    // Bands of rows, so a huge rectangle is not read whole when its first pixels are opaque.
    const BAND: i32 = 64;
    (r.y0..r.y1).step_by(BAND as usize).all(|y| {
        let band = Rect { y0: y, y1: (y + BAND).min(r.y1), ..r };
        surf.read_region(band).chunks_exact(fmt.channels()).all(|q| to_rgba(&fmt, q)[3] <= 0.0)
    })
}

/// `select.object` with `"point"`: the Object Finder's object under the point (cached), else
/// the reference application's area answer for a flat target (#12863).
fn object_at_point(s: &mut Session, p: &Value, (x, y): (f32, f32)) -> Result<Value> {
    let all_layers = b(p, "sampleAllLayers", false);
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let (px, py) = (x.floor() as i32, y.floor() as i32);
    let target = sampled_target(&d.doc, d.active_layer, all_layers);
    // #5246's clear answer stands for the cases its comment names - a completely empty sampled
    // layer and a point outside the canvas - and, as before #12863, for a Smart Object or type
    // target with no pixels under the point (#15472 review). Inside the canvas, a pixel layer
    // with pixels elsewhere is answered like the reference application instead (#12863), not
    // by the error.
    let here = Rect::from_xywh(px, py, 1, 1);
    if point_reports_sampled_pixels_error(&d.doc, d.active_layer, all_layers, here) {
        check_sampled_pixels(&d.doc, d.active_layer, all_layers, here)?;
    }
    let group = plain_group_active(&d.doc, d.active_layer, all_layers);
    let doc = std::sync::Arc::clone(&d.doc);
    let active = d.active_layer;
    let found = if group {
        // A plain group's click answers from the whole canvas (#12863), not from the composite
        // fallback `with_doc_sampler` gives it; the hover cache holds composite objects and is
        // not consulted.
        None
    } else {
        match s.object_finder().lookup(&doc, target, x, y) {
            Some(r) => Some(r),
            None => {
                let found = find_object(&doc, active, all_layers, x, y).map(std::sync::Arc::new);
                if let Some(r) = &found {
                    s.object_finder_mut().insert(&doc, target, std::sync::Arc::clone(r));
                }
                found
            }
        }
    };
    // A found object wins; the area answer is not cached (it is the layer's flatness, not a
    // remembered object, and a canvas-sized mask in the cache would swallow later real clicks).
    // #12863 measures the area answers with the layer's pixels as the edit target; with a targeted
    // layer mask or single channel the click keeps its pre-#12863 nothing-found (#15472 review).
    let area = if found.is_none() && pixels_targeted(s) { flat_area_answer(&doc, target, group, x, y) } else { None };
    // Hard Edge off (#12855) reshapes found objects only: soften_edge's taps sum to 1.0 and its
    // ramp maps the area answers' 254 back to fully selected, so an area answer keeps the
    // measured partly-selected coverage as it is (#15472 review).
    let region = match found.as_deref().or(area.as_ref()) {
        Some(r) if found.is_some() => object_edge(canvas, p, Some(r)),
        Some(r) => Some(Cow::Borrowed(r)),
        None => None,
    };
    let out = apply(s, "Object Selection", region.as_deref(), mode(p, "replace"))?;
    if let Some(d) = s.active() {
        let doc = std::sync::Arc::clone(&d.doc);
        s.object_finder_mut().follow(&doc);
    }
    Ok(out)
}

/// The reference application's Object Selection alert in the modes it refuses (#12867).
/// Multichannel and Duotone weren't measured and keep selecting.
pub fn object_mode_refusal(mode: photocraft_color::ColorMode) -> Option<&'static str> {
    use photocraft_color::ColorMode;
    match mode {
        ColorMode::Indexed => Some(
            "Could not complete your request because the tool does not work with Indexed Color images. Please convert the image to RGB or another supported mode before using..",
        ),
        ColorMode::Bitmap => Some(
            "Could not complete your request because the tool does not work with Bitmap images. Please convert the image to RGB or another supported mode before using..",
        ),
        _ => None,
    }
}

fn object_selection(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some(why) = s.active().and_then(|d| object_mode_refusal(d.doc.mode)) {
        return Err(EngineError::Other(why.into()));
    }
    let record_empty = match p.get("recordEmptyArea") {
        None => false,
        Some(Value::Bool(v)) => *v,
        Some(_) => return Err(bad("select.object", "`recordEmptyArea` must be true or false")),
    };
    if p.get("rect").is_none()
        && let Some(pt) = point_param(p)
    {
        return object_at_point(s, p, pt);
    }
    let rect = rect_param(p).ok_or_else(|| bad("select.object", "needs \"rect\": [x, y, w, h] or \"point\": [x, y]"))?;
    let all_layers = b(p, "sampleAllLayers", false);
    let d = s.active().ok_or(EngineError::NoDocument)?;
    // The tool's drag over an empty area: the reference application finds nothing but still
    // records the step (#12861); scripts keep the #5246 error.
    if record_empty && no_sampled_pixels(&d.doc, d.active_layer, all_layers, rect) {
        let selected = s.edit("Object Selection", |doc, _| Ok(doc.selection.is_some()))?;
        return Ok(json!({ "selected": selected, "changed": false }));
    }
    check_sampled_pixels(&d.doc, d.active_layer, all_layers, rect)?;
    let canvas = d.doc.bounds();
    let found = with_sampler(s, all_layers, |smp, doc| grabcut::object_select(smp, doc.bounds(), rect, 160_000))?;
    apply(s, "Object Selection", object_edge(canvas, p, found.as_ref()).as_deref(), mode(p, "replace"))
}

/// Select Subject's last result (#2865), so running it again on unchanged pixels answers at
/// once instead of recomputing superpixels, saliency and GrabCut (P29). Keyed by the pixels it
/// was computed from: valid while [`same_pixels`](crate::layer_multi_cmds::same_pixels) (a
/// tile-pointer comparison) holds against that document, so selection-only edits such as
/// Deselect keep it. One entry per session; the document is held weakly, so the cache never
/// keeps a closed or edited document's tiles alive (the session also drops it on pixel edits
/// and when its document closes). Its memory is the mask: one byte per pixel of its bounds.
#[derive(Debug, Default)]
pub struct SubjectCache {
    doc: std::sync::Weak<Document>,
    doc_id: Option<photocraft_doc::DocId>,
    /// The sampled layer; `None` = the composite.
    target: Option<LayerId>,
    region: Option<std::sync::Arc<Region>>,
}

impl SubjectCache {
    /// Forget the result if it was computed from the document `id` (it is closing).
    pub(crate) fn forget_doc(&mut self, id: photocraft_doc::DocId) {
        if self.doc_id == Some(id) {
            *self = Self::default();
        }
    }
}

fn select_subject(s: &mut Session, p: &Value) -> Result<Value> {
    // The selected layer's pixels; the composite only when no pixel layer is selected.
    let all_layers = b(p, "sampleAllLayers", false);
    let d = s.active().ok_or(EngineError::NoDocument)?;
    // What `with_doc_sampler` samples: the active layer's pixels, else the composite.
    let target = d.active_layer.filter(|id| !all_layers && d.doc.layer(*id).and_then(Layer::surface).is_some());
    let c = s.slot_or_default::<SubjectCache>();
    let hit = c.target == target && c.doc.upgrade().is_some_and(|old| crate::layer_multi_cmds::same_pixels(&old, &d.doc));
    let region = if hit {
        c.region.clone()
    } else {
        let doc = std::sync::Arc::clone(&d.doc);
        let region = with_doc_sampler(&doc, d.active_layer, all_layers, |smp, doc| subject::select_subject(smp, doc.bounds())).map(std::sync::Arc::new);
        *s.slot_or_default_mut::<SubjectCache>() = SubjectCache { doc: std::sync::Arc::downgrade(&doc), doc_id: Some(doc.id), target, region: region.clone() };
        region
    };
    let out = apply(s, "Select Subject", region.as_deref(), mode(p, "replace"))?;
    // Follow the document past the selection edit just made (same pixels), so a short history
    // dropping the older state doesn't turn the next repeat into a miss.
    let cache = s.slot_or_default::<SubjectCache>();
    let follow = s
        .active()
        .filter(|d| cache.doc_id == Some(d.doc.id) && cache.doc.upgrade().is_some_and(|old| crate::layer_multi_cmds::same_pixels(&old, &d.doc)))
        .map(|d| std::sync::Arc::downgrade(&d.doc));
    if let Some(doc) = follow {
        s.slot_or_default_mut::<SubjectCache>().doc = doc;
    }
    Ok(out)
}

fn focus_area(s: &mut Session, p: &Value) -> Result<Value> {
    let (range, noise) = (f(p, "range", 0.5).clamp(0.0, 1.0), f(p, "noise", 0.0).clamp(0.0, 1.0));
    let region = with_sampler(s, b(p, "sampleAllLayers", true), |smp, doc| focus::focus_area(smp, doc.bounds(), range, noise))?;
    apply(s, "Focus Area", region.as_ref(), mode(p, "replace"))
}

fn refine_params(p: &Value) -> RefineParams {
    RefineParams {
        radius: f(p, "radius", 0.0).clamp(0.0, 250.0),
        smart_radius: b(p, "smartRadius", false),
        smooth: f(p, "smooth", 0.0).clamp(0.0, 100.0),
        feather: f(p, "feather", 0.0).clamp(0.0, 1000.0),
        contrast: f(p, "contrast", 0.0).clamp(0.0, 100.0),
        shift_edge: f(p, "shiftEdge", 0.0).clamp(-100.0, 100.0),
    }
}

/// Most Refine Edge Brush strokes, and points across them, one Select and Mask takes.
pub const MAX_STROKES: usize = 4096;
pub const MAX_STROKE_POINTS: usize = 100_000;

/// The Refine Edge Brush strokes of `refineStrokes`:
/// `[{"points":[[x,y],…],"size":px,"mode":"add|subtract"}]`, in painting order.
fn refine_strokes(p: &Value) -> Result<Vec<matting::EdgeStroke>> {
    const CMD: &str = "select.refineEdge";
    let Some(v) = p.get("refineStrokes").filter(|v| !v.is_null()) else { return Ok(Vec::new()) };
    let list = v.as_array().ok_or_else(|| bad(CMD, "refineStrokes must be an array of strokes"))?;
    if list.len() > MAX_STROKES {
        return Err(bad(CMD, &format!("refineStrokes holds more than {MAX_STROKES} strokes")));
    }
    let mut total = 0usize;
    let mut strokes = Vec::with_capacity(list.len());
    for stroke in list {
        let o = stroke.as_object().ok_or_else(|| bad(CMD, "each refineStrokes entry must be {\"points\":[[x,y],…],\"size\":px,\"mode\":\"add|subtract\"}"))?;
        let pts = o.get("points").and_then(Value::as_array).filter(|a| !a.is_empty()).ok_or_else(|| bad(CMD, "each stroke needs a non-empty points array"))?;
        total = total.saturating_add(pts.len());
        if total > MAX_STROKE_POINTS {
            return Err(bad(CMD, &format!("refineStrokes holds more than {MAX_STROKE_POINTS} points")));
        }
        let points = pts
            .iter()
            .map(|pt| {
                let xy = pt.as_array().filter(|a| a.len() == 2)?;
                let (x, y) = (xy.first()?.as_f64()?, xy.get(1)?.as_f64()?);
                let ok = |v: f64| v.is_finite() && v.abs() <= 1e7;
                (ok(x) && ok(y)).then_some((x as f32, y as f32))
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| bad(CMD, "stroke points must be [x,y] pairs of finite numbers"))?;
        let range = matting::BRUSH_SIZE_RANGE;
        let size = match o.get("size") {
            None => 30.0,
            Some(v) => v
                .as_f64()
                .filter(|s| s.is_finite() && (f64::from(*range.start())..=f64::from(*range.end())).contains(s))
                .ok_or_else(|| bad(CMD, &format!("stroke size must be {}..{} px", range.start(), range.end())))? as f32,
        };
        let add = match o.get("mode").map(|m| m.as_str()) {
            None | Some(Some("add")) => true,
            Some(Some("subtract")) => false,
            Some(_) => return Err(bad(CMD, "stroke mode must be add|subtract")),
        };
        strokes.push(matting::EdgeStroke { points, size, add });
    }
    if matting::swept_area(&strokes) > matting::MAX_SWEPT_AREA {
        return Err(bad(CMD, "refineStrokes paint too large an area"));
    }
    Ok(strokes)
}

/// Select and Mask: applies the workspace's tool `edits` to the selection (or layer mask), refines
/// its edge, then outputs to the selection, a layer mask on the active layer, a new layer or a new
/// document (each with or without a mask).
/// Decontaminating colours needs new pixels, so it upgrades `selection` / `layerMask` output to
/// `newLayerWithMask` (as Photoshop does). New-layer outputs hide the source layer; new-document
/// outputs leave the source document untouched.
fn refine_edge(s: &mut Session, p: &Value) -> Result<Value> {
    let params = refine_params(p);
    let strokes = refine_strokes(p)?;
    let decontaminate = b(p, "decontaminate", false);
    let amount = f(p, "amount", 100.0).clamp(0.0, 100.0);
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let active = d.active_layer.filter(|id| d.doc.layer(*id).is_some());
    let default_output =
        if d.doc.selection.is_none() && active.and_then(|id| d.doc.layer(id)).and_then(|l| l.mask.as_ref()).is_some() { "layerMask" } else { "selection" };
    let mut output = p.get("output").and_then(Value::as_str).unwrap_or(default_output).to_string();
    if !matches!(output.as_str(), "selection" | "layerMask" | "newLayer" | "newLayerWithMask" | "newDocument" | "newDocumentWithMask") {
        return Err(bad("select.refineEdge", "output must be selection|layerMask|newLayer|newLayerWithMask|newDocument|newDocumentWithMask"));
    }
    if decontaminate && matches!(output.as_str(), "selection" | "layerMask") {
        output = "newLayerWithMask".into();
    }
    let edits = select_mask_edits::parse(p)?;
    // Before anything is painted: a thin stroke across a huge canvas paints few pixels but many tiles.
    select_mask_edits::check_size(&edits, d.doc.bounds())?;
    select_mask_edits::check_strokes(&strokes, d.doc.bounds())?;
    let region = with_sampler(s, b(p, "sampleAllLayers", false), |smp, doc| {
        let mask_surf = active.and_then(|id| doc.layer(id)).and_then(|l| l.mask.as_ref()).map(|m| &m.surface);
        let target_mask = p.get("target").and_then(Value::as_str) == Some("mask");
        let start = if target_mask { mask_surf.or(doc.selection.as_ref()) } else { doc.selection.as_ref().or(mask_surf) };
        // The workspace's tool edits change the selection before its edge is refined.
        let edited = if edits.is_empty() { None } else { select_mask_edits::apply(doc, active, start.cloned(), &edits) };
        let Some(sel) = (if edits.is_empty() { start } else { edited.as_ref() }) else { return Ok(None) };
        let brush = matting::EdgeBrush::new(&strokes, doc.bounds());
        matting::refine_surface(smp, sel, doc.bounds(), &params, brush.as_ref()).map_err(|e| EngineError::Other(e.to_string()))
    })??;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let active = d.active_layer.filter(|id| d.doc.layer(*id).is_some());
    match output.as_str() {
        "selection" => {
            let selected = s.edit("Select and Mask", |doc, _| {
                doc.selection = region.clone();
                Ok(doc.selection.is_some())
            })?;
            Ok(json!({ "output": output, "selected": selected }))
        }
        "layerMask" => {
            let id = active.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            s.edit("Select and Mask", |doc, _| {
                let surface = region.clone().unwrap_or_else(|| photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8));
                crate::extra_cmds::background_to_layer_for_mask(doc, id);
                let existing = doc.layer(id).and_then(|l| l.mask.as_ref()).cloned().unwrap_or_else(LayerMask::reveal_all);
                doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.mask = Some(LayerMask { surface, ..existing });
                doc.selection = None;
                Ok(())
            })?;
            Ok(json!({ "output": output, "layer": id.0, "selected": false }))
        }
        _ => {
            let id = active.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            let src_layer = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            let src = src_layer.surface().ok_or_else(|| {
                EngineError::Other(format!("the active layer is {} {} layer without pixels", src_layer.content.article(), src_layer.content.kind_name()))
            })?;
            let name = d.doc.copy_name(&src_layer.name);
            let empty = photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8);
            let reg = region.as_ref().unwrap_or(&empty);
            let pixels = if decontaminate { matting::decontaminate_surface(src, reg, params.radius, amount) } else { src.clone() };
            let with_mask = matches!(output.as_str(), "newLayerWithMask" | "newDocumentWithMask");
            let (content, mask) = if with_mask {
                (pixels, Some(LayerMask { surface: reg.clone(), ..LayerMask::reveal_all() }))
            } else {
                (matting::masked_copy_surface(&pixels, reg, reg.content_bounds()), None)
            };
            if output.starts_with("newDocument") {
                // Same canvas, colour mode, depth and profile, so the pixels need no conversion.
                let mut doc = Document::new("Untitled", d.doc.size, d.doc.mode, d.doc.depth);
                doc.resolution_dpi = d.doc.resolution_dpi;
                doc.icc_profile = d.doc.icc_profile.clone();
                doc.color_table = d.doc.color_table.clone();
                doc.duotone = d.doc.duotone.clone();
                let mut layer = Layer::new(src_layer.name.clone(), LayerContent::Raster(content));
                layer.mask = mask;
                let layer_id = layer.id;
                doc.layers.push(layer);
                let i = s.add_document(doc, None);
                return Ok(json!({ "output": output, "document": i, "layer": layer_id.0, "selected": false }));
            }
            let new_id = s.edit("Select and Mask", |doc, act| {
                let mut layer = Layer::new(name, LayerContent::Raster(content));
                layer.mask = mask;
                let nid = doc.insert_above(Some(id), layer);
                if let Some(l) = doc.layer_mut(id) {
                    l.visible = false;
                }
                doc.selection = None;
                *act = Some(nid);
                Ok(nid)
            })?;
            Ok(json!({ "output": output, "layer": new_id.0, "selected": false }))
        }
    }
}

macro_rules! spec {
    ($id:literal, $label:literal, [$($m:literal),*], $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: None, params: $params, enabled: $en, run: $run, journal: true }
    };
}

/// Smart selection command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "select.quick",
            "Quick Selection",
            [],
            r##"{"points":[[x,y],…],"size":px=30,"mode":"add|subtract|replace"="add","sampleAllLayers":bool=false,"enhanceEdge":bool=false}"##,
            has_doc,
            quick_selection
        ),
        spec!(
            "select.object",
            "Object Selection",
            [],
            r##"{"rect":[x,y,w,h] | "point":[x,y],"mode":"replace|add|subtract|intersect"="replace","sampleAllLayers":bool=false,"hardEdge":bool=true,"recordEmptyArea":bool=false} (point: the Object Finder's object under the point, as hovering previews it; hardEdge off: a softer rim, up to one pixel further out on the left and top and two on the right and bottom; recordEmptyArea: a rect where the sampled layer has no pixels answers changed: false and records an "Object Selection" History step that leaves the selection as it is instead of the error, as the tool's drag does)"##,
            has_doc,
            object_selection
        ),
        spec!(
            "select.subject",
            "Subject",
            ["Select"],
            r##"{"sampleAllLayers":bool=false,"mode":"replace|add|subtract|intersect"="replace"} (samples the selected layer; the composite when no pixel layer is selected)"##,
            has_doc,
            select_subject
        ),
        spec!(
            "select.refineEdge",
            "Refine Edge",
            [],
            r##"{"radius":px=0,"smartRadius":bool=false,"smooth":0..100=0,"feather":px=0,"contrast":0..100=0,"shiftEdge":-100..100=0,"decontaminate":bool=false,"amount":0..100=100,"output":"selection|layerMask|newLayer|newLayerWithMask|newDocument|newDocumentWithMask"="selection","sampleAllLayers":bool=false,"refineStrokes":[{"points":[[x,y],…],"size":px=30,"mode":"add|subtract"="add"},…]=[],"edits":[{"tool":"quick","points":[[x,y],…],"size":px=30,"mode":"add|subtract"="add","sampleAllLayers":bool=false} | {"tool":"brush","points":[[x,y],…],"size":px=30,"hardness":0..100=100,"mode":"add|subtract"="add"} | {"tool":"lasso","points":[[x,y],…≥3],"mode":"add|subtract"="add"} | {"tool":"object","rect":[x,y,w,h] | "point":[x,y],"mode":"add|subtract"="add","sampleAllLayers":bool=false} | {"tool":"subject","mode":"replace|add|subtract"="replace","sampleAllLayers":bool=true} | {"tool":"clear"} | {"tool":"invert"},…]=[]} (refineStrokes: the Refine Edge Brush, in painting order: add refines the painted area from the image, subtract keeps the original edge there; edits: the workspace's Quick Selection, Brush, Lasso, Object Selection, Select Subject, Clear Selection and Invert, applied in order to the selection before its edge is refined)"##,
            has_selection_or_mask,
            refine_edge
        ),
        spec!(
            "select.focusArea",
            "Focus Area…",
            ["Select"],
            r##"{"range":0..1=0.5,"noise":0..1=0,"sampleAllLayers":bool=true,"mode":"replace|add|subtract|intersect"="replace"}"##,
            has_doc,
            focus_area
        ),
    ]
}

#[cfg(test)]
mod object_hard_edge_tests;
#[cfg(test)]
mod object_transparent_layer_tests;
#[cfg(test)]
mod object_uniform_target_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod object_mode_refusal_tests;

#[cfg(test)]
mod quick_mode_refusal_tests;
