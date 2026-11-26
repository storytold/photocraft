use super::*;
use photocraft_io::NativeInitialView;

fn parent_session() -> (Session, DocId, LayerId, SmartContentsId) {
    let mut s = crate::session();
    s.execute("file.new", json!({"width":8,"height":6})).unwrap();
    let layer = LayerId(s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap());
    let state = s.active().unwrap();
    let parent = state.doc.id;
    let contents = smart(&state.doc, layer).unwrap().contents_id;
    (s, parent, layer, contents)
}

fn child_import(invalid: bool) -> (photocraft_io::ImportResult, LayerId, LayerId) {
    let mut child = Document::new("Source", photocraft_geom::Size::new(8, 6), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut first = photocraft_doc::Layer::raster("First source layer", photocraft_color::PixelFormat::RGBA8);
    first.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 8, 6), &[1., 0., 0., 1.]);
    let second = photocraft_doc::Layer::raster("Second source layer", photocraft_color::PixelFormat::RGBA8);
    let (first_id, second_id) = (first.id, second.id);
    child.layers.extend([first, second]);
    let view = NativeInitialView {
        selected_layers: vec![first_id, second_id],
        active_layer: Some(first_id),
        samplers: vec![[2., 3.]],
        selected_path: invalid.then_some(0),
        ..Default::default()
    };
    (
        photocraft_io::ImportResult {
            document: child,
            initial_view: Some(view),
            warnings: vec!["Source import note".into()],
            source_read_only: true,
            preview_only: false,
            historical_xcf_sample_order: None,
        },
        first_id,
        second_id,
    )
}

#[test]
fn prepared_contents_keeps_the_source_view_snapshot_and_warnings_without_retargeting_parent() {
    let (mut s, parent, layer, contents) = parent_session();
    let before = s.active().unwrap().clone();
    let parent_snapshot = before.snapshots()[0].layers.clone();
    let (imported, first, second) = child_import(false);
    let result = admit_contents_import(&mut s, parent, layer, contents, "source.pcraft".into(), imported).unwrap();
    let index = result["document"].as_u64().unwrap() as usize;
    assert_eq!(result["warnings"], json!(["Source import note"]));
    let state = &s.documents()[index];
    assert_eq!(state.active_layer(), Some(first));
    assert_eq!(state.selection(), [first, second]);
    assert_eq!(state.color_samplers, [[2., 3.]]);
    assert_eq!(state.snapshots()[0].layers.active, Some(first));
    assert_eq!(state.snapshots()[0].layers.selected, [first, second]);
    assert_eq!((state.revision, state.saved_revision, state.history.past_len()), (1, 1, 0));
    assert_eq!(state.doc.layers[0].surface().unwrap().pixel(0, 0), vec![1., 0., 0., 1.]);
    assert!(state.source_read_only);
    assert!(state.protected_import_source.is_none(), "embedded contents grant no ambient source path");
    assert!(Arc::ptr_eq(&s.documents()[0].doc, &before.doc));
    assert_eq!(s.documents()[0].snapshots()[0].layers, parent_snapshot);
    assert!(s.smart_links.iter().any(|link| link.parent == parent && link.child == state.doc.id && link.contents_id == contents));
    // Re-entering an already-open child preserves the user's subsequent selection.
    s.select_layer(second).unwrap();
    s.set_active(0);
    let again = s.execute("layer.smartObjects.editContents", json!({"layer":layer.0})).unwrap();
    assert_eq!(again["document"], index);
    assert_eq!(s.active().unwrap().active_layer(), Some(second));
    assert_eq!(s.active().unwrap().snapshots()[0].layers.active, Some(first));
}

#[test]
fn invalid_contents_hint_keeps_the_child_default_view_and_reports_the_difference() {
    let (mut s, parent, layer, contents) = parent_session();
    let before = s.active().unwrap().doc.clone();
    let (imported, _, second) = child_import(true);
    let result = admit_contents_import(&mut s, parent, layer, contents, "source.pcraft".into(), imported).unwrap();
    let warnings = result["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 2);
    assert_eq!(warnings[0], "Source import note");
    assert!(warnings[1].as_str().unwrap().contains("selected path is absent"));
    let child = s.active().unwrap();
    assert_eq!(child.active_layer(), Some(second));
    assert_eq!(child.selection(), [second]);
    assert!(child.color_samplers.is_empty());
    assert_eq!(child.snapshots()[0].layers.active, Some(second));
    assert_eq!(child.doc.layers[0].surface().unwrap().pixel(0, 0), vec![1., 0., 0., 1.]);
    assert!(Arc::ptr_eq(&s.documents()[0].doc, &before));
}
