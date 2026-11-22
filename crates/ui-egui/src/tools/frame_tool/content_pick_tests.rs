//! Frame drags pick content under the drag independently of the active layer (#13860).
use super::*;
use crate::canvas::ToolEvent;

fn raster(name: &str, bounds: Rect, format: photocraft_color::PixelFormat) -> Layer {
    let mut layer = Layer::raster(name, format);
    layer.surface_mut().unwrap().fill_rect(bounds, &[1., 0., 0., 1.]);
    layer
}

fn scene() -> (Box<PhotocraftApp>, LayerId, LayerId, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":600,"height":400})).unwrap();
    let ids = app
        .session
        .edit("Sources", |doc, active| {
            let red = raster("Red", Rect::new(50, 50, 150, 130), doc.pixel_format());
            let blue = raster("Blue", Rect::new(300, 200, 420, 260), doc.pixel_format());
            let empty = Layer::raster("Empty", doc.pixel_format());
            let ids = (red.id, blue.id, empty.id);
            doc.layers.extend([red, blue, empty]);
            *active = Some(ids.2);
            Ok(ids)
        })
        .unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Frame;
    (app, ids.0, ids.1, ids.2)
}

#[test]
fn topmost_overlapping_content_is_independent_of_active_layer() {
    let (mut app, red, blue, empty) = scene();
    for active in [empty, blue] {
        app.run("layer.select", json!({"layer":active.0})).unwrap();
        assert_eq!(content_layer(&app, [80, 60, 200, 160]).unwrap(), Some(red.0));
        assert_eq!(content_layer(&app, [40, 40, 450, 300]).unwrap(), Some(blue.0));
        assert_eq!(content_layer(&app, [450, 300, 550, 350]).unwrap(), None);
    }
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1. }, m);
    crate::canvas::tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1. }, m);
    crate::canvas::tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

#[test]
fn hidden_locked_and_nonimage_candidates_are_skipped_but_lower_eligible_content_is_found() {
    for kind in ["hidden", "all", "position", "text", "shape", "hidden group", "locked group"] {
        let (mut app, red, _, _) = scene();
        app.session
            .edit("Top candidate", |doc, _| {
                let mut top = raster("Top", Rect::new(50, 50, 150, 130), doc.pixel_format());
                match kind {
                    "hidden" => top.visible = false,
                    "all" => top.locks.all = true,
                    "position" => top.locks.position = true,
                    "text" => {
                        top.content = LayerContent::Text(photocraft_doc::TextLayer { cache: top.surface().cloned(), ..Default::default() });
                    }
                    "shape" => {
                        top.content = LayerContent::Shape(photocraft_doc::ShapeLayer { cache: top.surface().cloned(), ..Default::default() });
                    }
                    _ => {
                        top = Layer::group("Parent", vec![top]);
                        top.visible = kind != "hidden group";
                        top.locks.all = kind == "locked group";
                    }
                }
                doc.layers.push(top);
                Ok(())
            })
            .unwrap();
        assert_eq!(content_layer(&app, [80, 60, 200, 160]).unwrap(), Some(red.0), "{kind} must not obscure eligible Red");
        app.session
            .edit("Remove Red", |doc, _| {
                doc.remove(red).unwrap();
                Ok(())
            })
            .unwrap();
        assert_eq!(content_layer(&app, [80, 60, 200, 160]).unwrap(), None, "{kind} must not become frame content");
    }
    let (app, _, _, _) = scene();
    assert_eq!(content_layer(&app, [0, 0, 40, 40]).unwrap(), None, "Background alone is ineligible");
}

#[test]
fn real_drag_takes_nested_content_preserves_its_parent_and_is_one_undo_step() {
    let (mut app, red, _, empty) = scene();
    let group = app
        .session
        .edit("Nest Red", |doc, active| {
            let red = doc.remove(red).unwrap();
            let group = Layer::group("Outer", vec![Layer::group("Inner", vec![red])]);
            let inner = group.children().unwrap()[0].id;
            doc.layers.push(group);
            *active = Some(empty);
            Ok(inner)
        })
        .unwrap();
    let before = app.session.active().unwrap().history.entries().len();
    drag(&mut app, [80., 60.], [200., 160.]);
    let st = app.session.active().unwrap();
    let frame = st.doc.walk().into_iter().find_map(|(_, _, l)| l.frame.as_ref().map(|_| l)).unwrap();
    assert_eq!(frame.children().unwrap()[0].id, red);
    assert_eq!(st.doc.layer(group).unwrap().children().unwrap()[0].id, frame.id);
    assert_eq!(st.history.entries().len(), before + 1);
    assert_eq!(st.history.entries().last().map(String::as_str), Some("New Frame"));
    app.run("edit.undo", json!({})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(group).unwrap().children().unwrap()[0].id, red);
    assert!(!st.doc.walk().iter().any(|(_, _, l)| l.frame.is_some()));
}

#[test]
fn smart_content_is_taken_and_existing_frame_children_are_left_in_their_frame() {
    let (mut app, red, _, _) = scene();
    let v = app.run("layer.smartObjects.convertToSmartObject", json!({"layer":red.0})).unwrap();
    let smart = LayerId(v["layer"].as_u64().unwrap());
    assert_eq!(content_layer(&app, [80, 60, 200, 160]).unwrap(), Some(smart.0));
    drag(&mut app, [80., 60.], [200., 160.]);
    let st = app.session.active().unwrap();
    let frame = st.doc.walk().into_iter().find_map(|(_, _, l)| l.frame.as_ref().map(|_| l)).unwrap();
    assert_eq!(frame.children().unwrap()[0].id, smart);
    assert_eq!(content_layer(&app, [80, 60, 200, 160]).unwrap(), None, "the Smart image stays in its original frame");
}

#[test]
fn nonoverlap_drag_makes_one_empty_frame_even_with_a_raster_active() {
    let (mut app, red, _, _) = scene();
    app.run("layer.select", json!({"layer":red.0})).unwrap();
    let before = app.session.active().unwrap().history.entries().len();
    drag(&mut app, [450., 300.], [550., 350.]);
    let st = app.session.active().unwrap();
    let frame = st.doc.walk().into_iter().find_map(|(_, _, l)| l.frame.as_ref().map(|_| l)).unwrap();
    assert!(frame.children().unwrap().is_empty());
    assert!(st.doc.layer(red).is_some());
    assert_eq!(st.history.entries().len(), before + 1);
    assert_eq!(st.history.entries().last().map(String::as_str), Some("New Frame"));
}

#[test]
fn a_large_selection_does_not_make_upper_raster_pixels_overlap_the_drag() {
    let (mut app, red, blue, _) = scene();
    app.run("select.rect", json!({"x":0,"y":0,"width":600,"height":400})).unwrap();
    assert_eq!(content_layer(&app, [80, 60, 200, 160]).unwrap(), Some(red.0), "Blue's selection rectangle is not its content");
    assert_eq!(content_layer(&app, [40, 40, 450, 300]).unwrap(), Some(blue.0));
    assert_eq!(content_layer(&app, [450, 300, 550, 350]).unwrap(), None, "unpainted selected space is not a candidate");
    let history = app.session.active().unwrap().history.entries();
    drag(&mut app, [80., 60.], [200., 160.]);
    let st = app.session.active().unwrap();
    let frame = st.doc.walk().into_iter().find_map(|(_, _, l)| l.frame.as_ref().map(|_| l)).unwrap();
    assert_eq!(frame.children().unwrap()[0].id, red);
    assert!(matches!(st.doc.layer(blue).unwrap().content, LayerContent::Raster(_)));
    assert_eq!(st.history.entries().len(), history.len() + 1);
    assert_eq!(st.history.entries().last().map(String::as_str), Some("New Frame"));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().history.entries(), history);
}

#[test]
fn raster_selection_limits_content_without_replacing_its_offset_bounds() {
    let (mut app, red, blue, _) = scene();
    app.run("select.rect", json!({"x":80,"y":60,"width":30,"height":30})).unwrap();
    // Red is 100x80, so the frame must be at least ~34x27; overlap still follows the selection.
    assert_eq!(content_layer(&app, [80, 60, 114, 88]).unwrap(), Some(red.0));
    assert_eq!(content_layer(&app, [110, 50, 150, 130]).unwrap(), None, "unselected pixels are outside the raster candidate bounds");
    assert_eq!(content_layer(&app, [310, 210, 350, 250]).unwrap(), None);
    app.run("select.rect", json!({"x":500,"y":350,"width":40,"height":40})).unwrap();
    assert_eq!(content_layer(&app, [500, 350, 540, 390]).unwrap(), None);
    app.run("select.deselect", json!({})).unwrap();
    assert_eq!(content_layer(&app, [80, 60, 114, 88]).unwrap(), Some(red.0));
    assert_eq!(content_layer(&app, [310, 210, 350, 250]).unwrap(), Some(blue.0));
}

#[test]
fn smart_candidate_bounds_remain_whole_when_a_pixel_selection_is_elsewhere() {
    let (mut app, red, blue, empty) = scene();
    app.run("layer.select", json!({"layer":blue.0})).unwrap();
    let result = app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let smart = LayerId(result["layer"].as_u64().unwrap());
    app.run("layer.select", json!({"layer":empty.0})).unwrap();
    app.run("select.rect", json!({"x":80,"y":60,"width":30,"height":30})).unwrap();
    assert_eq!(content_layer(&app, [310, 210, 350, 250]).unwrap(), Some(smart.0));
    assert_eq!(content_layer(&app, [80, 60, 130, 110]).unwrap(), Some(red.0));
    assert_eq!(content_layer(&app, [500, 350, 550, 390]).unwrap(), None);
}

#[test]
fn rendered_empty_fallback_does_not_prevent_a_real_drag_from_making_a_frame() {
    for lower_content in [false, true] {
        let (mut app, red, blue, _) = scene();
        let (masked, large) = app
            .session
            .edit("Fallback candidates", |doc, _| {
                if !lower_content {
                    doc.remove(red).unwrap();
                }
                let mut masked = raster("Masked", Rect::new(50, 50, 150, 130), doc.pixel_format());
                masked.mask = Some(photocraft_doc::LayerMask::hide_all());
                let large = raster("Large", Rect::new(0, 0, 600, 400), doc.pixel_format());
                let ids = (masked.id, large.id);
                doc.layers.extend([masked, large]);
                Ok(ids)
            })
            .unwrap();
        let expected = lower_content.then_some(red);
        let picked = content_layer(&app, [80, 60, 120, 100]).unwrap();
        let history = app.session.active().unwrap().history.entries();
        assert!(!app.ui.status_error);
        drag(&mut app, [80., 60.], [120., 100.]);
        assert!(!app.ui.status_error, "the real drag must not attempt a zero-alpha conversion: {}", app.ui.status);
        assert_eq!(picked, expected.map(|id| id.0));
        let st = app.session.active().unwrap();
        let frames: Vec<_> = st.doc.walk().into_iter().filter_map(|(_, _, l)| l.frame.as_ref().map(|_| l)).collect();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].children().unwrap().first().map(|l| l.id), expected);
        assert_eq!(frames[0].children().unwrap().len(), usize::from(lower_content));
        for id in [masked, large, blue] {
            assert!(matches!(st.doc.layer(id).unwrap().content, LayerContent::Raster(_)));
        }
        assert!(st.doc.layer(masked).unwrap().mask.as_ref().unwrap().enabled);
        assert_eq!(st.history.entries().len(), history.len() + 1);
        assert_eq!(st.history.entries().last().map(String::as_str), Some("New Frame"));
        app.run("edit.undo", json!({})).unwrap();
        let st = app.session.active().unwrap();
        assert_eq!(st.history.entries(), history);
        assert!(!st.doc.walk().iter().any(|(_, _, l)| l.frame.is_some()));
        assert_eq!(st.doc.layer(red).is_some(), lower_content);
        for id in [masked, large, blue] {
            assert!(matches!(st.doc.layer(id).unwrap().content, LayerContent::Raster(_)));
        }
    }
}

#[test]
fn zero_opacity_raster_still_qualifies_for_the_real_frame_capture() {
    let (mut app, red, _, _) = scene();
    app.session
        .edit("Transparent source opacity", |doc, _| {
            doc.layer_mut(red).unwrap().opacity = 0.;
            doc.layers.push(raster("Large", Rect::new(0, 0, 600, 400), doc.pixel_format()));
            Ok(())
        })
        .unwrap();
    let history = app.session.active().unwrap().history.entries();
    let picked = content_layer(&app, [80, 60, 120, 100]).unwrap();
    assert!(!app.ui.status_error);
    drag(&mut app, [80., 60.], [120., 100.]);
    assert!(!app.ui.status_error, "conversion normalizes opacity while rendering: {}", app.ui.status);
    assert_eq!(picked, Some(red.0));
    let st = app.session.active().unwrap();
    let frames: Vec<_> = st.doc.walk().into_iter().filter_map(|(_, _, l)| l.frame.as_ref().map(|_| l)).collect();
    assert_eq!(frames.len(), 1);
    let child = &frames[0].children().unwrap()[0];
    assert_eq!(child.id, red);
    assert!(matches!(child.content, LayerContent::Smart(_)));
    assert_eq!(child.opacity, 0.);
    assert_eq!(st.history.entries().len(), history.len() + 1);
    app.run("edit.undo", json!({})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.history.entries(), history);
    assert!(matches!(st.doc.layer(red).unwrap().content, LayerContent::Raster(_)));
    assert_eq!(st.doc.layer(red).unwrap().opacity, 0.);
    assert!(!st.doc.walk().iter().any(|(_, _, l)| l.frame.is_some()));
}

#[test]
fn render_refusal_remains_an_error_without_creating_an_empty_frame() {
    let (mut app, red, _, _) = scene();
    app.session
        .edit("Sparse oversized canvas", |doc, _| {
            // This changes only logical size: render refuses the region before allocating its pixels.
            doc.size = photocraft_geom::Size::new(32769, 32768);
            Ok(())
        })
        .unwrap();
    let st = app.session.active().unwrap();
    let history = st.history.entries();
    let expected = photocraft_engine::smart_cmds::layer_to_smart(&st.doc, st.doc.layer(red).unwrap()).unwrap_err().to_string();
    assert_eq!(content_layer(&app, [80, 60, 200, 160]).unwrap_err(), expected);
    assert!(!app.ui.status_error);
    // Exercise the production commit/error route without painting the deliberately huge canvas.
    make_frame(&mut app, KRect::new(80., 60., 200., 160.));
    assert!(app.ui.status_error);
    assert_eq!(app.ui.status, expected);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.entries(), history);
    assert!(matches!(st.doc.layer(red).unwrap().content, LayerContent::Raster(_)));
    assert!(!st.doc.walk().iter().any(|(_, _, l)| l.frame.is_some()));
}
