//! Standalone model-based image enlargement; keeps the native source and alpha.
use crate::{
    EngineError, Result, Session,
    ai_cmds::{Snapshot, Source},
    commands::CommandSpec,
};
use photocraft_ai::ImageSize;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, serde::Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct Parameters {
    scale: u32,
    output_size: Option<ImageSize>,
}
impl Default for Parameters {
    fn default() -> Self {
        Self { scale: 2, output_size: None }
    }
}
fn destination(source: ImageSize, p: &Parameters) -> Result<ImageSize> {
    if !(2..=16).contains(&p.scale) {
        return Err(EngineError::Other("AI Upscale scale must be between 2 and 16".into()));
    }
    let output = match p.output_size {
        Some(size) => size,
        None => ImageSize {
            width: source.width.checked_mul(p.scale).ok_or_else(|| EngineError::Other("Upscale width overflow".into()))?,
            height: source.height.checked_mul(p.scale).ok_or_else(|| EngineError::Other("Upscale height overflow".into()))?,
        },
    };
    let ratio_error = (u64::from(output.width) * u64::from(source.height)).abs_diff(u64::from(output.height) * u64::from(source.width));
    if output.width < source.width
        || output.height < source.height
        || output == source
        || output.width > i32::MAX as u32
        || output.height > i32::MAX as u32
        || ratio_error > u64::from(source.width.max(source.height))
    {
        return Err(EngineError::Other("AI Upscale requires a larger size preserving the image aspect ratio".into()));
    }
    Ok(output)
}
fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let p: Parameters = serde_json::from_value(if p.is_null() { json!({}) } else { p.clone() })
        .map_err(|e| EngineError::BadParams { cmd: "image.aiUpscale".into(), msg: e.to_string() })?;
    let snapshot = Snapshot::capture_canvas(s.active().ok_or(EngineError::NoDocument)?, Source::Composite)?;
    let output = destination(snapshot.canvas_size(), &p)?;
    snapshot.validate_upscale_destination(output)?;
    let document = snapshot.document();
    #[cfg(target_arch = "wasm32")]
    {
        let _ = document;
        crate::web_ai_cmds::run(
            s,
            snapshot,
            serde_json::to_value(p).map_err(|e| EngineError::Other(e.to_string()))?,
            "image.aiUpscale",
            "AI Upscale",
            photocraft_ai::Operation::Upscale,
            photocraft_geom::Rect::from_xywh(0, 0, output.width, output.height),
            Some((output, 0, 0)),
        )
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let preferences = s.prefs().integrations.clone();
        crate::jobs::run_external(
            s,
            "image.aiUpscale",
            "AI Upscale",
            Some(document),
            move |ctx| {
                let cancel = ctx.clone();
                let progress = ctx.clone();
                let control =
                    photocraft_ai::Control::connected(move || cancel.cancelled(), move |_, fraction| progress.progress(fraction, "Upscaling with ComfyUI"));
                let backend = photocraft_ai::native::comfy::ComfyUi::configured(
                    &preferences.comfyui_endpoint,
                    &preferences.comfyui_api_key_env,
                    preferences.comfyui_api_key.as_ref(),
                )
                .map_err(map)?;
                photocraft_ai::native::block_on(backend.upscale_preflight(&control)).map_err(map)?;
                snapshot.finish_upscale(output, |patch| {
                    ctx.check()?;
                    photocraft_ai::native::block_on(backend.upscale(patch, &control)).map_err(map)
                })
            },
            |session, result| {
                let id = session.retain_ai_result(result)?;
                Ok(json!({"result":id,"preview":true}))
            },
        )
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn map(error: photocraft_ai::AiError) -> EngineError {
    if error == photocraft_ai::AiError::Cancelled { EngineError::Cancelled } else { EngineError::Other(error.to_string()) }
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "image.aiUpscale",
        label: "AI Upscale…",
        menu: &["Image"],
        shortcut: None,
        params: r#"{"scale":2..16=2,"outputSize":{"width":px,"height":px}?} → {result,preview}; visible native composite, preserve alpha/ICC/aspect, separate Real-ESRGAN workflow, explicit apply/new document"#,
        enabled: crate::ai_runtime_cmds::enabled,
        run,
        journal: false,
    }]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upscale_dimensions_validate_before_provider_requests() {
        let source = ImageSize { width: 1800, height: 1200 };
        assert_eq!(destination(source, &Parameters::default()).unwrap(), ImageSize { width: 3600, height: 2400 });
        for p in [
            Parameters { scale: 0, output_size: None },
            Parameters { scale: 2, output_size: Some(ImageSize { width: 3840, height: 2160 }) },
            Parameters { scale: 2, output_size: Some(source) },
        ] {
            assert!(destination(source, &p).is_err());
        }
        let mut s = Session::new();
        assert!(s.execute("image.aiUpscale", json!({})).is_err());
        s.execute("file.new", json!({"width":32,"height":32})).unwrap();
        for p in [
            json!({"scale":0}),
            json!({"scale":"four"}),
            json!({"outputSize":{"width":0,"height":0}}),
            json!({"outputSize":{"width":300000,"height":300000}}),
            json!({"prompt":"unused"}),
        ] {
            assert!(s.execute("image.aiUpscale", p).is_err());
        }
        assert!(s.jobs().is_empty());
    }
}
