//! In-app AI Assistant panel and MCP connector setup for Claude and Gemini subscriptions.

#[cfg(not(target_arch = "wasm32"))]
pub mod connector;
#[cfg(not(target_arch = "wasm32"))]
pub mod gemini_image;
mod panel;
#[cfg(not(target_arch = "wasm32"))]
pub mod providers;

use crate::PhotocraftApp;

/// Which subscription AI provider the in-app panel targets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AiProvider {
    #[default]
    Claude,
    Gemini,
}

impl AiProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Gemini => "Gemini",
        }
    }
}

/// Speaker role for a row in the AI Assistant transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
    Tool,
    System,
}

/// One entry in the panel's conversation log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub text: String,
    pub tool_name: Option<String>,
}

/// Runtime state of the AI Assistant panel.
#[derive(Default)]
pub struct AiAssistantState {
    pub provider: AiProvider,
    /// Optional model override (`""` = provider CLI default; e.g. `"sonnet"`, `"opus"`, `"gemini-2.5-flash"`).
    pub model: String,
    pub input: String,
    pub messages: Vec<ChatMessage>,
    pub claude_session_id: Option<String>,
    pub show_connectors: bool,
    pub status_banner: Option<(bool, String)>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) gemini_api_key_draft: String,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) anthropic_api_key_draft: String,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) keys_loaded: bool,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) detected: Option<providers::DetectedClis>,
    #[cfg(not(target_arch = "wasm32"))]
    runner: Option<providers::ActiveRun>,
    #[cfg(not(target_arch = "wasm32"))]
    image_gen_rx: Option<std::sync::mpsc::Receiver<gemini_image::ImageGenResult>>,
}

impl AiAssistantState {
    /// Whether an AI subprocess turn or image generation is currently active.
    pub fn is_running(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.runner.is_some() || self.image_gen_rx.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }

    /// Cancel any active turn cleanly.
    pub fn cancel(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(run) = self.runner.take() {
                run.cancel();
                self.messages.push(ChatMessage { role: ChatRole::System, text: "Stopped.".to_owned(), tool_name: None });
            }
            if self.image_gen_rx.take().is_some() {
                self.messages.push(ChatMessage { role: ChatRole::System, text: "Stopped image generation.".to_owned(), tool_name: None });
            }
        }
    }

    /// Reset the conversation and session handle.
    pub fn clear_chat(&mut self) {
        self.cancel();
        self.messages.clear();
        self.claude_session_id = None;
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn ensure_keys_loaded(&mut self) {
        if !self.keys_loaded {
            self.keys_loaded = true;
            let loaded = gemini_image::load_keys();
            self.gemini_api_key_draft = loaded.gemini_api_key;
            self.anthropic_api_key_draft = loaded.anthropic_api_key;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn active_gemini_key(&mut self) -> Option<String> {
        self.ensure_keys_loaded();
        let trimmed = self.gemini_api_key_draft.trim();
        if trimmed.is_empty() { None } else { Some(trimmed.to_owned()) }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn active_anthropic_key(&mut self) -> Option<String> {
        self.ensure_keys_loaded();
        let trimmed = self.anthropic_api_key_draft.trim();
        if trimmed.is_empty() { None } else { Some(trimmed.to_owned()) }
    }

    /// Generate a photorealistic image via Gemini API (`gemini-2.5-flash-image`) and place it onto the canvas.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn generate_image_current(&mut self, ctx: &egui::Context) {
        let prompt = self.input.trim().to_owned();
        if prompt.is_empty() || self.is_running() {
            return;
        }
        let Some(api_key) = self.active_gemini_key() else {
            self.show_connectors = true;
            self.messages.push(ChatMessage {
                role: ChatRole::System,
                text: "Enter a free Gemini API key (aistudio.google.com/apikey) in Connectors above to generate images.".to_owned(),
                tool_name: None,
            });
            return;
        };
        self.input.clear();
        self.messages.push(ChatMessage { role: ChatRole::User, text: format!("[Generate Image] {prompt}"), tool_name: None });
        self.messages.push(ChatMessage {
            role: ChatRole::Tool,
            text: "gemini-3.1-flash-image".to_owned(),
            tool_name: Some("gemini_generate_image".to_owned()),
        });
        self.image_gen_rx = Some(gemini_image::spawn_generate(prompt, api_key, ctx.clone()));
    }

    /// Submit `self.input` to the selected subscription CLI.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn send_current(&mut self, ctx: &egui::Context) {
        let prompt = self.input.trim().to_owned();
        if prompt.is_empty() || self.is_running() {
            return;
        }
        self.input.clear();
        self.messages.push(ChatMessage { role: ChatRole::User, text: prompt.clone(), tool_name: None });

        let gemini_api_key = self.active_gemini_key();
        let anthropic_api_key = self.active_anthropic_key();
        let detected = self.detected.get_or_insert_with(providers::DetectedClis::detect);
        let Some(cli_bin) = detected.path_for(self.provider).map(std::path::Path::to_path_buf) else {
            self.messages.push(ChatMessage {
                role: ChatRole::System,
                text: format!("{} CLI was not found on PATH. Install it and sign in with your subscription, or use Connectors above.", self.provider.label()),
                tool_name: None,
            });
            return;
        };

        let req = providers::RunRequest {
            provider: self.provider,
            model: (!self.model.trim().is_empty()).then(|| self.model.trim().to_owned()),
            cli_bin,
            photocraft_cli: detected.photocraft_cli.clone(),
            prompt,
            session_id: self.claude_session_id.clone(),
            gemini_api_key,
            anthropic_api_key,
            ctx: ctx.clone(),
        };
        self.runner = Some(providers::spawn(req));
    }

    /// Drain incremental events from the active worker thread.
    #[cfg(not(target_arch = "wasm32"))]
    fn poll(&mut self, ctx: &egui::Context) {
        let events: Vec<providers::StreamEvent> = {
            let Some(run) = self.runner.as_ref() else { return };
            let mut batch = Vec::new();
            while let Ok(ev) = run.rx.try_recv() {
                batch.push(ev);
            }
            batch
        };
        if events.is_empty() {
            return;
        }
        let mut finished = false;
        for ev in events {
            match ev {
                providers::StreamEvent::TextDelta(delta) => self.append_assistant_delta(&delta),
                providers::StreamEvent::ToolUse { name, summary } => {
                    self.messages.push(ChatMessage { role: ChatRole::Tool, text: summary, tool_name: Some(name) });
                }
                providers::StreamEvent::SessionId(sid) => {
                    self.claude_session_id = Some(sid);
                }
                providers::StreamEvent::Error(err) => {
                    self.messages.push(ChatMessage { role: ChatRole::System, text: err, tool_name: None });
                }
                providers::StreamEvent::Done => {
                    finished = true;
                }
            }
        }
        if finished {
            self.runner = None;
        }
        ctx.request_repaint();
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn append_assistant_delta(&mut self, delta: &str) {
        if let Some(last) = self.messages.last_mut()
            && last.role == ChatRole::Assistant
        {
            last.text.push_str(delta);
            return;
        }
        self.messages.push(ChatMessage { role: ChatRole::Assistant, text: delta.to_owned(), tool_name: None });
    }
}

/// Poll the AI Assistant worker each logic frame.
pub(crate) fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        app.ai_assistant.poll(ctx);
        let img_res = app.ai_assistant.image_gen_rx.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(res) = img_res {
            app.ai_assistant.image_gen_rx = None;
            apply_generated_image(app, res);
            ctx.request_repaint();
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (app, ctx);
}

#[cfg(not(target_arch = "wasm32"))]
fn apply_generated_image(app: &mut PhotocraftApp, res: gemini_image::ImageGenResult) {
    match res {
        gemini_image::ImageGenResult::Ready { bytes, ext, caption } => {
            if app.session.active().is_none() {
                let _ = app.run("file.new", serde_json::json!({"width": 1024, "height": 1024, "name": "Gemini Artwork"}));
            }
            let name = format!("gemini-generated.{ext}");
            let opts = serde_json::json!({"fit": true, "smartObject": true});
            match photocraft_engine::file_cmds::place_bytes(&mut app.session, &name, bytes, None, &opts) {
                Ok(_) => {
                    let note = caption.unwrap_or_else(|| "Placed generated image onto the canvas as a Smart Object layer.".to_owned());
                    app.ai_assistant.messages.push(ChatMessage { role: ChatRole::Assistant, text: note, tool_name: None });
                }
                Err(e) => {
                    app.ai_assistant.messages.push(ChatMessage {
                        role: ChatRole::System,
                        text: format!("Generated image could not be placed: {e}"),
                        tool_name: None,
                    });
                }
            }
        }
        gemini_image::ImageGenResult::Error(err) => {
            app.ai_assistant.messages.push(ChatMessage { role: ChatRole::System, text: err, tool_name: None });
        }
    }
}

/// Cancel any running AI turn when the application exits.
pub(crate) fn shutdown(app: &mut PhotocraftApp) {
    app.ai_assistant.cancel();
}

/// Render the AI Assistant panel.
pub(crate) fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    panel::show(app, ui);
}
