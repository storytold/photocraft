//! Brush engine commands: the Brush/Pencil/Mixer Brush/Color Replacement tools, brush presets and
//! the session brush (`tools.setBrush`). The UI's options bar and Brush Settings panel call these.
//!
//! Brushes are [`BrushSettings`] serialised as camelCase JSON; any command taking a `"brush"`
//! object deep-merges it onto the session brush, so partial objects work
//! (`{"scattering": {"enabled": true}}`). Strokes are deterministic: the jitter seed is the `seed`
//! param or a hash of the points, so a journaled command replays to identical pixels.

use photocraft_doc::LayerContent;
use photocraft_geom::Rect;
use photocraft_paint::mixer::{MixerSettings, apply_mixer_stroke};
use photocraft_paint::replace::{Limits, ReplaceMode, ReplaceSettings, Sampling, apply_color_replacement};
use photocraft_paint::{BrushPreset, BrushSettings, GrayTile, Stroke, StrokePoint, TipShape, render_stroke};
use photocraft_raster::Surface;
use serde_json::{Value, json};

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
    Ok(pts)
}

/// Deep-merge `patch` into `base` (objects merge key by key; anything else replaces).
fn merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
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
    let mut v = serde_json::to_value(base).map_err(|e| bad(cmd, e.to_string()))?;
    merge(&mut v, patch);
    serde_json::from_value(v).map_err(|e| bad(cmd, format!("invalid brush: {e}")))
}

fn find_preset<'a>(s: &'a Session, name: &str, cmd: &str) -> Result<&'a BrushPreset> {
    photocraft_paint::presets::find(&s.tools.presets, name).ok_or_else(|| bad(cmd, format!("no brush preset named `{name}`")))
}

/// Resolve the brush for a stroke command: session brush ← `preset` ← `brush` ← legacy scalar
/// params; colours from the session; seed from `seed` or the points.
pub fn resolve_brush(s: &Session, p: &Value, cmd: &str) -> Result<BrushSettings> {
    let mut b = s.tools.brush.clone();
    if let Some(name) = p.get("preset").and_then(Value::as_str) {
        b = find_preset(s, name, cmd)?.brush.clone().with_protected_texture(&s.tools.brush);
    }
    if let Some(patch) = p.get("brush").filter(|v| v.is_object()) {
        b = merge_brush(&b, patch, cmd)?;
    }
    if let Some(v) = num(p, "size") {
        b.size = v.max(0.5);
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
        b.smoothing.amount = v.clamp(0.0, 1.0);
    }
    b.color = color(p.get("color"), s.tools.foreground);
    b.background = s.tools.background;
    b.erase = flag(p, "erase", false);
    b.seed = match p.get("seed").and_then(Value::as_u64) {
        Some(v) => v,
        None => photocraft_paint::rng::seed_from_bytes(p.get("points").map(|v| v.to_string()).unwrap_or_default().as_bytes()),
    };
    Ok(b)
}

fn layer_id(s: &Session, p: &Value) -> Result<photocraft_doc::LayerId> {
    match p.get("layer").and_then(Value::as_u64) {
        Some(id) => Ok(photocraft_doc::LayerId(id)),
        None => s.active().and_then(|d| d.active_layer).ok_or(EngineError::Other("no active layer".into())),
    }
}

fn damage_json(s: &mut Session, dmg: Rect) -> Value {
    if let Some(st) = s.active_mut() {
        st.last_damage = Some(dmg);
    }
    json!({ "damage": [dmg.x0, dmg.y0, dmg.width(), dmg.height()] })
}

/// Stroke with a resolved brush onto the target layer (pixels or mask).
fn stroke_with(s: &mut Session, p: &Value, label: &str, brush: BrushSettings, pts: Vec<StrokePoint>, auto_erase: bool) -> Result<Value> {
    let gray = is_mask_target(p) || crate::channel_cmds::is_channel_target(p);
    let id = if crate::channel_cmds::is_channel_target(p) { None } else { Some(layer_id(s, p)?) };
    let zoom = num(p, "zoom").unwrap_or(1.0);
    let bg = s.tools.background;
    let fg = brush.color;
    // On a mask or channel the eraser paints the background colour (Photoshop).
    let brush = if gray && brush.erase { BrushSettings { erase: false, color: bg, ..brush } } else { brush };
    let dmg = s.edit(label, |doc, _| {
        let sel = doc.selection.clone();
        let (surf, lock) = crate::channel_cmds::target_surface(doc, id, p)?;
        let mut brush = brush;
        // Erasing a layer with locked transparency (e.g. the Background) can't remove opacity, so
        // it paints the background colour instead (Photoshop). `gray` targets are handled above.
        if brush.erase && lock {
            brush.erase = false;
            brush.color = bg;
        }
        if auto_erase {
            // Pencil Auto Erase: starting on foreground-coloured pixels paints the background colour.
            let c = surf.rgba(pts[0].x.floor() as i32, pts[0].y.floor() as i32);
            if c[3] > 0.0 && (0..3).all(|i| (c[i] - fg[i]).abs() < 1.5 / 255.0) {
                brush.color = bg;
            }
        }
        Ok(render_stroke(surf, &brush, &pts, sel.as_ref(), lock, zoom))
    })?;
    Ok(damage_json(s, dmg))
}

/// Applies the options-bar blend `mode` to a brush. `"mode"` accepts any blend-mode name
/// (normal|multiply|screen|…). The Eraser has no blend mode in Photoshop, so it is forced to Normal.
fn with_blend_mode(mut b: BrushSettings, p: &Value) -> BrushSettings {
    if b.erase {
        b.mode = photocraft_color::BlendMode::Normal;
    } else if let Some(m) = p.get("mode").and_then(Value::as_str).and_then(crate::commands::blend_from_str) {
        b.mode = m;
    }
    b
}

/// `paint.stroke`: the Brush (and Eraser) tool.
pub fn paint_stroke(s: &mut Session, p: &Value) -> Result<Value> {
    let pts = parse_points(p, "paint.stroke")?;
    let brush = with_blend_mode(resolve_brush(s, p, "paint.stroke")?, p);
    let label = if brush.erase { "Eraser" } else { "Brush Tool" };
    stroke_with(s, p, label, brush, pts, false)
}

fn pencil(s: &mut Session, p: &Value) -> Result<Value> {
    let pts = parse_points(p, "paint.pencil")?;
    let mut brush = with_blend_mode(resolve_brush(s, p, "paint.pencil")?, p);
    brush.aliased = true;
    if num(p, "hardness").is_none() {
        brush.hardness = 1.0;
    }
    let label = if brush.erase { "Eraser" } else { "Pencil" };
    stroke_with(s, p, label, brush, pts, flag(p, "autoErase", false))
}

fn pct(p: &Value, k: &str, d: f32) -> f32 {
    num(p, k).unwrap_or(d).clamp(0.0, 100.0) / 100.0
}

fn mixer_brush(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "paint.mixerBrush";
    let pts = parse_points(p, cmd)?;
    let brush = resolve_brush(s, p, cmd)?;
    let m = MixerSettings { wet: pct(p, "wet", 50.0), load: pct(p, "load", 50.0), mix: pct(p, "mix", 50.0), flow: pct(p, "flow", 100.0) };
    let sample_all = flag(p, "sampleAllLayers", false);
    let (clean, load_after) = (flag(p, "cleanAfterStroke", true), flag(p, "loadAfterStroke", true));
    let mut state = s.tools.mixer.clone();
    if load_after || state.reservoir.is_none() {
        state.load(color(p.get("color"), s.tools.foreground));
    }
    let id = if crate::channel_cmds::is_channel_target(p) { None } else { Some(layer_id(s, p)?) };
    let stroke = Stroke { brush, points: pts };
    let dmg = s.edit("Mixer Brush", |doc, _| {
        let sel = doc.selection.clone();
        let sample = if sample_all {
            let ds = photocraft_paint::dabs(&stroke);
            let ctx = photocraft_paint::BrushContext::new(&stroke.brush);
            let area = ds.iter().fold(Rect::EMPTY, |r, d| r.union(&ctx.dab_rect(d, false)));
            let buf = photocraft_compose::render(doc, area);
            let mut surf = Surface::new(photocraft_color::PixelFormat::RGBA32F);
            let flat: Vec<f32> = buf.px.iter().flatten().copied().collect();
            if !area.is_empty() {
                surf.write_region(area, &flat);
            }
            Some(surf)
        } else {
            None
        };
        let (surf, lock) = crate::channel_cmds::target_surface(doc, id, p)?;
        Ok(apply_mixer_stroke(surf, sample.as_ref(), &stroke, &m, &mut state, sel.as_ref(), lock))
    })?;
    if clean {
        state.clean();
    }
    s.tools.mixer = state;
    Ok(damage_json(s, dmg))
}

fn color_replacement(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "paint.colorReplacement";
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
    let stroke = Stroke { brush, points: pts };
    let dmg = s.edit("Color Replacement", |doc, _| {
        let sel = doc.selection.clone();
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let lock = l.locks.transparency;
        let surf = paint_surface(l, &json!({}))?;
        Ok(apply_color_replacement(surf, &stroke, &rs, sel.as_ref(), lock))
    })?;
    Ok(damage_json(s, dmg))
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
                "tip": match &pr.brush.tip { TipShape::Round => "round", TipShape::Sampled(_) => "sampled" },
            });
            if full {
                v["brush"] = brush_json(&pr.brush);
            }
            v
        }).collect::<Vec<_>>()
    }))
}

fn name_param(p: &Value, cmd: &str) -> Result<String> {
    p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).ok_or_else(|| bad(cmd, "missing `name`"))
}

fn upsert(s: &mut Session, preset: BrushPreset) {
    match s.tools.presets.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&preset.name)) {
        Some(x) => *x = preset,
        None => s.tools.presets.push(preset),
    }
}

fn presets_save(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.save";
    let name = name_param(p, cmd)?;
    let brush = match p.get("brush").filter(|v| v.is_object()) {
        Some(patch) => merge_brush(&s.tools.brush, patch, cmd)?,
        None => s.tools.brush.clone(),
    };
    upsert(s, BrushPreset { name: name.clone(), brush, builtin: false });
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
    Ok(json!({ "count": s.tools.presets.len() }))
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
    let mut v = vec![0.0f32; w * h];
    let sc = sel.channels();
    let sv = sel.read_region(area);
    for y in 0..h {
        for x in 0..w {
            let c = surf.rgba(area.x0 + x as i32, area.y0 + y as i32);
            let l = photocraft_color::blend::lum([c[0], c[1], c[2]]);
            v[y * w + x] = ((1.0 - l) * c[3] * sv[(y * w + x) * sc]).clamp(0.0, 1.0);
        }
    }
    // Crop to the painted extent.
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if v[y * w + x] > 1e-4 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return Err(bad(cmd, "the selected pixels contain no paint (all white or transparent)"));
    }
    let (tw, th) = ((x1 - x0) as u32, (y1 - y0) as u32);
    if tw.max(th) > MAX_TIP {
        return Err(bad(cmd, format!("selection too large for a brush tip ({tw}×{th}; max {MAX_TIP} px)")));
    }
    let mut data = Vec::with_capacity((tw * th) as usize);
    for y in y0..y1 {
        data.extend_from_slice(&v[y * w + x0..y * w + x1]);
    }
    let tip = GrayTile::from_f32(tw, th, &data);
    let brush = BrushSettings { tip: TipShape::Sampled(tip), size: tw.max(th) as f32, spacing: 0.25, pressure_size: false, ..BrushSettings::default() };
    upsert(s, BrushPreset { name: name.clone(), brush: brush.clone(), builtin: false });
    s.tools.brush = brush;
    Ok(json!({ "name": name, "width": tw, "height": th }))
}

fn set_brush(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "tools.setBrush";
    let mut b = s.tools.brush.clone();
    if let Some(name) = p.get("preset").and_then(Value::as_str) {
        b = find_preset(s, name, cmd)?.brush.clone().with_protected_texture(&s.tools.brush);
    }
    if flag(p, "reset", false) {
        b = BrushSettings::default();
    }
    let mut patch = p.clone();
    if let Some(o) = patch.as_object_mut() {
        for k in ["preset", "reset", "coalesce"] {
            o.remove(k);
        }
        if let Some(inner) = o.remove("brush") {
            let tmp = merge_brush(&b, &inner, cmd)?;
            b = tmp;
        }
    }
    b = merge_brush(&b, &patch, cmd)?;
    s.tools.brush = b;
    Ok(brush_json(&s.tools.brush))
}

macro_rules! spec {
    ($id:literal, $label:literal, $params:literal, $en:expr, $run:expr, $journal:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[], shortcut: None, params: $params, enabled: $en, run: $run, journal: $journal }
    };
}

/// Brush command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("paint.pencil", "Pencil", r##"{"points":[[x,y,pressure?,tiltX?,tiltY?,rotation?,timeMs?,wheel?],…],"brush":{…}?,"preset":name?,"size":px?,"opacity":0..1?,"color":"#rrggbb"?=foreground,"mode":"normal|multiply|screen|…"="normal","erase":bool?,"autoErase":bool=false,"seed":u64?,"target":"pixels"|"mask"|"quickMask"|{"channel":i}=Channels panel target}"##, has_paintable, pencil, true),
        spec!("paint.mixerBrush", "Mixer Brush", r##"{"points":[…],"brush":{…}?,"preset":name?,"size":px?,"wet":0..100=50,"load":0..100=50,"mix":0..100=50,"flow":0..100=100,"color":"#rrggbb"?=foreground,"sampleAllLayers":bool=false,"cleanAfterStroke":bool=true,"loadAfterStroke":bool=true,"seed":u64?}"##, has_paintable, mixer_brush, true),
        spec!("paint.colorReplacement", "Color Replacement", r##"{"points":[…],"brush":{…}?,"size":px?,"mode":"hue|saturation|color|luminosity"="color","sampling":"continuous|once|backgroundSwatch"="continuous","limits":"contiguous|discontiguous|findEdges"="contiguous","tolerance":0..100=30,"antiAlias":bool=true,"color":"#rrggbb"?=foreground,"seed":u64?}"##, crate::commands::has_paintable, color_replacement, true),
        spec!("brush.presets.list", "List Brush Presets", r##"{"full":bool=false}"##, always, presets_list, false),
        spec!("brush.presets.save", "Save Brush Preset", r##"{"name":string,"brush":{…BrushSettings}?=current brush}"##, always, presets_save, true),
        spec!("brush.presets.delete", "Delete Brush Preset", r##"{"name":string}"##, always, presets_delete, true),
        spec!("brush.defineFromSelection", "Define Brush Preset…", r##"{"name":string}"##, has_selection_and_pixels, define_from_selection, true),
        spec!("brush.get", "Get Brush", "{}", always, |s, _| Ok(brush_json(&s.tools.brush)), false),
        spec!("tools.setBrush", "Set Brush", r##"{"preset":name?,"reset":bool?,…BrushSettings fields (camelCase, deep-merged)}"##, always, set_brush, true),
    ]
}

#[cfg(test)]
mod tests;
