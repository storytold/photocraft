use serde_json::json;

use super::*;

fn session(w: u32, h: u32, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "white", "depth": depth})).unwrap();
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

/// Paints the Background in three vertical stripes: red `..40`, green `40..60`, blue `60..`.
fn stripes(s: &mut Session) -> LayerId {
    let (w, h) = (doc(s).size.width as i32, doc(s).size.height as i32);
    s.edit("stripes", |doc, _| {
        let bg = doc.background_mut().ok_or(EngineError::Other("no background".into()))?;
        if let LayerContent::Raster(surf) = &mut bg.content {
            for (x0, x1, c) in [(0, 40, [1.0, 0.0, 0.0, 1.0]), (40, 60, [0.0, 1.0, 0.0, 1.0]), (60, w, [0.0, 0.0, 1.0, 1.0])] {
                let fmt = surf.format();
                let px = photocraft_raster::from_rgba(&fmt, c);
                surf.fill_rect(Rect::new(x0, 0, x1, h), &px);
            }
        }
        Ok(())
    })
    .unwrap();
    doc(s).background().unwrap().id
}

fn rgb(s: &Session, id: LayerId, x: i32, y: i32) -> [f32; 3] {
    match &doc(s).layer(id).unwrap().content {
        LayerContent::Raster(surf) => {
            let p = surf.pixel(x, y);
            [p[0], p[1], p[2]]
        }
        _ => panic!("not a pixel layer"),
    }
}

#[test]
fn a_vertical_band_is_removed_and_the_sides_join_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut s = session(100, 30, depth);
        let bg = stripes(&mut s);
        let steps = s.active().unwrap().history.past_len();
        let r = s.execute("image.cutOut", json!({"from": 40, "to": 60})).unwrap();
        assert_eq!((r["width"].as_u64(), r["height"].as_u64(), r["removed"].as_i64()), (Some(80), Some(30), Some(20)), "{r}");
        assert_eq!(rgb(&s, bg, 39, 10), [1.0, 0.0, 0.0], "red stays at depth {depth}");
        assert_eq!(rgb(&s, bg, 40, 10), [0.0, 0.0, 1.0], "blue joins it at depth {depth}");
        assert_eq!(rgb(&s, bg, 79, 10), [0.0, 0.0, 1.0]);
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one history step");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(doc(&s).size.width, 100);
        assert_eq!(rgb(&s, bg, 50, 10), [0.0, 1.0, 0.0]);
    }
}

#[test]
fn a_horizontal_band_removes_rows_and_moves_guides() {
    let mut s = session(40, 100, 8);
    s.edit("guides", |doc, _| {
        doc.guides.horizontal = vec![10.0, 50.0, 90.0];
        doc.guides.vertical = vec![20.0];
        Ok(())
    })
    .unwrap();
    s.execute("image.cutOut", json!({"axis": "horizontal", "from": 60, "to": 30})).unwrap();
    assert_eq!((doc(&s).size.width, doc(&s).size.height), (40, 70));
    assert_eq!(doc(&s).guides.horizontal, vec![10.0, 30.0, 60.0], "inside the band → the seam; past it → moved up");
    assert_eq!(doc(&s).guides.vertical, vec![20.0]);
}

#[test]
fn live_layers_past_the_band_move_and_ones_it_crosses_are_rasterized() {
    let mut s = session(200, 60, 8);
    let after = s.execute("shape.create", json!({"rect": [150, 10, 30, 30], "fill": "#000000"})).unwrap()["layer"].as_u64().unwrap();
    let across = s.execute("shape.create", json!({"rect": [40, 10, 80, 30], "fill": "#000000"})).unwrap()["layer"].as_u64().unwrap();
    let before = s.execute("shape.create", json!({"rect": [5, 10, 20, 20], "fill": "#000000"})).unwrap()["layer"].as_u64().unwrap();
    let inside = s.execute("shape.create", json!({"rect": [70, 10, 20, 20], "fill": "#000000"})).unwrap()["layer"].as_u64().unwrap();
    let r = s.execute("image.cutOut", json!({"from": 60, "to": 100})).unwrap();
    assert_eq!(r["rasterized"], json!([across]), "{r}");
    assert_eq!(r["deleted"], json!([inside]), "{r}");
    assert!(doc(&s).layer(LayerId(inside)).is_none());
    assert!(s.active().unwrap().active_layer.is_some_and(|a| doc(&s).layer(a).is_some()), "the active layer was deleted: another takes over");
    let info = |s: &mut Session, id: u64| s.execute("shape.info", json!({"layer": id})).unwrap();
    assert_eq!(info(&mut s, after)["bounds"], json!([110, 10, 30, 30]), "moved left by the band");
    assert_eq!(info(&mut s, before)["bounds"], json!([5, 10, 20, 20]), "untouched");
    assert!(matches!(doc(&s).layer(LayerId(across)).unwrap().content, LayerContent::Raster(_)));
    // One step undoes the rasterizing too.
    s.execute("edit.undo", json!({})).unwrap();
    assert!(matches!(doc(&s).layer(LayerId(across)).unwrap().content, LayerContent::Shape(_)));
    assert!(doc(&s).layer(LayerId(inside)).is_some());
    assert_eq!(doc(&s).size.width, 200);
}

#[test]
fn a_sawtooth_or_wave_edge_adds_one_seam_layer_on_top() {
    for edge in ["sawtooth", "wave"] {
        let mut s = session(120, 80, 8);
        s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
        // The active layer already on top (the case the app hit), then one below the top.
        let _ = s.execute("image.cutOut", json!({"from": 100, "to": 110, "edge": edge})).unwrap();
        assert_eq!(doc(&s).layers.last().unwrap().name, "Cut Out Edge", "{edge}");
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Under"})).unwrap();
        s.execute("layer.arrange.sendBackward", json!({})).unwrap();
        let r = s.execute("image.cutOut", json!({"from": 50, "to": 70, "edge": edge, "edgeColor": "#ff0000"})).unwrap();
        let seam = LayerId(r["edgeLayer"].as_u64().unwrap());
        let top = doc(&s).layers.last().unwrap();
        assert_eq!((top.id, top.name.as_str()), (seam, "Cut Out Edge"), "{edge}");
        let LayerContent::Shape(sh) = &top.content else { panic!("seam is a shape") };
        assert!(sh.path.subpaths[0].knots.len() > 4 && !sh.path.subpaths[0].closed);
        // Undo removes the seam and restores the width in one step.
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(doc(&s).size.width, 120);
        assert!(doc(&s).layer(seam).is_none());
    }
}

#[test]
fn bad_params_are_errors_and_leave_the_document_alone() {
    let mut s = session(50, 50, 8);
    let before = doc(&s).clone();
    for p in [
        json!({}),
        json!({"from": 10}),
        json!({"from": "10", "to": 20}),
        json!({"from": f64::NAN, "to": 20}),
        json!({"from": 10, "to": 10}),
        json!({"from": 0, "to": 50}),
        json!({"from": -100, "to": 1e12}),
        json!({"from": 60, "to": 80}),
        json!({"from": 10, "to": 20, "axis": "diagonal"}),
        json!({"from": 10, "to": 20, "edge": "zigzag"}),
        json!({"from": 10, "to": 20, "edge": "wave", "edgeColor": 7}),
    ] {
        assert!(s.execute("image.cutOut", p.clone()).is_err(), "{p}");
        assert_eq!(doc(&s), &before, "{p}");
    }
    let mut empty = Session::new();
    assert!(empty.execute("image.cutOut", json!({"from": 1, "to": 2})).is_err());
}

#[test]
fn two_cuts_are_two_undo_steps_even_with_a_full_history() {
    let mut s = session(200, 40, 8);
    s.edit_prefs(|p| p.performance.history_states = 2);
    for _ in 0..3 {
        s.execute("layer.new.layer", json!({})).unwrap();
    }
    s.execute("image.cutOut", json!({"from": 10, "to": 20})).unwrap();
    s.execute("image.cutOut", json!({"from": 10, "to": 20})).unwrap();
    assert_eq!(doc(&s).size.width, 180);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(doc(&s).size.width, 190, "one undo takes back one cut");
}
