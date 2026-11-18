//! The Layers panel filter's control channel and the app's side of it. craft-layers' `LayerFilter`
//! (kept in `UiState::layer_filter`) draws the filter row and decides what each type lists; this
//! module reads `ui.set {layerFilter}` into a `LayerFilter` ([`patched`]) and applies a filter to
//! the panel with the selection change Photoshop makes ([`apply`], [`deselect_hidden`]). The types
//! are Kind, Name, Effect (#10880), Mode, Attribute (#10881), Color (#5898), Smart Object,
//! Artboard and Group (#10883).

use craft::layers::ui::filter::{EffectKind, FilterBy, FilterKind, LayerAttribute, LayerFilter, SmartKind};
use photocraft_color::BlendMode;
use photocraft_doc::LabelColor;
use serde::Serialize;
use serde_json::Value;

use crate::PhotocraftApp;

/// The fields `ui.set {layerFilter}` takes.
const FIELDS: [&str; 8] = ["by", "kinds", "name", "effect", "mode", "attribute", "color", "smart"];

/// The control-channel id of `v`: its serde name.
fn id_of<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// The one of `all` that `v` names by id, or an error listing the ids.
fn choose<T: Serialize + Copy>(field: &str, what: &str, all: &[T], v: &Value) -> Result<T, String> {
    let ids = || all.iter().map(id_of).collect::<Vec<_>>().join(", ");
    let name = v.as_str().ok_or_else(|| format!("layerFilter.{field} must be one of {}", ids()))?;
    all.iter().copied().find(|t| id_of(t) == name).ok_or_else(|| format!("layerFilter.{field}: unknown {what} `{name}` (one of: {})", ids()))
}

/// The toggles of `all` that the list `v` names by `name`, each once, in the order given.
fn toggles<T: Copy + PartialEq>(field: &str, v: &Value, all: &[T], name: impl Fn(T) -> String) -> Result<Vec<T>, String> {
    let list = v.as_array().ok_or_else(|| format!("layerFilter.{field} must be a list"))?;
    let mut out = Vec::new();
    for k in list {
        let Some(t) = k.as_str().and_then(|k| all.iter().copied().find(|t| name(*t) == k)) else {
            let names = all.iter().map(|t| name(*t)).collect::<Vec<_>>().join(", ");
            return Err(format!("layerFilter.{field}: unknown toggle {k} (toggles: {names})"));
        };
        if !out.contains(&t) {
            out.push(t);
        }
    }
    Ok(out)
}

/// `ui.set {layerFilter: {by?, kinds?, name?, effect?, mode?, attribute?, color?, smart?}}`:
/// `current` with the given fields replaced, or an error naming the bad field (nothing changes
/// then). `by` is a [`FilterBy`] id; `kinds` a list of [`FilterKind::name`]s; `effect`, `attribute`
/// and `smart` ids of [`EffectKind`], [`LayerAttribute`] and [`SmartKind`]; `mode` one of the layer
/// blend modes such as `Multiply` (not Pass Through, which the Mode list doesn't offer, #10878);
/// `color` a label colour id (`none`, `red`, … `gray`, [`LabelColor::id`]). Artboard has no
/// criteria of its own: the selected layers pick the artboards; nor has Group.
pub fn patched(current: &LayerFilter, v: &Value) -> Result<LayerFilter, String> {
    let Some(o) = v.as_object() else { return Err("layerFilter must be an object".into()) };
    if let Some(k) = o.keys().find(|k| !FIELDS.contains(&k.as_str())) {
        return Err(format!("unknown layerFilter field `{k}` (fields: {})", FIELDS.join(", ")));
    }
    let mut f = current.clone();
    if let Some(by) = o.get("by") {
        f.by = choose::<FilterBy>("by", "type", &FilterBy::ALL, by)?;
    }
    if let Some(kinds) = o.get("kinds") {
        f.kinds = toggles("kinds", kinds, &FilterKind::ALL, |k: FilterKind| k.name().to_string())?;
    }
    if let Some(name) = o.get("name") {
        f.name = name.as_str().ok_or("layerFilter.name must be a string")?.to_string();
    }
    if let Some(mode) = o.get("mode") {
        let name = mode.as_str().ok_or("layerFilter.mode must be a blend mode name")?;
        let m = photocraft_engine::commands::blend_from_str(name).filter(|m| BlendMode::LAYER_MODES.contains(m)).ok_or_else(|| {
            let modes: Vec<&str> = BlendMode::LAYER_MODES.iter().map(|m| m.label()).collect();
            format!("layerFilter.mode: `{name}` is not a layer blend mode the filter lists (modes: {})", modes.join(", "))
        })?;
        f.mode = super::layers_tree::blend_key(m);
    }
    if let Some(e) = o.get("effect") {
        f.effect = choose::<EffectKind>("effect", "effect", &EffectKind::ALL, e)?;
    }
    if let Some(a) = o.get("attribute") {
        f.attribute = choose::<LayerAttribute>("attribute", "attribute", &LayerAttribute::ALL, a)?;
    }
    if let Some(color) = o.get("color") {
        let id = color.as_str().ok_or("layerFilter.color must be a label colour id")?;
        let c = LabelColor::ALL.into_iter().find(|c| c.id() == id).ok_or_else(|| {
            let ids: Vec<&str> = LabelColor::ALL.iter().map(|c| c.id()).collect();
            format!("layerFilter.color: unknown colour `{id}` (colours: {})", ids.join(", "))
        })?;
        f.color = super::layers_tree::ui_label(c);
    }
    if let Some(smart) = o.get("smart") {
        f.smart = toggles("smart", smart, &SmartKind::ALL, |k: SmartKind| id_of(&k))?;
    }
    Ok(f)
}

/// Store `filter` as the Layers panel's filter; a change deselects what it hides
/// ([`deselect_hidden`]), an unchanged filter does nothing.
pub fn apply(app: &mut PhotocraftApp, filter: LayerFilter) {
    let before = std::mem::replace(&mut app.ui.layer_filter, filter);
    deselect_hidden(app, &before);
}

/// When the panel's filter is no longer `before`, deselect the layers it hides, as the panel's
/// own filter row does (#10879): the listed ones stay selected, one of them active, and a Smart
/// Filter row selected on its own stays so only while it is listed. Like any layer selection this
/// is no history step.
pub(crate) fn deselect_hidden(app: &mut PhotocraftApp, before: &LayerFilter) {
    let filter = app.ui.layer_filter.clone();
    if filter == *before {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let doc = st.doc.id;
    let picked = crate::smart_ui::selected(&app.smart_filter_picks, &st.doc, st.active_layer);
    let pick_hidden = picked.is_some() && super::layers_tree::PhotoLayers::new(app, st, &filter, false, picked).selected_filter().is_none();
    let select = filter.deselect_hidden(&super::layers_tree::PhotoLayers::new(app, st, &filter, false, None));
    if pick_hidden {
        crate::smart_ui::deselect(&mut app.smart_filter_picks, doc);
    }
    let Some(select) = select else { return };
    let Some(st) = app.session.active() else { return };
    let mut actions = Vec::new();
    super::layers_events::selection_actions(select.change, &st.selected_layers(), &mut actions);
    for (id, params) in actions {
        let _ = app.run(&id, params);
    }
}

pub mod find;

pub mod attribute;

#[cfg(test)]
mod attribute_tests;
#[cfg(test)]
mod find_tests;
#[cfg(test)]
mod selection_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod kind_mode_tests;
