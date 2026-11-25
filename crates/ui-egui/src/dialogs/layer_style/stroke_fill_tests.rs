use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use photocraft_color::Color;
use photocraft_doc::effects::{Gradient, GradientStyle, StrokeFx};

use super::tests::{app_with_layer, cancel, draft, edit, first_of, harness, layer_effects, ok, param};
use super::*;

fn stroke(fx: &Effect) -> &StrokeFx {
    let Effect::Stroke(stroke) = fx else { panic!("expected a Stroke effect") };
    stroke
}

fn gradient(paint: &FxPaint) -> &Gradient {
    let FxPaint::Gradient(gradient) = paint else { panic!("expected a gradient fill") };
    gradient
}

#[test]
fn saved_stroke_endpoints_survive_open_and_reset_to_default() {
    let gradient = json!({
        "fillType": "gradient", "from": "#ff0000", "to": "#0000ff", "opacityStops": [[0, 90], [1, 55]],
        "style": "reflected", "angle": 43, "scale": 72.5, "reverse": true,
        "size": 9, "position": "outside", "opacity": 84, "overprint": true
    });
    let mut legacy = gradient.clone();
    legacy.as_object_mut().unwrap().remove("fillType");
    let mut to_only = gradient.clone();
    to_only.as_object_mut().unwrap().remove("from");
    let mut explicit_stops = to_only.clone();
    explicit_stops.as_object_mut().unwrap().remove("to");
    explicit_stops["stops"] = json!([[0, "#ff0000"], [0.35, "#00ff00"], [1, "#0000ff"]]);
    let solid = json!({"color": "#22cc55", "size": 3, "position": "center"});
    let mut solid_with_inactive_gradient = solid.clone();
    solid_with_inactive_gradient["fillType"] = json!("color");
    solid_with_inactive_gradient["from"] = gradient["from"].clone();
    solid_with_inactive_gradient["to"] = gradient["to"].clone();
    for (saved, command) in
        [(&gradient, &gradient), (&legacy, &legacy), (&to_only, &to_only), (&explicit_stops, &explicit_stops), (&solid_with_inactive_gradient, &solid)]
    {
        let mut app = app_with_layer(32);
        let expected = photocraft_engine::layer_style::effect_from_params("stroke", command).unwrap();
        app.run("layer.layerStyle.makeDefault", json!({"kind": "stroke", "params": saved})).unwrap();
        let id = open(&mut app, Some("stroke")).unwrap();
        ok(&mut app, id);
        assert_eq!(layer_effects(&app.session), vec![expected.clone()], "new Stroke from {saved}");

        app.run("layer.layerStyle.stroke", json!({"color": "#abcdef"})).unwrap();
        let id = open(&mut app, Some("stroke")).unwrap();
        let mut h = harness(app);
        h.get_by_role_and_label(Role::Button, "Reset to Default").click();
        h.run_steps(3);
        ok(h.state_mut(), id);
        assert_eq!(layer_effects(&h.state().session), vec![expected], "reset Stroke from {saved}");
    }
}

#[test]
fn stroke_fill_choice_previews_the_gradient_and_commits_a_single_edit() {
    let mut app = app_with_layer(32);
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("layer.layerStyle.stroke", json!({"color": "#00ff00", "size": 4})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let original = layer_effects(&app.session);
    let id = open(&mut app, Some("stroke")).unwrap();
    let instance = first_of(&app, id, EffectKind::Stroke);
    let mut h = harness(app);
    assert!(h.query_by_role_and_label(Role::ComboBox, "Gradient").is_none());
    h.get_by_role_and_label(Role::ComboBox, "Fill Type").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::ListBoxOption, "Gradient").click();
    h.run_steps(4);
    assert!(h.query_by_role_and_label(Role::ComboBox, "Gradient").is_some());
    assert_eq!(param(&draft(h.state(), id).style, instance, "fillType"), Some(StyleValue::Text("gradient".into())));

    let (preview, _) = preview_document(h.state()).unwrap();
    let shown = stroke(&preview.layer(layer).unwrap().effects.items[0]);
    let g = gradient(&shown.paint);
    assert!(g.stops[0].1.to_rgb()[0] < g.stops[1].1.to_rgb()[0]);
    let original_pixel = photocraft_compose::flatten(&h.state().session.active().unwrap().doc).get(1, 1);
    let preview_pixel = photocraft_compose::flatten(&preview).get(1, 1);
    assert_ne!(preview_pixel, original_pixel, "the outline uses gradient pixels in the canvas preview");
    assert_eq!(layer_effects(&h.state().session), original);

    edit(h.state_mut(), id, |d| d.set_param(instance, "style", StyleValue::Text("diamond".into())));
    let picker = color_picker::open(h.state_mut(), id.value(), &instance.key(), "stops", Some(0)).unwrap();
    crate::color_picker_ui::testing::set_color(h.state_mut(), picker, "#0000ff");
    let (preview, _) = preview_document(h.state()).unwrap();
    assert_eq!(gradient(&stroke(&preview.layer(layer).unwrap().effects.items[0]).paint).stops[0].1, Color::rgb(0.0, 0.0, 1.0));
    ok(h.state_mut(), picker);
    h.run_steps(3);

    h.get_by_role_and_label(Role::ComboBox, "Fill Type").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::ListBoxOption, "Color").click();
    h.run_steps(4);
    let (preview, _) = preview_document(h.state()).unwrap();
    assert_eq!(stroke(&preview.layer(layer).unwrap().effects.items[0]).paint, stroke(&original[0]).paint);
    assert!(h.query_by_role_and_label(Role::ComboBox, "Gradient").is_none());
    h.get_by_role_and_label(Role::ComboBox, "Fill Type").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::ListBoxOption, "Gradient").click();
    h.run_steps(4);

    ok(h.state_mut(), id);
    let applied = layer_effects(&h.state().session);
    let g = gradient(&stroke(&applied[0]).paint);
    assert_eq!(g.style, GradientStyle::Diamond);
    assert_eq!(g.stops[0].1, Color::rgb(0.0, 0.0, 1.0));
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(layer_effects(&h.state().session), original);
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(layer_effects(&h.state().session), applied);
}

#[test]
fn stroke_gradient_snapshot_survives_cancel_and_preserves_unedited_values_on_save() {
    let mut app = app_with_layer(32);
    app.run("layer.layerStyle.stroke", json!({"from": "#fc0024", "to": "#04fe2a"})).unwrap();
    let state = app.session.active_mut().unwrap();
    let layer = state.active_layer.unwrap();
    let effect = &mut Arc::make_mut(state.doc_mut()).layer_mut(layer).unwrap().effects.items[0];
    let Effect::Stroke(s) = effect else { panic!("expected a Stroke effect") };
    let FxPaint::Gradient(g) = &mut s.paint else { panic!("expected a gradient fill") };
    // The stroke can carry an imported baked gradient too large to be a new stops parameter.
    g.stops = (0..300)
        .map(|i| {
            let t = i as f32 / 299.0;
            (t, Color::rgb(t, 0.612_345, 1.0 - t))
        })
        .collect();
    g.opacity_stops = vec![(0.0, 0.263_457), (1.0, 1.0)];
    g.scale = 1.234_567;
    g.offset = (0.32, -0.18);
    g.align = false;
    let original = layer_effects(&app.session);
    let mut expected = original.clone();
    let Effect::Stroke(s) = &mut expected[0] else { panic!("expected a Stroke effect") };
    let FxPaint::Gradient(g) = &mut s.paint else { panic!("expected a gradient fill") };
    g.reverse = true;

    let id = open(&mut app, Some("stroke")).unwrap();
    let instance = first_of(&app, id, EffectKind::Stroke);
    edit(&mut app, id, |d| d.set_param(instance, "reverse", StyleValue::Bool(true)));
    cancel(&mut app, id);
    assert_eq!(layer_effects(&app.session), original);

    let id = open(&mut app, Some("stroke")).unwrap();
    let instance = first_of(&app, id, EffectKind::Stroke);
    edit(&mut app, id, |d| d.set_param(instance, "reverse", StyleValue::Bool(true)));
    let default = json!({"size": 8});
    app.run("layer.layerStyle.makeDefault", json!({"kind": "stroke", "params": default})).unwrap();
    let oversized = style_params_json(&draft(&app, id).style.effect(instance).unwrap().params);
    assert!(app.run("layer.layerStyle.makeDefault", json!({"kind": "stroke", "params": oversized})).is_err());
    let stored = app.run("layer.layerStyle.defaultFor", json!({"kind": "stroke"})).unwrap();
    assert_eq!(stored["params"], default, "an oversized imported gradient cannot replace the saved default");
    save_new_style(&mut app, id, layer);
    let preset = photocraft_engine::presets::styles::find_style(&app.session, &json!({"preset": "Style"}), "test").unwrap();
    assert_eq!(preset.effects, expected);
    assert_eq!(layer_effects(&app.session), original, "saving a preset leaves the document untouched");
    ok(&mut app, id);
    assert_eq!(layer_effects(&app.session), expected);
}

#[test]
fn an_imported_pattern_survives_switching_back_to_its_fill_type() {
    let mut app = app_with_layer(32);
    app.run("layer.layerStyle.stroke", json!({})).unwrap();
    let state = app.session.active_mut().unwrap();
    let layer = state.active_layer.unwrap();
    let effect = &mut Arc::make_mut(state.doc_mut()).layer_mut(layer).unwrap().effects.items[0];
    let Effect::Stroke(s) = effect else { panic!("expected a Stroke effect") };
    s.paint = FxPaint::Pattern { name: "Imported".into(), id: "imported-pattern".into(), scale: 0.75 };
    let original = layer_effects(&app.session);
    let id = open(&mut app, Some("stroke")).unwrap();
    let mut h = harness(app);
    h.get_by_role_and_label(Role::ComboBox, "Fill Type").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::ListBoxOption, "Gradient").click();
    h.run_steps(4);
    let (preview, _) = preview_document(h.state()).unwrap();
    assert!(matches!(stroke(&preview.layer(layer).unwrap().effects.items[0]).paint, FxPaint::Gradient(_)));
    h.get_by_role_and_label(Role::ComboBox, "Fill Type").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::ListBoxOption, "Pattern").click();
    h.run_steps(4);
    ok(h.state_mut(), id);
    assert_eq!(layer_effects(&h.state().session), original);
}
