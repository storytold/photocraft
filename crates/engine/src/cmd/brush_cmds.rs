//! Brush engine commands: the Brush/Pencil/Mixer Brush/Color Replacement tools, brush presets and
//! the session brush (`tools.setBrush`). The UI's options bar and Brush Settings panel call these.
//!
//! Brushes are [`BrushSettings`] serialised as camelCase JSON; any command taking a `"brush"`
//! object deep-merges it onto the session brush, so partial objects work
//! (`{"scattering": {"enabled": true}}`). Strokes are deterministic: the jitter seed is the `seed`
//! param or a hash of the points, so a journaled command replays to identical pixels.

use photocraft_doc::{
    Document, LayerContent, LayerId,
    mask_paint::{EffectKind, Stack},
};
use photocraft_geom::Rect;
#[cfg(not(target_arch = "wasm32"))]
use photocraft_geom::TileCoord;
use photocraft_paint::dynamics::DabGenerator;
use photocraft_paint::mixer::{MixerSettings, MixerState};
use photocraft_paint::replace::{Limits, ReplaceMode, ReplaceSettings, Sampling, apply_color_replacement, color_replacement_effect};
use photocraft_paint::tile::PREVIEW_SIDE;
use photocraft_paint::{
    BrushPreset, BrushSettings, Dab, GrayTile, Pattern, PresetCaptures, StoredTile, Stroke, StrokePoint, StrokeRenderer, TipShape, render_stroke,
};
use photocraft_raster::Surface;
#[cfg(not(target_arch = "wasm32"))]
use photocraft_raster::Tile;
use serde_json::{Value, json};
#[cfg(not(target_arch = "wasm32"))]
use std::collections::BTreeSet;
use std::{
    collections::{HashMap, VecDeque},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use crate::commands::{CommandSpec, has_paintable, is_mask_target, paint_surface};
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}
fn num(p: &Value, k: &str) -> Option<f32> {
    p.get(k).and_then(Value::as_f64).map(|v| v as f32)
}
fn flag(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn color(v: Option<&Value>, d: [f32; 4]) -> [f32; 4] {
    match v {
        Some(Value::Array(a)) if a.len() >= 3 => {
            let c: Vec<f32> = a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
            [c[0], c[1], c[2], c.get(3).copied().unwrap_or(1.0)]
        }
        Some(Value::String(h)) => {
            let h = h.trim_start_matches('#');
            let c = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|v| f32::from(v) / 255.0);
            match (c(0), c(2), c(4)) {
                (Some(r), Some(g), Some(b)) if h.len() == 6 || h.len() == 8 => [r, g, b, c(6).unwrap_or(1.0)],
                _ => d,
            }
        }
        _ => d,
    }
}
fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// Largest stroke coordinate accepted (a few times the largest document side, 300 000 px).
pub(crate) const MAX_COORD: f64 = 1_000_000.0;

/// Parse `points`: arrays `[x, y, pressure?, tiltX?, tiltY?, rotation?, timeMs?, wheel?]` or
/// objects `{"x":…, "y":…, "pressure":…, "tiltX":…, …, "time":…}`.
pub fn parse_points(p: &Value, cmd: &str) -> Result<Vec<StrokePoint>> {
    let arr = p.get("points").and_then(Value::as_array).ok_or_else(|| bad(cmd, "missing `points`"))?;
    let pts: Vec<StrokePoint> = arr
        .iter()
        .filter_map(|v| match v {
            Value::Array(a) => {
                let g = |i: usize| a.get(i).and_then(Value::as_f64);
                let mut sp = StrokePoint::new(g(0)?, g(1)?, g(2).unwrap_or(1.0) as f32);
                sp.tilt_x = g(3).unwrap_or(0.0) as f32;
                sp.tilt_y = g(4).unwrap_or(0.0) as f32;
                sp.rotation = g(5).unwrap_or(0.0) as f32;
                sp.time = g(6).unwrap_or(0.0);
                sp.wheel = g(7).unwrap_or(1.0) as f32;
                Some(sp)
            }
            Value::Object(_) => serde_json::from_value(v.clone()).ok(),
            _ => None,
        })
        .collect();
    if pts.is_empty() {
        return Err(bad(cmd, "`points` is empty"));
    }
    check_coords(&pts, cmd)?;
    Ok(pts)
}

/// Optional point indices where a single committed brush gesture starts a disconnected path.
/// Indices are boundaries into the flat `points` array; no synthetic connector point is added.
fn parse_breaks(p: &Value, points_len: usize, cmd: &str) -> Result<Vec<usize>> {
    let Some(values) = p.get("breaks") else { return Ok(Vec::new()) };
    let values = values.as_array().ok_or_else(|| bad(cmd, "`breaks` must be an array of point indices"))?;
    let mut breaks = Vec::with_capacity(values.len());
    for value in values {
        let index = value.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad(cmd, "`breaks` must contain nonnegative integer indices"))?;
        if index == 0 || index >= points_len || breaks.last().is_some_and(|last| index <= *last) {
            return Err(bad(cmd, "`breaks` must be strictly increasing indices inside `points`"));
        }
        breaks.push(index);
    }
    Ok(breaks)
}

/// A stroke runs dab by dab along its length: an absurd coordinate would mean billions of dabs.
/// Shared by the painting and the retouch tools' `points` (#976).
pub(crate) fn check_coords(pts: &[StrokePoint], cmd: &str) -> Result<()> {
    if pts.iter().any(|q| !(q.x.abs() <= MAX_COORD && q.y.abs() <= MAX_COORD)) {
        return Err(bad(cmd, format!("point coordinates must be finite and within ±{MAX_COORD}")));
    }
    Ok(())
}

/// Deep-merge `patch` into `base` (objects merge key by key; anything else replaces).
fn merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        // A different variant of an externally tagged enum (`{"tile":…}` → `{"procedural":…}`)
        // replaces the old one instead of merging into a two-variant object.
        (Value::Object(b), Value::Object(p)) if b.len() == 1 && p.len() == 1 && b.keys().next() != p.keys().next() => {
            *b = p.clone();
        }
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                match b.get_mut(k) {
                    Some(bv) if bv.is_object() && v.is_object() => merge(bv, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, p) => *b = p.clone(),
    }
}

/// Apply a JSON brush patch onto a brush.
pub fn merge_brush(base: &BrushSettings, patch: &Value, cmd: &str) -> Result<BrushSettings> {
    // Bitmaps the patch doesn't touch (sampled tips, pattern tiles: up to megabytes of base64) skip
    // the JSON round trip, so a Brush Settings slider on an imported tip stays cheap.
    let touches = |path: &[&str]| {
        let mut v = patch;
        for k in path {
            match v.get(k) {
                Some(x) => v = x,
                // A non-object on the way replaces the whole section.
                None => return !v.is_object(),
            }
        }
        true
    };
    let mut light = base.clone();
    let tip = (!touches(&["tip"])).then(|| std::mem::take(&mut light.tip));
    let dual_tip = (!touches(&["dualBrush", "tip"])).then(|| std::mem::take(&mut light.dual_brush.tip));
    let pattern = (!touches(&["texture", "pattern"])).then(|| std::mem::take(&mut light.texture.pattern));
    let mut v = serde_json::to_value(&light).map_err(|e| bad(cmd, e.to_string()))?;
    merge(&mut v, patch);
    // A legacy command sets both axes. The base JSON already has the new keys,
    // so copy the alias into axes the patch didn't set before deserialising it.
    if let Some(pose_patch) = patch.get("pose").and_then(Value::as_object)
        && let Some(legacy) = pose_patch.get("overrideTilt")
        && let Some(pose) = v.get_mut("pose").and_then(Value::as_object_mut)
    {
        for axis in ["overrideTiltX", "overrideTiltY"] {
            if !pose_patch.contains_key(axis) {
                pose.insert(axis.to_string(), legacy.clone());
            }
        }
    }
    let mut out: BrushSettings = serde_json::from_value(v).map_err(|e| bad(cmd, format!("invalid brush: {e}")))?;
    if let Some(t) = tip {
        out.tip = t;
    }
    if let Some(t) = dual_tip {
        out.dual_brush.tip = t;
    }
    if let Some(p) = pattern {
        out.texture.pattern = p;
    }
    if patch.get("size").is_some() && !touches(&["dualBrush", "size"]) {
        let new_size = out.size;
        out.size = base.size;
        out.resize_with_dual_brush(new_size);
    }
    Ok(out)
}

/// Rejects brush sizes and Scatter amounts outside their documented ranges, before a brush is
/// stored or painted with.
pub(crate) fn validate_brush(brush: &BrushSettings, cmd: &str) -> Result<()> {
    let max = photocraft_paint::MAX_BRUSH_SIZE;
    if !brush.size.is_finite() || brush.size > max {
        return Err(bad(cmd, format!("brush size must be finite and at most {max} px")));
    }
    if brush.dual_brush.enabled && (!brush.dual_brush.size.is_finite() || brush.dual_brush.size > max) {
        return Err(bad(cmd, format!("dual brush size must be finite and at most {max} px")));
    }
    // Scatter may exceed 1 (100 %) but not the 1000 % maximum: a huge amount pushed dab centres to
    // infinity (#977). `{:?}` keeps 1e38 short and shows infinity as `inf`.
    let scatter_max = photocraft_paint::MAX_SCATTER;
    let in_range = |v: f32| (0.0..=scatter_max).contains(&v);
    let sc = brush.scattering.scatter.jitter;
    if brush.scattering.enabled && !in_range(sc) {
        return Err(bad(cmd, format!("scatter amount {sc:?} is out of range: it must be between 0 and {scatter_max} (1000 %)")));
    }
    let ds = brush.dual_brush.scatter;
    if brush.dual_brush.enabled && !in_range(ds) {
        return Err(bad(cmd, format!("dual brush scatter amount {ds:?} is out of range: it must be between 0 and {scatter_max} (1000 %)")));
    }
    Ok(())
}

fn find_preset<'a>(s: &'a Session, name: &str, cmd: &str) -> Result<&'a BrushPreset> {
    photocraft_paint::presets::find(&s.tools.presets, name).ok_or_else(|| bad(cmd, format!("no brush preset named `{name}`")))
}

/// Resolve the brush for a stroke command: session brush ← `preset` ← `brush` ← legacy scalar
/// params; colours from the session; seed from `seed` or the points.
pub fn resolve_brush(s: &Session, p: &Value, cmd: &str) -> Result<BrushSettings> {
    let mut b = s.tools.brush.clone();
    if let Some(name) = p.get("preset").and_then(Value::as_str) {
        b = find_preset(s, name, cmd)?.picked_over(&s.tools.brush);
    }
    if let Some(patch) = p.get("brush").filter(|v| v.is_object()) {
        b = merge_brush(&b, patch, cmd)?;
    }
    // A library preset's tips stay on disk until a stroke uses them (#1843).
    s.load_brush_tips(&mut b).map_err(|e| bad(cmd, e))?;
    if let Some(v) = num(p, "size") {
        b.resize_with_dual_brush(v.max(0.5));
    }
    if let Some(v) = num(p, "hardness") {
        b.hardness = v.clamp(0.0, 1.0);
    }
    if let Some(v) = num(p, "opacity") {
        b.opacity = v.clamp(0.0, 1.0);
    }
    if let Some(v) = num(p, "flow") {
        b.flow = v.clamp(0.0, 1.0);
    }
    if let Some(v) = num(p, "spacing") {
        b.spacing = v.clamp(0.01, 10.0);
    }
    if let Some(v) = num(p, "smoothing") {
        b.smoothing.enabled = true;
        b.smoothing.amount = v.clamp(0.0, 1.0);
    }
    b.color = color(p.get("color"), s.tools.foreground);
    b.color_cmyk = s.tools.picked_cmyk([b.color[0], b.color[1], b.color[2]]).map(|k| ([b.color[0], b.color[1], b.color[2]], k));
    b.background = s.tools.background;
    b.erase = flag(p, "erase", false);
    b.seed = match p.get("seed").and_then(Value::as_u64) {
        Some(v) => v,
        None => photocraft_paint::rng::seed_from_bytes(p.get("points").map(|v| v.to_string()).unwrap_or_default().as_bytes()),
    };
    validate_brush(&b, cmd)?;
    Ok(b)
}

fn layer_id(s: &Session, p: &Value) -> Result<photocraft_doc::LayerId> {
    match p.get("layer").and_then(Value::as_u64) {
        Some(id) => Ok(photocraft_doc::LayerId(id)),
        None => s.active().and_then(|d| d.active_layer).ok_or(EngineError::Other("no active layer".into())),
    }
}

fn damage_json(s: &mut Session, dmg: Rect) -> Value {
    s.hint_damage(dmg);
    json!({ "damage": [dmg.x0, dmg.y0, dmg.width(), dmg.height()] })
}

/// The target layer (`None` for a channel), brush and zoom of a stroke. On a mask or channel the
/// eraser paints the background colour (Photoshop).
pub(crate) fn stroke_target(s: &Session, p: &Value, brush: BrushSettings) -> Result<(Option<photocraft_doc::LayerId>, BrushSettings, f32)> {
    let gray = is_mask_target(p) || crate::channel_cmds::is_channel_target(p);
    let id = if crate::channel_cmds::is_channel_target(p) { None } else { Some(layer_id(s, p)?) };
    let stack =
        id.and_then(|id| s.active().and_then(|st| st.doc.layer(id))).and_then(|l| l.mask.as_ref()).is_some_and(|m| m.paint.is_some()) && is_mask_target(p);
    let brush = if gray && brush.erase && !stack { BrushSettings { erase: false, color: s.tools.background, ..brush } } else { brush };
    let mut brush = brush;
    if stack {
        let mut gray = [0.0];
        photocraft_raster::from_rgba_into(&photocraft_color::PixelFormat::GRAY8, brush.color, &mut gray);
        let value = gray.first().copied().unwrap_or(0.0);
        brush.color = [value, value, value, brush.color[3]];
    }
    Ok((id, brush, num(p, "zoom").unwrap_or(1.0)))
}

/// Erasing a layer with locked transparency (e.g. the Background) can't remove opacity, so it
/// paints the background colour instead (Photoshop).
pub(crate) fn erase_locked(brush: &mut BrushSettings, lock: bool, bg: [f32; 4]) {
    if brush.erase && lock {
        brush.erase = false;
        brush.color = bg;
    }
}

/// The document's own CMYK profile (absent, built-in or not a CMYK document: `None`, the
/// default conversions), which the stroke's colour conversions must honour — the compositors
/// and composite exports enter the same scope.
pub(crate) fn doc_cmyk_space(s: &Session) -> Option<std::sync::Arc<photocraft_color::convert::CmykSpace>> {
    photocraft_color::convert::CmykSpace::for_profile(s.active().and_then(|d| d.doc.icc_profile.as_ref()))
}

/// Stroke with a resolved brush onto the target layer (pixels or mask).
fn stroke_with(s: &mut Session, cmd: &str, p: &Value, label: &str, brush: BrushSettings, pts: Vec<StrokePoint>, auto_erase: bool) -> Result<Value> {
    let bg = s.tools.background;
    let fg = brush.color;
    let symmetry = s.active().and_then(|st| st.symmetry.clone());
    let (id, brush, zoom) = stroke_target(s, p, brush)?;
    let breaks = parse_breaks(p, pts.len(), cmd)?;
    let cmyk = doc_cmyk_space(s);
    // The live stroke the canvas already rendered for exactly these params: take its pixels
    // instead of rendering the whole stroke again (the hitch at pen-up).
    let live = s.live_commit.take().filter(|l| l.matches(s, cmd, p, &pts));
    if let Some(live) = &live {
        let st = s.active().ok_or(EngineError::NoDocument)?;
        if crate::channel_cmds::read_target_surface(&st.doc, id, p)? == &live.surface {
            let mut result = damage_json(s, Rect::EMPTY);
            result["fromLive"] = json!(true);
            return Ok(result);
        }
    }
    let from_live = live.is_some();
    let dmg = photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
        s.edit(label, |doc, _| {
            if let Some(l) = live {
                let (surf, _) = crate::channel_cmds::target_surface(doc, id, p)?;
                *surf = l.surface;
                return Ok(l.bounds);
            }
            let sel = doc.selection.clone();
            let table = crate::mode_cmds::paint_color_table(doc, p);
            let (surf, lock) = crate::channel_cmds::target_surface(doc, id, p)?;
            let mut brush = brush;
            erase_locked(&mut brush, lock, bg);
            if auto_erase {
                // A stroke in the table's colour starts on what the foreground painted as.
                let fg = table.as_ref().map_or(fg, |t| crate::mode_cmds::table_color(t, fg));
                apply_auto_erase(&mut brush, surf, pts.first(), fg, bg);
            }
            let unpainted = table.as_ref().map(|_| surf.clone());
            let mut damage = Rect::EMPTY;
            let mut start = 0;
            for end in breaks.iter().copied().chain(std::iter::once(pts.len())) {
                let segment = &pts[start..end];
                if let Some(axis) = &symmetry {
                    let reflected = axis.reflected_passes(segment);
                    let copies: Vec<_> = reflected
                        .iter()
                        .filter(|p| crate::symmetry_cmds::SymmetryAxis::has_distinct_mirror(segment, p))
                        .map(|p| {
                            let mut renderer = StrokeRenderer::new(&brush, Some(surf.format()), zoom);
                            renderer.push(p);
                            renderer.finish();
                            renderer
                        })
                        .collect();
                    if copies.is_empty() {
                        damage = damage.union(&render_stroke(surf, &brush, segment, sel.as_ref(), lock, zoom));
                    } else {
                        let pre = surf.clone();
                        let mut original = StrokeRenderer::new(&brush, Some(surf.format()), zoom);
                        original.push(segment);
                        original.finish();
                        damage = damage.union(&original.composite_union_many(&copies, &pre, surf, sel.as_ref(), lock));
                    }
                } else {
                    damage = damage.union(&render_stroke(surf, &brush, segment, sel.as_ref(), lock, zoom));
                }
                start = end;
            }
            if let (Some(table), Some(pre)) = (&table, &unpainted) {
                crate::mode_cmds::keep_in_color_table(table, pre, surf, damage);
            }
            Ok(damage)
        })
    })?;
    let mut r = damage_json(s, dmg);
    if from_live {
        r["fromLive"] = json!(true);
    }
    Ok(r)
}

/// Pencil Auto Erase: a stroke that starts on a pixel of the foreground colour paints the
/// background colour instead (Photoshop).
fn apply_auto_erase(brush: &mut BrushSettings, surf: &Surface, start: Option<&StrokePoint>, fg: [f32; 4], bg: [f32; 4]) {
    let Some(p0) = start else { return };
    let c = surf.rgba(p0.x.floor() as i32, p0.y.floor() as i32);
    if c[3] > 0.0 && (0..3).all(|i| (c[i] - fg[i]).abs() < 1.5 / 255.0) {
        brush.color = bg;
    }
}

/// The Pencil's brush: the session brush made aliased (every pixel fully painted or untouched,
/// dabs on the pixel grid), hard and at full flow, with the options-bar mode.
pub(crate) fn pencil_brush(s: &Session, p: &Value) -> Result<BrushSettings> {
    let mut brush = with_blend_mode(resolve_brush(s, p, "paint.pencil")?, p);
    if flag(p, "block", false) {
        return block_brush(brush, p);
    }
    brush.aliased = true;
    if num(p, "hardness").is_none() {
        brush.hardness = 1.0;
    }
    if num(p, "flow").is_none() {
        brush.flow = 1.0;
    }
    Ok(brush)
}

/// Edge of the Eraser's Block in screen pixels (Photoshop): constant on screen, so it covers
/// `BLOCK_SCREEN_PX / zoom` document pixels.
pub const BLOCK_SCREEN_PX: f64 = 16.0;

/// The Eraser's Block edge in document pixels at view `zoom`: `16 / zoom`, rounded to whole
/// pixels (a zoom that is not a positive number counts as 100 %).
pub fn block_size(zoom: f64) -> f32 {
    let zoom = if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 };
    (BLOCK_SCREEN_PX / zoom).round().clamp(1.0, f64::from(photocraft_paint::brush::MAX_BRUSH_SIZE)) as f32
}

/// The Eraser's Block mode (`"block": true`): a hard, aliased square of
/// [`BLOCK_SCREEN_PX`] screen pixels at the stroke's `"zoom"`, i.e. `16 / zoom` document
/// pixels rounded to whole pixels. Size, opacity, flow, hardness and the brush's dynamics don't
/// apply (Photoshop); colour, erase, smoothing and the seed are the resolved brush's.
fn block_brush(b: BrushSettings, p: &Value) -> Result<BrushSettings> {
    let zoom = match p.get("zoom") {
        None => 1.0,
        Some(v) => v.as_f64().filter(|z| z.is_finite() && *z > 0.0).ok_or_else(|| bad("paint.pencil", "`zoom` must be a positive number"))?,
    };
    Ok(BrushSettings {
        size: block_size(zoom),
        aliased: true,
        square: true,
        pressure_size: false,
        pressure_opacity: false,
        spacing: 0.25,
        color: b.color,
        color_cmyk: b.color_cmyk,
        background: b.background,
        erase: b.erase,
        smoothing: b.smoothing,
        seed: b.seed,
        ..BrushSettings::default()
    })
}

/// Is the stroke in Photoshop's Clear painting mode (`"mode": "clear"`, #3439)? It erases with the
/// Brush's or Pencil's own brush; unlike a blend mode it is not a layer mode, so it is not a
/// [`photocraft_color::BlendMode`].
fn clear_mode(p: &Value) -> bool {
    p.get("mode").and_then(Value::as_str).is_some_and(|m| m.eq_ignore_ascii_case("clear"))
}

/// Applies the options-bar blend `mode` to a brush. `"mode"` accepts any blend-mode name
/// (normal|multiply|screen|…), or `behind` to paint beneath existing pixels, or `clear` to erase with the brush. The Eraser has no blend mode in
/// Photoshop, so it is forced to Normal.
pub(crate) fn with_blend_mode(mut b: BrushSettings, p: &Value) -> BrushSettings {
    b.erase |= clear_mode(p);
    if b.erase {
        b.mode = photocraft_color::BlendMode::Normal;
        b.behind = false;
    } else if p.get("mode").and_then(Value::as_str).is_some_and(|m| m.eq_ignore_ascii_case("behind")) {
        b.mode = photocraft_color::BlendMode::Normal;
        b.behind = true;
    } else if let Some(m) = p.get("mode").and_then(Value::as_str).and_then(crate::commands::blend_from_str) {
        b.mode = m;
        b.behind = false;
    }
    b
}

/// `paint.stroke`: the Brush (and Eraser) tool.
pub fn paint_stroke(s: &mut Session, p: &Value) -> Result<Value> {
    let pts = parse_points(p, "paint.stroke")?;
    let brush = with_blend_mode(resolve_brush(s, p, "paint.stroke")?, p);
    let label = if brush.erase && !clear_mode(p) { "Eraser" } else { "Brush Tool" };
    if flag(p, "fill", false) {
        return fill_region(s, p, label, brush, pts);
    }
    stroke_with(s, "paint.stroke", p, label, brush, pts, false)
}

/// `paint.stroke {fill: true}`: fill the region enclosed by `points` (their polygon) with the
/// brush's colour and opacity, its edge anti-aliased and softened by the brush hardness, instead of
/// stamping the outline. A closed gesture then paints a filled dab, not a ring (#3659).
fn fill_region(s: &mut Session, p: &Value, label: &str, brush: BrushSettings, pts: Vec<StrokePoint>) -> Result<Value> {
    if clear_mode(p) {
        // `clear` maps onto the eraser; a clear fill would erase the region the outline encloses,
        // which a selection + delete does deliberately (#3659). Reject before the eraser check so
        // the message names the real conflict.
        return Err(bad("paint.stroke", "\"fill\" cannot be combined with \"mode\":\"clear\"; select the enclosed region and delete it instead"));
    }
    if brush.erase {
        return Err(bad("paint.stroke", "\"fill\" does not support the Eraser"));
    }
    if pts.len() < 3 {
        return Err(bad("paint.stroke", "\"fill\" needs at least three points (a closed region)"));
    }
    let (id, brush, _zoom) = stroke_target(s, p, brush)?;
    // A hard brush (hardness 1) keeps the anti-aliased polygon edge; a soft one feathers it by half
    // the brush width, the way its tip fades.
    let feather_px = ((1.0 - brush.hardness.clamp(0.0, 1.0)) * brush.size * 0.5).max(0.0);
    let (color, opacity, mode) = (brush.color, brush.opacity.clamp(0.0, 1.0), brush.mode);
    // A colour picked as CMYK fills with its inks in a CMYK target where it covers fully, as
    // Edit › Fill does (blend_color_mask blends partial coverage through the colour's RGB).
    let picked = brush.color_cmyk.filter(|(rgb, _)| *rgb == [color[0], color[1], color[2]]).map(|(_, inks)| inks);
    let cmyk = doc_cmyk_space(s);
    let dmg = photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
        s.edit(label, |doc, _| {
            let sel = doc.selection.clone();
            let canvas = doc.bounds();
            let (surf, lock) = crate::channel_cmds::target_surface(doc, id, p)?;
            // The polygon's whole-pixel bounds, grown by the feather's reach (3σ) so a soft edge
            // fades out instead of being clipped.
            let (mut bx0, mut by0, mut bx1, mut by1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
            for sp in &pts {
                if !(sp.x.is_finite() && sp.y.is_finite()) {
                    return Err(bad("paint.stroke", "non-finite point in a fill"));
                }
                bx0 = bx0.min(sp.x);
                by0 = by0.min(sp.y);
                bx1 = bx1.max(sp.x);
                by1 = by1.max(sp.y);
            }
            let ix = |v: f64| v.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
            let reach = photocraft_algo::selection::feather_reach(feather_px);
            let pad = i32::try_from(reach).unwrap_or(i32::MAX).saturating_add(2);
            let bounds = Rect::new(
                ix(bx0.floor()).saturating_sub(pad),
                ix(by0.floor()).saturating_sub(pad),
                ix(bx1.ceil()).saturating_add(pad),
                ix(by1.ceil()).saturating_add(pad),
            )
            .intersect(&canvas);
            if bounds.is_empty() {
                return Ok(Rect::new(0, 0, 0, 0));
            }
            let (w, h) = (bounds.width() as usize, bounds.height() as usize);
            // The coverage, its feathered copy (and the blur's scratch), the selection read and
            // the mask are each a buffer over the whole region, which can be the whole canvas:
            // reserve that peak first, so a region too big for memory is an error, not an abort.
            const FILL: &str = "filling a painted region";
            let peak = w.checked_mul(h).and_then(|n| n.checked_mul(5)).ok_or_else(|| crate::allocation::error(FILL, "requested size overflowed"))?;
            drop(crate::allocation::capacity::<f32>(peak, FILL)?);
            let xy: Vec<(f32, f32)> = pts.iter().map(|sp| (sp.x as f32, sp.y as f32)).collect();
            let mut cov = photocraft_algo::selection::polygon(&xy, bounds, true);
            if feather_px > 0.01 {
                cov = photocraft_algo::selection::feather(&cov, w, h, feather_px);
            }
            if let Some(sel) = sel.as_ref() {
                let sv = crate::allocation::read_region(sel, bounds, FILL)?;
                for (c, s) in cov.iter_mut().zip(&sv) {
                    *c *= s.clamp(0.0, 1.0);
                }
            }
            let coverage = photocraft_algo::selection::mask_to_surface(&cov, bounds);
            crate::fill_cmds::blend_color_mask(surf, bounds, color, picked, &coverage, mode, opacity, lock)?;
            Ok(bounds)
        })
    })?;
    Ok(damage_json(s, dmg))
}

/// A `paint.stroke` rendered while it is drawn, onto a copy of the active document, so the canvas
/// shows the real dabs before the stroke commits. Committing the same params and points with
/// `"seed": live.seed` gives the same pixels.
pub struct LiveStroke {
    /// The active document with the stroke so far.
    pub doc: std::sync::Arc<photocraft_doc::Document>,
    /// Jitter seed to pass to `paint.stroke`.
    pub seed: u64,
    cmd: String,
    renderer: StrokeRenderer,
    /// Exact DabGenerator used by the opt-in native mask GPU preview. CPU rendering remains the
    /// default; the GPU path retains every original point and can replay them through renderer if
    /// GPU setup, dab upload, or release readback fails.
    native_mask_gpu_generator: Option<DabGenerator>,
    native_mask_gpu_dabs: Vec<NativeMaskGpuDab>,
    native_mask_gpu_damage: Rect,
    native_mask_gpu_patch_applied: bool,
    brush: BrushSettings,
    zoom: f32,
    symmetry: Option<crate::symmetry_cmds::PaintingSymmetry>,
    mirrors: Vec<StrokeRenderer>,
    mirror_distinct: Vec<bool>,
    /// What the symmetric preview drew last, once a mirror differs from the stroke, and how many
    /// passes it merged (a mirror joining redraws every tile).
    union_live: Option<(photocraft_paint::UnionLive, usize)>,
    pre: Surface,
    sel: Option<Surface>,
    lock: bool,
    layer: Option<photocraft_doc::LayerId>,
    params: Value,
    /// Where the doc shows the stroke's end as finishing it would draw it (see `push`).
    tail: Rect,
    finished_bounds: Rect,
    /// The document's CMYK profile the commit enters (`stroke_with`), so the preview shows the
    /// pixels a `paint.stroke` with `seed` will write.
    cmyk: Option<std::sync::Arc<photocraft_color::convert::CmykSpace>>,
    /// Every point rendered so far, in order.
    points: Vec<StrokePoint>,
    /// Point indices where a new disconnected renderer path starts.
    breaks: Vec<usize>,
    segment_break_pending: bool,
    /// The document and revision the stroke started on.
    base: (photocraft_doc::DocId, u64),
    /// Unquantized prefixes below the active mask Paint layer, kept only for this live stroke.
    mask_prefix: Option<MaskPrefixCache>,
    /// UV shell ownership used only while previewing a bound mask stack with Blur.
    mask_shell_ids: Option<std::sync::Arc<Vec<u32>>>,
    /// Optional cadence for materializing the flattened multi-effect mask preview during a live stroke.
    mask_preview_cadence: Option<Duration>,
    /// Damage accumulated since the last flattened mask preview refresh.
    mask_preview_pending: Vec<Rect>,
    /// Conservative extent of all flattened-cache invalidation caused by this live stroke.
    /// The final command uses it to prevent its source-dab hint from hiding downstream Blur
    /// pixels that the canonical Session edit materializes at commit.
    mask_preview_commit_damage: Rect,
    /// Start of the current mask preview cadence interval.
    mask_preview_last_refresh: Option<Instant>,
    /// Most recent worker dispatch, independent of result publication. Without this clock,
    /// every input can replace an in-flight job before the first Blur preview is applied.
    mask_preview_last_submit: Option<Instant>,
    /// Latest input generation whose flattened preview still needs to be published.
    mask_preview_generation: u64,
    /// Latest generation submitted to the optional worker.
    mask_preview_submitted: u64,
    /// A worker result for this generation exceeded the bounded compose path; defer exact sync
    /// fallback until flush/release instead of stalling a pointer-move poll.
    mask_preview_unsupported: Option<u64>,
    /// Native-only, opt-in mask-stack cache compositor. Stroke samples are still rendered on the caller.
    #[cfg(not(target_arch = "wasm32"))]
    mask_preview_worker: Option<MaskPreviewWorker>,
    /// Dirty active Paint tiles since the initial worker snapshot. Jobs carry only shared tile
    /// Arcs for this cumulative set, so replaced/coalesced jobs still reconstruct the latest source.
    #[cfg(not(target_arch = "wasm32"))]
    mask_preview_patch_tiles: BTreeSet<TileCoord>,
    /// Once a worker job with the initial full stack has been queued, later jobs patch it.
    #[cfg(not(target_arch = "wasm32"))]
    mask_preview_snapshot_sent: bool,
    /// Too many dirty tiles for a bounded incremental worker packet; leave preview stale until the
    /// canonical Session commit materializes the exact final cache.
    #[cfg(not(target_arch = "wasm32"))]
    mask_preview_patch_overflow: bool,
    /// Last worker job timings/counters, retained for optional UI diagnostics.
    #[cfg(not(target_arch = "wasm32"))]
    mask_preview_last_metrics: Option<MaskPreviewMetrics>,
    #[cfg(not(target_arch = "wasm32"))]
    mask_preview_snapshot_ms_total: f64,
    #[cfg(not(target_arch = "wasm32"))]
    mask_preview_patch_collect_ms_total: f64,
    #[cfg(not(target_arch = "wasm32"))]
    mask_preview_stack_snapshots: usize,
}

/// The round-dab subset shared with the native GPU mask preview. Values come directly from the
/// CPU DabGenerator; this type does not interpolate or approximate input points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeMaskGpuDab {
    pub center: [f32; 2],
    pub radius: f32,
    pub alpha: f32,
    pub opacity: f32,
}

/// A dirty final Paint-surface patch read back once when the native GPU preview is committed.
/// `pixels_rgba8` is straight RGBA, row-major, with `rect.x0/y0` as its origin.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeMaskGpuPatch {
    pub rect: Rect,
    pub pixels_rgba8: Vec<u8>,
}

fn native_mask_gpu_dab(dab: Dab) -> Option<NativeMaskGpuDab> {
    let center = [dab.center.x as f32, dab.center.y as f32];
    let radius = dab.radius * dab.roundness;
    (center.iter().all(|value| value.is_finite())
        && radius.is_finite()
        && radius >= 3.0
        && dab.alpha.is_finite()
        && (0.0..=1.0).contains(&dab.alpha)
        && dab.opacity.is_finite()
        && (0.0..=1.0).contains(&dab.opacity)
        && dab.roundness == 1.0
        && dab.proj_scale == 1.0
        && dab.wet == 1.0
        && dab.mix == 1.0)
        .then_some(NativeMaskGpuDab { center, radius, alpha: dab.alpha, opacity: dab.opacity })
}

#[cfg(not(target_arch = "wasm32"))]
struct MaskPreviewJob {
    client: u64,
    generation: u64,
    /// Full COW snapshot is sent once per stroke; later updates patch only dirty tiles of its
    /// active Paint surface on the worker thread.
    stack: Option<Stack>,
    active_index: usize,
    paint_tiles: Vec<MaskPreviewTilePatch>,
    snapshot_ms: f64,
    patch_collect_ms: f64,
    damage: Vec<Rect>,
    canvas: Rect,
    shell_ids: Option<std::sync::Arc<Vec<u32>>>,
}

#[cfg(not(target_arch = "wasm32"))]
struct MaskPreviewTilePatch {
    coord: TileCoord,
    tile: Option<std::sync::Arc<Tile>>,
}

#[cfg(not(target_arch = "wasm32"))]
struct MaskPreviewResult {
    client: u64,
    generation: u64,
    composed: Option<(f32, Vec<(Rect, Vec<f32>)>)>,
    metrics: MaskPreviewMetrics,
}

/// Timing and bounded-work counters for the most recently completed mask-preview worker job.
/// These are diagnostic values, not a frame-time guarantee.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MaskPreviewMetrics {
    pub snapshot_ms: f64,
    pub patch_collect_ms: f64,
    pub patch_apply_ms: f64,
    pub compose_ms: f64,
    pub patch_tiles: usize,
    pub stack_snapshots: usize,
    pub composed_regions: usize,
    pub composed_pixels: u64,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct MaskPreviewWorkerState {
    active_client: Option<u64>,
    pending: Option<MaskPreviewJob>,
    result: Option<MaskPreviewResult>,
}

#[cfg(not(target_arch = "wasm32"))]
fn apply_mask_preview_tile_patches(stack: &mut Stack, active_index: usize, patches: Vec<MaskPreviewTilePatch>) -> bool {
    let Some(layer) = stack.layers.get_mut(active_index).filter(|layer| matches!(layer.kind, EffectKind::Paint)) else { return false };
    let surface = &mut layer.surface;
    let default = surface.default_bytes().to_vec();
    if default.is_empty() {
        return false;
    }
    for patch in patches {
        if let Some(source) = patch.tile {
            if surface.tile(patch.coord).is_some_and(|target| std::sync::Arc::ptr_eq(target, &source) || target.bytes() == source.bytes()) {
                continue;
            }
            let target = surface.tile_mut(patch.coord);
            if source.bytes().len() != target.bytes().len() {
                return false;
            }
            target.bytes_mut().copy_from_slice(source.bytes());
        } else {
            if surface.tile(patch.coord).is_none_or(|target| target.bytes().chunks_exact(default.len()).all(|pixel| pixel == default)) {
                continue;
            }
            let target = surface.tile_mut(patch.coord);
            for pixel in target.bytes_mut().chunks_exact_mut(default.len()) {
                pixel.copy_from_slice(&default);
            }
        }
    }
    true
}

/// Keep spatially separate live dabs separate in CPU Blur previews. A bounded list falls back to
/// one canvas region only after enough independent islands have accumulated.
const MAX_MASK_PREVIEW_REGIONS: usize = 64;
#[cfg(not(target_arch = "wasm32"))]
const MAX_MASK_PREVIEW_PATCH_TILES: usize = 4_096;

fn accumulate_mask_preview_damage_with_reach(regions: &mut Vec<Rect>, damage: Rect, canvas: Rect, reach: i32) {
    if damage.is_empty() || canvas.is_empty() {
        return;
    }
    let reach = reach.max(0);
    let mut merged = damage.intersect(&canvas);
    if merged.is_empty() {
        return;
    }
    let mut index = 0;
    while index < regions.len() {
        let Some(existing) = regions.get(index).copied() else { break };
        let existing_output = existing.inflate(reach).intersect(&canvas);
        let merged_output = merged.inflate(reach).intersect(&canvas);
        if !existing_output.intersect(&merged_output).is_empty() {
            merged = merged.union(&existing);
            regions.swap_remove(index);
            index = 0;
        } else {
            index += 1;
        }
    }
    if regions.len() >= MAX_MASK_PREVIEW_REGIONS {
        regions.clear();
        regions.push(canvas);
    } else {
        regions.push(merged);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn record_mask_preview_dirty_tiles(tiles: &mut BTreeSet<TileCoord>, damage: Rect) -> bool {
    if damage.is_empty() {
        return true;
    }
    let Some(last_x) = damage.x1.checked_sub(1) else { return false };
    let Some(last_y) = damage.y1.checked_sub(1) else { return false };
    let first = TileCoord::containing(damage.x0, damage.y0);
    let last = TileCoord::containing(last_x, last_y);
    let width = i64::from(last.tx) - i64::from(first.tx) + 1;
    let height = i64::from(last.ty) - i64::from(first.ty) + 1;
    let Some(count) = width.checked_mul(height).and_then(|count| usize::try_from(count).ok()) else { return false };
    if count > MAX_MASK_PREVIEW_PATCH_TILES {
        return false;
    }
    let mut added = 0usize;
    for ty in first.ty..=last.ty {
        for tx in first.tx..=last.tx {
            added += usize::from(!tiles.contains(&TileCoord::new(tx, ty)));
        }
    }
    if tiles.len().saturating_add(added) > MAX_MASK_PREVIEW_PATCH_TILES {
        return false;
    }
    for ty in first.ty..=last.ty {
        for tx in first.tx..=last.tx {
            tiles.insert(TileCoord::new(tx, ty));
        }
    }
    true
}

fn mask_preview_damage_bounds(regions: &[Rect]) -> Rect {
    regions.iter().fold(Rect::EMPTY, |bounds, region| bounds.union(region))
}

/// A handle into one process-wide worker. Sharing a thread bounds work left behind by canceled
/// strokes: globally there is at most one running snapshot, one replaceable pending snapshot and
/// one result.
#[cfg(not(target_arch = "wasm32"))]
struct MaskPreviewWorker {
    state: std::sync::Arc<(std::sync::Mutex<MaskPreviewWorkerState>, std::sync::Condvar)>,
    client: u64,
}

#[cfg(not(target_arch = "wasm32"))]
static MASK_PREVIEW_CLIENT: AtomicU64 = AtomicU64::new(1);

#[cfg(not(target_arch = "wasm32"))]
static MASK_PREVIEW_WORKER: std::sync::OnceLock<Option<std::sync::Arc<(std::sync::Mutex<MaskPreviewWorkerState>, std::sync::Condvar)>>> =
    std::sync::OnceLock::new();

#[cfg(not(target_arch = "wasm32"))]
impl MaskPreviewWorker {
    fn new() -> Option<Self> {
        let state = MASK_PREVIEW_WORKER
            .get_or_init(|| {
                let state = std::sync::Arc::new((std::sync::Mutex::new(MaskPreviewWorkerState::default()), std::sync::Condvar::new()));
                let worker_state = state.clone();
                std::thread::Builder::new()
                    .name("mask-blur-preview".into())
                    .spawn(move || {
                        let mut source_client = None;
                        let mut source_stack: Option<Stack> = None;
                        loop {
                            let job = {
                                let (lock, wake) = &*worker_state;
                                let mut state = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                                loop {
                                    if source_client.is_some() && source_client != state.active_client {
                                        source_client = None;
                                        source_stack = None;
                                    }
                                    if let Some(job) = state.pending.take() {
                                        break job;
                                    }
                                    state = wake.wait(state).unwrap_or_else(std::sync::PoisonError::into_inner);
                                }
                            };
                            let MaskPreviewJob {
                                client,
                                generation,
                                stack,
                                active_index,
                                paint_tiles,
                                snapshot_ms,
                                patch_collect_ms,
                                damage,
                                canvas,
                                shell_ids,
                            } = job;
                            if let Some(snapshot) = stack {
                                source_stack = Some(snapshot);
                                source_client = Some(client);
                            }
                            let mut metrics = MaskPreviewMetrics { snapshot_ms, patch_collect_ms, patch_tiles: paint_tiles.len(), ..Default::default() };
                            let composed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                if source_client != Some(client) {
                                    return None;
                                }
                                let stack = source_stack.as_mut()?;
                                let patch_started = Instant::now();
                                if !apply_mask_preview_tile_patches(stack, active_index, paint_tiles) {
                                    return None;
                                }
                                metrics.patch_apply_ms = patch_started.elapsed().as_secs_f64() * 1000.0;
                                let shell_ids = shell_ids.as_deref().and_then(|ids| photocraft_compose::mask_paint::ShellIdMap::new(canvas, ids));
                                let mut output = Vec::with_capacity(damage.len());
                                let mut default = None;
                                let compose_started = Instant::now();
                                for damage in &damage {
                                    let composed = if let Some(shell_ids) = shell_ids {
                                        photocraft_compose::mask_paint::compose_damage_with_shell_ids(stack, *damage, canvas, shell_ids)
                                    } else {
                                        photocraft_compose::mask_paint::compose_damage(stack, *damage, canvas)
                                    }?;
                                    default = Some(composed.0);
                                    metrics.composed_regions += 1;
                                    metrics.composed_pixels =
                                        metrics.composed_pixels.saturating_add(u64::from(composed.1.width()) * u64::from(composed.1.height()));
                                    output.push((composed.1, composed.2));
                                }
                                metrics.compose_ms = compose_started.elapsed().as_secs_f64() * 1000.0;
                                Some((default?, output))
                            }))
                            .unwrap_or(None);
                            let (lock, wake) = &*worker_state;
                            let mut state = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                            if state.active_client == Some(client)
                                && state.result.as_ref().is_none_or(|result| (result.client, result.generation) <= (client, generation))
                            {
                                state.result = Some(MaskPreviewResult { client, generation, composed, metrics });
                            }
                            wake.notify_all();
                        }
                    })
                    .ok()?;
                Some(state)
            })
            .as_ref()?
            .clone();
        Some(Self { state, client: MASK_PREVIEW_CLIENT.fetch_add(1, Ordering::Relaxed).max(1) })
    }

    fn submit(&self, job: MaskPreviewJob) {
        let (lock, wake) = &*self.state;
        let mut state = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // A newer cumulative dirty-tile patch subsumes the queued patch. Preserve its one-time
        // stack snapshot if the worker has not taken that first job yet.
        let mut job = job;
        if job.stack.is_none()
            && let Some(previous) = state.pending.take()
            && previous.client == job.client
            && previous.stack.is_some()
        {
            job.stack = previous.stack;
        }
        // Input itself was already rendered in order; only this latest cumulative state matters.
        state.active_client = Some(self.client);
        state.result = None;
        state.pending = Some(job);
        wake.notify_one();
    }

    fn take_result(&self) -> Option<MaskPreviewResult> {
        let mut state = self.state.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.result.as_ref().is_some_and(|result| result.client != self.client) {
            state.result = None;
        }
        state.result.take()
    }

    fn wait_for_result(&self) {
        let (lock, wake) = &*self.state;
        let state = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.result.as_ref().is_none_or(|result| result.client != self.client) {
            let _ = wake.wait_timeout(state, Duration::from_millis(100)).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for MaskPreviewWorker {
    fn drop(&mut self) {
        let (lock, wake) = &*self.state;
        let mut state = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.active_client == Some(self.client) {
            state.active_client = None;
            if state.pending.as_ref().is_some_and(|job| job.client == self.client) {
                state.pending = None;
            }
            if state.result.as_ref().is_some_and(|result| result.client == self.client) {
                state.result = None;
            }
            wake.notify_all();
        }
    }
}

/// Reuses the immutable portion of a mask stack while the active Paint surface changes.
/// Each entry is one 32×32 f32 block (4 KiB); the FIFO cap bounds live-stroke memory at 2 MiB.
struct MaskPrefixCache {
    layer: LayerId,
    active: usize,
    shell_ids: Option<std::sync::Arc<Vec<u32>>>,
    blocks: HashMap<MaskPrefixBlock, Vec<f32>>,
    order: VecDeque<MaskPrefixBlock>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct MaskPrefixBlock {
    x: i32,
    y: i32,
}

impl MaskPrefixBlock {
    const SIDE: i32 = 32;

    fn containing(x: i32, y: i32) -> Self {
        Self { x: x.div_euclid(Self::SIDE), y: y.div_euclid(Self::SIDE) }
    }

    fn rect(self) -> Rect {
        Rect::from_xywh(self.x.saturating_mul(Self::SIDE), self.y.saturating_mul(Self::SIDE), Self::SIDE as u32, Self::SIDE as u32)
    }
}

#[cfg(feature = "uv-workbench")]
fn uv_mask_shell_ids(s: &Session, doc: &Document, layer: Option<LayerId>, params: &Value) -> Option<std::sync::Arc<Vec<u32>>> {
    if !is_mask_target(params) || doc.size.width == 0 || doc.size.height == 0 {
        return None;
    }
    let stack = doc.layer(layer?)?.mask.as_ref()?.paint.as_ref()?;
    if !stack.layers.iter().any(|layer| layer.visible && layer.opacity > 0.0 && matches!(layer.kind, EffectKind::Blur { .. })) {
        return None;
    }
    let mut materials = std::collections::BTreeSet::new();
    for channel in photocraft_uv::preview::Channel::MAPS {
        for (material, binding) in s.uv.map_bindings(channel).iter().enumerate() {
            if *binding == Some(doc.id) {
                materials.insert(material);
            }
        }
    }
    if materials.is_empty() {
        return None;
    }
    s.uv.mask_shell_ids(doc.size.width, doc.size.height, &materials.into_iter().collect::<Vec<_>>(), [0, 0])
}

#[cfg(not(feature = "uv-workbench"))]
fn uv_mask_shell_ids(_s: &Session, _doc: &Document, _layer: Option<LayerId>, _params: &Value) -> Option<std::sync::Arc<Vec<u32>>> {
    None
}

impl MaskPrefixCache {
    const MAX_BLOCKS: usize = 512;

    fn begin(doc: &Document, layer: Option<LayerId>, params: &Value, shell_ids: Option<std::sync::Arc<Vec<u32>>>) -> Option<Self> {
        if !is_mask_target(params) {
            return None;
        }
        let layer = layer?;
        let stack = doc.layer(layer)?.mask.as_ref()?.paint.as_ref()?;
        let active = stack.active;
        if !matches!(stack.layers.get(active)?.kind, EffectKind::Paint) {
            return None;
        }
        // A prefix cache can only apply pointwise layers after the active Paint. Blur needs the
        // changed source rectangle plus its downstream spatial halo recomposed as one stack, so
        // skip creating a cache that would be rejected on every dab and use that exact path.
        if stack.layers.iter().skip(active.saturating_add(1)).any(|layer| layer.visible && layer.opacity > 0.0 && matches!(layer.kind, EffectKind::Blur { .. }))
        {
            return None;
        }
        Some(Self { layer, active, shell_ids, blocks: HashMap::new(), order: VecDeque::new() })
    }

    fn refresh(&mut self, doc: &mut Document, damage: Rect) -> bool {
        if damage.is_empty() || damage.tile_count() > 64 {
            return false;
        }
        let first = MaskPrefixBlock::containing(damage.x0, damage.y0);
        let last = MaskPrefixBlock::containing(damage.x1.saturating_sub(1), damage.y1.saturating_sub(1));
        let block_count = (i64::from(last.x) - i64::from(first.x) + 1) as u64 * (i64::from(last.y) - i64::from(first.y) + 1) as u64;
        if block_count > Self::MAX_BLOCKS as u64 {
            return false;
        }
        for y in first.y..=last.y {
            for x in first.x..=last.x {
                let block = MaskPrefixBlock { x, y };
                let block_rect = block.rect();
                let region = damage.intersect(&block_rect);
                if region.is_empty() {
                    continue;
                }
                if !self.blocks.contains_key(&block) {
                    let prefix = {
                        let Some(stack) = doc.layer(self.layer).and_then(|layer| layer.mask.as_ref()).and_then(|mask| mask.paint.as_ref()) else {
                            return false;
                        };
                        if stack.active != self.active || !matches!(stack.layers.get(self.active).map(|layer| &layer.kind), Some(EffectKind::Paint)) {
                            return false;
                        }
                        let prefix = if let Some(ids) = self.shell_ids.as_deref()
                            && let Some(shell_ids) = photocraft_compose::mask_paint::ShellIdMap::new(doc.bounds(), ids)
                        {
                            photocraft_compose::mask_paint::compose_prefix_with_shell_ids(stack, self.active, block_rect, shell_ids)
                        } else {
                            photocraft_compose::mask_paint::compose_prefix(stack, self.active, block_rect)
                        };
                        let Some(prefix) = prefix else { return false };
                        prefix
                    };
                    while self.blocks.len() >= Self::MAX_BLOCKS {
                        let Some(evicted) = self.order.pop_front() else { return false };
                        self.blocks.remove(&evicted);
                    }
                    self.order.push_back(block);
                    self.blocks.insert(block, prefix);
                }
                let Some(prefix) = self.blocks.get(&block) else { return false };
                let Some(initial) = crop_mask_prefix(prefix, block_rect, region) else { return false };
                let Some(mask) = doc.layer_mut(self.layer).and_then(|layer| layer.mask.as_mut()) else { return false };
                if !photocraft_compose::mask_paint::refresh_from_prefix(mask, region, self.active, &initial) {
                    return false;
                }
            }
        }
        true
    }
}

fn crop_mask_prefix(prefix: &[f32], tile: Rect, region: Rect) -> Option<Vec<f32>> {
    if !tile.contains_rect(&region) || region.is_empty() {
        return None;
    }
    let width = usize::try_from(region.width()).ok()?;
    let height = usize::try_from(region.height()).ok()?;
    let stride = usize::try_from(tile.width()).ok()?;
    if prefix.len() != stride.checked_mul(usize::try_from(tile.height()).ok()?)? {
        return None;
    }
    let x0 = usize::try_from(region.x0.checked_sub(tile.x0)?).ok()?;
    let y0 = usize::try_from(region.y0.checked_sub(tile.y0)?).ok()?;
    let mut cropped = Vec::with_capacity(width.checked_mul(height)?);
    for y in y0..y0.checked_add(height)? {
        let start = y.checked_mul(stride)?.checked_add(x0)?;
        cropped.extend_from_slice(prefix.get(start..start.checked_add(width)?)?);
    }
    Some(cropped)
}

/// A finished live stroke offered to its own commit ([`Session::offer_live_stroke`],
/// [`Session::offer_live_retouch`]): when the committing command matches it (same command,
/// document revision, params, seed and points), it takes these pixels instead of rendering the
/// stroke again.
pub struct LiveCommit {
    /// Mixer load before and after the stroke; transient, never serialized into the journal.
    pub(crate) mixer: Option<(MixerState, MixerState)>,
    pub(crate) cmd: String,
    pub(crate) base: (photocraft_doc::DocId, u64),
    pub(crate) params: Value,
    /// The Brush's and Pencil's seed; the retouching tools have none.
    pub(crate) seed: Option<u64>,
    pub(crate) points: Vec<StrokePoint>,
    pub(crate) breaks: Vec<usize>,
    pub(crate) surface: Surface,
    pub(crate) bounds: Rect,
}

impl LiveCommit {
    /// The params that decide the pixels, without the points and seed (compared separately).
    fn settings(p: &Value) -> Value {
        let mut p = p.clone();
        if let Some(o) = p.as_object_mut() {
            o.remove("points");
            o.remove("seed");
        }
        p
    }

    pub(crate) fn matches(&self, s: &Session, cmd: &str, p: &Value, pts: &[StrokePoint]) -> bool {
        let base = s.active().map(|st| (st.doc.id, st.revision));
        let breaks = parse_breaks(p, pts.len(), cmd).unwrap_or_default();
        self.cmd == cmd
            && base == Some(self.base)
            && p.get("seed").and_then(Value::as_u64) == self.seed
            && self.points == pts
            && self.breaks == breaks
            && Self::settings(p) == Self::settings(&self.params)
    }
}

impl LiveStroke {
    /// Start from `paint.stroke` params; their `points` are rendered.
    pub fn begin(s: &Session, p: &Value) -> Result<Self> {
        Self::begin_with(s, "paint.stroke", p)
    }

    /// Start a live stroke of `cmd`: `paint.stroke` (the Brush and Eraser) or `paint.pencil`.
    /// Committing `cmd` with the same params and `"seed": live.seed` gives the same pixels.
    pub fn begin_with(s: &Session, cmd: &str, p: &Value) -> Result<Self> {
        Self::begin_internal(s, cmd, p, None, false)
    }

    /// Start a live mask stroke with an off-thread flattened-stack cache preview when supported. This
    /// installs the worker before rendering the initial point, avoiding a first-dab UI stall.
    /// Unsupported targets and wasm retain the original synchronous path.
    pub fn begin_with_mask_preview_worker(s: &Session, cmd: &str, p: &Value, cadence: Duration) -> Result<Self> {
        Self::begin_internal(s, cmd, p, Some(cadence), false)
    }

    /// Whether a stroke can use the native grayscale mask GPU brush path. This is a static,
    /// pre-input check: if it returns false, callers must begin the ordinary CPU stroke before
    /// sending any points. The deliberately narrow subset guarantees every generated dab is a
    /// computed, round dab with an effective radius of at least three pixels.
    pub fn can_use_native_mask_gpu_brush(s: &Session, cmd: &str, p: &Value) -> bool {
        if cfg!(target_arch = "wasm32")
            || cmd != "paint.stroke"
            || !is_mask_target(p)
            || flag(p, "fill", false)
            || s.active().is_none_or(|st| st.symmetry.is_some())
        {
            return false;
        }
        let Ok(brush) = resolve_brush(s, p, cmd).map(|brush| with_blend_mode(brush, p)) else { return false };
        let Ok((Some(layer_id), brush, _)) = stroke_target(s, p, brush) else { return false };
        let Some(st) = s.active() else { return false };
        let Some(mask) = st.doc.layer(layer_id).and_then(|layer| layer.mask.as_ref()) else { return false };
        let Some(stack) = mask.paint.as_ref() else { return false };
        let Some(active) = stack.layers.get(stack.active) else { return false };
        if !matches!(active.kind, EffectKind::Paint) || active.surface.format() != photocraft_color::PixelFormat::RGBA8 {
            return false;
        }
        if st.doc.selection.is_some() || flag(p, "uvWrap", false) {
            return false;
        }
        use photocraft_color::BlendMode;
        matches!(brush.tip, TipShape::Round)
            && brush.mode == BlendMode::Normal
            && !brush.erase
            && !brush.aliased
            && !brush.pressure_size
            && brush.size.is_finite()
            && brush.size >= 6.0
            && brush.hardness.is_finite()
            && (0.0..=1.0).contains(&brush.hardness)
            && brush.roundness == 1.0
            && !brush.shape_dynamics.enabled
            && !brush.scattering.enabled
            && !brush.texture.enabled
            && !brush.dual_brush.enabled
            && !brush.color_dynamics.enabled
            && !brush.noise
            && !brush.wet_edges
            && brush.color.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v))
    }

    /// Start an all-or-CPU eligible native mask GPU stroke. Eligibility is checked before the
    /// initial point is passed to the dab generator. The caller may still fall back before display
    /// if GPU resource creation fails; no canonical document pixels are changed until patch apply.
    pub fn begin_with_native_mask_gpu_brush(s: &Session, cmd: &str, p: &Value) -> Result<Self> {
        if !Self::can_use_native_mask_gpu_brush(s, cmd, p) {
            return Err(bad(cmd, "brush settings or target are outside the native mask GPU subset"));
        }
        Self::begin_internal(s, cmd, p, None, true)
    }

    fn begin_internal(s: &Session, cmd: &str, p: &Value, worker_cadence: Option<Duration>, native_mask_gpu_brush: bool) -> Result<Self> {
        has_paintable(s).map_err(EngineError::Other)?;
        let pts = parse_points(p, cmd)?;
        let pencil = match cmd {
            "paint.stroke" => false,
            "paint.pencil" => true,
            other => return Err(bad(other, "live strokes are `paint.stroke` or `paint.pencil`")),
        };
        let brush = if pencil { pencil_brush(s, p)? } else { with_blend_mode(resolve_brush(s, p, cmd)?, p) };
        let (seed, fg) = (brush.seed, brush.color);
        let (layer, mut brush, zoom) = stroke_target(s, p, brush)?;
        let st = s.active().ok_or(EngineError::NoDocument)?;
        let base = (st.doc.id, st.revision);
        let mut doc = (*st.doc).clone();
        let sel = doc.selection.clone();
        let table = crate::mode_cmds::paint_color_table(&doc, p);
        let (surf, lock) = crate::channel_cmds::target_surface(&mut doc, layer, p)?;
        erase_locked(&mut brush, lock, s.tools.background);
        if pencil && flag(p, "autoErase", false) {
            let fg = table.as_ref().map_or(fg, |t| crate::mode_cmds::table_color(t, fg));
            apply_auto_erase(&mut brush, surf, pts.first(), fg, s.tools.background);
        }
        let renderer = StrokeRenderer::new(&brush, Some(surf.format()), zoom);
        let native_mask_gpu_generator = native_mask_gpu_brush.then(|| DabGenerator::new(&brush, zoom));
        let renderer_brush = brush.clone();
        let symmetry = s.active().and_then(|st| st.symmetry.clone());
        let mirrors: Vec<_> = (0..symmetry.as_ref().map_or(0, |s| s.mirror_count())).map(|_| StrokeRenderer::new(&brush, Some(surf.format()), zoom)).collect();
        let mirror_distinct = vec![false; mirrors.len()];
        let pre = surf.clone();
        let mask_shell_ids = uv_mask_shell_ids(s, &doc, layer, p);
        let mask_prefix = MaskPrefixCache::begin(&doc, layer, p, mask_shell_ids.clone());
        let mut live = Self {
            doc: std::sync::Arc::new(doc),
            seed,
            cmd: cmd.into(),
            renderer,
            native_mask_gpu_generator,
            native_mask_gpu_dabs: Vec::new(),
            native_mask_gpu_damage: Rect::EMPTY,
            native_mask_gpu_patch_applied: false,
            brush: renderer_brush,
            zoom,
            symmetry,
            mirrors,
            mirror_distinct,
            union_live: None,
            pre,
            sel,
            lock,
            layer,
            params: p.clone(),
            tail: Rect::EMPTY,
            finished_bounds: Rect::EMPTY,
            cmyk: doc_cmyk_space(s),
            points: Vec::new(),
            breaks: Vec::new(),
            segment_break_pending: false,
            base,
            mask_prefix,
            mask_shell_ids,
            mask_preview_cadence: None,
            mask_preview_pending: Vec::new(),
            mask_preview_commit_damage: Rect::EMPTY,
            mask_preview_last_refresh: None,
            mask_preview_last_submit: None,
            mask_preview_generation: 0,
            mask_preview_submitted: 0,
            mask_preview_unsupported: None,
            #[cfg(not(target_arch = "wasm32"))]
            mask_preview_worker: None,
            #[cfg(not(target_arch = "wasm32"))]
            mask_preview_patch_tiles: BTreeSet::new(),
            #[cfg(not(target_arch = "wasm32"))]
            mask_preview_snapshot_sent: false,
            #[cfg(not(target_arch = "wasm32"))]
            mask_preview_patch_overflow: false,
            #[cfg(not(target_arch = "wasm32"))]
            mask_preview_last_metrics: None,
            #[cfg(not(target_arch = "wasm32"))]
            mask_preview_snapshot_ms_total: 0.0,
            #[cfg(not(target_arch = "wasm32"))]
            mask_preview_patch_collect_ms_total: 0.0,
            #[cfg(not(target_arch = "wasm32"))]
            mask_preview_stack_snapshots: 0,
        };
        if let Some(cadence) = worker_cadence {
            let _ = live.set_mask_preview_worker(cadence);
        }
        live.push(&pts)?;
        Ok(live)
    }

    /// The stroke as its commit would take it over (see [`LiveCommit`]).
    pub fn into_commit(mut self) -> Result<LiveCommit> {
        if self.native_mask_gpu_generator.is_some() && !self.native_mask_gpu_patch_applied {
            self.fallback_native_mask_gpu_to_cpu()?;
        }
        let bounds = self.bounds().union(&self.mask_preview_commit_damage);
        let (surf, _) = crate::channel_cmds::target_surface(std::sync::Arc::make_mut(&mut self.doc), self.layer, &self.params)?;
        let mut params = self.params;
        if !self.breaks.is_empty() {
            params["breaks"] = json!(self.breaks);
        }
        Ok(LiveCommit {
            mixer: None,
            cmd: self.cmd,
            base: self.base,
            params,
            seed: Some(self.seed),
            points: self.points,
            breaks: self.breaks,
            surface: surf.clone(),
            bounds,
        })
    }

    /// Everything the stroke has touched so far.
    pub fn bounds(&self) -> Rect {
        let bounds = self.finished_bounds.union(&self.renderer.bounds()).union(&self.tail).union(&self.native_mask_gpu_damage);
        self.mirrors.iter().zip(&self.mirror_distinct).filter(|(_, distinct)| **distinct).fold(bounds, |b, (r, _)| b.union(&r.bounds()))
    }

    /// Finish the current disconnected path without ending the physical gesture or committing
    /// history. The next pushed point starts a fresh renderer, so no coverage can bridge a UV
    /// island boundary. The final command still commits the accumulated document once.
    pub fn break_segment(&mut self) -> Result<Rect> {
        check_coords(&self.points, &self.cmd)?;
        if !self.supports_mask_segments() {
            return Err(bad(&self.cmd, "disconnected segments are only enabled for mask strokes with downstream Blur"));
        }
        self.break_path()
    }

    /// Finish the current disconnected brush path while keeping the physical gesture and
    /// history edit alive. Used by 3D surface painting when the cursor alternates between a
    /// source sample and its mirrored UV target.
    pub fn break_path(&mut self) -> Result<Rect> {
        check_coords(&self.points, &self.cmd)?;
        if self.points.is_empty() || self.segment_break_pending {
            return Ok(Rect::EMPTY);
        }
        if let Some(generator) = self.native_mask_gpu_generator.as_mut() {
            // A UV seam or reflected symmetry pass is a discontinuity in texture space. Flush
            // the current smoothing tail and restart the GPU dab walker so it cannot stamp a
            // connecting segment across unrelated UV islands.
            let (mut dabs, mut dual) = (Vec::new(), Vec::new());
            generator.finish(&mut dabs, &mut dual);
            if !dual.is_empty() {
                self.segment_break_pending = true;
                return self.fallback_native_mask_gpu_to_cpu();
            }
            let Some(damage) = self.append_native_mask_gpu_dabs(&dabs) else {
                self.segment_break_pending = true;
                return self.fallback_native_mask_gpu_to_cpu();
            };
            self.native_mask_gpu_damage = self.native_mask_gpu_damage.union(&damage);
            self.native_mask_gpu_generator = Some(DabGenerator::new(&self.brush, self.zoom));
            self.segment_break_pending = true;
            return Ok(damage);
        }
        let cmyk = self.cmyk.clone();
        photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
            let old_tail = self.tail;
            let mut segment_bounds = self.renderer.bounds().union(&old_tail);
            for (renderer, distinct) in self.mirrors.iter().zip(&self.mirror_distinct) {
                if *distinct {
                    segment_bounds = segment_bounds.union(&renderer.bounds());
                }
            }
            let damage = {
                let (surf, _) = crate::channel_cmds::target_surface(std::sync::Arc::make_mut(&mut self.doc), self.layer, &self.params)?;
                if !old_tail.is_empty() {
                    surf.write_region(old_tail, &self.pre.read_region(old_tail));
                }
                self.renderer.finish();
                for mirror in &mut self.mirrors {
                    mirror.finish();
                }
                let damage = if self.mirror_distinct.iter().any(|distinct| *distinct) {
                    let copies = self.mirrors.iter().zip(&self.mirror_distinct).filter(|(_, distinct)| **distinct).map(|(renderer, _)| renderer);
                    self.renderer.composite_union_many(copies, &self.pre, surf, self.sel.as_ref(), self.lock)
                } else {
                    self.renderer.composite(&self.pre, surf, self.sel.as_ref(), self.lock, true)
                };
                self.pre = surf.clone();
                self.renderer = StrokeRenderer::new(&self.brush, Some(surf.format()), self.zoom);
                self.mirrors = (0..self.symmetry.as_ref().map_or(0, |s| s.mirror_count()))
                    .map(|_| StrokeRenderer::new(&self.brush, Some(surf.format()), self.zoom))
                    .collect();
                damage
            };
            self.tail = Rect::EMPTY;
            self.finished_bounds = self.finished_bounds.union(&segment_bounds);
            self.union_live = None;
            self.refresh_mask_preview(damage);
            self.segment_break_pending = true;
            self.mirror_distinct.fill(false);
            Ok(damage)
        })
    }

    /// Disconnected path boundaries included in the eventual single commit.
    pub fn segment_breaks(&self) -> &[usize] {
        &self.breaks
    }

    /// Whether this stroke is the native-mask Blur path that supports UV island breaks.
    pub fn supports_mask_segments(&self) -> bool {
        self.native_mask_gpu_generator.is_some() || is_mask_target(&self.params) && self.has_visible_downstream_blur()
    }

    /// `(brush, pointer)` document positions while smoothing holds the brush back from the
    /// pointer: the canvas draws a line between them.
    pub fn smoothing_string(&self) -> Option<([f64; 2], [f64; 2])> {
        self.native_mask_gpu_generator.as_ref().map_or_else(|| self.renderer.smoothing_string(), DabGenerator::smoothing_string)
    }

    /// Does the stroke change with time while the pointer is held still (airbrush Build-up, or
    /// smoothing still catching up)? See `photocraft_paint::dynamics::DabGenerator::wants_time`.
    pub fn wants_time(&self) -> bool {
        self.native_mask_gpu_generator.as_ref().map_or_else(|| self.renderer.wants_time(), DabGenerator::wants_time)
    }

    /// Defer flattened mask-cache updates until this cadence elapses. Stroke painting itself
    /// remains exact and immediate; only the derived `mask.surface` preview is throttled.
    /// Callers should enable this only when their active preview can tolerate a temporarily
    /// stale flattened cache. The first point rendered by `begin_with` is always eager because
    /// the policy is opt-in after construction.
    pub fn set_mask_preview_cadence(&mut self, cadence: Duration) {
        if !is_mask_target(&self.params) {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.mask_preview_worker.is_some() {
                let _ = self.flush_mask_preview();
            }
            self.mask_preview_worker = None;
        }
        self.mask_preview_cadence = Some(cadence);
        self.mask_preview_last_refresh = Some(Instant::now());
    }

    /// Enable off-thread flattened-cache updates for mask Paint strokes with sublayers. Returns
    /// false for ordinary masks, non-Paint active effects, wasm, or when the worker cannot start.
    /// Brush sampling and renderer accumulation remain synchronous and ordered on the caller.
    pub fn set_mask_preview_worker(&mut self, cadence: Duration) -> bool {
        if self.native_mask_gpu_generator.is_some() {
            return false;
        }
        if !self.has_deferred_mask_preview_work() {
            return false;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(worker) = MaskPreviewWorker::new() else { return false };
            self.mask_preview_worker = Some(worker);
            self.mask_preview_cadence = Some(cadence);
            self.mask_preview_last_refresh = Some(Instant::now());
            self.mask_preview_last_submit = None;
            self.mask_preview_unsupported = None;
            return true;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = cadence;
            false
        }
    }

    fn has_deferred_mask_preview_work(&self) -> bool {
        if !is_mask_target(&self.params) {
            return false;
        }
        let Some(id) = self.layer else { return false };
        self.doc
            .layer(id)
            .and_then(|layer| layer.mask.as_ref())
            .and_then(|mask| mask.paint.as_ref())
            .is_some_and(|stack| stack.layers.len() > 1 && stack.layers.get(stack.active).is_some_and(|layer| matches!(layer.kind, EffectKind::Paint)))
    }

    fn has_visible_downstream_blur(&self) -> bool {
        let Some(id) = self.layer else { return false };
        self.doc.layer(id).and_then(|layer| layer.mask.as_ref()).and_then(|mask| mask.paint.as_ref()).is_some_and(|stack| {
            stack
                .layers
                .iter()
                .skip(stack.active.saturating_add(1))
                .any(|layer| layer.visible && layer.opacity > 0.0 && matches!(layer.kind, EffectKind::Blur { .. }))
        })
    }

    /// Poll and apply the latest completed native mask preview without waiting. The returned
    /// rectangle is the flattened cache region that the caller should invalidate for display.
    pub fn poll_mask_preview(&mut self) -> Option<Rect> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut published = None;
            let result = self.mask_preview_worker.as_ref()?.take_result();
            if let Some(result) = result {
                if result.generation <= self.mask_preview_generation {
                    self.mask_preview_last_metrics = Some(result.metrics);
                    published = self.publish_mask_preview_result(result, false);
                }
            }
            if !self.mask_preview_pending.is_empty()
                && self.mask_preview_submitted < self.mask_preview_generation
                && mask_preview_cadence_due(self.mask_preview_last_submit, Instant::now(), self.mask_preview_cadence.unwrap_or_default())
            {
                self.submit_mask_preview();
            }
            published
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }

    /// Whether the off-thread mask cache still has damage that has not been published.
    pub fn mask_preview_pending(&self) -> bool {
        !self.mask_preview_pending.is_empty() && self.mask_preview_unsupported != Some(self.mask_preview_generation)
    }

    /// Take worker-side timing/counter diagnostics for the latest completed preview job.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn take_mask_preview_metrics(&mut self) -> Option<MaskPreviewMetrics> {
        let mut metrics = self.mask_preview_last_metrics.take()?;
        metrics.snapshot_ms = self.mask_preview_snapshot_ms_total;
        metrics.patch_collect_ms = self.mask_preview_patch_collect_ms_total;
        metrics.stack_snapshots = self.mask_preview_stack_snapshots;
        Some(metrics)
    }

    /// WebAssembly uses the synchronous mask-preview path and has no worker diagnostics.
    #[cfg(target_arch = "wasm32")]
    pub fn take_mask_preview_metrics(&mut self) -> Option<MaskPreviewMetrics> {
        None
    }

    /// Materialize any mask-cache damage accumulated by a throttled live preview.
    /// Safe to call repeatedly. With the worker enabled this waits for the latest ordered input
    /// snapshot and publishes it; the canonical Session edit still refreshes the committed cache.
    pub fn flush_mask_preview(&mut self) -> Option<Rect> {
        #[cfg(not(target_arch = "wasm32"))]
        if self.mask_preview_worker.is_some() {
            if self.mask_preview_pending.is_empty() {
                return None;
            }
            if self.mask_preview_unsupported == Some(self.mask_preview_generation) {
                let pending = std::mem::take(&mut self.mask_preview_pending);
                self.refresh_mask_preview_regions_now(&pending);
                self.mask_preview_last_refresh = Some(Instant::now());
                return Some(self.expanded_mask_preview_damage(mask_preview_damage_bounds(&pending)));
            }
            if self.mask_preview_submitted < self.mask_preview_generation {
                self.submit_mask_preview();
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                if let Some(result) = self.mask_preview_worker.as_ref().and_then(MaskPreviewWorker::take_result) {
                    if result.generation <= self.mask_preview_generation {
                        let is_current = result.generation == self.mask_preview_generation;
                        self.mask_preview_last_metrics = Some(result.metrics);
                        let published = self.publish_mask_preview_result(result, is_current);
                        if is_current && published.is_some() {
                            return published;
                        }
                    }
                }
                if self.mask_preview_submitted < self.mask_preview_generation {
                    self.submit_mask_preview();
                }
                if let Some(worker) = &self.mask_preview_worker {
                    worker.wait_for_result();
                } else {
                    break;
                }
            }
            // Worker failure falls through to the existing exact synchronous path.
        }
        let pending = std::mem::take(&mut self.mask_preview_pending);
        if pending.is_empty() {
            return None;
        }
        let bounds = mask_preview_damage_bounds(&pending);
        self.refresh_mask_preview_regions_now(&pending);
        self.mask_preview_last_refresh = Some(Instant::now());
        Some(self.expanded_mask_preview_damage(bounds))
    }

    /// Drop transient preview work before committing, returning the display invalidation it would
    /// have published. The normal `paint.stroke` Session edit recomposes the authoritative mask
    /// cache synchronously before publishing the updated document, so preview composition here
    /// would duplicate the same work and delay pointer-up.
    pub fn defer_mask_preview_to_commit(&mut self) -> Vec<Rect> {
        let pending = std::mem::take(&mut self.mask_preview_pending);
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.mask_preview_worker = None;
        }
        self.mask_preview_submitted = self.mask_preview_generation;
        self.mask_preview_unsupported = None;
        pending.into_iter().map(|damage| self.expanded_mask_preview_damage(damage)).filter(|damage| !damage.is_empty()).collect()
    }

    fn submit_mask_preview(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(worker) = &self.mask_preview_worker else { return };
            if self.mask_preview_patch_overflow {
                self.mask_preview_unsupported = Some(self.mask_preview_generation);
                return;
            }
            let Some(id) = self.layer else { return };
            let Some(stack) = self.doc.layer(id).and_then(|layer| layer.mask.as_ref()).and_then(|mask| mask.paint.as_ref()) else { return };
            if self.mask_preview_pending.is_empty() {
                return;
            }
            let generation = self.mask_preview_generation;
            if generation == 0 {
                return;
            }
            let active_index = stack.active;
            let Some(active) = stack.layers.get(active_index).filter(|layer| matches!(layer.kind, EffectKind::Paint)) else { return };
            let snapshot_started = Instant::now();
            let snapshot = (!self.mask_preview_snapshot_sent).then(|| stack.clone());
            let snapshot_sent = snapshot.is_some();
            let snapshot_ms = snapshot_started.elapsed().as_secs_f64() * 1000.0;
            let patch_started = Instant::now();
            let paint_tiles =
                self.mask_preview_patch_tiles.iter().map(|coord| MaskPreviewTilePatch { coord: *coord, tile: active.surface.tile(*coord).cloned() }).collect();
            let patch_collect_ms = patch_started.elapsed().as_secs_f64() * 1000.0;
            worker.submit(MaskPreviewJob {
                client: worker.client,
                generation,
                stack: snapshot,
                active_index,
                paint_tiles,
                snapshot_ms,
                patch_collect_ms,
                damage: self.mask_preview_pending.clone(),
                canvas: self.doc.bounds(),
                shell_ids: self.mask_shell_ids.clone(),
            });
            if snapshot_sent {
                self.mask_preview_stack_snapshots += 1;
            }
            self.mask_preview_snapshot_ms_total += snapshot_ms;
            self.mask_preview_patch_collect_ms_total += patch_collect_ms;
            self.mask_preview_snapshot_sent = true;
            self.mask_preview_submitted = generation;
            self.mask_preview_last_submit = Some(Instant::now());
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn publish_mask_preview_result(&mut self, result: MaskPreviewResult, allow_sync_fallback: bool) -> Option<Rect> {
        if result.generation > self.mask_preview_generation {
            return None;
        }
        let is_current = result.generation == self.mask_preview_generation;
        let Some((default, regions)) = result.composed else {
            if is_current && !allow_sync_fallback {
                self.mask_preview_unsupported = Some(result.generation);
                return None;
            }
            if !is_current {
                // This older snapshot cannot resolve the newer pending edits; retain them and
                // let the cadence submit a current job. Final flush owns any synchronous fallback.
                return None;
            }
            let pending = std::mem::take(&mut self.mask_preview_pending);
            let bounds = mask_preview_damage_bounds(&pending);
            self.refresh_mask_preview_regions_now(&pending);
            self.mask_preview_last_refresh = Some(Instant::now());
            return Some(self.expanded_mask_preview_damage(bounds));
        };
        let mut damage = Rect::EMPTY;
        if !regions.is_empty() {
            let id = self.layer?;
            let doc = std::sync::Arc::make_mut(&mut self.doc);
            let mask = doc.layer_mut(id)?.mask.as_mut()?;
            mask.surface.set_default_pixel(&[default]);
            for (bounds, pixels) in regions {
                mask.surface.write_region(bounds, &pixels);
                damage = damage.union(&bounds);
            }
        }
        if is_current {
            self.mask_preview_pending.clear();
            self.mask_preview_unsupported = None;
        }
        self.mask_preview_last_refresh = Some(Instant::now());
        (!damage.is_empty()).then(|| self.expanded_mask_preview_damage(damage))
    }

    fn mask_preview_blur_reach(&self) -> i32 {
        let Some(id) = self.layer else { return 0 };
        let Some(stack) = self.doc.layer(id).and_then(|layer| layer.mask.as_ref()).and_then(|mask| mask.paint.as_ref()) else { return 0 };
        photocraft_compose::mask_paint::downstream_blur_reach(stack, stack.active)
    }

    fn expanded_mask_preview_damage(&self, damage: Rect) -> Rect {
        damage.inflate(self.mask_preview_blur_reach()).intersect(&self.doc.bounds())
    }

    fn refresh_mask_preview(&mut self, damage: Rect) {
        if damage.is_empty() || !is_mask_target(&self.params) {
            return;
        }
        let reach = self.mask_preview_blur_reach();
        #[cfg(not(target_arch = "wasm32"))]
        if self.mask_preview_worker.is_some() {
            self.mask_preview_commit_damage = self.mask_preview_commit_damage.union(&self.expanded_mask_preview_damage(damage));
            self.mask_preview_generation = self.mask_preview_generation.wrapping_add(1).max(1);
            if self.mask_preview_patch_overflow || !record_mask_preview_dirty_tiles(&mut self.mask_preview_patch_tiles, damage.intersect(&self.doc.bounds())) {
                self.mask_preview_patch_overflow = true;
                self.mask_preview_unsupported = Some(self.mask_preview_generation);
                self.mask_preview_pending.clear();
                return;
            }
            accumulate_mask_preview_damage_with_reach(&mut self.mask_preview_pending, damage, self.doc.bounds(), reach);
            if mask_preview_cadence_due(self.mask_preview_last_submit, Instant::now(), self.mask_preview_cadence.unwrap_or_default()) {
                self.submit_mask_preview();
            }
            return;
        }
        if let Some(cadence) = self.mask_preview_cadence {
            accumulate_mask_preview_damage_with_reach(&mut self.mask_preview_pending, damage, self.doc.bounds(), reach);
            if mask_preview_cadence_due(self.mask_preview_last_refresh, Instant::now(), cadence) {
                let _ = self.flush_mask_preview();
            }
            return;
        }
        self.refresh_mask_preview_now(damage);
    }

    fn refresh_mask_preview_now(&mut self, damage: Rect) {
        if damage.is_empty() || !is_mask_target(&self.params) {
            return;
        }
        let Some(id) = self.layer else { return };
        let doc = std::sync::Arc::make_mut(&mut self.doc);
        if self.mask_prefix.as_mut().is_some_and(|cache| cache.refresh(doc, damage)) {
            return;
        }
        let canvas = doc.bounds();
        if let Some(mask) = doc.layer_mut(id).and_then(|layer| layer.mask.as_mut()) {
            if let Some(shell_ids) = self.mask_shell_ids.as_deref().and_then(|ids| photocraft_compose::mask_paint::ShellIdMap::new(canvas, ids)) {
                photocraft_compose::mask_paint::refresh_damage_in_canvas_with_shell_ids(mask, damage, canvas, shell_ids);
            } else {
                photocraft_compose::mask_paint::refresh_damage_in_canvas(mask, damage, canvas);
            }
        }
    }

    fn refresh_mask_preview_regions_now(&mut self, damage: &[Rect]) {
        for region in damage {
            self.refresh_mask_preview_now(*region);
        }
    }

    /// True while this live stroke is taking the opt-in round-mask GPU route. The GPU renderer is
    /// owned by UI; engine only retains authoritative points and DabGenerator output.
    pub fn native_mask_gpu_brush_active(&self) -> bool {
        self.native_mask_gpu_generator.is_some()
    }

    /// The unmodified active Paint surface to use as the GPU preview's base.
    pub fn native_mask_gpu_base_surface(&self) -> Option<&Surface> {
        self.native_mask_gpu_generator.as_ref().map(|_| &self.pre)
    }

    /// Settings for the narrow grayscale round-brush GPU path.
    pub fn native_mask_gpu_brush_settings(&self) -> Option<(f32, f32, [f32; 4])> {
        self.native_mask_gpu_generator.as_ref().map(|_| (self.brush.hardness, self.brush.opacity, self.brush.color))
    }

    /// Drain dabs produced since the last call. Every value comes from the same DabGenerator that
    /// the normal renderer uses, including pressure, transfer dynamics, smoothing and spacing.
    pub fn take_native_mask_gpu_dabs(&mut self) -> Vec<NativeMaskGpuDab> {
        if self.native_mask_gpu_generator.is_none() {
            return Vec::new();
        }
        std::mem::take(&mut self.native_mask_gpu_dabs)
    }

    /// Finish smoothing/build-up and drain final dabs before the one release readback.
    pub fn finish_native_mask_gpu_dabs(&mut self) -> Vec<NativeMaskGpuDab> {
        let Some(generator) = self.native_mask_gpu_generator.as_mut() else { return Vec::new() };
        let (dabs, dual) = {
            let (mut dabs, mut dual) = (Vec::new(), Vec::new());
            generator.finish(&mut dabs, &mut dual);
            (dabs, dual)
        };
        if !dual.is_empty() || dabs.len() > 65_536 {
            let _ = self.fallback_native_mask_gpu_to_cpu();
            return Vec::new();
        }
        let Some(damage) = self.append_native_mask_gpu_dabs(&dabs) else {
            let _ = self.fallback_native_mask_gpu_to_cpu();
            return Vec::new();
        };
        self.native_mask_gpu_damage = self.native_mask_gpu_damage.union(&damage);
        std::mem::take(&mut self.native_mask_gpu_dabs)
    }

    fn append_native_mask_gpu_dabs(&mut self, dabs: &[Dab]) -> Option<Rect> {
        const MAX_PENDING_DABS: usize = 65_536;
        if self.native_mask_gpu_dabs.len().checked_add(dabs.len())? > MAX_PENDING_DABS {
            return None;
        }
        let mut damage = Rect::EMPTY;
        for dab in dabs {
            let converted = native_mask_gpu_dab(*dab)?;
            let reach = photocraft_paint::tip_reach(converted.radius, self.brush.hardness).max(converted.radius) + 1.0;
            let floor = |value: f32| value.floor().clamp(i32::MIN as f32, i32::MAX as f32) as i32;
            let ceil = |value: f32| value.ceil().clamp(i32::MIN as f32, i32::MAX as f32) as i32;
            let rect = Rect::new(
                floor(converted.center[0] - reach),
                floor(converted.center[1] - reach),
                ceil(converted.center[0] + reach),
                ceil(converted.center[1] + reach),
            )
            .intersect(&self.doc.bounds());
            damage = damage.union(&rect);
            self.native_mask_gpu_dabs.push(converted);
        }
        Some(damage)
    }

    /// Apply full-result RGBA8 patches returned by UI's single GPU release readback to the
    /// transient document. All patches validate before any write; canonical Session history is
    /// still created by the matching `paint.stroke` LiveCommit.
    pub fn apply_native_mask_gpu_patches(&mut self, patches: &[NativeMaskGpuPatch]) -> Result<Rect> {
        if self.native_mask_gpu_generator.is_none() || self.native_mask_gpu_patch_applied {
            return Err(bad(&self.cmd, "there is no pending native mask GPU stroke to patch"));
        }
        let bounds = self.doc.bounds();
        let mut total_bytes = 0u64;
        for patch in patches {
            let pixels =
                u64::from(patch.rect.width()).checked_mul(u64::from(patch.rect.height())).ok_or_else(|| bad(&self.cmd, "GPU mask patch size overflow"))?;
            let bytes = pixels.checked_mul(4).ok_or_else(|| bad(&self.cmd, "GPU mask patch byte size overflow"))?;
            if patch.rect.is_empty() || patch.rect.intersect(&bounds) != patch.rect || u64::try_from(patch.pixels_rgba8.len()).ok() != Some(bytes) {
                return Err(bad(&self.cmd, "GPU mask patch has an invalid rectangle or byte count"));
            }
            total_bytes = total_bytes.checked_add(bytes).ok_or_else(|| bad(&self.cmd, "GPU mask patch byte size overflow"))?;
            if total_bytes > 64 * 1024 * 1024 {
                return Err(bad(&self.cmd, "GPU mask patch exceeds the bounded readback limit"));
            }
        }
        let (surface, _) = crate::channel_cmds::target_surface(std::sync::Arc::make_mut(&mut self.doc), self.layer, &self.params)?;
        if surface.format() != photocraft_color::PixelFormat::RGBA8 {
            return Err(bad(&self.cmd, "GPU mask patch format does not match the active Paint surface"));
        }
        let mut damage = Rect::EMPTY;
        for patch in patches {
            let width = patch.rect.width() as usize;
            let mut samples = Vec::with_capacity(width * 4);
            for y in patch.rect.y0..patch.rect.y1 {
                let row_start = (y - patch.rect.y0) as usize * width * 4;
                let row = &patch.pixels_rgba8[row_start..row_start + width * 4];
                samples.clear();
                samples.extend(row.iter().map(|sample| f32::from(*sample) / 255.0));
                surface.write_region(Rect::new(patch.rect.x0, y, patch.rect.x1, y + 1), &samples);
            }
            damage = damage.union(&patch.rect);
        }
        if damage.is_empty() {
            return Err(bad(&self.cmd, "GPU mask release readback contained no changed pixels"));
        }
        self.native_mask_gpu_damage = self.native_mask_gpu_damage.union(&damage);
        self.native_mask_gpu_patch_applied = true;
        self.native_mask_gpu_generator = None;
        self.native_mask_gpu_dabs.clear();
        Ok(damage)
    }

    /// Switch back to the deterministic CPU path and replay every original input point. The GPU
    /// texture is owned by the caller and should be discarded as soon as this returns.
    pub fn fallback_native_mask_gpu_to_cpu(&mut self) -> Result<Rect> {
        if self.native_mask_gpu_generator.is_none() {
            return Ok(Rect::EMPTY);
        }
        self.native_mask_gpu_generator = None;
        self.native_mask_gpu_dabs.clear();
        self.native_mask_gpu_patch_applied = false;
        self.native_mask_gpu_damage = Rect::EMPTY;
        let cmyk = self.cmyk.clone();
        photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
            let (surface, _) = crate::channel_cmds::target_surface(std::sync::Arc::make_mut(&mut self.doc), self.layer, &self.params)?;
            let ends = self.breaks.iter().copied().chain(std::iter::once(self.points.len())).collect::<Vec<_>>();
            let mut start = 0;
            let mut damage = Rect::EMPTY;
            for end in ends {
                let segment = self.points.get(start..end).ok_or_else(|| bad(&self.cmd, "invalid live stroke segment boundary"))?;
                let mut renderer = StrokeRenderer::new(&self.brush, Some(self.pre.format()), self.zoom);
                renderer.push(segment);
                renderer.finish();
                damage = damage.union(&renderer.composite(&self.pre, surface, self.sel.as_ref(), self.lock, true));
                self.pre = surface.clone();
                start = end;
            }
            self.finished_bounds = self.finished_bounds.union(&damage);
            self.tail = Rect::EMPTY;
            self.union_live = None;
            self.mirror_distinct.fill(false);
            self.segment_break_pending = true;
            self.renderer = StrokeRenderer::new(&self.brush, Some(self.pre.format()), self.zoom);
            self.mirrors = (0..self.symmetry.as_ref().map_or(0, |s| s.mirror_count()))
                .map(|_| StrokeRenderer::new(&self.brush, Some(self.pre.format()), self.zoom))
                .collect();
            self.refresh_mask_preview(damage);
            Ok(damage)
        })
    }

    /// Render more points; returns the rectangle that changed. The doc shows the stroke as
    /// committing it now would: with smoothing, the brush lags behind the pointer and catches up
    /// when the stroke ends, so that catch-up tail is drawn too (and redrawn on every step), and
    /// nothing new appears on release.
    /// Invalid coordinates reject the whole batch without changing the preview.
    pub fn push(&mut self, pts: &[StrokePoint]) -> Result<Rect> {
        check_coords(pts, &self.cmd)?;
        if !pts.is_empty() && self.segment_break_pending {
            self.breaks.push(self.points.len());
            self.segment_break_pending = false;
        }
        self.points.extend_from_slice(pts);
        if self.native_mask_gpu_generator.is_some() {
            let (mut dabs, mut dual) = (Vec::new(), Vec::new());
            if let Some(generator) = self.native_mask_gpu_generator.as_mut() {
                generator.push(pts, &mut dabs, &mut dual);
            }
            if !dual.is_empty() {
                return self.fallback_native_mask_gpu_to_cpu();
            }
            let Some(damage) = self.append_native_mask_gpu_dabs(&dabs) else {
                return self.fallback_native_mask_gpu_to_cpu();
            };
            self.native_mask_gpu_damage = self.native_mask_gpu_damage.union(&damage);
            return Ok(damage);
        }
        let cmyk = self.cmyk.clone();
        let dmg = photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || -> Result<Rect> {
            self.renderer.push(pts);
            if let Some(symmetry) = &self.symmetry {
                for ((mirror, distinct), reflected) in self.mirrors.iter_mut().zip(&mut self.mirror_distinct).zip(symmetry.reflected_passes(pts)) {
                    *distinct |= crate::symmetry_cmds::SymmetryAxis::has_distinct_mirror(pts, &reflected);
                    mirror.push(&reflected);
                }
            }
            let (surf, _) = crate::channel_cmds::target_surface(std::sync::Arc::make_mut(&mut self.doc), self.layer, &self.params)?;
            if self.mirror_distinct.iter().any(|d| *d) {
                // The stroke and its distinct mirrors composite as one stroke, as the commit
                // merges them; only the tiles that changed are redrawn.
                let old = self.tail;
                let passes = 1 + self.mirror_distinct.iter().filter(|d| **d).count();
                let (mut live, full, mut dmg) = match self.union_live.take() {
                    Some((live, merged)) => (live, merged != passes, Rect::EMPTY),
                    None => {
                        // Leaving the single-pass preview: take its smoothing tail back first.
                        if !old.is_empty() {
                            surf.write_region(old, &self.pre.read_region(old));
                        }
                        (photocraft_paint::UnionLive::default(), true, old)
                    }
                };
                let mut renderers: Vec<&mut StrokeRenderer> = std::iter::once(&mut self.renderer)
                    .chain(self.mirrors.iter_mut().zip(&self.mirror_distinct).filter(|(_, d)| **d).map(|(r, _)| r))
                    .collect();
                dmg = dmg.union(&StrokeRenderer::composite_union_live(&mut renderers, &mut live, full, &self.pre, surf, self.sel.as_ref(), self.lock));
                self.tail = old.union(&live.bounds());
                self.union_live = Some((live, passes));
                self.refresh_mask_preview(dmg);
                return Ok(dmg);
            }
            let mut dmg = Rect::EMPTY;
            let old = std::mem::replace(&mut self.tail, Rect::EMPTY);
            if !old.is_empty() {
                // Back to the stroke without the previous tail.
                surf.write_region(old, &self.pre.read_region(old));
                self.renderer.mark_dirty(old);
                dmg = old;
            }
            dmg = dmg.union(&self.renderer.composite(&self.pre, surf, self.sel.as_ref(), self.lock, false));
            if let Some(mut tail) = self.renderer.tail_preview() {
                self.tail = tail.composite(&self.pre, surf, self.sel.as_ref(), self.lock, false);
                dmg = dmg.union(&self.tail);
            }
            self.refresh_mask_preview(dmg);
            Ok(dmg)
        })?;
        // The preview shows the pixels the commit keeps (see `stroke_with`).
        if let Some(table) = crate::mode_cmds::paint_color_table(&self.doc, &self.params) {
            let (surf, _) = crate::channel_cmds::target_surface(std::sync::Arc::make_mut(&mut self.doc), self.layer, &self.params)?;
            crate::mode_cmds::keep_in_color_table(&table, &self.pre, surf, dmg);
        }
        Ok(dmg)
    }
}

fn mask_preview_cadence_due(last_refresh: Option<Instant>, now: Instant, cadence: Duration) -> bool {
    cadence.is_zero() || last_refresh.is_none_or(|last| now.saturating_duration_since(last) >= cadence)
}

fn pencil(s: &mut Session, p: &Value) -> Result<Value> {
    let pts = parse_points(p, "paint.pencil")?;
    let brush = pencil_brush(s, p)?;
    let label = if brush.erase && !clear_mode(p) { "Eraser" } else { "Pencil" };
    stroke_with(s, "paint.pencil", p, label, brush, pts, flag(p, "autoErase", false))
}

fn pct(p: &Value, k: &str, d: f32) -> f32 {
    num(p, k).unwrap_or(d).clamp(0.0, 100.0) / 100.0
}

pub mod mixer_live;
fn mixer_brush(s: &mut Session, p: &Value) -> Result<Value> {
    mixer_live::run(s, p)
}

/// `brush.mixer.load`: Load Brush fills the Mixer Brush's reservoir with the color. The next stroke
/// keeps that paint unless a different foreground is chosen first. Returns the paint the brush now
/// carries.
fn mixer_load(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.mixer.load";
    let paint = match p.get("color").filter(|v| !v.is_null()) {
        None => Some(s.tools.foreground),
        Some(Value::String(h)) => crate::commands::parse_hex(h),
        Some(Value::Array(a)) => {
            let c: Option<Vec<f32>> = a.iter().map(|x| x.as_f64().filter(|x| (0.0..=1.0).contains(x)).map(|x| x as f32)).collect();
            match c.as_deref() {
                Some(&[r, g, b]) => Some([r, g, b, 1.0]),
                Some(&[r, g, b, a]) => Some([r, g, b, a]),
                _ => None,
            }
        }
        Some(_) => None,
    };
    let paint = paint.ok_or_else(|| bad(cmd, "`color` must be \"#rrggbb\" or [r, g, b, a?] in 0..1"))?;
    s.tools.mixer.load(paint);
    s.tools.mixer.color = Some(s.tools.foreground);
    Ok(serde_json::to_value(&s.tools.mixer).unwrap_or_default())
}

/// `brush.mixer.clean`: Clean Brush empties the Mixer Brush. The next stroke starts empty unless
/// a different color is chosen first. Returns the paint the brush now carries.
fn mixer_clean(s: &mut Session, _: &Value) -> Result<Value> {
    s.tools.mixer.clean();
    s.tools.mixer.color = Some(s.tools.foreground);
    Ok(serde_json::to_value(&s.tools.mixer).unwrap_or_default())
}

/// `paint.mixerLoad`: ⌥-click with the Mixer Brush loads the reservoir from the canvas colour at
/// (`x`, `y`) (Photoshop's Current Brush Load), from the layers Sample All Layers names. The
/// foreground colour is left alone. Over transparency or off the canvas nothing is loaded.
fn mixer_load_pixel(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "paint.mixerLoad";
    let coord = |k: &str| {
        let v = p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite());
        v.ok_or_else(|| bad(cmd, format!("`{k}` must be a finite number (document pixels)")))
    };
    let (x, y) = (coord("x")?, coord("y")?);
    let size = crate::sample_cmds::size_param(cmd, p)?;
    let which = if flag(p, "sampleAllLayers", s.tools.brush.mixer.sample_all_layers) {
        crate::sample_cmds::SampleLayers::All
    } else {
        crate::sample_cmds::SampleLayers::Current
    };
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let [r, g, b, a] = crate::sample_cmds::sample_color(&d.doc, d.active_layer, x, y, size, which);
    if a.is_nan() || a <= 0.0 {
        return Ok(json!({"loaded": false}));
    }
    let foreground = s.tools.foreground;
    s.tools.mixer.load_sampled([r, g, b, 1.0], foreground);
    Ok(json!({"loaded": true, "color": [r, g, b, 1.0]}))
}

/// `paint.colorReplacement`'s stroke and settings. The replacement colour is the brush colour
/// (the foreground unless `color` says otherwise). It paints layer pixels only: a mask, alpha
/// channel or Quick Mask `target` is refused rather than recolouring the layer instead.
fn color_replacement_setup(s: &Session, p: &Value) -> Result<(Stroke, ReplaceSettings, photocraft_doc::LayerId)> {
    let cmd = "paint.colorReplacement";
    if let Some(t) = p.get("target").filter(|t| t.as_str() != Some("pixels")) {
        return Err(bad(cmd, format!("Color Replacement paints layer pixels only (`target` {t} is not \"pixels\")")));
    }
    let pts = parse_points(p, cmd)?;
    let brush = resolve_brush(s, p, cmd)?;
    fn parse<T: serde::de::DeserializeOwned>(p: &Value, cmd: &str, k: &str) -> Result<Option<T>> {
        p.get(k).cloned().map(serde_json::from_value).transpose().map_err(|e| bad(cmd, format!("`{k}`: {e}")))
    }
    let rs = ReplaceSettings {
        mode: parse(p, cmd, "mode")?.unwrap_or(ReplaceMode::Color),
        sampling: parse(p, cmd, "sampling")?.unwrap_or(Sampling::Continuous),
        limits: parse(p, cmd, "limits")?.unwrap_or(Limits::Contiguous),
        tolerance: pct(p, "tolerance", 30.0),
        anti_alias: flag(p, "antiAlias", true),
        color: brush.color,
        background: s.tools.background,
    };
    let id = layer_id(s, p)?;
    Ok((Stroke { brush, points: pts }, rs, id))
}

fn color_replacement(s: &mut Session, p: &Value) -> Result<Value> {
    let (stroke, rs, id) = color_replacement_setup(s, p)?;
    let dmg = s.edit("Color Replacement", |doc, _| {
        let sel = doc.selection.clone();
        let lock = doc.effective_locks(id).transparency;
        let surf = paint_surface(doc, id, &json!({}))?;
        Ok(apply_color_replacement(surf, &stroke, &rs, sel.as_ref(), lock))
    })?;
    Ok(damage_json(s, dmg))
}

/// A `paint.colorReplacement` stroke shown while it is drawn: the same params, and the same
/// recolouring per dab as the commit, so the preview at the last point is the committed result.
pub fn live_color_replacement(s: &Session, p: &Value) -> Result<crate::retouch_cmds::LiveDab> {
    let (stroke, rs, id) = color_replacement_setup(s, p)?;
    crate::retouch_cmds::LiveDab::with_effect(s, "paint.colorReplacement", &json!({}), stroke, Some(id), 0, |pre| {
        Box::new(color_replacement_effect(pre.clone(), rs))
    })?
    .with_tail_preview()
}

fn brush_json(b: &BrushSettings) -> Value {
    serde_json::to_value(b).unwrap_or(Value::Null)
}

fn presets_list(s: &mut Session, p: &Value) -> Result<Value> {
    let full = flag(p, "full", false);
    Ok(json!({
        "presets": s.tools.presets.iter().map(|pr| {
            let mut v = json!({
                "name": pr.name,
                "builtin": pr.builtin,
                "size": pr.brush.size,
                "hardness": pr.brush.hardness,
                "tip": match &pr.brush.tip { TipShape::Round => "round", TipShape::Sampled(_) | TipShape::Stored(_) => "sampled" },
            });
            if full {
                // Full settings embed the bitmaps: load stored tips (a tip that can't be loaded
                // stays a reference to the store).
                let mut b = pr.brush.clone();
                let _ = s.load_brush_tips(&mut b);
                v["brush"] = brush_json(&b);
            }
            v
        }).collect::<Vec<_>>()
    }))
}

fn name_param(p: &Value, cmd: &str) -> Result<String> {
    p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).ok_or_else(|| bad(cmd, "missing `name`"))
}

/// Replace the preset of that name, or append it at the end of the list.
fn upsert(s: &mut Session, preset: BrushPreset) {
    match s.tools.presets.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&preset.name)) {
        Some(x) => *x = preset,
        None => s.tools.presets.push(preset),
    }
    s.brush_presets_changed();
}

fn presets_save(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.save";
    let name = name_param(p, cmd)?;
    let mut brush = match p.get("brush").filter(|v| v.is_object()) {
        Some(patch) => merge_brush(&s.tools.brush, patch, cmd)?,
        None => s.tools.brush.clone(),
    };
    // The New Brush Preset dialog's boxes; without them the preset captures what it always did.
    let captures = PresetCaptures { size: flag(p, "captureSize", true), tool_settings: flag(p, "toolSettings", true), color: flag(p, "color", false) };
    if captures.color {
        brush.color = s.tools.foreground;
    }
    upsert(s, BrushPreset { name: name.clone(), brush, builtin: false, group: String::new(), folder: Vec::new(), captures });
    // The saved brush is the current brush now (Photoshop selects the brush it just saved).
    s.tools.current_preset = Some(name.clone());
    s.tools.brush_changed();
    Ok(json!({ "name": name, "count": s.tools.presets.len() }))
}

fn presets_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.delete";
    let name = name_param(p, cmd)?;
    let before = s.tools.presets.len();
    s.tools.presets.retain(|x| !x.name.eq_ignore_ascii_case(&name));
    if s.tools.presets.len() == before {
        return Err(bad(cmd, format!("no brush preset named `{name}`")));
    }
    forget_deleted_preset(s, &name);
    Ok(json!({ "count": s.tools.presets.len() }))
}

/// After preset `name` was deleted (here or in Edit › Presets › Preset Manager): the current
/// brush no longer comes from a preset, and the library is saved.
pub(crate) fn forget_deleted_preset(s: &mut Session, name: &str) {
    if s.tools.current_preset.as_ref().is_some_and(|c| c.eq_ignore_ascii_case(name)) {
        s.tools.current_preset = None;
        // The remembered brush names the preset it was picked from: the name went away.
        s.tools.brush_changed();
    }
    s.brush_presets_changed();
}

/// Overwrite an existing preset with the current brush (or the current brush plus a `brush`
/// patch), in place: no new preset, the position, name and group stay. The preset defaults to the
/// one the current brush was picked from. A built-in that is updated becomes the user's preset
/// (built-ins regenerate with their factory settings).
fn presets_update(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.update";
    let name = match p.get("name").filter(|v| !v.is_null()) {
        Some(_) => name_param(p, cmd)?,
        None => s.tools.current_preset.clone().ok_or_else(|| bad(cmd, "the current brush was not picked from a preset"))?,
    };
    let brush = match p.get("brush").filter(|v| v.is_object()) {
        Some(patch) => merge_brush(&s.tools.brush, patch, cmd)?,
        None => s.tools.brush.clone(),
    };
    let pr = s.tools.presets.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&name)).ok_or_else(|| bad(cmd, format!("no brush preset named `{name}`")))?;
    pr.brush = brush;
    pr.builtin = false;
    let (name, group) = (pr.name.clone(), pr.group.clone());
    s.brush_presets_changed();
    Ok(json!({ "name": name, "group": group }))
}

/// Mark a preset as the one the current brush was picked from. The tip grids' click copies only
/// the tip fields (the rest of the brush stays), so the engine cannot infer the preset from the
/// brush: the UI names it. Selection is by identity, so look-alike duplicates stay distinct.
fn presets_set_current(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.setCurrent";
    let name = name_param(p, cmd)?;
    if !s.tools.presets.iter().any(|x| x.name.eq_ignore_ascii_case(&name)) {
        return Err(bad(cmd, format!("no brush preset named `{name}`")));
    }
    s.tools.current_preset = Some(name.clone());
    s.tools.brush_changed();
    Ok(json!({ "name": name }))
}

fn has_selection_and_pixels(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document")?;
    if d.doc.selection.is_none() {
        return Err("no selection".into());
    }
    let l = d.active_layer.and_then(|id| d.doc.layer(id)).ok_or("no active layer")?;
    if matches!(l.content, LayerContent::Raster(_)) { Ok(()) } else { Err("active layer is not a pixel layer".into()) }
}

/// Largest sampled tip side (like Photoshop's 5000 px limit, kept smaller for preset size).
pub const MAX_TIP: u32 = 2500;

fn define_from_selection(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.defineFromSelection";
    let name = name_param(p, cmd)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let sel = d.doc.selection.as_ref().ok_or_else(|| bad(cmd, "no selection"))?;
    let id = d.active_layer.ok_or(EngineError::Other("no active layer".into()))?;
    let surf = d.doc.layer(id).and_then(|l| l.surface()).ok_or_else(|| bad(cmd, "not a pixel layer"))?;
    let area = sel.content_bounds().intersect(&d.doc.bounds());
    if area.is_empty() {
        return Err(bad(cmd, "the selection is empty"));
    }
    // Paint amount = darkness × alpha × selection (black = full paint, like Photoshop).
    let (w, h) = (area.width() as usize, area.height() as usize);
    // The sampled tip is one f32 buffer over the whole selection; a large selection must fail
    // gracefully instead of aborting the process on allocation, so reserve it fallibly. The
    // multiply is checked too: on a 32-bit target (wasm32) it can overflow where the rest of
    // this command is already fallible.
    let len = w.checked_mul(h).ok_or_else(|| crate::allocation::error("defining a brush from the selection", "requested size overflowed"))?;
    let mut v = crate::allocation::filled(len, 0.0, "defining a brush from the selection")?;
    let sc = sel.channels();
    // The selection mask is a second buffer of the same size; it must fail gracefully too
    // (review of #5469), so read it through the fallible allocator.
    let sv = crate::allocation::read_region(sel, area, "defining a brush from the selection")?;
    for y in 0..h {
        for x in 0..w {
            let c = surf.rgba(area.x0 + x as i32, area.y0 + y as i32);
            let l = photocraft_color::blend::lum([c[0], c[1], c[2]]);
            v[y * w + x] = ((1.0 - l) * c[3] * sv[(y * w + x) * sc]).clamp(0.0, 1.0);
        }
    }
    let (brush, tw, th) = sampled_brush(cmd, &v, w, h)?;
    upsert(s, BrushPreset { name: name.clone(), brush: brush.clone(), builtin: false, group: String::new(), folder: Vec::new(), captures: Default::default() });
    s.tools.brush = brush;
    s.tools.current_preset = Some(name.clone());
    s.tools.brush_changed();
    Ok(json!({ "name": name, "width": tw, "height": th }))
}

/// A sampled-tip brush from a paint-amount map (`w`×`h`, row-major, 0 = no paint, 1 = full):
/// cropped to the painted extent, at most [`MAX_TIP`] px a side. Shared by Define Brush Preset
/// and importing an image file as a brush (`brush.presets.importImage`).
pub(crate) fn sampled_brush(cmd: &str, v: &[f32], w: usize, h: usize) -> Result<(BrushSettings, u32, u32)> {
    let at = |x: usize, y: usize| v.get(y * w + x).copied().unwrap_or(0.0);
    // Crop to the painted extent.
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if at(x, y) > 1e-4 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return Err(bad(cmd, "the pixels contain no paint (all white or transparent)"));
    }
    let (tw, th) = ((x1 - x0) as u32, (y1 - y0) as u32);
    if tw.max(th) > MAX_TIP {
        return Err(bad(cmd, format!("too large for a brush tip ({tw}×{th}; max {MAX_TIP} px)")));
    }
    let mut data = Vec::with_capacity((tw * th) as usize);
    for y in y0..y1 {
        data.extend((x0..x1).map(|x| at(x, y)));
    }
    let tip = GrayTile::from_f32(tw, th, &data);
    let brush = BrushSettings { tip: TipShape::Sampled(tip), size: tw.max(th) as f32, spacing: 0.25, pressure_size: false, ..BrushSettings::default() };
    Ok((brush, tw, th))
}

pub(crate) fn set_brush(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "tools.setBrush";
    let mut b = s.tools.brush.clone();
    let mut picked = None;
    let mut picked_color = None;
    if let Some(name) = p.get("preset").and_then(Value::as_str) {
        let preset = find_preset(s, name, cmd)?;
        b = preset.picked_over(&s.tools.brush);
        picked_color = preset.captures.color.then_some(preset.brush.color);
        picked = Some(name.to_string());
    }
    let reset = flag(p, "reset", false);
    if reset {
        b = BrushSettings::default();
    }
    let mut patch = p.clone();
    if let Some(o) = patch.as_object_mut() {
        for k in ["preset", "reset", "coalesce"] {
            o.remove(k);
        }
        if let Some(inner) = o.remove("brush") {
            let tmp = merge_brush(&b, &inner, cmd)?;
            // Before the second merge: a non-finite value would serialise as `null` there and be
            // reported as a type error instead of naming the value (#977).
            validate_brush(&tmp, cmd)?;
            b = tmp;
        }
    }
    b = merge_brush(&b, &patch, cmd)?;
    validate_brush(&b, cmd)?;
    // The picked preset stays "the current brush" across later edits, so `brush.presets.update`
    // knows what to overwrite. Only once the brush is accepted: a failed call leaves no state
    // behind (a `reset` wins over a `preset`, as it replaces the whole brush).
    if reset {
        s.tools.current_preset = None;
    } else if let Some(name) = picked {
        s.tools.current_preset = Some(name);
        // A preset saved with Include Color brings its colour as the foreground (Photoshop).
        if let Some(c) = picked_color {
            s.tools.foreground = c;
        }
    }
    // The tool's brush always paints with full tips: load a library preset's from the store.
    s.load_brush_tips(&mut b).map_err(|e| bad(cmd, e))?;
    let before = std::mem::replace(&mut s.tools.brush, b);
    // A coalesced gesture (one slider drag) journals as one call: remember the brush it started from.
    let key = p.get("coalesce").and_then(Value::as_str).filter(|_| p.get("preset").is_none() && p.get("reset").is_none());
    let continuing = |s: &Session, k: &str| {
        s.tools.brush_gesture.as_ref().is_some_and(|(g, _)| g == k)
            && s.journal.last().is_some_and(|(id, lp)| id == cmd && lp.get("coalesce").and_then(Value::as_str) == Some(k))
    };
    match key {
        Some(k) if continuing(s, k) => {}
        Some(k) => s.tools.brush_gesture = Some((k.to_string(), before)),
        None => s.tools.brush_gesture = None,
    }
    // The one accepted brush change everyone (UI sliders, control channel, presets) lands here.
    s.tools.brush_changed();
    Ok(brush_json(&s.tools.brush))
}

/// Remember the current brush in `Preferences::last_brush` (see [`remembered_brush`]). Called
/// every frame while the pointer is up, so it is change-driven: `ToolState::brush_rev` is compared
/// with the revision already saved and an unchanged brush clones and serialises nothing.
pub fn remember_brush(s: &mut Session) {
    if s.tools.brush_rev == s.tools.brush_saved_rev {
        return;
    }
    let Some(now) = remembered_brush(s) else { return };
    if s.prefs().last_brush != now {
        s.prefs.edit(|p| p.last_brush = now);
    }
    s.tools.brush_saved_rev = s.tools.brush_rev;
}

/// The current brush as JSON for `Preferences::last_brush`: settings plus tip references, no full
/// bitmap. `None` while a sampled bitmap too big to embed is current (only before the store
/// externalizes it; see [`ref_tip`]), so the previous remembered brush stands.
fn remembered_brush(s: &Session) -> Option<Value> {
    Some(json!({ "preset": s.tools.current_preset.clone(), "brush": remembered_settings(&s.tools.brush)? }))
}

/// A brush as JSON for the preferences (the last brush, each painting tool's own brush): its
/// settings with stored tips as references, never their full bitmaps. `None` while a sampled
/// bitmap too big to embed is in it (only before the preset store externalizes it).
pub fn remembered_settings(b: &BrushSettings) -> Option<Value> {
    let (tip, dual, pattern) = (ref_tip(&b.tip)?, ref_tip(&b.dual_brush.tip)?, ref_pattern(&b.texture.pattern)?);
    let mut c = b.clone();
    (c.tip, c.dual_brush.tip, c.texture.pattern) = (tip, dual, pattern);
    serde_json::to_value(&c).ok()
}

/// A brush remembered by [`remembered_settings`], read back at launch: `None` when the value is
/// not a brush or fails the checks `tools.setBrush` makes (a size or scatter out of range). The
/// options-bar values are clamped to the ranges their controls allow (size 1..max px; hardness,
/// opacity, flow and smoothing 0..100 %), and stored tips load when the store is attached (else
/// they paint with their previews until `Session::attach_preset_store`).
pub fn restored_settings(s: &Session, v: &Value) -> Option<BrushSettings> {
    if !v.is_object() {
        return None;
    }
    let mut b = merge_brush(&BrushSettings::default(), v, "brush.restore").ok()?;
    validate_brush(&b, "brush.restore").ok()?;
    b.size = b.size.clamp(1.0, photocraft_paint::MAX_BRUSH_SIZE);
    for f in [&mut b.hardness, &mut b.opacity, &mut b.flow, &mut b.smoothing.amount] {
        *f = if f.is_nan() { 1.0 } else { f.clamp(0.0, 1.0) };
    }
    let _ = s.load_brush_tips(&mut b);
    Some(b)
}

/// Restore `Preferences::last_brush` into the session at launch (a missing or malformed value
/// keeps the default brush). Stored tips load when the preset store attaches.
pub fn restore_brush(s: &mut Session) {
    let v = s.prefs().last_brush.clone();
    let Some(b) = v.get("brush").and_then(|b| restored_settings(s, b)) else { return };
    s.tools.brush = b;
    s.tools.current_preset = v.get("preset").and_then(Value::as_str).map(str::to_string);
    // What was just restored is exactly what the preferences hold: nothing new to write.
    s.tools.brush_saved_rev = s.tools.brush_rev;
}

/// A tip for remembering: a loaded stored tip drops its full bitmap (it serialises as a reference
/// then); a sampled bitmap bigger than a preview would enter the prefs document whole.
fn ref_tip(t: &TipShape) -> Option<TipShape> {
    match t {
        TipShape::Stored(r) if r.full.is_some() => Some(TipShape::Stored(StoredTile { full: None, ..r.clone() })),
        TipShape::Sampled(g) if g.width > PREVIEW_SIDE || g.height > PREVIEW_SIDE => None,
        t => Some(t.clone()),
    }
}

fn ref_pattern(p: &Pattern) -> Option<Pattern> {
    match p {
        Pattern::Stored(r) if r.full.is_some() => Some(Pattern::Stored(StoredTile { full: None, ..r.clone() })),
        Pattern::Tile(g) if g.width > PREVIEW_SIDE || g.height > PREVIEW_SIDE => None,
        p => Some(p.clone()),
    }
}

/// The brush as JSON with the bitmaps (sampled tips, pattern tiles) that `skip` names replaced by
/// placeholders: they can be megabytes of base64, and skipped ones are equal on both sides.
fn light_json(b: &BrushSettings, skip: (bool, bool, bool)) -> Value {
    let mut c = b.clone();
    if skip.0 {
        c.tip = TipShape::Round;
    }
    if skip.1 {
        c.dual_brush.tip = TipShape::Round;
    }
    if skip.2 {
        c.texture.pattern = photocraft_paint::Pattern::default();
    }
    serde_json::to_value(&c).unwrap_or(Value::Null)
}

/// Fields of `new` that differ from `old`, nested objects diffed key by key: the minimal
/// `tools.setBrush` `brush` patch that turns `old` into `new` (`{}` when nothing changed).
pub fn brush_patch(old: &BrushSettings, new: &BrushSettings) -> Value {
    let skip = (old.tip == new.tip, old.dual_brush.tip == new.dual_brush.tip, old.texture.pattern == new.texture.pattern);
    fn diff(a: &Value, b: &Value) -> Option<Value> {
        match (a, b) {
            (Value::Object(ao), Value::Object(bo)) => {
                let mut out = serde_json::Map::new();
                for (k, bv) in bo {
                    match ao.get(k) {
                        Some(av) => {
                            if let Some(d) = diff(av, bv) {
                                out.insert(k.clone(), d);
                            }
                        }
                        None => {
                            out.insert(k.clone(), bv.clone());
                        }
                    }
                }
                (!out.is_empty()).then_some(Value::Object(out))
            }
            // Enums with payloads (tips, patterns) are replaced whole.
            _ => (a != b).then(|| b.clone()),
        }
    }
    let mut patch = diff(&light_json(old, skip), &light_json(new, skip)).unwrap_or_else(|| json!({}));
    // This readback is derived from the axis flags. Sending it alongside a
    // one-axis delta would also overwrite the other axis through the legacy alias.
    if let Some(pose) = patch.get_mut("pose").and_then(Value::as_object_mut) {
        pose.remove("overrideTilt");
    }
    patch
}

/// Journal hook ([`Session::execute`]): consecutive `tools.setBrush` calls with the same
/// `coalesce` key (one drag in the options bar or the Brush Settings panel) are one journal
/// entry, the patch from the brush before the gesture to the brush now. Returns true when the
/// call was folded into the previous entry.
pub fn coalesce_journal(s: &mut Session, id: &str, params: &Value) -> bool {
    if id != "tools.setBrush" {
        return false;
    }
    let Some(key) = params.get("coalesce").and_then(Value::as_str) else { return false };
    let Some((gk, start)) = s.tools.brush_gesture.as_ref() else { return false };
    if gk != key {
        return false;
    }
    let patch = brush_patch(start, &s.tools.brush);
    match s.journal.last_mut() {
        Some((last, lp)) if last == id && lp.get("coalesce").and_then(Value::as_str) == Some(key) => {
            *lp = json!({ "brush": patch, "coalesce": key });
            true
        }
        _ => false,
    }
}

macro_rules! spec {
    ($id:literal, $label:literal, $params:literal, $en:expr, $run:expr, $journal:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[], shortcut: None, params: $params, enabled: $en, run: $run, journal: $journal }
    };
}

/// Brush command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "brush.mixer.sample",
            "Sample Mixer Brush",
            r#"{"points":[[x,y,pressure?,tiltX?,tiltY?,rotation?]],"sampleAllLayers":bool=brush setting,"size":px?,"target":"pixels"|"mask"|"quickMask"|{"channel":i}}"#,
            |s| s.active().map(|_| ()).ok_or_else(|| "no document".into()),
            mixer_live::sample,
            true
        ),
        spec!(
            "paint.pencil",
            "Pencil",
            r##"{"points":[[x,y,pressure?,tiltX?,tiltY?,rotation?,timeMs?,wheel?],…],"brush":{…}?,"preset":name?,"size":0.5..5000 px?,"opacity":0..1?,"color":"#rrggbb"?=foreground,"mode":"normal|multiply|screen|…|behind (paints beneath existing pixels)|clear (erases with this brush)"="normal","erase":bool?,"block":bool=false (a hard 16×16 screen-px square, 16/zoom doc px; size, opacity, flow, hardness and dynamics ignored: the Eraser's Block mode),"zoom":view zoom=1,"autoErase":bool=false,"seed":u64?,"target":"pixels"|"mask"|"filterMask"|"quickMask"|{"channel":i}=Channels panel target}"##,
            has_paintable,
            pencil,
            true
        ),
        spec!(
            "paint.mixerBrush",
            "Mixer Brush",
            r##"{"points":[…] (long segments are interpolated),"brush":{…}?,"preset":name?,"size":0.5..5000 px?,"wet":0..100=brush.mixer.wet,"load":0..100=brush.mixer.load,"mix":0..100=brush.mixer.mix,"flow":0..100=brush.mixer.flow,"color":"#rrggbb"?=foreground,"sampleAllLayers":bool=brush.mixer.sampleAllLayers,"cleanAfterStroke":bool=brush.mixer.cleanAfterStroke,"loadAfterStroke":bool=brush.mixer.loadAfterStroke,"seed":u64?,"mixerState":{reservoir,amount,pickup,spatial,color} (optional captured replay state; spatial is null or 4096 RGBA colors)}"##,
            has_paintable,
            mixer_brush,
            true
        ),
        spec!("brush.mixer.load", "Load Mixer Brush", r##"{"color":"#rrggbb"|[r,g,b,a?] 0..1?=foreground}"##, always, mixer_load, true),
        spec!("brush.mixer.clean", "Clean Mixer Brush", "{}", always, mixer_clean, true),
        spec!(
            "paint.mixerLoad",
            "Load Mixer Brush",
            r##"{"x":px,"y":px,"size":"point"|1|3|5|11|31|51|101=1,"sampleAllLayers":bool=brush.mixer.sampleAllLayers} → {"loaded":bool,"color"?:[r,g,b,a]} (loads the Mixer Brush reservoir from the canvas colour there, as ⌥-click does; the foreground colour is unchanged; the next strokes keep that load while the foreground stays as it is)"##,
            has_paintable,
            mixer_load_pixel,
            true
        ),
        spec!(
            "paint.colorReplacement",
            "Color Replacement",
            r##"{"points":[…],"brush":{…}?,"size":px?,"mode":"hue|saturation|color|luminosity"="color","sampling":"continuous|once|backgroundSwatch"="continuous","limits":"contiguous|discontiguous|findEdges"="contiguous","tolerance":0..100=30,"antiAlias":bool=true,"color":"#rrggbb"?=foreground,"target":"pixels"="pixels","seed":u64?}"##,
            crate::commands::has_paintable,
            color_replacement,
            true
        ),
        spec!("brush.presets.list", "List Brush Presets", r##"{"full":bool=false}"##, always, presets_list, false),
        spec!(
            "brush.presets.save",
            "Save Brush Preset",
            r##"{"name":string,"brush":{…BrushSettings}?=current brush,"captureSize":bool=true (false: picking it keeps the tool's size),"toolSettings":bool=true (false: picking it keeps the tool's mode, opacity, flow and pressure buttons),"color":bool=false (picking it sets the foreground to the current foreground)}"##,
            always,
            presets_save,
            true
        ),
        spec!(
            "brush.presets.update",
            "Update Brush Preset",
            r##"{"name":string?=the preset the current brush was picked from,"brush":{…BrushSettings}?=current brush}"##,
            always,
            presets_update,
            true
        ),
        spec!("brush.presets.delete", "Delete Brush Preset", r##"{"name":string}"##, always, presets_delete, true),
        spec!("brush.presets.setCurrent", "Set Current Brush Preset", r##"{"name":string}"##, always, presets_set_current, true),
        spec!("brush.defineFromSelection", "Define Brush Preset…", r##"{"name":string}"##, has_selection_and_pixels, define_from_selection, true),
        spec!("brush.get", "Get Brush", "{}", always, |s, _| Ok(brush_json(&s.tools.brush)), false),
        spec!(
            "tools.setBrush",
            "Set Brush",
            r##"{"preset":name?,"reset":bool?,…BrushSettings fields (camelCase, deep-merged; pose.overrideTiltX/Y:bool; pose.overrideTilt:legacy combined alias)}"##,
            always,
            set_brush,
            true
        ),
    ]
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod mixer_live_tests;
