//! Explicit API configuration. Secrets never enter journals, actions, documents or responses.
use crate::{EngineError, Result, Session, commands::CommandSpec};
pub use photocraft_gen::api_config::{Profile, Protocol, preset};
use serde_json::Value;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;

pub const COMMAND: &str = "ai.provider.settings";

#[cfg(not(target_arch = "wasm32"))]
pub fn open_settings(path: std::path::PathBuf) -> Result<photocraft_gen::api_settings::Settings> {
    let namespace = blake3::hash(path.to_string_lossy().as_bytes()).to_hex().to_string();
    photocraft_gen::api_settings::Settings::open(path, namespace).map_err(|e| EngineError::Other(e.to_string()))
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let bad = || EngineError::BadParams { cmd: COMMAND.into(), msg: "Specify action get|save|removeKey and provider gemini|seedream|flux".into() };
        let action = p.get("action").and_then(Value::as_str).ok_or_else(bad)?;
        let provider = p.get("provider").and_then(Value::as_str).filter(|s| photocraft_gen::api_config::PROVIDERS.contains(s)).ok_or_else(bad)?;
        let fail = |e: photocraft_gen::Error| EngineError::Other(e.to_string());
        match action {
            "get" => {}
            "save" => {
                let profile: Profile = serde_json::from_value(p.get("profile").cloned().ok_or_else(bad)?).map_err(|_| bad())?;
                let key = match p.get("apiKey") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(s)) if !s.is_empty() => Some(s.as_str()),
                    _ => return Err(bad()),
                };
                let remember = p.get("rememberKey").and_then(Value::as_bool).ok_or_else(bad)?;
                s.ai_api.save(provider, profile, key, remember).map_err(fail)?;
            }
            "removeKey" => s.ai_api.remove_key(provider).map_err(fail)?,
            _ => return Err(bad()),
        }
        // A locked OS vault must not prevent entering a session-only key. Report its state
        // separately, without returning or formatting the underlying platform error.
        let (has_key, credential_error) = match s.ai_api.has_key(provider) {
            Ok(yes) => (yes, None),
            Err(e) => (false, Some(e.to_string())),
        };
        Ok(
            json!({"profile":s.ai_api.profile(provider).map_err(fail)?,"hasKey":has_key,"rememberKey":s.ai_api.remembered(provider),"persistentStorage":s.ai_api.persistent(),"credentialError":credential_error}),
        )
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (s, p);
        Err(EngineError::Other("API image editing is available in the desktop app".into()))
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: COMMAND,
        label: "AI Provider Settings",
        menu: &[],
        shortcut: None,
        params: r#"{"action":"get|save|removeKey","provider":"gemini|seedream|flux","profile":{protocol,endpoint,model}?,"apiKey":secret?,"rememberKey":bool?}; local settings only, no network; never returns secrets"#,
        enabled: |_| if cfg!(target_arch = "wasm32") { Err("API image editing requires the desktop app".into()) } else { Ok(()) },
        run,
        journal: false,
    }]
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    #[test]
    fn configuration_is_secret_free_and_not_recorded_even_in_actions() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":6})).unwrap();
        s.execute("actions.record", json!({"name":"test"})).unwrap();
        let before = s.journal.clone();
        let result = s
            .execute(COMMAND, json!({"action":"save","provider":"gemini","profile":preset("gemini"),"apiKey":"private-test-key","rememberKey":false}))
            .unwrap();
        assert_eq!(result["hasKey"], true);
        assert!(!result.to_string().contains("private-test-key"));
        assert_eq!(s.journal, before);
        s.execute("actions.stop", json!({})).unwrap();
        assert!(!format!("{:?}", s.actions).contains("private-test-key"));
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
        let result = s.execute(COMMAND, json!({"action":"removeKey","provider":"gemini"})).unwrap();
        assert_eq!(result["hasKey"], false);
    }

    #[test]
    fn invalid_configuration_and_unconfigured_edit_fail_without_network_or_mutation() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":6})).unwrap();
        for p in [
            Value::Null,
            json!({}),
            json!({"action":"save","provider":"other"}),
            json!({"action":"save","provider":"gemini","profile":42}),
            json!({"action":"get","provider":42}),
        ] {
            assert!(s.execute(COMMAND, p).is_err());
        }
        for p in [
            json!({"prompt":"test edit"}),
            json!({"prompt":"test edit","provider":"codex"}),
            json!({"prompt":"test edit","provider":"gemini"}),
            json!({"prompt":"test edit","provider":"flux"}),
        ] {
            assert!(s.execute("edit.ai.api", p).is_err());
        }
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
    }
}
