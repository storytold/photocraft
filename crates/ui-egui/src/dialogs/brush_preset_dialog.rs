//! The New Brush Preset dialog (#4490), as in Photoshop: the brush's tip and size on the left, the
//! name and what the preset captures in the middle, OK and Cancel on the right. Every way of making
//! a preset opens it: the picker's [+] and its gear menu, the Brush Settings [+] and the Brushes
//! tab's [+]. Nothing is saved until OK.
use egui::{Color32, pos2, vec2};
use photocraft_paint::{BrushPreset, BrushSettings};
use serde_json::{Map, Value, json};

use crate::{PhotocraftApp, state::DialogKind, theme::Tokens, widgets};

const MARK: &str = "__newBrushPreset";
const PREVIEW: f32 = 96.0;

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// The name the dialog suggests: the current preset's name and the first free number after it
/// ("Soft Round 1"), so OK never overwrites a preset; "Brush N" when the brush isn't a preset's.
pub fn suggested_name(presets: &[BrushPreset], current: Option<&str>) -> String {
    match current.filter(|n| presets.iter().any(|p| p.name.eq_ignore_ascii_case(n))) {
        Some(base) => (1..).map(|n| format!("{base} {n}")).find(|n| !taken(presets, n)).unwrap_or_else(|| base.to_string()),
        None => crate::brush_panel::new_preset_name(presets),
    }
}

fn taken(presets: &[BrushPreset], name: &str) -> bool {
    presets.iter().any(|p| p.name.eq_ignore_ascii_case(name.trim()))
}

/// Open the dialog for the tool's brush, or for `brush` (the Brush Settings panel's copy, which
/// may be ahead of the tool's by a frame). Returns the dialog's id.
pub fn open(app: &mut PhotocraftApp, brush: Option<&BrushSettings>) -> u64 {
    let tools = &app.session.tools;
    let mut fields = Map::new();
    fields.insert(MARK.into(), json!(true));
    fields.insert("__label".into(), json!("New Brush Preset…"));
    fields.insert("name".into(), json!(suggested_name(&tools.presets, tools.current_preset.as_deref())));
    fields.insert("captureSize".into(), json!(true));
    fields.insert("toolSettings".into(), json!(false));
    fields.insert("color".into(), json!(false));
    if let Some(b) = brush.and_then(|b| serde_json::to_value(b).ok()) {
        fields.insert("brush".into(), b);
    }
    app.ui.open_dialog(DialogKind::Command, fields)
}

fn on(f: &Map<String, Value>, key: &str) -> bool {
    f.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn name(f: &Map<String, Value>) -> &str {
    f.get("name").and_then(Value::as_str).unwrap_or("").trim()
}

/// Why OK is greyed out, if it is.
fn problem(app: &PhotocraftApp, f: &Map<String, Value>) -> Option<&'static str> {
    if name(f).is_empty() {
        Some(tl!("Enter a name for the brush preset"))
    } else if taken(&app.session.tools.presets, name(f)) {
        Some(tl!("A brush preset with this name already exists"))
    } else {
        None
    }
}

fn check(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, label: &str) {
    let mut v = on(f, key);
    if widgets::checkbox(ui, &mut v, label).changed() {
        f.insert(key.into(), json!(v));
    }
}

/// The dialog's three columns; it owns its OK and Cancel, so they stand in their own column.
pub fn body(app: &PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, interactive: bool) -> Option<bool> {
    let t = Tokens::get(ui.ctx());
    let mut outcome = None;
    let brush: BrushSettings = f.get("brush").and_then(|b| serde_json::from_value(b.clone()).ok()).unwrap_or_else(|| app.session.tools.brush.clone());
    ui.horizontal_top(|ui| {
        // The tip as the preset cards draw it, and the size under it.
        let (r, _) = ui.allocate_exact_size(vec2(PREVIEW, PREVIEW), egui::Sense::hover());
        ui.painter().rect_filled(r, t.radius_sm, t.field);
        let tip = crate::brush_preview::tip_texture(ui.ctx(), "new-preset-tip", &brush.tip, (brush.hardness, brush.angle, brush.roundness), 64, t.text);
        let side = (PREVIEW - 30.0) * crate::brush_sections::thumb_scale(brush.size);
        let tr = egui::Rect::from_center_size(pos2(r.center().x, r.top() + 8.0 + (PREVIEW - 30.0) / 2.0), vec2(side, side));
        ui.painter().image(tip.id(), tr, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        ui.painter().text(
            pos2(r.center().x, r.bottom() - 5.0),
            egui::Align2::CENTER_BOTTOM,
            format!("{}", brush.size.round() as i64),
            egui::FontId::proportional(11.0),
            t.text_dim,
        );
        ui.add_space(14.0);
        ui.vertical(|ui| {
            crate::direction::dialog_row(ui, |ui| {
                ui.label(tl!("Name:"));
                let mut text = f.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                let id = ui.scope_id().with("new-brush-preset-name");
                let r = widgets::text_field(ui, id, &mut text, "", 220.0, false).accessible_name(tl!("Name:"));
                // Photoshop opens on the name, all of it selected, so typing replaces it.
                let once = id.with("focused");
                if !ui.ctx().data(|d| d.get_temp::<bool>(once).unwrap_or(false)) {
                    r.request_focus();
                    if egui::TextEdit::load_state(ui.ctx(), id).is_some() {
                        crate::field_tab::select_all(ui.ctx(), id, &text);
                        ui.ctx().data_mut(|d| d.insert_temp(once, true));
                    }
                }
                if r.changed() {
                    f.insert("name".into(), json!(text));
                }
            });
            ui.add_space(6.0);
            check(ui, f, "captureSize", tl!("Capture Brush Size in Preset"));
            check(ui, f, "toolSettings", tl!("Include Tool Settings"));
            // Include Color belongs to the tool settings: only with them, as in Photoshop.
            crate::direction::dialog_row(ui, |ui| {
                ui.add_space(20.0);
                ui.add_enabled_ui(on(f, "toolSettings"), |ui| check(ui, f, "color", tl!("Include Color")));
            });
            if let Some(why) = problem(app, f).filter(|_| !name(f).is_empty()) {
                ui.add_space(4.0);
                ui.colored_label(t.text_dim, why);
            }
        });
        ui.add_space(14.0);
        ui.vertical(|ui| {
            let ok = problem(app, f).is_none();
            if ui.add_enabled_ui(ok, |ui| widgets::primary_button(ui, tl!("OK"), 84.0)).inner.clicked() {
                outcome = Some(true);
            }
            ui.add_space(4.0);
            if widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked() {
                outcome = Some(false);
            }
        });
    });
    if outcome.is_none() && interactive && problem(app, f).is_none() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        outcome = Some(true);
    }
    outcome
}

/// OK: save the preset with the boxes as chosen. It becomes the current brush, as in Photoshop.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    if let Some(why) = problem(app, f) {
        return Err(why.into());
    }
    let tool_settings = on(f, "toolSettings");
    let mut p = json!({
        "name": name(f),
        "captureSize": on(f, "captureSize"),
        "toolSettings": tool_settings,
        "color": tool_settings && on(f, "color"),
    });
    if let Some(b) = f.get("brush") {
        p["brush"] = b.clone();
    }
    app.run("brush.presets.save", p)
}

#[cfg(test)]
mod tests;

/// The Name row mirrors in a right-to-left UI: the caption sits at the reading start and the
/// field follows it, the exact mirror of the left-to-right row.
#[cfg(test)]
mod rtl_tests {
    use super::*;
    use crate::direction::{Dir, direction_scope};
    use egui_kittest::kittest::Queryable;

    /// The dialog body drawn in `dir`, with the column it draws in.
    fn harness(dir: Dir) -> (egui_kittest::Harness<'static, (PhotocraftApp, Map<String, Value>, egui::Rect)>, egui::Rect) {
        let _dir = direction_scope(dir);
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(520.0, 320.0))
            .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ALL[0]))
            .build_ui_state(
                |ui, s: &mut (PhotocraftApp, Map<String, Value>, egui::Rect)| {
                    s.2 = ui.max_rect();
                    let _ = body(&s.0, ui, &mut s.1, true);
                },
                (app, open_fields(), egui::Rect::ZERO),
            );
        h.run_steps(2);
        let col = h.state().2;
        (h, col)
    }

    fn open_fields() -> Map<String, Value> {
        let mut f = Map::new();
        f.insert(MARK.into(), json!(true));
        f.insert("name".into(), json!("Preset 1"));
        f
    }

    /// The row reads from the right in a right-to-left UI and keeps its left-to-right order and
    /// gaps otherwise.
    #[test]
    fn the_name_row_mirrors_in_a_right_to_left_ui() {
        let (ltr, ltr_col) = harness(Dir::Ltr);
        let ltr_field = ltr.get_by_role(egui::accesskit::Role::TextInput).rect();
        assert!(ltr_field.left() - ltr_col.left() > 30.0, "LTR: the caption comes before the field: {ltr_field:?} in {ltr_col:?}");

        let (rtl, rtl_col) = harness(Dir::Rtl);
        let rtl_field = rtl.get_by_role(egui::accesskit::Role::TextInput).rect();
        assert!(rtl_col.right() - rtl_field.right() > 30.0, "RTL: the caption comes after the field: {rtl_field:?} in {rtl_col:?}");
        let centre = ltr_col.left() + rtl_col.right();
        assert!((ltr_field.left() + rtl_field.right() - centre).abs() < 1.0, "the field mirrors: {ltr_field:?} / {rtl_field:?}");
    }
}
