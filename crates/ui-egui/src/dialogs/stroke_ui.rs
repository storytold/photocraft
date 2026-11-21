//! Edit › Stroke…: a Photoshop-style options dialog for the existing `edit.stroke` command.
//! The engine remains directly callable with parameters by CLI, MCP and Actions.

use std::borrow::Cow;

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::{Label, UnitKind};
use craft::ui::runtime::memory::SaveKey;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::blend_options::{blending_params, blending_rows};
use crate::dialogs::fill_ui::hex;
use crate::dialogs::params::{captured, open_param};

/// The command of the dialog, which is also its dialog key.
pub const COMMAND: &str = "edit.stroke";
const REMEMBER: &str = "photocraft.dialog.edit.stroke";
const LOCATIONS: [(&str, &str); 3] = [("inside", "Inside"), ("center", "Center"), ("outside", "Outside")];

/// The dialog's rows.
fn rows(foreground: [f32; 3]) -> Vec<Param> {
    let locations = LOCATIONS.iter().map(|(k, l)| (k.to_string(), Label::from(tl!(l)))).collect();
    let mut rows = vec![
        Param::new("width", tl!("Width:"), ParamKind::number(Some(1.0), Some(250.0), 1.0, 1.0, 0)).unit(UnitKind::Pixels),
        Param::new("color", tl!("Color:"), ParamKind::color(Some(foreground))),
        Param::new("location", tl!("Location:"), ParamKind::radio(locations, Some("center".into()))),
    ];
    rows.extend(blending_rows());
    rows
}

/// Open Stroke with the current editing target captured, retaining the command's channel-view behavior.
pub fn open(app: &mut PhotocraftApp) -> DialogId {
    let target = app.with_mask_target(COMMAND, json!({}));
    let target = target.get("target").cloned().unwrap_or_else(|| crate::canvas::paint_target(app));
    let mut held = json!({ "target": target });
    if let Some(st) = app.session.active() {
        held["document"] = json!(st.doc.id.0);
        held["layer"] = json!(st.active_layer);
    }
    let [r, g, b, _] = app.session.tools.foreground;
    let rows = rows([r, g, b]);
    let title = Label::from(tl!("Stroke"));
    open_param(app, COMMAND, DialogFields::new(), held, move |fields| {
        ParamDialog::new(COMMAND, title.clone(), Cow::Owned(rows.clone())).values(Bind::bound(fields)).remember(SaveKey::named(REMEMBER))
    })
}

/// The `edit.stroke` params of the dialog's fields and the captured editing target.
pub fn params(f: &DialogFields, held: &Value) -> Value {
    let mut p = blending_params(f);
    if let Some(width) = f.number("width") {
        p.insert("width".into(), json!(width));
    }
    if let Some(color) = f.color("color") {
        p.insert("color".into(), json!(hex(color)));
    }
    if let Some(location) = f.text("location") {
        p.insert("location".into(), json!(location));
    }
    if let Some(target) = held.get("target") {
        p.insert("target".into(), target.clone());
    }
    Value::Object(p)
}

/// Whether the dialog `key` is the Stroke dialog.
pub(crate) fn owns(key: &str) -> bool {
    key == COMMAND
}

/// OK is held until the document and layer are the ones the dialog opened on.
pub(crate) fn check(app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    let captured = &captured(app, &result.key);
    let st = app.session.active().ok_or_else(|| Label::from(tl!("The document is closed")))?;
    if captured.get("document").and_then(Value::as_u64) != Some(st.doc.id.0) || captured.get("layer") != Some(&json!(st.active_layer)) {
        return Err(Label::from(tl!("The document or active layer changed. Open Stroke again.")));
    }
    Ok(())
}

/// Stroke with the confirmed choices and, when it succeeds, remember them for the next open.
pub(crate) fn run(app: &mut PhotocraftApp, ctx: &egui::Context, result: ParamResult) {
    let captured = captured(app, &result.key);
    match app.run(COMMAND, params(&result.fields, &captured)) {
        Ok(_) => result.remember(ctx),
        Err(e) => crate::notices::error(app, e),
    }
}

#[cfg(test)]
mod tests;
