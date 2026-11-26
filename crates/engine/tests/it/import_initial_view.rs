//! Imported editing-view regressions that execute the paint command registry.
//! Keep these in the public engine binary, alongside the other paint integration tests.

use photocraft_engine::{Session, mask_view_cmds};
use photocraft_io::NativeInitialView;
use serde_json::json;

#[test]
fn imported_mask_preview_keeps_pixel_edits_independent_of_its_display() {
    for (command, params, expected) in
        [("image.adjustments.invert", json!({}), [0.0, 1.0, 1.0, 1.0]), ("edit.fill", json!({"contents": "color", "color": "#0000ff"}), [0.0, 0.0, 1.0, 1.0])]
    {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":2,"height":2,"background":"#ff0000"})).unwrap();
        session.execute("layer.layerMask.revealAll", json!({})).unwrap();
        let layer = session.active().unwrap().active_layer.unwrap();
        session.execute("edit.fill", json!({"target":"mask","contents":"color","color":"#808080"})).unwrap();
        let before = session.active().unwrap().doc.layer(layer).unwrap().mask.as_ref().unwrap().surface.clone();
        assert!((0.0..1.0).contains(&before.sample_channel(0, 0, 0)));
        session
            .apply_import_initial_view(Some(NativeInitialView {
                active_layer: Some(layer),
                selected_layers: vec![layer],
                show_layer_mask: true,
                ..Default::default()
            }))
            .unwrap();
        assert_eq!(mask_view_cmds::current(session.active().unwrap()).unwrap().mode, mask_view_cmds::MaskViewMode::Gray);
        session.execute(command, params).unwrap();
        let state = session.active().unwrap();
        let edited = state.doc.layer(layer).unwrap();
        assert_eq!(edited.surface().unwrap().pixel(0, 0), expected, "{command}");
        assert_eq!(edited.mask.as_ref().unwrap().surface, before, "{command}");
        assert!(mask_view_cmds::current(state).is_some());
    }
}

#[test]
fn opened_snapshot_restores_the_imported_layer_target() {
    let mut source = Session::new();
    source.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    let first = source.active().unwrap().active_layer.unwrap();
    source.execute("layer.new.layer", json!({"name": "second"})).unwrap();
    let second = source.active().unwrap().active_layer.unwrap();
    let mut session = Session::new();
    session.add_document((*source.active().unwrap().doc).clone(), None);
    session.select_layer(second).unwrap();
    let before = session.active().unwrap().revision;
    let marker = session.active_layer_changes();
    let view = NativeInitialView { selected_layers: vec![first, second], active_layer: Some(first), ..Default::default() };
    session.apply_import_initial_view(Some(view.clone())).unwrap();
    let changed = session.active_layer_changes();
    assert_ne!(changed, marker);
    session.apply_import_initial_view(Some(view)).unwrap();
    assert_eq!(session.active_layer_changes(), changed);
    assert_eq!(session.active().unwrap().revision, before);
    session.execute("layer.select", json!({"layer": second.0})).unwrap();
    session.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    session.execute("history.selectSnapshot", json!({"snapshot": 0})).unwrap();
    let state = session.active().unwrap();
    assert_eq!(state.active_layer, Some(first));
    assert_eq!(state.selected_layers, vec![first, second]);
}
