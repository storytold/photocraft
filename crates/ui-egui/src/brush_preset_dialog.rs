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
    let panel = "brushes";
    let selection = app
        .ui
        .presets_ui
        .browser_selection_snapshot
        .get(panel)
        .cloned()
        .or_else(|| crate::pickers::adapters::brush_engine_adapter::active_brush_selection(&app.ui.presets_ui, panel));
    let destination = Some(crate::pickers::adapters::brush_engine_adapter::new_preset_destination(
        &app.session.tools.presets,
        &mut app.ui.presets_ui,
        panel,
        selection.as_ref(),
    ));
    open_at_panel(app, panel, brush, destination)
}

/// Open the creation dialog with an optional browser insertion destination.
pub fn open_at(app: &mut PhotocraftApp, brush: Option<&BrushSettings>, destination: Option<(String, Vec<String>, usize)>) -> u64 {
    open_at_panel(app, "brushes", brush, destination)
}

pub fn open_at_named(
    app: &mut PhotocraftApp,
    brush: Option<&BrushSettings>,
    destination: Option<(String, Vec<String>, usize)>,
    name: Option<&str>,
) -> u64 {
    open_at_panel_named(app, "brushes", brush, destination, name)
}

pub fn open_at_panel(
    app: &mut PhotocraftApp,
    panel: &str,
    brush: Option<&BrushSettings>,
    destination: Option<(String, Vec<String>, usize)>,
) -> u64 {
    open_at_panel_named(app, panel, brush, destination, None)
}

pub fn open_at_panel_named(
    app: &mut PhotocraftApp,
    panel: &str,
    brush: Option<&BrushSettings>,
    destination: Option<(String, Vec<String>, usize)>,
    name: Option<&str>,
) -> u64 {
    let tools = &app.session.tools;
    let mut fields = Map::new();
    fields.insert(MARK.into(), json!(true));
    fields.insert("__label".into(), json!("New Brush Preset…"));
    fields.insert("__insertPanel".into(), json!(panel));
    fields.insert("name".into(), json!(suggested_name(&tools.presets, tools.current_preset.as_deref())));
    if let Some(name) = name { fields.insert("name".into(), json!(name)); }
    fields.insert("captureSize".into(), json!(true));
    fields.insert("toolSettings".into(), json!(false));
    fields.insert("color".into(), json!(false));
    if let Some(b) = brush.and_then(|b| serde_json::to_value(b).ok()) {
        fields.insert("brush".into(), b);
    }
    if let Some((group, folder, index)) = destination {
        fields.insert("__insertGroup".into(), json!(group));
        fields.insert("__insertFolder".into(), json!(folder));
        fields.insert("__insertIndex".into(), json!(index));
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
            ui.horizontal(|ui| {
                ui.label(tl!("Name:"));
                let mut text = f.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                let id = ui.id().with("new-brush-preset-name");
                let r = widgets::text_field(ui, id, &mut text, "", 220.0, false);
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
            ui.horizontal(|ui| {
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
    let result = app.run("brush.presets.save", p)?;
    let created_name = result.get("name").and_then(Value::as_str).unwrap_or(name(f));
    if let (Some(group), Some(folder), Some(index)) =
        (f.get("__insertGroup").and_then(Value::as_str), f.get("__insertFolder").and_then(Value::as_array), f.get("__insertIndex").and_then(Value::as_u64))
    {
        let folder: Vec<String> = folder.iter().filter_map(Value::as_str).map(str::to_owned).collect();
        let params = json!({"name":created_name, "group":group, "folder":folder, "index":index});
        app.run("brush.presets.move", params)?;
        let panel = f.get("__insertPanel").and_then(Value::as_str).unwrap_or("brushes");
        crate::pickers::adapters::brush_engine_adapter::expand_brush_destination(app, panel, group, &folder);
    }
    // Saving a brush from any entry point creates the active preset. Keep the shared browser's
    // selected identity in sync, including root-level creation without an insertion destination.
    if let Some(preset) = app.session.tools.presets.iter().find(|preset| preset.name == created_name) {
        let key = crate::pickers::adapters::brush_engine_adapter::preview_identity(preset);
        let panel = f.get("__insertPanel").and_then(Value::as_str).unwrap_or("brushes");
        let projection = crate::pickers::adapters::brush_engine_adapter::project_brush_browser(
            &app.session.tools.presets,
            &mut app.ui.presets_ui,
            panel,
        );
        let group = crate::pickers::adapters::brush_engine_adapter::container_key_for_preset(&projection, preset);
        app.ui.presets_ui.clear_browser_selection(panel);
        app.ui.presets_ui.set_selected_item(panel, &group, &[], &key);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::confirm;
    use serde_json::{Map, Value, json};

    fn fields(name: &str, group: &str, folder: &[&str], index: usize) -> Map<String, Value> {
        let mut fields = Map::new();
        fields.insert("name".into(), json!(name));
        fields.insert("captureSize".into(), json!(true));
        fields.insert("toolSettings".into(), json!(false));
        fields.insert("color".into(), json!(false));
        fields.insert("__insertGroup".into(), json!(group));
        fields.insert("__insertFolder".into(), json!(folder));
        fields.insert("__insertIndex".into(), json!(index));
        fields
    }

    #[test]
    fn confirming_creation_selects_the_new_name_key_in_the_shared_browser() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        confirm(&mut app, &fields("Created brush", "", &[], 0)).unwrap();
        let preset = app.session.tools.presets.iter().find(|preset| preset.name == "Created brush").unwrap();
        let projection = crate::pickers::adapters::brush_engine_adapter::project_brush_browser(
            &app.session.tools.presets, &mut app.ui.presets_ui, "brushes",
        );
        let group = crate::pickers::adapters::brush_engine_adapter::container_key_for_preset(&projection, preset);
        assert!(app.ui.presets_ui.is_selected_item("brushes", &group, &[], "Created brush"));
        assert!(app.ui.presets_ui.browser_selection.get("brushes").is_none_or(|selection| selection.is_empty()));
    }

    #[test]
    fn confirming_creation_moves_the_saved_preset_into_the_main_engine_folder() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("brush.presets.save", json!({"name":"First"})).unwrap();
        app.run("brush.presets.save", json!({"name":"Second"})).unwrap();
        app.run("brush.presets.move", json!({"name":"First", "group":"Pens", "folder":["Fine"]})).unwrap();
        app.run("brush.presets.move", json!({"name":"Second", "group":"Pens", "folder":["Fine"]})).unwrap();

        confirm(&mut app, &fields("Created brush", "Pens", &["Fine"], 1)).unwrap();
        let created = app.session.tools.presets.iter().find(|preset| preset.name == "Created brush").unwrap();
        assert_eq!(created.group, "Pens");
        assert_eq!(created.folder, vec!["Fine".to_owned()]);
    }

    #[test]
    fn opening_does_not_expand_a_folder_but_confirmation_expands_its_shared_group_key() {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("brush.presets.save", json!({"name":"Ink"})).unwrap();
        app.run("brush.presets.move", json!({"name":"Ink", "group":"Pens", "folder":["Fine"]})).unwrap();
        let projection = crate::pickers::adapters::brush_engine_adapter::project_brush_browser(
            &app.session.tools.presets, &mut app.ui.presets_ui, "brushes",
        );
        let key = projection.containers.values().find(|container| container.engine_group == "Pens" && container.folder == vec!["Fine".to_owned()])
            .unwrap().key.clone();
        let expanded = format!("brushes/{}", crate::pickers::state::escape_hierarchy_key_part(&key));
        app.ui.presets_ui.expanded_groups.remove(&expanded);
        crate::brush_preset_dialog::open_at(&mut app, None, Some(("Pens".into(), vec!["Fine".into()], 1)));
        assert!(!app.ui.presets_ui.expanded_groups.contains(&expanded));
        confirm(&mut app, &fields("New Ink", "Pens", &["Fine"], 1)).unwrap();
        assert!(app.ui.presets_ui.expanded_groups.contains(&expanded));
    }
}
