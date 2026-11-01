//! Bounded, allowlisted diagnostics: provider messages can echo keys, prompts or image data.
use super::{Duration, Error, Protocol, Value, error};

const MAX_ERROR_BODY: usize = 64 * 1024;
const FIELDS: &[&str] =
    &["model", "image", "size", "prompt", "stream", "response_format", "output_format", "background", "sequential_image_generation", "watermark"];

pub(super) async fn http(response: reqwest::Response, protocol: Protocol) -> Error {
    let status = response.status().as_u16();
    let detail = if protocol == Protocol::Seedream {
        tokio::time::timeout(Duration::from_secs(2), read(response)).await.ok().flatten().as_ref().and_then(detail)
    } else {
        None
    };
    let hint = match status {
        401 | 403 => "Check the API key, model access and region",
        404 => "Check the API address and model ID",
        429 => "Rate limit or quota exceeded; check the provider console before retrying",
        400 | 422 => "The provider rejected the image, model or request options",
        300..=399 => "Redirects are disabled; enter the final API address",
        _ => "The image service is unavailable; no automatic retry was made",
    };
    error(&format!("Image API HTTP {status}. {}. No layer was added.", detail.as_deref().unwrap_or(hint)))
}

async fn read(mut response: reqwest::Response) -> Option<Value> {
    if response.content_length().is_some_and(|n| n > MAX_ERROR_BODY as u64) {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len().saturating_add(chunk.len()) > MAX_ERROR_BODY {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).ok()
}

/// Only literals from our vocabulary leave this module; never interpolate provider text.
pub(super) fn detail(value: &Value) -> Option<String> {
    let rejected = value.get("error")?;
    let code = rejected.get("code")?.as_str().filter(|code| code.len() <= 128)?;
    let (code, reason) = if let Some(review) = crate::api_error::review_code(code) {
        (review.code, review.kind.description())
    } else {
        match code {
            "InvalidParameter.UnsupportedParameter" => ("InvalidParameter.UnsupportedParameter", "This model does not support the parameter"),
            "InvalidEndpoint.ClosedEndpoint" => ("InvalidEndpoint.ClosedEndpoint", "The model endpoint is closed or unavailable"),
            "InvalidImageURL.InvalidFormat" => ("InvalidImageURL.InvalidFormat", "The provider could not decode the reference image"),
            "InvalidImageURL.EmptyURL" => ("InvalidImageURL.EmptyURL", "The provider received no reference image"),
            // Unknown suffixes are not echoed: even a code field can contain reflected secrets.
            code => match code.split('.').next()? {
                "InvalidParameter" => ("InvalidParameter", "Check the rejected parameter and model requirements"),
                "MissingParameter" => ("MissingParameter", "A required parameter is missing"),
                "InvalidSubscription" => ("InvalidSubscription", "Check account eligibility in the provider console"),
                _ => return None,
            },
        }
    };
    let field = rejected.get("param").and_then(Value::as_str).and_then(known_field).or_else(|| {
        let message = rejected.get("message")?.as_str().filter(|message| message.len() <= 4096)?.to_ascii_lowercase();
        for marker in ["parameter ", "parameters "] {
            if let Some((_, rest)) = message.split_once(marker) {
                let rest = rest.trim_start_matches(['\'', '"', '`']);
                let token = rest.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').next()?;
                if let Some(field) = known_field(token) {
                    return Some(field);
                }
            }
        }
        None
    });
    Some(match field {
        Some(field) => format!("{code}; parameter: {field}. {reason}"),
        None => format!("{code}. {reason}; check the provider console for details"),
    })
}

fn known_field(value: &str) -> Option<&'static str> {
    FIELDS.iter().copied().find(|field| *field == value)
}
