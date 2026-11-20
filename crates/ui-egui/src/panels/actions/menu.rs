//! The Actions panel's menu: the rows the app offers, their states and the commands they fire.

use craft::automation::ui::actions::{ActionsMenuAction, ActionsMenuFacts, ActionsPanelCommands};
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::{CommandRef, CommandSet as _, CommandState};
#[cfg(not(target_arch = "wasm32"))]
use craft::ui::runtime::env::Label;
use photocraft_engine::actions_edit_cmds;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;

use crate::PhotocraftApp;

/// Loads one `.atn` from the configured Actions folder. The path rides on the command.
#[cfg(not(target_arch = "wasm32"))]
const FOLDER_LOAD: &str = "ui.actionsFolder.load";

/// What the app knows that the menu's states depend on.
fn facts(app: &PhotocraftApp) -> ActionsMenuFacts {
    let path_selected = app.ui.selected_path.as_deref().is_some_and(|name| actions_edit_cmds::selected_recording_path(&app.session, name).is_ok());
    ActionsMenuFacts::new()
        .recording(app.session.actions.recording.is_some())
        .path_selected(path_selected)
        .waiting(super::playback::pending(app))
        .tool_recording(app.session.actions.record_tools)
}

/// Whether the app offers `command`: Set Options…, Batch… and Button Mode have no counterpart here.
fn offered(command: ActionsPanelCommands) -> bool {
    !matches!(command, ActionsPanelCommands::SetOptions | ActionsPanelCommands::Batch | ActionsPanelCommands::ButtonMode)
}

/// The rows of the panel menu, in the framework's groups.
pub(crate) fn panel_rows(app: &PhotocraftApp) -> Vec<PanelRow> {
    let mut rows = craft::automation::ui::actions::panel_rows(facts(app), &offered);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let files = crate::actions_folder::files(app);
        if !files.is_empty() {
            let items: Vec<PanelRow> = files
                .iter()
                .map(|path| PanelRow::Command(CommandRef::with_params(FOLDER_LOAD, json!({ "path": path }))))
                .collect();
            rows.push(PanelRow::Separator);
            rows.push(PanelRow::Submenu(Label::from(tl!("Actions Folder")), items));
        }
    }
    rows
}

/// The state of every command the rows name; they come from the same list, so a state never exists for a row the
/// menu does not show.
pub(crate) fn states(app: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    let facts = facts(app);
    let (panel, list) = (&app.ui.actions.panel, &app.session.actions.list);
    let mut states: Vec<(CommandRef, CommandState)> = panel_rows(app)
        .into_iter()
        .filter_map(|row| match row {
            PanelRow::Command(command) => {
                let state = ActionsPanelCommands::from_ref(&command)?.state(panel, list, facts);
                Some((command, state))
            }
            _ => None,
        })
        .collect();
    #[cfg(not(target_arch = "wasm32"))]
    states.extend(folder_states(app));
    states
}

/// States for the Actions Folder file rows, which are not framework commands.
#[cfg(not(target_arch = "wasm32"))]
fn folder_states(app: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    crate::actions_folder::files(app)
        .iter()
        .map(|path| {
            let title = crate::file_open::display_name(path);
            (CommandRef::with_params(FOLDER_LOAD, json!({ "path": path })), CommandState::new().enabled(true).title(title))
        })
        .collect()
}

/// Runs a fired command of the Actions panel; `false` for a command of another set. Its event goes through the one
/// mapping that every Actions event takes.
pub(crate) fn run_fired(app: &mut PhotocraftApp, _ctx: &egui::Context, command: &CommandRef) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    if command.id == FOLDER_LOAD {
        load_folder_file(app, command);
        return true;
    }
    let Some(command) = ActionsPanelCommands::from_ref(command) else { return false };
    // A command the app does not offer (Button Mode answers `SettingsChanged`) has nothing to run.
    if !offered(command) {
        return true;
    }
    let facts = facts(app);
    if let Some(ActionsMenuAction::Event(event)) = command.run(&mut app.ui.actions.panel, &app.session.actions.list, facts) {
        super::apply_actions_events(app, vec![event]);
    }
    true
}

/// Loads the `.atn` named by an Actions Folder menu row.
#[cfg(not(target_arch = "wasm32"))]
fn load_folder_file(app: &mut PhotocraftApp, command: &CommandRef) {
    let Some(path) = command.params.get("path").and_then(|value| value.as_str()).filter(|path| !path.is_empty()) else {
        app.ui.status = "expected an action file".into();
        app.ui.status_error = true;
        return;
    };
    if let Err(error) = super::files::load_source(app, path, json!({ "path": path })) {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}
