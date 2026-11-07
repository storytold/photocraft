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

/// A PNG of `w × h` in four coloured quadrants (red, green / blue, white), written to `dir/name`:
/// artwork that shows if a placement flips or shifts it.
fn quadrants_file(dir: &std::path::Path, name: &str, w: u32, h: u32) -> String {
    let mut doc = Document::new("q", Size::new(w, h), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut l = Layer::raster("q", doc.pixel_format());
    let px: Vec<f32> = (0..h)
        .flat_map(|y| {
            (0..w).flat_map(move |x| match (2 * x < w, 2 * y < h) {
                (true, true) => [1.0, 0.0, 0.0, 1.0],
                (false, true) => [0.0, 1.0, 0.0, 1.0],
                (true, false) => [0.0, 0.0, 1.0, 1.0],
                (false, false) => [1.0, 1.0, 1.0, 1.0],
            })
        })
        .collect();
    l.surface_mut().unwrap().write_region(Rect::new(0, 0, w as i32, h as i32), &px);
    doc.layers = vec![l];
    let path = dir.join(name);
    std::fs::write(&path, photocraft_io::export(&doc, "png", &Default::default()).unwrap().bytes).unwrap();
    path.to_str().unwrap().to_string()
}

/// The share of pixels opaque in both renders whose colours match (within 0.25 per channel).
fn colours_agree(a: &Surface, b: &Surface) -> f32 {
    let r = a.content_bounds().union(&b.content_bounds());
    let (pa, pb) = (a.read_region(r), b.read_region(r));
    let n = a.format().channels();
    let (mut both, mut same) = (0usize, 0usize);
    for (p, q) in pa.chunks_exact(n).zip(pb.chunks_exact(n)) {
        if p[n - 1] > 0.99 && q[n - 1] > 0.99 {
            both += 1;
            same += usize::from(p[..n - 1].iter().zip(&q[..n - 1]).all(|(x, y)| (x - y).abs() < 0.25));
        }
    }
    same as f32 / both.max(1) as f32
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
/// smaller file with `fit: "stretch"`. Both must cover the same pixels, the same way round.
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
    replace(&mut full, id, &quadrants_file(&dir, "full.png", w, h), None);
    let (mut small, id2) = smart_session();
    set_placement(&mut small, id2, |sm| setup(sm, b));
    let (sw, sh) = size(w, h);
    replace(&mut small, id2, &quadrants_file(&dir, "small.png", sw, sh), Some("stretch"));
    let (a, c) = (rendered(&full, id), rendered(&small, id2));
    assert!(near(a.content_bounds(), c.content_bounds(), 1), "{tag}: {:?} vs {:?}", a.content_bounds(), c.content_bounds());
    assert!(alpha_diff(&a, &c) < 0.03, "{tag}: alpha diff {}", alpha_diff(&a, &c));
    // The artwork lands the same way round: only the quadrant edges, resampled, may differ.
    let agree = colours_agree(&a, &c);
    assert!(agree > 0.7, "{tag}: colours agree on {agree}");
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

/// A preset warp over a box narrower than the contents extrapolates past it. Stretching keeps
/// that: the custom mesh it becomes covers the old contents, so new contents map, everywhere, to
/// where the old warp took the matching old pixels (a mesh over the preset's own box would clamp).
#[test]
fn stretch_keeps_a_preset_warps_extrapolation_past_its_box() {
    use photocraft_geom::warp::WarpStyle;
    let (old, new) = ([0.0, 0.0, 100.0, 60.0], [0.0, 0.0, 50.0, 40.0]);
    let preset = Warp::preset(WarpStyle::Arc, 40.0, [20.0, 10.0, 80.0, 50.0]);
    let mut sm = SmartObject::new(SmartSource::Embedded { file_name: "x.png".into(), bytes: Arc::new(Vec::new()) }, Affine::IDENTITY, None);
    sm.warp = Some(preset.clone());
    super::super::placing::stretch_placement(&mut sm, old, new);
    let w = sm.warp.unwrap();
    let (kx, ky) = (2.0, 1.5);
    for (x, y) in [(0.0, 0.0), (49.0, 39.0), (2.0, 30.0), (25.0, 20.0), (48.0, 3.0)] {
        let (a, b) = w.map(x, y);
        let (c, d) = preset.map(kx * x, ky * y);
        assert!((a - c).abs() < 1e-6 && (b - d).abs() < 1e-6, "({x}, {y}): {:?} vs {:?}", (a, b), (c, d));
    }
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

#[test]
fn clip_to_canvas_renders_only_the_canvas_and_the_same_pixels_inside_it() {
    let dir = tmp_dir("clip");
    let design = png_file(&dir, "d.png", 30, 20, [0.2, 0.7, 0.4]);
    // Placed 3× and bent, so the warped contents spill far past the 64 × 48 canvas.
    let setup = |sm: &mut SmartObject, b: [f64; 4]| {
        sm.transform = Affine { m: [3.0, 0.0, 0.0, 3.0, -20.0, -10.0] };
        let mesh = BezierMesh::identity(b, 1, 1).map_points(|p| [p[0] + 0.1 * p[1], p[1] + 3.0 * (p[0] / (b[2] - b[0]) * 3.0).sin()]);
        sm.warp = Some(Warp::custom(mesh, b));
    };
    let (mut full, id) = smart_session();
    let b = old_box(&full, id);
    set_placement(&mut full, id, |sm| setup(sm, b));
    let whole = replace_contents_with(&mut full, id, "d.png", std::fs::read(&design).unwrap(), Fit::Stretch, false).unwrap();
    assert_eq!(whole["warnings"], json!([]));
    let (mut clipped, id2) = smart_session();
    set_placement(&mut clipped, id2, |sm| setup(sm, b));
    let cut = replace_contents_with(&mut clipped, id2, "d.png", std::fs::read(&design).unwrap(), Fit::Stretch, true).unwrap();
    // The caller is told the layer holds only what shows, since a saved document would keep that.
    let name = clipped.active().unwrap().doc.layer(id2).unwrap().name.clone();
    let warnings = cut["warnings"].as_array().unwrap();
    assert!(
        warnings.len() == 1 && warnings[0].as_str().is_some_and(|w| w.contains(&format!("\"{name}\" (layer {})", id2.0)) && w.contains("only where it shows")),
        "{warnings:?}"
    );
    let canvas = clipped.active().unwrap().doc.bounds();
    let (a, c) = (rendered(&full, id), rendered(&clipped, id2));
    assert!(!canvas.contains_rect(&a.content_bounds()), "the full render spills: {:?}", a.content_bounds());
    assert!(canvas.contains_rect(&c.content_bounds()), "clipped: {:?}", c.content_bounds());
    let inside = a.content_bounds().intersect(&canvas);
    assert_eq!(c.read_region(inside), a.read_region(inside));
    // The flattened documents are the same.
    assert_eq!(flat(&full), flat(&clipped));
    // A later unclipped render of the same file is whole again (clipped renders are cached apart).
    clipped.execute("edit.undo", json!({})).unwrap();
    replace_contents_with(&mut clipped, id2, "d.png", std::fs::read(&design).unwrap(), Fit::Stretch, false).unwrap();
    assert_eq!(rendered(&clipped, id2).content_bounds(), a.content_bounds());
    std::fs::remove_dir_all(&dir).ok();
}

/// Full and clipped renders of a placement spilling past the canvas, with `extra` applied to the
/// layer first; returns both flattened documents.
fn full_and_clipped_flat(tag: &str, extra: impl Fn(&mut Session, LayerId)) -> (Vec<[f32; 4]>, Vec<[f32; 4]>) {
    let dir = tmp_dir(tag);
    let design = png_file(&dir, "d.png", 30, 20, [0.2, 0.7, 0.4]);
    let setup = |sm: &mut SmartObject, b: [f64; 4]| {
        sm.transform = Affine { m: [3.0, 0.0, 0.0, 3.0, -20.0, -10.0] };
        let mesh = BezierMesh::identity(b, 1, 1).map_points(|p| [p[0] + 0.1 * p[1], p[1]]);
        sm.warp = Some(Warp::custom(mesh, b));
    };
    let mut out = Vec::new();
    for clip in [false, true] {
        let (mut s, id) = smart_session();
        let b = old_box(&s, id);
        set_placement(&mut s, id, |sm| setup(sm, b));
        extra(&mut s, id);
        replace_contents_with(&mut s, id, "d.png", std::fs::read(&design).unwrap(), Fit::Stretch, clip).unwrap();
        out.push(flat(&s));
    }
    std::fs::remove_dir_all(&dir).ok();
    let clipped = out.pop().unwrap();
    (out.pop().unwrap(), clipped)
}

#[test]
fn clip_to_canvas_keeps_the_flattened_result_of_a_layer_with_effects() {
    // A stroke follows the layer's edge: cutting the layer at the canvas would draw it there.
    let (full, clipped) = full_and_clipped_flat("fx", |s, id| {
        s.execute("layer.layerStyle.stroke", json!({"layer": id.0, "size": 3, "position": "inside", "color": "#ff0000"})).unwrap();
    });
    assert_eq!(full, clipped);
}

#[test]
fn clip_to_canvas_keeps_the_flattened_result_inside_a_group_with_effects() {
    // A group's inside stroke follows the edge of what its children cover: a child cut at the
    // canvas would draw it there.
    let (full, clipped) = full_and_clipped_flat("group-fx", |s, id| {
        s.execute("layer.groupLayers", json!({})).unwrap();
        let group = s.active().unwrap().active_layer.unwrap();
        assert_eq!(s.active().unwrap().doc.path_of(id).map(|p| p.len()), Some(2), "the smart object is in the group");
        s.execute("layer.layerStyle.stroke", json!({"layer": group.0, "size": 3, "position": "inside", "color": "#ff0000"})).unwrap();
    });
    assert_eq!(full, clipped);
}

#[test]
fn clip_to_canvas_keeps_the_flattened_result_of_a_layer_with_smart_filters() {
    // A blur reads past the canvas edge; a clipped layer would repeat the edge instead.
    let (full, clipped) = full_and_clipped_flat("sf", |s, id| {
        s.execute("filter.blur.gaussianBlur", json!({"layer": id.0, "radius": 4})).unwrap();
    });
    assert_eq!(full, clipped);
}

#[test]
fn clip_to_canvas_renders_only_what_the_layer_mask_reveals() {
    // A mask that hides everything but a 10 × 6 strip: the clipped render stays inside the strip
    // (plus the mask's margin) and the flattened document is the same as a full render.
    let mask_strip = |s: &mut Session, id: LayerId| {
        s.edit("mask", |doc, _| {
            let mut m = photocraft_doc::LayerMask::hide_all();
            m.surface.fill_rect(Rect::new(20, 12, 30, 18), &[1.0]);
            doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.mask = Some(m);
            Ok(())
        })
        .unwrap();
    };
    let (full, clipped) = full_and_clipped_flat("mask", mask_strip);
    assert_eq!(full, clipped);

    let dir = tmp_dir("mask-bounds");
    let (mut s, id) = smart_session();
    let b = old_box(&s, id);
    set_placement(&mut s, id, |sm| {
        sm.transform = Affine { m: [3.0, 0.0, 0.0, 3.0, -20.0, -10.0] };
        sm.warp = Some(Warp::custom(BezierMesh::identity(b, 1, 1).map_points(|p| [p[0] + 0.1 * p[1], p[1]]), b));
    });
    mask_strip(&mut s, id);
    replace_contents_with(&mut s, id, "d.png", std::fs::read(png_file(&dir, "d.png", 30, 20, [0.2, 0.7, 0.4])).unwrap(), Fit::Stretch, true).unwrap();
    let r = rendered(&s, id).content_bounds();
    assert!(Rect::new(20, 12, 30, 18).inflate(2).contains_rect(&r), "rendered {r:?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn clip_to_canvas_warns_once_per_instance_even_when_they_share_a_name() {
    let dir = tmp_dir("clip-names");
    let design = std::fs::read(png_file(&dir, "d.png", 30, 20, [0.2, 0.7, 0.4])).unwrap();
    let (mut s, id) = smart_session();
    let b = old_box(&s, id);
    set_placement(&mut s, id, |sm| {
        sm.transform = Affine { m: [3.0, 0.0, 0.0, 3.0, -20.0, -10.0] };
        sm.warp = Some(Warp::custom(BezierMesh::identity(b, 1, 1).map_points(|p| [p[0] + 0.1 * p[1], p[1]]), b));
    });
    s.execute("layer.duplicate", json!({"layer": id.0})).unwrap();
    let both = s
        .edit("rename", |doc, _| {
            let ids: Vec<LayerId> = doc.walk().into_iter().filter(|(_, _, l)| matches!(l.content, LayerContent::Smart(_))).map(|(_, _, l)| l.id).collect();
            for &l in &ids {
                doc.layer_mut(l).ok_or(EngineError::NoLayer(l))?.name = "Design".into();
            }
            Ok(ids)
        })
        .unwrap();
    assert_eq!(both.len(), 2);
    let r = replace_contents_with(&mut s, id, "d.png", design, Fit::Stretch, true).unwrap();
    let warned: Vec<&str> = r["warnings"].as_array().unwrap().iter().filter_map(|w| w.as_str()).collect();
    assert_eq!(warned.len(), 2, "{warned:?}");
    for l in both {
        assert!(warned.iter().any(|w| w.starts_with(&format!("\"Design\" (layer {})", l.0))), "{l:?}: {warned:?}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

fn hide(s: &mut Session, id: LayerId) {
    s.edit("hide", |doc, _| {
        doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.visible = false;
        Ok(())
    })
    .unwrap();
}

/// Moves `id` into a new hidden group at the top of the document.
fn hide_in_group(s: &mut Session, id: LayerId) {
    s.edit("group", |doc, _| {
        let path = doc.path_of(id).ok_or(EngineError::NoLayer(id))?;
        let [i] = path[..] else { return Err(other("expected a top-level layer")) };
        let layer = doc.layers.remove(i);
        let mut g = Layer::group("G", vec![layer]);
        g.visible = false;
        doc.layers.push(g);
        Ok(())
    })
    .unwrap();
}

#[test]
fn clip_to_canvas_leaves_hidden_instances_unrendered() {
    // Mockups keep a hidden "design here" layer whose copies show the design. The hidden one is
    // not in the flattened document: a clipped replace leaves it unrendered, and the flattened
    // result is the same as a full render.
    let (full, clipped) = full_and_clipped_flat("hidden", |s, id| {
        s.execute("layer.duplicate", json!({"layer": id.0})).unwrap();
        hide(s, id);
    });
    assert_eq!(full, clipped);

    let dir = tmp_dir("hidden-cache");
    let design = std::fs::read(png_file(&dir, "d.png", 30, 20, [0.2, 0.7, 0.4])).unwrap();
    for (clip, group) in [(true, false), (true, true), (false, false)] {
        let (mut s, id) = smart_session();
        s.execute("layer.duplicate", json!({"layer": id.0})).unwrap();
        if group {
            hide_in_group(&mut s, id)
        } else {
            hide(&mut s, id)
        }
        let r = replace_contents_with(&mut s, id, "d.png", design.clone(), Fit::Stretch, clip).unwrap();
        let doc = &s.active().unwrap().doc;
        // Told about the hidden instance only; the copy is unwarped, so the clip leaves it whole.
        let warned: Vec<&str> = r["warnings"].as_array().unwrap().iter().filter_map(|w| w.as_str()).collect();
        let hidden = format!("\"{}\" (layer {}) is hidden", doc.layer(id).unwrap().name, id.0);
        assert_eq!(warned.len(), usize::from(clip), "clip {clip}, group {group}: {warned:?}");
        assert!(warned.iter().all(|w| w.starts_with(&hidden)), "clip {clip}, group {group}: {warned:?}");
        let copy = doc.walk().into_iter().find(|(_, _, l)| l.id != id && matches!(l.content, LayerContent::Smart(_))).map(|(_, _, l)| l.id).unwrap();
        assert_eq!(smart(doc, id).unwrap().cache.is_none(), clip, "clip {clip}, group {group}: the hidden instance is left unrendered only with clip");
        assert!(
            smart(doc, copy).unwrap().cache.as_ref().is_some_and(|c| !c.content_bounds().is_empty()),
            "clip {clip}, group {group}: the visible copy is rendered"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A warped placement whose whole render is past Memory Usage is refused, but with the clip only
/// the canvas is rendered, so it goes through (the test budget is 20 MB).
#[test]
fn clip_to_canvas_budgets_the_clipped_render_not_the_whole_placement() {
    let dir = tmp_dir("clip-budget");
    let design = std::fs::read(png_file(&dir, "d.png", 30, 20, [0.2, 0.7, 0.4])).unwrap();
    let huge = |sm: &mut SmartObject, b: [f64; 4]| {
        // The 30 × 20 file 120 times over: 3600 × 2400 px of 8-bit RGBA, 35 MB.
        sm.transform = Affine { m: [120.0, 0.0, 0.0, 120.0, -1500.0, -1000.0] };
        sm.warp = Some(Warp::custom(BezierMesh::identity(b, 1, 1).map_points(|p| [p[0] + 0.1 * p[1], p[1]]), b));
    };
    for clip in [false, true] {
        let (mut s, id) = smart_session();
        let b = old_box(&s, id);
        s.edit("setup", |doc, _| {
            huge(smart_mut(doc, id)?, b);
            Ok(())
        })
        .unwrap();
        let r = replace_contents_with(&mut s, id, "d.png", design.clone(), Fit::Stretch, clip);
        assert_eq!(r.is_ok(), clip, "clip {clip}: {r:?}");
    }
    std::fs::remove_dir_all(&dir).ok();
}
