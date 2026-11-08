//! Confirmed Actions library edits and their single, definition-only undo.
use std::collections::BTreeMap;
use std::io::{self, Write};

use serde::Serialize;
use serde_json::{Value, json};

use crate::actions_cmds::{Action, PLAY_SHORTCUT_PREFIX, not_playing};
use crate::{Result, Session};

use super::bad;

#[cfg(test)]
mod tests;

type Shortcuts = BTreeMap<String, String>;
type Alternates = BTreeMap<String, Vec<String>>;

/// Snapshot of the definitions and shortcut bindings changed by one library edit.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryUndo {
    label: &'static str,
    document_history: Vec<(photocraft_doc::DocId, u64, bool)>,
    list: Vec<Action>,
    sets: Vec<String>,
    rows: Vec<String>,
    shortcuts: Shortcuts,
    alternates: Alternates,
    after_shortcuts: Shortcuts,
    after_alternates: Alternates,
}

pub(crate) fn enabled(s: &Session) -> std::result::Result<(), String> {
    idle(s, "actions library").map_err(|error| error.to_string())
}

fn idle(s: &Session, cmd: &str) -> Result<()> {
    not_playing(s, cmd)?;
    if s.actions.recording.is_some() {
        return Err(bad(cmd, "stop recording before changing the Actions library"));
    }
    Ok(())
}

pub(crate) fn confirmed_idle(s: &Session, p: &Value, cmd: &str) -> Result<()> {
    idle(s, cmd)?;
    if p.get("confirm").and_then(Value::as_bool) != Some(true) {
        return Err(bad(cmd, "changing the Actions library requires `confirm`: true"));
    }
    Ok(())
}

fn action_bindings<V: Clone>(bindings: &BTreeMap<String, V>) -> BTreeMap<String, V> {
    bindings.iter().filter(|(id, _)| id.starts_with(PLAY_SHORTCUT_PREFIX)).map(|(id, value)| (id.clone(), value.clone())).collect()
}

fn replace_bindings(s: &mut Session, shortcuts: Shortcuts, alternates: Alternates) {
    s.edit_prefs(|prefs| {
        prefs.shortcuts.retain(|id, _| !id.starts_with(PLAY_SHORTCUT_PREFIX));
        prefs.alternate_shortcuts.retain(|id, _| !id.starts_with(PLAY_SHORTCUT_PREFIX));
        prefs.shortcuts.extend(shortcuts);
        prefs.alternate_shortcuts.extend(alternates);
    });
}

/// Match the preset store's JSON byte limit before publishing a definition.
pub(crate) fn check_library_size(list: &[Action], sets: &[String], rows: &[String], cmd: &str) -> Result<()> {
    #[derive(Serialize)]
    struct Definition<'a> {
        version: u32,
        actions: &'a [Action],
        #[serde(skip_serializing_if = "<[_]>::is_empty")]
        sets: &'a [String],
        #[serde(skip_serializing_if = "<[_]>::is_empty")]
        set_rows: &'a [String],
    }
    struct Budget(u64);
    impl Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self.0.checked_sub(bytes.len() as u64).ok_or_else(|| io::Error::other("Actions library exceeds the preset store byte limit"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer_pretty(Budget(crate::preset_store::MAX_ACTIONS_BYTES), &Definition { version: 1, actions: list, sets, set_rows: rows })
        .map_err(|error| bad(cmd, error.to_string()))
}

/// Replace only library data. Append uses None to leave both binding maps in place.
pub(crate) fn publish_library(s: &mut Session, list: Vec<Action>, sets: Vec<String>, rows: Vec<String>, new_shortcuts: Option<Shortcuts>, label: &'static str) {
    let shortcuts = action_bindings(&s.prefs().shortcuts);
    let alternates = action_bindings(&s.prefs().alternate_shortcuts);
    let (after_shortcuts, after_alternates) = match new_shortcuts {
        Some(keys) => {
            replace_bindings(s, keys.clone(), Alternates::new());
            (keys, Alternates::new())
        }
        None => (shortcuts.clone(), alternates.clone()),
    };
    let document_history = s.documents().iter().map(|st| (st.doc.id, st.history.row_ids().max().unwrap_or(0), st.history.is_cleared())).collect();
    // A document edit after a library command must start a new undo step.
    for st in &mut s.docs {
        st.coalesce = None;
    }
    let undo = LibraryUndo {
        label,
        document_history,
        list: std::mem::replace(&mut s.actions.list, list),
        sets: std::mem::replace(&mut s.actions.sets, sets),
        rows: std::mem::replace(&mut s.actions.set_rows, rows),
        shortcuts,
        alternates,
        after_shortcuts,
        after_alternates,
    };
    s.actions.set_rows_revision = s.actions.set_rows_revision.wrapping_add(1);
    s.actions.touch();
    s.actions.library_undo = Some(Box::new(undo));
}

pub(super) fn clear(s: &mut Session, p: &Value) -> Result<Value> {
    confirmed_idle(s, p, "actions.clearAll")?;
    let count = s.actions.list.len();
    if count != 0 || !s.actions.sets.is_empty() || !s.actions.set_rows.is_empty() {
        publish_library(s, Vec::new(), Vec::new(), Vec::new(), Some(Shortcuts::new()), "Undo Clear All Actions");
    }
    Ok(json!({"cleared":count}))
}

pub(super) fn reset(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "actions.reset";
    const NAME: &str = "PhotoCraft Basics";
    confirmed_idle(s, p, CMD)?;
    let append = match p.get("mode").and_then(Value::as_str) {
        Some("append") => true,
        Some("replace") => false,
        _ => return Err(bad(CMD, "`mode` must be append or replace")),
    };
    let mut list = if append { s.actions.list.clone() } else { Vec::new() };
    let sets = if append { s.actions.sets.clone() } else { Vec::new() };
    let mut rows = if append { s.actions.set_rows.clone() } else { Vec::new() };
    let slot = rows.len();
    rows.push(NAME.into());
    for (name, command) in [("Invert Colors", "image.adjustments.invert"), ("Desaturate", "image.adjustments.desaturate")] {
        list.push(Action { set: Some(NAME.into()), set_slot: Some(slot), ..Action::new(name, vec![(command.into(), json!({}))]) });
    }
    check_library_size(&list, &sets, &rows, CMD)?;
    publish_library(s, list, sets, rows, (!append).then(Shortcuts::new), "Undo Reset Actions");
    Ok(json!({"set":NAME,"slot":slot,"appended":append,"actions":2}))
}

fn undo_state(s: &Session) -> Result<&LibraryUndo> {
    const CMD: &str = "actions.undoLibrary";
    idle(s, CMD)?;
    let undo = s.actions.library_undo.as_deref().ok_or_else(|| bad(CMD, "no Actions library change is available to undo"))?;
    if action_bindings(&s.prefs().shortcuts) != undo.after_shortcuts || action_bindings(&s.prefs().alternate_shortcuts) != undo.after_alternates {
        return Err(bad(CMD, "Actions shortcuts changed after the library edit"));
    }
    Ok(undo)
}

/// Label for Edit Undo when the latest library change precedes no undoable document edit.
pub fn undo_library_label(s: &Session) -> Option<&'static str> {
    let undo = undo_state(s).ok()?;
    if let Some(st) = s.active().filter(|st| st.history.can_undo()) {
        let (_, last_id, was_cleared) = undo.document_history.iter().find(|(id, _, _)| *id == st.doc.id).copied().unwrap_or((st.doc.id, 0, false));
        let current_id = st.history.row_id(st.history.past_len()).unwrap_or(0);
        if current_id > last_id || (st.history.is_cleared() && !was_cleared) {
            return None;
        }
    }
    Some(undo.label)
}

pub(super) fn undo_enabled(s: &Session) -> std::result::Result<(), String> {
    undo_state(s).map(|_| ()).map_err(|error| error.to_string())
}

pub(super) fn undo(s: &mut Session, _: &Value) -> Result<Value> {
    undo_state(s)?;
    let undo = s.actions.library_undo.take().ok_or_else(|| bad("actions.undoLibrary", "no Actions library change is available to undo"))?;
    s.actions.list = undo.list;
    s.actions.sets = undo.sets;
    s.actions.set_rows = undo.rows;
    s.actions.set_rows_revision = s.actions.set_rows_revision.wrapping_add(1);
    replace_bindings(s, undo.shortcuts, undo.alternates);
    s.actions.touch();
    Ok(json!({"undone":true}))
}
