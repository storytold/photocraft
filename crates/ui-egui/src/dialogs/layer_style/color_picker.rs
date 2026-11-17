//! The Color Picker child of the Layer Style dialog: one picker over one colour of the draft, named
//! by the key the dialog's colour request carries (`"fx2/color"`, or `"fx2/stops/1"` for colour stop 1 of
//! a gradient).

use craft::color_ui::ui::{ColorPickerDialog, ColorPickerResult};
use craft::imaging::layer_style::{EffectId, EffectSpec, FieldKind, LayerStyle, LayerStyleDraft, StyleValue};
use craft::imaging::ui::layer_style::{HostedLayerStyle, LayerStyleDialog};
use craft::theme::Srgba;
use craft::ui::dialog::{DialogId, DialogResult};
use craft::ui::runtime::requests::ColorRequest;

use super::{Child, child_of, take_child};
use crate::PhotocraftApp;

/// The effect, field and colour stop a request key names.
fn parse(key: &str) -> Result<(&str, &str, Option<usize>), String> {
    let mut parts = key.split('/');
    let (Some(effect), Some(field)) = (parts.next(), parts.next()) else { return Err("invalid colour key".into()) };
    let stop = match parts.next() {
        None => None,
        Some(i) => Some(i.parse::<usize>().map_err(|_| "invalid gradient stop")?),
    };
    Ok((effect, field, stop))
}

/// The colour the draft of the dialog `parent` holds at `key`; refuses a stale dialog or effect, an
/// unknown field and a field that is not a colour (or, with a stop, not a gradient).
fn color_at(app: &PhotocraftApp, parent: DialogId, key: &str) -> Result<Srgba, String> {
    let (effect, field, stop) = parse(key)?;
    let dialog = app.framework.dialogs.get_by_id::<HostedLayerStyle>(parent).ok_or("no Layer Style dialog")?;
    let id = EffectId::from_key(effect).ok_or("Layer Style effect no longer exists")?;
    let instance = dialog.draft().style.effect(id).ok_or("Layer Style effect no longer exists")?;
    let spec = EffectSpec::photoshop(instance.kind);
    let kind = &spec.field(field).ok_or("unknown Layer Style field")?.kind;
    let shown = instance.params.over(&spec.defaults);
    match (stop, kind, shown.get(field)) {
        (None, FieldKind::Color, Some(StyleValue::Color(c))) => Ok(*c),
        (None, ..) => Err("not a Layer Style color parameter".into()),
        (Some(i), FieldKind::Gradient, Some(StyleValue::Gradient(g))) => g.color.get(i).map(|s| s.color).ok_or_else(|| "no such gradient stop".into()),
        (Some(_), ..) => Err("not a Layer Style gradient parameter".into()),
    }
}

/// Opens the picker a request of the dialog `parent` asked for, as a child of it.
pub(super) fn open_request(app: &mut PhotocraftApp, parent: DialogId, request: ColorRequest) -> Result<DialogId, String> {
    if child_of(app, parent).is_some() {
        return Err("a Layer Style child dialog is already open".into());
    }
    let opening = color_at(app, parent, &request.key)?;
    let key = request.key.clone();
    // An instance keeps the picker's result apart from the other pickers' (`take_by_id` only).
    let picker = ColorPickerDialog::from_request(request).instance(parent);
    let id = app.framework.dialogs.open_child(picker, parent);
    app.layer_style_child = Some((parent, Child::Picker { id, key, opening }));
    Ok(id)
}

/// Opens the picker on `effect`'s `field` (and colour `stop` of a gradient) for the open Layer Style
/// dialog whose id value is `parent`: the control protocol's `colorPicker`.
pub(crate) fn open(app: &mut PhotocraftApp, parent: u64, effect: &str, field: &str, stop: Option<usize>) -> Result<DialogId, String> {
    let (id, ..) = app.layer_style_target.filter(|(id, ..)| id.value() == parent).ok_or("no Layer Style dialog")?;
    let key = match stop {
        Some(i) => format!("{effect}/{field}/{i}"),
        None => format!("{effect}/{field}"),
    };
    let color = color_at(app, id, &key)?;
    open_request(app, id, ColorRequest::new(key, color).title(tl!("Color Picker (Layer Style Color)").to_string()))
}

/// The draft's style with the open picker's colour laid over it, for the canvas preview only: Cancel
/// needs no rollback. `None` when the dialog is closed.
pub(super) fn preview_style(app: &PhotocraftApp) -> Option<LayerStyle> {
    let (parent, ..) = app.layer_style_target?;
    let style = app.framework.dialogs.get_by_id::<HostedLayerStyle>(parent)?.draft().style.clone();
    let Some(Child::Picker { id, key, opening }) = child_of(app, parent) else { return Some(style) };
    let Some(current) = app.framework.dialogs.get_by_id::<ColorPickerDialog>(*id).and_then(ColorPickerDialog::current) else { return Some(style) };
    if current.color == *opening {
        return Some(style);
    }
    let mut draft = LayerStyleDraft::new(style);
    LayerStyleDialog::set_color(&mut draft, key, current.color);
    Some(draft.style)
}

/// Once per pass: while the picker is open nothing is written (the preview reads it); when it is
/// gone, OK sets the draft's colour and Cancel leaves it.
pub(super) fn answered(app: &mut PhotocraftApp, parent: DialogId) {
    let Some(Child::Picker { id, opening, .. }) = child_of(app, parent) else { return };
    let (id, opening) = (*id, *opening);
    if app.framework.dialogs.get_by_id::<ColorPickerDialog>(id).is_some() {
        return;
    }
    take_child(app, parent);
    // An OK with the colour it opened with writes nothing: a parameter the effect lacks stays missing.
    if let Some(DialogResult::Ok(ColorPickerResult { key, picked, .. })) = app.framework.dialogs.take_by_id::<ColorPickerDialog>(id)
        && picked.color != opening
        && let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedLayerStyle>(parent)
    {
        dialog.set_color(&key, picked.color);
    }
}

#[cfg(test)]
mod tests;
