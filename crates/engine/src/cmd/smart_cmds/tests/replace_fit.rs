//! #11529: Replace Contents fits the new contents into the frame the old ones filled.
//!
//! The report: a vector (`.ai`) smart object from a print file, Replace Contents with a PNG. The
//! old transform was kept, so the art rendered about eight times too large and `warnings` came
//! back empty. Photoshop keeps the frame — its position, rotation and size — and puts the new art
//! into it; and it is the same frame every further Replace Contents puts art into, because the fit
//! maps each replacement's box exactly onto it. A warp belongs to the contents' source space, so
//! it follows the fit instead of being left behind on the replaced contents' pixels.

use super::*;
use photocraft_doc::VectorPreview;
use photocraft_geom::warp::{BezierMesh, Warp, WarpStyle};

/// `.ai` contents as PSD import keeps them: bytes for the external editor, which PhotoCraft
/// stores but can't draw itself.
const AI: &[u8] = b"%PDF-1.7\nillustrator artwork";
/// The vector's source frame, in source pixels: 20 × 10.
const PREVIEW: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#336699"/></svg>"##;
/// The frame those contents fill when the object is placed at (4, 6): 20 × 10 document pixels.
const FRAME: [[f64; 2]; 4] = [[4.0, 6.0], [24.0, 6.0], [24.0, 16.0], [4.0, 16.0]];

/// A document holding one vector smart object named `name`, placed by `transform`, its `.ai`
/// contents shown from the preview PSD import stores beside them when `preview`.
fn vector_session(name: &str, transform: Affine, preview: bool) -> (Session, LayerId) {
    let mut s = session(8);
    let mut sm = SmartObject::new(SmartSource::Embedded { file_name: "Logo.ai".into(), bytes: Arc::new(AI.to_vec()) }, transform, None);
    if preview {
        sm.vector_preview = Some(VectorPreview { source_hash: *blake3::hash(AI).as_bytes(), svg: Arc::new(PREVIEW.as_bytes().to_vec()) });
    }
    let layer = Layer::new(name, LayerContent::Smart(sm));
    let id = layer.id;
    s.edit("vector smart object", |doc, active| {
        doc.layers.push(layer);
        *active = Some(id);
        Ok(())
    })
    .unwrap();
    (s, id)
}

fn smart_of(doc: &Document, id: LayerId) -> SmartObject {
    match &doc.layer(id).unwrap().content {
        LayerContent::Smart(sm) => sm.clone(),
        other => panic!("not a smart object: {}", other.kind_name()),
    }
}

/// The first smart object of `doc`, as a PSD or `.pcraft` reopen lists it.
fn first_smart(doc: &Document) -> (LayerId, SmartObject) {
    doc.walk().into_iter().find_map(|(_, _, l)| matches!(&l.content, LayerContent::Smart(_)).then(|| (l.id, smart_of(doc, l.id)))).expect("a smart object")
}

/// Sets the object's warp, as a Warp-mode commit or a PSD import would.
fn set_warp(s: &mut Session, id: LayerId, w: Warp) {
    s.edit("warp", |doc, _| {
        let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!("smart object") };
        sm.warp = Some(w);
        Ok(())
    })
    .unwrap();
}

/// The corners of the contents' source frame in document pixels, clockwise from its top-left:
/// the frame Photoshop keeps when it replaces contents.
fn corners(doc: &Document, sm: &SmartObject) -> [[f64; 2]; 4] {
    let [x0, y0, x1, y1] = source_box(doc, sm).expect("the contents' source frame");
    let h = placement(sm);
    [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| {
        let (px, py) = h.apply(x, y);
        [px, py]
    })
}

fn same_corners(a: &[[f64; 2]; 4], b: &[[f64; 2]; 4]) -> bool {
    near(a, b, 1e-9)
}

fn near(a: &[[f64; 2]], b: &[[f64; 2]], tol: f64) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(p, q)| p.iter().zip(q).all(|(x, y)| (x - y).abs() < tol))
}

/// The warp box's corners and edge midpoints, warped and placed: the frame as a warped object
/// actually stands in it.
fn warped_quad(sm: &SmartObject) -> Vec<[f64; 2]> {
    let w = sm.warp.clone().expect("a warp");
    let [x0, y0, x1, y1] = w.bounds;
    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let h = placement(sm);
    [(x0, y0), (mx, y0), (x1, y0), (x1, my), (x1, y1), (mx, y1), (x0, y1), (x0, my)]
        .iter()
        .map(|&(x, y)| {
            let (u, v) = w.map(x, y);
            let (px, py) = h.apply(u, v);
            [px, py]
        })
        .collect()
}

fn bbox(points: &[[f64; 2]]) -> [f64; 4] {
    points.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

/// An opaque red PNG of `w` × `h` px, written into `dir`.
fn png(dir: &std::path::Path, name: &str, w: u32, h: u32) -> String {
    let doc = photocraft_doc::Document::with_background(
        name,
        Size::new(w, h),
        photocraft_color::ColorMode::Rgb,
        photocraft_color::SampleType::U8,
        photocraft_color::Color::rgba(1.0, 0.0, 0.0, 1.0),
    );
    let path = dir.join(name);
    std::fs::write(&path, photocraft_io::export(&doc, "png", &Default::default()).unwrap().bytes).unwrap();
    path.to_str().unwrap().to_string()
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pcraft-replace-fit-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Replaces `id`'s contents with a fresh PNG of `w` × `h` px and returns the command's result.
fn replace(s: &mut Session, dir: &std::path::Path, id: LayerId, name: &str, w: u32, h: u32) -> Value {
    s.execute("layer.smartObjects.replaceContents", json!({"layer": id.0, "path": png(dir, name, w, h)})).unwrap()
}

/// The issue's case: a vector smart object whose `.ai` contents measure 20 × 10 source pixels,
/// replaced by a PNG four times its pixel size. The frame is what stays, so the art can't render
/// too large, and a replacement of the frame's own shape has nothing to report.
#[test]
fn a_larger_raster_replacement_is_fitted_into_the_vectors_frame() {
    let dir = temp_dir("fit");
    let (mut s, id) = vector_session("Topper", Affine::translate(4.0, 6.0), true);
    let doc = s.active().unwrap().doc.clone();
    let before = corners(&doc, &smart_of(&doc, id));
    assert_eq!(before, FRAME, "the vector fills 20 × 10 px");
    let r = replace(&mut s, &dir, id, "wide.png", 40, 20);
    assert_eq!(r["warnings"], json!([]), "a replacement of the frame's shape says nothing: {r}");
    let doc = s.active().unwrap().doc.clone();
    let sm = smart_of(&doc, id);
    assert!(same_corners(&corners(&doc, &sm), &before), "the frame the vector filled is the frame the PNG fills: {:?}", corners(&doc, &sm));
    assert_eq!(sm.transform.m, [0.5, 0.0, 0.0, 0.5, 4.0, 6.0], "a uniform scale of the placement, its shape kept");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A rotated and skewed placement is a frame too: Replace Contents keeps it whole.
#[test]
fn a_replacement_fits_a_rotated_frame_without_turning_it_straight() {
    let dir = temp_dir("rotated");
    let placed_at = Affine::translate(4.0, 6.0).mul(&Affine { m: [2.0, 0.5, -0.25, 1.5, 0.0, 0.0] });
    let (mut s, id) = vector_session("Topper", placed_at, true);
    let doc = s.active().unwrap().doc.clone();
    let before = corners(&doc, &smart_of(&doc, id));
    replace(&mut s, &dir, id, "wide.png", 40, 20);
    let doc = s.active().unwrap().doc.clone();
    assert!(same_corners(&corners(&doc, &smart_of(&doc, id)), &before), "the tilted frame is unchanged: {:?}", corners(&doc, &smart_of(&doc, id)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Distort and Perspective store a projective map; the fit composes onto it and leaves it
/// projective, so the fourth corner survives replacing the contents.
#[test]
fn a_replacement_fits_a_perspective_frame_and_keeps_it_projective() {
    let dir = temp_dir("perspective");
    let (mut s, id) = vector_session("Topper", Affine::translate(4.0, 6.0), true);
    s.edit("distort", |doc, _| {
        let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!("smart object") };
        sm.perspective = Some([1.0, 0.0, 4.0, 0.25, 1.0, 6.0, 0.002, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let doc = s.active().unwrap().doc.clone();
    let before = corners(&doc, &smart_of(&doc, id));
    replace(&mut s, &dir, id, "wide.png", 40, 20);
    let doc = s.active().unwrap().doc.clone();
    let sm = smart_of(&doc, id);
    assert!(sm.perspective.is_some(), "a Distort or Perspective placement isn't flattened by a replace");
    assert!(same_corners(&corners(&doc, &sm), &before), "the projective frame is unchanged: {:?}", corners(&doc, &sm));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Contents of another shape fill the frame rather than sitting small in it — the report's own
/// workaround dragged the corners onto the frame, which is a stretch — and the stretch is
/// reported (#11529 asks for the empty list to stop meaning "nothing was reported").
#[test]
fn a_replacement_of_another_shape_fills_the_frame_and_says_so() {
    let dir = temp_dir("shape");
    let (mut s, id) = vector_session("Topper", Affine::translate(4.0, 6.0), true);
    let r = replace(&mut s, &dir, id, "square.png", 20, 20);
    assert_eq!(
        r["warnings"],
        json!(["layer \"Topper\": the new contents are 20 × 20 px and the frame they replace is 20 × 10 px, so they are stretched to fill the frame"]),
        "{r}"
    );
    let doc = s.active().unwrap().doc.clone();
    let sm = smart_of(&doc, id);
    assert!(same_corners(&corners(&doc, &sm), &FRAME), "the square spans the whole frame, not half of it: {:?}", corners(&doc, &sm));
    assert_eq!(sm.transform.m, [1.0, 0.0, 0.0, 0.5, 4.0, 6.0], "per-axis, so the frame's shape wins");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The review's chained case: each Replace Contents fills the frame the object has, so a swap back
/// to the original shape fills it again instead of creeping into the previous contents' smaller
/// footprint (20 × 10 → 20 × 20 → 20 × 10 came out 10 × 5 px).
#[test]
fn a_chain_of_replacements_always_fills_the_same_frame() {
    let dir = temp_dir("chain");
    let (mut s, id) = vector_session("Topper", Affine::translate(4.0, 6.0), true);
    for (name, w, h) in [("square.png", 20u32, 20), ("wide.png", 40, 20), ("same.png", 20, 10), ("tall.png", 10, 40)] {
        let r = replace(&mut s, &dir, id, name, w, h);
        let doc = s.active().unwrap().doc.clone();
        let got = corners(&doc, &smart_of(&doc, id));
        assert!(same_corners(&got, &FRAME), "after replacing with {name} the frame is still the frame: {got:?}");
        assert!(r["warnings"].as_array().is_some_and(|v| v.len() <= 1), "at most the one shape report: {r}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The frame is not extra state to keep in step with the object: it is the placement of the
/// current source box, so it is exactly as durable as the transform and the contents it is read
/// from. Replace, save the document as `.pcraft`, reopen, replace again: still the same frame.
#[test]
fn the_frame_survives_a_pcraft_save_and_reopen() {
    let dir = temp_dir("pcraft");
    let (mut s, id) = vector_session("Topper", Affine::translate(4.0, 6.0), true);
    replace(&mut s, &dir, id, "square.png", 20, 20);
    let saved = encode_source(&s.active().unwrap().doc).unwrap();
    let mut s = Session::new();
    s.add_document(decode_source("Topper.pcraft", &saved).unwrap(), None);
    let doc = s.active().unwrap().doc.clone();
    let (id, restored) = first_smart(&doc);
    assert!(same_corners(&corners(&doc, &restored), &FRAME), "the reopened frame is the frame: {:?}", corners(&doc, &restored));
    replace(&mut s, &dir, id, "wide.png", 40, 20);
    let doc = s.active().unwrap().doc.clone();
    assert!(same_corners(&corners(&doc, &smart_of(&doc, id)), &FRAME), "and the next replacement fills it");
    let _ = std::fs::remove_dir_all(&dir);
}

/// PSD is the format the report's print file came in as: the frame is read back out of the saved
/// placed layer, so a replacement that goes through a PSD save and reopen still fills the box the
/// layer was converted with.
#[test]
fn the_frame_survives_a_psd_save_and_reopen() {
    let dir = temp_dir("psd");
    let mut s = session(8);
    paint(&mut s);
    convert(&mut s);
    let (id, sm) = first_smart(&s.active().unwrap().doc);
    let original = corners(&s.active().unwrap().doc, &sm);
    assert_eq!(bbox(&original), [-6.0, 5.0, 40.0, 38.0], "the converted layer's own box");
    replace(&mut s, &dir, id, "square.png", 8, 8);
    let bytes = photocraft_io::export(&s.active().unwrap().doc, "psd", &Default::default()).unwrap().bytes;
    let mut s = Session::new();
    s.add_document(photocraft_io::import("topper.psd", &bytes).unwrap().document, None);
    let doc = s.active().unwrap().doc.clone();
    let (id, reopened) = first_smart(&doc);
    assert!(near(&corners(&doc, &reopened), original.as_slice(), 0.5), "PSD keeps the frame: {original:?} then {:?}", corners(&doc, &reopened));
    replace(&mut s, &dir, id, "wide.png", 92, 66);
    let doc = s.active().unwrap().doc.clone();
    let after = corners(&doc, &smart_of(&doc, id));
    assert!(near(&after, original.as_slice(), 0.5), "a replace after a PSD round trip fills it again: {original:?} then {after:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Contents already the size of the frame: nothing moves and nothing is said, so a `.psd` edited
/// in place or an action that replaces and re-replaces stays quiet and lossless.
#[test]
fn a_replacement_of_the_same_frame_is_left_alone_and_silent() {
    let dir = temp_dir("same");
    let (mut s, id) = vector_session("Topper", Affine::translate(4.0, 6.0), true);
    let r = replace(&mut s, &dir, id, "same.png", 20, 10);
    assert_eq!(r["warnings"], json!([]), "{r}");
    let doc = s.active().unwrap().doc.clone();
    assert_eq!(smart_of(&doc, id).transform, Affine::translate(4.0, 6.0), "the placement is bit-for-bit what it was");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Copies of one smart object each keep their own frame, so one Replace Contents fits every
/// instance's new art into the box that instance stands in.
#[test]
fn each_shared_instance_keeps_its_own_frame() {
    let dir = temp_dir("instances");
    let (mut s, id) = vector_session("Topper", Affine::translate(4.0, 6.0), true);
    let copy = {
        let doc = s.active().unwrap().doc.clone();
        let mut l = doc.layer(id).unwrap().clone();
        l.id = LayerId::fresh();
        l.name = "Copy".into();
        if let LayerContent::Smart(sm) = &mut l.content {
            sm.transform = Affine::translate(30.0, 20.0);
        }
        let id = l.id;
        s.edit("second instance", move |doc, _| {
            doc.layers.push(l);
            Ok(())
        })
        .unwrap();
        id
    };
    let r = replace(&mut s, &dir, id, "wide.png", 40, 20);
    assert_eq!(r["warnings"], json!([]), "{r}");
    let doc = s.active().unwrap().doc.clone();
    for (id, x0, y0) in [(id, 4.0, 6.0), (copy, 30.0, 20.0)] {
        let got = corners(&doc, &smart_of(&doc, id));
        assert!(near(&got, &[[x0, y0], [x0 + 20.0, y0], [x0 + 20.0, y0 + 10.0], [x0, y0 + 10.0]], 1e-9), "instance {} keeps its own frame: {:?}", id.0, got);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Contents whose frame can't be measured at all (no preview, no PSD placed layer): the placement
/// has to stay as it was, which is the one case the issue accepts — provided it is reported.
#[test]
fn an_unmeasurable_frame_keeps_the_transform_and_says_so() {
    let dir = temp_dir("unmeasurable");
    let (mut s, id) = vector_session("Topper", Affine::translate(4.0, 6.0), false);
    let r = replace(&mut s, &dir, id, "wide.png", 40, 20);
    assert_eq!(
        r["warnings"],
        json!(["layer \"Topper\": the frame of the replaced contents could not be measured, so its transform was kept: the new contents may not fit it"]),
        "{r}"
    );
    let doc = s.active().unwrap().doc.clone();
    assert_eq!(smart_of(&doc, id).transform, Affine::translate(4.0, 6.0), "nothing was scaled without measuring");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A custom warp lives in the contents' source space, so a replacement moves it with the fit: its
/// box becomes the new contents' box and its points step back by the fit's inverse, so the warped
/// art keeps spanning the same warped frame instead of shrinking to half of it with the placement.
#[test]
fn a_custom_warp_follows_the_fit_into_the_new_contents() {
    let dir = temp_dir("warp-custom");
    let (mut s, id) = vector_session("Topper", Affine::translate(4.0, 6.0), true);
    // A bow: the middle columns sag, so the warped frame is not the plain box.
    let mesh = BezierMesh::fit(&|u, v| [20.0 * u, 10.0 * v + 6.0 * u * (1.0 - u)], vec![0.0, 0.5, 1.0], vec![0.0, 0.5, 1.0]);
    set_warp(&mut s, id, Warp::custom(mesh, [0.0, 0.0, 20.0, 10.0]));
    let before = warped_quad(&smart_of(&s.active().unwrap().doc, id));
    let box_before = bbox(&before);
    assert!(box_before[3] - box_before[1] > 10.0, "the bow reaches past the 10 px box: {box_before:?}");
    let r = replace(&mut s, &dir, id, "wide.png", 40, 20);
    assert_eq!(r["warnings"], json!([]), "the warp followed the fit, so there is nothing to report: {r}");
    let doc = s.active().unwrap().doc.clone();
    let sm = smart_of(&doc, id);
    let warp = sm.warp.clone().expect("the warp is still there");
    assert_eq!(warp.style, WarpStyle::Custom);
    assert_eq!(warp.bounds, [0.0, 0.0, 40.0, 20.0], "the warp is re-based on the new contents' pixels");
    let after = warped_quad(&sm);
    assert!(near(&after, &before, 1e-9), "the warped art spans the frame it spanned before, not half of it: {before:?} then {after:?}");
    assert_eq!(bbox(&after), box_before, "same warped extent");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A uniform fit is a similarity, so a preset stays a preset: the same bend percentage over the
/// new contents' box, and the warped art still spans the same warped frame.
#[test]
fn a_uniform_fit_keeps_a_preset_warp_a_preset() {
    let dir = temp_dir("warp-preset");
    let (mut s, id) = vector_session("Topper", Affine::translate(4.0, 6.0), true);
    set_warp(&mut s, id, Warp::preset(WarpStyle::Arc, 50.0, [0.0, 0.0, 20.0, 10.0]));
    let before = warped_quad(&smart_of(&s.active().unwrap().doc, id));
    let r = replace(&mut s, &dir, id, "wide.png", 40, 20);
    assert_eq!(r["warnings"], json!([]), "{r}");
    let doc = s.active().unwrap().doc.clone();
    let sm = smart_of(&doc, id);
    let warp = sm.warp.clone().expect("the warp is still there");
    assert_eq!(warp.style, WarpStyle::Arc, "a similarity keeps the preset, so its editor and its PSD save do");
    assert_eq!((warp.bounds, warp.bend), ([0.0, 0.0, 40.0, 20.0], 50.0), "re-based on the new contents, same bend");
    let after = warped_quad(&sm);
    assert!(near(&after, &before, 1e-9), "the arc spans the same warped frame: {before:?} then {after:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A stretched fit is not a similarity: one bend percentage would then measure the wrong axis, so
/// a preset is stored as the mesh that draws it exactly (as a rotated Warp commit stores a
/// cylinder as its mesh), while a cylinder keeps its seven values with the depth following the
/// change of aspect, so a PSD save still writes `warpCylinder`. Either way the warped art keeps
/// spanning the same warped frame instead of a wrong fraction of it.
#[test]
fn a_stretched_fit_keeps_the_warp_bending_the_same_way() {
    let dir = temp_dir("warp-stretch");
    for (style, values) in [(WarpStyle::Arc, Vec::new()), (WarpStyle::Cylinder, vec![0.0, 0.0, 20.0, 10.0, 0.3, 0.3, 0.5])] {
        let (mut s, id) = vector_session("Topper", Affine::translate(4.0, 6.0), true);
        let w = match style {
            WarpStyle::Cylinder => Warp::cylinder(&values, [0.0, 0.0, 20.0, 10.0]).expect("a valid cylinder"),
            st => Warp::preset(st, 50.0, [0.0, 0.0, 20.0, 10.0]),
        };
        set_warp(&mut s, id, w);
        let before = warped_quad(&smart_of(&s.active().unwrap().doc, id));
        replace(&mut s, &dir, id, "square.png", 20, 20);
        let doc = s.active().unwrap().doc.clone();
        let sm = smart_of(&doc, id);
        let warp = sm.warp.clone().expect("the warp is still there");
        let want = if style == WarpStyle::Cylinder { WarpStyle::Cylinder } else { WarpStyle::Custom };
        assert_eq!(warp.style, want, "{style:?} under a stretch");
        assert_eq!(warp.bounds, [0.0, 0.0, 20.0, 20.0], "re-based on the new contents' pixels");
        if style == WarpStyle::Cylinder {
            // The fit halves y onto the frame, so the box maps up and the ellipses' depth, in
            // units of the half axis, follows the change of aspect.
            assert_eq!(warp.values, vec![0.0, 0.0, 20.0, 20.0, 0.6, 0.6, 0.5], "the edges map, the depth follows the aspect");
        }
        let after = warped_quad(&sm);
        assert!(near(&after, &before, 1e-9), "{style:?} spans the same warped frame: {before:?} then {after:?}");
        assert!(near(&corners(&doc, &sm), FRAME.as_slice(), 1e-9), "{style:?} still fills the frame");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The report's other complaint: an `.ai` file edited elsewhere is refused with
/// `codec: unrecognized image format`, which says nothing about what to do instead. PhotoCraft
/// has no AI, PDF or EPS importer, so the refusal names the way out and leaves the document alone.
#[test]
fn an_edited_ai_replacement_is_refused_with_the_way_out() {
    let dir = temp_dir("ai-sniff");
    let edited = dir.join("Topper edited.ai");
    std::fs::write(&edited, b"%PDF-1.6\nre-saved by another app").unwrap();
    let (mut s, id) = vector_session("Topper", Affine::translate(4.0, 6.0), true);
    let before = s.active().unwrap().doc.clone();
    let err = s.execute("layer.smartObjects.replaceContents", json!({"layer": id.0, "path": edited.to_str().unwrap()})).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("doesn't open AI, PDF or EPS artwork"), "{msg}");
    assert!(msg.contains("layer.smartObjects.updateVectorSource"), "{msg}");
    assert!(!msg.contains("unrecognized image format"), "{msg}");
    assert_eq!(s.active().unwrap().doc, before, "a refused replace changes nothing");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The clearer refusal replaces `codec: unrecognized image format`; it adds no new refusal. A
/// readable file whose name claims a vector format still replaces the contents (#11529).
#[test]
fn a_readable_replacement_named_like_a_vector_still_replaces() {
    let dir = temp_dir("misnamed");
    let (mut s, id) = vector_session("Topper", Affine::translate(4.0, 6.0), true);
    let r = s.execute("layer.smartObjects.replaceContents", json!({"layer": id.0, "path": png(&dir, "actually-a-png.pdf", 40, 20)})).unwrap();
    assert_eq!(r["warnings"], json!([]), "the file reads, so its name says nothing: {r}");
    let doc = s.active().unwrap().doc.clone();
    assert!(same_corners(&corners(&doc, &smart_of(&doc, id)), &FRAME), "and it is fitted like any other");
    let _ = std::fs::remove_dir_all(&dir);
}
