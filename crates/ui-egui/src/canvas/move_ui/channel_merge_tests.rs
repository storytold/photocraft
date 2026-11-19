//! Channel-edit regressions retained across the shared UI migration.
use super::super::*;
use super::*;
use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Layer, Size};

#[test]
fn channel_move_repaints_a_stationary_drag_after_changing_the_background() {
    for colors in [vec![0], vec![0, 2]] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":32,"height":16,"background":"transparent"})).unwrap();
        app.run("edit.fill", json!({"color":"#804020"})).unwrap();
        app.run("channel.target", json!({"colors":colors})).unwrap();
        app.run("tools.setColors", json!({"background":"#000000"})).unwrap();
        app.sync_views();
        app.ui.tool = Tool::Move;
        app.ui.tool_options.move_auto_select = false;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 8.0, y: 8.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 12.0, y: 8.0, pressure: 1.0 }, m);
        let (old, old_key) = super::super::display_doc(&mut app, 0).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        let layer = app.session.active().unwrap().active_layer.unwrap();
        app.run("tools.setColors", json!({"background":"#ffffff"})).unwrap();
        assert_eq!(app.session.active().unwrap().revision, revision);
        let (shown, key) = super::super::display_doc(&mut app, 0).unwrap();
        let mut expected = (*before).clone();
        let target = photocraft_engine::color_channels::selected(&app.session).unwrap();
        photocraft_engine::channel_cmds::move_color_channels(&mut expected, layer, target, 4, 0, app.session.tools.background).unwrap();
        assert!(shown.layer(layer).unwrap().surface() == expected.layer(layer).unwrap().surface(), "changed background must reach the preview");
        assert!(!std::sync::Arc::ptr_eq(&old, &shown));
        assert_ne!(old_key, key, "the canvas needs a fresh preview key even when the pointer stays still");
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        assert!(super::super::damage(&app, before.id, revision, old_key, key).is_some());
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 12.0, y: 8.0 }, m);
        assert!(
            app.session.active().unwrap().doc.layer(layer).unwrap().surface() == expected.layer(layer).unwrap().surface(),
            "preview must match the committed move"
        );
    }
}

#[test]
fn dragging_with_a_colour_channel_targeted_moves_that_channel_alone() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    app.session.execute("channel.target", json!({"channel": "red"})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 36.0, y: 16.0, pressure: 1.0 }, m);
    let shown = super::super::display_doc(&mut app, 0).unwrap().0;
    let id = app.session.active().unwrap().active_layer.unwrap();
    let at = |d: &photocraft_doc::Document, x, y| d.layer(id).and_then(|l| l.surface()).map(|s| s.pixel(x, y)).unwrap();
    // The white square's red is now at x 28..44; its green and blue stayed at x 8..24.
    assert_eq!(at(&shown, 30, 16), vec![1.0, 0.0, 0.0, 1.0], "preview: red moved in");
    assert_eq!(at(&shown, 12, 16)[1..], [1.0, 1.0, 1.0], "preview: green, blue and the layer stay");
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 36.0, y: 16.0 }, m);
    let d = &app.session.active().unwrap().doc;
    assert_eq!(at(d, 30, 16), vec![1.0, 0.0, 0.0, 1.0], "commit: red moved in");
    assert_eq!(at(d, 12, 16)[1..], [1.0, 1.0, 1.0], "commit: green, blue and the layer stay");
}

#[test]
fn refused_cpu_channel_move_recovers_at_the_previous_valid_offset() {
    for colors in [vec![1], vec![0, 2]] {
        let mut app = photo(100, 100, 8);
        app.session.execute("channel.target", json!({"colors":colors})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 55.0, y: 50.0, pressure: 1.0 }, m);
        let (good, key) = super::super::display_doc(&mut app, 0).unwrap();
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 10_000_050.0, y: 50.0, pressure: 1.0 }, m);
        assert!(super::super::display_doc(&mut app, 0).is_none());
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 55.0, y: 50.0, pressure: 1.0 }, m);
        let (recovered, next) = super::super::display_doc(&mut app, 0).expect("returning to the previous good offset must redraw");
        assert_ne!(key, next, "the recovered frame receives a fresh damage key");
        assert_eq!(recovered.layers[1].surface(), good.layers[1].surface());
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    }
}

#[test]
fn selecting_several_colours_releases_a_retained_single_channel_gpu_preview() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width":32,"height":32,"background":"white"})).unwrap();
    app.sync_views();
    let st = app.session.active().unwrap();
    app.move_gpu_preview = Some(Box::new(super::super::MoveGpuPreview {
        doc: st.doc.id,
        snapshot: std::sync::Arc::downgrade(&st.doc),
        layer: st.active_layer.unwrap(),
        colors: photocraft_engine::color_channels::ColorChannels::from_indices([0], 3).unwrap(),
        background: [1.0; 4],
        factor: 1,
        bounds: st.doc.bounds(),
        gpu: None,
        pending: false,
        shown: true,
    }));
    app.session.execute("channel.target", json!({"colors":[0,2]})).unwrap();
    assert!(super::super::channel_texture(&mut app, 0, None).is_none());
    assert!(app.move_gpu_preview.is_none());
    assert!(super::super::display_doc(&mut app, 0).is_none());
}

#[test]
fn colour_set_selected_move_redraws_stationary_transparency_between_frames() {
    for depth in [8, 16, 32] {
        for feather in [0, 3] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 2048, "height": 512, "depth": depth, "background": "transparent"})).unwrap();
            let id = app.session.active().unwrap().active_layer.unwrap();
            let square = Rect::new(768, 256, 800, 288);
            app.session
                .edit("stationary alpha", |doc, _| {
                    doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(square, &[1.0, 0.0, 0.0, 1.0]);
                    Ok(())
                })
                .unwrap();
            app.run("select.all", json!({})).unwrap();
            if feather > 0 {
                app.run("select.modify.feather", json!({"radius": feather})).unwrap();
            }
            app.run("channel.target", json!({"colors": [0, 2]})).unwrap();
            app.session.tools.background = [1.0; 4];
            app.ui.tool = Tool::Move;
            app.ui.tool_options.move_auto_select = false;
            app.ui.extras.snap = false;
            app.ui.view.show.smart_guides = false;
            crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 780.0, y: 270.0, pressure: 1.0 }, egui::Modifiers::NONE);
            let mut frames = Vec::new();
            for x in [1280.0, 1780.0] {
                crate::canvas::tool_event(&mut app, ToolEvent::Move { x, y: 270.0, pressure: 1.0 }, egui::Modifiers::NONE);
                frames.push(super::super::display_doc(&mut app, 0).unwrap());
            }
            let pixel = |doc: &Document| photocraft_compose::render(doc, square).get(780, 270);
            assert_eq!(pixel(&frames[0].0), [0.0, 0.0, 0.0, 1.0]);
            assert_eq!(pixel(&frames[1].0), [1.0, 0.0, 1.0, 1.0]);
            let st = app.session.active().unwrap();
            let damage = super::super::damage(&app, st.doc.id, st.revision, frames[0].1, frames[1].1).unwrap();
            assert!(damage.contains(780, 270), "{depth}/{feather}: stationary alpha changed outside repaint {damage:?}");
            crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 1780.0, y: 270.0 }, egui::Modifiers::NONE);
            assert_eq!(app.session.active().unwrap().doc.layer(id), frames[1].0.layer(id));
        }
    }
}

#[test]
fn colour_set_selected_drag_previews_and_commits_only_selected_native_samples() {
    for colors in [vec![0, 1], vec![0, 2], vec![1, 2]] {
        let (mut app, id) = selected_pixels(8);
        app.run("select.modify.feather", json!({"radius":1})).unwrap();
        app.run("channel.target", json!({"colors":colors})).unwrap();
        app.ui.tool_options.move_auto_select = true;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        let mut oracle = photocraft_engine::Session::new();
        oracle.tools.background = app.session.tools.background;
        oracle.add_document((*before).clone(), None);
        oracle.execute("edit.transform", json!({"target":{"colors":colors},"dx":10,"dy":4,"interpolation":"nearest"})).unwrap();
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 26.0, y: 20.0, pressure: 1.0 }, m);
        assert!(photocraft_engine::float_cmds::floating(app.session.active().unwrap()).is_none());
        let preview = super::super::display_doc(&mut app, 0).unwrap().0;
        assert_eq!(preview.layer(id).unwrap().surface(), oracle.active().unwrap().doc.layer(id).unwrap().surface());
        assert_eq!(preview.selection, oracle.active().unwrap().doc.selection);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 16.0, y: 16.0, pressure: 1.0 }, m);
        let reset = super::super::display_doc(&mut app, 0).unwrap().0;
        assert_eq!(reset.layer(id).unwrap().surface(), before.layer(id).unwrap().surface());
        assert_eq!(reset.selection, before.selection);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 26.0, y: 20.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 26.0, y: 20.0 }, m);
        assert!(!app.ui.status_error, "{}", app.ui.status);
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.layer(id).unwrap().surface(), preview.layer(id).unwrap().surface());
        assert_eq!(st.doc.selection, preview.selection);
        assert_eq!(st.history.past_len(), history + 1);
        app.session.undo();
        assert_eq!(app.session.active().unwrap().doc.layer(id).unwrap().surface(), before.layer(id).unwrap().surface());
    }
}

#[test]
fn colour_set_selected_drag_outline_follows_the_pointer_and_resets_on_release() {
    for depth in [8, 16, 32] {
        for colors in [[0, 1], [0, 2], [1, 2]] {
            let (mut app, _) = selected_pixels(depth);
            app.run("select.modify.feather", json!({"radius": 1})).unwrap();
            app.run("channel.target", json!({"colors": colors})).unwrap();
            let original = app.session.active().unwrap().doc.selection.clone().unwrap();
            let m = egui::Modifiers::NONE;
            assert_eq!(
                super::super::native_selection_cursor(&app, craft::keymap::Mods::NONE, false),
                Some(craft::behavior::PaintedCursor::badged(craft::behavior::CursorIcon::Default, craft::behavior::CursorBadge::Scissors).into())
            );
            assert_eq!(
                super::super::native_selection_cursor(&app, craft::keymap::Mods::ALT, false),
                Some(craft::behavior::CursorIcon::Default.into()),
                "copying a set is unsupported"
            );
            crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
            for (x, y, delta) in [(26.0, 21.0, (10, 5)), (16.0, 16.0, (0, 0)), (12.0, 23.0, (-4, 7))] {
                crate::canvas::tool_event(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
                assert_eq!(crate::canvas::tools::selection::shown_offset(&app), Some(delta), "{depth}-bit, {colors:?}");
                assert_eq!(
                    super::super::native_selection_cursor(&app, craft::keymap::Mods::NONE, true),
                    Some(craft::behavior::PaintedCursor::arrow(true).into())
                );
                let shown = super::super::display_doc(&mut app, 0).unwrap().0;
                assert_eq!(shown.selection.as_ref().unwrap().content_bounds(), original.content_bounds().translate(delta.0, delta.1));
                assert_eq!(app.session.active().unwrap().doc.selection.as_ref(), Some(&original), "the live offset does not edit the source");
            }
            crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 12.0, y: 23.0 }, m);
            assert_eq!(crate::canvas::tools::selection::shown_offset(&app), None, "the committed outline must not be offset twice");
            assert_eq!(app.session.active().unwrap().doc.selection.as_ref().unwrap().content_bounds(), original.content_bounds().translate(-4, 7));
        }
    }
}

#[test]
fn colour_set_selected_shift_drag_uses_diagonals_in_preview_and_commit() {
    for depth in [8, 16, 32] {
        for colors in [[0, 1], [0, 2], [1, 2]] {
            let (mut app, _) = selected_pixels(depth);
            app.run("channel.target", json!({"colors": colors})).unwrap();
            let original = app.session.active().unwrap().doc.selection.clone().unwrap();
            let m = egui::Modifiers::SHIFT;
            crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
            crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 28.0, pressure: 1.0 }, m);
            assert_eq!(super::super::drag_offset(&app), Some((13, 13)), "{depth}-bit, {colors:?}: 45 degrees");
            let shown = super::super::display_doc(&mut app, 0).unwrap().0;
            assert_eq!(shown.selection.as_ref().unwrap().content_bounds(), original.content_bounds().translate(13, 13));
            crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 28.0 }, m);
            assert_eq!(app.session.active().unwrap().doc.selection, shown.selection, "{depth}-bit, {colors:?}: release matches preview");
        }
    }
}

#[test]
fn colour_set_selected_drag_does_not_snap_to_its_starting_selection() {
    for colors in [[0, 1], [0, 2], [1, 2]] {
        let (mut app, _) = selected_pixels(8);
        app.run("channel.target", json!({"colors": colors})).unwrap();
        app.ui.extras.snap = true;
        app.ui.view.snap_to.document_bounds = false;
        app.ui.view.snap_to.layers = false;
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
        let response = crate::canvas::tool_event_response(&mut app, ToolEvent::Move { x: 18.0, y: 17.0, pressure: 1.0 }, m);
        assert_eq!(super::super::drag_offset(&app), Some((2, 1)), "{colors:?}: a small drag must not snap back to its own selection");
        assert!(response.snapped.iter().all(|line| line.kind != craft::behavior::geometry::SnapKind::Selection));
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 18.0, y: 17.0 }, m);
        assert_eq!(app.session.active().unwrap().doc.selection.as_ref().unwrap().content_bounds(), Rect::new(10, 9, 26, 25));
    }
}

#[test]
fn colour_set_command_alt_drag_from_selection_tools_is_refused_before_duplication() {
    for tool in [Tool::RectMarquee, Tool::EllipseMarquee, Tool::Lasso, Tool::PolygonLasso, Tool::MagicWand] {
        let (mut app, _) = selected_pixels(16);
        app.run("channel.target", json!({"colors": [0, 2]})).unwrap();
        app.ui.tool = tool;
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        let view = app.session.active().unwrap().channel_view.clone();
        let m = egui::Modifiers { command: true, mac_cmd: true, alt: true, ..Default::default() };
        drag(&mut app, [30.0, 30.0], [40.0, 30.0], m);
        let st = app.session.active().unwrap();
        assert!(std::sync::Arc::ptr_eq(&st.doc, &before), "{tool:?}: refusal must precede duplicating or moving pixels");
        assert_eq!(st.history.past_len(), history, "{tool:?}");
        assert_eq!(st.channel_view, view, "{tool:?}");
        assert!(app.ui.status_error, "{tool:?}: explain the unsupported gesture");
        assert!(!app.canvas_tools.mover.dragging());
    }
}

#[test]
fn colour_set_retained_behind_filter_mask_view_moves_the_whole_smart_object() {
    for colors in [vec![0], vec![0, 2]] {
        for mode in ["gray", "overlay"] {
            let (mut app, _) = selected_pixels(16);
            app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
            let id = app.session.active().unwrap().active_layer.unwrap();
            app.session
                .edit("filter mask", |doc, _| {
                    let photocraft_doc::LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!("not a smart object") };
                    sm.filter_mask = Some(photocraft_doc::LayerMask::hide_all());
                    Ok(())
                })
                .unwrap();
            app.run("channel.target", json!({"colors": colors})).unwrap();
            app.run("view.layerMask", json!({"mask": "filter", "mode": mode})).unwrap();
            app.ui.tool_options.move_auto_select = false;
            assert!(super::super::color_target(&app).is_none(), "{mode}, {colors:?}: the shown filter mask overrides colour rows");
            app.run("layer.lockLayers", json!({"pixels": true})).unwrap();
            let before = app.session.active().unwrap().doc.clone();
            let history = app.session.active().unwrap().history.past_len();
            let oracle = photocraft_engine::layer_multi_cmds::moved_target(&before, &[id], 10, 5, false).unwrap();
            let m = egui::Modifiers::NONE;
            crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
            crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 26.0, y: 21.0, pressure: 1.0 }, m);
            assert_eq!(crate::canvas::tools::selection::shown_offset(&app), None, "whole-object movement does not carry a single layer's selection");
            let shown = super::super::display_doc(&mut app, 0).unwrap().0;
            assert_eq!(shown.layer(id), oracle.layer(id), "{mode}, {colors:?}: preview follows whole-object routing");
            assert_eq!(shown.selection, before.selection);
            crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 26.0, y: 21.0 }, m);
            let st = app.session.active().unwrap();
            assert_eq!(st.doc.layer(id), oracle.layer(id), "{mode}, {colors:?}: pixel lock permits object movement");
            assert_eq!(st.doc.selection, before.selection);
            assert_eq!(st.history.past_len(), history + 1);
            assert!(app.session.undo());
            assert_eq!(app.session.active().unwrap().doc.layer(id), before.layer(id));
        }
    }
}

#[test]
fn colour_set_filter_mask_view_keeps_its_target_when_alt_drag_duplicates() {
    for mode in ["gray", "overlay"] {
        let (mut app, _) = selected_pixels(16);
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let original = app.session.active().unwrap().active_layer.unwrap();
        app.session
            .edit("filter mask", |doc, _| {
                let photocraft_doc::LayerContent::Smart(sm) = &mut doc.layer_mut(original).unwrap().content else { panic!("not a smart object") };
                sm.filter_mask = Some(photocraft_doc::LayerMask::hide_all());
                Ok(())
            })
            .unwrap();
        app.run("channel.target", json!({"colors": [0, 2]})).unwrap();
        app.run("view.layerMask", json!({"mask": "filter", "mode": mode})).unwrap();
        app.ui.tool_options.move_auto_select = false;
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        let oracle = photocraft_engine::layer_multi_cmds::moved_target(&before, &[original], 10, 5, false).unwrap();
        drag(&mut app, [16.0, 16.0], [26.0, 21.0], egui::Modifiers::ALT);
        assert!(!app.ui.status_error, "{mode}: {}", app.ui.status);
        let st = app.session.active().unwrap();
        let copy = st.active_layer.unwrap();
        assert_ne!(copy, original);
        assert_eq!(st.doc.layer(original), before.layer(original), "the original object stays put");
        let photocraft_doc::LayerContent::Smart(moved) = &st.doc.layer(copy).unwrap().content else { panic!("copy is not smart") };
        let photocraft_doc::LayerContent::Smart(expected) = &oracle.layer(original).unwrap().content else { panic!("oracle is not smart") };
        assert_eq!(moved.transform, expected.transform, "{mode}: the copy moves as a whole object");
        assert_eq!(st.doc.selection, before.selection, "the selection is not a native colour target");
        assert_eq!(photocraft_engine::mask_view_cmds::current(st).unwrap().layer, copy, "view target follows the copied mask");
        assert_eq!(st.history.past_len(), history + 1, "copy and movement share one undo step");
        assert!(app.session.undo());
        assert_eq!(app.session.active().unwrap().doc.layers, before.layers);
    }
}

#[test]
fn colour_set_command_drags_from_selection_tools_use_native_selected_move() {
    for colors in [vec![0], vec![0, 2]] {
        for tool in
            [Tool::RectMarquee, Tool::EllipseMarquee, Tool::SingleRowMarquee, Tool::SingleColumnMarquee, Tool::Lasso, Tool::PolygonLasso, Tool::MagicWand]
        {
            for start in [[16.0, 16.0], [30.0, 30.0]] {
                let (mut app, id) = selected_pixels(16);
                app.run("channel.target", json!({"colors": colors})).unwrap();
                app.ui.tool = tool;
                app.ui.extras.snap = true;
                app.ui.view.snap_to.layers = false;
                app.ui.view.snap_to.document_bounds = false;
                let before = app.session.active().unwrap().doc.clone();
                let history = app.session.active().unwrap().history.past_len();
                let mut oracle = photocraft_engine::Session::new();
                oracle.tools.background = app.session.tools.background;
                oracle.add_document((*before).clone(), None);
                oracle.execute("edit.transform", json!({"target": {"colors": colors}, "dx": 2, "dy": 1, "interpolation": "nearest"})).unwrap();
                let m = egui::Modifiers { command: true, mac_cmd: true, ..Default::default() };
                crate::canvas::tool_event(&mut app, ToolEvent::Down { x: start[0], y: start[1], pressure: 1.0 }, m);
                crate::canvas::tool_event(&mut app, ToolEvent::Move { x: start[0] + 2.0, y: start[1] + 1.0, pressure: 1.0 }, m);
                assert!(!app.ui.status_error, "{tool:?}, {start:?}: {}", app.ui.status);
                assert_eq!(super::super::drag_offset(&app), Some((2, 1)), "{tool:?}, {start:?}: native movement, without self-snapping");
                assert_eq!(crate::canvas::tools::selection::shown_offset(&app), Some((2, 1)));
                let shown = super::super::display_doc(&mut app, 0).unwrap().0;
                assert_eq!(shown.layer(id), oracle.active().unwrap().doc.layer(id));
                assert_eq!(shown.selection, oracle.active().unwrap().doc.selection);
                assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &before));
                crate::canvas::tool_event(&mut app, ToolEvent::Up { x: start[0] + 2.0, y: start[1] + 1.0 }, m);
                let st = app.session.active().unwrap();
                assert_eq!(st.doc.layer(id), shown.layer(id));
                assert_eq!(st.doc.selection, shown.selection);
                assert_eq!(st.history.past_len(), history + 1);
                assert_eq!(app.ui.tool, tool, "the shortcut does not permanently change tools");
            }
        }
    }
}

#[test]
fn colour_set_temporary_move_keeps_shift_constraints_after_releasing_command() {
    for tool in [Tool::RectMarquee, Tool::Brush] {
        for start in [[16.0, 16.0], [30.0, 30.0]] {
            let (mut app, _) = selected_pixels(16);
            app.run("channel.target", json!({"colors": [0, 2]})).unwrap();
            app.ui.tool = tool;
            let m = egui::Modifiers { command: true, mac_cmd: true, ..Default::default() };
            crate::canvas::tool_event(&mut app, ToolEvent::Down { x: start[0], y: start[1], pressure: 1.0 }, m);
            crate::canvas::tool_event(&mut app, ToolEvent::Move { x: start[0] + 14.0, y: start[1] + 12.0, pressure: 1.0 }, m | egui::Modifiers::SHIFT);
            assert_eq!(super::super::drag_offset(&app), Some((13, 13)), "{tool:?}: diagonal preview");
            let shown = super::super::display_doc(&mut app, 0).unwrap().0;
            crate::canvas::tool_event(&mut app, ToolEvent::Up { x: start[0] + 14.0, y: start[1] + 12.0 }, egui::Modifiers::SHIFT);
            assert_eq!(app.session.active().unwrap().doc.selection, shown.selection, "{tool:?}: release preserves the constrained offset");
            assert_eq!(app.ui.tool, tool);
        }
    }
}

#[test]
fn colour_set_copy_guard_rechecks_after_auto_select_ends_mask_view() {
    for filter in [false, true] {
        for mode in ["gray", "overlay"] {
            let (mut app, _) = selected_pixels(16);
            app.run("select.deselect", json!({})).unwrap();
            if filter {
                app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
            }
            let source = app.session.active().unwrap().active_layer.unwrap();
            if filter {
                app.session
                    .edit("filter mask", |doc, _| {
                        let photocraft_doc::LayerContent::Smart(sm) = &mut doc.layer_mut(source).unwrap().content else { panic!("not smart") };
                        sm.filter_mask = Some(photocraft_doc::LayerMask::hide_all());
                        Ok(())
                    })
                    .unwrap();
            } else {
                app.run("layer.layerMask.revealAll", json!({})).unwrap();
            }
            app.run("layer.new.layer", json!({"name": "pick me"})).unwrap();
            let picked = app.session.active().unwrap().active_layer.unwrap();
            app.session
                .edit("other patch", |doc, _| {
                    doc.layer_mut(picked).unwrap().surface_mut().unwrap().fill_rect(Rect::new(40, 8, 56, 24), &[1.0; 4]);
                    Ok(())
                })
                .unwrap();
            app.run("layer.select", json!({"layer": source.0})).unwrap();
            app.run("channel.target", json!({"colors": [0, 2]})).unwrap();
            app.run("view.layerMask", json!({"mask": if filter { "filter" } else { "layer" }, "mode": mode})).unwrap();
            assert!(super::super::color_target(&app).is_none());
            let before = app.session.active().unwrap().doc.clone();
            let history = app.session.active().unwrap().history.past_len();
            drag(&mut app, [46.0, 16.0], [56.0, 16.0], egui::Modifiers::ALT);
            let st = app.session.active().unwrap();
            assert!(std::sync::Arc::ptr_eq(&st.doc, &before), "{filter}/{mode}: no copy or movement after the pick exposes a native set");
            assert_eq!(st.active_layer, Some(picked), "Auto-Select must exercise the changed target");
            assert_eq!(st.history.past_len(), history);
            assert!(app.ui.status_error);
            assert!(!app.canvas_tools.mover.dragging());
        }
    }
}

#[test]
fn colour_set_exposed_before_alt_drag_duplication_cancels_without_moving_the_original() {
    for selected in [false, true] {
        let (mut app, _) = selected_pixels(16);
        if selected {
            let original = app.session.active().unwrap().active_layer.unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
            app.run("layer.select", json!({"layer": original.0, "mode": "add"})).unwrap();
        } else {
            app.run("select.deselect", json!({})).unwrap();
        }
        app.ui.tool_options.move_auto_select = false;
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, egui::Modifiers::ALT);
        app.session.execute("channel.target", json!({"colors": [0, 2]})).unwrap();
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 26.0, y: 16.0, pressure: 1.0 }, egui::Modifiers::ALT);
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 26.0, y: 16.0 }, egui::Modifiers::ALT);
        assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &before));
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
        assert!(app.ui.status_error);
        assert!(!app.canvas_tools.mover.dragging());
    }
}

#[test]
fn selected_background_channel_drags_match_keyboard_and_native_oracles() {
    fn background(depth: u32, colors: &[usize]) -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64, "depth": depth})).unwrap();
        app.session
            .edit("background patch", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(8, 8, 24, 24), &[0.9, 0.3, 0.2, 1.0]);
                Ok(())
            })
            .unwrap();
        app.run("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
        app.run("channel.target", json!({"colors": colors})).unwrap();
        app.sync_views();
        app.ui.tool = Tool::Move;
        app.ui.tool_options.move_auto_select = false;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        app
    }
    for depth in [8, 16, 32] {
        for colors in [vec![0], vec![0, 2]] {
            let mut app = background(depth, &colors);
            let id = app.session.active().unwrap().active_layer.unwrap();
            let before = app.session.active().unwrap().doc.clone();
            let mut oracle = photocraft_engine::Session::new();
            oracle.tools.background = app.session.tools.background;
            oracle.add_document((*before).clone(), None);
            oracle.execute("edit.transform", json!({"target": {"colors": colors}, "dx": 10, "dy": 5, "interpolation": "nearest"})).unwrap();
            let expected = oracle.active().unwrap().doc.layer(id).unwrap().surface().unwrap().clone();
            let modifiers = egui::Modifiers::NONE;
            crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, modifiers);
            crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 26.0, y: 21.0, pressure: 1.0 }, modifiers);
            let shown = super::super::display_doc(&mut app, 0).unwrap().0;
            assert_eq!(shown.layer(id).unwrap().surface().unwrap(), &expected);
            assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
            crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 26.0, y: 21.0 }, modifiers);
            assert!(offset(&app).is_none(), "native colour samples never float as RGBA");
            let st = app.session.active().unwrap();
            assert_eq!(st.doc.layer(id).unwrap().surface().unwrap(), &expected, "{depth}, {colors:?}");
            assert!(st.doc.is_background(id));
            assert!(app.session.undo());
            assert!(app.session.active().unwrap().doc.layers == before.layers);
            assert!(app.session.redo());
            assert_eq!(app.session.active().unwrap().doc.layer(id).unwrap().surface().unwrap(), &expected);
            let mut keyboard = background(depth, &colors);
            crate::move_mods::nudge(&mut keyboard, 10.0, 5.0, false);
            let st = keyboard.session.active().unwrap();
            assert!(photocraft_engine::float_cmds::floating(st).is_none());
            assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap(), &expected);
            assert_eq!(st.doc.selection, oracle.active().unwrap().doc.selection);
        }
    }
}

#[test]
fn viewed_mask_move_preview_agrees_with_edit_target_precedence() {
    for edit in ["composite", "alpha", "quick"] {
        let (mut app, _) = selected_pixels(16);
        app.run("select.deselect", json!({})).unwrap();
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        app.run("layer.layerMask.linked", json!({"linked": false})).unwrap();
        app.session
            .edit("mask patch", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().mask.as_mut().unwrap().surface.fill_rect(Rect::new(4, 6, 8, 10), &[0.0]);
                Ok(())
            })
            .unwrap();
        if edit == "alpha" {
            app.run("channel.new", json!({"name": "saved"})).unwrap();
            app.run("channel.target", json!({"channel": "saved"})).unwrap();
        } else if edit == "quick" {
            app.run("select.editInQuickMaskMode", json!({})).unwrap();
        }
        app.run("view.layerMask", json!({"mode": "overlay"})).unwrap();
        app.ui.tool_options.move_auto_select = false;
        if edit != "composite" {
            assert!(!app.layer_mask_targeted());
        }
        let before = app.session.active().unwrap().doc.clone();
        let id = app.session.active().unwrap().active_layer.unwrap();
        let expected = photocraft_engine::layer_multi_cmds::moved_target(&before, &[id], 10, 5, edit == "composite").unwrap();
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 26.0, y: 21.0, pressure: 1.0 }, m);
        let shown = super::super::display_doc(&mut app, 0).unwrap().0;
        assert!(shown.layers == expected.layers, "{edit}: preview follows target precedence");
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 26.0, y: 21.0 }, m);
        assert!(app.session.active().unwrap().doc.layers == shown.layers, "{edit}: preview and commit have the same target");
        assert!(app.session.undo());
        assert!(app.session.active().unwrap().doc.layers == before.layers);
    }
}

#[test]
fn channel_drag_across_artboards_keeps_the_layer_on_its_board() {
    for colors in [vec![0], vec![0, 2]] {
        let (mut app, red, a, _) = two_boards();
        app.run("channel.target", json!({"colors":colors})).unwrap();
        let st = app.session.active().unwrap();
        let before = st.doc.clone();
        let steps = st.history.past_len();
        let mut oracle = photocraft_engine::Session::new();
        oracle.tools.background = app.session.tools.background;
        oracle.add_document((*before).clone(), None);
        oracle.execute("layer.translate", json!({"layer":red.0,"target":{"colors":colors},"dx":190,"dy":10})).unwrap();

        drag(&mut app, [20.0, 20.0], [210.0, 30.0], egui::Modifiers::NONE);
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.artboard_of(red), Some(a));
        assert_eq!(st.doc.layer(red), oracle.active().unwrap().doc.layer(red));
        assert_eq!(st.history.past_len(), steps + 1);
        assert!(app.session.undo());
        assert_eq!(app.session.active().unwrap().doc.layers, before.layers);
    }
}

#[test]
fn pattern_preview_keeps_native_channel_move_preview_and_commit_equal() {
    for colors in [vec![0], vec![0, 2]] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 16})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.session
            .edit("opaque colour samples", |doc, active| {
                let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                surface.fill_rect(Rect::new(0, 0, 32, 16), &[0.2, 0.4, 0.6, 1.0]);
                surface.fill_rect(Rect::new(28, 4, 32, 12), &[0.8, 0.5, 0.1, 1.0]);
                Ok(())
            })
            .unwrap();
        app.run("channel.target", json!({"colors": colors})).unwrap();
        app.ui.tool = Tool::Move;
        app.ui.view.pattern_preview = true;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        let id = app.session.active().unwrap().active_layer.unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let mut expected = photocraft_engine::Session::new();
        expected.tools.background = app.session.tools.background;
        expected.add_document((*before).clone(), None);
        expected.execute("layer.translate", json!({"layer": id.0, "target": {"colors": colors}, "dx": 6, "dy": 0, "wrap": true})).unwrap();
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 30.0, y: 8.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 36.0, y: 8.0, pressure: 1.0 }, m);
        let (shown, _) = super::super::display_doc(&mut app, 0).unwrap();
        assert_eq!(shown.layer(id), expected.active().unwrap().doc.layer(id));
        app.ui.view.pattern_preview = false;
        assert_ne!(super::super::display_doc(&mut app, 0).unwrap().0.layer(id), shown.layer(id), "the unwrapped view must differ");
        app.ui.view.pattern_preview = true;
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 36.0, y: 8.0 }, m);
        assert_eq!(app.session.active().unwrap().doc.layer(id), shown.layer(id));
        assert!(app.session.undo());
        assert_eq!(app.session.active().unwrap().doc.layers, before.layers);
    }
}

#[test]
fn selected_single_channel_copy_gestures_refuse_before_cutting_or_duplicating_rgba() {
    for tool in [
        Tool::Move,
        Tool::RectMarquee,
        Tool::EllipseMarquee,
        Tool::SingleRowMarquee,
        Tool::SingleColumnMarquee,
        Tool::Lasso,
        Tool::PolygonLasso,
        Tool::MagicWand,
    ] {
        let (mut app, _) = selected_pixels(16);
        app.run("channel.target", json!({"channel":"red"})).unwrap();
        app.ui.tool = tool;
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        let modifiers = egui::Modifiers { alt: true, command: tool != Tool::Move, mac_cmd: tool != Tool::Move, ..Default::default() };
        let response = crate::canvas::tool_event_response(&mut app, ToolEvent::Move { x: 16.0, y: 16.0, pressure: 1.0 }, modifiers);
        assert_eq!(response.cursor, Some(craft::behavior::CursorIcon::Default.into()), "{tool:?}: do not offer an unsupported copy");
        drag(&mut app, [16.0, 16.0], [26.0, 21.0], modifiers);
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "{tool:?}");
        assert!(app.ui.status_error, "{tool:?}: explain the unsupported copy gesture");
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
        assert!(photocraft_engine::float_cmds::floating(app.session.active().unwrap()).is_none());
        crate::move_mods::nudge(&mut app, 1.0, 0.0, true);
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
    }
}

#[test]
fn cancelling_a_native_selection_tool_drag_releases_its_preview_without_editing() {
    let (mut app, _) = selected_pixels(16);
    app.run("channel.target", json!({"colors":[0,2]})).unwrap();
    app.ui.tool = Tool::RectMarquee;
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    let m = egui::Modifiers::COMMAND;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 26.0, y: 21.0, pressure: 1.0 }, m);
    assert!(super::super::display_doc(&mut app, 0).is_some());
    assert!(app.move_preview.is_some());
    crate::canvas::testing::key(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
    assert!(app.move_preview.is_none());
    assert!(app.move_gpu_preview.is_none());
    assert!(app.canvas_tools.selection.pixel_drag_offset().is_none());
    assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
}

mod gpu_tests {
    use super::*;

    #[test]
    fn gpu_channel_move_matches_native_commit_and_tracks_visibility() {
        let _gpu_test = photocraft_testkit::gpu_lock();
        let Ok(rs) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())
        })) else {
            eprintln!("no GPU adapter for Move preview readback");
            return;
        };
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for colors in [vec![0], vec![0, 2]] {
                let mut doc = Document::with_background("Move GPU oracle", Size::new(320, 32), ColorMode::Rgb, depth, Color::WHITE);
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                app.session.add_document(doc.clone(), None);
                app.session.tools.background = [0.35, 0.65, 0.85, 1.0];
                app.sync_views();
                app.session.execute("layer.new.layer", json!({})).unwrap();
                let layer = app.session.active().unwrap().active_layer.unwrap();
                app.session
                    .edit("opaque painted layer", |d, _| {
                        let s = d.layer_mut(layer).unwrap().surface_mut().unwrap();
                        let base = if depth == SampleType::F32 { [0.1, -0.2, 1.8, 1.0] } else { [0.1, 0.3, 0.5, 1.0] };
                        let square = if depth == SampleType::F32 { [2.25, 0.7, -0.1, 1.0] } else { [0.9, 0.7, 0.2, 1.0] };
                        s.fill_rect(Rect::new(0, 0, 320, 32), &base);
                        s.fill_rect(Rect::new(250, 4, 270, 28), &square);
                        Ok(())
                    })
                    .unwrap();
                app.session.execute("channel.target", json!({"colors":colors})).unwrap();
                app.ui.tool = Tool::Move;
                app.ui.extras.snap = false;
                app.ui.view.show.smart_guides = false;

                if !crate::channel_transform_gpu::test_supported(&rs) {
                    return;
                }
                app.set_wgpu(rs.clone());
                let ctx = app.ctx().clone();
                let navigator = crate::canvas::navigator_texture(&mut app, &ctx, 0).unwrap();
                let m = egui::Modifiers::NONE;
                crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 260.0, y: 16.0, pressure: 1.0 }, m);
                let mut cached_texture = None;
                let before = app.session.active().unwrap().doc.clone();
                crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 100_000_260.0, y: 16.0, pressure: 1.0 }, m);
                assert!(crate::move_ui::channel_texture(&mut app, 0, None).is_none(), "native work-area overflow cannot return a GPU preview");
                assert!(!crate::move_ui::gpu_showing(&app, 0));
                assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
                for (dx, visible) in [(1, false), (17, true), (-3, false)] {
                    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 260.0 + f64::from(dx), y: 16.0, pressure: 1.0 }, m);
                    for name in ["red", "green", "blue"] {
                        app.session.execute("channel.setVisible", json!({"channel":name,"visible":visible || name == "red"})).unwrap();
                    }
                    let tex = crate::channel_transform_gpu::test_wait(&rs, || crate::move_ui::channel_texture(&mut app, 0, None));
                    assert!(
                        tex.is_some(),
                        "GPU path at {depth:?}, offset {dx}; drag={:?}, stack={:?}, source_cached={}",
                        crate::move_ui::drag_offset(&app),
                        app.session.active().unwrap().doc.layers.iter().map(|l| (l.id, l.visible, l.surface().is_some())).collect::<Vec<_>>(),
                        app.move_gpu_preview.as_ref().is_some_and(|p| p.gpu.is_some())
                    );
                    if let Some(previous) = cached_texture {
                        assert_eq!(tex, Some(previous), "eye changes and new offsets reuse the captured GPU source");
                    }
                    cached_texture = tex;
                    app.move_preview = None;
                    assert_eq!(crate::canvas::navigator_texture(&mut app, &ctx, 0), Some(navigator));
                    assert!(app.move_preview.is_none(), "the Navigator must not force a CPU resample during native GPU dragging");
                    if dx == 17 {
                        assert!(
                            crate::channel_transform_gpu::test_wait(&rs, || crate::move_ui::channel_texture(&mut app, 0, Some(999))).is_none(),
                            "another display uses its own CPU conversion"
                        );
                        assert!(crate::move_ui::gpu_showing(&app, 0), "a secondary display does not hide the main GPU preview");
                        let secondary = crate::move_ui::display_doc(&mut app, 0).unwrap().0;
                        crate::channel_transform_gpu::test_wait(&rs, || crate::move_ui::channel_texture(&mut app, 0, None)).unwrap();
                        let again = crate::move_ui::display_doc(&mut app, 0).unwrap().0;
                        assert!(std::sync::Arc::ptr_eq(&secondary, &again), "main GPU frames must retain the secondary CPU cache");

                        assert!(
                            crate::channel_transform_gpu::test_wait(&rs, || crate::move_ui::channel_texture(&mut app, 0, None)).is_some(),
                            "main display keeps its GPU source"
                        );
                    }
                    assert!(crate::move_ui::gpu_showing(&app, 0));
                    let actual = app.move_gpu_preview.as_ref().unwrap().gpu.as_ref().unwrap().readback();
                    doc = (*app.session.active().unwrap().doc).clone();
                    let mut moved = doc.clone();
                    photocraft_engine::channel_cmds::move_color_channels(
                        &mut moved,
                        layer,
                        photocraft_engine::color_channels::ColorChannels::from_indices(colors.iter().copied(), 3).unwrap(),
                        dx,
                        0,
                        app.session.tools.background,
                    )
                    .unwrap();
                    let view = &app.session.active().unwrap().channel_view;
                    let expected = crate::channel_view::render_for_display(
                        &moved,
                        view,
                        moved.bounds(),
                        1,
                        crate::channel_view::Look::default(),
                        None,
                        &crate::channel_view::Screen::default(),
                    )
                    .filter(|pixels| pixels.iter().all(|p| p.a() == 255))
                    .unwrap_or_else(|| crate::canvas::display_image(None, &photocraft_compose::flatten(&moved)).pixels);
                    let max = actual
                        .iter()
                        .zip(&expected)
                        .flat_map(|(a, b)| a.to_array().into_iter().zip(b.to_array()).map(|(x, y)| x.abs_diff(y)))
                        .max()
                        .unwrap_or(0);
                    assert!(max <= 1, "{depth:?}, offset {dx}, visible {visible}: max display difference {max}");
                }
                app.set_wgpu(rs.clone());
                assert!(app.move_gpu_preview.is_none(), "replacing the GPU drops textures from the old device");
                assert!(
                    crate::channel_transform_gpu::test_wait(&rs, || crate::move_ui::channel_texture(&mut app, 0, None)).is_some(),
                    "the current drag can recapture on the new GPU"
                );
                crate::gpu_status::fall_back(&mut app, &photocraft_gpu::Fault::Error("Validation Error".into()));
                assert!(app.move_gpu_preview.is_none(), "CPU fallback releases the GPU drag capture");
                // The new shared recovery prompt owns pointer input until dismissed.
                let ctx = app.ctx().clone();
                for dialog in app.framework.dialogs.open_dialogs() {
                    app.framework.dialogs.cancel_by_id(&ctx, dialog.id);
                }
                app.set_wgpu(rs.clone());
                assert!(
                    crate::channel_transform_gpu::test_wait(&rs, || crate::move_ui::channel_texture(&mut app, 0, None)).is_some(),
                    "retry preserves the active drag"
                );
                crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 257.0, y: 16.0 }, m);
                assert!(app.move_gpu_preview.is_none(), "release drops cached GPU source");
                assert_eq!(
                    app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().pixel(260, 16)[1],
                    doc.layer(layer).unwrap().surface().unwrap().pixel(260, 16)[1]
                );
                // A translucent top pixel reveals Background. The one-layer shader must decline
                // that stack and keep the normal CPU preview, including on a reduced source.
                app.session
                    .edit("translucent top", |d, _| {
                        d.layer_mut(layer).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 1, 1), &[0.0, 0.0, 0.0, 0.5]);
                        Ok(())
                    })
                    .unwrap();
                crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 260.0, y: 16.0, pressure: 1.0 }, m);
                crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 277.0, y: 16.0, pressure: 1.0 }, m);
                assert!(crate::channel_transform_gpu::test_wait(&rs, || crate::move_ui::channel_texture(&mut app, 0, None)).is_none());
                assert!(crate::move_ui::display_doc(&mut app, 0).is_some(), "transparent stack keeps its CPU preview");
                app.canvas_tools.mover = Default::default();
                app.sync_views();
                assert!(app.move_gpu_preview.is_none(), "cancel releases the cached GPU source");
            }
        }
    }

    #[test]
    fn gpu_move_fills_only_the_sparse_source_bounds() {
        let _gpu_test = photocraft_testkit::gpu_lock();
        let Ok(rs) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())
        })) else {
            eprintln!("no GPU adapter for sparse Move preview readback");
            return;
        };
        let mut doc = Document::new("sparse Move", Size::new(320, 32), ColorMode::Rgb, SampleType::U8);
        let mut layer = Layer::raster("sparse", doc.pixel_format());
        layer.surface_mut().unwrap().fill_rect(Rect::new(245, 4, 271, 28), &[0.8, 0.3, 0.1, 1.0]);
        let id = layer.id;
        doc.layers.push(layer);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.add_document(doc.clone(), None);
        app.sync_views();
        app.session.execute("channel.target", json!({"channel":"red"})).unwrap();
        for channel in ["red", "green", "blue"] {
            app.session.execute("channel.setVisible", json!({"channel":channel,"visible":true})).unwrap();
        }
        app.ui.tool = Tool::Move;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;

        if !crate::channel_transform_gpu::test_supported(&rs) {
            return;
        }
        app.set_wgpu(rs.clone());
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 255.0, y: 16.0, pressure: 1.0 }, m);
        for dx in [10, -7, 28] {
            crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 255.0 + f64::from(dx), y: 16.0, pressure: 1.0 }, m);
            assert!(crate::channel_transform_gpu::test_wait(&rs, || crate::move_ui::channel_texture(&mut app, 0, None)).is_some());
            let actual = app.move_gpu_preview.as_ref().unwrap().gpu.as_ref().unwrap().readback();
            let mut moved = doc.clone();
            photocraft_engine::channel_cmds::move_color_channel(&mut moved, id, 0, dx, 0, app.session.tools.background).unwrap();
            let expected = crate::canvas::display_image(None, &photocraft_compose::flatten(&moved)).pixels;
            let max = actual.iter().zip(&expected).flat_map(|(a, b)| a.to_array().into_iter().zip(b.to_array()).map(|(x, y)| x.abs_diff(y))).max().unwrap_or(0);
            assert!(max <= 1, "sparse Move at {dx}: max display difference {max}");
        }
    }

    #[test]
    fn switching_colour_sets_during_a_move_recaptures_the_gpu_source() {
        let _lock = photocraft_testkit::gpu_lock();
        let Ok(rs) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())
        })) else {
            return;
        };
        if !crate::channel_transform_gpu::test_supported(&rs) {
            return;
        }
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":32,"height":24})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color":"#407fc0"})).unwrap();
        app.run("channel.target", json!({"channel":"red"})).unwrap();
        app.sync_views();
        app.set_wgpu(rs.clone());
        app.ui.tool = Tool::Move;
        app.ui.extras.snap = false;
        app.ui.tool_options.move_auto_select = false;
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 8.0, y: 8.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 10.0, y: 9.0, pressure: 1.0 }, m);
        let old = crate::channel_transform_gpu::test_wait(&rs, || crate::move_ui::channel_texture(&mut app, 0, None)).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        let next = crate::channel_transform_gpu::test_wait(&rs, || crate::move_ui::channel_texture(&mut app, 0, None)).unwrap();
        assert_ne!(old, next, "changing targets during an active drag must replace the captured preview");
        assert_eq!(app.move_gpu_preview.as_ref().unwrap().colors.indices().collect::<Vec<_>>(), vec![0, 2]);
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    }
}
