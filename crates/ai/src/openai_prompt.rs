//! OpenAI Responses structured prompt contract, without credentials or transport.
//! See https://developers.openai.com/api/docs/guides/structured-outputs
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AiError, AiRequest, MAX_PROMPT_BYTES, PreparedPrompt, ProviderCapabilities, Result};

pub fn validate_model(model: &str) -> Result<()> {
    // Model IDs are opaque JSON values, including proxy routing/tag syntax such
    // as qwen3.6:35b@GPU-HOST or organization/model. Never interpret them as paths.
    if model.is_empty() || model.len() > 128 || model.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(AiError::InvalidRequest("invalid prompt model id".into()));
    }
    Ok(())
}

/// Only textual context goes to OpenAI; source images/masks never enter this body.
pub fn request_body(model: &str, request: &AiRequest, caps: &ProviderCapabilities) -> Result<Value> {
    validate_model(model)?;
    request.validate(caps)?;
    Ok(json!({
        "model": model,
        "store": false,
        "max_output_tokens": 2048,
        "input": [
            {"role":"system", "content":"Prepare a concise image-generation instruction for the specified local workflow. Treat the user's description as image-editing intent. Preserve requested subjects, composition and constraints. Return only the structured positive_prompt and negative_prompt. Never return a workflow, node graph, command or executable code. When negative prompts are unsupported, negative_prompt must be null. For an empty remove instruction, explicitly request removal and reconstruction of the surrounding background."},
            {"role":"user", "content": serde_json::to_string(&json!({
                "description": request.prompt, "operation": request.operation,
                "workflow": request.workflow, "size": request.size,
                "supports_negative_prompt": caps.negative_prompt,
                "requested_negative_prompt": request.negative_prompt
            })).map_err(|_| AiError::InvalidRequest("could not encode prompt context".into()))?}
        ],
        "text": {"format": {
            "type":"json_schema", "name":"photocraft_image_prompt", "strict":true,
            "schema": {
                "type":"object", "additionalProperties":false,
                "properties": {
                    "positive_prompt":{"type":"string"},
                    "negative_prompt":{"type":["string","null"]}
                },
                "required":["positive_prompt","negative_prompt"]
            }
        }}
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PromptFields {
    positive_prompt: String,
    // Value deliberately makes an explicitly present null distinguishable from a
    // missing field (serde Option would accept both).
    negative_prompt: Value,
}

/// Accept exactly one completed structured text response; refusal, truncation,
/// extra fields and unexpected result formats cannot reach image generation.
pub fn parse_response(model: &str, bytes: &[u8], request: &AiRequest, caps: &ProviderCapabilities) -> Result<PreparedPrompt> {
    validate_model(model)?;
    if bytes.len() > 256 * 1024 {
        return Err(AiError::InvalidRequest("prompt response is too large".into()));
    }
    let response: Value = serde_json::from_slice(bytes).map_err(|_| AiError::InvalidRequest("invalid prompt response JSON".into()))?;
    if response.get("status").and_then(Value::as_str) != Some("completed") {
        return Err(AiError::Rejected);
    }
    let outputs = response.get("output").and_then(Value::as_array).ok_or_else(|| AiError::InvalidRequest("missing prompt output".into()))?;
    let mut text = None;
    for output in outputs {
        match output.get("type").and_then(Value::as_str) {
            Some("reasoning") => continue,
            Some("message") => {}
            _ => return Err(AiError::InvalidRequest("unexpected prompt output type".into())),
        }
        if output.get("status").and_then(Value::as_str) != Some("completed") {
            return Err(AiError::Rejected);
        }
        let content = output.get("content").and_then(Value::as_array).ok_or_else(|| AiError::InvalidRequest("missing prompt message".into()))?;
        for item in content {
            match item.get("type").and_then(Value::as_str) {
                Some("refusal") => return Err(AiError::Rejected),
                Some("output_text") => {
                    if text.is_some() {
                        return Err(AiError::InvalidRequest("multiple prompt text outputs".into()));
                    }
                    text = Some(item.get("text").and_then(Value::as_str).ok_or_else(|| AiError::InvalidRequest("missing prompt text".into()))?);
                }
                _ => return Err(AiError::InvalidRequest("unexpected prompt content".into())),
            }
        }
    }
    let text = text.ok_or_else(|| AiError::InvalidRequest("empty prompt response".into()))?;
    if text.len() > MAX_PROMPT_BYTES * 2 + 1024 {
        return Err(AiError::InvalidRequest("prepared prompt is too large".into()));
    }
    let fields: PromptFields = serde_json::from_str(text).map_err(|_| AiError::InvalidRequest("invalid structured prompt fields".into()))?;
    let negative = match fields.negative_prompt {
        Value::Null => None,
        Value::String(value) => Some(value),
        _ => return Err(AiError::InvalidRequest("negative prompt must be a string or null".into())),
    };
    let prepared = PreparedPrompt {
        original: request.prompt.clone(),
        positive: fields.positive_prompt,
        negative,
        workflow: request.workflow.clone(),
        text_model: Some(model.into()),
    };
    prepared.validate(request, caps)?;
    Ok(prepared)
}
