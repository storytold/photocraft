use craft::imaging::layer_style::{
    ChoiceOption, EffectField, EffectKind, EffectSpec, FieldKind, LayerStyle, LayerStyleDraft, LayerStylePage, StyleParams, StyleValue,
};
use craft::imaging::ui::layer_style::HostedLayerStyle;
use craft::ui::dialog::DialogId;
use photocraft_doc::Effect;
use photocraft_doc::effects::{FxPaint, Gradient, GradientStyle};
use serde_json::{Map, Value, json};

use super::{fx_of, gradient_field, specs, values_of};
use crate::PhotocraftApp;

pub(super) const KEY: &str = "fillType";
const GRADIENT: &str = "gradient";
const COLOR: &str = "color";
const PATTERN: &str = "pattern";
const GRADIENT_FIELDS: &[&str] = &["stops", "reverse", "style", "angle", "scale"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Fields {
    gradient: bool,
    pattern: bool,
    imported_pattern: bool,
}

impl Fields {
    pub(super) fn of(draft: &LayerStyleDraft) -> Option<Self> {
        let LayerStylePage::Effect(id) = draft.page else { return None };
        let e = draft.style.effect(id).filter(|e| e.kind == EffectKind::Stroke)?;
        let imported_pattern = fx_of(&e.params)
            .and_then(|fx| serde_json::from_value::<Effect>(fx).ok())
            .is_some_and(|e| matches!(e, Effect::Stroke(s) if matches!(s.paint, FxPaint::Pattern { .. })));
        let fill = fill_type(&e.params);
        Some(Self { gradient: fill == GRADIENT, pattern: fill == PATTERN, imported_pattern })
    }

    pub(super) fn configure(self, spec: &mut EffectSpec) {
        let color = spec.fields.iter().find(|field| field.key == "color").cloned();
        spec.fields.retain(|field| field.key != "color");
        let mut options = vec![
            ChoiceOption { value: COLOR.into(), label: tl!("Color").to_string().into() },
            ChoiceOption { value: GRADIENT.into(), label: tl!("Gradient").to_string().into() },
        ];
        if self.imported_pattern {
            options.push(ChoiceOption { value: PATTERN.into(), label: tl!("Pattern").to_string().into() });
        }
        spec.fields.push(EffectField { key: KEY.into(), label: tl!("Fill Type").to_string().into(), kind: FieldKind::Choice(options.into()) });
        if self.gradient {
            let gradient = EffectSpec::photoshop(EffectKind::GradientOverlay);
            spec.fields.extend(gradient.fields.into_iter().filter(|f| GRADIENT_FIELDS.contains(&f.key.as_ref())));
        } else if !self.pattern {
            spec.fields.extend(color);
        }
    }
}

pub(super) fn fill_type(params: &StyleParams) -> &str {
    match params.get(KEY) {
        Some(StyleValue::Text(fill)) => fill,
        _ => COLOR,
    }
}

pub(super) fn picker_kind(params: &StyleParams, field: &str) -> Option<EffectKind> {
    match (fill_type(params), field) {
        (GRADIENT, gradient_field::KEY) => Some(EffectKind::GradientOverlay),
        (COLOR, "color") => Some(EffectKind::Stroke),
        _ => None,
    }
}

pub(super) fn configure(specs: &mut [EffectSpec], fields: Fields) {
    if let Some(spec) = specs.iter_mut().find(|spec| spec.kind == EffectKind::Stroke) {
        fields.configure(spec);
    }
}

pub(super) fn defaults(values: &mut Value) {
    values[KEY] = json!(COLOR);
    gradient_values(values, &Gradient::default());
}

pub(super) fn load_default(values: &mut Value, user: &Value) {
    // Older automation could save a Stroke default with endpoints or stops and no Fill Type.
    if user.get(KEY).is_none()
        && (user.get("from").is_some() || user.get(gradient_field::KEY).is_some())
        && let Some(Effect::Stroke(stroke)) = photocraft_engine::layer_style::effect_from_params("stroke", user)
    {
        paint_values(values, &stroke.paint);
    }
    if let Some(values) = values.as_object_mut() {
        for key in ["from", "to", "method", "smoothness"] {
            values.remove(key);
        }
    }
}

pub(super) fn paint_values(values: &mut Value, paint: &FxPaint) {
    values[KEY] = match paint {
        FxPaint::Color(c) => {
            values["color"] = json!(super::hex(c));
            json!(COLOR)
        }
        FxPaint::Gradient(g) => {
            gradient_values(values, g);
            json!(GRADIENT)
        }
        FxPaint::Pattern { .. } => json!(PATTERN),
    };
}

fn gradient_values(values: &mut Value, g: &Gradient) {
    let (stops, opacity) = gradient_field::stops_to_json(&gradient_field::stops_of_doc(&g.stops, &g.opacity_stops));
    values[gradient_field::KEY] = stops;
    values[gradient_field::OPACITY_KEY] = opacity;
    values["reverse"] = json!(g.reverse);
    values["style"] = json!(match g.style {
        GradientStyle::Linear => "linear",
        GradientStyle::Radial => "radial",
        GradientStyle::Angle => "angle",
        GradientStyle::Reflected => "reflected",
        GradientStyle::Diamond => "diamond",
    });
    values["angle"] = json!(g.angle);
    values["scale"] = json!((g.scale * 100.0).round());
}

pub(super) fn is_imported_pattern(params: &Value) -> bool {
    params[KEY].as_str() == Some(PATTERN)
}

pub(super) fn apply_params(params: &mut Value, snapshot: Option<&Value>) {
    let gradient = params[KEY].as_str() == Some(GRADIENT);
    let pattern = is_imported_pattern(params);
    let opened = snapshot.and_then(|fx| serde_json::from_value::<Effect>(fx.clone()).ok());
    let shown = opened.as_ref().map(|e| values_of(e, 0.0));
    let Some(edits) = params.as_object_mut() else { return };
    if let Some(shown) = &shown {
        for key in ["opacity", "blend", "size", "position", "overprint"] {
            if edits.get(key) == shown.get(key) {
                edits.remove(key);
            }
        }
    }
    if gradient {
        edits.remove("color");
        if let Some(shown) = shown.as_ref().filter(|shown| shown[KEY].as_str() == Some(GRADIENT)) {
            // Compare colour and opacity independently so an edit to either list leaves the other
            // in its original colour space and precision. Geometry can also be outside UI limits.
            let current = gradient_field::stops_from_json(&Value::Object(edits.clone()));
            let opening = gradient_field::stops_from_json(shown);
            let lists = current.as_ref().zip(opening.as_ref());
            for (key, same) in [
                (gradient_field::KEY, lists.is_some_and(|(a, b)| a.color == b.color)),
                (gradient_field::OPACITY_KEY, lists.is_some_and(|(a, b)| a.opacity == b.opacity)),
            ] {
                if same || edits.get(key) == shown.get(key) {
                    edits.remove(key);
                }
            }
            for key in ["reverse", "style", "angle", "scale"] {
                if edits.get(key) == shown.get(key) {
                    edits.remove(key);
                }
            }
        }
    } else {
        edits.remove(KEY);
        for key in GRADIENT_FIELDS.iter().copied().chain([gradient_field::OPACITY_KEY]) {
            edits.remove(key);
        }
        if pattern || shown.as_ref().is_some_and(|s| s[KEY].as_str() == Some(COLOR) && edits.get("color") == s.get("color")) {
            edits.remove("color");
        }
    }
}

fn fields_cache(id: DialogId) -> egui::Id {
    egui::Id::new(("layer-style-stroke-fill", id.value()))
}

pub(super) fn remember(ctx: &egui::Context, id: DialogId, fields: Fields) {
    ctx.data_mut(|data| data.insert_temp(fields_cache(id), fields));
}

pub(super) fn refresh(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId) {
    let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedLayerStyle>(id) else { return };
    let Some(fields) = Fields::of(dialog.draft()) else { return };
    if ctx.data(|data| data.get_temp::<Fields>(fields_cache(id))) == Some(fields) {
        return;
    }
    let light = dialog.draft().style.global_light;
    let user: Map<String, Value> = app.session.presets().layer_defaults.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let mut specs = specs(&super::blend_keys(), &user, light);
    configure(&mut specs, fields);
    // The shared host's field-spec setter takes ownership. Move only the host's configuration;
    // its dialog identity, draft, pending children and history all remain on the existing instance.
    let placeholder = HostedLayerStyle::new("", LayerStyleDraft::new(LayerStyle::default()), Vec::new());
    *dialog = std::mem::replace(dialog, placeholder).specs(specs);
    remember(ctx, id, fields);
    ctx.request_repaint();
}
