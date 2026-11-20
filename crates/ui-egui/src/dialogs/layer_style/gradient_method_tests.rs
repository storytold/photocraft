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
    let fx = &mut std::sync::Arc::make_mut(&mut state.doc).layer_mut(state.active_layer.unwrap()).unwrap().effects.items[0];
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
