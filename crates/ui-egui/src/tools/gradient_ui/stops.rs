//! The one conversion between the document's gradient (`Fill::Gradient`, `GradientPreset`) and the framework's
//! (`GradientStops`, `GradientFill`).
//!
//! Colour stops are sorted by location on both sides, so a `StopColor::index` is the engine's sorted index. A stop's
//! `tag` is its place in that sorted list plus one (0 for a stop the editor added), which lets `apply` give an
//! unchanged stop its document `Color` back: CMYK and Lab stops stay in their model. A preset stop's tag is 1 for the
//! live foreground colour and 2 for the background.

use craft::behavior::kurbo::Vec2;
use craft::color_ui::ui::gradient::{ColorStop, GradientStops, OpacityStop};
use craft::draw::ui::gradient::{GradientFill, GradientStyle};
use craft::theme::Srgba;
use photocraft_compose::gradient_fill as gf;
use photocraft_doc::{Color, Fill, GradientStyle as DocStyle};
use photocraft_engine::gradient_fill_cmds as cmds;
use photocraft_engine::presets::gradients::{GradientPreset, StopColor};
use serde_json::{Value, json};

/// The tag of a preset stop that follows the foreground colour.
const FOREGROUND: u64 = 1;
/// The tag of a preset stop that follows the background colour.
const BACKGROUND: u64 = 2;
/// How close two channels are for a stop to count as unchanged.
const SAME: f32 = 1e-6;

fn style_of(style: DocStyle) -> GradientStyle {
    match style {
        DocStyle::Linear => GradientStyle::Linear,
        DocStyle::Radial => GradientStyle::Radial,
        DocStyle::Angle => GradientStyle::Angle,
        DocStyle::Reflected => GradientStyle::Reflected,
        DocStyle::Diamond => GradientStyle::Diamond,
    }
}

fn doc_style(style: GradientStyle) -> DocStyle {
    match style {
        GradientStyle::Radial => DocStyle::Radial,
        GradientStyle::Angle => DocStyle::Angle,
        GradientStyle::Reflected => DocStyle::Reflected,
        GradientStyle::Diamond => DocStyle::Diamond,
        _ => DocStyle::Linear,
    }
}

/// The style the tool options name (`"linear"`, `"radial"`, …); linear for a name it doesn't know.
pub(crate) fn style_named(name: &str) -> GradientStyle {
    match name.to_ascii_lowercase().as_str() {
        "radial" => GradientStyle::Radial,
        "angle" => GradientStyle::Angle,
        "reflected" => GradientStyle::Reflected,
        "diamond" => GradientStyle::Diamond,
        _ => GradientStyle::Linear,
    }
}

/// The name the tool options and the gradient commands give `style`; the inverse of [`style_named`].
pub(crate) fn style_name(style: GradientStyle) -> &'static str {
    cmds::style_name(doc_style(style))
}

fn sorted<T: Copy>(list: &[(f32, T)]) -> Vec<(f32, T)> {
    let mut list = list.to_vec();
    list.sort_by(|a, b| a.0.total_cmp(&b.0));
    list
}

fn srgba(color: &Color) -> Srgba {
    let [r, g, b] = color.to_rgb();
    Srgba::new(r, g, b, color.alpha)
}

fn is_opaque(opacity: &[(f32, f32)]) -> bool {
    opacity.iter().all(|(_, a)| (a - 1.0).abs() < SAME)
}

/// The framework's stops of the gradient fill `fill`; empty stops for any other fill.
pub(crate) fn of_fill(fill: &Fill) -> GradientStops {
    let Fill::Gradient { stops, opacity_stops, midpoints, .. } = fill else { return GradientStops::default() };
    let color = sorted(stops)
        .iter()
        .enumerate()
        .map(|(i, (t, c))| ColorStop::new(*t, srgba(c)).with_midpoint(midpoints.get(i).copied().unwrap_or(0.5)).with_tag(i as u64 + 1))
        .collect();
    let opacity = sorted(opacity_stops).iter().enumerate().map(|(i, (t, a))| OpacityStop::new(*t, *a).with_tag(i as u64 + 1)).collect();
    GradientStops::with_opacity(color, opacity)
}

/// The framework's stops of the tool gradient `preset`, its foreground and background stops resolved to `fg` and `bg`
/// and tagged so they stay live.
pub(crate) fn of_preset(preset: &GradientPreset, fg: [f32; 4], bg: [f32; 4]) -> GradientStops {
    let color = sorted(&preset.stops)
        .iter()
        .map(|(t, c)| match c {
            StopColor::Foreground => ColorStop::new(*t, Srgba::rgb(fg[0], fg[1], fg[2])).with_tag(FOREGROUND),
            StopColor::Background => ColorStop::new(*t, Srgba::rgb(bg[0], bg[1], bg[2])).with_tag(BACKGROUND),
            StopColor::Rgb(rgb) => ColorStop::new(*t, Srgba::rgb(rgb[0], rgb[1], rgb[2])),
        })
        .collect();
    let opacity = if is_opaque(&preset.opacity) { Vec::new() } else { sorted(&preset.opacity).iter().map(|(t, a)| OpacityStop::new(*t, *a)).collect() };
    GradientStops::with_opacity(color, opacity)
}

/// `fill` with the stops `stops`: a stop whose sRGB did not change keeps its document colour, a picked colour becomes
/// RGB (the document's mode is applied where the fill is committed). Midpoints are centred when every one is.
/// A fill that is not a gradient is returned as it is.
pub(crate) fn apply(fill: &Fill, stops: &GradientStops) -> Fill {
    let Fill::Gradient { stops: old, .. } = fill else { return fill.clone() };
    let old = sorted(old);
    let colors: Vec<(f32, Color)> = stops
        .color
        .iter()
        .map(|s| {
            let kept = usize::try_from(s.tag).ok().and_then(|tag| tag.checked_sub(1)).and_then(|i| old.get(i)).map(|(_, c)| *c).filter(|c| {
                let [r, g, b] = c.to_rgb();
                (r - s.color.r).abs() < SAME && (g - s.color.g).abs() < SAME && (b - s.color.b).abs() < SAME
            });
            (s.location, kept.unwrap_or_else(|| Color::rgba(s.color.r, s.color.g, s.color.b, s.color.a)))
        })
        .collect();
    let mut mids: Vec<f32> = stops.color.iter().take(stops.color.len().saturating_sub(1)).map(|s| s.midpoint).collect();
    if mids.iter().all(|m| (m - 0.5).abs() < SAME) {
        mids.clear();
    }
    let opacity: Vec<(f32, f32)> = stops.opacity.iter().map(|s| (s.location, s.opacity)).collect();
    let mut out = fill.clone();
    if let Fill::Gradient { stops, midpoints, opacity_stops, .. } = &mut out {
        *stops = colors;
        *midpoints = mids;
        *opacity_stops = if is_opaque(&opacity) { Vec::new() } else { opacity };
    }
    out
}

/// The framework's value of the gradient fill `fill`; a plain linear fill of no stops for any other fill.
pub(crate) fn fill_value(fill: &Fill) -> GradientFill {
    let mut value = GradientFill::new(of_fill(fill));
    if let Fill::Gradient { angle, scale, style, reverse, offset, dither, align, .. } = fill {
        value.style = style_of(*style);
        value.angle = f64::from(*angle);
        value.scale = f64::from(*scale);
        value.reverse = *reverse;
        value.dither = *dither;
        value.align = *align;
        value.offset = Vec2::new(f64::from(offset.0), f64::from(offset.1));
    }
    value
}

/// `fill` with the framework's value `value`: the stops through [`apply`], the other fields as they are.
pub(crate) fn apply_value(fill: &Fill, value: &GradientFill) -> Fill {
    let mut out = apply(fill, &value.stops);
    if let Fill::Gradient { angle, scale, style, reverse, offset, dither, align, .. } = &mut out {
        *style = doc_style(value.style);
        *angle = value.angle as f32;
        *scale = value.scale as f32;
        *reverse = value.reverse;
        *dither = value.dither;
        *align = value.align;
        *offset = (value.offset.x as f32, value.offset.y as f32);
    }
    out
}

/// The `gradient.fill.set` params of `stops`: `stops`, `transparency` (percent) and `midpoints`.
pub(crate) fn fill_params(stops: &GradientStops) -> Value {
    let color: Vec<Value> = stops.color.iter().map(|s| json!([s.location, [s.color.r, s.color.g, s.color.b, s.color.a]])).collect();
    let transparency: Vec<Value> = stops.opacity.iter().map(|s| json!([s.location, s.opacity * 100.0])).collect();
    let midpoints: Vec<f32> = stops.color.iter().take(stops.color.len().saturating_sub(1)).map(|s| s.midpoint).collect();
    json!({"stops": color, "transparency": transparency, "midpoints": midpoints})
}

/// The `gradient.fill.set` params that turn `before` into `after`: only the fields that differ, so a checkbox leaves the
/// stops (and a CMYK document's stop colours) alone. An empty object when nothing differs.
pub(crate) fn value_params(before: &GradientFill, after: &GradientFill) -> Value {
    let mut p = serde_json::Map::new();
    if before.stops != after.stops
        && let Value::Object(stops) = fill_params(&after.stops)
    {
        p.extend(stops);
    }
    if before.style != after.style {
        p.insert("style".into(), json!(style_name(after.style)));
    }
    if before.angle != after.angle {
        p.insert("angle".into(), json!(after.angle));
    }
    if before.scale != after.scale {
        p.insert("scale".into(), json!(after.scale * 100.0));
    }
    if before.reverse != after.reverse {
        p.insert("reverse".into(), json!(after.reverse));
    }
    if before.dither != after.dither {
        p.insert("dither".into(), json!(after.dither));
    }
    if before.align != after.align {
        p.insert("align".into(), json!(after.align));
    }
    if before.offset != after.offset {
        p.insert("offset".into(), json!([after.offset.x * 100.0, after.offset.y * 100.0]));
    }
    Value::Object(p)
}

/// The `gradient.presets.stop {action: "set"}` params of `stops`: a stop tagged foreground or background stays live.
pub(crate) fn preset_params(stops: &GradientStops) -> Value {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let color: Vec<Value> = stops
        .color
        .iter()
        .map(|s| match s.tag {
            FOREGROUND => json!([s.location, "foreground"]),
            BACKGROUND => json!([s.location, "background"]),
            _ => json!([s.location, format!("#{:02x}{:02x}{:02x}", q(s.color.r), q(s.color.g), q(s.color.b))]),
        })
        .collect();
    let transparency: Vec<Value> = stops.opacity.iter().map(|s| json!([s.location, s.opacity * 100.0])).collect();
    json!({"action": "set", "stops": color, "transparency": transparency})
}

/// The sampler of a gradient bar or swatch over the compositor's ramp, with the foreground and background tags shown
/// as the live colours `fg` and `bg`.
pub(crate) fn sampler(fg: [f32; 4], bg: [f32; 4]) -> impl Fn(&GradientStops, f32) -> Srgba {
    move |stops: &GradientStops, at: f32| {
        let mut shown = stops.clone();
        for s in &mut shown.color {
            match s.tag {
                FOREGROUND => s.color = Srgba::rgb(fg[0], fg[1], fg[2]),
                BACKGROUND => s.color = Srgba::rgb(bg[0], bg[1], bg[2]),
                _ => {}
            }
        }
        // The ramp reads a fill; the layout fields don't matter for a ramp.
        let base = Fill::gradient(Vec::new(), 90.0, 1.0, DocStyle::Linear, false);
        match gf::Ramp::new(&apply(&base, &shown)) {
            Some(ramp) => {
                let [r, g, b, a] = ramp.sample(at);
                Srgba::new(r, g, b, a)
            }
            None => Srgba::BLACK,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill() -> Fill {
        let stops = vec![(0.5, Color::rgb(1.0, 0.0, 0.0)), (0.0, Color::rgb(0.0, 0.0, 1.0)), (1.0, Color::rgb(0.0, 1.0, 0.0))];
        Fill::gradient(stops, 45.0, 2.0, DocStyle::Radial, true)
    }

    /// A stop dragged past its neighbour keeps its document colour, and the midpoints follow the stops.
    #[test]
    fn a_moved_stop_keeps_its_colour_and_a_picked_one_becomes_rgb() {
        let before = fill();
        let mut stops = of_fill(&before);
        assert_eq!(stops.color.iter().map(|s| s.tag).collect::<Vec<_>>(), [1, 2, 3], "sorted index plus one");
        stops.color[2].location = 0.25;
        stops.color.sort_by(|a, b| a.location.total_cmp(&b.location));
        stops.color[0].color = Srgba::rgb(0.2, 0.4, 0.6);
        let Fill::Gradient { stops: after, .. } = apply(&before, &stops) else { panic!("a gradient") };
        assert_eq!(after.iter().map(|s| s.0).collect::<Vec<_>>(), [0.0, 0.25, 0.5]);
        assert_eq!(after[0].1, Color::rgb(0.2, 0.4, 0.6));
        assert_eq!(after[1].1, Color::rgb(0.0, 1.0, 0.0));
        assert_eq!(after[2].1, Color::rgb(1.0, 0.0, 0.0));
    }

    /// The value round trip carries dither and offset, and `value_params` names only what changed.
    #[test]
    fn a_value_round_trips_and_its_diff_names_only_the_change() {
        // Sorted, as `apply` writes its stops.
        let mut before = Fill::gradient(vec![(0.0, Color::rgb(0.0, 0.0, 1.0)), (1.0, Color::rgb(0.0, 1.0, 0.0))], 45.0, 2.0, DocStyle::Radial, true);
        if let Fill::Gradient { dither, offset, .. } = &mut before {
            *dither = true;
            *offset = (0.1, -0.2);
        }
        let value = fill_value(&before);
        assert!(value.dither && value.style == GradientStyle::Radial);
        assert_eq!(apply_value(&before, &value), before);
        let mut after = value.clone();
        after.reverse = false;
        assert_eq!(value_params(&value, &after), json!({"reverse": false}));
        assert_eq!(value_params(&value, &value), json!({}));
    }

    #[test]
    fn preset_foreground_and_background_stay_live_in_the_params() {
        let preset = GradientPreset::foreground_to_background();
        let stops = of_preset(&preset, [1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(stops.color[0].color, Srgba::rgb(1.0, 0.0, 0.0));
        let p = preset_params(&stops);
        assert_eq!(p["stops"], json!([[0.0, "foreground"], [1.0, "background"]]));
    }
}
