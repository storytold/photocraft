//! Window › Actions, as engine commands (`actions.record` / `stop` / `play` / `list` / `get` /
//! `delete` / `move` / `rename` / `toggle`).
//!
//! The list lives on [`Session`] so the panel, the CLI, the control channel and MCP share one
//! copy. Recording copies replayable journal entries (commands whose [`CommandSpec::journal`] is
//! set, except action-editing commands and the already-expanded `edit.transform.again` and
//! `edit.transform.againCopy`) into an action. A replayable command's journal entry replaces the
//! entries of the commands it ran on its own behalf, so they are recorded once (#3067). Playback
//! runs those steps with [`Session::execute`] and stops at the first error, leaving one history
//! entry for the whole play by default (`actions.setHistory` opts into per-step history).
//! Steps switched off with `actions.toggle` (the Actions panel check marks) are skipped by
//! playback, Batch and droplets.
//!
//! An untrusted session installs [`Session::authorize`]. `actions.play` calls it for every nested
//! step, because the outer `actions.play` id would otherwise hide a recorded `file.*` command from
//! the top-level check.

use std::collections::{BTreeSet, VecDeque};

use craft::automation::actions::ActionListError;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The action model is craft-automation's: one list type for the engine, the CLI and the panel.
pub use craft::automation::actions::{Action, ActionColor, ActionList, ActionStep, ActionsFile, moved_index};

mod history;
pub(crate) use history::ActionHistory;
pub use history::{ActionHistoryContinuation, finish_pending_history, suspend_history};
pub(crate) use history::{before_edit as before_action_edit, begin_document_history, finish_document_history, history_rows, reconcile_history_removal};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// The refusal of a rejected [`ActionList`] edit, worded as the list words it.
pub(crate) fn list_error(cmd: &str, error: ActionListError) -> EngineError {
    bad(cmd, error.to_string())
}

/// Mark action `idx` as written by automation when an automation gate is installed.
pub(crate) fn note_automation_edit(s: &mut Session, idx: usize) {
    if s.authorize.is_some() {
        // An index past the end is a stale mark: there is no action to flag.
        let _ = s.actions.list.set_automation(idx, true);
    }
}

/// Mark the action being recorded as automation's when `id` (with `params`) becomes one of its
/// steps while an automation gate is installed: a step a control client, MCP or synthetic input
/// runs during the user's recording keeps automation's gate when the action plays.
pub fn note_automation_step(s: &mut Session, id: &str, params: &Value) {
    note_step_origin(s, s.authorize.is_some(), id, params);
}

/// Mark the action being recorded as automation's when `id` (with `params`) becomes one of its
/// steps on behalf of a request that ran under an automation gate (`automation`). A background
/// job's step is journaled when the job is applied, after its request returned and its gate was
/// removed, so the job passes the origin it captured when it was admitted rather than whatever
/// gate the caller polling it has.
pub(crate) fn note_step_origin(s: &mut Session, automation: bool, id: &str, params: &Value) {
    if automation
        && recorded(id, params)
        && let Some((_, idx)) = s.actions.recording
    {
        let _ = s.actions.list.set_automation(idx, true);
    }
}

/// The gate to install while `action` plays: automation's, for an action automation wrote that
/// plays with none installed. `None` when the session's own gate (or none) applies.
pub fn kept_gate(s: &Session, action: &Action) -> Option<fn(&str, &Value) -> Result<()>> {
    if action.automation && s.authorize.is_none() { s.automation_gate } else { None }
}

pub const MAX_PLAYBACK_STEPS: u64 = 100_000;

/// How deep batches and droplets may run inside one another (a droplet whose steps run a droplet).
pub const MAX_BATCH_DEPTH: u8 = 4;

/// The actions list and the recording / playback flags. The list holds its own sets and
/// revision (the preset store skips unchanged writes by [`ActionList::revision`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionState {
    /// The recorded actions, their sets and their revision: the user's list, the only one stored.
    pub list: ActionList,
    /// The actions a running Batch brought along (a droplet's `calledActions`). Playback finds the
    /// actions it calls here first ([`playable`](Self::playable)); the user's list stays in place.
    pub called: Option<ActionList>,
    /// Painting-tool strokes are recorded only when the user enables them for this session.
    pub record_tools: bool,
    /// Journal length when recording started, and the action index being recorded into.
    pub recording: Option<(usize, usize)>,
    /// Nesting depth of `actions.play`, bounded by the playback stack limit.
    pub playing: u8,
    /// Active action names, for cycle detection in engine and desktop playback.
    pub playback_stack: Vec<String>,
    /// Number of steps executed by the current top-level playback tree.
    pub playback_steps: u64,
    /// Set for a batch nested in a played action or another batch: the playback counter it carries
    /// is shared by every input, so a played action inside it does not restart the count.
    pub budget_shared: bool,
    /// How many batches (`file.automate.batch`, and droplets through it) are running inside one
    /// another: each runs its steps in a scratch session that carries this on, plus one.
    pub batch_depth: u8,
    /// How many scripts (`file.scripts.browse`) are running inside one another.
    pub script_depth: u8,
    /// Steps run by the current top-level script and everything it calls.
    pub script_steps: u64,
    /// What the undo of the last Clear All, Reset or Replace restores beside the list (which keeps
    /// its own, [`ActionList::library_undo`]): the action shortcuts and the document history marks.
    pub library_undo: Option<Box<crate::actions_edit_cmds::LibraryUndo>>,
    /// Clear All, Reset or Replace ran before the preset store attached: the list is the whole
    /// library, so attaching writes it instead of merging the stored one in.
    pub(crate) replaced_before_attachment: bool,
    /// Bumped by each `actions.record`, so a background job knows whether the recording it was
    /// started in is still the one running when its result lands.
    pub take: u64,
}

impl ActionState {
    /// The list playback finds actions in: a running Batch's [`called`](Self::called) actions,
    /// else the user's list.
    pub fn playable(&self) -> &ActionList {
        self.called.as_ref().unwrap_or(&self.list)
    }
}

/// Visibility gestures name a layer for replay, since document-local ids change on another image.
pub(crate) fn recorded_params(s: &Session, id: &str, mut p: Value) -> Value {
    if s.actions.recording.is_none() || id != "layer.setProps" || p.get("visible").is_none() {
        return p;
    }
    let Some(st) = s.active() else { return p };
    let Some(layer) = p.get("layer").and_then(Value::as_u64).map(photocraft_doc::LayerId).or(st.active_layer).and_then(|id| st.doc.layer(id)) else { return p };
    let ordinal = st.doc.walk().into_iter().filter(|(_, _, l)| l.name == layer.name).position(|(_, _, l)| l.id == layer.id).unwrap_or(0);
    if let Some(object) = p.as_object_mut() {
        object.remove("layer");
        object.remove("coalesce");
        object.insert("_actionLayer".into(), json!({"name": layer.name, "ordinal": ordinal}));
    }
    p
}

/// Resolve the recorded visibility target identically in engine and shell playback, and drop
/// the `.atn` event or Custom descriptor remembered for Save Actions: the command, its journal
/// entry (Last Filter), a smart filter and the step's dialog never see it.
pub fn playback_params(s: &Session, id: &str, p: &Value) -> Result<Value> {
    let mut p = p.clone();
    if id == "type.create" && (p.get("xPercent").is_some() || p.get("yPercent").is_some()) {
        if p.get("box").is_some() || p.get("x").is_some() || p.get("y").is_some() {
            return Err(bad(id, "recorded type position cannot mix percentages with pixel coordinates or a box"));
        }
        let size = s.active().ok_or(EngineError::NoDocument)?.doc.size;
        for (percent_key, pixel_key, dimension) in [("xPercent", "x", size.width), ("yPercent", "y", size.height)] {
            let percent = p.get(percent_key).and_then(Value::as_f64).ok_or_else(|| bad(id, format!("invalid `{percent_key}`")))?;
            let pixel = percent / 100.0 * f64::from(dimension);
            if !pixel.is_finite() || pixel.abs() > f64::from(f32::MAX) {
                return Err(bad(id, format!("`{percent_key}` is outside the canvas coordinate range")));
            }
            if let Some(object) = p.as_object_mut() {
                object.remove(percent_key);
                object.insert(pixel_key.into(), json!(pixel));
            }
        }
    }
    if (id.starts_with("filter.gallery.") || id == crate::atn_cmds::MENU_ITEM)
        && let Some(object) = p.as_object_mut()
    {
        object.remove(crate::atn_cmds::RECORDED_EVENT);
    }
    if id == "filter.other.custom"
        && let Some(object) = p.as_object_mut()
    {
        object.remove("__atnCustom");
    }
    if id == "layer.setProps"
        && let Some(target) = p.get("_actionLayer")
    {
        let name = target.get("name").and_then(Value::as_str).ok_or_else(|| bad(id, "missing recorded layer name"))?;
        let ordinal =
            target.get("ordinal").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).ok_or_else(|| bad(id, "invalid recorded layer ordinal"))?;
        let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
        let layer = doc
            .walk()
            .into_iter()
            .filter(|(_, _, l)| l.name == name)
            .nth(ordinal)
            .map(|(_, _, l)| l.id)
            .ok_or_else(|| bad(id, format!("recorded layer `{name}` does not exist")))?;
        if let Some(object) = p.as_object_mut() {
            object.remove("_actionLayer");
            object.insert("layer".into(), json!(layer.0));
        }
    }
    Ok(p)
}

/// Replay commands and named action calls, excluding edits to the action definitions and the
/// already-expanded Again wrapper.
pub fn replayable(id: &str) -> bool {
    // Again and Again on a Copy journal their concrete steps (`edit.transform`, after
    // `layer.duplicate` for the copy) and record those: the wrappers look up the last transform in
    // the journal, so they fail where no transform ran yet, e.g. an action played in a new session.
    id == "actions.play"
        || (!id.starts_with("actions.")
            // Browse executes its steps through Session::execute, so those concrete
            // commands are already journaled. Recording the wrapper as well replays
            // every script edit twice (#3057).
            && !matches!(id, "edit.transform.again" | "edit.transform.againCopy" | "file.scripts.browse")
            && (shell_view_command(id) || shell_save_command(id) || crate::commands::find(id).is_some_and(|c| c.journal)))
}

/// File › Save, Save As and Save a Copy, and File › Export › Export As, recorded by the desktop
/// shell once the file has a path (#2032, #4373, #7207). The shell replays them; batch and
/// droplets write to their destination folder instead.
pub fn shell_save_command(id: &str) -> bool {
    matches!(id, "file.save" | "file.saveAs" | "file.saveACopy" | "file.export.exportAs")
}

/// The failure for a played Save step whose document `target` (the one active when the step
/// started) is no longer open: a Save Document script event closed it. Playback stops there, so
/// the next step without a document, such as the Close half of an imported Close with Save: Yes,
/// doesn't act on whichever document is active now (#7367).
pub fn save_target_closed(s: &Session, id: &str, target: Option<photocraft_doc::DocId>) -> Option<String> {
    let target = target.filter(|_| shell_save_command(id))?;
    (!s.documents().iter().any(|d| d.doc.id == target)).then(|| {
        "the document was closed during the save (by a Save Document script event); playback stopped so later steps don't act on another document".into()
    })
}

/// Charge one step against the complete nested playback tree.
pub fn consume_playback_step(s: &mut Session) -> bool {
    // Inside a script the step also counts against the script's total: each top-level playback
    // resets its own counter, so a script playing actions in a row would never reach the limit.
    if s.actions.script_depth > 0 {
        if s.actions.script_steps >= crate::automate_cmds::MAX_SCRIPT_STEPS {
            return false;
        }
        s.actions.script_steps += 1;
    }
    if s.actions.playback_steps >= MAX_PLAYBACK_STEPS {
        false
    } else {
        s.actions.playback_steps += 1;
        true
    }
}

/// Steps of the playback tree the next `actions.play` starts in has charged so far: a top-level
/// call starts a new tree ([`begin_playback`] resets the counter).
fn playback_steps_used(s: &Session) -> u64 {
    if s.actions.playing > 0 || s.actions.budget_shared { s.actions.playback_steps } else { 0 }
}

/// Steps of `action` playback walks past without running them: the ones before `from` and the
/// disabled ones after it. [`begin_playback`] charges them, so every stored step costs one unit
/// of the playback budget once per call, run or not.
fn skipped_steps(action: &Action, from: usize) -> u64 {
    let len = action.steps.len();
    let from = from.min(len);
    let disabled = action.disabled.range(from..len).count();
    u64::try_from(from.saturating_add(disabled)).unwrap_or(u64::MAX)
}
/// View-only menu commands recorded by the desktop shell. Headless playback reports
/// them as unsupported instead of silently dropping them; Batch and droplets skip them.
pub fn shell_view_command(id: &str) -> bool {
    matches!(id, "view.fitOnScreen" | "view.actualPixels" | "view.zoomIn" | "view.zoomOut")
}

/// Shared validation for headless and shell playback. Does not start playback. Returns the
/// action's index in [`ActionState::playable`], the action and the step playback starts at.
pub fn playback_plan(s: &Session, p: &Value) -> Result<(usize, Action, usize)> {
    if s.actions.playing > 0 && s.actions.playback_stack.is_empty() {
        return Err(bad("actions.play", "an action is already playing"));
    }
    let list = s.actions.playable();
    let idx = resolve(list.actions(), p, "actions.play")?;
    // Every stored step of the call is charged to the playback tree (run, disabled or skipped),
    // so refuse a call whose steps no longer fit before copying it: repeated calls to a large
    // action would otherwise copy and walk it each time for free.
    let len = list.step_count(idx);
    if playback_steps_used(s).saturating_add(u64::try_from(len).unwrap_or(u64::MAX)) > MAX_PLAYBACK_STEPS {
        // A limit, not a bad parameter: an enclosing batch must fail the input instead of saving it.
        return Err(EngineError::BatchLimit(format!("invalid parameters for `actions.play`: action playback exceeded the {MAX_PLAYBACK_STEPS}-step limit")));
    }
    let action = list.action(idx).cloned().ok_or_else(|| bad("actions.play", "no such action"))?;
    // The recording's index is into the user's list, which a Batch's called actions stand in for.
    let records_it = s.actions.called.is_none() && s.actions.recording.is_some_and(|(_, i)| i == idx);
    if s.actions.playback_stack.iter().any(|name| name == &action.name) || records_it {
        return Err(bad("actions.play", format!("recursive action call to `{}`", action.name)));
    }
    if s.actions.playback_stack.len() >= 16 {
        return Err(bad("actions.play", "action nesting exceeds 16 levels"));
    }
    let from = from_step(p, action.steps.len())?;
    Ok((idx, action, from))
}

/// The (command, params) of each step of an action, as [`visit_calls`] takes them.
fn step_calls(steps: &mut [ActionStep]) -> impl Iterator<Item = (&str, &mut Value)> {
    steps.iter_mut().map(|step| (step.command.as_str(), &mut step.params))
}

/// The (command, params) of each journal or batch step pair, as [`visit_calls`] takes them.
pub(crate) fn pair_calls(steps: &mut [(String, Value)]) -> impl Iterator<Item = (&str, &mut Value)> {
    steps.iter_mut().map(|(id, params)| (id.as_str(), params))
}

/// Visit calls in the current namespace, leaving embedded inline Batch scopes untouched.
fn visit_calls<'a>(steps: impl IntoIterator<Item = (&'a str, &'a mut Value)>, f: &mut impl FnMut(&str, &mut Value, bool)) {
    let mut pending: Vec<_> = steps.into_iter().map(|(id, params)| (id.to_owned(), params)).collect();
    pending.reverse();
    while let Some((id, params)) = pending.pop() {
        let scoped = id == "file.automate.batch" && params.get("calledActions").is_some_and(|v| !v.is_null());
        if id == "file.automate.batch" {
            let inline = if params.get("steps").is_some() { Some("steps") } else { params.get("action").is_some_and(Value::is_array).then_some("action") };
            if let Some(key) = inline {
                if scoped {
                    continue;
                }
                // A present invalid `steps` still wins over `action`, just as at runtime.
                let Some(steps) = params.get_mut(key).and_then(Value::as_array_mut) else { continue };
                for step in steps.iter_mut().rev() {
                    let call = match step {
                        Value::Array(a) => {
                            let Some(id) = a.first().and_then(Value::as_str).map(str::to_owned) else { continue };
                            a.get_mut(1).map(|params| (id, params))
                        }
                        Value::Object(o) => {
                            let Some(id) = o.get("command").or_else(|| o.get("id")).and_then(Value::as_str).map(str::to_owned) else { continue };
                            o.get_mut("params").map(|params| (id, params))
                        }
                        // String steps and omitted params use `{}`, with no action target.
                        _ => None,
                    };
                    if let Some(call) = call {
                        pending.push(call);
                    }
                }
                continue;
            }
        }
        f(&id, params, scoped);
    }
}

/// Shift the numeric selectors of one step of the session list after `count` actions were
/// inserted at `at`, including calls in unscoped inline Batch steps. It is the `shift` hook of
/// [`ActionList::duplicate_action`] and [`ActionList::duplicate_set`].
pub(crate) fn shift_action_calls(step: &mut ActionStep, at: usize, count: usize) {
    visit_calls(step_calls(std::slice::from_mut(step)), &mut |id, params, _| {
        if !matches!(id, "actions.play" | "file.automate.batch") {
            return;
        }
        let Some(value) = params.get("action") else { return };
        let Some(index) = num(value).and_then(|i| usize::try_from(i).ok()) else { return };
        if index >= at {
            params["action"] = if let Some(fractional) = value.as_f64().filter(|_| value.is_f64()) {
                json!(fractional + count as f64)
            } else {
                json!(index.saturating_add(count))
            };
        }
    });
}

/// Capture transitive Play Action, Conditional and Batch dependencies for a standalone droplet
/// (#2794, #6804). Ordinary references become names; normalized copies keep source-list order.
/// A named Batch with its own action list selects an untouched source copy at an exported index,
/// after the ordinary copies. Inline embedded bodies retain their own references. Missing
/// targets remain deferred failures, as before.
pub fn called_actions(list: &[Action], steps: &mut [(String, Value)]) -> Vec<Action> {
    /// Resolve in the current namespace before capture rewrites the reference.
    fn target(list: &[Action], params: &Value, id: &str) -> Result<usize> {
        if id == "file.automate.batch" { crate::file_cmds::batch_action(list, params) } else { resolve(list, params, id) }
    }
    /// Capture ordinary calls transitively, but retain an untouched source for scoped named Batch.
    fn targets<'a>(
        list: &[Action],
        steps: impl IntoIterator<Item = (&'a str, &'a mut Value)>,
        depth: usize,
        queue: &mut VecDeque<(usize, usize)>,
        scoped: &mut BTreeSet<usize>,
    ) {
        if depth > 16 {
            return;
        }
        visit_calls(steps, &mut |id, params, own_actions| match id {
            "actions.play" | "file.automate.batch" => {
                let Ok(i) = target(list, params, id) else { return };
                // The reference is rewritten by `remap` once the exported list is known.
                if own_actions {
                    scoped.insert(i);
                } else {
                    queue.push_back((i, depth));
                }
            }
            "actions.conditional" => {
                for key in ["then", "else"] {
                    let Some(name) = params.get(key).and_then(Value::as_str) else { continue };
                    if let Some(i) = list.iter().position(|a| a.name == name) {
                        queue.push_back((i, depth));
                    }
                }
            }
            _ => {}
        });
    }
    let mut queue = VecDeque::new();
    let mut scoped = BTreeSet::new();
    targets(list, pair_calls(steps), 1, &mut queue, &mut scoped);
    let mut found = std::collections::BTreeMap::new();
    // Visit shallow calls first, so a deeper path cannot hide a reachable dependency.
    while let Some((i, depth)) = queue.pop_front() {
        if depth > 16 || found.contains_key(&i) {
            continue;
        }
        let Some(mut action) = list.get(i).cloned() else { continue };
        // Disabled steps never play, so the actions they call are not dependencies.
        for (step, call) in action.steps.iter_mut().enumerate() {
            if !action.disabled.contains(&step) {
                targets(list, step_calls(std::slice::from_mut(call)), depth + 1, &mut queue, &mut scoped);
            }
        }
        found.insert(i, action);
    }
    // Where each captured source action lands in the exported list (source order).
    let exported: std::collections::BTreeMap<usize, usize> = found.keys().enumerate().map(|(at, &i)| (i, at)).collect();
    let mut found: Vec<_> = found.into_values().collect();
    let names: Vec<String> = found.iter().map(|a| a.name.clone()).collect();
    let mut originals = Vec::new();
    let mut slots = std::collections::BTreeMap::new();
    for i in scoped {
        if let Some(action) = list.get(i) {
            slots.insert(i, found.len() + originals.len());
            originals.push(action.clone());
        }
    }
    // A named Batch selects from the caller before switching to its embedded list. Its source
    // must keep raw references, even when normal Play needs a normalized copy of the same action.
    let final_len = found.len() + originals.len();
    let mut remap = |id: &str, params: &mut Value, scoped: bool| {
        if !matches!(id, "actions.play" | "file.automate.batch") {
            return;
        }
        if scoped
            && let Ok(i) = target(list, params, id)
            && let Some(slot) = slots.get(&i)
        {
            params["action"] = json!(slot);
            // The pinned source slot identifies its set; a numeric Batch choice cannot carry a named set.
            if let Some(object) = params.as_object_mut() {
                object.remove("set");
            }
            return;
        }
        // An ordinary call becomes the captured action's name, which reads well and resolves to
        // it while the name is unique in the exported list. Two captured actions of one name (in
        // different sets) would both resolve to the first, so such a call keeps its exported index.
        if !scoped && let Ok(i) = target(list, params, id) {
            let Some(name) = list.get(i).map(|a| a.name.clone()) else { return };
            match exported.get(&i) {
                Some(&at) if names.iter().filter(|n| **n == name).count() > 1 => {
                    params["action"] = json!(at);
                    if let Some(object) = params.as_object_mut() {
                        object.remove("set");
                    }
                }
                _ => params["action"] = json!(name),
            }
            return;
        }
        // Appended source copies must not make an originally missing numeric target valid.
        if let Some(i) = params.get("action").and_then(num).and_then(|i| usize::try_from(i).ok())
            && i >= list.len()
            && i < final_len
        {
            params["action"] = json!(final_len);
        }
    };
    visit_calls(pair_calls(steps), &mut remap);
    for action in &mut found {
        visit_calls(step_calls(&mut action.steps), &mut remap);
    }
    found.extend(originals);
    found
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

fn num(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f.round() as i64))
}

pub(crate) fn resolve(list: &[Action], p: &Value, cmd: &str) -> Result<usize> {
    let Some(v) = p.get("action") else {
        return Err(bad(cmd, "needs \"action\" (a name or an index)"));
    };
    if let Some(i) = num(v) {
        if i < 0 {
            return Err(bad(cmd, "action index is negative"));
        }
        let Some(i) = usize::try_from(i).ok() else {
            return Err(bad(cmd, "action index is out of range"));
        };
        if i >= list.len() {
            return Err(bad(cmd, format!("no action at index {i}")));
        }
        return Ok(i);
    }
    match v.as_str() {
        Some(name) if !name.is_empty() => list.iter().position(|a| a.name == name).ok_or_else(|| bad(cmd, format!("no action named `{name}`"))),
        Some(_) => Err(bad(cmd, "action name is empty")),
        None => Err(bad(cmd, "\"action\" must be a name or an index")),
    }
}

fn from_step(p: &Value, len: usize) -> Result<usize> {
    let Some(v) = p.get("from") else { return Ok(0) };
    if v.is_null() {
        return Ok(0);
    }
    let Some(i) = num(v) else {
        return Err(bad("actions.play", "\"from\" must be a step index"));
    };
    if i < 0 {
        return Err(bad("actions.play", "\"from\" is negative"));
    }
    let Some(i) = usize::try_from(i).ok() else {
        return Err(bad("actions.play", "\"from\" is out of range"));
    };
    if i > len {
        return Err(bad("actions.play", format!("\"from\" {i} is past the last step ({len})")));
    }
    Ok(i)
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let actions: Vec<Value> = s.actions.list.actions().iter().map(|a| json!({"name": a.name, "steps": a.steps.len(), "set": a.set})).collect();
    let recording = s.actions.recording.and_then(|(_, i)| s.actions.list.action(i).map(|a| a.name.clone()));
    Ok(json!({"actions": actions, "sets": crate::actions_edit_cmds::set_names(s), "recording": recording, "toolRecording": s.actions.record_tools}))
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let idx = resolve(s.actions.list.actions(), p, "actions.get")?;
    let action = &s.actions.list.actions()[idx];
    let steps: Vec<Value> = action.steps.iter().map(|step| json!([step.command, step.params])).collect();
    Ok(json!({"name": action.name, "steps": steps, "disabled": action.disabled, "dialogs": action.dialogs, "set": action.set, "color": action.color.name()}))
}

/// `actions.record`: start recording into an existing `action` or a new one named `name`, and mark
/// it as automation's when an automation gate is installed.
fn record(s: &mut Session, p: &Value) -> Result<Value> {
    not_playing(s, "actions.record")?;
    if s.actions.recording.is_some() {
        return Err(bad("actions.record", "already recording; stop first"));
    }
    let idx = if p.get("action").is_some() {
        resolve(s.actions.list.actions(), p, "actions.record")?
    } else {
        let name = match p.get("name") {
            None | Some(Value::Null) => None,
            Some(Value::String(n)) if !n.is_empty() => Some(n.clone()),
            Some(Value::String(_)) => return Err(bad("actions.record", "\"name\" is empty")),
            Some(_) => return Err(bad("actions.record", "\"name\" must be a string")),
        };
        let set = match p.get("set") {
            None | Some(Value::Null) => None,
            // An existing set, empty ones included, keeps its exact name (an older actions.json or
            // a `.atn` may pad it); a new one is trimmed.
            Some(Value::String(n)) if crate::actions_edit_cmds::set_names(s).contains(n) => Some(n.clone()),
            Some(Value::String(n)) if !n.trim().is_empty() => Some(n.trim().to_string()),
            Some(_) => return Err(bad("actions.record", "\"set\" must be a set name or null")),
        };
        let name = name.unwrap_or_else(|| s.actions.list.unique_name(""));
        // `push`, not `new_action`: an existing set keeps its exact name, which `new_action` would trim.
        s.actions.list.push(Action { set, ..Action::new(name, Vec::new()) }).map_err(|e| list_error("actions.record", e))?
    };
    s.actions.recording = Some((s.journal.len(), idx));
    note_automation_edit(s, idx);
    s.actions.take = s.actions.take.wrapping_add(1);
    let name = s.actions.list.action(idx).map(|a| a.name.clone()).unwrap_or_default();
    Ok(json!({"action": name, "index": idx}))
}

/// Whether a journal entry is recorded as an action step: a replayable command, except Check
/// Spelling's corrections and lookups.
fn recorded(id: &str, p: &Value) -> bool {
    replayable(id) && (id != "edit.checkSpelling" || crate::type_spell_cmds::records(p))
}

/// Commands recorded with their step's dialog on: Check Spelling.
const RECORDED_WITH_DIALOG: &[&str] = &["edit.checkSpelling"];

/// Replayable journal entries not yet committed to this action while recording.
pub fn pending_steps(s: &Session, action: usize) -> impl Iterator<Item = &(String, Value)> {
    let from = s.actions.recording.filter(|(_, i)| *i == action).map_or(s.journal.len(), |(from, _)| from);
    s.journal.iter().skip(from).filter(|(id, p)| recorded(id, p) && (s.actions.record_tools || !tool_stroke(id)))
}

/// The point-based painting tools committed by the canvas; fills and gradients remain commands.
pub(crate) fn tool_stroke(id: &str) -> bool {
    matches!(
        id,
        "paint.stroke"
            | "paint.pencil"
            | "paint.mixerBrush"
            | "paint.colorReplacement"
            | "paint.blur"
            | "paint.sharpen"
            | "paint.smudge"
            | "paint.dodge"
            | "paint.burn"
            | "paint.sponge"
            | "paint.cloneStamp"
            | "paint.healingBrush"
            | "paint.historyBrush"
            | "paint.artHistoryBrush"
            | "paint.patternStamp"
            | "paint.backgroundEraser"
            | "paint.spotHealing"
            | "paint.remove"
    )
}

pub(crate) fn flush_recording(s: &mut Session) -> Result<()> {
    let Some((_, idx)) = s.actions.recording else { return Ok(()) };
    if s.actions.list.action(idx).is_none() {
        return Err(bad("actions.stop", "the action being recorded no longer exists"));
    }
    let steps = fold_recorded_nudges(pending_steps(s, idx).cloned());
    s.actions.recording = Some((s.journal.len(), idx));
    if !steps.is_empty() {
        let first = s.actions.list.step_count(idx);
        let with_dialog: Vec<usize> =
            steps.iter().enumerate().filter(|(_, (id, _))| RECORDED_WITH_DIALOG.contains(&id.as_str())).map(|(i, _)| first + i).collect();
        let stop = |e: ActionListError| list_error("actions.stop", e);
        s.actions.list.append_recorded(idx, steps.into_iter().map(ActionStep::from)).map_err(stop)?;
        for step in with_dialog {
            s.actions.list.set_dialog(idx, step, true).map_err(stop)?;
        }
    }
    Ok(())
}

/// Consecutive Move-tool keystrokes in one UI nudge run are one relative action step.
/// The UI's `coalesce` run id distinguishes a held key from a later, separate gesture.
fn fold_recorded_nudges(steps: impl IntoIterator<Item = (String, Value)>) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::new();
    let mut last_run: Option<String> = None;
    for (id, mut params) in steps {
        let wrap_of = |p: &Value| p.get("wrap").and_then(Value::as_bool).unwrap_or(false);
        let run = (id == "layer.translate")
            .then(|| params.as_object())
            .flatten()
            .filter(|p| p.keys().all(|key| matches!(key.as_str(), "dx" | "dy" | "coalesce" | "wrap")) && p.get("wrap").is_none_or(|v| v.is_boolean()))
            .and_then(|p| p.get("coalesce").and_then(Value::as_str).filter(|run| run.starts_with("nudge:")))
            .map(str::to_string);
        if let Some(run) = &run {
            // `layer.translate` moves each step by whole pixels, so the fold adds rounded steps.
            let dx = params.get("dx").and_then(Value::as_f64).map(f64::round);
            let dy = params.get("dy").and_then(Value::as_f64).map(f64::round);
            if let (Some(dx), Some(dy)) = (dx, dy)
                && dx.is_finite()
                && dy.is_finite()
            {
                if last_run.as_ref() == Some(run)
                    && let Some((last_id, previous)) = out.last_mut()
                    && last_id == &id
                    && wrap_of(previous) == wrap_of(&params)
                {
                    let sum_x = previous.get("dx").and_then(Value::as_f64).unwrap_or(0.0) + dx;
                    let sum_y = previous.get("dy").and_then(Value::as_f64).unwrap_or(0.0) + dy;
                    if sum_x.is_finite() && sum_y.is_finite() {
                        previous["dx"] = json!(sum_x);
                        previous["dy"] = json!(sum_y);
                        continue;
                    }
                }
                if let Some(object) = params.as_object_mut() {
                    object.remove("coalesce");
                    object.insert("dx".into(), json!(dx));
                    object.insert("dy".into(), json!(dy));
                }
                last_run = Some(run.clone());
                out.push((id, params));
                continue;
            }
        }
        last_run = None;
        out.push((id, params));
    }
    out
}

fn stop(s: &mut Session, _p: &Value) -> Result<Value> {
    not_playing(s, "actions.stop")?;
    let Some((_, idx)) = s.actions.recording else {
        return Err(bad("actions.stop", "not recording"));
    };
    flush_recording(s)?;
    s.actions.recording = None;
    let action = &s.actions.list.actions()[idx];
    Ok(json!({"action": action.name, "steps": action.steps.len()}))
}

/// A step Photoshop plays as a silent no-op although its menu item is greyed: Deselect with an
/// open document and nothing selected (actions often start with it to clear any selection, #6744),
/// and an Arrange that can't move the layer, such as Bring to Front on a new adjustment layer that
/// is already on top (#8453). The CLI's `run` and `batch` scripts skip these too (#8802).
pub fn noop_step(s: &Session, id: &str, params: &Value) -> bool {
    let Some(d) = s.active() else { return false };
    let dir = match id {
        "select.deselect" => return d.doc.selection.is_none(),
        "layer.arrange.bringForward" | "layer.arrange.bringToFront" => 1,
        "layer.arrange.sendBackward" | "layer.arrange.sendToBack" => -1,
        _ => return false,
    };
    crate::commands::layer_param(s, params).is_ok_and(|layer| super::ops::arrange_at_end(&d.doc, layer, dir))
}

pub const SAVE_CONTENTS: &str = "layer.smartObjects.saveContents";
pub fn saves_smart_contents(s: &Session, id: &str) -> bool {
    id == "file.save" && s.is_enabled(SAVE_CONTENTS)
}

/// Run enabled steps from `from`, retaining child results when nested work fails.
fn run_recorded(s: &mut Session, action: &Action, from: usize) -> (u64, Option<Value>) {
    let mut ran = 0u64;
    let mut failed = None;
    for (i, ActionStep { command: id, params }) in action.enabled_steps().skip_while(|(i, _)| *i < from) {
        if !consume_playback_step(s) {
            failed = Some(json!({"step": i, "id": id, "error": format!("action playback exceeded the {}-step limit", MAX_PLAYBACK_STEPS), "limit": true}));
            break;
        }
        let effective = crate::file_cmds::batch_open_params(s, id, params);
        let params = effective.as_ref().unwrap_or(params);
        if let Some(auth) = s.authorize
            && let Err(e) = auth(id, params)
        {
            failed = Some(json!({"step": i, "id": id, "error": e.to_string()}));
            break;
        }
        if noop_step(s, id, params) {
            ran += 1;
            continue;
        }
        match playback_params(s, id, params).and_then(|p| if saves_smart_contents(s, id) { s.execute(SAVE_CONTENTS, json!({})) } else { s.execute(id, p) }) {
            Ok(value) => {
                // A nested batch or action that stopped at a limit stops this action too.
                if let Some(error) = limit_stop(&value) {
                    // The child result stays, so the files a nested batch wrote before the stop are visible.
                    let mut failure = json!({"step": i, "id": id, "error": error, "limit": true});
                    failure["result"] = value;
                    failed = Some(failure);
                    break;
                }
                if let Some(error) = nested_failure(id, &value) {
                    // Like Scripts, retain the child result so partial Batch outputs remain visible.
                    let mut failure = json!({"step": i, "id": id, "error": error});
                    failure["result"] = value;
                    failed = Some(failure);
                    break;
                }
                ran += 1;
            }
            Err(e) => {
                failed = Some(json!({"step": i, "id": id, "error": e.to_string()}));
                if matches!(e, EngineError::BatchLimit(_))
                    && let Some(f) = failed.as_mut()
                {
                    f["limit"] = json!(true);
                }
                break;
            }
        }
    }
    (ran, failed)
}

/// Suspend recording while a called action executes. Keep its commands in the journal
/// for diagnostics, but record just the named call in the parent action.
pub fn begin_playback(s: &mut Session, action: &Action) -> Result<Option<(usize, usize)>> {
    begin_playback_from(s, action, 0)
}

/// [`begin_playback`] for a call that starts at step `from`: the steps before it are charged to the
/// playback budget too.
pub fn begin_playback_from(s: &mut Session, action: &Action, from: usize) -> Result<Option<(usize, usize)>> {
    flush_recording(s)?;
    // The steps that run are charged one by one as they run; the ones walked past are charged
    // here ([`playback_plan`] checked that the whole action fits). Refused before the history
    // scope opens, so a rejected call leaves none behind.
    let skipped = skipped_steps(action, from);
    let max = crate::automate_cmds::MAX_SCRIPT_STEPS;
    if s.actions.script_depth > 0 && s.actions.script_steps.saturating_add(skipped) > max {
        return Err(bad("actions.play", format!("the script ran more than {max} steps in all")));
    }
    if s.actions.playing == 0 {
        history::begin(s, &action.name);
        // A batch nested in a played action or another batch carries the shared count on.
        if !s.actions.budget_shared {
            s.actions.playback_steps = 0;
        }
    }
    s.actions.playback_steps = s.actions.playback_steps.saturating_add(skipped);
    if s.actions.script_depth > 0 {
        s.actions.script_steps = s.actions.script_steps.saturating_add(skipped);
    }
    let recording = s.actions.recording.take();
    s.actions.playing = s.actions.playing.saturating_add(1);
    s.actions.playback_stack.push(action.name.clone());
    Ok(recording)
}

/// Unwind the playback stack and finish grouping only at the outer boundary, then restore suspended recording.
pub fn finish_playback(s: &mut Session, recording: Option<(usize, usize)>, action: &Action, from: usize) {
    s.actions.playback_stack.pop();
    s.actions.playing = s.actions.playing.saturating_sub(1);
    if s.actions.playing == 0 {
        history::finish_playback(s);
    }
    if let Some((_, idx)) = recording {
        restore_recording(s, idx, &action.name, from);
    }
}

/// Resume recording into action `idx` and journal the named call that suspended it.
pub fn restore_recording(s: &mut Session, idx: usize, action: &str, from: usize) {
    s.actions.recording = Some((s.journal.len(), idx));
    let mut params = json!({"action": action});
    if from > 0 {
        params["from"] = json!(from);
    }
    s.journal.push(("actions.play".into(), params));
}

/// The message of a nesting or playback-limit stop reported by `value`, the result of a step: an
/// input of a nested batch or droplet marked `"limit": true` in `errors`, or a played action whose
/// failed step carries it. Such a stop fails the enclosing input rather than being swallowed.
pub fn limit_stop(value: &Value) -> Option<String> {
    let flagged = |v: &Value| v.get("limit") == Some(&json!(true));
    let found =
        value.get("errors").and_then(Value::as_array).and_then(|a| a.iter().find(|e| flagged(e))).or_else(|| value.get("failed").filter(|f| flagged(f)))?;
    Some(found.get("error").and_then(Value::as_str).unwrap_or("batch limit reached").to_string())
}

/// What stops the caller of a Batch even where per-file errors don't (a direct Batch in a script
/// or a CLI command list): the script ran out of steps inside it, it was cancelled, or its
/// requested error log could not be written.
pub fn batch_stop(value: &Value) -> Option<String> {
    if let Some(error) = value.get("scriptLimitError").and_then(Value::as_str) {
        return Some(error.to_string());
    }
    if value.get("cancelled") == Some(&Value::Bool(true)) {
        return Some("Batch was cancelled".into());
    }
    value.get("errorLogError").and_then(Value::as_str).map(|error| format!("Batch error log failed: {error}"))
}

/// Nested playback and Batch return structured failures even though the command succeeded.
/// Promote it to the caller's failed step so no later parent steps run after a child fails.
/// Batch text summarizes counts and the first failure; its full result retains every file outcome.
pub fn nested_failure(id: &str, value: &Value) -> Option<String> {
    if id == "file.scripts.browse" && value.get("ok") == Some(&Value::Bool(false)) {
        let error = value
            .get("results")
            .and_then(Value::as_array)
            .and_then(|results| results.iter().find_map(|result| result.get("error").and_then(Value::as_str)))
            .unwrap_or("unknown error");
        return Some(format!("script failed: {error}"));
    }
    if id == "file.automate.batch" {
        let errors = value.get("errors").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        let Some(first) = errors.first() else {
            // Every input succeeded, yet the run was stopped or its requested log is missing:
            // later parent steps must not run as if the Batch had finished.
            return batch_stop(value);
        };
        let error = first.get("error")?.as_str()?;
        let file = first.get("file").or_else(|| first.get("document")).and_then(Value::as_str).unwrap_or("unknown input");
        let saved = value.get("files").and_then(Value::as_array).map_or(0, Vec::len);
        return Some(format!("Batch: {} file(s) failed, {saved} saved; first failure `{file}`: {error}", errors.len()));
    }
    (id == "actions.play" || id == "actions.conditional").then(|| value.get("failed")).flatten().map(|failure| {
        format!(
            "action `{}` step {} failed: {}",
            value.get("action").and_then(Value::as_str).unwrap_or(""),
            failure.get("step").and_then(Value::as_u64).unwrap_or(0).saturating_add(1),
            failure.get("error").and_then(Value::as_str).unwrap_or("unknown error")
        )
    })
}

/// `actions.play`: run an action's enabled steps from `from`, under automation's kept gate
/// ([`kept_gate`]) when automation wrote it and no gate is installed. A call journaled into a
/// recording while a gate is installed marks the recorded action ([`note_automation_step`]).
fn play(s: &mut Session, p: &Value) -> Result<Value> {
    let (_, action, from) = playback_plan(s, p)?;
    let recording = begin_playback_from(s, &action, from)?;
    // Nested calls and commands the steps run on their own behalf see the kept gate too.
    let kept = kept_gate(s, &action);
    if kept.is_some() {
        s.authorize = kept;
    }
    let (ran, failed) = run_recorded(s, &action, from);
    if kept.is_some() {
        s.authorize = None;
    }
    finish_playback(s, recording, &action, from);
    // The call journaled into a recording is automation's step when its caller runs under a gate
    // (the kept gate is already removed, so the user's own play of a marked action is not).
    note_automation_step(s, "actions.play", p);
    Ok(match failed {
        Some(f) => json!({"action": action.name, "ran": ran, "failed": f}),
        None => json!({"action": action.name, "ran": ran}),
    })
}

pub(crate) fn not_playing(s: &Session, cmd: &str) -> Result<()> {
    if s.actions.playing > 0 { Err(bad(cmd, "cannot edit actions during playback")) } else { Ok(()) }
}

/// Move to a final zero-based index, either in an action's steps or in the action list.
fn move_item(s: &mut Session, p: &Value) -> Result<Value> {
    not_playing(s, "actions.move")?;
    let idx = resolve(s.actions.list.actions(), p, "actions.move")?;
    let index = |key: &str| -> Result<usize> {
        p.get(key)
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| bad("actions.move", format!("{key} must be a non-negative integer")))
    };
    let to = index("to")?;
    if p.get("step").is_some() {
        let step = index("step")?;
        let len = s.actions.list.step_count(idx).saturating_add(pending_steps(s, idx).count());
        if step >= len || to >= len {
            return Err(bad("actions.move", "step or destination is out of range"));
        }
        if s.actions.recording.is_some_and(|(_, i)| i == idx) {
            flush_recording(s)?;
        }
        s.actions.list.move_step(idx, step, idx, to).map_err(|e| list_error("actions.move", e))?;
        let result = json!({"action": s.actions.list.actions()[idx].name, "step": to});
        note_automation_edit(s, idx);
        Ok(result)
    } else {
        let name = s.actions.list.actions()[idx].name.clone();
        s.actions.list.move_action(idx, to).map_err(|e| list_error("actions.move", e))?;
        if let Some((_, recording)) = s.actions.recording.as_mut() {
            *recording = moved_index(*recording, idx, to);
        }
        Ok(json!({"action": name, "index": to}))
    }
}

/// `actions.delete`: delete an action, or with `step` one of its steps (which marks the action
/// as automation's when an automation gate is installed).
fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    not_playing(s, "actions.delete")?;
    let idx = resolve(s.actions.list.actions(), p, "actions.delete")?;
    if let Some(step) = p.get("step") {
        let step = step.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad("actions.delete", "step must be a non-negative integer"))?;
        let len = s.actions.list.step_count(idx).saturating_add(pending_steps(s, idx).count());
        if step >= len {
            return Err(bad("actions.delete", "step is out of range"));
        }
        // Commit pending entries before editing the action; never erase the command
        // journal or undo the edit that produced the deleted recording step.
        if s.actions.recording.is_some_and(|(_, i)| i == idx) {
            flush_recording(s)?;
        }
        s.actions.list.delete_step(idx, step).map_err(|e| list_error("actions.delete", e))?;
        let action = &s.actions.list.actions()[idx];
        let result = json!({"action": action.name, "deletedStep": step, "steps": action.steps.len()});
        note_automation_edit(s, idx);
        return Ok(result);
    }
    if s.actions.recording.is_some() {
        return Err(bad("actions.delete", "stop recording before deleting an action"));
    }
    let name = s.actions.list.delete_action(idx).map_err(|e| list_error("actions.delete", e))?.name;
    // Its function keys go with it, or the next action given the same name would inherit them.
    remove_shortcuts(s, std::iter::once(name.as_str()));
    Ok(json!({"deleted": name}))
}

/// The Actions panel check marks. With `step`, switch one step on or off; without it, switch
/// every step (all on unless all already are, like clicking the action's check mark).
fn toggle(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.toggle";
    not_playing(s, cmd)?;
    let idx = resolve(s.actions.list.actions(), p, cmd)?;
    let enabled = match p.get("enabled") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err(bad(cmd, "\"enabled\" must be true or false")),
    };
    let step = match p.get("step") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(cmd, "step must be a non-negative integer"))?),
    };
    let len = s.actions.list.step_count(idx).saturating_add(pending_steps(s, idx).count());
    if step.is_some_and(|step| step >= len) {
        return Err(bad(cmd, "step is out of range"));
    }
    if s.actions.recording.is_some_and(|(_, i)| i == idx) {
        flush_recording(s)?;
    }
    let action = &s.actions.list.actions()[idx];
    let edit = match step {
        Some(step) => {
            let on = enabled.unwrap_or(!action.step_enabled(step));
            s.actions.list.set_step(idx, step, on)
        }
        None => {
            let on = enabled.unwrap_or(!action.disabled.is_empty());
            s.actions.list.set_all(idx, on)
        }
    };
    edit.map_err(|e| list_error(cmd, e))?;
    let action = &s.actions.list.actions()[idx];
    let result = json!({"action": action.name, "disabled": action.disabled});
    note_automation_edit(s, idx);
    Ok(result)
}

/// Prefix of the persisted shortcut override that plays a named action (`actions.play:<name>`).
pub const PLAY_SHORTCUT_PREFIX: &str = "actions.play:";

/// Prefix of the shortcut keys PhotoCraft files of the Actions library commands era stored per
/// action id (`actions.playById:<id>`); [`name_id_shortcuts`] moves them to names on load.
const PLAY_BY_ID_PREFIX: &str = "actions.playById:";

/// Whether a shortcut key plays an action: it belongs to the Actions library, not to a set.
pub fn is_action_key(key: &str) -> bool {
    key.starts_with(PLAY_SHORTCUT_PREFIX)
}

/// Remove the function keys (primary and alternate) of the actions `names`.
pub(crate) fn remove_shortcuts<'a>(s: &mut Session, names: impl IntoIterator<Item = &'a str>) {
    let prefs = s.prefs();
    let keys: Vec<String> = names
        .into_iter()
        .map(|name| format!("{PLAY_SHORTCUT_PREFIX}{name}"))
        .filter(|key| prefs.shortcuts.contains_key(key) || prefs.alternate_shortcuts.contains_key(key))
        .collect();
    if keys.is_empty() {
        return;
    }
    s.edit_prefs(|prefs| {
        for key in &keys {
            prefs.shortcuts.remove(key);
            prefs.alternate_shortcuts.remove(key);
        }
    });
}

/// The stored id → name map of [`MergedFile::ids`](craft::automation::actions::MergedFile::ids).
fn id_names(ids: &[(String, String)]) -> std::collections::HashMap<&str, &str> {
    ids.iter().map(|(id, name)| (id.as_str(), name.as_str())).collect()
}

/// Names are the list's only identity: the calls a loaded PhotoCraft file made by stored action id
/// (`actionId`, a Conditional's `thenActionId` / `elseActionId`) name the action
/// [`ActionList::load`] gave that id (`ids`: stored id, name) instead. A call whose id no action
/// has keeps the name it carried.
pub(crate) fn name_id_calls(list: &mut ActionList, ids: &[(String, String)]) {
    let names = id_names(ids);
    for index in 0..list.actions().len() {
        for step in 0..list.step_count(index) {
            let mut edited = list.actions()[index].steps[step].clone();
            let mut changed = false;
            visit_calls(step_calls(std::slice::from_mut(&mut edited)), &mut |id, params, scoped| {
                let fields: &[(&str, &str, &str)] = match id {
                    "actions.play" | "file.automate.batch" | "file.automate.createDroplet" => &[("actionId", "action", "set")],
                    "actions.conditional" => &[("thenActionId", "then", "thenSet"), ("elseActionId", "else", "elseSet")],
                    _ => &[],
                };
                let Some(object) = params.as_object_mut() else { return };
                for (field, key, scope) in fields {
                    let Some(stored) = object.remove(*field) else { continue };
                    changed = true;
                    // A Batch with its own action list selects in that list, which the ids don't name.
                    if let Some(name) = stored.as_str().and_then(|stored| names.get(stored)).filter(|_| !scoped) {
                        object.insert((*key).into(), json!(name));
                        object.remove(*scope);
                    }
                }
            });
            if changed {
                let _ = list.replace_step(index, step, edited);
            }
        }
    }
}

/// The `actions.playById:<id>` shortcut keys (primary and alternate) of a loaded PhotoCraft file's
/// actions become `actions.play:<name>`, with the names [`ActionList::load`] gave the ids; a key
/// whose id no action has goes.
pub(crate) fn name_id_shortcuts(s: &mut Session, ids: &[(String, String)]) {
    let names = id_names(ids);
    let prefs = s.prefs();
    if !prefs.shortcuts.keys().chain(prefs.alternate_shortcuts.keys()).any(|key| key.starts_with(PLAY_BY_ID_PREFIX)) {
        return;
    }
    let named = |key: &str| key.strip_prefix(PLAY_BY_ID_PREFIX).map(|id| names.get(id).map(|name| format!("{PLAY_SHORTCUT_PREFIX}{name}")));
    s.edit_prefs(|prefs| {
        for (key, value) in std::mem::take(&mut prefs.shortcuts) {
            match named(&key) {
                None => {
                    prefs.shortcuts.entry(key).or_insert(value);
                }
                Some(Some(name)) => {
                    prefs.shortcuts.insert(name, value);
                }
                Some(None) => {}
            }
        }
        for (key, value) in std::mem::take(&mut prefs.alternate_shortcuts) {
            match named(&key) {
                None => {
                    prefs.alternate_shortcuts.entry(key).or_insert(value);
                }
                Some(Some(name)) => {
                    prefs.alternate_shortcuts.insert(name, value);
                }
                Some(None) => {}
            }
        }
    });
}

/// Rename an action in place. Nested `actions.play` steps, named Batch steps and `actions.conditional` branches
/// that call it by name, and its function-key binding, follow the new name so nothing silently
/// stops working.
fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    not_playing(s, "actions.rename")?;
    let idx = resolve(s.actions.list.actions(), p, "actions.rename")?;
    let name = match p.get("name") {
        Some(Value::String(n)) if !n.trim().is_empty() => n.clone(),
        Some(Value::String(_)) => return Err(bad("actions.rename", "\"name\" is empty")),
        Some(_) => return Err(bad("actions.rename", "\"name\" must be a string")),
        None => return Err(bad("actions.rename", "needs \"name\"")),
    };
    let old = s.actions.list.action(idx).map(|a| a.name.clone()).ok_or_else(|| bad("actions.rename", "no such action"))?;
    let pending = s.actions.recording.map_or(s.journal.len(), |(start, _)| start);
    // Names are list-wide, so a call naming `old` reaches this action; a named Batch is checked
    // against the list before the rename because its set can still refuse it.
    let before = s.actions.list.actions().to_vec();
    let follow = |id: &str, params: &mut Value, old: &str, new: &str| rename_call(&before, idx, id, params, old, new);
    s.actions.list.rename(idx, &name, |step, old, new| follow(&step.command, &mut step.params, old, new)).map_err(|e| list_error("actions.rename", e))?;
    // The list renames its own steps; the journal entries not yet recorded are the engine's.
    let name = s.actions.list.actions()[idx].name.clone();
    if old == name {
        return Ok(json!({"action": name, "index": idx}));
    }
    for (id, params) in s.journal.iter_mut().skip(pending) {
        follow(id, params, &old, &name);
    }
    let (from, to) = (format!("{PLAY_SHORTCUT_PREFIX}{old}"), format!("{PLAY_SHORTCUT_PREFIX}{name}"));
    if s.prefs().shortcuts.contains_key(&from) || s.prefs().alternate_shortcuts.contains_key(&from) {
        s.edit_prefs(|prefs| {
            if let Some(key) = prefs.shortcuts.remove(&from) {
                prefs.shortcuts.insert(to.clone(), key);
            }
            if let Some(keys) = prefs.alternate_shortcuts.remove(&from) {
                prefs.alternate_shortcuts.insert(to, keys);
            }
        });
    }
    Ok(json!({"action": name, "index": idx}))
}

/// Rewrite one call's action name `old` to `new` in `params`, for the command `id` that made it.
/// `before` is the list before the rename and `idx` the renamed action.
fn rename_call(before: &[Action], idx: usize, id: &str, params: &mut Value, old: &str, new: &str) {
    let keys: &[&str] = match id {
        "actions.play" => &["action"],
        "actions.conditional" => &["then", "else"],
        // A named Batch selects from the caller's list, by name alone or within its set;
        // explicit `steps` run instead of any named action, so those are left alone.
        "file.automate.batch" if params.get("steps").is_none() && crate::file_cmds::batch_action(before, params).ok() == Some(idx) => &["action"],
        _ => &[],
    };
    for key in keys {
        if params.get(*key).and_then(Value::as_str) == Some(old) {
            params[*key] = json!(new);
        }
    }
}

fn set_color(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "actions.setColor";
    not_playing(s, cmd)?;
    let idx = resolve(s.actions.list.actions(), p, cmd)?;
    let Some(color) = p.get("color").and_then(Value::as_str) else {
        return Err(bad(cmd, "needs \"color\""));
    };
    // `ActionColor::from_name` reads an unknown name as no colour, which a command must refuse.
    let Some(chosen) = ActionColor::ALL.into_iter().find(|each| each.name() == color) else {
        return Err(bad(cmd, "color must be empty, red, orange, yellow, green, blue, violet, or gray"));
    };
    s.actions.list.set_color(idx, chosen).map_err(|e| list_error(cmd, e))?;
    let name = s.actions.list.actions()[idx].name.clone();
    Ok(json!({"action": name, "color": color}))
}

/// Active playback and retained dialog continuations keep their chosen history policy.
pub(crate) fn history_policy_in_use(s: &Session) -> bool {
    s.actions.playing != 0 || s.action_history.is_some() || s.paused_action_history.iter().any(|lease| lease.strong_count() != 0)
}

/// Create a single history state alone; `actions.setPlayback` sets it with the performance mode.
fn set_history(s: &mut Session, p: &Value) -> Result<Value> {
    not_playing(s, "actions.setHistory")?;
    if history_policy_in_use(s) {
        return Err(bad("actions.setHistory", "cannot change action history settings during playback"));
    }
    let enabled = p.get("singleState").and_then(Value::as_bool).ok_or_else(|| bad("actions.setHistory", "singleState must be a boolean"))?;
    s.edit_prefs(|prefs| prefs.actions.create_single_history_state = enabled);
    Ok(json!({"singleState":enabled}))
}

/// Playback Options: `performance`, Pause For's whole `seconds` (1 to 60) and `singleState`, each
/// optional, all validated before any is stored, and saved in preferences. `{}` reads them.
fn set_playback(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "actions.setPlayback";
    not_playing(s, CMD)?;
    if !p.is_object() {
        return Err(bad(CMD, "params must be an object"));
    }
    let performance = match p.get("performance") {
        None => None,
        Some(v) => {
            Some(v.as_str().and_then(crate::prefs::PlaybackPerformance::parse).ok_or_else(|| bad(CMD, "performance must be accelerated, step or pause"))?)
        }
    };
    let range = crate::prefs::Actions::PAUSE_SECONDS;
    let seconds = match p.get("seconds") {
        None => None,
        Some(v) => Some(
            v.as_u64()
                .and_then(|n| u8::try_from(n).ok())
                .filter(|n| range.contains(n))
                .ok_or_else(|| bad(CMD, format!("seconds must be a whole number from {} to {}", range.start(), range.end())))?,
        ),
    };
    let single = match p.get("singleState") {
        None => None,
        Some(v) => Some(v.as_bool().ok_or_else(|| bad(CMD, "singleState must be a boolean"))?),
    };
    if single.is_some_and(|single| single != s.prefs().actions.create_single_history_state) && history_policy_in_use(s) {
        return Err(bad(CMD, "cannot change action history settings during playback"));
    }
    if performance.is_some() || seconds.is_some() || single.is_some() {
        s.edit_prefs(|prefs| {
            let a = &mut prefs.actions;
            a.performance = performance.unwrap_or(a.performance);
            a.pause_seconds = seconds.unwrap_or(a.pause_seconds);
            a.create_single_history_state = single.unwrap_or(a.create_single_history_state);
        });
    }
    let a = &s.prefs().actions;
    Ok(json!({"performance": a.performance.name(), "seconds": a.pause_seconds(), "singleState": a.create_single_history_state}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "actions.setHistory",
            label: "Set Actions History",
            menu: &[],
            shortcut: None,
            params: r##"{"singleState":bool} → {singleState}. Persist Create a single history state (default true). Playback Options can use this setting without changing its performance mode."##,
            enabled: always,
            run: set_history,
            journal: false,
        },
        CommandSpec {
            id: "actions.setPlayback",
            label: "Playback Options",
            menu: &[],
            shortcut: None,
            params: r##"{"performance":"accelerated"|"step"|"pause"?, "seconds":1..60?, "singleState":bool?} → {performance, seconds, singleState}. The Actions panel menu's Playback Options, saved in preferences (`actions.performance`, `actions.pauseSeconds`, `actions.createSingleHistoryState`). Step by Step and Pause For pace a person's Play in the app, about one second between steps plus Pause For's whole seconds; actions.play from agents, the CLI and Batch always runs at once. Omitted keys keep their value; {} reads them. Every value is checked before any is stored."##,
            enabled: always,
            run: set_playback,
            journal: false,
        },
        CommandSpec {
            id: "actions.move",
            label: "Move Action or Step",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index, "step":index?, "to":index}. Move a step within its action, or the action itself. to is the final zero-based index. Pending recorded steps can be moved."##,
            enabled: always,
            run: move_item,
            journal: false,
        },
        CommandSpec {
            id: "actions.list",
            label: "List Actions",
            menu: &[],
            shortcut: None,
            params: "{} → {actions:[{name, steps:count, set:name|null}], sets:[name…] (every set in panel order, empty ones too), recording:name|null, toolRecording:bool}",
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "actions.get",
            label: "Get Action",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index} → {name, steps:[[id, params]…], disabled:[step…], dialogs:[step…], set:name|null, color:str} (steps is the shape file.automate.batch and droplets take; disabled lists the 0-based steps playback skips; dialogs the steps whose dialog Photoshop would show; set the action set from Load Actions; color is the Button Mode swatch, or empty)"##,
            enabled: always,
            run: get,
            journal: false,
        },
        CommandSpec {
            id: "actions.record",
            label: "Record Action",
            menu: &[],
            shortcut: None,
            params: r##"{"name":str? (new action, default "Action N"),"set":name? (the set it goes in, as New Action records into the selected set)} or {"action":name|index} (append) → {action, index}"##,
            enabled: always,
            run: record,
            journal: false,
        },
        CommandSpec {
            id: "actions.stop",
            label: "Stop Recording",
            menu: &[],
            shortcut: None,
            params: "{} → {action, steps:count} (steps recorded since actions.record, queries and action-editing commands omitted)",
            enabled: always,
            run: stop,
            journal: false,
        },
        CommandSpec {
            id: "actions.play",
            label: "Play Action",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index, "from":step?} → {action, ran, failed?:{step, id, error}}. step and from are 0-based; steps switched off with actions.toggle are skipped. Stops at the first error (the command still returns ok, with failed set) and groups document edits in one Action: <name> history entry by default; actions.setHistory can retain per-step history. Nested calls are supported up to 16 levels; cycles are rejected. Recording stores a named call rather than its expanded commands. Each step is checked with Session::authorize when one is installed."##,
            enabled: always,
            run: play,
            journal: false,
        },
        CommandSpec {
            id: "actions.delete",
            label: "Delete Action",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index, "step":index?} → {deleted:name} or {action, deletedStep, steps:count}. step is 0-based, including pending recorded steps. Whole-action deletion is refused while recording."##,
            enabled: always,
            run: delete,
            journal: false,
        },
        CommandSpec {
            id: "actions.toggle",
            label: "Toggle Action Steps",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index, "step":index?, "enabled":bool?} → {action, disabled:[step…]}. Switch one 0-based step (including pending recorded steps) on or off for playback, Batch and droplets; omitted enabled flips it. Without step, switch every step: on unless all already are, or as enabled says."##,
            enabled: always,
            run: toggle,
            journal: false,
        },
        CommandSpec {
            id: "actions.rename",
            label: "Rename Action",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index, "name":str} → {action, index}. The name must not be empty or whitespace-only and must be unique. Nested Play Action steps and the action's function key follow the rename."##,
            enabled: always,
            run: rename,
            journal: false,
        },
        CommandSpec {
            id: "actions.setColor",
            label: "Set Action Color",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index, "color":""|"red"|"orange"|"yellow"|"green"|"blue"|"violet"|"gray"} → {action, color}. The Button Mode colour. Empty clears it."##,
            enabled: always,
            run: set_color,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod playback_tests;

#[cfg(test)]
mod security_tests {
    use super::*;

    #[test]
    fn playback_budget_is_shared_by_nested_calls() {
        let mut session = Session::new();
        session.actions.playback_steps = MAX_PLAYBACK_STEPS - 1;
        assert!(consume_playback_step(&mut session));
        assert!(!consume_playback_step(&mut session));
    }

    /// A played action inside a nested batch (no enclosing playback) continues the shared count the
    /// batch carries instead of restarting it for each input.
    #[test]
    fn a_nested_batch_input_does_not_reset_the_shared_playback_count() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        session.actions.list.push(Action::new("A", vec![ActionStep::new("select.all", json!({}))])).unwrap();
        session.actions.budget_shared = true;
        session.actions.playback_steps = MAX_PLAYBACK_STEPS;
        let r = session.execute("actions.play", json!({"action": "A"}));
        assert!(matches!(r, Err(EngineError::BatchLimit(_))), "the carried count leaves no room for the action: {r:?}");
        session.actions.budget_shared = false;
        let r = session.execute("actions.play", json!({"action": "A"})).unwrap();
        assert_eq!(r["ran"], 1, "a top-level call starts its own count");
    }

    /// An action of mostly disabled steps, called over and over from a parent, is charged for
    /// every step it stores, so the calls stop at the shared limit instead of each copying and
    /// walking the whole action for free.
    #[test]
    fn disabled_steps_are_charged_to_the_playback_budget() {
        let mut session = Session::new();
        let mut child = Action::new("Child", (0..1000).map(|_| ActionStep::new("select.all", json!({}))).collect());
        child.disabled = (0..1000).collect();
        let calls = MAX_PLAYBACK_STEPS as usize / 1000 * 2;
        let parent = Action::new("Parent", (0..calls).map(|_| ActionStep::new("actions.play", json!({"action": "Child"}))).collect());
        session.actions.list.push(child).unwrap();
        session.actions.list.push(parent).unwrap();
        let r = session.execute("actions.play", json!({"action": "Parent"})).unwrap();
        let failed = r["failed"]["error"].as_str().unwrap();
        assert!(failed.contains("step limit"), "{failed}");
        // Each call charges its 1000 stored steps plus the Play step itself.
        let ran = r["ran"].as_u64().unwrap();
        assert!(ran < MAX_PLAYBACK_STEPS / 1000, "{ran}");
    }

    /// Limit stops are recognised from a nested batch's `errors` and from a played action's `failed`;
    /// an ordinary failure is not one.
    #[test]
    fn limit_stops_are_found_in_nested_results() {
        let batch = json!({"files": [], "errors": [{"file": "a", "error": "x"}, {"file": "b", "error": "too deep", "limit": true}]});
        assert_eq!(limit_stop(&batch).as_deref(), Some("too deep"));
        let played = json!({"action": "A", "failed": {"step": 2, "id": "x", "error": "over", "limit": true}});
        assert_eq!(limit_stop(&played).as_deref(), Some("over"));
        assert_eq!(limit_stop(&json!({"errors": [{"file": "a", "error": "x"}], "failed": {"step": 0, "error": "y"}})), None);
    }

    #[test]
    fn steps_before_from_are_charged() {
        let action = Action::new("A", (0..10).map(|_| ActionStep::new("select.all", json!({}))).collect());
        assert_eq!(skipped_steps(&action, 7), 7);
        assert_eq!(skipped_steps(&action, 50), 10);
        let mut disabled = action.clone();
        disabled.disabled = [1, 8, 9, 40].into_iter().collect();
        assert_eq!(skipped_steps(&disabled, 7), 9);
        assert_eq!(skipped_steps(&disabled, 0), 3);
    }
}
