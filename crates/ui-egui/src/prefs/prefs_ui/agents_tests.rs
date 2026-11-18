#![cfg(not(target_arch = "wasm32"))]
//! Agents in the open window (`agents.rs`): the agent link follows the preferences and the
//! ArtCraft Launcher's switch, and agent requests get the app's policy while it runs. The link, the
//! switch and the Agents page have their own trials in craft-automation (`tests/it/agents.rs`).

use super::tests::app_with_store;
use super::*;
use craft::automation::agents::{AgentLink, AgentSettings, AgentSwitch, LinkProblem};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// One upkeep pass.
fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    super::save::tick(app, ctx, Instant::now());
}

/// An app whose agent link keeps its files in a fresh folder, with `switch` as the launcher's
/// switch. The policy it installs refuses commands, names a folder `/missing` as not used, and
/// records the settings it was asked for.
fn linked_app(switch: AgentSwitch) -> (Box<PhotocraftApp>, tempfile::TempDir, Arc<Mutex<Vec<AgentSettings>>>) {
    let (mut app, _) = app_with_store();
    let dir = tempfile::tempdir().unwrap();
    app.services.agent_link = Some(AgentLink::new(Some(dir.path().to_path_buf()), "PhotoCraft test"));
    app.services.agent_switch = Some(switch);
    let asked: Arc<Mutex<Vec<AgentSettings>>> = Arc::default();
    let seen = asked.clone();
    app.services.agent_automation = Some(Box::new(move |settings: &AgentSettings| {
        seen.lock().unwrap().push(settings.clone());
        let services = crate::AutomationServices { command: Some(Box::new(|_, _| Err("policy".into()))), authorize: Some(|_, _| Ok(())), ..Default::default() };
        (services, (settings.read_root == "/missing").then(|| "Automation Read Folder is not used".to_string()))
    }));
    (app, dir, asked)
}

fn port(app: &PhotocraftApp) -> Option<u16> {
    app.services.agent_link.as_ref().and_then(|link| link.status().port)
}

#[test]
fn preferences_start_and_stop_the_agent_link_and_the_policy_follows() {
    let (mut app, _dir, asked) = linked_app(AgentSwitch::fixed(false, false));
    let ctx = egui::Context::default();
    // Off as saved: nothing listens, agent requests get nothing.
    tick(&mut app, &ctx);
    tick(&mut app, &ctx);
    assert!(app.control.is_none() && port(&app).is_none() && app.services.automation_command.is_none());
    assert!(asked.lock().unwrap().is_empty());
    // On: the control channel and the policy over the chosen folder, without a restart.
    app.run("prefs.set", json!({"values": {"integrations.connectAgents": true, "integrations.readRoot": "/work"}})).unwrap();
    tick(&mut app, &ctx);
    assert!(port(&app).is_some() && app.control.is_some());
    assert!(app.services.automation_command.is_some());
    assert!(app.session.automation_gate.is_some(), "actions agents record keep the policy");
    assert_eq!(asked.lock().unwrap().last().map(|s| s.read_root.clone()).as_deref(), Some("/work"));
    // An unrelated preference doesn't touch the link.
    app.run("prefs.set", json!({"values": {"performance.historyStates": 12}})).unwrap();
    tick(&mut app, &ctx);
    assert_eq!(asked.lock().unwrap().len(), 1);
    // A folder the policy can't use is named on the page, and the server keeps running.
    let running = port(&app);
    app.run("prefs.set", json!({"values": {"integrations.readRoot": "/missing"}})).unwrap();
    tick(&mut app, &ctx);
    assert_eq!(port(&app), running);
    let problem = app.services.agent_link.as_ref().and_then(|link| link.status().problem.clone());
    assert!(matches!(problem, Some(LinkProblem::App(_))), "{problem:?}");
    // ui.inspect reports the link.
    let inspected = crate::control::inspect(&app, &ctx);
    assert_eq!(inspected["agentLink"]["port"], json!(running));
    assert_eq!(inspected["agentLink"]["mcpFlag"], json!(false));
    // Off: the channel and the policy go.
    app.run("prefs.set", json!({"values": {"integrations.connectAgents": false}})).unwrap();
    tick(&mut app, &ctx);
    assert!(app.control.is_none() && port(&app).is_none() && app.services.automation_command.is_none());
}

/// With the AI-MCP flag the ArtCraft Launcher's switch is the one consent: it starts the link,
/// turning it off there stops it again, and a Control Port saved before doesn't keep it on.
#[test]
fn with_the_flag_the_launcher_switch_alone_turns_the_agent_link_on_and_off() {
    let (mut app, _dir, asked) = linked_app(AgentSwitch::fixed(true, false));
    app.run("prefs.set", json!({"values": {"integrations.connectAgents": true}})).unwrap();
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    assert!(port(&app).is_none(), "a saved Control Port needs the launcher's switch");
    app.services.agent_switch = Some(AgentSwitch::fixed(true, true));
    tick(&mut app, &ctx);
    let on = port(&app);
    assert!(on.is_some() && app.control.is_some(), "the launcher's switch alone starts it");
    tick(&mut app, &ctx);
    assert_eq!((port(&app), asked.lock().unwrap().len()), (on, 1), "no change, no restart");
    app.services.agent_switch = Some(AgentSwitch::fixed(true, false));
    tick(&mut app, &ctx);
    assert!(port(&app).is_none() && app.control.is_none(), "off in the launcher stops it");
}

/// Without the AI-MCP flag nothing changes from before: the launcher's switch means nothing, and a
/// Control Port saved in the preferences keeps its connection.
#[test]
fn without_the_flag_the_launcher_switch_means_nothing() {
    let (mut app, _dir, _) = linked_app(AgentSwitch::fixed(false, true));
    let ctx = egui::Context::default();
    tick(&mut app, &ctx);
    assert!(port(&app).is_none(), "the launcher's switch needs the flag");
    app.run("prefs.set", json!({"values": {"integrations.connectAgents": true}})).unwrap();
    tick(&mut app, &ctx);
    assert!(port(&app).is_some(), "a saved Control Port still counts");
}

#[test]
fn agent_choices_and_paths_validate_through_prefs_set() {
    let (mut app, _) = app_with_store();
    for v in ["launch", "headless", "error"] {
        app.run("prefs.set", json!({"values": {"integrations.whenAppClosed": v}})).unwrap();
        assert_eq!(app.session.prefs().integrations.agents.when_app_closed.name(), v);
    }
    assert!(app.run("prefs.set", json!({"values": {"integrations.whenAppClosed": "sometimes"}})).is_err());
    assert!(app.run("prefs.set", json!({"values": {"integrations.controlPort": 70000}})).is_err());
    app.run("prefs.set", json!({"values": {"integrations.writeRoot": "/tmp/out"}})).unwrap();
    assert_eq!(app.session.prefs().integrations.agents.write_root, "/tmp/out");
}
