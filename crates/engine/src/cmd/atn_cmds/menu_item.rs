//! Descriptor-free menu intent is distinct from a command with recorded settings.
//! Only Image Size is supported here; other absent descriptors keep their existing mapping.

use super::{AtnEvent, AtnItem, EngineError, Result, Session, Value, bad, json};

/// An Insert Menu Item step with no recorded command descriptor.
pub const MENU_ITEM: &str = "actions.menuItem";

/// Validate the supported menu intent before the shell gates and opens its actual command.
pub fn menu_item_command(p: &Value) -> Result<&str> {
    let object = p.as_object().ok_or_else(|| bad(MENU_ITEM, "parameters must be an object"))?;
    if object.keys().any(|key| !matches!(key.as_str(), "id" | "eventKind")) {
        return Err(bad(MENU_ITEM, "only `id` and `eventKind` are supported; a menu item has no recorded settings"));
    }
    if object.get("eventKind").is_some_and(|kind| !matches!(kind.as_str(), Some("code" | "name"))) {
        return Err(bad(MENU_ITEM, "`eventKind` must be name or code"));
    }
    match p.get("id").and_then(Value::as_str) {
        Some("image.imageSize") => Ok("image.imageSize"),
        _ => Err(bad(MENU_ITEM, "only the Image Size menu item is supported")),
    }
}

/// Preserve the exact code/name kind only for the supported descriptor-free event.
pub(super) fn from_event(event: &AtnEvent) -> Option<Value> {
    let kind = match event {
        AtnEvent::Code([b'I', b'm', b'g', b'S']) => "code",
        AtnEvent::Name(name) if name == b"imageSize" => "name",
        _ => return None,
    };
    Some(json!({"id": "image.imageSize", "eventKind": kind}))
}

/// Write absent settings as `None`, never as an empty or current-target descriptor.
pub(super) fn export(p: &Value) -> Option<AtnItem> {
    menu_item_command(p).ok()?;
    let event = if p.get("eventKind").and_then(Value::as_str) == Some("code") { AtnEvent::Code(*b"ImgS") } else { AtnEvent::name("imageSize") };
    Some(AtnItem { event, dictionary: b"Image Size".to_vec(), descriptor: None, ..Default::default() })
}

/// Headless execution cannot supply settings that were deliberately left for a dialog.
pub(super) fn run(_: &mut Session, p: &Value) -> Result<Value> {
    let id = menu_item_command(p)?;
    Err(EngineError::Other(format!("interactive-required: `{id}` needs its menu item dialog; no recorded settings were supplied")))
}
