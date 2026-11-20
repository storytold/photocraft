//! The history coalesce key of a gesture, one rule for every control that edits the document
//! through `EditEvent`s: the events of one drag or typed value share a key, so the engine records
//! them as one step.

use craft::behavior::EditEvent;

/// The coalesce key for `event` under `prefix`: `prefix:<gesture id>` while a gesture runs
/// (`Update`, `Commit`, `Cancel`), `None` for `Begin` (the first edit opens the step) and for
/// `Set` (belongs to no gesture, so the engine records a step of its own).
pub(crate) fn coalesce<T>(prefix: &str, event: &EditEvent<T>) -> Option<String> {
    match event {
        EditEvent::Begin { .. } => None,
        _ => event.id().map(|id| format!("{prefix}:{}", id.0)),
    }
}
