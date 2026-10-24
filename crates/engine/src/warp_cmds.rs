//! Edit › Transform › Warp, Layer › Smart Objects › Warp and split warps.
//!
//! A warp is a [`Warp`] (preset style + bend/distortions, or a custom bicubic Bezier mesh)
//! defined over a source box in document space. Pixel layers are resampled once
//! (`photocraft_algo::warp`); smart objects store the warp in their source space and re-render
//! from the source, so editing a smart object's warp never degrades it.
//!
//! The split and grid commands edit a mesh: either one passed in (`"warp"`, returned edited —
//! what the Free Transform warp UI uses) or the active smart object's stored warp (a history step).

use photocraft_algo::transform::{Homography, Interp};
use photocraft_algo::warp::{warp_mesh_gray, warp_mesh_surface_with};
use photocraft_doc::{Document, Layer, LayerContent, LayerId, Locks, Rect};
use photocraft_geom::warp::{BezierMesh, Warp, WarpStyle};
use photocraft_raster::{Interrupt, Surface};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    s.active().and_then(|d| d.active_layer).map(|_| ()).ok_or_else(|| "no active layer".into())
}

fn active_smart(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document")?;
    match d.active_layer.and_then(|id| d.doc.layer(id)).map(|l| &l.content) {
        Some(LayerContent::Smart(_)) => Ok(()),
        _ => Err("the active layer is not a smart object".into()),
    }
}

fn rect_f(r: Rect) -> [f64; 4] {
    [f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1)]
}

/// `p` through a projective map: a smart object's placement (affine, or Distort / Perspective).
fn hom_apply(h: &Homography, p: [f64; 2]) -> [f64; 2] {
    let (x, y) = h.apply(p[0], p[1]);
    [x, y]
}

/// The box a warp of `layer` starts from (Free Transform's frame).
/// For a warped smart object that is its (unwarped) source box, so re-warping replaces the warp
/// over the same frame.
pub fn warp_bounds(doc: &Document, layer: &Layer) -> Rect {
    if let LayerContent::Smart(sm) = &layer.content
        && let Some(w) = &sm.warp
    {
        let t = crate::smart_cmds::placement(sm);
        let c = [[w.bounds[0], w.bounds[1]], [w.bounds[2], w.bounds[1]], [w.bounds[2], w.bounds[3]], [w.bounds[0], w.bounds[3]]].map(|p| hom_apply(&t, p));
        let b = c.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
        return Rect::new(b[0].round() as i32, b[1].round() as i32, b[2].round() as i32, b[3].round() as i32);
    }
    crate::transform_cmds::transform_bounds(doc, layer)
}

/// A smart object's stored warp expressed in document space (for editing), if it has one.
pub fn smart_warp_doc_space(layer: &Layer) -> Option<Warp> {
    let LayerContent::Smart(sm) = &layer.content else { return None };
    let w = sm.warp.as_ref()?;
    let t = crate::smart_cmds::placement(sm);
    let mesh = w.to_mesh(1, 1).map_points(|p| hom_apply(&t, p));
    let corners = [[w.bounds[0], w.bounds[1]], [w.bounds[2], w.bounds[1]], [w.bounds[2], w.bounds[3]], [w.bounds[0], w.bounds[3]]].map(|p| hom_apply(&t, p));
    let b = corners.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
    Some(Warp::custom(mesh, b))
}

/// Parses the warp params: a full `"warp"` object, or `"style"` (+ `"bend"`, `"hDistort"`,
/// `"vDistort"`, `"vertical"`; `"values"` for a cylinder), `"mesh"` and `"grid"` over `rect`.
pub fn warp_from_params(cmd: &str, p: &Value, rect: [f64; 4]) -> Result<Warp> {
    if let Some(w) = p.get("warp") {
        let mut w: Warp = serde_json::from_value(w.clone()).map_err(|e| bad(cmd, format!("bad `warp`: {e}")))?;
        // A cylinder is its values: the mesh is rebuilt from them, whatever was passed.
        if w.style == WarpStyle::Cylinder {
            if w.vertical || w.h_distort != 0.0 || w.v_distort != 0.0 {
                return Err(bad(cmd, CYLINDER_SHAPE));
            }
            w = Warp::cylinder(&w.values, w.bounds).ok_or_else(|| bad(cmd, CYLINDER_VALUES))?;
        }
        if w.mesh.as_ref().is_some_and(|m| !m.is_valid()) {
            return Err(bad(cmd, "malformed warp mesh"));
        }
        return Ok(w);
    }
    let num = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).unwrap_or(d);
    let style = match p.get("style").and_then(Value::as_str) {
        Some(st) => WarpStyle::parse(st).ok_or_else(|| bad(cmd, format!("unknown warp style \"{st}\"")))?,
        None if p.get("mesh").is_some() || p.get("grid").is_some() => WarpStyle::Custom,
        None => return Err(bad(cmd, "pass `style` (arc, flag, …, custom), `mesh` or `warp`")),
    };
    let mut w = Warp::preset(style, num("bend", 50.0), rect);
    w.h_distort = num("hDistort", 0.0);
    w.v_distort = num("vDistort", 0.0);
    w.vertical = p.get("vertical").and_then(Value::as_bool).unwrap_or(false);
    if style == WarpStyle::Cylinder {
        if w.vertical || w.h_distort != 0.0 || w.v_distort != 0.0 {
            return Err(bad(cmd, CYLINDER_SHAPE));
        }
        let values: Option<Vec<f64>> = p.get("values").and_then(Value::as_array).and_then(|a| a.iter().map(Value::as_f64).collect());
        return values.and_then(|v| Warp::cylinder(&v, rect)).ok_or_else(|| bad(cmd, CYLINDER_VALUES));
    }
    if style == WarpStyle::Custom {
        let mesh = match p.get("mesh") {
            Some(m) => {
                let m: BezierMesh = serde_json::from_value(m.clone()).map_err(|e| bad(cmd, format!("bad `mesh`: {e}")))?;
                if !m.is_valid() {
                    return Err(bad(cmd, "malformed mesh (need (3·cols+1)×(3·rows+1) points and knots 0..1)"));
                }
                m
            }
            None => {
                let g = p.get("grid").and_then(Value::as_array);
                let at = |i: usize| g.and_then(|a| a.get(i)).and_then(Value::as_u64).unwrap_or(1) as usize;
                BezierMesh::identity(rect, at(0), at(1))
            }
        };
        w.mesh = Some(mesh);
    }
    Ok(w)
}

const CYLINDER_VALUES: &str =
    "a cylinder needs `values`: seven numbers [left, bottom, right, top, top depth, bottom depth, wrap 0..1], the right silhouette right of the left one";
const CYLINDER_SHAPE: &str = "a cylinder is horizontal, with no hDistort or vDistort";

/// A document-space cylinder `w` stored on a smart object, whose placement's inverse is `inv` and
/// whose warp box `w.bounds` lands on `sb` in source space. A placement that only scales and
/// moves keeps it a cylinder (written to PSD as one): the silhouettes and edges map, and the
/// ellipses' depth, in units of the half axis, follows the change of aspect. Otherwise (rotated,
/// skewed, flipped, in perspective) it becomes its mesh, a custom warp.
fn cylinder_in_source(w: &Warp, inv: &Homography, sb: [f64; 4]) -> Option<Warp> {
    let m = inv.0;
    let axis_aligned = m[8] != 0.0 && m[1] == 0.0 && m[3] == 0.0 && m[6] == 0.0 && m[7] == 0.0;
    let (sx, sy) = (m[0] / m[8], m[4] / m[8]);
    if axis_aligned && sx > 0.0 && sy > 0.0 && w.values.len() == 7 {
        let v = &w.values;
        let (x, y) = (|v: f64| sx * v + m[2] / m[8], |v: f64| sy * v + m[5] / m[8]);
        let k = sy / sx;
        if let Some(c) = Warp::cylinder(&[x(v[0]), y(v[1]), x(v[2]), y(v[3]), v[4] * k, v[5] * k, v[6]], sb) {
            return Some(c);
        }
    }
    let mesh = w.to_mesh(1, 1).map_points(|p| hom_apply(inv, p));
    Some(Warp::custom(mesh, sb))
}

/// Whether the custom warp `w` is exactly what Warp mode shows for `layer`'s stored cylinder
/// ([`smart_warp_doc_space`]: its mesh over the same box), so committing it untouched changes
/// nothing. Stored as a custom warp instead, the cylinder would be lost, as a PSD keeps it only
/// as `warpCylinder` (#4112).
fn is_stored_cylinder(layer: &Layer, w: &Warp) -> bool {
    let LayerContent::Smart(sm) = &layer.content else { return false };
    if w.style != WarpStyle::Custom || sm.warp.as_ref().is_none_or(|s| s.style != WarpStyle::Cylinder) {
        return false;
    }
    let (Some(shown), Some(m)) = (smart_warp_doc_space(layer), w.mesh.as_ref()) else { return false };
    let Some(o) = shown.mesh.as_ref() else { return false };
    let tol = 1e-6 * (1.0 + o.points.iter().flatten().chain(&shown.bounds).fold(0.0f64, |a, v| a.max(v.abs())));
    let near = |a: &[f64], b: &[f64]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol);
    o.us == m.us && o.vs == m.vs && near(&shown.bounds, &w.bounds) && near(o.points.as_flattened(), m.points.as_flattened())
}

/// A Background is turned into a normal layer by the caller first
/// ([`crate::extra_cmds::background_to_layer`]).
/// `Err(Cancelled)` once `ctl` is cancelled: between the steps of a warp that don't check it.
fn cancelled(ctl: &Interrupt) -> Result<()> {
    if ctl.cancelled() { Err(EngineError::Cancelled) } else { Ok(()) }
}

/// Warp a whole layer, carrying its linked masks without the isolated-channel area cap.
/// `ctl` reports progress and cancels (`Err(Cancelled)`, the layer left half-done for the caller
/// to discard).
#[allow(clippy::too_many_arguments)]
fn warp_layer(
    doc_sel: Option<&Surface>,
    group: Locks,
    l: &mut Layer,
    w: &Warp,
    rect: Rect,
    interp: Interp,
    frames: Option<&Frames>,
    ctl: &Interrupt,
) -> Result<()> {
    let locks = l.locks.union(group);
    if locks.position || locks.all {
        return Err(EngineError::Other(format!("layer \"{}\" is locked", l.name)));
    }
    let map = |x: f64, y: f64| w.map(x, y);
    // A selection limits the linked mask exactly as it limits the pixels (#4175, as Free Transform
    // and Perspective Warp do, #3083); a layer that moves whole takes its whole mask along.
    let mask_sel = doc_sel.filter(|_| !crate::transform_cmds::moves_whole(l));
    // The masks (and so a fill's frame) move, but a fill layer's cached render isn't warped:
    // drop it, the fill re-renders from its model.
    l.fill_cache = None;
    match &mut l.content {
        LayerContent::Group(g) => {
            for c in g.children.iter_mut() {
                warp_layer(None, locks, c, w, rect, interp, frames, ctl)?;
            }
        }
        LayerContent::Text(_) => return Err(EngineError::Other("Warp needs rasterized type (or use Type › Warp Text)".into())),
        // The path is warped and the shape stays a shape (#3009).
        LayerContent::Shape(sh) => crate::vector_cmds::map_shape(sh, None, Some(w))?,
        LayerContent::Smart(sm) => {
            // Store in source space: undo the placement (affine, or Distort / Perspective) around the warp.
            let inv = crate::smart_cmds::placement(sm).inverse().ok_or_else(|| EngineError::Other("the smart object's transform is degenerate".into()))?;
            let corners =
                [[w.bounds[0], w.bounds[1]], [w.bounds[2], w.bounds[1]], [w.bounds[2], w.bounds[3]], [w.bounds[0], w.bounds[3]]].map(|p| hom_apply(&inv, p));
            let sb = corners.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
            let src_warp = match w.style {
                WarpStyle::None => None,
                WarpStyle::Custom => {
                    let mesh = w.to_mesh(1, 1).map_points(|p| hom_apply(&inv, p));
                    // A `mesh` parameter's knots span the contents, not the warp box's image (#4113).
                    match frames.and_then(|f| f.get(&l.id)).and_then(|&c| content_frame_warp(&mesh, c, sb)) {
                        Some(cw) => Some(cw),
                        None => Some(Warp::custom(mesh, sb)),
                    }
                }
                WarpStyle::Cylinder => cylinder_in_source(w, &inv, sb),
                _ => Some(Warp { bounds: sb, mesh: None, ..w.clone() }),
            };
            sm.warp = src_warp.filter(|w| !w.is_identity());
            // Fallback appearance for sources that can't be re-rendered.
            if let Some(c) = &mut sm.cache {
                *c = warp_mesh_surface_with(c, rect, &map, interp, ctl).ok_or(EngineError::Cancelled)?;
            }
            if let Some(m) = &mut sm.filter_mask {
                m.surface = warp_mesh_gray(&m.surface, &map, interp);
            }
        }
        _ => {
            if let Some(surf) = l.surface_mut() {
                *surf = match doc_sel {
                    Some(sel) => {
                        let (lifted, mut rest) = crate::transform_cmds::split_selected(surf, sel)?;
                        let moved = warp_mesh_surface_with(&lifted, rect, &map, interp, ctl).ok_or(EngineError::Cancelled)?;
                        cancelled(ctl)?;
                        crate::transform_cmds::composite_over(&mut rest, &moved);
                        rest.prune();
                        rest
                    }
                    None => {
                        // Content outside the warp box stays where it is.
                        let fmt = surf.format();
                        let warped = warp_mesh_surface_with(surf, rect, &map, interp, ctl).ok_or(EngineError::Cancelled)?;
                        cancelled(ctl)?;
                        // Tiles are shared, so a surface that has alpha already costs nothing to copy.
                        let mut rest = if fmt.alpha { surf.clone() } else { surf.convert(photocraft_color::PixelFormat::new(fmt.mode, fmt.sample, true)) };
                        crate::pixels::try_clear_surface(&mut rest, rect, None)?;
                        if rest.content_bounds().is_empty() {
                            // The box held all of the layer: the warp is the whole result.
                            warped
                        } else {
                            cancelled(ctl)?;
                            crate::transform_cmds::composite_over(&mut rest, &warped);
                            rest.prune();
                            rest
                        }
                    }
                };
            }
        }
    }
    if let Some(m) = l.mask.as_mut()
        && m.linked
    {
        m.surface = match mask_sel {
            // Use the selection directly when its frame fits the warp box. An explicit
            // smaller box still needs the clipped coverage built by `warp_scalar`.
            Some(sel) if sel.content_bounds().intersect(&rect) == sel.content_bounds() => crate::transform_cmds::move_gray_selected(
                &m.surface,
                sel,
                &|lifted, src| warp_mesh_surface_with(lifted, src, &map, interp, ctl).ok_or(EngineError::Cancelled),
                &|s| Ok(warp_mesh_gray(s, &map, interp)),
            )?,
            Some(sel) => warp_scalar(&m.surface, Some(sel), rect, w, interp, ctl)?,
            // Whole-layer transforms carry the complete linked mask, whose content can
            // extend beyond the layer's pixel bounds (or the explicit warp frame). The
            // scalar-channel working-area cap must not reject an ordinary layer warp.
            None => warp_mesh_gray(&m.surface, &map, interp),
        };
    }
    if let Some(vm) = l.vector_mask.as_mut()
        && vm.linked
    {
        vm.path = crate::vector_cmds::map_path(&vm.path, None, Some(w))?;
    }
    Ok(())
}

/// Source boxes of the smart objects under a warped layer, by layer id.
type Frames = std::collections::HashMap<LayerId, [f64; 4]>;

fn source_frames(doc: &Document, l: &Layer, out: &mut Frames) {
    match &l.content {
        LayerContent::Group(g) => g.children.iter().for_each(|c| source_frames(doc, c, out)),
        LayerContent::Smart(sm) => {
            if let Some(b) = crate::smart_cmds::source_box(doc, sm) {
                out.insert(l.id, b);
            }
        }
        _ => {}
    }
}

/// Rescales `knots` (spanning the box starting at `c0`, `cs` long) to span the frame `t0`, `ts`
/// (which contains it). Returns the knots and whether the frame extends past the box at the start / end.
fn reframe_knots(knots: &[f64], c0: f64, cs: f64, t0: f64, ts: f64) -> (Vec<f64>, bool, bool) {
    let pre = (c0 - t0) / ts > 1e-9;
    let post = (t0 + ts - (c0 + cs)) / ts > 1e-9;
    let mut out: Vec<f64> = Vec::with_capacity(knots.len() + 2);
    if pre {
        out.push(0.0);
    }
    out.extend(knots.iter().map(|k| (c0 + k * cs - t0) / ts));
    if post {
        out.push(1.0);
    }
    // Keep the ends exactly 0 and 1.
    if let Some(f) = out.first_mut() {
        *f = 0.0;
    }
    if let Some(l) = out.last_mut() {
        *l = 1.0;
    }
    (out, pre, post)
}

/// `items` with three more copies of the first / last one at the front (`pre`) / back (`post`).
fn pad<T: Clone>(items: &[T], pre: bool, post: bool) -> Option<Vec<T>> {
    let mut out = Vec::with_capacity(items.len() + 6);
    if pre {
        out.extend(std::iter::repeat_n(items.first()?.clone(), 3));
    }
    out.extend_from_slice(items);
    if post {
        out.extend(std::iter::repeat_n(items.last()?.clone(), 3));
    }
    Some(out)
}

/// A stored smart-object warp from `mesh` (points in source space) whose knots span the source
/// box `c`: the knots are re-expressed over the frame `hull(c, b)` (`b` = the warp box's image in
/// source space), with a degenerate patch padding every side where the frame extends past `c`.
/// The surface over `c` is exactly the mesh's (#4113).
fn content_frame_warp(mesh: &BezierMesh, c: [f64; 4], b: [f64; 4]) -> Option<Warp> {
    let t = [c[0].min(b[0]), c[1].min(b[1]), c[2].max(b[2]), c[3].max(b[3])];
    let (cw, ch) = (c[2] - c[0], c[3] - c[1]);
    if !(cw > 0.0 && ch > 0.0 && t.iter().all(|v| v.is_finite())) {
        return None;
    }
    let (us, left, right) = reframe_knots(&mesh.us, c[0], cw, t[0], t[2] - t[0]);
    let (vs, top, bottom) = reframe_knots(&mesh.vs, c[1], ch, t[1], t[3] - t[1]);
    let rows: Vec<Vec<[f64; 2]>> = mesh.points.chunks(mesh.nx()).map(|r| pad(r, left, right)).collect::<Option<_>>()?;
    let rows = pad(&rows, top, bottom)?;
    let m = BezierMesh { us, vs, points: rows.into_iter().flatten().collect() };
    m.is_valid().then(|| Warp::custom(m, t))
}

/// Largest warp frame side: far above any real layer, far below what would exhaust memory.
const MAX_WARP_SIDE: u32 = 100_000;

fn target(s: &Session, cmd: &str, p: &Value) -> Result<(LayerId, Rect)> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = match p.get("layer").and_then(Value::as_u64) {
        Some(v) => LayerId(v),
        None => st.active_layer.ok_or(EngineError::Other("no active layer".into()))?,
    };
    let layer = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let rect = match p.get("rect").and_then(Value::as_array) {
        Some(r) if r.len() == 4 => {
            let v: Vec<i32> = r.iter().map(|x| x.as_f64().unwrap_or(0.0).round() as i32).collect();
            Rect::new(v[0], v[1], v[2], v[3])
        }
        _ => warp_bounds(&st.doc, layer),
    };
    if rect.is_empty() {
        return Err(bad(cmd, "nothing to warp"));
    }
    let (w, h) = (rect.width(), rect.height());
    if w > MAX_WARP_SIDE || h > MAX_WARP_SIDE {
        return Err(bad(cmd, format!("rect too large for warp: {w} x {h} (max {MAX_WARP_SIDE} per side)")));
    }
    Ok((id, rect))
}

/// The Warp Text settings a transform warp gives a type layer: a preset style (or none) keeps the
/// type editable, as Photoshop's Edit › Transform › Warp on type does (#3004).
fn type_warp(cmd: &str, w: &Warp) -> Result<Option<photocraft_doc::text::TextWarp>> {
    let clamp = |v: f64| if v.is_finite() { v.clamp(-100.0, 100.0) as f32 } else { 0.0 };
    match w.style {
        WarpStyle::None => Ok(None),
        WarpStyle::Custom if w.is_identity() => Ok(None),
        WarpStyle::Custom => Err(bad(cmd, "type can only take a warp style (arc, flag, …); a custom mesh needs rasterized type")),
        WarpStyle::Cylinder => Err(bad(cmd, "type can only take a warp style (arc, flag, …); a cylinder needs rasterized type")),
        s => Ok(Some(photocraft_doc::text::TextWarp {
            style: s.psd_name().into(),
            value: clamp(w.bend),
            horizontal_distortion: clamp(w.h_distort),
            vertical_distortion: clamp(w.v_distort),
            horizontal: !w.vertical,
        })),
    }
}

/// Warp on a type layer sets its Warp Text style: one history step, and the type stays editable.
fn warp_type_layer(s: &mut Session, cmd: &str, id: LayerId, w: &Warp) -> Result<Value> {
    let tw = type_warp(cmd, w)?;
    let unchanged = s.active().and_then(|d| d.doc.layer(id)).is_some_and(|l| matches!(&l.content, LayerContent::Text(t) if t.warp == tw));
    if unchanged {
        return Ok(json!({"layer": id.0, "changed": false}));
    }
    s.edit("Warp", |doc, _| {
        let group = crate::transform_cmds::group_locks(doc, id);
        let (dpi, format) = (doc.resolution_dpi, doc.pixel_format());
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let locks = l.locks.union(group);
        if locks.position || locks.all {
            return Err(EngineError::Other(format!("layer \"{}\" is locked", l.name)));
        }
        let LayerContent::Text(t) = &mut l.content else { return Err(EngineError::Other("not a type layer".into())) };
        t.warp = tw.clone();
        crate::type_cmds::refresh_at(dpi, format, t);
        Ok(())
    })?;
    Ok(json!({"layer": id.0, "changed": true, "warp": tw}))
}

/// Lift only scalar samples inside the source frame, leaving all other samples in place.
/// Coverage also includes default-valued samples, which must move with a selected mask.
fn warp_scalar(s: &Surface, selection: Option<&Surface>, frame: Rect, w: &Warp, interp: Interp, ctl: &Interrupt) -> Result<Surface> {
    let area = selection.map_or(frame, |sel| frame.intersect(&sel.content_bounds()));
    let mut coverage = Surface::new(photocraft_color::PixelFormat::new(photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::F32, false));
    for tile in area.tiles() {
        cancelled(ctl)?;
        let r = tile.rect().intersect(&area);
        let values = match selection {
            Some(sel) => crate::allocation::read_region(sel, r, "clipping a Warp selection")?,
            None => crate::allocation::filled(r.width() as usize * r.height() as usize, 1.0, "framing a channel Warp")?,
        };
        coverage.try_write_region(r, &values)?;
    }
    let map = |x, y| w.map(x, y);
    crate::transform_cmds::move_gray_selected(
        s,
        &coverage,
        &|src, bounds| warp_mesh_surface_with(src, bounds, &map, interp, ctl).ok_or(EngineError::Cancelled),
        &|_| Err(bad("edit.transform.warp", "expected a scalar channel")),
    )
}

fn apply(s: &mut Session, p: &Value, cmd: &str) -> Result<Value> {
    let p = &crate::channel_transform::params(
        s,
        &if cmd == "layer.smartObjects.warp" && p.get("target").is_none() {
            let mut p = p.clone();
            p["target"] = json!("pixels");
            p
        } else {
            p.clone()
        },
    )?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let layer = p.get("layer").and_then(Value::as_u64).map(LayerId).or(st.active_layer);
    if let Some(plane) = crate::channel_transform::plane(&st.doc, layer, p, s.tools.background)? {
        crate::channel_transform::check_locks(&st.doc, layer, p)?;
        let bounds = crate::channel_transform::bounds(&st.doc, &plane, p)?;
        let mut rect = match p.get("rect") {
            Some(r) => {
                let r: [f64; 4] = serde_json::from_value(r.clone()).map_err(|_| bad(cmd, "rect must contain four numbers"))?;
                if r.iter().any(|v| !v.is_finite() || v.abs() > f64::from(1 << 28)) {
                    return Err(bad(cmd, "warp frame is out of range"));
                }
                r
            }
            None => rect_f(bounds),
        };
        crate::channel_transform::validate_area(plane.content_bounds())?;
        if let Some(sel) = &st.doc.selection {
            crate::channel_transform::validate_area(sel.content_bounds())?;
        }
        let w = warp_from_params(cmd, p, rect)?;
        // A complete inline warp carries its source frame. An explicit rect can restrict it.
        if p.get("rect").is_none() && p.get("warp").is_some() {
            rect = w.bounds;
        }
        if rect.iter().any(|v| !v.is_finite() || v.abs() > f64::from(1 << 28)) || rect[2] <= rect[0] || rect[3] <= rect[1] {
            return Err(bad(cmd, "warp frame is empty or out of range"));
        }
        let frame = Rect::new(rect[0].floor() as i32, rect[1].floor() as i32, rect[2].ceil() as i32, rect[3].ceil() as i32);
        crate::channel_transform::validate_area(frame)?;
        let mesh = w.to_mesh(1, 1);
        let b = mesh.points.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, v| {
            [b[0].min(v[0]), b[1].min(v[1]), b[2].max(v[0]), b[3].max(v[1])]
        });
        if mesh.points.iter().flatten().any(|v| !v.is_finite() || v.abs() > f64::from(1 << 28)) {
            return Err(bad(cmd, "warp coordinates are out of range"));
        }
        crate::channel_transform::validate_area(Rect::new(b[0].floor() as i32, b[1].floor() as i32, b[2].ceil() as i32, b[3].ceil() as i32))?;
        if w.is_identity() {
            return Ok(json!({"changed": false}));
        }
        let interp = Interp::parse(p.get("interpolation").and_then(Value::as_str).unwrap_or("bicubic"));
        s.edit("Warp", |doc, _| {
            let moved = warp_scalar(&plane, doc.selection.as_ref(), frame, &w, interp, &Interrupt::NONE)?;
            crate::channel_transform::write_plane(doc, layer, p, moved)?;
            if let Some(sel) = &doc.selection {
                doc.selection = Some(warp_scalar(sel, None, frame, &w, Interp::Bilinear, &Interrupt::NONE)?).filter(|s| !s.content_bounds().is_empty());
            }
            Ok(())
        })?;
        return Ok(json!({"changed": true, "rect": rect}));
    }
    let (id, rect) = target(s, cmd, p)?;
    let w = warp_from_params(cmd, p, rect_f(rect))?;
    if matches!(s.active().and_then(|d| d.doc.layer(id)).map(|l| &l.content), Some(LayerContent::Text(_))) {
        return warp_type_layer(s, cmd, id, &w);
    }
    let is_smart = matches!(s.active().and_then(|d| d.doc.layer(id)).map(|l| &l.content), Some(LayerContent::Smart(_)));
    if w.is_identity() && !is_smart {
        return Ok(json!({"layer": id.0, "changed": false}));
    }
    // An untouched cylinder (Warp mode, then Enter) stays as it is, with no history step.
    if p.get("mesh").is_none() && s.active().and_then(|d| d.doc.layer(id)).is_some_and(|l| is_stored_cylinder(l, &w)) {
        return Ok(json!({"layer": id.0, "changed": false}));
    }
    let interp = Interp::parse(p.get("interpolation").and_then(Value::as_str).unwrap_or("bicubic"));
    let content_knots = p.get("mesh").is_some() && p.get("warp").is_none();
    // A background job when started with `Session::start` (as Free Transform, #4218): the warp of
    // a large layer runs on a worker with progress, and can be cancelled.
    crate::jobs::edit_job(
        s,
        "Warp",
        move |doc, _, ctx| ctx.stage(0.0, 1.0, "Warping", |ctl| warp_doc(doc, id, &w, rect, interp, content_knots, ctl)),
        move |()| json!({"layer": id.0, "changed": true, "rect": [rect.x0, rect.y0, rect.x1, rect.y1]}),
    )
}

/// The edit of [`apply`]: the layer and the selection outline.
fn warp_doc(doc: &mut Document, id: LayerId, w: &Warp, rect: Rect, interp: Interp, content_knots: bool, ctl: &Interrupt) -> Result<()> {
    let sel = doc.selection.clone();
    let frames = content_knots.then(|| {
        let mut f = Frames::new();
        if let Some(l) = doc.layer(id) {
            source_frames(doc, l, &mut f);
        }
        f
    });
    let is_group = doc.layer(id).is_some_and(Layer::is_group);
    let group = crate::transform_cmds::group_locks(doc, id);
    crate::extra_cmds::background_to_layer(doc, id);
    let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
    warp_layer(if is_group { None } else { sel.as_ref() }, group, l, w, rect, interp, frames.as_ref(), ctl)?;
    let snapshot = doc.clone();
    if let Some(l) = doc.layer_mut(id) {
        crate::transform_cmds::refresh_text(&snapshot, l);
    }
    if let Some(sel) = &doc.selection {
        doc.selection = Some(warp_mesh_gray(sel, &|x, y| w.map(x, y), Interp::Bilinear)).filter(|s| !s.content_bounds().is_empty());
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub enum Split {
    Crosswise,
    Horizontal,
    Vertical,
    Remove,
}

/// The default warp draws rule-of-thirds guides on its single patch. Make those real sections
/// before a split, so the cut divides the grid on screen instead of replacing it.
fn commit_default_sections(mesh: &mut BezierMesh) {
    let _ = mesh.split_u(1.0 / 3.0);
    let _ = mesh.split_u(2.0 / 3.0);
    let _ = mesh.split_v(1.0 / 3.0);
    let _ = mesh.split_v(2.0 / 3.0);
}

/// Applies a split (or removes the split nearest `at`) on `w`'s mesh (presets become custom).
pub fn split_warp(w: &Warp, at: Option<[f64; 2]>, how: Split) -> Option<Warp> {
    let mut mesh = w.to_mesh(1, 1);
    if !matches!(how, Split::Remove) && w.style == WarpStyle::Custom && mesh.us.len() == 2 && mesh.vs.len() == 2 {
        commit_default_sections(&mut mesh);
    }
    let (s, t) = match at {
        Some(p) => mesh.param_at(p),
        None => (0.5, 0.5),
    };
    // Split inside the patch: the middle of the patch containing (s, t) when no point is given.
    let mid = |k: &[f64], v: f64| {
        let i = k.windows(2).position(|w| v <= w[1]).unwrap_or(k.len() - 2);
        (k[i] + k[i + 1]) / 2.0
    };
    let (s, t) = if at.is_some() { (s, t) } else { (mid(&mesh.us, s), mid(&mesh.vs, t)) };
    let ok = match how {
        Split::Crosswise => {
            let a = mesh.split_u(s);
            let b = mesh.split_v(t);
            a || b
        }
        // A horizontal split line divides rows; a vertical one divides columns.
        Split::Horizontal => mesh.split_v(t),
        Split::Vertical => mesh.split_u(s),
        Split::Remove => {
            let near = |k: &[f64], v: f64| (1..k.len() - 1).min_by(|a, b| (k[*a] - v).abs().total_cmp(&(k[*b] - v).abs()));
            let (iu, iv) = (near(&mesh.us, s), near(&mesh.vs, t));
            let du = iu.map_or(f64::MAX, |i| (mesh.us[i] - s).abs());
            let dv = iv.map_or(f64::MAX, |i| (mesh.vs[i] - t).abs());
            match (iu, iv) {
                (Some(i), _) if du <= dv => mesh.remove_split_u(i),
                (_, Some(i)) => mesh.remove_split_v(i),
                (Some(i), None) => mesh.remove_split_u(i),
                _ => false,
            }
        }
    };
    ok.then(|| Warp::custom(mesh, w.bounds))
}

fn param_rect(p: &Value) -> [f64; 4] {
    let Some(r) = p.get("rect").and_then(Value::as_array) else {
        return [0.0, 0.0, 1.0, 1.0];
    };
    let at = |i: usize, d: f64| r.get(i).and_then(Value::as_f64).unwrap_or(d);
    [at(0, 0.0), at(1, 0.0), at(2, 1.0), at(3, 1.0)]
}

/// A warp passed inline (`warp`, `mesh` or `style`). `None` means "use the active smart object".
fn inline_warp(cmd: &str, p: &Value) -> Result<Option<Warp>> {
    if p.get("warp").is_none() && p.get("mesh").is_none() && p.get("style").is_none() {
        return Ok(None);
    }
    Ok(Some(warp_from_params(cmd, p, param_rect(p))?))
}

/// These calls only return an edited mesh; they cannot change the stored document warp.
pub(crate) fn inline_edit(id: &str, p: &Value) -> bool {
    matches!(
        id,
        "edit.transform.splitWarpCrosswise"
            | "edit.transform.splitWarpHorizontally"
            | "edit.transform.splitWarpVertically"
            | "edit.transform.removeWarpSplit"
            | "edit.transform.warpGrid"
    ) && (p.get("warp").is_some() || p.get("mesh").is_some() || p.get("style").is_some())
}

fn split(s: &mut Session, p: &Value, cmd: &str, how: Split) -> Result<Value> {
    let at = p.get("at").and_then(Value::as_array).and_then(|a| Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?]));
    if let Some(w) = inline_warp(cmd, p)? {
        let out = split_warp(&w, at, how).ok_or_else(|| bad(cmd, "nothing to split or remove there"))?;
        return Ok(json!({"warp": out}));
    }
    // The active smart object's stored warp.
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = st.active_layer.ok_or(EngineError::Other("no active layer".into()))?;
    let layer = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let LayerContent::Smart(sm) = &layer.content else {
        return Err(bad(cmd, "pass the warp being edited (`warp`), or select a warped smart object"));
    };
    let w = sm.warp.clone().unwrap_or_else(|| {
        let b = layer.surface().map_or(Rect::EMPTY, Surface::content_bounds);
        let inv = crate::smart_cmds::placement(sm).inverse().unwrap_or(Homography::IDENTITY);
        let c = [hom_apply(&inv, [f64::from(b.x0), f64::from(b.y0)]), hom_apply(&inv, [f64::from(b.x1), f64::from(b.y1)])];
        Warp::none([c[0][0].min(c[1][0]), c[0][1].min(c[1][1]), c[0][0].max(c[1][0]), c[0][1].max(c[1][1])])
    });
    let t = crate::smart_cmds::placement(sm);
    let at_src = at.and_then(|a| t.inverse().map(|inv| hom_apply(&inv, a)));
    let out = split_warp(&w, at_src, how).ok_or_else(|| bad(cmd, "nothing to split or remove there"))?;
    let label = if matches!(how, Split::Remove) { "Remove Warp Split" } else { "Split Warp" };
    s.edit(label, |doc, _| {
        if let Some(Layer { content: LayerContent::Smart(sm), .. }) = doc.layer_mut(id) {
            sm.warp = Some(out.clone());
        }
        crate::smart_cmds::refresh(doc, id).map(|_| ())
    })?;
    Ok(json!({"layer": id.0, "warp": out}))
}

/// Square preset size. `"default"` is 1. Numbers outside 1..=64 are rejected, not clamped.
fn grid_size(cmd: &str, p: &Value) -> Result<usize> {
    let Some(v) = p.get("size").or_else(|| p.get("grid")) else {
        return Err(bad(cmd, "pass `size` (1, 3, 4, 5 or \"default\")"));
    };
    if v.as_array().is_some() {
        return Err(bad(cmd, "pass `size` as a number or \"default\", \"3\", \"4\" or \"5\""));
    }
    let n = match v {
        Value::String(s) => match s.as_str() {
            "default" => 1,
            "3" | "3x3" => 3,
            "4" | "4x4" => 4,
            "5" | "5x5" => 5,
            _ => return Err(bad(cmd, "grid size must be default, 3, 4 or 5")),
        },
        Value::Number(num) => num.as_u64().ok_or_else(|| bad(cmd, "grid size must be a whole number from 1 to 64"))?,
        _ => return Err(bad(cmd, "grid size must be a number or default, 3, 4 or 5")),
    };
    if !(1..=64).contains(&n) {
        return Err(bad(cmd, "grid size must be from 1 to 64"));
    }
    Ok(n as usize)
}

/// Even `n`×`n` patches fitted to `w`. The surface stays; knots sit at `i/n`.
fn resize_warp(w: &Warp, n: usize) -> Warp {
    let mesh = w.to_mesh(1, 1);
    let knots: Vec<f64> = (0..=n).map(|i| i as f64 / n as f64).collect();
    Warp::custom(BezierMesh::fit(&|s, t| mesh.eval(s, t), knots.clone(), knots), w.bounds)
}

fn grid(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "edit.transform.warpGrid";
    let n = grid_size(CMD, p)?;
    if let Some(w) = inline_warp(CMD, p)? {
        return Ok(json!({"warp": resize_warp(&w, n)}));
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = st.active_layer.ok_or(EngineError::Other("no active layer".into()))?;
    let layer = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let LayerContent::Smart(sm) = &layer.content else {
        return Err(bad(CMD, "pass the warp being edited (`warp`), or select a smart object"));
    };
    let w = sm.warp.clone().unwrap_or_else(|| {
        let b = layer.surface().map_or(Rect::EMPTY, Surface::content_bounds);
        // The layer's pixels back in source space, through the full placement (affine, or
        // Distort / Perspective).
        let r = rect_f(b);
        let c = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]];
        let c = match crate::smart_cmds::placement(sm).inverse() {
            Some(inv) => c.map(|p| hom_apply(&inv, p)),
            None => c,
        };
        Warp::none(c.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]))
    });
    let out = resize_warp(&w, n);
    s.edit("Warp Grid", |doc, _| {
        if let Some(Layer { content: LayerContent::Smart(sm), .. }) = doc.layer_mut(id) {
            sm.warp = Some(out.clone());
        }
        crate::smart_cmds::refresh(doc, id).map(|_| ())
    })?;
    Ok(json!({"layer": id.0, "warp": out}))
}

pub fn specs() -> Vec<CommandSpec> {
    const P: &str = r##"{"layer":id?,"rect":[x0,y0,x1,y1]? (source box; default = layer content/selection bounds; scalar targets use inline warp bounds when given, otherwise target/selection bounds, and preserve samples outside the box),"style":"custom|none|arc|arcLower|arcUpper|arch|bulge|shellLower|shellUpper|flag|wave|fish|rise|fisheye|inflate|squeeze|twist|cylinder","bend":%=50,"hDistort":%,"vDistort":%,"vertical":bool,"values":[left,bottom,right,top,topDepth,bottomDepth,wrap]? (cylinder: silhouettes and edges in document px, the ellipses' depths in half-axis units, wrap 0..1; horizontal only),"mesh":{"us":[0,…,1],"vs":[0,…,1],"points":[[x,y]…]} ((3c+1)×(3r+1) control points, row-major, document px; on a smart object `us`/`vs` span the source contents, so points equal to the placement are an identity warp even when it is skewed),"grid":[cols,rows]?,"warp":{full warp object}?,"interpolation":"bicubic|bilinear|nearest","target":"pixels"|"mask"|"quickMask"|{"channel":i}|{"color":i}=Channels panel target (explicit layer without target transforms the layer; explicit mask transforms alone regardless of linking)} — on a type layer a style (or none) sets its Warp Text and the type stays editable"##;
    const S: &str = r##"{"warp":{…}? (the warp being edited; returned split),"rect":[x0,y0,x1,y1]?,"at":[x,y]? (document point; default = middle of the patch)} — without `warp`, edits the active smart object's warp. A 1×1 custom mesh is divided at the thirds first (the guides drawn on the default grid), so splitWarpCrosswise on an identity custom warp returns a 4×4 mesh (knots at 0, 1/3, 1/2, 2/3 and 1), not a 2×2. A preset is fitted as one patch and split without that step."##;
    const G: &str = r##"{"warp":{…}? (the warp being edited; returned with the new mesh),"rect":[x0,y0,x1,y1]?,"size":n|"default"|"3"|"4"|"5" (square patches, 1..=64; 1 and "default" are the single-patch grid)} — without `warp`/`style`/`mesh`, edits the active smart object's warp. The surface is kept and the new lines are real splits at i/n."##;
    vec![
        CommandSpec {
            id: "edit.transform.warp",
            label: "Warp",
            menu: &["Edit", "Transform"],
            shortcut: None,
            params: P,
            enabled: has_layer,
            journal: true,
            run: |s, p| apply(s, p, "edit.transform.warp"),
        },
        CommandSpec {
            id: "layer.smartObjects.warp",
            label: "Warp",
            menu: &["Layer", "Smart Objects"],
            shortcut: None,
            params: P,
            enabled: active_smart,
            journal: true,
            run: |s, p| apply(s, p, "layer.smartObjects.warp"),
        },
        CommandSpec {
            id: "edit.transform.splitWarpCrosswise",
            label: "Split Warp Crosswise",
            menu: &["Edit", "Transform"],
            shortcut: None,
            params: S,
            enabled: has_layer,
            journal: true,
            run: |s, p| split(s, p, "edit.transform.splitWarpCrosswise", Split::Crosswise),
        },
        CommandSpec {
            id: "edit.transform.splitWarpHorizontally",
            label: "Split Warp Horizontally",
            menu: &["Edit", "Transform"],
            shortcut: None,
            params: S,
            enabled: has_layer,
            journal: true,
            run: |s, p| split(s, p, "edit.transform.splitWarpHorizontally", Split::Horizontal),
        },
        CommandSpec {
            id: "edit.transform.splitWarpVertically",
            label: "Split Warp Vertically",
            menu: &["Edit", "Transform"],
            shortcut: None,
            params: S,
            enabled: has_layer,
            journal: true,
            run: |s, p| split(s, p, "edit.transform.splitWarpVertically", Split::Vertical),
        },
        CommandSpec {
            id: "edit.transform.removeWarpSplit",
            label: "Remove Warp Split",
            menu: &["Edit", "Transform"],
            shortcut: None,
            params: S,
            enabled: has_layer,
            journal: true,
            run: |s, p| split(s, p, "edit.transform.removeWarpSplit", Split::Remove),
        },
        CommandSpec {
            id: "edit.transform.warpGrid",
            label: "Warp Grid",
            menu: &[],
            shortcut: None,
            params: G,
            enabled: has_layer,
            journal: true,
            run: |s, p| grid(s, p),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_warp_box_holding_the_whole_layer_gives_what_compositing_over_the_cleared_layer_did() {
        for depth in [8, 16, 32] {
            let s = session(depth);
            let st = s.active().unwrap();
            let l = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
            let surf = l.surface().unwrap().clone();
            let rect = surf.content_bounds();
            let w = Warp::preset(WarpStyle::Flag, 35.0, rect_f(rect));
            // What the warp built before the shortcut: the warp composited over the cleared layer.
            let map = |x: f64, y: f64| w.map(x, y);
            let warped = warp_mesh_surface_with(&surf, rect, &map, Interp::Bicubic, &Interrupt::NONE).unwrap();
            let mut want = surf.clone();
            crate::pixels::try_clear_surface(&mut want, rect, None).unwrap();
            crate::transform_cmds::composite_over(&mut want, &warped);
            want.prune();
            let mut got = l.clone();
            warp_layer(None, Locks::default(), &mut got, &w, rect, Interp::Bicubic, None, &Interrupt::NONE).unwrap();
            let got = got.surface().unwrap();
            let b = want.content_bounds();
            assert_eq!(got.content_bounds(), b, "depth {depth}");
            let (g, wv) = (got.read_region(b), want.read_region(b));
            // Exact at 8 and 16 bits. At 32 the composite's `t·a / a` rounds a few values by an
            // ulp; the shortcut keeps the warp's own.
            let tol = if depth == 32 { 1e-6 } else { 0.0 };
            let worst = g.iter().zip(&wv).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            assert!(worst <= tol, "depth {depth}: off by {worst}");
        }
    }

    fn session(depth: u64) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 80, "height": 60, "depth": depth})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("paint", |doc, active| {
            let l = doc.layer_mut(active.unwrap()).unwrap();
            let surf = l.surface_mut().unwrap();
            for y in 10..40 {
                for x in 10..60 {
                    let v = ((x * 3 + y * 5) % 11) as f32 / 10.0;
                    surf.write_pixel(x, y, &[v, 1.0 - v, 0.5, 1.0]);
                }
            }
            Ok(())
        })
        .unwrap();
        s
    }

    fn active_surface(s: &Session) -> Surface {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().clone()
    }

    /// Warp on type sets its Warp Text (#3004): the layer stays type, one step; a custom mesh is
    /// refused rather than rasterizing.
    #[test]
    fn warp_style_on_type_is_warp_text() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
        s.execute("type.create", json!({"x": 10, "y": 60, "text": "Hi"})).unwrap();
        let n = s.active().unwrap().history.past_len();
        s.execute("edit.transform.warp", json!({"style": "arc", "bend": 30})).unwrap();
        let st = s.active().unwrap();
        let LayerContent::Text(t) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("type") };
        assert_eq!(t.warp.as_ref().map(|w| (w.style.as_str(), w.value)), Some(("warpArc", 30.0)));
        assert_eq!(st.history.past_len(), n + 1);
        let mesh = BezierMesh::identity([0.0, 0.0, 10.0, 10.0], 1, 1).map_points(|p| [p[0] * 2.0, p[1]]);
        assert!(s.execute("edit.transform.warp", json!({"warp": Warp::custom(mesh, [0.0, 0.0, 10.0, 10.0])})).is_err());
    }

    /// The seven values of a cylinder over the session's painted box (x 10..60, y 10..40).
    const CYL: [f64; 7] = [10.0, 40.0, 60.0, 10.0, 0.3, 0.3, 0.5];

    /// `style: "cylinder"` takes its seven values (#4112); anything that isn't a horizontal
    /// cylinder is refused, never a silent no-op, and the layer is left alone.
    #[test]
    fn cylinder_style_needs_its_values() {
        let mut s = session(8);
        let before = active_surface(&s);
        for p in [
            json!({"style": "cylinder"}),
            json!({"style": "cylinder", "values": [10, 40, 60, 10, 0.3, 0.3]}),
            json!({"style": "cylinder", "values": [60, 40, 10, 10, 0.3, 0.3, 0.5]}),
            json!({"style": "cylinder", "values": [10, 40, 60, 10, 0.3, 0.3, "x"]}),
            json!({"style": "cylinder", "values": "seven"}),
            json!({"style": "cylinder", "values": CYL, "vertical": true}),
            json!({"style": "cylinder", "values": CYL, "hDistort": 10}),
            json!({"warp": {"style": "cylinder", "bounds": [10.0, 10.0, 60.0, 40.0]}}),
            json!({"warp": {"style": "cylinder", "values": CYL, "bounds": [10.0, 10.0, 60.0, 40.0], "vertical": true}}),
            json!({"warp": {"style": "cylinder", "values": CYL, "bounds": [10.0, 10.0, 60.0, 40.0], "hDistort": 10.0}}),
            json!({"warp": {"style": "cylinder", "values": CYL, "bounds": [10.0, 10.0, 60.0, 40.0], "vDistort": -5.0}}),
        ] {
            let e = s.execute("edit.transform.warp", p.clone()).unwrap_err().to_string();
            assert!(e.contains("cylinder"), "{p}: {e}");
        }
        assert_eq!(active_surface(&s), before);
        // On type it is refused like a custom mesh: Warp Text has no cylinder.
        s.execute("type.create", json!({"x": 10, "y": 50, "text": "Hi"})).unwrap();
        assert!(s.execute("edit.transform.warp", json!({"style": "cylinder", "values": CYL})).is_err());
    }

    #[test]
    fn cylinder_warps_pixels_at_every_depth() {
        for depth in [8u64, 16, 32] {
            let mut s = session(depth);
            let r = s.execute("edit.transform.warp", json!({"style": "cylinder", "values": CYL})).unwrap();
            assert_eq!(r["changed"], true, "{depth}");
            let after = active_surface(&s);
            let b = after.content_bounds();
            // Between the silhouettes, the edges bowed down by the ellipses.
            assert!(b.x0 >= 9 && b.x1 <= 61 && b.y0 >= 9 && b.y1 > 40, "{depth}: {b:?}");
            // The same cylinder as a full warp object (its mesh rebuilt from the values) agrees.
            let mut t = session(depth);
            let w = Warp::cylinder(&CYL, [10.0, 10.0, 60.0, 40.0]).unwrap();
            let mut garbled = w.clone();
            garbled.mesh = None;
            t.execute("edit.transform.warp", json!({"warp": garbled})).unwrap();
            assert_eq!(active_surface(&t), after, "{depth}");
        }
    }

    /// On a smart object the cylinder is stored in source space: still a cylinder (so it goes to
    /// PSD as one) when the placement only scales and moves, its mesh when it rotates.
    #[test]
    fn smart_object_cylinder_is_stored_in_source_space() {
        let stored = |s: &Session| {
            let st = s.active().unwrap();
            let LayerContent::Smart(sm) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("smart") };
            sm.warp.clone().unwrap()
        };
        let mut s = session(8);
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        s.execute("layer.smartObjects.warp", json!({"style": "cylinder", "values": CYL, "rect": [10, 10, 60, 40]})).unwrap();
        let w = stored(&s);
        assert_eq!(w.style, WarpStyle::Cylinder);
        let LayerContent::Smart(sm) = &s.active().unwrap().doc.layers[1].content else { panic!("smart") };
        let inv = crate::smart_cmds::placement(sm).inverse().unwrap();
        let src = |x: f64, y: f64| inv.apply(x, y);
        let ((l, t), (r, bt)) = (src(10.0, 10.0), src(60.0, 40.0));
        for (a, b) in w.values.iter().zip([l, bt, r, t, 0.3, 0.3, 0.5]) {
            assert!((a - b).abs() < 1e-9, "{:?}", w.values);
        }
        // Twice as wide: the silhouettes double, the ellipses' depth (in half axes) halves.
        let mut s = session(8);
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        s.execute("edit.transform", json!({"scaleX": 200, "scaleY": 100})).unwrap();
        let wide = [-15.0, 40.0, 85.0, 10.0, 0.3, 0.3, 0.5];
        s.execute("layer.smartObjects.warp", json!({"style": "cylinder", "values": wide, "rect": [-15, 10, 85, 40]})).unwrap();
        let w = stored(&s);
        assert_eq!(w.style, WarpStyle::Cylinder);
        assert!((w.values[4] - 0.6).abs() < 1e-9 && (w.values[5] - 0.6).abs() < 1e-9, "{:?}", w.values);
        // The silhouettes land on the source's own edges (its contents span the scaled box).
        let LayerContent::Smart(sm) = &s.active().unwrap().doc.layers[1].content else { panic!("smart") };
        let inv = crate::smart_cmds::placement(sm).inverse().unwrap();
        assert!((w.values[0] - inv.apply(-15.0, 10.0).0).abs() < 1e-9 && (w.values[2] - inv.apply(85.0, 10.0).0).abs() < 1e-9, "{:?}", w.values);
        // Rotated: a custom mesh with the same shape.
        let mut s = session(8);
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        s.execute("edit.transform", json!({"rotate": 30})).unwrap();
        s.execute("layer.smartObjects.warp", json!({"style": "cylinder", "values": CYL, "rect": [10, 10, 60, 40]})).unwrap();
        let w = stored(&s);
        assert_eq!(w.style, WarpStyle::Custom);
        assert_eq!(w.mesh.as_ref().map(|m| (m.nx(), m.ny())), Some((13, 4)));
    }

    /// Warp mode shows a stored cylinder as its mesh, and Enter commits that mesh untouched: the
    /// smart object keeps its cylinder (else the next PSD save writes no warp), while a moved
    /// point still makes it a custom warp.
    #[test]
    fn untouched_cylinder_committed_from_warp_mode_stays_a_cylinder() {
        for scale in [100, 150] {
            let mut s = session(8);
            s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
            s.execute("edit.transform", json!({"scaleX": scale, "scaleY": 100})).unwrap();
            let id = s.active().unwrap().active_layer.unwrap();
            let rect = warp_bounds(&s.active().unwrap().doc, s.active().unwrap().doc.layer(id).unwrap());
            let [x0, x1] = [f64::from(rect.x0), f64::from(rect.x1)];
            let values = [x0, 40.0, x1, 10.0, 0.3, 0.3, 0.5];
            s.execute("layer.smartObjects.warp", json!({"style": "cylinder", "values": values, "rect": [rect.x0, 10, rect.x1, 40]})).unwrap();
            let stored = |s: &Session| {
                let st = s.active().unwrap();
                let LayerContent::Smart(sm) = &st.doc.layer(id).unwrap().content else { panic!("smart") };
                sm.warp.clone().unwrap()
            };
            let cylinder = stored(&s);
            assert_eq!(cylinder.style, WarpStyle::Cylinder, "{scale}");
            let shown = smart_warp_doc_space(s.active().unwrap().doc.layer(id).unwrap()).unwrap();
            assert_eq!(shown.style, WarpStyle::Custom);
            s.execute("select.all", json!({})).unwrap();
            let (sel, steps) = (s.active().unwrap().doc.selection.clone(), s.active().unwrap().history.past_len());
            let r = s.execute("edit.transform.warp", json!({"layer": id.0, "rect": shown.bounds, "warp": shown, "interpolation": "bicubic"})).unwrap();
            assert_eq!(r["changed"], false, "{scale}");
            assert_eq!(stored(&s), cylinder, "{scale}: untouched");
            let st = s.active().unwrap();
            assert!(st.doc.selection == sel && st.history.past_len() == steps, "{scale}: no step, the selection stays");
            // The same mesh over another box is another warp.
            let mut wider = shown.clone();
            wider.bounds[2] += 10.0;
            s.execute("edit.transform.warp", json!({"layer": id.0, "rect": wider.bounds, "warp": wider})).unwrap();
            assert_eq!(stored(&s).style, WarpStyle::Custom, "{scale}: other box");
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(stored(&s), cylinder);
            let mut moved = shown.clone();
            if let Some(m) = moved.mesh.as_mut() {
                m.points[20][1] += 5.0;
            }
            s.execute("edit.transform.warp", json!({"layer": id.0, "rect": moved.bounds, "warp": moved})).unwrap();
            assert_eq!(stored(&s).style, WarpStyle::Custom, "{scale}: edited");
        }
    }

    #[test]
    fn identity_and_bend_zero_are_no_ops_at_every_depth() {
        for depth in [8u64, 16, 32] {
            let mut s = session(depth);
            let before = active_surface(&s);
            let r = s.execute("edit.transform.warp", json!({"style": "custom"})).unwrap();
            assert_eq!(r["changed"], false);
            s.execute("edit.transform.warp", json!({"style": "flag", "bend": 0})).unwrap();
            // A custom identity mesh passed explicitly (forces a resample) stays within 1/255.
            let m = BezierMesh::identity([10.0, 10.0, 60.0, 40.0], 2, 2);
            let mut w = Warp::custom(m, [10.0, 10.0, 60.0, 40.0]);
            w.mesh.as_mut().unwrap().points[5][0] += 1e-12; // not exactly identity
            s.execute("edit.transform.warp", json!({"warp": w})).unwrap();
            let after = active_surface(&s);
            let r = Rect::new(10, 10, 60, 40);
            assert_eq!(after.content_bounds(), r, "{depth}");
            let worst = before.read_region(r).iter().zip(after.read_region(r)).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            assert!(worst <= 1.0 / 255.0, "{depth}: {worst}");
        }
    }

    #[test]
    fn arc_warps_pixels_and_undoes() {
        let mut s = session(8);
        let before = active_surface(&s);
        s.execute("edit.transform.warp", json!({"style": "arc", "bend": 50})).unwrap();
        let after = active_surface(&s);
        // As in Photoshop: the bottom edge keeps its corners, the top rises and spreads.
        let c = after.content_bounds();
        assert!(c.y1 <= 41 && c.y0 < 8 && c.x0 < 8 && c.x1 > 62, "{c:?}");
        s.undo();
        assert_eq!(active_surface(&s), before);
        assert!(s.execute("edit.transform.warp", json!({"style": "spiral"})).is_err());
        assert!(s.execute("edit.transform.warp", json!({})).is_err());
        assert!(s.execute("edit.transform.warp", json!({"mesh": {"us": [0, 1], "vs": [0, 1], "points": [[0, 0]]}})).is_err());
    }

    #[test]
    fn custom_mesh_moves_a_control_point() {
        let mut s = session(16);
        let mut m = BezierMesh::identity([10.0, 10.0, 60.0, 40.0], 1, 1);
        m.points[15] = [70.0, 50.0]; // bottom-right corner
        s.execute("edit.transform.warp", json!({"mesh": m})).unwrap();
        let b = active_surface(&s).content_bounds();
        assert!(b.x1 >= 69 && b.y1 >= 49, "{b:?}");
    }

    #[test]
    fn split_cuts_existing_sections_instead_of_replacing_the_grid() {
        let b = [0.0, 0.0, 90.0, 60.0];
        let plain = Warp::custom(BezierMesh::identity(b, 1, 1), b);
        let cut = split_warp(&plain, Some([b[0] + 0.25 * 90.0, 30.0]), Split::Vertical).unwrap();
        let m = cut.mesh.unwrap();
        for u in [0.0, 0.25, 1.0 / 3.0, 2.0 / 3.0, 1.0] {
            assert!(m.us.iter().any(|k| (k - u).abs() < 1e-6), "default guides stay, cut at 0.25: {:?}", m.us);
        }
        assert_eq!(m.vs, vec![0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0]);
        let grid = BezierMesh::identity(b, 3, 3);
        let cut = split_warp(&Warp::custom(grid.clone(), b), Some([45.0, 10.0]), Split::Vertical).unwrap();
        let m = cut.mesh.unwrap();
        assert_eq!(m.vs, grid.vs);
        assert_eq!(m.us.len(), grid.us.len() + 1);
        for u in [0.0, 1.0 / 3.0, 0.5, 2.0 / 3.0, 1.0] {
            assert!(m.us.iter().any(|k| (k - u).abs() < 1e-6), "{:?}", m.us);
        }
        let (nx0, nx1) = (grid.nx(), m.nx());
        for j in 0..grid.ny() {
            for i in 0..3 {
                assert_eq!(m.points[j * nx1 + i], grid.points[j * nx0 + i]);
            }
        }
    }

    #[test]
    fn split_commands_edit_meshes_exactly() {
        let mut s = session(8);
        let w = Warp::preset(WarpStyle::Bulge, 30.0, [10.0, 10.0, 60.0, 40.0]);
        let r = s.execute("edit.transform.splitWarpCrosswise", json!({"warp": w, "at": [30.0, 20.0]})).unwrap();
        let split: Warp = serde_json::from_value(r["warp"].clone()).unwrap();
        let m = split.mesh.as_ref().unwrap();
        assert_eq!((m.nx(), m.ny()), (7, 7));
        let r = s.execute("edit.transform.splitWarpVertically", json!({"warp": split})).unwrap();
        let v: Warp = serde_json::from_value(r["warp"].clone()).unwrap();
        assert_eq!(v.mesh.as_ref().unwrap().us.len(), 4);
        let r = s.execute("edit.transform.removeWarpSplit", json!({"warp": v, "at": [30.0, 20.0]})).unwrap();
        let back: Warp = serde_json::from_value(r["warp"].clone()).unwrap();
        assert!(back.mesh.as_ref().unwrap().us.len() + back.mesh.as_ref().unwrap().vs.len() < 7);
        // Splitting never changes the shape.
        for (x, y) in [(15.0, 15.0), (40.0, 33.0)] {
            let (a, b) = (w.map(x, y), split.map(x, y));
            assert!((a.0 - b.0).abs() < 0.6 && (a.1 - b.1).abs() < 0.6, "{a:?} {b:?}");
        }
    }

    #[test]
    fn smart_object_warp_is_lossless() {
        for depth in [8u64, 16, 32] {
            let mut s = session(depth);
            s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
            let original = active_surface(&s);
            s.execute("layer.smartObjects.warp", json!({"style": "wave", "bend": 60})).unwrap();
            let warped = active_surface(&s);
            assert_ne!(warped, original);
            let st = s.active().unwrap();
            let LayerContent::Smart(sm) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!() };
            assert!(sm.warp.is_some());
            // Removing the warp re-renders the source exactly (no accumulated resampling).
            s.execute("layer.smartObjects.warp", json!({"style": "none"})).unwrap();
            assert_eq!(active_surface(&s), original, "{depth}");
            // Warping twice equals warping once with the second warp.
            s.execute("layer.smartObjects.warp", json!({"style": "arc", "bend": 20})).unwrap();
            s.execute("layer.smartObjects.warp", json!({"style": "wave", "bend": 60})).unwrap();
            assert_eq!(active_surface(&s), warped, "{depth}");
            // Split on the stored warp keeps the appearance (within resampling of the mesh fit).
            s.execute("edit.transform.splitWarpCrosswise", json!({})).unwrap();
            let st = s.active().unwrap();
            let LayerContent::Smart(sm) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!() };
            assert_eq!(sm.warp.as_ref().unwrap().style, WarpStyle::Custom);
        }
    }

    #[test]
    fn crosswise_split_of_an_identity_custom_warp_is_four_by_four() {
        let mut s = session(8);
        let r = s.execute("edit.transform.splitWarpCrosswise", json!({"style": "custom", "rect": [0.0, 0.0, 90.0, 60.0]})).unwrap();
        let out: Warp = serde_json::from_value(r["warp"].clone()).unwrap();
        let m = out.mesh.unwrap();
        assert_eq!((m.us.len(), m.vs.len()), (5, 5), "thirds, then the centre cut: {:?}", (m.us.clone(), m.vs.clone()));
        for u in [0.0, 1.0 / 3.0, 0.5, 2.0 / 3.0, 1.0] {
            assert!(m.us.iter().any(|k| (k - u).abs() < 1e-6), "{:?}", m.us);
            assert!(m.vs.iter().any(|k| (k - u).abs() < 1e-6), "{:?}", m.vs);
        }
    }

    #[test]
    fn extreme_rect_is_bad_params_not_a_giant_warp() {
        let mut s = session(8);
        for cmd in ["edit.transform.warp", "layer.smartObjects.warp"] {
            if cmd == "layer.smartObjects.warp" {
                s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
            }
            let e = s.execute(cmd, json!({"style": "arc", "bend": 50, "rect": [-1e30, -1e30, 1e30, 1e30]})).unwrap_err();
            assert!(matches!(&e, EngineError::BadParams { msg, .. } if msg.contains("rect too large")), "{cmd}: {e}");
        }
        s.execute("layer.smartObjects.warp", json!({"style": "arc", "bend": 50, "rect": [10, 10, 60, 40]})).unwrap();
    }

    #[test]
    fn warp_grid_fits_the_surface_and_rejects_bad_sizes() {
        let mut s = session(8);
        assert!(s.execute("edit.transform.warpGrid", json!({})).is_err());
        assert!(s.execute("edit.transform.warpGrid", json!({"size": 0})).is_err());
        assert!(s.execute("edit.transform.warpGrid", json!({"size": -3})).is_err());
        assert!(s.execute("edit.transform.warpGrid", json!({"size": 65})).is_err());
        assert!(s.execute("edit.transform.warpGrid", json!({"size": 1.5})).is_err());
        assert!(s.execute("edit.transform.warpGrid", json!({"size": "custom"})).is_err());
        assert!(s.execute("edit.transform.warpGrid", json!({"size": "nope"})).is_err());
        assert!(s.execute("edit.transform.warpGrid", json!({"grid": [3, 3]})).is_err());
        let b = [10.0, 10.0, 60.0, 40.0];
        let mut bent = BezierMesh::identity(b, 1, 1);
        bent.points[0][0] += 12.0;
        let r = s.execute("edit.transform.warpGrid", json!({"warp": Warp::custom(bent.clone(), b), "size": 4})).unwrap();
        let out: Warp = serde_json::from_value(r["warp"].clone()).unwrap();
        let m = out.mesh.unwrap();
        assert_eq!(out.style, WarpStyle::Custom);
        assert_eq!(m.us, vec![0.0, 0.25, 0.5, 0.75, 1.0]);
        assert_eq!(m.vs, m.us);
        assert!((m.eval(0.0, 0.0)[0] - bent.eval(0.0, 0.0)[0]).abs() < 1e-3);
        assert!((m.eval(1.0, 1.0)[0] - b[2]).abs() < 1e-3);
        let r = s.execute("edit.transform.warpGrid", json!({"warp": Warp::custom(m, b), "size": "default"})).unwrap();
        let back: Warp = serde_json::from_value(r["warp"].clone()).unwrap();
        assert_eq!(back.mesh.unwrap().us.len(), 2);
    }

    /// A selection limits the warp of a linked mask too, as it does the pixels (#4175; Free
    /// Transform and Perspective Warp did this already, #3083): the moved part of the mask lines
    /// up with the moved pixels and the unselected part stays put.
    #[test]
    fn linked_mask_with_a_selection_warps_only_the_selected_region() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        s.edit("paint", |doc, active| {
            let l = doc.layer_mut(active.unwrap()).unwrap();
            l.surface_mut().unwrap().fill_rect(Rect::new(10, 10, 30, 20), &[1.0, 0.0, 0.0, 1.0]);
            // Hide the left quarter (selected) and the right quarter (not selected).
            let m = l.mask.as_mut().unwrap();
            m.surface.fill_rect(Rect::new(10, 10, 15, 20), &[0.0]);
            m.surface.fill_rect(Rect::new(25, 10, 30, 20), &[0.0]);
            m.linked = true;
            Ok(())
        })
        .unwrap();
        let mask_at = |s: &Session, x, y| {
            let st = s.active().unwrap();
            st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.sample_channel(x, y, 0)
        };
        let alpha_at = |s: &Session, x, y| {
            let st = s.active().unwrap();
            st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)[3]
        };
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
        // A mesh that translates the selected box 50 px down.
        let b = [10.0, 10.0, 20.0, 20.0];
        let mesh = BezierMesh::identity(b, 1, 1).map_points(|p| [p[0], p[1] + 50.0]);
        s.execute("edit.transform.warp", json!({"rect": [10, 10, 20, 20], "mesh": mesh})).unwrap();
        // Moved pixels and their mask line up.
        assert!(alpha_at(&s, 12, 65) > 0.99);
        assert!(mask_at(&s, 12, 65) < 0.01, "selected hidden quarter moved");
        assert!(mask_at(&s, 17, 65) > 0.99, "selected visible quarter moved");
        // The vacated area reads as revealed, with transparent pixels.
        assert!(alpha_at(&s, 12, 15) < 0.01);
        assert!(mask_at(&s, 12, 15) > 0.99);
        // The unselected part of the mask stays, aligned with the unselected pixels.
        assert!(alpha_at(&s, 27, 15) > 0.99);
        assert!(mask_at(&s, 27, 15) < 0.01, "unselected mask stays put");
        assert!(mask_at(&s, 27, 65) > 0.99, "and isn't copied along");
        for (x, y) in [(11, 61), (13, 68), (16, 61), (18, 68)] {
            let want = if x < 15 { 0.0 } else { 1.0 };
            assert!((mask_at(&s, x, y) - want).abs() < 0.01, "({x},{y}): {}", mask_at(&s, x, y));
        }
    }

    /// Esc stops the warp of a selected linked mask too, not only of the pixels. A fill layer has
    /// no pixels to warp, so the mask is the only part that can see the cancel.
    #[test]
    fn a_cancelled_warp_stops_in_a_selected_linked_mask() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
        s.execute("layer.newFillLayer.solidColor", json!({})).unwrap();
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        s.edit("link", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().mask.as_mut().unwrap().linked = true;
            Ok(())
        })
        .unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
        let st = s.active().unwrap();
        let (sel, mut l) = (st.doc.selection.clone().unwrap(), st.doc.layer(st.active_layer.unwrap()).unwrap().clone());
        let rect = Rect::new(10, 10, 20, 20);
        let w = Warp::preset(WarpStyle::Flag, 35.0, rect_f(rect));
        let ctl = Interrupt::cancel_only(&|| true);
        let r = warp_layer(Some(&sel), Locks::default(), &mut l, &w, rect, Interp::Bicubic, None, &ctl);
        assert!(matches!(r, Err(EngineError::Cancelled)), "{r:?}");
    }
}
