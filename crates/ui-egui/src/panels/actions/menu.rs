//! The Actions panel's menu: the rows the app offers, their states and the commands they fire.

use craft::automation::ui::actions::{ActionsMenuAction, ActionsMenuFacts, ActionsPanelCommands};
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::{CommandRef, CommandSet as _, CommandState};
use photocraft_engine::actions_edit_cmds;

use crate::PhotocraftApp;

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
    craft::automation::ui::actions::panel_rows(facts(app), &offered)
}

/// The state of every command the rows name; they come from the same list, so a state never exists for a row the
/// menu does not show.
pub(crate) fn states(app: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    let facts = facts(app);
    let (panel, list) = (&app.ui.actions.panel, &app.session.actions.list);
    // Load Actions… is the one library row the framework leaves always on. The app's own
    // `actions.load` refuses while a recording or a paced playback holds the library (a load would
    // replace the set the recording writes into), and the reference application greys the row
    // beside Clear All, Reset and Replace while recording. Grey it with the rule the command
    // itself checks, so the menu never offers a row that can only fail.
    let busy = facts.recording || facts.waiting;
    panel_rows(app)
        .into_iter()
        .filter_map(|row| match row {
            PanelRow::Command(command) => {
                let row_command = ActionsPanelCommands::from_ref(&command)?;
                let state = row_command.state(panel, list, facts);
                let state = if row_command == ActionsPanelCommands::Load { state.enabled(!busy) } else { state };
                Some((command, state))
            }
            _ => None,
        })
        .collect()
}

/// What [`states`] reads: the list, the menu's facts and the panel's state.
#[derive(craft::ui::contract::MemoProps)]
struct StatesFacts<'a> {
    list: u64,
    recording: bool,
    path_selected: bool,
    waiting: bool,
    tool_recording: bool,
    panel: &'a craft::automation::ui::actions::ActionsPanelState,
}

/// [`states`], derived from [`StatesFacts`].
pub(crate) fn source_states(app: &PhotocraftApp, name: &'static str) -> std::sync::Arc<crate::app::framework::SourceStates> {
    let menu = facts(app);
    let facts = StatesFacts {
        list: app.session.actions.list.revision(),
        recording: menu.recording,
        path_selected: menu.path_selected,
        waiting: menu.waiting,
        tool_recording: menu.tool_recording,
        panel: &app.ui.actions.panel,
    };
    crate::app::framework::derived_states(app, name, &facts, || states(app))
}

/// Runs a fired command of the Actions panel; `false` for a command of another set. Its event goes through the one
/// mapping that every Actions event takes.
pub(crate) fn run_fired(app: &mut PhotocraftApp, _ctx: &egui::Context, command: &CommandRef) -> bool {
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
