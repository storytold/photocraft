use super::*;
use photocraft_doc::Document;
use photocraft_io::{NativeImportTarget, NativeInitialView};
use serde_json::json;

#[test]
fn selected_layers_without_active_layer_keep_the_imported_default_view_and_warn() {
    let mut session = crate::session();
    session.execute("file.new", json!({"width":8,"height":6})).unwrap();
    session.execute("layer.new.layer", json!({"name":"existing edit"})).unwrap();
    let previous = session.active().unwrap().clone();
    let previous_save = photocraft_format::save_to_bytes(&previous.doc, &Default::default()).unwrap();
    let mut imported = Document::new("retained", photocraft_geom::Size::new(8, 6), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    imported.layers.extend(["first", "last"].map(|name| photocraft_doc::Layer::raster(name, photocraft_color::PixelFormat::RGBA8)));
    let selected = imported.layers[0].id;
    let mut control = crate::session();
    control.add_document(imported.clone(), None);
    let default_view = control.active().unwrap();
    let (index, warning) = session.admit_import_with_initial_view(
        imported,
        None,
        true,
        Some(NativeInitialView { selected_layers: vec![selected], samplers: vec![[2., 3.]], ..Default::default() }),
    );
    assert!(warning.unwrap().contains("selected layers require an active layer"));
    assert_eq!(session.documents().len(), 2);
    assert_eq!(session.active_index(), Some(index));
    let state = session.active().unwrap();
    assert_eq!(state.doc, default_view.doc);
    assert!(Arc::ptr_eq(&state.doc, &state.snapshots().first().unwrap().doc));
    assert!(state.source_read_only);
    assert_eq!(state.active_layer, default_view.active_layer);
    assert!(state.active_layer.is_some());
    assert_eq!(state.selected_layers, default_view.selected_layers);
    assert_eq!(state.layer_anchor, default_view.layer_anchor);
    assert_eq!(state.channel_view, default_view.channel_view);
    assert_eq!(state.color_samplers, default_view.color_samplers);
    assert_eq!((state.revision, state.saved_revision, state.history.past_len()), (default_view.revision, default_view.saved_revision, 0));
    assert_eq!(state.snapshots().first().unwrap().layers, default_view.snapshots().first().unwrap().layers);
    let saved = photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap();
    let reopened = photocraft_io::import("retained.pcraft", &saved).unwrap();
    assert!(reopened.initial_view.is_none());
    assert_eq!(reopened.document.layer_count(), 2);
    let existing = &session.documents()[0];
    assert!(Arc::ptr_eq(&existing.doc, &previous.doc));
    assert_eq!(existing.active_layer, previous.active_layer);
    assert_eq!(existing.selected_layers, previous.selected_layers);
    assert_eq!(existing.layer_anchor, previous.layer_anchor);
    assert_eq!(existing.path, previous.path);
    assert_eq!(existing.source_read_only, previous.source_read_only);
    assert_eq!(
        (existing.revision, existing.saved_revision, existing.history.past_len()),
        (previous.revision, previous.saved_revision, previous.history.past_len())
    );
    assert_eq!(photocraft_format::save_to_bytes(&existing.doc, &Default::default()).unwrap(), previous_save);
}

#[test]
fn invalid_optional_view_keeps_admitted_document_and_existing_documents_unchanged() {
    for (existing, active) in [(0, None), (3, None), (3, Some(0)), (3, Some(1))] {
        let mut session = crate::session();
        for index in 0..existing {
            session.execute("file.new", json!({"width": 8, "height": 6, "name": format!("existing {index}")})).unwrap();
            session.execute("layer.layerMask.revealAll", json!({})).unwrap();
            let layer = session.active().unwrap().active_layer.unwrap();
            session
                .apply_import_initial_view(Some(NativeInitialView {
                    selected_layers: vec![layer],
                    active_layer: Some(layer),
                    target: NativeImportTarget::LayerMask,
                    ..Default::default()
                }))
                .unwrap();
        }
        session.active = active;
        session.note_active_change();
        session.sync_tools_mask_target();
        let before = session.documents().to_vec();
        let order = session.open_document_order.clone();
        let mut imported = Document::new("retained", photocraft_geom::Size::new(8, 6), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        imported.layers.extend(["first", "last"].map(|name| photocraft_doc::Layer::raster(name, photocraft_color::PixelFormat::RGBA8)));
        let default_layer = imported.top_layer();
        let hinted_layer = imported.layers.iter().find(|layer| Some(layer.id) != default_layer).unwrap().id;
        imported.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 8, 6), &[0.2, 0.4, 0.6, 1.]);
        let expected_pixels = imported.layers[0].surface().unwrap().clone();
        if let Some(state) = before.first() {
            imported.id = state.doc.id; // Admission must remap this collision without changing the prior identity.
        }
        let (index, warning) = session.admit_import_with_initial_view(
            imported,
            None,
            true,
            Some(NativeInitialView {
                selected_layers: vec![hinted_layer],
                active_layer: Some(hinted_layer),
                samplers: vec![[2., 3.]],
                selected_path: Some(0),
                ..Default::default()
            }),
        );
        assert!(warning.unwrap().contains("selected path is absent"));
        assert_eq!(session.active_index(), Some(index));
        assert_eq!(session.documents().len(), before.len() + 1);
        assert_eq!(&session.open_document_order[..order.len()], order.as_slice());
        let admitted = session.active().unwrap();
        assert!(admitted.source_read_only);
        assert!(admitted.path.is_none());
        assert_eq!(admitted.active_layer, default_layer);
        assert_eq!(admitted.selected_layers, default_layer.into_iter().collect::<Vec<_>>());
        assert!(admitted.color_samplers.is_empty());
        assert!(admitted.initial_path_selection.is_none());
        assert_eq!(admitted.channel_view.target, crate::channel_cmds::ChannelTarget::Composite);
        assert!(!session.tools.mask_targeted);
        assert_eq!(admitted.history.past_len(), 0);
        assert_eq!(admitted.snapshots().first().unwrap().layers.active, default_layer);
        assert_eq!(admitted.snapshots().first().unwrap().layers.selected, admitted.selected_layers);
        assert_eq!(admitted.doc.layers[0].surface().unwrap().pixel(0, 0), expected_pixels.pixel(0, 0));
        for (state, previous) in session.documents().iter().zip(before) {
            assert!(Arc::ptr_eq(&state.doc, &previous.doc));
            assert_eq!(state.path, previous.path);
            assert_eq!(state.source_read_only, previous.source_read_only);
            assert_eq!(state.active_layer, previous.active_layer);
            assert_eq!(state.selected_layers, previous.selected_layers);
            assert_eq!(state.channel_view, previous.channel_view);
            assert_eq!(state.color_samplers, previous.color_samplers);
            assert_eq!(
                (state.revision, state.saved_revision, state.history.past_len()),
                (previous.revision, previous.saved_revision, previous.history.past_len())
            );
        }
    }
}

#[test]
fn import_view_is_validated_against_the_admitted_document_and_warns_on_late_rejection() {
    let mut session = crate::session();
    session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    let original = session.active().unwrap().doc.clone();
    let mut imported = Document::new("pending", photocraft_geom::Size::new(8, 6), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    imported.paths.push(photocraft_doc::vector::NamedPath { name: "Selected saved path".into(), ..Default::default() });
    let view = NativeInitialView { selected_path: Some(0), ..Default::default() };
    view.validate(&imported).unwrap();
    // Preparation may change the pending document after a decoder-time check.
    imported.paths.clear();
    let (index, warning) = session.admit_import_with_initial_view(imported, None, true, Some(view));
    assert!(warning.unwrap().contains("selected path is absent"));
    assert_eq!(session.documents().len(), 2);
    assert_eq!(session.active_index(), Some(index));
    assert!(session.active().unwrap().initial_path_selection.is_none());
    assert!(Arc::ptr_eq(&session.documents()[0].doc, &original));
}

#[test]
fn changed_selected_path_is_omitted_while_other_view_hints_survive() {
    let mut session = crate::session();
    let mut imported =
        photocraft_doc::Document::new("Paths", photocraft_geom::Size::new(8, 8), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    for name in ["A", "B", "C"] {
        imported.paths.push(photocraft_doc::vector::NamedPath { name: name.into(), ..Default::default() });
    }
    let view = NativeInitialView {
        selected_path: Some(1),
        selected_path_identity: Some(photocraft_io::initial_view::path_identity(&imported.paths[1])),
        samplers: vec![[1., 1.]],
        ..Default::default()
    };
    imported.paths.remove(0);
    let (_, warning) = session.admit_import_with_initial_view(imported, None, false, Some(view));
    assert!(warning.unwrap().contains("changed during admission"));
    let state = session.active().unwrap();
    assert_eq!(state.initial_path_selection, None);
    assert_eq!(state.color_samplers, vec![[1., 1.]]);
}

#[test]
fn imported_mask_and_channel_views_target_existing_native_items_without_edits() {
    let mut session = crate::session();
    session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    session.execute("layer.layerMask.revealAll", json!({})).unwrap();
    session.execute("channel.new", json!({})).unwrap();
    session.execute("channel.new", json!({})).unwrap();
    let state = session.active().unwrap();
    let layer = state.active_layer.unwrap();
    let before = (state.revision, state.history.past_len(), state.doc.clone());
    session
        .apply_import_initial_view(Some(NativeInitialView {
            selected_layers: vec![layer],
            active_layer: Some(layer),
            target: NativeImportTarget::LayerMask,
            show_layer_mask: true,
            visible_alpha_channels: vec![1],
            samplers: vec![[2.5, 3.5]],
            ..Default::default()
        }))
        .unwrap();
    let state = session.active().unwrap();
    assert_eq!(state.channel_view.target, crate::channel_cmds::ChannelTarget::LayerMask);
    assert_eq!(state.channel_view.layer_mask.as_ref().unwrap().layer, layer);
    assert!(state.channel_view.alpha_shown(1));
    assert!(session.tools.mask_targeted);
    assert_eq!((state.revision, state.history.past_len()), (before.0, before.1));
    assert!(Arc::ptr_eq(&state.doc, &before.2));
    session.apply_import_initial_view(Some(NativeInitialView { target: NativeImportTarget::AlphaChannel(0), ..Default::default() })).unwrap();
    assert_eq!(session.active().unwrap().channel_view.target, crate::channel_cmds::ChannelTarget::Alpha(0));
    assert_eq!(session.active().unwrap().active_layer, Some(layer), "an empty layer hint preserves the current layer target");
    assert_eq!(session.active().unwrap().selected_layers, vec![layer]);
    assert!(!session.tools.mask_targeted);
}

#[test]
fn initial_view_does_not_retarget_an_older_snapshot() {
    let mut session = crate::session();
    session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    let original = session.active().unwrap().active_layer.unwrap();
    session.execute("layer.new.layer", json!({"name": "later"})).unwrap();
    let later = session.active().unwrap().active_layer.unwrap();
    session.apply_import_initial_view(Some(NativeInitialView { selected_layers: vec![later], active_layer: Some(later), ..Default::default() })).unwrap();
    session.execute("history.selectSnapshot", json!({"snapshot": 0})).unwrap();
    let state = session.active().unwrap();
    assert_eq!(state.active_layer, Some(original));
    assert_eq!(state.selected_layers, vec![original]);
    assert!(state.doc.layer(later).is_none());
}

#[test]
/// Apply imported view hints without history or revision changes and verify native reopen does not replay them.
fn native_initial_view_is_atomic_view_state_and_not_restored_by_reopen() {
    let mut session = crate::session();
    session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    let first = session.active().unwrap().active_layer.unwrap();
    session.execute("layer.new.layer", json!({"name": "second"})).unwrap();
    let before = session.active().unwrap().revision;
    let view = NativeInitialView { selected_layers: vec![first], active_layer: Some(first), samplers: vec![[2.5, 3.5]], ..Default::default() };
    session.apply_import_initial_view(Some(view)).unwrap();
    let st = session.active().unwrap();
    assert_eq!(st.active_layer, Some(first));
    assert_eq!(st.color_samplers, vec![[2.5, 3.5]]);
    assert_eq!(st.revision, before);
    let bytes = photocraft_format::save_to_bytes(&st.doc, &Default::default()).unwrap();
    let imported = photocraft_io::import("view.pcraft", &bytes).unwrap();
    assert!(imported.initial_view.is_none());
    let mut reopened = crate::session();
    reopened.add_document(imported.document, None);
    assert!(reopened.active().unwrap().color_samplers.is_empty());
    assert!(reopened.active().unwrap().initial_path_selection.is_none());
}

#[test]
/// Reject invalid view references and nonfinite samplers without changing document identity, selection or revision.
fn invalid_native_initial_view_leaves_all_existing_state_unchanged() {
    let mut session = crate::session();
    session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    let before = session.active().unwrap().clone();
    for view in [
        NativeInitialView { selected_layers: before.selected_layers.clone(), samplers: vec![[2., 3.]], ..Default::default() },
        NativeInitialView { samplers: vec![[f64::NAN, 0.0]], ..Default::default() },
        NativeInitialView { target: NativeImportTarget::AlphaChannel(0), ..Default::default() },
        NativeInitialView { selected_path: Some(0), ..Default::default() },
    ] {
        assert!(session.apply_import_initial_view(Some(view)).is_err());
        let st = session.active().unwrap();
        assert!(Arc::ptr_eq(&before.doc, &st.doc));
        assert_eq!(st.active_layer, before.active_layer);
        assert_eq!(st.selected_layers, before.selected_layers);
        assert_eq!(st.layer_anchor, before.layer_anchor);
        assert_eq!(st.channel_view, before.channel_view);
        assert_eq!(st.color_samplers, before.color_samplers);
        assert_eq!(st.revision, before.revision);
        assert_eq!(st.saved_revision, before.saved_revision);
        assert_eq!(st.history.past_len(), before.history.past_len());
        assert_eq!(st.snapshots().first().unwrap().layers, before.snapshots().first().unwrap().layers);
    }
}

#[test]
fn imported_mask_preview_ends_when_its_owner_or_mask_no_longer_exists() {
    for action in ["select", "delete", "undo"] {
        let mut session = crate::session();
        session.execute("file.new", json!({"width":8,"height":6})).unwrap();
        let owner = session.active().unwrap().active_layer.unwrap();
        let other = session.execute("layer.new.layer", json!({"name":"Other"})).unwrap()["layer"].as_u64().unwrap();
        session.execute("layer.select", json!({"layer":owner.0})).unwrap();
        session.execute("layer.layerMask.revealAll", json!({})).unwrap();
        let history = session.active().unwrap().history.past_len();
        session
            .apply_import_initial_view(Some(NativeInitialView {
                selected_layers: vec![owner],
                active_layer: Some(owner),
                show_layer_mask: true,
                ..Default::default()
            }))
            .unwrap();
        assert!(crate::mask_view_cmds::current(session.active().unwrap()).is_some());
        assert!(crate::mask_view_cmds::editing(session.active().unwrap()).is_none());
        assert_eq!(session.active().unwrap().history.past_len(), history);
        match action {
            "select" => {
                session.execute("layer.select", json!({"layer":other})).unwrap();
            }
            "delete" => {
                session.execute("layer.layerMask.delete", json!({"layer":owner.0})).unwrap();
            }
            _ => {
                session.execute("edit.undo", json!({})).unwrap();
            }
        }
        assert!(session.active().unwrap().channel_view.layer_mask.is_none(), "{action}");
        assert!(crate::mask_view_cmds::current(session.active().unwrap()).is_none(), "{action}");
    }
}
