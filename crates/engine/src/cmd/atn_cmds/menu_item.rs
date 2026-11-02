//! Descriptor-free menu intent is distinct from a command with recorded settings.
//! Image Size and unambiguous mapped filter events keep absent settings distinct from empty ones.

use super::events::{EVENTS, Event, event_for};
use super::{AtnEvent, AtnItem, EngineError, Result, Session, Value, bad, json};

/// Only unambiguous mapped filter events and the verified Image Size event have menu identities.
fn menu_event(id: &str) -> Option<&'static Event> {
    if id != "image.imageSize" && !id.starts_with("filter.") {
        return None;
    }
    EVENTS.iter().find(|e| e.ids.len() == 1 && e.ids.first().copied() == Some(id) && crate::commands::find(id).is_some())
}

/// Menu choices use existing event mappings rather than inventing event names from command ids.
pub fn menu_item_commands() -> Vec<&'static str> {
    let mut seen = std::collections::HashSet::new();
    EVENTS.iter().flat_map(|e| e.ids.iter().copied()).filter(|id| menu_event(id).is_some() && seen.insert(*id)).collect()
}

/// Stateless filter menu items execute immediately; settings-bearing ones require the editor.
pub fn menu_item_needs_settings(id: &str) -> bool {
    crate::commands::find(id).is_none_or(|c| !c.params.trim_start().starts_with("{}"))
}

/// An Insert Menu Item step with no recorded command descriptor.
pub const MENU_ITEM: &str = "actions.menuItem";

/// Validate menu identity before the shell gates and opens its actual command.
pub fn menu_item_command(p: &Value) -> Result<&str> {
    let object = p.as_object().ok_or_else(|| bad(MENU_ITEM, "parameters must be an object"))?;
    if object.keys().any(|key| !matches!(key.as_str(), "id" | "eventKind")) {
        return Err(bad(MENU_ITEM, "only `id` and `eventKind` are supported; a menu item has no recorded settings"));
    }
    if object.get("eventKind").is_some_and(|kind| !matches!(kind.as_str(), Some("code" | "name"))) {
        return Err(bad(MENU_ITEM, "`eventKind` must be name or code"));
    }
    let id = p.get("id").and_then(Value::as_str).ok_or_else(|| bad(MENU_ITEM, "select a menu command"))?;
    let event = menu_event(id).ok_or_else(|| bad(MENU_ITEM, "the command has no unambiguous supported menu event"))?;
    if p.get("eventKind").and_then(Value::as_str) == Some("code") && event.code.len() != 4 {
        return Err(bad(MENU_ITEM, "this menu event has only a string name"));
    }
    Ok(id)
}

/// Preserve the exact code/name kind only for the supported descriptor-free event.
pub(super) fn from_event(event: &AtnEvent) -> Option<Value> {
    let mapped = event_for(event)?;
    let id = *mapped.ids.first()?;
    menu_event(id)?;
    let kind = match event {
        AtnEvent::Code(code) if mapped.code.as_bytes() == code => "code",
        AtnEvent::Name(name) if mapped.name.as_bytes() == name => "name",
        _ => return None,
    };
    Some(json!({"id":id,"eventKind":kind}))
}

/// Write absent settings as `None`, never as an empty or current-target descriptor.
pub(super) fn export(p: &Value) -> Option<AtnItem> {
    let mapped = menu_event(menu_item_command(p).ok()?)?;
    let event = if p.get("eventKind").and_then(Value::as_str) == Some("code") {
        AtnEvent::Code(mapped.code.as_bytes().try_into().ok()?)
    } else {
        AtnEvent::name(mapped.name)
    };
    Some(AtnItem { event, dictionary: mapped.label.as_bytes().to_vec(), descriptor: None, ..Default::default() })
}

/// Headless execution runs stateless items and refuses settings deliberately left for a dialog.
pub(super) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let id = menu_item_command(p)?;
    if !menu_item_needs_settings(id) {
        return s.execute(id, json!({}));
    }
    Err(EngineError::Other(format!("interactive-required: `{id}` needs its menu item dialog; no recorded settings were supplied")))
}
