//! The History Brush's source (#2966): the History panel's source column, at the left of every
//! state and snapshot, sets what the History Brush (and Fill › History) restores from. Exactly one
//! source per document; by default the oldest state held (Photoshop's default is the first
//! snapshot, the document as opened, which that state is until it falls off the history).
//!
//! A state source is kept by state, not by index ([`photocraft_ops::History::set_brush_source`]),
//! so it stays on its state while older states fall off, and that state is kept when it would fall
//! off the history itself. A snapshot source is kept by snapshot id ([`HistorySource::Snapshot`]),
//! a separate namespace from the states. Once the source state or snapshot is deleted, or a redo
//! state source is discarded by a new step, there is no source until one is set, and the History
//! Brush refuses to paint without an explicit `state` or `snapshot`, as Photoshop does.
//! (Photoshop keeps a source discarded by a new step in the list; that needs states outside the
//! linear history, so here it is gone.) Edit › Purge › Histories keeps the snapshots (and a
//! snapshot source) and resets a state source to the default.
//!
//! Rows: `state` is a History panel row everywhere here (this command, `paint.historyBrush`, Fill
//! › History and `historySource` in `doc_inspect`): the states (0 = oldest), the current one, then
//! the greyed redo states, the next redo first.

use std::sync::Arc;

use photocraft_doc::Document;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{DocState, EngineError, Result, Session};

const CMD: &str = "history.setSource";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

/// What the History Brush paints from: a History panel state row, or a snapshot by id. The two
/// are separate, so snapshot 1 and state row 1 never stand for each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistorySource {
    /// A History panel row ([`photocraft_ops::History::row_id`]).
    State(usize),
    /// A History panel snapshot ([`crate::history_cmds::Snapshot::id`]).
    Snapshot(u64),
}

/// The document's History Brush source, `None` once the source state or snapshot is gone.
pub fn source(st: &DocState) -> Option<HistorySource> {
    match st.source_snapshot.or_else(|| st.history.brush_source_snapshot()) {
        Some(id) => st.snapshots().iter().any(|s| s.id == id).then_some(HistorySource::Snapshot(id)),
        None => st.history.brush_source().map(HistorySource::State),
    }
}

pub fn source_of(st: &DocState, src: HistorySource) -> Option<Arc<Document>> {
    match src {
        HistorySource::State(i) => st.history.row_doc(i, &st.doc),
        HistorySource::Snapshot(id) => st.snapshots().iter().find(|s| s.id == id).map(|s| s.doc.clone()),
    }
}

/// A `state` or `snapshot` param (`None` when neither is passed), checked against the document.
pub fn parse(st: &DocState, p: &Value, cmd: &str) -> Result<Option<HistorySource>> {
    let bad = |msg: String| EngineError::BadParams { cmd: cmd.into(), msg };
    let get = |k: &str| p.get(k).filter(|v| !v.is_null());
    let index = |k: &str, v: &Value| v.as_u64().ok_or_else(|| bad(format!("`{k}` must be a non-negative integer, not {v}")));
    match (get("state"), get("snapshot")) {
        (Some(_), Some(_)) => Err(bad("pass a `state` or a `snapshot`, not both".into())),
        (Some(v), None) => {
            let rows = st.history.entries().len().saturating_add(st.history.redo_labels().count());
            let i = usize::try_from(index("state", v)?).ok().filter(|i| *i < rows);
            let i = i.ok_or_else(|| bad(format!("no history state {v}: a History panel row (0..{rows}, the greyed redo rows last)")))?;
            Ok(Some(HistorySource::State(i)))
        }
        (None, Some(v)) => {
            let id = index("snapshot", v)?;
            if !st.snapshots().iter().any(|s| s.id == id) {
                return Err(bad(format!("no snapshot {id}")));
            }
            Ok(Some(HistorySource::Snapshot(id)))
        }
        (None, None) => Ok(None),
    }
}

/// `history.setSource {state}` or `{snapshot}`: make a History panel row or a snapshot the History
/// Brush's source.
fn set_source(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    match parse(st, p, CMD)? {
        None => return Err(bad("pass the `state` (a History panel row, 0 = oldest) or the `snapshot` id to paint from")),
        Some(HistorySource::State(i)) => {
            if !st.history.set_brush_source(i) {
                return Err(bad(format!("no history state {i}")));
            }
            st.source_snapshot = None;
        }
        Some(HistorySource::Snapshot(id)) => {
            // No state is the source now, so none is kept past its time.
            st.history.reset_brush_source();
            st.source_snapshot = Some(id);
        }
    }
    Ok(source_json(st))
}

/// The source as `doc_inspect` reports it: `{state,label}` (a History panel row) or
/// `{snapshot,name}`; `null` once its state or snapshot is gone.
pub fn source_json(st: &DocState) -> Value {
    match source(st) {
        None => Value::Null,
        Some(HistorySource::State(i)) => {
            let label = st.history.entries().into_iter().chain(st.history.redo_labels().map(str::to_string)).nth(i);
            json!({ "state": i, "label": label })
        }
        Some(HistorySource::Snapshot(id)) => {
            json!({ "snapshot": id, "name": st.snapshots().iter().find(|s| s.id == id).map(|s| s.name.clone()) })
        }
    }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Set History Brush Source",
        menu: &[],
        shortcut: None,
        params: r##"{"state":row} | {"snapshot":id} (state: a History panel row, 0 = oldest state, then the current one, then the greyed redo states; snapshot: a history.newSnapshot id) → {state,label} | {snapshot,name}: paint.historyBrush and Fill › History use this source when neither `state` nor `snapshot` is passed"##,
        enabled: has_doc,
        run: set_source,
        journal: true,
    }]
}
