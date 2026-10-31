//! File workflows capture live layer targeting at the same point as the document pixels.

use std::path::Path;

use photocraft_doc::{ColorMode, Document, Layer, LayerId, SampleType};
use photocraft_engine::Session;
use photocraft_geom::Size;
use serde_json::json;

/// Create three painted layers so workflow tests persist both pixels and a non-top target.
fn scene() -> Session {
    let mut doc = Document::new("source", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    for name in ["Bottom", "Middle", "Top"] {
        let mut layer = Layer::raster(name, doc.pixel_format());
        layer.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.2, 0.4, 0.6, 1.0]);
        doc.layers.push(layer);
    }
    let mut session = Session::new();
    session.add_document(doc, None);
    session
}

/// Look up a named fixture layer without assuming IDs survive a format roundtrip.
fn id(session: &Session, name: &str) -> LayerId {
    session.active().unwrap().doc.walk().into_iter().find(|(_, _, layer)| layer.name == name).unwrap().2.id
}

/// Describe effective targeting by stable names rather than file-specific layer IDs.
fn targets(session: &Session) -> (Option<String>, Vec<String>) {
    let state = session.active().unwrap();
    let name = |id| state.doc.layer(id).unwrap().name.clone();
    (state.active_layer.map(name), state.selected_layers().into_iter().map(name).collect())
}

/// Select Bottom and Middle with Middle primary and Bottom as a distinct runtime range anchor.
fn multiple(session: &mut Session) {
    let (bottom, middle) = (id(session, "Bottom"), id(session, "Middle"));
    session.restore_layer_selection(Some(middle), &[bottom, middle], Some(bottom)).unwrap();
}

/// Import a written file into a fresh session, including constructor normalization of targeting.
fn open(path: &Path) -> Session {
    let bytes = std::fs::read(path).unwrap();
    let doc = photocraft_io::import(path.to_str().unwrap(), &bytes).unwrap().document;
    let mut session = Session::new();
    session.add_document(doc, Some(path.to_str().unwrap().into()));
    session
}

/// Revert is one clean history step whose Undo/Redo restore the complete old/saved selected sets.
#[test]
fn revert_restores_saved_targeting_and_undo_redo_keep_both_targets() {
    let dir = tempfile::tempdir().unwrap();
    for ext in ["psd", "psb", "pcraft"] {
        for deselected in [false, true] {
            let mut original = scene();
            multiple(&mut original);
            if deselected {
                original.execute("select.deselectLayers", json!({})).unwrap();
            }
            let saved = targets(&original);
            let path = dir.path().join(format!("revert-{deselected}.{ext}"));
            original.execute("file.saveACopy", json!({"path": path})).unwrap();

            let mut session = open(&path);
            session.select_layer(id(&session, "Top")).unwrap();
            let before = targets(&session);
            let history = session.active().unwrap().history.past_len();
            session.execute("file.revert", json!({})).unwrap();
            assert_eq!(targets(&session), saved, "{ext}, deselected={deselected}");
            assert!(!session.active().unwrap().is_dirty());
            assert!(session.active().unwrap().doc.saved_layer_selection.is_none(), "Revert consumes transport state");
            assert_eq!(session.active().unwrap().history.past_len(), history + 1);

            assert!(session.undo());
            assert_eq!(targets(&session), before, "Undo restores the pre-Revert selection");
            assert!(session.redo());
            assert_eq!(targets(&session), saved, "Redo restores the whole imported selection");
        }
    }
}

/// Deferred saves use their captured selection after Close or a document switch; ordinary batch
/// completion captures the final active document's targeting when there is no earlier save point.
#[test]
fn batch_keeps_targeting_from_a_save_followed_by_close_or_document_switch() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.pcraft");
    scene().execute("file.saveACopy", json!({"path": input})).unwrap();
    for ext in ["psd", "pcraft"] {
        for ending in ["close", "switch", "final"] {
            let output = dir.path().join(format!("{ending}-{ext}"));
            let mut steps = vec![json!(["layer.select", {"name": "Bottom"}]), json!(["layer.select", {"name": "Middle", "mode": "add"}])];
            if ending != "final" {
                steps.push(json!(["file.saveAs", {}]));
                // These later view changes must not rewrite the earlier saved target.
                steps.push(json!(["layer.select", {"name": "Top"}]));
                steps.push(match ending {
                    "close" => json!(["file.close", {}]),
                    _ => json!(["file.new", {"name": "Other", "width": 1, "height": 1}]),
                });
            }
            let result = Session::new().execute("file.automate.batch", json!({"input": [input], "output": output, "format": ext, "steps": steps})).unwrap();
            assert_eq!(result["errors"], json!([]), "{ending}, {ext}: {result}");
            let path = output.join(format!("source.{ext}"));
            assert_eq!(result["files"], json!([path]));
            assert_eq!(targets(&open(&path)), (Some("Middle".into()), vec!["Bottom".into(), "Middle".into()]), "{ending}, {ext}");
        }
    }
}

/// Save Contents embeds the child target in native bytes, proven by reopening through a fresh
/// parent session rather than reusing the already-open child editing state.
#[test]
fn smart_contents_commit_captures_child_selection_for_the_next_open() {
    let mut session = scene();
    session.execute("select.allLayers", json!({})).unwrap();
    let converted = session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let smart = converted["layer"].as_u64().unwrap();
    session.execute("layer.smartObjects.editContents", json!({"layer": smart})).unwrap();
    multiple(&mut session);
    let expected = targets(&session);
    assert_eq!(expected, (Some("Middle".into()), vec!["Bottom".into(), "Middle".into()]));
    let result = session.execute("layer.smartObjects.saveContents", json!({})).unwrap();
    assert_eq!(result["updated"], true);
    assert!(!session.active().unwrap().is_dirty());
    session.execute("file.close", json!({})).unwrap();
    assert_eq!(session.documents().len(), 1);

    // A fresh parent session forces the result to travel through the embedded native file,
    // rather than switching back to the already-open child.
    let bytes = photocraft_format::save_to_bytes(&session.active().unwrap().save_snapshot(), &Default::default()).unwrap();
    let parent = photocraft_format::load_from_bytes(&bytes).unwrap();
    let mut reopened = Session::new();
    reopened.add_document(parent, None);
    reopened.execute("layer.smartObjects.editContents", json!({})).unwrap();
    assert_eq!(targets(&reopened), expected);
    assert!(reopened.active().unwrap().doc.saved_layer_selection.is_none());
}

/// Flattened copies target their replacement Background and leave the source selection untouched.
#[test]
fn flattened_layered_copies_do_not_keep_source_layer_ids() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = scene();
    multiple(&mut session);
    let before = targets(&session);
    for ext in ["psd", "pcraft"] {
        let path = dir.path().join(format!("flattened.{ext}"));
        session.execute("file.saveACopy", json!({"path": path, "layers": false})).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let imported = photocraft_io::import(path.to_str().unwrap(), &bytes).unwrap().document;
        assert_eq!(imported.layers.len(), 1);
        assert_eq!(imported.layers.first().unwrap().name, "Background");
        if let Some(selection) = &imported.saved_layer_selection {
            assert!(selection.active.into_iter().chain(selection.selected.iter().copied()).all(|id| imported.layer(id).is_some()));
        }
        assert_eq!(targets(&open(&path)), (Some("Background".into()), vec!["Background".into()]));
        assert_eq!(targets(&session), before, "writing a flattened copy leaves live targeting intact");
    }
}
