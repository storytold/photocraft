//! Actions panel: record and replay command sequences.
//!
//! The list and the recording flag live on the engine session (`actions.record` / `stop` /
//! `play` / `list` / `get` / `delete` / `move` / `rename`), so the panel, the CLI and MCP share them.
//! The rows, the footer and the context menus are the framework's [`ActionsPanel`]; this module
//! keeps the panel's state, maps its events to engine commands ([`apply_actions_events`]) and holds
//! the selection rules Batch, Droplet and Play share.

use craft::automation::ui::actions::{
    ActionMove, ActionPlace, ActionSelection, ActionsEvent, ActionsPanel, ActionsPanelState, ActionsSettings, DIALOG_COLUMN, INCLUDE_COLUMN,
};
use craft::ui::contract::{SweepStep, ToggleEdit};
use photocraft_engine::actions_cmds::ActionsExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use photocraft_engine::actions_cmds::{self, Action};

use crate::PhotocraftApp;

use host::PhotoActions;

pub(crate) mod dialogs;
mod files;
mod host;
pub(crate) mod library;
pub(crate) mod menu;
pub(crate) mod menu_item;
pub(crate) mod playback;
pub(crate) mod record_again;
pub(crate) mod runner;

#[cfg(test)]
pub(crate) use runner::play_targets;
pub(crate) use runner::{
    Gate, PausedPlaybacks, abandoned, authorize_owned, authorize_panel_preview, check_editor, is_editor, journal_view, open_recorded_editor, play, report_play,
    resume, resume_spelling, run_step_dialog, spelling_change, stopped, suspended_guard, token_of,
};

#[cfg(test)]
mod drag_drop_tests;
#[cfg(test)]
mod files_tests;
#[cfg(test)]
mod library_tests;
#[cfg(test)]
mod menu_tests;
#[cfg(test)]
mod panel_menu_tests;
#[cfg(test)]
mod playback_tests;
#[cfg(test)]
mod record_again_tests;
#[cfg(test)]
mod runner_tests;
#[cfg(test)]
mod stop_tests;
#[cfg(test)]
mod tests;

/// The Actions panel's state: the framework panel's selection, expansion and settings. The action
/// list itself is [`ActionsExt::actions`](photocraft_engine::actions_cmds::ActionsExt::actions). Only Button Mode and the collapsed sets
/// are saved (see [`saved`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "saved::Saved", into = "saved::Saved")]
pub struct ActionsUi {
    /// What the framework panel keeps between passes.
    pub panel: ActionsPanelState,
    /// The action whose in-place rename the next pass opens (the panel menu's Rename), taken once.
    rename: Option<usize>,
}

/// The saved form of [`ActionsUi`], with the keys earlier saved states wrote.
mod saved {
    use serde::{Deserialize, Serialize};

    use super::{ActionsSettings, ActionsUi};

    /// Button Mode and the collapsed sets.
    #[derive(Default, Serialize, Deserialize)]
    #[serde(default)]
    pub(super) struct Saved {
        pub(super) button_mode: bool,
        pub(super) collapsed_sets: Vec<String>,
    }

    impl From<Saved> for ActionsUi {
        fn from(saved: Saved) -> Self {
            let mut ui = ActionsUi::default();
            ui.panel.settings = ActionsSettings::new(saved.button_mode, saved.collapsed_sets);
            ui
        }
    }

    impl From<ActionsUi> for Saved {
        fn from(ui: ActionsUi) -> Self {
            Saved { button_mode: ui.panel.settings.button_mode, collapsed_sets: ui.panel.settings.collapsed_sets }
        }
    }
}

pub(crate) fn label_of(id: &str) -> String {
    photocraft_engine::commands::find(id)
        .map(|c| c.label)
        .or_else(|| crate::menus::UI_COMMANDS.iter().find(|c| c.0 == id).map(|c| c.1))
        // Shell menu items recorded as steps, like Save a Copy (#4373).
        .or_else(|| crate::menu_catalog::CATALOG.iter().find(|c| c.3 == id).map(|c| c.1))
        .map(|label| label.trim_end_matches('…').to_string())
        .unwrap_or_else(|| id.to_string())
}

/// The action a selected row belongs to: the action itself or the action of a selected step.
fn selected_action_index(app: &PhotocraftApp) -> Option<usize> {
    match app.ui.actions.panel.selected.as_ref()? {
        ActionSelection::Action(i) | ActionSelection::Step(i, _) => Some(*i).filter(|i| *i < app.session.actions().list.actions().len()),
        _ => None,
    }
}

/// The selected action, else the first action of the selected set (Batch and Create Droplet start
/// on the panel's selection), else the first one.
pub fn selected_action(app: &PhotocraftApp) -> Option<&Action> {
    let list = &app.session.actions().list;
    selected_action_index(app)
        .and_then(|i| list.action(i))
        .or_else(|| selected_set_actions(app).first().and_then(|i| list.action(*i)))
        .or_else(|| list.action(0))
}

/// The Set and Action dropdowns of the Batch and Create Droplet dialogs, as in the reference
/// application: each action name once and each set once, in list order. A loaded library can hold
/// one name in two sets, so the dialog sends the set too and the command picks that set's action.
pub fn action_form_choices(app: &PhotocraftApp) -> (Vec<String>, Vec<String>) {
    let (mut names, mut sets) = (Vec::new(), Vec::new());
    for action in app.session.actions().list.actions() {
        if !names.contains(&action.name) {
            names.push(action.name.clone());
        }
        if let Some(set) = action.set.as_ref().filter(|set| !sets.contains(*set)) {
            sets.push(set.clone());
        }
    }
    (names, sets)
}

/// The selected set folder, if it still exists.
pub(crate) fn selected_set(app: &PhotocraftApp) -> Option<&str> {
    match app.ui.actions.panel.selected.as_ref()? {
        ActionSelection::Set(name) if app.session.actions().list.sets().contains(&name.as_str()) => Some(name),
        _ => None,
    }
}

/// The actions of `set`, in list order (empty for an empty or unknown set).
fn set_actions(app: &PhotocraftApp, set: &str) -> Vec<usize> {
    let list = &app.session.actions().list;
    (0..list.actions().len()).filter(|i| list.set_of(*i) == Some(set)).collect()
}

/// The actions of the selected set, in list order (empty when no set is selected, or it is empty).
fn selected_set_actions(app: &PhotocraftApp) -> Vec<usize> {
    selected_set(app).map(|set| set_actions(app, set)).unwrap_or_default()
}

#[cfg(test)]
/// What Play plays: the selected action; with a set folder selected, every action of that set in
/// order, as Photoshop plays a selected set (nothing for an empty set); else the first action.
pub(crate) fn playback_targets(app: &PhotocraftApp) -> Vec<usize> {
    if selected_set(app).is_some() {
        return selected_set_actions(app);
    }
    playback_selection(app).into_iter().collect()
}

#[cfg(test)]
/// Index of the playback target. Photoshop's Play button uses the first action when
/// the panel has actions but none of its rows is selected.
fn playback_selection(app: &PhotocraftApp) -> Option<usize> {
    if app.session.actions().list.actions().is_empty() { None } else { Some(selected_action_index(app).unwrap_or(0)) }
}

/// Steps in the `[[id, params], …]` shape batch and droplets already accept, without the
/// steps switched off in the panel.
pub fn action_steps(action: &Action) -> Vec<Value> {
    action.enabled_steps().map(|(_, step)| json!([step.command, step.params])).collect()
}

/// Named action bindings use the existing persisted shortcut overrides. Keeping the
/// name in the key means selecting or deleting another row cannot retarget F6.
pub(crate) const SHORTCUT_PREFIX: &str = actions_cmds::PLAY_SHORTCUT_PREFIX;

pub(crate) fn shortcut_id(name: &str) -> String {
    format!("{SHORTCUT_PREFIX}{name}")
}

pub(crate) fn assign_shortcut(app: &mut PhotocraftApp, name: &str, shortcut: &str) -> Result<Value, String> {
    use photocraft_engine::prefs::normalize_shortcut;
    let id = shortcut_id(name);
    let mut set = serde_json::Map::new();
    let mut alternates = serde_json::Map::new();
    // A function key belongs to only one action, as its main key or an alternate, however the
    // key was spelled. Ordinary menu bindings remain intact and become available again when the
    // action binding is removed.
    if let Some(key) = normalize_shortcut(shortcut) {
        let same = |assigned: &String| normalize_shortcut(assigned).as_ref() == Some(&key);
        let prefs = app.session.prefs();
        for (other, assigned) in &prefs.shortcuts {
            if other.starts_with(SHORTCUT_PREFIX) && other != &id && same(assigned) {
                set.insert(other.clone(), json!(""));
            }
        }
        for (other, assigned) in &prefs.alternate_shortcuts {
            if other.starts_with(SHORTCUT_PREFIX) && other != &id && assigned.iter().any(same) {
                alternates.insert(other.clone(), json!(assigned.iter().filter(|a| !same(a)).collect::<Vec<_>>()));
            }
        }
    }
    set.insert(id, json!(shortcut));
    app.run("edit.keyboardShortcuts", json!({"set": set, "alternates": alternates, "allowUnknown": true, "removeConflicts": false}))
}

/// After Clear All, Reset, Replace or their undo the rows hold other actions: none stays
/// selected, open or in rename.
pub(crate) fn library_changed(app: &mut PhotocraftApp) {
    let ui = &mut app.ui.actions;
    ui.panel.selected = None;
    ui.panel.expanded.clear();
    ui.rename = None;
}

/// The set New Action records into: the selected action's, else the selected set folder's.
pub(crate) fn new_action_set(app: &PhotocraftApp) -> Option<String> {
    match selected_action_index(app) {
        Some(i) => app.session.actions().list.set_of(i).map(str::to_owned),
        None => selected_set(app).map(str::to_owned),
    }
}

/// Start recording: into action `into`, or into a new action in [`new_action_set`] when `None`.
/// The recorded action is selected and shown open, with its set open.
pub(crate) fn begin_recording(app: &mut PhotocraftApp, into: Option<usize>) {
    if app.session.recording.target.is_some() {
        return;
    }
    let params = match into {
        Some(i) => json!({"action": i}),
        None => new_action_set(app).map_or(json!({}), |set| json!({"set": set})),
    };
    let Ok(v) = app.run("actions.record", params) else { return };
    let Some(i) = v.get("index").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()) else { return };
    let Some(action) = app.session.actions().list.action(i) else { return };
    let (name, set) = (action.name.clone(), action.set.clone());
    let panel = &mut app.ui.actions.panel;
    panel.selected = Some(ActionSelection::Action(i));
    panel.expanded.insert(name);
    // Its set too, in case its folder is collapsed.
    if let Some(set) = set {
        panel.settings.collapsed_sets.retain(|c| *c != set);
    }
}

/// Duplicate one action or step and select the copy. A copied action is open when its source is;
/// the action of a copied step opens.
fn duplicate_row(app: &mut PhotocraftApp, action: usize, step: Option<usize>) -> Result<Value, String> {
    let source_open = app.session.actions().list.action(action).is_some_and(|a| app.ui.actions.panel.expanded.contains(&a.name));
    let result = app.run("actions.duplicate", json!({"action": action, "step": step}))?;
    let index = result["index"].as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("the copy has no action index")?;
    let copied_step = result.get("step").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
    let name = app.session.actions().list.action(index).map(|a| a.name.clone());
    let panel = &mut app.ui.actions.panel;
    panel.selected = Some(match copied_step {
        Some(s) => ActionSelection::Step(index, s),
        None => ActionSelection::Action(index),
    });
    if let Some(name) = name
        && (step.is_some() || source_open)
    {
        panel.expanded.insert(name);
    }
    Ok(result)
}

/// Delete one action or step. False when the engine refused.
fn delete_row(app: &mut PhotocraftApp, action: usize, step: Option<usize>) -> bool {
    let mut params = json!({"action": action});
    if let Some(step) = step {
        params["step"] = json!(step);
    }
    app.run("actions.delete", params).is_ok()
}

fn rename_action(app: &mut PhotocraftApp, index: usize, name: String) {
    if let Err(e) = app.run("actions.rename", json!({"action": index, "name": name})) {
        let template = "Could not rename the action: {error}";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e)]);
        app.ui.status_error = true;
    }
}

/// Draws the Actions panel and runs what the user did in it.
pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let rename = app.ui.actions.rename.take();
    let paced = playback::pending(app);
    let host = PhotoActions::new(&app.session, paced);
    let mut widget = ActionsPanel::new(&mut app.ui.actions.panel, &app.session.actions().list).host(&host);
    if let Some(action) = rename {
        widget = widget.start_rename(action);
    }
    let out = widget.show(ui);
    apply_actions_events(app, out.events);
}

/// Runs the events of the panel, its footer, its row menus and its panel menu, in order. An error
/// other than a cancelled dialog goes to the status bar. A pass applies only the events it was
/// given.
pub(crate) fn apply_actions_events(app: &mut PhotocraftApp, events: Vec<ActionsEvent>) {
    for event in events {
        if let Err(e) = apply_event(app, event)
            && e != "cancelled"
        {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// Plays `targets` from step `from` and reports the result in the status bar.
fn play_and_report(app: &mut PhotocraftApp, targets: &[usize], from: usize) {
    match runner::play_targets(app, targets, from, 0) {
        Ok(Some(v)) => runner::report_play(app, &v),
        Ok(None) => {}
        Err(e) => {
            let template = "Action playback failed: {error}";
            app.ui.status = crate::i18n::fmt(tl!(template), &[("error", &e)]);
            app.ui.status_error = true;
        }
    }
}

/// The action and the step position after the selected row, for inserting a step.
fn insert_target(app: &PhotocraftApp) -> Result<(usize, Option<usize>), String> {
    match app.ui.actions.panel.selected.as_ref() {
        Some(ActionSelection::Action(i)) if *i < app.session.actions().list.actions().len() => Ok((*i, None)),
        Some(ActionSelection::Step(i, s)) if *i < app.session.actions().list.actions().len() => Ok((*i, Some(s.saturating_add(1)))),
        _ => Err(tl!("Select an action first.").to_string()),
    }
}

fn apply_event(app: &mut PhotocraftApp, event: ActionsEvent) -> Result<(), String> {
    match event {
        ActionsEvent::Play { action, from, .. } => play_and_report(app, &[action], from),
        ActionsEvent::PlaySet(set) => {
            let targets = set_actions(app, &set);
            play_and_report(app, &targets, 0);
        }
        ActionsEvent::BeginRecording { into, .. } => begin_recording(app, into),
        ActionsEvent::NewAction => begin_recording(app, None),
        ActionsEvent::StopRecording => {
            if !playback::stop(app) && app.session.recording.target.is_some() {
                app.run("actions.stop", json!({}))?;
            }
        }
        ActionsEvent::NewSet => dialogs::new_set_dialog(app).map(drop)?,
        ActionsEvent::InsertStop => dialogs::insert_stop_dialog(app).map(drop)?,
        ActionsEvent::Load => files::load_dialog(app).map(drop)?,
        ActionsEvent::ClearAll => library::clear_all(app).map(drop)?,
        ActionsEvent::Reset => library::reset(app).map(drop)?,
        ActionsEvent::Replace => library::replace(app).map(drop)?,
        ActionsEvent::Save { set, .. } => files::save_dialog(app, set).map(drop)?,
        ActionsEvent::Delete(row) => {
            let deleted = match row {
                ActionSelection::Action(i) => delete_row(app, i, None),
                ActionSelection::Step(i, s) => delete_row(app, i, Some(s)),
                // Deleting a whole set is not offered.
                _ => false,
            };
            if deleted {
                // The panel cannot tell which rows the delete removed.
                app.ui.actions.panel.selected = None;
            }
        }
        ActionsEvent::Duplicate(row) => match row {
            ActionSelection::Action(i) => duplicate_row(app, i, None).map(drop)?,
            ActionSelection::Step(i, s) => duplicate_row(app, i, Some(s)).map(drop)?,
            ActionSelection::Set(set) => {
                let result = app.run("actions.duplicate", json!({"set": set}))?;
                if let Some(copy) = result["set"].as_str() {
                    app.ui.actions.panel.selected = Some(ActionSelection::Set(copy.to_owned()));
                }
            }
            _ => {}
        },
        ActionsEvent::Rename { action, text, .. } => rename_action(app, action, text),
        ActionsEvent::SetColor { action, color, .. } => {
            app.run("actions.setColor", json!({"action": action, "color": color.name()}))?;
        }
        ActionsEvent::AssignShortcut { action, choice, .. } => {
            let name = app.session.actions().list.action(action).map(|a| a.name.clone()).ok_or("no such action")?;
            let key = choice.map_or_else(String::new, |c| photocraft_engine::atn_cmds::function_key_name(u16::from(c.key), c.shift, c.command));
            assign_shortcut(app, &name, &key)?;
        }
        ActionsEvent::Reorder(ActionMove { row, place, .. }) => reorder(app, row, place)?,
        ActionsEvent::Toggle(SweepStep::Flip(edit) | SweepStep::Restore(edit)) => toggle_cell(app, &edit)?,
        ActionsEvent::RecordAgain { action, step, .. } => record_again::open(app, action, step).map(drop)?,
        ActionsEvent::InsertMenuItem => {
            let (action, at) = insert_target(app)?;
            menu_item::open(app, action, at).map(drop)?;
        }
        ActionsEvent::InsertPath => {
            let (action, at) = insert_target(app)?;
            app.run("actions.insertPath", json!({"action": action, "at": at, "name": app.ui.selected_path}))?;
        }
        ActionsEvent::PlaybackOptions => playback::open_dialog(app).map(drop)?,
        ActionsEvent::ToolRecording(on) => {
            app.run("actions.allowToolRecording", json!({"enabled": on}))?;
        }
        // Set Options and Batch are not offered; ShowOnly and its undo never arrive, the panel
        // reports per-cell Flip and Restore steps.
        _ => {}
    }
    Ok(())
}

/// A drop of an action (into a set or out of one) or of a step (within its action). A step has no
/// place in another action: the engine moves steps only inside their action. `index` is the final
/// position `actions.move` takes.
fn reorder(app: &mut PhotocraftApp, row: ActionSelection, place: ActionPlace) -> Result<(), String> {
    match (row, place) {
        (ActionSelection::Action(action), ActionPlace::Action { index, set, .. }) => {
            app.run("actions.move", json!({"action": action, "to": index}))?;
            let current = app.session.actions().list.set_of(index).map(str::to_owned);
            if current != set {
                app.run("actions.assignSet", json!({"action": index, "set": set}))?;
            }
        }
        (ActionSelection::Step(action, step), ActionPlace::Step { action: into, index, .. }) if action == into => {
            app.run("actions.move", json!({"action": action, "step": step, "to": index}))?;
        }
        _ => {}
    }
    Ok(())
}

/// Sets the cell a sweep step names: a step's include or dialog cell, or every step of an action
/// for its include cell.
fn toggle_cell(app: &mut PhotocraftApp, edit: &ToggleEdit<ActionSelection>) -> Result<(), String> {
    let include = edit.column == INCLUDE_COLUMN;
    if !include && edit.column != DIALOG_COLUMN {
        return Ok(());
    }
    match &edit.key {
        ActionSelection::Step(action, step) => set_step_cell(app, *action, *step, include, edit.on),
        ActionSelection::Action(action) if include => app.run("actions.toggle", json!({"action": action, "enabled": edit.on})).map(drop),
        _ => Ok(()),
    }
}

/// Sets one step's include (`include`) or dialog flag to `on`. With `enabled` the engine commands
/// set the flag; without it they flip it.
fn set_step_cell(app: &mut PhotocraftApp, action: usize, step: usize, include: bool, on: bool) -> Result<(), String> {
    let command = if include { "actions.toggle" } else { "actions.dialog" };
    app.run(command, json!({"action": action, "step": step, "enabled": on})).map(drop)
}
