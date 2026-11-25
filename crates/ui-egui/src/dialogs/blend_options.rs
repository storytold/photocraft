//! The Blending rows that Fill, Stroke and the Smart Filter Blending Options dialog share: the mode list is the
//! Layers panel's, so a mode has one key and one shortcut everywhere. Fill, Stroke and Fade grey the modes the active
//! document's colour mode and depth can't use (#13806, #14647).

use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamKind};
use craft::ui::runtime::env::Label;
use photocraft_color::BlendMode;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

/// A blend-mode row `key` over the Layers panel's mode list (`group` adds Pass Through), opening on `current`.
pub(crate) fn blend_param(key: &str, label: impl Into<Label>, group: bool, current: BlendMode) -> Param {
    let modes = crate::panels::blend_options(group, current)
        .into_iter()
        .map(|(m, name)| (crate::panels::blend_entry(m.key().to_string(), m), Label::from(name)))
        .collect();
    Param::new(key, label, ParamKind::blend(modes, Some(current.key().to_string())))
}

/// A Mode row `"mode"`, labelled `label`, opening on Normal, with the modes the active document can't use greyed
/// (listed, never chosen). A remembered mode that is greyed opens as Normal: the framework drops a saved value that
/// names a disabled option.
pub(crate) fn document_mode_param(app: &PhotocraftApp, label: impl Into<Label>) -> Param {
    let greyed = app.session.active().map_or(&[][..], |st| photocraft_engine::fill_cmds::greyed_modes(st.doc.mode, st.doc.depth));
    blend_param("mode", label, false, BlendMode::Normal).disabled_options(greyed.iter().map(|m| m.key().to_string()))
}

/// The mode a blend row `key` holds; Normal when the field is missing or unreadable.
pub(crate) fn blend_mode(fields: &DialogFields, key: &str) -> BlendMode {
    fields.text(key).and_then(photocraft_engine::commands::parse_blend).unwrap_or(BlendMode::Normal)
}

/// The Blending section, Mode, Opacity (percent) and Preserve Transparency: the rows Fill and Stroke share.
pub(crate) fn blending_rows(app: &PhotocraftApp) -> Vec<Param> {
    vec![
        Param::section(tl!("Blending")),
        document_mode_param(app, tl!("Mode:")),
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

#[cfg(test)]
mod document_modes_tests;
