//! History panel › click a state: `history.selectState` returns the document to that state in one
//! step (#2966), as Photoshop does. The panel used to run Edit › Undo / Redo once per state: each
//! bumped the document revision, so the canvas could no longer tell what the jump changed and
//! recomposited (and re-uploaded) the whole document, plus N rounds of per-command bookkeeping.
//!
//! States are numbered as the History panel lists them, like `history.deleteState`: 0 is the
//! oldest state still held, `past` (the number of undo states) is the current one, and the redo
//! states (dimmed in the panel) follow it. Jumping back leaves the skipped states redoable;
//! jumping forward redoes them. Nothing is discarded until the next edit.

use serde_json::{Value, json};

use crate::commands::{CommandSpec, int};
use crate::{EngineError, Result, Session};

const CMD: &str = "history.selectState";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

fn select_state(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let past = st.history.past_len();
    let last = past.saturating_add(st.history.redo_labels().count());
    let v = match p.get("state") {
        None | Some(Value::Null) => return Err(bad("missing `state` (the History entry to return to, 0 = oldest)")),
        Some(_) => int(p, "state").ok_or_else(|| bad("`state` must be a number"))?,
    };
    let target = usize::try_from(v).ok().filter(|i| *i <= last).ok_or_else(|| bad(format!("`state` = {v} is not a state (0..={last})")))?;
    // A background job is computing from the current state: it must not move under it.
    if s.active_job().is_some() {
        return Err(EngineError::Other("the history can't move while a command is still running on this document".into()));
    }
    let keep_visibility = !s.visibility_undoable();
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    if target == past {
        return Ok(json!({ "state": target, "changed": false }));
    }
    st.coalesce = None;
    // Step the history itself (pointer moves, no pixels touched), then make the reached state
    // current once: one revision, one damage rect for the canvas.
    let start = st.doc.clone();
    let mut doc = start.clone();
    let mut layers = None;
    let mut at = past;
    while at != target {
        // History Options: visibility changes that are not history states stay as they are,
        // step by step as Undo / Redo keep them, except on the layers whose visibility the step
        // passed over changed.
        let undoing = target < at;
        let mut changed = if undoing { st.history.current_visibility().to_vec() } else { Vec::new() };
        let step = if undoing { st.history.undo(doc.clone()) } else { st.history.redo(doc.clone()) };
        let Some((mut d, l)) = step else { break };
        if keep_visibility {
            if !undoing {
                changed = st.history.current_visibility().to_vec();
            }
            crate::carry_visibility(&doc, &mut d, &changed);
        }
        doc = d;
        layers = Some(l);
        at = if target < at { at - 1 } else { at + 1 };
    }
    let Some(layers) = layers else {
        return Err(EngineError::Other("the history can't step right now".into()));
    };
    // Save As changes file identity outside history, as Edit › Undo keeps it.
    if doc.name != start.name {
        std::sync::Arc::make_mut(&mut doc).name.clone_from(&start.name);
    }
    let damage = crate::layer_multi_cmds::step_damage(&start, &doc);
    *st.doc_mut() = doc;
    // Stepping only over selection changes keeps a clean document clean (#6865).
    let keep_clean = st.keeps_clean(&start);
    crate::restore_target(st, layers);
    st.bump_revision(keep_clean);
    st.last_damage = damage;
    st.record_damage();
    if at != target {
        // Not reached (the bounds were checked): the document is consistent at the state reached.
        return Err(EngineError::Other(format!("the history stopped at state {at}")));
    }
    Ok(json!({ "state": target, "changed": true, "current": st.history.entries().last() }))
}

fn can_select(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or_else(|| "no document open".to_string())?;
    if st.history.can_undo() || st.history.can_redo() { Ok(()) } else { Err("the history holds only one state".into()) }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Select History State",
        menu: &[],
        shortcut: None,
        params: r##"{"state":index} (History entry as the panel lists them: 0 = oldest, then the current state, then the redo states) → {state,changed,current?}: returns the document to that state in one step; the states after it stay redoable"##,
        enabled: can_select,
        run: select_state,
        journal: true,
    }]
}
