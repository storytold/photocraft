//! A `SoLd` whose `Sz`, warp `bounds` and mesh live in the descriptor's own units rather than
//! the embedded file's pixels (#10207): the source scales to them before warp and transform
//! apply, so a re-render (Replace Contents, refresh) keeps the warp and the frame. Before the
//! fix the source pixels were fed to them as if they were the same space, dropping the warp's
//! shape and placing the contents `src / Sz` times too large.

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_io::blocks::{parse_placed_size, parse_placed_warp, parse_smart};
use photocraft_psd::descriptor::{Descriptor, ObjectArray, Value, VersionedDescriptor};
use photocraft_raster::Surface;

/// A `SoLd` block: `soLD`, version, the versioned descriptor of `size`, `corners` and `warp`.
fn sold(size: (f64, f64), corners: [f64; 8], warp: Descriptor) -> Vec<u8> {
    sold_persp(size, corners, None, warp)
}

/// [`sold`] with a `nonAffineTransform` quad (the projective placement a warped `SoLd` stores).
fn sold_persp(size: (f64, f64), corners: [f64; 8], persp: Option<[f64; 8]>, warp: Descriptor) -> Vec<u8> {
    let mut d = Descriptor::new("soLD")
        .with("Sz  ", Value::Descriptor(Descriptor::new("Pnt ").with("Wdth", Value::Double(size.0)).with("Hght", Value::Double(size.1))))
        .with("Trnf", Value::List(corners.iter().map(|v| Value::Double(*v)).collect()))
        .with("warp", Value::Descriptor(warp));
    if let Some(q) = persp {
        d = d.with("nonAffineTransform", Value::List(q.iter().map(|v| Value::Double(*v)).collect()));
    }
    let mut b = b"soLD".to_vec();
    b.extend_from_slice(&4u32.to_be_bytes());
    b.extend_from_slice(&VersionedDescriptor::new(d).to_bytes());
    b
}

/// A `warp` descriptor: `warpCustom` over `bounds` with a 16-point envelope mesh.
fn warp_desc(bounds: [f64; 4], points: Vec<[f64; 2]>) -> Descriptor {
    warp_desc_orders(bounds, points, 4.0, 4.0)
}

/// [`warp_desc`] with explicit `uOrder`/`vOrder`: Photoshop's custom envelope stores a
/// `4 × 2` grid (8 points) for a warp cubic in one direction and linear in the other.
fn warp_desc_orders(bounds: [f64; 4], points: Vec<[f64; 2]>, u_order: f64, v_order: f64) -> Descriptor {
    let [x0, y0, x1, y1] = bounds;
    let mesh = Descriptor::new("rationalPoint")
        .with("Hrzn", Value::UnitFloats { unit: *b"#Pxl", values: points.iter().map(|p| p[0]).collect() })
        .with("Vrtc", Value::UnitFloats { unit: *b"#Pxl", values: points.iter().map(|p| p[1]).collect() });
    Descriptor::new("warp")
        .with("uOrder", Value::Double(u_order))
        .with("vOrder", Value::Double(v_order))
        .with(
            "warpStyle",
            Value::Enumerated { type_id: photocraft_psd::descriptor::Id::new("warpStyle"), value: photocraft_psd::descriptor::Id::new("warpCustom") },
        )
        .with("warpValue", Value::Double(0.0))
        .with(
            "bounds",
            Value::Descriptor(
                Descriptor::new("classFloatRect")
                    .with("Top ", Value::Double(y0))
                    .with("Left", Value::Double(x0))
                    .with("Btom", Value::Double(y1))
                    .with("Rght", Value::Double(x1)),
            ),
        )
        .with(
            "customEnvelopeWarp",
            Value::Descriptor(
                Descriptor::new("customEnvelopeWarp").with("meshPoints", Value::ObjectArray(ObjectArray { prefix: points.len() as u32, body: mesh })),
            ),
        )
}

/// A `warp` descriptor saying `warpNone`, as Photoshop stores on an unwarped placement.
fn warp_none(bounds: [f64; 4]) -> Descriptor {
    let [x0, y0, x1, y1] = bounds;
    Descriptor::new("warp")
        .with(
            "warpStyle",
            Value::Enumerated { type_id: photocraft_psd::descriptor::Id::new("warpStyle"), value: photocraft_psd::descriptor::Id::new("warpNone") },
        )
        .with("warpValue", Value::Double(0.0))
        .with(
            "bounds",
            Value::Descriptor(
                Descriptor::new("classFloatRect")
                    .with("Top ", Value::Double(y0))
                    .with("Left", Value::Double(x0))
                    .with("Btom", Value::Double(y1))
                    .with("Rght", Value::Double(x1)),
            ),
        )
}

/// Writes a demo PNG next to `dir` only when `PHOTOCRAFT_PLACED_UNITS_OUT` names a directory —
/// the visual-review artifacts stay out of the default `cargo test` run.
fn keep(name: &str, bytes: &[u8]) {
    if let Ok(dir) = std::env::var("PHOTOCRAFT_PLACED_UNITS_OUT") {
        std::fs::write(std::path::Path::new(&dir).join(format!("photocraft-10207-{name}.png")), bytes).unwrap();
    }
}

fn red(side: i32) -> Surface {
    let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    s.fill_rect(Rect::new(0, 0, side, side), &[1.0, 0.0, 0.0, 1.0]);
    s
}

fn about(b: Rect, x0: i32, y0: i32, x1: i32, y1: i32) {
    assert!((b.x0 - x0).abs() <= 1 && (b.y0 - y0).abs() <= 1 && (b.x1 - x1).abs() <= 1 && (b.y1 - y1).abs() <= 1, "{b:?} vs ({x0}, {y0}, {x1}, {y1})");
}

/// Before/after PNGs of the rotated `SoLd` (#10253's shape) for visual review: the same
/// descriptor rendered with `placed_size = None` (the old code's behaviour) and `Some`.
/// Writes into `PHOTOCRAFT_PLACED_UNITS_OUT` when set.
#[test]
fn demo_rotated_sold_before_after_pngs() {
    let (c, s) = (30.0f64.to_radians().cos(), 30.0f64.to_radians().sin());
    let corners = [10.0, 10.0, 10.0 + 8.0 * c, 10.0 + 8.0 * s, 10.0 + 8.0 * (c - s), 10.0 + 8.0 * (s + c), 10.0 - 8.0 * s, 10.0 + 8.0 * c];
    let data = sold((8.0, 8.0), corners, warp_none([0.0, 0.0, 8.0, 8.0]));
    let (_, transform) = parse_smart(b"SoLd", &data);

    let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    for j in 0..4 {
        for i in 0..4 {
            let px = if (i + j) % 2 == 0 { [1.0, 0.15, 0.15, 1.0] } else { [1.0, 1.0, 1.0, 1.0] };
            src.fill_rect(Rect::new(i * 4, j * 4, i * 4 + 4, j * 4 + 4), &px);
        }
    }

    for (tag, placed) in [("before", None), ("after", Some((8.0, 8.0)))] {
        let out = photocraft_algo::warp::place_source(&src, Rect::new(0, 0, 16, 16), &transform, None, placed).unwrap();
        let mut canvas = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        canvas.fill_rect(Rect::new(0, 0, 64, 64), &[0.16, 0.16, 0.18, 1.0]);
        let b = out.tile_bounds();
        canvas.write_region(b, &out.read_region(b));
        // The `Trnf` quad the contents should fill, in yellow.
        for e in 0..4 {
            let (p, q) = ([corners[e * 2], corners[e * 2 + 1]], [corners[(e * 2 + 2) % 8], corners[(e * 2 + 3) % 8]]);
            for k in 0..=64 {
                let (x, y) = (p[0] + (q[0] - p[0]) * k as f64 / 64.0, p[1] + (q[1] - p[1]) * k as f64 / 64.0);
                canvas.fill_rect(Rect::new(x as i32, y as i32, x as i32 + 1, y as i32 + 1), &[1.0, 0.85, 0.0, 1.0]);
            }
        }
        let mut doc = photocraft_doc::Document::new(tag, photocraft_geom::Size::new(64, 64), ColorMode::Rgb, SampleType::U8);
        doc.layers.push(photocraft_doc::Layer::new(tag, photocraft_doc::LayerContent::Raster(canvas)));
        let png = photocraft_io::export(&doc, "png", &Default::default()).unwrap().bytes;
        keep(tag, &png);
    }
}

/// The warped "after" (#10207's shape): a fold mesh over an 8-unit box with a 16 px source;
/// the matching before-image is rendered by the same descriptor under the pre-fix code.
/// Writes into `PHOTOCRAFT_PLACED_UNITS_OUT` when set.
#[test]
fn demo_warped_sold_after_png() {
    // A pronounced curl: the bottom control row folds back over the top (y -6).
    let mut points = Vec::new();
    for j in 0..4 {
        for i in 0..4 {
            points.push([i as f64 * 8.0 / 3.0, if j == 3 { -6.0 } else { j as f64 * 8.0 / 3.0 }]);
        }
    }
    let corners = [10.0, 4.0, 18.0, 4.0, 18.0, 10.0, 10.0, 10.0];
    let data = sold((8.0, 8.0), corners, warp_desc([0.0, 0.0, 8.0, 8.0], points));
    let warp = parse_placed_warp(b"SoLd", &data).expect("a custom warp");
    let (_, transform) = parse_smart(b"SoLd", &data);

    let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    for j in 0..4 {
        for i in 0..4 {
            let px = if (i + j) % 2 == 0 { [1.0, 0.15, 0.15, 1.0] } else { [1.0, 1.0, 1.0, 1.0] };
            src.fill_rect(Rect::new(i * 4, j * 4, i * 4 + 4, j * 4 + 4), &px);
        }
    }
    let r = Rect::new(0, 0, 16, 16);

    for (tag, out) in [("warp-after", photocraft_algo::warp::place_source(&src, r, &transform, Some(&warp), Some((8.0, 8.0))).unwrap())] {
        let mut canvas = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        canvas.fill_rect(Rect::new(0, 0, 64, 64), &[0.16, 0.16, 0.18, 1.0]);
        let b = out.tile_bounds();
        canvas.write_region(b, &out.read_region(b));
        for e in 0..4 {
            let (p, q) = ([corners[e * 2], corners[e * 2 + 1]], [corners[(e * 2 + 2) % 8], corners[(e * 2 + 3) % 8]]);
            for k in 0..=64 {
                let (x, y) = (p[0] + (q[0] - p[0]) * k as f64 / 64.0, p[1] + (q[1] - p[1]) * k as f64 / 64.0);
                canvas.fill_rect(Rect::new(x as i32, y as i32, x as i32 + 1, y as i32 + 1), &[1.0, 0.85, 0.0, 1.0]);
            }
        }
        let mut doc = photocraft_doc::Document::new(tag, photocraft_geom::Size::new(64, 64), ColorMode::Rgb, SampleType::U8);
        doc.layers.push(photocraft_doc::Layer::new(tag, photocraft_doc::LayerContent::Raster(canvas)));
        let png = photocraft_io::export(&doc, "png", &Default::default()).unwrap().bytes;
        keep(tag, &png);
    }
}

/// An unwarped `SoLd` whose `Sz` is half the source's pixels, as Photoshop writes when the
/// placed resolution differs: the 16 px source renders into the 8 × 8 frame, not 16 × 16.
#[test]
fn a_sold_sz_smaller_than_the_source_scales_the_contents_down() {
    let corners = [10.0, 10.0, 18.0, 10.0, 18.0, 18.0, 10.0, 18.0];
    let data = sold((8.0, 8.0), corners, warp_none([0.0, 0.0, 8.0, 8.0]));
    assert_eq!(parse_placed_size(b"SoLd", &data), Some((8.0, 8.0)));
    assert!(parse_placed_warp(b"SoLd", &data).is_none());
    let (_, transform) = parse_smart(b"SoLd", &data);
    let src = red(16);
    let out = photocraft_algo::warp::place_source(&src, Rect::new(0, 0, 16, 16), &transform, None, Some((8.0, 8.0))).unwrap();
    about(out.content_bounds(), 10, 10, 18, 18);
}

/// The rotated placement of #10253: the same `Sz`-vs-pixels mismatch, with the `Trnf` quad
/// rotated 30° — the scaled contents land inside the rotated frame instead of scattered past
/// the clip shape.
#[test]
fn a_rotated_sold_scales_the_contents_into_the_rotated_frame() {
    let (c, s) = (30.0f64.to_radians().cos(), 30.0f64.to_radians().sin());
    let corners = [10.0, 10.0, 10.0 + 8.0 * c, 10.0 + 8.0 * s, 10.0 + 8.0 * (c - s), 10.0 + 8.0 * (s + c), 10.0 - 8.0 * s, 10.0 + 8.0 * c];
    let data = sold((8.0, 8.0), corners, warp_none([0.0, 0.0, 8.0, 8.0]));
    let (_, transform) = parse_smart(b"SoLd", &data);
    let src = red(16);
    let out = photocraft_algo::warp::place_source(&src, Rect::new(0, 0, 16, 16), &transform, None, Some((8.0, 8.0))).unwrap();
    // The rotated 8 × 8 frame's doc-space box (6,10)-(17,21), within resampling bleed; the
    // unscaled render the bug produced would span roughly (2,10)-(24,32).
    let b = out.content_bounds();
    assert!((b.x0 - 6).abs() <= 1 && (b.y0 - 10).abs() <= 1 && b.width() <= 15 && b.height() <= 16, "{b:?}");
}

/// Photoshop's actual custom envelope: a `warpCustom` `SoLd` stores `uOrder = 4, vOrder = 2`
/// with eight mesh points — cubic in `u`, linear in `v` — and the `nonAffineTransform` quad
/// frames the mesh's control-point box, which spills past `bounds`. Measured on a real file
/// (Photoshop 2026, arc bend 50): the mesh and corners below are its stored values; the layer
/// rendered as (−138, 3)–(584, 218). Degree-elevating `v` to cubic makes the surface a single
/// bicubic patch, and the box the projective frames is the mesh's, not `bounds`' (#10207).
#[test]
fn a_custom_envelope_4x2_meshes_and_frames_on_its_control_box() {
    // warped-grad.psd: bounds 0..800 × 0..600, Sz 800 × 600, `Trnf` and `nonAffineTransform`.
    let points = vec![
        [-424.264, 175.736],
        [30.964, -279.493],
        [769.036, -279.493],
        [1224.264, 175.736],
        [0.0, 600.0],
        [220.914, 379.086],
        [579.086, 379.086],
        [800.0, 600.0],
    ];
    let trnf = [-203.874, -85.411, 606.454, -85.411, 606.454, 251.423, -203.874, 251.423];
    let persp = [-203.874, 74.617, 606.454, -85.411, 567.009, 121.970, -88.497, 251.423];
    let data = sold_persp((800.0, 600.0), trnf, Some(persp), warp_desc_orders([0.0, 0.0, 800.0, 600.0], points.clone(), 4.0, 2.0));

    let warp = parse_placed_warp(b"SoLd", &data).expect("the 4 x 2 envelope is a warp");
    let mesh = warp.mesh.as_ref().expect("degree-elevated to a single patch");
    assert_eq!(mesh.points.len(), 16);
    // Column elevation puts the linear direction's control points at the thirds.
    assert!((mesh.point(0, 1)[0] - (2.0 * points[0][0] + points[4][0]) / 3.0).abs() < 1e-9);
    assert!((mesh.point(0, 2)[1] - (points[0][1] + 2.0 * points[4][1]) / 3.0).abs() < 1e-9);
    assert_eq!(mesh.point(3, 0), points[3]);
    assert_eq!(mesh.point(3, 3), points[7]);

    // The projective frames the mesh's control box (−424..1224 × −279..600), not `bounds`.
    let h = photocraft_io::blocks::parse_smart_perspective(b"SoLd", &data).expect("non-affine corners");
    let h = photocraft_algo::transform::Homography(h);
    let (dx, dy) = h.apply(-424.264, -279.493);
    assert!((dx - persp[0]).abs() < 0.5 && (dy - persp[1]).abs() < 0.5, "the control box's corner ({dx:.1}, {dy:.1}) should land at the quad's first corner");

    // The full render lands where Photoshop put the layer: (−138, 3)–(584, 218).
    let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    src.fill_rect(Rect::new(0, 0, 800, 600), &[1.0, 0.0, 0.0, 1.0]);
    let out = photocraft_algo::warp::place_source_projective(&src, Rect::new(0, 0, 800, 600), &h, Some(&warp), Some((800.0, 600.0))).unwrap();
    let b = out.content_bounds();
    assert!((b.x0 + 138).abs() <= 2 && (b.y0 - 3).abs() <= 2 && (b.x1 - 584).abs() <= 2 && (b.y1 - 218).abs() <= 2, "{b:?}");
}

/// The reporter's shape: a custom warp defined over a box half the source's size — the curl is
/// applied to the whole source (u, v stay inside `0..=1`), not extrapolated past the patch.
#[test]
fn a_custom_warp_in_placed_units_stays_warped_at_the_frame() {
    // A 1-patch mesh over [0,0,8,8] whose bottom control row folds up to y 4; its control-point
    // box [0,0,8,4] is what the corners frame (see `placed_frame`).
    let mut points = Vec::new();
    for j in 0..4 {
        for i in 0..4 {
            points.push([i as f64 * 8.0 / 3.0, if j == 3 { 4.0 } else { j as f64 * 8.0 / 3.0 }]);
        }
    }
    let corners = [10.0, 10.0, 18.0, 10.0, 18.0, 14.0, 10.0, 14.0];
    let data = sold((8.0, 8.0), corners, warp_desc([0.0, 0.0, 8.0, 8.0], points));
    assert_eq!(parse_placed_size(b"SoLd", &data), Some((8.0, 8.0)));
    let warp = parse_placed_warp(b"SoLd", &data).expect("a custom warp");
    let (_, transform) = parse_smart(b"SoLd", &data);
    let src = red(16);
    let out = photocraft_algo::warp::place_source(&src, Rect::new(0, 0, 16, 16), &transform, Some(&warp), Some((8.0, 8.0))).unwrap();
    about(out.content_bounds(), 10, 10, 18, 14);
}

/// `Sz` in another resolution's units — a placed box of 8 pt at 72 ppi against a 300 ppi
/// source (the 300/72 ≈ 4.17 ratio, not the integer 2 of the other tests).
#[test]
fn a_sold_sz_in_points_scales_the_source_at_300_ppi() {
    let corners = [10.0, 10.0, 18.0, 10.0, 18.0, 18.0, 10.0, 18.0];
    let data = sold((8.0, 8.0), corners, warp_none([0.0, 0.0, 8.0, 8.0]));
    let (_, transform) = parse_smart(b"SoLd", &data);
    // 8 pt ≙ 33.3 px at 300 ppi: the source scales down to the placed box.
    let src = red(33);
    let out = photocraft_algo::warp::place_source(&src, Rect::new(0, 0, 33, 33), &transform, None, Some((8.0, 8.0))).unwrap();
    about(out.content_bounds(), 10, 10, 18, 18);
}

/// A zero, negative, NaN or missing `Sz` is no size at all: `parse_placed_size` rejects it
/// (and `parse_smart` falls back to identity), the renderer's own guard rejects it too, and
/// nothing divides by it — the source's rect is the placed box.
#[test]
fn a_bad_or_missing_sz_is_rejected_not_divided_by() {
    let corners = [10.0, 10.0, 18.0, 10.0, 18.0, 18.0, 10.0, 18.0];
    for sz in [(0.0, 0.0), (-8.0, 8.0), (f64::NAN, 8.0)] {
        let data = sold(sz, corners, warp_none([0.0, 0.0, 8.0, 8.0]));
        assert_eq!(parse_placed_size(b"SoLd", &data), None, "{sz:?}");
        assert_eq!(parse_smart(b"SoLd", &data).1, photocraft_geom::Affine::IDENTITY, "{sz:?}");
    }
    let d = Descriptor::new("soLD")
        .with("Trnf", Value::List(corners.iter().map(|v| Value::Double(*v)).collect()))
        .with("warp", Value::Descriptor(warp_none([0.0, 0.0, 8.0, 8.0])));
    let mut no_sz = b"soLD".to_vec();
    no_sz.extend_from_slice(&4u32.to_be_bytes());
    no_sz.extend_from_slice(&VersionedDescriptor::new(d).to_bytes());
    assert_eq!(parse_placed_size(b"SoLd", &no_sz), None);

    let src = red(16);
    let r = Rect::new(0, 0, 16, 16);
    let t = photocraft_geom::Affine::translate(10.0, 10.0);
    let expect = photocraft_algo::warp::place_source(&src, r, &t, None, None).unwrap().content_bounds();
    for sz in [(0.0, 0.0), (-8.0, 8.0), (f64::NAN, 8.0)] {
        let out = photocraft_algo::warp::place_source(&src, r, &t, None, Some(sz)).unwrap();
        assert_eq!(out.content_bounds(), expect, "{sz:?}");
    }
}
