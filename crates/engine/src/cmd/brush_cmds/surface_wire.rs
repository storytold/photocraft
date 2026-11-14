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
        self.0.local([p.x, p.y]).map(|(scale, squash_angle, squash)| Local { scale, squash_angle, squash })
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
        if brush.erase || !(opts.color || opts.perspective) {
            return Ok(Self::default());
        }
        opts.validate().map_err(|e| bad(cmd, e))?;
        let mut w = Self::default();
        if opts.perspective {
            let map = plane_map(s).ok_or_else(|| bad(cmd, NO_PLANES))?;
            w.map = Some(map);
        }
        if opts.color && !is_gray_target(s, p)? {
            let field = shading_field(s, cmd, opts.sample, opts.detail, layer)?;
            w.shade = Some((Arc::new(FieldShade(field)), opts.strength));
        }
        Ok(w)
    }
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

/// What the brush cursor draws at document point `(x, y)` with a tip of radius `base_radius`
/// while Follow Perspective is on: `(radius, proj_angle, proj_scale)` of the dab a stroke
/// starting there would lay down first (it anchors the stroke's size, so it keeps `base_radius`;
/// see [`surface_dab_shape`]), with the plane's foreshortening. `None` when Follow Perspective is
/// off, the document has no planes for this canvas, or the point is off every plane: the cursor
/// then stays a plain round outline.
pub fn brush_cursor_shape(s: &mut Session, x: f64, y: f64, base_radius: f32) -> Option<(f32, f32, f32)> {
    if !s.tools.brush.surface.perspective {
        return None;
    }
    let map = plane_map(s)?;
    let local = map.local(Point::new(x, y))?;
    Some(surface_dab_shape(base_radius, Some(local.scale), &local))
}

fn cursor_shape(s: &mut Session, p: &Value) -> Result<Value> {
    s.active().ok_or(EngineError::NoDocument)?;
    let coord = |k: &str| {
        p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad(CURSOR_SHAPE, format!("`{k}` must be a finite number (document pixels)")))
    };
    let (x, y) = (coord("x")?, coord("y")?);
    let base = (s.tools.brush.size / 2.0).max(0.5);
    Ok(match brush_cursor_shape(s, x, y, base) {
        Some((radius, proj_angle, proj_scale)) => json!({"onPlane": true, "radius": radius, "projAngle": proj_angle, "projScale": proj_scale}),
        None => json!({"onPlane": false}),
    })
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CURSOR_SHAPE,
        label: "Brush Cursor Shape",
        menu: &[],
        shortcut: None,
        params: r##"{"x":px,"y":px} → {"onPlane":true,"radius":px,"projAngle":radians (counter-clockwise, y up),"projScale":0.05..1} | {"onPlane":false}: the brush cursor's outline at that point while the brush's Surface › Follow Perspective is on (the first dab of a stroke starting there)"##,
        enabled: |s| s.active().map(|_| ()).ok_or_else(|| "no document open".into()),
        run: cursor_shape,
        journal: false,
    }]
}
