//! History panel actions beyond stepping (#1117): Delete (the trash button) and New Document (the
//! "Create new document from current state" button). Stepping to a state is Edit › Undo / Redo.
//!
//! States are indexed like [`photocraft_ops::History::entries`]: 0 is the oldest state still
//! held, the last index is the current document. Snapshots are not implemented.

use serde_json::{Value, json};

use crate::commands::{CommandSpec, int};
use crate::image_cmds::for_each_layer;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// The `state` param as an index into the active document's history entries, defaulting to the
/// current state.
fn state_index(s: &Session, cmd: &str, p: &Value) -> Result<(usize, usize)> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let current = st.history.past_len();
    let i = match p.get("state") {
        None | Some(Value::Null) => current,
        Some(_) => {
            let v = int(p, "state").ok_or_else(|| bad(cmd, "`state` must be a number"))?;
            usize::try_from(v).ok().filter(|i| *i <= current).ok_or_else(|| bad(cmd, format!("`state` = {v} is not a state (0..={current})")))?
        }
    };
    Ok((i, current))
}

/// History panel › Delete: drop the state and every state after it, as Photoshop does in linear
/// history. The document returns to the state before it, and the deleted steps can't be redone.
/// The oldest state (the document as opened) can't be deleted.
fn delete_state(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "history.deleteState";
    let (i, current) = state_index(s, CMD, p)?;
    if i == 0 {
        return Err(bad(CMD, "the oldest state can't be deleted"));
    }
    // Step back to the state before `i`; the steps from `i` on are then redo states.
    for _ in i..=current {
        if !s.undo() {
            return Err(EngineError::Other("the history can't step back right now".into()));
        }
    }
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    st.history.clear_redo();
    let entries = st.history.entries();
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
    for_each_layer(&mut doc.layers, &mut |l| l.id = photocraft_doc::LayerId::fresh());
    let name = doc.name.clone();
    let index = s.add_document(doc, None);
    Ok(json!({ "document": index, "name": name }))
}

fn can_delete(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| d.history.can_undo()).map(|_| ()).ok_or_else(|| "the oldest state can't be deleted".into())
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "history.deleteState",
            label: "Delete State",
            menu: &[],
            shortcut: None,
            params: r##"{"state":index?} (History entry, 0 = oldest; default the current state) → {states,current}: removes the state and every later one"##,
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
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A document with three steps after Open: entries Open, New Layer, Fill, Fill.
    fn session() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 20, "height": 10})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
        s
    }

    fn entries(s: &Session) -> Vec<String> {
        s.active().unwrap().history.entries()
    }

    #[test]
    fn delete_current_state_steps_back_and_drops_it() {
        let mut s = session();
        let before = entries(&s);
        let prev_doc = s.active().unwrap().history.state(before.len() - 2).unwrap();
        let r = s.execute("history.deleteState", json!({})).unwrap();
        assert_eq!(r["states"], json!(before.len() - 1));
        assert_eq!(entries(&s), before[..before.len() - 1].to_vec());
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &prev_doc), "the document is the state before");
        assert!(!s.active().unwrap().history.can_redo(), "the deleted state can't be redone");
        assert!(s.execute("edit.redo", json!({})).is_err());
    }

    #[test]
    fn delete_an_earlier_state_drops_it_and_every_later_one() {
        let mut s = session();
        let first = s.active().unwrap().history.state(0).unwrap();
        s.execute("history.deleteState", json!({"state": 1})).unwrap();
        assert_eq!(entries(&s), vec!["Open".to_string()]);
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &first));
        assert!(!s.active().unwrap().history.can_redo());
        // Redo states after an undo go too.
        let mut s = session();
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("history.deleteState", json!({})).unwrap();
        assert_eq!(entries(&s).len(), 2);
        assert!(!s.active().unwrap().history.can_redo());
    }

    #[test]
    fn delete_state_refuses_the_oldest_state_and_bad_params() {
        let mut s = Session::new();
        assert!(s.execute("history.deleteState", json!({})).is_err(), "no document");
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        assert!(!s.is_enabled("history.deleteState"), "only the opening state");
        assert!(s.execute("history.deleteState", json!({})).is_err());
        let mut s = session();
        let before = entries(&s);
        for p in [json!({"state": 0}), json!({"state": -1}), json!({"state": 99}), json!({"state": "x"}), json!({"state": f64::MAX}), json!({"state": [1]})] {
            assert!(s.execute("history.deleteState", p.clone()).is_err(), "{p}");
            assert_eq!(entries(&s), before, "{p} changed nothing");
        }
    }

    #[test]
    fn new_document_from_a_state_copies_it_into_a_fresh_document() {
        let mut s = session();
        let src = s.active().unwrap().doc.clone();
        let label = entries(&s).last().cloned().unwrap();
        let r = s.execute("history.newDocument", json!({})).unwrap();
        assert_eq!(s.documents().len(), 2);
        assert_eq!(r["document"], json!(1));
        assert_eq!(r["name"], json!(label), "named after the state");
        let new = s.active().unwrap();
        assert_eq!(new.doc.size, src.size);
        assert_eq!(new.doc.layers.len(), src.layers.len());
        assert_ne!(new.doc.id, src.id);
        assert_eq!(new.history.entries().len(), 1, "a fresh history");
        // An earlier state, with a chosen name; the source document is untouched.
        s.set_active(0);
        let before = entries(&s);
        let open = s.active().unwrap().history.state(0).unwrap();
        s.execute("history.newDocument", json!({"state": 0, "name": "Start"})).unwrap();
        let new = s.active().unwrap();
        assert_eq!(new.doc.name, "Start");
        assert_eq!(new.doc.layers.len(), open.layers.len());
        s.set_active(0);
        assert_eq!(entries(&s), before);
    }

    #[test]
    fn new_document_refuses_bad_params() {
        let mut s = Session::new();
        assert!(s.execute("history.newDocument", json!({})).is_err(), "no document");
        let mut s = session();
        for p in [json!({"state": -1}), json!({"state": 99}), json!({"state": "x"}), json!({"state": f64::NAN.to_string()})] {
            assert!(s.execute("history.newDocument", p.clone()).is_err(), "{p}");
        }
        assert_eq!(s.documents().len(), 1);
    }
}
