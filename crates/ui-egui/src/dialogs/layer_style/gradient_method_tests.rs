use photocraft_doc::Effect;
use serde_json::json;

use super::tests::{
    ColorStop, DialogId, EffectId, EffectKind, GradientStops, LayerStyleDraft, Srgba, StyleValue, app_with_layer, draft, edit, first_of, ok, param,
};
use super::*;
use crate::PhotocraftApp;

fn app() -> Box<PhotocraftApp> {
    app_with_layer(32)
}

fn effect(app: &PhotocraftApp) -> &Effect {
    let state = app.session.active().unwrap();
    &state.doc.layer(state.active_layer.unwrap()).unwrap().effects.items[0]
}

/// Opens the dialog on Gradient Overlay and returns it with the overlay's id.
fn open_gradient(app: &mut PhotocraftApp) -> (DialogId, EffectId) {
    let id = open(app, Some(EffectKind::GradientOverlay.key())).unwrap();
    let overlay = first_of(app, id, EffectKind::GradientOverlay);
    (id, overlay)
}

fn method(app: &PhotocraftApp, id: DialogId, overlay: EffectId) -> Option<String> {
    match param(&draft(app, id).style, overlay, "method") {
        Some(StyleValue::Text(t)) => Some(t),
        _ => None,
    }
}

#[test]
fn method_picker_is_pending_until_confirm_and_survives_reopening() {
    let mut app = app();
    app.run("layer.layerStyle.gradientOverlay", json!({"from": "#ff0000", "to": "#0000ff"})).unwrap();
    let original = effect(&app).clone();
    let (id, overlay) = open_gradient(&mut app);
    edit(&mut app, id, |d: &mut LayerStyleDraft| d.set_param(overlay, "method", StyleValue::Text("smooth".into())));
    assert_eq!(effect(&app), &original, "cancel can still discard all pending edits");
    assert_eq!(method(&app, id, overlay).as_deref(), Some("smooth"));
    ok(&mut app, id);
    let saved = effect(&app).clone();
    assert_ne!(saved, original, "the picked method rebuilds the curve");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(effect(&app), &original, "one undo step");
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(effect(&app), &saved);

    let (id, overlay) = open_gradient(&mut app);
    assert_eq!(method(&app, id, overlay).as_deref(), Some("smooth"));
    let Some(StyleValue::Gradient(stops)) = param(&draft(&app, id).style, overlay, "stops") else { panic!("stops load") };
    assert!(stops.color[0].color.same_rgb8(Srgba::rgb(1.0, 0.0, 0.0)));
    ok(&mut app, id);
    assert_eq!(effect(&app), &saved, "OK without edits leaves the rendered curve exact");
}

#[test]
fn baked_overlay_is_unchanged_until_method_selection_then_edits_the_whole_curve() {
    use photocraft_color::Color;
    use photocraft_io::{GradientMethod, bake_gradient};

    let mut app = app();
    app.run("layer.layerStyle.gradientOverlay", json!({})).unwrap();
    // PSD import and older method commands leave a baked curve without a source record.
    let state = app.session.active_mut().unwrap();
    let active_layer = state.active_layer.unwrap();
    let fx = &mut std::sync::Arc::make_mut(state.doc_mut()).layer_mut(active_layer).unwrap().effects.items[0];
    let Effect::GradientOverlay { gradient, interpolation, .. } = fx else { panic!("expected overlay") };
    gradient.stops = bake_gradient(vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], &[0.5], 1.0, GradientMethod::Smooth);
    *interpolation = None;
    let source_ends = vec![(0.0, gradient.stops.first().unwrap().1), (1.0, gradient.stops.last().unwrap().1)];

    let original = effect(&app).clone();
    let (id, overlay) = open_gradient(&mut app);
    assert_eq!(method(&app, id, overlay).as_deref(), Some("custom"));
    ok(&mut app, id);
    assert_eq!(effect(&app), &original, "OK without a method pick leaves the baked curve");

    let (id, overlay) = open_gradient(&mut app);
    edit(&mut app, id, |d| d.set_param(overlay, "method", StyleValue::Text("linear".into())));
    ok(&mut app, id);
    let selected = effect(&app).clone();
    let Effect::GradientOverlay { gradient, interpolation, .. } = &selected else { panic!("expected overlay") };
    assert_eq!(gradient.stops, bake_gradient(source_ends, &[0.5], 1.0, GradientMethod::Linear));
    assert_eq!(interpolation.as_ref().unwrap().stops.len(), 2);

    let (id, overlay) = open_gradient(&mut app);
    assert_eq!(method(&app, id, overlay).as_deref(), Some("linear"));
    let stops = GradientStops::new(vec![ColorStop::new(0.0, Srgba::rgb(1.0, 0.0, 0.0)), ColorStop::new(1.0, Srgba::rgb(0.0, 0.0, 1.0))]);
    edit(&mut app, id, |d| d.set_param(overlay, "stops", StyleValue::Gradient(stops)));
    ok(&mut app, id);
    let edited = effect(&app).clone();
    let Effect::GradientOverlay { gradient, .. } = &edited else { panic!("expected overlay") };
    assert_eq!(gradient.stops, bake_gradient(vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))], &[0.5], 1.0, GradientMethod::Linear));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(effect(&app), &selected);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(effect(&app), &edited);
    let (id, _) = open_gradient(&mut app);
    ok(&mut app, id);
    assert_eq!(effect(&app), &edited);
}

/// #13557: an overlay read from a PSD keeps Photoshop's gradient; the Layer Style page shows its
/// three stops and own opacity stops (not the baked ones) with its method and Smoothness.
#[test]
fn imported_overlay_shows_photoshops_stops() {
    use photocraft_color::Color;
    use photocraft_doc::effects::GradientInterpolation;
    let mut src = GradientInterpolation::new(
        "perceptual",
        100.0,
        vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (0.5, Color::rgb(0.0, 1.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))],
    );
    src.midpoints = vec![0.3, 0.5];
    src.opacity_stops = vec![(0.0, 1.0), (1.0, 0.25)];
    let gradient = photocraft_doc::Gradient {
        stops: photocraft_io::gradient_bake::bake_interpolation(&src),
        opacity_stops: photocraft_io::gradient_bake::bake_interpolation_opacity(&src).unwrap(),
        ..Default::default()
    };
    assert!(gradient.stops.len() > 90 && gradient.opacity_stops.len() > 2);
    let fx = Effect::GradientOverlay {
        common: photocraft_doc::FxCommon::new(photocraft_color::BlendMode::Normal, 1.0),
        gradient,
        dither: false,
        interpolation: Some(src),
    };
    let v = values_of(&fx, 120.0);
    assert_eq!(v[gradient_field::KEY].as_array().map(Vec::len), Some(3), "{v}");
    assert_eq!(v[gradient_field::OPACITY_KEY], json!([[0.0, 100.0], [1.0, 25.0]]));
    assert_eq!((v["method"].as_str(), v["smoothness"].as_f64()), (Some("perceptual"), Some(100.0)));
}
