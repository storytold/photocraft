//! Tests that run paint commands, moved from the engine-core `mode_cmds` tests: 7 test(s).

use photocraft_color::ColorMode;
use photocraft_doc::{Document, Size};
use photocraft_engine::Session;
use serde_json::json;

use photocraft_geom::Rect;

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

#[test]
fn rotate_arbitrary_rotates_grouped_locked_children_and_preserves_lock() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    s.execute("layer.setProps", json!({"locks": {"position": true}})).unwrap();
    s.execute("layer.groupLayers", json!({})).unwrap();

    // 1. 30 degree rotation succeeds and canvas grows to 22x22.
    let r30 = s.execute("image.rotation.arbitrary", json!({"angle": 30, "interpolation": "nearest"})).unwrap();
    assert_eq!((r30["width"].as_u64(), r30["height"].as_u64()), (Some(22), Some(22)));

    // 2. Child remains position-locked after canvas rotation.
    let d = doc(&s);
    let group = &d.layers[1];
    let photocraft_doc::LayerContent::Group(g) = &group.content else { panic!("expected group") };
    assert_eq!(g.children.len(), 1);
    let child = &g.children[0];
    assert_eq!(child.name, "Paint");
    assert!(child.locks.position, "position lock must be preserved on child");

    // 3. Free Transform of the child alone still refuses with lock error.
    let child_id = child.id.0;
    s.execute("layer.select", json!({"layer": child_id})).unwrap();
    let err = s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 0, 0]})).unwrap_err();
    assert!(err.to_string().contains("locked"), "child Free Transform must remain refused");

    // 4. Undo restores 16x16 size.
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(doc(&s).size, Size::new(16, 16));

    // 5. 90 degree rotation also succeeds and composite matches dedicated 90cw turn.
    let r90 = s.execute("image.rotation.arbitrary", json!({"angle": 90, "direction": "cw", "interpolation": "nearest"})).unwrap();
    assert_eq!((r90["width"].as_u64(), r90["height"].as_u64()), (Some(16), Some(16)));
    let d_arb = doc(&s).clone();
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("image.imageRotation.90cw", json!({})).unwrap();
    let d_cw = doc(&s);
    assert_eq!(photocraft_compose::flatten(&d_arb).px, photocraft_compose::flatten(d_cw).px);
}

#[test]
fn rotate_arbitrary_turns_unlinked_layer_masks() {
    for depth in [8, 16] {
        for grouped in [false, true] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth, "background": "white"})).unwrap();
            s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
            s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
            s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
            s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
            s.execute("layer.layerMask.linked", json!({"linked": false})).unwrap();
            let id = s.active().unwrap().active_layer.unwrap();
            if grouped {
                s.execute("layer.groupLayers", json!({})).unwrap();
            }
            let mask = |s: &Session| doc(s).layer(id).unwrap().mask.clone().unwrap();
            let base = s.active().unwrap().doc.clone();
            s.execute("image.rotation.arbitrary", json!({"angle": 180, "interpolation": "nearest"})).unwrap();
            let m = mask(&s);
            assert!(!m.linked);
            let at = |x, y| m.surface.sample_channel(x, y, 0);
            assert!(at(34, 24) > 0.9, "{depth}-bit, grouped {grouped}: the mask turned with its layer");
            assert!(at(4, 4) < 0.1, "{depth}-bit, grouped {grouped}: nothing left at the old place");
            // Rotate 180° and a crop turned by 180° over the whole canvas give the same mask.
            for (id_, p) in [("image.imageRotation.180", json!({})), ("image.crop", json!({"x": 0, "y": 0, "width": 40, "height": 30, "angle": 180}))] {
                *s.active_mut().unwrap().doc_mut() = base.clone();
                s.execute(id_, p).unwrap();
                let turned = mask(&s);
                for y in 0..30 {
                    for x in 0..40 {
                        let (a, b) = (at(x, y), turned.surface.sample_channel(x, y, 0));
                        assert!((a - b).abs() < 1e-3, "{id_}, {depth}-bit, grouped {grouped}: ({x},{y}) {a} vs {b}");
                    }
                }
            }
            // 90° pins the direction too (180° is its own inverse).
            *s.active_mut().unwrap().doc_mut() = base.clone();
            s.execute("image.rotation.arbitrary", json!({"angle": 90, "interpolation": "nearest"})).unwrap();
            let arbitrary = mask(&s);
            *s.active_mut().unwrap().doc_mut() = base.clone();
            s.execute("image.imageRotation.90cw", json!({})).unwrap();
            let turned = mask(&s);
            for y in 0..40 {
                for x in 0..30 {
                    let (a, b) = (arbitrary.surface.sample_channel(x, y, 0), turned.surface.sample_channel(x, y, 0));
                    assert!((a - b).abs() < 1e-3, "90cw, {depth}-bit, grouped {grouped}: ({x},{y}) {a} vs {b}");
                }
            }
        }
    }
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
fn brush_stroke_stays_in_indexed_color_table() {
    let mut s = black_white_indexed(false);
    assert_eq!(doc(&s).color_table.as_ref().unwrap().colors, vec![[0, 0, 0], [255, 255, 255]]);
    s.execute("paint.stroke", json!({"points": [[8, 8, 1]], "size": 8, "color": "#ff0000", "opacity": 1, "flow": 1})).unwrap();
    // Red's nearest entry is black.
    assert_eq!(doc(&s).layers[0].surface().unwrap().rgba(8, 8), [0.0, 0.0, 0.0, 1.0]);
    // Soft edges and partial opacity blend, and the blends map back onto the table too.
    s.execute("paint.stroke", json!({"points": [[4, 12], [14, 12]], "size": 6, "hardness": 0.0, "color": "#808080", "opacity": 0.5})).unwrap();
    let table = doc(&s).color_table.clone().unwrap();
    assert!(colours(doc(&s)).iter().all(|c| table.colors.contains(c)), "off-table pixels: {:?}", colours(doc(&s)));
    assert_eq!(doc(&s).mode, ColorMode::Indexed);
}

#[test]
fn brush_stroke_keeps_indexed_transparency_binary() {
    let mut s = black_white_indexed(true);
    let table = doc(&s).color_table.clone().unwrap();
    assert!(table.transparent.is_some());
    s.execute("paint.stroke", json!({"points": [[0, 8], [15, 8]], "size": 8, "hardness": 0.0, "color": "#ff0000", "opacity": 0.7})).unwrap();
    let d = doc(&s);
    let surf = d.layers[0].surface().unwrap();
    for (x, y) in (0..16).flat_map(|y| (0..16).map(move |x| (x, y))) {
        let a = surf.rgba(x, y)[3];
        assert!(a == 0.0 || a == 1.0, "alpha {a} at ({x},{y})");
    }
    assert!(colours(d).iter().all(|c| table.colors.contains(c)), "off-table pixels: {:?}", colours(d));
}

#[test]
fn live_brush_preview_matches_indexed_commit() {
    let mut s = black_white_indexed(false);
    let params = json!({"points": [[4, 8], [12, 8]], "size": 7, "hardness": 0.0, "color": "#ff0000", "opacity": 0.6, "smoothing": 0.0, "seed": 1});
    let preview = photocraft_engine::brush_cmds::LiveStroke::begin(&s, &params).unwrap();
    let table = doc(&s).color_table.clone().unwrap();
    assert!(colours(&preview.doc).iter().all(|c| table.colors.contains(c)), "preview off the table: {:?}", colours(&preview.doc));
    s.execute("paint.stroke", params).unwrap();
    let (live, committed) = (preview.doc.layers[0].surface().unwrap(), doc(&s).layers[0].surface().unwrap());
    for (x, y) in (0..16).flat_map(|y| (0..16).map(move |x| (x, y))) {
        assert_eq!(live.rgba(x, y), committed.rgba(x, y), "preview/commit at ({x},{y})");
    }
}

#[test]
fn indexed_brush_stroke_leaves_pixels_it_did_not_paint() {
    let mut s = black_white_indexed(false);
    // An off-table pixel inside the stroke's bounds but away from its dabs (Paint Bucket can
    // still leave one) is not the stroke's to remap.
    s.edit("setup", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let px = photocraft_raster::from_rgba(&surf.format(), [1.0, 0.0, 0.0, 1.0]);
        surf.write_region(Rect::new(13, 2, 14, 3), &px);
        Ok(())
    })
    .unwrap();
    s.execute("paint.stroke", json!({"points": [[2, 2], [14, 14]], "size": 2, "hardness": 1.0, "color": "#ff0000", "smoothing": 0.0})).unwrap();
    let surf = doc(&s).layers[0].surface().unwrap();
    assert_eq!(surf.rgba(13, 2), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(surf.rgba(8, 8), [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn pencil_auto_erase_knows_the_indexed_foreground() {
    let mut s = black_white_indexed(false);
    let pencil = json!({"points": [[8, 8]], "size": 3, "color": "#ff0000", "autoErase": true});
    s.execute("paint.pencil", pencil.clone()).unwrap();
    assert_eq!(doc(&s).layers[0].surface().unwrap().rgba(8, 8), [0.0, 0.0, 0.0, 1.0]);
    // Red painted as black, so a stroke starting there auto-erases to the (white) background.
    let preview = photocraft_engine::brush_cmds::LiveStroke::begin_with(&s, "paint.pencil", &pencil).unwrap();
    assert_eq!(preview.doc.layers[0].surface().unwrap().rgba(8, 8), [1.0, 1.0, 1.0, 1.0]);
    s.execute("paint.pencil", pencil).unwrap();
    assert_eq!(doc(&s).layers[0].surface().unwrap().rgba(8, 8), [1.0, 1.0, 1.0, 1.0]);
}
