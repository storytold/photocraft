//! Save History in Document (#6053) through the headless server: `doc.save` writes the undo
//! history into a `.pcraft` file when the preference is on, and `doc.open` brings it back.

use photocraft_automation::Headless;
use serde_json::json;

fn temp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pc-saved-history-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn edited(save_history: bool) -> Headless {
    let mut h = Headless::trusted_local();
    h.handle("engine.execute", json!({"command": "prefs.set", "params": {"path": "performance.saveHistoryInDocument", "value": save_history}})).unwrap();
    h.handle("doc.new", json!({"width": 32, "height": 32})).unwrap();
    h.handle("engine.execute", json!({"command": "layer.new.layer", "params": {}})).unwrap();
    h.handle("engine.execute", json!({"command": "edit.fill", "params": {"color": "#00ff00"}})).unwrap();
    h
}

fn has_history(path: &std::path::Path) -> bool {
    let bytes = std::fs::read(path).unwrap();
    photocraft_format::zip::ZipReader::new(&bytes).unwrap().find(photocraft_format::saved_history::HISTORY).is_some()
}

#[test]
fn saving_with_the_preference_on_keeps_the_steps_for_the_next_session() {
    let dir = temp("on");
    let file = dir.join("steps.pcraft");
    let mut h = edited(true);
    h.handle("doc.save", json!({"path": file.to_str().unwrap()})).unwrap();
    assert!(has_history(&file));

    let mut next = Headless::trusted_local();
    next.handle("doc.open", json!({"path": file.to_str().unwrap()})).unwrap();
    assert_eq!(next.session.active().unwrap().history.past_len(), 2);
    assert!(next.session.undo(), "the fill can be undone after reopening");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn saving_with_the_preference_off_writes_no_history() {
    let dir = temp("off");
    let file = dir.join("steps.pcraft");
    let mut h = edited(false);
    h.handle("doc.save", json!({"path": file.to_str().unwrap()})).unwrap();
    assert!(!has_history(&file));
    std::fs::remove_dir_all(dir).unwrap();
}
