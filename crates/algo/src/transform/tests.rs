use super::*;

#[test]
fn output_size_prices_the_unclipped_extent_independently_of_position() {
    let src = Rect::new(0, 0, 24, 16);
    for dx in [0.0, 900_000.0, 10_000_000.0] {
        let h = Homography([1.0, 0.0, dx, 0.0, 1.0, -dx, 0.0, 0.0, 1.0]);
        assert_eq!(warp_output_size(src, &h), Some((26, 18)));
    }
    for sx in [1e7, -1e7] {
        let h = Homography([sx, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        assert_eq!(warp_output_size(src, &h), Some((240_000_002, 18)));
    }
}

/// The unclipped size matches the clipped bounds where nothing is clipped, saturates when it
/// can't be represented, and is `None` for empty or singular inputs.
#[test]
fn output_size_saturates_overflows_and_preserves_filter_support() {
    let src = Rect::new(0, 0, 24, 16);
    let fractional = Homography([0.5, 0.0, 0.25, 0.0, 1.0, 0.75, 0.0, 0.0, 1.0]);
    let bounds = warp_output_bounds(src, &fractional).unwrap();
    assert_eq!(warp_output_size(src, &fractional), Some((u64::from(bounds.width()), u64::from(bounds.height()))));
    let huge = Homography([1e308, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    assert_eq!(warp_output_size(src, &huge), Some((u64::MAX, u64::MAX)));
    assert_eq!(warp_output_size(Rect::EMPTY, &Homography::IDENTITY), None);
    assert_eq!(warp_output_size(src, &Homography([0.0; 9])), None);
}

/// [`warp_output_clipped`] says yes exactly when [`warp_output_bounds`] clamps a side (#7807).
#[test]
fn output_clipped_matches_the_clamped_bounds() {
    let src = Rect::new(0, 0, 24, 16);
    let at = |dx: f64| Homography([1.0, 0.0, dx, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    let edge = f64::from(WARP_REACH) - 24.0;
    assert!(!warp_output_clipped(src, &at(edge)), "the right edge lands on the reach");
    assert_eq!(warp_output_bounds(src, &at(edge)).map(|b| b.x1), Some(WARP_REACH + 1));
    assert!(warp_output_clipped(src, &at(edge + 1.0)), "one pixel past it");
    assert_eq!(warp_output_bounds(src, &at(edge + 1.0)).map(|b| b.x1), Some(WARP_REACH + 1), "clamped");
    assert!(warp_output_clipped(src, &at(-f64::from(WARP_REACH) - 1.0)));
    assert!(!warp_output_clipped(Rect::EMPTY, &at(1e12)), "nothing to clip");
}

#[test]
fn the_interruptible_warp_reports_progress_and_stops_when_cancelled() {
    let mut s = Surface::new(PixelFormat::RGBA8);
    s.fill_rect(Rect::new(0, 0, 600, 400), &[0.2, 0.4, 0.6, 1.0]);
    let r = s.content_bounds();
    let h = Homography::rect_to_quad([0.0, 0.0, 600.0, 400.0], [[10.0, 0.0], [590.0, 20.0], [600.0, 400.0], [0.0, 380.0]]).unwrap();
    let reported = std::sync::Mutex::new(0.0f32);
    let progress = |p: f32| {
        let mut m = reported.lock().unwrap();
        *m = m.max(p);
    };
    let never = || false;
    let out = warp_surface_with(&s, r, &h, Interp::Bicubic, &Interrupt::new(&never, &progress)).unwrap().expect("not cancelled");
    assert_eq!(out.read_region(r), warp_surface(&s, r, &h, Interp::Bicubic).unwrap().read_region(r));
    assert!(*reported.lock().unwrap() > 0.9);
    let always = || true;
    assert!(warp_surface_with(&s, r, &h, Interp::Bicubic, &Interrupt::cancel_only(&always)).unwrap().is_none());
}

/// Perspective Crop's map lands each quad corner exactly on its rectangle corner; degenerate,
/// self-crossing, counter-clockwise and non-finite quads have none.
#[test]
fn quad_to_rect_maps_corners_and_refuses_bad_quads() {
    let q = [[30.0, 12.0], [140.0, 25.0], [128.0, 110.0], [14.0, 95.0]];
    let h = quad_to_rect(&q, 100.0, 80.0).unwrap();
    for (c, want) in q.iter().zip([[0.0, 0.0], [100.0, 0.0], [100.0, 80.0], [0.0, 80.0]]) {
        let (x, y) = h.apply(c[0], c[1]);
        assert!((x - want[0]).abs() < 1e-9 && (y - want[1]).abs() < 1e-9, "{c:?} -> ({x}, {y})");
    }
    let (w, ht) = quad_size(&q);
    assert!((w - 112.874).abs() < 1e-3 && (ht - 85.185).abs() < 1e-3, "{w} x {ht}");
    for bad in [
        [[0.0, 0.0], [50.0, 0.0], [100.0, 0.0], [0.0, 50.0]],
        [[0.0, 0.0], [50.0, 50.0], [50.0, 0.0], [0.0, 50.0]],
        [[0.0, 0.0], [0.0, 50.0], [50.0, 50.0], [50.0, 0.0]],
        [[0.0, 0.0], [f64::NAN, 0.0], [50.0, 50.0], [0.0, 50.0]],
        [[0.0, 0.0]; 4],
    ] {
        assert!(quad_to_rect(&bad, 10.0, 10.0).is_none(), "{bad:?}");
    }
    assert!(quad_to_rect(&q, 0.0, 10.0).is_none());
}

fn rgba() -> Surface {
    Surface::new(PixelFormat::RGBA8)
}

/// The "pixel stretch" (#3773): a 1 px column scaled 100x stays solid across the stretch, as in
/// Photoshop. Samples past the source edge repeat the edge, and only the outline is antialiased.
#[test]
fn stretched_single_column_stays_opaque() {
    let mut s = rgba();
    s.fill_rect(Rect::new(0, 0, 1, 4), &[0.0, 1.0, 0.0, 1.0]);
    for interp in [Interp::Bicubic, Interp::Bilinear] {
        let h = Homography::rect_to_quad([0.0, 0.0, 1.0, 4.0], [[0.0, 0.0], [100.0, 0.0], [100.0, 4.0], [0.0, 4.0]]).unwrap();
        let w = warp_surface(&s, Rect::new(0, 0, 1, 4), &h, interp).unwrap();
        assert_eq!(w.content_bounds(), Rect::new(0, 0, 100, 4), "{interp:?}");
        for x in [0, 1, 25, 50, 75, 99] {
            for y in 0..4 {
                assert!((w.sample_channel(x, y, 3) - 1.0).abs() < 1e-6, "{interp:?} alpha at ({x}, {y}) = {}", w.sample_channel(x, y, 3));
            }
        }
    }
}

/// Photoshop 27.11's Free Transform, Bicubic, 200 % about the centre of a 64 px image whose left
/// half is grey 64 and right half grey 192: the row across the edge, columns 26..36.
#[test]
fn bicubic_upscale_matches_photoshop() {
    let mut src = rgba();
    let g = |v: f32| v / 255.0;
    src.fill_rect(Rect::new(0, 0, 32, 64), &[g(64.0), g(64.0), g(64.0), 1.0]);
    src.fill_rect(Rect::new(32, 0, 64, 64), &[g(192.0), g(192.0), g(192.0), 1.0]);
    let q = [[-32.0, -32.0], [96.0, -32.0], [96.0, 96.0], [-32.0, 96.0]];
    let h = Homography::rect_to_quad([0.0, 0.0, 64.0, 64.0], q).unwrap();
    let out = warp_surface(&src, Rect::new(0, 0, 64, 64), &h, Interp::Bicubic).unwrap();
    let photoshop = [64.0, 64.0, 64.0, 60.0, 51.0, 93.0, 163.0, 206.0, 197.0, 192.0];
    for (i, want) in photoshop.iter().enumerate() {
        let got = out.pixel(26 + i as i32, 50)[0] * 255.0;
        assert!((got - want).abs() <= 1.5, "column {}: {got} vs {want}", 26 + i);
    }
}

#[test]
fn homography_maps_rect_corners_and_inverts() {
    let r = [10.0, 20.0, 110.0, 70.0];
    let q = [[0.0, 0.0], [200.0, 10.0], [180.0, 150.0], [-20.0, 120.0]];
    let h = Homography::rect_to_quad(r, q).unwrap();
    for (p, e) in [(10.0, 20.0), (110.0, 20.0), (110.0, 70.0), (10.0, 70.0)].iter().zip(q) {
        let (x, y) = h.apply(p.0, p.1);
        assert!((x - e[0]).abs() < 1e-9 && (y - e[1]).abs() < 1e-9, "{p:?} → {x},{y} vs {e:?}");
    }
    let inv = h.inverse().unwrap();
    let (x, y) = inv.apply(90.0, 60.0);
    let (bx, by) = h.apply(x, y);
    assert!((bx - 90.0).abs() < 1e-9 && (by - 60.0).abs() < 1e-9);
    assert!(Homography::rect_to_quad(r, [[0.0, 0.0]; 4]).is_none() || Homography::rect_to_quad(r, [[0.0, 0.0]; 4]).unwrap().inverse().is_none());
}

#[test]
fn steep_distort_keeps_every_pixel_inside_the_quad() {
    // Corner C is almost flat, so the horizon of the inverse map crosses the 256-px tiles that
    // also hold quad pixels; those tiles must still read every source pixel that lands in them.
    let mut s = rgba();
    s.fill_rect(Rect::new(0, 0, 100, 100), &[1.0, 0.0, 0.0, 1.0]);
    let r = s.content_bounds();
    let h = Homography::rect_to_quad([0.0, 0.0, 100.0, 100.0], [[0.0, 0.0], [300.0, 0.0], [155.0, 155.0], [0.0, 300.0]]).unwrap();
    let w = warp_surface(&s, r, &h, Interp::Nearest).unwrap();
    for (x, y) in [(20, 20), (252, 14), (252, 42), (14, 252), (42, 252)] {
        let a = w.read_region(Rect::new(x, y, x + 1, y + 1))[3];
        assert!(a > 0.99, "({x},{y}) inside the quad is transparent: {a}");
    }
    for (x, y) in [(200, 200), (280, 100), (100, 280)] {
        assert_eq!(w.read_region(Rect::new(x, y, x + 1, y + 1))[3], 0.0, "({x},{y}) outside the quad is painted");
    }
}

/// #4863: a Distort with the bottom corners pulled in and one dragged past the horizon (a
/// concave quad) leaves part of the source behind the horizon. That part maps to the far side,
/// where it showed as a mirrored copy wrapped round infinity; only the front side is drawn.
#[test]
fn a_distort_past_the_horizon_draws_only_its_front_side() {
    let mut s = rgba();
    s.fill_rect(Rect::new(0, 0, 100, 100), &[0.0, 0.5, 1.0, 1.0]);
    let r = s.content_bounds();
    let h = Homography::rect_to_quad([0.0, 0.0, 100.0, 100.0], [[0.0, 0.0], [300.0, 0.0], [60.0, 60.0], [0.0, 300.0]]).unwrap();
    let corners = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)].map(|(x, y)| h.w(x, y));
    assert!(corners.iter().any(|w| *w > 0.0) && corners.iter().any(|w| *w < 0.0), "the source straddles the horizon: {corners:?}");
    let front = h.front_side([0.0, 0.0, 100.0, 100.0]);
    let inv = h.inverse().unwrap();
    let clip = Rect::new(-400, -400, 700, 700);
    for interp in [Interp::Nearest, Interp::Bilinear] {
        let w = warp_surface_clipped(&s, r, &h, interp, clip).unwrap();
        let px = w.read_region(clip);
        let (mut drawn, mut behind) = (0, 0);
        for y in clip.y0..clip.y1 {
            for x in clip.x0..clip.x1 {
                let i = ((y - clip.y0) * clip.width() as i32 + (x - clip.x0)) as usize * 4;
                if px[i + 3] > 0.0 {
                    drawn += 1;
                    // The source point this pixel samples is on the far side of the horizon.
                    if front * inv.w(x as f64 + 0.5, y as f64 + 0.5) <= 0.0 {
                        behind += 1;
                    }
                }
            }
        }
        assert!(drawn > 1000, "{interp:?}: the front side is drawn ({drawn} px)");
        assert_eq!(behind, 0, "{interp:?}: {behind} pixels come from behind the horizon");
    }
    // A quad wholly in front of the horizon is drawn exactly as before.
    let ok = Homography::rect_to_quad([0.0, 0.0, 100.0, 100.0], [[0.0, 0.0], [300.0, 0.0], [155.0, 155.0], [0.0, 300.0]]).unwrap();
    assert_eq!(ok.front_side([0.0, 0.0, 100.0, 100.0]), ok.w(50.0, 50.0).signum());
}

/// #7557: a Perspective Crop quad low on a strongly converging photo puts the horizon across
/// the layer with most of it (and its centre) on the far side. The clip maps wholly into the
/// quad, so its side is the one drawn, not the source's majority.
#[test]
fn a_clip_wholly_on_one_side_draws_that_side() {
    let mut s = rgba();
    s.fill_rect(Rect::new(0, 0, 100, 100), &[1.0, 0.0, 0.0, 1.0]);
    let q = [[40.0, 70.0], [60.0, 70.0], [90.0, 95.0], [10.0, 95.0]];
    let h = quad_to_rect(&q, 50.0, 39.0).unwrap();
    assert!(h.front_side([0.0, 0.0, 100.0, 100.0]) * h.w(50.0, 80.0) < 0.0, "the source's majority is behind the quad's side");
    for interp in [Interp::Nearest, Interp::Bilinear, Interp::Bicubic] {
        let w = warp_surface_clipped(&s, s.content_bounds(), &h, interp, Rect::new(0, 0, 50, 39)).unwrap();
        assert_eq!(w.rgba(25, 20), [1.0, 0.0, 0.0, 1.0], "{interp:?}");
    }
}

#[test]
fn identity_and_translation_are_exact() {
    let mut s = rgba();
    s.fill_rect(Rect::new(4, 4, 20, 12), &[1.0, 0.5, 0.0, 1.0]);
    let r = s.content_bounds();
    for interp in [Interp::Nearest, Interp::Bilinear, Interp::Bicubic] {
        let h = Homography([1.0, 0.0, 7.0, 0.0, 1.0, -3.0, 0.0, 0.0, 1.0]);
        let o = warp_surface(&s, r, &h, interp).unwrap();
        assert_eq!(o.content_bounds(), Rect::new(11, 1, 27, 9), "{interp:?}");
        assert_eq!(o.pixel(15, 5), vec![1.0, 128.0 / 255.0, 0.0, 1.0], "{interp:?}");
    }
}

#[test]
fn translation_preserves_colour_range_at_every_depth() {
    use photocraft_color::{ColorMode, SampleType};

    let h = Homography([1.0, 0.0, 7.0, 0.0, 1.0, -3.0, 0.0, 0.0, 1.0]);
    for sample in SampleType::ALL {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for alpha in [false, true] {
                let mut s = Surface::new(PixelFormat::new(mode, sample, alpha));
                let mut pixel = if mode == ColorMode::Rgb { vec![2.0, -0.25, 0.5] } else { vec![2.0] };
                if alpha {
                    pixel.push(0.5);
                }
                s.fill_rect(Rect::new(4, 4, 8, 8), &pixel);
                let mut expected = s.pixel(5, 5);
                if !alpha {
                    expected.push(1.0);
                }
                for interp in [Interp::Nearest, Interp::Bilinear, Interp::Bicubic] {
                    let out = warp_surface(&s, s.content_bounds(), &h, interp).unwrap();
                    assert_eq!(out.pixel(12, 2), expected, "{sample:?}/{mode:?}/alpha={alpha}/{interp:?}");
                    assert_eq!(out.pixel(0, 0).last(), Some(&0.0));
                }
            }
        }
    }
}

#[test]
fn bilinear_translation_interpolates_signed_hdr_colours() {
    let mut s = Surface::new(PixelFormat::RGBA32F);
    s.write_pixel(0, 0, &[2.0, -0.5, 0.25, 0.5]);
    s.write_pixel(1, 0, &[4.0, -0.25, 0.75, 0.5]);
    let h = Homography([1.0, 0.0, 0.5, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    let out = warp_surface(&s, s.content_bounds(), &h, Interp::Bilinear).unwrap();
    assert_eq!(out.pixel(1, 0), vec![3.0, -0.375, 0.5, 0.5]);
}

#[test]
fn scale_doubles_size_and_rotation_keeps_area() {
    let mut s = rgba();
    s.fill_rect(Rect::new(0, 0, 40, 20), &[0.2, 0.4, 0.6, 1.0]);
    let r = s.content_bounds();
    let up =
        warp_surface(&s, r, &Homography::rect_to_quad([0.0, 0.0, 40.0, 20.0], [[0.0, 0.0], [80.0, 0.0], [80.0, 40.0], [0.0, 40.0]]).unwrap(), Interp::Bicubic)
            .unwrap();
    let b = up.content_bounds();
    assert!(b.width() >= 80 && b.width() <= 82 && b.height() >= 40 && b.height() <= 42, "{b:?}");
    // 90° rotation about (20, 10): a 40×20 box becomes 20×40.
    let q = [[30.0, -10.0], [30.0, 30.0], [10.0, 30.0], [10.0, -10.0]];
    let rot = warp_surface(&s, r, &Homography::rect_to_quad([0.0, 0.0, 40.0, 20.0], q).unwrap(), Interp::Bilinear).unwrap();
    let b = rot.content_bounds();
    assert!(b.width().abs_diff(20) <= 2 && b.height().abs_diff(40) <= 2, "{b:?}");
    let p = rot.pixel(20, 10);
    assert!((p[0] - 0.2).abs() < 0.01 && p[3] > 0.99, "{p:?}");
}

/// #6203: a bicubic rotation of a sprite whose edge pixels are partly transparent overshoots
/// its alpha past 1. The colour must stay the sprite's, not grow with the overshoot into a
/// light border.
#[test]
fn rotation_keeps_the_colour_of_a_soft_edged_sprite() {
    let mut s = rgba();
    s.fill_rect(Rect::new(10, 10, 30, 30), &[0.8, 0.6, 0.4, 0.5]);
    s.fill_rect(Rect::new(11, 11, 29, 29), &[0.8, 0.6, 0.4, 1.0]);
    let (c, k) = (20.0, std::f64::consts::FRAC_1_SQRT_2 * 10.0);
    let q = [[c, c - 2.0 * k], [c + 2.0 * k, c], [c, c + 2.0 * k], [c - 2.0 * k, c]];
    let h = Homography::rect_to_quad([10.0, 10.0, 30.0, 30.0], q).unwrap();
    for interp in [Interp::Bicubic, Interp::Bilinear] {
        let out = warp_surface(&s, s.content_bounds(), &h, interp).unwrap();
        let b = out.content_bounds();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let p = out.pixel(x, y);
                if p[3] > 0.0 {
                    let off = (0..3).map(|i| (p[i] - [0.8, 0.6, 0.4][i]).abs()).fold(0.0f32, f32::max);
                    assert!(off <= 1.0 / 255.0, "{interp:?} ({x}, {y}): {p:?}");
                }
            }
        }
    }
}

#[test]
fn large_downscale_is_prefiltered() {
    // A 1px checkerboard shrunk 8× must come out ~50% grey, not aliased black/white.
    let mut s = rgba();
    for y in 0..64 {
        for x in 0..64 {
            let v = ((x + y) % 2) as f32;
            s.fill_rect(Rect::new(x, y, x + 1, y + 1), &[v, v, v, 1.0]);
        }
    }
    let h = Homography::rect_to_quad([0.0, 0.0, 64.0, 64.0], [[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]]).unwrap();
    let o = warp_surface(&s, s.content_bounds(), &h, Interp::Bicubic).unwrap();
    let p = o.pixel(4, 4);
    assert!((p[0] - 0.5).abs() < 0.1, "{p:?}");
}

/// 64 × 64 opaque white lines on black, `width` px wide every `period` px: columns when
/// `vertical`, rows otherwise.
fn lines(sample: photocraft_color::SampleType, vertical: bool, width: i32, period: i32) -> Surface {
    let mut s = Surface::new(PixelFormat::new(photocraft_color::ColorMode::Rgb, sample, true));
    s.fill_rect(Rect::new(0, 0, 64, 64), &[0.0, 0.0, 0.0, 1.0]);
    for i in (0..64).filter(|i| i % period >= period - width) {
        s.fill_rect(if vertical { Rect::new(i, 0, i + 1, 64) } else { Rect::new(0, i, 64, i + 1) }, &[1.0, 1.0, 1.0, 1.0]);
    }
    s
}

/// #4864: Free Transform to 100 % × 25 % shrinks only y, so 1 px columns keep their full
/// contrast along x. The Bicubic prefilter used one factor for both axes, √|det J| = 50 %, which
/// averaged the columns to flat grey and stretched them back. The shrunk axis is still
/// prefiltered: rows 3 px apart come out an even third grey instead of aliasing into bands.
#[test]
fn a_one_axis_reduction_keeps_the_other_axis_sharp() {
    let h = Homography::rect_to_quad([0.0, 0.0, 64.0, 64.0], [[0.0, 0.0], [64.0, 0.0], [64.0, 16.0], [0.0, 16.0]]).unwrap();
    for sample in photocraft_color::SampleType::ALL {
        let s = lines(sample, true, 1, 2);
        let o = warp_surface(&s, s.content_bounds(), &h, Interp::Bicubic).unwrap();
        for x in 4..60 {
            let (a, b) = (o.pixel(x, 8)[0], o.pixel(x + 1, 8)[0]);
            assert!((a - b).abs() > 0.9, "{sample:?}: columns {x} and {}: {a} vs {b}", x + 1);
        }
        let s = lines(sample, false, 1, 3);
        let o = warp_surface(&s, s.content_bounds(), &h, Interp::Bicubic).unwrap();
        for y in 2..14 {
            let v = o.pixel(32, y)[0];
            assert!((v - 1.0 / 3.0).abs() < 0.1, "{sample:?}: row {y}: {v}");
        }
    }
}

/// #4864: both axes shrink, by different amounts (87.5 % × 25 %). Each needs its own filter:
/// one designed for 25 % blurs 4 px bars that only shrink to 87.5 % along x.
#[test]
fn unequal_reductions_filter_each_axis_for_its_own_scale() {
    let h = Homography::rect_to_quad([0.0, 0.0, 64.0, 64.0], [[0.0, 0.0], [56.0, 0.0], [56.0, 16.0], [0.0, 16.0]]).unwrap();
    for sample in photocraft_color::SampleType::ALL {
        let s = lines(sample, true, 4, 8);
        let o = warp_surface(&s, s.content_bounds(), &h, Interp::Bicubic).unwrap();
        let steepest = (4..50).map(|x| (o.pixel(x, 8)[0] - o.pixel(x + 1, 8)[0]).abs()).fold(0.0f32, f32::max);
        assert!(steepest > 0.8, "{sample:?}: bar edges blurred along x, steepest step {steepest}");
    }
}

/// #4864: the prefilter's factors, per axis. An affine map gives the same pair with or without
/// a clip, so a clipped warp matches the full one; a perspective Distort keeps the one factor at
/// its centre, as before; a skew reduces nothing, and a squash along a slant keeps the one
/// factor; a map past the horizon stays in range; and a map that isn't finite reduces nothing.
#[test]
fn prefilter_scales_are_per_axis_and_finite() {
    let r = Rect::new(0, 0, 64, 64);
    let quad = |q| Homography::rect_to_quad([0.0, 0.0, 64.0, 64.0], q).unwrap();
    assert_eq!(prefilter_scales(&quad([[0.0, 0.0], [64.0, 0.0], [64.0, 16.0], [0.0, 16.0]]), r, None), (1.0, 0.25));
    let eighth = Homography([0.125, 0.0, 24.0, 0.0, 0.125, 30.0, 0.0, 0.0, 1.0]);
    assert_eq!(prefilter_scales(&eighth, r, None), (0.125, 0.125));
    assert_eq!(prefilter_scales(&eighth, r, Some(Rect::new(10, 10, 52, 52))), (0.125, 0.125));
    let trapezoid = quad([[0.0, 0.0], [64.0, 0.0], [48.0, 24.0], [16.0, 24.0]]);
    let one = trapezoid.local_scale(32.0, 32.0);
    assert!(one > 0.4 && one < 0.5, "{one}");
    assert_eq!(prefilter_scales(&trapezoid, r, None), (one, one));
    assert_eq!(prefilter_scales(&quad([[0.0, 0.0], [64.0, 0.0], [256.0, 64.0], [192.0, 64.0]]), r, None), (1.0, 1.0));
    // Skewed by 2 and flattened to 5 %, and a rhombus squashed to 25 % along its diagonal: both
    // keep the one factor, √|det J|. A 4× area rule left the skew's warp 5 % along its slant (the
    // one factor 10 %), and the column lengths left the rhombus's 34 % (the one factor 50 %).
    let (fx, fy) = prefilter_scales(&quad([[0.0, 0.0], [64.0, 0.0], [192.0, 3.2], [128.0, 3.2]]), r, None);
    assert!(fx == fy && (fx * fy - 0.05).abs() < 1e-9, "{fx} x {fy}");
    let h = std::f64::consts::FRAC_1_SQRT_2 * 64.0;
    let (fx, fy) = prefilter_scales(&quad([[0.0, 0.0], [h, h / 4.0], [0.0, h / 2.0], [-h, h / 4.0]]), r, None);
    assert!(fx == fy && (fx - 0.5).abs() < 1e-9, "{fx} x {fy}");
    let past = Homography::rect_to_quad([0.0, 0.0, 100.0, 100.0], [[0.0, 0.0], [300.0, 0.0], [60.0, 60.0], [0.0, 300.0]]).unwrap();
    for clip in [None, Some(Rect::new(-400, -400, 700, 700))] {
        let (fx, fy) = prefilter_scales(&past, Rect::new(0, 0, 100, 100), clip);
        assert!(fx > 0.0 && fx <= 1.0 && fy > 0.0 && fy <= 1.0, "{clip:?}: {fx} x {fy}");
    }
    for h in [Homography([f64::NAN; 9]), Homography([f64::INFINITY, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])] {
        assert_eq!(prefilter_scales(&h, r, None), (1.0, 1.0), "{h:?}");
        assert_eq!(prefilter_scales(&h, r, Some(r)), (1.0, 1.0), "{h:?}");
    }
}

#[test]
fn surfaces_without_alpha_gain_it() {
    let mut s = Surface::new(PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8, false));
    s.fill_rect(Rect::new(0, 0, 10, 10), &[1.0, 0.0, 0.0]);
    let h = Homography([1.0, 0.0, 5.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    let o = warp_surface(&s, Rect::new(0, 0, 10, 10), &h, Interp::Nearest).unwrap();
    assert!(o.format().alpha);
    assert_eq!(o.pixel(2, 2)[3], 0.0);
    assert_eq!(o.pixel(7, 2), vec![1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn homography_is_identity() {
    assert!(Homography::IDENTITY.is_identity());
    assert!(Homography([2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0]).is_identity());
    assert!(!Homography([1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).is_identity());
    assert!(!Homography([1.0, 0.1, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).is_identity());
}

/// A uniform 50 % scale turned through `rect_to_quad`, as Free Transform builds it, keeps the
/// one factor exactly. Its column lengths differ from the factor in the last bits at most
/// angles, which picked the order of the two passes and changed the output by a level.
#[test]
fn a_turned_uniform_scale_keeps_the_one_factor() {
    let r = Rect::new(0, 0, 6000, 4000);
    for tenth in (5..3600).step_by(37) {
        let (s, c) = (f64::from(tenth) / 10.0).to_radians().sin_cos();
        let at = |x: f64, y: f64| {
            let (dx, dy) = (x - 3000.0, y - 2000.0);
            [3000.0 + 0.5 * (c * dx - s * dy) + 17.3, 2000.0 + 0.5 * (s * dx + c * dy) - 4.1]
        };
        let h = Homography::rect_to_quad([0.0, 0.0, 6000.0, 4000.0], [at(0.0, 0.0), at(6000.0, 0.0), at(6000.0, 4000.0), at(0.0, 4000.0)]).unwrap();
        let one = h.local_scale(3000.0, 2000.0);
        assert_eq!(prefilter_scales(&h, r, None), (one, one), "{tenth} tenths of a degree");
    }
}

/// #4864: a rhombus squashed to 25 % along its diagonal (rotated 45°, then squashed vertically).
/// Both columns of the map are 73 % long, so prefiltering each axis by its column left the warp
/// a 34 % reduction along the slant, and 2 px diagonal stripes came out as moiré (standard
/// deviation 0.27 around the 0.5 grey they average to, at 8 bits); the one factor, 50 %, gives
/// 0.06, and so does this.
#[test]
fn a_squash_along_a_slant_does_not_alias() {
    let k = std::f64::consts::FRAC_1_SQRT_2 * 128.0;
    let h = Homography::rect_to_quad([0.0, 0.0, 128.0, 128.0], [[0.0, 0.0], [k, k / 4.0], [0.0, k / 2.0], [-k, k / 4.0]]).unwrap();
    for sample in photocraft_color::SampleType::ALL {
        let mut s = Surface::new(PixelFormat::new(photocraft_color::ColorMode::Rgb, sample, true));
        s.fill_rect(Rect::new(0, 0, 128, 128), &[0.0, 0.0, 0.0, 1.0]);
        for y in 0..128 {
            for x in (0..128).filter(|x| (x + y) % 4 < 2) {
                s.fill_rect(Rect::new(x, y, x + 1, y + 1), &[1.0, 1.0, 1.0, 1.0]);
            }
        }
        let o = warp_surface(&s, s.content_bounds(), &h, Interp::Bicubic).unwrap();
        let b = o.content_bounds();
        let opaque = |x: i32, y: i32| (-1..=1).all(|j| (-1..=1).all(|i| o.pixel(x + i, y + j)[3] > 0.999));
        let v: Vec<f64> =
            (b.y0..b.y1).flat_map(|y| (b.x0..b.x1).map(move |x| (x, y))).filter(|&(x, y)| opaque(x, y)).map(|(x, y)| f64::from(o.pixel(x, y)[0])).collect();
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        let sd = (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / v.len() as f64).sqrt();
        assert!(v.len() > 1000 && (mean - 0.5).abs() < 0.02 && sd < 0.1, "{sample:?}: {} pixels, mean {mean}, sd {sd}", v.len());
    }
}

/// #4864: whatever the map, rect or clip (zero, NaN, huge, tiny, on or past the horizon,
/// collapsed to a line), each factor stays within [1/64, 1].
#[test]
fn prefilter_scales_stay_in_range_for_hostile_maps() {
    let big = Rect::new(i32::MIN / 2, i32::MIN / 2, i32::MAX / 2, i32::MAX / 2);
    let rects = [Rect::new(0, 0, 64, 64), Rect::new(-5, -7, -4, -6), big, Rect::new(0, 0, 1, 100_000), Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX)];
    let maps = [
        Homography([0.0; 9]),
        Homography([f64::NAN; 9]),
        Homography([1e300, 0.0, 0.0, 0.0, 1e300, 0.0, 0.0, 0.0, 1.0]),
        Homography([1e-300, 0.0, 0.0, 0.0, 1e-300, 0.0, 0.0, 0.0, 1.0]),
        Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -1.0]),
        Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0]),
        Homography([1.0, 0.0, 0.0, 0.0, 1e-9, 0.0, 0.0, 0.0, 1.0]),
        Homography([1.0, 1e9, 0.0, 0.0, 1e-9, 0.0, 0.0, 0.0, 1.0]),
        Homography([f64::MAX, f64::MAX, 0.0, f64::MAX, f64::MAX, 0.0, 0.0, 0.0, 1.0]),
        Homography([-0.25, 0.0, 0.0, 0.0, -0.5, 0.0, 0.0, 0.0, 1.0]),
        Homography([0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 1e-3, 1e-3, 1.0]),
        Homography([0.5, 0.0, 0.0, 0.0, 0.5, 0.0, -1e-3, -1e-3, 1.0]),
        Homography([f64::MIN_POSITIVE, 0.0, 0.0, 0.0, f64::MIN_POSITIVE, 0.0, 0.0, 0.0, 1.0]),
        Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, f64::MIN_POSITIVE]),
    ];
    let clips =
        [None, Some(Rect::new(0, 0, 64, 64)), Some(big), Some(Rect::new(1000, 1000, 1001, 1001)), Some(Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX))];
    for r in rects {
        for h in maps {
            for c in clips {
                let (fx, fy) = prefilter_scales(&h, r, c);
                assert!((1.0 / 64.0..=1.0).contains(&fx) && (1.0 / 64.0..=1.0).contains(&fy), "{h:?} {r:?} {c:?}: {fx} x {fy}");
            }
        }
    }
}

/// #4864: a Bicubic warp by a degenerate map, or one that shrinks a single axis far below the
/// 1/64 the prefilter stops at, returns finite pixels instead of panicking (32-bit, where a NaN
/// would show).
#[test]
fn bicubic_warp_survives_degenerate_and_extreme_maps() {
    let mut s = Surface::new(PixelFormat::new(photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::F32, true));
    s.fill_rect(Rect::new(0, 0, 40, 30), &[0.3, 0.6, 0.9, 1.0]);
    let maps = [
        Homography([0.0; 9]),
        Homography([f64::NAN; 9]),
        Homography([1e-300, 0.0, 0.0, 0.0, 1e-300, 0.0, 0.0, 0.0, 1.0]),
        Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -1.0]),
        Homography([1.0, 0.0, 0.0, 0.0, 1e-9, 0.0, 0.0, 0.0, 1.0]),
        Homography([1e-9, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]),
        Homography([1.0, 0.0, 0.0, 0.0, 0.001, 0.0, 0.0, 0.0, 1.0]),
        Homography([0.001, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]),
        Homography([1.0, 3.0, 0.0, 0.0, 0.01, 0.0, 0.0, 0.0, 1.0]),
        Homography([-0.25, 0.0, 0.0, 0.0, -0.5, 0.0, 0.0, 0.0, 1.0]),
        Homography([0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.05, 0.0, 1.0]),
        Homography([0.5, 0.0, 0.0, 0.0, 0.5, 0.0, -0.05, -0.05, 1.0]),
    ];
    for h in maps {
        for clip in [None, Some(Rect::new(-50, -50, 50, 50))] {
            let o = match clip {
                None => warp_surface(&s, s.content_bounds(), &h, Interp::Bicubic),
                Some(c) => warp_surface_clipped(&s, s.content_bounds(), &h, Interp::Bicubic, c),
            };
            if let Ok(o) = o {
                let b = o.content_bounds();
                if !b.is_empty() {
                    assert!(o.read_region(b).iter().all(|v| v.is_finite()), "{h:?}");
                }
            }
        }
    }
}

/// #4864: the per-axis prefilter keeps a flat layer flat in Grayscale, RGB, CMYK and Lab, at
/// every depth, with or without alpha (2 to 5 channels through each pass).
#[test]
fn the_per_axis_prefilter_keeps_flat_colour_in_every_mode() {
    use photocraft_color::{ColorMode, SampleType};
    let quads = [
        ([[0.0, 0.0], [64.0, 0.0], [64.0, 16.0], [0.0, 16.0]], (32, 8)),
        ([[0.0, 0.0], [56.0, 0.0], [56.0, 16.0], [0.0, 16.0]], (28, 8)),
        ([[0.0, 0.0], [64.0, 0.0], [48.0, 24.0], [16.0, 24.0]], (32, 12)),
        ([[0.0, 0.0], [8.0, 0.0], [8.0, 64.0], [0.0, 64.0]], (4, 32)),
    ];
    for mode in [ColorMode::Grayscale, ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        for sample in SampleType::ALL {
            for alpha in [false, true] {
                let fmt = PixelFormat::new(mode, sample, alpha);
                let n = fmt.channels();
                let col: Vec<f32> = (0..n).map(|c| if alpha && c == n - 1 { 1.0 } else { 0.2 + 0.15 * c as f32 }).collect();
                let mut s = Surface::new(fmt);
                s.fill_rect(Rect::new(0, 0, 64, 64), &col);
                for (q, (x, y)) in quads {
                    let h = Homography::rect_to_quad([0.0, 0.0, 64.0, 64.0], q).unwrap();
                    let o = warp_surface(&s, s.content_bounds(), &h, Interp::Bicubic).unwrap();
                    let p = o.pixel(x, y);
                    let m = o.format().channels();
                    assert_eq!(m, mode.color_channels() + 1);
                    assert!((p[m - 1] - 1.0).abs() < 1e-3, "{mode:?} {sample:?} {alpha} {q:?}: alpha {p:?}");
                    for c in 0..mode.color_channels() {
                        assert!((p[c] - col[c]).abs() < 2.5 / 255.0, "{mode:?} {sample:?} {alpha} {q:?}: channel {c} {p:?} vs {col:?}");
                    }
                }
            }
        }
    }
}

/// #4864: squashed to 100 % × 25 %, the layer's left and right edges stay as crisp as a direct
/// (Bilinear) sample. The one factor (50 %) also reduced x, which left the first column inside
/// the left edge at 0.93 alpha against Bilinear's 1.
#[test]
fn a_one_axis_reduction_keeps_the_side_edges_sharp() {
    let h = Homography::rect_to_quad([0.0, 0.0, 64.0, 64.0], [[10.0, 0.0], [74.0, 0.0], [74.0, 16.0], [10.0, 16.0]]).unwrap();
    for sample in photocraft_color::SampleType::ALL {
        let mut s = Surface::new(PixelFormat::new(photocraft_color::ColorMode::Rgb, sample, true));
        s.fill_rect(Rect::new(0, 0, 64, 64), &[0.5, 0.5, 0.5, 1.0]);
        let bc = warp_surface(&s, s.content_bounds(), &h, Interp::Bicubic).unwrap();
        let bl = warp_surface(&s, s.content_bounds(), &h, Interp::Bilinear).unwrap();
        for x in [9, 10, 11, 72, 73, 74] {
            let (a, b) = (bc.pixel(x, 8)[3], bl.pixel(x, 8)[3]);
            assert!((a - b).abs() < 0.02, "{sample:?} x {x}: Bicubic alpha {a} vs Bilinear {b}");
        }
    }
}
