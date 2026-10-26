//! Explicit configuration credentials. Debug output never reveals their contents.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Secret(String);
impl Secret {
    pub fn new(value: String) -> crate::Result<Self> {
        let secret = Self(value);
        secret.validate()?;
        Ok(secret)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn validate(&self) -> crate::Result<()> {
        if self.0.len() > 4096 || self.0.chars().any(char::is_control) {
            return Err(crate::AiError::InvalidRequest("API key must contain at most 4096 bytes and no control characters".into()));
        }
        Ok(())
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

/// Redact private credential fields in public preference/control snapshots.
pub fn redact(value: &mut serde_json::Value) {
    fn visit(value: &mut serde_json::Value, depth: usize) {
        if depth >= 64 {
            *value = serde_json::Value::Null;
            return;
        }
        match value {
            serde_json::Value::Object(object) => {
                for (name, value) in object {
                    if ["comfyuiDependencyApiKey", "comfyuiApiKey", "openaiPromptApiKey", "openaiImageApiKey"]
                        .iter()
                        .any(|key| name == key || name == &format!("integrations.{key}"))
                    {
                        *value = serde_json::Value::Null;
                    } else {
                        visit(value, depth + 1);
                    }
                }
            }
            serde_json::Value::Array(array) => {
                for value in array {
                    visit(value, depth + 1);
                }
            }
            _ => {}
        }
    }
    visit(value, 0);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_json_persists_but_debug_and_public_views_redact() {
        let secret = Secret::new("synthetic-private-key".into()).unwrap();
        assert!(!format!("{secret:?}").contains("synthetic-private-key"));
        assert_eq!(serde_json::to_value(&secret).unwrap(), "synthetic-private-key");
        let mut data = serde_json::json!({"values":{"integrations.openaiPromptApiKey":"synthetic-private-key","integrations":{"openaiImageApiKey":"other","comfyuiEndpoint":"http://localhost:8188"}}});
        redact(&mut data);
        assert!(!data.to_string().contains("synthetic-private-key"));
        assert!(!data.to_string().contains("other"));
        assert!(data.to_string().contains("http://localhost:8188"));
        assert!(Secret::new("bad\nkey".into()).is_err());
    }
}
