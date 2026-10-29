//! AI commands: bounded snapshots, a worker-owned pipeline and explicit preview application.
//! Nothing here runs until the user has agreed, in Preferences › AI, to send images and prompts
//! to the configured endpoint; automation reaches these commands only if the operator allows it
//! (`photocraft_automation::workspace`).
use crate::{
    EngineError, Result, Session,
    ai_cmds::{Snapshot, Source},
    commands::CommandSpec,
};
use photocraft_ai::{AiRequest, ImageSize, Operation, WorkflowId};
use serde::Deserialize;
use serde_json::{Value, json};

/// The commands that upload document pixels to the provider.
pub const COMMANDS: [&str; 4] = ["edit.generateImage", "edit.aiEdit", "edit.generativeFill", "edit.generativeRemove"];

/// Environment switch that lets automation (MCP, CLI, control channel, action playback) run the
/// AI commands. It is read from the process environment, which none of those channels can change.
pub const AI_UPLOADS_ENV: &str = "PHOTOCRAFT_ALLOW_AUTOMATION_AI_UPLOADS";

/// The gate every automation entry point applies: AI commands upload document pixels to a
/// provider, so they are refused unless the operator allowed it.
pub fn authorize_automation(id: &str, _: &Value) -> Result<()> {
    if COMMANDS.contains(&id) && std::env::var(AI_UPLOADS_ENV).map_or(true, |v| v != "1") {
        return Err(EngineError::Other(format!(
            "automation command `{id}` uploads document pixels to an AI provider and is disabled; the operator can allow it with {AI_UPLOADS_ENV}=1 in the app's environment"
        )));
    }
    Ok(())
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct Parameters {
    prompt: String,
    seed: u64,
    context_margin: u32,
    image_model: Option<String>,
    image_quality: Option<String>,
    output_size: Option<ImageSize>,
}
pub(super) fn enabled(s: &Session) -> std::result::Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = s;
        Err("AI is not available in the browser build".into())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if !s.prefs().integrations.ai_ready() {
            return Err("Turn on AI in Preferences › AI first".into());
        }
        if s.jobs().iter().any(|job| COMMANDS.contains(&job.command.as_str())) {
            return Err("An AI job is already running; wait or cancel it first".into());
        }
        let state = s.active().ok_or_else(|| "No document".to_string())?;
        Snapshot::capture_canvas(state, Source::Composite).map(|_| ()).map_err(|e| e.to_string())
    }
}
fn selected(s: &Session) -> std::result::Result<(), String> {
    enabled(s)?;
    if s.active().and_then(|s| s.doc.selection.as_ref()).is_none_or(|s| s.content_bounds().is_empty()) {
        return Err("Select an area first".into());
    }
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn start(_: &mut Session, _: &Value, _: Operation, command: &'static str, _: &'static str) -> Result<Value> {
    Err(EngineError::Other(format!("{command}: AI is not available in the browser build")))
}
#[cfg(not(target_arch = "wasm32"))]
fn start(s: &mut Session, p: &Value, operation: Operation, command: &'static str, label: &'static str) -> Result<Value> {
    let parameters: Parameters = serde_json::from_value(if p.is_null() { json!({}) } else { p.clone() })
        .map_err(|e| EngineError::BadParams { cmd: command.into(), msg: e.to_string() })?;
    if !s.prefs().integrations.ai_ready() {
        return Err(EngineError::Other("Turn on AI in Preferences › AI first".into()));
    }
    if parameters.prompt.len() > photocraft_ai::MAX_PROMPT_BYTES {
        return Err(EngineError::Other("AI prompt is too long".into()));
    }
    if matches!(operation, Operation::Generate | Operation::Edit | Operation::Fill) && parameters.prompt.trim().is_empty() {
        return Err(EngineError::Other("Enter a prompt".into()));
    }
    if parameters.output_size.is_some() && operation != Operation::Generate {
        return Err(EngineError::Other("Output size presets are only available for new image generation".into()));
    }
    let state = s.active().ok_or(EngineError::NoDocument)?;
    let snapshot = if operation == Operation::Generate {
        Snapshot::capture_canvas(state, Source::Composite)?
    } else {
        Snapshot::capture(state, Source::Composite, parameters.context_margin)?
    };
    let size: ImageSize = parameters.output_size.unwrap_or_else(|| snapshot.size());
    let image_model = parameters.image_model.unwrap_or_else(|| "gpt-image-2.5-sunburst".into());
    photocraft_ai::native::openai::validate_model(&image_model).map_err(|e| EngineError::Other(e.to_string()))?;
    // Generation asks for the requested size directly; edits work on a bounded copy.
    let mut working = if operation == Operation::Generate {
        size.pixels().map_err(|e| EngineError::Other(e.to_string()))?;
        size
    } else {
        Snapshot::working_size(size, 3840)?
    };
    let api = photocraft_ai::image_resolution::openai_size(&image_model, working).map_err(|e| EngineError::Other(e.to_string()))?;
    if operation == Operation::Generate && (api.width < size.width || api.height < size.height) {
        return Err(EngineError::Other("The selected image model cannot generate this size directly; choose a supported size".into()));
    }
    working = ImageSize { width: api.width.min(size.width), height: api.height.min(size.height) };
    if working.width < 32 || working.height < 32 {
        return Err(EngineError::Other("AI working area must have at least 32 pixels on each side".into()));
    }
    let document = snapshot.document();
    let request = AiRequest {
        operation,
        workflow: WorkflowId { name: "openai-images".into(), revision: 1 },
        prompt: parameters.prompt,
        negative_prompt: None,
        size: working,
        variants: 1,
        seed: parameters.seed,
        source: None,
        edit_mask: None,
        references: vec![],
    };
    let preferences = s.prefs().integrations.clone();
    crate::jobs::run_external(
        s,
        command,
        label,
        Some(document),
        move |ctx| {
            let cancel = ctx.clone();
            let progress = ctx.clone();
            let control = photocraft_ai::Control::connected(
                move || cancel.cancelled(),
                move |stage, fraction| {
                    progress.progress(
                        fraction,
                        match stage {
                            photocraft_ai::provider::Stage::Preparing => "Preparing AI input",
                            photocraft_ai::provider::Stage::Generating => "Waiting for the image provider",
                            photocraft_ai::provider::Stage::Ready => "AI preview ready",
                        },
                    )
                },
            );
            ctx.check()?;
            let input = if operation == Operation::Generate { snapshot.prepare_generation(request, size)? } else { snapshot.prepare(request)? };
            let result = photocraft_ai::native::block_on(async {
                let mut images = photocraft_ai::native::openai::OpenAiImages::configured(
                    &preferences.openai_image_endpoint,
                    &preferences.openai_image_api_key_env,
                    preferences.openai_image_api_key.as_ref(),
                    &image_model,
                )?;
                images.set_quality(parameters.image_quality.as_deref().unwrap_or("low"))?;
                images.validate_request(input.request())?;
                photocraft_ai::generate(input.request(), &images, &control).await
            })
            .map_err(|e| if e == photocraft_ai::AiError::Cancelled { EngineError::Cancelled } else { EngineError::Other(e.to_string()) })?;
            ctx.check()?;
            input.finish(result)
        },
        |session, result| {
            let id = session.retain_ai_result(result)?;
            Ok(json!({"result":id,"preview":true}))
        },
    )
}
macro_rules! action {
    ($fn:ident,$op:ident,$id:literal,$label:literal) => {
        fn $fn(s: &mut Session, p: &Value) -> Result<Value> {
            start(s, p, Operation::$op, $id, $label)
        }
    };
}
action!(generate, Generate, "edit.generateImage", "Generate Image…");
action!(edit, Edit, "edit.aiEdit", "Edit Image with AI…");
action!(fill, Fill, "edit.generativeFill", "Generative Fill…");
action!(remove, Remove, "edit.generativeRemove", "Generative Remove…");
pub fn specs() -> Vec<CommandSpec> {
    [("edit.generateImage","Generate Image…",generate as fn(&mut Session,&Value)->Result<Value>,enabled as fn(&Session)->std::result::Result<(),String>),
     ("edit.aiEdit","Edit Image with AI…",edit,enabled),
     ("edit.generativeFill","Generative Fill…",fill,selected),
     ("edit.generativeRemove","Generative Remove…",remove,selected)].into_iter().map(|(id,label,run,enabled)|CommandSpec {
        id,label,run,enabled,menu:&["Edit"][..],shortcut:None,journal:false,
        params:r#"{"prompt":text,"seed":u64=0,"contextMargin":0..1024=0,"imageModel":id="gpt-image-2.5-sunburst","imageQuality":"low|medium|high|xhigh|max|auto"="low","outputSize":{"width":px,"height":px}? (generation only)} → {result,preview}; uploads the working image, mask and prompt to the configured provider (only after the user agreed in Preferences › AI); no automatic document edit"#
     }).collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nothing_runs_until_the_user_agrees_to_the_endpoint() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":64,"height":64})).unwrap();
        assert!(enabled(&s).is_err());
        for cmd in COMMANDS {
            assert!(s.execute(cmd, json!({"prompt":"test"})).is_err(), "{cmd}");
        }
        assert!(s.jobs().is_empty());
    }
    #[test]
    fn invalid_commands_never_contact_a_provider() {
        let mut s = Session::new();
        s.execute("prefs.set", json!({"path":"integrations.aiConsentedEndpoint","value":s.prefs().integrations.openai_image_endpoint.clone()})).unwrap();
        assert!(s.execute("edit.generateImage", json!({"prompt":"test"})).is_err());
        s.execute("file.new", json!({"width":64,"height":64})).unwrap();
        for (cmd, p) in [
            ("edit.generateImage", json!({})),
            ("edit.generativeFill", json!({"prompt":"test"})),
            ("edit.generativeRemove", json!({})),
            ("edit.aiEdit", json!({"prompt":42})),
            ("edit.aiEdit", json!({"prompt":"test","outputSize":{"width":64,"height":64}})),
            ("edit.generateImage", json!({"prompt":"test","unexpected":true})),
        ] {
            assert!(s.execute(cmd, p).is_err(), "{cmd}");
        }
        assert!(s.jobs().is_empty());
    }
}
