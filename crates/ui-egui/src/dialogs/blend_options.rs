//! The Blending rows that Fill, Stroke and the Smart Filter Blending Options dialog share: the mode list is the
//! Layers panel's, so a mode has one key and one shortcut everywhere. Fill, Stroke and Fade grey the modes the active
//! document's colour mode and depth can't use (#13806, #14647).

use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamKind, ParamResult};
use craft::ui::runtime::env::{Label, UnitKind};
use craft::ui::runtime::memory::{MemoryStage, SaveKey, saved_value, set_saved_value};
use photocraft_color::BlendMode;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

const PRESERVE: &str = "preserveTransparency";

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

/// The Blending section, Mode, Opacity (percent) and Preserve Transparency: the rows Fill and Stroke share. A
/// `forced` Preserve Transparency ([`forced_preserve_transparency`]) shows greyed at that state and is not remembered.
pub(crate) fn blending_rows(app: &PhotocraftApp, forced: Option<bool>) -> Vec<Param> {
    let mut preserve = Param::new(PRESERVE, tl!("Preserve Transparency"), ParamKind::bool(forced.unwrap_or(false)));
    if forced.is_some() {
        // No value enables it, so neither the user nor automation can change the target's state.
        preserve = preserve.enabled_when_in(PRESERVE, Vec::<FieldValue>::new()).not_remembered();
    }
    vec![
        Param::section(tl!("Blending")),
        document_mode_param(app, tl!("Mode:")),
        // Whole percents from 1 (New Layer's row starts at 0): a typed 0 is clamped to 1 with the range alert (#14649).
        Param::new("opacity", tl!("Opacity:"), ParamKind::number(Some(1.0), Some(100.0), 100.0, 1.0, 0)).unit(UnitKind::Percent),
        preserve,
    ]
}

/// The Preserve Transparency that Fill's or Stroke's `target` (as `edit.fill` takes it) forces: off on the Background
/// layer or other pixels without transparency, on with Lock Transparent Pixels. `None` for other layers, masks and
/// channels, where the box offers the remembered choice.
pub(crate) fn forced_preserve_transparency(app: &PhotocraftApp, target: &Value) -> Option<bool> {
    let st = app.session.active()?;
    let id = st.active_layer?;
    if photocraft_engine::channel_cmds::gray_target(&json!({ "target": target })).is_some() {
        return None;
    }
    // The Background also carries the transparency lock, so it is tested first.
    if st.doc.is_background(id) || st.doc.layer(id)?.surface().is_some_and(|s| !s.format().alpha) {
        return Some(false);
    }
    st.doc.effective_locks(id).transparency.then_some(true)
}

/// Remembers the choices of `result`. A forced Preserve Transparency is not among them, so the choice remembered
/// before is kept for the next open.
pub(crate) fn remember_blending(ctx: &egui::Context, result: &ParamResult) {
    let key = SaveKey::named(super::params::memory_scope(&result.key));
    let before = saved_value::<Option<DialogFields>>(ctx, &key).and_then(|saved| saved.get(PRESERVE).cloned());
    result.remember(ctx);
    if let Some(before) = before
        && let Some(mut saved) = saved_value::<Option<DialogFields>>(ctx, &key)
        && !saved.contains(PRESERVE)
    {
        saved.set(PRESERVE, before);
        set_saved_value(ctx, &key, Some(saved));
    }
}

/// The opening fields of the dialog `key` (Fill or Stroke) over [`blending_rows`]: Opacity 1 when the remembered
/// Opacity is below the row's 1% minimum, empty otherwise. The framework drops a remembered value the row can't hold,
/// so a 0% saved before the row started at 1 would reopen at the 100% default, a full-strength edit (#14649).
pub(crate) fn opening_fields(ctx: &egui::Context, key: &str) -> DialogFields {
    let saved: Option<DialogFields> = MemoryStage::peek_saved(ctx, &SaveKey::named(super::params::memory_scope(key)));
    let mut fields = DialogFields::new();
    if saved.and_then(|saved| saved.number("opacity")).is_some_and(|opacity| opacity < 1.0) {
        fields.set("opacity", 1.0);
    }
    fields
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
