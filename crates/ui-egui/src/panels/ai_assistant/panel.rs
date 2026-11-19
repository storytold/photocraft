//! AI Assistant dockable panel UI for PhotoCraft.

#[cfg(not(target_arch = "wasm32"))]
use craft::ui::buttons::Button;
use craft::ui::contract::Show as _;
#[cfg(not(target_arch = "wasm32"))]
use craft::ui::contract::{Bind, Props as _};
use craft::ui::display::Text;
use craft::ui::runtime::env::Label;
#[cfg(not(target_arch = "wasm32"))]
use craft::ui::text::TextField;
#[cfg(not(target_arch = "wasm32"))]
use egui::{Color32, RichText, ScrollArea};
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;

#[cfg(not(target_arch = "wasm32"))]
use super::{AiProvider, ChatMessage, ChatRole};
use crate::PhotocraftApp;

#[cfg(not(target_arch = "wasm32"))]
const QUICK_PROMPTS: &[(&str, &str)] = &[
    ("Inspect document", "Inspect the active PhotoCraft document and summarize its size, color mode, and layers."),
    ("Auto-enhance tone", "Inspect the active document and apply a balanced exposure and contrast improvement."),
    ("List commands", "List the main editing commands available on the current document."),
];

/// Draw the AI Assistant panel body.
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = app;
        Text::new(Label::from(tl!("AI Assistant requires the desktop app to connect to Claude or Gemini."))).show(ui);
        return;
    }

    #[cfg(not(target_arch = "wasm32"))]
    show_desktop(app, ui);
}

#[cfg(not(target_arch = "wasm32"))]
fn show_desktop(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    // Skip raw egui widgets during craft's invisible measure/sizing pass so widget IDs are never
    // registered twice in the same egui pass at different rects (which triggers debug ID-clash overlays).
    if ui.is_sizing_pass() || !ui.is_visible() {
        ui.allocate_space(ui.available_size());
        return;
    }
    if app.ai_assistant.detected.is_none() {
        let detected = super::providers::DetectedClis::detect();
        if detected.claude.is_none() && detected.gemini.is_none() {
            app.ai_assistant.show_connectors = true;
        }
        app.ai_assistant.detected = Some(detected);
    }

    ui.push_id("ai_header", |ui| draw_header(app, ui));
    ui.separator();

    if app.ai_assistant.show_connectors {
        ui.push_id("ai_connectors", |ui| draw_connector_section(app, ui));
        ui.separator();
    }

    if let Some((ok, msg)) = app.ai_assistant.status_banner.clone() {
        let color = if ok { Color32::from_rgb(90, 190, 120) } else { Color32::from_rgb(230, 110, 100) };
        ui.push_id("ai_banner", |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(msg).small().color(color));
                if ui.small_button("×").clicked() {
                    app.ai_assistant.status_banner = None;
                }
            });
        });
        ui.separator();
    }

    let footer_h = 104.0;
    let body_h = (ui.available_height() - footer_h).max(80.0);
    ScrollArea::vertical().id_salt("photocraft_ai_chat_scroll").max_height(body_h).stick_to_bottom(true).show(ui, |ui| {
        if app.ai_assistant.messages.is_empty() {
            ui.push_id("ai_empty", |ui| draw_empty_state(app, ui));
        } else {
            ui.push_id("ai_messages", |ui| draw_messages(&app.ai_assistant.messages, ui));
        }
    });

    ui.separator();
    ui.push_id("ai_composer", |ui| draw_composer(app, ui));
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_header(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let claude_ok = app.ai_assistant.detected.as_ref().is_some_and(|d| d.claude.is_some());
    let gemini_ok = app.ai_assistant.detected.as_ref().is_some_and(|d| d.gemini.is_some());

    ui.horizontal_wrapped(|ui| {
        for (prov, ok) in [(AiProvider::Claude, claude_ok), (AiProvider::Gemini, gemini_ok)] {
            let dot = if ok { "● " } else { "○ " };
            let selected = app.ai_assistant.provider == prov;
            let text = RichText::new(format!("{dot}{}", prov.label())).strong();
            if ui.selectable_label(selected, text).clicked() && app.ai_assistant.provider != prov {
                app.ai_assistant.provider = prov;
                app.ai_assistant.model.clear();
            }
        }
        ui.separator();
        if ui.selectable_label(app.ai_assistant.show_connectors, tl!("Connectors")).clicked() {
            app.ai_assistant.show_connectors = !app.ai_assistant.show_connectors;
        }
        if Button::new(Label::from(tl!("New"))).show(ui).clicked() {
            app.ai_assistant.clear_chat();
        }
    });

    draw_model_row(app, ui);
    draw_bridge_status(app, ui);
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_model_row(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let presets: &[(&str, &str)] = match app.ai_assistant.provider {
        AiProvider::Claude => &[("Auto", ""), ("Sonnet", "sonnet"), ("Opus", "opus"), ("Haiku", "haiku")],
        AiProvider::Gemini => &[
            ("Auto", ""),
            ("3.5 Flash Lite", "gemini-3.5-flash-lite"),
            ("3.8 Flash", "gemini-3.8-flash"),
            ("4 Hydrogen", "gemini-4-hydrogen"),
            ("4 Neon", "gemini-4-neon"),
        ],
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Model:").small().weak());
        for (label, id) in presets {
            let selected = app.ai_assistant.model == *id;
            if ui.selectable_label(selected, RichText::new(*label).small()).clicked() {
                app.ai_assistant.model = (*id).to_owned();
            }
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_bridge_status(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let port = app.services.agent_link.as_ref().and_then(|link| link.status().port);
    let managed_by_launcher = app.services.agent_switch.as_ref().is_some_and(|switch| switch.mcp_flag());
    let mut toggle: Option<bool> = None;
    ui.horizontal_wrapped(|ui| {
        if let Some(p) = port {
            ui.label(RichText::new(format!("MCP Live (127.0.0.1:{p})")).small().color(Color32::from_rgb(90, 190, 120)));
            if !managed_by_launcher && ui.small_button(tl!("Disable")).clicked() {
                toggle = Some(false);
            }
        } else if managed_by_launcher {
            ui.label(
                RichText::new(tl!("MCP bridge off — enable PhotoCraft in ArtCraft Launcher (Settings › AI agents)"))
                    .small()
                    .color(Color32::from_rgb(220, 170, 70)),
            );
        } else {
            ui.label(RichText::new(tl!("MCP bridge off")).small().color(Color32::from_rgb(220, 170, 70)));
            if ui.small_button(tl!("Enable Live Bridge")).clicked() {
                toggle = Some(true);
            }
        }
        if app.ai_assistant.is_running() {
            ui.spinner();
        }
    });
    if let Some(enable) = toggle {
        match app.run("prefs.set", json!({"path": "integrations.connectAgents", "value": enable})) {
            Ok(_) => {
                let msg = if enable { "Live MCP bridge enabled." } else { "Live MCP bridge disabled." };
                app.ai_assistant.status_banner = Some((true, msg.to_owned()));
            }
            Err(err) => {
                app.ai_assistant.status_banner = Some((false, format!("Failed to update bridge setting: {err}")));
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_connector_section(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    use super::connector::{self, ConnectorTarget};

    let cli_path = app.ai_assistant.detected.as_ref().and_then(|d| d.photocraft_cli.clone());
    ui.label(RichText::new(tl!("Subscription Connectors (MCP)")).strong());

    if let Some(cli) = cli_path {
        ui.label(RichText::new(format!("CLI: {}", cli.display())).small().weak());
        ui.horizontal_wrapped(|ui| {
            if Button::new(Label::from("Claude Desktop")).show(ui).clicked() {
                match connector::install(ConnectorTarget::ClaudeDesktop, &cli) {
                    Ok(p) => app.ai_assistant.status_banner = Some((true, format!("Installed in {}", p.display()))),
                    Err(e) => app.ai_assistant.status_banner = Some((false, e)),
                }
            }
            if Button::new(Label::from("Gemini CLI")).show(ui).clicked() {
                match connector::install(ConnectorTarget::GeminiCli, &cli) {
                    Ok(p) => app.ai_assistant.status_banner = Some((true, format!("Installed in {}", p.display()))),
                    Err(e) => app.ai_assistant.status_banner = Some((false, e)),
                }
            }
            if Button::new(Label::from(tl!("Copy Claude Code Cmd"))).show(ui).clicked() {
                let cmd = connector::snippet(ConnectorTarget::ClaudeCodeCli, &cli);
                ui.ctx().copy_text(cmd);
                app.ai_assistant.status_banner = Some((true, "Copied `claude mcp add` command.".to_owned()));
            }
        });
    } else {
        Text::new(Label::from(tl!("Build or install `photocraft-cli` next to PhotoCraft to enable MCP connectors."))).show(ui);
    }

    if ui.small_button(tl!("Re-scan CLIs")).clicked() {
        app.ai_assistant.detected = Some(super::providers::DetectedClis::detect());
    }

    ui.add_space(6.0);
    draw_gemini_key_row(app, ui);
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_gemini_key_row(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    app.ai_assistant.ensure_keys_loaded();
    let mut save_gemini = false;
    let mut save_anthropic = false;

    ui.label(RichText::new("Gemini API Key (chat + image gen)").small().strong());
    ui.push_id("ai_key_gemini", |ui| {
        let out = TextField::new(Label::from("Gemini API Key"), Bind::bound(&mut app.ai_assistant.gemini_api_key_draft))
            .id_salt("ai_gemini_key_input")
            .masked()
            .placeholder("AIza... (aistudio.google.com/apikey)")
            .show(ui);
        if !out.events.is_empty() {
            app.ai_assistant.gemini_key_dirty = true;
            app.ai_assistant.gemini_key_from_env = false;
        }
        ui.horizontal(|ui| {
            if ui.small_button("Paste").clicked()
                && let Some(text) = super::gemini_image::read_clipboard_text()
            {
                match super::gemini_image::validate_api_key(&text) {
                    Ok(valid) => {
                        app.ai_assistant.gemini_api_key_draft = valid.to_owned();
                        app.ai_assistant.gemini_key_dirty = true;
                        app.ai_assistant.gemini_key_from_env = false;
                        save_gemini = true;
                    }
                    Err(e) => app.ai_assistant.status_banner = Some((false, format!("Clipboard Gemini API key rejected: {e}"))),
                }
            }
            if ui.small_button("Save").clicked() {
                save_gemini = true;
            }
        });
    });

    ui.add_space(4.0);
    ui.label(RichText::new("Claude API Key (optional — leave empty for subscription)").small().strong());
    ui.push_id("ai_key_claude", |ui| {
        let out = TextField::new(Label::from("Claude API Key"), Bind::bound(&mut app.ai_assistant.anthropic_api_key_draft))
            .id_salt("ai_claude_key_input")
            .masked()
            .placeholder("sk-ant-... (optional)")
            .show(ui);
        if !out.events.is_empty() {
            app.ai_assistant.anthropic_key_dirty = true;
            app.ai_assistant.anthropic_key_from_env = false;
        }
        ui.horizontal(|ui| {
            if ui.small_button("Paste").clicked()
                && let Some(text) = super::gemini_image::read_clipboard_text()
            {
                match super::gemini_image::validate_api_key(&text) {
                    Ok(valid) => {
                        app.ai_assistant.anthropic_api_key_draft = valid.to_owned();
                        app.ai_assistant.anthropic_key_dirty = true;
                        app.ai_assistant.anthropic_key_from_env = false;
                        save_anthropic = true;
                    }
                    Err(e) => app.ai_assistant.status_banner = Some((false, format!("Clipboard Claude API key rejected: {e}"))),
                }
            }
            if ui.small_button("Save").clicked() {
                save_anthropic = true;
            }
        });
    });

    if save_gemini {
        persist_single_key(app, true);
    }
    if save_anthropic {
        persist_single_key(app, false);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn persist_single_key(app: &mut PhotocraftApp, is_gemini: bool) {
    let (label, env_var, draft, from_env, dirty) = if is_gemini {
        (
            "Gemini API key",
            "GEMINI_API_KEY",
            app.ai_assistant.gemini_api_key_draft.trim().to_owned(),
            app.ai_assistant.gemini_key_from_env,
            app.ai_assistant.gemini_key_dirty,
        )
    } else {
        (
            "Claude API key",
            "ANTHROPIC_API_KEY",
            app.ai_assistant.anthropic_api_key_draft.trim().to_owned(),
            app.ai_assistant.anthropic_key_from_env,
            app.ai_assistant.anthropic_key_dirty,
        )
    };
    if from_env && !dirty {
        app.ai_assistant.status_banner = Some((true, format!("{label} is loaded from {env_var}; edit it to save a local override.")));
        return;
    }
    if !draft.is_empty()
        && let Err(e) = super::gemini_image::validate_api_key(&draft)
    {
        app.ai_assistant.status_banner = Some((false, format!("{label} rejected: {e}")));
        return;
    }
    let mut stored = super::gemini_image::load_stored_keys();
    if is_gemini {
        stored.gemini_api_key = draft.clone();
    } else {
        stored.anthropic_api_key = draft.clone();
    }
    match super::gemini_image::save_keys(&stored) {
        Ok(_) => {
            if is_gemini {
                app.ai_assistant.gemini_key_dirty = false;
                app.ai_assistant.gemini_key_from_env = false;
            } else {
                app.ai_assistant.anthropic_key_dirty = false;
                app.ai_assistant.anthropic_key_from_env = false;
            }
            let msg = if draft.is_empty() { format!("Cleared {label}.") } else { format!("Saved {label} (0600).") };
            app.ai_assistant.status_banner = Some((true, msg));
        }
        Err(e) => app.ai_assistant.status_banner = Some((false, e)),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_empty_state(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let provider = app.ai_assistant.provider;
    let has_cli = app.ai_assistant.detected.as_ref().and_then(|d| d.path_for(provider)).is_some();

    if !has_cli {
        let hint = match provider {
            AiProvider::Claude => "Install Claude Code (`claude`) and sign in with your Claude subscription, or click Connectors above for Claude Desktop.",
            AiProvider::Gemini => "Install Gemini CLI (`gemini`) and sign in with your Google/Gemini subscription, or click Connectors above.",
        };
        ui.label(RichText::new(hint).small().color(Color32::from_rgb(220, 170, 70)));
        ui.add_space(6.0);
    } else {
        Text::new(Label::from(format!("Connected via {} subscription CLI. Ask anything about your open document:", provider.label()))).show(ui);
        ui.add_space(6.0);
    }

    let mut chosen: Option<&'static str> = None;
    ui.horizontal_wrapped(|ui| {
        for (title, prompt) in QUICK_PROMPTS {
            if ui.small_button(*title).clicked() {
                chosen = Some(*prompt);
            }
        }
    });
    if let Some(prompt) = chosen {
        let live_port = app.services.agent_link.as_ref().and_then(|link| link.status().port);
        app.ai_assistant.input = prompt.to_owned();
        app.ai_assistant.send_current(live_port, ui.ctx());
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_messages(messages: &[ChatMessage], ui: &mut egui::Ui) {
    for msg in messages {
        match msg.role {
            ChatRole::User => {
                ui.label(RichText::new("You").strong().small());
                ui.label(&msg.text);
            }
            ChatRole::Assistant => {
                ui.label(RichText::new("Assistant").strong().small().color(Color32::from_rgb(130, 180, 255)));
                ui.label(&msg.text);
            }
            ChatRole::Tool => {
                let name = msg.tool_name.as_deref().unwrap_or("tool");
                let line = if msg.text.is_empty() { format!("Tool: {name}") } else { format!("Tool: {name} {}", msg.text) };
                ui.label(RichText::new(line).small().monospace().weak());
            }
            ChatRole::System => {
                ui.label(RichText::new(&msg.text).small().color(Color32::from_rgb(230, 110, 100)));
            }
        }
        ui.add_space(6.0);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_composer(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let running = app.ai_assistant.is_running();
    let out = TextField::new(Label::from("Prompt"), Bind::bound(&mut app.ai_assistant.input))
        .id_salt("ai_prompt_input")
        .lines(3)
        .submits()
        .placeholder("Ask Claude or Gemini to inspect or edit this image…")
        .show(ui);

    let mut trigger_send = out.submitted;
    let mut trigger_image = false;

    ui.horizontal(|ui| {
        if running {
            if Button::new(Label::from(tl!("Stop"))).show(ui).clicked() {
                app.ai_assistant.cancel();
            }
        } else {
            let has_input = !app.ai_assistant.input.trim().is_empty();
            if Button::new(Label::from(tl!("Send"))).enabled(has_input).show(ui).clicked() {
                trigger_send = true;
            }
            if Button::new(Label::from("Generate Image")).enabled(has_input).show(ui).clicked() {
                trigger_image = true;
            }
        }
    });

    if trigger_image && !running {
        let target_doc = app.session.active().map(|st| st.doc.id);
        app.ai_assistant.generate_image_current(target_doc, ui.ctx());
    } else if trigger_send && !running {
        let live_port = app.services.agent_link.as_ref().and_then(|link| link.status().port);
        app.ai_assistant.send_current(live_port, ui.ctx());
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn collect_clash_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        let mut out = Vec::new();
        for cs in shapes {
            collect_shape(&cs.shape, &mut out);
        }
        out
    }

    fn collect_shape(shape: &egui::epaint::Shape, out: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Text(t) => {
                let s = t.galley.text();
                if s.contains("First use of") || s.contains("Second use of") || s.contains("Double use of") {
                    out.push(s.to_owned());
                }
            }
            egui::epaint::Shape::Vec(v) => {
                for child in v {
                    collect_shape(child, out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn floating_ai_panel_has_zero_id_clashes_across_provider_and_connector_toggles() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.ai_assistant.detected = Some(super::super::providers::DetectedClis::default());
        crate::panel_docking::show_panel(&mut app, "aiAssistant", Some(true)).unwrap();
        let ctx = app.ctx().clone();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0));

        let scenarios: &[(AiProvider, bool, Option<(bool, &str)>)] = &[
            (AiProvider::Claude, false, None),
            (AiProvider::Gemini, true, None),
            (AiProvider::Claude, true, Some((true, "Saved API keys (0600)."))),
            (AiProvider::Gemini, false, Some((false, "Rate limit."))),
            (AiProvider::Claude, true, None),
        ];

        for (step, (provider, show_connectors, banner)) in scenarios.iter().enumerate() {
            app.ai_assistant.provider = *provider;
            app.ai_assistant.show_connectors = *show_connectors;
            app.ai_assistant.status_banner = banner.map(|(ok, s)| (ok, s.to_owned()));

            let mut events = Vec::new();
            if step % 2 == 1 {
                let pos = egui::pos2(100.0, 100.0);
                events.push(egui::Event::PointerMoved(pos));
                events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
                events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
            }
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    crate::panel_docking::show(&mut app, ui, false);
                });
            });
            output.textures_delta.clear();
            let clashes = collect_clash_texts(&output.shapes);
            assert!(clashes.is_empty(), "step {step} ({provider:?}, connectors={show_connectors}) had egui ID clashes: {clashes:#?}");
        }
    }
}
