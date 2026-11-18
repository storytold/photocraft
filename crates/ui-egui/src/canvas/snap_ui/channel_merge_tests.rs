//! Channel-edit regressions retained across the shared UI migration.
use super::*;

#[test]
fn colour_set_selected_drag_snaps_its_selection_frame_to_guides() {
    for colors in [[0, 1], [0, 2], [1, 2]] {
        let mut app = app_with_box();
        app.ui.tool = Tool::Move;
        app.ui.extras.snap = true;
        app.ui.view.show.smart_guides = false;
        app.ui.view.snap_to.layers = false;
        app.ui.view.snap_to.document_bounds = false;
        app.run("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 90})).unwrap();
        app.run("channel.target", json!({"colors": colors})).unwrap();
        app.session
            .edit("guide", |doc, _| {
                doc.guides.vertical.push(200.0);
                Ok(())
            })
            .unwrap();
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, m);
        assert_eq!(app.canvas_tools.mover.dragged_box().unwrap().0, [20.0, 20.0, 120.0, 110.0]);
        let lines = crate::canvas::tool_event_response(&mut app, ToolEvent::Move { x: 137.0, y: 60.0, pressure: 1.0 }, m).snapped;
        assert!(lines.iter().any(|line| line.along == Axis::Y && line.position == 200.0 && line.kind == SnapKind::Guide));
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 137.0, y: 60.0 }, m);
        assert_eq!(app.session.active().unwrap().doc.selection.as_ref().unwrap().content_bounds(), photocraft_geom::Rect::new(100, 20, 200, 110));
    }
}

fn app_with_linked_channel_mover(colors: &[usize]) -> Box<PhotocraftApp> {
    let mut app = app_with_box();
    app.run("layer.new.layer", json!({"name":"linked"})).unwrap();
    app.run("select.rect", json!({"x":150,"y":120,"width":40,"height":20})).unwrap();
    app.run("edit.fill", json!({"color":"#00ff00"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("layer.select", json!({"name":"mover"})).unwrap();
    app.run("layer.select", json!({"name":"linked","mode":"add"})).unwrap();
    app.run("layer.linkLayers", json!({"linked":true})).unwrap();
    app.run("layer.select", json!({"name":"mover"})).unwrap();
    app.run("channel.target", json!({"colors":colors})).unwrap();
    app.run("channel.setVisible", json!({"channel":"composite","visible":true})).unwrap();
    app.run("tools.setColors", json!({"background":"#000000"})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = false;
    app
}

#[test]
fn native_channel_drag_frames_only_its_active_layer() {
    for colors in [&[2][..], &[0, 2][..]] {
        let mut app = app_with_linked_channel_mover(colors);
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, egui::Modifiers::NONE);
        assert_eq!(app.canvas_tools.mover.dragged_box().unwrap().0, [40.0, 40.0, 90.0, 80.0], "{colors:?}: only the active layer moves");
        assert_eq!(near(&app, &moving_set(&app), Axis::X, 120.0), Some((SnapKind::LayerEdge, 120.0)), "{colors:?}: the linked layer stays a target");
    }
}

#[test]
fn native_channel_drag_snaps_to_stationary_linked_layers() {
    for colors in [&[2][..], &[0, 2][..]] {
        let mut app = app_with_linked_channel_mover(colors);
        let st = app.session.active().unwrap();
        let mover = st.active_layer.unwrap();
        let linked = st.doc.walk().into_iter().find(|(_, _, l)| l.name == "linked").unwrap().2.id;
        let before = st.doc.clone();
        // The Blue block's bottom at 80 approaches the linked layer at 120 by 3 px.
        // It must snap to +40, not move by the unsnapped +37 offset.
        move_drag(&mut app, [60.0, 97.0], egui::Modifiers::NONE);
        let doc = &app.session.active().unwrap().doc;
        let after = doc.layer(mover).unwrap().surface().unwrap();
        assert_eq!(after.sample_channel(60, 78, 2), 0.0, "{colors:?}: snapped block starts at y=80");
        assert_eq!(after.sample_channel(60, 118, 2), 1.0, "{colors:?}: snapped block reaches y=120");
        assert_eq!(doc.layer(linked), before.layer(linked), "{colors:?}: the linked layer stays put");
        let old = before.layer(mover).unwrap().surface().unwrap().read_region(doc.bounds());
        let new = after.read_region(doc.bounds());
        for (old, new) in old.as_chunks::<4>().0.iter().zip(new.as_chunks::<4>().0) {
            for component in 0..4 {
                if !colors.contains(&component) {
                    assert_eq!(old[component].to_bits(), new[component].to_bits(), "{colors:?}: untargeted component {component}");
                }
            }
        }
    }
}
