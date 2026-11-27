//! Keep editable stops separate from the dense stops used by the compositors.

use photocraft_doc::effects::GradientInterpolation;
use photocraft_doc::{Color, Gradient};
use serde_json::Value;

use super::{color_of, f_of, overlay_gradient};

fn edit_endpoint(stop: Option<&mut (f32, Color)>, params: &Value, key: &str) {
    if let (Some(stop), Some(color)) = (stop, color_of(params, key)) {
        // The dialog displays hex colours. Opening it must not quantize the saved source.
        let [r, g, b, _] = color.to_rgba8();
        let [sr, sg, sb, _] = stop.1.to_rgba8();
        if !params.get(key).is_some_and(Value::is_string) || [r, g, b] != [sr, sg, sb] {
            let mut edited = color.in_mode(stop.1.mode);
            edited.alpha = stop.1.alpha;
            stop.1 = edited;
        }
    }
}

/// `params` without a `from`/`to` hex string that only repeats the gradient's current end as the
/// dialog shows it (8-bit): confirming the dialog unchanged must not quantize a baked or
/// imported curve's exact ends (as `edit_endpoint` guards the recorded source).
fn without_unchanged_ends(gradient: &Gradient, params: &Value) -> Value {
    let mut params = params.clone();
    for (key, end) in [("from", gradient.stops.first()), ("to", gradient.stops.last())] {
        let shown = |c: &Color| {
            let [r, g, b, _] = c.to_rgba8();
            [r, g, b]
        };
        let same = params.get(key).is_some_and(Value::is_string) && end.zip(color_of(&params, key)).is_some_and(|(end, color)| shown(&end.1) == shown(&color));
        if same && let Some(object) = params.as_object_mut() {
            object.remove(key);
        }
    }
    params
}

/// Edits recorded source stops, initializing legacy curves from their end colours.
pub(super) fn overlay(gradient: &mut Gradient, interpolation: &mut Option<GradientInterpolation>, params: &Value) {
    // Reset to Default restores the factory's unbaked interpolation too.
    if params.get("method").and_then(Value::as_str) == Some("custom") {
        if let Some(source) = interpolation.take() {
            gradient.stops = source.stops;
        }
        overlay_gradient(gradient, &without_unchanged_ends(gradient, params));
        return;
    }
    let requested = params.get("method").and_then(Value::as_str).filter(|name| photocraft_io::GradientMethod::from_name(name).is_some());
    if interpolation.is_none() && requested.is_none() {
        overlay_gradient(gradient, &without_unchanged_ends(gradient, params));
        return;
    }
    let previous = interpolation.clone();
    let source = interpolation.get_or_insert_with(|| {
        GradientInterpolation::new(
            requested.unwrap_or("classic").to_ascii_lowercase(),
            100.0,
            // Without a source record, intermediate stops may be baked samples (older PSD
            // imports or method commands). Keep the legacy rebuild-from-ends behaviour on first
            // choice.
            vec![(0.0, gradient.stops.first().map_or(Color::BLACK, |s| s.1)), (1.0, gradient.stops.last().map_or(Color::WHITE, |s| s.1))],
        )
    });
    if let Some(method) = requested {
        source.method = method.to_ascii_lowercase();
    }
    if let Some(smoothness) = f_of(params, "smoothness").filter(|n| n.is_finite()) {
        source.smoothness = smoothness.clamp(0.0, 100.0);
    }
    if let Ok(Some(stops)) = super::stops_param(params, "layer.layerStyle.gradientOverlay") {
        // Midpoints belong to the segments: a different number of stops starts them halfway.
        if stops.len() != source.stops.len() {
            source.midpoints.clear();
        }
        source.stops = stops;
    }
    // A gradient read from a PSD keeps Photoshop's own opacity stops (#13557): an edit replaces
    // them and is baked with the colours. Without a record the opacity stops are used as given.
    let own_opacity = !source.opacity_stops.is_empty();
    if own_opacity && let Ok(Some(opacity)) = super::opacity_stops_param(params, "layer.layerStyle.gradientOverlay") {
        if opacity.len() != source.opacity_stops.len() {
            source.opacity_midpoints.clear();
        }
        if opacity.is_empty() {
            // No opacity stops: the gradient is opaque, as without a record.
            gradient.opacity_stops.clear();
        }
        source.opacity_stops = opacity;
    }
    edit_endpoint(source.stops.first_mut(), params, "from");
    edit_endpoint(source.stops.last_mut(), params, "to");
    if previous.as_ref() != Some(source) && photocraft_io::gradient_bake::interpolation_method(source).is_some() {
        // Missing midpoint entries mean 50 %, including for multi-stop source gradients.
        gradient.stops = photocraft_io::gradient_bake::bake_interpolation(source);
        if let Some(opacity) = photocraft_io::gradient_bake::bake_interpolation_opacity(source) {
            gradient.opacity_stops = opacity;
        }
    }
    // Geometry edits must not replace just the ends of the baked curve.
    let mut geometry = params.clone();
    if let Some(object) = geometry.as_object_mut() {
        for key in ["method", "smoothness", "from", "to", "stops"] {
            object.remove(key);
        }
        if own_opacity {
            object.remove("opacityStops");
        }
    }
    overlay_gradient(gradient, &geometry);
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::BlendMode;
    use photocraft_doc::{Effect, FxCommon};
    use photocraft_io::{GradientMethod, bake_gradient};
    use serde_json::json;

    #[test]
    fn choosing_a_method_on_a_baked_overlay_rebuilds_from_its_end_colours() {
        let ends = vec![(0.0, Color::BLACK), (1.0, Color::WHITE)];
        for imported_method in [GradientMethod::Perceptual, GradientMethod::Smooth] {
            let gradient = Gradient {
                stops: bake_gradient(ends.clone(), &[0.5], 1.0, imported_method),
                opacity_stops: vec![(0.0, 0.25), (0.6, 0.8), (1.0, 0.5)],
                align: false,
                offset: (0.2, -0.1),
                ..Gradient::default()
            };
            assert!(gradient.stops.len() > 2);
            let source_ends = vec![(0.0, gradient.stops.first().unwrap().1), (1.0, gradient.stops.last().unwrap().1)];
            let mut fx =
                Effect::GradientOverlay { common: FxCommon::new(BlendMode::Normal, 1.0), gradient: gradient.clone(), dither: true, interpolation: None };
            super::super::overlay_effect(&mut fx, &json!({"method": "linear"}));
            let mut expected = gradient;
            expected.stops = bake_gradient(source_ends.clone(), &[0.5], 1.0, GradientMethod::Linear);
            assert_eq!(
                fx,
                Effect::GradientOverlay {
                    common: FxCommon::new(BlendMode::Normal, 1.0),
                    gradient: expected,
                    dither: true,
                    interpolation: Some(GradientInterpolation::new("linear", 100.0, source_ends)),
                }
            );
        }
    }

    #[test]
    fn endpoint_edits_after_selecting_a_method_recolour_the_whole_baked_overlay() {
        for same_edit in [false, true] {
            let mut fx = Effect::GradientOverlay {
                common: FxCommon::new(BlendMode::Normal, 1.0),
                gradient: Gradient {
                    stops: bake_gradient(vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], &[0.5], 1.0, GradientMethod::Smooth),
                    ..Gradient::default()
                },
                dither: false,
                interpolation: None,
            };
            if same_edit {
                super::super::overlay_effect(&mut fx, &json!({"method": "classic", "from": "#ff0000", "to": "#0000ff"}));
            } else {
                super::super::overlay_effect(&mut fx, &json!({"method": "classic"}));
                super::super::overlay_effect(&mut fx, &json!({"from": "#ff0000", "to": "#0000ff"}));
            }
            let ends = vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))];
            let Effect::GradientOverlay { gradient, interpolation, .. } = fx else { panic!("expected overlay") };
            assert_eq!(gradient.stops, bake_gradient(ends.clone(), &[0.5], 1.0, GradientMethod::Classic));
            assert_eq!(interpolation.unwrap().stops, ends);
        }
    }

    #[test]
    fn changing_method_keeps_all_source_stops_and_transparency() {
        let mut gradient = Gradient {
            stops: vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (0.3, Color::rgb(0.0, 1.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))],
            opacity_stops: vec![(0.0, 0.25), (1.0, 0.8)],
            align: false,
            offset: (0.2, -0.1),
            ..Gradient::default()
        };
        let original = gradient.clone();
        let mut source = Some(GradientInterpolation::new("classic", 100.0, original.stops.clone()));
        gradient.stops = bake_gradient(original.stops.clone(), &[0.5, 0.5], 1.0, GradientMethod::Classic);
        for method in ["classic", "smooth", "perceptual", "linear", "classic"] {
            overlay(&mut gradient, &mut source, &json!({"method": method}));
            let source = source.as_ref().unwrap();
            assert_eq!(source.method, method);
            assert_eq!(source.stops, original.stops);
            let expected = photocraft_io::bake_gradient(original.stops.clone(), &[0.5, 0.5], 1.0, photocraft_io::GradientMethod::from_name(method).unwrap());
            assert_eq!(gradient.stops, expected);
            assert_eq!(gradient.opacity_stops, original.opacity_stops);
            assert_eq!(gradient.align, original.align);
            assert_eq!(gradient.offset, original.offset);
        }
        overlay(&mut gradient, &mut source, &json!({"from": "#112233", "to": "#445566"}));
        let source = source.unwrap();
        assert_eq!(source.stops.len(), 3);
        assert_eq!(source.stops[1], original.stops[1], "endpoint edits preserve the middle source stop");
        assert_eq!(gradient.stops, bake_gradient(source.stops, &[0.5, 0.5], 1.0, GradientMethod::Classic));
    }

    #[test]
    fn colour_edit_rebakes_the_curve_and_geometry_edits_leave_it_exact() {
        let mut gradient = Gradient::default();
        let mut source = None;
        overlay(&mut gradient, &mut source, &json!({"method": "smooth", "from": "#ff0000", "to": "#0000ff"}));
        let before = gradient.stops.clone();
        overlay(&mut gradient, &mut source, &json!({"from": "#00ff00", "angle": 45}));
        assert_ne!(gradient.stops, before);
        let expected = photocraft_io::bake_gradient(
            vec![(0.0, Color::rgb(0.0, 1.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))],
            &[0.5],
            1.0,
            photocraft_io::GradientMethod::Smooth,
        );
        assert_eq!(gradient.stops, expected);
        overlay(&mut gradient, &mut source, &json!({"angle": 90, "reverse": true, "scale": 75}));
        assert_eq!(gradient.stops, expected);
        assert_eq!(gradient.angle, 90.0);
        assert!(gradient.reverse);
        assert_eq!(gradient.scale, 0.75);
        // The offset (#10763) is geometry too: a fraction of the layer's bounds, left alone by
        // junk and clamped.
        overlay(&mut gradient, &mut source, &json!({"offset": [0.25, -0.1]}));
        assert_eq!(gradient.offset, (0.25, -0.1));
        assert_eq!(gradient.stops, expected);
        overlay(&mut gradient, &mut source, &json!({"offset": "far"}));
        overlay(&mut gradient, &mut source, &json!({"offset": [f64::NAN, 0.0]}));
        assert_eq!(gradient.offset, (0.25, -0.1));
        overlay(&mut gradient, &mut source, &json!({"offset": [100.0, -100.0]}));
        assert_eq!(gradient.offset, (4.0, -4.0));
    }

    #[test]
    fn reopening_does_not_quantize_source_colours_or_rebake() {
        let mut gradient = Gradient::default();
        gradient.stops[0].1.alpha = 0.37;
        let mut source = None;
        overlay(&mut gradient, &mut source, &json!({"from": [0.123456, 0.234567, 0.345678], "method": "perceptual"}));
        let before = (gradient.clone(), source.clone());
        assert_eq!(source.as_ref().unwrap().stops[0].1.alpha, 0.37);
        let [r, g, b, _] = source.as_ref().unwrap().stops[0].1.to_rgba8();
        overlay(&mut gradient, &mut source, &json!({"from": format!("#{r:02x}{g:02x}{b:02x}"), "to": "#ffffff", "method": "perceptual", "smoothness": 100}));
        assert_eq!((gradient, source), before);
    }

    #[test]
    fn old_gradients_are_unchanged_until_a_method_is_chosen() {
        let mut gradient = Gradient::default();
        gradient.stops.insert(1, (0.4, Color::rgb(0.1, 0.9, 0.2)));
        let original = gradient.clone();
        let mut source = None;
        overlay(&mut gradient, &mut source, &json!({"method": "custom", "opacity": 50}));
        assert_eq!(gradient, original);
        assert!(source.is_none());
    }

    #[test]
    fn endpoint_edits_keep_the_source_colour_model() {
        for mode in [photocraft_doc::ColorMode::Grayscale, photocraft_doc::ColorMode::Cmyk, photocraft_doc::ColorMode::Lab] {
            let mut gradient = Gradient { stops: vec![(0.0, Color::BLACK.in_mode(mode)), (1.0, Color::WHITE.in_mode(mode))], ..Gradient::default() };
            let mut source = None;
            overlay(&mut gradient, &mut source, &json!({"method": "classic"}));
            overlay(&mut gradient, &mut source, &json!({"from": "#ff0000"}));
            assert!(source.as_ref().unwrap().stops.iter().all(|s| s.1.mode == mode));
        }
    }

    #[test]
    fn reset_to_factory_defaults_removes_the_recorded_method() {
        let mut gradient = Gradient::default();
        let mut source = None;
        overlay(&mut gradient, &mut source, &json!({"method": "smooth", "from": "#ff0000"}));
        overlay(&mut gradient, &mut source, &super::super::effect_defaults("gradientOverlay"));
        assert!(source.is_none());
        assert_eq!(gradient, Gradient::default());
    }
}

#[cfg(test)]
mod integration_stop_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn arbitrary_source_edits_rebake_and_preserve_opacity() {
        let mut gradient = Gradient::default();
        let mut source = None;
        overlay(
            &mut gradient,
            &mut source,
            &json!({"method":"smooth","stops":[[0.0,"#ff0000"],[0.3,"#00ff00"],[1.0,"#0000ff"]],"opacityStops":[[0.0,20.0],[1.0,80.0]]}),
        );
        let stops = source.as_ref().unwrap().stops.clone();
        assert_eq!(stops.len(), 3);
        assert_eq!(gradient.stops, photocraft_io::bake_gradient(stops, &[], 1.0, photocraft_io::GradientMethod::Smooth));
        assert_eq!(gradient.opacity_stops, vec![(0.0, 0.2), (1.0, 0.8)]);
    }
}

#[cfg(test)]
mod photoshop_source_tests {
    use super::*;
    use serde_json::json;

    /// An overlay as PSD import leaves a Photoshop gradient (#13557): three stops with a midpoint
    /// off the middle and its own opacity fade, rendered baked.
    fn imported() -> (Gradient, Option<GradientInterpolation>) {
        let mut src = GradientInterpolation::new(
            "perceptual",
            100.0,
            vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (0.5, Color::rgb(0.0, 1.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))],
        );
        src.midpoints = vec![0.3, 0.5];
        src.opacity_stops = vec![(0.0, 1.0), (1.0, 0.2)];
        src.opacity_midpoints = vec![0.4];
        let gradient = Gradient {
            stops: photocraft_io::gradient_bake::bake_interpolation(&src),
            opacity_stops: photocraft_io::gradient_bake::bake_interpolation_opacity(&src).unwrap(),
            ..Gradient::default()
        };
        (gradient, Some(src))
    }

    #[test]
    fn edits_bake_photoshops_midpoints_and_opacity_stops() {
        let (mut gradient, mut source) = imported();
        // Geometry alone leaves the bake as it is.
        let before = gradient.stops.clone();
        overlay(&mut gradient, &mut source, &json!({"angle": 30}));
        assert_eq!(gradient.stops, before);
        // A colour edit keeps the midpoints in the bake.
        overlay(&mut gradient, &mut source, &json!({"stops": [[0, "#ff0000"], [0.5, "#ffff00"], [1, "#0000ff"]]}));
        let src = source.clone().unwrap();
        assert_eq!(src.midpoints, vec![0.3, 0.5]);
        assert_eq!(gradient.stops, photocraft_io::gradient_bake::bake_interpolation(&src));
        // A different number of stops starts every segment halfway.
        overlay(&mut gradient, &mut source, &json!({"stops": [[0, "#ff0000"], [1, "#0000ff"]]}));
        assert!(source.as_ref().unwrap().midpoints.is_empty());
        // Photoshop's opacity stops are edited and baked, not used as linear stops.
        overlay(&mut gradient, &mut source, &json!({"opacityStops": [[0, 100], [0.5, 50], [1, 0]]}));
        let src = source.unwrap();
        assert_eq!(src.opacity_stops, vec![(0.0, 1.0), (0.5, 0.5), (1.0, 0.0)]);
        assert!(src.opacity_midpoints.is_empty());
        assert_eq!(Some(gradient.opacity_stops), photocraft_io::gradient_bake::bake_interpolation_opacity(&src));
    }

    /// An explicit empty `opacityStops` makes the overlay opaque: Photoshop's opacity stops go
    /// and so does the transparency rendered from them.
    #[test]
    fn empty_opacity_stops_make_an_imported_overlay_opaque() {
        let (mut gradient, mut source) = imported();
        assert!(gradient.opacity_stops.iter().any(|o| o.1 < 0.99));
        overlay(&mut gradient, &mut source, &json!({"opacityStops": []}));
        let src = source.unwrap();
        assert!(src.opacity_stops.is_empty() && src.opacity_midpoints.is_empty(), "{src:?}");
        assert!(gradient.opacity_stops.iter().all(|o| o.1 >= 1.0 - 1e-4), "{:?}", gradient.opacity_stops);
    }
}
