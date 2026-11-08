//! Free Transform of several selected layers by one box (#6501).

use super::*;

/// A 200 x 200 document: a red 40 x 20 block on "Paint" (x 10..50, y 10..30) and a type layer
/// over it, both selected, the type layer active (as after creating it and ⌘-clicking Paint).
fn two_layers() -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    let paint = LayerId(s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap()["layer"].as_u64().unwrap());
    s.edit("paint", |doc, _| {
        doc.layer_mut(paint).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 10, 50, 30), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let text = LayerId(s.execute("type.create", json!({"x": 80, "y": 120, "text": "Hi", "size": 30})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.select", json!({"layer": paint.0})).unwrap();
    s.execute("layer.select", json!({"layer": text.0, "mode": "add"})).unwrap();
    (s, paint, text)
}

fn bounds(s: &Session, id: LayerId) -> Rect {
    let st = s.active().unwrap();
    transform_bounds(&st.doc, st.doc.layer(id).unwrap())
}

fn text_matrix(s: &Session, id: LayerId) -> [f64; 6] {
    let st = s.active().unwrap();
    let LayerContent::Text(t) = &st.doc.layer(id).unwrap().content else { panic!("not a type layer") };
    t.transform.m
}

#[test]
fn free_transform_moves_every_selected_layer_in_one_step() {
    let (mut s, paint, text) = two_layers();
    let (p0, t0, m0) = (bounds(&s, paint), bounds(&s, text), text_matrix(&s, text));
    let steps = s.active().unwrap().history.past_len();
    // Before #6501 only the active layer moved, whichever kind it was.
    let r = s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 30, 40]})).unwrap();
    assert_eq!(r["layers"], json!([paint.0, text.0]));
    assert_eq!(bounds(&s, paint), p0.translate(30, 40));
    assert_eq!(bounds(&s, text), t0.translate(30, 40));
    let st = s.active().unwrap();
    assert_eq!(st.history.past_len(), steps + 1, "one history step");
    let history = s.execute("document.inspect", json!({})).unwrap()["history"].clone();
    assert_eq!(history.as_array().unwrap().last(), Some(&json!("Free Transform")));
    // The type stays editable: it keeps the transform instead of pixels.
    assert_eq!(text_matrix(&s, text)[4..], [m0[4] + 30.0, m0[5] + 40.0]);
    s.undo();
    assert_eq!((bounds(&s, paint), bounds(&s, text)), (p0, t0), "and one Undo takes both back");
}

#[test]
fn the_default_frame_is_the_selected_layers_together() {
    let (mut s, paint, text) = two_layers();
    let both = bounds(&s, paint).union(&bounds(&s, text));
    let m0 = text_matrix(&s, text);
    // 200 % about the joined frame's centre: each layer scales about that point, not its own.
    let r = s.execute("edit.transform", json!({"scaleX": 200, "scaleY": 200})).unwrap();
    let rect: Vec<f64> = serde_json::from_value(r["rect"].clone()).unwrap();
    assert_eq!(rect, [both.x0, both.y0, both.x1, both.y1].map(f64::from));
    let (cx, cy) = (f64::from(both.x0 + both.x1) / 2.0, f64::from(both.y0 + both.y1) / 2.0);
    let p = bounds(&s, paint);
    assert_eq!((f64::from(p.x0), f64::from(p.y0)), (2.0 * 10.0 - cx, 2.0 * 10.0 - cy));
    let m = text_matrix(&s, text);
    assert!((m[0] - 2.0).abs() < 1e-9 && (m[3] - 2.0).abs() < 1e-9, "{m:?}");
    assert!((m[4] - (2.0 * m0[4] - cx)).abs() < 1e-9 && (m[5] - (2.0 * m0[5] - cy)).abs() < 1e-9, "{m:?}");
}

fn preset_layers(depth: SampleType) -> (Session, [LayerId; 2]) {
    let mut doc = Document::with_background("preset frame", photocraft_doc::Size::new(32, 24), ColorMode::Rgb, depth, photocraft_color::Color::WHITE);
    for (name, rect, color) in [("first", Rect::new(2, 4, 6, 8), [1.0, 0.0, 0.0, 1.0]), ("second", Rect::new(18, 12, 22, 16), [0.0, 1.0, 0.0, 1.0])] {
        let mut surface = Surface::new(doc.pixel_format());
        surface.fill_rect(rect, &color);
        surface.write_pixel(rect.x0, rect.y0, &[0.0, 0.0, 1.0, 1.0]);
        doc.layers.push(Layer::new(name, LayerContent::Raster(surface)));
    }
    let ids = [doc.layers[1].id, doc.layers[2].id];
    let mut s = Session::new();
    s.add_document(doc, None);
    s.execute("layer.select", json!({"layer":ids[0].0})).unwrap();
    s.execute("layer.select", json!({"layer":ids[1].0,"mode":"add"})).unwrap();
    (s, ids)
}

#[test]
fn rotate_and_flip_presets_use_the_frame_of_the_layers_they_transform() {
    let cases = [
        ("edit.transform.rotate180", [Rect::new(18, 12, 22, 16), Rect::new(2, 4, 6, 8)]),
        ("edit.transform.rotate90Cw", [Rect::new(14, 0, 18, 4), Rect::new(6, 16, 10, 20)]),
        ("edit.transform.rotate90Ccw", [Rect::new(6, 16, 10, 20), Rect::new(14, 0, 18, 4)]),
        ("edit.transform.flipHorizontal", [Rect::new(18, 4, 22, 8), Rect::new(2, 12, 6, 16)]),
        ("edit.transform.flipVertical", [Rect::new(2, 12, 6, 16), Rect::new(18, 4, 22, 8)]),
    ];
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for (command, expected) in cases {
            let (mut s, ids) = preset_layers(depth);
            let before = s.active().unwrap().doc.clone();
            let steps = s.active().unwrap().history.past_len();
            s.execute(command, json!({})).unwrap();
            assert_eq!(ids.map(|id| bounds(&s, id)), expected, "{depth:?}/{command}");
            assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
            let after = s.active().unwrap().doc.clone();
            assert!(s.undo());
            assert!(s.active().unwrap().doc.layers == before.layers);
            assert!(s.redo());
            assert!(s.active().unwrap().doc.layers == after.layers);
        }
    }
}

#[test]
fn preset_frames_keep_explicit_layer_targets_with_a_selected_background() {
    for list in [false, true] {
        let (mut s, ids) = preset_layers(SampleType::U16);
        let before = s.active().unwrap().doc.clone();
        let bg = before.layers[0].id;
        s.execute("layer.select", json!({"layer":bg.0,"mode":"add"})).unwrap();
        let params = if list { json!({"layers":[ids[0].0]}) } else { json!({"layer":ids[1].0}) };
        s.execute("edit.transform.flipHorizontal", params).unwrap();
        assert_eq!(ids.map(|id| bounds(&s, id)), [Rect::new(2, 4, 6, 8), Rect::new(18, 12, 22, 16)]);
        let after = &s.active().unwrap().doc;
        assert!(after.layer(bg) == before.layer(bg));
        let untouched = ids[usize::from(list)];
        assert!(after.layer(untouched) == before.layer(untouched));
        if list {
            let target = after.layer(ids[0]).unwrap().surface().unwrap();
            assert_eq!(target.sample_channel(5, 4, 2), 1.0, "the blue marker must actually flip");
            assert_eq!(target.sample_channel(2, 4, 2), 0.0);
        }
    }
}

#[test]
fn whole_layer_presets_refuse_a_background_selected_with_other_layers() {
    let (mut s, _) = preset_layers(SampleType::U16);
    let bg = s.active().unwrap().doc.layers[0].id;
    s.execute("layer.select", json!({"layer":bg.0,"mode":"add"})).unwrap();
    let st = s.active().unwrap();
    let (before, revision, steps) = (st.doc.clone(), st.revision, st.history.past_len());
    for command in ["edit.transform.flipHorizontal", "edit.transform.rotate90Cw"] {
        let error = s.execute(command, json!({})).unwrap_err().to_string();
        assert!(error.contains("\"Background\" is locked"), "{command}: {error}");
        let st = s.active().unwrap();
        assert!(st.doc == before && st.revision == revision && st.history.past_len() == steps);
    }
}

#[test]
fn select_all_preset_still_uses_the_selection_frame() {
    let (mut s, ids) = preset_layers(SampleType::U8);
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.transform.flipHorizontal", json!({})).unwrap();
    assert_eq!(ids.map(|id| bounds(&s, id)), [Rect::new(26, 4, 30, 8), Rect::new(10, 12, 14, 16)]);
}

#[test]
fn an_explicit_layer_still_transforms_that_layer_alone() {
    let (mut s, paint, text) = two_layers();
    let t0 = bounds(&s, text);
    let r = s.execute("edit.transform", json!({"layer": paint.0, "matrix": [1, 0, 0, 1, 5, 0]})).unwrap();
    assert_eq!(r["layers"], json!([paint.0]));
    assert_eq!(bounds(&s, paint), Rect::new(15, 10, 55, 30));
    assert_eq!(bounds(&s, text), t0);
}

#[test]
fn the_layers_param_names_the_layers_and_a_listed_group_carries_its_children() {
    let (mut s, paint, text) = two_layers();
    let t0 = bounds(&s, text);
    s.execute("layer.select", json!({"layer": paint.0})).unwrap();
    let group = LayerId(s.execute("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
    // The group and the layer in it: the layer moves once, with its group.
    let r = s.execute("edit.transform", json!({"layers": [paint.0, group.0], "matrix": [1, 0, 0, 1, 7, 0]})).unwrap();
    assert_eq!(r["layers"], json!([group.0]));
    assert_eq!(bounds(&s, paint), Rect::new(17, 10, 57, 30));
    assert_eq!(bounds(&s, text), t0, "not listed, not moved");
}

#[test]
fn the_background_among_a_selection_of_several_refuses_as_locked() {
    // Photoshop shows its locked-layer alert rather than leaving the Background out (#11101).
    let (mut s, paint, text) = two_layers();
    let bg = s.active().unwrap().doc.layers[0].id;
    s.execute("layer.select", json!({"layer": bg.0, "mode": "add"})).unwrap();
    let st = s.active().unwrap();
    let (before, revision) = (st.doc.clone(), st.revision);
    let err = s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 3, 0]})).unwrap_err().to_string();
    assert!(err.contains("\"Background\" is locked"), "{err}");
    let st = s.active().unwrap();
    assert!(st.doc == before && st.revision == revision, "nothing moved, not even {paint:?} or {text:?}");
    assert!(st.doc.is_background(bg), "still the Background");
    assert_eq!(st.doc.layer(bg).unwrap().surface().unwrap().content_bounds(), Rect::new(0, 0, 200, 200));
}

#[test]
fn a_selection_limits_the_pixels_of_every_selected_layer() {
    let (mut s, paint, text) = two_layers();
    let t0 = bounds(&s, text);
    // The left half of the block: only those pixels of Paint move; the type moves whole (#2630).
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 30, "height": 200})).unwrap();
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 0, 100]})).unwrap();
    let st = s.active().unwrap();
    let surf = st.doc.layer(paint).unwrap().surface().unwrap();
    assert_eq!(surf.content_bounds(), Rect::new(10, 10, 50, 130));
    assert_eq!(surf.sample_channel(20, 15, 3), 0.0, "the selected part left");
    assert_eq!(surf.sample_channel(40, 15, 3), 1.0, "the rest stayed");
    assert_eq!(surf.sample_channel(20, 115, 3), 1.0, "and landed below");
    assert_eq!(bounds(&s, text), t0.translate(0, 100));
}

#[test]
fn a_locked_layer_among_them_refuses_the_whole_transform() {
    let (mut s, paint, text) = two_layers();
    s.execute("layer.setProps", json!({"layer": text.0, "locks": {"position": true}})).unwrap();
    s.execute("layer.select", json!({"layer": paint.0})).unwrap();
    s.execute("layer.select", json!({"layer": text.0, "mode": "add"})).unwrap();
    let st = s.active().unwrap();
    let (before, revision) = (st.doc.clone(), st.revision);
    let err = s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 5, 0]})).unwrap_err().to_string();
    assert!(err.contains("locked"), "{err}");
    let st = s.active().unwrap();
    assert!(st.doc == before && st.revision == revision, "nothing moved, not even the unlocked layer");
}

#[test]
fn one_layer_past_the_pixel_budget_leaves_every_layer_as_it_was() {
    let (mut s, paint, _) = two_layers();
    // A 1 px dot, beside the 40 px block: only the block, stretched, outgrows the test budget.
    let dot = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.edit("dot", |doc, _| {
        doc.layer_mut(dot).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 40, 11, 41), &[0.0, 1.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let st = s.active().unwrap();
    let (before, revision) = (st.doc.clone(), st.revision);
    let k = 4e4;
    assert!(s.execute("edit.transform", json!({"layers": [dot.0, paint.0], "matrix": [k, 0, 0, 1, -10.0 * k, 0]})).is_err());
    let st = s.active().unwrap();
    assert!(st.doc == before && st.revision == revision);
}

#[test]
fn distort_with_type_among_the_layers_is_refused() {
    let (mut s, _, _) = two_layers();
    let before = s.active().unwrap().doc.clone();
    let quad = json!({"rect": [0, 0, 100, 100], "quad": [[0, 0], [100, 0], [120, 100], [-20, 100]]});
    assert!(s.execute("edit.transform", quad).is_err());
    assert!(s.active().unwrap().doc == before);
}

#[test]
fn transform_again_repeats_on_every_selected_layer() {
    let (mut s, paint, text) = two_layers();
    let (p0, t0) = (bounds(&s, paint), bounds(&s, text));
    let both = p0.union(&t0);
    let r = [both.x0, both.y0, both.x1, both.y1].map(f64::from);
    let quad = [[r[0] + 10.0, r[1]], [r[2] + 10.0, r[1]], [r[2] + 10.0, r[3]], [r[0] + 10.0, r[3]]];
    // Nearest neighbour: no resampling fringe to blur the bounds.
    s.execute("edit.transform", json!({"rect": r, "quad": quad, "interpolation": "nearest"})).unwrap();
    s.execute("edit.transform.again", json!({})).unwrap();
    assert_eq!(bounds(&s, paint), p0.translate(20, 0));
    assert_eq!(bounds(&s, text), t0.translate(20, 0));
}

#[test]
fn transform_again_on_a_copy_copies_and_moves_the_active_layer_only() {
    let (mut s, paint, text) = two_layers();
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 10, 0]})).unwrap();
    let (p1, t1) = (bounds(&s, paint), bounds(&s, text));
    s.execute("edit.transform.againCopy", json!({})).unwrap();
    let st = s.active().unwrap();
    let copy = st.active_layer.unwrap();
    assert!(copy != paint && copy != text);
    assert_eq!(bounds(&s, copy), t1.translate(10, 0), "the copy of the active type layer moved on");
    assert_eq!((bounds(&s, paint), bounds(&s, text)), (p1, t1), "the originals stayed");
}

#[test]
fn a_null_layer_beside_layers_counts_as_absent() {
    let (mut s, paint, text) = two_layers();
    let t0 = bounds(&s, text);
    let r = s.execute("edit.transform", json!({"layers": [paint.0], "layer": null, "matrix": [1, 0, 0, 1, 5, 0]})).unwrap();
    assert_eq!(r["layers"], json!([paint.0]));
    assert_eq!(bounds(&s, paint), Rect::new(15, 10, 55, 30));
    assert_eq!(bounds(&s, text), t0);
}

#[test]
fn bad_layers_params_are_errors_not_panics() {
    let (mut s, paint, _) = two_layers();
    let before = s.active().unwrap().doc.clone();
    let m = json!([1, 0, 0, 1, 5, 0]);
    for p in [
        json!({"layers": [], "matrix": m}),
        json!({"layers": "all", "matrix": m}),
        json!({"layers": [paint.0, "x"], "matrix": m}),
        json!({"layers": [-1], "matrix": m}),
        json!({"layers": [987_654], "matrix": m}),
        json!({"layers": [u64::MAX], "matrix": m}),
        json!({"layers": (0..10_001).collect::<Vec<u64>>(), "matrix": m}),
        json!({"layers": [paint.0], "layer": paint.0, "matrix": m}),
        json!({"layers": [paint.0], "target": "quickMask", "matrix": m}),
        json!({"layers": [paint.0], "target": {"channel": 0}, "matrix": m}),
    ] {
        assert!(s.execute("edit.transform", p.clone()).is_err(), "{p}");
    }
    assert!(s.active().unwrap().doc == before);
}

#[test]
fn straighten_layer_wants_one_selected_layer() {
    // #12033: the reference greys Straighten Layer out while two layers are selected, so it no
    // longer turns the whole selection the way Free Transform does
    // (`free_transform_moves_every_selected_layer_in_one_step` covers that).
    let (mut s, paint, text) = two_layers();
    let (p0, m0) = (bounds(&s, paint), text_matrix(&s, text));
    s.execute("image.analysis.rulerTool", json!({"start": [10, 30], "end": [70, 25]})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    assert_eq!(s.active().unwrap().selected_layers().len(), 2, "both selected, the type layer active");
    assert!(!s.is_enabled("image.analysis.straightenLayer"), "two selected layers");
    assert!(s.execute("image.analysis.straightenLayer", json!({})).is_err());
    assert_eq!((bounds(&s, paint), text_matrix(&s, text)), (p0, m0), "nothing turned");
    assert_eq!(s.active().unwrap().history.past_len(), steps, "and no history step");

    // With one layer selected, only that layer turns.
    s.execute("layer.select", json!({"layer": paint.0})).unwrap();
    s.execute("image.analysis.rulerTool", json!({"start": [10, 30], "end": [70, 25]})).unwrap();
    assert!(s.is_enabled("image.analysis.straightenLayer"), "a single layer");
    s.execute("image.analysis.straightenLayer", json!({})).unwrap();
    assert!(bounds(&s, paint) != p0);
    assert_eq!(text_matrix(&s, text), m0, "an unselected layer stays put");
}

#[test]
fn a_recorded_transform_of_the_selected_layers_replays_on_the_selection_in_another_document() {
    let (mut s, paint, text) = two_layers();
    s.execute("actions.record", json!({"name": "Move both"})).unwrap();
    // What the canvas box commits: the ids it opened on, listed (#6501).
    s.execute("edit.transform", json!({"layers": [paint.0, text.0], "matrix": [1, 0, 0, 1, 30, 40]})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let step = s.actions.list[0].steps[0].1.clone();
    assert!(step.get("layers").is_none(), "the step plays on the selection, not on these ids: {step}");
    // Another document, its ids shifted by an extra layer under the two that get selected.
    s.execute("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    let spare = LayerId(s.execute("layer.new.layer", json!({"name": "Spare"})).unwrap()["layer"].as_u64().unwrap());
    let a = LayerId(s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap());
    let b = LayerId(s.execute("layer.new.layer", json!({"name": "B"})).unwrap()["layer"].as_u64().unwrap());
    assert_ne!((a, b), (paint, text), "the ids differ from the recorded ones");
    s.edit("paint", |doc, _| {
        for (id, x) in [(spare, 100), (a, 10), (b, 60)] {
            doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(Rect::new(x, 10, x + 20, 30), &[0.0, 0.0, 1.0, 1.0]);
        }
        Ok(())
    })
    .unwrap();
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    let (a0, b0, spare0) = (bounds(&s, a), bounds(&s, b), bounds(&s, spare));
    let r = s.execute("actions.play", json!({"action": 0})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
    assert_eq!((bounds(&s, a), bounds(&s, b)), (a0.translate(30, 40), b0.translate(30, 40)), "the selected layers moved");
    assert_eq!(bounds(&s, spare), spare0, "the unselected layer stayed");
}

/// A 200 x 200 document with three 20 x 20 blocks, "A" (x 10..30), "B" (x 60..80) and "C"
/// (x 120..140), all at y 10..30. A and B are linked; C isn't. Only A is selected, and active.
fn linked_layers() -> (Session, LayerId, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    let mut ids = Vec::new();
    for (name, x) in [("A", 10), ("B", 60), ("C", 120)] {
        let id = LayerId(s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap());
        s.edit("paint", |doc, _| {
            doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(Rect::new(x, 10, x + 20, 30), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        ids.push(id);
    }
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    assert_eq!(s.execute("layer.linkLayers", json!({})).unwrap()["linked"], true);
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    (s, a, b, c)
}

fn moved_layers(r: &Value) -> Vec<u64> {
    let mut ids: Vec<u64> = serde_json::from_value(r["layers"].clone()).unwrap();
    ids.sort_unstable();
    ids
}

/// `got` is `want` give or take a pixel of resampling on each edge.
fn assert_near(got: Rect, want: Rect) {
    let close = [(got.x0, want.x0), (got.y0, want.y0), (got.x1, want.x1), (got.y1, want.y1)].iter().all(|(g, w)| (g - w).abs() <= 1);
    assert!(close, "{got:?} is not {want:?}");
}

/// A 200 % scale of A and B about the centre (45, 20) of their joined frame.
fn scaled_pair(s: &Session, a: LayerId, b: LayerId) {
    assert_near(bounds(s, a), Rect::new(-25, 0, 15, 40));
    assert_near(bounds(s, b), Rect::new(75, 0, 115, 40));
}

#[test]
fn transforming_one_linked_layer_transforms_its_link_group() {
    // Before #11217 only A scaled; Photoshop scales every layer linked to it, by the same box.
    let (mut s, a, b, c) = linked_layers();
    let c0 = bounds(&s, c);
    let r = s.execute("edit.transform", json!({"layer": a.0, "scaleX": 200, "scaleY": 200, "interpolation": "nearest"})).unwrap();
    assert_eq!(moved_layers(&r), [a.0, b.0]);
    let rect: Vec<f64> = serde_json::from_value(r["rect"].clone()).unwrap();
    assert_eq!(rect, [10.0, 10.0, 80.0, 30.0], "the frame covers the link group");
    scaled_pair(&s, a, b);
    assert_eq!(bounds(&s, c), c0, "an unlinked layer stays put");
    s.undo();
    assert_eq!((bounds(&s, a), bounds(&s, b)), (Rect::new(10, 10, 30, 30), Rect::new(60, 10, 80, 30)), "one Undo takes both back");
}

#[test]
fn transforming_the_selected_linked_layer_transforms_its_link_group() {
    let (mut s, a, b, c) = linked_layers();
    let c0 = bounds(&s, c);
    // No `layer`: the layer selection (A alone) is the target.
    let r = s.execute("edit.transform", json!({"scaleX": 200, "scaleY": 200, "interpolation": "nearest"})).unwrap();
    assert_eq!(moved_layers(&r), [a.0, b.0]);
    scaled_pair(&s, a, b);
    assert_eq!(bounds(&s, c), c0);
}

#[test]
fn listed_layers_do_not_pull_in_their_linked_layers() {
    let (mut s, a, b, _) = linked_layers();
    let r = s.execute("edit.transform", json!({"layers": [a.0], "matrix": [1, 0, 0, 1, 5, 0]})).unwrap();
    assert_eq!(moved_layers(&r), [a.0]);
    assert_eq!(bounds(&s, a), Rect::new(15, 10, 35, 30));
    assert_eq!(bounds(&s, b), Rect::new(60, 10, 80, 30), "listed layers are exactly the ones moved");
}

#[test]
fn moving_a_linked_layer_by_free_transform_moves_each_linked_layer_once() {
    let (mut s, a, b, c) = linked_layers();
    let c0 = bounds(&s, c);
    // A and B both selected, and linked: each moves by the offset once, not once per link.
    s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    let r = s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 30, 40]})).unwrap();
    assert_eq!(moved_layers(&r), [a.0, b.0]);
    assert_eq!(bounds(&s, a), Rect::new(40, 50, 60, 70));
    assert_eq!(bounds(&s, b), Rect::new(90, 50, 110, 70));
    assert_eq!(bounds(&s, c), c0);
    // And from A alone, through the active layer.
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("edit.transform", json!({"dx": -30, "dy": -40})).unwrap();
    assert_eq!((bounds(&s, a), bounds(&s, b)), (Rect::new(10, 10, 30, 30), Rect::new(60, 10, 80, 30)));
}

#[test]
fn a_locked_linked_layer_refuses_the_transform_of_its_link_group() {
    // As with several selected layers: one that can't move leaves every layer as it was.
    let (mut s, a, b, _) = linked_layers();
    s.execute("layer.setProps", json!({"layer": b.0, "locks": {"position": true}})).unwrap();
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    let st = s.active().unwrap();
    let (before, revision) = (st.doc.clone(), st.revision);
    let err = s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 5, 0]})).unwrap_err().to_string();
    assert!(err.contains("locked"), "{err}");
    let st = s.active().unwrap();
    assert!(st.doc == before && st.revision == revision);
}

/// The Background (white, 200 x 200) linked to "A", a red block at x 10..30, y 10..30; A is
/// selected, and active.
fn background_linked_to_a_layer() -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    let bg = s.active().unwrap().doc.layers[0].id;
    let a = LayerId(s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap());
    s.edit("paint", |doc, _| {
        doc.layer_mut(a).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 10, 30, 30), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("layer.select", json!({"layer": bg.0})).unwrap();
    s.execute("layer.select", json!({"layer": a.0, "mode": "add"})).unwrap();
    assert_eq!(s.execute("layer.linkLayers", json!({})).unwrap()["linked"], true);
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    (s, bg, a)
}

#[test]
fn a_linked_background_stays_out_of_a_transform_of_its_partner() {
    // #11217 review: the link group widening must not pull in a Background the transform didn't
    // target, any more than a selection of several does.
    let (mut s, bg, a) = background_linked_to_a_layer();
    for p in [json!({"matrix": [1, 0, 0, 1, 5, 0]}), json!({"layer": a.0, "matrix": [1, 0, 0, 1, 5, 0]})] {
        let r = s.execute("edit.transform", p.clone()).unwrap();
        assert_eq!(moved_layers(&r), [a.0], "{p}");
        let st = s.active().unwrap();
        assert!(st.doc.is_background(bg), "{p}: still the Background");
        assert_eq!(st.doc.layer(bg).unwrap().surface().unwrap().content_bounds(), Rect::new(0, 0, 200, 200), "{p}: and where it was");
    }
    assert_eq!(bounds(&s, a), Rect::new(20, 10, 40, 30), "A moved both times");
}

#[test]
fn a_transform_of_a_linked_background_still_transforms_it() {
    let (mut s, bg, a) = background_linked_to_a_layer();
    let r = s.execute("edit.transform", json!({"layer": bg.0, "matrix": [1, 0, 0, 1, 5, 0]})).unwrap();
    assert!(moved_layers(&r).contains(&bg.0), "the targeted Background moves: {r}");
    let st = s.active().unwrap();
    assert!(!st.doc.is_background(bg), "turned into a normal layer first, as in Photoshop");
    assert_eq!(bounds(&s, a), Rect::new(15, 10, 35, 30), "with the layer linked to it");
}

#[test]
fn a_recorded_transform_of_other_layers_than_the_selection_keeps_their_ids() {
    let (mut s, paint, _) = two_layers();
    s.execute("actions.record", json!({"name": "Paint only"})).unwrap();
    s.execute("edit.transform", json!({"layers": [paint.0], "matrix": [1, 0, 0, 1, 5, 0]})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[0].steps[0].1["layers"], json!([paint.0]));
}

#[test]
fn explicit_layers_bypass_panel_colour_but_reject_an_explicit_colour_target() {
    let (mut s, paint, text) = two_layers();
    s.execute("channel.target", json!({"channel":"red"})).unwrap();
    let p0 = bounds(&s, paint);
    let t0 = bounds(&s, text);
    let r = s.execute("edit.transform", json!({"layers":[paint.0,text.0],"dx":5,"interpolation":"nearest"})).unwrap();
    assert_eq!(r["layers"], json!([paint.0, text.0]));
    assert_eq!(bounds(&s, paint), p0.translate(5, 0));
    assert_eq!(bounds(&s, text), t0.translate(5, 0));
    s.undo();
    let doc = s.active().unwrap().doc.clone();
    let steps = s.active().unwrap().history.past_len();
    assert!(s.execute("edit.transform", json!({"layers":[paint.0,text.0],"target":{"color":0},"dx":5})).is_err());
    assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc));
    assert_eq!(s.active().unwrap().history.past_len(), steps);
}

#[test]
fn panel_colour_moves_only_the_active_raster_when_several_layers_are_selected() {
    let (mut s, paint, text) = two_layers();
    s.execute("layer.select", json!({"layer":paint.0,"mode":"add"})).unwrap();
    s.execute("channel.target", json!({"channel":"red"})).unwrap();
    let t0 = text_matrix(&s, text);
    s.tools.background = [0.0, 0.0, 0.0, 1.0];
    let original = s.active().unwrap().doc.layer(paint).unwrap().surface().unwrap().clone();
    s.execute("edit.transform", json!({"dx":5,"interpolation":"nearest"})).unwrap();
    let st = s.active().unwrap();
    let after = st.doc.layer(paint).unwrap().surface().unwrap();
    assert_eq!(text_matrix(&s, text), t0);
    assert_eq!(after.read_region(Rect::new(12, 15, 13, 16)), vec![0.0, 0.0, 0.0, 1.0]);
    assert_eq!(after.read_region(Rect::new(52, 15, 53, 16)), vec![1.0, 0.0, 0.0, 0.0]);
    for y in 0..40 {
        for x in 0..60 {
            let (mut a, mut b) = ([0.0; 4], [0.0; 4]);
            after.read_pixel(x, y, &mut a);
            original.read_pixel(x, y, &mut b);
            assert_eq!(a[1..], b[1..]);
        }
    }
    s.undo();
    assert_eq!(
        s.active().unwrap().doc.layer(paint).unwrap().surface().unwrap().to_interleaved(Rect::new(0, 0, 60, 40)),
        original.to_interleaved(Rect::new(0, 0, 60, 40))
    );
}

#[test]
fn scalar_resamplers_observe_cancellation_during_a_plane() {
    use std::sync::atomic::{AtomicBool, Ordering};
    for selected in [false, true] {
        let fmt = photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
        let mut source = Surface::new(fmt);
        let area = Rect::new(0, 0, 256, 256);
        source.fill_rect(area, &[0.5]);
        let mut selection = Surface::new(fmt);
        selection.fill_rect(area, &[0.7]);
        let before = source.clone();
        let cancel = AtomicBool::new(false);
        let check = || cancel.load(Ordering::Relaxed);
        let progress = |_: f32| cancel.store(true, Ordering::Relaxed);
        let ctl = Interrupt::new(&check, &progress);
        let h = Homography([8.0, 0.0, 0.0, 0.0, 8.0, 0.0, 0.0, 0.0, 1.0]);
        let result =
            if selected { warp_gray_selected_with(&source, &selection, &h, Interp::Bicubic, &ctl) } else { warp_gray_with(&source, &h, Interp::Bicubic, &ctl) };
        assert!(cancel.load(Ordering::Relaxed), "the native resampler must report progress through the job's interrupt");
        assert!(matches!(result, Err(EngineError::Cancelled)));
        assert!(source == before);
    }
}

#[test]
fn ordinary_layer_transform_does_not_use_the_scalar_selection_area_cap() {
    let (mut s, _) = preset_layers(SampleType::U8);
    s.edit("large sparse selection", |doc, _| {
        let mut selection = Surface::new(photocraft_color::PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        selection.write_pixel(-10000, 0, &[1.0]);
        selection.write_pixel(10000, 10000, &[1.0]);
        doc.selection = Some(selection);
        Ok(())
    })
    .unwrap();
    let before = s.active().unwrap().doc.clone();
    s.execute("edit.transform", json!({"matrix":[1,0,0,1,0,0]})).unwrap();
    assert!(s.active().unwrap().doc.layers == before.layers);
    assert!(s.active().unwrap().doc.selection == before.selection);
}
