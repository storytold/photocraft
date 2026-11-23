use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_doc::effects::{BevelTechnique, FxPaint, GlowSource};
use photocraft_doc::{BlendMode, BlendRange, Effect, Knockout, StrokeFx};
use photocraft_engine::presets::PresetsExt;
use serde_json::{Value, json};

use super::*;
use crate::PhotocraftApp;

// PLAN: assumed facade paths for the framework types; the other layer-style test files import them from here.
pub(super) use craft::color_ui::gradient::{ColorStop, GradientStops};
pub(super) use craft::imaging::layer_style::{
    EffectId, EffectKind, EffectSpec, FieldKind, LayerStyle, LayerStyleDraft, LayerStylePage, StyleParams, StyleValue,
};
pub(super) use craft::imaging::ui::layer_style::HostedLayerStyle;
pub(super) use craft::theme::Srgba;
pub(super) use craft::ui::dialog::DialogId;

pub(super) const KIND_KEYS: [&str; 10] =
    ["bevelEmboss", "stroke", "innerShadow", "innerGlow", "satin", "colorOverlay", "gradientOverlay", "patternOverlay", "outerGlow", "dropShadow"];

/// The effect kind a command name uses.
pub(super) fn kind(key: &str) -> EffectKind {
    EffectKind::from_key(key).expect("a known effect key")
}

/// The factory parameters of a kind, which the draft's `add_instance` starts from.
pub(super) fn defaults(kind: EffectKind) -> StyleParams {
    EffectSpec::photoshop(kind).defaults
}

/// An app with a `size` square document and one empty layer.
pub(super) fn app_with_layer(size: u32) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": size, "height": size})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app
}

/// The draft the open Layer Style dialog `id` holds.
pub(super) fn draft(app: &PhotocraftApp, id: DialogId) -> &LayerStyleDraft {
    app.framework.dialogs.get_by_id::<HostedLayerStyle>(id).expect("the Layer Style dialog is open").draft()
}

/// Edits the draft of the open Layer Style dialog `id`.
pub(super) fn edit(app: &mut PhotocraftApp, id: DialogId, change: impl FnOnce(&mut LayerStyleDraft)) {
    change(app.framework.dialogs.get_mut_by_id::<HostedLayerStyle>(id).expect("the Layer Style dialog is open").draft_mut());
}

/// OK on dialog `id`, as the button does.
pub(super) fn ok(app: &mut PhotocraftApp, id: DialogId) {
    let _ = crate::dialogs::testing::confirm(app, id);
}

/// Cancel on dialog `id`.
pub(super) fn cancel(app: &mut PhotocraftApp, id: DialogId) {
    let _ = crate::dialogs::testing::cancel(app, id);
}

/// The first instance of `kind` in the dialog's draft.
pub(super) fn first_of(app: &PhotocraftApp, id: DialogId, kind: EffectKind) -> EffectId {
    draft(app, id).style.instances(kind).next().expect("the draft has the kind")
}

/// A parameter of one instance.
pub(super) fn param(style: &LayerStyle, id: EffectId, key: &str) -> Option<StyleValue> {
    style.effect(id).and_then(|e| e.params.get(key).cloned())
}

fn num(style: &LayerStyle, id: EffectId, key: &str) -> Option<f64> {
    style.effect(id).and_then(|e| e.params.number(key))
}

fn text(style: &LayerStyle, id: EffectId, key: &str) -> Option<String> {
    match param(style, id, key) {
        Some(StyleValue::Text(t)) => Some(t),
        _ => None,
    }
}

/// The effects of the active layer.
pub(super) fn layer_effects(s: &photocraft_engine::Session) -> Vec<Effect> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().effects.items.clone()
}

fn session() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

fn strokes_of(s: &photocraft_engine::Session) -> Vec<StrokeFx> {
    layer_effects(s)
        .into_iter()
        .filter_map(|e| match e {
            Effect::Stroke(st) => Some(st),
            _ => None,
        })
        .collect()
}

/// The style the dialog shows for the active layer.
fn drafted(s: &photocraft_engine::Session) -> LayerStyle {
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    draft_of(st.doc.layer(id).unwrap(), st.doc.global_light.angle, st.doc.mode)
}

/// Opens the active layer's style as the dialog does and confirms it unchanged.
fn reapply(s: &mut photocraft_engine::Session) {
    let style = drafted(s);
    let id = s.active().unwrap().active_layer.unwrap();
    apply(&style, id, None, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
}

/// One round-trip row: runs `setup`, reads the draft, confirms it unchanged and checks the engine kept every effect.
fn row(setup: &[(&str, Value)]) -> (photocraft_engine::Session, LayerStyle) {
    let mut s = session();
    for (cmd, p) in setup {
        s.execute(cmd, p.clone()).unwrap();
    }
    let before = layer_effects(&s);
    let style = drafted(&s);
    reapply(&mut s);
    assert_eq!(layer_effects(&s), before, "OK without edits changes nothing: {setup:?}");
    (s, style)
}

pub(super) fn harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(1500.0, 1000.0)).with_context(app.ctx().clone()).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            app.framework.dialogs.run(ui);
            take(app, ui.ctx());
        },
        app,
    );
    h.run_steps(4);
    h
}

/// With the dialog's layer gone, a drag on the image has nothing to move, so the Hand tool pans as usual:
/// `gradient_layer_bounds` finds the dialog's layer, or nothing.
#[test]
fn the_gradient_drag_needs_its_layer() {
    let mut s = session();
    let empty = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    assert_eq!(gradient_layer_bounds(&s, id), Some([64.0, 64.0]), "the dialog's layer");
    assert_eq!(gradient_layer_bounds(&s, empty), None, "an empty layer paints no gradient");
    assert_eq!(gradient_layer_bounds(&s, photocraft_doc::LayerId(id.0 + 1000)), None, "a layer that is gone");
}

/// Double-clicking an effect row of a layer that isn't active selects that layer first, so the dialog holds its
/// effects (and OK edits it), not the active layer's.
#[test]
fn effect_row_of_another_layer_opens_that_layers_style() {
    let mut app = app_with_layer(32);
    let with_fx = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.layerStyle.dropShadow", json!({"size": 5})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let plain = app.session.active().unwrap().active_layer.unwrap();
    assert_ne!(plain, with_fx);

    let id = open_for_layer(&mut app, with_fx, Some(EffectKind::DropShadow.key())).unwrap();
    assert_eq!(app.session.active().unwrap().active_layer, Some(with_fx), "the row's layer becomes active");
    let shadow = {
        let effects = &draft(&app, id).style.effects;
        assert_eq!(effects.len(), 1, "the existing drop shadow, not a new instance");
        assert_eq!(effects[0].kind, EffectKind::DropShadow);
        assert_eq!(effects[0].params.number("size"), Some(5.0));
        effects[0].id
    };
    edit(&mut app, id, |d| d.set_param(shadow, "size", StyleValue::Number(9.0)));
    ok(&mut app, id);
    let doc = &app.session.active().unwrap().doc;
    let Effect::DropShadow(sh) = &doc.layer(with_fx).unwrap().effects.items[0] else { panic!("a drop shadow") };
    assert_eq!(sh.size, 9.0, "OK edits the row's layer");
    assert!(doc.layer(plain).unwrap().effects.items.is_empty());
}

/// Make Default (#9477): a stroke the dialog creates afterwards starts from the saved values; other kinds keep the
/// factory ones.
#[test]
fn new_effects_start_from_the_made_default() {
    let mut app = app_with_layer(32);
    app.run("layer.layerStyle.makeDefault", json!({"kind": "stroke", "params": {"size": 7, "position": "outside"}})).unwrap();

    let id = open(&mut app, Some(EffectKind::Stroke.key())).unwrap();
    let stroke = first_of(&app, id, EffectKind::Stroke);
    let style = &draft(&app, id).style;
    assert_eq!(num(style, stroke, "size"), Some(7.0));
    assert_eq!(text(style, stroke, "position").as_deref(), Some("outside"));
    assert_eq!(num(style, stroke, "opacity"), defaults(EffectKind::Stroke).number("opacity"), "unsaved keys keep the factory value");
    cancel(&mut app, id);

    let id = open(&mut app, Some(EffectKind::DropShadow.key())).unwrap();
    let shadow = first_of(&app, id, EffectKind::DropShadow);
    assert_eq!(num(&draft(&app, id).style, shadow, "size"), defaults(EffectKind::DropShadow).number("size"));
}

/// The app's conversion between engine effects and the draft is the format boundary where params were lost before
/// (#6698, the Inner Glow source): each row loads an engine style into the draft and confirms it unchanged.
#[test]
fn open_and_confirm_preserves_two_strokes() {
    // The manga/typesetting case: a 3 px black stroke under a 7 px white one; Overprint on the first.
    let (s, style) = row(&[
        ("layer.layerStyle.stroke", json!({"size": 3, "color": "#000000", "overprint": true})),
        ("layer.layerStyle.stroke", json!({"size": 7, "color": "#ffffff", "add": true})),
    ]);
    assert_eq!(style.instances(EffectKind::Stroke).count(), 2, "both instances load");
    let strokes = strokes_of(&s);
    assert_eq!((strokes[0].size, strokes[1].size), (3.0, 7.0));
    assert_eq!(strokes[0].paint, FxPaint::Color(photocraft_color::Color::BLACK));
    assert_eq!(strokes[1].paint, FxPaint::Color(photocraft_color::Color::WHITE));
    assert!(strokes[0].overprint);

    // Bevel: gloss anti-alias, technique and shading values.
    let (s, style) = row(&[(
        "layer.layerStyle.bevelEmboss",
        json!({
            "glossContour": "Ring - Double", "glossAntiAlias": true, "technique": "chiselSoft",
            "highlightBlend": "Normal", "highlightColor": "#e08020", "highlightOpacity": 25,
            "shadowBlend": "Multiply", "shadowColor": "#204080", "shadowOpacity": 40
        }),
    )]);
    let bevel = style.instances(EffectKind::BevelEmboss).next().unwrap();
    assert_eq!(text(&style, bevel, "technique").as_deref(), Some("chiselSoft"));
    assert_eq!(num(&style, bevel, "highlightOpacity"), Some(25.0));
    assert_eq!(num(&style, bevel, "shadowOpacity"), Some(40.0));
    let Effect::BevelEmboss(b) = &layer_effects(&s)[0] else { panic!("a bevel") };
    assert!(b.gloss_anti_alias);
    assert_eq!(b.technique, BevelTechnique::ChiselSoft);
    assert_eq!(b.highlight.blend, BlendMode::Normal);
    assert!((b.highlight.opacity - 0.25).abs() < 0.001);
    assert_eq!(b.shadow.blend, BlendMode::Multiply);
    assert!((b.shadow.opacity - 0.4).abs() < 0.001);

    // Percent fields.
    let (_, style) =
        row(&[("layer.layerStyle.outerGlow", json!({"spread": 6, "range": 40})), ("layer.layerStyle.dropShadow", json!({"spread": 12, "add": true}))]);
    let glow = style.instances(EffectKind::OuterGlow).next().unwrap();
    let shadow = style.instances(EffectKind::DropShadow).next().unwrap();
    assert_eq!((num(&style, glow, "spread"), num(&style, glow, "range")), (Some(6.0), Some(40.0)));
    assert_eq!(num(&style, shadow, "spread"), Some(12.0));

    // Inner Glow's Center source (OK used to turn it into Edge), technique and range; Outer Glow's technique.
    let (s, style) = row(&[
        ("layer.layerStyle.innerGlow", json!({"source": "center", "technique": "precise", "range": 30})),
        ("layer.layerStyle.outerGlow", json!({"technique": "precise"})),
    ]);
    let inner = style.instances(EffectKind::InnerGlow).next().unwrap();
    assert_eq!(text(&style, inner, "source").as_deref(), Some("center"));
    assert_eq!(text(&style, inner, "technique").as_deref(), Some("precise"));
    assert_eq!(num(&style, inner, "range"), Some(30.0));
    assert!(layer_effects(&s).iter().any(|e| matches!(e, Effect::InnerGlow(g) if g.source == GlowSource::Center)));
}

#[test]
fn disabled_instances_are_kept() {
    let mut s = session();
    s.execute("layer.layerStyle.colorOverlay", json!({"color": "#ff0000"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let mut d = LayerStyleDraft::new(drafted(&s));
    d.toggle(EffectId(1));
    apply(&d.style, id, None, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    let items = layer_effects(&s);
    assert_eq!(items.len(), 1, "configured but switched off stays");
    assert!(!items[0].enabled());
}

/// The preview swatch shows the dialog's pending style, built away from the document.
#[test]
fn pending_style_is_the_dialogs_and_leaves_the_document_alone() {
    let mut app = app_with_layer(16);
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let id = open(&mut app, Some(EffectKind::Stroke.key())).unwrap();
    let stroke = first_of(&app, id, EffectKind::Stroke);
    edit(&mut app, id, |d| d.set_param(stroke, "size", StyleValue::Number(9.0)));
    let style = pending_style(&app.session.patterns, &draft(&app, id).style).expect("the pending style builds");
    let sizes: Vec<f32> = style
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::Stroke(s) => Some(s.size),
            _ => None,
        })
        .collect();
    assert_eq!(sizes, [9.0]);
    assert!(app.session.active().unwrap().doc.layer(layer).unwrap().effects.items.is_empty(), "the document is untouched");
    let rgba = photocraft_engine::presets::styles::thumbnail(&app.session, &style, 32);
    assert_eq!(rgba.len(), 32 * 32 * 4);
}

#[test]
fn baked_gradient_overlay_keeps_endpoint_colours_on_reopen() {
    let mut s = session();
    s.execute("layer.layerStyle.gradientOverlay", json!({"from": "#ff0000", "to": "#0000ff", "method": "classic"})).unwrap();
    let Effect::GradientOverlay { gradient, .. } = &layer_effects(&s)[0] else { panic!() };
    assert!(gradient.stops.len() > 2, "classic bakes intermediate stops");
    let style = drafted(&s);
    let overlay = style.instances(EffectKind::GradientOverlay).next().unwrap();
    let Some(StyleValue::Gradient(source)) = param(&style, overlay, "stops") else { panic!("the source stops load") };
    assert!(source.color.first().unwrap().color.same_rgb8(Srgba::rgb(1.0, 0.0, 0.0)));
    assert!(source.color.last().unwrap().color.same_rgb8(Srgba::rgb(0.0, 0.0, 1.0)));
    reapply(&mut s);
    let Effect::GradientOverlay { gradient, .. } = &layer_effects(&s)[0] else { panic!() };
    assert!(gradient.stops.len() > 2);
    assert_eq!(gradient.stops.first().unwrap().1, photocraft_color::Color::rgb(1.0, 0.0, 0.0));
    assert_eq!(gradient.stops.last().unwrap().1, photocraft_color::Color::rgb(0.0, 0.0, 1.0));
}

#[test]
fn gradient_strokes_survive_the_dialog() {
    let mut s = session();
    s.execute("layer.layerStyle.stroke", json!({"size": 4, "from": "#ff0000", "to": "#0000ff"})).unwrap();
    let style = drafted(&s);
    let stroke = style.instances(EffectKind::Stroke).next().unwrap();
    let params = &style.effect(stroke).unwrap().params;
    assert!(params.get("color").is_none(), "the dialog doesn't model the gradient, so no colour is stored for it");
    assert!(params.0.values().any(|v| matches!(v, StyleValue::Other(_))), "the gradient travels as unmodelled data");
    reapply(&mut s);
    let strokes = strokes_of(&s);
    assert_eq!(strokes.len(), 1);
    assert!(
        matches!(&strokes[0].paint, FxPaint::Gradient(g) if g.stops.len() == 2
            && g.stops[0].1 == photocraft_color::Color::rgb(1.0, 0.0, 0.0)
            && g.stops[1].1 == photocraft_color::Color::rgb(0.0, 0.0, 1.0)),
        "the gradient is carried, not reset",
    );
}

#[test]
fn preview_applies_the_style_without_touching_the_document() {
    let mut app = app_with_layer(32);
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let id = open(&mut app, Some(EffectKind::ColorOverlay.key())).unwrap();
    let (revision, past) = {
        let st = app.session.active().unwrap();
        (st.revision, st.history.past_len())
    };
    let stroke = {
        let mut added = None;
        edit(&mut app, id, |d| added = Some(d.add_instance(EffectKind::Stroke, &defaults(EffectKind::Stroke))));
        added.unwrap()
    };
    let hash = draft(&app, id).render_hash();
    edit(&mut app, id, |d| d.page = LayerStylePage::Effect(stroke));
    assert_eq!(draft(&app, id).render_hash(), hash, "switching pages doesn't re-render");
    edit(&mut app, id, |d| d.set_param(stroke, "size", StyleValue::Number(5.0)));
    assert_ne!(draft(&app, id).render_hash(), hash);
    let (shown, _) = preview_document(&app).expect("the preview renders");
    let st = app.session.active().unwrap();
    assert_eq!((shown.layer(layer).unwrap().effects.items.len(), st.doc.layer(layer).unwrap().effects.items.len()), (2, 0));
    assert_eq!((st.revision, st.history.past_len()), (revision, past), "the session is untouched");
}

#[test]
fn blending_page_round_trips_advanced_blending_and_blend_if() {
    for mode in ["rgb", "cmyk", "grayscale"] {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16, "mode": mode})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let (id, mut l, doc_mode, gray, light) = {
            let st = s.active().unwrap();
            let id = st.active_layer.unwrap();
            let gray = if st.doc.mode.color_channels() == 1 { 1 } else { 0 };
            (id, st.doc.layer(id).unwrap().clone(), st.doc.mode, gray, st.doc.global_light.angle)
        };
        l.advanced.knockout = Knockout::Shallow;
        l.advanced.blend_clipped = false;
        l.excluded_channels = 0b1;
        l.blend_if.set(gray, [BlendRange { black: [10, 30], white: [255, 255] }, BlendRange::FULL]);
        // The session's layer still has the defaults: applying the draft of the edited copy must set them.
        let style = draft_of(&l, light, doc_mode);
        apply(&style, id, None, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        let st = s.active().unwrap();
        let got = st.doc.layer(id).unwrap();
        assert_eq!(got.advanced.knockout, Knockout::Shallow, "{mode}");
        assert!(!got.advanced.blend_clipped, "{mode}");
        assert_eq!(got.excluded_channels, 0b1, "{mode}");
        assert_eq!(got.blend_if.get(gray)[0], BlendRange { black: [10, 30], white: [255, 255] }, "{mode}");
        // Reopening shows the same blending.
        let again = draft_of(got, st.doc.global_light.angle, st.doc.mode);
        assert_eq!(again.blending, style.blending, "{mode}");
    }
}

/// A contour the Contour Editor child answers reaches the engine through the dialog's `set_contour`.
#[test]
fn contour_picker_and_editor_edit_the_effect_curve() {
    let mut app = app_with_layer(32);
    app.run("layer.layerStyle.dropShadow", json!({"contour": "Cone"})).unwrap();
    let cone = {
        let id = open(&mut app, None).unwrap();
        let shadow = first_of(&app, id, EffectKind::DropShadow);
        let Some(StyleValue::Contour(c)) = param(&draft(&app, id).style, shadow, "contour") else { panic!("a contour") };
        cancel(&mut app, id);
        c
    };
    app.run("layer.layerStyle.dropShadow", json!({"contour": "Linear"})).unwrap();
    let id = open(&mut app, None).unwrap();
    let shadow = first_of(&app, id, EffectKind::DropShadow);
    // PLAN: assumed the contour key `fxN/contour`, as colour keys are `fxN/<field>`; the editor child's own gestures are
    // craft-imaging's `ContourEditorDialog` trials.
    let answered = app.framework.dialogs.get_mut_by_id::<HostedLayerStyle>(id).unwrap().set_contour(&format!("{}/contour", shadow.key()), cone);
    assert!(answered, "the dialog takes the contour for its field");
    ok(&mut app, id);
    let Effect::DropShadow(sh) = &layer_effects(&app.session)[0] else { panic!("a drop shadow") };
    assert_eq!(sh.contour, photocraft_engine::layer_style::builtin_contour("Cone").unwrap());
}

fn shadow_angle(s: &photocraft_engine::Session) -> (f32, bool) {
    let Some(Effect::DropShadow(sh)) = layer_effects(s).into_iter().find(|e| matches!(e, Effect::DropShadow(_))) else { panic!("a drop shadow") };
    (sh.angle, sh.use_global_light)
}

/// #350: the Angle slider was dead; the angle and Use Global Light reach the effect, and the shared light moves only
/// with a globally lit angle.
#[test]
fn drop_shadow_angle_reaches_the_effect() {
    let mut app = app_with_layer(32);

    // Use Global Light off: the per-effect angle is stored and used.
    let id = open(&mut app, Some(EffectKind::DropShadow.key())).unwrap();
    let shadow = first_of(&app, id, EffectKind::DropShadow);
    let light = app.session.active().unwrap().doc.global_light.angle;
    edit(&mut app, id, |d| {
        d.set_param(shadow, "useGlobalLight", StyleValue::Bool(false));
        d.set_param(shadow, "angle", StyleValue::Number(45.0));
    });
    ok(&mut app, id);
    assert_eq!(shadow_angle(&app.session), (45.0, false));
    assert_eq!(app.session.active().unwrap().doc.global_light.angle, light, "a local angle leaves the shared light");

    // Use Global Light on: the Angle slider drives the document's shared light angle.
    let id = open(&mut app, Some(EffectKind::DropShadow.key())).unwrap();
    let shadow = first_of(&app, id, EffectKind::DropShadow);
    edit(&mut app, id, |d| {
        d.set_param(shadow, "useGlobalLight", StyleValue::Bool(true));
        d.set_param(shadow, "angle", StyleValue::Number(30.0));
    });
    ok(&mut app, id);
    assert_eq!(app.session.active().unwrap().doc.global_light.angle, 30.0, "the Angle slider moves the shared light");
    assert!(shadow_angle(&app.session).1);

    // Reopening shows the effective angle and the check state.
    let id = open(&mut app, Some(EffectKind::DropShadow.key())).unwrap();
    let shadow = first_of(&app, id, EffectKind::DropShadow);
    let style = &draft(&app, id).style;
    assert_eq!(style.effect(shadow).and_then(|e| e.params.bool("useGlobalLight")), Some(true));
    assert_eq!(num(style, shadow, "angle"), Some(30.0));
}

#[test]
fn bevel_angle_change_is_not_overridden_by_unchanged_drop_shadow_angle() {
    let mut app = app_with_layer(32);
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let id = open(&mut app, None).unwrap();
    edit(&mut app, id, |d| {
        let bevel = d.add_instance(EffectKind::BevelEmboss, &defaults(EffectKind::BevelEmboss));
        d.set_param(bevel, "angle", StyleValue::Number(45.0));
        d.add_instance(EffectKind::DropShadow, &defaults(EffectKind::DropShadow));
    });
    ok(&mut app, id);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.global_light.angle, 45.0, "the angle that changed wins, not the first globally lit one");
    assert_eq!(st.doc.layer(layer).unwrap().effects.items.len(), 2);
}

/// No angle changed: OK runs no `layer.layerStyle.globalLight` step and the document's light stays.
#[test]
fn unchanged_global_light_angle_is_not_applied() {
    let s = session();
    let id = s.active().unwrap().active_layer.unwrap();
    let mut d = LayerStyleDraft::new(drafted(&s));
    let bevel = d.add_instance(EffectKind::BevelEmboss, &defaults(EffectKind::BevelEmboss));
    assert!(d.style.effect(bevel).is_some());
    let mut commands = Vec::new();
    // The dialog opened with the angle the draft still holds.
    apply(&d.style, id, Some(f64::from(d.style.global_light)), |cmd, _| {
        commands.push(cmd.to_string());
        Ok(Value::Null)
    })
    .unwrap();
    assert!(!commands.iter().any(|cmd| cmd == "layer.layerStyle.globalLight"), "{commands:?}");
    commands.clear();
    apply(&d.style, id, Some(47.5), |cmd, _| {
        commands.push(cmd.to_string());
        Ok(Value::Null)
    })
    .unwrap();
    assert!(commands.iter().any(|cmd| cmd == "layer.layerStyle.globalLight"), "a changed angle is applied: {commands:?}");

    let mut app = app_with_layer(32);
    let light = app.session.active().unwrap().doc.global_light.angle;
    let dialog = open(&mut app, None).unwrap();
    edit(&mut app, dialog, |d| {
        d.add_instance(EffectKind::BevelEmboss, &defaults(EffectKind::BevelEmboss));
    });
    ok(&mut app, dialog);
    assert_eq!(app.session.active().unwrap().doc.global_light.angle, light);
}

#[test]
fn unedited_bevel_shading_keeps_imported_precision() {
    for size in [None, Some(12.0)] {
        let mut s = session();
        s.execute("layer.layerStyle.bevelEmboss", json!({})).unwrap();
        s.edit("imported shading", |doc, active| {
            let Effect::BevelEmboss(b) = &mut doc.layer_mut(active.unwrap()).unwrap().effects.items[0] else { panic!("bevel") };
            b.highlight_color = photocraft_color::Color::rgb(0.12345, 0.45678, 0.78901);
            b.shadow_color = photocraft_color::Color::rgb(0.98765, 0.54321, 0.13579);
            b.highlight.opacity = 0.3525;
            b.shadow.opacity = 0.6175;
            Ok(())
        })
        .unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        let original = layer_effects(&s)[0].clone();
        let mut d = LayerStyleDraft::new(drafted(&s));
        let bevel = d.style.instances(EffectKind::BevelEmboss).next().unwrap();
        if let Some(size) = size {
            d.set_param(bevel, "size", StyleValue::Number(size));
        }
        apply(&d.style, id, None, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        let Effect::BevelEmboss(before) = original else { panic!("bevel") };
        let Effect::BevelEmboss(after) = &layer_effects(&s)[0] else { panic!("bevel") };
        assert_eq!(after.highlight, before.highlight);
        assert_eq!(after.shadow, before.shadow);
        assert_eq!(after.highlight_color, before.highlight_color);
        assert_eq!(after.shadow_color, before.shadow_color);
        if let Some(size) = size {
            assert_eq!(f64::from(after.size), size, "the unrelated edit still applies");
        }
    }
}

/// Pattern 5 (#14940): a PSD carries Inner Shadow values outside the page's typed ranges -
/// `parse_shadow` keeps Dstn, blur and lagl exactly as the file has them - and the hosted page's
/// OK re-sends every field it models. An untouched OK must not rewrite what the file imported
/// (Size 300 becoming 250 moves pixels); a field the user did edit still meets the page's clamps.
#[test]
fn unedited_inner_shadow_keeps_imported_out_of_range_values() {
    for typed in [None, Some(40_001.0)] {
        let mut s = session();
        s.execute("layer.layerStyle.innerShadow", json!({})).unwrap();
        s.edit("imported out-of-range values", |doc, active| {
            let Effect::InnerShadow(sh) = &mut doc.layer_mut(active.unwrap()).unwrap().effects.items[0] else { panic!("inner shadow") };
            sh.distance = 40_000.0;
            sh.size = 300.0;
            sh.angle = 200.0;
            sh.use_global_light = false;
            Ok(())
        })
        .unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        let mut d = LayerStyleDraft::new(drafted(&s));
        let shadow = d.style.instances(EffectKind::InnerShadow).next().unwrap();
        if let Some(distance) = typed {
            d.set_param(shadow, "distance", StyleValue::Number(distance));
        }
        apply(&d.style, id, None, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        let Effect::InnerShadow(after) = &layer_effects(&s)[0] else { panic!("inner shadow") };
        // The typed distance is past the page's 30000, so an EDITED field is clamped; an untouched
        // one keeps the imported value.
        assert_eq!(f64::from(after.distance), typed.map_or(40_000.0, |d| d.min(30_000.0)), "distance {typed:?}");
        assert_eq!(f64::from(after.size), 300.0, "an untouched Size 300 keeps what the file imported");
        assert_eq!(f64::from(after.angle), 200.0, "an untouched Angle 200 keeps what the file imported");
    }
}

/// The other side of the guard above: while the effect follows the shared light, the Angle field
/// shows that light and not the effect's own angle, so an untouched OK keeps writing the light's
/// angle as it did before this change - the imported angle was never what the field showed.
/// Guard test: it passes before and after the change on purpose.
#[test]
fn an_inner_shadow_lit_by_the_global_light_still_takes_the_shown_angle() {
    let mut s = session();
    s.execute("layer.layerStyle.innerShadow", json!({})).unwrap();
    s.edit("imported angle while lit", |doc, active| {
        let Effect::InnerShadow(sh) = &mut doc.layer_mut(active.unwrap()).unwrap().effects.items[0] else { panic!("inner shadow") };
        sh.angle = 200.0;
        sh.use_global_light = true;
        Ok(())
    })
    .unwrap();
    s.execute("layer.layerStyle.globalLight", json!({"angle": 45.0})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let d = LayerStyleDraft::new(drafted(&s));
    apply(&d.style, id, Some(45.0), |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    let Effect::InnerShadow(after) = &layer_effects(&s)[0] else { panic!("inner shadow") };
    assert_eq!(f64::from(after.angle), 45.0, "the field showed the light's 45, so OK stores 45");
}

fn overlay_color(doc: &photocraft_doc::Document, layer: photocraft_doc::LayerId) -> photocraft_color::Color {
    match &doc.layer(layer).unwrap().effects.items[0] {
        Effect::ColorOverlay { color, .. } => *color,
        other => panic!("{other:?}"),
    }
}

#[test]
fn preview_off_shows_the_original_until_reenabled() {
    let mut app = app_with_layer(32);
    app.run("layer.layerStyle.colorOverlay", json!({"color": "#ff0000"})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let red = photocraft_color::Color::rgb(1.0, 0.0, 0.0);
    let green = photocraft_color::Color::rgb(0.0, 1.0, 0.0);
    let id = open(&mut app, Some(EffectKind::ColorOverlay.key())).unwrap();
    let overlay = first_of(&app, id, EffectKind::ColorOverlay);
    let shown = |app: &PhotocraftApp| preview_document(app).map_or_else(|| app.session.active().unwrap().doc.clone(), |p| p.0);

    edit(&mut app, id, |d| {
        d.preview = false;
        d.set_param(overlay, "color", StyleValue::Color(Srgba::rgb(0.0, 1.0, 0.0)));
    });
    assert_eq!(overlay_color(&shown(&app), layer), red, "preview off shows the document");
    edit(&mut app, id, |d| d.preview = true);
    assert_eq!(overlay_color(&shown(&app), layer), green, "preview on shows the pending style");
    assert_eq!(overlay_color(&app.session.active().unwrap().doc, layer), red, "the document is untouched until OK");
    // OK commits regardless of the check.
    edit(&mut app, id, |d| d.preview = false);
    ok(&mut app, id);
    assert_eq!(overlay_color(&app.session.active().unwrap().doc, layer), green, "OK commits with Preview off");
}

fn pixel(app: &mut PhotocraftApp) -> Vec<f32> {
    serde_json::from_value(app.run("document.pixel", json!({"x": 32, "y": 32})).unwrap()).unwrap()
}

/// #3648's steps in one dialog session: disable the stroke row, add a Color Overlay, untick Preview. Preview off
/// shows the document as saved (the stroke was never disabled in it) until OK; nothing disabled comes back, and OK
/// applies the dialog's state.
#[test]
fn issue_steps_disable_stroke_add_overlay_untick_preview() {
    let mut app = app_with_layer(64);
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("layer.layerStyle.stroke", json!({"size": 40, "position": "inside", "color": "#00ff00"})).unwrap();
    let id = open(&mut app, None).unwrap();
    let stroke = first_of(&app, id, EffectKind::Stroke);
    assert_eq!(draft(&app, id).style.effects.len(), 1, "the saved stroke is the only row");
    assert!(draft(&app, id).style.effect(stroke).unwrap().on);
    edit(&mut app, id, |d| {
        d.toggle(stroke);
        let overlay = d.add_instance(EffectKind::ColorOverlay, &defaults(EffectKind::ColorOverlay));
        d.set_param(overlay, "color", StyleValue::Color(Srgba::rgb(0.0, 0.0, 1.0)));
    });
    let (shown, _) = preview_document(&app).expect("the preview renders");
    let items = &shown.layer(layer).unwrap().effects.items;
    assert!(items.iter().any(|e| matches!(e, Effect::Stroke(_)) && !e.enabled()), "the preview leaves the stroke off");
    assert!(items.iter().any(|e| matches!(e, Effect::ColorOverlay { .. }) && e.enabled()), "the preview shows the overlay");

    edit(&mut app, id, |d| d.preview = false);
    let green = pixel(&mut app);
    assert!(green[1] > 0.9 && green[0] < 0.1, "preview off shows the saved stroke: {green:?}");

    ok(&mut app, id);
    let effects = &app.session.active().unwrap().doc.layer(layer).unwrap().effects;
    assert!(effects.items.iter().any(|e| matches!(e, Effect::Stroke(_)) && !e.enabled()), "OK disables the stroke");
    assert!(effects.items.iter().any(|e| matches!(e, Effect::ColorOverlay { .. }) && e.enabled()), "OK keeps the overlay on");
    assert!(effects.enabled, "the list's master switch keeps its value");
    let blue = pixel(&mut app);
    assert!(blue[2] > 0.9 && blue[1] < 0.1, "after OK only the overlay renders: {blue:?}");
}

/// New Style… saves the enabled instances and the pending opacity; a preset row applies it to the draft only.
#[test]
fn new_style_saves_pending_state_and_a_swatch_apply_restores_it() {
    let mut app = app_with_layer(16);
    app.run("layer.layerStyle.blendingOptions", json!({"opacity": 40})).unwrap();
    let id = open(&mut app, Some(EffectKind::Stroke.key())).unwrap();
    let stroke = first_of(&app, id, EffectKind::Stroke);
    edit(&mut app, id, |d| d.set_param(stroke, "size", StyleValue::Number(9.0)));
    let mut h = harness(app);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "New Style…").click();
    h.run_steps(3);
    // The preset is saved at once under the default name "Style".
    assert_eq!(h.state().framework.dialogs.stack().top(), Some(id), "New Style… opens no other dialog");
    let preset = h.state().session.presets().styles.iter().flat_map(|g| g.items.iter()).find(|st| st.name == "Style").expect("the preset is created");
    assert_eq!(preset.effects.len(), 1, "only the enabled instance is saved");
    assert_eq!(preset.opacity, Some(0.4), "the pending opacity is saved");

    // A second layer gets the style back through the preset row.
    cancel(h.state_mut(), id);
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    let second = open(h.state_mut(), None).unwrap();
    edit(h.state_mut(), second, |d| d.page = LayerStylePage::Styles);
    h.run_steps(3);
    // The Styles page's row click asks for this apply (the framework's browser and its trials own the click).
    apply_style_preset(h.state_mut(), second, "Style");
    h.run_steps(3);
    let strokes: Vec<_> = draft(h.state(), second).style.instances(EffectKind::Stroke).collect();
    assert_eq!(strokes.len(), 1);
    assert_eq!(num(&draft(h.state(), second).style, strokes[0], "size"), Some(9.0));
    assert!(layer_effects(&h.state().session).is_empty(), "the document is untouched until OK");
    ok(h.state_mut(), second);
    let sizes: Vec<f32> = strokes_of(&h.state().session).iter().map(|s| s.size).collect();
    assert_eq!(sizes, vec![9.0], "confirming commits the style preset to the layer");
}

#[test]
fn style_click_does_not_mutate_document_until_confirm() {
    let mut app = app_with_layer(16);
    let id = open(&mut app, None).unwrap();
    edit(&mut app, id, |d| d.page = LayerStylePage::Styles);
    let mut h = harness(app);
    h.get_by_label("Black Stroke").click();
    h.run_steps(3);
    assert!(layer_effects(&h.state().session).is_empty(), "the document still has no effects");
    assert_eq!(draft(h.state(), id).style.effects.len(), 1, "the draft has the preset loaded");
    ok(h.state_mut(), id);
    assert_eq!(layer_effects(&h.state().session).len(), 1);
}

/// #14944: the Outer Glow page is grouped Photoshop-style - Noise right under Opacity, the rows of
/// the Structure, Elements and Quality blocks. The hosted page draws `specs()` rows in order, so
/// the page order is pinned here; the group TITLES belong to the framework's page (craft-libs has
/// no header row on an effect page yet) and the other pages keep the framework's order (#14917 is
/// the Inner Glow's own ticket).
#[test]
fn the_outer_glow_page_rows_group_noise_under_opacity() {
    let row_keys = |want: &str| {
        specs(&blend_keys(), &Map::new(), 120.0)
            .into_iter()
            .find(|s| s.kind.key() == want)
            .expect("the spec for the kind")
            .fields
            .iter()
            .map(|f| f.key.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(row_keys("outerGlow"), ["blend", "opacity", "noise", "color", "technique", "spread", "size", "contour", "range"]);
    assert_eq!(
        row_keys("innerGlow"),
        ["blend", "opacity", "color", "technique", "source", "choke", "size", "contour", "range", "noise"],
        "the Inner Glow page stays as it is"
    );
    assert_eq!(
        row_keys("dropShadow"),
        ["blend", "color", "opacity", "angle", "useGlobalLight", "distance", "spread", "size", "contour", "noise", "knocksOut"],
        "the Drop Shadow page stays as it is"
    );
}

/// #14944: a new Outer Glow opens at the reference application's factory values, and OK stores
/// exactly those.
#[test]
fn a_new_outer_glow_opens_at_the_factory_values_and_ok_keeps_them() {
    let mut app = app_with_layer(32);
    let id = open(&mut app, Some(EffectKind::OuterGlow.key())).unwrap();
    let glow = first_of(&app, id, EffectKind::OuterGlow);
    let style = &draft(&app, id).style;
    assert_eq!(text(style, glow, "blend").as_deref(), Some("Screen"));
    assert_eq!(text(style, glow, "technique").as_deref(), Some("softer"));
    assert_eq!(
        (num(style, glow, "opacity"), num(style, glow, "spread"), num(style, glow, "size"), num(style, glow, "range")),
        (Some(35.0), Some(0.0), Some(7.0), Some(50.0)),
        "the page's own seed"
    );
    assert_eq!(style.effect(glow).and_then(|e| e.params.bool("antiAlias")), Some(false));
    let Some(StyleValue::Color(c)) = param(style, glow, "color") else { panic!("the colour row loads as a colour") };
    assert_eq!(color_picker_ui::hex(c.to_rgb_array()), "#ffffff", "the page's own colour");
    ok(&mut app, id);
    let Effect::OuterGlow(g) = &layer_effects(&app.session)[0] else { panic!("{:?}", layer_effects(&app.session)) };
    assert!((g.common.opacity * 100.0 - 35.0).abs() < 1e-4 && (g.size - 7.0).abs() < 1e-4, "OK stored {g:?}");
    let FxPaint::Color(c) = &g.paint else { panic!("{:?}", g.paint) };
    assert_eq!(hex(c), "#ffffff", "OK stored the colour");
}

/// #14944: the factory values never reach an Outer Glow the layer already carries: the page shows
/// its own values and OK keeps them, as it does for a glow read from a PSD or `.pcraft`.
#[test]
fn an_existing_outer_glow_is_edited_with_its_own_values() {
    let mut app = app_with_layer(32);
    app.run("layer.layerStyle.outerGlow", json!({"opacity": 75, "color": "#ffffbe", "size": 5})).unwrap();
    let id = open(&mut app, Some(EffectKind::OuterGlow.key())).unwrap();
    let glow = first_of(&app, id, EffectKind::OuterGlow);
    let style = &draft(&app, id).style;
    assert_eq!((num(style, glow, "opacity"), num(style, glow, "size")), (Some(75.0), Some(5.0)), "the page shows what the layer has");
    let Some(StyleValue::Color(c)) = param(style, glow, "color") else { panic!("the colour row loads as a colour") };
    assert_eq!(color_picker_ui::hex(c.to_rgb_array()), "#ffffbe");
    ok(&mut app, id);
    let Effect::OuterGlow(g) = &layer_effects(&app.session)[0] else { panic!("no outer glow") };
    assert!((g.common.opacity * 100.0 - 75.0).abs() < 1e-4 && (g.size - 5.0).abs() < 1e-4, "OK kept {g:?}");
    let FxPaint::Color(c) = &g.paint else { panic!("{:?}", g.paint) };
    assert_eq!(hex(c), "#ffffbe", "OK kept the colour");
}

/// The preview swatch draws over the patterns' pixels: a tile that fails to read and recovers keeps the draft's render
/// hash, and the swatch still keys a new picture, so it does not keep the zeros it read during the error.
#[test]
fn the_preview_swatch_follows_a_scratch_read_error_transition() {
    let mut app = app_with_layer(32);
    let id = open(&mut app, Some(EffectKind::PatternOverlay.key())).unwrap();
    let key = swatch_key(draft(&app, id));
    assert_eq!(swatch_key(draft(&app, id)), key, "nothing changed: the same picture");
    crate::scratch_reads::tests::simulate_read_error_transition();
    assert_eq!(draft(&app, id).render_hash(), key.1, "the draft is unchanged");
    assert_ne!(swatch_key(draft(&app, id)), key, "the swatch draws again");
}
