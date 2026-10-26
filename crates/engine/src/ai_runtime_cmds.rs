//! AI commands: bounded snapshots, a worker-owned pipeline and explicit preview application.
use crate::{
    EngineError, Result, Session,
    ai_cmds::{Snapshot, Source},
    commands::CommandSpec,
};
use photocraft_ai::{AiRequest, ImageSize, Operation, PromptPreparation, WorkflowId};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct Parameters {
    prompt: String,
    negative_prompt: Option<String>,
    prepare_prompt: bool,
    text_model: Option<String>,
    seed: u64,
    context_margin: u32,
    expansion: Option<crate::ai_cmds::Expansion>,
    image_provider: Option<String>,
    image_model: Option<String>,
    image_quality: Option<String>,
    output_size: Option<ImageSize>,
    detail_upscale: bool,
    resolution_mode: Option<String>,
    sampling_steps: Option<u32>,
}
pub(super) fn enabled(s: &Session) -> std::result::Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    if s.ai.gateway.is_none() {
        return Err("The browser AI gateway is not configured".into());
    }
    {
        if s.jobs().iter().any(|job| {
            matches!(
                job.command.as_str(),
                "edit.generateImage"
                    | "edit.aiEdit"
                    | "edit.generativeFill"
                    | "edit.generativeRemove"
                    | "edit.aiBackground"
                    | "layer.aiRemoveBackground"
                    | "image.generativeExpand"
                    | "image.aiUpscale"
            )
        }) {
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
fn start(s: &mut Session, p: &Value, operation: Operation, command: &'static str, label: &'static str) -> Result<Value> {
    let parameters: Parameters = serde_json::from_value(if p.is_null() { json!({}) } else { p.clone() })
        .map_err(|e| EngineError::BadParams { cmd: command.into(), msg: e.to_string() })?;
    if parameters.prompt.len() > photocraft_ai::MAX_PROMPT_BYTES
        || parameters.negative_prompt.as_ref().is_some_and(|v| v.len() > photocraft_ai::MAX_PROMPT_BYTES)
    {
        return Err(EngineError::Other("AI prompt is too long".into()));
    }
    if matches!(operation, Operation::Generate | Operation::Edit | Operation::Fill | Operation::Expand) && parameters.prompt.trim().is_empty() {
        return Err(EngineError::Other("Enter a prompt".into()));
    }
    let state = s.active().ok_or(EngineError::NoDocument)?;
    let snapshot = if operation == Operation::Generate {
        Snapshot::capture_canvas(state, Source::Composite)?
    } else {
        Snapshot::capture(state, Source::Composite, parameters.context_margin)?
    };
    let native_generation = match parameters.resolution_mode.as_deref().unwrap_or(if operation == Operation::Generate { "native" } else { "reduced" }) {
        "native" if operation == Operation::Generate => true,
        "reduced" => false,
        _ => return Err(EngineError::Other("Resolution mode must be native (generation only) or reduced".into())),
    };
    if native_generation && parameters.detail_upscale {
        return Err(EngineError::Other("Direct generation does not upscale; use Image > AI Upscale separately".into()));
    }
    if parameters.sampling_steps.is_some_and(|steps| !(1..=100).contains(&steps)) {
        return Err(EngineError::Other("Sampling steps must be between 1 and 100".into()));
    }
    let background = command == "edit.aiBackground";
    if parameters.detail_upscale && operation == Operation::SegmentSubject {
        return Err(EngineError::Other("Foreground cutouts preserve native source pixels and do not use image detail upscaling".into()));
    }
    if parameters.output_size.is_some() && operation != Operation::Generate {
        return Err(EngineError::Other("Output size presets are only available for new image generation".into()));
    }
    let edges = parameters.expansion.unwrap_or_default();
    let size: ImageSize = if background {
        snapshot.canvas_size()
    } else if operation == Operation::Expand {
        snapshot.expansion_size(edges)?
    } else {
        parameters.output_size.unwrap_or_else(|| snapshot.size())
    };
    let direct = match parameters.image_provider.as_deref().unwrap_or("comfyui") {
        "comfyui" => false,
        "openai" => true,
        _ => return Err(EngineError::Other("Unknown AI image provider".into())),
    };
    if direct && (background || operation == Operation::SegmentSubject || operation == Operation::Expand) {
        return Err(EngineError::Other("Direct OpenAI Images supports generate/edit/fill/remove working areas".into()));
    }
    let image_model = parameters.image_model.unwrap_or_else(|| "gpt-image-2.5-sunburst".into());
    let mut working = if native_generation {
        size.pixels().map_err(|e| EngineError::Other(e.to_string()))?;
        size
    } else {
        Snapshot::working_size(size, if direct { 3840 } else { 1024 })?
    };
    if direct {
        let api = photocraft_ai::image_resolution::openai_size(&image_model, working).map_err(|e| EngineError::Other(e.to_string()))?;
        if native_generation && (api.width < size.width || api.height < size.height) {
            return Err(EngineError::Other(
                "The selected OpenAI image model cannot directly generate this size; choose a supported size or explicitly select reduced generation".into(),
            ));
        }
        working = ImageSize { width: api.width.min(size.width), height: api.height.min(size.height) };
    }
    if working.width < 32 || working.height < 32 {
        return Err(EngineError::Other("AI working area must have at least 32 pixels on each side".into()));
    }
    let document = snapshot.document();
    let request = AiRequest {
        operation,
        workflow: WorkflowId {
            name: if direct {
                "openai-images"
            } else if operation == Operation::SegmentSubject {
                "birefnet"
            } else {
                "qwen-image21"
            }
            .into(),
            revision: 1,
        },
        prompt: parameters.prompt,
        negative_prompt: parameters.negative_prompt,
        size: working,
        variants: 1,
        seed: parameters.seed,
        source: None,
        edit_mask: None,
        references: vec![],
        prompt_preparation: if parameters.prepare_prompt { PromptPreparation::OpenAiText } else { PromptPreparation::Manual },
    };
    #[cfg(target_arch = "wasm32")]
    {
        let roi = if operation == Operation::Generate || operation == Operation::Expand || background {
            photocraft_geom::Rect::from_xywh(0, 0, size.width, size.height)
        } else {
            snapshot.roi()
        };
        let canvas = if operation == Operation::Generate {
            Some((size, 0, 0))
        } else if operation == Operation::Expand {
            Some((size, edges.left, edges.top))
        } else {
            None
        };
        let _ = (document, request);
        crate::web_ai_cmds::run(s, snapshot, p.clone(), command, label, if background { Operation::Fill } else { operation }, roi, canvas)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let preferences = s.prefs().integrations.clone();
        photocraft_ai::openai_prompt::validate_model(&image_model).map_err(|e| EngineError::Other(e.to_string()))?;
        let model = parameters.text_model.unwrap_or_else(|| s.ai_prompt_model().into());
        photocraft_ai::openai_prompt::validate_model(&model).map_err(|e| EngineError::Other(e.to_string()))?;
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
                            if background { 0.3 + fraction * 0.7 } else { fraction },
                            match stage {
                                photocraft_ai::provider::Stage::Preparing => "Preparing AI input",
                                photocraft_ai::provider::Stage::Prompting => "Preparing prompt with OpenAI",
                                photocraft_ai::provider::Stage::Generating => {
                                    if direct {
                                        "Generating with OpenAI"
                                    } else {
                                        "Generating with ComfyUI"
                                    }
                                }
                                photocraft_ai::provider::Stage::Ready => "AI preview ready",
                            },
                        )
                    },
                );
                ctx.check()?;
                let mut input = if operation == Operation::Generate {
                    snapshot.prepare_generation(request, size)?
                } else if background {
                    snapshot.prepare_background(request)?
                } else if operation == Operation::Expand {
                    snapshot.prepare_expansion(request, edges)?
                } else {
                    snapshot.prepare(request)?
                };
                let result = photocraft_ai::native::block_on(async {
                    if parameters.detail_upscale {
                        photocraft_ai::native::comfy::ComfyUi::configured(
                            &preferences.comfyui_endpoint,
                            &preferences.comfyui_api_key_env,
                            preferences.comfyui_api_key.as_ref(),
                        )?
                        .upscale_preflight(&control)
                        .await?;
                    }
                    let prompt = if input.request().prompt_preparation == PromptPreparation::OpenAiText {
                        Some(photocraft_ai::native::openai::OpenAiPrompt::configured(
                            &preferences.openai_prompt_endpoint,
                            &preferences.openai_prompt_api_key_env,
                            preferences.openai_prompt_api_key.as_ref(),
                            &model,
                        )?)
                    } else {
                        None
                    };
                    let images: Box<dyn photocraft_ai::ImageProvider> = if direct {
                        let mut images = photocraft_ai::native::openai::OpenAiImages::configured(
                            &preferences.openai_image_endpoint,
                            &preferences.openai_image_api_key_env,
                            preferences.openai_image_api_key.as_ref(),
                            &image_model,
                        )?;
                        images.set_quality(parameters.image_quality.as_deref().unwrap_or("low"))?;
                        images.validate_request(input.request())?;
                        Box::new(images)
                    } else {
                        let mut images = photocraft_ai::native::comfy::ComfyUi::configured(
                            &preferences.comfyui_endpoint,
                            &preferences.comfyui_api_key_env,
                            preferences.comfyui_api_key.as_ref(),
                        )?;
                        if let Some(steps) = parameters.sampling_steps {
                            images.set_sampling_steps(steps)?;
                        }
                        if background {
                            let cancel = ctx.clone();
                            let progress = ctx.clone();
                            let mask_control = photocraft_ai::Control::connected(
                                move || cancel.cancelled(),
                                move |_, fraction| progress.progress(fraction * 0.3, "Finding foreground with ComfyUI"),
                            );
                            let mask_request = input.background_mask_request();
                            images.preflight(&mask_request, &mask_control).await?;
                            let mask_result = photocraft_ai::generate(&mask_request, &images, None, &mask_control).await?;
                            let mask = match mask_result.artifacts.first() {
                                Some(photocraft_ai::Artifact::ForegroundMask(mask)) => mask,
                                _ => return Err(photocraft_ai::AiError::InvalidImage("missing foreground mask".into())),
                            };
                            input.use_background_mask(mask).map_err(|e| photocraft_ai::AiError::InvalidRequest(e.to_string()))?;
                        }
                        images.preflight(input.request(), &control).await?;
                        Box::new(images)
                    };
                    photocraft_ai::generate(input.request(), images.as_ref(), prompt.as_ref().map(|p| p as &dyn photocraft_ai::PromptProvider), &control).await
                })
                .map_err(|e| if e == photocraft_ai::AiError::Cancelled { EngineError::Cancelled } else { EngineError::Other(e.to_string()) })?;
                ctx.check()?;
                if parameters.detail_upscale {
                    let upscaler = photocraft_ai::native::comfy::ComfyUi::configured(
                        &preferences.comfyui_endpoint,
                        &preferences.comfyui_api_key_env,
                        preferences.comfyui_api_key.as_ref(),
                    )
                    .map_err(|e| EngineError::Other(e.to_string()))?;
                    let mut patch = 0u32;
                    input.finish_with_upscaler(result, |image| {
                        ctx.check()?;
                        patch = patch.saturating_add(1);
                        ctx.progress(0.95, &format!("Restoring AI detail, patch {patch}"));
                        photocraft_ai::native::block_on(upscaler.upscale(image, &control))
                            .map_err(|e| if e == photocraft_ai::AiError::Cancelled { EngineError::Cancelled } else { EngineError::Other(e.to_string()) })
                    })
                } else {
                    input.finish(result)
                }
            },
            |session, result| {
                let id = session.retain_ai_result(result)?;
                Ok(json!({"result":id,"preview":true}))
            },
        )
    }
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
action!(segment, SegmentSubject, "layer.aiRemoveBackground", "Remove Background with AI…");
action!(background, Edit, "edit.aiBackground", "Replace Background with AI…");
action!(expand, Expand, "image.generativeExpand", "Generative Expand…");
pub fn specs() -> Vec<CommandSpec> {
    [("edit.generateImage","Generate Image…",generate as fn(&mut Session,&Value)->Result<Value>,enabled as fn(&Session)->std::result::Result<(),String>,&["Edit"][..]),
     ("edit.aiEdit","Edit Image with AI…",edit,enabled,&["Edit"][..]),
     ("edit.aiBackground","Replace Background with AI…",background,enabled,&["Edit"][..]),
     ("edit.generativeFill","Generative Fill…",fill,selected,&["Edit"][..]),
     ("edit.generativeRemove","Generative Remove…",remove,selected,&["Edit"][..]),
     ("layer.aiRemoveBackground","Remove Background with AI…",segment,enabled,&["Layer"][..]),
     ("image.generativeExpand","Generative Expand…",expand,enabled,&["Image"][..])].into_iter().map(|(id,label,run,enabled,menu)|CommandSpec {
        id,label,run,enabled,menu,shortcut:None,journal:false,
        params:r#"{"prompt":text,"negativePrompt":text?,"preparePrompt":bool=false,"textModel":id="gpt-4.1-mini","seed":u64=0,"contextMargin":0..1024=0,"imageProvider":"comfyui|openai"="comfyui","imageModel":id="gpt-image-2.5-sunburst","imageQuality":"low|medium|high|xhigh|max|auto"="low","resolutionMode":"native|reduced" (native by default for generation, reduced for edits),"samplingSteps":1..100? (ComfyUI defaults: 25 generation, 8 edits),"detailUpscale":bool=false (requires reduced generation),"outputSize":{"width":px,"height":px}? (generation only),"expansion":{"left":0,"top":0,"right":128,"bottom":0}} → {result,preview}; native provider workflow, no automatic document edit"#
     }).collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_commands_never_contact_a_provider() {
        let mut s = Session::new();
        assert!(s.execute("edit.generateImage", json!({"prompt":"test"})).is_err());
        s.execute("file.new", json!({"width":64,"height":64})).unwrap();
        for (cmd, p) in [
            ("edit.generateImage", json!({})),
            ("edit.generativeFill", json!({"prompt":"test"})),
            ("edit.generativeRemove", json!({})),
            ("edit.aiEdit", json!({"prompt":42})),
            ("layer.aiRemoveBackground", json!({"unexpected":true})),
            ("edit.generateImage", json!({"prompt":"test","resolutionMode":"native","detailUpscale":true})),
            ("edit.generateImage", json!({"prompt":"test","samplingSteps":0})),
            ("edit.generateImage", json!({"prompt":"test","samplingSteps":101})),
            ("edit.generateImage", json!({"prompt":"test","resolutionMode":"unknown"})),
            ("edit.aiEdit", json!({"prompt":"test","resolutionMode":"native"})),
            ("edit.generateImage", json!({"prompt":"test","imageProvider":"openai","outputSize":{"width":7680,"height":4320}})),
        ] {
            assert!(s.execute(cmd, p).is_err(), "{cmd}");
        }
        assert!(s.jobs().is_empty());
    }
}
