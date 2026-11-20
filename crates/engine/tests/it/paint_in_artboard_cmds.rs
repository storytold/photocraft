//! Tests that run painting commands, moved from the engine-core `artboard_cmds` tests: 2 test(s).

use photocraft_color::Color;
use photocraft_doc::{Document, Layer, LayerId, Size};
use photocraft_engine::Session;
use photocraft_engine::artboard_cmds::*;
use serde_json::json;

use photocraft_geom::Rect;

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 80, "depth": depth})).unwrap();
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn add_square(s: &mut Session, r: Rect, rgba: [f32; 4]) -> LayerId {
    s.edit("sq", |doc, active| {
        let fmt = doc.pixel_format();
        let mut l = Layer::raster(doc.next_layer_name("Layer"), fmt);
        l.surface_mut().unwrap().fill_rect(r, &photocraft_raster::from_rgba(&fmt, rgba));
        let id = doc.insert_above(None, l);
        *active = Some(id);
        Ok(id)
    })
    .unwrap()
}

fn board_rect(s: &Session, id: u64) -> Rect {
    doc(s).layer(LayerId(id)).unwrap().artboard().unwrap().rect
}

/// #1531: a board 40×40 at (10, 10) holding a red square at (10..30)², the artboard active.
fn board_with_square(s: &mut Session) -> LayerId {
    add_square(s, Rect::new(10, 10, 30, 30), [1.0, 0.0, 0.0, 1.0]);
    let gid = LayerId(s.execute("layer.new.artboardFromLayers", json!({"name": "Board"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.artboard.set", json!({"width": 40, "height": 40})).unwrap();
    s.execute("layer.select", json!({"layer": gid.0})).unwrap();
    gid
}

fn red_at(s: &Session, x: i32, y: i32) -> bool {
    let px = photocraft_compose::render(doc(s), Rect::new(x, y, x + 1, y + 1)).px[0];
    px[0] > 0.99 && px[1] < 0.01 && px[3] > 0.99
}

/// Moving or making a board past the canvas origin re-bases the document; the Color Sampler
/// points and the painting symmetry shift with it and stay on the same pixels.
#[test]
fn a_board_past_the_origin_moves_the_color_samplers_and_symmetry_with_the_document() {
    // #9302: the re-based document moved everything but the Color Sampler points and the
    // painting symmetry, which then sat on other pixels. They follow, as with Canvas Size.
    let samplers = |s: &Session| s.active().unwrap().color_samplers.clone();
    let center = |s: &Session| s.active().unwrap().symmetry.as_ref().and_then(|y| y.preset()).map(|p| p.center);
    for (cmd, p) in [("layer.translate", json!({"dx": -60, "dy": -30})), ("layer.artboard.set", json!({"x": -50, "y": -20}))] {
        let mut s = session(8);
        let gid = board_with_square(&mut s);
        s.execute("view.colorSamplers.add", json!({"x": 15, "y": 15})).unwrap();
        s.execute("paint.setSymmetry", json!({"mode": "vertical", "center": [70.0, 20.0]})).unwrap();
        s.execute("layer.select", json!({"layer": gid.0})).unwrap();
        s.execute(cmd, p).unwrap();
        // The board lands at (0, 0) after the document shifted right 50 and down 20; the sampler
        // is pinned to the canvas, not the board, so it shifts with the document only. Sampler
        // points sit on pixel centres (#11725): one placed at 15, 15 is stored as 15.5, 15.5.
        assert_eq!(board_rect(&s, gid.0), Rect::new(0, 0, 40, 40), "{cmd}");
        assert_eq!(samplers(&s), vec![[65.5, 35.5]], "{cmd}");
        assert_eq!(center(&s), Some([120.0, 40.0]), "{cmd}: the symmetry stays on its pixels");
    }
    // A new board left of the canvas shifts the photo right by its overhang.
    let mut s = session(8);
    add_square(&mut s, Rect::new(10, 10, 30, 30), [1.0, 0.0, 0.0, 1.0]);
    s.execute("view.colorSamplers.add", json!({"x": 15, "y": 15})).unwrap();
    s.execute("layer.new.artboard", json!({"rect": [-200, 0, 100, 80]})).unwrap();
    assert_eq!(samplers(&s), vec![[215.5, 15.5]]);
    assert!(red_at(&s, 215, 15));
    // A board made from layers (or a group) left of the canvas shifts by their overhang.
    for cmd in ["layer.new.artboardFromLayers", "layer.new.artboardFromGroup"] {
        let mut s = session(8);
        add_square(&mut s, Rect::new(10, 10, 30, 30), [1.0, 0.0, 0.0, 1.0]);
        s.execute("view.colorSamplers.add", json!({"x": 15, "y": 15})).unwrap();
        add_square(&mut s, Rect::new(-30, 40, -10, 60), [0.0, 0.0, 1.0, 1.0]);
        if cmd.ends_with("Group") {
            s.execute("layer.groupLayers", json!({})).unwrap();
        }
        s.execute(cmd, json!({})).unwrap();
        assert_eq!(samplers(&s), vec![[45.5, 15.5]], "{cmd}");
        assert!(red_at(&s, 45, 15), "{cmd}");
    }
    // The "+" button left of a board at the origin shifts by the board and the gap.
    let mut s = session(8);
    let gid = board_with_square(&mut s);
    s.execute("layer.artboard.set", json!({"x": 0, "y": 0})).unwrap();
    s.execute("view.colorSamplers.add", json!({"x": 5, "y": 5})).unwrap();
    s.execute("layer.artboard.addBeside", json!({"layer": gid.0, "side": "left"})).unwrap();
    let by = 40 + GAP;
    assert_eq!(board_rect(&s, gid.0), Rect::new(by, 0, by + 40, 40));
    assert_eq!(samplers(&s), vec![[f64::from(by) + 5.5, 5.5]]);
    assert!(red_at(&s, by + 5, 5));
    // A board right of the canvas shifts nothing.
    s.execute("layer.artboard.addBeside", json!({"layer": gid.0, "side": "right"})).unwrap();
    assert_eq!(samplers(&s), vec![[f64::from(by) + 5.5, 5.5]]);
}

/// Duplicating a board whose document has boards past the origin (one or several selected
/// layers) re-bases it; the Color Sampler points shift with the document.
#[test]
fn duplicating_a_board_beside_one_past_the_origin_moves_the_color_samplers_with_the_document() {
    // #9302 review: Layer › Duplicate puts the copy right of its board, but `fit_canvas` still
    // re-bases a document whose boards were already past the origin (as a PSD can hold them).
    // Single (`layer`) and several selected layers (`duplicate_selected`) both follow the shift.
    for multi in [false, true] {
        let mut s = session(8);
        let gid = board_with_square(&mut s);
        s.edit("past the origin", |doc, _| {
            doc.layer_mut(gid).and_then(|l| l.artboard_mut()).unwrap().rect = Rect::new(-40, 10, 0, 50);
            Ok(())
        })
        .unwrap();
        let sq = add_square(&mut s, Rect::new(60, 10, 80, 30), [1.0, 0.0, 0.0, 1.0]);
        s.execute("view.colorSamplers.add", json!({"x": 65, "y": 15})).unwrap();
        if multi {
            photocraft_engine::layer_multi_cmds::set_selection(&mut s, vec![gid, sq], Some(gid), Some(gid)).unwrap();
            s.execute("layer.duplicate", json!({})).unwrap();
        } else {
            s.execute("layer.duplicate", json!({"layer": gid.0})).unwrap();
        }
        assert_eq!(board_rect(&s, gid.0), Rect::new(0, 10, 40, 50), "multi {multi}: the document shifted right 40");
        assert_eq!(s.active().unwrap().color_samplers, vec![[105.5, 15.5]], "multi {multi}");
        assert!(red_at(&s, 105, 15), "multi {multi}: the sampler is still on the square");
    }
}
