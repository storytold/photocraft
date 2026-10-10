//! Read-only inventory of bundled and explicitly installed user camera profiles.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use serde_json::{Value, json};

const ID: &str = "raw.profiles";
fn run(_: &mut Session, p: &Value) -> Result<Value> {
    let bad = |s: &str| EngineError::BadParams { cmd: ID.into(), msg: s.into() };
    let object = p.as_object().ok_or_else(|| bad("expected an object"))?;
    if object.keys().any(|k| !matches!(k.as_str(), "model" | "refresh")) {
        return Err(bad("unknown camera-profile query parameter"));
    }
    let model = match p.get("model") {
        None => None,
        Some(v) => Some(v.as_str().filter(|v| v.len() <= 128).ok_or_else(|| bad("model must be text up to 128 bytes"))?.to_lowercase()),
    };
    let refresh = match p.get("refresh") {
        None => false,
        Some(v) => v.as_bool().ok_or_else(|| bad("refresh must be boolean"))?,
    };
    let profiles: Vec<_> = photocraft_io::raw::available_profiles(refresh)
        .into_iter()
        .filter(|p| model.as_ref().is_none_or(|m| p.get("cameraModel").and_then(Value::as_str).is_some_and(|s| s.to_lowercase().contains(m))))
        .collect();
    let models: std::collections::BTreeSet<_> = profiles.iter().filter_map(|p| p.get("cameraModel").and_then(Value::as_str)).collect();
    let bundled = profiles.iter().filter(|p| p.get("source").and_then(Value::as_str) == Some("bundled-dcp")).count();
    Ok(
        json!({"models":models.len(),"count":profiles.len(),"bundledCount":bundled,"profiles":profiles,"source":"bundled-and-user-profiles","rawDecoderSupportIsSeparate":true}),
    )
}
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Camera Profiles",
        menu: &[],
        shortcut: None,
        params: r#"{"model":substring? (all models by default),"refresh":bool=false} -> {models,count,profiles}; local DCP inventory, separate from RAW decoding support"#,
        enabled: crate::commands::always,
        run,
        journal: false,
    }]
}
