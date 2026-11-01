//! BFL's submit/poll/download workflow. Never resubmit a task or forward its key to a CDN.
use super::{ApiProvider, Duration, Error, MAX_IMAGE, Value, error, network_error};
use reqwest::{Client, Response, header::HeaderValue};
use url::Url;

const MAX_METADATA: usize = 1024 * 1024;

fn checked_url(value: &str, origin: &Url, polling: bool) -> Result<Url, Error> {
    if value.len() > 8192 {
        return Err(error("FLUX returned an oversized result address"));
    }
    let url = Url::parse(value).map_err(|_| error("FLUX returned an invalid result address"))?;
    let same = url.origin() == origin.origin();
    let official = matches!(origin.host_str(), Some("api.bfl.ai" | "api.us.bfl.ai" | "api.eu.bfl.ai"));
    let allowed = if polling {
        matches!(url.host_str(), Some("api.bfl.ai" | "api.us.bfl.ai" | "api.eu.bfl.ai"))
    } else {
        url.host_str().is_some_and(|host| host.ends_with(".bfl.ai") || host.ends_with(".bfl.ml"))
    };
    if !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !(same || (official && allowed && url.scheme() == "https" && url.port_or_known_default() == Some(443)))
    {
        return Err(error(
            "FLUX returned an unsupported result host. Polling stays on the configured service or BFL API; downloads use that service or BFL delivery hosts.",
        ));
    }
    Ok(url)
}

async fn bytes(mut response: Response, limit: usize) -> Result<Vec<u8>, Error> {
    if !response.status().is_success() {
        return Err(error(&format!("FLUX HTTP {}. Check the API key, model, quota or service status; no task was resubmitted.", response.status().as_u16())));
    }
    if response.content_length().is_some_and(|size| size > limit as u64) {
        return Err(error("FLUX response exceeds the size limit"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(error("FLUX response exceeds the size limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn metadata(response: Response) -> Result<Value, Error> {
    serde_json::from_slice(&bytes(response, MAX_METADATA).await?).map_err(|_| error("FLUX returned invalid task metadata"))
}

pub(super) async fn edit(provider: &ApiProvider, client: &Client, body: Value, auth: HeaderValue) -> Result<Vec<u8>, Error> {
    let origin = Url::parse(&provider.profile.request_url()).map_err(|_| error("Invalid FLUX API address"))?;
    let submitted = metadata(client.post(origin.clone()).header("x-key", auth.clone()).json(&body).send().await.map_err(network_error)?).await?;
    let poll = submitted.get("polling_url").and_then(Value::as_str).ok_or_else(|| error("FLUX returned no polling address; no task was resubmitted"))?;
    let poll = checked_url(poll, &origin, true)?;
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let result = metadata(client.get(poll.clone()).header("x-key", auth.clone()).send().await.map_err(network_error)?).await?;
        match result.get("status").and_then(Value::as_str) {
            Some("Pending" | "Processing") => continue,
            Some("Ready") => {
                let sample = result.pointer("/result/sample").and_then(Value::as_str).ok_or_else(|| error("FLUX completed without an image"))?;
                let sample = checked_url(sample, &origin, false)?;
                // No API key or other authentication header on result downloads, even on the same host.
                return bytes(client.get(sample).send().await.map_err(network_error)?, MAX_IMAGE).await;
            }
            Some("Error" | "Failed" | "Request Moderated" | "Content Moderated") => {
                return Err(error("FLUX could not complete this image edit or rejected its content. No layer was added."));
            }
            _ => return Err(error("FLUX returned an unsupported task status; no layer was added")),
        }
    }
}

#[cfg(test)]
mod tests;
