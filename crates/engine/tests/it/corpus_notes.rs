//! Corpus test (feature `corpus`; run with `cargo xtask test-corpus`): `file.import.notes` reads the
//! annotations of a real PSD.
#![cfg(feature = "corpus")]

use photocraft_engine::Session;
use serde_json::json;

#[test]
fn import_notes_from_a_psd() {
    let psd = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/psd/ag-psd/read-write/annotations/src.psd");
    assert!(std::path::Path::new(psd).exists(), "{psd} is missing: run `cargo xtask corpus --all`");
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 80})).unwrap();
    s.execute("file.import.notes", json!({"path": psd})).unwrap();
    assert_eq!(s.active().unwrap().doc.notes.len(), 2);
}
