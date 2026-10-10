//! Art History Brush: stylized, oriented dabs sampled from a history state.

use photocraft_algo::art_history::{self, Dab, MAX_COVERAGE, MAX_DABS, RNG_SEED, StampError, StampParams, Style};
use photocraft_color::BlendMode;
use photocraft_geom::Rect;
use photocraft_paint::MAX_BRUSH_SIZE;
use photocraft_paint::retouch::{Region, alpha_index};
use photocraft_raster::{from_rgba_into, to_rgba};
use serde_json::{Value, json};

use super::*;

const CMD: &str = "paint.artHistoryBrush";
const MAX_POINTS: usize = 4096;

fn err(msg: impl Into<String>) -> EngineError {
    bad(CMD, msg)
}

fn object(p: &Value) -> Result<&serde_json::Map<String, Value>> {
    p.as_object().ok_or_else(|| err("expected an object"))
}

fn finite(v: Option<&Value>, _key: &str) -> Result<Option<f32>> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_f64().filter(|x| x.is_finite()).map(|x| x as f32).ok_or_else(|| err("invalid number")).map(Some),
    }
}

fn finite_key(p: &Value, key: &str, default: f32) -> Result<f32> {
    match finite(p.get(key), key)? {
        None => Ok(default),
        Some(v) => Ok(v),
    }
}

fn ranged(p: &Value, key: &str, default: f32, lo: f32, hi: f32) -> Result<f32> {
    let v = finite_key(p, key, default)?;
    if (lo..=hi).contains(&v) { Ok(v) } else { Err(err(format!("`{key}` must be a number {lo}..{hi}"))) }
}

fn points(p: &Value) -> Result<Vec<[f32; 2]>> {
    let Some(arr) = p.get("points").and_then(Value::as_array) else {
        return Err(err("no stroke points"));
    };
    if arr.is_empty() {
        return Err(err("no stroke points"));
    }
    if arr.len() > MAX_POINTS {
        return Err(err("too many points"));
    }
    let mut out = Vec::with_capacity(arr.len());
    for q in arr {
        let Some(a) = q.as_array() else {
            return Err(err("invalid number"));
        };
        let (Some(x), Some(y)) = (a.first(), a.get(1)) else {
            return Err(err("invalid number"));
        };
        let x = x.as_f64().filter(|v| v.is_finite()).ok_or_else(|| err("invalid number"))?;
        let y = y.as_f64().filter(|v| v.is_finite()).ok_or_else(|| err("invalid number"))?;
        if let Some(pr) = a.get(2)
            && pr.as_f64().filter(|v| v.is_finite()).is_none()
        {
            return Err(err("invalid number"));
        }
        out.push([x as f32, y as f32]);
    }
    if out.is_empty() {
        return Err(err("no stroke points"));
    }
    Ok(out)
}

fn style_of(p: &Value) -> Result<Style> {
    match p.get("style") {
        None | Some(Value::Null) => Ok(Style::TightMedium),
        Some(v) => {
            let Some(s) = v.as_str() else {
                return Err(err("unknown Art History style"));
            };
            Style::parse(s).ok_or_else(|| err("unknown Art History style"))
        }
    }
}

fn history_source(s: &Session, p: &Value) -> Result<(photocraft_doc::Document, usize)> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let past = st.history.past_len();
    let idx = match p.get("state") {
        None | Some(Value::Null) => 0usize,
        Some(v) => {
            let Some(i) = v.as_u64() else {
                return Err(err("invalid number"));
            };
            usize::try_from(i).map_err(|_| err(format!("no history state {i} (0..={past})")))?
        }
    };
    let doc = match idx {
        i if i == past => st.doc.clone(),
        i => st.history.state(i).ok_or_else(|| err(format!("no history state {i} (0..={past})")))?,
    };
    Ok(((*doc).clone(), idx))
}

fn union_bounds(dabs: &[Dab]) -> Result<Rect> {
    let mut r = Rect::EMPTY;
    for d in dabs {
        let (x0, y0, x1, y1) = art_history::dab_bounds(d).ok_or_else(|| err("stroke too large"))?;
        r = r.union(&Rect::new(x0, y0, x1, y1));
    }
    if r.is_empty() {
        return Err(err("no stroke points"));
    }
    Ok(r)
}

fn selection_mask(sel: Option<&photocraft_raster::Surface>, rect: Rect) -> Option<Vec<f32>> {
    let sel = sel?;
    let w = rect.width() as usize;
    let h = rect.height() as usize;
    let mut v = vec![0.0f32; w.saturating_mul(h)];
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let i = ((y - rect.y0) as usize).saturating_mul(w).saturating_add((x - rect.x0) as usize);
            if let Some(slot) = v.get_mut(i) {
                *slot = sel.sample_channel(x, y, 0);
            }
        }
    }
    Some(v)
}

#[allow(clippy::too_many_arguments)]
fn stamp_region(
    dest: &mut Region,
    src: &Region,
    dabs: &[Dab],
    hardness: f32,
    opacity: f32,
    flow: f32,
    tolerance: f32,
    halo: i32,
    sel: Option<&[f32]>,
    lock: bool,
    alpha: Option<usize>,
    blend: BlendMode,
    fmt: photocraft_color::PixelFormat,
) -> Result<u32> {
    if dest.ch != src.ch {
        return Err(err("source and destination do not match"));
    }
    if blend != BlendMode::Normal {
        return stamp_blended(dest, src, dabs, hardness, opacity, flow, tolerance, sel, lock, alpha, blend, fmt);
    }
    let dest_w = dest.width();
    let dest_h = dest.height();
    let dest_origin = (dest.rect.x0, dest.rect.y0);
    let src_w = src.width();
    let src_h = src.height();
    let src_origin = (src.rect.x0, src.rect.y0);
    let ch = dest.ch;
    let mut p = StampParams {
        dest: &mut dest.data,
        dest_w,
        dest_h,
        dest_origin,
        src: &src.data,
        src_w,
        src_h,
        src_origin,
        ch,
        dabs,
        hardness,
        opacity,
        flow,
        tolerance,
        tile: 64,
        halo,
        selection: sel,
        lock_transparent: lock,
        alpha,
    };
    match art_history::stamp(&mut p) {
        Ok(n) => Ok(n),
        Err(StampError::Empty) => Err(err("no stroke points")),
        Err(StampError::Mismatched) => Err(err("source and destination do not match")),
        Err(StampError::TooLarge) => Err(err("stroke too large")),
    }
}

#[allow(clippy::too_many_arguments)]
fn stamp_blended(
    dest: &mut Region,
    src: &Region,
    dabs: &[Dab],
    hardness: f32,
    opacity: f32,
    flow: f32,
    tolerance: f32,
    sel: Option<&[f32]>,
    lock: bool,
    alpha: Option<usize>,
    blend: BlendMode,
    fmt: photocraft_color::PixelFormat,
) -> Result<u32> {
    let mut dummy = dest.clone();
    let dest_w = dummy.width();
    let dest_h = dummy.height();
    let dest_origin = (dummy.rect.x0, dummy.rect.y0);
    let src_w = src.width();
    let src_h = src.height();
    let src_origin = (src.rect.x0, src.rect.y0);
    let ch = dummy.ch;
    let mut p = StampParams {
        dest: &mut dummy.data,
        dest_w,
        dest_h,
        dest_origin,
        src: &src.data,
        src_w,
        src_h,
        src_origin,
        ch,
        dabs,
        hardness,
        opacity,
        flow,
        tolerance,
        // Dest/src already include the halo; one tile is the whole buffer.
        tile: 0,
        halo: 0,
        selection: sel,
        lock_transparent: lock,
        alpha,
    };
    match art_history::stamp(&mut p) {
        Ok(0) => return Ok(0),
        Ok(_) => {}
        Err(StampError::Empty) => return Err(err("no stroke points")),
        Err(StampError::Mismatched) => return Err(err("source and destination do not match")),
        Err(StampError::TooLarge) => return Err(err("stroke too large")),
    }
    let n = dest.ch;
    let mut written = 0u32;
    for y in dest.rect.y0..dest.rect.y1 {
        for x in dest.rect.x0..dest.rect.x1 {
            let i = dest.index(x, y) * n;
            let (Some(before), Some(after)) = (dest.data.get(i..i + n), dummy.data.get(i..i + n)) else { continue };
            if before == after {
                continue;
            }
            let d = to_rgba(&fmt, before);
            let s = to_rgba(&fmt, after);
            let mut o = photocraft_color::blend::composite(blend, d, s, 1.0);
            if lock {
                o[3] = d[3];
            }
            let mut enc = [0.0f32; 8];
            let m = from_rgba_into(&fmt, o, &mut enc);
            if m != n {
                return Err(err("source and destination do not match"));
            }
            if let Some(slot) = dest.data.get_mut(i..i + n) {
                slot.copy_from_slice(&enc[..n]);
            }
            written = written.saturating_add(1);
        }
    }
    Ok(written)
}

/// Run `paint.artHistoryBrush`.
pub fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let _ = object(p)?;
    if s.active().is_none() {
        return Err(EngineError::NoDocument);
    }
    let pts = points(p)?;
    if pts.len() > MAX_DABS {
        return Err(err("too many points"));
    }
    let size = finite_key(p, "size", s.tools.brush.size)?;
    if !size.is_finite() {
        return Err(err("invalid number"));
    }
    if size <= 0.0 || size > MAX_BRUSH_SIZE {
        return Err(err("stroke too large"));
    }
    let hardness = ranged(p, "hardness", s.tools.brush.hardness * 100.0, 0.0, 100.0)? / 100.0;
    let opacity = ranged(p, "opacity", 100.0, 0.0, 100.0)? / 100.0;
    let flow = ranged(p, "flow", 100.0, 0.0, 100.0)? / 100.0;
    let area = ranged(p, "area", 50.0, 0.0, 100.0)?;
    let tolerance = ranged(p, "tolerance", 50.0, 0.0, 100.0)? / 100.0;
    let style = style_of(p)?;
    let mode = blend_param(p, CMD)?;
    let (stroke, id) = parse_brush(s, p, CMD)?;
    let _ = stroke;
    let (mut source_doc, state_idx) = history_source(s, p)?;
    let src_surface = crate::channel_cmds::target_surface(&mut source_doc, id, p)
        .map(|(surf, _)| surf.clone())
        .map_err(|_| EngineError::Other("the target did not exist (as pixels, a mask or a channel) in that history state".into()))?;

    let seed = RNG_SEED ^ (state_idx as u32);
    let dabs = art_history::place_dabs(&pts, size, area, style, seed).map_err(|e| match e {
        StampError::TooLarge => err("stroke too large"),
        _ => err("no stroke points"),
    })?;
    if dabs.is_empty() {
        return Err(err("no stroke points"));
    }
    let halo = art_history::halo_radius(size, style).ok_or_else(|| err("stroke too large"))?;
    let bounds = union_bounds(&dabs)?;
    let outer = bounds.inflate(halo);
    let area_px = (outer.width() as u64).saturating_mul(outer.height() as u64);
    if outer.is_empty() || area_px > MAX_COVERAGE {
        return Err(err("stroke too large"));
    }

    let dmg = run_stroke(s, "Art History Brush", id, p, |_, surf, sel, lock| {
        let fmt = surf.format();
        let src = if src_surface.format() == fmt { src_surface.clone() } else { src_surface.convert(fmt) };
        if src.channels() != surf.channels() {
            return Err(err("source and destination do not match"));
        }
        let mut dest = Region::read(surf, outer);
        let src_reg = Region::read(&src, outer);
        if dest.ch != src_reg.ch || dest.width() != src_reg.width() || dest.height() != src_reg.height() {
            return Err(err("source and destination do not match"));
        }
        let sel_mask = selection_mask(sel, outer);
        let n = stamp_region(&mut dest, &src_reg, &dabs, hardness, opacity, flow, tolerance, halo, sel_mask.as_deref(), lock, alpha_index(&fmt), mode, fmt)?;
        if n == 0 && tolerance <= 0.0 {
            return Err(err("no stroke points"));
        }
        dest.write(surf);
        Ok(bounds)
    })?;
    Ok(json!({ "damage": damage_json(dmg) }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::ColorMode;
    use photocraft_doc::LayerContent;
    use serde_json::json;

    fn session(w: u32, h: u32, depth: u64, mode: &str) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": w, "height": h, "depth": depth, "mode": mode})).unwrap();
        s
    }

    fn rgba(s: &Session, x: i32, y: i32) -> [f32; 4] {
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
    }

    fn paint_layer(s: &mut Session, f: impl Fn(i32, i32) -> [f32; 4]) {
        s.edit("setup", |doc, active| {
            let b = doc.bounds();
            let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            let fmt = surf.format();
            let mut data = Vec::new();
            for y in b.y0..b.y1 {
                for x in b.x0..b.x1 {
                    data.extend(photocraft_raster::from_rgba(&fmt, f(x, y)));
                }
            }
            surf.write_region(b, &data);
            Ok(())
        })
        .unwrap();
    }

    fn stroke(s: &mut Session, extra: Value) -> std::result::Result<Value, EngineError> {
        let mut p = json!({"points": [[8, 16], [40, 16]], "size": 12, "hardness": 100, "area": 50, "tolerance": 0, "style": "tightMedium"});
        if let Some(map) = extra.as_object() {
            for (k, v) in map {
                p[k] = v.clone();
            }
        }
        s.execute("paint.artHistoryBrush", p)
    }

    #[test]
    fn restores_open_state_at_every_depth_and_undoes() {
        for (depth, mode) in [(8u64, "rgb"), (16, "rgb"), (32, "rgb"), (16, "gray")] {
            let mut s = session(48, 32, depth, mode);
            paint_layer(&mut s, |_, _| [0.1, 0.8, 0.2, 1.0]);
            s.execute("edit.fill", json!({"color": "#2030c0"})).unwrap();
            let before = rgba(&s, 20, 16);
            stroke(&mut s, json!({"state": 1})).unwrap();
            let after = rgba(&s, 20, 16);
            if mode == "gray" {
                assert!(after[0] > before[0] + 0.1, "restored lighter gray {depth}: before {before:?} after {after:?}");
            } else {
                assert!(before[2] > 0.5, "{mode} {depth} {before:?}");
                assert!(after[1] > after[2], "restored green {mode} {depth}: {after:?}");
            }
            let steps = s.active().unwrap().history.past_len();
            stroke(&mut s, json!({"style": "looseCurl"})).unwrap();
            assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one mouse-up = one step");
            assert!(s.undo());
            assert!(s.undo());
            let undone = rgba(&s, 20, 16);
            for c in 0..3 {
                assert!((undone[c] - before[c]).abs() < 0.05, "undo {mode} {depth}");
            }
            assert!(s.redo());
        }
    }

    #[test]
    fn state_one_uses_that_snapshot() {
        let mut s = session(40, 24, 8, "rgb");
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
        stroke(&mut s, json!({"state": 1, "points": [[4, 12], [20, 12]], "size": 10})).unwrap();
        let p = rgba(&s, 10, 12);
        assert!(p[0] > 0.9 && p[1] < 0.2, "state 1 is red, got {p:?}");
    }

    #[test]
    fn bad_params_are_errors() {
        let mut s = session(32, 24, 8, "rgb");
        paint_layer(&mut s, |_, _| [0.2, 0.3, 0.4, 1.0]);
        assert!(s.execute("paint.artHistoryBrush", json!([])).is_err());
        assert!(s.execute("paint.artHistoryBrush", json!({})).is_err());
        assert!(s.execute("paint.artHistoryBrush", json!({"points": []})).is_err());
        assert!(
            s.execute("paint.artHistoryBrush", json!({"points": [[0, 0]], "style": "nope"})).unwrap_err().to_string().contains("unknown Art History style")
        );
        assert!(s.execute("paint.artHistoryBrush", json!({"points": [[0, 0]], "state": 99})).is_err());
        assert!(s.execute("paint.artHistoryBrush", json!({"points": [[0, 0]], "state": u64::MAX})).is_err());
        assert!(s.execute("paint.artHistoryBrush", json!({"points": [[0, 0]], "area": -1})).is_err());
        assert!(s.execute("paint.artHistoryBrush", json!({"points": [[0, 0]], "size": "nan"})).is_err());
        let many: Vec<[f32; 2]> = (0..10_000).map(|i| [i as f32, 0.0]).collect();
        assert!(s.execute("paint.artHistoryBrush", json!({"points": many})).unwrap_err().to_string().contains("too many points"));
        assert!(s.execute("paint.artHistoryBrush", json!({"points": [[5, 5]], "size": 8000})).unwrap_err().to_string().contains("stroke too large"));
        assert!(!Session::new().is_enabled("paint.artHistoryBrush"));
        assert!(crate::commands::find("paint.artHistoryBrush").is_some());
    }

    #[test]
    fn pixel_lock_and_type_layer_error() {
        let mut s = session(24, 24, 8, "rgb");
        s.execute("layer.setProps", json!({"locks": {"pixels": true}})).unwrap();
        let e = stroke(&mut s, json!({})).unwrap_err().to_string();
        assert!(e.contains("locked"), "{e}");
        s.execute("layer.setProps", json!({"locks": {"pixels": false}})).unwrap();
        s.execute("type.create", json!({"x": 8, "y": 16, "text": "Hi", "size": 12})).unwrap();
        assert!(stroke(&mut s, json!({})).is_err());
        let kind = s.active().and_then(|d| d.active_layer.and_then(|id| d.doc.layer(id))).map(|l| matches!(l.content, LayerContent::Text(_)));
        assert_eq!(kind, Some(true));
    }

    #[test]
    fn cmyk_does_not_panic() {
        let mut s = session(32, 24, 8, "cmyk");
        paint_layer(&mut s, |_, _| [0.2, 0.3, 0.4, 1.0]);
        s.execute("edit.fill", json!({"color": "#404040"})).ok();
        let _ = stroke(&mut s, json!({"tolerance": 0}));
        assert_eq!(s.active().unwrap().doc.mode, ColorMode::Cmyk);
    }

    #[test]
    fn command_is_registered() {
        let spec = crate::commands::find("paint.artHistoryBrush").expect("registered");
        assert_eq!(spec.label, "Art History Brush");
        assert!(spec.params.contains("style"));
        assert!(spec.journal);
        assert!(!Session::new().is_enabled("paint.artHistoryBrush"));
        let s = session(16, 16, 8, "rgb");
        assert!(s.is_enabled("paint.artHistoryBrush"));
    }
}
