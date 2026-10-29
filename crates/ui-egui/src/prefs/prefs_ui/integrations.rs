use super::*;

/// AI is off until the user names an image provider and agrees to send to it. Rendering or
/// saving this section never contacts the provider.
pub(super) fn integrations_fields(ui: &mut egui::Ui, obj: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(tl!("Choose an OpenAI-compatible image provider. Nothing is sent until you run an AI command.")).color(t.text_dim));
    ui.add_space(8.0);
    ui.label(RichText::new(tl!("Image provider")).font(crate::theme::semibold(12.5)).color(t.text));
    egui::Grid::new("api-fields").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        let mut value = obj.get("openaiImageEndpoint").and_then(Value::as_str).unwrap_or_default().to_string();
        let label = ui.label(RichText::new(tl!("API base URL")).color(t.text_dim));
        if ui.add(egui::TextEdit::singleline(&mut value).desired_width(300.0).char_limit(2048)).labelled_by(label.id).changed() {
            obj.insert("openaiImageEndpoint".into(), json!(value));
        }
        ui.end_row();
        let mut value = obj.get("openaiImageApiKey").and_then(Value::as_str).unwrap_or_default().to_string();
        let label = ui.label(RichText::new(tl!("API key")).color(t.text_dim));
        if ui
            .add(egui::TextEdit::singleline(&mut value).password(true).desired_width(300.0).char_limit(4096).hint_text(tl!("Enter or replace API key")))
            .labelled_by(label.id)
            .changed()
        {
            obj.insert("openaiImageApiKey".into(), json!(value));
        }
        ui.end_row();
    });
    if ui.small_button(tl!("Clear API key")).clicked() {
        obj.insert("openaiImageApiKey".into(), json!(""));
    }
    ui.collapsing(tl!("Optional environment variable"), |ui| {
        let mut value = obj.get("openaiImageApiKeyEnv").and_then(Value::as_str).unwrap_or_default().to_string();
        if ui.add(egui::TextEdit::singleline(&mut value).desired_width(300.0).char_limit(128)).changed() {
            obj.insert("openaiImageApiKeyEnv".into(), json!(value));
        }
    });
    ui.label(RichText::new(tl!("Direct image generation and editing using the Images API.")).small().color(t.text_faint));
    ui.add_space(8.0);
    let endpoint = obj.get("openaiImageEndpoint").and_then(Value::as_str).unwrap_or_default().trim_end_matches('/').to_string();
    let mut agreed = !endpoint.is_empty() && obj.get("aiConsentedEndpoint").and_then(Value::as_str).is_some_and(|v| v.trim_end_matches('/') == endpoint);
    if ui.checkbox(&mut agreed, tl!("Turn on AI and send my prompts and images to this provider")).changed() {
        obj.insert("aiConsentedEndpoint".into(), json!(if agreed { endpoint.clone() } else { String::new() }));
    }
    ui.label(
        RichText::new(format!(
            "{} {endpoint}",
            tl!("When you run an AI command, PhotoCraft sends your prompt and, for edits, a working copy of the image or selection (at most 3840 px) with its selection mask to")
        ))
        .small()
        .color(t.text_dim),
    );
    ui.label(
        RichText::new(tl!(
            "Changing the API base URL turns AI off until you agree again. Automation and MCP cannot run AI commands unless the operator allows it."
        ))
        .small()
        .color(t.text_dim),
    );
    ui.label(
        RichText::new(tl!(
            "API keys stay in memory for this session and are never written to your preferences. To keep a key between sessions, name an environment variable instead; a key entered here takes precedence over it."
        ))
        .small()
        .color(t.text_dim),
    );
    if let Ok(integrations) = serde_json::from_value::<prefs::Integrations>(Value::Object(obj.clone()))
        && let Err(error) = integrations.validate()
    {
        ui.add_space(6.0);
        ui.label(RichText::new(error).color(t.danger));
    }
}
