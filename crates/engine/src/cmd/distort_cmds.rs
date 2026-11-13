//! Filter › Liquify, Edit › Puppet Warp and Edit › Perspective Warp (plus their Layer › Smart
//! Objects twins).
//!
//! All three take their whole edit as data, so each is one replayable command and one history
//! step: Liquify the brush strokes (replayed onto a fresh displacement field,
//! `photocraft_algo::liquify`), Puppet Warp the pins (`photocraft_algo::puppet`, ARAP on a mesh
//! of the opaque region), Perspective Warp the planes (`photocraft_algo::perspective`). On a
//! smart object each becomes a smart filter (as in Photoshop): the params are stored with the
//! smart object and re-applied to a fresh render of the source, so editing never accumulates
//! resampling. The params are the data in every case (strokes, pins and planes are small
//! compared with a field or a mesh, and replay is deterministic), so nothing needs a blob.

use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use photocraft_algo::liquify::{LiquifyField, LiquifyStroke, apply_liquify, apply_liquify_with, auto_cell};
use photocraft_algo::perspective::{PerspectiveMap, Plane, Straighten, straighten};
use photocraft_algo::puppet::{PuppetDensity, PuppetMesh, PuppetMode, PuppetPin, PuppetWarp, puppet_warp, puppet_warp_with_mask_with, warp_with_mesh};
use photocraft_algo::transform::Interp;
use photocraft_algo::warp::{Fold, warp_mesh_gray, warp_mesh_surface, warp_mesh_surface_with};
use photocraft_color::PixelFormat;
use photocraft_doc::{Document, Layer, LayerContent, LayerId, Locks, SmartFilter};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::photo_cmds::Stopwatch;
use crate::{EngineError, Result, Session};

pub const LIQUIFY: &str = "filter.liquify";
pub const PUPPET: &str = "edit.puppetWarp";
pub const PUPPET_SMART: &str = "layer.smartObjects.puppetWarp";
pub const PERSPECTIVE: &str = "edit.perspectiveWarp";
pub const PERSPECTIVE_SMART: &str = "layer.smartObjects.perspectiveWarp";

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

// ---------- params ----------

/// Liquify params: the strokes and the field resolution (`meshSize`, px per field node).
pub fn liquify_params(cmd: &str, p: &Value, canvas: Rect) -> Result<(Vec<LiquifyStroke>, f64)> {
    let strokes: Vec<LiquifyStroke> = match p.get("strokes") {
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| bad(cmd, format!("bad `strokes`: {e}")))?,
        None => Vec::new(),
    };
    for s in &strokes {
        s.validate().map_err(|e| bad(cmd, e))?;
    }
    let cell = match p.get("meshSize") {
        None | Some(Value::Null) => auto_cell(canvas),
        Some(v) => {
            let c = v.as_f64().ok_or_else(|| bad(cmd, "`meshSize` is px per field node (1..64)"))?;
            if !(1.0..=64.0).contains(&c) {
                return Err(bad(cmd, "`meshSize` must be 1..64"));
            }
            c
        }
    };
    Ok((strokes, cell))
}

/// Puppet Warp params.
pub fn puppet_params(cmd: &str, p: &Value) -> Result<PuppetWarp> {
    let pins: Vec<PuppetPin> = match p.get("pins") {
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|e| bad(cmd, format!("bad `pins` (each {{\"src\":[x,y],\"dst\":[x,y],\"rotate\":deg?,\"depth\":n?}}): {e}")))?,
        None => Vec::new(),
    };
    if pins.iter().any(|q| q.src.iter().chain(&q.dst).any(|v| !v.is_finite()) || q.rotate.is_some_and(|r| !r.is_finite())) {
        return Err(bad(cmd, "pin coordinates must be finite"));
    }
    let mode = match p.get("mode").and_then(Value::as_str) {
        Some(m) => PuppetMode::parse(m).ok_or_else(|| bad(cmd, format!("unknown mode \"{m}\" (rigid|normal|distort)")))?,
        None => PuppetMode::Normal,
    };
    let density = match p.get("density").and_then(Value::as_str) {
        Some(d) => PuppetDensity::parse(d).ok_or_else(|| bad(cmd, format!("unknown density \"{d}\" (fewer|normal|more)")))?,
        None => PuppetDensity::Normal,
    };
    let expansion = p.get("expansion").and_then(Value::as_f64).unwrap_or(2.0);
    if !expansion.is_finite() || expansion.abs() > 200.0 {
        return Err(bad(cmd, "`expansion` must be -200..200 px"));
    }
    Ok(PuppetWarp { pins, mode, density, expansion })
}

/// The most vertices a stored Puppet Warp mesh may have (Photoshop's meshes have hundreds); the
/// PSD importer reads no more.
const MAX_STORED_VERTS: usize = 1 << 16;

/// A Puppet Warp's stored mesh, as a Photoshop file keeps it (`mesh`: `{"pins", "verts",
/// "deformed", "tris"}`, document pixels): the mesh solved for `pins`, drawn as it is instead of
/// solving again. `None` when there is none, it is malformed, or the filter was edited since: its
/// pins are no longer the ones it was solved for, or a solver setting is set (imported params
/// set none).
pub fn stored_puppet_mesh(p: &Value) -> Option<(PuppetMesh, Vec<[f64; 2]>)> {
    let m = p.get("mesh")?;
    if m.get("pins") != p.get("pins") || ["mode", "density", "expansion"].iter().any(|k| p.get(*k).is_some()) {
        return None;
    }
    let points = |k: &str| -> Option<Vec<[f64; 2]>> {
        let v = m.get(k)?.as_array()?;
        if v.len() > MAX_STORED_VERTS {
            return None;
        }
        v.iter().map(|q| serde_json::from_value::<[f64; 2]>(q.clone()).ok()).collect()
    };
    let (verts, deformed) = (points("verts")?, points("deformed")?);
    let tris = m.get("tris")?.as_array()?;
    if tris.len() > 2 * MAX_STORED_VERTS {
        return None;
    }
    let tris: Vec<[usize; 3]> = tris.iter().map(|t| serde_json::from_value(t.clone()).ok()).collect::<Option<_>>()?;
    Some((PuppetMesh { verts, tris, spacing: 0.0 }, deformed))
}

/// `params` of smart filter `command` on `sm`, without a stored Puppet Warp mesh made for another
/// placement than `sm`'s (`mesh.placement`, recorded on import): a mesh in document pixels doesn't
/// fit a smart object moved, transformed or warped since, so its pins are solved instead.
pub fn placed_puppet_params<'a>(command: &str, params: &'a Value, sm: &photocraft_doc::SmartObject) -> std::borrow::Cow<'a, Value> {
    let stale = command == PUPPET_SMART && params.get("mesh").is_some_and(|m| m.get("placement") != Some(&sm.placement_json()));
    if !stale {
        return std::borrow::Cow::Borrowed(params);
    }
    let mut p = params.clone();
    if let Some(o) = p.as_object_mut() {
        o.remove("mesh");
    }
    std::borrow::Cow::Owned(p)
}

/// Perspective Warp params (planes, after the optional straighten).
pub fn perspective_params(cmd: &str, p: &Value) -> Result<Vec<Plane>> {
    let mut planes: Vec<Plane> = match p.get("planes") {
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| bad(cmd, format!("bad `planes` (each {{\"src\":[4×[x,y]],\"dst\":[4×[x,y]]}}): {e}")))?,
        None => return Err(bad(cmd, "pass `planes`: [{\"src\":[[x,y]×4],\"dst\":[[x,y]×4]}]")),
    };
    if planes.is_empty() {
        return Err(bad(cmd, "`planes` is empty"));
    }
    if planes.iter().any(|q| q.src.iter().chain(&q.dst).flatten().any(|v| !v.is_finite())) {
        return Err(bad(cmd, "plane corners must be finite"));
    }
    if let Some(s) = p.get("straighten").and_then(Value::as_str) {
        let mode = Straighten::parse(s).ok_or_else(|| bad(cmd, format!("unknown straighten \"{s}\" (horizontal|vertical|auto)")))?;
        straighten(&mut planes, mode);
    }
    if PerspectiveMap::new(&planes).is_none() {
        return Err(bad(cmd, "a plane is degenerate (its corners must form a quad)"));
    }
    Ok(planes)
}

fn interp(p: &Value) -> Interp {
    Interp::parse(p.get("interpolation").and_then(Value::as_str).unwrap_or("bicubic"))
}

// ---------- pixel operations ----------

/// Rows [`mix_by_selection`] blends between cancel checks.
const MIX_BAND_ROWS: usize = 64;

/// Mixes `new` over `old` by the selection's coverage (premultiplied when there is alpha).
///
/// Where the selection covers nothing the result is `old`, so only the part of the content
/// bounds the selection reaches is blended into a copy of `old`: a small selection over distant
/// sparse content no longer expands the empty space between them. Refused, before any dense
/// buffer, when that part would still pass the working budget. Checks `ctl` once per band of
/// [`MIX_BAND_ROWS`] rows; `Err(Cancelled)` once it is cancelled (#210).
fn mix_by_selection(old: &Surface, new: &Surface, sel: &Surface, ctl: &Interrupt) -> Result<Surface> {
    let content = old.content_bounds().union(&new.content_bounds());
    // Outside its tiles the selection's coverage is its default: an empty default lets the
    // blend keep to the selection's bounds.
    let sel_default_empty = sel.default_pixel().first().is_none_or(|v| *v <= 0.0);
    let area = if sel_default_empty { content.intersect(&sel.content_bounds()) } else { content };
    let mut out = old.clone();
    if area.is_empty() {
        return Ok(out);
    }
    let fmt = old.format();
    let n = fmt.channels();
    crate::allocation::check_dense(area, (n as u64).saturating_mul(12), "blending Liquify through the selection")?;
    let (o, w) = (old.read_region(area), new.read_region(area));
    let mut r = vec![0.0f32; o.len()];
    let aw = area.width() as usize;
    let band = aw * n * MIX_BAND_ROWS;
    for (j, ((ob, nb), rb)) in o.chunks(band).zip(w.chunks(band)).zip(r.chunks_mut(band)).enumerate() {
        if ctl.cancelled() {
            return Err(EngineError::Cancelled);
        }
        let first = j * aw * MIX_BAND_ROWS;
        for (i, ((op, np), rp)) in ob.chunks_exact(n).zip(nb.chunks_exact(n)).zip(rb.chunks_exact_mut(n)).enumerate() {
            let i = first + i;
            let k = sel.sample_channel(area.x0 + (i % aw) as i32, area.y0 + (i / aw) as i32, 0).clamp(0.0, 1.0);
            if !fmt.alpha {
                for c in 0..n {
                    rp[c] = op[c] + (np[c] - op[c]) * k;
                }
                continue;
            }
            let a = n - 1;
            let oa = op[a] + (np[a] - op[a]) * k;
            rp[a] = oa;
            for c in 0..a {
                let pm = op[c] * op[a] + (np[c] * np[a] - op[c] * op[a]) * k;
                rp[c] = if oa > 0.0 { (pm / oa).clamp(0.0, 1.0) } else { 0.0 };
            }
        }
    }
    out.write_region(area, &r);
    out.prune();
    Ok(out)
}

/// Liquifies a surface over `canvas` (the field's extent).
pub fn liquify_surface(surf: &Surface, strokes: &[LiquifyStroke], cell: f64, canvas: Rect) -> Surface {
    let field = LiquifyField::from_strokes(canvas, cell, strokes);
    apply_liquify(surf, &field)
}

/// Puppet-warps a surface's content; `None` when the result would span more than
/// [`photocraft_algo::puppet::MAX_WARP_PIXELS`].
pub fn puppet_surface(surf: &Surface, w: &PuppetWarp, interp: Interp) -> Option<Surface> {
    let b = surf.content_bounds();
    if w.is_identity() || b.is_empty() {
        return Some(surf.clone());
    }
    puppet_warp(surf, b, w, interp)
}

/// Perspective-warps a surface's content.
pub fn perspective_surface(surf: &Surface, planes: &[Plane], interp: Interp) -> Surface {
    perspective_surface_with(surf, planes, interp, &Interrupt::NONE).unwrap_or_else(|| surf.clone())
}

/// [`perspective_surface`], checking `ctl` per destination tile; `None` once it is cancelled.
pub fn perspective_surface_with(surf: &Surface, planes: &[Plane], interp: Interp, ctl: &Interrupt) -> Option<Surface> {
    let b = surf.content_bounds();
    if PerspectiveMap::is_identity(planes) || b.is_empty() {
        return Some(surf.clone());
    }
    let Some(m) = PerspectiveMap::new(planes) else { return Some(surf.clone()) };
    warp_mesh_surface_with(surf, b, &|x, y| m.map(x, y), Fold::Rows, interp, ctl)
}

/// Smart-filter hook: re-applies a stored Liquify / Puppet / Perspective Warp to a rendered
/// smart object (`None` for other ids).
pub fn apply_to_surface(id: &str, params: &Value, surf: &Surface, canvas: Rect) -> Option<Surface> {
    match id {
        LIQUIFY => {
            let (strokes, cell) = liquify_params(id, params, canvas).ok()?;
            Some(liquify_surface(surf, &strokes, cell, canvas))
        }
        // A stored warp too far to render leaves the smart object as it is (#1019).
        PUPPET | PUPPET_SMART => {
            // A mesh stored with the filter (from a Photoshop file) is drawn as it is; one that
            // can't be drawn leaves the pins to solve.
            let b = surf.content_bounds();
            if let Some((mesh, deformed)) = stored_puppet_mesh(params).filter(|_| !b.is_empty())
                && let Some(out) = warp_with_mesh(surf, b, &mesh, &deformed, interp(params))
            {
                return Some(out);
            }
            Some(puppet_surface(surf, &puppet_params(id, params).ok()?, interp(params)).unwrap_or_else(|| surf.clone()))
        }
        PERSPECTIVE | PERSPECTIVE_SMART => Some(perspective_surface(surf, &perspective_params(id, params).ok()?, interp(params))),
        _ => None,
    }
}

// ---------- commands ----------

fn target(s: &Session, p: &Value) -> Result<LayerId> {
    crate::commands::layer_param(s, p)
}

fn pixel_layer(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    let id = d.active_layer.ok_or("no active layer")?;
    match &d.doc.layer(id).ok_or("no active layer")?.content {
        LayerContent::Raster(_) => Ok(()),
        LayerContent::Smart(sm) if sm.cache.is_some() => Ok(()),
        other => Err(format!("needs a pixel layer or smart object (active layer is {} {} layer)", other.article(), other.kind_name())),
    }
}

fn smart_layer(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    match d.active_layer.and_then(|id| d.doc.layer(id)).map(|l| &l.content) {
        Some(LayerContent::Smart(_)) => Ok(()),
        _ => Err("the active layer is not a smart object".into()),
    }
}

fn check_locks(l: &Layer, group: Locks, position: bool) -> Result<()> {
    let locks = l.locks.union(group);
    if locks.all || locks.pixels || (position && locks.position) {
        return Err(EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", l.name)));
    }
    Ok(())
}

/// Deforms a linked layer mask the same way, limited to the selection when there is one.
type MaskFn = Option<Box<dyn Fn(&Surface, Option<&Surface>, &Interrupt) -> Result<Surface> + Send>>;
/// Deforms a layer's pixels (with the selection and canvas); `None` when they would not change.
/// An `Err` leaves the layer as it was.
type LayerFn = Box<dyn Fn(&Surface, Option<&Surface>, Rect, &Interrupt) -> Result<Option<Surface>> + Send>;

/// Shared driver: a smart object records `cmd` as a smart filter; a pixel layer is edited in
/// place by `f(surface, selection)`. A background job when started with `Session::start` (#210):
/// the edit runs on a worker against a copy of the document, with progress, and can be
/// cancelled; its result applies as one undo step. When `f` changes nothing there is no step
/// (`changed: false`). `finish` completes the command's result.
#[allow(clippy::too_many_arguments)]
fn run_on_layer(
    s: &mut Session,
    cmd: &str,
    label: &str,
    p: &Value,
    moves: bool,
    f: LayerFn,
    mask: MaskFn,
    finish: impl FnOnce(Value) -> Value + Send + 'static,
) -> Result<Value> {
    let id = target(s, p)?;
    let colors = crate::color_channels::target(s, p)?.is_some();
    let mut params = p.clone();
    if let Value::Object(m) = &mut params {
        m.remove("layer");
    }
    let base = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let (cmd, name, step) = (cmd.to_string(), label.to_string(), label.to_string());
    // `jobs::edit_job` without its unconditional step: the worker's copy comes back only when
    // the layer changed (as `edit_job`, inline for `execute` and on the web).
    crate::jobs::run(
        s,
        label,
        true,
        move |ctx| {
            ctx.check()?;
            let mut doc = (*base).clone();
            let canvas = doc.bounds();
            let selection = doc.selection.clone();
            let group = crate::transform_cmds::group_locks(&doc, id);
            // A Background gains transparency when the distortion moves pixels (as in Photoshop).
            if moves && !colors {
                crate::extra_cmds::background_to_layer(&mut doc, id);
            }
            let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
            if let LayerContent::Smart(_) = l.content {
                // As filters do: a pixel or full lock refuses the smart filter (the position lock
                // doesn't, since the smart object itself stays put).
                check_locks(l, group, false)?;
                let sf = SmartFilter { command: cmd, params, blend: photocraft_color::BlendMode::Normal, opacity: 1.0, visible: true };
                ctx.stage(0.0, 1.0, &name, |_| crate::smart_cmds::add_smart_filter(&mut doc, id, sf, selection.as_ref()))?;
                return Ok(Some(doc));
            }
            check_locks(l, group, moves && !colors)?;
            let LayerContent::Raster(surf) = &mut l.content else {
                return Err(EngineError::Other(format!("{name} needs a pixel layer (rasterize it first)")));
            };
            let Some(out) = ctx.stage(0.0, 0.9, &name, |ctl| f(surf, selection.as_ref(), canvas, ctl))? else {
                return Ok(None);
            };
            ctx.check()?;
            // A linked layer mask follows a geometric warp, only where the pixels moved (#3083).
            let moved_mask = match (&mask, l.mask.as_ref()) {
                (Some(mf), Some(m)) if !colors && m.linked => Some(ctx.stage(0.9, 1.0, &name, |ctl| mf(&m.surface, selection.as_ref(), ctl))?),
                _ => None,
            };
            ctx.check()?;
            *surf = out;
            if let (Some(moved), Some(m)) = (moved_mask, l.mask.as_mut()) {
                m.surface = moved;
            }
            Ok(Some(doc))
        },
        move |s, doc: Option<Document>| {
            let changed = doc.is_some();
            if let Some(doc) = doc {
                s.edit(&step, move |d, _| {
                    *d = doc;
                    Ok(())
                })?;
            }
            Ok(finish(json!({"layer": id.0, "changed": changed})))
        },
    )
}

fn liquify(s: &mut Session, p: &Value) -> Result<Value> {
    let canvas = s.active().ok_or(EngineError::NoDocument)?.doc.bounds();
    let (strokes, cell) = liquify_params(LIQUIFY, p, canvas)?;
    let id = target(s, p)?;
    if strokes.is_empty() {
        return Ok(json!({"layer": id.0, "changed": false}));
    }
    let t0 = Stopwatch::start();
    // The field is built on the worker with the warp: replaying the strokes over a large canvas
    // takes a while. Its largest displacement comes back with the result (pixel layers; a smart
    // object's filter builds its own field when it renders).
    let found: Arc<OnceLock<f32>> = Arc::default();
    let report = found.clone();
    run_on_layer(
        s,
        LIQUIFY,
        "Liquify",
        p,
        false,
        Box::new(move |surf, sel, _, ctl| {
            let field = LiquifyField::from_strokes(canvas, cell, &strokes);
            let _ = report.set(field.max_displacement());
            if field.is_identity() {
                return Ok(None);
            }
            let out = apply_liquify_with(surf, &field, ctl).ok_or(EngineError::Cancelled)?;
            Ok(Some(match sel {
                Some(sel) => mix_by_selection(surf, &out, sel, ctl)?,
                None => out,
            }))
        }),
        None,
        move |mut r| {
            if let Some(max) = found.get() {
                r["maxDisplacement"] = json!(max);
            }
            r["ms"] = json!(t0.ms());
            r
        },
    )
}

/// Lifts the selected pixels (when there is a selection), deforms them with `f` and puts them
/// back over the rest; without a selection deforms the whole surface.
fn deform_selected(surf: &Surface, sel: Option<&Surface>, f: &dyn Fn(&Surface) -> Result<Surface>) -> Result<Surface> {
    match sel {
        Some(sel) => {
            let (lifted, mut rest) = crate::transform_cmds::split_selected(surf, sel)?;
            let moved = f(&lifted)?;
            crate::transform_cmds::composite_over(&mut rest, &moved);
            rest.prune();
            Ok(rest)
        }
        None => {
            let fmt = surf.format();
            let src = if fmt.alpha { surf.clone() } else { surf.convert(PixelFormat::new(fmt.mode, fmt.sample, true)) };
            f(&src)
        }
    }
}

fn puppet(s: &mut Session, p: &Value, cmd: &str) -> Result<Value> {
    let w = puppet_params(cmd, p)?;
    let id = target(s, p)?;
    let smart = matches!(s.active().and_then(|d| d.doc.layer(id)).map(|l| &l.content), Some(LayerContent::Smart(_)));
    if w.is_identity() && !smart {
        return Ok(json!({"layer": id.0, "changed": false}));
    }
    let it = interp(p);
    let name = cmd.to_string();
    // A linked layer mask moves with the pixels on the same mesh (#4910): the layer's warp
    // solves the mesh once and leaves the moved mask here for the mask step.
    let linked = s.active().and_then(|d| d.doc.layer(id)).and_then(|l| l.mask.as_ref()).filter(|m| m.linked).map(|m| m.surface.clone());
    let moved_mask: Arc<Mutex<Option<Surface>>> = Arc::default();
    let left = moved_mask.clone();
    let layer_fn: LayerFn = Box::new(move |surf, sel, _, ctl| {
        deform_selected(surf, sel, &|x| {
            let b = x.content_bounds();
            if b.is_empty() {
                return Ok(x.clone());
            }
            let too_far = || bad(&name, "the pins stretch the content too far: the result would cover more than 2^30 pixels");
            let (out, mask) = puppet_warp_with_mask_with(x, b, &w, it, linked.as_ref(), ctl).map_err(|_| EngineError::Cancelled)?.ok_or_else(too_far)?;
            *left.lock().unwrap_or_else(PoisonError::into_inner) = mask;
            Ok(out)
        })
        .map(Some)
    });
    let mask_fn: MaskFn = Some(Box::new(move |m, _, _| Ok(moved_mask.lock().unwrap_or_else(PoisonError::into_inner).take().unwrap_or_else(|| m.clone()))));
    run_on_layer(s, cmd, "Puppet Warp", p, true, layer_fn, mask_fn, |r| r)
}

fn perspective(s: &mut Session, p: &Value, cmd: &str) -> Result<Value> {
    let planes = perspective_params(cmd, p)?;
    let id = target(s, p)?;
    let smart = matches!(s.active().and_then(|d| d.doc.layer(id)).map(|l| &l.content), Some(LayerContent::Smart(_)));
    if PerspectiveMap::is_identity(&planes) && !smart {
        return Ok(json!({"layer": id.0, "changed": false, "planes": planes}));
    }
    let it = interp(p);
    let map = PerspectiveMap::new(&planes);
    let mask_fn: MaskFn = Some(Box::new(move |m, sel, _| {
        let Some(map) = &map else { return Ok(m.clone()) };
        let f = |x, y| map.map(x, y);
        let whole = |m: &Surface| Ok(warp_mesh_gray(m, &f, Interp::Bilinear));
        match sel {
            Some(sel) => crate::transform_cmds::move_gray_selected(m, sel, &|lifted, src| Ok(warp_mesh_surface(lifted, src, &f, Interp::Bilinear)), &whole),
            None => whole(m),
        }
    }));
    let warped = planes.clone();
    run_on_layer(
        s,
        cmd,
        "Perspective Warp",
        p,
        true,
        Box::new(move |surf, sel, _, ctl| {
            deform_selected(surf, sel, &|x| perspective_surface_with(x, &warped, it, ctl).ok_or(EngineError::Cancelled)).map(Some)
        }),
        mask_fn,
        move |mut r| {
            r["planes"] = json!(planes);
            r
        },
    )
}

pub fn specs() -> Vec<CommandSpec> {
    const L: &str = r##"{"strokes":[{"tool":"forwardWarp|reconstruct|smooth|twirlCw|twirlCcw|pucker|bloat|pushLeft|freeze|thaw|lassoMask|reconstructAll","size":px=100,"density":0-100=50,"pressure":0-100=100,"rate":0-100=80,"points":[[x,y,pressure?,ms?]…] (ms: milliseconds since the previous point; a point with it follows the pointer exactly and times the stationary tools in real time, as the dialog records them),"amount":%? (twirlCcw: twirlCw reversed, what an Alt-drag with Twirl Clockwise records; reconstructAll; lassoMask: 1 freezes the polygon in points, 0 thaws it)}…],"meshSize":px? (field resolution, px per node; default 2, or 4 above 4 MP),"layer":id?} — strokes replay in order on a fresh field; a selection limits the effect; on a smart object it becomes a smart filter"##;
    const P: &str = r##"{"pins":[{"src":[x,y],"dst":[x,y],"rotate":deg?,"depth":n?}…],"mode":"rigid|normal|distort"="normal","density":"fewer|normal|more"="normal","expansion":px=2,"interpolation":"bicubic|bilinear|nearest","layer":id?} — mesh over the opaque region, as-rigid-as-possible; on a smart object it becomes a smart filter"##;
    const Q: &str = r##"{"planes":[{"src":[[x,y]×4],"dst":[[x,y]×4]}…] (corners clockwise from top-left; corners that coincide in src are linked),"straighten":"horizontal|vertical|auto"?,"interpolation":"bicubic|bilinear|nearest","layer":id?}"##;
    vec![
        CommandSpec {
            id: LIQUIFY,
            label: "Liquify…",
            menu: &["Filter"],
            shortcut: Some("Cmd+Shift+X"),
            params: L,
            enabled: crate::filters::has_filterable_layer,
            journal: true,
            run: liquify,
        },
        CommandSpec {
            id: PUPPET,
            label: "Puppet Warp",
            menu: &["Edit"],
            shortcut: None,
            params: P,
            enabled: pixel_layer,
            journal: true,
            run: |s, p| puppet(s, p, PUPPET),
        },
        CommandSpec {
            id: PUPPET_SMART,
            label: "Puppet Warp",
            menu: &[],
            shortcut: None,
            params: P,
            enabled: smart_layer,
            journal: true,
            run: |s, p| puppet(s, p, PUPPET_SMART),
        },
        CommandSpec {
            id: PERSPECTIVE,
            label: "Perspective Warp",
            menu: &["Edit"],
            shortcut: None,
            params: Q,
            enabled: pixel_layer,
            journal: true,
            run: |s, p| perspective(s, p, PERSPECTIVE),
        },
        CommandSpec {
            id: PERSPECTIVE_SMART,
            label: "Perspective Warp",
            menu: &[],
            shortcut: None,
            params: Q,
            enabled: smart_layer,
            journal: true,
            run: |s, p| perspective(s, p, PERSPECTIVE_SMART),
        },
    ]
}

#[cfg(test)]
mod tests;
