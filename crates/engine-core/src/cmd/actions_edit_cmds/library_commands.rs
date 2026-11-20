//! Clear All Actions, Reset Actions… and Replace Actions… (`actions.replace`, in `atn_cmds`), and
//! the one undo of the last of them (`actions.undoLibrary`).
//!
//! The list keeps the actions and sets from before the command ([`ActionList::library_undo`]).
//! The engine keeps what lies outside the list: the action shortcuts before and after, and where
//! each document's history stood, so Edit › Undo offers the library undo only while no document
//! edit came after it.
use std::collections::BTreeMap;

use craft::automation::actions::{ActionList, ActionListError, LibraryEdit};
use serde_json::{Value, json};

use crate::actions_cmds::{Action, ActionStep, ActionsFile, is_action_key, list_error, not_playing};
use crate::preset_store::PresetStoreExt;
use crate::{Result, Session};

use super::bad;

#[cfg(test)]
mod tests;

type Shortcuts = BTreeMap<String, String>;
type Alternates = BTreeMap<String, Vec<String>>;

/// The set Reset Actions… restores.
pub const BASICS_SET: &str = "PhotoCraft Basics";

/// What the undo of the last Clear All, Reset or Replace restores beside the list.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryUndo {
    /// The command, as the list recorded it; the undo applies only while the list still offers it.
    edit: LibraryEdit,
    /// Each open document's last history row and whether its history was cleared.
    document_history: Vec<(photocraft_doc::DocId, u64, bool)>,
    shortcuts: Shortcuts,
    alternates: Alternates,
    after_shortcuts: Shortcuts,
    after_alternates: Alternates,
    replaced_before_attachment: bool,
}

pub(crate) fn enabled(s: &Session) -> std::result::Result<(), String> {
    idle(s, "actions library").map_err(|error| error.to_string())
}

fn idle(s: &Session, cmd: &str) -> Result<()> {
    not_playing(s, cmd)?;
    if s.recording.target.is_some() {
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
    bindings.iter().filter(|(id, _)| is_action_key(id)).map(|(id, value)| (id.clone(), value.clone())).collect()
}

fn replace_bindings(s: &mut Session, shortcuts: Shortcuts, alternates: Alternates) {
    s.edit_prefs(|prefs| {
        prefs.shortcuts.retain(|id, _| !is_action_key(id));
        prefs.alternate_shortcuts.retain(|id, _| !is_action_key(id));
        prefs.shortcuts.extend(shortcuts);
        prefs.alternate_shortcuts.extend(alternates);
    });
}

/// Refuses a library the preset store could not save: its file is over the store's byte limit.
fn check_library_size(list: &ActionList, cmd: &str) -> Result<()> {
    let text = list.to_file().to_json().map_err(|error| bad(cmd, error.to_string()))?;
    if text.len() as u64 > crate::preset_store::MAX_ACTIONS_BYTES {
        return Err(bad(cmd, "Actions library exceeds the preset store byte limit"));
    }
    Ok(())
}

/// Runs a whole-list command (`edit`, one of [`ActionList::clear_all`], [`ActionList::reset`] or
/// [`ActionList::replace_all`]) on a copy of the list, refuses the result when the store could not
/// save it, then makes it the list and keeps what its undo restores outside the list.
/// `shortcuts` replaces every action shortcut (`None` keeps them).
pub(crate) fn publish_library<R>(s: &mut Session, cmd: &str, shortcuts: Option<Shortcuts>, edit: impl FnOnce(&mut ActionList) -> R) -> Result<R> {
    let mut list = s.actions.list.clone();
    let result = edit(&mut list);
    let kind = list.library_undo().ok_or_else(|| bad(cmd, "not an Actions library command"))?;
    check_library_size(&list, cmd)?;
    let before = (action_bindings(&s.prefs().shortcuts), action_bindings(&s.prefs().alternate_shortcuts));
    let replacing = shortcuts.is_some();
    let after = match shortcuts {
        Some(keys) => (keys, Alternates::new()),
        None => before.clone(),
    };
    if after != before {
        replace_bindings(s, after.0.clone(), after.1.clone());
    }
    let document_history = s.documents().iter().map(|st| (st.doc.id, st.history.row_ids().max().unwrap_or(0), st.history.is_cleared())).collect();
    // A document edit after a library command starts a new undo step.
    for st in &mut s.docs {
        st.coalesce = None;
    }
    let undo = LibraryUndo {
        edit: kind,
        document_history,
        shortcuts: before.0,
        alternates: before.1,
        after_shortcuts: after.0,
        after_alternates: after.1,
        replaced_before_attachment: s.actions.replaced_before_attachment,
    };
    if replacing && s.preset_store().is_none() {
        s.actions.replaced_before_attachment = true;
    }
    s.actions.list = list;
    s.actions.library_undo = Some(Box::new(undo));
    Ok(result)
}

pub(super) fn clear(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "actions.clearAll";
    confirmed_idle(s, p, CMD)?;
    let count = s.actions.list.actions().len();
    let prefs = s.prefs();
    let keys = prefs.shortcuts.keys().chain(prefs.alternate_shortcuts.keys()).any(|id| is_action_key(id));
    // Before the store attaches, an empty list still clears the stored library.
    if count != 0 || !s.actions.list.sets().is_empty() || keys || (s.preset_store().is_none() && !s.actions.replaced_before_attachment) {
        publish_library(s, CMD, Some(Shortcuts::new()), ActionList::clear_all)?;
    }
    Ok(json!({"cleared": count}))
}

/// The actions Reset Actions… restores, in `set`.
fn basics(set: &str) -> ActionsFile {
    let actions = [("Invert Colors", "image.adjustments.invert"), ("Desaturate", "image.adjustments.desaturate")]
        .into_iter()
        .map(|(name, command)| Action { set: Some(set.to_owned()), ..Action::new(name, vec![ActionStep::new(command, json!({}))]) })
        .collect();
    ActionsFile::new(actions, Vec::new())
}

pub(super) fn reset(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "actions.reset";
    confirmed_idle(s, p, CMD)?;
    let append = match p.get("mode").and_then(Value::as_str) {
        Some("append") => true,
        Some("replace") => false,
        _ => return Err(bad(CMD, "`mode` must be append or replace")),
    };
    // Appended beside a set of that name, the defaults get a set of their own (`PhotoCraft Basics 2`).
    let set = if append { crate::preset_import_cmds::UniqueNames::new_exact(s.actions.list.sets()).unique(BASICS_SET) } else { BASICS_SET.to_owned() };
    let defaults = basics(&set);
    let merged = publish_library(s, CMD, (!append).then(Shortcuts::new), |list| list.reset(defaults, append, |_, _| {}))?;
    let names: Vec<&str> = merged.added.iter().map(|&index| s.actions.list.actions()[index].name.as_str()).collect();
    Ok(json!({"set": set, "appended": append, "actions": names}))
}

fn label(edit: LibraryEdit) -> &'static str {
    match edit {
        LibraryEdit::ClearAll => "Undo Clear All Actions",
        LibraryEdit::Reset => "Undo Reset Actions",
        LibraryEdit::Replace => "Undo Replace Actions",
        _ => "Undo Actions Library Change",
    }
}

fn undo_state(s: &Session) -> Result<&LibraryUndo> {
    const CMD: &str = "actions.undoLibrary";
    idle(s, CMD)?;
    let undo = s
        .actions
        .library_undo
        .as_deref()
        .filter(|undo| s.actions.list.library_undo() == Some(undo.edit))
        .ok_or_else(|| list_error(CMD, ActionListError::NothingToUndo))?;
    if action_bindings(&s.prefs().shortcuts) != undo.after_shortcuts || action_bindings(&s.prefs().alternate_shortcuts) != undo.after_alternates {
        return Err(bad(CMD, "Actions shortcuts changed after the library edit"));
    }
    Ok(undo)
}

/// Label for Edit › Undo when the latest library change precedes no undoable document edit.
pub fn undo_library_label(s: &Session) -> Option<&'static str> {
    let undo = undo_state(s).ok()?;
    if let Some(st) = s.active().filter(|st| st.history.can_undo()) {
        let (_, last_id, was_cleared) = undo.document_history.iter().find(|(id, _, _)| *id == st.doc.id).copied().unwrap_or((st.doc.id, 0, false));
        let current_id = st.history.row_id(st.history.past_len()).unwrap_or(0);
        if current_id > last_id || (st.history.is_cleared() && !was_cleared) {
            return None;
        }
    }
    Some(label(undo.edit))
}

pub(super) fn undo_enabled(s: &Session) -> std::result::Result<(), String> {
    undo_state(s).map(|_| ()).map_err(|error| error.to_string())
}

pub(super) fn undo(s: &mut Session, _: &Value) -> Result<Value> {
    const CMD: &str = "actions.undoLibrary";
    undo_state(s)?;
    let undo = s.actions.library_undo.take().ok_or_else(|| list_error(CMD, ActionListError::NothingToUndo))?;
    s.actions.list.undo_library().map_err(|error| list_error(CMD, error))?;
    if s.preset_store().is_none() {
        s.actions.replaced_before_attachment = undo.replaced_before_attachment;
    }
    replace_bindings(s, undo.shortcuts, undo.alternates);
    Ok(json!({"undone": true}))
}
