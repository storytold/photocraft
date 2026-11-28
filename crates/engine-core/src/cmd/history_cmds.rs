//! History panel actions beyond stepping (#1117): Delete (the trash button) and New Document (the
//! "Create new document from current state" button). Stepping to a state is Edit › Undo / Redo.
//!
//! States are indexed like [`photocraft_ops::History::entries`]: 0 is the oldest state still
//! held, the last index is the current document.
//!
//! Snapshots (#2966, the camera button) are named copies of the document that outlive the
//! states: `history.newSnapshot` takes one, `history.selectSnapshot` brings it back as a new,
//! undoable state. They live on [`crate::DocState::snapshots`] for the session only, as in
//! Photoshop, and are addressed by an id that is never reused. Their tiles are shared
//! copy-on-write with the document and its history; the tiles only snapshots hold count against
//! the history's pixel budget ([`crate::DocState::trim_history`]), so a Merged Layers snapshot
//! (a full flattened copy) makes room by dropping the oldest states rather than taking memory
//! beyond the budget unseen (#4530). Edit › Purge keeps them.

use std::sync::Arc;

use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_ops::LayerTarget;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, int};
use crate::image_cmds::reidentify_copied_layers;
use crate::{EngineError, Result, Session};

/// A History panel snapshot: the document as it was when taken, and the layers it targeted.
#[derive(Clone, Debug, craft_ui_macros::PlainData)]
pub struct Snapshot {
    pub id: u64,
    pub name: String,
    pub doc: Arc<Document>,
    pub layers: LayerTarget,
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// The `state` param as an index into the active document's history entries, defaulting to the
/// current state.
fn state_index(s: &Session, cmd: &str, p: &Value) -> Result<(usize, usize)> {
    state_index_upto(s, cmd, p, false)
}

/// [`state_index`], also accepting the later (dimmed, redo) states with `later`.
fn state_index_upto(s: &Session, cmd: &str, p: &Value, later: bool) -> Result<(usize, usize)> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let current = st.history.past_len();
    let last = if later { current.saturating_add(st.history.redo_labels().count()) } else { current };
    let i = match p.get("state") {
        None | Some(Value::Null) => current,
        Some(_) => {
            let v = int(p, "state").ok_or_else(|| bad(cmd, "`state` must be a number"))?;
            usize::try_from(v).ok().filter(|i| *i <= last).ok_or_else(|| bad(cmd, format!("`state` = {v} is not a state (0..={last})")))?
        }
    };
    Ok((i, current))
}

/// History panel › Delete: drop the state and every state after it, as Photoshop does in linear
/// history (in non-linear history only that state, also a later one:
/// `history_options_cmds::delete_state`). The document returns to the state before it, and the
/// deleted steps can't be redone. The oldest state (the document as opened) can't be deleted.
fn delete_state(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "history.deleteState";
    // History Options › Allow Non-Linear History: only that state goes, a later (dimmed) one too.
    let non_linear = s.active().is_some_and(|st| st.history.non_linear);
    let (i, current) = state_index_upto(s, CMD, p, non_linear)?;
    if i == 0 {
        return Err(bad(CMD, "the oldest state can't be deleted"));
    }
    let rows = crate::actions_cmds::history_rows(s);
    if non_linear {
        crate::history_options_cmds::delete_state(s, i, current)?;
    } else {
        // Step back to the state before `i`; the steps from `i` on are then redo states.
        for _ in i..=current {
            if !s.undo() {
                return Err(EngineError::Other("the history can't step back right now".into()));
            }
        }
        s.active_mut().ok_or(EngineError::NoDocument)?.history.clear_redo();
    }
    crate::actions_cmds::reconcile_history_removal(s, &rows);
    let entries = s.active().ok_or(EngineError::NoDocument)?.history.entries();
    Ok(json!({ "states": entries.len(), "current": entries.last() }))
}

/// History panel › New Document: a new untitled document holding the document as it was at a
/// state (the current one by default), named after that state, with a history of its own.
fn new_document(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "history.newDocument";
    let (i, current) = state_index(s, CMD, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let src = if i == current { Some(st.doc.clone()) } else { st.history.state(i) };
    let src = src.ok_or_else(|| bad(CMD, format!("state {i} is not held")))?;
    let label = st.history.entries().get(i).cloned().unwrap_or_default();
    let mut doc = (*src).clone();
    doc.id = photocraft_doc::DocId::fresh();
    doc.name = match p.get("name").and_then(Value::as_str) {
        Some(n) if !n.trim().is_empty() => n.to_string(),
        _ if !label.trim().is_empty() => label,
        _ => format!("{} copy", doc.name),
    };
    reidentify_copied_layers(&mut doc);
    let name = doc.name.clone();
    let index = s.add_document(doc, None);
    Ok(json!({ "document": index, "name": name }))
}

/// The `snapshot` param: the index of the active document's snapshot with that id.
fn snapshot_index(s: &Session, cmd: &str, p: &Value) -> Result<usize> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = p.get("snapshot").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "`snapshot` must be a snapshot id"))?;
    st.snapshots().iter().position(|snap| snap.id == id).ok_or_else(|| bad(cmd, format!("no snapshot {id}")))
}

/// The name `history.newSnapshot` gives a snapshot without one: "Snapshot N" (the New Snapshot
/// dialog suggests it too).
pub fn next_snapshot_name(st: &crate::DocState) -> String {
    format!("Snapshot {}", st.next_snapshot)
}

/// A non-blank `name` param, trimmed; `None` when absent.
fn name_param(cmd: &str, p: &Value) -> Result<Option<String>> {
    match p.get("name") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(n)) if !n.trim().is_empty() => Ok(Some(n.trim().to_string())),
        Some(_) => Err(bad(cmd, "`name` must be a non-empty string")),
    }
}

/// History panel › Create new snapshot (the camera button): a named copy of the document,
/// "Snapshot N" by default. `from` is the New Snapshot dialog's From: the full document (default),
/// its layers merged into one, or only the active layer. Not a history state.
fn new_snapshot(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "history.newSnapshot";
    let name = name_param(CMD, p)?;
    let from = match p.get("from") {
        None | Some(Value::Null) => "fullDocument",
        Some(Value::String(f)) if matches!(f.as_str(), "fullDocument" | "mergedLayers" | "currentLayer") => f.as_str(),
        Some(_) => return Err(bad(CMD, "`from` must be \"fullDocument\", \"mergedLayers\" or \"currentLayer\"")),
    };
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let one_layer = |doc: &Document, layer: Layer| {
        let mut doc = doc.clone();
        let id = layer.id;
        doc.layers = vec![layer].into();
        (Arc::new(doc), LayerTarget { active: Some(id), selected: vec![id] })
    };
    let (doc, layers) = match from {
        "mergedLayers" => {
            let fmt = st.doc.pixel_format();
            one_layer(&st.doc, Layer::new("Merged Layers", LayerContent::Raster(crate::file_cmds::flattened(&st.doc, fmt))))
        }
        "currentLayer" => {
            let layer = st.active_layer.and_then(|id| st.doc.layer(id)).ok_or_else(|| bad(CMD, "no active layer"))?;
            one_layer(&st.doc, layer.clone())
        }
        _ => (st.doc.clone(), st.layer_target()),
    };
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let name = name.unwrap_or_else(|| next_snapshot_name(st));
    let id = st.next_snapshot;
    st.next_snapshot = id.saturating_add(1);
    // An action's New Snapshot with "replace existing": a snapshot of that name gives way.
    if p.get("replaceExisting").and_then(Value::as_bool) == Some(true) {
        st.snapshots_mut().retain(|snap| snap.name != name);
    }
    st.snapshots_mut().push(Snapshot { id, name: name.clone(), doc, layers });
    Ok(json!({ "snapshot": id, "name": name }))
}

/// Clicking a snapshot: the document returns to it as one new, undoable state named after it.
fn select_snapshot(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "history.selectSnapshot";
    let i = snapshot_index(s, CMD, p)?;
    // A background job is computing from the current state: it must not move under it.
    if s.active_job().is_some() {
        return Err(EngineError::Other("the document is busy with a background job".into()));
    }
    let snap = s.active().and_then(|st| st.snapshots().get(i).cloned()).ok_or(EngineError::NoDocument)?;
    s.edit(&snap.name, |doc, active| {
        // Keep the identity (colour state, views and caches are keyed by it) and the tab name.
        let (id, name) = (doc.id, doc.name.clone());
        *doc = (*snap.doc).clone();
        doc.id = id;
        doc.name = name;
        *active = snap.layers.active.filter(|l| doc.layer(*l).is_some()).or_else(|| doc.top_layer());
        Ok(())
    })?;
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    if st.active_layer == snap.layers.active {
        st.selection_mut().clone_from(&snap.layers.selected);
        crate::fix_selection(st);
    }
    Ok(json!({ "snapshot": snap.id, "name": snap.name }))
}

/// Renaming a snapshot (double-click its name).
fn rename_snapshot(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "history.renameSnapshot";
    let i = snapshot_index(s, CMD, p)?;
    let name = name_param(CMD, p)?.ok_or_else(|| bad(CMD, "`name` is required"))?;
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let snap = st.snapshots_mut().get_mut(i).ok_or_else(|| bad(CMD, "no such snapshot"))?;
    snap.name.clone_from(&name);
    Ok(json!({ "snapshot": snap.id, "name": name }))
}

/// Deleting a snapshot; the document and its history are unchanged.
fn delete_snapshot(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "history.deleteSnapshot";
    let i = snapshot_index(s, CMD, p)?;
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let snap = st.snapshots_mut().remove(i);
    Ok(json!({ "snapshot": snap.id, "snapshots": st.snapshots().len() }))
}

/// A state other than the oldest exists to delete: a past or the current one, and with Allow
/// Non-Linear History a later (dimmed) one too, also while the oldest state is current.
fn can_delete(s: &Session) -> std::result::Result<(), String> {
    s.active()
        .filter(|d| d.history.past_len() > 0 || (d.history.non_linear && d.history.can_redo()))
        .map(|_| ())
        .ok_or_else(|| "the oldest state can't be deleted".into())
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// `history.undo` / `history.redo` enable predicates, matching `edit.undo` / `edit.redo`.
fn can_undo(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| d.history.can_undo()).map(|_| ()).ok_or_else(|| "nothing to undo".into())
}

fn can_redo(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| d.history.can_redo()).map(|_| ()).ok_or_else(|| "nothing to redo".into())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        // `history.*` aliases of Edit › Undo / Redo, so a client reaching for the History namespace
        // finds them (`edit.undo` / `edit.redo` remain the menu items and own the shortcuts, #3675).
        CommandSpec {
            id: "history.undo",
            label: "Undo",
            menu: &[],
            shortcut: None,
            params: "{} (the same action as `edit.undo`)",
            enabled: can_undo,
            run: |s, _| Ok(json!(s.undo())),
            journal: false,
        },
        CommandSpec {
            id: "history.redo",
            label: "Redo",
            menu: &[],
            shortcut: None,
            params: "{} (the same action as `edit.redo`)",
            enabled: can_redo,
            run: |s, _| Ok(json!(s.redo())),
            journal: false,
        },
        CommandSpec {
            id: "history.deleteState",
            label: "Delete State",
            menu: &[],
            shortcut: None,
            params: r##"{"state":index?} (History entry, 0 = oldest; default the current state) → {states,current}: removes the state and every later one (with Allow Non-Linear History only that state, a later dimmed one too)"##,
            enabled: can_delete,
            run: delete_state,
            journal: true,
        },
        CommandSpec {
            id: "history.newDocument",
            label: "New Document from State",
            menu: &[],
            shortcut: None,
            params: r##"{"state":index?,"name":str?} (History entry, 0 = oldest; default the current state) → {document,name}"##,
            enabled: has_doc,
            run: new_document,
            journal: true,
        },
        CommandSpec {
            id: "history.newSnapshot",
            label: "New Snapshot",
            menu: &[],
            shortcut: None,
            params: r##"{"name":str?="Snapshot N","from":"fullDocument"|"mergedLayers"|"currentLayer"="fullDocument","replaceExisting":bool=false (drop snapshots of the same name first)} → {snapshot,name}: a named copy of the document for the session (not a history state)"##,
            enabled: has_doc,
            run: new_snapshot,
            journal: true,
        },
        CommandSpec {
            id: "history.selectSnapshot",
            label: "Select Snapshot",
            menu: &[],
            shortcut: None,
            params: r##"{"snapshot":id} → {snapshot,name}: the document returns to the snapshot as a new history state named after it"##,
            enabled: has_doc,
            run: select_snapshot,
            journal: true,
        },
        CommandSpec {
            id: "history.renameSnapshot",
            label: "Rename Snapshot",
            menu: &[],
            shortcut: None,
            params: r##"{"snapshot":id,"name":str} → {snapshot,name}"##,
            enabled: has_doc,
            run: rename_snapshot,
            journal: true,
        },
        CommandSpec {
            id: "history.deleteSnapshot",
            label: "Delete Snapshot",
            menu: &[],
            shortcut: None,
            params: r##"{"snapshot":id} → {snapshot,snapshots}: the document and its history are unchanged"##,
            enabled: has_doc,
            run: delete_snapshot,
            journal: true,
        },
    ]
}
