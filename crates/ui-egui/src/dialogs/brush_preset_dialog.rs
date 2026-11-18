//! The New Brush Preset dialog (#4490), as in Photoshop: the brush's tip and size on the left, the
//! name and what the preset captures beside it, OK and Cancel below. Every way of making a preset
//! opens it: the picker's [+] and its gear menu, the Brush Settings [+] and the Brushes tab's [+].
//! Nothing is saved until OK.
use craft::draw::ui::brush_presets::tip_thumb_scale;
use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::primitives::textures::TextureCache;
use craft::ui::runtime::env::Label;
use egui::{Color32, pos2, vec2};
use photocraft_paint::{BrushPreset, BrushSettings};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::params;
use crate::theme::Tokens;

/// The key of the dialog.
pub const KEY: &str = "brushPresets.new";
const PREVIEW: f32 = 96.0;

/// Whether the hosted dialog with `key` is this one.
pub(crate) fn owns(key: &str) -> bool {
    key == KEY
}

/// The name the dialog suggests: the current preset's name and the first free number after it
/// ("Soft Round 1"), so OK never overwrites a preset; "Brush N" when the brush isn't a preset's.
pub fn suggested_name(presets: &[BrushPreset], current: Option<&str>) -> String {
    match current.filter(|n| presets.iter().any(|p| p.name.eq_ignore_ascii_case(n))) {
        Some(base) => (1..).map(|n| format!("{base} {n}")).find(|n| !taken(presets.iter().map(|p| p.name.as_str()), n)).unwrap_or_else(|| base.to_string()),
        None => crate::brush_panel::new_preset_name(presets),
    }
}

/// Whether `name` is one of `names`, as presets compare names.
fn taken<'a>(names: impl IntoIterator<Item = &'a str>, name: &str) -> bool {
    names.into_iter().any(|n| n.eq_ignore_ascii_case(name.trim()))
}

fn preset_names(app: &PhotocraftApp) -> Vec<String> {
    app.session.tools.presets.iter().map(|p| p.name.clone()).collect()
}

/// Open the dialog for the tool's brush, or for `brush` (the Brush Settings panel's copy, which
/// may be ahead of the tool's by a frame). Returns the dialog's id.
pub fn open(app: &mut PhotocraftApp, brush: Option<&BrushSettings>) -> DialogId {
    let tools = &app.session.tools;
    let names = preset_names(app);
    let suggested = suggested_name(&tools.presets, tools.current_preset.as_deref());
    let shown: BrushSettings = brush.unwrap_or(&tools.brush).clone();
    let captured = match brush.and_then(|b| serde_json::to_value(b).ok()) {
        Some(b) => json!({ "brush": b }),
        None => Value::Null,
    };
    params::open_param(app, KEY, DialogFields::new(), captured, move |fields: &mut DialogFields| {
        // Before the first pass opens the values the name is the suggestion.
        let typed = if fields.contains("name") { name(fields).to_string() } else { suggested.trim().to_string() };
        let why = problem(&names, &typed);
        let tip = shown.clone();
        let rows = vec![
            Param::custom("tip", tl!("Tip"), move |ui, _fields, _cx| tip_row(ui, &tip)),
            Param::new("name", tl!("Name:"), ParamKind::text(suggested.clone())),
            Param::new("captureSize", tl!("Capture Brush Size in Preset"), ParamKind::bool(true)),
            Param::new("toolSettings", tl!("Include Tool Settings"), ParamKind::bool(false)),
            // Include Color belongs to the tool settings: only with them, as in Photoshop.
            Param::new("color", tl!("Include Color"), ParamKind::bool(false)).enabled_when("toolSettings", true),
        ];
        let mut dialog = ParamDialog::command(KEY, tl!("New Brush Preset…"), rows).values(Bind::bound(fields)).ok_enabled(why.is_none());
        if let Some(why) = why {
            dialog = dialog.ok_reason(Label::from(why));
        }
        dialog
    })
}

/// The tip as the preset cards draw it, and the size under it.
fn tip_row(ui: &mut egui::Ui, brush: &BrushSettings) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (r, response) = ui.allocate_exact_size(vec2(PREVIEW, PREVIEW), egui::Sense::hover());
    ui.painter().rect_filled(r, t.radius_sm, t.field);
    let sig = crate::brush_preview::tip_sig(&brush.tip, brush.hardness, brush.angle, brush.roundness, (false, false));
    let colour = t.text;
    let texture = TextureCache::texture(ui.ctx(), ("new-preset-tip", sig, colour), || {
        crate::brush_preview::tip_image(&brush.tip, (brush.hardness, brush.angle, brush.roundness), 64, colour)
    });
    let side = (PREVIEW - 30.0) * tip_thumb_scale(f64::from(brush.size));
    let tr = egui::Rect::from_center_size(pos2(r.center().x, r.top() + 8.0 + (PREVIEW - 30.0) / 2.0), vec2(side, side));
    ui.painter().image(texture, tr, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    ui.painter().text(
        pos2(r.center().x, r.bottom() - 5.0),
        egui::Align2::CENTER_BOTTOM,
        format!("{}", brush.size.round() as i64),
        egui::FontId::proportional(11.0),
        t.text_dim,
    );
    response
}

fn on(f: &DialogFields, key: &str) -> bool {
    f.bool(key).unwrap_or(false)
}

fn name(f: &DialogFields) -> &str {
    f.text("name").unwrap_or("").trim()
}

/// Why OK is greyed out, if it is.
fn problem(names: &[String], typed: &str) -> Option<&'static str> {
    if typed.trim().is_empty() {
        Some(tl!("Enter a name for the brush preset"))
    } else if taken(names.iter().map(String::as_str), typed) {
        Some(tl!("A brush preset with this name already exists"))
    } else {
        None
    }
}

/// OK: save the preset with the boxes as chosen. It becomes the current brush, as in Photoshop.
pub(crate) fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let f = &result.fields;
    if let Some(why) = problem(&preset_names(app), name(f)) {
        return super::alerts::report(app, Err(why.into()));
    }
    let tool_settings = on(f, "toolSettings");
    let mut p = json!({
        "name": name(f),
        "captureSize": on(f, "captureSize"),
        "toolSettings": tool_settings,
        "color": tool_settings && on(f, "color"),
    });
    let brush = params::captured(app, KEY)["brush"].clone();
    if brush != Value::Null {
        p["brush"] = brush;
    }
    let outcome = app.run("brush.presets.save", p);
    super::alerts::report(app, outcome);
}

#[cfg(test)]
mod tests;
