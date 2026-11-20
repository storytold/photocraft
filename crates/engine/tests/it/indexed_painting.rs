//! Painting in an Indexed Color document keeps every pixel on the color table, in the live preview
//! as in the committed stroke.

use photocraft_engine::Session;
use photocraft_engine::brush_cmds::LiveStroke;
use photocraft_engine::doc::Document;
use serde_json::json;

fn paint(s: &mut Session, f: impl Fn(i32, i32) -> [f32; 4]) {
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                data.extend(photocraft_raster::from_rgba(&fmt, f(x, y)));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn colours(d: &Document) -> std::collections::HashSet<[u8; 3]> {
    let surf = d.layers[0].surface().unwrap();
    let mut set = std::collections::HashSet::new();
    for y in 0..d.size.height as i32 {
        for x in 0..d.size.width as i32 {
            let c = surf.rgba(x, y);
            if c[3] > 0.0 {
                set.insert([c[0], c[1], c[2]].map(|v| (v * 255.0).round() as u8));
            }
        }
    }
    set
}

/// A white 16×16 document in Indexed Color with the two-entry black/white table (#8669's repro).
fn black_white_indexed(transparency: bool) -> Session {
    let mut s = Session::new();
    let background = if transparency { "transparent" } else { "white" };
    s.execute("file.new", json!({"width": 16, "height": 16, "background": background})).unwrap();
    paint(&mut s, |x, _| {
        if x < 2 {
            [0.0; 4]
        } else if x < 4 {
            [0.0, 0.0, 0.0, 1.0]
        } else {
            [1.0; 4]
        }
    });
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "transparency": transparency, "dither": "none"})).unwrap();
    s
}

#[test]
fn live_brush_preview_matches_indexed_commit() {
    let mut s = black_white_indexed(false);
    let params = json!({"points": [[4, 8], [12, 8]], "size": 7, "hardness": 0.0, "color": "#ff0000", "opacity": 0.6, "smoothing": 0.0, "seed": 1});
    let preview = LiveStroke::begin(&s, &params).unwrap();
    let table = doc(&s).color_table.clone().unwrap();
    assert!(colours(&preview.doc).iter().all(|c| table.colors.contains(c)), "preview off the table: {:?}", colours(&preview.doc));
    s.execute("paint.stroke", params).unwrap();
    let (live, committed) = (preview.doc.layers[0].surface().unwrap(), doc(&s).layers[0].surface().unwrap());
    for (x, y) in (0..16).flat_map(|y| (0..16).map(move |x| (x, y))) {
        assert_eq!(live.rgba(x, y), committed.rgba(x, y), "preview/commit at ({x},{y})");
    }
}

#[test]
fn pencil_auto_erase_knows_the_indexed_foreground() {
    let mut s = black_white_indexed(false);
    let pencil = json!({"points": [[8, 8]], "size": 3, "color": "#ff0000", "autoErase": true});
    s.execute("paint.pencil", pencil.clone()).unwrap();
    assert_eq!(doc(&s).layers[0].surface().unwrap().rgba(8, 8), [0.0, 0.0, 0.0, 1.0]);
    // Red painted as black, so a stroke starting there auto-erases to the (white) background.
    let preview = LiveStroke::begin_with(&s, "paint.pencil", &pencil).unwrap();
    assert_eq!(preview.doc.layers[0].surface().unwrap().rgba(8, 8), [1.0, 1.0, 1.0, 1.0]);
    s.execute("paint.pencil", pencil).unwrap();
    assert_eq!(doc(&s).layers[0].surface().unwrap().rgba(8, 8), [1.0, 1.0, 1.0, 1.0]);
}
