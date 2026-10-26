//! Bounded binary gateway envelope. Pixels stay in native .pcraft data, not JSON.
use crate::{AiError, Result};
use serde::{Serialize, de::DeserializeOwned};

const MAGIC: &[u8; 8] = b"PCAI0001";
pub const MAX_DOCUMENT_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_METADATA_BYTES: usize = 128 * 1024;
pub const MAX_PREVIEW_BYTES: usize = 1024 * 1024 * 4;
pub const MAX_PACKET_BYTES: usize = MAX_DOCUMENT_BYTES + MAX_METADATA_BYTES + MAX_PREVIEW_BYTES + 12;
pub const JSON_COMMANDS: &[&str] = &["edit.aiDescribeImage", "comfy.dependencies.check", "comfy.dependencies.install"];
pub const COMMANDS: &[&str] = &[
    "comfy.dependencies.check",
    "comfy.dependencies.install",
    "edit.aiDescribeImage",
    "edit.generateImage",
    "edit.aiEdit",
    "edit.generativeFill",
    "edit.generativeRemove",
    "edit.aiBackground",
    "layer.aiRemoveBackground",
    "image.generativeExpand",
    "image.aiUpscale",
];

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub id: String,
    pub command: String,
    pub params: serde_json::Value,
    pub providers: serde_json::Value,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Status {
    pub state: String,
    pub progress: f32,
    pub message: String,
    pub error: Option<String>,
}

pub fn encode<T: Serialize>(metadata: &T, payload: &[u8]) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(metadata).map_err(|_| AiError::InvalidRequest("gateway metadata".into()))?;
    let length = 12usize.checked_add(json.len()).and_then(|n| n.checked_add(payload.len())).filter(|n| *n <= MAX_PACKET_BYTES);
    if json.len() > MAX_METADATA_BYTES || length.is_none() {
        return Err(AiError::StorageFull);
    }
    let mut out = Vec::new();
    out.try_reserve_exact(length.ok_or(AiError::StorageFull)?).map_err(|_| AiError::StorageFull)?;
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(payload);
    Ok(out)
}

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<(T, &[u8])> {
    if bytes.len() > MAX_PACKET_BYTES || bytes.get(..8) != Some(MAGIC.as_slice()) {
        return Err(AiError::InvalidRequest("invalid gateway envelope".into()));
    }
    let raw: [u8; 4] = bytes.get(8..12).and_then(|s| s.try_into().ok()).ok_or_else(|| AiError::InvalidRequest("truncated gateway envelope".into()))?;
    let length = u32::from_le_bytes(raw) as usize;
    if length > MAX_METADATA_BYTES {
        return Err(AiError::StorageFull);
    }
    let end = 12usize.checked_add(length).ok_or(AiError::StorageFull)?;
    let metadata = bytes.get(12..end).ok_or_else(|| AiError::InvalidRequest("truncated gateway metadata".into()))?;
    let payload = bytes.get(end..).ok_or_else(|| AiError::InvalidRequest("truncated gateway payload".into()))?;
    Ok((serde_json::from_slice(metadata).map_err(|_| AiError::InvalidRequest("invalid gateway metadata".into()))?, payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    #[test]
    fn binary_envelope_preserves_payload_and_rejects_hostile_lengths() {
        let bytes = encode(&json!({"model":"qwen3.6:35b@GPU-HOST"}), &[0, 255, 1]).unwrap();
        let (value, payload): (Value, _) = decode(&bytes).unwrap();
        assert_eq!(value["model"], "qwen3.6:35b@GPU-HOST");
        assert_eq!(payload, &[0, 255, 1]);
        for size in 0..bytes.len() - 3 {
            assert!(decode::<Value>(&bytes[..size]).is_err());
        }
        let mut forged = bytes.clone();
        forged[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode::<Value>(&forged).is_err());
        assert!(encode(&"x".repeat(MAX_METADATA_BYTES), &[]).is_err());
    }
}
