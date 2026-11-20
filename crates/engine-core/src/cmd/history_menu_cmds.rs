//! The History panel menu (☰, #2966): Step Forward, Step Backward and Clear History. Delete and
//! New Document are [`crate::history_cmds`]; New Snapshot and History Options live elsewhere.
//!
//! Step Forward / Step Backward move one state along the History list, like Edit › Redo / Undo
//! (which step through the list too), but report a step that isn't there as an error. Clear
//! History takes every state but the current one off the list without changing the image; it is
//! undoable: Edit › Undo puts the states back until the next step is recorded.

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// `{states, current, label}`: the list length, the current state's index (0 = oldest) and its
/// name, after a History panel menu command.
fn list(s: &Session) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let entries = st.history.entries();
    Ok(json!({ "states": entries.len() + st.history.redo_labels().count(), "current": st.history.past_len(), "label": entries.last() }))
}

fn step(s: &mut Session, forward: bool) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let possible = if forward { st.history.can_redo() } else { st.history.can_undo() };
    if !possible {
        return Err(EngineError::Other(if forward { "no later state to step forward to" } else { "no earlier state to step back to" }.into()));
    }
    let moved = if forward { s.redo() } else { s.undo() };
    if !moved {
        return Err(EngineError::Other("the history can't step right now".into()));
    }
    list(s)
}

/// History panel › Clear History: only the current state stays in the list; the image doesn't
/// change. Edit › Undo brings the states back.
fn clear(s: &mut Session, _p: &Value) -> Result<Value> {
    if s.active_job().is_some() {
        return Err(EngineError::Other("the history can't change while a job runs".into()));
    }
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    st.coalesce = None;
    if !st.history.clear_states() {
        return Err(EngineError::Other("the current state is the only state".into()));
    }
    // Edit › Fade blends against the previous state, which is no longer in the list.
    s.edit_state.fade = None;
    list(s)
}

fn can_step_forward(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.history.can_redo() { Ok(()) } else { Err("no later state".into()) }
}

fn can_step_backward(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.history.can_undo() { Ok(()) } else { Err("no earlier state".into()) }
}

fn can_clear(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.history.past_len() > 0 || st.history.can_redo() { Ok(()) } else { Err("the current state is the only state".into()) }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "history.stepForward",
            label: "Step Forward",
            menu: &[],
            shortcut: None,
            params: r##"{} → {states,current,label}: makes the next History state current (Err when there is none)"##,
            enabled: can_step_forward,
            run: |s, _| step(s, true),
            journal: true,
        },
        CommandSpec {
            id: "history.stepBackward",
            label: "Step Backward",
            menu: &[],
            shortcut: None,
            params: r##"{} → {states,current,label}: makes the previous History state current (Err when there is none)"##,
            enabled: can_step_backward,
            run: |s, _| step(s, false),
            journal: true,
        },
        CommandSpec {
            id: "history.clear",
            label: "Clear History",
            menu: &[],
            shortcut: None,
            params: r##"{} → {states,current,label}: removes every History state but the current one without changing the image; Edit › Undo restores them"##,
            enabled: can_clear,
            run: clear,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {

    use super::*;

    fn entries(s: &Session) -> Vec<String> {
        s.active().unwrap().history.entries()
    }

    #[test]
    fn history_menu_commands_refuse_without_a_document_and_ignore_odd_params() {
        let mut s = crate::session();
        for id in ["history.stepForward", "history.stepBackward", "history.clear"] {
            assert!(!s.is_enabled(id), "{id}");
            assert!(s.execute(id, json!({})).is_err(), "{id} without a document");
        }
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        for id in ["history.stepForward", "history.stepBackward", "history.clear"] {
            assert!(!s.is_enabled(id), "{id}: only the opening state");
            for p in [json!({}), json!(null), json!([1]), json!({"state": -1}), json!("x")] {
                assert!(s.execute(id, p.clone()).is_err(), "{id} {p}");
            }
            assert_eq!(entries(&s), ["New"]);
        }
    }
}
