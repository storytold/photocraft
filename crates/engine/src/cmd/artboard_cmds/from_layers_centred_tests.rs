//! #14568: Layer › New › Artboard from Layers with a size of the user's own centres the board on
//! the selected layers' union bounds, as the reference application does: x0 = ceil(cx - w / 2),
//! y0 = ceil(cy - h / 2), so a half pixel on the left or top rounds up. An axis without a size
//! keeps the union's edges.

use super::*;
use photocraft_geom::Rect;

/// The issue's document, 800 x 600 with the white Background, "Red" at `red` and "Blue" at
/// 400,300 to 600,450, both selected.
fn session(red: [i32; 4]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    let mut ids = Vec::new();
    for (name, [x0, y0, x1, y1], rgba) in [("Red", red, [1.0, 0.0, 0.0, 1.0]), ("Blue", [400, 300, 600, 450], [0.0, 0.0, 1.0, 1.0])] {
        let id = s
            .edit(name, |doc, active| {
                let fmt = doc.pixel_format();
                let mut l = Layer::raster(name, fmt);
                l.surface_mut().unwrap().fill_rect(Rect { x0, y0, x1, y1 }, &photocraft_raster::from_rgba(&fmt, rgba));
                let id = doc.insert_above(None, l);
                *active = Some(id);
                Ok(id)
            })
            .unwrap();
        ids.push(id);
    }
    for (i, id) in ids.iter().enumerate() {
        s.execute("layer.select", json!({"layer": id.0, "mode": if i == 0 { "replace" } else { "add" }})).unwrap();
    }
    s
}

/// Runs Artboard from Layers and returns the board as x0, y0, x1, y1, from the command's answer,
/// after checking the document holds the same board.
fn board(s: &mut Session, params: Value) -> [i32; 4] {
    let r = s.execute("layer.new.artboardFromLayers", params.clone()).unwrap();
    let d = &s.active().unwrap().doc;
    let boards = d.artboards();
    assert_eq!(boards.len(), 1, "{params}");
    let rect = boards[0].2.rect;
    assert_eq!(r["rect"], json!([rect.x0, rect.y0, rect.width(), rect.height()]), "{params}");
    [rect.x0, rect.y0, rect.x1, rect.y1]
}

const UNION: [i32; 4] = [100, 100, 300, 250];

#[test]
fn a_custom_size_is_centred_on_the_selected_layers() {
    // Union 100,100 to 600,450, centre 350,275; the reference application's boards.
    for (w, h, want) in [(300, 200, [200, 175, 500, 375]), (301, 201, [200, 175, 501, 376]), (4, 4, [348, 273, 352, 277]), (272, 340, [214, 105, 486, 445])] {
        let mut s = session(UNION);
        assert_eq!(board(&mut s, json!({"width": w, "height": h})), want, "{w} x {h}");
    }
}

#[test]
fn a_half_pixel_on_the_left_or_top_rounds_up() {
    // Union 101,101 to 600,450, centre 350.5,275.5: 300 x 200 starts at 200.5,175.5 -> 201,176.
    let mut s = session([101, 101, 300, 250]);
    assert_eq!(board(&mut s, json!({"width": 300, "height": 200})), [201, 176, 501, 376]);
}

#[test]
fn the_default_size_is_the_union_as_before() {
    let mut s = session(UNION);
    assert_eq!(board(&mut s, json!({"width": 500, "height": 350})), [100, 100, 600, 450]);
    let mut s = session(UNION);
    assert_eq!(board(&mut s, json!({})), [100, 100, 600, 450]);
    let mut s = session(UNION);
    assert_eq!(board(&mut s, json!({"width": 0, "height": 0})), [100, 100, 600, 450]);
}

#[test]
fn one_axis_given_is_centred_and_the_other_keeps_the_union() {
    let mut s = session(UNION);
    assert_eq!(board(&mut s, json!({"width": 300})), [200, 100, 500, 450]);
    let mut s = session(UNION);
    assert_eq!(board(&mut s, json!({"width": 0, "height": 201})), [100, 175, 600, 376]);
}

#[test]
fn a_board_past_the_canvas_still_grows_it() {
    // 1000 wide on the 800 wide canvas: the board starts left of the origin; the canvas grows to
    // show it (how far is #14565's rule) and the whole board stays visible.
    let mut s = session(UNION);
    let r = s.execute("layer.new.artboardFromLayers", json!({"width": 1000, "height": 350})).unwrap();
    let d = &s.active().unwrap().doc;
    let Rect { x0, y0, x1, y1 } = d.artboards()[0].2.rect;
    assert_eq!((x1 - x0, y1 - y0), (1000, 350));
    // The command answers with the board as stored, after the document's shift (CodeRabbit on
    // #15571).
    assert_eq!(r["rect"], json!([x0, y0, 1000, 350]));
    let canvas = d.bounds();
    assert!(canvas.x0 <= x0 && canvas.y0 <= y0 && x1 <= canvas.x1 && y1 <= canvas.y1, "{canvas:?}");
}

#[test]
fn huge_sizes_are_refused_without_a_panic() {
    for params in [json!({"width": 9.0e18}), json!({"height": i64::MAX}), json!({"width": 5_000_000_000_i64})] {
        let mut s = session(UNION);
        assert!(s.execute("layer.new.artboardFromLayers", params.clone()).is_err(), "{params}");
        assert!(s.active().unwrap().doc.artboards().is_empty(), "{params}");
    }
}
