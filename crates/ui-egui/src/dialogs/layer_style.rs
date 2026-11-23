//! Photoshop-style Layer Style dialog: the framework's `HostedLayerStyle` on the dialog host over a
//! `LayerStyleDraft`. This module converts the layer's effects to and from the draft's typed style
//! (`draft_of`, `apply`), answers the dialog's asks (`take`) and builds the canvas preview.
//!
//! The effect parameters keep the command grammar (`layer.layerStyle.replace`): a parameter the
//! dialog's field tables do not name, and the `fx` snapshot of each loaded effect, travel in the
//! draft as `StyleValue::Other` so everything the dialog doesn't model (gradient strokes, imported
//! contours, PSD data) survives an apply.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use craft::color_ui::gradient::GradientStops;
use craft::color_ui::ui::ColorPickerDialog;
use craft::imaging::layer_style::{
    BlendIf, BlendIfRange, Blending, EffectId, EffectInstance, EffectKind, EffectSpec, FieldKind, Knockout, LayerStyle, LayerStyleDraft, LayerStylePage,
    StyleParams, StyleValue,
};
use craft::imaging::ui::layer_style::{ContourEditorDialog, HostedLayerStyle, LayerStyleAsk};
use craft::theme::Srgba;
use craft::ui::contract::Entry;
use craft::ui::contract::Tracked;
use craft::ui::dialog::{DialogId, DialogResult};
use craft::ui::patterns::presets::{PresetFns, PresetRow, PresetSource, ThumbInfo};
use craft::ui::primitives::ContentPainterCx;
use craft::ui::primitives::textures::TextureCache;
use craft::ui::runtime::env::Env;
use egui::{ColorImage, Pos2, Rect, pos2};
use photocraft_doc::effects::{FxPaint, GlowSource, GlowTechnique, StrokePosition};
use photocraft_doc::{Document, Effect, Layer, LayerId};
use photocraft_engine::Session;
use photocraft_engine::pattern_cmds::PatternLibrary;
use photocraft_engine::presets::styles::StylePreset;
use serde_json::{Map, Value, json};

use crate::panels::preset_browser::StyleLibrary;
use crate::{PhotocraftApp, color_picker_ui};

#[cfg(test)]
mod bevel_reset_angle_tests;
pub(crate) mod color_picker;
mod contour;
mod gradient_field;
#[cfg(test)]
mod gradient_method_tests;

/// The Styles preset gallery page id.
pub const STYLES_PAGE: &str = "styles";
/// The Blending Options page id (not an effect).
pub const BLENDING: &str = "blendingOptions";

/// Effect kinds in Photoshop's list order: (command kind, label).
pub const KINDS: &[(&str, &str)] = &[
    ("bevelEmboss", "Bevel & Emboss"),
    ("stroke", "Stroke"),
    ("innerShadow", "Inner Shadow"),
    ("innerGlow", "Inner Glow"),
    ("satin", "Satin"),
    ("colorOverlay", "Color Overlay"),
    ("gradientOverlay", "Gradient Overlay"),
    ("patternOverlay", "Pattern Overlay"),
    ("outerGlow", "Outer Glow"),
    ("dropShadow", "Drop Shadow"),
];

fn kind_of(e: &Effect) -> &'static str {
    photocraft_engine::layer_style::kind_of(e)
}

/// The engine's factory defaults for `kind`. Gradient Overlay's From and To are left out: the page
/// models a stop list, and the engine would let a From/To pair override it.
fn defaults(kind: &str) -> Value {
    let mut p = photocraft_engine::layer_style::effect_defaults(kind);
    if kind == "gradientOverlay" {
        drop_from_to(&mut p);
    }
    p
}

fn drop_from_to(p: &mut Value) {
    if let Some(o) = p.as_object_mut() {
        o.remove("from");
        o.remove("to");
    }
}

/// `params` over the factory defaults of `kind`.
fn with_defaults(kind: &str, params: &Value) -> Value {
    let mut p = defaults(kind);
    if let (Some(o), Some(pv)) = (p.as_object_mut(), params.as_object()) {
        for (k, v) in pv {
            o.insert(k.clone(), v.clone());
        }
    }
    p
}

/// Params for a new instance: the user default saved by Make Default over the factory
/// defaults (#9477), with the shared light angle filled in.
fn fresh_params(kind: &str, light: f32, user: Option<&Value>) -> Value {
    let mut p = user.map_or_else(|| defaults(kind), |u| with_defaults(kind, u));
    if p.get("useGlobalLight").and_then(Value::as_bool) == Some(true) {
        p["angle"] = json!(light);
    }
    p
}

fn hex(c: &photocraft_doc::Color) -> String {
    let [r, g, b, _] = c.to_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Current values of an existing effect, in the command's parameter units —
/// only the keys this dialog models. Everything else (gradient strokes, multi-stop
/// gradients, imported contours) is left out so the apply overlay can't clobber it.
/// `light` is the document's global light angle: an effect that uses it shows that angle.
fn values_of(e: &Effect, light: f32) -> Value {
    let mut v = defaults(kind_of(e));
    let set = |v: &mut Value, k: &str, x: Value| {
        v[k] = x;
    };
    let drop_keys = |v: &mut Value, keys: &[&str]| {
        if let Some(o) = v.as_object_mut() {
            for k in keys {
                o.remove(*k);
            }
        }
    };
    match e {
        Effect::DropShadow(s) | Effect::InnerShadow(s) => {
            set(&mut v, "blend", json!(s.common.blend.label()));
            set(&mut v, "opacity", json!((s.common.opacity * 100.0).round()));
            set(&mut v, "color", json!(hex(&s.color)));
            set(&mut v, "angle", json!(if s.use_global_light { light } else { s.angle }));
            set(&mut v, "useGlobalLight", json!(s.use_global_light));
            set(&mut v, "distance", json!(s.distance));
            set(&mut v, if matches!(e, Effect::DropShadow(_)) { "spread" } else { "choke" }, json!((s.spread * 100.0).round()));
            set(&mut v, "size", json!(s.size));
            set(&mut v, "contour", photocraft_engine::layer_style::contour_param(&s.contour));
            set(&mut v, "antiAlias", json!(s.anti_alias));
            set(&mut v, "noise", json!((s.noise * 100.0).round()));
            if matches!(e, Effect::DropShadow(_)) {
                set(&mut v, "knocksOut", json!(s.knocks_out));
            }
        }
        Effect::OuterGlow(g) | Effect::InnerGlow(g) => {
            set(&mut v, "blend", json!(g.common.blend.label()));
            set(&mut v, "opacity", json!((g.common.opacity * 100.0).round()));
            if let FxPaint::Color(c) = &g.paint {
                set(&mut v, "color", json!(hex(c)));
            } else {
                drop_keys(&mut v, &["color"]);
            }
            set(&mut v, "technique", json!(if g.technique == GlowTechnique::Precise { "precise" } else { "softer" }));
            set(&mut v, if matches!(e, Effect::OuterGlow(_)) { "spread" } else { "choke" }, json!((g.spread * 100.0).round()));
            set(&mut v, "size", json!(g.size));
            set(&mut v, "range", json!((g.range * 100.0).round()));
            set(&mut v, "contour", photocraft_engine::layer_style::contour_param(&g.contour));
            set(&mut v, "antiAlias", json!(g.anti_alias));
            set(&mut v, "noise", json!((g.noise * 100.0).round()));
            if matches!(e, Effect::InnerGlow(_)) {
                set(&mut v, "source", json!(if g.source == GlowSource::Center { "center" } else { "edge" }));
            }
        }
        Effect::Stroke(s) => {
            set(&mut v, "blend", json!(s.common.blend.label()));
            set(&mut v, "opacity", json!((s.common.opacity * 100.0).round()));
            set(&mut v, "size", json!(s.size));
            set(
                &mut v,
                "position",
                json!(match s.position {
                    StrokePosition::Inside => "inside",
                    StrokePosition::Center => "center",
                    _ => "outside",
                }),
            );
            set(&mut v, "overprint", json!(s.overprint));
            if let FxPaint::Color(c) = &s.paint {
                set(&mut v, "color", json!(hex(c)));
            } else {
                // A gradient/pattern stroke: the dialog's colour field doesn't model
                // the paint, so the key is left out and the snapshot carries it.
                drop_keys(&mut v, &["color"]);
            }
        }
        Effect::ColorOverlay { common, color } => {
            set(&mut v, "blend", json!(common.blend.label()));
            set(&mut v, "opacity", json!((common.opacity * 100.0).round()));
            set(&mut v, "color", json!(hex(color)));
        }
        Effect::GradientOverlay { common, gradient: g, interpolation, .. } => {
            set(&mut v, "blend", json!(common.blend.label()));
            set(&mut v, "opacity", json!((common.opacity * 100.0).round()));
            let stops = interpolation.as_ref().map_or(&g.stops, |source| &source.stops);
            if let Some(source) = interpolation {
                set(&mut v, "method", json!(source.method));
                set(&mut v, "smoothness", json!(source.smoothness));
            }
            let (stops, opacity) = gradient_field::stops_to_json(&gradient_field::stops_of_doc(stops, &g.opacity_stops));
            set(&mut v, gradient_field::KEY, stops);
            set(&mut v, gradient_field::OPACITY_KEY, opacity);
            set(&mut v, "reverse", json!(g.reverse));
            set(&mut v, "offset", json!([g.offset.0, g.offset.1]));
            set(
                &mut v,
                "style",
                json!(match g.style {
                    photocraft_doc::effects::GradientStyle::Radial => "radial",
                    photocraft_doc::effects::GradientStyle::Angle => "angle",
                    photocraft_doc::effects::GradientStyle::Reflected => "reflected",
                    photocraft_doc::effects::GradientStyle::Diamond => "diamond",
                    photocraft_doc::effects::GradientStyle::Linear => "linear",
                }),
            );
            set(&mut v, "angle", json!(g.angle));
            set(&mut v, "scale", json!((g.scale * 100.0).round()));
        }
        Effect::PatternOverlay { common, name, id, scale, angle, link, phase } => {
            set(&mut v, "blend", json!(common.blend.label()));
            set(&mut v, "opacity", json!((common.opacity * 100.0).round()));
            set(&mut v, "pattern", json!(if id.is_empty() { name } else { id }));
            set(&mut v, "scale", json!((scale * 100.0).round()));
            set(&mut v, "angle", json!(angle));
            set(&mut v, "link", json!(link));
            set(&mut v, "phaseX", json!(phase.0));
            set(&mut v, "phaseY", json!(phase.1));
        }
        Effect::Satin(s) => {
            set(&mut v, "blend", json!(s.common.blend.label()));
            set(&mut v, "opacity", json!((s.common.opacity * 100.0).round()));
            set(&mut v, "color", json!(hex(&s.color)));
            set(&mut v, "angle", json!(s.angle));
            set(&mut v, "distance", json!(s.distance));
            set(&mut v, "size", json!(s.size));
            set(&mut v, "invert", json!(s.invert));
            set(&mut v, "contour", photocraft_engine::layer_style::contour_param(&s.contour));
            set(&mut v, "antiAlias", json!(s.anti_alias));
        }
        Effect::BevelEmboss(b) => {
            set(
                &mut v,
                "style",
                json!(match b.style {
                    photocraft_doc::effects::BevelStyle::OuterBevel => "outer",
                    photocraft_doc::effects::BevelStyle::Emboss => "emboss",
                    photocraft_doc::effects::BevelStyle::PillowEmboss => "pillow",
                    photocraft_doc::effects::BevelStyle::StrokeEmboss => "stroke",
                    photocraft_doc::effects::BevelStyle::InnerBevel => "inner",
                }),
            );
            set(
                &mut v,
                "technique",
                json!(match b.technique {
                    photocraft_doc::effects::BevelTechnique::ChiselHard => "chiselHard",
                    photocraft_doc::effects::BevelTechnique::ChiselSoft => "chiselSoft",
                    photocraft_doc::effects::BevelTechnique::Smooth => "smooth",
                }),
            );
            set(&mut v, "depth", json!((b.depth * 100.0).round()));
            set(&mut v, "size", json!(b.size));
            set(&mut v, "soften", json!(b.soften));
            set(&mut v, "angle", json!(if b.use_global_light { light } else { b.angle }));
            set(&mut v, "useGlobalLight", json!(b.use_global_light));
            set(&mut v, "altitude", json!(b.altitude));
            set(&mut v, "direction", json!(if b.up { "up" } else { "down" }));
            set(&mut v, "glossContour", photocraft_engine::layer_style::contour_param(&b.gloss_contour));
            set(&mut v, "glossAntiAlias", json!(b.gloss_anti_alias));
            set(&mut v, "highlightBlend", json!(b.highlight.blend.label()));
            set(&mut v, "highlightColor", json!(hex(&b.highlight_color)));
            set(&mut v, "highlightOpacity", json!((b.highlight.opacity * 100.0).round()));
            set(&mut v, "shadowBlend", json!(b.shadow.blend.label()));
            set(&mut v, "shadowColor", json!(hex(&b.shadow_color)));
            set(&mut v, "shadowOpacity", json!((b.shadow.opacity * 100.0).round()));
            set(&mut v, "contour", json!(b.contour.is_some()));
            if let Some(c) = &b.contour {
                set(&mut v, "contourName", photocraft_engine::layer_style::contour_param(&c.contour));
                set(&mut v, "contourRange", json!((c.range * 100.0).round()));
                set(&mut v, "contourAntiAlias", json!(c.anti_alias));
            }
        }
    }
    v
}

// ------------------------------------------------------------------ the draft's typed style

/// The point a parameter value holds (a Gradient Overlay's `offset`).
fn point(v: &Value) -> Option<[f64; 2]> {
    let a = v.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
}

/// A parameter no field table types: by its JSON type, else kept as JSON text.
fn json_value(v: &Value) -> StyleValue {
    match v {
        Value::Bool(b) => StyleValue::Bool(*b),
        Value::Number(n) => StyleValue::Number(n.as_f64().unwrap_or(0.0)),
        Value::String(s) => StyleValue::Text(s.clone()),
        other => StyleValue::Other(other.to_string()),
    }
}

/// The draft's parameters for the command parameters `values` of an effect of `kind`: each typed by
/// the spec's field kind, the rest by their JSON type. `fx` is the effect snapshot, kept as `Other`
/// under `"fx"`.
fn style_params(kind: EffectKind, values: &Value, fx: Option<&Value>) -> StyleParams {
    let spec = EffectSpec::photoshop(kind);
    let mut params = BTreeMap::new();
    for (key, v) in values.as_object().into_iter().flatten() {
        // The transparency stops are part of the Gradient value.
        if key == gradient_field::OPACITY_KEY {
            continue;
        }
        let typed = match spec.field(key).map(|f| &f.kind) {
            Some(FieldKind::Color) => v.as_str().and_then(color_picker_ui::parse_hex).map(|rgb| StyleValue::Color(Srgba::from_rgb_array(rgb))),
            Some(FieldKind::Contour { .. }) => contour::from_param(v).map(StyleValue::Contour),
            Some(FieldKind::Gradient) => gradient_field::stops_from_json(values).map(StyleValue::Gradient),
            _ if key == "offset" => point(v).map(StyleValue::Point),
            _ => None,
        };
        params.insert(key.clone(), typed.unwrap_or_else(|| json_value(v)));
    }
    if let Some(fx) = fx {
        params.insert("fx".into(), StyleValue::Other(fx.to_string()));
    }
    StyleParams(params)
}

/// The command parameters for the draft's `params`, without the `fx` snapshot ([`fx_of`]).
fn style_params_json(params: &StyleParams) -> Value {
    let mut o = Map::new();
    for (key, v) in &params.0 {
        let value = match v {
            StyleValue::Number(n) => json!(n),
            StyleValue::Bool(b) => json!(b),
            StyleValue::Text(t) => json!(t),
            StyleValue::Color(c) => json!(color_picker_ui::hex(c.to_rgb_array())),
            StyleValue::Contour(c) => contour::to_param(c),
            StyleValue::Gradient(g) => {
                let (stops, opacity) = gradient_field::stops_to_json(g);
                o.insert(key.clone(), stops);
                o.insert(gradient_field::OPACITY_KEY.into(), opacity);
                continue;
            }
            StyleValue::Point(p) => json!(p),
            StyleValue::Other(_) if key == "fx" => continue,
            StyleValue::Other(s) => serde_json::from_str(s).unwrap_or(Value::Null),
            // A value kind the dialog does not know carries nothing the command grammar names.
            _ => continue,
        };
        o.insert(key.clone(), value);
    }
    Value::Object(o)
}

/// Adds each effect's command parameters (`params`, as [`style_params_json`] gives them) to the open Layer Style dialog's
/// entry of `dialogs`, the `ui.inspect` listing: the framework's own listing names the effects but not their values, and
/// an agent reads a gradient's stops or a colour from here.
pub(crate) fn describe_params(app: &PhotocraftApp, dialogs: &mut Value) {
    let Some((id, ..)) = app.layer_style_target else { return };
    let Some(dialog) = app.framework.dialogs.get_by_id::<HostedLayerStyle>(id) else { return };
    let key = id.value().to_string();
    let Some(entry) = dialogs.as_array_mut().and_then(|list| list.iter_mut().find(|d| d["id"] == key.as_str())) else { return };
    let Some(effects) = entry.pointer_mut("/fields/effects").and_then(Value::as_array_mut) else { return };
    for effect in effects {
        let instance = effect["id"].as_str().and_then(EffectId::from_key).and_then(|id| dialog.draft().style.effect(id));
        if let Some(instance) = instance {
            effect["params"] = style_params_json(&instance.params);
        }
    }
}

/// The effect snapshot the instance was loaded with; none for an instance added in the dialog.
fn fx_of(params: &StyleParams) -> Option<Value> {
    match params.get("fx") {
        Some(StyleValue::Other(s)) => serde_json::from_str(s).ok().filter(Value::is_object),
        _ => None,
    }
}

fn effect_params(kind: EffectKind, e: &Effect, light: f32) -> StyleParams {
    style_params(kind, &values_of(e, light), serde_json::to_value(e).ok().as_ref())
}

/// The draft's style for `layer`: one instance per effect (`fxN`, N from 1, in the layer's order),
/// the Blending Options of a `mode` document and the document's global `light` angle.
pub(crate) fn draft_of(layer: &Layer, light: f32, mode: photocraft_color::ColorMode) -> LayerStyle {
    let effects = layer
        .effects
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let kind = EffectKind::from_key(kind_of(e))?;
            Some(EffectInstance { id: EffectId(i as u32 + 1), kind, on: e.enabled(), params: effect_params(kind, e, light) })
        })
        .collect();
    let mut blending = Blending::default();
    set_blending_fields(&mut blending, layer, mode);
    LayerStyle { effects, blending, global_light: light }
}

/// Colour channel names of a document mode, as Blending Options lists them (Channels, Blend If).
pub fn channel_names(mode: photocraft_color::ColorMode) -> Vec<&'static str> {
    use photocraft_color::ColorMode as M;
    match mode {
        M::Rgb | M::Indexed | M::Multichannel => vec!["R", "G", "B"],
        M::Cmyk => vec!["C", "M", "Y", "K"],
        M::Lab => vec!["L", "a", "b"],
        _ => vec!["Gray"],
    }
}

/// Reads `layer`'s Blending Options of a `mode` document into `b`.
pub(crate) fn set_blending_fields(b: &mut Blending, layer: &Layer, mode: photocraft_color::ColorMode) {
    let n = channel_names(mode).len();
    let a = &layer.advanced;
    // Blend If entries in the command's form: index 0 is Gray for colour documents; a one-channel
    // document's Gray is its channel's own entry (index 1).
    let first = if n == 1 { 1 } else { 0 };
    b.blend = layer.blend.label().to_string();
    b.opacity = (layer.opacity * 100.0).round();
    b.fill_opacity = (layer.fill_opacity * 100.0).round();
    b.channels = (0..n).map(|i| layer.excluded_channels & (1 << i) == 0).collect();
    b.knockout = match a.knockout.name() {
        "shallow" => Knockout::Shallow,
        "deep" => Knockout::Deep,
        _ => Knockout::None,
    };
    b.blend_interior_effects_as_group = a.blend_interior;
    b.blend_clipped_layers_as_group = a.blend_clipped;
    b.transparency_shapes_layer = a.transparency_shapes;
    b.layer_mask_hides_effects = a.layer_mask_hides_effects;
    b.vector_mask_hides_effects = a.vector_mask_hides_effects;
    b.blend_if = (first..=n)
        .map(|i| {
            let [this, under] = layer.blend_if.get(i);
            BlendIf { channel: i, this_layer: BlendIfRange::ordered(this.to_bytes()), underlying: BlendIfRange::ordered(under.to_bytes()) }
        })
        .collect();
}

/// The parameters of `layer.layerStyle.blendingOptions` for `b`.
fn blending_json(b: &Blending, layer: LayerId) -> Value {
    json!({
        "layer": layer.0,
        "blend": b.blend,
        "opacity": b.opacity,
        "fillOpacity": b.fill_opacity,
        "channels": b.channels,
        "knockout": match b.knockout {
            Knockout::None => "none",
            Knockout::Shallow => "shallow",
            Knockout::Deep => "deep",
        },
        "blendInteriorEffectsAsGroup": b.blend_interior_effects_as_group,
        "blendClippedLayersAsGroup": b.blend_clipped_layers_as_group,
        "transparencyShapesLayer": b.transparency_shapes_layer,
        "layerMaskHidesEffects": b.layer_mask_hides_effects,
        "vectorMaskHidesEffects": b.vector_mask_hides_effects,
        "blendIf": b.blend_if.iter().map(|e| json!({"channel": e.channel, "thisLayer": e.this_layer.0, "underlying": e.underlying.0})).collect::<Vec<_>>(),
    })
}

// ------------------------------------------------------------------ opening

/// Opens the Layer Style dialog on the active layer; `select` is a `KINDS` key, `STYLES_PAGE` or
/// `BLENDING`. `None` when there is no active layer, or when a Free Transform box refuses to apply.
pub fn open(app: &mut PhotocraftApp, select: Option<&str>) -> Option<DialogId> {
    // An open Free Transform box is applied first, as in Photoshop (#4821): the canvas shows the
    // box's preview while it is open, so the dialog's preview would not show until Enter.
    if app.ui.transform.is_some() {
        crate::transform_tool::commit(app);
        if app.ui.transform.is_some() && crate::transform_tool::committing(app).is_none() {
            return None; // refused: the box stays open, with the reason in the status bar
        }
    }
    let idx = app.session.active_index()?;
    let st = app.session.active()?;
    let layer_id = st.active_layer?;
    let layer = st.doc.layer(layer_id)?.clone();
    let (light, mode, doc) = (st.doc.global_light.angle, st.doc.mode, st.doc.id);
    let user: Map<String, Value> = app.session.presets.layer_defaults.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let keys = blend_keys();
    let specs = specs(&keys, &user, light);
    let mut draft = LayerStyleDraft::new(draft_of(&layer, light, mode));
    match select {
        None => {
            if let Some(first) = draft.style.effects.first() {
                draft.page = LayerStylePage::Effect(first.id);
            }
        }
        Some(STYLES_PAGE) => draft.page = LayerStylePage::Styles,
        Some(BLENDING) => draft.page = LayerStylePage::Blending,
        Some(key) => {
            if let Some(kind) = EffectKind::from_key(key) {
                match specs.iter().find(|s| s.kind == kind) {
                    Some(spec) if draft.style.instances(kind).next().is_none() => {
                        draft.add_instance(kind, &spec.defaults);
                    }
                    _ => draft.open_on(kind),
                }
            }
        }
    }
    let channels = channel_names(mode).into_iter().map(String::from).collect();
    let hosted = HostedLayerStyle::new(tl!("Layer Style").to_string(), draft, channels)
        .specs(specs)
        .blend_keys(keys)
        .blend_modes(blend_modes(&layer))
        .patterns(PresetSource::new(pattern_list(app), PresetFns::new(|patterns, env| pattern_rows(patterns, env))))
        .user_contours(contour::user_contours(app))
        .sampler(contour::sampler)
        .style_presets(style_presets(app))
        .gradient_presets(gradient_presets(app))
        .preview_thumb(preview_thumb(app))
        .canvas(crate::canvas::document_display::CanvasHost::primary(app, idx).canvas_id());
    let id = app.framework.dialogs.open(hosted);
    app.layer_style_target = Some((id, doc, layer_id));
    Some(id)
}

/// Double-click on an effect row of the Layers panel: open Layer Style for that row's layer. A
/// row of another layer selects it first, as its right-click menu does (Photoshop does the same).
pub fn open_for_layer(app: &mut PhotocraftApp, layer: LayerId, select: Option<&str>) -> Option<DialogId> {
    let active = app.session.active().is_some_and(|s| s.active_layer == Some(layer));
    if !active {
        app.run("layer.select", json!({"layer": layer.0})).ok()?;
    }
    open(app, select)
}

/// The layer the open Layer Style dialog edits; `None` when it is closed or another document is active.
pub(crate) fn open_layer(app: &PhotocraftApp) -> Option<LayerId> {
    let (id, doc, layer) = app.layer_style_target?;
    app.framework.dialogs.get_by_id::<HostedLayerStyle>(id)?;
    (app.session.active()?.doc.id == doc).then_some(layer)
}

/// The effect pages with the user's defaults (Make Default) over the factory ones.
fn specs(keys: &HashMap<String, String>, user: &Map<String, Value>, light: f32) -> Vec<EffectSpec> {
    KINDS
        .iter()
        .filter_map(|(key, _)| {
            let kind = EffectKind::from_key(key)?;
            let mut spec = EffectSpec::photoshop(kind).with_blend_keys(keys);
            if kind == EffectKind::OuterGlow {
                // #14944: Photoshop groups the Outer Glow rows Structure / Elements / Quality and
                // puts Noise right under Opacity. The hosted page draws the spec's rows in order,
                // so the host moves the row; the group titles belong to the framework's page.
                if let Some(noise) = spec.fields.iter().position(|f| f.key == "noise") {
                    let field = spec.fields.remove(noise);
                    let after = spec.fields.iter().position(|f| f.key == "opacity").unwrap_or(0);
                    spec.fields.insert(after + 1, field);
                }
            }
            spec.defaults = style_params(kind, &fresh_params(key, light, user.get(*key)), None).over(&spec.defaults);
            Some(spec)
        })
        .collect()
}

/// The blend modes of the dialog's selects: Photoshop's, plus the layer's own Paint.NET mode when
/// it has one (an imported .pdn).
fn blend_modes(layer: &Layer) -> Vec<Entry<String>> {
    let own = photocraft_color::BlendMode::PAINT_NET_MODES.into_iter().find(|m| m.label() == layer.blend.label());
    photocraft_color::BlendMode::LAYER_MODES.into_iter().chain(own).map(|m| Entry::new(m.label().to_string(), m.label().to_string())).collect()
}

/// English blend-mode name to the app's key: PhotoCraft keys modes by their label.
fn blend_keys() -> HashMap<String, String> {
    photocraft_color::BlendMode::LAYER_MODES.into_iter().map(|m| (m.label().to_string(), m.label().to_string())).collect()
}

/// The patterns a style can use (document's, then the library's), as (id, name).
pub fn pattern_list(app: &PhotocraftApp) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let doc_pats = app.session.active().map(|d| d.doc.patterns.as_slice()).unwrap_or_default();
    doc_pats
        .iter()
        .chain(app.session.patterns.items.iter())
        .filter(|p| seen.insert(p.id.clone()))
        .map(|p| (p.id.clone(), p.display_name().to_string()))
        .collect()
}

/// One row per pattern of [`pattern_list`], keyed by its id.
fn pattern_rows(patterns: &[(String, String)], _: &Env) -> Vec<PresetRow<String>> {
    patterns.iter().map(|(id, name)| PresetRow::item(id.clone(), name.clone(), 0)).collect()
}

/// The Gradients panel's presets resolved with the tool colours, for the Gradient field's select.
fn gradient_presets(app: &PhotocraftApp) -> Vec<(String, GradientStops)> {
    let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
    app.session
        .presets
        .gradients
        .iter()
        .flat_map(|g| g.items.iter())
        .filter_map(|p| {
            let (stops, opacity) = p.fill_stops(fg, bg);
            (stops.len() >= 2).then(|| (p.name.clone(), gradient_field::stops_of_doc(&stops, &opacity)))
        })
        .collect()
}

/// What the Styles page's presets are built from: each style group's name and its styles' names, and the library the
/// thumbnails draw from, as the app held them when the dialog opened.
#[derive(craft::ui::contract::MemoProps)]
struct StylePresets {
    groups: Vec<(String, Vec<String>)>,
    library: std::sync::Arc<Tracked<StyleLibrary>>,
}

/// The Styles page's presets.
fn style_presets(app: &mut PhotocraftApp) -> PresetSource<'static, String> {
    let groups = app.session.presets.styles.iter().map(|group| (group.name.clone(), group.items.iter().map(|st| st.name.clone()).collect())).collect();
    let inputs = StylePresets { groups, library: StyleLibrary::held(app) };
    PresetSource::new(inputs, PresetFns::new(style_rows).thumb(style_thumb))
}

/// The Styles page's rows: each group with its presets, presets keyed by name and groups by `group/<name>`.
fn style_rows(inputs: &StylePresets, _: &Env) -> Vec<PresetRow<String>> {
    let mut rows = Vec::new();
    for (group, styles) in &inputs.groups {
        rows.push(PresetRow::group(format!("group/{group}"), group.clone(), 0).count(styles.len()));
        rows.extend(styles.iter().map(|name| PresetRow::item(name.clone(), name.clone(), 1)));
    }
    rows
}

/// Paints a style preset's thumbnail; the Styles panel shares the textures.
fn style_thumb(inputs: &StylePresets, cx: &ContentPainterCx, rect: Rect, info: ThumbInfo<'_, String>) {
    inputs.library.paint(cx, rect, info.key);
}

fn paint_picture(cx: &ContentPainterCx, rect: Rect, key: impl std::hash::Hash + 'static, image: impl FnOnce() -> ColorImage) {
    let texture = TextureCache::texture(cx.painter.ctx(), key, image);
    cx.picture(texture, rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), 0.0);
}

/// The texture store key of the preview swatch: the draft's render hash and the scratch read-error generation, as the
/// swatch draws over the patterns' pixels, which a tile that fails to read shows as zeros until it reads again
/// (`crate::scratch_reads`).
fn swatch_key(draft: &LayerStyleDraft) -> (&'static str, u64, u64) {
    ("layer-style-swatch", draft.render_hash(), crate::scratch_reads::generation())
}

/// Paints the preview swatch: the draft's style on a sample square, rebuilt when its render hash changes.
fn preview_thumb(app: &PhotocraftApp) -> impl Fn(&ContentPainterCx, Rect, &LayerStyleDraft) + Send + Sync + 'static {
    let patterns = app.session.patterns.clone();
    move |cx, rect, draft| {
        paint_picture(cx, rect, swatch_key(draft), || {
            let mut scratch = Session::new();
            scratch.patterns = patterns.clone();
            match pending_style(&patterns, &draft.style) {
                Some(preset) => crate::panels::preset_browser::style_image(&scratch, &preset),
                None => ColorImage::from_rgba_unmultiplied([1, 1], &[0, 0, 0, 0]),
            }
        });
    }
}

// ------------------------------------------------------------------ applying

fn effect_entry(fx: Option<&Value>, kind: &str, mut params: Value, initial_light_angle: Option<f64>) -> Value {
    match fx {
        Some(fx) => {
            if kind == "bevelEmboss"
                && let Ok(original) = serde_json::from_value::<Effect>(fx.clone())
                && let Some(edits) = params.as_object_mut()
            {
                let shown = values_of(&original, initial_light_angle.unwrap_or(120.0) as f32);
                // Unedited display values must not quantize imported shading in the snapshot.
                for key in ["highlightBlend", "highlightColor", "highlightOpacity", "shadowBlend", "shadowColor", "shadowOpacity"] {
                    if edits.get(key) == shown.get(key) {
                        edits.remove(key);
                    }
                }
            }
            if kind == "innerShadow"
                && let Ok(original) = serde_json::from_value::<Effect>(fx.clone())
                && let Some(edits) = params.as_object_mut()
            {
                let shown = values_of(&original, initial_light_angle.unwrap_or(120.0) as f32);
                // A file can carry a Distance or Size outside the page's typed ranges (#14940) that
                // the command clamps. An untouched field must not send it back through the clamp.
                for key in ["distance", "size"] {
                    if edits.get(key) == shown.get(key) {
                        edits.remove(key);
                    }
                }
                // The Angle field shows the shared light while the effect is lit by it, so the
                // effect's own angle is only what the field showed when the check is off; with it on,
                // the light angle is written as before.
                if shown.get("useGlobalLight") == Some(&Value::Bool(false)) && edits.get("angle") == shown.get("angle") {
                    edits.remove("angle");
                }
            }
            if kind == "gradientOverlay"
                && let Ok(original) = serde_json::from_value::<Effect>(fx.clone())
                && let Some(edits) = params.as_object_mut()
            {
                // Unedited stops must not rebuild a baked curve from their rounded copy.
                let shown = values_of(&original, initial_light_angle.unwrap_or(120.0) as f32);
                let unchanged = gradient_field::stops_from_json(&Value::Object(edits.clone()))
                    .zip(gradient_field::stops_from_json(&shown))
                    .is_some_and(|(edited, opened)| gradient_field::same(&edited, &opened));
                if unchanged {
                    edits.remove(gradient_field::KEY);
                    edits.remove(gradient_field::OPACITY_KEY);
                }
            }
            json!({"kind": kind, "fx": fx, "params": params})
        }
        // A new instance: the page shows the factory defaults merged with the edits, and the engine
        // builds from the full set.
        None => json!({"kind": kind, "params": with_defaults(kind, &params)}),
    }
}

/// The global light angle to apply: the style's own when it moved, else the angle of an enabled
/// instance lit by it that differs from the opening angle (an unchanged angle is not applied, so an
/// unchanged shadow angle does not undo a bevel angle).
fn light_to_apply(style: &LayerStyle, initial: Option<f64>) -> Option<f64> {
    let shared = f64::from(style.global_light);
    if initial != Some(shared) {
        return Some(shared);
    }
    let mut light_angle = None;
    for e in style.effects.iter().filter(|e| e.on) {
        if e.params.bool("useGlobalLight") == Some(true)
            && let Some(angle) = e.params.number("angle")
            && initial.is_none_or(|i| angle != i)
        {
            light_angle = Some(angle);
        }
    }
    light_angle
}

/// Runs the style's commands through `run`: blending options, then the whole effect list in one
/// `layer.layerStyle.replace` (instances keep their order; their snapshots carry what the dialog
/// doesn't model). `light` is the document's global light angle when the dialog opened.
// The typed draft cannot hold a malformed parameter group, so there is nothing to validate.
pub(crate) fn apply(
    style: &LayerStyle,
    layer: LayerId,
    light: Option<f64>,
    mut run: impl FnMut(&str, Value) -> Result<Value, String>,
) -> Result<Value, String> {
    run("layer.layerStyle.blendingOptions", blending_json(&style.blending, layer))?;
    let entries: Vec<Value> = style
        .effects
        .iter()
        .map(|e| {
            let mut params = style_params_json(&e.params);
            params["enabled"] = json!(e.on);
            effect_entry(fx_of(&e.params).as_ref(), e.kind.key(), params, light)
        })
        .collect();
    run("layer.layerStyle.replace", json!({"layer": layer.0, "effects": entries}))?;
    // An effect lit by the global light follows the document's light angle, so the Angle slider
    // drives that shared angle rather than the per-effect angle the compositor ignores.
    if let Some(angle) = light_to_apply(style, light) {
        run("layer.layerStyle.globalLight", json!({"angle": angle}))?;
    }
    Ok(Value::Null)
}

/// The child dialog the Layer Style dialog has open, kept until its result is taken: the dialog host
/// holds a result by the child's id only, and the id is not derivable once the child is gone.
#[derive(Clone)]
pub(crate) enum Child {
    /// A colour picker over the draft colour `key` names, opened at `opening`.
    Picker {
        /// The picker's dialog id.
        id: DialogId,
        /// The colour request key.
        key: String,
        /// The colour it opened with.
        opening: Srgba,
    },
    /// A Contour Editor over the contour `key` names, opened on `initial`.
    Editor {
        /// The editor's dialog id.
        id: DialogId,
        /// The contour request key.
        key: String,
        /// The contour it opened with.
        initial: craft::imaging::layer_style::Contour,
    },
}

/// The child of the Layer Style dialog `parent`.
fn child_of(app: &PhotocraftApp, parent: DialogId) -> Option<&Child> {
    app.layer_style_child.as_ref().filter(|(p, _)| *p == parent).map(|(_, c)| c)
}

/// Takes the child record of `parent`.
fn take_child(app: &mut PhotocraftApp, parent: DialogId) -> Option<Child> {
    if app.layer_style_child.as_ref().is_some_and(|(p, _)| *p == parent) { app.layer_style_child.take().map(|(_, c)| c) } else { None }
}

/// The Layer Style dialog `parent` closed: its child goes with it.
fn discard_child(app: &mut PhotocraftApp, parent: DialogId) {
    match take_child(app, parent) {
        Some(Child::Picker { id, .. }) => drop(app.framework.dialogs.take_by_id::<ColorPickerDialog>(id)),
        Some(Child::Editor { id, .. }) => drop(app.framework.dialogs.take_by_id::<ContourEditorDialog>(id)),
        None => {}
    }
}

fn fail(app: &mut PhotocraftApp, error: String) {
    app.ui.status = error;
    app.ui.status_error = true;
}

/// Once per pass, after the host ran: the dialog's result (OK applies the style) or, while it is
/// open, its asks and its children's answers.
pub(crate) fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some((id, doc, layer)) = app.layer_style_target else { return };
    if let Some(result) = app.framework.dialogs.take_by_id::<HostedLayerStyle>(id) {
        app.layer_style_target = None;
        app.style_preview = None;
        discard_child(app, id);
        if let DialogResult::Ok(style) = result {
            finish(app, doc, layer, &style);
        }
        return;
    }
    if app.framework.dialogs.get_by_id::<HostedLayerStyle>(id).is_none() {
        app.layer_style_target = None;
        return;
    }
    // The document or layer went away under the dialog: the cancel result is taken on the next pass.
    if !app.session.active().is_some_and(|s| s.doc.id == doc && s.doc.layer(layer).is_some()) {
        app.framework.dialogs.cancel_by_id(ctx, id);
        return;
    }
    let asks = app.framework.dialogs.get_mut_by_id::<HostedLayerStyle>(id).map(HostedLayerStyle::take_asks).unwrap_or_default();
    for ask in asks {
        match ask {
            LayerStyleAsk::PickColor(request) => {
                if let Err(error) = color_picker::open_request(app, id, request) {
                    fail(app, error);
                }
            }
            LayerStyleAsk::ApplyStyle(name) => apply_style_preset(app, id, &name),
            LayerStyleAsk::NewStyle => save_new_style(app, id, layer),
            LayerStyleAsk::MakeDefault(kind, params) => {
                // The stored defaults reach new instances from the next open and, through `set_defaults`, the open dialog.
                match app.run("layer.layerStyle.makeDefault", json!({"kind": kind.key(), "params": style_params_json(&params)})) {
                    Ok(_) => {
                        if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedLayerStyle>(id) {
                            dialog.set_defaults(kind, params);
                        }
                    }
                    Err(error) => fail(app, error),
                }
            }
            LayerStyleAsk::ResetToDefault(instance) => reset_to_default(app, id, instance),
            LayerStyleAsk::Contour(action) => contour::library_action(app, action),
            LayerStyleAsk::EditContour(request) => contour::open_editor(app, id, request),
            _ => {}
        }
    }
    color_picker::answered(app, id);
    contour::answered(app, id);
}

/// OK: replaces the layer's effects with the style's, as one history step.
fn finish(app: &mut PhotocraftApp, doc: photocraft_doc::DocId, layer: LayerId, style: &LayerStyle) {
    let Some(light) = app.session.active().filter(|s| s.doc.id == doc).map(|s| f64::from(s.doc.global_light.angle)) else {
        return fail(app, "Layer Style target document changed".into());
    };
    if let Err(error) = apply(style, layer, Some(light), |id, p| app.run(id, p)) {
        fail(app, error);
    }
}

/// Applies a style preset to the draft without touching the document: Cancel leaves the document
/// untouched, OK commits.
fn apply_style_preset(app: &mut PhotocraftApp, dialog: DialogId, name: &str) {
    let Ok(style) = photocraft_engine::presets::styles::find_style(&app.session, &json!({"preset": name}), "apply_style_preset") else { return };
    let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedLayerStyle>(dialog) else { return };
    let light = dialog.draft().style.global_light;
    let effects =
        style.effects.iter().filter_map(|e| EffectKind::from_key(kind_of(e)).map(|kind| (kind, e.enabled(), effect_params(kind, e, light)))).collect();
    dialog.draft_mut().apply_preset(effects);
    let b = &mut dialog.draft_mut().style.blending;
    if let Some(blend) = style.blend {
        b.blend = blend.label().to_string();
    }
    if let Some(fo) = style.fill_opacity {
        b.fill_opacity = (fo * 100.0).round();
    }
    if let Some(o) = style.opacity {
        b.opacity = (o * 100.0).round();
    }
}

/// Saves the draft's enabled instances and blending page as a style preset.
fn save_new_style(app: &mut PhotocraftApp, dialog: DialogId, layer: LayerId) {
    let Some(style) = app.framework.dialogs.get_by_id::<HostedLayerStyle>(dialog).map(|d| d.draft().style.clone()) else { return };
    let entries: Vec<Value> =
        style.effects.iter().filter(|e| e.on).map(|e| json!([e.kind.key(), with_defaults(e.kind.key(), &style_params_json(&e.params))])).collect();
    let b = &style.blending;
    let params = json!({"layer": layer.0, "effects": entries, "blend": b.blend, "fillOpacity": b.fill_opacity, "opacity": b.opacity});
    if let Ok(v) = app.run("style.presets.new", params) {
        app.ui.status = crate::i18n::fmt(tl!("Style {name} saved"), &[("name", v["name"].as_str().unwrap_or("Style"))]);
    }
}

/// Writes the user's default (or the factory defaults before the first Make Default) into the instance.
/// An effect lit by the shared light takes the dialog's light angle, not the default's own `angle`
/// (120 for the factory): the same seeding [`fresh_params`] and the draft's `add_instance` apply to a
/// new instance, so Reset to Default leaves a globally lit effect at the shared angle (#14843).
fn reset_to_default(app: &mut PhotocraftApp, dialog: DialogId, instance: EffectId) {
    let Some((kind, light)) = app
        .framework
        .dialogs
        .get_by_id::<HostedLayerStyle>(dialog)
        .and_then(|d| d.draft().style.effect(instance).map(|e| (e.kind, f64::from(d.draft().style.global_light))))
    else {
        return;
    };
    let Ok(v) = app.run("layer.layerStyle.defaultFor", json!({"kind": kind.key()})) else { return };
    let mut values = v["params"].clone();
    drop_from_to(&mut values);
    if values.get("useGlobalLight").and_then(Value::as_bool) == Some(true)
        && let Some(o) = values.as_object_mut()
    {
        o.insert("angle".into(), json!(light));
    }
    let params = style_params(kind, &values, None);
    if let Some(d) = app.framework.dialogs.get_mut_by_id::<HostedLayerStyle>(dialog) {
        for (key, value) in params.0 {
            d.draft_mut().set_param(instance, &key, value);
        }
    }
}

// ------------------------------------------------------------------ canvas preview

/// The style the canvas shows for document `idx` with its memo key: the draft's style (an open
/// picker's colour laid over it) while Preview is on; `None` for another document, with Preview off
/// or while the box is held to compare. Key: the style's render hash mixed with the revision.
fn preview_source(app: &PhotocraftApp, idx: usize) -> Option<(LayerStyle, LayerId, u64)> {
    let (id, doc, layer) = app.layer_style_target?;
    if app.session.active_index() != Some(idx) {
        return None;
    }
    let st = app.session.documents().get(idx)?;
    if st.doc.id != doc {
        return None;
    }
    let dialog = app.framework.dialogs.get_by_id::<HostedLayerStyle>(id)?;
    if !dialog.draft().preview || dialog.is_comparing() {
        return None;
    }
    let style = color_picker::preview_style(app)?;
    let key = (style.render_hash() ^ st.revision.wrapping_mul(0x9e37_79b9_7f4a_7c15)) | 1 << 63;
    Some((style, layer, key))
}

/// The active document with `style` applied, run on a scratch session (no history).
fn build_preview(app: &PhotocraftApp, style: &LayerStyle, layer: LayerId) -> Result<Document, String> {
    let st = app.session.active().ok_or("no active document")?;
    let light = f64::from(st.doc.global_light.angle);
    let mut s = Session::new();
    s.patterns = app.session.patterns.clone();
    s.add_document((*st.doc).clone(), None);
    apply(style, layer, Some(light), |id, p| s.execute(id, p).map_err(|e| e.to_string()))?;
    s.active().map(|d| (*d.doc).clone()).ok_or_else(|| "no preview document".into())
}

#[cfg(test)]
/// The preview of the open dialog's style on the active document, built afresh, with its key.
pub(crate) fn preview_document(app: &PhotocraftApp) -> Option<(Arc<Document>, u64)> {
    let (style, layer, key) = preview_source(app, app.session.active_index()?)?;
    build_preview(app, &style, layer).ok().map(|doc| (Arc::new(doc), key))
}

/// The Layer Style step of `canvas::display_doc`: the preview for document `idx`, built once per
/// key into `app.style_preview` (the only writer); `None` shows the document.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let Some((style, layer, key)) = preview_source(app, idx) else {
        app.style_preview = None;
        return None;
    };
    if app.style_preview.as_ref().map(|p| p.0) != Some(key) {
        let shown = build_preview(app, &style, layer).map(Arc::new);
        if let Err(error) = &shown {
            fail(app, error.clone());
        }
        app.style_preview = Some((key, shown));
    }
    app.style_preview.as_ref()?.1.as_ref().ok().map(|doc| (doc.clone(), key))
}

/// The dialog's style as a preset, built on a scratch layer the way the canvas preview builds it on
/// the real one (instance snapshots, blending options and patterns all apply).
pub(crate) fn pending_style(patterns: &PatternLibrary, style: &LayerStyle) -> Option<StylePreset> {
    let mut s = Session::new();
    s.patterns = patterns.clone();
    s.execute("file.new", json!({"width": 8, "height": 8})).ok()?;
    let layer = LayerId(s.execute("layer.new.layer", json!({})).ok()?.get("layer")?.as_u64()?);
    apply(style, layer, None, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).ok()?;
    let l = s.active()?.doc.layer(layer)?;
    Some(StylePreset {
        name: String::new(),
        effects: l.effects.items.clone(),
        blend: Some(l.blend),
        opacity: Some(l.opacity),
        fill_opacity: Some(l.fill_opacity),
    })
}

/// The width and height of the layer a Gradient Overlay's offset is a fraction of
/// (`photocraft_compose::layer_bounds`, what the renderer paints against): `None` once that layer
/// is gone or while it is empty (a gradient paints nothing there), when a drag on the image has
/// nothing to move and the Hand tool pans.
pub(crate) fn gradient_layer_bounds(session: &Session, layer: LayerId) -> Option<[f64; 2]> {
    let st = session.active()?;
    let b = photocraft_compose::layer_bounds(st.doc.layer(layer)?, st.doc.bounds());
    let [w, h] = [f64::from(b.width()), f64::from(b.height())];
    (w > 0.0 && h > 0.0).then_some([w, h])
}

#[cfg(test)]
mod tests;
