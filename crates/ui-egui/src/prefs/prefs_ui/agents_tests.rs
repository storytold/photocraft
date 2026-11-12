//! Preferences › MCP (`agents.rs`): the live agent connection and its buttons.

use super::tests::app_with_store;
use super::*;
use std::sync::{Arc, Mutex};

#[test]
fn ai_integrations_start_and_stop_the_agent_connection_through_the_service() {
    use std::sync::mpsc::channel;
    let (mut app, _) = app_with_store();
    let ctx = egui::Context::default();
    let calls: Arc<Mutex<Vec<prefs::Integrations>>> = Arc::default();
    let seen = calls.clone();
    let (tx, rx) = channel::<crate::ControlRequest>();
    let mut rx = Some(rx);
    app.services.agent_link = Some(Box::new(move |p: &prefs::Integrations, _: &egui::Context| {
        seen.lock().unwrap().push(p.clone());
        let mut out = crate::AgentLink { stopped: !p.connect_agents, ..Default::default() };
        if p.connect_agents {
            out.requests = rx.take();
            out.status.port = Some(4321);
            out.automation.command = Some(Box::new(|_, _| Err("policy".into())));
        }
        out
    }));
    // The first frame applies the saved settings (off), once.
    tick(&mut app, &ctx);
    tick(&mut app, &ctx);
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert!(app.control_rx.is_none() && app.agent_link.port.is_none());
    // On: the request channel and the automation policy are installed, without a restart.
    app.run("prefs.set", json!({"values": {"integrations.connectAgents": true, "integrations.readRoot": "/work"}})).unwrap();
    tick(&mut app, &ctx);
    assert_eq!(calls.lock().unwrap().last().map(|p| (p.connect_agents, p.read_root.clone())), Some((true, "/work".into())));
    assert!(app.control_rx.is_some());
    assert!(app.services.automation_command.is_some());
    assert_eq!(app.agent_link.port, Some(4321));
    assert!(tx.send(crate::ControlRequest::new("ui.inspect", json!({})).0).is_ok(), "the app holds the receiver");
    // An unrelated preference doesn't touch the connection.
    app.run("prefs.set", json!({"values": {"performance.historyStates": 12}})).unwrap();
    tick(&mut app, &ctx);
    assert_eq!(calls.lock().unwrap().len(), 2);
    // Off: the channel and the policy go.
    app.run("prefs.set", json!({"values": {"integrations.connectAgents": false}})).unwrap();
    tick(&mut app, &ctx);
    assert_eq!(calls.lock().unwrap().len(), 3);
    assert!(app.control_rx.is_none() && app.services.automation_command.is_none());
    assert_eq!(app.agent_link.port, None);
    // ui.inspect reports it.
    assert!(crate::control::inspect(&app, &ctx).get("agentLink").is_some());
}

/// The buttons that set up an AI app only show once agents may connect, and a click reaches
/// the service with the app's id and shows its answer.
#[test]
fn ai_integrations_offer_the_client_buttons_only_when_agents_may_connect() {
    use egui_kittest::{Harness, kittest::Queryable};
    let (mut app, _) = app_with_store();
    let clicked: Arc<Mutex<Vec<String>>> = Arc::default();
    let seen = clicked.clone();
    app.services.agent_clients = Some(Box::new(move |id: &str| {
        seen.lock().unwrap().push(id.to_string());
        Ok(format!("Added PhotoCraft to {id}."))
    }));
    open_preferences(&mut app, "integrations");
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    assert!(h.query_by_label("Add to Claude Desktop").is_none(), "off: nothing to set up yet");
    assert!(h.query_by_label("Copy Configuration").is_none());
    h.get_by_label("Let AI agents connect (MCP)").click();
    h.run_steps(3);
    for label in ["Add to Claude Desktop", "Add to Claude Code", "Add to Cursor", "Copy Configuration", "Copy Prompt for an Agent"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    h.get_by_label("Add to Cursor").click();
    h.run_steps(3);
    assert_eq!(*clicked.lock().unwrap(), ["cursor"]);
    assert!(h.query_by_label("Added PhotoCraft to cursor.").is_some(), "the answer is shown");
    // The prompt for an agent with a terminal goes to the clipboard, with a note saying so.
    h.get_by_label("Copy Prompt for an Agent").click();
    h.run_steps(3);
    assert_eq!(*clicked.lock().unwrap(), ["cursor", "prompt"]);
    assert!(h.query_by_label_contains("Prompt copied").is_some(), "the copy is confirmed");
    // Unticked again: the buttons go.
    h.get_by_label("Let AI agents connect (MCP)").click();
    h.run_steps(3);
    assert!(h.query_by_label("Add to Cursor").is_none());
}

#[test]
fn ai_integrations_choices_and_paths_validate_through_prefs_set() {
    let (mut app, _) = app_with_store();
    for v in ["launch", "headless", "error"] {
        app.run("prefs.set", json!({"values": {"integrations.whenAppClosed": v}})).unwrap();
        assert_eq!(app.session.prefs().integrations.when_app_closed.name(), v);
    }
    assert!(app.run("prefs.set", json!({"values": {"integrations.whenAppClosed": "sometimes"}})).is_err());
    assert!(app.run("prefs.set", json!({"values": {"integrations.controlPort": 70000}})).is_err());
    app.run("prefs.set", json!({"values": {"integrations.writeRoot": "/tmp/out"}})).unwrap();
    assert_eq!(app.session.prefs().integrations.write_root, "/tmp/out");
}

/// A connection that is on but failed to start (its port taken) is tried again by itself, with the
/// same settings, and the automation gate follows the live policy.
#[test]
fn a_failed_start_is_retried_without_a_setting_change() {
    let (mut app, _) = app_with_store();
    let ctx = egui::Context::default();
    let calls: Arc<Mutex<u32>> = Arc::default();
    let seen = calls.clone();
    app.services.agent_link = Some(Box::new(move |p: &prefs::Integrations, _: &egui::Context| {
        let mut n = seen.lock().unwrap();
        *n += 1;
        let mut out = crate::AgentLink::default();
        if p.connect_agents && *n >= 3 {
            out.status.port = Some(4321);
            out.automation.authorize = Some(|_, _| Ok(()));
        } else if p.connect_agents {
            out.status.error = Some("Couldn't listen on 127.0.0.1:4321: in use".into());
        }
        out
    }));
    app.run("prefs.set", json!({"values": {"integrations.connectAgents": true}})).unwrap();
    tick(&mut app, &ctx);
    assert_eq!(*calls.lock().unwrap(), 1);
    assert!(app.agent_link.port.is_none() && app.prefs_rt.agent_link_retry.is_some(), "a retry is scheduled");
    // Not yet due: nothing happens.
    tick(&mut app, &ctx);
    assert_eq!(*calls.lock().unwrap(), 1);
    // Due: tried again with the same settings, still failing, then started.
    app.prefs_rt.agent_link_retry = Some(0.0);
    tick(&mut app, &ctx);
    assert_eq!(*calls.lock().unwrap(), 2);
    app.prefs_rt.agent_link_retry = Some(0.0);
    tick(&mut app, &ctx);
    assert_eq!(*calls.lock().unwrap(), 3);
    assert_eq!(app.agent_link.port, Some(4321));
    assert!(app.prefs_rt.agent_link_retry.is_none(), "running: no more retries");
    assert!(app.session.automation_gate.is_some(), "actions agents record keep the policy");
    tick(&mut app, &ctx);
    assert_eq!(*calls.lock().unwrap(), 3);
}
