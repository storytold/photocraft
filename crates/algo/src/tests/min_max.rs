//! Minimum / Maximum at fractional radii (#10474): the result moves from the hard element at the
//! radius toward the next larger one, so every radius in the dialog changes the image.
use super::*;

fn both_ops(radius: f32, preserve: Preserve) -> [FilterParams; 2] {
    [FilterParams::Minimum { radius, preserve }, FilterParams::Maximum { radius, preserve }]
}

/// Grey or RGB test images over `R`, opaque: the textured pattern, a horizontal ramp and a hard
/// vertical edge.
fn images(depth: SampleType) -> Vec<Surface> {
    let ramp = |v: &dyn Fn(i32) -> f32| {
        let mut s = Surface::new(fmt(depth));
        let px: Vec<f32> = (R.y0..R.y1).flat_map(|_| (R.x0..R.x1).flat_map(|x| [v(x), v(x), v(x), 1.0])).collect();
        s.write_region(R, &px);
        s
    };
    vec![pattern(depth, R), ramp(&|x| x as f32 / 39.0), ramp(&|x| if x < 20 { 0.0 } else { 1.0 })]
}

#[test]
fn fractional_min_max_radius_changes_the_image_10474() {
    for depth in [SampleType::U16, SampleType::F32] {
        let s = pattern(depth, R);
        let src = s.read_region(R);
        for preserve in [Preserve::Squareness, Preserve::Roundness] {
            for (lo, hi) in both_ops(0.2, preserve).into_iter().zip(both_ops(0.5, preserve)) {
                let at_02 = run(&s, &lo).read_region(R);
                assert!(at_02 != src, "{depth:?} {} r 0.2 did nothing", lo.label());
                assert!(run(&s, &hi).read_region(R) != at_02, "{depth:?} {} r 0.5 same as r 0.2", hi.label());
            }
            for (lo, hi) in both_ops(1.0, preserve).into_iter().zip(both_ops(1.2, preserve)) {
                assert!(run(&s, &lo).read_region(R) != run(&s, &hi).read_region(R), "{depth:?} {preserve:?} {} r 1.2 same as r 1", hi.label());
            }
        }
    }
}

#[test]
fn half_radius_grades_a_single_pixel() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let tol = tolerance(depth);
        // Maximum spreads a white pixel on black; Minimum a black pixel on white, the same way.
        for (max, back, dot) in [(true, 0.0, 1.0), (false, 1.0, 0.0)] {
            let mut s = flat(depth, R, [back, back, back, 1.0]);
            s.write_pixel(20, 15, &[dot, dot, dot, 1.0]);
            for preserve in [Preserve::Squareness, Preserve::Roundness] {
                let p = if max { FilterParams::Maximum { radius: 0.5, preserve } } else { FilterParams::Minimum { radius: 0.5, preserve } };
                let out = run(&s, &p);
                // Away from the layer's edge, where Minimum also pulls in the transparent outside.
                for y in 10..20i32 {
                    for x in 15..25i32 {
                        let (dx, dy) = ((x - 20).abs(), (y - 15).abs());
                        let near = match preserve {
                            Preserve::Squareness => dx.max(dy) == 1,
                            Preserve::Roundness => dx + dy == 1,
                        };
                        let want = if dx + dy == 0 {
                            dot
                        } else if near {
                            0.5
                        } else {
                            back
                        };
                        let px = out.pixel(x, y);
                        assert!((px[0] - want).abs() <= tol && px[3] == 1.0, "{depth:?} {} at {dx},{dy}: {px:?}", p.label());
                    }
                }
            }
        }
    }
}

#[test]
fn smallest_and_largest_radii() {
    for depth in [SampleType::U8, SampleType::F32] {
        let mut s = flat(depth, R, [0.0, 0.0, 0.0, 1.0]);
        s.write_pixel(20, 15, &[1.0, 1.0, 1.0, 1.0]);
        for preserve in [Preserve::Squareness, Preserve::Roundness] {
            let out = run(&s, &FilterParams::Maximum { radius: 0.2, preserve });
            assert!((out.pixel(21, 15)[0] - 0.2).abs() <= tolerance(depth), "{depth:?} {preserve:?}: {:?}", out.pixel(21, 15));
            let diagonal = if preserve == Preserve::Squareness { 0.2 } else { 0.0 };
            assert!((out.pixel(21, 16)[0] - diagonal).abs() <= tolerance(depth), "{depth:?} {preserve:?}: {:?}", out.pixel(21, 16));
        }
    }
    // The dialog's top end runs without a panic or a hang. On a layer this small both radii
    // reach the whole layer from every pixel, so they agree.
    let small = Rect::new(0, 0, 6, 5);
    let s = pattern(SampleType::F32, small);
    for preserve in [Preserve::Squareness, Preserve::Roundness] {
        for (half, full) in both_ops(499.5, preserve).into_iter().zip(both_ops(500.0, preserve)) {
            let a = apply(&s, &half, small, small, None);
            let b = apply(&s, &full, small, small, None);
            assert_eq!(a.read_region(small), b.read_region(small), "{}", half.label());
        }
    }
}

/// An opaque white square on transparent black: the half-transparent fringe of r 0.5 stays white
/// (the blend is premultiplied, so the transparent black doesn't darken it).
#[test]
fn fractional_fringes_blend_premultiplied() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let tol = tolerance(depth);
        let mut s = Surface::new(fmt(depth));
        s.fill_rect(Rect::new(10, 8, 20, 16), &[1.0, 1.0, 1.0, 1.0]);
        for preserve in [Preserve::Squareness, Preserve::Roundness] {
            for p in both_ops(0.5, preserve) {
                let out = apply(&s, &p, R, R, None);
                // Maximum's fringe is outside the square, Minimum's inside its edge.
                let fringe = if matches!(p, FilterParams::Maximum { .. }) { (9, 11) } else { (10, 11) };
                let a = out.pixel(fringe.0, fringe.1)[3];
                assert!((a - 0.5).abs() <= tol, "{depth:?} {} fringe alpha {a}", p.label());
                for y in R.y0..R.y1 {
                    for x in R.x0..R.x1 {
                        let px = out.pixel(x, y);
                        let want = if px[3] > 0.0 { 1.0 } else { 0.0 };
                        assert!(px[..3].iter().all(|v| (v - want).abs() <= tol), "{depth:?} {} at {x},{y}: {px:?}", p.label());
                    }
                }
            }
        }
    }
}

#[test]
fn fractional_sweep_is_monotone_and_continuous() {
    // Compared as they show: colour times alpha, and alpha. At the layer's edge Minimum keeps the
    // colour while alpha fades, then drops it at alpha 0.
    let premultiplied = |v: Vec<f32>| v.as_chunks::<4>().0.iter().flat_map(|[r, g, b, a]| [r * a, g * a, b * a, *a]).collect::<Vec<_>>();
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for (i, s) in images(depth).iter().enumerate() {
            let src = premultiplied(s.read_region(R));
            for preserve in [Preserve::Squareness, Preserve::Roundness] {
                for max in [false, true] {
                    let at = |radius: f32| {
                        let p = if max { FilterParams::Maximum { radius, preserve } } else { FilterParams::Minimum { radius, preserve } };
                        premultiplied(run(s, &p).read_region(R))
                    };
                    let at_1 = at(1.0);
                    let mut prev = (0.0, src.clone());
                    for step in 4..=40 {
                        let r = step as f32 * 0.05;
                        let cur = at(r);
                        for (j, (a, b)) in prev.1.iter().zip(&cur).enumerate() {
                            assert!(if max { b >= a } else { b <= a }, "{depth:?} image {i} {preserve:?} max {max}: sample {j} r {} {a} → r {r} {b}", prev.0);
                        }
                        // Below 1 every step moves every sample that r 1 moves (U8 steps are too fine).
                        if r < 1.0 && depth != SampleType::U8 {
                            for (j, ((a, b), (s0, s1))) in prev.1.iter().zip(&cur).zip(src.iter().zip(&at_1)).enumerate() {
                                assert!(s0 == s1 || a != b, "{depth:?} image {i} {preserve:?} max {max}: sample {j} stuck at r {r}");
                            }
                        }
                        prev = (r, cur);
                    }
                    for k in [1.0f32, 2.0] {
                        let (below, exact) = (at(k - 1e-3), at(k));
                        for (j, (a, b)) in below.iter().zip(&exact).enumerate() {
                            assert!(
                                (a - b).abs() <= 2e-3 + tolerance(depth),
                                "{depth:?} image {i} {preserve:?} max {max}: sample {j} jumps at r {k}: {a} → {b}"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Release timing on 36 MP RGBA8:
/// `cargo test -p photocraft-algo --release --lib -- --ignored --nocapture min_max_fractional_timing`.
#[test]
#[ignore]
fn min_max_fractional_timing() {
    let r = Rect::new(0, 0, 6000, 6000);
    let s = pattern(SampleType::U8, r);
    for preserve in [Preserve::Squareness, Preserve::Roundness] {
        for radius in [1.0f32, 1.5, 5.0, 5.5] {
            let p = FilterParams::Minimum { radius, preserve };
            let mut ms: Vec<f64> = (0..3)
                .map(|_| {
                    let t = std::time::Instant::now();
                    std::hint::black_box(apply(&s, &p, r, r, None));
                    t.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            ms.sort_by(f64::total_cmp);
            eprintln!("Minimum {preserve:?} r {radius}: {:.0} ms (median of 3)", ms[1]);
        }
    }
}
