//! #10195: every automation save transports the current layer targeting, including inactive files.

use photocraft_automation::{AuthorizedWorkspace, Headless, files};
use photocraft_engine::Session;
use serde_json::json;

fn target(s: &Session) -> (Option<String>, Vec<String>) {
    let st = s.active().unwrap();
    let active = st.active_layer.and_then(|id| st.doc.layer(id)).map(|l| l.name.clone());
    let mut selected: Vec<_> = st.selected_layers().iter().map(|id| st.doc.layer(*id).unwrap().name.clone()).collect();
    selected.sort();
    (active, selected)
}

fn assert_saved(path: &std::path::Path, expected: &(Option<String>, Vec<String>)) {
    let mut reopened = Session::new();
    reopened.add_document(files::open(path).unwrap().document, None);
    assert_eq!(&target(&reopened), expected, "{}", path.display());
    assert!(reopened.active().unwrap().doc.saved_layer_selection.is_none(), "file targeting is consumed on admission");
}

#[test]
fn trusted_and_capability_saves_restore_current_non_top_multi_and_empty_layer_targets() {
    let dir = std::env::temp_dir().join(format!("photocraft-saved-layer-selection-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for scoped in [false, true] {
        for depth in [8, 16, 32] {
            for ext in ["psd", "psb", "pcraft"] {
                let mut h =
                    if scoped { Headless::with_workspace(AuthorizedWorkspace::new(Some(&dir), Some(&dir)).unwrap()) } else { Headless::trusted_local() };
                h.command_run("file.new", json!({"width": 2, "height": 1, "depth": depth, "background": "white"})).unwrap();
                h.command_run("layer.new.layer", json!({"name": "Middle"})).unwrap();
                h.command_run("layer.new.layer", json!({"name": "Top"})).unwrap();
                h.command_run("layer.select", json!({"name": "Middle"})).unwrap();
                let name = format!("{scoped}-{depth}.{ext}");
                let path = dir.join(&name);
                let save_path = if scoped { std::path::Path::new(&name) } else { path.as_path() };
                let before = target(&h.session);
                h.save(None, Some(save_path), None, &Default::default()).unwrap();
                assert_saved(&path, &before);
                assert!(!h.session.active().unwrap().is_dirty());

                // Selecting another layer stays clean, but an explicit pathless Save writes it.
                h.command_run("layer.select", json!({"name": "Background", "mode": "add"})).unwrap();
                assert!(!h.session.active().unwrap().is_dirty());
                let before = target(&h.session);
                h.save(None, None, None, &Default::default()).unwrap();
                assert_saved(&path, &before);

                h.command_run("select.deselectLayers", json!({})).unwrap();
                h.save(None, None, None, &Default::default()).unwrap();
                assert_saved(&path, &(None, Vec::new()));
                assert_eq!(target(&h.session), (None, Vec::new()));
                assert!(h.session.active().unwrap().doc.saved_layer_selection.is_none());
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn indexed_saves_take_the_inactive_documents_target_and_failure_preserves_it() {
    let dir = std::env::temp_dir().join(format!("photocraft-inactive-layer-selection-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = Headless::trusted_local();
    h.command_run("file.new", json!({"width": 2, "height": 1, "background": "white"})).unwrap();
    h.command_run("layer.new.layer", json!({"name": "First middle"})).unwrap();
    h.command_run("layer.new.layer", json!({"name": "First top"})).unwrap();
    h.command_run("layer.select", json!({"name": "First middle"})).unwrap();
    let first = target(&h.session);
    h.command_run("file.new", json!({"width": 2, "height": 1, "name": "Second"})).unwrap();
    let second = target(&h.session);
    let second_id = h.session.active().unwrap().doc.id;
    for ext in ["psd", "psb", "pcraft"] {
        let path = dir.join(format!("first.{ext}"));
        h.save(Some(0), Some(&path), None, &Default::default()).unwrap();
        assert_saved(&path, &first);
        assert_eq!(h.session.active().unwrap().doc.id, second_id);
        assert_eq!(target(&h.session), second);
    }
    let before = h.session.documents()[0].clone();
    let blocked = dir.join("blocked");
    std::fs::write(&blocked, b"file, not a directory").unwrap();
    assert!(h.save(Some(0), Some(&blocked.join("failed.psd")), None, &Default::default()).is_err());
    let after = &h.session.documents()[0];
    assert_eq!(
        (after.active_layer, &after.selected_layers, &after.path, after.saved_revision),
        (before.active_layer, &before.selected_layers, &before.path, before.saved_revision)
    );
    assert!(std::sync::Arc::ptr_eq(&before.doc, &after.doc), "a failed save never replaces the live document with its snapshot");
    assert_eq!(h.session.active().unwrap().doc.id, second_id);
    drop(h);
    std::fs::remove_dir_all(dir).unwrap();
}
