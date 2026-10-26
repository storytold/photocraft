//! Explicit, document-independent dependency checks and fixed-catalog downloads.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use serde_json::Value;
#[cfg(any(test, not(target_arch = "wasm32")))]
use serde_json::json;
fn enabled(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}
fn run(s: &mut Session, p: &Value, install: bool) -> Result<Value> {
    let input = if install {
        let input: photocraft_ai::dependencies::Install = serde_json::from_value(p.clone()).map_err(|e| EngineError::Other(e.to_string()))?;
        input.selected().map_err(|e| EngineError::Other(e.to_string()))?;
        Some(input)
    } else {
        if !p.is_null() && p.as_object().is_none_or(|o| !o.is_empty()) {
            return Err(EngineError::Other("Dependency check accepts no parameters".into()));
        }
        None
    };
    let command = if install { "comfy.dependencies.install" } else { "comfy.dependencies.check" };
    let label = if install { "Download ComfyUI Models" } else { "Check ComfyUI Dependencies" };
    #[cfg(target_arch = "wasm32")]
    {
        let _ = input;
        crate::web_ai_cmds::run_json(s, None, p.clone(), command, label)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let providers = s.prefs().integrations.clone();
        crate::jobs::run_external(
            s,
            command,
            label,
            None,
            move |ctx| {
                let cancel = ctx.clone();
                let progress = ctx.clone();
                let control =
                    photocraft_ai::Control::connected(move || cancel.cancelled(), move |_, f| progress.progress(f, "Downloading verified ComfyUI models"));
                let result = photocraft_ai::native::block_on(async {
                    let comfy = photocraft_ai::native::comfy::ComfyUi::configured(
                        &providers.comfyui_endpoint,
                        &providers.comfyui_api_key_env,
                        providers.comfyui_api_key.as_ref(),
                    )?;
                    let nodes = comfy.dependency_nodes(&control).await?;
                    let models = if providers.comfyui_dependency_endpoint.is_empty() && input.is_none() {
                        json!({"models":[],"note":"Configure the PhotoCraft dependency service to check installed models and download missing files."})
                    } else {
                        let service = photocraft_ai::native::dependencies::Dependencies::configured(
                            &providers.comfyui_dependency_endpoint,
                            providers.comfyui_dependency_api_key.as_ref(),
                        )?;
                        if let Some(input) = input { service.install(&input, &control).await? } else { service.report(&control).await? }
                    };
                    Ok(json!({"nodes":nodes,"models":models}))
                })
                .map_err(|e| if e == photocraft_ai::AiError::Cancelled { EngineError::Cancelled } else { EngineError::Other(e.to_string()) })?;
                ctx.check()?;
                Ok(result)
            },
            |_, result| Ok(json!({"dependencyReport":result})),
        )
    }
}
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "comfy.dependencies.check",
            label: "Check ComfyUI Dependencies…",
            menu: &["Edit", "AI"],
            shortcut: None,
            params: "{}",
            enabled,
            run: |s, p| run(s, p, false),
            journal: false,
        },
        CommandSpec {
            id: "comfy.dependencies.install",
            label: "Download ComfyUI Models",
            menu: &[],
            shortcut: None,
            params: r#"{"models":[catalog ID],"acceptLicenses":true}"#,
            enabled,
            run: |s, p| run(s, p, true),
            journal: false,
        },
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_download_requests_never_start_a_worker() {
        let mut session = Session::new();
        for p in [json!({}), json!({"models":["../model"],"acceptLicenses":true}), json!({"models":["birefnet"],"acceptLicenses":false})] {
            assert!(session.execute("comfy.dependencies.install", p).is_err());
        }
        assert!(session.jobs().is_empty());
    }
}
