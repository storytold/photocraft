//! Select menu and selection tools: Magic Wand, Color Range, Modify
//! (expand/contract/border/smooth/feather), Grow, Similar, Lasso.
//! Selections are grayscale coverage surfaces clipped to the canvas.

use photocraft_algo::selection::{self as sel, SelectionMode};
use photocraft_doc::Document;
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
fn has_selection(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| d.doc.selection.is_some()).map(|_| ()).ok_or_else(|| "no selection".into())
}

fn f(p: &Value, k: &str, d: f32) -> f32 {
    p.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}
fn b(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn mode(p: &Value) -> SelectionMode {
    SelectionMode::parse(p.get("mode").and_then(Value::as_str).unwrap_or("replace"))
}

/// RGBA pixels the tools sample: the composite, or the active layer.
fn sample_pixels(s: &Session, all_layers: bool) -> Result<(Rect, Vec<[f32; 4]>)> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let area = d.doc.bounds();
    let layer = d.active_layer.and_then(|id| d.doc.layer(id)).and_then(|l| l.surface());
    match (all_layers, layer) {
        (false, Some(surf)) => {
            let mut px = vec![[0.0f32; 4]; area.width() as usize * area.height() as usize];
            surf.read_rgba_into(area, &mut px);
            Ok((area, px))
        }
        _ => Ok((area, photocraft_compose::render(&d.doc, area).px)),
    }
}

fn set_selection(s: &mut Session, label: &str, area: Rect, mask: Vec<f32>, m: SelectionMode) -> Result<Value> {
    let selected = s.edit(label, |doc, _| {
        doc.selection = sel::combine(doc.selection.as_ref(), &mask, area, m);
        Ok(doc.selection.is_some())
    })?;
    Ok(json!({ "selected": selected }))
}

fn current_mask(doc: &Document) -> (Rect, Vec<f32>) {
    let area = doc.bounds();
    (area, sel::mask_from_surface(doc.selection.as_ref(), area))
}

/// 8-bit RGBA of the active layer (or the composite with `all_layers`) over the canvas.
pub(crate) fn sample_rgba8(s: &Session, all_layers: bool) -> Result<(Rect, Vec<[u8; 4]>)> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let area = d.doc.bounds();
    let layer = d.active_layer.and_then(|id| d.doc.layer(id)).and_then(|l| l.surface());
    match (all_layers, layer) {
        (false, Some(surf)) => Ok((area, sel::rgba8_image(surf, area))),
        _ => {
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            Ok((area, photocraft_compose::render(&d.doc, area).px.iter().map(|p| p.map(q)).collect()))
        }
    }
}

fn magic_wand(s: &mut Session, p: &Value) -> Result<Value> {
    let (x, y) = (f(p, "x", 0.0).floor() as i32, f(p, "y", 0.0).floor() as i32);
    let (area, img) = sample_rgba8(s, b(p, "sampleAllLayers", false))?;
    let region = sel::wand_region(&img, area, (x, y), f(p, "tolerance", 32.0), b(p, "contiguous", true), b(p, "antiAlias", true));
    drop(img);
    let m = mode(p);
    let selected = s.edit("Magic Wand", |doc, _| {
        doc.selection = sel::combine_region(doc.selection.as_ref(), region.as_ref(), m);
        Ok(doc.selection.is_some())
    })?;
    Ok(json!({ "selected": selected }))
}

fn parse_color(p: &Value) -> [f32; 3] {
    match p.get("color") {
        Some(Value::Array(a)) if a.len() >= 3 => [0, 1, 2].map(|i| a[i].as_f64().unwrap_or(0.0) as f32),
        Some(Value::String(h)) => {
            let h = h.trim_start_matches('#');
            let c = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map_or(0.0, |v| f32::from(v) / 255.0);
            [c(0), c(2), c(4)]
        }
        _ => [0.0; 3],
    }
}

/// Eyedropper samples from `key` (`[[x, y], …]` in document pixels): the colour under each point
/// and where it was picked (area-local, for Localized Color Clusters).
fn eyedropper_samples(p: &Value, key: &str, area: Rect, px: &[[f32; 4]], bad: &impl Fn(String) -> EngineError) -> Result<Vec<sel::RangeSample>> {
    let Some(pts) = p.get(key) else { return Ok(Vec::new()) };
    let pts = pts.as_array().ok_or_else(|| bad(format!("`{key}` must be [[x, y], …]")))?;
    let w = area.width() as usize;
    let mut out = Vec::with_capacity(pts.len().min(4096));
    for pt in pts {
        let xy = pt.as_array().and_then(|a| match a.as_slice() {
            [x, y] => Some((x.as_f64()?, y.as_f64()?)),
            _ => None,
        });
        let Some((x, y)) = xy.filter(|(x, y)| x.is_finite() && y.is_finite()) else {
            return Err(bad(format!("bad point {pt} in `{key}` (want [x, y])")));
        };
        let (xi, yi) = (x.floor() as i32, y.floor() as i32);
        let q = if area.contains(xi, yi) { px.get((yi - area.y0) as usize * w + (xi - area.x0) as usize) } else { None };
        let Some(&[r, g, b, _]) = q else {
            return Err(bad(format!("point [{x}, {y}] in `{key}` is outside the canvas")));
        };
        out.push(sel::RangeSample { color: [r, g, b], at: Some(((xi - area.x0) as f32, (yi - area.y0) as f32)) });
    }
    Ok(out)
}

/// Composite RGB (the compositor's output profile for the document) → CIE Lab D50.
fn lab_transform(s: &Session) -> Result<std::sync::Arc<photocraft_cms::Transform>> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let src = crate::color_cmds::composite_profile(&d.doc);
    photocraft_cms::cached(&src, photocraft_cms::Builtin::LabD50.profile(), photocraft_cms::TransformOptions::default())
        .map_err(|e| EngineError::Other(format!("colour management: {e}")))
}

/// The Lab profile's normalised encoding (ICC v4) → L 0–100, a and b −128–127.
fn decode_lab(v: [f32; 3]) -> [f32; 3] {
    [v[0] * 100.0, v[1] * 255.0 - 128.0, v[2] * 255.0 - 128.0]
}

fn colour_to_lab(t: &photocraft_cms::Transform, rgb: [f32; 3]) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    t.eval(&rgb.map(|v| v.clamp(0.0, 1.0)), &mut out);
    decode_lab(out)
}

/// RGBA pixels → Lab + alpha (L, a, b, alpha).
fn pixels_to_lab(t: &photocraft_cms::Transform, px: &[[f32; 4]]) -> Vec<[f32; 4]> {
    let src: Vec<[f32; 4]> = px.iter().map(|p| [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0), p[2].clamp(0.0, 1.0), p[3]]).collect();
    let mut out = vec![[0.0f32; 4]; px.len()];
    t.convert_f32(src.as_flattened(), 4, out.as_flattened_mut(), 4, true);
    for p in &mut out {
        let [l, a, b] = decode_lab([p[0], p[1], p[2]]);
        *p = [l, a, b, p[3]];
    }
    out
}

/// Prepared Sampled Colors query shared by the command and its interactive preview.
/// Samples always come from the original document; reducing an image must not change them.
pub struct ColorRangeSamples {
    to_lab: std::sync::Arc<photocraft_cms::Transform>,
    range: sel::LabRange,
    grid: bool,
    fuzziness: f32,
    localized: Option<f32>,
    plus_at: Vec<(f32, f32)>,
}

impl ColorRangeSamples {
    /// Prepare the query without allocating a full-resolution composite. Point samples use
    /// exactly the same compositor/profile as the final selection.
    pub fn new(s: &Session, p: &Value) -> Result<Self> {
        let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
        let area = doc.bounds();
        let bad = |msg: String| EngineError::BadParams { cmd: "select.colorRange".into(), msg };
        let sample = |key: &str| -> Result<Vec<sel::RangeSample>> {
            let Some(pts) = p.get(key) else { return Ok(Vec::new()) };
            let pts = pts.as_array().ok_or_else(|| bad(format!("`{key}` must be [[x, y], …]")))?;
            let mut samples = Vec::new();
            for point in pts {
                let q = point.as_array().and_then(|v| match v.as_slice() {
                    [x, y] => Some((x.as_f64()?, y.as_f64()?)),
                    _ => None,
                });
                let Some((x, y)) = q.filter(|(x, y)| x.is_finite() && y.is_finite()) else {
                    return Err(bad(format!("bad point {point} in `{key}` (want [x, y])")));
                };
                let (x, y) = (x.floor() as i32, y.floor() as i32);
                if !area.contains(x, y) {
                    return Err(bad(format!("point [{x}, {y}] in `{key}` is outside the canvas")));
                }
                let r = Rect::from_xywh(x, y, 1, 1);
                let pixel = if !b(p, "sampleAllLayers", true) {
                    s.active().and_then(|d| d.active_layer.and_then(|id| d.doc.layer(id))).and_then(|l| l.surface()).map(|surf| {
                        let mut px = [[0.0; 4]];
                        surf.read_rgba_into(r, &mut px);
                        px[0]
                    })
                } else {
                    None
                };
                let px = match pixel {
                    Some(px) => px,
                    None => photocraft_compose::render(doc, r).px.first().copied().unwrap_or([0.0; 4]),
                };
                samples.push(sel::RangeSample { color: [px[0], px[1], px[2]], at: Some(((x - area.x0) as f32, (y - area.y0) as f32)) });
            }
            Ok(samples)
        };
        Self::with_samples(s, p, area, &sample("points")?, &sample("subtractPoints")?)
    }

    fn with_samples(s: &Session, p: &Value, area: Rect, plus: &[sel::RangeSample], minus: &[sel::RangeSample]) -> Result<Self> {
        let bad = |msg: String| EngineError::BadParams { cmd: "select.colorRange".into(), msg };
        let fuzz = |d: f32| f(p, "fuzziness", d).max(0.0);
        // The order the eyedropper clicks came in ("+" a point, "-" a subtracted point):
        // Photoshop applies them one after another. Without it, all additions come first.
        let order: Vec<bool> = match p.get("order") {
            None => std::iter::repeat_n(true, plus.len()).chain(std::iter::repeat_n(false, minus.len())).collect(),
            Some(o) => {
                let o = o.as_array().ok_or_else(|| bad("`order` must be a list of \"+\" and \"-\"".into()))?;
                let order = o
                    .iter()
                    .map(|v| match v.as_str() {
                        Some("+") => Ok(true),
                        Some("-") => Ok(false),
                        _ => Err(bad(format!("bad entry {v} in `order` (want \"+\" or \"-\")"))),
                    })
                    .collect::<Result<Vec<bool>>>()?;
                if order.iter().filter(|x| **x).count() != plus.len() || order.iter().filter(|x| !**x).count() != minus.len() {
                    return Err(bad("`order` must list each of `points` (\"+\") and `subtractPoints` (\"-\") once".into()));
                }
                order
            }
        };
        let mut fixed: Vec<[f32; 3]> = Vec::new();
        if let Some(cs) = p.get("colors") {
            let cs = cs.as_array().ok_or_else(|| bad("`colors` must be a list of colours".into()))?;
            fixed.extend(cs.iter().map(|c| parse_color(&json!({ "color": c }))));
        }
        if p.get("color").is_some() {
            fixed.push(parse_color(p));
        }
        // Localized Color Clusters: Range is a percentage of the canvas's longer side.
        let plus_at: Vec<(f32, f32)> = plus.iter().filter_map(|x| x.at).collect();
        let localized = if b(p, "localized", false) {
            if plus_at.is_empty() {
                return Err(bad("localized clusters need eyedropper `points`".into()));
            }
            Some(f(p, "range", 100.0).clamp(0.0, 100.0) / 100.0 * area.width().max(area.height()) as f32)
        } else {
            None
        };
        // Photoshop compares colours in Lab, through the document's profile
        // (sel::lab_range_coverage). The samples span one Lab box; "Add to Sample" grows it,
        // "Subtract from Sample" reshapes it (sel::LabRange::subtract).
        let to_lab = lab_transform(s)?;
        // 8-bit documents are compared on the 8-bit Lab grid, samples and pixels alike.
        let grid = s.active().is_some_and(|d| d.doc.depth == photocraft_doc::SampleType::U8);
        let q = |c: [f32; 3]| if grid { sel::quantize_lab8(c) } else { c };
        let colour_to_lab = |t: &photocraft_cms::Transform, c: [f32; 3]| q(colour_to_lab(t, c));
        let foreground = || {
            let [r, g, b, _] = s.tools.foreground;
            sel::LabRange::point(colour_to_lab(&to_lab, [r, g, b]))
        };
        let fixed_lab: Vec<[f32; 3]> = fixed.iter().map(|c| colour_to_lab(&to_lab, *c)).collect();
        let mut range = sel::LabRange::spanning(&fixed_lab);
        let (mut pi, mut mi) = (plus.iter(), minus.iter());
        for add in order {
            if add {
                if let Some(x) = pi.next() {
                    let c = colour_to_lab(&to_lab, x.color);
                    range = Some(range.map_or(sel::LabRange::point(c), |r| r.grow(&c)));
                }
            } else if let Some(x) = mi.next() {
                range = Some(range.unwrap_or_else(foreground).subtract(&colour_to_lab(&to_lab, x.color), fuzz(40.0)));
            }
        }
        let range = range.unwrap_or_else(foreground);
        Ok(Self { to_lab, range, grid, fuzziness: fuzz(40.0), localized, plus_at })
    }

    /// Coverage of a composite rectangle. `scale` is the original document pixels represented
    /// by one input pixel; `origin` is in original document pixels (for a visible canvas crop).
    pub fn coverage(&self, px: &[[f32; 4]], width: usize, scale: f32, origin: [f32; 2]) -> Vec<f32> {
        let mut lab = pixels_to_lab(&self.to_lab, px);
        if self.grid {
            for p in &mut lab {
                let [l, a, b] = sel::quantize_lab8([p[0], p[1], p[2]]);
                *p = [l, a, b, p[3]];
            }
        }
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let points: Vec<_> = self.plus_at.iter().map(|(x, y)| ((x - origin[0]) / scale, (y - origin[1]) / scale)).collect();
        sel::color_range_lab(&lab, width, &self.range, self.fuzziness, self.grid, self.localized.map(|r| (r / scale, points.as_slice())))
    }
}

fn color_range(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "select.colorRange";
    let bad = |msg: String| EngineError::BadParams { cmd: CMD.into(), msg };
    let preset = p.get("select").map_or(Some("sampledColors"), Value::as_str).ok_or_else(|| bad("`select` must be a string".into()))?;
    let invert = b(p, "invert", false);
    let fuzz = |d: f32| f(p, "fuzziness", d).max(0.0);
    let (area, mut mask) = match preset {
        "sampledColors" => {
            let (area, px) = sample_pixels(s, b(p, "sampleAllLayers", true))?;
            let w = area.width() as usize;
            // The dialog's eyedroppers: colours picked on the image, at their positions.
            let plus = eyedropper_samples(p, "points", area, &px, &bad)?;
            let minus = eyedropper_samples(p, "subtractPoints", area, &px, &bad)?;
            let query = ColorRangeSamples::with_samples(s, p, area, &plus, &minus)?;
            let mask = query.coverage(&px, w, 1.0, [0.0, 0.0]);
            (area, mask)
        }
        "reds" | "yellows" | "greens" | "cyans" | "blues" | "magentas" => {
            let center = match preset {
                "reds" => 0.0,
                "yellows" => 60.0,
                "greens" => 120.0,
                "cyans" => 180.0,
                "blues" => 240.0,
                _ => 300.0,
            };
            let (area, px) = sample_pixels(s, b(p, "sampleAllLayers", true))?;
            (area, sel::hue_range(&px, center))
        }
        "highlights" | "midtones" | "shadows" => {
            // Range in levels (Highlights / Shadows: one value, Midtones: [low, high]); Fuzziness
            // is a percentage of the tonal scale.
            let level = |v: &Value| v.as_f64().filter(|x| (0.0..=255.0).contains(x)).map(|x| x as f32);
            let r = p.get("tonalRange");
            let (lo, hi) = match (preset, r) {
                ("shadows", None) => (0.0, 65.0),
                ("highlights", None) => (190.0, 255.0),
                (_, None) => (105.0, 150.0),
                ("shadows", Some(v)) => (0.0, level(v).ok_or_else(|| bad(format!("tonalRange: want a level 0..255 (got {v})")))?),
                ("highlights", Some(v)) => (level(v).ok_or_else(|| bad(format!("tonalRange: want a level 0..255 (got {v})")))?, 255.0),
                (_, Some(v)) => v
                    .as_array()
                    .and_then(|a| match a.as_slice() {
                        [l, h] => Some((level(l)?, level(h)?)),
                        _ => None,
                    })
                    .filter(|(l, h)| l <= h)
                    .ok_or_else(|| bad(format!("tonalRange: want [low, high] levels 0..255 (got {v})")))?,
            };
            let (area, px) = sample_pixels(s, b(p, "sampleAllLayers", true))?;
            (area, sel::tone_range(&px, lo, hi, fuzz(20.0).min(100.0) / 100.0 * 255.0))
        }
        "outOfGamut" => {
            // Pixels the current proof setup (View › Proof Setup) can't reproduce, as the gamut
            // warning shows them: judged on the composite.
            let d = s.active().ok_or(EngineError::NoDocument)?;
            let pv = s.color.proof(d.doc.id);
            let (m, _) = crate::color_cmds::gamut_mask(&d.doc, &pv.setup, pv.gamut_threshold)?;
            (d.doc.bounds(), m.iter().map(|v| f32::from(*v) / 255.0).collect())
        }
        other => {
            return Err(bad(format!(
                "unknown select `{other}` (sampledColors, reds, yellows, greens, cyans, blues, magentas, highlights, midtones, shadows, outOfGamut)"
            )));
        }
    };
    if invert {
        for v in &mut mask {
            *v = 1.0 - *v;
        }
    }
    set_selection(s, "Color Range", area, mask, mode(p))
}

fn modify(s: &mut Session, p: &Value, op: &str) -> Result<Value> {
    // The dialogs' ranges (Photoshop's limits); anything else is refused, never clamped silently.
    let (min, max): (f32, f32) = match op {
        "border" => (1.0, 200.0),
        "feather" => (0.1, 1000.0),
        _ => (1.0, 500.0),
    };
    let r = f(p, "radius", 1.0);
    // Below the dialog's minimum (a negative or zero radius, #993) or above its maximum is
    // refused: a clamp to 0 left the selection alone (Expand, Contract) or cleared it (Border)
    // while reporting success.
    if !(min..=max).contains(&r) {
        return Err(EngineError::BadParams { cmd: format!("select.modify.{op}"), msg: format!("radius must be a number in {min}..{max}") });
    }
    // Photoshop's "Apply effect at canvas bounds": when on, the canvas edge is a selection edge
    // (Select All then Contract shrinks from the edges); when off, the selection is taken to
    // continue past the canvas. Border has no such option: its band always follows the canvas edge,
    // or a Select All would border to nothing.
    let at_bounds = op == "border" || b(p, "applyAtCanvasBounds", false);
    let (area, m) = current_mask(&s.active().ok_or(EngineError::NoDocument)?.doc);
    let (w, h) = (area.width() as usize, area.height() as usize);
    // Feather reaches 3σ = 1.5 r; the others reach r. A margin past that keeps the canvas edge out of
    // reach. A selection clear of the canvas edge needs none: both readings agree there.
    let reach = if op == "feather" { r * 1.5 } else { r };
    let pad = if touches_edge(&m, w, h) { reach.ceil() as usize + 2 } else { 0 };
    let grown = |n: usize| pad.checked_mul(2).and_then(|p| n.checked_add(p));
    let (Some(pw), Some(ph)) = (grown(w), grown(h)) else {
        return Err(EngineError::BadParams { cmd: format!("select.modify.{op}"), msg: "the selection is too large to modify".into() });
    };
    if pw.checked_mul(ph).is_none_or(|n| n > MAX_MODIFY_PIXELS) {
        return Err(EngineError::BadParams { cmd: format!("select.modify.{op}"), msg: "the selection is too large to modify".into() });
    }
    let padded = pad_mask(&m, w, h, pad, at_bounds);
    let out = match op {
        "expand" => sel::expand(&padded, pw, ph, r),
        "contract" => sel::contract(&padded, pw, ph, r),
        "border" => sel::border(&padded, pw, ph, r),
        "smooth" => sel::smooth(&padded, pw, ph, r),
        _ => sel::feather(&padded, pw, ph, r),
    };
    let out = crop_mask(&out, pw, pad, w, h);
    let label = match op {
        "expand" => "Expand Selection",
        "contract" => "Contract Selection",
        "border" => "Border Selection",
        "smooth" => "Smooth Selection",
        _ => "Feather Selection",
    };
    set_selection(s, label, area, out, SelectionMode::Replace)
}

/// The largest padded mask Modify works on (a 300000 × 300000 canvas is 9e10 pixels; this caps a
/// padded copy at ~4 GB of f32, beyond which the edit is refused rather than aborting on allocation).
const MAX_MODIFY_PIXELS: usize = 1 << 30;

/// Whether any pixel on the outermost rows or columns of `m` (`w`×`h`) is selected.
fn touches_edge(m: &[f32], w: usize, h: usize) -> bool {
    let sel = |x: usize, y: usize| m.get(y * w + x).is_some_and(|v| *v > 0.0);
    (0..w).any(|x| sel(x, 0) || sel(x, h.saturating_sub(1))) || (0..h).any(|y| sel(0, y) || sel(w.saturating_sub(1), y))
}

/// `m` (`w`×`h`) with a `pad`-pixel margin: unselected when `outside_empty`, else the nearest
/// edge pixel repeated (the selection continues past the canvas).
fn pad_mask(m: &[f32], w: usize, h: usize, pad: usize, outside_empty: bool) -> Vec<f32> {
    let pw = w + 2 * pad;
    let mut out = vec![0.0f32; pw * (h + 2 * pad)];
    if w == 0 || h == 0 {
        return out;
    }
    for (py, row) in out.chunks_exact_mut(pw).enumerate() {
        let inside_y = (pad..pad + h).contains(&py);
        if outside_empty && !inside_y {
            continue;
        }
        let y = py.saturating_sub(pad).min(h - 1);
        for (px, v) in row.iter_mut().enumerate() {
            let inside_x = (pad..pad + w).contains(&px);
            if outside_empty && !inside_x {
                continue;
            }
            let x = px.saturating_sub(pad).min(w - 1);
            *v = m.get(y * w + x).copied().unwrap_or(0.0);
        }
    }
    out
}

/// The `w`×`h` centre of a mask padded by `pad_mask`.
fn crop_mask(m: &[f32], pw: usize, pad: usize, w: usize, h: usize) -> Vec<f32> {
    m.chunks_exact(pw).skip(pad).take(h).flat_map(|row| row.iter().skip(pad).take(w).copied()).collect()
}

/// Grow (contiguous) / Similar (anywhere): pixels whose colour lies within
/// the selected pixels' colour range widened by the tolerance.
fn grow_similar(s: &mut Session, p: &Value, contiguous: bool) -> Result<Value> {
    let tol = f(p, "tolerance", 32.0) / 255.0;
    let (area, px) = sample_pixels(s, b(p, "sampleAllLayers", false))?;
    let (_, m) = current_mask(&s.active().ok_or(EngineError::NoDocument)?.doc);
    let (mut lo, mut hi) = ([f32::MAX; 4], [f32::MIN; 4]);
    for (q, v) in px.iter().zip(&m) {
        if *v >= 0.5 {
            for c in 0..4 {
                lo[c] = lo[c].min(q[c]);
                hi[c] = hi[c].max(q[c]);
            }
        }
    }
    if lo[0] > hi[0] {
        return Err(EngineError::Other("selection is empty".into()));
    }
    let inside = |q: &[f32; 4]| (0..4).all(|c| q[c] >= lo[c] - tol - 1e-6 && q[c] <= hi[c] + tol + 1e-6);
    let (w, h) = (area.width() as usize, area.height() as usize);
    let mut out: Vec<f32> = m.iter().map(|v| if *v >= 0.5 { 1.0 } else { 0.0 }).collect();
    if contiguous {
        let mut stack: Vec<usize> = (0..out.len()).filter(|i| out[*i] > 0.0).collect();
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            let n = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)];
            for j in n.into_iter().flatten() {
                if out[j] == 0.0 && inside(&px[j]) {
                    out[j] = 1.0;
                    stack.push(j);
                }
            }
        }
    } else {
        for (o, q) in out.iter_mut().zip(&px) {
            if inside(q) {
                *o = 1.0;
            }
        }
    }
    set_selection(s, if contiguous { "Grow" } else { "Similar" }, area, out, SelectionMode::Replace)
}

fn lasso(s: &mut Session, p: &Value) -> Result<Value> {
    let pts: Vec<(f32, f32)> = p
        .get("points")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|q| Some((q.get(0)?.as_f64()? as f32, q.get(1)?.as_f64()? as f32))).collect())
        .unwrap_or_default();
    if pts.len() < 3 {
        return Err(EngineError::BadParams { cmd: "select.lasso".into(), msg: "needs at least 3 points".into() });
    }
    polygon_selection(s, "select.lasso", "Lasso", &pts, p)
}

/// Selects the polygon `pts` with the lasso tools' `mode`, `antiAlias` and `feather` params, as
/// one history step called `label`.
pub(crate) fn polygon_selection(s: &mut Session, cmd: &str, label: &str, pts: &[(f32, f32)], p: &Value) -> Result<Value> {
    let feather = f(p, "feather", 0.0);
    // Select › Modify › Feather's range.
    if !(0.0..=1000.0).contains(&feather) {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: "feather must be a number in 0..1000".into() });
    }
    let area = s.active().ok_or(EngineError::NoDocument)?.doc.bounds();
    let mut mask = sel::polygon(pts, area, b(p, "antiAlias", true));
    if feather > 0.0 {
        mask = sel::feather(&mask, area.width() as usize, area.height() as usize, feather);
    }
    set_selection(s, label, area, mask, mode(p))
}

macro_rules! spec {
    ($id:literal, $label:literal, [$($m:literal),*], $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: None, params: $params, enabled: $en, run: $run, journal: true }
    };
}

/// Selection command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "select.magicWand",
            "Magic Wand",
            [],
            r##"{"x":px,"y":px,"tolerance":0..255=32,"contiguous":bool=true,"antiAlias":bool=true,"sampleAllLayers":bool=false,"mode":"replace|add|subtract|intersect"="replace"}"##,
            has_doc,
            magic_wand
        ),
        spec!(
            "select.colorRange",
            "Color Range…",
            ["Select"],
            r##"{"select":"sampledColors|reds|yellows|greens|cyans|blues|magentas|highlights|midtones|shadows|outOfGamut"="sampledColors","color":"#rrggbb"=foreground,"colors":["#rrggbb",…]?,"points":[[x,y],…]? (eyedropper samples),"subtractPoints":[[x,y],…]? (minus eyedropper),"order":["+"|"-",…]? (click order of points and subtractPoints; default all points first),"fuzziness":0..200=40 (tones: 0..100 %=20),"localized":bool=false,"range":0..100=100 (% of the longer side),"tonalRange":level|[lo,hi] (shadows 65, highlights 190, midtones [105,150]),"invert":bool=false,"sampleAllLayers":bool=true,"mode":"replace|add|subtract|intersect"="replace"}"##,
            has_doc,
            color_range
        ),
        spec!("select.modify.border", "Border…", ["Select", "Modify"], r##"{"radius":1..200=1}"##, has_selection, |s, p| modify(s, p, "border")),
        spec!("select.modify.smooth", "Smooth…", ["Select", "Modify"], r##"{"radius":1..500=1,"applyAtCanvasBounds":bool=false}"##, has_selection, |s, p| {
            modify(s, p, "smooth")
        }),
        spec!("select.modify.expand", "Expand…", ["Select", "Modify"], r##"{"radius":1..500=1,"applyAtCanvasBounds":bool=false}"##, has_selection, |s, p| {
            modify(s, p, "expand")
        }),
        spec!(
            "select.modify.contract",
            "Contract…",
            ["Select", "Modify"],
            r##"{"radius":1..500=1,"applyAtCanvasBounds":bool=false}"##,
            has_selection,
            |s, p| modify(s, p, "contract")
        ),
        spec!(
            "select.modify.feather",
            "Feather…",
            ["Select", "Modify"],
            r##"{"radius":0.1..1000=1,"applyAtCanvasBounds":bool=false}"##,
            has_selection,
            |s, p| modify(s, p, "feather")
        ),
        spec!("select.grow", "Grow", ["Select"], r##"{"tolerance":0..255=32,"sampleAllLayers":bool=false}"##, has_selection, |s, p| grow_similar(s, p, true)),
        spec!("select.similar", "Similar", ["Select"], r##"{"tolerance":0..255=32,"sampleAllLayers":bool=false}"##, has_selection, |s, p| grow_similar(
            s, p, false
        )),
        spec!(
            "select.lasso",
            "Lasso",
            [],
            r##"{"points":[[x,y],…],"mode":"replace|add|subtract|intersect"="replace","antiAlias":bool=true,"feather":px=0}"##,
            has_doc,
            lasso
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
        s.edit("paint", |doc, _| {
            let bg = doc.layers[0].surface_mut().unwrap();
            bg.fill_rect(Rect::new(5, 5, 15, 15), &[1.0, 0.0, 0.0, 1.0]);
            bg.fill_rect(Rect::new(25, 5, 35, 15), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        s
    }

    fn coverage(s: &Session, x: i32, y: i32) -> f32 {
        s.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))
    }

    #[test]
    fn magic_wand_contiguous_and_modes() {
        let mut s = session();
        s.execute("select.magicWand", json!({"x": 7, "y": 7, "antiAlias": false})).unwrap();
        assert_eq!(coverage(&s, 10, 10), 1.0);
        assert_eq!(coverage(&s, 30, 10), 0.0);
        s.execute("select.magicWand", json!({"x": 30, "y": 7, "mode": "add", "antiAlias": false})).unwrap();
        assert_eq!(coverage(&s, 30, 10), 1.0);
        s.execute("select.magicWand", json!({"x": 7, "y": 7, "mode": "subtract", "antiAlias": false})).unwrap();
        assert_eq!(coverage(&s, 10, 10), 0.0);
        s.execute("select.magicWand", json!({"x": 7, "y": 7, "contiguous": false, "antiAlias": false})).unwrap();
        assert_eq!(coverage(&s, 30, 10), 1.0);
    }

    #[test]
    fn color_range_prepared_query_matches_the_command_at_all_depths() {
        use photocraft_doc::{Color, ColorMode, SampleType, Size};
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for profile in [None, Some(photocraft_cms::Builtin::DisplayP3.profile().to_bytes())] {
                let mut doc = Document::with_background("samples", Size::new(24, 12), ColorMode::Rgb, depth, Color::WHITE);
                doc.icc_profile = profile;
                let layer = doc.layers[0].surface_mut().unwrap();
                layer.fill_rect(Rect::new(0, 0, 8, 12), &[0.65, 0.3, 0.2, 1.0]);
                layer.fill_rect(Rect::new(8, 0, 16, 12), &[0.2, 0.3, 0.65, 1.0]);
                let mut s = Session::new();
                s.add_document(doc, None);
                for p in [
                    json!({"points": [[2, 3]], "fuzziness": 0}),
                    json!({"points": [[2, 3], [10, 3]], "subtractPoints": [[10, 3]], "order": ["+", "+", "-"], "fuzziness": 40}),
                    json!({"points": [[2, 3]], "localized": true, "range": 50, "fuzziness": 80}),
                ] {
                    let q = ColorRangeSamples::new(&s, &p).unwrap();
                    let doc = s.active().unwrap().doc.clone();
                    let pixels = photocraft_compose::render(&doc, doc.bounds());
                    let preview = q.coverage(&pixels.px, 24, 1.0, [0.0, 0.0]);
                    s.execute("select.colorRange", p).unwrap();
                    let actual = current_mask(&s.active().unwrap().doc).1;
                    for (shown, selected) in preview.iter().zip(actual) {
                        let rounded = (shown.clamp(0.0, 1.0) * 255.0 + 0.5).floor() / 255.0;
                        assert_eq!(rounded, selected, "{depth:?}: original-pixel preview and OK differ");
                    }
                }
            }
        }
    }

    #[test]
    fn color_range_prepared_query_samples_visible_layers_not_the_active_layer() {
        use photocraft_doc::{Layer, LayerContent};
        let mut s = session_colours();
        let doc = s.active().unwrap().doc.clone();
        let mut top = Layer::new("blue foreground", LayerContent::Raster(doc.layers[0].surface().unwrap().clone()));
        top.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 40, 30), &[0.0, 0.0, 1.0, 1.0]);
        let mut hidden = Layer::new("hidden green", LayerContent::Raster(top.surface().unwrap().clone()));
        hidden.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 40, 30), &[0.0, 1.0, 0.0, 1.0]);
        hidden.visible = false;
        s.edit("layers", |doc, _| {
            doc.layers.push(top);
            doc.layers.push(hidden);
            Ok(())
        })
        .unwrap();
        for all_layers in [false, true] {
            let p = json!({"points": [[7, 7]], "sampleAllLayers": all_layers, "fuzziness": 0});
            let query = ColorRangeSamples::new(&s, &p).unwrap();
            let (area, px) = sample_pixels(&s, all_layers).unwrap();
            let shown = query.coverage(&px, area.width() as usize, 1.0, [0.0, 0.0]);
            s.execute("select.colorRange", p).unwrap();
            assert_eq!(shown[7 * 40 + 7], coverage(&s, 7, 7));
            assert_eq!(shown[24 * 40 + 10], coverage(&s, 10, 24));
        }
        for p in [json!({"points": [[-1, 0]]}), json!({"points": [[40, 1]]}), json!({"points": "wrong"}), json!({"points": [[null, 0]]})] {
            assert!(ColorRangeSamples::new(&s, &p).is_err(), "{p}");
        }
    }

    #[test]
    fn color_range_selects_by_colour() {
        let mut s = session();
        s.execute("select.colorRange", json!({"color": "#ff0000", "fuzziness": 40})).unwrap();
        // The sampled colour itself is fully selected (as with Photoshop's eyedropper).
        assert_eq!(coverage(&s, 10, 10), 1.0);
        assert_eq!(coverage(&s, 20, 20), 0.0);
        s.execute("select.colorRange", json!({"color": "#ff0000", "fuzziness": 100})).unwrap();
        assert_eq!(coverage(&s, 10, 10), 1.0);
        assert_eq!(coverage(&s, 20, 20), 0.0);
    }

    #[test]
    fn color_range_compares_in_lab_like_photoshop() {
        // Photoshop weighs hue (Lab a/b) three times as much as lightness; a per-channel RGB
        // distance can't tell a lighter shade from a redder one.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 30, "height": 10})).unwrap();
        s.edit("paint", |doc, _| {
            let bg = doc.layers[0].surface_mut().unwrap();
            let v = |c: u8| f32::from(c) / 255.0;
            bg.fill_rect(Rect::new(0, 0, 10, 10), &[v(200), v(140), v(120), 1.0]); // skin
            bg.fill_rect(Rect::new(10, 0, 20, 10), &[v(220), v(160), v(140), 1.0]); // lighter skin
            bg.fill_rect(Rect::new(20, 0, 30, 10), &[v(220), v(140), v(120), 1.0]); // redder skin
            Ok(())
        })
        .unwrap();
        s.execute("select.colorRange", json!({"points": [[5, 5]], "fuzziness": 40})).unwrap();
        assert!(coverage(&s, 5, 5) > 0.98);
        // Both differ from the sample by 20 levels in one RGB channel, yet the lighter one is
        // kept clearly more (about 0.55 vs 0.31).
        let (lighter, redder) = (coverage(&s, 15, 5), coverage(&s, 25, 5));
        assert!(lighter > redder + 0.15 && redder > 0.0, "lighter {lighter}, redder {redder}");
    }

    /// Red squares (from `session`), a blue square, a black and a mid-grey strip on white.
    fn session_colours() -> Session {
        let mut s = session();
        s.edit("paint", |doc, _| {
            let bg = doc.layers[0].surface_mut().unwrap();
            bg.fill_rect(Rect::new(5, 20, 15, 28), &[0.0, 0.0, 1.0, 1.0]);
            bg.fill_rect(Rect::new(20, 20, 25, 28), &[0.0, 0.0, 0.0, 1.0]);
            bg.fill_rect(Rect::new(30, 20, 35, 28), &[0.5, 0.5, 0.5, 1.0]);
            Ok(())
        })
        .unwrap();
        s
    }

    #[test]
    fn color_range_several_samples_points_and_invert() {
        let mut s = session_colours();
        // Several samples span one Lab box (Photoshop's Minimum/Maximum): red and blue take
        // both squares, black, grey and white stay out.
        s.execute("select.colorRange", json!({"colors": ["#ff0000", "#0000ff"], "fuzziness": 100})).unwrap();
        assert_eq!((coverage(&s, 10, 10), coverage(&s, 30, 10), coverage(&s, 10, 24)), (1.0, 1.0, 1.0));
        assert_eq!((coverage(&s, 2, 2), coverage(&s, 22, 24), coverage(&s, 32, 24)), (0.0, 0.0, 0.0));
        // Eyedropper point on the blue square; `invert` flips the result.
        s.execute("select.colorRange", json!({"points": [[7, 22]], "fuzziness": 100, "invert": true})).unwrap();
        assert_eq!((coverage(&s, 10, 24), coverage(&s, 10, 10), coverage(&s, 2, 2)), (0.0, 1.0, 1.0));
        // Localized clusters: picked on the left red square, a 25% range (10 px) leaves the right one.
        s.execute("select.colorRange", json!({"points": [[10, 10]], "fuzziness": 100, "localized": true, "range": 25})).unwrap();
        assert_eq!(coverage(&s, 10, 10), 1.0);
        assert!(coverage(&s, 13, 10) > 0.5);
        assert_eq!(coverage(&s, 30, 10), 0.0);
        // One undo step per call.
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(coverage(&s, 2, 2), 1.0);
    }

    #[test]
    fn color_range_subtract_follows_photoshop_and_the_click_order() {
        // Grey strips from dark to light; samples at both ends span the whole run.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 50, "height": 10})).unwrap();
        s.edit("paint", |doc, _| {
            let bg = doc.layers[0].surface_mut().unwrap();
            for (i, v) in [0.30f32, 0.40, 0.50, 0.60, 0.70].iter().enumerate() {
                bg.fill_rect(Rect::new(i as i32 * 10, 0, i as i32 * 10 + 10, 10), &[*v, *v, *v, 1.0]);
            }
            Ok(())
        })
        .unwrap();
        let run = |s: &mut Session, extra: Value| {
            let mut p = json!({"points": [[5, 5], [45, 5]], "fuzziness": 10});
            p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            s.execute("select.colorRange", p).unwrap();
            (0..5).map(|i| coverage(s, i * 10 + 5, 5)).collect::<Vec<f32>>()
        };
        assert!(run(&mut s, json!({})).iter().all(|v| *v > 0.99), "both samples span every strip");
        // Subtracting a colour inside the range moves the nearer edge onto it: the strips beyond
        // it drop out, the subtracted grey itself stays selected (Photoshop does the same).
        let cut = run(&mut s, json!({"subtractPoints": [[15, 5]]}));
        assert!(cut[0] < 0.5 && cut[1] > 0.99 && cut[4] > 0.99, "{cut:?}");
        // Order matters: adding the dark strip back after the subtraction restores it.
        let readd = run(&mut s, json!({"points": [[5, 5], [45, 5], [5, 5]], "subtractPoints": [[15, 5]], "order": ["+", "+", "-", "+"]}));
        assert!(readd[0] > 0.99, "{readd:?}");
        let sub_last = run(&mut s, json!({"points": [[5, 5], [45, 5], [5, 5]], "subtractPoints": [[15, 5]], "order": ["+", "+", "+", "-"]}));
        assert!(sub_last[0] < 0.5, "{sub_last:?}");
        // A malformed or inconsistent order is an error, never a panic.
        for bad in [json!(["+"]), json!(["+", "+", "x"]), json!("+-"), json!(["-", "-", "+"])] {
            assert!(s.execute("select.colorRange", json!({"points": [[5, 5], [45, 5]], "subtractPoints": [[15, 5]], "order": bad})).is_err(), "{bad}");
        }
    }

    #[test]
    fn color_range_presets() {
        let mut s = session_colours();
        s.execute("select.colorRange", json!({"select": "reds"})).unwrap();
        assert_eq!((coverage(&s, 10, 10), coverage(&s, 30, 10)), (1.0, 1.0));
        assert_eq!((coverage(&s, 10, 24), coverage(&s, 2, 2), coverage(&s, 32, 24)), (0.0, 0.0, 0.0));
        s.execute("select.colorRange", json!({"select": "blues"})).unwrap();
        assert_eq!((coverage(&s, 10, 24), coverage(&s, 10, 10)), (1.0, 0.0));
        s.execute("select.colorRange", json!({"select": "greens"})).unwrap();
        assert!(s.active().unwrap().doc.selection.is_none() || coverage(&s, 10, 10) == 0.0);
        // Tones: black is a shadow, white a highlight, 50% grey (128) a midtone.
        s.execute("select.colorRange", json!({"select": "shadows"})).unwrap();
        assert_eq!((coverage(&s, 22, 24), coverage(&s, 2, 2), coverage(&s, 32, 24)), (1.0, 0.0, 0.0));
        s.execute("select.colorRange", json!({"select": "highlights"})).unwrap();
        assert_eq!((coverage(&s, 22, 24), coverage(&s, 2, 2)), (0.0, 1.0));
        s.execute("select.colorRange", json!({"select": "midtones", "fuzziness": 0})).unwrap();
        assert_eq!((coverage(&s, 32, 24), coverage(&s, 2, 2), coverage(&s, 22, 24)), (1.0, 0.0, 0.0));
        // A wider shadow range takes the grey too.
        s.execute("select.colorRange", json!({"select": "shadows", "tonalRange": 140, "fuzziness": 0})).unwrap();
        assert_eq!(coverage(&s, 32, 24), 1.0);
        // Modes combine as for the other selection commands.
        s.execute("select.colorRange", json!({"select": "reds", "mode": "add"})).unwrap();
        assert_eq!((coverage(&s, 32, 24), coverage(&s, 10, 10)), (1.0, 1.0));
    }

    #[test]
    fn color_range_out_of_gamut_matches_the_gamut_warning() {
        let mut s = session();
        s.edit("paint", |doc, _| {
            // Saturated sRGB blue is outside the default CMYK proof; mid grey is inside.
            let bg = doc.layers[0].surface_mut().unwrap();
            bg.fill_rect(Rect::new(0, 20, 20, 30), &[0.0, 0.0, 1.0, 1.0]);
            bg.fill_rect(Rect::new(20, 20, 40, 30), &[0.5, 0.5, 0.5, 1.0]);
            Ok(())
        })
        .unwrap();
        s.execute("select.colorRange", json!({"select": "outOfGamut"})).unwrap();
        assert_eq!((coverage(&s, 5, 25), coverage(&s, 30, 25)), (1.0, 0.0));
        let warn = s.execute("view.gamutWarning", json!({"on": true})).unwrap();
        let sel = s.active().unwrap().doc.selection.clone().unwrap();
        let mut n = 0;
        for y in 0..30 {
            for x in 0..40 {
                n += usize::from(sel.sample_channel(x, y, 0) > 0.5);
            }
        }
        assert_eq!(warn["outOfGamut"].as_u64(), Some(n as u64));
    }

    #[test]
    fn color_range_rejects_bad_params() {
        let mut s = session_colours();
        for p in [
            json!({"select": "skin"}),
            json!({"select": 3}),
            json!({"points": [[100, 2]]}),
            json!({"points": [[1]]}),
            json!({"points": "here"}),
            json!({"colors": "#ff0000"}),
            json!({"localized": true}),
            json!({"select": "midtones", "tonalRange": [200, 100]}),
            json!({"select": "shadows", "tonalRange": 300}),
        ] {
            assert!(s.execute("select.colorRange", p.clone()).is_err(), "{p}");
        }
        assert!(s.active().unwrap().doc.selection.is_none());
    }

    #[test]
    fn modify_commands() {
        let mut s = session();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
        s.execute("select.modify.expand", json!({"radius": 2})).unwrap();
        assert_eq!(coverage(&s, 8, 15), 1.0);
        s.execute("select.modify.contract", json!({"radius": 4})).unwrap();
        assert_eq!(coverage(&s, 11, 15), 0.0);
        assert_eq!(coverage(&s, 15, 15), 1.0);
        s.execute("select.modify.border", json!({"radius": 2})).unwrap();
        assert_eq!(coverage(&s, 15, 15), 0.0);
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
        s.execute("select.modify.feather", json!({"radius": 4})).unwrap();
        let e = coverage(&s, 10, 15);
        assert!(e > 0.2 && e < 0.8, "{e}");
        s.execute("select.modify.smooth", json!({"radius": 2})).unwrap();
        assert!(s.active().unwrap().doc.selection.is_some());
        assert!(!s.is_enabled("select.nothing"));
        // Out-of-range or non-finite radii are refused and leave the selection alone.
        for op in ["feather", "smooth", "expand", "contract", "border"] {
            for r in [json!(1e300), json!(5000), json!(f64::MAX), json!(-5), json!(-0.5), json!(f64::MIN)] {
                assert!(s.execute(&format!("select.modify.{op}"), json!({"radius": r})).is_err(), "{op} {r}");
            }
        }
        assert!(s.active().unwrap().doc.selection.is_some());
    }

    #[test]
    fn modify_select_all_honours_canvas_bounds() {
        let all = || {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
            s.execute("select.all", json!({})).unwrap();
            s
        };
        // Off (Photoshop's default): the selection continues past the canvas, so the edges stay.
        for op in ["contract", "smooth", "expand", "feather"] {
            let mut s = all();
            s.execute(&format!("select.modify.{op}"), json!({"radius": 4})).unwrap();
            assert_eq!(coverage(&s, 0, 15), 1.0, "{op}");
            assert_eq!(coverage(&s, 20, 0), 1.0, "{op}");
        }
        // On: the canvas edge is a selection edge (#1277).
        let mut s = all();
        s.execute("select.modify.contract", json!({"radius": 4, "applyAtCanvasBounds": true})).unwrap();
        assert_eq!(coverage(&s, 2, 15), 0.0);
        assert_eq!(coverage(&s, 20, 28), 0.0);
        assert_eq!(coverage(&s, 20, 15), 1.0);
        let mut s = all();
        s.execute("select.modify.feather", json!({"radius": 4, "applyAtCanvasBounds": true})).unwrap();
        let e = coverage(&s, 0, 15);
        assert!(e > 0.2 && e < 0.8, "{e}");
        assert_eq!(coverage(&s, 20, 15), 1.0);
        let mut s = all();
        s.execute("select.modify.smooth", json!({"radius": 4, "applyAtCanvasBounds": true})).unwrap();
        assert!(coverage(&s, 0, 0) < 1.0);
        // Border always follows the canvas edge: a band inside it, not an empty selection.
        let mut s = all();
        s.execute("select.modify.border", json!({"radius": 4})).unwrap();
        assert!(coverage(&s, 0, 15) > 0.0);
        assert_eq!(coverage(&s, 20, 15), 0.0);
    }

    #[test]
    fn modify_away_from_the_canvas_edge_ignores_canvas_bounds() {
        for at in [false, true] {
            let mut s = session();
            s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
            s.execute("select.modify.contract", json!({"radius": 4, "applyAtCanvasBounds": at})).unwrap();
            assert_eq!(coverage(&s, 11, 15), 0.0);
            assert_eq!(coverage(&s, 15, 15), 1.0);
        }
    }

    #[test]
    fn modify_refuses_radii_below_the_minimum() {
        let mut s = session();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
        // Below each dialog's minimum (1 px, Feather 0.1 px) is refused, not clamped to 0: a
        // negative Border radius used to clear the selection and report success (#993).
        let below = [
            ("border", -5.0),
            ("border", 0.5),
            ("smooth", -5.0),
            ("smooth", 0.0),
            ("expand", -5.0),
            ("expand", 0.0),
            ("contract", -5.0),
            ("contract", 0.9),
            ("feather", -5.0),
            ("feather", 0.0),
            ("feather", 0.05),
        ];
        let before = current_mask(&s.active().unwrap().doc).1;
        for (op, r) in below {
            let err = s.execute(&format!("select.modify.{op}"), json!({"radius": r})).unwrap_err();
            assert!(matches!(err, EngineError::BadParams { .. }), "{op} {r}: {err}");
            assert!(current_mask(&s.active().unwrap().doc).1 == before, "{op} {r}: the selection changed");
        }
        // The minimum itself is still accepted.
        s.execute("select.modify.feather", json!({"radius": 0.1})).unwrap();
        s.execute("select.modify.expand", json!({"radius": 1})).unwrap();
        assert_eq!(coverage(&s, 9, 15), 1.0);
    }

    #[test]
    fn geometry_params_that_would_wrap_are_rejected() {
        let mut s = session();
        // 2^32 + 50 wrapped to `x = 50` and 3e9 to a negative coordinate through `as i32`;
        // both are bad-params errors now, whatever the front door (UI, CLI, control, MCP).
        for x in [4_294_967_346_i64, 3_000_000_000_i64] {
            let err = s.execute("select.rect", json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap_err();
            assert!(err.to_string().contains("32-bit"), "{err}");
        }
        let err = s.execute("document.pixel", json!({"x": 4_294_967_346_i64, "y": 0})).unwrap_err();
        assert!(err.to_string().contains("32 bits"), "{err}");
        // In-range coordinates, including negative and past-canvas ones, are unaffected.
        s.execute("select.rect", json!({"x": -100, "y": -100, "width": 500, "height": 500})).unwrap();
        s.execute("document.pixel", json!({"x": 0, "y": 0})).unwrap();
    }

    #[test]
    fn grow_and_similar() {
        let mut s = session();
        s.execute("select.rect", json!({"x": 6, "y": 6, "width": 2, "height": 2})).unwrap();
        s.execute("select.grow", json!({"tolerance": 5})).unwrap();
        assert_eq!(coverage(&s, 14, 14), 1.0);
        assert_eq!(coverage(&s, 30, 10), 0.0);
        s.execute("select.similar", json!({"tolerance": 5})).unwrap();
        assert_eq!(coverage(&s, 30, 10), 1.0);
        assert_eq!(coverage(&s, 20, 20), 0.0);
    }

    #[test]
    fn lasso_polygon() {
        let mut s = session();
        s.execute("select.lasso", json!({"points": [[0, 0], [20, 0], [0, 20]]})).unwrap();
        assert_eq!(coverage(&s, 2, 2), 1.0);
        assert_eq!(coverage(&s, 18, 18), 0.0);
        assert!(s.execute("select.lasso", json!({"points": [[0, 0], [1, 1]]})).is_err());
        s.execute("select.lasso", json!({"points": [[0, 0], [20, 0], [0, 20]], "mode": "subtract"})).unwrap();
        assert!(s.active().unwrap().doc.selection.is_none());
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(coverage(&s, 2, 2), 1.0);
    }
}
