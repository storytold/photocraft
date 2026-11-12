//! Preferences › AI Integrations: the agent connection (`integrations.connectAgents`), the folders
//! agents may use, what `photocraft-cli mcp --live` does while the app is closed, and the buttons
//! that copies the configuration for an MCP client. Changes apply at once, without a restart.

use super::dialogs::is_preferences;
use super::fields::{choice_label, humanize};
use super::{Map, PhotocraftApp, RichText, Tokens, Value, json, prefs};

/// Seconds between tries while the agent connection is on but couldn't start (its port taken).
pub(super) const AGENT_LINK_RETRY_S: f64 = 5.0;

/// Start, move or stop the agent connection when its settings change
/// ([`crate::Services::agent_link`]); the first frame applies the saved settings. A connection
/// that is on but failed to start is tried again every [`AGENT_LINK_RETRY_S`], so freeing its
/// port is enough, without touching a setting. The ArtCraft Launcher's agents switch
/// ([`crate::Services::launcher_agents`]) counts as the box ticked: the person turned agents on for
/// every Craft app there.
pub(super) fn sync_agent_link(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let launcher = app.services.launcher_agents.as_ref().is_some_and(|on| on());
    let Some(link) = app.services.agent_link.as_mut() else { return };
    let mut effective = app.session.prefs().integrations.clone();
    effective.connect_agents |= launcher;
    let current = &effective;
    let now = crate::gpu_canvas::now_ms() / 1000.0;
    let retry_due = app.prefs_rt.agent_link_retry.is_some_and(|at| now >= at);
    if app.prefs_rt.agent_link.as_ref() == Some(current) && !retry_due {
        return;
    }
    let update = link(current, ctx);
    let failed_start = current.connect_agents && current.allow_agent_control && update.status.port.is_none();
    app.prefs_rt.agent_link = Some(current.clone());
    app.prefs_rt.agent_link_retry = failed_start.then_some(now + AGENT_LINK_RETRY_S);
    if failed_start {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(AGENT_LINK_RETRY_S));
    }
    if update.stopped {
        app.control_rx = None;
    }
    if let Some(rx) = update.requests {
        app.control_rx = Some(rx);
    }
    app.services.set_automation(update.automation);
    // Actions automation records keep the policy as their gate when the user plays them, as they
    // do with `--control` (`PhotocraftApp::new`); the gate stays once agents have connected.
    app.session.automation_gate = app.session.automation_gate.or(app.services.automation_authorize);
    app.agent_link = update.status;
    app.agent_link.launcher = launcher;
}

/// The rows of the section. A button click is left in `f["__agentAction"]` for [`agent_action`].
pub(super) fn integration_rows(ui: &mut egui::Ui, obj: &mut Map<String, Value>, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let status = f.get("__agentLink").cloned().unwrap_or(Value::Null);
    let launch_flags = status.get("launchFlags").and_then(Value::as_bool) == Some(true);
    let launcher = status.get("launcher").and_then(Value::as_bool) == Some(true);
    let mut on = obj.get("connectAgents").and_then(Value::as_bool).unwrap_or(false);
    let label = humanize("connectAgents");
    if launcher && !launch_flags {
        // On for the whole suite: shown ticked, and PhotoCraft's own choice is kept as it was.
        ui.add_enabled_ui(false, |ui| crate::widgets::checkbox(ui, &mut true, tl!(&label)));
        ui.label(RichText::new(tl!("Turned on in ArtCraft Launcher › Settings › AI agents, for every Craft app.")).color(t.text_dim));
    } else {
        ui.add_enabled_ui(!launch_flags, |ui| crate::widgets::checkbox(ui, &mut on, tl!(&label)));
    }
    obj.insert("connectAgents".into(), json!(on));
    let on = on || launcher;
    ui.label(
        RichText::new(tl!("AI apps such as Claude can then open, edit and save documents here. Only programs on this computer can connect.")).color(t.text_dim),
    );
    ui.add_space(4.0);
    let port = status.get("port").and_then(Value::as_u64);
    let error = status.get("error").and_then(Value::as_str);
    let (line, color) = match (launch_flags, port, error) {
        (true, _, _) => (tl!("Managed by the launch options (--control).").to_string(), t.text_dim),
        (false, _, Some(e)) => (e.to_string(), t.danger),
        (false, Some(p), None) => (crate::i18n::fmt(tl!("Agents can connect on 127.0.0.1:{port}."), &[("port", &p.to_string())]), t.accent_text),
        (false, None, None) => (tl!("Off: agents can't connect.").to_string(), t.text_faint),
    };
    ui.label(RichText::new(line).color(color));
    ui.add_space(8.0);
    egui::Grid::new("prefs-integrations").num_columns(2).spacing([14.0, 7.0]).show(ui, |ui| {
        let port_label = ui.label(RichText::new(tl!(&humanize("controlPort"))).color(t.text_dim));
        ui.horizontal(|ui| {
            let mut p = obj.get("controlPort").and_then(Value::as_i64).unwrap_or(0);
            ui.add(egui::DragValue::new(&mut p).range(0..=65535).custom_parser(crate::widgets::parse_num)).labelled_by(port_label.id);
            obj.insert("controlPort".into(), json!(p));
            ui.label(RichText::new(tl!("0 picks a free port; AI apps find it by themselves")).color(t.text_faint));
        });
        ui.end_row();
        for (key, pick) in [("readRoot", "chooseRead"), ("writeRoot", "chooseWrite")] {
            let label = humanize(key);
            ui.label(RichText::new(tl!(&label)).color(t.text_dim));
            ui.horizontal(|ui| {
                let mut path = obj.get(key).and_then(Value::as_str).unwrap_or("").to_string();
                crate::widgets::text_field(ui, ui.scope_id().with(key), &mut path, tl!("None"), 260.0, false);
                obj.insert(key.into(), json!(path));
                if crate::widgets::secondary_button(ui, tl!("Browse…"), 70.0).clicked() {
                    f.insert("__agentAction".into(), json!(pick));
                }
            });
            ui.end_row();
        }
        let label = humanize("whenAppClosed");
        ui.label(RichText::new(tl!(&label)).color(t.text_dim));
        let mut cur = obj.get("whenAppClosed").and_then(Value::as_str).unwrap_or("launch").to_string();
        let options = prefs::choices("integrations.whenAppClosed").unwrap_or(&[]);
        let labels: Vec<String> = options.iter().map(|o| choice_label(o)).collect();
        let pairs: Vec<(String, &str)> = options.iter().map(|o| o.to_string()).zip(labels.iter().map(String::as_str)).collect();
        crate::widgets::dropdown(ui, "pref-integrations.whenAppClosed", &label, &mut cur, &pairs, 220.0);
        obj.insert("whenAppClosed".into(), json!(cur));
        ui.end_row();
    });
    // Adding the Craft apps to an MCP client is the ArtCraft Launcher's job (Settings › AI agents,
    // one entry for every app, storytold/craft-ai-integrations#195). Without the launcher, the
    // configuration to paste is here, once agents may connect (launch options manage it otherwise).
    if on && !launch_flags && f.get("__agentClients").and_then(Value::as_bool) == Some(true) {
        ui.add_space(10.0);
        ui.label(RichText::new(tl!("Connect an AI app")).font(crate::theme::semibold(12.5)).color(t.text));
        ui.horizontal_wrapped(|ui| {
            if crate::widgets::secondary_button(ui, tl!("Copy Configuration"), 120.0).clicked() {
                f.insert("__agentAction".into(), json!("json"));
            }
            ui.label(RichText::new(tl!("Or add every Craft app at once in ArtCraft Launcher › Settings › AI agents.")).color(t.text_faint));
        });
    }
    if let Some(result) = f.get("__agentResult").and_then(Value::as_object) {
        let ok = result.get("ok").and_then(Value::as_bool) == Some(true);
        let text = result.get("message").and_then(Value::as_str).unwrap_or("");
        ui.label(RichText::new(text).color(if ok { t.text_dim } else { t.danger }));
    }
}

/// A button of [`integration_rows`]: choose a folder (into the dialog's working copy), or add
/// PhotoCraft to an MCP client through [`crate::Services::agent_clients`].
pub(super) fn agent_action(app: &mut PhotocraftApp, ctx: &egui::Context, f: &mut Map<String, Value>, action: &str, dialog: u64) {
    let key = match action {
        "chooseRead" => Some("readRoot"),
        "chooseWrite" => Some("writeRoot"),
        _ => None,
    };
    if let Some(key) = key {
        // Into the Preferences dialog that asked: closed meanwhile (or reopened as a new one), the
        // answer is dropped rather than landing in another working copy.
        let picked = app.ask_folder(move |app, path| {
            if let Some(d) = app.ui.dialogs.iter_mut().find(|d| d.id == dialog && is_preferences(&d.fields))
                && let Some(obj) = d.fields.get_mut("values").and_then(|v| v.get_mut("integrations")).and_then(Value::as_object_mut)
            {
                obj.insert(key.into(), json!(path));
            }
            Ok(Value::Null)
        });
        if let Err(e) = picked {
            f.insert("__agentResult".into(), json!({"ok": false, "message": e}));
        }
        return;
    }
    let Some(add) = app.services.agent_clients.as_mut() else { return };
    let result = match add(action) {
        Ok(text) if action == "json" => {
            ctx.copy_text(text);
            json!({"ok": true, "message": tl!("Configuration copied.")})
        }
        Ok(message) => json!({"ok": true, "message": message}),
        Err(e) => json!({"ok": false, "message": e}),
    };
    f.insert("__agentResult".into(), result);
}
