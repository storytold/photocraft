//! #14565: when a new artboard reaches past the canvas, the canvas grows by the whole old canvas
//! width or height on that side, as in the reference application; a board past the left or top
//! edge shifts the document by that growth. Moving or resizing a board keeps the exact growth.

use super::*;
use photocraft_geom::Rect;

/// The issue's document: 800 x 600, white Background, and "Group 1" holding one layer with
/// pixels at `r` (it may reach past the canvas).
fn session(r: Rect) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    let group = s
        .edit("group", |doc, active| {
            let fmt = doc.pixel_format();
            let mut l = Layer::raster("Layer 1", fmt);
            l.surface_mut().unwrap().fill_rect(r, &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
            let id = doc.insert_above(None, Layer::group("Group 1", vec![l]));
            *active = Some(id);
            Ok(id)
        })
        .unwrap();
    (s, group)
}

fn size(s: &Session) -> (u32, u32) {
    let d = &s.active().unwrap().doc;
    (d.size.width, d.size.height)
}

fn board(s: &Session) -> [i32; 4] {
    let d = &s.active().unwrap().doc;
    let r = d.artboards()[0].2.rect;
    [r.x0, r.y0, r.x1, r.y1]
}

fn from_group(s: &mut Session, group: LayerId) {
    s.execute("layer.new.artboardFromGroup", json!({"layer": group.0})).unwrap();
}

#[test]
fn a_board_past_the_right_edge_grows_the_canvas_by_its_width() {
    let (mut s, group) = session(Rect::new(100, 100, 900, 450));
    from_group(&mut s, group);
    assert_eq!(size(&s), (1600, 600));
    assert_eq!(board(&s), [100, 100, 900, 450], "nothing shifted");
}

#[test]
fn a_board_past_the_bottom_edge_grows_the_canvas_by_its_height() {
    let (mut s, group) = session(Rect::new(100, 100, 500, 650));
    from_group(&mut s, group);
    assert_eq!(size(&s), (800, 1200));
    assert_eq!(board(&s), [100, 100, 500, 650]);
}

#[test]
fn a_board_past_the_left_and_top_edges_shifts_everything_by_the_growth() {
    let (mut s, group) = session(Rect::new(-100, -50, 600, 450));
    s.execute("view.colorSamplers.add", json!({"x": 10.0, "y": 20.0})).unwrap();
    from_group(&mut s, group);
    assert_eq!(size(&s), (1600, 1200));
    assert_eq!(board(&s), [700, 550, 1400, 1050]);
    // The group's pixels moved with it, and so did the Color Sampler (#9302).
    let d = &s.active().unwrap().doc;
    let r = d.layer(group).unwrap().children().unwrap()[0].surface().unwrap().content_bounds();
    assert_eq!((r.x0, r.y0, r.x1, r.y1), (700, 550, 1400, 1050));
    let l = s.execute("view.colorSamplers.list", json!({})).unwrap();
    assert_eq!(l["samplers"][0]["position"], json!([810.5, 620.5]));
}

#[test]
fn new_artboard_grows_the_canvas_the_same_way() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    s.execute("layer.new.artboard", json!({"rect": [100, 100, 800, 350]})).unwrap();
    assert_eq!(size(&s), (1600, 600));
    assert_eq!(board(&s), [100, 100, 900, 450]);
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    s.execute("layer.new.artboard", json!({"rect": [-100, -50, 700, 500]})).unwrap();
    assert_eq!(size(&s), (1600, 1200));
    assert_eq!(board(&s), [700, 550, 1400, 1050]);
}

#[test]
fn a_board_inside_the_canvas_keeps_it() {
    let (mut s, group) = session(Rect::new(100, 100, 700, 500));
    from_group(&mut s, group);
    assert_eq!(size(&s), (800, 600));
    assert_eq!(board(&s), [100, 100, 700, 500]);
}

#[test]
fn a_board_more_than_one_canvas_past_an_edge_still_fits() {
    // Not measured: the canvas grows as far as the board reaches, so it stays fully visible.
    let (mut s, group) = session(Rect::new(100, 100, 2000, 450));
    from_group(&mut s, group);
    assert_eq!(size(&s), (2000, 600));
    let (mut s, group) = session(Rect::new(-1000, 100, 500, 450));
    from_group(&mut s, group);
    assert_eq!(size(&s), (1800, 600));
    assert_eq!(board(&s), [0, 100, 1500, 450]);
}

#[test]
fn one_undo_restores_the_canvas_and_every_position() {
    let (mut s, group) = session(Rect::new(-100, -50, 600, 450));
    from_group(&mut s, group);
    assert_eq!(size(&s), (1600, 1200));
    assert!(s.undo());
    assert_eq!(size(&s), (800, 600));
    let d = &s.active().unwrap().doc;
    assert!(d.artboards().is_empty());
    let r = d.layer(group).unwrap().children().unwrap()[0].surface().unwrap().content_bounds();
    assert_eq!((r.x0, r.y0, r.x1, r.y1), (-100, -50, 600, 450));
}

#[test]
fn moving_a_board_past_the_edge_keeps_the_exact_growth() {
    // Not measured (the Artboard tool's Auto-size Canvas): today's growth to the board's edge.
    let (mut s, group) = session(Rect::new(100, 100, 700, 500));
    from_group(&mut s, group);
    s.execute("layer.artboard.set", json!({"layer": group.0, "x": 250})).unwrap();
    assert_eq!(size(&s), (850, 600));
}
