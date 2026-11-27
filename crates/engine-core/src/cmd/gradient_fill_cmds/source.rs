//! A Gradient Fill layer's kept Photoshop gradient (#13557). A fill read from a PSD with
//! Smoothness, a method other than Classic or Lab stops renders dense stops baked from
//! Photoshop's own few stops, which it keeps as [`GradientInterpolation`]. Its editors work on
//! a *view* of the fill holding those stops ([`source_view`]); an edited view is written back
//! into the record and baked again ([`rebake`]).

use photocraft_color::ColorMode;
use photocraft_doc::Fill;
use photocraft_doc::effects::GradientInterpolation;
use photocraft_io::gradient_bake;
use serde_json::Value;

use super::bad;
use crate::Result;

/// The fill as its editors show it: when it keeps Photoshop's own gradient, that gradient's
/// colour stops and midpoints (and its own opacity stops, when it has them) in place of the
/// baked ones. Any other fill is returned as it is.
pub fn source_view(f: &Fill) -> Fill {
    let mut out = f.clone();
    if let Fill::Gradient { stops, midpoints, opacity_stops, interpolation: Some(src), .. } = &mut out {
        let mut held = src.stops.clone();
        held.sort_by(|a, b| a.0.total_cmp(&b.0));
        *stops = held;
        *midpoints = src.midpoints.clone();
        if !src.opacity_stops.is_empty() {
            let mut held = src.opacity_stops.clone();
            held.sort_by(|a, b| a.0.total_cmp(&b.0));
            *opacity_stops = held;
        }
    }
    out
}

/// Colour stops, midpoints, opacity stops and (method, Smoothness) of a view.
type BakeInputs<'a> = (&'a Vec<(f32, photocraft_color::Color)>, &'a Vec<f32>, &'a Vec<(f32, f32)>, Option<(&'a str, f32)>);

/// The parts of a view that baking depends on.
fn bake_inputs(f: &Fill) -> Option<BakeInputs<'_>> {
    match f {
        Fill::Gradient { stops, midpoints, opacity_stops, interpolation, .. } => {
            Some((stops, midpoints, opacity_stops, interpolation.as_ref().map(|s| (s.method.as_str(), s.smoothness))))
        }
        _ => None,
    }
}

/// `edited` (a [`source_view`] of `original`, as `before`, with edits) as a fill again. When
/// nothing baking depends on changed, the original stops are kept exactly (geometry edits don't
/// bake again); else the view's stops become Photoshop's gradient and are baked. A gradient
/// that is plain again (Classic, Smoothness 0, not Lab) drops its record: its stops and
/// midpoints render as they are.
pub fn rebake(mut edited: Fill, before: &Fill, original: &Fill) -> Fill {
    let unchanged = bake_inputs(&edited) == bake_inputs(before);
    let Fill::Gradient { stops, midpoints, opacity_stops, interpolation, .. } = &mut edited else { return edited };
    if unchanged {
        if let Fill::Gradient { stops: s0, midpoints: m0, opacity_stops: o0, interpolation: i0, .. } = original {
            (*stops, *midpoints, *opacity_stops, *interpolation) = (s0.clone(), m0.clone(), o0.clone(), i0.clone());
        }
        return edited;
    }
    let Some(src) = interpolation else { return edited };
    let lab = stops.iter().all(|(_, c)| c.mode == ColorMode::Lab);
    let opaque = opacity_stops.iter().all(|o| o.1 >= 1.0 - 1e-4);
    if opaque {
        src.opacity_stops.clear();
        src.opacity_midpoints.clear();
    } else {
        if opacity_stops.len() != src.opacity_stops.len() {
            src.opacity_midpoints.clear();
        }
        src.opacity_stops = opacity_stops.clone();
    }
    let method = gradient_bake::interpolation_method(src);
    if method.is_none_or(|m| m == gradient_bake::Method::Classic) && src.smoothness <= 0.0 && !lab {
        // A fill holds no opacity midpoints: ones off the middle stay baked in.
        if let Some(baked) = gradient_bake::bake_interpolation_opacity(src) {
            *opacity_stops = baked;
        }
        *interpolation = None;
        return edited;
    }
    if stops.len() != src.stops.len() {
        // A new segment starts halfway; the view's midpoints already follow its stops.
        midpoints.resize(stops.len().saturating_sub(1), 0.5);
    }
    src.stops = stops.clone();
    src.midpoints = if midpoints.iter().all(|m| (m - 0.5).abs() < 1e-6) { Vec::new() } else { std::mem::take(midpoints) };
    midpoints.clear();
    *stops = gradient_bake::bake_interpolation(src);
    if let Some(baked) = gradient_bake::bake_interpolation_opacity(src) {
        *opacity_stops = baked;
    }
    edited
}

/// `gradient.fill.set`'s `method` and `smoothness` (Photoshop's interpolation) on a view: they
/// start a record when the fill has none (its stops and midpoints as they are).
pub fn apply_interpolation_params(cmd: &str, view: &mut Fill, p: &Value) -> Result<()> {
    let method = match p.get("method") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_str()
                .and_then(gradient_bake::Method::from_name)
                .ok_or_else(|| bad(cmd, "`method` is \"classic\", \"perceptual\", \"linear\" or \"smooth\""))?,
        ),
    };
    let smoothness = match p.get("smoothness") {
        None | Some(Value::Null) => None,
        Some(v) => {
            Some(v.as_f64().filter(|s| s.is_finite() && (0.0..=100.0).contains(s)).ok_or_else(|| bad(cmd, "`smoothness` is a percentage 0..100"))? as f32)
        }
    };
    if method.is_none() && smoothness.is_none() {
        return Ok(());
    }
    let Fill::Gradient { stops, midpoints, interpolation, .. } = view else { return Err(bad(cmd, "not a gradient fill")) };
    let src = interpolation.get_or_insert_with(|| {
        let mut src = GradientInterpolation::new("classic", 0.0, stops.clone());
        src.midpoints = midpoints.clone();
        Box::new(src)
    });
    if let Some(m) = method {
        src.method = m.name().to_string();
    }
    if let Some(s) = smoothness {
        src.smoothness = s;
    }
    Ok(())
}
