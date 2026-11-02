//! Public, non-secret configuration shared by the engine and all frontends.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Protocol {
    GeminiInteractions,
    GeminiGenerateContent,
    Seedream,
    Flux,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub protocol: Protocol,
    /// Full request URL; generateContent additionally substitutes `{model}`.
    pub endpoint: String,
    pub model: String,
}

pub const PROVIDERS: [&str; 3] = ["gemini", "seedream", "flux"];

pub fn preset(provider: &str) -> Option<Profile> {
    match provider {
        "flux" => Some(Profile { protocol: Protocol::Flux, endpoint: "https://api.bfl.ai/v1/{model}".into(), model: "flux-2-pro".into() }),
        "gemini" => Some(Profile {
            protocol: Protocol::GeminiInteractions,
            endpoint: "https://generativelanguage.googleapis.com/v1beta/interactions".into(),
            model: "gemini-nano-banana-2.1".into(),
        }),
        "seedream" => Some(Profile {
            protocol: Protocol::Seedream,
            endpoint: "https://ark.cn-beijing.volces.com/api/v3/images/generations".into(),
            model: "doubao-seedream-5-0-flash-260915".into(),
        }),
        _ => None,
    }
}
