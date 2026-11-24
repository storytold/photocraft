use super::*;
use photocraft_engine::channel_cmds::ChannelTarget;
use photocraft_io::{NativeImportTarget, NativeInitialView};

#[test]
fn live_brush_respects_the_saved_editing_target_while_the_imported_mask_is_displayed() {
    for depth in [8, 16, 32] {
        for (target, paints_mask) in [(NativeImportTarget::Composite, false), (NativeImportTarget::LayerMask, true)] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width":64,"height":64,"depth":depth,"background":"#ff0000"})).unwrap();
            app.run("layer.layerMask.revealAll", json!({})).unwrap();
            app.run("edit.fill", json!({"target":"mask","contents":"color","color":"#808080"})).unwrap();
            let layer = app.session.active().unwrap().active_layer.unwrap();
            app.session
                .apply_import_initial_view(Some(NativeInitialView {
                    selected_layers: vec![layer],
                    active_layer: Some(layer),
                    target,
                    show_layer_mask: true,
                    ..Default::default()
                }))
                .unwrap();
            app.sync_views();
            let expected = if paints_mask { ChannelTarget::LayerMask } else { ChannelTarget::Composite };
            assert_eq!(app.session.active().unwrap().channel_view.target, expected);
            assert!(photocraft_engine::mask_view_cmds::current(app.session.active().unwrap()).is_some());
            assert_eq!(paint_target(&app), json!(if paints_mask { "mask" } else { "pixels" }));
            assert_eq!(app.ui.mask_target, paints_mask);
            assert_eq!(app.layer_mask_targeted(), paints_mask);
            app.run("tools.setColors", json!({"foreground":[0.,0.,1.,1.]})).unwrap();
            app.run("tools.setBrush", json!({"brush":{"size":12,"hardness":1.,"smoothing":{"amount":0.}}})).unwrap();
            app.ui.tool = crate::Tool::Brush;
            let state = app.session.active().unwrap();
            let before = state.doc.layer(layer).unwrap();
            let pixels = before.surface().unwrap().clone();
            let mask = before.mask.as_ref().unwrap().surface.clone();
            let history = state.history.past_len();
            tool_event(&mut app, ToolEvent::Down { x: 12., y: 32., pressure: 1. }, egui::Modifiers::NONE);
            tool_event(&mut app, ToolEvent::Move { x: 32., y: 32., pressure: 1. }, egui::Modifiers::NONE);
            let preview = display_doc(&mut app, 0).0;
            let live = preview.layer(layer).unwrap();
            if paints_mask {
                assert_eq!(live.surface().unwrap(), &pixels);
                assert_ne!(live.mask.as_ref().unwrap().surface, mask);
            } else {
                assert_ne!(live.surface().unwrap(), &pixels);
                assert_eq!(live.mask.as_ref().unwrap().surface, mask);
                let painted = live.surface().unwrap().rgba(28, 32);
                assert!(painted[2] > painted[0], "{depth}-bit brush edits blue pixels behind the mask view");
            }
            tool_event(&mut app, ToolEvent::Up { x: 32., y: 32. }, egui::Modifiers::NONE);
            let state = app.session.active().unwrap();
            let committed = state.doc.layer(layer).unwrap();
            assert_eq!(committed.surface().unwrap(), live.surface().unwrap());
            assert_eq!(committed.mask.as_ref().unwrap().surface, live.mask.as_ref().unwrap().surface);
            assert_eq!(state.channel_view.target, expected);
            assert!(photocraft_engine::mask_view_cmds::current(state).is_some());
            assert_eq!(state.history.past_len(), history + 1);
        }
    }
}
