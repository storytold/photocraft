//! Fixed, reviewable model catalog. Clients can request IDs, never paths or URLs.
use crate::{AiError, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Model {
    pub id: String,
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub url: String,
    pub license: String,
    pub license_url: String,
    pub features: Vec<String>,
}
pub fn catalog() -> Result<Vec<Model>> {
    serde_json::from_str(include_str!("dependency-models.json")).map_err(|_| AiError::InvalidRequest("invalid bundled dependency catalog".into()))
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Install {
    pub models: Vec<String>,
    pub accept_licenses: bool,
}
impl Install {
    pub fn selected(&self) -> Result<Vec<Model>> {
        if !self.accept_licenses || self.models.is_empty() || self.models.len() > 5 {
            return Err(AiError::InvalidRequest("Select models and accept their licenses before downloading".into()));
        }
        let catalog = catalog()?;
        let mut selected = Vec::new();
        for id in &self.models {
            if selected.iter().any(|m: &Model| &m.id == id) {
                return Err(AiError::InvalidRequest("duplicate dependency".into()));
            }
            selected.push(catalog.iter().find(|m| &m.id == id).cloned().ok_or_else(|| AiError::InvalidRequest("unknown dependency".into()))?);
        }
        Ok(selected)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_is_fixed_and_install_input_cannot_supply_paths() {
        let models = catalog().unwrap();
        assert_eq!(models.len(), 5);
        for m in &models {
            assert!(m.url.starts_with("https://"));
            assert!(m.license_url.starts_with("https://"));
            assert_eq!(m.sha256.len(), 64);
            assert!(m.sha256.bytes().all(|c| c.is_ascii_hexdigit()));
            assert!(!m.path.contains("..") && !m.path.starts_with('/'));
            assert!(m.size > 0);
        }
        for request in
            [Install { models: vec!["../bad".into()], accept_licenses: true }, Install { models: vec![models[0].id.clone()], accept_licenses: false }]
        {
            assert!(request.selected().is_err());
        }
        assert!(serde_json::from_str::<Install>(r#"{"models":["birefnet"],"acceptLicenses":true,"url":"https://example.invalid"}"#).is_err());
    }
}
