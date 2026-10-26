use crate::dynamics::DabGenerator;
use crate::interpolate::StrokeInterpolator;
use crate::{BrushSettings, ColorDynamics, Dab, Dynamic, Scattering, Smoothing, Stroke, StrokePoint, StrokeRenderer, dabs, render_stroke};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

fn arc() -> Vec<StrokePoint> {
    (0..=6)
        .map(|i| {
            let angle = f64::from(i) * std::f64::consts::PI / 6.0;
            StrokePoint { time: f64::from(i) * 16.0, ..StrokePoint::new(128.0 + 100.0 * angle.cos(), 128.0 + 100.0 * angle.sin(), 0.2 + i as f32 * 0.1) }
        })
        .collect()
}

fn path(pts: &[StrokePoint]) -> Vec<StrokePoint> {
    let mut curve = StrokeInterpolator::default();
    let mut out = Vec::new();
    for p in pts {
        curve.push(*p, &mut out);
    }
    curve.finish(&mut out);
    out
}

fn brush() -> BrushSettings {
    BrushSettings { size: 3.0, pressure_size: false, spacing: 0.1, ..Default::default() }
}

#[test]
fn sparse_fast_arc_is_curved_even_with_smoothing_off() {
    let pts = arc();
    let out = path(&pts);
    assert_eq!(out.first(), pts.first());
    assert_eq!(out.last(), pts.last());
    assert!(out.len() > pts.len() * 3, "curvature adds geometry, rather than more dabs on chords");
    let generated = dabs(&Stroke { brush: brush(), points: pts });
    let error = generated.iter().map(|d| ((d.center.x - 128.0).hypot(d.center.y - 128.0) - 100.0).abs()).fold(0.0_f64, f64::max);
    assert!(error < 0.5, "sparse semicircle's radial error {error} px (straight chords were 3.407 px)");
}

#[test]
fn straight_lines_corners_reversals_and_taps_keep_their_geometry() {
    for pts in [
        vec![StrokePoint::new(2.0, 3.0, 0.4)],
        vec![StrokePoint::new(2.0, 3.0, 0.4), StrokePoint::new(99.0, 31.0, 0.9)],
        vec![StrokePoint::new(0.0, 0.0, 1.0), StrokePoint::new(40.0, 0.0, 1.0), StrokePoint::new(40.0, 40.0, 1.0)],
        vec![StrokePoint::new(0.0, 0.0, 1.0), StrokePoint::new(40.0, 0.0, 1.0), StrokePoint::new(0.0, 0.0, 1.0)],
        vec![StrokePoint::new(0.0, 0.0, 1.0), StrokePoint::new(1.0, 0.0, 1.0), StrokePoint::new(20.0, 0.0, 1.0)],
    ] {
        assert_eq!(path(&pts), pts);
    }
}

#[test]
fn curved_path_keeps_sensor_ranges_times_and_short_rotation() {
    let pts: Vec<_> = arc()
        .into_iter()
        .enumerate()
        .map(|(i, p)| StrokePoint { tilt_x: i as f32 * 5.0, tilt_y: -(i as f32) * 3.0, rotation: if i == 0 { 359.0 } else { 1.0 }, wheel: i as f32 / 6.0, ..p })
        .collect();
    let out = path(&pts);
    assert!(out.windows(2).all(|w| w[0].time <= w[1].time));
    for p in &out {
        assert!((0.2..=0.81).contains(&p.pressure));
        assert!((0.0..=30.0).contains(&p.tilt_x) && (-18.0..=0.0).contains(&p.tilt_y));
        assert!((0.0..=1.0).contains(&p.wheel));
        assert!(p.rotation >= 359.0 || p.rotation <= 1.0, "barrel rotation takes the short way: {}", p.rotation);
    }
    let mut held = pts.clone();
    let last = *held.last().unwrap();
    held.push(StrokePoint { time: last.time + 20.0, pressure: 0.1, ..last });
    assert_eq!(path(&held).last(), held.last(), "a stationary pressure change is retained");
}

#[test]
fn dab_generation_is_identical_across_batches_and_provisional_finishes() {
    let pts = arc();
    for smoothing in [0.0, 0.1, 0.7] {
        for spacing_enabled in [false, true] {
            let mut b = BrushSettings { smoothing: Smoothing { amount: smoothing, ..Default::default() }, spacing_enabled, seed: 87, ..brush() };
            b.scattering = Scattering { enabled: true, count: 3, scatter: Dynamic::jitter(0.3), ..Default::default() };
            b.color_dynamics = ColorDynamics { enabled: true, hue_jitter: 0.4, ..Default::default() };
            b.dual_brush.enabled = true;
            let generate = |chunk: usize, previews: bool| {
                let mut g = DabGenerator::new(&b, 1.0);
                let (mut primary, mut dual) = (Vec::<Dab>::new(), Vec::<Dab>::new());
                for pts in pts.chunks(chunk) {
                    g.push(pts, &mut primary, &mut dual);
                    if previews {
                        let mut preview = g.clone();
                        preview.finish(&mut Vec::new(), &mut Vec::new());
                    }
                }
                g.finish(&mut primary, &mut dual);
                (primary, dual)
            };
            let expected = generate(pts.len(), false);
            for chunk in 1..=pts.len() {
                assert_eq!(generate(chunk, true), expected, "chunk={chunk}, smoothing={smoothing}, spacing={spacing_enabled}");
            }
        }
    }
}

#[test]
fn curve_tail_preview_matches_finished_pixels_at_every_depth() {
    let pts = arc();
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for amount in [0.0, 0.1, 0.7] {
            let fmt = PixelFormat::new(ColorMode::Rgb, depth, true);
            let b = BrushSettings { opacity: 0.6, flow: 0.4, smoothing: Smoothing { amount, ..Default::default() }, ..brush() };
            let pre = Surface::new(fmt);
            let mut renderer = StrokeRenderer::new(&b, Some(fmt), 1.0);
            for n in 1..=pts.len() {
                renderer.push(&pts[n - 1..n]);
                let mut preview = pre.clone();
                renderer.composite(&pre, &mut preview, None, false, true);
                if let Some(mut tail) = renderer.tail_preview() {
                    tail.composite(&pre, &mut preview, None, false, false);
                }
                let mut expected = pre.clone();
                render_stroke(&mut expected, &b, &pts[..n], None, false, 1.0);
                let region = Rect::new(20, 120, 235, 235);
                assert_eq!(preview.read_region(region), expected.read_region(region), "depth={depth:?}, smoothing={amount}, prefix={n}");
            }
        }
    }
}

#[test]
fn dense_tail_coverage_keeps_older_tiles_inside_its_bounding_rectangle() {
    let b = BrushSettings { size: 16.0, hardness: 0.4, smoothing: Smoothing { amount: 0.1, ..Default::default() }, ..brush() };
    let pts = [(60.0, 50.0), (80.0, 40.0), (110.0, 60.0), (140.0, 45.0), (170.0, 65.0)];
    let mut renderer = StrokeRenderer::new(&b, None, 1.0);
    let mut found_hole = false;
    for (x, y) in pts {
        renderer.push(&[StrokePoint::new(x, y, 1.0)]);
        let mut finished = renderer.clone();
        finished.finish();
        if let Some(mut tail) = renderer.tail_preview() {
            let r = tail.take_dirty_rect();
            let expected = finished.coverage_in(r);
            let merged = renderer.coverage_in_with_tail(r, &tail);
            assert_eq!(merged, expected, "a sparse tail cannot erase older coverage in the holes between its tiles");
            found_hole |= tail.coverage_in(r) != expected;
        }
    }
    assert!(found_hole, "the regression must exercise a tail whose rectangle contains older tiles it does not copy");
}

#[test]
fn sequential_effects_keep_their_immediate_path_and_match_their_commit() {
    let b = brush();
    let pts = arc();
    let mut live = StrokeRenderer::for_sequential_dabs(&b, None, 1.0).record_dabs();
    for chunk in pts.chunks(1) {
        live.push(chunk);
    }
    assert!(live.tail_dabs().is_empty(), "sequential state is never held back for a replaceable curve segment");
    live.finish();
    let mut captured = Vec::new();
    let mut surface = Surface::new(PixelFormat::RGBA32F);
    crate::retouch::apply_dab_stroke(&mut surface, &Stroke { brush: b, points: pts }, None, false, 0, |_, fp| captured.push(fp.dab));
    assert_eq!(live.dabs(), captured);
}

#[test]
fn aliased_pencil_bypasses_curve_reconstruction() {
    let b = BrushSettings { aliased: true, size: 1.0, ..brush() };
    let pts = arc();
    let mut g = DabGenerator::new(&b, 1.0);
    let (mut generated, mut dual) = (Vec::new(), Vec::new());
    g.push(&pts, &mut generated, &mut dual);
    let count = generated.len();
    assert!(count > pts.len());
    g.finish(&mut generated, &mut dual);
    assert_eq!(count, generated.len(), "Pencil has no provisional curve tail");
}

#[test]
fn degenerate_and_uneven_samples_have_bounded_finite_output() {
    let mut pts = vec![StrokePoint::new(0.0, 0.0, 1.0), StrokePoint::new(100.0, 0.0, 1.0), StrokePoint::new(100.00001, 0.00001, 1.0)];
    pts.extend([StrokePoint::new(101.0, 100.0, 1.0), StrokePoint::new(101.0, 100.0, 0.1), StrokePoint::new(1e100, 1e100, 0.1)]);
    let out = path(&pts);
    assert!(out.len() <= pts.len() * 1024);
    assert!(out.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
    assert_eq!(out.last(), pts.last());
    let mut curve = StrokeInterpolator::default();
    for p in [StrokePoint::new(f64::NAN, 0.0, 1.0), StrokePoint::new(f64::INFINITY, 1.0, 1.0), StrokePoint::new(0.0, 0.0, 1.0)] {
        curve.push(p, &mut Vec::new());
    }
    curve.finish(&mut Vec::new());
}
