//! One unpublished action value, a capture-only recorder, and one logical definition Save.
use std::collections::BTreeMap;

use super::{Action, CommandSpec, Result, Session, Value, bad, json, not_playing, replayable};

const OPEN: &str = "actions.editor.open";
const GET: &str = "actions.editor.get";
const UPDATE: &str = "actions.editor.update";
const STEP: &str = "actions.editor.step";
const RECORD: &str = "actions.editor.record";
const STOP: &str = "actions.editor.stop";
const SAVE: &str = "actions.editor.save";
const DISCARD: &str = "actions.editor.discard";
const MAX_STEPS: usize = 100_000;
type Bindings = (BTreeMap<String, String>, BTreeMap<String, Vec<String>>);

/// Runtime-only editor state. Complete values retain fields introduced by later backend phases.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub(super) id: u64,
    value: Action,
    opening: Action,
    source: Option<usize>,
    library_revision: u64,
    shortcut: String,
    opening_shortcut: String,
    keys: BTreeMap<String, String>,
    alternates: BTreeMap<String, Vec<String>>,
    pub(super) recording: Option<usize>,
}

/// Only action bindings participate in the editor's shortcut stale guard.
fn action_keys<T: Clone>(keys: &BTreeMap<String, T>) -> BTreeMap<String, T> {
    keys.iter().filter(|(key, _)| key.starts_with(super::PLAY_SHORTCUT_PREFIX)).map(|(key, value)| (key.clone(), value.clone())).collect()
}

/// Unsupported future fields must fail explicitly instead of being silently discarded.
fn fields(p: &Value, allowed: &[&str], cmd: &str) -> Result<()> {
    let Some(object) = p.as_object() else {
        return if p.is_null() { Ok(()) } else { Err(bad(cmd, "params must be an object")) };
    };
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(bad(cmd, format!("unsupported editor field `{key}`")));
    }
    Ok(())
}

/// Mutating requests must name this editor, never whichever editor happens to be open next.
fn owned<'a>(s: &'a Session, p: &Value, cmd: &str) -> Result<&'a Draft> {
    let id = p.get("draft").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "`draft` must be the editor id"))?;
    s.actions.editor.as_ref().filter(|draft| draft.id == id).ok_or_else(|| bad(cmd, "this editor is no longer open"))
}

/// Pending capture uses the same logical-command filter as Classic recording.
fn pending<'a>(s: &'a Session, draft: &Draft) -> impl Iterator<Item = &'a (String, Value)> {
    s.journal.iter().skip(draft.recording.unwrap_or(s.journal.len())).filter(|(id, _)| replayable(id))
}

/// Build the visible value without flushing into either destination or changing its cursor.
fn visible(s: &Session, draft: &Draft) -> Result<Action> {
    let count = pending(s, draft).count();
    if count > 0 && count > MAX_STEPS.saturating_sub(draft.value.steps.len()) {
        return Err(bad(RECORD, "the draft exceeds the 100000-step limit"));
    }
    let mut value = draft.value.clone();
    value.steps.extend(pending(s, draft).cloned());
    Ok(value)
}

/// Structured inspection serializes the complete Action, not a metadata whitelist.
fn view(draft: &Draft, value: &Action) -> Value {
    json!({"draft":draft.id, "action":value, "shortcut":draft.shortcut,
        "dirty": value != &draft.opening || draft.shortcut != draft.opening_shortcut,
        "recording":draft.recording.is_some(), "source":draft.source, "libraryRevision":draft.library_revision})
}

/// Keep untouched legacy names, but validate newly supplied names before changing a draft.
fn name(value: &Value, old: Option<&str>, cmd: &str) -> Result<String> {
    let text = value.as_str().ok_or_else(|| bad(cmd, "`name` must be a string"))?;
    if Some(text) != old && (text.trim().is_empty() || text.len() > 65_536) {
        return Err(bad(cmd, "a new name must be nonblank and at most 65536 bytes"));
    }
    Ok(text.to_string())
}

/// Allocate only the displayed default name; opening does not create or reserve a library row.
fn new_value(s: &Session, p: &Value) -> Result<Action> {
    let name = match p.get("name") {
        Some(value) => name(value, None, OPEN)?,
        None => {
            let mut n = s.actions.list.len().saturating_add(1);
            while s.actions.list.iter().any(|action| action.name == format!("Action {n}")) {
                n = n.checked_add(1).ok_or_else(|| bad(OPEN, "cannot allocate an action name"))?;
            }
            format!("Action {n}")
        }
    };
    if s.actions.list.iter().any(|action| action.name == name) {
        return Err(bad(OPEN, "an action with this name already exists"));
    }
    let set = match p.get("set") {
        None | Some(Value::Null) => None,
        Some(Value::String(set)) if crate::actions_edit_cmds::set_names(s).contains(set) => Some(set.clone()),
        Some(Value::String(set)) if !set.trim().is_empty() => Some(set.trim().to_string()),
        Some(_) => return Err(bad(OPEN, "`set` must be a nonblank name or null")),
    };
    Ok(Action { set, ..Action::new(name, Vec::new()) })
}

/// Open one complete action clone, without changing the library, shortcuts or preset store.
fn open(s: &mut Session, p: &Value) -> Result<Value> {
    fields(p, &["action", "name", "set"], OPEN)?;
    not_playing(s, OPEN)?;
    if s.actions.editor.is_some() || s.actions.recording_active() {
        return Err(bad(OPEN, "close the open editor or stop recording first"));
    }
    let source = p.get("action").map(|_| super::resolve(&s.actions.list, p, OPEN)).transpose()?;
    if source.is_some() && (p.get("name").is_some() || p.get("set").is_some()) {
        return Err(bad(OPEN, "open an existing action without replacement name/set fields"));
    }
    let value = match source {
        Some(index) => s.actions.list.get(index).cloned().ok_or_else(|| bad(OPEN, "no such action"))?,
        None => new_value(s, p)?,
    };
    let id = s.actions.take.checked_add(1).ok_or_else(|| bad(OPEN, "recording identities exhausted"))?;
    let shortcut = s.prefs().shortcuts.get(&format!("{}{}", super::PLAY_SHORTCUT_PREFIX, value.name)).cloned().unwrap_or_default();
    let draft = Draft {
        id,
        opening: value.clone(),
        value,
        source,
        library_revision: s.actions.rev,
        opening_shortcut: shortcut.clone(),
        shortcut,
        keys: action_keys(&s.prefs().shortcuts),
        alternates: action_keys(&s.prefs().alternate_shortcuts),
        recording: None,
    };
    let response = view(&draft, &draft.value);
    s.actions.take = id;
    s.actions.editor = Some(draft);
    Ok(response)
}

/// Inspect the current draft; an explicit id also rejects a stale read.
fn get(s: &mut Session, p: &Value) -> Result<Value> {
    fields(p, &["draft"], GET)?;
    let draft = if p.get("draft").is_some() { owned(s, p, GET)? } else { s.actions.editor.as_ref().ok_or_else(|| bad(GET, "no editor is open"))? };
    Ok(view(draft, &visible(s, draft)?))
}

/// Only deliberately edited shortcuts are normalized; imported unrelated strings remain intact.
fn shortcut(text: &str, old: &str) -> Result<String> {
    if text == old || text.is_empty() {
        return Ok(text.to_string());
    }
    let (key, shift, command) = crate::atn_cmds::parse_function_key(text).ok_or_else(|| bad(UPDATE, "shortcut must be empty or F1..F15 with Cmd/Shift"))?;
    Ok(crate::atn_cmds::function_key_name(key, shift, command))
}

/// Stage current metadata fields; future backend fields remain in the complete clone untouched.
fn update(s: &mut Session, p: &Value) -> Result<Value> {
    fields(p, &["draft", "name", "color", "shortcut"], UPDATE)?;
    not_playing(s, UPDATE)?;
    let draft = owned(s, p, UPDATE)?;
    let mut next = draft.clone();
    if let Some(value) = p.get("name") {
        next.value.name = name(value, Some(&next.value.name), UPDATE)?;
    }
    if let Some(value) = p.get("color") {
        let color = value.as_str().ok_or_else(|| bad(UPDATE, "`color` must be a string"))?;
        if color != next.value.color && !super::BUTTON_COLORS.contains(&color) {
            return Err(bad(UPDATE, "choose a measured action colour"));
        }
        next.value.color = color.to_string();
    }
    if let Some(value) = p.get("shortcut") {
        next.shortcut = shortcut(value.as_str().ok_or_else(|| bad(UPDATE, "`shortcut` must be a string"))?, &next.shortcut)?;
    }
    let response = view(&next, &visible(s, &next)?);
    s.actions.editor = Some(next);
    Ok(response)
}

/// A user-supplied step index, checked before any draft cursor or flag changes.
fn index(p: &Value, key: &str, len: usize, insert: bool) -> Result<usize> {
    p.get(key)
        .and_then(Value::as_u64)
        .and_then(|i| usize::try_from(i).ok())
        .filter(|i| *i < len || (insert && *i == len))
        .ok_or_else(|| bad(STEP, format!("`{key}` is out of range")))
}

/// Validate newly authored content, without interpreting or normalizing an untouched opaque step.
fn new_step(p: &Value, old: Option<&(String, Value)>, draft: &Draft) -> Result<(String, Value)> {
    let id = match p.get("id") {
        Some(Value::String(id)) => id.as_str(),
        Some(_) => return Err(bad(STEP, "`id` must be a command string")),
        None => old.map(|step| step.0.as_str()).ok_or_else(|| bad(STEP, "supply the step `id`"))?,
    };
    let params = p.get("params").cloned().or_else(|| old.map(|step| step.1.clone())).unwrap_or_else(|| json!({}));
    if old.is_some_and(|step| step.0 == id && step.1 == params) {
        return Ok((id.to_string(), params));
    }
    if !crate::actions_edit_cmds::is_step_command(id) || !params.is_object() {
        return Err(bad(STEP, "a new step needs a known command and object params"));
    }
    if id == "actions.play"
        && (params.get("action").and_then(Value::as_str) == Some(draft.value.name.as_str())
            || draft.source.is_some_and(|source| params.get("action").and_then(Value::as_u64) == Some(source as u64)))
    {
        return Err(bad(STEP, "an action cannot play itself"));
    }
    Ok((id.to_string(), params))
}

/// Remap actual step flags, retaining malformed legacy indices rather than cleaning them up.
fn remap(value: &mut Action, old_len: usize, f: impl Fn(usize) -> Option<usize>) {
    value.remap_steps(|i| if i >= old_len { Some(i) } else { f(i) });
}

/// Insert or replace after complete validation, retaining all unrelated Action fields.
fn insert_or_replace(value: &mut Action, draft: &Draft, p: &Value, op: &str) -> Result<()> {
    let len = value.steps.len();
    let at = index(p, "step", len, op == "insert")?;
    let step = new_step(p, if op == "replace" { value.steps.get(at) } else { None }, draft)?;
    if op == "insert" {
        if len >= MAX_STEPS {
            return Err(bad(STEP, "the draft exceeds the 100000-step limit"));
        }
        value.steps.insert(at, step);
        remap(value, len, |i| Some(if i >= at { i + 1 } else { i }));
    } else if let Some(original) = value.steps.get_mut(at) {
        *original = step;
    }
    Ok(())
}

/// Structural edits copy opaque steps verbatim and keep enabled/dialog flags attached.
fn rearrange(value: &mut Action, p: &Value, op: &str) -> Result<()> {
    let len = value.steps.len();
    let at = index(p, "step", len, false)?;
    match op {
        "delete" => {
            value.steps.remove(at);
            remap(value, len, |i| if i == at { None } else { Some(if i > at { i - 1 } else { i }) });
        }
        "move" => {
            let to = index(p, "to", len, false)?;
            let step = value.steps.remove(at);
            value.steps.insert(to, step);
            remap(value, len, |i| Some(super::moved_index(i, at, to)));
        }
        "duplicate" => {
            if len >= MAX_STEPS {
                return Err(bad(STEP, "the draft exceeds the 100000-step limit"));
            }
            let step = value.steps.get(at).cloned().ok_or_else(|| bad(STEP, "step is out of range"))?;
            let (disabled, dialog) = (value.disabled.contains(&at), value.dialogs.contains(&at));
            value.steps.insert(at + 1, step);
            remap(value, len, |i| Some(if i > at { i + 1 } else { i }));
            if disabled {
                value.disabled.insert(at + 1);
            }
            if dialog {
                value.dialogs.insert(at + 1);
            }
        }
        _ => return Err(bad(STEP, "unknown structural step operation")),
    }
    Ok(())
}

/// Stage one step operation; even invalid operations leave pending recording untouched.
fn step(s: &mut Session, p: &Value) -> Result<Value> {
    not_playing(s, STEP)?;
    let draft = owned(s, p, STEP)?;
    let mut value = visible(s, draft)?;
    let op = p.get("op").and_then(Value::as_str).ok_or_else(|| bad(STEP, "supply a step `op`"))?;
    fields(
        p,
        match op {
            "insert" | "replace" => &["draft", "op", "step", "id", "params"],
            "move" => &["draft", "op", "step", "to"],
            "enabled" | "dialog" => &["draft", "op", "step", "enabled"],
            _ => &["draft", "op", "step"],
        },
        STEP,
    )?;
    match op {
        "insert" | "replace" => insert_or_replace(&mut value, draft, p, op)?,
        "delete" | "move" | "duplicate" => rearrange(&mut value, p, op)?,
        "enabled" | "dialog" => {
            let at = index(p, "step", value.steps.len(), false)?;
            let enabled = p.get("enabled").and_then(Value::as_bool).ok_or_else(|| bad(STEP, "`enabled` must be true or false"))?;
            let flags = if op == "dialog" { &mut value.dialogs } else { &mut value.disabled };
            if enabled == (op == "dialog") {
                flags.insert(at);
            } else {
                flags.remove(&at);
            }
        }
        _ => return Err(bad(STEP, "op must be insert, replace, delete, move, duplicate, enabled or dialog")),
    }
    let mut next = draft.clone();
    next.value = value;
    if next.recording.is_some() {
        next.recording = Some(s.journal.len());
    }
    let response = view(&next, &next.value);
    s.actions.editor = Some(next);
    Ok(response)
}

/// Start a capture-only take; no placeholder action or preset-store revision is created.
fn record(s: &mut Session, p: &Value) -> Result<Value> {
    fields(p, &["draft"], RECORD)?;
    not_playing(s, RECORD)?;
    let mut next = owned(s, p, RECORD)?.clone();
    if s.actions.recording_active() {
        return Err(bad(RECORD, "already recording; stop first"));
    }
    let take = s.actions.take.checked_add(1).ok_or_else(|| bad(RECORD, "recording identities exhausted"))?;
    next.recording = Some(s.journal.len());
    let response = view(&next, &next.value);
    s.actions.take = take;
    s.actions.editor = Some(next);
    Ok(response)
}

/// Flush completed logical commands into the draft only, without publishing it.
pub(super) fn flush(s: &mut Session) -> Result<()> {
    let Some(draft) = s.actions.editor.as_ref().filter(|draft| draft.recording.is_some()) else { return Ok(()) };
    let value = visible(s, draft)?;
    let mut next = draft.clone();
    next.value = value;
    next.recording = Some(s.journal.len());
    s.actions.editor = Some(next);
    Ok(())
}

/// Stop this editor's take while retaining its unpublished steps and normal document edits.
fn stop(s: &mut Session, p: &Value) -> Result<Value> {
    fields(p, &["draft"], STOP)?;
    not_playing(s, STOP)?;
    let draft = owned(s, p, STOP)?;
    if draft.recording.is_none() {
        return Err(bad(STOP, "this draft is not recording"));
    }
    let mut next = draft.clone();
    next.value = visible(s, draft)?;
    next.recording = None;
    let response = view(&next, &next.value);
    s.actions.editor = Some(next);
    Ok(response)
}

/// Reject external library/shortcut edits before preparing any definition publication.
fn unchanged(s: &Session, draft: &Draft) -> Result<()> {
    let source_changed = draft.source.is_some_and(|index| s.actions.list.get(index) != Some(&draft.opening));
    if s.actions.rev != draft.library_revision
        || source_changed
        || action_keys(&s.prefs().shortcuts) != draft.keys
        || action_keys(&s.prefs().alternate_shortcuts) != draft.alternates
    {
        return Err(bad(SAVE, "the action library or action shortcuts changed; keep or discard this draft and reopen"));
    }
    Ok(())
}

/// Rewrite only the existing named call fields, sharing Classic Rename's reference rules.
fn rename_calls(params: &mut Value, id: &str, old: &str, new: &str) {
    for key in super::call_keys(id) {
        if params.get(*key).and_then(Value::as_str) == Some(old) {
            params[*key] = json!(new);
        }
    }
}

/// Only changed references need prepared copies; no library or Session snapshot is taken.
fn references(s: &Session, draft: &Draft, value: &mut Action) -> Vec<(usize, usize, Value)> {
    let mut changes = Vec::new();
    if draft.source.is_none() || draft.opening.name == value.name {
        return changes;
    }
    for (id, params) in &mut value.steps {
        rename_calls(params, id, &draft.opening.name, &value.name);
    }
    for (action, item) in s.actions.list.iter().enumerate().filter(|(i, _)| Some(*i) != draft.source) {
        for (step, (id, params)) in item.steps.iter().enumerate() {
            if super::call_keys(id).iter().any(|key| params.get(*key).and_then(Value::as_str) == Some(draft.opening.name.as_str())) {
                let mut next = params.clone();
                rename_calls(&mut next, id, &draft.opening.name, &value.name);
                changes.push((action, step, next));
            }
        }
    }
    changes
}

/// Prepare only the two binding maps, retaining ordinary menu shortcuts and opaque values.
fn bindings(s: &Session, draft: &Draft, value: &Action) -> Bindings {
    let mut keys = s.prefs().shortcuts.clone();
    let mut alternates = s.prefs().alternate_shortcuts.clone();
    let old = format!("{}{}", super::PLAY_SHORTCUT_PREFIX, draft.opening.name);
    let new = format!("{}{}", super::PLAY_SHORTCUT_PREFIX, value.name);
    if draft.source.is_some() && old != new {
        if let Some(key) = keys.remove(&old) {
            keys.insert(new.clone(), key);
        }
        if let Some(key) = alternates.remove(&old) {
            alternates.insert(new.clone(), key);
        }
    }
    if draft.shortcut != draft.opening_shortcut {
        if !draft.shortcut.is_empty() {
            for (id, key) in &mut keys {
                if id.starts_with(super::PLAY_SHORTCUT_PREFIX) && id != &new && key == &draft.shortcut {
                    key.clear();
                }
            }
        }
        keys.insert(new, draft.shortcut.clone());
    }
    (keys, alternates)
}

/// Atomically publish a validated logical definition update, never a sequence of live commands.
fn save(s: &mut Session, p: &Value) -> Result<Value> {
    fields(p, &["draft"], SAVE)?;
    not_playing(s, SAVE)?;
    let draft = owned(s, p, SAVE)?;
    if s.actions.recording.is_some() {
        return Err(bad(SAVE, "stop Classic recording first"));
    }
    unchanged(s, draft)?;
    let mut value = visible(s, draft)?;
    if value == draft.opening && draft.shortcut == draft.opening_shortcut {
        // Save closes this capture even when it has no completed steps. The committed
        // library, shortcut maps and their revisions still remain exactly unchanged.
        let mut next = draft.clone();
        next.recording = None;
        let response = view(&next, &value);
        s.actions.editor = Some(next);
        return Ok(response);
    }
    if (draft.source.is_none() || value.name != draft.opening.name)
        && s.actions.list.iter().enumerate().any(|(i, action)| Some(i) != draft.source && action.name == value.name)
    {
        return Err(bad(SAVE, "an action with this name already exists"));
    }
    let changes = references(s, draft, &mut value);
    let (keys, alternates) = bindings(s, draft, &value);
    let target = draft.source.unwrap_or(s.actions.list.len());
    let changed = draft.source.is_none() || value != draft.opening || !changes.is_empty();
    let prefs_changed = keys != s.prefs().shortcuts || alternates != s.prefs().alternate_shortcuts;
    let mut next = draft.clone();
    next.value = value.clone();
    next.opening = value.clone();
    next.source = Some(target);
    next.library_revision = if changed { s.actions.rev.saturating_add(1) } else { s.actions.rev };
    next.opening_shortcut = next.shortcut.clone();
    next.keys = action_keys(&keys);
    next.alternates = action_keys(&alternates);
    next.recording = None;
    let response = view(&next, &value);
    if target == s.actions.list.len() {
        s.actions.list.try_reserve(1).map_err(|_| bad(SAVE, "not enough memory for the action"))?;
    }
    // All validation and copies are complete. No command execution or fallible work follows.
    for (action, step, params) in changes {
        if let Some((_, old)) = s.actions.list.get_mut(action).and_then(|action| action.steps.get_mut(step)) {
            *old = params;
        }
    }
    if target == s.actions.list.len() {
        s.actions.list.push(value);
    } else if let Some(old) = s.actions.list.get_mut(target) {
        *old = value;
    }
    if changed {
        s.actions.touch();
    }
    if prefs_changed {
        s.edit_prefs(|prefs| {
            prefs.shortcuts = keys;
            prefs.alternate_shortcuts = alternates;
        });
    }
    s.actions.editor = Some(next);
    Ok(response)
}

/// Discard only this editor, never the live library, another recorder or the global journal.
fn discard(s: &mut Session, p: &Value) -> Result<Value> {
    fields(p, &["draft"], DISCARD)?;
    not_playing(s, DISCARD)?;
    let id = owned(s, p, DISCARD)?.id;
    s.actions.editor = None;
    Ok(json!({"discarded":id}))
}

/// Editor commands are metadata operations, not recorded action steps or menu entries.
fn command(id: &'static str, label: &'static str, params: &'static str, run: fn(&mut Session, &Value) -> Result<Value>) -> CommandSpec {
    CommandSpec { id, label, menu: &[], shortcut: None, params, enabled: super::always, run, journal: false }
}

/// Expose the bounded backend editor contract through normal engine dispatch.
pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        command(
            OPEN,
            "Open Action Draft",
            r#"{"action":name|index?} or {"name":str?,"set":str?} → draft view. One unpublished complete Action; untouched New Save is a no-op."#,
            open,
        ),
        command(
            GET,
            "Get Action Draft",
            r#"{"draft":id?} → {draft,action,shortcut,dirty,recording,source,libraryRevision}. Includes completed pending logical steps without publishing."#,
            get,
        ),
        command(
            UPDATE,
            "Update Action Draft",
            r#"{"draft":id,"name":str?,"color":str?,"shortcut":str?} → draft view. Stage only; measured colours or unchanged opaque original; shortcut empty or Cmd/Shift+F1..F15."#,
            update,
        ),
        command(
            STEP,
            "Edit Action Draft Step",
            r#"{"draft":id,"op":"insert"|"replace"|"delete"|"move"|"duplicate"|"enabled"|"dialog","step":index,"id":command?,"params":object?,"to":index?,"enabled":bool?} → draft view. Bounds/flags validated before mutation; opaque unedited steps retained."#,
            step,
        ),
        command(
            RECORD,
            "Record Action Draft",
            r#"{"draft":id} → draft view. Capture normalized logical commands into this draft only; mutually exclusive with Classic recording."#,
            record,
        ),
        command(
            STOP,
            "Stop Action Draft Recording",
            r#"{"draft":id} → draft view. Completed captured steps stay unpublished. Late jobs do not enter an ended or later take."#,
            stop,
        ),
        command(
            SAVE,
            "Save Action Draft",
            r#"{"draft":id} → clean draft view. Logical atomic definition/reference/shortcut commit after opening-state guards; failures retain the draft. Save ends its own capture, even empty; no-op leaves committed definitions/shortcuts/revisions unchanged. Existing preset I/O warning/retry semantics apply."#,
            save,
        ),
        command(
            DISCARD,
            "Discard Action Draft",
            r#"{"draft":id} → {discarded:id}. Drop only its draft/capture; retain document edits, journal, current library and shortcuts."#,
            discard,
        ),
    ]
}
