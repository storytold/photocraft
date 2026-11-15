//! Color Settings' Settings menu (#14229): picking a named setup loads its values into the
//! dialog's controls, except Blend Text Colors Using Gamma, which keeps what it shows; changing
//! any control afterwards shows "Custom" at once. OK sends the menu's entry as `preset`.

use egui::Ui;
use photocraft_engine::color_cmds::{COLOR_SETTINGS_PRESETS, CUSTOM_SETTINGS, ColorSettings, is_settings_preset, preset_setup};
use serde_json::{Map, Value, json};

use crate::theme::Tokens;

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
/// The working RGB "Monitor Color" sets: the monitor profile in use, refreshed by the dialog.
pub(crate) const MONITOR_RGB: &str = "__monitorRgb";
/// What a setup leaves as the controls show it.
const KEPT: [&str; 2] = ["blendTextGamma", "monitorProfile"];

pub(super) fn owns(command: &str, key: &str) -> bool {
    command == COMMAND && key == KEY
}

/// The menu entry the fields hold (Custom for anything the menu doesn't have).
fn current(f: &Map<String, Value>) -> String {
    f.get(KEY).and_then(Value::as_str).filter(|n| is_settings_preset(n)).unwrap_or(CUSTOM_SETTINGS).to_string()
}

/// The Settings row, on top of the dialog.
pub(super) fn row(ui: &mut Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let mut cur = current(f);
    let mut opts: Vec<(String, &str)> = vec![(CUSTOM_SETTINGS.to_string(), CUSTOM_SETTINGS), (OTHER.to_string(), OTHER)];
    opts.extend(COLOR_SETTINGS_PRESETS.iter().map(|n| (n.to_string(), *n)));
    let before = cur.clone();
    ui.horizontal(|ui| {
        let name = super::label("settings");
        ui.label(egui::RichText::new(&name).color(t.text_dim));
        crate::widgets::dropdown_wheel_grouped_enabled(
            ui,
            "flt-edit.colorSettings-preset",
            &name,
            &mut cur,
            &opts,
            250.0,
            |v| GROUP_STARTS.contains(&v.as_str()),
            |v| v != OTHER,
        );
    });
    // The keys step onto the greyed label too; it is no value.
    if cur == OTHER {
        cur = before;
    }
    f.insert(KEY.into(), json!(cur));
}

/// After the controls drew: load a newly picked setup (from the menu or `ui.dialog.set`), or show
/// Custom once a control differs from the values the shown setup gave.
pub(super) fn sync(ctx: &egui::Context, f: &mut Map<String, Value>) {
    let shown = current(f);
    let Some(applied) = f.get(APPLIED).and_then(Value::as_str).map(str::to_string) else {
        // Opening: the controls hold the stored setup's values.
        f.insert(APPLIED.into(), json!(shown));
        f.insert(BASE.into(), Value::Object(tracked(f)));
        return;
    };
    if shown != applied {
        let monitor_rgb = f.get(MONITOR_RGB).and_then(Value::as_str).unwrap_or("srgb").to_string();
        if let Some(Value::Object(setup)) = preset_setup(&ColorSettings::default(), &shown, &monitor_rgb).and_then(|c| serde_json::to_value(c).ok()) {
            for (k, v) in setup {
                if is_tracked(&k) && !KEPT.contains(&k.as_str()) {
                    f.insert(k, v);
                }
            }
        }
        f.insert(APPLIED.into(), json!(shown));
        f.insert(BASE.into(), Value::Object(tracked(f)));
        ctx.request_repaint();
        return;
    }
    if shown == CUSTOM_SETTINGS {
        return;
    }
    let mut base = match f.remove(BASE) {
        Some(Value::Object(b)) => b,
        _ => Map::new(),
    };
    let mut edited = false;
    for (k, v) in tracked(f) {
        match base.get(&k) {
            Some(b) => edited |= !same(b, &v),
            // A control that filled in its default after opening is no edit.
            None => {
                base.insert(k, v);
            }
        }
    }
    f.insert(BASE.into(), Value::Object(base));
    if edited {
        f.insert(KEY.into(), json!(CUSTOM_SETTINGS));
        f.insert(APPLIED.into(), json!(CUSTOM_SETTINGS));
        ctx.request_repaint();
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
