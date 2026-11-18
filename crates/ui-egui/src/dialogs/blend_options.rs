//! The Blending rows that Fill, Stroke and the Smart Filter Blending Options dialog share: the mode list is the
//! Layers panel's, so a mode has one key and one shortcut everywhere.

use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamKind};
use craft::ui::runtime::env::Label;
use photocraft_color::BlendMode;
use serde_json::{Map, Value, json};

use crate::panels::layers_tree::blend_key;

/// A blend-mode row `key` over the Layers panel's mode list (`group` adds Pass Through), opening on `current`.
pub(crate) fn blend_param(key: &str, label: impl Into<Label>, group: bool, current: BlendMode) -> Param {
    let modes =
        crate::panels::blend_options(group, current).into_iter().map(|(m, name)| (crate::panels::blend_entry(blend_key(m), m), Label::from(name))).collect();
    Param::new(key, label, ParamKind::blend(modes, Some(blend_key(current))))
}

/// The mode a blend row `key` holds; Normal when the field is missing or unreadable.
pub(crate) fn blend_mode(fields: &DialogFields, key: &str) -> BlendMode {
    fields.text(key).and_then(photocraft_engine::commands::blend_from_str).unwrap_or(BlendMode::Normal)
}

/// The Blending section, Mode, Opacity (percent) and Preserve Transparency: the rows Fill and Stroke share.
pub(crate) fn blending_rows() -> Vec<Param> {
    vec![
        Param::section(tl!("Blending")),
        blend_param("mode", tl!("Mode:"), false, BlendMode::Normal),
        super::new_layer_ui::opacity_param(),
        Param::new("preserveTransparency", tl!("Preserve Transparency"), ParamKind::bool(false)),
    ]
}

/// The engine params of [`blending_rows`].
pub(crate) fn blending_params(fields: &DialogFields) -> Map<String, Value> {
    let mut p = Map::new();
    if let Some(mode) = fields.text("mode") {
        p.insert("mode".into(), json!(mode));
    }
    if let Some(opacity) = fields.number("opacity") {
        p.insert("opacity".into(), json!(opacity));
    }
    if let Some(on) = fields.bool("preserveTransparency") {
        p.insert("preserveTransparency".into(), json!(on));
    }
    p
}
