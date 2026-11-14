//! A small frame does not take a much larger candidate layer (#13861).
use super::*;

fn candidate(bounds: Rect) -> (PhotocraftApp, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":600,"height":400})).unwrap();
    let layer = app
        .session
        .edit("Source", |doc, active| {
            let mut layer = Layer::raster("Source", doc.pixel_format());
            layer.surface_mut().unwrap().fill_rect(bounds, &[1.0, 0.0, 0.0, 1.0]);
            let id = layer.id;
            doc.layers.push(layer);
            *active = Some(id);
            Ok(id)
        })
        .unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Frame;
    (app, layer)
}

#[test]
fn threshold_compares_each_dimension_instead_of_area_or_overlap() {
    let (app, layer) = candidate(Rect::new(50, 50, 250, 250));
    for (width, height, taken) in [(62, 62, false), (66, 66, false), (68, 68, true), (60, 140, false), (200, 20, false), (240, 76, true), (78, 240, true)] {
        assert_eq!(content_layer(&app, [60, 60, 60 + width, 60 + height]), taken.then_some(layer.0), "{width} x {height}");
    }
}

#[test]
fn small_canvas_drags_make_empty_frames_and_large_enough_drags_take_content_in_one_step() {
    for (bounds, side, taken) in [(Rect::new(50, 50, 150, 130), 20., false), (Rect::new(50, 50, 150, 130), 40., true), (Rect::new(0, 0, 600, 400), 100., false)]
    {
        let (mut app, layer) = candidate(bounds);
        let before = app.session.active().unwrap().history.entries().len();
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 60., y: 60., pressure: 1. }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 60. + side, y: 60. + side, pressure: 1. }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 60. + side, y: 60. + side }, m);
        let st = app.session.active().unwrap();
        let frame = st.doc.walk().into_iter().find_map(|(_, _, l)| l.frame.as_ref().map(|_| l)).unwrap();
        assert_eq!(frame.children().unwrap().first().map(|l| l.id), taken.then_some(layer));
        assert_eq!(st.history.entries().len(), before + 1);
        assert_eq!(st.history.entries().last().map(String::as_str), Some("New Frame"));
        assert!(st.doc.layer(layer).is_some(), "empty drags preserve the original layer");
        app.run("edit.undo", json!({})).unwrap();
        assert!(!app.session.active().unwrap().doc.walk().iter().any(|(_, _, l)| l.frame.is_some()));
    }
}

#[test]
fn degenerate_nonoverlapping_and_extreme_rectangles_are_safe() {
    let (app, layer) = candidate(Rect::new(50, 50, 250, 250));
    for r in [[60, 60, 60, 200], [200, 200, 60, 60], [250, 50, 350, 150], [500, 500, 600, 600]] {
        assert_eq!(content_layer(&app, r), None);
    }
    assert_eq!(content_layer(&app, [i64::MIN, i64::MIN, i64::MAX, i64::MAX]), Some(layer.0));
    assert_eq!(content_layer(&app, [i64::MAX, i64::MAX, i64::MIN, i64::MIN]), None);
    let (empty, _) = candidate(Rect::new(0, 0, 0, 0));
    assert_eq!(content_layer(&empty, [0, 0, 100, 100]), None);
}

#[test]
fn raster_threshold_uses_the_whole_layer_with_a_smaller_selection() {
    let (mut app, layer) = candidate(Rect::new(0, 0, 600, 400));
    app.run("select.rect", json!({"x":100,"y":100,"width":60,"height":60})).unwrap();
    assert_eq!(content_layer(&app, [100, 100, 120, 120]), None, "a selection must not shrink the layer-size threshold");
    assert_eq!(content_layer(&app, [80, 80, 280, 220]), Some(layer.0), "a frame large enough for the whole layer still qualifies");
    let history = app.session.active().unwrap().history.entries();
    let mods = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 100., y: 100., pressure: 1. }, mods);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 120., y: 120., pressure: 1. }, mods);
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 120., y: 120. }, mods);
    let st = app.session.active().unwrap();
    let frame = st.doc.walk().into_iter().find_map(|(_, _, l)| l.frame.as_ref().map(|_| l)).unwrap();
    assert!(frame.children().unwrap().is_empty());
    assert!(matches!(st.doc.layer(layer).unwrap().content, LayerContent::Raster(_)));
    assert_eq!(st.history.entries().len(), history.len() + 1);
    assert_eq!(st.history.entries().last().map(String::as_str), Some("New Frame"));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().active_layer, Some(layer));
    assert_eq!(app.session.active().unwrap().history.entries(), history);
}

#[test]
fn larger_and_disjoint_selections_keep_the_layer_threshold_and_overlap_distinct() {
    let (mut app, layer) = candidate(Rect::new(50, 50, 150, 130));
    app.run("select.rect", json!({"x":0,"y":0,"width":600,"height":400})).unwrap();
    assert_eq!(content_layer(&app, [60, 60, 100, 100]), Some(layer.0), "a larger selection must not enlarge the threshold");
    assert_eq!(content_layer(&app, [700, 500, 900, 700]), None);
    app.run("select.rect", json!({"x":300,"y":200,"width":60,"height":60})).unwrap();
    assert_eq!(content_layer(&app, [300, 200, 500, 400]), None, "a selection with no layer content stays ineligible");
    app.run("select.deselect", json!({})).unwrap();
    assert_eq!(content_layer(&app, [60, 60, 100, 100]), Some(layer.0));
}

#[test]
fn offset_raster_and_whole_moving_layers_keep_their_size_with_a_selection() {
    let (mut app, layer) = candidate(Rect::new(240, 120, 440, 320));
    app.run("select.rect", json!({"x":250,"y":130,"width":60,"height":60})).unwrap();
    assert_eq!(content_layer(&app, [250, 130, 316, 196]), None);
    assert_eq!(content_layer(&app, [250, 130, 318, 198]), Some(layer.0));
    for kind in ["shape", "smart"] {
        let (mut app, _) = candidate(Rect::new(50, 50, 250, 250));
        if kind == "shape" {
            app.run("shape.create", json!({"kind":"rect","rect":[50,50,200,200],"fill":"#3366cc"})).unwrap();
        } else {
            app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        }
        let id = app.session.active().unwrap().active_layer.unwrap();
        app.run("select.rect", json!({"x":60,"y":60,"width":20,"height":20})).unwrap();
        assert_eq!(content_layer(&app, [60, 60, 126, 126]), None, "{kind}");
        assert_eq!(content_layer(&app, [60, 60, 128, 128]), Some(id.0), "{kind}");
    }
}
