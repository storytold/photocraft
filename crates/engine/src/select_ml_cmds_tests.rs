use photocraft_genai::fake::{FakeNative, Options};
use photocraft_geom::Rect;
use serde_json::json;

use super::*;

/// A 64×48 document with one layer, talking to `url`.
fn session(url: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit_prefs(|p| {
        p.integrations.models_path = url.to_string();
        p.integrations.runtime_path = "fake".into();
        p.integrations.allow_research_models = false;
    });
    s
}

fn selection_bounds(s: &Session) -> Option<Rect> {
    s.active().unwrap().doc.selection.as_ref().map(|m| m.content_bounds())
}

#[test]
fn by_text_selects_what_the_model_found() {
    // The fake "finds" the centre quarter: x 16..48, y 12..36 on 64×48.
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    let steps = s.active().unwrap().history.past_len();
    let r = s.execute(BY_TEXT, json!({"prompt": "the lighthouse"})).unwrap();
    assert_eq!(r["selected"], true);
    assert_eq!(r["count"], 1);
    assert_eq!(r["instances"][0]["bounds"], json!([16, 12, 32, 24]));
    assert_eq!(r["instances"][0]["pixels"], 32 * 24);
    assert_eq!(selection_bounds(&s), Some(Rect::new(16, 12, 48, 36)));
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one undo step");

    let st = fake.state();
    assert_eq!(st.pictures.len(), 1, "the composite, no mask");
    let sent = st.pictures[0].clone();
    assert_eq!((sent.width, sent.height), (64, 48));
    let g = &st.runs[0].request;
    assert_eq!(g["prompt"], "the lighthouse");
    assert_eq!(g["params"]["threshold"], 0.5);
    assert_eq!(g["params"]["models"]["ckpt"], "sam3.1_multiplex_fp16.safetensors");
    drop(st);

    assert!(s.undo());
    assert_eq!(selection_bounds(&s), None);
}

#[test]
fn by_point_selects_the_object_under_the_point() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    let r = s.execute(BY_POINT, json!({"x": 30, "y": 20})).unwrap();
    assert_eq!(r["selected"], true);
    assert_eq!(selection_bounds(&s), Some(Rect::new(16, 12, 48, 36)), "what the fake detector found");
    let st = fake.state();
    let g = &st.runs[0].request;
    assert_eq!(g["params"]["points"], r#"[{"x":30,"y":20}]"#, "the point in the request's pixels");
    assert_eq!(g["params"]["models"]["ckpt"], "sam3.1_multiplex_fp16.safetensors");
    drop(st);
    for p in [json!({}), json!({"x": 30}), json!({"x": -1, "y": 2}), json!({"x": 64, "y": 2}), json!({"x": "a", "y": 2})] {
        assert!(matches!(s.execute(BY_POINT, p.clone()), Err(EngineError::BadParams { .. })), "{p}");
    }
}

/// The mask stack after the detector: `grow` widens (or shrinks) the mask, `fillHoles` fills
/// what it encloses, `soften` feathers its edge; validated, in that order.
#[test]
fn grow_fill_holes_and_soften_finish_the_detectors_mask() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    // The fake finds x 16..48, y 12..36; widened by 4 on every side.
    s.execute(BY_POINT, json!({"x": 30, "y": 20, "grow": 4})).unwrap();
    assert_eq!(selection_bounds(&s), Some(Rect::new(12, 8, 52, 40)));
    s.execute(BY_POINT, json!({"x": 30, "y": 20, "grow": -4})).unwrap();
    assert_eq!(selection_bounds(&s), Some(Rect::new(20, 16, 44, 32)), "shrunk");
    // Soften: a hard rectangle gets a feathered edge and keeps its centre.
    s.execute(BY_POINT, json!({"x": 30, "y": 20, "soften": 3})).unwrap();
    let m = s.active().unwrap().doc.selection.clone().unwrap();
    let inside = m.read_region(Rect::new(30, 20, 31, 21))[0];
    let edge = m.read_region(Rect::new(16, 12, 17, 13))[0];
    assert!(inside > 0.95 && edge < 0.75 && edge > 0.05, "feathered edge: {edge} vs {inside}");
    for p in [json!({"x": 1, "y": 1, "grow": 999}), json!({"x": 1, "y": 1, "soften": -1}), json!({"x": 1, "y": 1, "fillHoles": "yes"})] {
        assert!(matches!(s.execute(BY_POINT, p.clone()), Err(EngineError::BadParams { .. })), "{p}");
    }
    // Fill holes: a ring (and a notch open to the edge, which stays).
    let (w, h) = (8usize, 8usize);
    let mut ring = vec![0.0f32; w * h];
    for y in 1..7 {
        for x in 1..7 {
            ring[y * w + x] = 1.0;
        }
    }
    ring[3 * w + 3] = 0.0;
    ring[4 * w + 4] = 0.0;
    ring[w + 1] = 0.0;
    ring[w] = 0.0;
    let filled = fill_enclosed(&ring, w, h);
    assert_eq!((filled[3 * w + 3], filled[4 * w + 4]), (1.0, 1.0), "the enclosed gaps are filled");
    assert_eq!(filled[w + 1], 0.0, "the notch reaches the edge through (0, 1): kept");
    assert_eq!(filled[0], 0.0);
    assert_eq!(finish_coverage(ring.clone(), Rect::new(0, 0, 8, 8), 0.0, true, 0.0)[3 * w + 3], 1.0);
    assert_eq!(finish_coverage(ring.clone(), Rect::new(0, 0, 8, 8), 0.0, false, 0.0), ring, "nothing asked: untouched");
}

#[test]
fn a_box_and_negative_points_reach_the_detector_and_refine_softens_the_edge() {
    let fake = FakeNative::start_with(Options { missing_files: vec!["model.safetensors".into()], ..Options::default() }).unwrap();
    let mut s = session(&fake.url);
    // A box: the box template with PrimitiveBoundingBox in the request's pixels; no points.
    let r = s.execute(BY_POINT, json!({"box": [10, 8, 40, 30]})).unwrap();
    assert_eq!(r["selected"], true);
    {
        let st = fake.state();
        let g = &st.runs[0].request;
        assert_eq!((g["params"]["bx"].as_i64(), g["params"]["by"].as_i64()), (Some(10), Some(8)));
        assert_eq!((g["params"]["bw"].as_i64(), g["params"]["bh"].as_i64()), (Some(40), Some(30)));
        assert_eq!(g["params"]["points"], "[]");
        assert_eq!(g["params"]["negatives"], "[]");
    }
    assert_eq!(s.active().unwrap().history.entries().last().map(String::as_str), Some("Select by Box"));
    // Points to include and exclude, with x/y as the first positive.
    s.execute(BY_POINT, json!({"x": 30, "y": 20, "points": [[40, 20]], "negativePoints": [[5, 5], [60, 40]]})).unwrap();
    {
        let st = fake.state();
        let g = &st.runs[1].request;
        assert_eq!(g["params"]["points"], r#"[{"x":30,"y":20},{"x":40,"y":20}]"#);
        assert_eq!(g["params"]["negatives"], r#"[{"x":5,"y":5},{"x":60,"y":40}]"#);
        assert!(g["params"].get("bx").is_none(), "no box: the point template");
    }
    // A box with a negative size is normalised and clipped; a tiny one is refused.
    s.execute(BY_POINT, json!({"box": [50, 38, -40, -30]})).unwrap();
    assert_eq!(fake.state().runs[2].request["params"]["bx"], 10);
    for p in
        [json!({"box": [1, 1, 1, 1]}), json!({"box": [1, 2, 3]}), json!({"negativePoints": [[1, 1]]}), json!({"x": 1, "y": 1, "negativePoints": [[99, 1]]})]
    {
        assert!(matches!(s.execute(BY_POINT, p.clone()), Err(EngineError::BadParams { .. })), "{p}");
    }
    // Refine: the fake's hard rectangle gets partial coverage at its edge (the classical edge
    // refinement reads the pixels: a flat white picture gives it little, but the feather shows).
    s.execute(BY_POINT, json!({"x": 30, "y": 20, "refine": true})).unwrap();
    let m = s.active().unwrap().doc.selection.clone().unwrap();
    let inside = m.read_region(Rect::new(30, 20, 31, 21))[0];
    let edge = m.read_region(Rect::new(16, 12, 17, 13))[0];
    assert!(inside > 0.9, "{inside}");
    assert!(edge < inside, "the refined edge is softer than the centre: {edge} vs {inside}");
}

#[test]
fn a_probe_is_kept_for_the_click_that_follows_and_goes_stale_with_an_edit() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    let steps = s.active().unwrap().history.past_len();
    let journal = s.journal.len();
    let r = s.execute(PROBE, json!({"x": 30, "y": 20})).unwrap();
    let id = r["probe"].as_u64().unwrap();
    assert!(id > 0);
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(64), Some(48)), "the request size (small canvas: as is)");
    assert_eq!(r["bounds"], json!([16, 12, 32, 24]));
    let mask = r["mask"].as_str().unwrap();
    assert_eq!(mask.len(), (64 * 48usize).div_ceil(3) * 4, "base64 of width × height bytes");
    assert!(mask.ends_with("AAAA") && !mask.contains('='), "3072 bytes pad to nothing: {}", &mask[mask.len() - 8..]);
    assert_eq!(b64_encode(b"Man"), "TWFu");
    assert_eq!(b64_encode(b"Ma"), "TWE=");
    assert_eq!(b64_encode(b"M"), "TQ==");
    {
        let d = s.active().unwrap();
        assert!(d.doc.selection.is_none(), "a probe selects nothing");
        assert_eq!(d.history.past_len(), steps, "no undo step");
    }
    assert_eq!(s.journal.len(), journal, "not journaled");
    assert_eq!(fake.state().runs.len(), 1);
    assert_eq!(s.select_probes.len(), 1);
    // The click applies the kept probe: no second server call, one undo step.
    let r = s.execute(BY_POINT, json!({"x": 30, "y": 20, "probe": id})).unwrap();
    assert_eq!(r["probe"], true);
    assert_eq!(fake.state().runs.len(), 1);
    assert_eq!(selection_bounds(&s), Some(Rect::new(16, 12, 48, 36)));
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
    // After an edit the probe is stale: the server is asked.
    let r = s.execute(BY_POINT, json!({"x": 30, "y": 20, "probe": id, "mode": "add"})).unwrap();
    assert!(r.get("probe").is_none());
    assert_eq!(fake.state().runs.len(), 2);
    // An unknown probe too.
    s.execute(BY_POINT, json!({"x": 30, "y": 20, "probe": 999})).unwrap();
    assert_eq!(fake.state().runs.len(), 3);
    // Nothing under the pointer is an error that keeps nothing.
    let none = FakeNative::start_with(Options { segments: vec![], ..Options::default() }).unwrap();
    let mut s2 = session(&none.url);
    assert!(s2.execute(PROBE, json!({"x": 1, "y": 1})).is_err());
    assert!(s2.select_probes.is_empty());
    // Probes of big pictures go out small.
    let mut big = Session::new();
    big.execute("file.new", json!({"width": 2400, "height": 1800})).unwrap();
    big.edit_prefs(|p| {
        p.integrations.models_path = fake.url.clone();
        p.integrations.runtime_path = "fake".into();
        p.integrations.allow_research_models = false;
    });
    let r = big.execute(PROBE, json!({"x": 1200, "y": 900})).unwrap();
    assert!(r["width"].as_u64().unwrap() * r["height"].as_u64().unwrap() <= 768 * 768, "{r}");
    let sent = fake.state().pictures.last().unwrap().clone();
    assert!(u64::from(sent.width) * u64::from(sent.height) <= 768 * 768);
}

#[test]
fn modes_combine_with_the_current_selection() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    s.execute(BY_TEXT, json!({"prompt": "x", "mode": "add"})).unwrap();
    assert_eq!(selection_bounds(&s), Some(Rect::new(0, 0, 48, 36)), "union");
    s.execute(BY_TEXT, json!({"prompt": "x", "mode": "intersect"})).unwrap();
    assert_eq!(selection_bounds(&s), Some(Rect::new(16, 12, 48, 36)));
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 64, "height": 48})).unwrap();
    s.execute(BY_TEXT, json!({"prompt": "x", "mode": "subtract"})).unwrap();
    let m = s.active().unwrap().doc.selection.clone().unwrap();
    assert_eq!(m.read_region(Rect::new(30, 20, 31, 21)), vec![0.0], "hole where the object was");
    assert_eq!(m.read_region(Rect::new(2, 2, 3, 3)), vec![1.0]);
    s.execute(BY_TEXT, json!({"prompt": "x", "mode": "replace", "threshold": 0.3})).unwrap();
    assert_eq!(selection_bounds(&s), Some(Rect::new(16, 12, 48, 36)));
    assert_eq!(fake.state().runs.last().unwrap().request["params"]["threshold"], 0.3);
}

#[test]
fn instances_can_be_picked_one_at_a_time() {
    let fake = FakeNative::start_with(Options { segments: vec![[0.0, 0.0, 0.5, 1.0], [0.5, 0.0, 1.0, 1.0]], ..Options::default() }).unwrap();
    let mut s = session(&fake.url);
    let r = s.execute(BY_TEXT, json!({"prompt": "window"})).unwrap();
    assert_eq!(r["count"], 2);
    assert_eq!(selection_bounds(&s), Some(Rect::new(0, 0, 64, 48)), "all instances by default");
    s.execute(BY_TEXT, json!({"prompt": "window", "instance": 2})).unwrap();
    assert_eq!(selection_bounds(&s), Some(Rect::new(32, 0, 64, 48)));
    let e = s.execute(BY_TEXT, json!({"prompt": "window", "instance": 3})).unwrap_err();
    assert!(e.to_string().contains("only 2 instance"), "{e}");
    assert_eq!(selection_bounds(&s), Some(Rect::new(32, 0, 64, 48)), "a failed pick changes nothing");
}

#[test]
fn nothing_found_is_a_clear_error_that_changes_nothing() {
    let fake = FakeNative::start_with(Options { segments: vec![], ..Options::default() }).unwrap();
    let mut s = session(&fake.url);
    s.execute("select.rect", json!({"x": 1, "y": 1, "width": 5, "height": 5})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    let e = s.execute(BY_TEXT, json!({"prompt": "unicorn"})).unwrap_err();
    assert!(e.to_string().contains("nothing matching \"unicorn\""), "{e}");
    assert_eq!(selection_bounds(&s), Some(Rect::new(1, 1, 6, 6)));
    assert_eq!(s.active().unwrap().history.past_len(), steps);
}

#[test]
fn select_subject_ml_uses_a_fixed_phrase() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    s.execute(SUBJECT_ML, json!({})).unwrap();
    s.execute(SUBJECT_ML, json!({"what": "sky", "mode": "add"})).unwrap();
    let st = fake.state();
    assert_eq!(st.runs.len(), 2, "the main subject found at once: no fallback asked");
    assert_eq!(st.runs[0].request["prompt"], "the main subject");
    assert_eq!(st.runs[1].request["prompt"], "sky");
    drop(st);
    assert!(matches!(s.execute(SUBJECT_ML, json!({"what": "banana"})), Err(EngineError::BadParams { .. })));
}

#[test]
fn select_subject_falls_back_through_the_kinds_of_thing_a_picture_is_of_and_drops_specks() {
    // "the main subject" and "the foreground object" find nothing; "person" finds a figure and
    // a speck beside it.
    let fake = FakeNative::start_with(Options {
        segments_for: vec![
            ("the main subject".into(), vec![]),
            ("the foreground object".into(), vec![]),
            ("person".into(), vec![[0.25, 0.25, 0.75, 0.75], [0.9, 0.9, 0.92, 0.92]]),
        ],
        ..Options::default()
    })
    .unwrap();
    let mut s = session(&fake.url);
    let r = s.execute(SUBJECT_ML, json!({})).unwrap();
    let asked: Vec<String> = fake.state().runs.iter().map(|p| p.request["prompt"].as_str().unwrap_or_default().to_string()).collect();
    assert_eq!(asked, ["the main subject", "the foreground object", "person"]);
    assert_eq!(r["count"], 1, "the speck beside the figure is dropped: {r}");
    assert_eq!(selection_bounds(&s), Some(Rect::new(16, 12, 48, 36)));
    // Asked for by number, a speck stays an instance.
    let r = s.execute(BY_TEXT, json!({"prompt": "person", "instance": 2})).unwrap();
    assert_eq!(r["count"], 2);
    // Nothing anywhere: one clear error after the whole chain, the selection untouched.
    let none = FakeNative::start_with(Options { segments: vec![], ..Options::default() }).unwrap();
    let mut s2 = session(&none.url);
    let e = s2.execute(SUBJECT_ML, json!({})).unwrap_err().to_string();
    assert!(e.contains("nothing matching"), "{e}");
    assert_eq!(none.state().runs.len(), SUBJECT_FALLBACKS.len(), "every fallback was tried");
    assert!(s2.active().unwrap().doc.selection.is_none());
    // The server's own words for an empty mask batch read as "nothing found".
    let bad =
        FakeNative::start_with(Options { fail: Some("SaveImage: index 0 is out of bounds for dimension 0 with size 0".into()), ..Options::default() }).unwrap();
    let mut s3 = session(&bad.url);
    let e = s3.execute(BY_TEXT, json!({"prompt": "unicorn"})).unwrap_err().to_string();
    assert!(e.contains("nothing matching \"unicorn\""), "{e}");
}

#[test]
fn the_active_layer_can_be_sampled_instead_of_the_composite() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    s.execute(BY_TEXT, json!({"prompt": "x", "sampleAllLayers": false})).unwrap();
    let sent = fake.state().pictures[0].clone();
    assert_eq!((sent.width, sent.height), (64, 48));
    assert_eq!(sent.get(5, 5), Some([0, 0, 0, 0]), "the empty active layer is transparent, the white Background is not sampled");
}

#[test]
fn big_canvases_are_sent_downscaled_and_the_masks_come_back_at_canvas_size() {
    // 2400×1800 = 4.3 MP goes out at ≤ 2 MP; the fake finds the centre quarter of what it got.
    let fake = FakeNative::start().unwrap();
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 2400, "height": 1800})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit_prefs(|p| {
        p.integrations.models_path = fake.url.clone();
        p.integrations.runtime_path = "fake".into();
        p.integrations.allow_research_models = false;
    });
    let r = s.execute(BY_TEXT, json!({"prompt": "the lighthouse"})).unwrap();
    assert_eq!(r["count"], 1);
    let sent = fake.state().pictures[0].clone();
    assert!(u64::from(sent.width) * u64::from(sent.height) <= 2048 * 1024, "{}×{}", sent.width, sent.height);
    assert!(sent.width.is_multiple_of(16) && sent.height.is_multiple_of(16) && sent.width > sent.height);
    let b = selection_bounds(&s).unwrap();
    // The centre quarter of the canvas, give or take the resampling at the edge.
    assert!((b.x0 - 600).abs() <= 3 && (b.y0 - 450).abs() <= 3 && (b.x1 - 1800).abs() <= 3 && (b.y1 - 1350).abs() <= 3, "{b:?}");
    let pixels = r["instances"][0]["pixels"].as_u64().unwrap();
    assert!((pixels as i64 - 1200 * 900).abs() < 1200 * 900 / 50, "{pixels}");
}

#[test]
fn parameters_are_validated_and_the_command_needs_a_document() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    for p in [
        json!({}),
        json!({"prompt": ""}),
        json!({"prompt": "x", "mode": "weird"}),
        json!({"prompt": "x", "threshold": 2}),
        json!({"prompt": "x", "instance": 0}),
        json!({"prompt": "x", "instance": 1.5}),
        json!({"prompt": "x", "sampleAllLayers": "yes"}),
        json!({"prompt": "x", "template": "krea2-turbo/image"}),
    ] {
        let e = s.execute(BY_TEXT, p.clone()).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }), "{p}: {e}");
    }
    assert!(fake.state().runs.is_empty());
    let mut empty = Session::new();
    assert!(matches!(empty.execute(BY_TEXT, json!({"prompt": "x"})), Err(EngineError::Disabled(..))));
    let m = s.execute("generate.models", json!({})).unwrap();
    let sam = m["templates"].as_array().unwrap().iter().find(|t| t["id"] == "sam3.1/segment").unwrap();
    assert_eq!(sam["task"], "segment");
    assert_eq!(sam["license"], "community");
}

#[test]
fn select_subject_takes_its_edge_from_vitmatte_when_served() {
    // The fake's ViTMatte answers opaque: the selection is the detector's mask grown by the
    // trimap's unknown band, solid inside, nothing far out.
    let fake = FakeNative::start_with(Options { segments: vec![[0.25, 0.25, 0.75, 0.75]], ..Options::default() }).unwrap();
    let mut s = session(&fake.url);
    s.execute(SUBJECT_ML, json!({"refine": true})).unwrap();
    let m = s.active().unwrap().doc.selection.clone().unwrap();
    assert!(m.read_region(Rect::new(32, 24, 33, 25))[0] > 0.99);
    assert!(m.read_region(Rect::new(0, 0, 1, 1))[0] < 0.01);
    assert!(fake.state().runs.iter().any(|r| r.request["model"] == "vitmatte/refine"), "ViTMatte asked");
}

#[test]
fn vitmatte_tiles_a_large_crop_and_feathers_the_overlaps() {
    // A crop wider than one tile goes out in overlapping tiles at full size; tiles without
    // unknown pixels are not sent; the overlap blends to the same constant.
    let fake = FakeNative::start().unwrap();
    let backend = crate::generate_cmds::backend_for(&session(&fake.url), VITMATTE_TEMPLATE).unwrap().0;
    let (cw, ch) = (2000usize, 300usize);
    let px = vec![128u8; cw * ch * 4];
    let mut tri = vec![255u8; cw * ch];
    for t in tri.iter_mut().take(cw * 10) {
        *t = 128;
    }
    let ctx = crate::jobs::JobCtx::new();
    let a = matte_tiles(backend.as_ref(), &px, &tri, cw, ch, (0.0, 1.0), &ctx).unwrap().unwrap();
    assert_eq!(a.len(), cw * ch);
    assert!(a.iter().all(|v| (*v - 1.0).abs() < 1e-4), "the fake answers opaque; the trimap's 255 elsewhere");
    let st = fake.state();
    let sizes: Vec<_> = st.runs.iter().filter(|r| r.request["model"] == VITMATTE_TEMPLATE).map(|r| r.request["image"]["width"].clone()).collect();
    assert_eq!(sizes.len(), 3, "three 1024 tiles across 2000 px, overlapping by at least 128: {sizes:?}");
}
