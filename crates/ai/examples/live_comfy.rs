//! Explicit opt-in synthetic integration probe; never contacts a provider without an endpoint argument.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use photocraft_ai::*;
    let mut args = std::env::args().skip(1);
    let endpoint = args.next().ok_or("usage: live_comfy ENDPOINT generate|segment|fill OUTPUT.png")?;
    let mode = args.next().ok_or("missing operation")?;
    let output = args.next().ok_or("missing output PNG")?;
    let operation = match mode.as_str() {
        "generate" | "automatic" => Operation::Generate,
        "segment" => Operation::SegmentSubject,
        "fill" => Operation::Fill,
        _ => return Err("unknown operation".into()),
    };
    let size = ImageSize { width: 512, height: 512 };
    let samples = (0..512)
        .flat_map(|y| {
            (0..512).flat_map(move |x| {
                let cup = (160..352).contains(&x) && (150..390).contains(&y);
                if cup { [0.8, 0.03, 0.03, 1.0] } else { [0.9, 0.85, 0.75, 1.0] }
            })
        })
        .collect();
    let source = if operation == Operation::Generate { None } else { Some(RgbaImage::new(size, samples)?) };
    let edit_mask = if operation == Operation::Fill {
        Some(Mask::new(
            size,
            (0..512).flat_map(|y| (0..512).map(move |x| if (150..360).contains(&x) && (140..400).contains(&y) { 1.0 } else { 0.0 })).collect(),
        )?)
    } else {
        None
    };
    let request = AiRequest {
        operation,
        workflow: WorkflowId { name: if operation == Operation::SegmentSubject { "birefnet" } else { "qwen-image21" }.into(), revision: 1 },
        prompt: "Studio photograph of a blue ceramic mug on a plain beige background, soft light, no text.".into(),
        negative_prompt: None,
        size,
        variants: 1,
        seed: 42123,
        source,
        edit_mask,
        references: vec![],
        prompt_preparation: if mode == "automatic" { PromptPreparation::OpenAiText } else { PromptPreparation::Manual },
    };
    let result = native::block_on(async {
        let provider = native::comfy::ComfyUi::new(&endpoint, "")?;
        let control = Control::default();
        provider.preflight(&request, &control).await?;
        let prompt =
            if mode == "automatic" { Some(native::openai::OpenAiPrompt::new("https://api.openai.com/v1", "OPENAI_API_KEY", "gpt-4.1-mini")?) } else { None };
        generate(&request, &provider, prompt.as_ref().map(|p| p as &dyn PromptProvider), &control).await
    })?;
    let artifact = result.artifacts.first().ok_or("missing result")?;
    let image = match artifact {
        Artifact::Image(image) => image.clone(),
        Artifact::ForegroundMask(mask) => RgbaImage::new(size, mask.values().iter().flat_map(|v| [*v, *v, *v, 1.0]).collect())?,
    };
    std::fs::write(output, native::png(&image)?)?;
    println!("operation={mode}, size={}x{}, own_job={}", size.width, size.height, result.provider_job.as_deref().unwrap_or("none"));
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
