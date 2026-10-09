//! Actions panel: record and replay command sequences.
//!
//! The list and the recording flag live on the engine session (`actions.record` / `stop` /
//! `play` / `list` / `get` / `delete` / `move`), so the panel, the CLI and MCP share them. This module
//! keeps only which row is selected and which rows are expanded.

use egui::{Align2, Color32, Rect, Sense, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use photocraft_engine::actions_cmds::{self, Action};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// Which row is selected and which rows are expanded. The action list itself is
/// [`photocraft_engine::Session::actions`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ActionsUi {
    pub selected: Option<usize>,
    pub selected_step: Option<usize>,
    pub expanded: Vec<bool>,
    #[serde(skip)]
    reveal_recording: Option<(usize, usize)>,
}

fn label_of(id: &str) -> String {
    photocraft_engine::commands::find(id)
        .map(|c| c.label)
        .or_else(|| crate::menus::UI_COMMANDS.iter().find(|c| c.0 == id).map(|c| c.1))
        .map(|label| label.trim_end_matches('…').to_string())
        .unwrap_or_else(|| id.to_string())
}

/// The selected action, or the first one when nothing is selected.
pub fn selected_action(app: &PhotocraftApp) -> Option<&Action> {
    let list = &app.session.actions.list;
    let idx = app.ui.actions.selected.filter(|i| *i < list.len());
    idx.and_then(|i| list.get(i)).or_else(|| list.first())
}

/// Steps in the `[[id, params], …]` shape batch and droplets already accept.
pub fn action_steps(action: &Action) -> Vec<Value> {
    action.steps.iter().map(|(id, p)| json!([id, p])).collect()
}

/// Named action bindings use the existing persisted shortcut overrides. Keeping the
/// name in the key means selecting or deleting another row cannot retarget F6.
pub(crate) const SHORTCUT_PREFIX: &str = "actions.play:";

pub(crate) fn shortcut_id(name: &str) -> String {
    format!("{SHORTCUT_PREFIX}{name}")
}

pub(crate) fn assign_shortcut(app: &mut PhotocraftApp, name: &str, shortcut: &str) -> Result<Value, String> {
    let id = shortcut_id(name);
    let mut set = serde_json::Map::new();
    // A function key belongs to only one action. Ordinary menu bindings remain
    // intact and become available again when the action binding is removed.
    for (other, assigned) in &app.session.prefs().shortcuts {
        if other.starts_with(SHORTCUT_PREFIX) && other != &id && assigned == shortcut && !shortcut.is_empty() {
            set.insert(other.clone(), json!(""));
        }
    }
    set.insert(id, json!(shortcut));
    app.run("edit.keyboardShortcuts", json!({"set": set, "allowUnknown": true, "removeConflicts": false}))
}

/// Replay edits synchronously, with view steps handled by the shell in the same order.
/// Engine steps keep the headless action semantics (no dialogs or background jobs).
pub(crate) fn play(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let (action, from) = actions_cmds::playback_plan(&app.session, params).map_err(|e| e.to_string())?;
    let recording = actions_cmds::begin_playback(&mut app.session, &action).map_err(|e| e.to_string())?;
    let mut ran = 0u64;
    let mut failed = None;
    for (i, (id, p)) in action.steps.iter().enumerate().skip(from) {
        let result = (|| {
            if let Some(auth) = app.session.authorize {
                auth(id, p).map_err(|e| e.to_string())?;
            }
            if id == "actions.play" {
                play(app, p)
            } else if actions_cmds::shell_view_command(id) {
                app.sync_views();
                app.run(id, p.clone())
            } else {
                app.session.execute(id, p.clone()).map_err(|e| e.to_string())
            }
        })();
        match result {
            Ok(value) => {
                if let Some(error) = actions_cmds::nested_failure(id, &value) {
                    failed = Some(json!({"step": i, "id": id, "error": error}));
                    break;
                }
                ran += 1;
            }
            Err(error) => {
                failed = Some(json!({"step": i, "id": id, "error": error}));
                break;
            }
        }
    }
    actions_cmds::finish_playback(&mut app.session, recording, &action, from);
    Ok(match failed {
        Some(f) => json!({"action": action.name, "ran": ran, "failed": f}),
        None => json!({"action": action.name, "ran": ran}),
    })
}

/// Execute the actual menu command, not a synthetic keystroke. Fit is deferred
/// until canvas layout, so it uses the dimensions after all preceding action steps.
pub(crate) fn run_view(app: &mut PhotocraftApp, id: &str, params: Value) -> Result<Value, String> {
    app.sync_views();
    let i = app.session.active_index().ok_or("no document")?;
    let v = app.ui.views.get_mut(i).ok_or("no document view")?;
    v.fill_pending = false;
    match id {
        "view.fitOnScreen" => v.fit_pending = true,
        "view.zoomIn" => {
            v.fit_pending = false;
            v.zoom = crate::zoom_levels::step(v.zoom, 1, v.doc_size);
        }
        "view.zoomOut" => {
            v.fit_pending = false;
            v.zoom = crate::zoom_levels::step(v.zoom, -1, v.doc_size);
        }
        "view.actualPixels" => {
            v.fit_pending = false;
            v.zoom = 1.0;
        }
        _ => return Err(format!("unsupported view command: {id}")),
    }
    app.session.journal.push((id.into(), params));
    Ok(Value::Null)
}

/// Export the same versioned format as the persisted `actions.json` preset store.
fn export_actions_bytes(app: &PhotocraftApp) -> Result<Vec<u8>, String> {
    if app.session.actions.recording.is_some() || app.session.actions.playing > 0 {
        return Err("Stop recording or playing before exporting Actions".into());
    }
    let data = actions_cmds::ActionsFile { version: 1, actions: app.session.actions.list.clone() };
    let bytes = serde_json::to_vec_pretty(&data).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > photocraft_engine::preset_store::MAX_ACTIONS_BYTES {
        return Err("Actions exceed the 16 MiB export limit".into());
    }
    Ok(bytes)
}

/// Merge a portable action file into the list, preserving local actions. Rename
/// conflicting incoming names and update their nested named/indexed play steps.
fn import_actions_bytes(app: &mut PhotocraftApp, bytes: &[u8]) -> Result<Value, String> {
    if app.session.actions.recording.is_some() || app.session.actions.playing > 0 {
        return Err("Stop recording or playing before importing Actions".into());
    }
    if bytes.len() as u64 > photocraft_engine::preset_store::MAX_ACTIONS_BYTES {
        return Err("Actions file exceeds the 16 MiB import limit".into());
    }
    let mut file: actions_cmds::ActionsFile = serde_json::from_slice(bytes).map_err(|e| format!("Invalid Actions JSON: {e}"))?;
    if file.version != 1 {
        return Err(format!("Unsupported Actions file version {}", file.version));
    }
    if file.actions.len() > 1024 {
        return Err("An imported file may contain at most 1024 actions".into());
    }
    let mut names = std::collections::HashSet::new();
    for action in &file.actions {
        if action.name.trim().is_empty() || action.name.len() > 256 || !names.insert(action.name.clone()) {
            return Err("Action names must be unique, non-empty and at most 256 bytes".into());
        }
        if action.steps.len() > 10_000 {
            return Err(format!("Action '{}' has more than 10,000 steps", action.name));
        }
    }

    let existing = &app.session.actions.list;
    let mut reserved: std::collections::HashSet<String> = existing.iter().map(|a| a.name.clone()).chain(file.actions.iter().map(|a| a.name.clone())).collect();
    let mut mapped = std::collections::HashMap::<String, String>::new();
    let original: Vec<String> = file.actions.iter().map(|a| a.name.clone()).collect();
    // Plan *all* destination names first: a parent may precede the child it calls.
    let mut planned = Vec::new();
    for mut action in file.actions.drain(..) {
        let name = action.name.clone();
        let target = if let Some(found) = existing.iter().find(|a| a.name == name && a.steps == action.steps) {
            found.name.clone()
        } else if let Some(found) = existing.iter().find(|a| a.name.starts_with(format!("{name} (Imported ").as_str()) && a.steps == action.steps) {
            found.name.clone()
        } else if existing.iter().any(|a| a.name == name) {
            let mut n = 2usize;
            loop {
                let candidate = format!("{name} (Imported {n})");
                if reserved.insert(candidate.clone()) {
                    break candidate;
                }
                n += 1;
            }
        } else {
            name.clone()
        };
        mapped.insert(name, target.clone());
        action.name = target;
        planned.push(action);
    }

    // Rewrite nested calls against the final name table, including portable index references.
    let retarget = |action: &mut Action, mapping: &std::collections::HashMap<String, String>| -> Result<(), String> {
        for (id, params) in &mut action.steps {
            if id != "actions.play" {
                continue;
            }
            let original_name = match params.get("action") {
                Some(Value::String(name)) => name.clone(),
                Some(Value::Number(n)) if n.as_u64().is_some() => {
                    let index = usize::try_from(n.as_u64().unwrap()).map_err(|_| "Nested action index out of range")?;
                    original.get(index).cloned().ok_or_else(|| format!("Invalid nested action index {index}"))?
                }
                _ => continue,
            };
            if let Some(target) = mapping.get(&original_name) {
                params["action"] = json!(target);
            }
        }
        Ok(())
    };
    for action in &mut planned {
        retarget(action, &mapped)?;
    }

    // An existing action can become identical *after* its nested calls are
    // retargeted. Reuse it instead of appending a duplicate during re-import.
    let mut corrected = std::collections::HashMap::<String, String>::new();
    for (source_name, action) in original.iter().zip(&mut planned) {
        if let Some(found) = existing.iter().find(|a| {
            (a.name == *source_name || a.name.starts_with(format!("{source_name} (Imported ").as_str())) && a.steps == action.steps
        }) {
            corrected.insert(action.name.clone(), found.name.clone());
            action.name = found.name.clone();
        }
    }
    for action in &mut planned {
        for (id, params) in &mut action.steps {
            if id == "actions.play"
                && let Some(current) = params.get("action").and_then(Value::as_str)
                && let Some(correct) = corrected.get(current)
            {
                params["action"] = json!(correct);
            }
        }
    }
    let insert: Vec<_> = planned
        .into_iter()
        .filter(|action| !existing.iter().any(|a| a.name == action.name && a.steps == action.steps))
        .collect();
    let imported = insert.len();
    if imported > 0 {
        app.session.actions.list.extend(insert);
        app.session.actions.rev = app.session.actions.rev.saturating_add(1);
        app.ui.actions.selected_step = None;
        app.ui.actions.selected = Some(app.session.actions.list.len() - imported);
    }
    app.ui.status_error = false;
    app.ui.status = format!("Imported {imported} actions ({} already present)", original.len() - imported);
    Ok(json!({"imported": imported, "skipped": original.len() - imported}))
}

fn import_actions_dialog(app: &mut PhotocraftApp) -> Result<Value, String> {
    app.pick_file_bytes(|app, _, bytes| import_actions_bytes(app, &bytes))
}

fn export_actions_dialog(app: &mut PhotocraftApp) -> Result<Value, String> {
    let bytes = export_actions_bytes(app)?;
    let count = app.session.actions.list.len();
    app.pick_save("Actions.json", move |app, path| {
        let writer = app.services.write.as_mut().ok_or("no writer configured")?;
        writer(&path, &bytes)?;
        app.ui.status_error = false;
        app.ui.status = format!("Exported {count} actions to {}", crate::file_open::display_name(&path));
        Ok(json!({"path": path, "count": count}))
    })
}

fn begin_recording(app: &mut PhotocraftApp, append: bool) {
    if app.session.actions.recording.is_some() {
        return;
    }
    let params = if append { app.ui.actions.selected.map_or(json!({}), |i| json!({"action": i})) } else { json!({}) };
    let Ok(v) = app.run("actions.record", params) else { return };
    let Some(i) = v.get("index").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()) else { return };
    app.ui.actions.selected = Some(i);
    app.ui.actions.selected_step = None;
    if app.ui.actions.expanded.len() <= i {
        app.ui.actions.expanded.resize(i + 1, false);
    }
    app.ui.actions.expanded[i] = true;
}

pub(crate) fn report_play(app: &mut PhotocraftApp, v: &Value) {
    let Some(failed) = v.get("failed").filter(|f| f.is_object()) else {
        let ran = v.get("ran").and_then(Value::as_u64).unwrap_or(0);
        app.ui.status = format!("Played {ran} steps");
        app.ui.status_error = false;
        return;
    };
    let step = failed.get("step").and_then(Value::as_u64).unwrap_or(0);
    let id = failed.get("id").and_then(Value::as_str).unwrap_or("");
    let error = failed.get("error").and_then(Value::as_str).unwrap_or("");
    app.ui.status = format!("Step {} ({id}) failed: {error}", step + 1);
    app.ui.status_error = true;
}

fn visible_steps(app: &PhotocraftApp, action: usize) -> Vec<(String, Value)> {
    app.session.actions.list.get(action).into_iter().flat_map(|a| a.steps.iter()).chain(actions_cmds::pending_steps(&app.session, action)).cloned().collect()
}

fn step_label(id: &str, params: &Value) -> String {
    if id == "actions.play" {
        let target = params.get("action").map(|v| v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string())).unwrap_or_default();
        format!("{}: {target}", label_of(id))
    } else {
        label_of(id)
    }
}

#[derive(Clone, Copy)]
struct DragRow {
    action: usize,
    step: Option<usize>,
}

/// Drops use an insertion line, converted to the final index expected by actions.move.
fn drag_row(ui: &egui::Ui, response: &egui::Response, row: DragRow, t: &Tokens) -> Option<(DragRow, usize)> {
    response.dnd_set_drag_payload(row);
    let source = response.dnd_hover_payload::<DragRow>()?;
    if source.step.is_some() != row.step.is_some() || (source.step.is_some() && source.action != row.action) {
        return None;
    }
    let after = ui.input(|i| i.pointer.hover_pos().is_some_and(|p| p.y >= response.rect.center().y));
    let index = row.step.unwrap_or(row.action);
    let from = source.step.unwrap_or(source.action);
    let slot = index.saturating_add(usize::from(after));
    let to = if from < slot { slot.saturating_sub(1) } else { slot };
    let y = if after { response.rect.bottom() } else { response.rect.top() };
    ui.painter().line_segment([pos2(response.rect.left() + 22.0, y), pos2(response.rect.right(), y)], Stroke::new(2.0, t.accent));
    response.dnd_release_payload::<DragRow>().map(|source| (*source, to))
}

fn move_row(app: &mut PhotocraftApp, source: DragRow, to: usize) {
    let mut params = json!({"action": source.action, "to": to});
    if let Some(step) = source.step {
        params["step"] = json!(step);
    }
    if app.run("actions.move", params).is_err() {
        return;
    }
    if source.step.is_some() {
        app.ui.actions.selected = Some(source.action);
        app.ui.actions.selected_step = Some(to);
    } else {
        app.ui.actions.expanded.resize(app.session.actions.list.len(), false);
        if source.action < app.ui.actions.expanded.len() && to < app.ui.actions.expanded.len() {
            let expanded = app.ui.actions.expanded.remove(source.action);
            app.ui.actions.expanded.insert(to, expanded);
        }
        app.ui.actions.selected = Some(to);
        app.ui.actions.selected_step = None;
    }
}

fn delete_selection(app: &mut PhotocraftApp) {
    let Some(action) = app.ui.actions.selected else { return };
    let step = app.ui.actions.selected_step;
    let mut params = json!({"action": action});
    if let Some(step) = step {
        params["step"] = json!(step);
    }
    if app.run("actions.delete", params).is_err() {
        return;
    }
    // Clear selection after deletion so a second click cannot delete a different
    // step or unexpectedly fall back to deleting the entire action.
    app.ui.actions.selected = None;
    app.ui.actions.selected_step = None;
    if step.is_none() && action < app.ui.actions.expanded.len() {
        app.ui.actions.expanded.remove(action);
    }
}

pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let recording = app.session.actions.recording;
    let rows: Vec<_> = app.session.actions.list.iter().enumerate().map(|(i, a)| (a.name.clone(), visible_steps(app, i))).collect();
    if app
        .ui
        .actions
        .selected
        .is_some_and(|i| i >= rows.len() || app.ui.actions.selected_step.is_some_and(|step| rows.get(i).is_none_or(|row| step >= row.1.len())))
    {
        app.ui.actions.selected = None;
        app.ui.actions.selected_step = None;
    }
    if app.ui.actions.expanded.len() > rows.len() {
        app.ui.actions.expanded.truncate(rows.len());
    }
    let max_h = (ui.available_height() - 70.0).clamp(80.0, 320.0);
    let mut play_idx = None;
    let mut moved = None;
    let mut file_action = None;
    let recording_row = recording.and_then(|(_, i)| rows.get(i).map(|(_, steps)| (i, steps.len())));
    let reveal = recording_row.filter(|row| app.ui.actions.reveal_recording != Some(*row));
    app.ui.actions.reveal_recording = recording_row;
    if let Some((i, _)) = reveal {
        app.ui.actions.expanded.resize(rows.len(), false);
        if let Some(expanded) = app.ui.actions.expanded.get_mut(i) {
            *expanded = true;
        }
    }
    egui::ScrollArea::vertical().id_salt("actions-rows").max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
        if rows.is_empty() {
            ui.label(egui::RichText::new(tl!("Record ● a sequence of edits, then play ▶ it on any document.")).color(t.text_faint).size(11.5));
        }
        if egui::DragAndDrop::has_payload_of_type::<DragRow>(ui.ctx()) {
            let clip = ui.clip_rect();
            if let Some(p) = ui.input(|i| i.pointer.hover_pos()).filter(|p| clip.contains(*p)) {
                let delta = if p.y < clip.top() + 22.0 {
                    8.0
                } else if p.y > clip.bottom() - 22.0 {
                    -8.0
                } else {
                    0.0
                };
                if delta != 0.0 {
                    ui.scroll_with_delta(vec2(0.0, delta));
                    ui.ctx().request_repaint();
                }
            }
        }
        for (i, (name, steps)) in rows.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click_and_drag());
            if reveal == Some((i, 0)) {
                ui.scroll_to_rect(rect, Some(egui::Align::BOTTOM));
            }
            let sel = app.ui.actions.selected == Some(i) && app.ui.actions.selected_step.is_none();
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, sel, name));
            if sel {
                ui.painter().rect_filled(rect, 0.0, t.row_selected);
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.5));
            }
            if let Some(change) = drag_row(ui, &resp, DragRow { action: i, step: None }, &t) {
                moved = Some(change);
            }
            let expanded = app.ui.actions.expanded.get(i).copied().unwrap_or(false);
            let tri = Rect::from_center_size(pos2(rect.left() + 12.0, rect.center().y), vec2(8.0, 8.0));
            let pts =
                if expanded { vec![tri.left_top(), tri.right_top(), tri.center_bottom()] } else { vec![tri.left_top(), tri.right_center(), tri.left_bottom()] };
            ui.painter().add(egui::Shape::convex_polygon(pts, t.text_dim, Stroke::NONE));
            crate::icons::paint(ui, Rect::from_center_size(pos2(rect.left() + 30.0, rect.center().y), vec2(16.0, 16.0)), "play", 11.0, t.icon);
            let recording_this = recording.is_some_and(|(_, r)| r == i);
            let nsteps = steps.len();
            ui.painter().text(pos2(rect.left() + 44.0, rect.center().y), Align2::LEFT_CENTER, name, egui::FontId::proportional(12.0), t.text);
            ui.painter().text(
                pos2(rect.right() - 8.0, rect.center().y),
                Align2::RIGHT_CENTER,
                app.session
                    .prefs()
                    .shortcuts
                    .get(&shortcut_id(name))
                    .filter(|s| !s.is_empty())
                    .map_or_else(|| format!("{nsteps} steps"), |key| format!("{key} · {nsteps} steps")),
                egui::FontId::proportional(11.0),
                t.text_faint,
            );
            if recording_this {
                ui.painter().circle_filled(pos2(rect.right() - 64.0, rect.center().y), 4.0, Color32::from_rgb(230, 60, 60));
            }
            if resp.clicked() {
                app.ui.actions.selected = Some(i);
                app.ui.actions.selected_step = None;
                if resp.interact_pointer_pos().is_some_and(|p| p.x < rect.left() + 20.0) {
                    if app.ui.actions.expanded.len() <= i {
                        app.ui.actions.expanded.resize(i + 1, false);
                    }
                    app.ui.actions.expanded[i] = !expanded;
                }
            }
            resp.context_menu(|ui| {
                ui.label(tl!("Keyboard Shortcuts"));
                let current = app.session.prefs().shortcuts.get(&shortcut_id(name)).cloned().unwrap_or_default();
                for key in std::iter::once(String::new()).chain((1..=12).map(|n| format!("F{n}"))) {
                    let label = if key.is_empty() { tl!("None").to_string() } else { key.clone() };
                    if ui.selectable_label(current == key, label).clicked() {
                        let _ = assign_shortcut(app, name, &key);
                        ui.close();
                    }
                }
            });
            if resp.double_clicked() {
                play_idx = Some(i);
            }
            if expanded {
                for (step, (id, params)) in steps.iter().enumerate() {
                    let (r, response) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click_and_drag());
                    if reveal == Some((i, step + 1)) {
                        ui.scroll_to_rect(r, Some(egui::Align::BOTTOM));
                    }
                    let label = step_label(id, params);
                    let selected = app.ui.actions.selected == Some(i) && app.ui.actions.selected_step == Some(step);
                    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &label));
                    if selected {
                        ui.painter().rect_filled(r, 0.0, t.row_selected);
                    } else if response.hovered() {
                        ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.5));
                    }
                    if let Some(change) = drag_row(ui, &response, DragRow { action: i, step: Some(step) }, &t) {
                        moved = Some(change);
                    }
                    ui.painter().text(pos2(r.left() + 44.0, r.center().y), Align2::LEFT_CENTER, label, egui::FontId::proportional(11.5), t.text_dim);
                    if response.clicked() {
                        app.ui.actions.selected = Some(i);
                        app.ui.actions.selected_step = Some(step);
                    }
                }
            }
        }
    });
    if let Some((source, to)) = moved {
        move_row(app, source, to);
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let recording_now = app.session.actions.recording.is_some();
        if crate::icons::button(ui, "square", 24.0, false, tl!("Stop playing/recording")).clicked() && recording_now {
            let _ = app.run("actions.stop", json!({}));
        }
        let (r, rec) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
        if rec.hovered() {
            ui.painter().rect_filled(r, 3.0, t.hover);
        }
        ui.painter().circle_filled(r.center(), 5.5, if recording_now { Color32::from_rgb(230, 60, 60) } else { t.icon });
        if rec.on_hover_text(tl!("Begin recording")).clicked() && !recording_now {
            begin_recording(app, true);
        }
        if crate::icons::button(ui, "play", 24.0, false, tl!("Play selection")).clicked() {
            play_idx = app.ui.actions.selected;
        }
        if crate::icons::button(ui, "plus", 24.0, false, tl!("Create new action")).clicked() && !recording_now {
            begin_recording(app, false);
        }
        let can_delete = app.ui.actions.selected.is_some() && (!recording_now || app.ui.actions.selected_step.is_some());
        ui.add_enabled_ui(can_delete, |ui| {
            let response = crate::icons::button(ui, "trash", 24.0, false, tl!("Delete"));
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, can_delete, tl!("Delete")));
            if response.clicked() {
                delete_selection(app);
            }
        });
        ui.menu_button("⋯", |ui| {
            if ui.add_enabled(!recording_now, egui::Button::new("Import Actions…")).clicked() {
                file_action = Some("import");
                ui.close();
            }
            if ui.add_enabled(!rows.is_empty() && !recording_now, egui::Button::new("Export Actions…")).clicked() {
                file_action = Some("export");
                ui.close();
            }
        });
    });
    if let Some(file_action) = file_action {
        let result = match file_action {
            "import" => import_actions_dialog(app),
            _ => export_actions_dialog(app),
        };
        if let Err(e) = result && e != "cancelled" {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    if let Some(i) = play_idx
        && let Ok(v) = app.run("actions.play", json!({"action": i}))
    {
        report_play(app, &v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_reveals_new_action_and_latest_step_in_a_long_panel() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":4,"height":4})).unwrap();
        for i in 0..30 {
            app.session.actions.list.push(Action { name: format!("Existing {i}"), steps: vec![] });
        }
        let mut h = Harness::builder().with_size(vec2(320.0, 260.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(4);
        begin_recording(h.state_mut(), false);
        h.run_steps(20);
        let rect = h.get_by_label("Action 31").rect();
        assert!(rect.top() >= 0.0 && rect.bottom() <= 205.0, "new action visible: {rect:?}");
        for _ in 0..20 {
            h.state_mut().run("view.zoomOut", json!({})).unwrap();
        }
        h.state_mut().run("view.fitOnScreen", json!({})).unwrap();
        h.run_steps(20);
        let rect = h.get_by_label("Fit on Screen").rect();
        assert!(rect.top() >= 0.0 && rect.bottom() <= 205.0, "latest step visible: {rect:?}");
    }

    #[test]
    fn drag_steps_and_action_headers_reorders_and_keeps_selection() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.actions.list.push(Action {
            name: "First".into(),
            steps: vec![("view.zoomOut".into(), json!({})), ("view.fitOnScreen".into(), json!({})), ("view.actualPixels".into(), json!({}))],
        });
        app.session.actions.list.push(Action { name: "Second".into(), steps: vec![] });
        app.ui.actions.expanded = vec![true, false];
        let mut h = Harness::builder().with_size(vec2(320.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(4);
        fn drag(h: &mut Harness<'_, PhotocraftApp>, from: egui::Pos2, to: egui::Pos2) {
            h.event(egui::Event::PointerMoved(from));
            h.run_steps(1);
            h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
            h.event(egui::Event::PointerMoved(from + vec2(10.0, 0.0)));
            h.run_steps(2);
            h.event(egui::Event::PointerMoved(to));
            h.run_steps(2);
            h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
            h.run_steps(3);
        }
        let from = h.get_by_label("Zoom Out").rect().center();
        let target = h.get_by_label("100%").rect();
        drag(&mut h, from, pos2(target.center().x, target.bottom() - 2.0));
        assert_eq!(
            h.state().session.actions.list[0].steps.iter().map(|st| st.0.as_str()).collect::<Vec<_>>(),
            ["view.fitOnScreen", "view.actualPixels", "view.zoomOut"]
        );
        assert_eq!(h.state().ui.actions.selected_step, Some(2));
        let from = h.get_by_label("Second").rect().center();
        let target = h.get_by_label("First").rect();
        drag(&mut h, from, pos2(target.center().x, target.top() + 2.0));
        assert_eq!(h.state().session.actions.list[0].name, "Second");
        assert_eq!(h.state().ui.actions.expanded, [false, true]);
        assert_eq!(h.state().ui.actions.selected, Some(0));
    }

    #[test]
    fn nested_shell_actions_include_view_steps_and_propagate_failures() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":40,"height":40})).unwrap();
        app.session
            .actions
            .list
            .push(Action { name: "Child".into(), steps: vec![("layer.new.layer".into(), json!({})), ("view.fitOnScreen".into(), json!({}))] });
        app.run("actions.record", json!({"name":"Parent"})).unwrap();
        app.run("actions.play", json!({"action":"Child"})).unwrap();
        app.run("actions.stop", json!({})).unwrap();
        assert_eq!(app.session.actions.list[1].steps, [("actions.play".into(), json!({"action":"Child"}))]);
        assert_eq!(step_label("actions.play", &json!({"action":"Child"})), "Play Action: Child");
        app.run("view.actualPixels", json!({})).unwrap();
        let r = app.run("actions.play", json!({"action":"Parent"})).unwrap();
        assert!(r.get("failed").is_none(), "{r}");
        assert!(app.ui.views[0].fit_pending);
        app.session.authorize =
            Some(|id, _| if id == "view.fitOnScreen" { Err(photocraft_engine::EngineError::Other("denied nested view".into())) } else { Ok(()) });
        let r = app.run("actions.play", json!({"action":"Parent"})).unwrap();
        assert!(r["failed"]["error"].as_str().unwrap().contains("denied nested view"));
        assert!(app.session.actions.playback_stack.is_empty());
        assert_eq!(app.session.actions.playing, 0);
    }

    #[test]
    fn panel_shows_pending_steps_and_trash_deletes_only_the_clicked_step() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
        begin_recording(&mut app, false);
        app.run("view.fitOnScreen", json!({})).unwrap();
        app.run("view.zoomOut", json!({})).unwrap();
        let mut h = Harness::builder().with_size(vec2(320.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(2);
        assert!(h.state().session.actions.recording.is_some());
        h.get_by_label("Fit on Screen");
        h.get_by_label("Zoom Out").click();
        h.run_steps(2);
        assert_eq!(h.state().ui.actions.selected_step, Some(1));
        h.get_by_label("Delete").click();
        h.run_steps(2);
        assert_eq!(visible_steps(h.state(), 0).iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["view.fitOnScreen"]);
        assert_eq!(h.state().session.actions.list.len(), 1);
        assert!(h.state().session.actions.recording.is_some());
        assert!(h.state().ui.actions.selected.is_none());
        delete_selection(h.state_mut());
        assert_eq!(h.state().session.actions.list.len(), 1, "repeated trash cannot delete the action");
        h.state_mut().run("view.zoomIn", json!({})).unwrap();
        h.run_steps(2);
        h.get_by_label("Zoom In");
        h.state_mut().run("actions.stop", json!({})).unwrap();
        h.run_steps(2);
        assert_eq!(visible_steps(h.state(), 0).iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["view.fitOnScreen", "view.zoomIn"]);
        h.get_by_label("Zoom In").click();
        h.run_steps(2);
        h.get_by_label("Delete").click();
        h.run_steps(2);
        assert_eq!(visible_steps(h.state(), 0).iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["view.fitOnScreen"]);
    }

    #[test]
    fn view_menu_commands_record_append_and_replay_after_resize() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 160, "height": 100})).unwrap();
        begin_recording(&mut app, false);
        app.run("image.canvasSize", json!({"width": 80, "height": 60})).unwrap();
        app.run("actions.stop", json!({})).unwrap();
        begin_recording(&mut app, true);
        crate::menus::invoke(&mut app, &ctx, "view.fitOnScreen", json!({})).unwrap();
        app.run("actions.stop", json!({})).unwrap();
        assert_eq!(app.session.actions.list.len(), 1);
        assert_eq!(app.session.actions.list[0].steps.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["image.canvasSize", "view.fitOnScreen"]);
        assert_eq!(label_of("view.fitOnScreen"), "Fit on Screen");
        // Persisted actions retain the menu step, and playback fits the new document.
        let saved = serde_json::to_string(&app.session.actions.list).unwrap();
        app.session.actions.list = serde_json::from_str(&saved).unwrap();
        app.run("file.new", json!({"width": 200, "height": 140})).unwrap();
        app.run("view.actualPixels", json!({})).unwrap();
        let result = app.run("actions.play", json!({"action": 0})).unwrap();
        assert_eq!(result["ran"], 2);
        assert!(result.get("failed").is_none(), "{result}");
        let idx = app.session.active_index().unwrap();
        assert!(app.ui.views[idx].fit_pending);
        let doc = &app.session.active().unwrap().doc;
        crate::canvas::fit_view(&mut app.ui.views[idx], doc, vec2(100.0, 100.0));
        assert_eq!(app.ui.views[idx].zoom, 0.75);
        assert_eq!(app.ui.views[idx].center, [40.0, 30.0]);
        assert!(!app.ui.views[idx].fit_pending);
    }

    #[test]
    fn view_action_failure_resume_and_authorization() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session
            .actions
            .list
            .push(Action { name: "Fit".into(), steps: vec![("view.fitOnScreen".into(), json!({})), ("view.actualPixels".into(), json!({}))] });
        let result = app.run("actions.play", json!({"action": "Fit"})).unwrap();
        assert_eq!(result["ran"], 0);
        assert_eq!(result["failed"]["step"], 0);
        assert_eq!(app.session.actions.playing, 0);
        app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
        app.session.authorize =
            Some(
                |id, _| {
                    if id == "view.fitOnScreen" { Err(photocraft_engine::EngineError::BadParams { cmd: id.into(), msg: "denied view".into() }) } else { Ok(()) }
                },
            );
        let result = app.run("actions.play", json!({"action": 0})).unwrap();
        assert_eq!(result["ran"], 0);
        assert!(result["failed"]["error"].as_str().unwrap().contains("denied view"));
        let result = app.run("actions.play", json!({"action": 0, "from": 1})).unwrap();
        assert_eq!(result["ran"], 1);
        assert!(!app.ui.views[0].fit_pending);
        assert!(app.run("actions.play", json!({"action": 0, "from": -1})).is_err());
        app.session.actions.playing = 1;
        assert!(app.run("actions.play", json!({"action": 0})).is_err());
    }

    #[test]
    fn record_and_replay_on_another_document() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
        app.sync_views();
        app.run("actions.record", json!({})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let _ = app.run("document.pixel", json!({"x": 1, "y": 1}));
        app.run("actions.stop", json!({})).unwrap();
        let a = &app.session.actions.list[0];
        assert_eq!(a.steps.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["layer.new.layer", "select.rect", "edit.fill"]);
        app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
        app.sync_views();
        let played = app.run("actions.play", json!({"action": 0})).unwrap();
        assert_eq!(played["ran"], 3);
        assert!(played.get("failed").is_none(), "{played}");
        let d = &app.session.active().unwrap().doc;
        assert_eq!(d.layers.len(), 2);
        assert_eq!(d.layers[1].surface().unwrap().pixel(5, 5), vec![1.0, 0.0, 0.0, 1.0]);
    }
    #[test]
    fn actions_file_round_trip_and_reimport_is_idempotent() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.actions.list.push(Action { name: "Draw".into(), steps: vec![("layer.new.layer".into(), json!({}))] });
        let bytes = export_actions_bytes(&app).unwrap();
        app.session.actions.list.clear();
        let r = import_actions_bytes(&mut app, &bytes).unwrap();
        assert_eq!(r["imported"], 1);
        assert_eq!(app.session.actions.list[0].name, "Draw");
        assert_eq!(app.session.actions.list[0].steps, vec![("layer.new.layer".into(), json!({}))]);
        let r = import_actions_bytes(&mut app, &bytes).unwrap();
        assert_eq!(r["imported"], 0);
        assert_eq!(app.session.actions.list.len(), 1);
    }

    #[test]
    fn import_renames_conflicts_and_retargets_nested_calls() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.actions.list.push(Action { name: "Child".into(), steps: vec![] });
        let imported = actions_cmds::ActionsFile {
            version: 1,
            actions: vec![
                Action { name: "Child".into(), steps: vec![("layer.new.layer".into(), json!({}))] },
                Action { name: "Parent".into(), steps: vec![("actions.play".into(), json!({"action": 0}))] },
                Action { name: "Named".into(), steps: vec![("actions.play".into(), json!({"action":"Child"}))] },
            ],
        };
        let bytes = serde_json::to_vec(&imported).unwrap();
        assert_eq!(import_actions_bytes(&mut app, &bytes).unwrap()["imported"], 3);
        assert_eq!(app.session.actions.list[1].name, "Child (Imported 2)");
        assert_eq!(app.session.actions.list[2].steps[0].1["action"], "Child (Imported 2)");
        assert_eq!(app.session.actions.list[3].steps[0].1["action"], "Child (Imported 2)");
        let rev = app.session.actions.rev;
        assert_eq!(import_actions_bytes(&mut app, &bytes).unwrap()["imported"], 0);
        assert_eq!(app.session.actions.rev, rev, "no writes on repeated import");
    }

    #[test]
    fn malformed_action_files_leave_existing_actions_untouched() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.actions.list.push(Action { name: "Keep".into(), steps: vec![] });
        for data in [b"not json".as_slice(), br#"{"version":2,"actions":[]}"#, br#"{"version":1,"actions":[{"name":"A"},{"name":"A"}]}"#] {
            assert!(import_actions_bytes(&mut app, data).is_err());
            assert_eq!(app.session.actions.list.len(), 1);
            assert_eq!(app.session.actions.list[0].name, "Keep");
        }
        app.session.actions.recording = Some((0, 0));
        assert!(import_actions_bytes(&mut app, b"{}").is_err());
        assert_eq!(app.session.actions.list.len(), 1);
    }


}
