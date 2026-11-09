use super::lines::Segment;
use super::upright::{self, guided, homography_from_segments, level_angle, vanishing_point};
use super::*;

fn pt(m: &[f64; 9], x: f64, y: f64) -> [f64; 2] {
    let w = m[6] * x + m[7] * y + m[8];
    [(m[0] * x + m[1] * y + m[2]) / w, (m[3] * x + m[4] * y + m[5]) / w]
}

/// A matrix fitted on Photoshop's output, in half-width units (centred, `u` pixels per unit),
/// as centred pixels.
fn from_units(h: [f64; 8], u: f64) -> [f64; 9] {
    [h[0], h[1], h[2] * u, h[3], h[4], h[5] * u, h[6] / u, h[7] / u, 1.0]
}

fn grid(w: f64, h: f64) -> Vec<[f64; 2]> {
    let mut g = Vec::new();
    for j in 0..7 {
        for i in 0..9 {
            g.push([(i as f64 + 0.5) * w / 9.0 - w / 2.0, (j as f64 + 0.5) * h / 7.0 - h / 2.0]);
        }
    }
    g
}

fn worst(a: &[f64; 9], b: &[f64; 9], w: f64, h: f64) -> f64 {
    grid(w, h)
        .iter()
        .map(|p| {
            let (x, y) = (pt(a, p[0], p[1]), pt(b, p[0], p[1]));
            let d = (x[0] - y[0]).hypot(x[1] - y[1]);
            // `f64::max` drops a NaN, which would let a broken matrix pass.
            assert!(d.is_finite(), "non-finite distance at {p:?}");
            d
        })
        .fold(0.0, f64::max)
}

fn t(f: impl FnOnce(&mut Transform)) -> Transform {
    let mut t = Transform::default();
    f(&mut t);
    t
}

#[test]
fn manual_sliders_match_photoshops_measured_homographies() {
    // (settings, Photoshop's homography in half-width units of a 1200 × 800 image)
    let cases: [(Transform, [f64; 8]); 6] = [
        (t(|t| t.vertical = 50.0), [0.9999, -0.00002, 0.00003, 0.0, 1.10756, 0.00021, 0.0, -0.2501]),
        (t(|t| t.horizontal = -50.0), [1.1074, 0.00001, -0.00003, 0.00001, 1.00035, 0.00011, 0.25016, -0.00003]),
        (t(|t| t.rotate = 10.0), [0.98458, -0.17364, 0.0, 0.17359, 0.98503, 0.00012, -0.00003, -0.00006]),
        (t(|t| t.aspect = -100.0), [1.09521, 0.0, 0.00002, -0.00001, 0.91312, 0.00011, -0.00003, 0.00003]),
        (
            t(|t| {
                t.vertical = 30.0;
                t.horizontal = 20.0
            }),
            [1.01342, 0.02038, 0.00005, 0.02031, 1.03075, 0.00017, -0.1, -0.1501],
        ),
        (
            t(|t| {
                t.vertical = -20.0;
                t.horizontal = 15.0;
                t.rotate = -3.0;
                t.aspect = 20.0;
                t.scale = 110.0;
                t.x = 10.0;
                t.y = -15.0;
            }),
            [1.07101, 0.06376, 0.1623, -0.0813, 1.14745, 0.1623, -0.08033, 0.09579],
        ),
    ];
    for (tr, ps) in cases {
        let ours = manual(&tr, 1200.0, 800.0);
        let d = worst(&ours, &from_units(ps, 600.0), 1200.0, 800.0);
        assert!(d < 1.5, "{tr:?}: {d} px from Photoshop");
    }
}

#[test]
fn offsets_follow_the_frame_and_keystones_the_diagonal() {
    // Offset X 10 moves a 1200 × 800 image 97.4 px right and a square 1000 one 63.6 px.
    let m = manual(&t(|t| t.x = 10.0), 1200.0, 800.0);
    assert!((m[2] - 97.4).abs() < 0.3, "{}", m[2]);
    let m = manual(&t(|t| t.x = 10.0), 1000.0, 1000.0);
    assert!((m[2] - 63.6).abs() < 0.3, "{}", m[2]);
    // Offset Y moves up.
    assert!(manual(&t(|t| t.y = 10.0), 1200.0, 800.0)[5] < 0.0);
    // Vertical 50: c · (half diagonal) = −0.3 at every size.
    for (w, h) in [(600.0, 400.0), (1500.0, 500.0), (2000.0, 2000.0)] {
        let m = manual(&t(|t| t.vertical = 50.0), w, h);
        assert!((m[7] * f64::hypot(w, h) / 2.0 + 0.3).abs() < 1e-9);
        assert!((m[4] / m[0] - 1.1078).abs() < 1e-3);
    }
}

#[test]
fn stretch_is_monotone_and_safe() {
    let mut last = 0.0;
    for k in 0..=100 {
        let s = stretch(k as f64);
        assert!(s >= last && s >= 1.0);
        last = s;
    }
    assert_eq!(stretch(f64::NAN), 1.0);
    assert!(stretch(1e9).is_finite());
    assert_eq!(stretch(-50.0), stretch(50.0));
}

#[test]
fn constrain_crop_covers_the_frame() {
    let (w, h) = (1200.0, 800.0);
    let m = manual(&t(|t| t.rotate = 10.0), w, h);
    let k = fill_factor(&m, w, h).expect("fits");
    assert!(k < 1.0 && k > 0.6, "{k}");
    let c = constrain(&m, w, h);
    assert!((fill_factor(&c, w, h).unwrap_or(0.0) - 1.0).abs() < 1e-6);
    // Already covering: unchanged.
    let big = manual(&t(|t| t.scale = 150.0), w, h);
    assert_eq!(constrain(&big, w, h), big);
    // A degenerate matrix leaves things alone instead of failing.
    let zero = [0.0; 9];
    assert_eq!(constrain(&zero, w, h), zero);
    assert!(fill_factor(&[f64::NAN; 9], w, h).is_none());
}

fn image(w: usize, h: usize, f: impl Fn(f64, f64) -> f32) -> Vec<[f32; 4]> {
    (0..w * h)
        .map(|i| {
            let v = f((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
            [v, v, v, 1.0]
        })
        .collect()
}

#[test]
fn warp_by_identity_keeps_pixels() {
    let (w, h) = (40, 30);
    let src = image(w, h, |x, y| ((x * 0.37 + y * 0.11).sin() * 0.5 + 0.5) as f32);
    let mut px = src.clone();
    warp(&mut px, w, h, &IDENTITY, false);
    for (a, b) in px.iter().zip(&src) {
        for c in 0..4 {
            assert!((a[c] - b[c]).abs() < 1e-5);
        }
    }
}

#[test]
fn warp_leaves_uncovered_pixels_transparent() {
    let (w, h) = (120, 80);
    let mut px = image(w, h, |_, _| 0.5);
    warp(&mut px, w, h, &manual(&t(|t| t.rotate = 10.0), w as f64, h as f64), false);
    assert_eq!(px[0][3], 0.0, "corner");
    let mid = px[(h / 2) * w + w / 2];
    assert!((mid[3] - 1.0).abs() < 1e-5 && (mid[0] - 0.5).abs() < 1e-4);
    // Scale 50 shrinks: the edge band is empty, the middle keeps its grey.
    let mut px = image(w, h, |_, _| 0.25);
    warp(&mut px, w, h, &manual(&t(|t| t.scale = 50.0), w as f64, h as f64), false);
    assert_eq!(px[5 * w + 5][3], 0.0);
    assert!((px[(h / 2) * w + w / 2][0] - 0.25).abs() < 1e-4);
}

#[test]
fn warp_survives_hostile_input() {
    let mut px = vec![[0.5f32; 4]; 10];
    warp(&mut px, 100, 100, &IDENTITY, false); // too few pixels: untouched
    assert_eq!(px[0], [0.5; 4]);
    let mut px = vec![[f32::NAN, 0.2, 0.3, 1.0]; 16 * 16];
    warp(&mut px, 16, 16, &manual(&t(|t| t.rotate = 5.0), 16.0, 16.0), true);
    let mut px = vec![[0.5f32; 4]; 16 * 16];
    warp(&mut px, 16, 16, &[0.0; 9], false); // singular: untouched
    assert_eq!(px[0], [0.5; 4]);
    warp(&mut px, 16, 16, &[f64::NAN; 9], false);
    warp(&mut [], 0, 0, &IDENTITY, false);
}

#[test]
fn transform_of_hostile_settings_is_clamped() {
    let p = CameraRaw {
        perspective_vertical: f32::NAN,
        perspective_horizontal: f32::INFINITY,
        perspective_rotate: 1e9,
        perspective_scale: -5.0,
        perspective_x: -1e30,
        ..CameraRaw::default()
    };
    let tr = Transform::of(&p);
    assert_eq!(tr.vertical, 0.0);
    assert_eq!(tr.horizontal, 0.0);
    assert_eq!(tr.rotate, 10.0);
    assert_eq!(tr.scale, 50.0);
    assert_eq!(tr.x, -100.0);
    assert!(manual(&tr, 100.0, 100.0).iter().all(|v| v.is_finite()));
}

#[test]
fn upright_modes_parse() {
    for m in [Upright::Off, Upright::Auto, Upright::Level, Upright::Vertical, Upright::Full, Upright::Guided] {
        assert_eq!(Upright::parse(m.id()), Some(m));
    }
    assert_eq!(Upright::parse(" FULL "), Some(Upright::Full));
    assert_eq!(Upright::parse(""), Some(Upright::Off));
    assert_eq!(Upright::parse("sideways"), None);
}

/// Segments of a facade: plumb verticals and level horizontals, as seen through `m`.
fn facade(m: &[f64; 9]) -> Vec<Segment> {
    let mut s = Vec::new();
    for i in -4..=4 {
        let x = i as f64 * 120.0;
        s.push(Segment { a: pt(m, x, -300.0), b: pt(m, x, 300.0) });
    }
    for j in -3..=3 {
        let y = j as f64 * 90.0;
        s.push(Segment { a: pt(m, -450.0, y), b: pt(m, 450.0, y) });
    }
    s
}

fn plumb_and_level(u: &[f64; 9], m: &[f64; 9]) -> (f64, f64) {
    let total = mul(u, m);
    let mut worst_v: f64 = 0.0;
    let mut worst_h: f64 = 0.0;
    for i in -4..=4 {
        let x = i as f64 * 120.0;
        let (a, b) = (pt(&total, x, -300.0), pt(&total, x, 300.0));
        let v = ((b[0] - a[0]) / (b[1] - a[1])).atan().to_degrees().abs();
        // `f64::max` drops a NaN, which would let a broken matrix pass.
        assert!(v.is_finite(), "non-finite vertical angle");
        worst_v = worst_v.max(v);
    }
    for j in -3..=3 {
        let y = j as f64 * 90.0;
        let (a, b) = (pt(&total, -450.0, y), pt(&total, 450.0, y));
        let hz = ((b[1] - a[1]) / (b[0] - a[0])).atan().to_degrees().abs();
        assert!(hz.is_finite(), "non-finite horizontal angle");
        worst_h = worst_h.max(hz);
    }
    (worst_v, worst_h)
}

#[test]
fn vertical_upright_undoes_a_tilted_camera() {
    let (w, h) = (1200.0, 800.0);
    let tilted = manual(
        &t(|t| {
            t.vertical = -35.0;
            t.rotate = 4.0
        }),
        w,
        h,
    );
    let u = homography_from_segments(&facade(&tilted), w, h, Upright::Vertical).expect("a correction");
    let (v, _) = plumb_and_level(&u, &tilted);
    assert!(v < 0.05, "verticals off by {v}°");
}

#[test]
fn full_upright_makes_verticals_plumb_and_horizontals_level() {
    let (w, h) = (1200.0, 800.0);
    let tilted = manual(
        &t(|t| {
            t.vertical = 25.0;
            t.horizontal = -30.0;
            t.rotate = -2.0
        }),
        w,
        h,
    );
    let u = homography_from_segments(&facade(&tilted), w, h, Upright::Full).expect("a correction");
    let (v, hz) = plumb_and_level(&u, &tilted);
    assert!(v < 0.05 && hz < 0.05, "{v}° / {hz}°");
}

#[test]
fn level_upright_only_rotates() {
    let (w, h) = (1200.0, 800.0);
    let turned = manual(&t(|t| t.rotate = 3.0), w, h);
    let a = level_angle(&facade(&turned)).expect("an angle");
    assert!((a + 3.0).abs() < 0.05, "{a}");
    let u = homography_from_segments(&facade(&turned), w, h, Upright::Level).expect("a rotation");
    assert!(u[6].abs() < 1e-12 && u[7].abs() < 1e-12);
}

#[test]
fn upright_finds_lines_in_pixels() {
    // A window grid photographed with the camera tilted up: Vertical straightens it.
    let (w, h) = (480usize, 360usize);
    let tilted = manual(&t(|t| t.vertical = -40.0), w as f64, h as f64);
    let mut px = image(w, h, |x, y| if ((x / 40.0).floor() as i64 + (y / 40.0).floor() as i64) % 2 == 0 { 0.85 } else { 0.15 });
    warp(&mut px, w, h, &tilted, false);
    let u = upright::analyse(&px, w, h, Upright::Vertical).expect("lines found");
    let (v, _) = plumb_and_level(&u, &tilted);
    assert!(v < 0.6, "verticals off by {v}°");
}

#[test]
fn guided_upright_follows_the_guides() {
    let (w, h) = (1000.0, 800.0);
    let tilted = manual(&t(|t| t.vertical = 30.0), w, h);
    // Two verticals of the facade, in 0 … 1 image coordinates.
    let g = |x: f64| -> [[f32; 2]; 2] {
        let (a, b) = (pt(&tilted, x, -300.0), pt(&tilted, x, 300.0));
        [[(a[0] / w + 0.5) as f32, (a[1] / h + 0.5) as f32], [(b[0] / w + 0.5) as f32, (b[1] / h + 0.5) as f32]]
    };
    let u = guided(&[g(-300.0), g(300.0)], w, h).expect("a correction");
    let (v, _) = plumb_and_level(&u, &tilted);
    assert!(v < 0.05, "{v}°");
    // Nothing usable: no correction, no panic.
    assert!(guided(&[], w, h).is_none());
    assert!(guided(&[[[f32::NAN, 0.0], [0.5, 0.5]]], w, h).is_none());
    assert!(guided(&[[[0.5, 0.5], [0.5, 0.5]]], w, h).is_none());
    // Nine copies of one guide: only four count, and identical lines meet nowhere.
    let _ = guided(&[[[0.0, 0.0], [1.0, 1.0]]; 9], w, h);
    // One guide: a rotation that makes it plumb.
    let one = guided(&[[[0.48, 0.1], [0.52, 0.9]]], w, h).expect("a rotation");
    assert!(one[6].abs() < 1e-12 && one[1].abs() > 0.01);
}

/// Angle (degrees) of guide `g` (0 … 1 image coordinates) from vertical / horizontal after `m`.
fn guide_angle(m: &[f64; 9], g: [[f32; 2]; 2], w: f64, h: f64, vertical: bool) -> f64 {
    let c = |p: [f32; 2]| pt(m, (f64::from(p[0]) - 0.5) * w, (f64::from(p[1]) - 0.5) * h);
    let (a, b) = (c(g[0]), c(g[1]));
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let r = if vertical { (dx / dy).atan() } else { (dy / dx).atan() }.to_degrees().abs();
    assert!(r.is_finite());
    r
}

#[test]
fn guided_levels_a_single_horizontal_next_to_a_single_vertical() {
    let (w, h) = (1000.0, 800.0);
    let (v, hz) = ([[0.5f32, 0.1], [0.5, 0.9]], [[0.1f32, 0.45], [0.9, 0.55]]);
    let u = guided(&[v, hz], w, h).expect("a correction");
    assert!(guide_angle(&u, hz, w, h, false) < 0.05, "{}°", guide_angle(&u, hz, w, h, false));
    assert!(guide_angle(&u, v, w, h, true) < 0.05);
}

#[test]
fn guided_follows_a_guide_tilted_past_detected_uprights_limit() {
    let (w, h) = (1000.0, 800.0);
    let dx = (0.8 * 20f64.to_radians().tan() * h / w) as f32;
    let g = [[0.4f32, 0.1], [0.4 + dx, 0.9]];
    let u = guided(&[g], w, h).expect("a 20° guide is still corrected");
    assert!(guide_angle(&u, g, w, h, true) < 0.05);
    // Detected lines still refuse a 20° roll: they were probably not the scene's verticals.
    let t = 20f64.to_radians();
    let turned = [t.cos(), -t.sin(), 0.0, t.sin(), t.cos(), 0.0, 0.0, 0.0, 1.0];
    assert!(homography_from_segments(&facade(&turned), w, h, Upright::Vertical).is_none());
}

#[test]
fn guided_fits_three_close_guides() {
    let (w, h) = (1000.0, 800.0);
    let tilted = manual(&t(|t| t.vertical = 30.0), w, h);
    let g = |x: f64| -> [[f32; 2]; 2] {
        let (a, b) = (pt(&tilted, x, -300.0), pt(&tilted, x, 300.0));
        [[(a[0] / w + 0.5) as f32, (a[1] / h + 0.5) as f32], [(b[0] / w + 0.5) as f32, (b[1] / h + 0.5) as f32]]
    };
    // Too close together for the detector's spread rule, but the user drew them.
    let u = guided(&[g(-40.0), g(0.0), g(40.0)], w, h).expect("a correction");
    let (v, _) = plumb_and_level(&u, &tilted);
    assert!(v < 0.05, "{v}°");
}

#[test]
fn warp_keeps_colour_at_opaque_edges() {
    // A fractional shift: the Catmull-Rom taps beyond the border are skipped and the remaining
    // weights sum above 1; the colour must not brighten by that factor.
    let (w, h) = (40, 30);
    let mut px = image(w, h, |_, _| 0.5);
    warp(&mut px, w, h, &manual(&t(|t| t.x = 0.3), w as f64, h as f64), true);
    for (i, q) in px.iter().enumerate() {
        if q[3] > 0.5 {
            assert!(q[..3].iter().all(|v| (v - 0.5).abs() < 1e-4), "pixel {i}: {q:?}");
        }
    }
}

#[test]
fn warp_does_not_spread_the_colour_of_transparent_pixels() {
    // A transparent column holding NaN colour (a float layer may store anything there).
    let (w, h) = (40, 30);
    let mut px = image(w, h, |_, _| 0.5);
    for y in 0..h {
        px[y * w + 20] = [f32::NAN, f32::NAN, f32::NAN, 0.0];
    }
    warp(&mut px, w, h, &manual(&t(|t| t.x = 0.3), w as f64, h as f64), true);
    for (i, q) in px.iter().enumerate() {
        assert!(q.iter().all(|v| v.is_finite()), "pixel {i}: {q:?}");
    }
}

#[test]
fn guided_levels_horizontals_when_the_verticals_give_no_correction() {
    // Two vertical-ish guides crossing at the centre's height: their vanishing point lies to the
    // side, which no vertical correction can send straight down. The horizontal guide still counts.
    let (w, h) = (1000.0, 800.0);
    let (a, b) = ([[0.1f32, 0.2], [0.3, 0.8]], [[0.1f32, 0.8], [0.3, 0.2]]);
    let hz = [[0.1f32, 0.45], [0.9, 0.55]];
    let u = guided(&[a, b, hz], w, h).expect("the horizontal guide is levelled");
    assert!(guide_angle(&u, hz, w, h, false) < 0.05, "{}°", guide_angle(&u, hz, w, h, false));
}

#[test]
fn guided_without_guides_is_neutral() {
    // Nothing to correct yet: the canvas-only framing and the transparency-lock refusal that
    // come with a real correction must not apply.
    let mut p = CameraRaw { perspective_upright: "guided".into(), ..CameraRaw::default() };
    assert!(neutral(&p) && p.is_identity());
    p.upright_guides = vec![[[0.48, 0.1], [0.52, 0.9]]];
    assert!(!neutral(&p));
    // An analysing mode may find lines in the pixels: not neutral before it looks.
    assert!(!neutral(&CameraRaw { perspective_upright: "vertical".into(), ..CameraRaw::default() }));
}

#[test]
fn vanishing_point_needs_support() {
    assert!(vanishing_point(&[], 100.0).is_none());
    let one = [Segment { a: [0.0, 0.0], b: [0.0, 10.0] }];
    assert!(vanishing_point(&one, 100.0).is_none());
    let degenerate = [Segment { a: [1.0, 1.0], b: [1.0, 1.0] }; 3];
    assert!(vanishing_point(&degenerate, 100.0).is_none());
}

#[test]
fn apply_runs_every_mode_without_failing() {
    let (w, h) = (96usize, 64usize);
    for mode in ["off", "auto", "level", "vertical", "full", "guided", "nonsense"] {
        let p = CameraRaw {
            perspective_upright: mode.into(),
            upright_guides: vec![[[0.2, 0.1], [0.25, 0.9]], [[0.8, 0.1], [0.75, 0.9]]],
            perspective_vertical: 20.0,
            constrain_crop: true,
            ..CameraRaw::default()
        };
        let mut px = image(w, h, |x, y| if (x as i64 / 12 + y as i64 / 12) % 2 == 0 { 0.8 } else { 0.2 });
        apply(&mut px, w, h, &p, false);
        assert!(px.iter().flatten().all(|v| v.is_finite()));
    }
    assert!(neutral(&CameraRaw::default()));
    assert!(!neutral(&CameraRaw { perspective_rotate: 1.0, ..CameraRaw::default() }));
}

#[test]
fn validate_rejects_hostile_geometry() {
    assert!(CameraRaw::default().validate().is_ok());
    for bad in [
        CameraRaw { perspective_upright: "sideways".into(), ..CameraRaw::default() },
        CameraRaw { upright_guides: vec![[[0.1, 0.1], [0.1, 0.9]]; 5], ..CameraRaw::default() },
        CameraRaw { upright_guides: vec![[[f32::NAN, 0.1], [0.1, 0.9]]], ..CameraRaw::default() },
        CameraRaw { upright_guides: vec![[[1.5, 0.1], [0.1, 0.9]]], ..CameraRaw::default() },
        CameraRaw { perspective_rotate: f32::INFINITY, ..CameraRaw::default() },
    ] {
        assert!(bad.validate().is_err(), "{:?}", bad.perspective_upright);
    }
    assert!(CameraRaw { perspective_upright: "Guided".into(), upright_guides: vec![[[0.1, 0.1], [0.1, 0.9]]], ..CameraRaw::default() }.validate().is_ok());
}

/// Debug aid: `UPRIGHT_RAW=path.f32 UPRIGHT_W=900 UPRIGHT_H=600 cargo test … upright_debug -- --ignored --nocapture`
/// prints the segments and vanishing points the analysis finds in a raw RGBA f32 dump.
#[test]
#[ignore]
fn upright_debug() {
    let (Ok(path), Ok(w), Ok(h)) = (std::env::var("UPRIGHT_RAW"), std::env::var("UPRIGHT_W"), std::env::var("UPRIGHT_H")) else { return };
    let (w, h): (usize, usize) = (w.parse().unwrap(), h.parse().unwrap());
    let bytes = std::fs::read(path).unwrap();
    let px: Vec<[f32; 4]> =
        bytes.as_chunks::<16>().0.iter().map(|c| std::array::from_fn(|k| f32::from_le_bytes([c[4 * k], c[4 * k + 1], c[4 * k + 2], c[4 * k + 3]]))).collect();
    upright::debug(&px, w, h);
}
