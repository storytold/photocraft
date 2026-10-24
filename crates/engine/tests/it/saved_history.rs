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
    assert_eq!(s.prefs().performance.history_in_document_mb, 64, "a refused value leaves the preference alone");
    assert_eq!(s.document_to_save(0).unwrap().saved_history.as_deref().unwrap().max_bytes, 64 << 20);
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

/// The `.pcraft` bytes the smart object layer `layer` of `s`'s active document embeds.
fn embedded_bytes(s: &Session, layer: u64) -> Vec<u8> {
    let doc = &s.active().unwrap().doc;
    let layer = doc.layers.iter().find(|l| l.id.0 == layer).unwrap();
    match &layer.content {
        photocraft_doc::LayerContent::Smart(so) => match &so.source {
            photocraft_doc::SmartSource::Embedded { bytes, .. } => bytes.to_vec(),
            other => panic!("not embedded: {other:?}"),
        },
        _ => panic!("not a smart object"),
    }
}

#[test]
fn placing_a_pcraft_with_history_embeds_it_without_the_history() {
    // The placed file travels inside the parent document: its earlier pixel states must not.
    let dir = std::env::temp_dir().join(format!("photocraft-placed-history-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let with = saved_bytes(&edited(true));
    assert!(has_history_section(&with));
    let path = dir.join("placed.pcraft");
    std::fs::write(&path, &with).unwrap();

    let mut t = Session::new();
    t.execute("file.new", json!({"width": 300, "height": 300})).unwrap();
    let placed = t.execute("file.placeEmbedded", json!({"path": path.to_str().unwrap()})).unwrap();
    let embedded = embedded_bytes(&t, placed["layer"].as_u64().unwrap());
    assert!(photocraft_format::is_pcraft(&embedded), "still a .pcraft file");
    assert!(!has_history_section(&embedded), "the saved history stays behind");

    let (original, kept) = (photocraft_format::load_from_bytes(&with).unwrap(), photocraft_format::load_from_bytes(&embedded).unwrap());
    assert!(kept.saved_history.is_none());
    assert_eq!(kept.size, original.size);
    assert_eq!(kept.layers, original.layers, "the placed contents are unchanged");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn placing_a_pcraft_without_history_embeds_its_bytes_as_they_are() {
    let dir = std::env::temp_dir().join(format!("photocraft-placed-plain-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let plain = saved_bytes(&edited(false));
    let path = dir.join("plain.pcraft");
    std::fs::write(&path, &plain).unwrap();

    let mut t = Session::new();
    t.execute("file.new", json!({"width": 300, "height": 300})).unwrap();
    let placed = t.execute("file.placeEmbedded", json!({"path": path.to_str().unwrap()})).unwrap();
    assert_eq!(embedded_bytes(&t, placed["layer"].as_u64().unwrap()), plain);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn save_a_copy_as_pcraft_follows_the_preference() {
    let dir = std::env::temp_dir().join(format!("photocraft-copy-history-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for on in [true, false] {
        let mut s = edited(on);
        let path = dir.join(format!("copy-{on}.pcraft"));
        s.execute("file.saveACopy", json!({"path": path.to_str().unwrap()})).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(has_history_section(&bytes), on, "Save History in Document {on}");
        let mut t = Session::new();
        reopen(&mut t, &bytes);
        assert_eq!(t.active().unwrap().history.can_undo(), on);
        assert!(s.active().unwrap().doc.saved_history.is_none(), "the open document itself carries no history");
    }
    // A flattened copy is not the document the history leads to.
    let mut s = edited(true);
    let path = dir.join("flat.pcraft");
    s.execute("file.saveACopy", json!({"path": path.to_str().unwrap(), "layers": false})).unwrap();
    assert!(!has_history_section(&std::fs::read(&path).unwrap()));
    std::fs::remove_dir_all(&dir).unwrap();
}
