//! Saved layer targeting survives file I/O and session admission (#10195).

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerId};
use photocraft_engine::Session;
use photocraft_geom::Size;
use photocraft_raster::Surface;
use serde_json::json;

/// Admit five named layers at the requested pixel format with the ordinary top-layer default.
fn scene(mode: ColorMode, depth: SampleType) -> Session {
    let mut doc = Document::new("Layers", Size::new(2, 2), mode, depth);
    for i in 1..=5 {
        doc.layers.push(Layer::new(format!("Layer {i}"), LayerContent::Raster(Surface::new(doc.pixel_format()))));
    }
    let mut session = Session::new();
    session.add_document(doc, None);
    session
}

/// Resolve a fixture name after import, where runtime IDs may differ from the saved document.
fn id(session: &Session, name: &str) -> LayerId {
    session.active().unwrap().doc.walk().into_iter().find(|(_, _, layer)| layer.name == name).unwrap().2.id
}

/// Compare targeting by stable fixture names, using the public effective selected-layer set.
fn target_names(session: &Session) -> (Option<String>, Vec<String>) {
    let state = session.active().unwrap();
    let name = |id| state.doc.layer(id).unwrap().name.clone();
    (state.active_layer.map(name), state.selected_layers().into_iter().map(name).collect())
}

/// Exercise both format import and session admission, which consumes the targeting transport field.
fn reopen(path: &std::path::Path) -> Session {
    let bytes = std::fs::read(path).unwrap();
    let imported = photocraft_io::import(path.to_str().unwrap(), &bytes).unwrap();
    let mut session = Session::new();
    session.add_document(imported.document, Some(path.to_str().unwrap().into()));
    session
}

/// A middle-layer selection survives the public Save a Copy path across PSD/PSB/native formats,
/// representative color models and 8-bit, 16-bit and 32-bit float document samples.
#[test]
fn third_layer_in_saved_copy_reopens_selected_at_every_depth() {
    let dir = tempfile::tempdir().unwrap();
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut session = scene(mode, depth);
            session.select_layer(id(&session, "Layer 3")).unwrap();
            for ext in ["psd", "psb", "pcraft"] {
                let path = dir.path().join(format!("selected.{ext}"));
                session.execute("file.saveACopy", json!({"path": path})).unwrap();
                assert_eq!(target_names(&reopen(&path)), (Some("Layer 3".into()), vec!["Layer 3".into()]), "{ext} {mode:?} {depth:?}");
            }
        }
    }
}

/// Explicit saves capture view-only changes while preserving the live document Arc and history.
#[test]
fn repeated_save_captures_the_latest_selection_without_an_edit() {
    let dir = tempfile::tempdir().unwrap();
    for ext in ["psd", "psb", "pcraft"] {
        let mut session = scene(ColorMode::Rgb, SampleType::U8);
        let original = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.can_undo();
        let path = dir.path().join(format!("repeat.{ext}"));
        for name in ["Layer 3", "Layer 1"] {
            session.select_layer(id(&session, name)).unwrap();
            assert!(!session.active().unwrap().is_dirty());
            session.execute("file.saveACopy", json!({"path": path})).unwrap();
            assert_eq!(target_names(&reopen(&path)), (Some(name.into()), vec![name.into()]));
            assert!(std::sync::Arc::ptr_eq(&original, &session.active().unwrap().doc));
            assert_eq!(session.active().unwrap().history.can_undo(), history);
        }
    }
}

/// Admission initializes the primary, selected set, anchor and history together; an empty set
/// remains deselected instead of being replaced by the ordinary top-layer default.
#[test]
fn multiple_selected_layers_and_explicit_deselection_reopen() {
    let dir = tempfile::tempdir().unwrap();
    for ext in ["psd", "psb", "pcraft"] {
        let mut session = scene(ColorMode::Rgb, SampleType::U8);
        let (one, three, five) = (id(&session, "Layer 1"), id(&session, "Layer 3"), id(&session, "Layer 5"));
        session.restore_layer_selection(Some(three), &[one, three, five], Some(one)).unwrap();
        let path = dir.path().join(format!("multiple.{ext}"));
        session.execute("file.saveACopy", json!({"path": path})).unwrap();
        let mut reopened = reopen(&path);
        assert_eq!(target_names(&reopened), (Some("Layer 3".into()), vec!["Layer 1".into(), "Layer 3".into(), "Layer 5".into()]));
        let state = reopened.active().unwrap();
        assert_eq!(state.layer_anchor, state.active_layer);
        assert_eq!(state.snapshots[0].layers.active, state.active_layer);
        assert_eq!(state.snapshots[0].layers.selected, state.selected_layers);
        assert!(state.doc.saved_layer_selection.is_none());
        let before_edit = target_names(&reopened);
        reopened.execute("layer.setProps", json!({"name": "Edited"})).unwrap();
        assert!(reopened.undo());
        assert_eq!(target_names(&reopened), before_edit, "the first edit's undo retains imported history targeting");

        session.restore_layer_selection(None, &[], None).unwrap();
        session.execute("file.saveACopy", json!({"path": path})).unwrap();
        assert_eq!(target_names(&reopen(&path)), (None, vec![]));
        assert!(!session.active().unwrap().is_dirty());
    }
}

/// Runtime targets are restored for nested children, their containing group and a background layer.
#[test]
fn nested_child_group_and_background_targeting_survive_session_admission() {
    let dir = tempfile::tempdir().unwrap();
    let mut doc = Document::with_background("Nested", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, photocraft_doc::Color::WHITE);
    let child = Layer::raster("Nested child", doc.pixel_format());
    doc.layers.push(Layer::group("Group", vec![child]));
    doc.layers.push(Layer::raster("Top", doc.pixel_format()));
    let mut session = Session::new();
    session.add_document(doc, None);
    for name in ["Nested child", "Group", "Background"] {
        session.select_layer(id(&session, name)).unwrap();
        for ext in ["psd", "psb", "pcraft"] {
            let path = dir.path().join(format!("nested.{ext}"));
            session.execute("file.saveACopy", json!({"path": path})).unwrap();
            assert_eq!(target_names(&reopen(&path)), (Some(name.into()), vec![name.into()]), "{name}, {ext}");
        }
    }
}
