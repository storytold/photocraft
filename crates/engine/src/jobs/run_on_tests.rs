//! [`run_on`]: a job applies to the document it was started for, whichever is current.

use serde_json::json;

use super::*;
use crate::Session;

/// Two documents, the second current; returns the first one's id.
fn two_documents() -> (Session, DocId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let first = s.active().unwrap().doc.id;
    s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    assert_ne!(s.active().unwrap().doc.id, first);
    (s, first)
}

#[test]
fn a_cooperative_session_applies_run_on_to_its_target_document() {
    let (mut s, target) = two_documents();
    let current = s.active().unwrap().doc.id;
    s.set_cooperative_jobs(true);
    // As inside `Session::start`: a cooperative session still runs this job inline.
    s.jobs.spawn = true;
    assert!(!s.spawning(), "a cooperative session spawns no worker");
    let v = run_on(&mut s, "Test", target, |_| Ok(()), move |s, ()| Ok(json!(s.active().map(|st| st.doc.id.0)))).unwrap();
    s.jobs.spawn = false;
    assert_eq!(v, json!(target.0), "apply ran on the document the job was started for");
    assert_eq!(s.active().unwrap().doc.id, current, "the current document is unchanged");
}
