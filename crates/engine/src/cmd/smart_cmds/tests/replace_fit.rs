//! Replace Contents with `fit: "stretch"`: a file of another size fills the old contents' box,
//! through the same transform, warp or perspective, as a file of the old size would; with
//! `fit: "contain"` it is scaled to fit inside that box, keeping its aspect, and centred.

use super::*;
use photocraft_geom::warp::{BezierMesh, Warp};

/// A solid PNG of `w × h` written to `dir/name`.
fn png_file(dir: &std::path::Path, name: &str, w: u32, h: u32, rgb: [f32; 3]) -> String {
    let doc = Document::with_background(
        "p",
        Size::new(w, h),
        photocraft_color::ColorMode::Rgb,
        photocraft_color::SampleType::U8,
        photocraft_color::Color::rgba(rgb[0], rgb[1], rgb[2], 1.0),
    );
    let path = dir.join(name);
    std::fs::write(&path, photocraft_io::export(&doc, "png", &Default::default()).unwrap().bytes).unwrap();
    path.to_str().unwrap().to_string()
}

/// A scratch directory for test `tag`, created if missing.
fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pcraft-fit-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A converted smart object (painted block, contents 46 × 33) and its layer id.
fn smart_session() -> (Session, LayerId) {
    let mut s = session(8);
    paint(&mut s);
    let id = LayerId(convert(&mut s));
    (s, id)
}

/// The old contents' box in source pixels.
fn old_box(s: &Session, id: LayerId) -> [f64; 4] {
    let doc = &s.active().unwrap().doc;
    source_box(doc, smart(doc, id).unwrap()).unwrap()
}

/// Sets the placement of `id` with `f` and re-renders it.
fn set_placement(s: &mut Session, id: LayerId, f: impl FnOnce(&mut SmartObject)) {
    s.edit("setup", |doc, _| {
        f(smart_mut(doc, id)?);
        refresh_or_fail(doc, id)
    })
    .unwrap();
}

/// The rendered pixels (cache) of smart object `id`.
fn rendered(s: &Session, id: LayerId) -> Surface {
    let doc = &s.active().unwrap().doc;
    smart(doc, id).unwrap().cache.clone().unwrap()
}

/// Runs `layer.smartObjects.replaceContents` on `id` from `path`, with `fit` when given.
fn replace(s: &mut Session, id: LayerId, path: &str, fit: Option<&str>) {
    let mut p = json!({"layer": id.0, "path": path});
    if let Some(f) = fit {
        p["fit"] = json!(f);
    }
    s.execute("layer.smartObjects.replaceContents", p).unwrap();
}

/// Mean |alpha difference| over the union of both renders' bounds.
fn alpha_diff(a: &Surface, b: &Surface) -> f32 {
    let r = a.content_bounds().union(&b.content_bounds());
    let (mut sum, mut n) = (0.0f32, 0.0f32);
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            sum += (a.rgba(x, y)[3] - b.rgba(x, y)[3]).abs();
            n += 1.0;
        }
    }
    sum / n.max(1.0)
}

/// Whether each edge of `a` is within `tol` pixels of `b`'s.
fn near(a: Rect, b: Rect, tol: i32) -> bool {
    (a.x0 - b.x0).abs() <= tol && (a.y0 - b.y0).abs() <= tol && (a.x1 - b.x1).abs() <= tol && (a.y1 - b.y1).abs() <= tol
}

/// Renders the same placement twice: once from a full-size file (`fit` keep), once from a
/// smaller file with `fit: "stretch"`. Both must cover the same pixels.
fn full_vs_stretched(tag: &str, setup: impl Fn(&mut SmartObject, [f64; 4])) {
    full_vs_stretched_from(tag, |w, h| (w.div_ceil(2), h.div_ceil(2)), setup);
}

/// [`full_vs_stretched`] from a file of `size(w, h)` for old contents of `w × h`.
fn full_vs_stretched_from(tag: &str, size: impl Fn(u32, u32) -> (u32, u32), setup: impl Fn(&mut SmartObject, [f64; 4])) {
    let dir = tmp_dir(tag);
    let (mut full, id) = smart_session();
    let b = old_box(&full, id);
    let (w, h) = ((b[2] - b[0]) as u32, (b[3] - b[1]) as u32);
    set_placement(&mut full, id, |sm| setup(sm, b));
    replace(&mut full, id, &png_file(&dir, "full.png", w, h, [1.0, 0.0, 0.0]), None);
    let (mut small, id2) = smart_session();
    set_placement(&mut small, id2, |sm| setup(sm, b));
    let (sw, sh) = size(w, h);
    replace(&mut small, id2, &png_file(&dir, "small.png", sw, sh, [1.0, 0.0, 0.0]), Some("stretch"));
    let (a, c) = (rendered(&full, id), rendered(&small, id2));
    assert!(near(a.content_bounds(), c.content_bounds(), 1), "{tag}: {:?} vs {:?}", a.content_bounds(), c.content_bounds());
    assert!(alpha_diff(&a, &c) < 0.03, "{tag}: alpha diff {}", alpha_diff(&a, &c));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn stretch_fills_the_old_box_through_an_affine_transform() {
    full_vs_stretched("affine", |sm, _| sm.transform = Affine { m: [1.5, 0.2, -0.1, 1.2, 3.0, 2.0] });
}

#[test]
fn stretch_fills_the_old_box_through_a_custom_warp() {
    full_vs_stretched("warp", |sm, b| {
        // A bulge: the middle control points pushed out.
        let mesh = BezierMesh::identity(b, 1, 1).map_points(|p| {
            let (cx, cy) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
            [p[0] + 0.15 * (p[0] - cx), p[1] + 0.25 * (p[1] - cy) * ((p[0] - cx).abs() / (b[2] - b[0]))]
        });
        sm.warp = Some(Warp::custom(mesh, b));
    });
}

#[test]
fn stretch_fills_the_old_box_through_a_warp_over_a_wider_box() {
    // The warp's box need not be the contents' box (it may spill past it).
    full_vs_stretched("warp-wide", |sm, b| {
        let wide = [b[0] - 4.0, b[1] - 2.0, b[2] + 6.0, b[3] + 3.0];
        let mesh = BezierMesh::identity(wide, 2, 1).map_points(|p| [p[0] * 0.9 + 0.05 * p[1], p[1]]);
        sm.warp = Some(Warp::custom(mesh, wide));
    });
}

#[test]
fn stretch_fills_the_old_box_through_a_preset_warp() {
    full_vs_stretched("preset", |sm, b| sm.warp = Some(Warp::preset(photocraft_geom::warp::WarpStyle::Arc, 30.0, b)));
}

#[test]
fn stretch_fills_the_old_box_through_a_perspective() {
    full_vs_stretched("perspective", |sm, _| sm.perspective = Some([1.0, 0.1, -6.0, 0.05, 1.1, 5.0, 0.002, 0.001, 1.0]));
}

#[test]
fn stretch_fills_the_old_box_through_a_warp_under_a_perspective() {
    // The warp acts in source space, before the perspective, so it alone absorbs the stretch.
    // A file of another aspect through a bulge: stretching the perspective instead would show
    // only the left half of the bulge, widened.
    full_vs_stretched_from(
        "warp-perspective",
        |w, h| (w.div_ceil(2), h),
        |sm, b| {
            let mesh = BezierMesh::identity(b, 1, 1).map_points(|p| {
                let (cx, cy) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
                [p[0] + 0.15 * (p[0] - cx), p[1] + 0.25 * (p[1] - cy) * ((p[0] - cx).abs() / (b[2] - b[0]))]
            });
            sm.warp = Some(Warp::custom(mesh, b));
            sm.perspective = Some([1.0, 0.1, -6.0, 0.05, 1.1, 5.0, 0.002, 0.001, 1.0]);
        },
    );
}

/// Under a perspective, `transform` stays the perspective's affine at the source origin, as after
/// any other placement edit: an SVG is rasterised at that scale, so a stale one would come out
/// blurred or oversampled.
#[test]
fn stretch_under_a_perspective_keeps_the_transform_in_step() {
    let dir = tmp_dir("perspective-transform");
    let (mut s, id) = smart_session();
    let h = Homography([1.0, 0.1, -6.0, 0.05, 1.1, 5.0, 0.002, 0.001, 1.0]);
    set_placement(&mut s, id, |sm| crate::smart_cmds::set_placement(sm, h));
    replace(&mut s, id, &png_file(&dir, "small.png", 5, 7, [0.0, 1.0, 0.0]), Some("stretch"));
    let sm = active_smart(&s);
    let mut expected = sm.clone();
    crate::smart_cmds::set_placement(&mut expected, Homography(sm.perspective.expect("still a perspective")));
    assert!(sm.transform.m.iter().zip(expected.transform.m).all(|(a, b)| (a - b).abs() < 1e-9), "{:?} vs {:?}", sm.transform, expected.transform);
    std::fs::remove_dir_all(&dir).ok();
}

/// A session gate that refuses every path Replace Contents asks to read.
fn deny_replace_paths(cmd: &str, p: &Value) -> Result<()> {
    if cmd == "layer.smartObjects.replaceContents" && p.get("path").is_some() {
        return Err(other(format!("source path refused: {}", p["path"])));
    }
    Ok(())
}

/// Without a stored size, stretch and contain read the old contents' box from the old source.
/// For a linked file that reads a path the document holds, so an untrusted session's gate sees
/// it first, and a refusal leaves the document as it was.
#[test]
fn a_fit_from_a_linked_source_asks_the_session_gate_first() {
    let dir = tmp_dir("linked");
    let (mut s, id) = smart_session();
    let old = png_file(&dir, "old.png", 20, 10, [0.0, 0.0, 1.0]);
    set_placement(&mut s, id, |sm| sm.source = SmartSource::Linked { path: old.clone() });
    let new = std::fs::read(png_file(&dir, "new.png", 10, 5, [1.0, 0.0, 0.0])).unwrap();
    s.authorize = Some(deny_replace_paths);
    let before = s.active().unwrap().doc.clone();
    for fit in [Fit::Stretch, Fit::Contain] {
        let e = replace_contents_fit(&mut s, id, "new.png", new.clone(), fit).unwrap_err().to_string();
        assert!(e.contains("source path refused") && e.contains("old.png"), "{fit:?}: {e}");
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc), "{fit:?}");
    }
    // Keep needs no old box; without a gate (a trusted session) the linked file is read.
    replace_contents_fit(&mut s, id, "new.png", new.clone(), Fit::Keep).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    s.authorize = None;
    replace_contents_fit(&mut s, id, "new.png", new, Fit::Stretch).unwrap();
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn keep_is_the_default_and_keeps_the_pixel_size() {
    let dir = tmp_dir("keep");
    let (mut s, id) = smart_session();
    let before = rendered(&s, id).content_bounds();
    replace(&mut s, id, &png_file(&dir, "small.png", 4, 4, [0.0, 0.0, 1.0]), None);
    let keep = rendered(&s, id).content_bounds();
    assert_eq!(keep, Rect::new(before.x0, before.y0, before.x0 + 4, before.y0 + 4));
    s.execute("edit.undo", json!({})).unwrap();
    replace(&mut s, id, &png_file(&dir, "small2.png", 4, 4, [0.0, 0.0, 1.0]), Some("keep"));
    assert_eq!(rendered(&s, id).content_bounds(), keep);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn stretch_is_one_undo_step() {
    let dir = tmp_dir("undo");
    let (mut s, id) = smart_session();
    let before = active_smart(&s);
    replace(&mut s, id, &png_file(&dir, "small.png", 5, 7, [0.0, 1.0, 0.0]), Some("stretch"));
    assert_ne!(active_smart(&s).transform, before.transform, "the placement was re-aimed");
    s.execute("edit.undo", json!({})).unwrap();
    let after = active_smart(&s);
    assert_eq!(after.transform, before.transform);
    assert_eq!(after.cache.unwrap().content_bounds(), before.cache.unwrap().content_bounds());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn stretch_with_a_same_size_file_changes_no_placement() {
    let dir = tmp_dir("same");
    let (mut s, id) = smart_session();
    let b = old_box(&s, id);
    let before = active_smart(&s);
    replace(&mut s, id, &png_file(&dir, "same.png", (b[2] - b[0]) as u32, (b[3] - b[1]) as u32, [0.0, 1.0, 0.0]), Some("stretch"));
    assert_eq!(active_smart(&s).transform, before.transform);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn contain_fits_the_file_inside_the_old_box_keeping_its_aspect() {
    // A square file into the 46 × 33 box: scaled to 33 × 33 and centred across the width, the
    // rest of the box left empty.
    let dir = tmp_dir("contain");
    let (mut s, id) = smart_session();
    let before = rendered(&s, id).content_bounds();
    let b = old_box(&s, id);
    let (bw, bh) = (b[2] - b[0], b[3] - b[1]);
    replace(&mut s, id, &png_file(&dir, "square.png", 10, 10, [0.0, 1.0, 0.0]), Some("contain"));
    let k = (bw / 10.0).min(bh / 10.0);
    let (px, py) = (((bw - 10.0 * k) / 2.0).round() as i32, ((bh - 10.0 * k) / 2.0).round() as i32);
    let want = Rect::new(before.x0 + px, before.y0 + py, before.x1 - px, before.y1 - py);
    let got = rendered(&s, id).content_bounds();
    assert!(near(got, want, 1), "{got:?} vs {want:?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn contain_with_the_boxs_own_aspect_places_the_file_like_stretch() {
    let dir = tmp_dir("contain-aspect");
    let (mut stretched, id) = smart_session();
    let b = old_box(&stretched, id);
    let (w, h) = (2 * (b[2] - b[0]) as u32, 2 * (b[3] - b[1]) as u32);
    replace(&mut stretched, id, &png_file(&dir, "a.png", w, h, [1.0, 0.0, 0.0]), Some("stretch"));
    let (mut contained, id2) = smart_session();
    replace(&mut contained, id2, &png_file(&dir, "b.png", w, h, [1.0, 0.0, 0.0]), Some("contain"));
    assert_eq!(active_smart(&contained).transform, active_smart(&stretched).transform);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn contain_is_one_undo_step() {
    let dir = tmp_dir("contain-undo");
    let (mut s, id) = smart_session();
    let before = active_smart(&s);
    replace(&mut s, id, &png_file(&dir, "small.png", 5, 7, [0.0, 1.0, 0.0]), Some("contain"));
    assert_ne!(active_smart(&s).transform, before.transform, "the placement was re-aimed");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(active_smart(&s).transform, before.transform);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_unknown_fit_is_refused_before_anything_changes() {
    let dir = tmp_dir("bad");
    let (mut s, id) = smart_session();
    let before = active_smart(&s);
    let p = json!({"layer": id.0, "path": png_file(&dir, "x.png", 3, 3, [1.0, 1.0, 1.0]), "fit": "cover"});
    let e = s.execute("layer.smartObjects.replaceContents", p).unwrap_err().to_string();
    assert!(e.contains("fit"), "{e}");
    assert_eq!(active_smart(&s).source, before.source);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn stretch_reads_a_psd_layers_box_from_its_stored_size_without_decoding_the_old_contents() {
    // A PSD placed layer whose old contents can't be read (a missing link): its soLD still
    // stores the 10 × 8 size the placement (2×, at 20, 10) was authored for.
    let dir = tmp_dir("sold");
    let mut s = session(8);
    let spec = photocraft_io::smart_map::PlacedSpec {
        idnt: "uuid-gone",
        placed: "uuid-gone",
        transform: Affine { m: [2.0, 0.0, 0.0, 2.0, 20.0, 10.0] },
        perspective: None,
        size: (10.0, 8.0),
        dpi: 72.0,
        warp: None,
        filter_fx: None,
    };
    let sold = photocraft_io::smart_map::sold_bytes(None, &spec, &mut Vec::new());
    let id = s
        .edit("place", |doc, active| {
            let mut sm = SmartObject::new(SmartSource::Linked { path: "uuid-gone".into() }, spec.transform, None);
            sm.psd_raw = Some(Arc::new(sold.clone()));
            let l = Layer::new("placed", LayerContent::Smart(sm));
            let id = doc.insert_above(*active, l);
            *active = Some(id);
            Ok(id)
        })
        .unwrap();
    replace(&mut s, id, &png_file(&dir, "small.png", 5, 4, [1.0, 0.0, 0.0]), Some("stretch"));
    assert_eq!(rendered(&s, id).content_bounds(), Rect::new(20, 10, 40, 26));
    std::fs::remove_dir_all(&dir).ok();
}

/// The digest is the buffer's blake3, remembered for the buffer, never served to a new one.
#[test]
fn contents_digest_is_blake3_and_never_stale_for_a_new_buffer() {
    let digest = super::super::rendering::contents_digest;
    let a = Arc::new(vec![1u8; 4096]);
    assert_eq!(digest(&a), *blake3::hash(&a).as_bytes());
    assert_eq!(digest(&a), *blake3::hash(&a).as_bytes(), "remembered");
    drop(a);
    // A new buffer (possibly at the freed address) must be hashed, not served the old digest.
    let b = Arc::new(vec![2u8; 4096]);
    assert_eq!(digest(&b), *blake3::hash(&b).as_bytes());
}

/// Three instances of one smart object share the new file: it is hashed once for all of them.
#[test]
fn replacing_shared_contents_hashes_the_new_file_once_for_all_instances() {
    let hashes = || super::super::rendering::CONTENTS_HASHES.with(std::cell::Cell::get);
    let dir = tmp_dir("hash-once");
    for fit in [None, Some("stretch")] {
        let (mut s, id) = smart_session();
        for _ in 0..2 {
            s.execute("layer.duplicate", json!({"layer": id.0})).unwrap();
        }
        let before = hashes();
        replace(&mut s, id, &png_file(&dir, "big.png", 300, 200, [0.5, 0.2, 0.9]), fit);
        let hashed = hashes() - before;
        assert_eq!(hashed, 1, "fit {fit:?}: hashed the new file {hashed} times");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// The file is decoded once to check it (and find its box); the re-render reuses that image.
#[test]
fn replacing_contents_decodes_the_new_file_once() {
    let dir = tmp_dir("decode-once");
    let (mut s, id) = smart_session();
    s.execute("layer.duplicate", json!({"layer": id.0})).unwrap();
    let path = png_file(&dir, "d.png", 121, 79, [0.3, 0.6, 0.9]);
    let decodes = || DECODES.with(std::cell::Cell::get);
    let before = decodes();
    // Keep: the old contents are not read.
    replace(&mut s, id, &path, None);
    assert_eq!(decodes() - before, 1, "decoded the new file {} times", decodes() - before);
    std::fs::remove_dir_all(&dir).ok();
}
