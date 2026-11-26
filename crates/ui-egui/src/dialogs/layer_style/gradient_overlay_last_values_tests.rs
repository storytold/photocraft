//! #14899: a new Gradient Overlay opens with the values of the last one committed with the dialog's OK, on any
//! document and after a restart. Cancel, commands run outside the dialog and Make Default leave that memory alone;
//! Reset to Default still gives the factory (or Make Default) values.

use photocraft_engine::Session;
use photocraft_engine::prefs::SavedPrefsExt;
use serde_json::{Value, json};

use super::tests::{ColorStop, DialogId, EffectKind, GradientStops, Srgba, StyleValue, app_with_layer, cancel, defaults, draft, edit, first_of, ok};
use super::*;
use crate::PhotocraftApp;

const GRADIENT: EffectKind = EffectKind::GradientOverlay;

fn new_document(app: &mut PhotocraftApp) {
    app.run("file.new", json!({"width": 20, "height": 20})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
}

fn open_gradient(app: &mut PhotocraftApp) -> DialogId {
    open(app, Some(GRADIENT.key())).unwrap()
}

/// The page values of the dialog's first Gradient Overlay, in the command's units.
fn page(app: &PhotocraftApp, id: DialogId) -> Value {
    let fx = first_of(app, id, GRADIENT);
    style_params_json(&draft(app, id).style.effect(fx).unwrap().params)
}

fn red_to_blue() -> GradientStops {
    GradientStops::new(vec![ColorStop::new(0.0, Srgba::rgb(1.0, 0.0, 0.0)), ColorStop::new(1.0, Srgba::rgb(0.0, 0.0, 1.0))])
}

/// Opens Gradient Overlay, sets Opacity 40, Method Perceptual and a red to blue gradient, and returns the page values.
fn set_overlay(app: &mut PhotocraftApp, id: DialogId) -> Value {
    let fx = first_of(app, id, GRADIENT);
    edit(app, id, |d| {
        d.set_param(fx, "opacity", StyleValue::Number(40.0));
        d.set_param(fx, "method", StyleValue::Text("perceptual".into()));
        d.set_param(fx, "stops", StyleValue::Gradient(red_to_blue()));
    });
    page(app, id)
}

/// Commits Opacity 40, Perceptual, red to blue with OK; returns the page values it committed.
fn commit_overlay(app: &mut PhotocraftApp) -> Value {
    let id = open_gradient(app);
    let expected = set_overlay(app, id);
    ok(app, id);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    expected
}

/// What a new Gradient Overlay opens with before any OK: the factory (or Make Default) values.
fn untouched_page(app: &mut PhotocraftApp) -> Value {
    let id = open_gradient(app);
    let values = page(app, id);
    cancel(app, id);
    values
}

fn assert_red_to_blue(values: &Value) {
    assert_eq!(values["opacity"], json!(40.0));
    assert_eq!(values["method"], json!("perceptual"));
    assert_eq!(values["stops"], json!([[0.0, "#ff0000"], [1.0, "#0000ff"]]));
}

#[test]
fn ok_values_open_the_next_gradient_overlay_on_a_new_document() {
    let mut app = app_with_layer(20);
    let expected = commit_overlay(&mut app);
    assert_red_to_blue(&expected);

    new_document(&mut app);
    let next = open_gradient(&mut app);
    let shown = page(&app, next);
    assert_red_to_blue(&shown);
    assert_eq!(shown, expected);
}

#[test]
fn ok_values_survive_a_restart() {
    let mut app = app_with_layer(20);
    let expected = commit_overlay(&mut app);
    app.framework.move_memories(&mut app.session);

    let mut session = Session::new();
    session.load_prefs_json(&app.session.prefs_to_json()).unwrap();
    drop(app);
    let mut reopened = PhotocraftApp::new(session, crate::Services::default());
    new_document(&mut reopened);
    let next = open_gradient(&mut reopened);
    assert_eq!(page(&reopened, next), expected);
}

#[test]
fn cancel_and_script_commands_leave_the_last_ok_alone() {
    let mut app = app_with_layer(20);
    let expected = commit_overlay(&mut app);

    new_document(&mut app);
    let id = open_gradient(&mut app);
    let fx = first_of(&app, id, GRADIENT);
    edit(&mut app, id, |d| d.set_param(fx, "opacity", StyleValue::Number(80.0)));
    cancel(&mut app, id);

    // A script's overlay does not count, nor does an OK that leaves an overlay untouched.
    app.run("layer.layerStyle.gradientOverlay", json!({"opacity": 66, "method": "smooth"})).unwrap();
    let unchanged = open(&mut app, Some(BLENDING)).unwrap();
    ok(&mut app, unchanged);

    new_document(&mut app);
    let next = open_gradient(&mut app);
    assert_eq!(page(&app, next), expected);
}

#[test]
fn a_disabled_overlay_does_not_replace_the_last_ok() {
    let mut app = app_with_layer(20);
    let expected = commit_overlay(&mut app);

    new_document(&mut app);
    let id = open_gradient(&mut app);
    let fx = first_of(&app, id, GRADIENT);
    edit(&mut app, id, |d| {
        d.set_param(fx, "opacity", StyleValue::Number(80.0));
        d.toggle(fx);
    });
    ok(&mut app, id);

    new_document(&mut app);
    let next = open_gradient(&mut app);
    assert_eq!(page(&app, next), expected);
}

#[test]
fn the_last_edited_overlay_of_the_ok_is_remembered() {
    let mut app = app_with_layer(20);
    commit_overlay(&mut app);
    // The layer now has the committed overlay; edit it again and OK.
    let id = open_gradient(&mut app);
    let fx = first_of(&app, id, GRADIENT);
    edit(&mut app, id, |d| d.set_param(fx, "opacity", StyleValue::Number(25.0)));
    ok(&mut app, id);

    new_document(&mut app);
    let next = open_gradient(&mut app);
    let shown = page(&app, next);
    assert_eq!(shown["opacity"], json!(25.0));
    assert_eq!(shown["method"], json!("perceptual"));
}

/// Make Default stores a default and survives Cancel, but a new overlay opens with the last OK; Reset to Default
/// then gives the Make Default value with every other value at its factory default.
#[test]
fn make_default_and_reset_keep_their_own_values() {
    let mut app = app_with_layer(20);
    let factory = untouched_page(&mut app);
    commit_overlay(&mut app);
    // A second OK at Opacity 100 and the factory method: what the next overlay opens with.
    new_document(&mut app);
    let id = open_gradient(&mut app);
    let fx = first_of(&app, id, GRADIENT);
    edit(&mut app, id, |d| d.set_param(fx, "opacity", StyleValue::Number(100.0)));
    let last_ok = page(&app, id);
    ok(&mut app, id);

    new_document(&mut app);
    let id = open_gradient(&mut app);
    app.run("layer.layerStyle.makeDefault", json!({"kind": GRADIENT.key(), "params": {"opacity": 40}})).unwrap();
    cancel(&mut app, id);

    new_document(&mut app);
    let id = open_gradient(&mut app);
    assert_eq!(page(&app, id), last_ok);
    let fx = first_of(&app, id, GRADIENT);
    reset_to_default(&mut app, id, fx);
    let reset = page(&app, id);
    assert_eq!(reset["opacity"], json!(40.0));
    assert_eq!(reset["method"], factory["method"], "the rest is the factory value");
    assert_eq!(reset["stops"], factory["stops"]);
}

#[test]
fn reset_to_default_gives_the_factory_values_after_an_ok() {
    let mut app = app_with_layer(20);
    let factory = untouched_page(&mut app);
    commit_overlay(&mut app);
    new_document(&mut app);
    let id = open_gradient(&mut app);
    let fx = first_of(&app, id, GRADIENT);
    reset_to_default(&mut app, id, fx);
    let reset = page(&app, id);
    for key in ["opacity", "method", "stops", "blend", "scale", "angle"] {
        assert_eq!(reset[key], factory[key], "{key}");
    }
}

/// The memory is per effect kind: committing a Gradient Overlay does not change what another kind opens with.
#[test]
fn other_kinds_keep_their_defaults() {
    let mut app = app_with_layer(20);
    commit_overlay(&mut app);
    new_document(&mut app);
    let id = open(&mut app, Some(EffectKind::ColorOverlay.key())).unwrap();
    let fx = first_of(&app, id, EffectKind::ColorOverlay);
    let shown = style_params_json(&draft(&app, id).style.effect(fx).unwrap().params);
    assert_eq!(shown["opacity"], style_params_json(&defaults(EffectKind::ColorOverlay))["opacity"]);
}

/// The layer's effects, as the document holds them.
fn layer_effects(app: &PhotocraftApp) -> Vec<photocraft_doc::Effect> {
    let state = app.session.active().unwrap();
    state.doc.layer(state.active_layer.unwrap()).unwrap().effects.items.clone()
}

/// A loaded overlay can carry more colour stops than a command takes (a file has no limit). After an
/// opacity-only OK on it, the next new overlay must still apply (#16354 review).
#[test]
fn a_recalled_gradient_with_more_stops_than_a_command_takes_still_applies() {
    use photocraft_color::Color;
    use photocraft_engine::layer_style::MAX_STOPS;

    let mut app = app_with_layer(20);
    app.run("layer.layerStyle.gradientOverlay", json!({})).unwrap();
    let state = app.session.active_mut().unwrap();
    let active_layer = state.active_layer.unwrap();
    let loaded = &mut std::sync::Arc::make_mut(state.doc_mut()).layer_mut(active_layer).unwrap().effects.items[0];
    let photocraft_doc::Effect::GradientOverlay { gradient, interpolation, .. } = loaded else { panic!("expected overlay") };
    let n = MAX_STOPS + 1;
    gradient.stops = (0..n).map(|i| (i as f32 / (n - 1) as f32, Color::rgb(i as f32 / (n - 1) as f32, 0.0, 0.0))).collect();
    *interpolation = None;

    let id = open_gradient(&mut app);
    let fx = first_of(&app, id, GRADIENT);
    edit(&mut app, id, |d| d.set_param(fx, "opacity", StyleValue::Number(60.0)));
    ok(&mut app, id);
    assert!(!app.ui.status_error, "the loaded overlay applies: {}", app.ui.status);

    new_document(&mut app);
    let id = open_gradient(&mut app);
    assert_eq!(page(&app, id)["opacity"], json!(60.0), "the rest of the last OK is recalled");
    ok(&mut app, id);
    assert!(!app.ui.status_error, "the new overlay applies: {}", app.ui.status);
    assert_eq!(layer_effects(&app).len(), 1, "the new overlay is on the layer");
}

/// Reset to Default must not keep a remembered key that neither Make Default nor the factory values
/// give: a recalled Smoothness 0 goes back to the engine's default (100). An explicit Make Default
/// Smoothness still wins (#16354 review).
#[test]
fn reset_to_default_drops_a_remembered_smoothness_no_default_gives() {
    let mut app = app_with_layer(20);
    let factory = untouched_page(&mut app);
    assert!(factory.get("smoothness").is_none(), "the factory page has no smoothness: {factory}");
    app.run("layer.layerStyle.gradientOverlay", json!({"from": "#ff0000", "to": "#0000ff", "method": "classic", "smoothness": 0})).unwrap();
    let id = open_gradient(&mut app);
    let fx = first_of(&app, id, GRADIENT);
    edit(&mut app, id, |d| d.set_param(fx, "opacity", StyleValue::Number(60.0)));
    ok(&mut app, id);
    assert!(!app.ui.status_error, "{}", app.ui.status);

    new_document(&mut app);
    let id = open_gradient(&mut app);
    assert_eq!(page(&app, id)["smoothness"].as_f64(), Some(0.0), "the last OK's smoothness is recalled");
    let fx = first_of(&app, id, GRADIENT);
    reset_to_default(&mut app, id, fx);
    let reset = page(&app, id);
    assert_eq!(reset["smoothness"].as_f64(), Some(100.0), "reset gives the engine's default smoothness: {reset}");
    assert_eq!(reset["method"], factory["method"]);

    app.run("layer.layerStyle.makeDefault", json!({"kind": GRADIENT.key(), "params": {"smoothness": 50}})).unwrap();
    reset_to_default(&mut app, id, fx);
    assert_eq!(page(&app, id)["smoothness"].as_f64(), Some(50.0), "Make Default's smoothness wins");
}

/// Replaces the colour and transparency stops of the active layer's first Gradient Overlay, as a
/// loaded file may hold them (the document type has no minimum or maximum).
fn load_stops(app: &mut PhotocraftApp, stops: Vec<(f32, photocraft_color::Color)>, opacity: Vec<(f32, f32)>) {
    let state = app.session.active_mut().unwrap();
    let active_layer = state.active_layer.unwrap();
    let loaded = &mut std::sync::Arc::make_mut(state.doc_mut()).layer_mut(active_layer).unwrap().effects.items[0];
    let photocraft_doc::Effect::GradientOverlay { gradient, interpolation, .. } = loaded else { panic!("expected overlay") };
    gradient.stops = stops;
    gradient.opacity_stops = opacity;
    *interpolation = None;
}

/// Opacity 60 with OK on the loaded overlay, then a new overlay on a new document: it opens with
/// Opacity 60 and must apply.
fn ok_then_new_overlay_applies(app: &mut PhotocraftApp) -> Value {
    let id = open_gradient(app);
    let fx = first_of(app, id, GRADIENT);
    edit(app, id, |d| d.set_param(fx, "opacity", StyleValue::Number(60.0)));
    ok(app, id);
    assert!(!app.ui.status_error, "the loaded overlay applies: {}", app.ui.status);

    new_document(app);
    let id = open_gradient(app);
    let shown = page(app, id);
    assert_eq!(shown["opacity"], json!(60.0), "the rest of the last OK is recalled: {shown}");
    ok(app, id);
    assert!(!app.ui.status_error, "the new overlay applies: {}", app.ui.status);
    assert_eq!(layer_effects(app).len(), 1, "the new overlay is on the layer");
    shown
}

/// A stop list the remembered values leave out keeps its Make Default value: the last OK is laid
/// over the Make Default entry, not put in its place (#16354 review).
#[test]
fn a_gradient_the_last_ok_leaves_out_keeps_its_make_default() {
    use photocraft_color::Color;

    let mut app = app_with_layer(20);
    let made = json!({"stops": [[0.0, "#00ff00"], [1.0, "#ffff00"]], "opacityStops": [], "scale": 50});
    app.run("layer.layerStyle.makeDefault", json!({"kind": GRADIENT.key(), "params": made})).unwrap();
    app.run("layer.layerStyle.gradientOverlay", json!({})).unwrap();
    // One colour stop, which a command refuses, so the last OK leaves the gradient out.
    load_stops(&mut app, vec![(0.5, Color::rgb(1.0, 0.0, 0.0))], vec![]);

    let shown = ok_then_new_overlay_applies(&mut app);
    assert_eq!(shown["stops"], made["stops"], "Make Default's gradient: {shown}");
}

/// A loaded overlay can hold one colour stop, which a command refuses (#16354 review). After an OK
/// on it the next new overlay must still apply, with the default gradient. One transparency stop is
/// harmless: the page reads fewer than two as none, so that gradient is remembered without it.
#[test]
fn a_recalled_gradient_a_command_refuses_is_not_remembered() {
    use photocraft_color::Color;

    let mut app = app_with_layer(20);
    let factory = untouched_page(&mut app);
    app.run("layer.layerStyle.gradientOverlay", json!({})).unwrap();
    load_stops(&mut app, vec![(0.5, Color::rgb(1.0, 0.0, 0.0))], vec![]);
    let shown = ok_then_new_overlay_applies(&mut app);
    assert_eq!(shown["stops"], factory["stops"], "the default gradient: {shown}");

    let mut app = app_with_layer(20);
    app.run("layer.layerStyle.gradientOverlay", json!({})).unwrap();
    load_stops(&mut app, vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))], vec![(0.5, 0.5)]);
    let shown = ok_then_new_overlay_applies(&mut app);
    assert_eq!(shown["stops"], json!([[0.0, "#ff0000"], [1.0, "#0000ff"]]), "the last OK's gradient: {shown}");
    assert_eq!(shown["opacityStops"], json!([]));
}

/// Reset to Default on a loaded overlay with a recorded interpolation must give the engine's default
/// Smoothness, not keep the recorded one (#16354 review).
#[test]
fn reset_to_default_replaces_a_loaded_overlays_recorded_smoothness() {
    let mut app = app_with_layer(20);
    app.run("layer.layerStyle.gradientOverlay", json!({"from": "#ff0000", "to": "#0000ff", "method": "classic", "smoothness": 0})).unwrap();
    app.run("layer.layerStyle.makeDefault", json!({"kind": GRADIENT.key(), "params": {"method": "perceptual"}})).unwrap();
    let id = open_gradient(&mut app);
    let fx = first_of(&app, id, GRADIENT);
    reset_to_default(&mut app, id, fx);
    ok(&mut app, id);
    assert!(!app.ui.status_error, "{}", app.ui.status);

    let effects = layer_effects(&app);
    let photocraft_doc::Effect::GradientOverlay { interpolation: Some(source), .. } = &effects[0] else {
        panic!("expected an interpolated overlay: {effects:?}")
    };
    assert_eq!(source.method, "perceptual");
    assert_eq!(source.smoothness, 100.0, "the engine's default smoothness");
}

/// The opacity of a Gradient Overlay effect, 0..=100.
fn overlay_opacity(e: &photocraft_doc::Effect) -> f32 {
    let photocraft_doc::Effect::GradientOverlay { common, .. } = e else { panic!("expected overlay: {e:?}") };
    (common.opacity * 100.0).round()
}

/// A loaded overlay with more colour or transparency stops than a command takes, changed only in
/// Opacity with OK: the next new overlay opens with all its stops and applies them (#16354 review).
#[test]
fn a_recalled_gradient_with_more_stops_than_a_command_takes_keeps_them() {
    use photocraft_color::Color;
    use photocraft_engine::layer_style::MAX_STOPS;

    let n = MAX_STOPS + 1;
    let at = |i: usize| i as f32 / (n - 1) as f32;
    let many_colours: Vec<(f32, Color)> = (0..n).map(|i| (at(i), Color::rgb(at(i), 0.0, 1.0 - at(i)))).collect();
    let two_colours = vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))];
    let many_opacities: Vec<(f32, f32)> = (0..n).map(|i| (at(i), if i % 2 == 0 { 1.0 } else { 0.5 })).collect();
    for (stops, opacity) in [(many_colours.clone(), vec![]), (two_colours, many_opacities.clone()), (many_colours, many_opacities)] {
        let (colours, opacities) = (stops.len(), opacity.len());
        let mut app = app_with_layer(20);
        app.run("layer.layerStyle.gradientOverlay", json!({})).unwrap();
        load_stops(&mut app, stops.clone(), opacity.clone());

        let shown = ok_then_new_overlay_applies(&mut app);
        assert_eq!(shown["stops"].as_array().map(Vec::len), Some(colours), "the new overlay opens with every colour stop");
        assert_eq!(shown["opacityStops"].as_array().map(Vec::len), Some(opacities), "and every transparency stop");
        let effects = layer_effects(&app);
        let photocraft_doc::Effect::GradientOverlay { gradient, .. } = &effects[0] else { panic!("expected overlay: {effects:?}") };
        assert_eq!(gradient.stops, stops, "the new overlay has the recalled colour stops");
        assert_eq!(gradient.opacity_stops, opacity, "and the recalled transparency stops");
        assert_eq!(overlay_opacity(&effects[0]), 60.0, "and the last OK's Opacity");
    }
}

/// A remembered snapshot that does not read as an effect (an edited settings file) is dropped: the
/// new overlay opens without it and applies with the remembered values.
#[test]
fn a_remembered_snapshot_that_does_not_read_is_dropped() {
    for fx in [json!("not json"), json!("{\"kind\": \"nothing\"}"), json!(7)] {
        let mut app = app_with_layer(20);
        let values = json!({"opacity": 60, "stops": [[0.0, "#ff0000"], [1.0, "#0000ff"]], "opacityStops": [], "fx": fx});
        last_ok(GRADIENT).store(app.framework.host(), &values);
        let id = open_gradient(&mut app);
        let instance = first_of(&app, id, GRADIENT);
        assert!(draft(&app, id).style.effect(instance).unwrap().params.get("fx").is_none(), "{fx}");
        assert_eq!(page(&app, id)["opacity"], json!(60.0), "{fx}");
        ok(&mut app, id);
        assert!(!app.ui.status_error, "{fx}: {}", app.ui.status);
        assert_eq!(overlay_opacity(&layer_effects(&app)[0]), 60.0, "{fx}");
    }
}
