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
use photocraft_algo::surface::{PlaneSurface, ShadingBuilder, ShadingField};
use photocraft_color::ColorMode;
use photocraft_doc::{DocId, LayerId};
use photocraft_geom::{Point, Rect};
use photocraft_paint::surface::{Local, SampleMode, ShadeSource, SurfaceMap, surface_dab_shape};
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
}

/// Follow Folds: the photo's shading as a rough height map `H = S - 1`. A dab over a slope is
/// foreshortened along the slope (a tilted surface seen from the front) and a dab over a high
/// point moves up (seen from slightly above).
///
/// With `h = max(base_radius / 2, 1.5)` px, the slope is the gradient of S by central
/// differences over `±h`, each side averaged over three samples across (12 reads a dab). The
/// tilt is `g = depth · 4h · ∇H` (K = 4h: a shading change of 1 over 2h px at full depth is
/// |g| = 2), the squash `1/√(1+|g|²)` (never below 0.2) along the gradient's direction, and the
/// bend shifts the dab up by `bend · 2 · base_radius · H`, at most `2 · base_radius`.
pub(crate) struct FoldMap {
    field: Arc<ShadingField>,
    base_radius: f32,
    depth: f32,
    bend: f32,
}

impl FoldMap {
    pub(crate) fn new(field: Arc<ShadingField>, base_radius: f32, depth: f32, bend: f32) -> Self {
        Self { field, base_radius: if base_radius.is_finite() { base_radius.max(0.5) } else { 0.5 }, depth, bend }
    }

    /// The fold's projection at `p`, `None` where it is flat (or off the canvas).
    fn local(&self, p: Point) -> Option<Local> {
        if !(p.x.is_finite() && p.y.is_finite()) {
            return None;
        }
        let r = f64::from(self.base_radius);
        let h = (r * 0.5).max(1.5);
        let s = |dx: f64, dy: f64| f64::from(self.field.shade_at((p.x + dx).floor() as i32, (p.y + dy).floor() as i32));
        let (mut gx, mut gy) = (0.0, 0.0);
        for j in [-0.5, 0.0, 0.5] {
            gx += s(h, j * h) - s(-h, j * h);
            gy += s(j * h, h) - s(j * h, -h);
        }
        // Shading change per pixel, then in tilt units.
        let k = f64::from(self.depth) * 4.0 * h / (3.0 * 2.0 * h);
        let (ux, uy) = (gx * k, gy * k);
        let mag = ux.hypot(uy);
        let squash = (1.0 / (1.0 + mag * mag).sqrt()).max(0.2);
        // The gradient's direction in the y-up convention (image y points down).
        let angle = if mag > 1e-9 { (-uy).atan2(ux) } else { 0.0 };
        let lim = 2.0 * r;
        let off_y = (-f64::from(self.bend) * lim * (s(0.0, 0.0) - 1.0)).clamp(-lim, lim);
        if squash >= 0.9999 && off_y == 0.0 {
            return None;
        }
        let local = Local { scale: 1.0, squash_angle: angle as f32, squash: squash as f32, offset: [0.0, off_y as f32] };
        local.is_valid().then_some(local)
    }
}

/// `scale · (I − (1 − squash) · u uᵀ)` with `u` the direction of `squash_angle`: the matrix that
/// maps a circle to the dab's ellipse, in y-up axes.
fn local_matrix(l: &Local) -> [[f64; 2]; 2] {
    let (sn, cs) = f64::from(l.squash_angle).sin_cos();
    let (k, sc) = (1.0 - f64::from(l.squash), f64::from(l.scale));
    [[sc * (1.0 - k * cs * cs), -sc * k * cs * sn], [-sc * k * cs * sn, sc * (1.0 - k * sn * sn)]]
}

/// Perspective `a` followed by fold `b`: their matrices multiplied and decomposed again; the
/// offsets add (the fold's is not scaled by the plane: the plane's scale has arbitrary units).
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

/// Follow Perspective and Follow Folds together. With a plane map, a dab off every plane is
/// left alone (fold or not): the stroke's size is anchored on the plane's scale.
struct CombinedMap {
    persp: Option<Arc<PlaneMap>>,
    fold: Option<FoldMap>,
}

impl SurfaceMap for CombinedMap {
    fn local(&self, p: Point) -> Option<Local> {
        let plane = match &self.persp {
            Some(m) => Some(m.local(p)?),
            None => None,
        };
        let fold = self.fold.as_ref().and_then(|f| f.local(p));
        match (plane, fold) {
            (Some(a), Some(b)) => combine(&a, &b).or(Some(a)),
            (a, b) => a.or(b),
        }
    }
}

/// The dab map for the planes and/or folds a stroke follows.
pub(crate) fn make_map(persp: Option<Arc<PlaneMap>>, fold: Option<FoldMap>) -> Option<Arc<dyn SurfaceMap>> {
    match (persp, fold) {
        (None, None) => None,
        (Some(p), None) => Some(p),
        (persp, fold) => Some(Arc::new(CombinedMap { persp, fold })),
    }
}

#[derive(Clone, PartialEq)]
struct FieldKey {
    doc: DocId,
    revision: u64,
    sample: SampleMode,
    detail: u32,
    layer: Option<LayerId>,
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
    fields: Mutex<Vec<(FieldKey, Arc<ShadingField>)>>,
    map: Mutex<Option<(MapKey, Arc<PlaneMap>)>>,
}

/// Shading and perspective for one stroke, ready to attach to its renderers.
#[derive(Default)]
pub(crate) struct Wiring {
    shade: Option<(Arc<dyn ShadeSource>, f32)>,
    map: Option<Arc<dyn SurfaceMap>>,
}

impl Wiring {
    /// The dab map the stroke follows (planes and/or folds), if any.
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
        if brush.erase || !(opts.color || opts.perspective || opts.folds) {
            return Ok(Self::default());
        }
        opts.validate().map_err(|e| bad(cmd, e))?;
        let mut w = Self::default();
        let persp = if opts.perspective { Some(plane_map(s).ok_or_else(|| bad(cmd, NO_PLANES))?) } else { None };
        // Folds are geometry, so they follow a mask or channel target too; the shading then
        // comes from the whole picture when there is no layer to read.
        let fold = if opts.folds {
            let field = fold_field(s, cmd, opts, layer)?;
            Some(FoldMap::new(field, (brush.size / 2.0).max(0.5), opts.fold_depth, opts.fold_bend))
        } else {
            None
        };
        w.map = make_map(persp, fold);
        if opts.color && !is_gray_target(s, p)? {
            let field = shading_field(s, cmd, opts.sample, opts.detail, layer)?;
            w.shade = Some((Arc::new(FieldShade(field)), opts.strength));
        }
        Ok(w)
    }
}

/// The shading field Follow Folds reads (the same cached field Surface Color uses).
fn fold_field(s: &Session, cmd: &str, opts: &photocraft_paint::SurfaceOptions, layer: Option<LayerId>) -> Result<Arc<ShadingField>> {
    let sample = if layer.is_none() { SampleMode::AllLayers } else { opts.sample };
    shading_field(s, cmd, sample, opts.detail, layer)
}

/// A mask, an alpha channel, Quick Mask or a single colour channel: no colour to shade.
fn is_gray_target(s: &Session, p: &Value) -> Result<bool> {
    Ok(is_mask_target(p) || crate::channel_cmds::is_channel_target(p) || crate::color_channels::target(s, p)?.is_some())
}

/// The shading field of the active document as it is now, cached by revision.
fn shading_field(s: &Session, cmd: &str, sample: SampleMode, detail: f32, layer: Option<LayerId>) -> Result<Arc<ShadingField>> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let key = FieldKey {
        doc: st.doc.id,
        revision: st.revision,
        sample,
        detail: detail.to_bits(),
        layer: if sample == SampleMode::CurrentLayer { layer } else { None },
    };
    if let Some((_, f)) = lock(&s.surface_cache.fields).iter().find(|(k, _)| *k == key) {
        return Ok(f.clone());
    }
    let field = Arc::new(build_field(&st.doc, cmd, sample, detail, layer)?);
    let mut cache = lock(&s.surface_cache.fields);
    cache.retain(|(k, _)| *k != key);
    if cache.len() >= KEEP_FIELDS {
        cache.remove(0);
    }
    cache.push((key, field.clone()));
    Ok(field)
}

/// A shading field of `doc` over its canvas: the visible composite, or the layer's own pixels.
/// The sample is read a band of rows at a time, so only the 16-bit luma of the whole canvas is
/// ever held, not its float pixels.
pub(crate) fn build_field(doc: &photocraft_doc::Document, cmd: &str, sample: SampleMode, detail: f32, layer: Option<LayerId>) -> Result<ShadingField> {
    let canvas = doc.bounds();
    check_size(canvas, cmd)?;
    match sample {
        SampleMode::AllLayers => {
            let b = ShadingBuilder::new(ColorMode::Rgb, true, canvas, detail).map_err(|e| bad(cmd, e))?;
            for_bands(b, cmd, |band| {
                let buf = photocraft_compose::render(doc, band);
                if buf.px.len() as u64 != u64::from(band.width()) * u64::from(band.height()) {
                    return Err(bad(cmd, "the document could not be rendered for Surface Color"));
                }
                let mut data = crate::allocation::capacity::<f32>(buf.px.len().saturating_mul(4), "the Surface Color sample")?;
                for px in &buf.px {
                    data.extend_from_slice(px);
                }
                Ok(Image { rect: band, ch: 4, data })
            })
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

/// What the brush cursor draws: the dab a stroke starting at the pointer would lay down first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursorShape {
    pub radius: f32,
    /// Radians, counter-clockwise, y up (as `Dab::proj_angle`).
    pub proj_angle: f32,
    /// 0.05..1; 1 is a round outline.
    pub proj_scale: f32,
    /// Where the outline's centre sits from the pointer, in image pixels (y down): Follow Folds'
    /// bend.
    pub offset: [f32; 2],
}

/// What the brush cursor draws at document point `(x, y)` with a tip of radius `base_radius` while
/// Follow Perspective and/or Follow Folds is on: the dab a stroke starting there lays down first
/// (it anchors the stroke's size, so it keeps `base_radius`; see [`surface_dab_shape`]), with the
/// plane's foreshortening, the fold's slope and the fold's bend. `None` when neither applies: the
/// options are off, the document has no planes for this canvas, the point is off every plane (with
/// Follow Perspective on), or the surface is flat there. The cursor then stays a plain round
/// outline. Follow Folds reads the shading field, which is built (and cached) on first use.
pub fn brush_cursor_shape(s: &mut Session, x: f64, y: f64, base_radius: f32) -> Option<CursorShape> {
    let opts = s.tools.brush.surface.clone();
    if !(opts.perspective || opts.folds) {
        return None;
    }
    let persp = if opts.perspective { Some(plane_map(s)?) } else { None };
    let fold = if opts.folds {
        let layer = s.active()?.active_layer;
        let field = fold_field(s, CURSOR_SHAPE, &opts, layer).ok()?;
        Some(FoldMap::new(field, base_radius, opts.fold_depth, opts.fold_bend))
    } else {
        None
    };
    let local = make_map(persp, fold)?.local(Point::new(x, y))?;
    let (radius, proj_angle, proj_scale) = surface_dab_shape(base_radius, Some(local.scale), &local);
    Some(CursorShape { radius, proj_angle, proj_scale, offset: local.offset })
}

fn cursor_shape(s: &mut Session, p: &Value) -> Result<Value> {
    s.active().ok_or(EngineError::NoDocument)?;
    let coord = |k: &str| {
        p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad(CURSOR_SHAPE, format!("`{k}` must be a finite number (document pixels)")))
    };
    let (x, y) = (coord("x")?, coord("y")?);
    let base = (s.tools.brush.size / 2.0).max(0.5);
    Ok(match brush_cursor_shape(s, x, y, base) {
        Some(c) => json!({"onPlane": true, "radius": c.radius, "projAngle": c.proj_angle, "projScale": c.proj_scale, "offset": c.offset}),
        None => json!({"onPlane": false}),
    })
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CURSOR_SHAPE,
        label: "Brush Cursor Shape",
        menu: &[],
        shortcut: None,
        params: r##"{"x":px,"y":px} → {"onPlane":true,"radius":px,"projAngle":radians (counter-clockwise, y up),"projScale":0.05..1,"offset":[dx,dy] px (y down)} | {"onPlane":false}: the brush cursor's outline at that point while the brush's Surface › Follow Perspective or Follow Folds is on (the first dab of a stroke starting there)"##,
        enabled: |s| s.active().map(|_| ()).ok_or_else(|| "no document open".into()),
        run: cursor_shape,
        journal: false,
    }]
}
