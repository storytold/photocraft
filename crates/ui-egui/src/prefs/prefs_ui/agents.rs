//! Agents in the open window: the agent link follows Preferences › Integrations and the ArtCraft
//! Launcher's switch without a restart. The link, the switch and the Agents page are the
//! framework's (`craft::automation::agents`, `custom_rows::AGENTS_PAGE`); PhotoCraft supplies what
//! agent requests may read, write and run (`Services::agent_automation`).

use craft::automation::agents::LinkProblem;
use craft::automation::control::ControlChannel;
use craft::ui::runtime::windows::WindowHost;

use super::PhotocraftApp;
use crate::AutomationServices;

/// Brings the agent link in line with the saved agent settings and the launcher's switch: the link
/// starts, moves or stops its control server, and does nothing while neither changed. With the
/// suite's `AI-MCP` flag the launcher's switch alone lets agents in; without it a Control Port
/// saved before the switch (`connect_agents`). While the server runs, agent requests get the
/// automation policy over the folders chosen; a folder that can't be used is named on the page.
pub(super) fn sync_agent_link(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let integrations = app.session.prefs().integrations.clone();
    let legacy = integrations.connect_agents;
    let allowed = app.services.agent_switch.as_ref().map_or(legacy, |switch| switch.allows(legacy));
    let Some(link) = app.services.agent_link.as_mut() else { return };
    let update = link.sync(allowed.then_some(&integrations.agents), &WindowHost::of(ctx));
    if update.stopped {
        app.control = None;
    }
    if let Some(requests) = update.requests {
        app.control = Some(ControlChannel::new(requests));
    }
    if !update.changed {
        return;
    }
    let (automation, problem) = match app.services.agent_automation.as_ref() {
        Some(policy) if link.status().port.is_some() => policy(&integrations.agents),
        _ => (AutomationServices::default(), None),
    };
    if let Some(problem) = problem {
        link.note(LinkProblem::App(problem));
    }
    app.services.set_automation(automation);
    // Actions automation records keep the policy as their gate when the user plays them, as they
    // do with `--control` (`PhotocraftApp::new`); the gate stays once agents have connected.
    app.session.automation_gate = app.session.automation_gate.or(app.services.automation_authorize);
}
