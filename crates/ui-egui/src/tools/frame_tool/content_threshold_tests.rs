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
    for (bounds, side, taken) in [
        (Rect::new(50, 50, 150, 130), 20., false),
        (Rect::new(50, 50, 150, 130), 40., true),
        (Rect::new(0, 0, 600, 400), 100., false),
    ] {
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
