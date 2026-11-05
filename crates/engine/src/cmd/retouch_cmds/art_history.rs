//! Art History Brush: stylized, oriented dabs sampled from a history state.

use photocraft_algo::art_history::{self, Dab, MAX_COVERAGE, MAX_DABS, RNG_SEED, StampError, StampParams, Style};
use photocraft_color::{BlendMode, PixelFormat};
use photocraft_geom::Rect;
use photocraft_paint::MAX_BRUSH_SIZE;
use photocraft_paint::retouch::{Region, alpha_index};
use photocraft_raster::{from_rgba_into, to_rgba};
use serde_json::{Value, json};

use super::{EngineError, Result, Session, bad, blend_param, damage_json, parse_brush, run_stroke};

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
    sel: Option<&[f32]>,
    lock: bool,
    alpha: Option<usize>,
) -> Result<u32> {
    if dest.ch != src.ch {
        return Err(err("source and destination do not match"));
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
        selection: sel,
        lock_transparent: lock,
        alpha,
        reference: None,
    };
    art_history::stamp(&mut p).map_err(stamp_err)
}

fn stamp_err(e: StampError) -> EngineError {
    match e {
        StampError::Empty => err("no stroke points"),
        StampError::Mismatched => err("source and destination do not match"),
        StampError::TooLarge => err("stroke too large"),
    }
}

/// `region`'s pixels with an alpha channel last: its own, or opaque for a format without one.
fn with_alpha(region: &Region, fmt: &PixelFormat) -> Result<Vec<f32>> {
    let n = region.ch;
    let cc = fmt.mode.color_channels();
    if n == 0 || cc > n {
        return Err(err("source and destination do not match"));
    }
    let a = alpha_index(fmt);
    let px = region.width().saturating_mul(region.height());
    let len = px.checked_mul(cc + 1).ok_or_else(|| err("stroke too large"))?;
    let mut out = Vec::with_capacity(len);
    for chunk in region.data.chunks_exact(n) {
        out.extend_from_slice(chunk.get(..cc).ok_or_else(|| err("source and destination do not match"))?);
        out.push(a.and_then(|a| chunk.get(a).copied()).unwrap_or(1.0));
    }
    if out.len() != len {
        return Err(err("source and destination do not match"));
    }
    Ok(out)
}

/// Any mode but Normal: the dabs build up in a transparent stroke buffer (dab over dab, as
/// Normal paints them onto the layer), which is composited onto the layer once, in `mode`.
/// Blending each dab with the already-mixed result instead would ring every dab edge.
#[allow(clippy::too_many_arguments)]
fn stamp_buffered(
    dest: &mut Region,
    src: &Region,
    fmt: &PixelFormat,
    dabs: &[Dab],
    hardness: f32,
    opacity: f32,
    flow: f32,
    tolerance: f32,
    sel: Option<&[f32]>,
    lock: bool,
    mode: BlendMode,
) -> Result<u32> {
    if dest.ch != src.ch || dest.rect != src.rect || dest.ch != fmt.channels() {
        return Err(err("source and destination do not match"));
    }
    let n = dest.ch;
    let cc = fmt.mode.color_channels();
    let bch = cc + 1;
    let paint = with_alpha(src, fmt)?;
    let layer = with_alpha(dest, fmt)?;
    let mut buffer = vec![0.0f32; layer.len()];
    let mut p = StampParams {
        dest: &mut buffer,
        dest_w: dest.width(),
        dest_h: dest.height(),
        dest_origin: (dest.rect.x0, dest.rect.y0),
        src: &paint,
        src_w: src.width(),
        src_h: src.height(),
        src_origin: (src.rect.x0, src.rect.y0),
        ch: bch,
        dabs,
        hardness,
        opacity,
        flow,
        tolerance,
        tile: 64,
        selection: sel,
        // The buffer starts transparent; the lock applies when it meets the layer, below.
        lock_transparent: false,
        alpha: Some(cc),
        reference: Some(&layer),
    };
    if art_history::stamp(&mut p).map_err(stamp_err)? == 0 {
        return Ok(0);
    }
    // Counted as they meet the layer: a stroke wholly on locked transparency writes nothing.
    let mut written = 0u32;
    let a = alpha_index(fmt);
    let mut native = vec![0.0f32; n];
    let mut enc = [0.0f32; 8];
    if n > enc.len() {
        return Err(err("source and destination do not match"));
    }
    for (px, stroke) in dest.data.chunks_exact_mut(n).zip(buffer.chunks_exact(bch)) {
        let Some(&sa) = stroke.get(cc) else { continue };
        if sa <= 0.0 {
            continue;
        }
        if lock && a.is_some_and(|a| px.get(a).is_some_and(|&v| v <= 0.0)) {
            continue;
        }
        let (Some(colour), Some(slot)) = (stroke.get(..cc), native.get_mut(..cc)) else { continue };
        slot.copy_from_slice(colour);
        if let Some(a) = a
            && let Some(v) = native.get_mut(a)
        {
            *v = sa;
        }
        let mut s = to_rgba(fmt, &native);
        s[3] = sa;
        let d = to_rgba(fmt, px);
        let o = if lock { photocraft_color::blend::composite_atop(mode, d, s, 1.0) } else { photocraft_color::blend::composite(mode, d, s, 1.0) };
        from_rgba_into(fmt, o, &mut enc);
        if let Some(e) = enc.get(..n) {
            px.copy_from_slice(e);
            written = written.saturating_add(1);
        }
    }
    super::clamp_samples(fmt, &mut dest.data);
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
    // Percent of the channel range; 0 (paint everywhere) is the default, as in Photoshop.
    let tolerance = ranged(p, "tolerance", 0.0, 0.0, 100.0)? / 100.0;
    let style = style_of(p)?;
    // Normal stamps straight onto the layer; any other mode goes through a stroke buffer.
    let mode = blend_param(p, CMD)?;
    let (stroke, id) = parse_brush(s, p, CMD)?;
    let _ = stroke;
    // The same source as the History Brush: the `state` or `snapshot` passed, else the History
    // panel's source, refused after a canvas size change.
    let src_surface = super::history_source(s, id, p, CMD)?;
    // Truncating a huge row index only changes the seed.
    let seed = RNG_SEED ^ p.get("state").and_then(Value::as_u64).map_or(0, |i| i as u32);
    let dabs = art_history::place_dabs(&pts, size, area, style, seed).map_err(|e| match e {
        StampError::TooLarge => err("stroke too large"),
        _ => err("no stroke points"),
    })?;
    if dabs.is_empty() {
        return Err(err("no stroke points"));
    }
    let bounds = union_bounds(&dabs)?;
    // The stamp reads each dab's bounds plus one pixel for bilinear sampling,
    // so only that margin counts against the coverage limit.
    let outer = bounds.inflate(1);
    let area_px = (outer.width() as u64).saturating_mul(outer.height() as u64);
    if outer.is_empty() || area_px > MAX_COVERAGE {
        return Err(err("stroke too large"));
    }

    // Nothing written (every dab under the tolerance, outside the selection or on locked
    // transparency): a no-op stroke, with no history step.
    let mut nothing_written = false;
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
        let n = if mode == BlendMode::Normal {
            stamp_region(&mut dest, &src_reg, &dabs, hardness, opacity, flow, tolerance, sel_mask.as_deref(), lock, alpha_index(&fmt))?
        } else {
            stamp_buffered(&mut dest, &src_reg, &fmt, &dabs, hardness, opacity, flow, tolerance, sel_mask.as_deref(), lock, mode)?
        };
        if n == 0 {
            nothing_written = true;
            return Err(err("the stroke changed no pixel"));
        }
        dest.write(surf);
        Ok(bounds)
    });
    let dmg = match dmg {
        Err(_) if nothing_written => return Ok(json!({ "damage": Value::Null })),
        r => r?,
    };
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
    fn default_tolerance_paints_a_moderate_history_edit() {
        let mut s = session(48, 32, 8, "rgb");
        paint_layer(&mut s, |_, _| [0.5, 0.5, 0.5, 1.0]);
        paint_layer(&mut s, |_, _| [0.6, 0.6, 0.6, 1.0]);
        let steps = s.active().unwrap().history.past_len();
        let p = json!({"points": [[8, 16], [40, 16]], "size": 12, "hardness": 100, "style": "tightMedium", "state": 1});
        s.execute("paint.artHistoryBrush", p).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
        assert!(rgba(&s, 24, 16)[0] < 0.55, "{:?}", rgba(&s, 24, 16));
    }

    #[test]
    fn stroke_under_the_tolerance_records_no_step() {
        let mut s = session(48, 32, 8, "rgb");
        paint_layer(&mut s, |_, _| [0.5, 0.5, 0.5, 1.0]);
        paint_layer(&mut s, |_, _| [0.6, 0.6, 0.6, 1.0]);
        let steps = s.active().unwrap().history.past_len();
        let r = stroke(&mut s, json!({"state": 1, "tolerance": 50})).unwrap();
        assert!(r["damage"].is_null(), "{r}");
        assert_eq!(s.active().unwrap().history.past_len(), steps);
        assert!((rgba(&s, 24, 16)[0] - 0.6).abs() < 0.01);
    }

    #[test]
    fn large_brush_click_is_not_too_large() {
        let mut s = session(2600, 2600, 8, "rgb");
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
        for (style, size) in [("looseCurl", 2000), ("tightMedium", 2400)] {
            let r = stroke(&mut s, json!({"state": 1, "points": [[1300, 1300]], "size": size, "style": style}));
            assert!(r.is_ok(), "{style} size {size}: {r:?}");
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

    /// With no `state`, the brush paints from the History panel's source, like the History Brush.
    #[test]
    fn paints_from_the_history_panel_source() {
        let mut s = session(40, 24, 8, "rgb");
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
        s.execute("history.setSource", json!({"state": 1})).unwrap();
        s.execute("paint.artHistoryBrush", json!({"points": [[4, 12], [30, 12]], "size": 10, "hardness": 100})).unwrap();
        let p = rgba(&s, 12, 12);
        assert!(p[0] > 0.9 && p[1] < 0.2, "the source row is red, got {p:?}");
    }

    /// A source of another canvas size is refused, not painted shifted.
    #[test]
    fn a_source_of_another_canvas_size_is_refused() {
        let mut s = session(40, 24, 8, "rgb");
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("image.canvasSize", json!({"width": 60, "height": 30, "anchor": "center"})).unwrap();
        let steps = s.active().unwrap().history.past_len();
        let e = stroke(&mut s, json!({"state": 1})).unwrap_err().to_string();
        assert!(e.contains("canvas size"), "{e}");
        assert_eq!(s.active().unwrap().history.past_len(), steps);
    }

    /// A stroke that writes nothing (wholly outside the selection, or on transparent pixels
    /// with Lock Transparency) does nothing, as with the tolerance: no error, no History step.
    #[test]
    fn a_stroke_that_writes_nothing_is_a_quiet_no_op() {
        let mut s = session(48, 32, 8, "rgb");
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
        let steps = s.active().unwrap().history.past_len();
        let r = stroke(&mut s, json!({"state": 1})).unwrap();
        assert!(r["damage"].is_null(), "{r}");
        assert_eq!(s.active().unwrap().history.past_len(), steps);
        assert!(rgba(&s, 24, 16)[2] > 0.9, "still blue");

        let mut s = session(48, 32, 8, "rgb");
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("lock", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().locks.transparency = true;
            Ok(())
        })
        .unwrap();
        let steps = s.active().unwrap().history.past_len();
        // Row 1: the new, transparent layer.
        let r = stroke(&mut s, json!({"state": 1})).unwrap();
        assert!(r["damage"].is_null(), "{r}");
        assert_eq!(s.active().unwrap().history.past_len(), steps);
    }

    /// Row 1 of the History panel is `first`, the layer now holds `now`.
    fn two_states(first: [f32; 4], now: [f32; 4]) -> Session {
        let mut s = session(48, 32, 8, "rgb");
        paint_layer(&mut s, move |_, _| first);
        paint_layer(&mut s, move |_, _| now);
        s
    }

    #[test]
    fn multiply_by_a_white_state_leaves_the_layer_alone() {
        let mut s = two_states([1.0, 1.0, 1.0, 1.0], [0.5, 0.5, 0.5, 1.0]);
        // Blending each dab with the already-mixed result darkened every dab edge; the stroke
        // buffer composites the whole stroke once, and grey × white is grey.
        stroke(&mut s, json!({"state": 1, "mode": "multiply"})).unwrap();
        for x in 0..48 {
            for y in [10, 13, 16, 19, 22] {
                let v = rgba(&s, x, y);
                assert!((v[0] - 0.5).abs() < 0.01 && (v[3] - 1.0).abs() < 1.0e-6, "({x}, {y}): {v:?}");
            }
        }
    }

    #[test]
    fn darken_and_lighten_keep_the_darker_or_lighter() {
        for (first, mode, centre) in [
            ([1.0, 1.0, 1.0, 1.0], "lighten", 1.0),
            ([1.0, 1.0, 1.0, 1.0], "darken", 0.5),
            ([0.0, 0.0, 0.0, 1.0], "darken", 0.0),
            ([0.0, 0.0, 0.0, 1.0], "lighten", 0.5),
        ] {
            let mut s = two_states(first, [0.5, 0.5, 0.5, 1.0]);
            stroke(&mut s, json!({"state": 1, "mode": mode})).unwrap();
            let v = rgba(&s, 24, 16);
            assert!((v[0] - centre).abs() < 0.02, "{mode} from {first:?}: {v:?}");
            // Outside the stroke the layer is untouched.
            assert!((rgba(&s, 24, 1)[0] - 0.5).abs() < 1.0e-6);
        }
    }

    #[test]
    fn luminosity_and_color_split_hue_from_brightness() {
        let red = [1.0, 0.0, 0.0, 1.0];
        let blue = [0.0, 0.0, 1.0, 1.0];
        let mut s = two_states(red, blue);
        stroke(&mut s, json!({"state": 1, "mode": "luminosity"})).unwrap();
        let lum = rgba(&s, 24, 16);
        // Red's luminosity on blue's hue: still blue, brighter.
        assert!(lum[2] > lum[0] && lum[2] > lum[1], "{lum:?}");
        assert!(lum[0] > 0.05 || lum[1] > 0.05, "brighter than pure blue: {lum:?}");

        let mut s = two_states(red, blue);
        stroke(&mut s, json!({"state": 1, "mode": "color"})).unwrap();
        let col = rgba(&s, 24, 16);
        // Red's hue at blue's (low) luminosity: a dark red.
        assert!(col[0] > col[1] && col[0] > col[2], "{col:?}");
        assert!(col[0] < 0.9, "kept blue's luminosity: {col:?}");
    }

    #[test]
    fn modes_paint_at_every_depth_and_model() {
        for (depth, model) in [(8u64, "rgb"), (16, "rgb"), (32, "rgb"), (8, "gray"), (16, "gray"), (8, "cmyk"), (8, "lab")] {
            let mut s = session(48, 32, depth, model);
            paint_layer(&mut s, |_, _| [0.9, 0.9, 0.9, 1.0]);
            paint_layer(&mut s, |_, _| [0.3, 0.3, 0.3, 1.0]);
            let steps = s.active().unwrap().history.past_len();
            let before = rgba(&s, 24, 16);
            let r = stroke(&mut s, json!({"state": 1, "mode": "lighten", "opacity": 100})).unwrap();
            assert!(!r["damage"].is_null(), "{depth} {model}: {r}");
            assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
            let after = rgba(&s, 24, 16);
            assert!(after[0] > before[0] + 0.2, "{depth} {model}: {before:?} -> {after:?}");
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(rgba(&s, 24, 16), before, "{depth} {model}: undo");
        }
    }

    #[test]
    fn a_moded_stroke_on_locked_transparency_is_a_no_op() {
        let mut s = session(48, 32, 8, "rgb");
        s.execute("layer.new.layer", json!({})).unwrap();
        // Row 2 holds opaque white on the layer; it is cleared again and locked.
        paint_layer(&mut s, |_, _| [1.0, 1.0, 1.0, 1.0]);
        paint_layer(&mut s, |_, _| [0.0, 0.0, 0.0, 0.0]);
        s.edit("lock", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().locks.transparency = true;
            Ok(())
        })
        .unwrap();
        let steps = s.active().unwrap().history.past_len();
        let r = stroke(&mut s, json!({"state": 2, "mode": "lighten"})).unwrap();
        assert!(r["damage"].is_null(), "{r}");
        assert_eq!(s.active().unwrap().history.past_len(), steps);
    }

    #[test]
    fn opacity_scales_a_moded_stroke() {
        let mut full = two_states([1.0, 1.0, 1.0, 1.0], [0.2, 0.2, 0.2, 1.0]);
        let mut half = two_states([1.0, 1.0, 1.0, 1.0], [0.2, 0.2, 0.2, 1.0]);
        stroke(&mut full, json!({"state": 1, "mode": "screen", "opacity": 100})).unwrap();
        stroke(&mut half, json!({"state": 1, "mode": "screen", "opacity": 30})).unwrap();
        let (f, h) = (rgba(&full, 24, 16)[0], rgba(&half, 24, 16)[0]);
        assert!(f > 0.95 && h > 0.25 && h < f - 0.2, "full {f} 30% {h}");
    }

    /// The stroke buffer composited in Normal must paint what stamping straight onto the layer
    /// paints (dabs build up the same way, whichever buffer they land in).
    #[test]
    fn the_stroke_buffer_in_normal_matches_the_direct_stamp() {
        let fmt = PixelFormat::RGBA8;
        let rect = Rect::new(0, 0, 64, 40);
        let px = |x: i32, y: i32| [((x * 7 + y * 3) % 50) as f32 / 50.0, (y % 9) as f32 / 9.0, 0.4, 1.0];
        let mut src = Region::new(rect, 4);
        let mut dest = Region::new(rect, 4);
        for y in 0..40 {
            for x in 0..64 {
                src.px_mut(x, y).copy_from_slice(&px(x, y));
                dest.px_mut(x, y).copy_from_slice(&[0.9, 0.1, 0.2, 1.0]);
            }
        }
        let dabs = art_history::place_dabs(&[[6.0, 20.0], [58.0, 22.0]], 14.0, 50.0, Style::LooseCurl, RNG_SEED).unwrap();
        let mut direct = dest.clone();
        let mut buffered = dest.clone();
        stamp_region(&mut direct, &src, &dabs, 0.6, 0.7, 0.8, 0.0, None, false, Some(3)).unwrap();
        stamp_buffered(&mut buffered, &src, &fmt, &dabs, 0.6, 0.7, 0.8, 0.0, None, false, BlendMode::Normal).unwrap();
        let worst = direct.data.iter().zip(&buffered.data).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(worst < 1.0e-4, "worst difference {worst}");
        assert_ne!(direct.data, dest.data, "the stroke painted");
    }

    #[test]
    fn unknown_modes_are_errors() {
        let mut s = two_states([1.0, 1.0, 1.0, 1.0], [0.5, 0.5, 0.5, 1.0]);
        let steps = s.active().unwrap().history.past_len();
        let before = rgba(&s, 24, 16);
        assert!(stroke(&mut s, json!({"state": 1, "mode": "sparkle"})).unwrap_err().to_string().contains("unknown blend mode"));
        assert_eq!(s.active().unwrap().history.past_len(), steps);
        assert_eq!(rgba(&s, 24, 16), before);
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
