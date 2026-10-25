use super::*;
use crate::canvas::{ToolEvent, tool_event};
use photocraft_doc::ShapeLayer;
fn fixture(kind: &str, tool: Tool) -> (PhotocraftApp, LayerId, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":220,"height":180,"background":"white"})).unwrap();
    let neighbour = LayerId(app.run("shape.create", json!({"rect":[140,30,40,40],"fill":"#0000ff"})).unwrap()["layer"].as_u64().unwrap());
    let id = LayerId(
        app.run(
            "shape.create",
            json!({"kind":kind,"rect":[20,30,70,50],"radii":if kind=="roundedRect" {[8,16,12,4]} else {[0;4]},"fill":"#ff0000","stroke":{"width":3}}),
        )
        .unwrap()["layer"]
            .as_u64()
            .unwrap(),
    );
    app.ui.tool = tool;
    app.ui.selected_path = Some("layer".into());
    app.ui.tool_options.move_auto_select = false;
    app.ui.extras.snap = false;
    app.ui.view.extras = true;
    app.ui.view.show.target_path = true;
    app.sync_views();
    (app, id, neighbour)
}
fn shape(doc: &Document, id: LayerId) -> ShapeLayer {
    match &doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => sh.clone(),
        _ => panic!("shape fixture"),
    }
}
fn down(app: &mut PhotocraftApp) {
    tool_event(app, ToolEvent::Down { x: 45.0, y: 50.0, pressure: 1.0 }, egui::Modifiers::NONE);
}
fn movement(app: &mut PhotocraftApp, d: [f64; 2]) {
    tool_event(app, ToolEvent::Move { x: 45.0 + d[0], y: 50.0 + d[1], pressure: 1.0 }, egui::Modifiers::NONE);
}
fn up(app: &mut PhotocraftApp, d: [f64; 2]) {
    tool_event(app, ToolEvent::Up { x: 45.0 + d[0], y: 50.0 + d[1] }, egui::Modifiers::NONE);
}
#[test]
fn live_rect_ellipse_and_rounded_shape_match_one_fractional_release_and_undo_redo() {
    for tool in [Tool::PathSelection, Tool::Move] {
        for kind in ["rect", "ellipse", "roundedRect"] {
            let (mut app, id, other) = fixture(kind, tool);
            let original = app.session.active().unwrap().doc.clone();
            let history = app.session.active().unwrap().history.past_len();
            down(&mut app);
            assert!(active(&app));
            // Pointer-down selects a layer (revision changes without a document/history edit).
            assert_eq!(app.session.active().unwrap().doc, original);
            assert_eq!(app.session.active().unwrap().history.past_len(), history);
            let revision = app.session.active().unwrap().revision;
            let mut last = None;
            for d in [[1.125, 2.25], [4.375, -3.125], [7.25, 6.375]] {
                movement(&mut app, d);
                let (shown, key) = display_doc(&mut app, 0).unwrap();
                let mut expected = shape(&original, id);
                photocraft_engine::vector_cmds::transform_shape(&mut expected, &Affine::translate(d[0], d[1]));
                let actual = shape(&shown, id);
                assert_eq!(actual.path, expected.path);
                assert_eq!(actual.live, expected.live);
                assert_eq!(actual.fill, expected.fill);
                assert_eq!(actual.stroke, expected.stroke);
                assert_eq!(shown.layer(other), original.layer(other));
                assert_eq!(app.session.active().unwrap().doc, original);
                assert_eq!(app.session.active().unwrap().revision, revision);
                assert_eq!(app.session.active().unwrap().history.past_len(), history);
                assert_eq!(display_doc(&mut app, 0).unwrap().1, key, "unchanged frame is cached");
                last = Some(shown);
            }
            let shown = last.unwrap();
            up(&mut app, [7.25, 6.375]);
            assert!(!active(&app));
            let committed = app.session.active().unwrap().doc.clone();
            assert_eq!(shape(&committed, id), shape(&shown, id), "preview and final fill/stroke/cache agree");
            assert_eq!(photocraft_compose::flatten(&committed), photocraft_compose::flatten(&shown));
            assert_eq!(committed.layer(other), original.layer(other));
            assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(app.session.active().unwrap().doc, original);
            app.run("edit.redo", json!({})).unwrap();
            assert_eq!(app.session.active().unwrap().doc, committed);
        }
    }
}
#[test]
fn subpixel_motion_is_not_throttled_by_paint_sample_distance() {
    let (mut app, id, _) = fixture("rect", Tool::PathSelection);
    let before = app.session.active().unwrap().doc.clone();
    down(&mut app);
    movement(&mut app, [0.125, 0.0625]);
    let (shown, _) = display_doc(&mut app, 0).unwrap();
    let a = shape(&before, id).path.subpaths[0].knots[0].anchor;
    let b = shape(&shown, id).path.subpaths[0].knots[0].anchor;
    assert_eq!([b.x - a.x, b.y - a.y], [0.125, 0.0625]);
    up(&mut app, [0.125, 0.0625]);
    assert_eq!(shape(&app.session.active().unwrap().doc, id), shape(&shown, id));
}
#[test]
fn escape_uses_real_shortcut_and_leaves_no_history_or_late_release_mutation() {
    for tool in [Tool::PathSelection, Tool::Move] {
        let (mut app, _, _) = fixture("ellipse", tool);
        let doc = app.session.active().unwrap().doc.clone();
        let n = app.session.active().unwrap().history.past_len();
        down(&mut app);
        movement(&mut app, [12.5, 7.25]);
        display_doc(&mut app, 0).unwrap();
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        let input = egui::RawInput {
            events: vec![
                egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
            ],
            ..Default::default()
        };
        ctx.run_ui(input, |ui| crate::shortcuts::handle(&mut app, ui.ctx())).textures_delta.clear();
        assert!(!active(&app));
        assert!(app.drag.is_none());
        up(&mut app, [12.5, 7.25]);
        assert_eq!(app.session.active().unwrap().doc, doc);
        assert_eq!(app.session.active().unwrap().history.past_len(), n);
    }
}
#[test]
fn locks_refuse_preview_and_late_release_does_not_fall_back() {
    for lock in ["all", "position", "pixels"] {
        let (mut app, _, _) = fixture("rect", Tool::PathSelection);
        app.run("layer.lockLayers", json!({(lock):true})).unwrap();
        let doc = app.session.active().unwrap().doc.clone();
        let n = app.session.active().unwrap().history.past_len();
        down(&mut app);
        movement(&mut app, [9.25, 5.5]);
        assert!(display_doc(&mut app, 0).is_none());
        up(&mut app, [9.25, 5.5]);
        assert_eq!(app.session.active().unwrap().doc, doc);
        assert_eq!(app.session.active().unwrap().history.past_len(), n);
    }
}
#[test]
fn stale_revision_target_or_tool_cancels_without_touching_a_neighbour() {
    for mode in 0..4 {
        let (mut app, _, other) = fixture("roundedRect", Tool::PathSelection);
        down(&mut app);
        movement(&mut app, [7.25, 4.5]);
        display_doc(&mut app, 0).unwrap();
        match mode {
            0 => app.session.active_mut().unwrap().revision += 1,
            1 => {
                app.run("layer.select", json!({"layer":other.0})).unwrap();
            }
            2 => app.ui.tool = Tool::Brush,
            _ => app.ui.selected_path = Some("work".into()),
        }
        let doc = app.session.active().unwrap().doc.clone();
        reconcile(&mut app, true);
        assert!(!active(&app));
        up(&mut app, [7.25, 4.5]);
        assert_eq!(app.session.active().unwrap().doc, doc);
    }
}
#[test]
fn document_switch_focus_loss_and_lost_pointer_discard_preview() {
    for mode in 0..3 {
        let (mut app, _, _) = fixture("rect", Tool::PathSelection);
        let doc = app.session.active().unwrap().doc.clone();
        down(&mut app);
        movement(&mut app, [8.5, 6.25]);
        display_doc(&mut app, 0).unwrap();
        match mode {
            0 => {
                app.run("file.new", json!({"width":100,"height":100})).unwrap();
            }
            1 => {}
            _ => app.drag = None,
        }
        reconcile(&mut app, mode != 1);
        assert!(!active(&app));
        assert!(display_doc(&mut app, 0).is_none());
        up(&mut app, [8.5, 6.25]);
        assert_eq!(app.session.documents()[0].doc, doc);
    }
}
#[test]
fn ordinary_raster_and_multilayer_move_keep_existing_preview_and_commit() {
    let (mut app, id, other) = fixture("rect", Tool::Move);
    app.run("layer.select", json!({"layer":other.0,"mode":"add"})).unwrap();
    let original = app.session.active().unwrap().doc.clone();
    down(&mut app);
    assert!(!owns(&app));
    movement(&mut app, [8.0, 6.0]);
    let (shown, _) = crate::move_ui::display_doc(&mut app, 0).unwrap();
    up(&mut app, [8.0, 6.0]);
    assert_eq!(shape(&app.session.active().unwrap().doc, id).path, shape(&shown, id).path);
    assert_ne!(shape(&app.session.active().unwrap().doc, other).path, shape(&original, other).path);
    app.run("layer.new.layer", json!({})).unwrap();
    down(&mut app);
    assert!(!owns(&app));
}
#[test]
fn invalid_coordinates_or_zero_motion_do_not_create_history() {
    let (mut app, _, _) = fixture("rect", Tool::PathSelection);
    let doc = app.session.active().unwrap().doc.clone();
    let n = app.session.active().unwrap().history.past_len();
    down(&mut app);
    up(&mut app, [0.0, 0.0]);
    assert_eq!(app.session.active().unwrap().history.past_len(), n);
    down(&mut app);
    motion(&mut app, [f64::NAN, 0.0]);
    up(&mut app, [20.0, 0.0]);
    assert_eq!(app.session.active().unwrap().doc, doc);
    assert_eq!(app.session.active().unwrap().history.past_len(), n);
}
#[test]
fn overlay_document_contains_only_the_translated_shape_once() {
    let (mut app, id, other) = fixture("roundedRect", Tool::PathSelection);
    let original = app.session.active().unwrap().doc.clone();
    down(&mut app);
    movement(&mut app, [8.125, 5.5]);
    let (shown, _) = crate::canvas::display_doc(&mut app, 0);
    assert_eq!(shown.layers.len(), original.layers.len());
    assert_ne!(shape(&shown, id).path, shape(&original, id).path);
    assert_eq!(shown.layer(other), original.layer(other));
    assert_eq!(shown.walk().iter().filter(|(_, _, l)| l.id == id).count(), 1);
    let xf = crate::canvas::ViewXform {
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(440.0, 360.0)),
        zoom: 2.0,
        center: [110.0, 90.0],
        flip: false,
        rotation: 27.0,
        aspect: 1.2,
    };
    let paint = |doc: &Document| {
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::background());
            crate::vector_ui::draw_overlay(&app, &painter, &xf, doc);
        });
        output.textures_delta.clear();
        output.shapes.into_iter().map(|s| s.shape).collect::<Vec<_>>()
    };
    let a = paint(&original);
    let b = paint(&shown);
    let paths = |shapes: &[egui::Shape]| shapes.iter().filter_map(|s| if let egui::Shape::Path(p) = s { Some(p.clone()) } else { None }).collect::<Vec<_>>();
    let aa = paths(&a);
    let bb = paths(&b);
    assert_eq!(aa.len(), 1);
    assert_eq!(bb.len(), 1, "no ghost outline");
    assert_eq!(aa[0].points.len(), bb[0].points.len());
    let shift = xf.to_screen(8.125, 5.5) - xf.to_screen(0.0, 0.0);
    for (a, b) in aa[0].points.iter().zip(&bb[0].points) {
        assert!((*b - *a - shift).length() < 0.002);
    }
    let anchors =
        |shapes: &[egui::Shape]| shapes.iter().filter_map(|s| if let egui::Shape::Rect(r) = s { Some(r.rect.center()) } else { None }).collect::<Vec<_>>();
    let aa = anchors(&a);
    let bb = anchors(&b);
    // Eight anchors each produce two Rect records: a filled square and its border.
    assert_eq!(aa.len(), 16);
    assert_eq!(aa.len(), bb.len());
    for (a, b) in aa.iter().zip(&bb) {
        assert!((*b - *a - shift).length() < 0.002, "anchor follows outline");
    }
}
#[test]
fn shape_move_shift_constraint_uses_the_same_event_coordinates_for_preview_and_release() {
    let (mut app, id, _) = fixture("rect", Tool::Move);
    let before = shape(&app.session.active().unwrap().doc, id);
    down(&mut app);
    let mods = egui::Modifiers { shift: true, ..Default::default() };
    tool_event(&mut app, ToolEvent::Move { x: 64.25, y: 51.125, pressure: 1.0 }, mods);
    let (shown, _) = display_doc(&mut app, 0).unwrap();
    let p = shape(&shown, id).path.subpaths[0].knots[0].anchor;
    let q = before.path.subpaths[0].knots[0].anchor;
    assert!((p.y - q.y).abs() < 1e-6, "Shift keeps this nearly horizontal drag horizontal");
    tool_event(&mut app, ToolEvent::Up { x: 64.25, y: 51.125 }, mods);
    assert_eq!(shape(&app.session.active().unwrap().doc, id), shape(&shown, id));
}
#[test]
fn move_alt_and_bounding_box_preference_keep_their_existing_workflows() {
    let (mut app, _, _) = fixture("rect", Tool::Move);
    tool_event(&mut app, ToolEvent::Down { x: 45.0, y: 50.0, pressure: 1.0 }, egui::Modifiers::ALT);
    assert!(!owns(&app));
    cancel(&mut app);
    app.drag = None;
    let (mut app, _, _) = fixture("rect", Tool::Move);
    // Exercise the existing explicit preference without changing persistent preferences.
    app.session.edit_prefs(|p| p.interface.show_bounding_box_when_dragging_layer = true);
    down(&mut app);
    assert!(!owns(&app));
}

#[test]
fn a_new_non_shape_press_drops_a_refused_shapes_release_ownership() {
    let (mut app, _, _) = fixture("rect", Tool::PathSelection);
    app.run("layer.lockLayers", json!({"position":true})).unwrap();
    down(&mut app);
    assert!(owns(&app));
    assert!(!active(&app));
    app.ui.tool = Tool::RectMarquee;
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
    assert!(!owns(&app), "a late-release tombstone must not swallow a new gesture");
}

#[test]
fn paint_deferral_tracks_the_real_shape_target_and_never_a_raster_pick() {
    let (mut app, id, other) = fixture("rect", Tool::Move);
    assert!(defer_canvas(&app, Some([45.0, 50.0]), egui::Modifiers::NONE));
    app.ui.tool_options.move_auto_select = true;
    assert!(defer_canvas(&app, Some([150.0, 40.0]), egui::Modifiers::NONE), "Auto-Select another Shape is eligible");
    app.run("layer.new.layer", json!({})).unwrap();
    let raster = app.session.active().unwrap().active_layer.unwrap();
    app.session
        .edit("raster fixture", |doc, _| {
            doc.layer_mut(raster).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(30, 40, 60, 70), &[0.0, 1.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.run("layer.select", json!({"layer":id.0})).unwrap();
    assert!(!defer_canvas(&app, Some([45.0, 50.0]), egui::Modifiers::NONE), "Raster above selected Shape keeps legacy timing");
    app.run("layer.select", json!({"layer":raster.0})).unwrap();
    assert!(defer_canvas(&app, Some([150.0, 40.0]), egui::Modifiers::NONE), "Shape above selected Raster gets fresh timing");
    app.ui.tool_options.move_auto_select = false;
    assert!(!defer_canvas(&app, Some([150.0, 40.0]), egui::Modifiers::NONE), "Without Auto-Select, active Raster remains the target");
    app.run("layer.select", json!({"layer":id.0})).unwrap();
    app.run("layer.select", json!({"layer":other.0,"mode":"add"})).unwrap();
    assert!(!defer_canvas(&app, Some([150.0, 40.0]), egui::Modifiers::NONE), "Multi-layer Move retains legacy timing");
}

fn canvas_focus_frame(app: &mut PhotocraftApp, primary: bool, focused: bool) {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let input = egui::RawInput { screen_rect: Some(rect), focused, ..Default::default() };
    let view = app.ui.views[0].clone();
    ctx.run_ui(input, |ui| {
        crate::canvas::canvas_view(app, ui, 0, rect, view.clone(), primary);
    })
    .textures_delta
    .clear();
}

#[test]
fn secondary_canvas_focus_preserves_primary_shape_drag_and_single_history_step() {
    for tool in [Tool::PathSelection, Tool::Move] {
        let (mut app, id, _) = fixture("rect", tool);
        let original = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        down(&mut app);
        movement(&mut app, [7.25, 4.5]);
        canvas_focus_frame(&mut app, true, true);
        assert!(active(&app));
        canvas_focus_frame(&mut app, false, false);
        assert!(active(&app), "secondary canvas must not cancel a primary Shape gesture");
        assert_eq!(app.session.active().unwrap().doc, original);
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
        movement(&mut app, [9.375, 6.125]);
        let (shown, _) = display_doc(&mut app, 0).unwrap();
        canvas_focus_frame(&mut app, false, false);
        assert!(active(&app));
        up(&mut app, [9.375, 6.125]);
        assert_eq!(shape(&app.session.active().unwrap().doc, id), shape(&shown, id));
        assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc, original);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(shape(&app.session.active().unwrap().doc, id), shape(&shown, id));
    }
}

#[test]
fn primary_canvas_focus_loss_cancels_shape_drag_without_history_or_late_release() {
    for tool in [Tool::PathSelection, Tool::Move] {
        let (mut app, _, _) = fixture("rect", tool);
        let original = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        down(&mut app);
        movement(&mut app, [7.25, 4.5]);
        assert!(active(&app));
        canvas_focus_frame(&mut app, true, false);
        assert!(!active(&app));
        up(&mut app, [7.25, 4.5]);
        assert_eq!(app.session.active().unwrap().doc, original);
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
    }
}

#[test]
fn returning_to_press_point_clears_prepared_bounds_and_corner_offsets() {
    for tool in [Tool::PathSelection, Tool::Move] {
        let (mut app, id, _) = fixture("roundedRect", tool);
        let original = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        down(&mut app);
        movement(&mut app, [8.5, 6.25]);
        let (_, first_key) = display_doc(&mut app, 0).unwrap();
        assert_eq!(shown_offset(&app, id), [8.5, 6.25]);
        assert!(layer_edges(&app, 0, id).is_some());
        movement(&mut app, [0.0, 0.0]);
        assert!(display_doc(&mut app, 0).is_none());
        assert_eq!(shown_offset(&app, id), [0.0, 0.0]);
        assert!(layer_edges(&app, 0, id).is_none());
        assert_eq!(crate::canvas::display_doc(&mut app, 0).0, original);
        movement(&mut app, [8.5, 6.25]);
        let (shown, second_key) = display_doc(&mut app, 0).unwrap();
        assert_ne!(second_key, first_key, "a resumed nonzero offset must prepare fresh state");
        assert_ne!(shape(&shown, id).path, shape(&original, id).path);
        movement(&mut app, [0.0, 0.0]);
        assert!(display_doc(&mut app, 0).is_none());
        up(&mut app, [0.0, 0.0]);
        assert_eq!(app.session.active().unwrap().doc, original);
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
    }
}
