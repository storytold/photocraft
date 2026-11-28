//! Color Settings' Settings menu (#14229): picking a named setup loads its values into the
//! dialog's controls, except Blend Text Colors Using Gamma, which keeps what it shows; changing
//! any control afterwards shows "Custom" at once. OK sends the menu's entry as `preset`.
//!
//! The menu is a custom row ([`row`]); [`reconcile`] does the rest after each pass of the dialog
//! host and after each `ui.dialog.set`, so a confirm that follows a script's set sends the same
//! name and values a draw would have shown.
//!
//! Blend Text Colors Using Gamma is a checkbox beside a 1.00 to 3.00 field ([`text_gamma_rows`],
//! #14243).

use craft::ui::contract::{Bind, Entry, Outcome as _, Show as _};
use craft::ui::dialog::{Alert, dialog_id};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamKind};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::{NumberField, RangeAlert};
use photocraft_engine::color_cmds::{COLOR_SETTINGS_PRESETS, CUSTOM_SETTINGS, ColorExt, ColorSettings, MIN_TEXT_GAMMA, is_settings_preset, preset_setup};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

pub(super) const COMMAND: &str = "edit.colorSettings";
const KEY: &str = "preset";
/// The menu's greyed group label: never a value.
const OTHER: &str = "Other";
/// The entries a separator line goes above.
const GROUP_STARTS: [&str; 3] = [OTHER, "Europe General Purpose 3", "Japan Color for Newspaper"];
/// The menu entry the controls last took their values from (or showed on opening).
const APPLIED: &str = "__presetApplied";
/// The controls' values at that moment: an edit makes them differ.
const BASE: &str = "__presetBase";
/// The setup whose working RGB is the monitor profile.
const MONITOR_COLOR: &str = "Monitor Color";
/// What a setup leaves as the controls show it.
const KEPT: [&str; 3] = [GAMMA, GAMMA_ON, "monitorProfile"];
/// Blend Text Colors Using Gamma's field and checkbox.
const GAMMA: &str = "blendTextGamma";
const GAMMA_ON: &str = "blendTextGammaOn";

/// The menu entry `value` names (Custom for anything the menu doesn't have).
fn entry_of(value: Option<&str>) -> String {
    value.filter(|n| is_settings_preset(n)).unwrap_or(CUSTOM_SETTINGS).to_string()
}

/// The Settings row, in place of the spec's plain choice: Custom, the greyed Other label and the
/// named setups in their groups.
pub(super) fn row(label: Label) -> Param {
    let name = label.clone();
    Param::custom(KEY, label, move |ui, fields, _| {
        let mut entries = Vec::new();
        let named = COLOR_SETTINGS_PRESETS.iter().map(|n| (*n, *n));
        for (value, label) in [(CUSTOM_SETTINGS, tl!("Custom")), (OTHER, tl!("Other"))].into_iter().chain(named) {
            if GROUP_STARTS.contains(&value) {
                entries.push(Entry::separator());
            }
            entries.push(Entry::new(value.to_string(), Label::from(label)).enabled(value != OTHER));
        }
        let out = Select::new(name.clone(), Bind::given(entry_of(fields.text(KEY))), &entries).show(ui);
        if let Some(picked) = out.value().cloned() {
            fields.set(KEY, FieldValue::Text(picked));
        }
        out.response().clone()
    })
}

/// Blend Text Colors Using Gamma, in place of the spec's two rows: the checkbox, with the field
/// beside it. The field shows two decimals, 1.00 to 3.00, and stays editable while the box is off;
/// a value typed or stepped into it checks the box. A value typed out of range is held to the
/// closest end with the range alert, written with the field's two decimals, which [`reconcile`]
/// opens; an emptied field puts its value back.
pub(super) fn text_gamma_rows(list: &mut Vec<Param>) {
    list.retain(|p| p.key != GAMMA_ON);
    let Some(at) = list.iter().position(|p| p.key == GAMMA) else { return };
    let (min, max) = (f64::from(MIN_TEXT_GAMMA), f64::from(photocraft_compose::psblend::MAX_TEXT_GAMMA));
    let default = f64::from(photocraft_compose::psblend::TEXT_GAMMA);
    let label = Label::from(tl!("Blend Text Colors Using Gamma").to_string());
    let on = Param::new(GAMMA_ON, label.clone(), ParamKind::bool(true)).beside_next();
    let field = Param::custom(GAMMA, Label::Empty, move |ui, fields, _| {
        let value = fields.number(GAMMA).filter(|v| v.is_finite()).unwrap_or(default);
        let shown = NumberField::new(label.clone(), Bind::given(Some(value))).fixed_decimals(2).range(min..=max).step(0.01).show(ui);
        if let Some(alert) = shown.out_of_range.clone() {
            ui.ctx().data_mut(|d| d.insert_temp(gamma_alert_id(), Some((alert, Env::of(ui)))));
        }
        if let Some(v) = shown.edit.value().copied().flatten().filter(|v| v.is_finite()) {
            fields.set(GAMMA, FieldValue::Number(((v * 100.0).round() / 100.0).clamp(min, max)));
            fields.set(GAMMA_ON, FieldValue::Bool(true));
        }
        shown.edit.response.clone()
    })
    .opens_with(FieldValue::Number(default));
    list.splice(at..=at, [on, field]);
}

/// Where the gamma field leaves its range alert for [`reconcile`], which can open it.
fn gamma_alert_id() -> egui::Id {
    egui::Id::unique("pc-color-settings-gamma-alert")
}

/// Opens the gamma field's range alert, titled as the dialog.
fn show_gamma_alert(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some((alert, env)) = ctx.data_mut(|d| d.remove_temp::<Option<(RangeAlert, Env)>>(gamma_alert_id())).flatten() else { return };
    let _ = crate::dialogs::alerts::show(app, Alert::range(tl!("Color Settings"), &alert, &env));
}

/// After a pass or a set: shows Custom for a name the menu doesn't have, loads a newly picked setup
/// (from the menu or `ui.dialog.set`), or shows Custom once a control differs from the values the
/// shown setup gave.
pub(crate) fn reconcile(app: &mut PhotocraftApp, ctx: &egui::Context) {
    show_gamma_alert(app, ctx);
    let Some(dialog) = app.framework.dialogs.get_by_id::<HostedParamDialog>(dialog_id(COMMAND)) else { return };
    let f = fields_map(dialog.fields());
    let shown = entry_of(f.get(KEY).and_then(Value::as_str));
    if f.get(KEY).and_then(Value::as_str) != Some(shown.as_str()) {
        set_hidden(app, KEY, json!(shown));
    }
    let Some(applied) = f.get(APPLIED).and_then(Value::as_str).map(str::to_string) else {
        // Opening: the controls hold the stored setup's values. A `ui.dialog.set` that picked
        // another entry before the first pass loads it.
        if shown != entry_of(Some(&app.session.color().settings.preset)) {
            load(app, ctx, &f, &shown);
            return;
        }
        // A control a `ui.dialog.set` changed before the first pass is an edit, not the baseline.
        if shown != CUSTOM_SETTINGS && differs_from_stored(app, &f) {
            set_hidden(app, KEY, json!(CUSTOM_SETTINGS));
            set_hidden(app, APPLIED, json!(CUSTOM_SETTINGS));
            return;
        }
        let base = tracked(&f);
        // A monitor profile set before the first pass moves Monitor Color's working RGB too.
        if shown == MONITOR_COLOR && monitor_rgb_moved(app, &f, &base) {
            load(app, ctx, &f, &shown);
            return;
        }
        set_hidden(app, APPLIED, json!(shown));
        set_hidden(app, BASE, Value::Object(base));
        return;
    };
    if shown != applied {
        load(app, ctx, &f, &shown);
        return;
    }
    if shown == CUSTOM_SETTINGS {
        return;
    }
    let mut base = match f.get(BASE) {
        Some(Value::Object(b)) => b.clone(),
        _ => Map::new(),
    };
    let (mut edited, mut grew) = (false, false);
    for (k, v) in tracked(&f) {
        match base.get(&k) {
            Some(b) => edited |= !same(b, &v),
            // A control that filled in its default after opening is no edit.
            None => {
                base.insert(k, v);
                grew = true;
            }
        }
    }
    if edited {
        set_hidden(app, KEY, json!(CUSTOM_SETTINGS));
        set_hidden(app, APPLIED, json!(CUSTOM_SETTINGS));
    } else if shown == MONITOR_COLOR && monitor_rgb_moved(app, &f, &base) {
        // Monitor Color's working RGB follows the monitor profile the dialog shows.
        load(app, ctx, &f, &shown);
    } else if grew {
        set_hidden(app, BASE, Value::Object(base));
    }
}

/// Loads the named setup into the controls (but the [`KEPT`] ones) and makes it the one they show.
/// "Monitor Color" takes the monitor profile the dialog shows as its working RGB.
fn load(app: &mut PhotocraftApp, ctx: &egui::Context, f: &Map<String, Value>, name: &str) {
    let monitor = f.get("monitorProfile").and_then(Value::as_str).unwrap_or("auto");
    let monitor_rgb = app.session.color().monitor_rgb_for(monitor);
    let id = dialog_id(COMMAND);
    if let Some(Value::Object(setup)) = preset_setup(&ColorSettings::default(), name, &monitor_rgb).and_then(|c| serde_json::to_value(c).ok()) {
        for (k, v) in setup {
            if is_tracked(&k) && !KEPT.contains(&k.as_str()) && f.contains_key(&k) {
                // A value a row refuses keeps what the row shows; the setups' values all fit the rows.
                let _ = app.framework.dialogs.set_json_field(ctx, id, &k, v);
            }
        }
    }
    let Some(dialog) = app.framework.dialogs.get_by_id::<HostedParamDialog>(id) else { return };
    let base = tracked(&fields_map(dialog.fields()));
    set_hidden(app, APPLIED, json!(name));
    set_hidden(app, BASE, Value::Object(base));
}

/// A tracked control no longer shows the stored Color Settings' value (for Reset, which the
/// settings don't hold, its opening value: off).
fn differs_from_stored(app: &PhotocraftApp, f: &Map<String, Value>) -> bool {
    let Ok(Value::Object(stored)) = serde_json::to_value(&app.session.color().settings) else { return false };
    let opening = |k: &str| stored.get(k).cloned().or_else(|| (k == "reset").then_some(Value::Bool(false)));
    tracked(f).iter().any(|(k, v)| opening(k).is_some_and(|s| !same(&s, v)))
}

/// The working RGB Monitor Color gave (in `base`) is no longer the one the shown monitor profile gives.
fn monitor_rgb_moved(app: &PhotocraftApp, f: &Map<String, Value>, base: &Map<String, Value>) -> bool {
    let monitor = f.get("monitorProfile").and_then(Value::as_str).unwrap_or("auto");
    base.get("workingRgb").and_then(Value::as_str) != Some(app.session.color().monitor_rgb_for(monitor).as_str())
}

/// Writes `value` under `key` of the open dialog: the menu's entry as text, the bookkeeping as JSON.
fn set_hidden(app: &mut PhotocraftApp, key: &str, value: Value) {
    let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedParamDialog>(dialog_id(COMMAND)) else { return };
    match value {
        Value::String(text) => dialog.set_field(key, FieldValue::Text(text)),
        value => dialog.set_field(key, FieldValue::Json(value)),
    }
}

fn fields_map(fields: &DialogFields) -> Map<String, Value> {
    match serde_json::to_value(fields) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

/// Keys whose change makes the values Custom: the params but the menu and the monitor profile.
fn is_tracked(k: &str) -> bool {
    !k.starts_with("__") && k != KEY && k != "monitorProfile"
}

fn tracked(f: &Map<String, Value>) -> Map<String, Value> {
    f.iter().filter(|(k, _)| is_tracked(k)).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// Equal values; numbers within the controls' rounding (a stored f32 gamma reads back as
/// 1.4500000476837158, the field shows 1.45).
fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => (x - y).abs() <= 1e-4,
        _ => a == b,
    }
}

#[cfg(test)]
#[path = "../color_settings_presets_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../color_settings_text_gamma_tests.rs"]
mod text_gamma_tests;
