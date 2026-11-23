//! The Gradient Overlay's gradient in the engine's command grammar and as the framework's
//! `GradientStops`. The command's `stops` is `[[position, "#rrggbb"], …]` and `opacityStops` is
//! `[[position, opacity %], …]`: what `layer.layerStyle.gradientOverlay` receives.

use craft::color_ui::gradient::{ColorStop, GradientStops, OpacityStop};
use craft::theme::Srgba;
use photocraft_color::Color;
use serde_json::{Value, json};

use crate::color_picker_ui;

/// The command parameter holding the colour stops.
pub(super) const KEY: &str = "stops";
/// The command parameter holding the transparency stops.
pub(super) const OPACITY_KEY: &str = "opacityStops";

/// The colour stop at `t` from one entry `[t, "#rrggbb" | Color]`; `None` for a malformed entry.
fn color_stop(entry: &Value) -> Option<ColorStop> {
    let t = entry.get(0)?.as_f64()?;
    let value = entry.get(1)?;
    let color = if let Some(hex) = value.as_str() {
        let [r, g, b] = color_picker_ui::parse_hex(hex)?;
        Color::rgb(r, g, b)
    } else {
        serde_json::from_value::<Color>(value.clone()).ok()?
    };
    if !(t.is_finite() && color.c.iter().all(|v| v.is_finite()) && color.alpha.is_finite()) {
        return None;
    }
    let [r, g, b] = color.to_rgb();
    Some(ColorStop::new(t.clamp(0.0, 1.0) as f32, Srgba::new(r, g, b, color.alpha)))
}

/// The transparency stop from one entry `[t, opacity %]`; `None` for a malformed entry.
fn opacity_stop(entry: &Value) -> Option<OpacityStop> {
    let (t, o) = (entry.get(0)?.as_f64()?, entry.get(1)?.as_f64()?);
    (t.is_finite() && o.is_finite()).then(|| OpacityStop::new(t.clamp(0.0, 1.0) as f32, (o / 100.0).clamp(0.0, 1.0) as f32))
}

/// The gradient of an effect's command parameters `params` (`stops` and `opacityStops`), stops in
/// location order; malformed entries are skipped. `None` with fewer than two colour stops. A single
/// transparency stop is not a list and reads as none (opaque).
pub(super) fn stops_from_json(params: &Value) -> Option<GradientStops> {
    let mut color: Vec<ColorStop> = params.get(KEY)?.as_array()?.iter().filter_map(color_stop).collect();
    if color.len() < 2 {
        return None;
    }
    color.sort_by(|a, b| a.location.total_cmp(&b.location));
    let mut opacity: Vec<OpacityStop> = params.get(OPACITY_KEY).and_then(Value::as_array).into_iter().flatten().filter_map(opacity_stop).collect();
    if opacity.len() < 2 {
        opacity.clear();
    }
    opacity.sort_by(|a, b| a.location.total_cmp(&b.location));
    Some(GradientStops::with_opacity(color, opacity))
}

/// The command's `(stops, opacityStops)` for `stops`. A colour that 8 bits hold exactly is its
/// `#rrggbb`; any other travels as the colour itself.
pub(super) fn stops_to_json(stops: &GradientStops) -> (Value, Value) {
    let color = stops
        .color
        .iter()
        .map(|s| {
            let c = s.color;
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            let exact = |v: f32| (v - f32::from(q(v)) / 255.0).abs() < 1e-6;
            if c.a == 1.0 && exact(c.r) && exact(c.g) && exact(c.b) {
                json!([s.location, color_picker_ui::hex([c.r, c.g, c.b])])
            } else {
                let mut color = Color::rgb(c.r, c.g, c.b);
                color.alpha = c.a;
                json!([s.location, color])
            }
        })
        .collect();
    let opacity = stops.opacity.iter().map(|s| json!([s.location, f64::from(s.opacity) * 100.0])).collect();
    (Value::Array(color), Value::Array(opacity))
}

/// Whether `a` and `b` hold the same stops up to the rounding a colour picks up through the command grammar.
pub(super) fn same(a: &GradientStops, b: &GradientStops) -> bool {
    let near = |x: f32, y: f32| (x - y).abs() < 1e-5;
    a.color.len() == b.color.len()
        && a.opacity.len() == b.opacity.len()
        && a.color.iter().zip(&b.color).all(|(x, y)| {
            near(x.location, y.location) && near(x.color.r, y.color.r) && near(x.color.g, y.color.g) && near(x.color.b, y.color.b) && near(x.color.a, y.color.a)
        })
        && a.opacity.iter().zip(&b.opacity).all(|(x, y)| near(x.location, y.location) && near(x.opacity, y.opacity))
}

/// The framework's gradient for a document gradient's colour and transparency stops.
pub(super) fn stops_of_doc(stops: &[(f32, Color)], opacity: &[(f32, f32)]) -> GradientStops {
    let color = stops
        .iter()
        .map(|(t, c)| {
            let [r, g, b] = c.to_rgb();
            ColorStop::new(*t, Srgba::new(r, g, b, c.alpha))
        })
        .collect();
    GradientStops::with_opacity(color, opacity.iter().map(|(t, o)| OpacityStop::new(*t, *o)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locations(g: &GradientStops) -> Vec<f32> {
        g.color.iter().map(|s| s.location).collect()
    }

    #[test]
    fn stops_round_trip_through_the_field_grammar() {
        let params = json!({"stops": [[0.0, "#ff0000"], [0.5, "#00ff00"], [1.0, "#0000ff"]], "opacityStops": [[0.0, 100.0], [1.0, 50.0]]});
        let g = stops_from_json(&params).unwrap();
        assert_eq!(locations(&g), [0.0, 0.5, 1.0]);
        assert_eq!(g.opacity.iter().map(|s| s.opacity).collect::<Vec<_>>(), [1.0, 0.5]);
        let (stops, opacity) = stops_to_json(&g);
        assert_eq!(stops, params["stops"]);
        assert_eq!(opacity, params["opacityStops"]);

        // Unsorted input is sorted and bad entries are skipped.
        let g = stops_from_json(&json!({"stops": [[1.0, "#ffffff"], ["x", "#ff0000"], [0.25, "#000000"], [0.5, 7]]})).unwrap();
        assert_eq!(locations(&g), [0.25, 1.0]);
        assert!(g.opacity.is_empty(), "no transparency stops is opaque");

        // Fewer than two colour stops, a non-list and a missing key are nothing; one transparency stop is none.
        assert!(stops_from_json(&json!({"stops": [[0.0, "#000000"]]})).is_none());
        assert!(stops_from_json(&json!({"stops": "red"})).is_none());
        assert!(stops_from_json(&json!({})).is_none());
        let g = stops_from_json(&json!({"stops": [[0.0, "#000000"], [1.0, "#ffffff"]], "opacityStops": [[0.0, 100.0]]})).unwrap();
        assert!(g.opacity.is_empty());
    }
}
