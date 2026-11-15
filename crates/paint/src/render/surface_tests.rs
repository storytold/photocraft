//! Surface Color (shading in the composite) and Follow Perspective (dabs on a plane).

use super::*;
use crate::dynamics::DabGenerator;
use crate::surface::Local;
use photocraft_geom::Point;

struct Constant(f32);

impl ShadeSource for Constant {
    fn shade(&self, _rect: Rect, out: &mut [f32]) {
        out.fill(self.0);
    }
}

/// Dark on even columns, light on odd ones.
struct Stripes;

impl ShadeSource for Stripes {
    fn shade(&self, rect: Rect, out: &mut [f32]) {
        let w = rect.width().max(1) as usize;
        for (i, o) in out.iter_mut().enumerate() {
            let x = rect.x0 + (i % w) as i32;
            *o = if x % 2 == 0 { 0.5 } else { 1.5 };
        }
    }
}

fn paint(fmt: PixelFormat, shade: Option<Arc<dyn ShadeSource>>) -> Surface {
    let b = BrushSettings { size: 30.0, hardness: 1.0, color: [0.5, 0.5, 0.5, 1.0], ..BrushSettings::default() };
    let mut r = StrokeRenderer::new(&b, Some(fmt), 1.0).with_shade(shade, 1.0);
    r.push(&[StrokePoint::new(40.0, 40.0, 1.0), StrokePoint::new(60.0, 40.0, 1.0)]);
    r.finish();
    let pre = Surface::new(fmt);
    let mut target = pre.clone();
    r.composite(&pre, &mut target, None, false, true);
    target
}

fn formats() -> [(PixelFormat, f32); 3] {
    [(PixelFormat::RGBA8, 0.01), (PixelFormat::RGBA16, 0.002), (PixelFormat::RGBA32F, 1e-5)]
}

#[test]
fn a_constant_one_shade_is_pixel_identical_to_no_shade() {
    let area = Rect::new(0, 0, 100, 80);
    for (fmt, _) in formats() {
        let plain = paint(fmt, None);
        let shaded = paint(fmt, Some(Arc::new(Constant(1.0))));
        let (a, b) = (plain.read_region(area), shaded.read_region(area));
        assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()), "{fmt:?}");
    }
}

#[test]
fn a_striped_shade_gives_a_striped_stroke_at_every_depth() {
    for (fmt, tol) in formats() {
        let t = paint(fmt, Some(Arc::new(Stripes)));
        let (dark, light) = (t.rgba(50, 40), t.rgba(51, 40));
        for c in 0..3 {
            assert!((dark[c] - 0.25).abs() <= tol, "{fmt:?} dark {dark:?}");
            assert!((light[c] - 0.75).abs() <= tol, "{fmt:?} light {light:?}");
        }
        assert!((dark[3] - 1.0).abs() <= tol && (light[3] - 1.0).abs() <= tol, "{fmt:?}");
    }
}

#[test]
fn shade_above_one_clamps_in_integer_formats_and_exceeds_one_in_float() {
    let b = BrushSettings { size: 30.0, color: [0.8, 0.8, 0.8, 1.0], ..BrushSettings::default() };
    for (fmt, hdr) in [(PixelFormat::RGBA8, false), (PixelFormat::RGBA32F, true)] {
        let mut r = StrokeRenderer::new(&b, Some(fmt), 1.0).with_shade(Some(Arc::new(Constant(2.0))), 1.0);
        r.push(&[StrokePoint::new(40.0, 40.0, 1.0)]);
        r.finish();
        let pre = Surface::new(fmt);
        let mut t = pre.clone();
        r.composite(&pre, &mut t, None, false, true);
        let v = t.rgba(40, 40)[0];
        if hdr {
            assert!((v - 1.6).abs() < 1e-5, "{v}");
        } else {
            assert!((v - 1.0).abs() < 0.01, "{v}");
        }
    }
}

#[test]
fn shading_is_not_applied_when_erasing_and_strength_zero_is_off() {
    let fmt = PixelFormat::RGBA32F;
    let mut base = Surface::new(fmt);
    base.write_region(Rect::new(0, 0, 100, 80), &vec![0.5; 100 * 80 * 4]);
    let erase = BrushSettings { size: 30.0, erase: true, ..BrushSettings::default() };
    let run = |shade: Option<Arc<dyn ShadeSource>>, strength: f32| {
        let mut r = StrokeRenderer::new(&erase, Some(fmt), 1.0).with_shade(shade, strength);
        r.push(&[StrokePoint::new(40.0, 40.0, 1.0)]);
        r.finish();
        let mut t = base.clone();
        r.composite(&base, &mut t, None, false, true);
        t.read_region(Rect::new(0, 0, 100, 80))
    };
    assert_eq!(run(None, 1.0), run(Some(Arc::new(Stripes)), 1.0));
    let paint = BrushSettings { size: 30.0, color: [0.4, 0.4, 0.4, 1.0], ..BrushSettings::default() };
    let run = |shade: Option<Arc<dyn ShadeSource>>, strength: f32| {
        let mut r = StrokeRenderer::new(&paint, Some(fmt), 1.0).with_shade(shade, strength);
        r.push(&[StrokePoint::new(40.0, 40.0, 1.0)]);
        r.finish();
        let mut t = base.clone();
        r.composite(&base, &mut t, None, false, true);
        t.read_region(Rect::new(0, 0, 100, 80))
    };
    assert_eq!(run(None, 1.0), run(Some(Arc::new(Stripes)), 0.0));
    assert_eq!(run(None, 1.0), run(Some(Arc::new(Stripes)), f32::NAN));
    assert_ne!(run(None, 1.0), run(Some(Arc::new(Stripes)), 1.0));
}

#[test]
fn shading_works_in_other_blend_modes_and_grayscale() {
    let gray = PixelFormat::new(ColorMode::Grayscale, SampleType::U16, true);
    let t = paint(gray, Some(Arc::new(Stripes)));
    assert!(t.rgba(50, 40)[0] < t.rgba(51, 40)[0]);
    let b = BrushSettings { size: 30.0, mode: BlendMode::Multiply, color: [0.5, 0.5, 0.5, 1.0], ..BrushSettings::default() };
    let fmt = PixelFormat::RGBA32F;
    let mut base = Surface::new(fmt);
    base.write_region(Rect::new(0, 0, 100, 80), &vec![1.0; 100 * 80 * 4]);
    let mut r = StrokeRenderer::new(&b, Some(fmt), 1.0).with_shade(Some(Arc::new(Stripes)), 1.0);
    r.push(&[StrokePoint::new(40.0, 40.0, 1.0)]);
    r.finish();
    let mut t = base.clone();
    r.composite(&base, &mut t, None, false, true);
    // Multiply over white is the colour itself: 0.5 * S.
    assert!((t.rgba(40, 40)[0] - 0.25).abs() < 1e-4);
    assert!((t.rgba(41, 40)[0] - 0.75).abs() < 1e-4);
}

/// Scale falls from about 2 at x = 0 to 1 at x = 200; squashed along 0.3 rad. Nothing past x = 400.
struct Receding;

impl SurfaceMap for Receding {
    fn local(&self, p: Point) -> Option<Local> {
        (p.x >= 0.0 && p.x < 400.0).then(|| Local { scale: (2.0 - p.x / 200.0) as f32, squash_angle: 0.3, squash: 0.5, offset: [0.0; 2] })
    }
}

struct OffPlane;

impl SurfaceMap for OffPlane {
    fn local(&self, _p: Point) -> Option<Local> {
        None
    }
}

fn dabs_of(brush: &BrushSettings, pts: &[StrokePoint], map: Option<Arc<dyn SurfaceMap>>) -> Vec<Dab> {
    let mut g = DabGenerator::new(brush, 1.0).with_surface(map);
    let (mut out, mut dual) = (Vec::new(), Vec::new());
    g.push(pts, &mut out, &mut dual);
    g.finish(&mut out, &mut dual);
    out
}

fn line() -> Vec<StrokePoint> {
    vec![StrokePoint::new(10.0, 50.0, 1.0), StrokePoint::new(190.0, 50.0, 1.0)]
}

#[test]
fn no_map_or_an_off_plane_map_gives_the_dabs_of_today() {
    let b = BrushSettings { size: 20.0, ..BrushSettings::default() };
    let plain = dabs_of(&b, &line(), None);
    assert!(plain.len() > 10);
    assert_eq!(plain, dabs_of(&b, &line(), Some(Arc::new(OffPlane))));
    assert_eq!(plain, crate::dabs(&crate::Stroke { brush: b, points: line() }));
}

#[test]
fn a_receding_plane_shrinks_and_squashes_the_dabs_from_the_first_one() {
    let b = BrushSettings { size: 20.0, ..BrushSettings::default() };
    let plain = dabs_of(&b, &line(), None);
    let on = dabs_of(&b, &line(), Some(Arc::new(Receding)));
    // The first dab keeps the brush size; the rest shrink as the plane recedes.
    assert_eq!(on[0].radius, 10.0);
    assert!(on.windows(2).all(|w| w[1].radius <= w[0].radius), "radii must not grow");
    let last = on.last().map_or(0.0, |d| d.radius);
    assert!(last < 6.0, "{last}");
    assert!(on.iter().all(|d| d.proj_angle == 0.3 && d.proj_scale == 0.5));
    // Spacing follows the dab size: smaller dabs sit closer, so the stroke has more of them.
    assert!(on.len() > plain.len(), "{} vs {}", on.len(), plain.len());
}

#[test]
fn only_the_on_plane_part_of_a_stroke_is_projected() {
    let b = BrushSettings { size: 20.0, ..BrushSettings::default() };
    let pts = [StrokePoint::new(300.0, 50.0, 1.0), StrokePoint::new(500.0, 50.0, 1.0)];
    let on = dabs_of(&b, &pts, Some(Arc::new(Receding)));
    let (inside, outside): (Vec<_>, Vec<_>) = on.iter().partition(|d| d.center.x < 400.0);
    assert!(!inside.is_empty() && !outside.is_empty());
    assert!(inside.iter().all(|d: &&Dab| d.proj_scale == 0.5));
    assert!(outside.iter().all(|d| d.radius == 10.0 && d.proj_scale == 1.0 && d.proj_angle == 0.0));
}

#[test]
fn the_renderer_threads_the_map_through_to_its_dabs() {
    let b = BrushSettings { size: 20.0, ..BrushSettings::default() };
    let mut r = StrokeRenderer::new(&b, None, 1.0).with_surface(Some(Arc::new(Receding))).record_dabs();
    r.push(&line());
    r.finish();
    let expected = dabs_of(&b, &line(), Some(Arc::new(Receding)));
    assert_eq!(r.dabs(), &expected[..]);
}

fn coverage_of(ctx: &BrushContext, d: &Dab) -> (Rect, Vec<f32>) {
    let rect = ctx.dab_rect(d, false);
    let mut cov = Vec::new();
    ctx.rasterize(d, false, rect, &mut cov);
    (rect, cov)
}

#[test]
fn a_circular_footprint_paints_the_plain_round_dab() {
    for hardness in [0.0, 0.5, 1.0] {
        let ctx = BrushContext::new(&BrushSettings { hardness, ..BrushSettings::default() });
        let plain = Dab::round(Point::new(30.25, 30.75), 12.0, 0.8);
        let mut foot = plain;
        foot.footprint = Some(crate::surface::Footprint { rays: [12.0; 32] });
        // The footprint ignores the projection a plane would give.
        foot.proj_scale = 0.4;
        let ((ra, a), (rb, b)) = (coverage_of(&ctx, &plain), coverage_of(&ctx, &foot));
        assert_eq!(ra, rb);
        assert!(a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-5), "hardness {hardness}");
    }
}

#[test]
fn a_footprint_cuts_the_dab_to_its_outline() {
    let ctx = BrushContext::new(&BrushSettings { hardness: 1.0, ..BrushSettings::default() });
    let mut d = Dab::round(Point::new(30.5, 30.5), 15.0, 1.0);
    // Ray 0 (east, +x) is short; the others reach the full radius.
    let mut rays = [15.0f32; 32];
    rays[0] = 5.0;
    d.footprint = Some(crate::surface::Footprint { rays });
    let (rect, cov) = coverage_of(&ctx, &d);
    let at = |x: i32, y: i32| cov[((y - rect.y0) * rect.width() as i32 + (x - rect.x0)) as usize];
    assert!(at(40, 30) < 0.01, "east, 10 px out, is past the short ray");
    assert!(at(21, 30) > 0.99, "west, 9 px out, is inside");
    assert!(at(33, 30) > 0.99, "east, 3 px out, is inside");
}

#[test]
fn a_footprint_leaves_an_elliptical_tip_alone() {
    // Flat depth gives a circular footprint; it must not turn a flat (roundness 0.2) tip round.
    for angle in [0.0, 40.0] {
        let ctx = BrushContext::new(&BrushSettings { hardness: 1.0, ..BrushSettings::default() });
        let mut plain = Dab::round(Point::new(30.25, 30.75), 12.0, 1.0);
        plain.roundness = 0.2;
        plain.angle = f32::to_radians(angle);
        let mut foot = plain;
        foot.footprint = Some(crate::surface::Footprint { rays: [12.0; 32] });
        let ((ra, a), (rb, b)) = (coverage_of(&ctx, &plain), coverage_of(&ctx, &foot));
        assert_eq!(ra, rb);
        assert!(a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-5), "angle {angle}");
        // And it really is flat: little coverage 8 px above the centre.
        let at = |x: i32, y: i32| a[((y - ra.y0) * ra.width() as i32 + (x - ra.x0)) as usize];
        if angle == 0.0 {
            assert!(at(30, 22) < 0.01 && at(38, 31) > 0.5);
        }
    }
}
