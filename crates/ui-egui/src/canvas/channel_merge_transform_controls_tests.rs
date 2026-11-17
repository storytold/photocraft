//! Channel-edit regressions retained across the shared UI migration.
use super::super::*;
use super::*;

#[test]
fn colour_set_transform_controls_match_the_native_frame_and_resize_at_the_displayed_corner() {
    for colors in [vec![0], vec![0, 2]] {
        for extra in [false, true] {
            for selected in [false, true] {
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                app.run("file.new", json!({"width": 200, "height": 200, "background": "transparent"})).unwrap();
                let id = app.session.active().unwrap().active_layer.unwrap();
                app.session
                    .edit("patch", |doc, _| {
                        doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(50, 50, 100, 100), &[1.0; 4]);
                        Ok(())
                    })
                    .unwrap();
                if extra {
                    app.run("layer.new.layer", json!({})).unwrap();
                    app.session
                        .edit("off-canvas layer", |doc, active| {
                            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(-30, -30, 240, 240), &[1.0; 4]);
                            Ok(())
                        })
                        .unwrap();
                    app.run("layer.select", json!({"layer": id.0, "mode": "add"})).unwrap();
                }
                if selected {
                    app.run("select.rect", json!({"x": 40, "y": 45, "width": 80, "height": 65})).unwrap();
                }
                app.run("channel.target", json!({"colors": colors})).unwrap();
                app.sync_views();
                app.ui.tool = Tool::Move;
                app.ui.tool_options.move_show_transform = true;
                let expected = if selected { [40.0, 45.0, 120.0, 110.0] } else { [0.0, 0.0, 200.0, 200.0] };
                let shown = frame(&app).unwrap();
                assert_eq!(corners(shown), expected, "{colors:?}, extra {extra}, selection {selected}");
                assert_eq!(frame_layers(&app).iter().map(|layer| layer.id).collect::<Vec<_>>(), [id]);
                let before = app.session.active().unwrap().doc.clone();
                crate::canvas::testing::shown(&mut app);
                crate::canvas::testing::press(&mut app, [shown.x0, shown.y0], egui::Modifiers::NONE);
                let initial = [[shown.x0, shown.y0], [shown.x1, shown.y0], [shown.x1, shown.y1], [shown.x0, shown.y1]];
                crate::canvas::testing::move_to(&mut app, [shown.x0 + 10.0, shown.y0 + 10.0], egui::Modifiers::NONE);
                let changed = app.ui.transform.as_ref().unwrap().quad_array();
                assert_ne!(changed[0], initial[0]);
                assert_eq!(changed[2], initial[2], "a corner resize keeps the opposite corner fixed, rather than translating the box");
                crate::transform_tool::cancel(&mut app);
                assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &before));
            }
        }
    }
}

#[test]
fn colour_set_transform_controls_are_available_on_background_samples() {
    for several in [false, true] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let bg = app.session.active().unwrap().active_layer.unwrap();
        if several {
            app.run("layer.new.layer", json!({})).unwrap();
            app.run("layer.select", json!({"layer": bg.0, "mode": "add"})).unwrap();
            assert_eq!(app.session.active().unwrap().selected_layers().len(), 2);
        }
        app.run("channel.target", json!({"colors": [0, 2]})).unwrap();
        app.sync_views();
        app.ui.tool = Tool::Move;
        app.ui.tool_options.move_show_transform = true;
        assert_eq!(corners(frame(&app).unwrap()), [0.0, 0.0, 200.0, 200.0]);
        assert_eq!(frame_layers(&app).iter().map(|l| l.id).collect::<Vec<_>>(), [bg]);
        let ctx = app.ctx().clone();
        assert_eq!(begin_controls(&mut app, &ctx), ControlsPress::Started);
        crate::transform_tool::cancel(&mut app);
    }
}
