//! Exercise target ownership through actual admission and app command routing.
use crate::{PhotocraftApp, Services};
use photocraft_engine::{Session, channel_cmds::ChannelTarget};
use photocraft_io::{NativeImportTarget, NativeInitialView};
use serde_json::json;
use std::sync::Arc;

#[test]
fn admitted_initial_view_does_not_inherit_the_previous_tabs_mask_target() {
    for preview in [false, true] {
        for (target, mask) in [(NativeImportTarget::Composite, false), (NativeImportTarget::LayerMask, true)] {
            let mut app = PhotocraftApp::new(Session::new(), Services::default());
            app.background_jobs = false;
            app.run("file.new", json!({"width":8,"height":8,"background":"transparent"})).unwrap();
            app.run("layer.layerMask.revealAll", json!({})).unwrap();
            assert!(app.ui.mask_target, "the real mask command targets the first tab's mask");
            assert!(app.layer_mask_targeted());

            let mut source = Session::new();
            source.execute("file.new", json!({"width":8,"height":8,"background":"transparent"})).unwrap();
            source.execute("edit.fill", json!({"contents":"color","color":"#ff0000"})).unwrap();
            source.execute("layer.layerMask.revealAll", json!({})).unwrap();
            let source = source.active().unwrap();
            let layer = source.active_layer.unwrap();
            let view = NativeInitialView { selected_layers: vec![layer], active_layer: Some(layer), target, show_layer_mask: preview, ..Default::default() };
            let (_, warning) = app.session.admit_import_with_initial_view(source.doc.as_ref().clone(), None, true, Some(view));
            assert!(warning.is_none());
            let admitted = app.session.active().unwrap();
            let document = admitted.doc.clone();
            let revision = admitted.revision;
            let history = admitted.history.past_len();
            app.sync_views();
            let admitted = app.session.active().unwrap();
            assert!(Arc::ptr_eq(&document, &admitted.doc));
            assert_eq!(admitted.revision, revision);
            assert_eq!(admitted.history.past_len(), history);
            assert_eq!(admitted.channel_view.target, if mask { ChannelTarget::LayerMask } else { ChannelTarget::Composite });
            assert_eq!(photocraft_engine::mask_view_cmds::current(admitted).is_some(), preview);
            assert_eq!(app.ui.mask_target, mask);
            assert_eq!(app.layer_mask_targeted(), mask);
            let before_pixels = admitted.doc.layer(layer).unwrap().surface().unwrap().clone();
            let before_mask = admitted.doc.layer(layer).unwrap().mask.as_ref().unwrap().surface.clone();
            app.run("edit.fill", json!({"contents":"color","color":"#0000ff"})).unwrap();
            let after = app.session.active().unwrap().doc.layer(layer).unwrap();
            if mask {
                assert_eq!(after.surface().unwrap(), &before_pixels);
                assert_ne!(after.mask.as_ref().unwrap().surface, before_mask);
            } else {
                assert_ne!(after.surface().unwrap(), &before_pixels);
                assert_eq!(after.mask.as_ref().unwrap().surface, before_mask);
                // A later intentional mask target in this same imported tab is
                // preserved by ordinary edits and repeated synchronization.
                app.run("layer.new.layer", json!({})).unwrap();
                app.run("layer.layerMask.revealAll", json!({})).unwrap();
                assert!(app.ui.mask_target);
                app.sync_views();
                assert!(app.ui.mask_target);
                assert!(app.layer_mask_targeted());
            }
        }
    }
}
