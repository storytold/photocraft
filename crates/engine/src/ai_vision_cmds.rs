//! Reference interpretation produces editable text, without modifying document pixels.
use crate::{
    EngineError, Result, Session,
    ai_cmds::{Snapshot, Source},
    commands::CommandSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
const COMMAND: &str = "edit.aiDescribeImage";
const LABEL: &str = "Create Prompt from Image…";

#[derive(Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct Parameters {
    instruction: String,
    text_model: Option<String>,
    language: Option<String>,
}
fn run(s: &mut Session, value: &Value) -> Result<Value> {
    let mut p: Parameters = serde_json::from_value(if value.is_null() { json!({}) } else { value.clone() })
        .map_err(|e| EngineError::BadParams { cmd: COMMAND.into(), msg: e.to_string() })?;
    let model = p.text_model.get_or_insert_with(|| s.ai_prompt_model().into()).clone();
    let language = p
        .language
        .get_or_insert_with(|| {
            let lang = &s.prefs().interface.language;
            if lang == "auto" { "en".into() } else { lang.clone() }
        })
        .clone();
    photocraft_ai::vision::request_body(&model, "data:image/png;base64,", &p.instruction, &language).map_err(|e| EngineError::Other(e.to_string()))?;
    let snapshot = Snapshot::capture_canvas(s.active().ok_or(EngineError::NoDocument)?, Source::Composite)?;
    #[cfg(target_arch = "wasm32")]
    {
        crate::web_ai_cmds::run_json(s, Some(snapshot), serde_json::to_value(p).map_err(|e| EngineError::Other(e.to_string()))?, COMMAND, LABEL)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let providers = s.prefs().integrations.clone();
        crate::jobs::run_external(
            s,
            COMMAND,
            LABEL,
            Some(snapshot.document()),
            move |ctx| {
                ctx.check()?;
                ctx.progress(0.05, "Preparing a bounded image for analysis");
                let image = snapshot.analysis_image()?;
                let cancel = ctx.clone();
                let progress = ctx.clone();
                let control = photocraft_ai::Control::connected(move || cancel.cancelled(), move |_, value| progress.progress(value, "Interpreting image"));
                ctx.progress(0.1, "Interpreting image with the configured prompt provider");
                let result = photocraft_ai::native::block_on(async {
                    photocraft_ai::native::openai::OpenAiPrompt::configured(
                        &providers.openai_prompt_endpoint,
                        &providers.openai_prompt_api_key_env,
                        providers.openai_prompt_api_key.as_ref(),
                        &model,
                    )?
                    .interpret(&image, &p.instruction, &language, &control)
                    .await
                })
                .map_err(|e| if e == photocraft_ai::AiError::Cancelled { EngineError::Cancelled } else { EngineError::Other(e.to_string()) })?;
                ctx.check()?;
                Ok(result)
            },
            |_, result| Ok(json!({"imageInterpretation":result})),
        )
    }
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: COMMAND,
        label: LABEL,
        menu: &["Edit", "AI"],
        shortcut: None,
        params: r#"{"instruction":string="","textModel":string?,"language":string?} Uploads a bounded composite to the prompt provider; returns description and recreation prompt."#,
        enabled: super::ai_runtime_cmds::enabled,
        run,
        journal: false,
    }]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interpretation_rejects_invalid_parameters_without_editing() {
        let mut s = Session::new();
        assert!(s.execute(COMMAND, json!({})).is_err());
        s.execute("file.new", json!({"width":32,"height":24})).unwrap();
        let revision = s.active().unwrap().revision;
        for value in [json!({"instruction":"x".repeat(4097)}), json!({"textModel":"bad\nmodel"}), json!({"language":"../en"}), json!({"unknown":true})] {
            assert!(s.execute(COMMAND, value).is_err());
        }
        assert_eq!(s.active().unwrap().revision, revision);
        assert!(s.jobs().is_empty());
    }
}
