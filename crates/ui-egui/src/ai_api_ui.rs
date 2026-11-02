//! API controls in the AI edit dialog. Secret drafts are never serialized or returned by inspect.
use crate::PhotocraftApp;
use photocraft_engine::cmd::ai_provider_cmds::{COMMAND, Profile, Protocol, preset};
use serde_json::{Map, Value, json};

#[derive(Clone, Default, PartialEq)]
pub struct ApiUi {
    provider: String,
    profile: Option<Profile>,
    saved: Option<Profile>,
    key: String,
    has_key: bool,
    persistent: bool,
    remember: bool,
    saved_remember: bool,
    message: Option<String>,
    message_error: bool,
}

impl std::fmt::Debug for ApiUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiUi").field("provider", &self.provider).field("has_key", &self.has_key).field("message", &self.message).finish_non_exhaustive()
    }
}

pub fn selected(fields: &Map<String, Value>) -> &str {
    fields.get("provider").and_then(Value::as_str).unwrap_or("codex")
}

pub fn can_confirm(app: &PhotocraftApp, fields: &Map<String, Value>) -> bool {
    let draft = &app.ui.ai_api;
    draft.provider == selected(fields)
        && draft.has_key
        && draft.key.is_empty()
        && draft.profile.is_some()
        && draft.profile == draft.saved
        && draft.remember == draft.saved_remember
}

fn apply(app: &mut PhotocraftApp, provider: &str, result: Result<Value, String>, success: Option<&str>) {
    match result {
        Ok(value) => {
            let profile = value.get("profile").cloned().and_then(|v| serde_json::from_value(v).ok());
            app.ui.ai_api = ApiUi {
                provider: provider.into(),
                saved: profile.clone(),
                profile,
                key: String::new(),
                has_key: value.get("hasKey").and_then(Value::as_bool).unwrap_or(false),
                persistent: value.get("persistentStorage").and_then(Value::as_bool).unwrap_or(false),
                remember: value.get("rememberKey").and_then(Value::as_bool).unwrap_or(false),
                saved_remember: value.get("rememberKey").and_then(Value::as_bool).unwrap_or(false),
                // Keep the source key: a saved message must follow live language changes too.
                message: value.get("credentialError").and_then(Value::as_str).map(str::to_owned).or_else(|| success.map(str::to_owned)),
                message_error: value.get("credentialError").and_then(Value::as_str).is_some(),
            };
        }
        Err(e) => {
            app.ui.ai_api.message = Some(e);
            app.ui.ai_api.message_error = true;
        }
    }
}

pub fn load(app: &mut PhotocraftApp, provider: &str) {
    app.ui.ai_api = ApiUi { provider: provider.into(), profile: preset(provider), ..Default::default() };
    let result = app.run(COMMAND, json!({"action":"get","provider":provider}));
    apply(app, provider, result, None);
}

pub fn selector(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) -> bool {
    let before = selected(fields).to_owned();
    let mut provider = before.clone();
    ui.horizontal(|ui| {
        let label = ui.label(tl!("Image service"));
        craft_ui::ComboBox::from_id_salt("ai-image-service")
            .selected_text(match provider.as_str() {
                "gemini" => "Gemini / Nano Banana",
                "seedream" => tl!("Doubao / Seedream"),
                "flux" => "FLUX / Black Forest Labs",
                _ => tl!("Codex (ChatGPT subscription)"),
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut provider, "codex".into(), tl!("Codex (ChatGPT subscription)"));
                ui.selectable_value(&mut provider, "gemini".into(), "Gemini / Nano Banana");
                ui.selectable_value(&mut provider, "seedream".into(), tl!("Doubao / Seedream"));
                ui.selectable_value(&mut provider, "flux".into(), "FLUX / Black Forest Labs");
            })
            .response
            .labelled_by(label.id);
    });
    if provider != before {
        crate::ai_account_ui::closed(app);
        fields.insert("provider".into(), json!(provider));
        if provider == "codex" {
            crate::ai_account_ui::opened(app);
        } else {
            load(app, &provider);
        }
    }
    if provider != "codex" && app.ui.ai_api.provider != provider {
        load(app, &provider);
    }
    provider != "codex"
}

fn protocol_label(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::GeminiInteractions => "Gemini Interactions",
        Protocol::GeminiGenerateContent => "Gemini generateContent",
        Protocol::Seedream => "Seedream",
        Protocol::Flux => "FLUX (BFL)",
    }
}

pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    let provider = selected(fields).to_owned();
    ui.label(tl!("This API sends the visible canvas and instruction to the configured service. Usage may incur charges billed by that service."));
    let draft = &mut app.ui.ai_api;
    let Some(profile) = draft.profile.as_mut() else {
        ui.label(tl!("Could not load API settings"));
        return;
    };
    ui.horizontal(|ui| {
        let label = ui.label(tl!("Model ID"));
        ui.add(egui::TextEdit::singleline(&mut profile.model).id_salt(("ai-api-model", &provider)).desired_width(300.0).char_limit(200)).labelled_by(label.id);
    });
    let open_field = format!("__aiApiSettingsOpen:{provider}");
    let open = fields.entry(open_field.clone()).or_insert_with(|| json!(!draft.has_key)).as_bool().unwrap_or(!draft.has_key);
    let mut save = false;
    let mut remove = false;
    let disclosure =
        egui::CollapsingHeader::new(tl!("API settings")).id_salt(("ai-api-settings", &provider)).default_open(open).open(Some(open)).show(ui, |ui| {
            ui.collapsing(tl!("API address and protocol"), |ui| {
                let label = ui.label(tl!("API address"));
                ui.add(egui::TextEdit::singleline(&mut profile.endpoint).id_salt(("ai-api-endpoint", &provider)).desired_width(400.0).char_limit(2048))
                    .labelled_by(label.id);
                ui.horizontal(|ui| {
                    let label = ui.label(tl!("API protocol"));
                    let before = profile.protocol;
                    craft_ui::ComboBox::from_id_salt("ai-api-protocol")
                        .selected_text(protocol_label(profile.protocol))
                        .show_ui(ui, |ui| {
                            for protocol in [Protocol::GeminiInteractions, Protocol::GeminiGenerateContent, Protocol::Seedream, Protocol::Flux] {
                                ui.selectable_value(&mut profile.protocol, protocol, protocol_label(protocol));
                            }
                        })
                        .response
                        .labelled_by(label.id);
                    if before != profile.protocol {
                        profile.endpoint = match profile.protocol {
                            Protocol::GeminiInteractions => "https://generativelanguage.googleapis.com/v1beta/interactions",
                            Protocol::GeminiGenerateContent => "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent",
                            Protocol::Seedream => "https://ark.cn-beijing.volces.com/api/v3/images/generations",
                            Protocol::Flux => "https://api.bfl.ai/v1/{model}",
                        }
                        .into();
                    }
                });
                ui.weak(tl!("Custom services must support the selected image-editing protocol. Changing the address requires a key for that address."));
            });
            let label = ui.label(if draft.has_key { tl!("API key configured — leave empty to keep it") } else { tl!("API key") });
            ui.add(egui::TextEdit::singleline(&mut draft.key).id_salt(("ai-api-key", &provider)).password(true).desired_width(400.0).char_limit(4096))
                .labelled_by(label.id);
            ui.add_enabled(draft.persistent, egui::Checkbox::new(&mut draft.remember, tl!("Remember API key in system credential store")));
            if !draft.remember {
                ui.weak(tl!("The API key is available for this session only."));
            }
            ui.horizontal_wrapped(|ui| {
                save = ui.button(tl!("Save API settings")).clicked();
                remove = ui.add_enabled(draft.has_key, egui::Button::new(tl!("Remove API key"))).clicked();
            });
        });
    if disclosure.header_response.clicked() {
        fields.insert(open_field, json!(!open));
        ui.ctx().request_repaint();
    }
    if let Some(message) = &draft.message {
        if draft.message_error {
            ui.colored_label(ui.visuals().error_fg_color, tl!(message.as_str()));
        } else {
            ui.label(tl!(message.as_str()));
        }
    }
    if draft.has_key && draft.profile == draft.saved && draft.key.is_empty() && draft.remember == draft.saved_remember {
        ui.weak(tl!("API configured. Saving settings does not generate an image."));
    } else {
        ui.weak(tl!("Save your API settings before editing."));
    }
    if save {
        // Clipboard copies often include a trailing newline or surrounding spaces. Trim only
        // the boundary; internal whitespace and non-ASCII characters must never be removed.
        let key = draft.key.trim();
        if !draft.key.is_empty() && (key.is_empty() || key.len() > 4096 || !key.bytes().all(|b| (33..=126).contains(&b))) {
            draft.message = Some("Enter a valid API key without whitespace".into());
            draft.message_error = true;
        } else {
            let params = json!({"action":"save","provider":provider,"profile":draft.profile,"apiKey":if draft.key.is_empty() { None } else { Some(key) },"rememberKey":draft.remember});
            let result = app.run(COMMAND, params);
            apply(app, &provider, result, Some("API settings saved"));
        }
        ui.ctx().request_repaint();
    } else if remove {
        let result = app.run(COMMAND, json!({"action":"removeKey","provider":provider}));
        apply(app, &provider, result, Some("API key removed"));
    }
    ui.separator();
    crate::view_cmds::form_body(ui, fields);
}

/// Confirmation contains only a provider name and instruction, never a key or draft URL.
pub fn edit_command(fields: &Map<String, Value>, command: String, params: Value) -> (String, Value) {
    if crate::ai_account_ui::owns(fields) && selected(fields) != "codex" {
        ("edit.ai.api".into(), json!({"provider":selected(fields),"prompt":fields.get("prompt")}))
    } else {
        (command, params)
    }
}

pub fn closed(app: &mut PhotocraftApp) {
    app.ui.ai_api = ApiUi::default();
}
