//! Preferences › Performance › Save History in Document (#6053): a `.pcraft` save keeps the undo
//! history when asked to, and opening the file brings it back.

use photocraft_engine::Session;
use serde_json::json;

/// A session with one document after three steps: New Layer, Fill, New Layer.
fn edited(save_history: bool) -> Session {
    let mut s = Session::new();
    s.execute("prefs.set", json!({"path": "performance.saveHistoryInDocument", "value": save_history})).unwrap();
    s.execute("file.new", json!({"width": 300, "height": 300})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

fn saved_bytes(s: &Session) -> Vec<u8> {
    let doc = s.document_to_save(s.active_index().unwrap()).unwrap();
    photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap()
}

fn has_history_section(bytes: &[u8]) -> bool {
    photocraft_format::zip::ZipReader::new(bytes).unwrap().find(photocraft_format::saved_history::HISTORY).is_some()
}

/// Opens `bytes` as a `.pcraft` file in `s`, the way File › Open does.
fn reopen(s: &mut Session, bytes: &[u8]) -> usize {
    let doc = photocraft_format::load_from_bytes(bytes).unwrap();
    s.open_document(doc, Some("reopened.pcraft".into())).0
}

#[test]
fn the_preference_is_off_by_default_and_then_saves_no_history() {
    assert!(!Session::new().prefs().performance.save_history_in_document);
    let s = edited(false);
    let doc = s.document_to_save(0).unwrap();
    assert!(doc.saved_history.is_none());
    assert!(!has_history_section(&saved_bytes(&s)));
}

#[test]
fn undo_works_after_reopening() {
    let s = edited(true);
    let entries = s.active().unwrap().history.entries();
    let filled = s.active().unwrap().history.state(2).unwrap();
    let bytes = saved_bytes(&s);
    assert!(has_history_section(&bytes));

    let mut t = Session::new();
    let i = reopen(&mut t, &bytes);
    let st = &t.documents()[i];
    assert_eq!(st.history.entries(), entries, "the History panel lists the same steps");
    assert!(st.doc.saved_history.is_none(), "the session took the history over");
    assert!(st.history.state(0).unwrap().saved_history.is_none());
    let layers = st.doc.layers.len();

    assert!(t.undo(), "the last step can be undone right after opening");
    let st = t.active().unwrap();
    assert_eq!(st.doc.layers.len(), layers - 1);
    assert_eq!(st.doc.layers, filled.layers, "undo goes back to the state that was saved");
    assert_eq!(st.doc.id, t.documents()[i].doc.id);
    assert!(t.undo() && t.undo(), "every saved state can be undone");
    assert!(!t.undo(), "nothing before the first state");
    assert!(t.redo() && t.redo() && t.redo());
    assert_eq!(t.active().unwrap().doc.layers.len(), layers);
}

#[test]
fn a_file_without_history_opens_with_none() {
    let s = edited(false);
    let mut t = Session::new();
    reopen(&mut t, &saved_bytes(&s));
    assert!(!t.active().unwrap().history.can_undo());
}

#[test]
fn opening_the_file_twice_keeps_each_history_on_its_own_document() {
    let bytes = saved_bytes(&edited(true));
    let mut t = Session::new();
    let a = reopen(&mut t, &bytes);
    let b = reopen(&mut t, &bytes);
    let (ida, idb) = (t.documents()[a].doc.id, t.documents()[b].doc.id);
    assert_ne!(ida, idb, "the second copy gets its own identity");
    t.set_active(b);
    assert!(t.undo());
    assert_eq!(t.documents()[b].doc.id, idb, "an undone state takes the identity of its tab");
    assert_eq!(t.documents()[a].doc.id, ida);
}

#[test]
fn the_size_limit_comes_from_the_preferences() {
    let mut s = edited(true);
    s.execute("prefs.set", json!({"path": "performance.historyInDocumentMb", "value": 64})).unwrap();
    let doc = s.document_to_save(0).unwrap();
    let h = doc.saved_history.as_deref().unwrap();
    assert_eq!(h.max_bytes, 64 << 20);
    assert_eq!(h.states.len(), s.active().unwrap().history.past_len());
    // Out of range values are refused.
    assert!(s.execute("prefs.set", json!({"path": "performance.historyInDocumentMb", "value": 0})).is_err());
}

#[test]
fn reopening_keeps_at_most_the_history_states_preference() {
    let bytes = saved_bytes(&edited(true));
    let mut t = Session::new();
    t.execute("prefs.set", json!({"path": "performance.historyStates", "value": 1})).unwrap();
    reopen(&mut t, &bytes);
    let st = t.active().unwrap();
    assert_eq!(st.history.past_len(), 1, "only the newest state is kept");
    assert!(t.undo() && !t.undo(), "one step back, then no more");
}

#[test]
fn a_profile_conversion_on_open_drops_the_history() {
    // Converting to the working space on open changes every pixel: the saved states, in the
    // file's profile, no longer lead to this document.
    let mut s = Session::new();
    s.execute("prefs.set", json!({"path": "performance.saveHistoryInDocument", "value": true})).unwrap();
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    s.execute("edit.assignProfile", json!({"profile": "adobe-rgb-compat"})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let bytes = saved_bytes(&s);
    assert!(has_history_section(&bytes));

    let mut t = Session::new();
    t.execute("edit.colorSettings", json!({"policyRgb": "convert", "askOnMismatch": false})).unwrap();
    reopen(&mut t, &bytes);
    assert!(!t.active().unwrap().history.can_undo());

    let mut u = Session::new();
    u.execute("edit.colorSettings", json!({"policyRgb": "preserve", "askOnMismatch": false})).unwrap();
    reopen(&mut u, &bytes);
    assert!(u.active().unwrap().history.can_undo(), "a kept profile keeps the history");
}

#[test]
fn file_commands_restore_it_on_open_and_leave_it_behind_on_revert_and_place() {
    let dir = std::env::temp_dir().join(format!("photocraft-saved-history-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("h.pcraft");
    std::fs::write(&path, saved_bytes(&edited(true))).unwrap();
    let path = path.to_str().unwrap();

    let mut t = Session::new();
    t.execute("file.openAs", json!({"path": path})).unwrap();
    let past = t.active().unwrap().history.past_len();
    assert_eq!(past, 3, "File › Open brings the saved steps back");
    t.execute("file.revert", json!({})).unwrap();
    let st = t.active().unwrap();
    assert!(st.doc.saved_history.is_none(), "Revert reloads the pixels, not the saved history");
    assert_eq!(st.history.past_len(), past + 1, "Revert is one more step on this history");

    let layers = t.active().unwrap().doc.layers.len();
    t.execute("file.placeEmbedded", json!({"path": path})).unwrap();
    let st = t.active().unwrap();
    assert_eq!(st.doc.layers.len(), layers + 1);
    assert!(st.doc.saved_history.is_none(), "placing doesn't bring another file's history along");
    std::fs::remove_dir_all(&dir).unwrap();
}
