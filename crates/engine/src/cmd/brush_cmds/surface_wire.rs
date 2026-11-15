//! Surface-aware painting in the engine: the brush's Surface options (see
//! [`photocraft_paint::SurfaceOptions`]) wired to the imaging code.
//!
//! * Surface Color: a [`ShadingField`] of the photo under the stroke (the visible composite, or
//!   the target layer), as the stroke renderer's shade source.
//! * Follow Perspective: the document's saved Vanishing Point planes as the dab generator's
//!   surface map.
//!
//! Fields and plane maps are cached per session: building one costs a pass over the canvas, and a
//! stroke, its live preview and the next stroke on the same document revision share it. The
//! shading always comes from the document as it was before the stroke.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use photocraft_algo::Image;
use photocraft_algo::surface::{DepthGeometry, PlaneSurface, ShadingBuilder, ShadingField};
use photocraft_color::ColorMode;
use photocraft_doc::{DocId, LayerId};
use photocraft_geom::{Point, Rect};
use photocraft_paint::surface::{Footprint, Local, SampleMode, ShadeSource, SurfaceMap, surface_dab_shape};
use photocraft_paint::{BrushSettings, StrokeRenderer};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, is_mask_target};
use crate::{EngineError, Result, Session};

pub const CURSOR_SHAPE: &str = "brush.cursorShape";
/// Most canvas pixels a shading field is built over (2 bytes of luma each; the sample itself is
/// read in bands).
pub(crate) const MAX_SAMPLE_PIXELS: u64 = 1 << 26;
/// Fields and plane maps one session keeps.
const KEEP_FIELDS: usize = 3;
const NO_PLANES: &str = "no perspective planes: define them in Filter ▸ Vanishing Point";

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The paint crate's shade source over an algo [`ShadingField`].
pub(crate) struct FieldShade(pub(crate) Arc<ShadingField>);

impl ShadeSource for FieldShade {
    fn shade(&self, rect: Rect, out: &mut [f32]) {
        self.0.shade_into(rect, out);
    }
}

/// The paint crate's surface map over the algo's [`PlaneSurface`].
pub(crate) struct PlaneMap(pub(crate) PlaneSurface);

impl SurfaceMap for PlaneMap {
    fn local(&self, p: Point) -> Option<Local> {
        self.0.local([p.x, p.y]).map(|(scale, squash_angle, squash)| Local { scale, squash_angle, squash, offset: [0.0; 2] })
    }

    /// The brush size is the size on the first plane's centre.
    fn reference_scale(&self) -> Option<f32> {
        self.0.reference_scale()
    }
}

/// Follow Depth: the document's depth channel (white = near) as a surface map. Over a slope the
/// dab is foreshortened along it, and with `size_by_depth` its size follows the depth (see
/// `photocraft_algo::surface::DepthGeometry::local`, which this adapts). The brush's radius
/// (size / 2) sets the scale the slope is measured at. The brush size is the size at the photo's
/// median depth (its middle distance); without `size_by_depth` the scale is 1 everywhere.
pub(crate) struct DepthMap {
    geometry: Arc<DepthGeometry>,
    radius: f32,
    relief: f32,
    size_by_depth: bool,
}

impl DepthMap {
    pub(crate) fn new(geometry: Arc<DepthGeometry>, radius: f32, relief: f32, size_by_depth: bool) -> Self {
        Self { geometry, radius: if radius.is_finite() { radius.max(0.5) } else { 0.5 }, relief, size_by_depth }
    }

    /// The depth projection at `p`; `None` off the canvas, and where it is flat and unscaled.
    fn local(&self, p: Point) -> Option<Local> {
        let (scale, squash_angle, squash) = self.geometry.local(p.x, p.y, self.radius, self.relief, self.size_by_depth)?;
        if !self.size_by_depth && squash >= 0.9999 {
            return None;
        }
        let local = Local { scale, squash_angle, squash, offset: [0.0; 2] };
        local.is_valid().then_some(local)
    }

    /// The outline of a dab of `radius` pixels at `p` bent over the depth (see
    /// `DepthGeometry::footprint`); `None` at relief 0.
    fn footprint(&self, p: Point, radius: f32) -> Option<Footprint> {
        if self.relief <= 0.0 {
            return None;
        }
        self.geometry.footprint(p.x, p.y, radius, self.relief).map(|rays| Footprint { rays })
    }

    /// The scale the brush size is meant at: the median depth's with `size_by_depth`, else 1.
    fn reference_scale(&self) -> f32 {
        if self.size_by_depth { self.geometry.reference_scale() } else { 1.0 }
    }
}

/// `scale · (I − (1 − squash) · u uᵀ)` with `u` the direction of `squash_angle`: the matrix that
/// maps a circle to the dab's ellipse, in y-up axes.
fn local_matrix(l: &Local) -> [[f64; 2]; 2] {
    let (sn, cs) = f64::from(l.squash_angle).sin_cos();
    let (k, sc) = (1.0 - f64::from(l.squash), f64::from(l.scale));
    [[sc * (1.0 - k * cs * cs), -sc * k * cs * sn], [-sc * k * cs * sn, sc * (1.0 - k * sn * sn)]]
}

/// Perspective `a` followed by depth `b`: their matrices multiplied and decomposed again, so the
/// scale is the plane's times the depth's (their references' product anchors it) and the squashes
/// compose. The offsets add.
fn combine(a: &Local, b: &Local) -> Option<Local> {
    let (m, n) = (local_matrix(a), local_matrix(b));
    let j = [
        [m[0][0] * n[0][0] + m[0][1] * n[1][0], m[0][0] * n[0][1] + m[0][1] * n[1][1]],
        [m[1][0] * n[0][0] + m[1][1] * n[1][0], m[1][0] * n[0][1] + m[1][1] * n[1][1]],
    ];
    let (s1, s2, phi) = photocraft_algo::surface::svd2(j)?;
    let minor = (phi + std::f64::consts::FRAC_PI_2).rem_euclid(2.0 * std::f64::consts::PI);
    let local = Local {
        scale: s1 as f32,
        squash_angle: if minor > std::f64::consts::PI { minor - 2.0 * std::f64::consts::PI } else { minor } as f32,
        squash: (s2 / s1).clamp(0.0, 1.0) as f32,
        offset: [a.offset[0] + b.offset[0], a.offset[1] + b.offset[1]],
    };
    local.is_valid().then_some(local)
}

/// `f` seen through the plane's foreshortening (its squash and direction; its scale is already in
/// the footprint's radius): the outline of the circle-like footprint under the plane's matrix
/// `I − (1 − squash) u uᵀ`, resampled at the footprint's own directions. Each output direction is
/// mapped back through the inverse matrix, where the footprint is read.
fn squash_footprint(f: &Footprint, plane: &Local) -> Option<Footprint> {
    if !plane.squash.is_finite() || !plane.squash_angle.is_finite() {
        return None;
    }
    let squash = plane.squash.clamp(0.05, 1.0);
    let (sn, cs) = plane.squash_angle.sin_cos();
    let k = 1.0 / squash - 1.0;
    let mut rays = f.rays;
    for (i, ray) in rays.iter_mut().enumerate() {
        let (dsn, dcs) = (i as f32 * std::f32::consts::TAU / photocraft_paint::surface::FOOTPRINT_RAYS as f32).sin_cos();
        let along = dcs * cs + dsn * sn;
        let (ux, uy) = (dcs + k * along * cs, dsn + k * along * sn);
        // |u| >= 1, so the ray only shortens.
        *ray = f.radius_at(uy.atan2(ux)) / ux.hypot(uy);
    }
    let out = Footprint { rays };
    out.is_valid().then_some(out)
}

/// Follow Perspective and Follow Depth together. With a plane map, a dab off every plane is left
/// alone (depth or not): the stroke is anchored on the plane. On a plane both apply, as in
/// [`combine`]. Without a plane map the depth scale itself is the size, relative to the median
/// depth.
struct CombinedMap {
    persp: Option<Arc<PlaneMap>>,
    depth: Option<DepthMap>,
}

impl SurfaceMap for CombinedMap {
    fn local(&self, p: Point) -> Option<Local> {
        let plane = match &self.persp {
            Some(m) => Some(m.local(p)?),
            None => None,
        };
        let depth = self.depth.as_ref().and_then(|d| d.local(p));
        match (plane, depth) {
            (Some(a), Some(b)) => combine(&a, &b).or(Some(a)),
            (a, b) => a.or(b),
        }
    }

    /// With depth, its footprint at the dab's own radius (which already holds the plane's scale),
    /// foreshortened by the plane's squash when planes are followed too; with planes only, none:
    /// their ellipse is exact. A footprint replaces the dab's projection in the renderer, so it
    /// must hold the plane's foreshortening itself.
    fn footprint(&self, p: Point, radius: f32, _local: &Local) -> Option<Footprint> {
        let depth = self.depth.as_ref()?;
        // Where the depth map has no projection (flat and unscaled) the dab stays as it is.
        depth.local(p)?;
        let bent = depth.footprint(p, radius)?;
        match &self.persp {
            Some(m) => squash_footprint(&bent, &m.local(p)?),
            None => Some(bent),
        }
    }

    /// The product of the plane's and the depth's references (as [`combine`] multiplies the scales).
    fn reference_scale(&self) -> Option<f32> {
        let plane = match &self.persp {
            Some(m) => Some(m.reference_scale()?),
            None => None,
        };
        let depth = self.depth.as_ref().map(DepthMap::reference_scale);
        match (plane, depth) {
            (Some(a), Some(b)) => Some(a * b),
            (a, b) => a.or(b),
        }
    }
}

/// The dab map for the planes and/or depth a stroke follows.
pub(crate) fn make_map(persp: Option<Arc<PlaneMap>>, depth: Option<DepthMap>) -> Option<Arc<dyn SurfaceMap>> {
    match (persp, depth) {
        (None, None) => None,
        (Some(p), None) => Some(p),
        (persp, depth) => Some(Arc::new(CombinedMap { persp, depth })),
    }
}

/// What a cached shading field was built for. The document's content is compared apart from this
/// ([`same_sources`]), not by revision: a stroke on the target layer must keep the cache.
#[derive(Clone, PartialEq)]
struct FieldKey {
    doc: DocId,
    sample: SampleMode,
    detail: u32,
    /// The layer being painted: hidden from an All Layers sample, the source of a Current Layer one.
    layer: Option<LayerId>,
}

/// A cached field, with the document it was built from (a copy-on-write handle: tiles are shared,
/// and holding it keeps their identity from being reused).
struct FieldEntry {
    key: FieldKey,
    doc: Arc<photocraft_doc::Document>,
    field: Arc<ShadingField>,
}

/// Do `a` and `b` hold the same pixels for a sample of kind `sample` aimed at `layer`?
///
/// No pixel is scanned. Layers compare field by field, and a surface compares its tile table
/// first by `Arc` pointer (`photocraft_raster::Tile` is `Eq` with a pointer shortcut), so a layer
/// no edit has touched costs one pointer test per tile. All Layers ignores the target layer
/// entirely (it is hidden from the sample), so painting on it, like changing the selection or the
/// history, keeps the field; any edit of another layer, or of the canvas, invalidates it. A target
/// inside a group is compared with its group, so there a stroke does invalidate (a rebuild per
/// stroke, as before). All Layers also compares the document's global light (layer
/// styles read it); other document-wide settings (patterns, the colour profile) are not compared.
fn same_sources(a: &photocraft_doc::Document, b: &photocraft_doc::Document, sample: SampleMode, layer: Option<LayerId>) -> bool {
    if a.size != b.size || a.mode != b.mode || a.depth != b.depth {
        return false;
    }
    match sample {
        SampleMode::CurrentLayer => layer.is_some_and(|id| matches!((a.layer(id), b.layer(id)), (Some(x), Some(y)) if x == y)),
        SampleMode::AllLayers => {
            // Layer styles with Use Global Light read the document's light, so it is part of the composite.
            a.global_light == b.global_light
                && a.layers.len() == b.layers.len()
                && a.layers.iter().zip(&b.layers).all(|(x, y)| x.id == y.id && (Some(x.id) == layer || x == y))
        }
    }
}

#[derive(Clone, PartialEq)]
struct MapKey {
    doc: DocId,
    planes: photocraft_doc::PerspectivePlanes,
    size: [u32; 2],
}

/// What a session keeps between strokes (see the module doc). Never serialized.
#[derive(Default)]
pub struct SurfaceCache {
    fields: Mutex<Vec<FieldEntry>>,
    map: Mutex<Option<(MapKey, Arc<PlaneMap>)>>,
    builds: std::sync::atomic::AtomicUsize,
    depth: Mutex<Option<DepthEntry>>,
    depth_builds: std::sync::atomic::AtomicUsize,
}

/// The depth geometry of a channel, with a snapshot of the channel's surface it was built from
/// (tile handles: an edit of the channel makes the snapshot differ; painting elsewhere does not).
struct DepthEntry {
    doc: DocId,
    name: String,
    surface: photocraft_raster::Surface,
    area: Rect,
    geometry: Arc<DepthGeometry>,
}

impl SurfaceCache {
    /// How many shading fields this session has built (a cache hit builds none).
    pub fn field_builds(&self) -> usize {
        self.builds.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// How many depth geometries this session has built.
    pub fn depth_builds(&self) -> usize {
        self.depth_builds.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Shading and perspective for one stroke, ready to attach to its renderers.
#[derive(Default)]
pub(crate) struct Wiring {
    shade: Option<(Arc<dyn ShadeSource>, f32)>,
    map: Option<Arc<dyn SurfaceMap>>,
}

impl Wiring {
    /// The dab map the stroke follows (planes and/or depth), if any.
    #[cfg(test)]
    pub(crate) fn map(&self) -> Option<&Arc<dyn SurfaceMap>> {
        self.map.as_ref()
    }

    /// Does the stroke use any surface effect?
    pub(crate) fn is_active(&self) -> bool {
        self.shade.is_some() || self.map.is_some()
    }

    /// `r` with the stroke's shading and plane map attached (before its first point).
    pub(crate) fn apply(&self, r: StrokeRenderer) -> StrokeRenderer {
        r.with_shade(self.shade.as_ref().map(|(s, _)| s.clone()), self.shade.as_ref().map_or(0.0, |(_, k)| *k)).with_surface(self.map.clone())
    }

    /// The wiring `brush` asks for on the active document, for a `paint.stroke` / `paint.pencil`
    /// aimed at `layer`. The Eraser has none. Shading is a colour effect, so a mask, channel or
    /// Quick Mask target skips it; perspective applies there too. Errors: Follow Perspective with
    /// no saved plane for this canvas, or a shading sample that cannot be built.
    pub(crate) fn build(s: &Session, cmd: &str, p: &Value, brush: &BrushSettings, layer: Option<LayerId>) -> Result<Self> {
        let opts = &brush.surface;
        if brush.erase || !(opts.color || opts.perspective || opts.depth) {
            return Ok(Self::default());
        }
        opts.validate().map_err(|e| bad(cmd, e))?;
        let mut w = Self::default();
        let persp = if opts.perspective { Some(plane_map(s).ok_or_else(|| bad(cmd, NO_PLANES))?) } else { None };
        // Depth is geometry, so it follows a mask or channel target too. A missing channel is
        // an error here (the stroke builds; the cursor only reads).
        let depth = if opts.depth {
            let geometry = depth_geometry(s, cmd, &opts.depth_channel)?;
            Some(DepthMap::new(geometry, (brush.size / 2.0).max(0.5), opts.relief, opts.size_by_depth))
        } else {
            None
        };
        w.map = make_map(persp, depth);
        if opts.color && !is_gray_target(s, p)? {
            let field = shading_field(s, cmd, opts.sample, opts.detail, layer)?;
            w.shade = Some((Arc::new(FieldShade(field)), opts.strength));
        }
        Ok(w)
    }
}

/// The depth geometry of the active document's alpha channel `name` (the first of that name):
/// the cached one while the channel's pixels are unchanged, else built (and kept).
fn depth_geometry(s: &Session, cmd: &str, name: &str) -> Result<Arc<DepthGeometry>> {
    if let Some(g) = cached_depth(s, name) {
        return Ok(g);
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let channel = st.doc.channels.iter().find(|c| c.name == name).ok_or_else(|| bad(cmd, format!("no {name} channel: run Image ▸ Estimate Depth")))?;
    let canvas = st.doc.bounds();
    check_size(canvas, cmd)?;
    // The estimator's full-precision plane while the channel is unchanged since the estimate (it
    // keeps narrow folds the quantised channel loses); else the channel itself.
    let geometry = if let Some((plane, pw, ph)) = precise_plane(s, st.doc.id, name, &channel.surface) {
        DepthGeometry::from_plane(canvas, &plane, pw as usize, ph as usize).map_err(|e| bad(cmd, e))?
    } else {
        // The channel is grayscale at the document depth; its first channel is the depth.
        let n = channel.surface.channels().max(1);
        let data = crate::allocation::read_region(&channel.surface, canvas, "the depth channel")?;
        let depth: Vec<f32> = if n == 1 { data } else { data.chunks_exact(n).map(|c| c.first().copied().unwrap_or(0.0)).collect() };
        DepthGeometry::new(canvas, &depth).map_err(|e| bad(cmd, e))?
    };
    let geometry = Arc::new(geometry);
    s.surface_cache.depth_builds.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    *lock(&s.surface_cache.depth) =
        Some(DepthEntry { doc: st.doc.id, name: name.to_string(), surface: channel.surface.clone(), area: canvas, geometry: geometry.clone() });
    Ok(geometry)
}

/// The full-precision depth plane of channel `name` (row-major `(w, h)` over the document bounds,
/// possibly at reduced resolution), when the channel is unchanged since the estimate.
fn precise_plane(s: &Session, doc: DocId, name: &str, surface: &photocraft_raster::Surface) -> Option<(Arc<Vec<f32>>, u32, u32)> {
    crate::depth_cmds::precise_depth_plane(s, doc, name, surface)
}

/// The cached depth geometry for channel `name`, if the channel's pixels are the ones it was
/// built from. Never builds: what the brush cursor reads.
pub(crate) fn cached_depth(s: &Session, name: &str) -> Option<Arc<DepthGeometry>> {
    let st = s.active()?;
    let channel = st.doc.channels.iter().find(|c| c.name == name)?;
    let cache = lock(&s.surface_cache.depth);
    let e = cache.as_ref()?;
    (e.doc == st.doc.id && e.name == name && e.area == st.doc.bounds() && e.surface == channel.surface).then(|| e.geometry.clone())
}

/// A mask, an alpha channel, Quick Mask or a single colour channel: no colour to shade.
fn is_gray_target(s: &Session, p: &Value) -> Result<bool> {
    Ok(is_mask_target(p) || crate::channel_cmds::is_channel_target(p) || crate::color_channels::target(s, p)?.is_some())
}

fn field_key(doc: &photocraft_doc::Document, sample: SampleMode, detail: f32, layer: Option<LayerId>) -> FieldKey {
    FieldKey { doc: doc.id, sample, detail: detail.to_bits(), layer }
}

/// The cached shading field for the active document as it is now, if there is one. Never builds:
/// what the brush cursor reads.
pub(crate) fn cached_shading_field(s: &Session, sample: SampleMode, detail: f32, layer: Option<LayerId>) -> Option<Arc<ShadingField>> {
    let st = s.active()?;
    let key = field_key(&st.doc, sample, detail, layer);
    lock(&s.surface_cache.fields).iter().find(|e| e.key == key && same_sources(&e.doc, &st.doc, sample, layer)).map(|e| e.field.clone())
}

/// The shading field of the active document as it is now: the cached one, else built (and kept).
fn shading_field(s: &Session, cmd: &str, sample: SampleMode, detail: f32, layer: Option<LayerId>) -> Result<Arc<ShadingField>> {
    if let Some(f) = cached_shading_field(s, sample, detail, layer) {
        return Ok(f);
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let key = field_key(&st.doc, sample, detail, layer);
    let field = Arc::new(build_field(&st.doc, cmd, sample, detail, layer)?);
    s.surface_cache.builds.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut cache = lock(&s.surface_cache.fields);
    cache.retain(|e| e.key != key);
    if cache.len() >= KEEP_FIELDS {
        cache.remove(0);
    }
    cache.push(FieldEntry { key, doc: st.doc.clone(), field: field.clone() });
    Ok(field)
}

/// A shading field of `doc` over its canvas: the visible composite, or the layer's own pixels.
///
/// All Layers hides `layer` (the one being painted) first, so the paint of earlier strokes never
/// feeds into the shading of later ones. Hiding, not removing, keeps the other layers' order; a
/// layer clipped to the hidden one disappears with it, which is what a hidden clipping base does
/// in the composite. The sample is rendered by `photocraft_compose::render_bands` (tiles in
/// parallel) and its luma accumulated per band, so only the 16-bit luma of the whole canvas is
/// ever held, not its float pixels.
pub(crate) fn build_field(doc: &photocraft_doc::Document, cmd: &str, sample: SampleMode, detail: f32, layer: Option<LayerId>) -> Result<ShadingField> {
    let canvas = doc.bounds();
    check_size(canvas, cmd)?;
    match sample {
        SampleMode::AllLayers => {
            // Cheap: the layers' surfaces are copy-on-write.
            let mut hidden;
            let doc = match layer.filter(|id| doc.layer(*id).is_some_and(|l| l.visible)) {
                Some(id) => {
                    hidden = doc.clone();
                    if let Some(l) = hidden.layer_mut(id) {
                        l.visible = false;
                    }
                    &hidden
                }
                None => doc,
            };
            let mut b = ShadingBuilder::new(ColorMode::Rgb, true, canvas, detail).map_err(|e| bad(cmd, e))?;
            let rows = i32::try_from(b.band_rows()).unwrap_or(i32::MAX);
            photocraft_compose::render_bands(doc, canvas, rows, |band| {
                if band.px.len() as u64 != u64::from(band.rect.width()) * u64::from(band.rect.height()) {
                    return Err(bad(cmd, "the document could not be rendered for Surface Color"));
                }
                b.add_band_rgba(band.rect, &band.px).map_err(|e| bad(cmd, e))
            })?;
            Ok(b.finish())
        }
        SampleMode::CurrentLayer => {
            let id = layer.ok_or_else(|| bad(cmd, "Surface Color needs a target layer"))?;
            let surf = doc.layer(id).and_then(photocraft_doc::Layer::surface).ok_or(EngineError::NoLayer(id))?;
            field_from_surface(surf, canvas, cmd, detail)
        }
    }
}

/// A shading field of `surf` over `canvas`, read in bands (see [`build_field`]).
pub(crate) fn field_from_surface(surf: &photocraft_raster::Surface, canvas: Rect, cmd: &str, detail: f32) -> Result<ShadingField> {
    check_size(canvas, cmd)?;
    let f = surf.format();
    let b = ShadingBuilder::new(f.mode, f.alpha, canvas, detail).map_err(|e| bad(cmd, e))?;
    for_bands(b, cmd, |band| Ok(Image::read(surf, band)))
}

fn check_size(canvas: Rect, cmd: &str) -> Result<()> {
    if u64::from(canvas.width()) * u64::from(canvas.height()) > MAX_SAMPLE_PIXELS {
        return Err(bad(cmd, format!("Surface Color needs a canvas of at most {MAX_SAMPLE_PIXELS} pixels")));
    }
    Ok(())
}

/// Feeds `builder` its bands (`read` gives the image of one band) and finishes it.
fn for_bands(mut builder: ShadingBuilder, cmd: &str, mut read: impl FnMut(Rect) -> Result<Image>) -> Result<ShadingField> {
    let area = builder.area();
    let rows = i32::try_from(builder.band_rows()).unwrap_or(i32::MAX);
    let mut y = area.y0;
    while y < area.y1 {
        let end = y.saturating_add(rows).min(area.y1);
        let band = Rect::new(area.x0, y, area.x1, end);
        let img = read(band)?;
        builder.add_band(&img, band).map_err(|e| bad(cmd, e))?;
        y = end;
    }
    Ok(builder.finish())
}

/// The saved perspective planes of the active document as a surface map, if there are any for
/// this canvas size. Cached while the planes and canvas stay the same.
pub(crate) fn plane_map(s: &Session) -> Option<Arc<PlaneMap>> {
    let st = s.active()?;
    let (w, h) = (st.doc.size.width, st.doc.size.height);
    let pp = &st.doc.perspective;
    if !pp.valid_for(w, h) {
        return None;
    }
    let key = MapKey { doc: st.doc.id, planes: pp.clone(), size: [w, h] };
    if let Some((k, m)) = lock(&s.surface_cache.map).as_ref()
        && *k == key
    {
        return Some(m.clone());
    }
    let scene = crate::vp_cmds::scene_from_planes(&pp.planes, pp.focal, st.doc.bounds())?;
    let map = Arc::new(PlaneMap(PlaneSurface::new(scene)));
    *lock(&s.surface_cache.map) = Some((key, map.clone()));
    Some(map)
}

/// What the brush cursor draws: the dab a stroke would lay down at the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursorShape {
    pub radius: f32,
    /// Radians, counter-clockwise, y up (as `Dab::proj_angle`).
    pub proj_angle: f32,
    /// 0.05..1; 1 is a round outline.
    pub proj_scale: f32,
    /// Where the outline's centre sits from the pointer, in image pixels (y down); always 0 for
    /// planes and depth (kept for the cursor drawing).
    pub offset: [f32; 2],
    /// The outline of a round computed tip bent over the depth (see [`Footprint`]): 32 distances
    /// in image pixels at angles `k · 2π / 32` (counter-clockwise, y up). `None`: the ellipse.
    pub outline: Option<[f32; 32]>,
}

/// What the brush cursor draws at document point `(x, y)` with a tip of radius `base_radius` while
/// Follow Perspective and/or Follow Depth is on: the dab a stroke lays down at that point, wherever
/// it started (sized against the map's reference scale, see [`surface_dab_shape`]), with the
/// plane's foreshortening and the depth slope's. `None` when neither applies: the options are off,
/// the document has no planes for this canvas, the point is off every plane (with Follow
/// Perspective on), or the depth is flat and unscaled there. The cursor then stays a plain round
/// outline. Follow Depth builds the depth geometry on a cache miss (one read of the channel, kept until
/// its content changes); without a usable channel the outline stays plain. The shading field is
/// never built here.
pub fn brush_cursor_shape(s: &mut Session, x: f64, y: f64, base_radius: f32) -> Option<CursorShape> {
    let opts = s.tools.brush.surface.clone();
    if !(opts.perspective || opts.depth) {
        return None;
    }
    let persp = if opts.perspective { Some(plane_map(s)?) } else { None };
    let depth = if opts.depth {
        // A missing or unusable channel leaves the outline plain; the first hover builds the grid.
        let geometry = depth_geometry(s, CURSOR_SHAPE, &opts.depth_channel).ok()?;
        Some(DepthMap::new(geometry, base_radius, opts.relief, opts.size_by_depth))
    } else {
        None
    };
    let map = make_map(persp, depth)?;
    let local = map.local(Point::new(x, y))?;
    let anchor = map.reference_scale().unwrap_or(local.scale);
    let (radius, proj_angle, proj_scale) = surface_dab_shape(base_radius, Some(anchor), &local);
    let outline = map.footprint(Point::new(x, y), radius, &local).filter(Footprint::is_valid).map(|f| f.rays);
    Some(CursorShape { radius, proj_angle, proj_scale, offset: local.offset, outline })
}

fn cursor_shape(s: &mut Session, p: &Value) -> Result<Value> {
    s.active().ok_or(EngineError::NoDocument)?;
    let coord = |k: &str| {
        p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad(CURSOR_SHAPE, format!("`{k}` must be a finite number (document pixels)")))
    };
    let (x, y) = (coord("x")?, coord("y")?);
    let base = (s.tools.brush.size / 2.0).max(0.5);
    Ok(match brush_cursor_shape(s, x, y, base) {
        Some(c) => json!({"onPlane": true, "radius": c.radius, "projAngle": c.proj_angle, "projScale": c.proj_scale, "offset": c.offset, "outline": c.outline}),
        None => json!({"onPlane": false}),
    })
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CURSOR_SHAPE,
        label: "Brush Cursor Shape",
        menu: &[],
        shortcut: None,
        params: r##"{"x":px,"y":px} → {"onPlane":true,"radius":px,"projAngle":radians (counter-clockwise, y up),"projScale":0.05..1,"offset":[dx,dy] px (y down),"outline":32 distances px at angles k·2π/32 (counter-clockwise, y up) | null} | {"onPlane":false}: the brush cursor's outline at that point while the brush's Surface › Follow Perspective or Follow Depth is on (the dab a stroke lays down at that point)"##,
        enabled: |s| s.active().map(|_| ()).ok_or_else(|| "no document open".into()),
        run: cursor_shape,
        journal: false,
    }]
}
