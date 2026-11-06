//! #11529: Replace Contents fits the new contents into the frame the old ones filled.
//!
//! The report: a vector (`.ai`) smart object from a print file, Replace Contents with a PNG. The
//! old transform was kept, so the art rendered about eight times too large and `warnings` came
//! back empty. Photoshop keeps the frame — its position, rotation and size — and puts the new
//! art into it.

use super::*;
use photocraft_doc::VectorPreview;

/// `.ai` contents as PSD import keeps them: bytes for the external editor, which PhotoCraft
/// stores but can't draw itself.
const AI: &[u8] = b"%PDF-1.7\nillustrator artwork";
/// The vector's source frame, in source pixels: 20 × 10.
const PREVIEW: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#336699"/></svg>"##;

/// A document holding one vector smart object named `name`, placed by `transform`.
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
    a.iter().zip(b).all(|(p, q)| p.iter().zip(q).all(|(x, y)| (x - y).abs() < 1e-9))
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

/// The issue's case: a vector smart object whose `.ai` contents measure 20 × 10 source pixels,
/// replaced by a PNG four times its pixel size. The frame is what stays, so the art can't render
/// too large, and a replacement of the frame's own shape has nothing to report.
#[test]
fn a_larger_raster_replacement_is_fitted_into_the_vectors_frame() {
    let dir = temp_dir("fit");
    let (mut s, id) = vector_session("Topper", Affine::translate(4.0, 6.0), true);
    let before = corners(&s.active().unwrap().doc, &smart_of(&s.active().unwrap().doc, id));
    assert_eq!(before, [[4.0, 6.0], [24.0, 6.0], [24.0, 16.0], [4.0, 16.0]], "the vector fills 20 × 10 px");
    let r = s.execute("layer.smartObjects.replaceContents", json!({"layer": id.0, "path": png(&dir, "wide.png", 40, 20)})).unwrap();
    assert_eq!(r["warnings"], json!([]), "a replacement that fills the frame says nothing: {r}");
    let doc = s.active().unwrap().doc.clone();
    let sm = smart_of(&doc, id);
    assert!(same_corners(&corners(&doc, &sm), &before), "the frame the vector filled is the frame the PNG fills: {:?}", corners(&doc, &sm));
    assert_eq!(sm.transform.m, [0.5, 0.0, 0.0, 0.5, 4.0, 6.0], "the fit is a scale of the placement, not a new one");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A rotated and skewed placement is a frame too: Replace Contents keeps it whole.
#[test]
fn a_replacement_fits_a_rotated_frame_without_turning_it_straight() {
    let dir = temp_dir("rotated");
    let placed = Affine::translate(4.0, 6.0).mul(&Affine { m: [2.0, 0.5, -0.25, 1.5, 0.0, 0.0] });
    let (mut s, id) = vector_session("Topper", placed, true);
    let before = corners(&s.active().unwrap().doc, &smart_of(&s.active().unwrap().doc, id));
    s.execute("layer.smartObjects.replaceContents", json!({"layer": id.0, "path": png(&dir, "wide.png", 40, 20)})).unwrap();
    let doc = s.active().unwrap().doc.clone();
    let sm = smart_of(&doc, id);
    assert!(same_corners(&corners(&doc, &sm), &before), "the tilted frame is unchanged: {:?}", corners(&doc, &sm));
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
    let before = corners(&s.active().unwrap().doc, &smart_of(&s.active().unwrap().doc, id));
    s.execute("layer.smartObjects.replaceContents", json!({"layer": id.0, "path": png(&dir, "wide.png", 40, 20)})).unwrap();
    let doc = s.active().unwrap().doc.clone();
    let sm = smart_of(&doc, id);
    assert!(sm.perspective.is_some(), "a Distort or Perspective placement isn't flattened by a replace");
    assert!(same_corners(&corners(&doc, &sm), &before), "the projective frame is unchanged: {:?}", corners(&doc, &sm));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The other half of the parity question: art of another shape can't fill the frame without being
/// distorted, so it is fitted inside it, centred, and `warnings` says so (#11529 asks for the
/// empty list to stop meaning "nothing was reported").
#[test]
fn a_replacement_of_another_shape_is_fitted_inside_the_frame_and_says_so() {
    let dir = temp_dir("shape");
    let (mut s, id) = vector_session("Topper", Affine::translate(4.0, 6.0), true);
    let r = s.execute("layer.smartObjects.replaceContents", json!({"layer": id.0, "path": png(&dir, "square.png", 20, 20)})).unwrap();
    assert_eq!(
        r["warnings"],
        json!(["layer \"Topper\": the new contents are 20 × 20 px and the frame was made for 20 × 10 px, so they are fitted inside it and don't fill it"]),
        "{r}"
    );
    let doc = s.active().unwrap().doc.clone();
    let sm = smart_of(&doc, id);
    let got = corners(&doc, &sm);
    // Half the frame's width free, split evenly: centred, not pushed into a corner.
    assert_eq!(got, [[9.0, 6.0], [19.0, 6.0], [19.0, 16.0], [9.0, 16.0]], "the square is centred in the 20 × 10 px frame");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Contents already the size of the frame: nothing moves and nothing is said, so a `.psd` edited
/// in place or an action that replaces and re-replaces stays quiet and lossless.
#[test]
fn a_replacement_of_the_same_frame_is_left_alone_and_silent() {
    let dir = temp_dir("same");
    let (mut s, id) = vector_session("Topper", Affine::translate(4.0, 6.0), true);
    let r = s.execute("layer.smartObjects.replaceContents", json!({"layer": id.0, "path": png(&dir, "same.png", 20, 10)})).unwrap();
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
    let r = s.execute("layer.smartObjects.replaceContents", json!({"layer": id.0, "path": png(&dir, "wide.png", 40, 20)})).unwrap();
    assert_eq!(r["warnings"], json!([]), "{r}");
    let doc = s.active().unwrap().doc.clone();
    for (id, x0, y0) in [(id, 4.0, 6.0), (copy, 30.0, 20.0)] {
        let got = corners(&doc, &smart_of(&doc, id));
        assert_eq!(got, [[x0, y0], [x0 + 20.0, y0], [x0 + 20.0, y0 + 10.0], [x0, y0 + 10.0]], "instance {} keeps its own frame", id.0);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Contents whose frame can't be measured at all (no preview, no PSD placed layer): the placement
/// has to stay as it was, which is the one case the issue accepts — provided it is reported.
#[test]
fn an_unmeasurable_frame_keeps_the_transform_and_says_so() {
    let dir = temp_dir("unmeasurable");
    let (mut s, id) = vector_session("Topper", Affine::translate(4.0, 6.0), false);
    let r = s.execute("layer.smartObjects.replaceContents", json!({"layer": id.0, "path": png(&dir, "wide.png", 40, 20)})).unwrap();
    assert_eq!(
        r["warnings"],
        json!(["layer \"Topper\": the frame of the replaced contents could not be measured, so its transform was kept: the new contents may not fit it"]),
        "{r}"
    );
    let doc = s.active().unwrap().doc.clone();
    assert_eq!(smart_of(&doc, id).transform, Affine::translate(4.0, 6.0), "nothing was scaled without measuring");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The report's other complaint: an `.ai` file edited elsewhere is refused with
/// `codec: unrecognized image format`, which says nothing about what to do instead. PhotoCraft
/// has no AI, PDF or EPS importer, so the refusal names the way out (its own vector handoff) and
/// leaves the document alone.
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
    let path = png(&dir, "actually-a-png.pdf", 40, 20);
    let r = s.execute("layer.smartObjects.replaceContents", json!({"layer": id.0, "path": path})).unwrap();
    assert_eq!(r["warnings"], json!([]), "the file reads, so its name says nothing: {r}");
    let doc = s.active().unwrap().doc.clone();
    assert!(same_corners(&corners(&doc, &smart_of(&doc, id)), &[[4.0, 6.0], [24.0, 6.0], [24.0, 16.0], [4.0, 16.0]]), "and it is fitted like any other");
    let _ = std::fs::remove_dir_all(&dir);
}
