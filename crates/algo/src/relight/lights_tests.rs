use super::*;
use crate::{FilterParams, Halo, RelightLightKind, apply, apply_tiled, output_area};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_raster::Surface;

fn fmt(s: SampleType) -> PixelFormat {
    PixelFormat::new(ColorMode::Rgb, s, true)
}

fn point(x: f32, y: f32, intensity: f32) -> RelightLight {
    RelightLight { kind: RelightLightKind::Point, x, y, intensity, ..RelightLight::default() }
}

fn with(lights: Vec<RelightLight>) -> FilterParams {
    FilterParams::Relight { angle: 45.0, elevation: 40.0, intensity: 40.0, ambient: 30.0, warmth: 0.0, softness: 15.0, lights }
}

fn run(s: &Surface, p: &FilterParams, bounds: Rect) -> Surface {
    let area = output_area(p, s.content_bounds(), bounds, None);
    apply(s, p, area, bounds, None)
}

fn luma(px: &[f32]) -> f32 {
    0.2126 * px[0] + 0.7152 * px[1] + 0.0722 * px[2]
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

fn sphere(st: SampleType, size: i32) -> Surface {
    let mut surf = Surface::new(fmt(st));
    let cx = size as f32 / 2.0;
    let rad = size as f32 * 0.42;
    let mut v = Vec::new();
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cx);
            let d2 = dx * dx + dy * dy;
            if d2 <= rad * rad {
                let sh = 0.22 + 0.78 * (rad * rad - d2).max(0.0).sqrt() / rad;
                v.extend_from_slice(&[0.55 * sh, 0.55 * sh, 0.55 * sh, 1.0]);
            } else {
                v.extend_from_slice(&[0.18, 0.18, 0.18, 1.0]);
            }
        }
    }
    surf.write_region(Rect::new(0, 0, size, size), &v);
    surf
}

/// Mean luma over the sphere's disc; `side` picks the left (-1), right (1) or whole (0) disc.
fn sphere_mean(s: &Surface, size: i32, side: i32) -> f32 {
    let cx = size as f32 / 2.0;
    let rad = size as f32 * 0.42;
    let (mut sum, mut n) = (0.0f32, 0.0f32);
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cx);
            if dx * dx + dy * dy > rad * rad || (side < 0 && dx >= -4.0) || (side > 0 && dx < 4.0) {
                continue;
            }
            sum += luma(&s.pixel(x, y));
            n += 1.0;
        }
    }
    sum / n.max(1.0)
}

fn flat(st: SampleType, r: Rect, px: [f32; 4]) -> Surface {
    let mut surf = Surface::new(fmt(st));
    surf.fill_rect(r, &px);
    surf
}

fn field_mean(s: &Surface, r: Rect) -> f32 {
    let v = s.read_region(r);
    let n = v.chunks_exact(4).count().max(1) as f32;
    v.chunks_exact(4).map(luma).sum::<f32>() / n
}

#[test]
fn empty_lights_match_one_explicit_directional_light() {
    let size = 48;
    let b = Rect::new(0, 0, size, size);
    let s = sphere(SampleType::F32, size);
    let legacy = FilterParams::Relight { angle: 120.0, elevation: 35.0, intensity: 70.0, ambient: 30.0, warmth: 0.0, softness: 15.0, lights: vec![] };
    let l = RelightLight { kind: RelightLightKind::Directional, angle: 120.0, elevation: 35.0, intensity: 70.0, ..RelightLight::default() };
    let a = run(&s, &legacy, b).read_region(b);
    let m = run(&s, &with(vec![l]), b).read_region(b);
    assert!(max_diff(&a, &m) <= 1e-5);
}

#[test]
fn point_light_brightens_its_own_side() {
    let size = 64;
    let b = Rect::new(0, 0, size, size);
    let s = sphere(SampleType::F32, size);
    let left = run(&s, &with(vec![point(0.0, 0.5, 90.0)]), b);
    let right = run(&s, &with(vec![point(1.0, 0.5, 90.0)]), b);
    assert!(sphere_mean(&left, size, -1) > sphere_mean(&left, size, 1));
    assert!(sphere_mean(&right, size, 1) > sphere_mean(&right, size, -1));
}

#[test]
fn two_lights_add() {
    let r = Rect::new(0, 0, 32, 32);
    let s = flat(SampleType::F32, r, [0.5, 0.5, 0.5, 1.0]);
    let one = run(&s, &with(vec![point(0.5, 0.5, 30.0)]), r);
    let two = run(&s, &with(vec![point(0.5, 0.5, 30.0), point(0.5, 0.5, 30.0)]), r);
    assert!(field_mean(&two, r) > field_mean(&one, r));
}

#[test]
fn coloured_light_tints() {
    let r = Rect::new(0, 0, 32, 32);
    let s = flat(SampleType::F32, r, [0.5, 0.5, 0.5, 1.0]);
    let red = RelightLight { color: [1.0, 0.0, 0.0], intensity: 80.0, ..point(0.5, 0.5, 80.0) };
    let px = run(&s, &with(vec![red]), r).pixel(16, 16);
    assert!(px[0] > px[2], "{px:?}");
}

#[test]
fn diffusion_lifts_the_dark_side() {
    let size = 64;
    let b = Rect::new(0, 0, size, size);
    let s = sphere(SampleType::F32, size);
    let lamp =
        |d: f32| RelightLight { kind: RelightLightKind::Directional, angle: 180.0, elevation: 20.0, intensity: 90.0, diffusion: d, ..RelightLight::default() };
    let hard = run(&s, &with(vec![lamp(0.0)]), b);
    let soft = run(&s, &with(vec![lamp(100.0)]), b);
    assert!(sphere_mean(&soft, size, 1) > sphere_mean(&hard, size, 1));
}

#[test]
fn zero_intensity_lights_are_identity() {
    let r = Rect::new(0, 0, 24, 16);
    let s = sphere(SampleType::U8, 24);
    let p = with(vec![point(0.2, 0.2, 0.0), point(0.8, 0.8, f32::NAN)]);
    assert_eq!(p.halo(), Halo::Radius(0));
    assert_eq!(p.halo_for(r), Halo::Radius(0));
    assert_eq!(run(&s, &p, r).read_region(r), s.read_region(r));
    assert!(matches!(with(vec![point(0.2, 0.2, 5.0)]).halo(), Halo::Radius(n) if n > 0));
}

#[test]
fn hostile_lights_stay_finite() {
    let r = Rect::new(0, 0, 32, 32);
    let s = sphere(SampleType::F32, 32);
    let nan = RelightLight { x: f32::NAN, y: f32::INFINITY, height: f32::NEG_INFINITY, angle: f32::NAN, color: [f32::NAN, 0.0, 1.0], ..point(0.5, 0.5, 50.0) };
    let inf = RelightLight { intensity: f32::INFINITY, diffusion: f32::NAN, ..RelightLight::default() };
    let wild = RelightLight { x: 1e30, y: -1e30, height: 1e-30, intensity: 1e30, diffusion: 1e30, ..RelightLight::default() };
    let many = vec![nan, inf, wild, point(0.1, 0.1, 50.0), point(0.9, 0.9, 50.0)];
    let out = run(&s, &with(many), r).read_region(r);
    assert!(out.iter().all(|v| v.is_finite()));
}

#[test]
fn tiles_match_a_full_apply_with_a_point_light() {
    let size = 64;
    let b = Rect::new(0, 0, size, size);
    let s = sphere(SampleType::F32, size);
    let p = with(vec![point(0.2, 0.3, 70.0), RelightLight { color: [1.0, 0.7, 0.4], ..point(0.9, 0.8, 50.0) }]);
    let area = output_area(&p, s.content_bounds(), b, None);
    let full = apply_tiled(&s, &p, area, b, None, 256, None);
    let tiled = apply_tiled(&s, &p, area, b, None, 32, None);
    let d = max_diff(&full.read_region(b), &tiled.read_region(b));
    assert!(d <= 1e-4, "tile seam {d}");
}

#[test]
fn lights_work_at_every_depth_and_on_grayscale() {
    let r = Rect::new(0, 0, 24, 24);
    let p = with(vec![point(0.2, 0.3, 70.0)]);
    for st in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut s = sphere(st, 24);
        s.write_pixel(2, 2, &[0.3, 0.3, 0.3, 0.5]);
        let out = run(&s, &p, r);
        assert!(out.read_region(r).iter().all(|v| v.is_finite()), "{st:?}");
        assert!((out.pixel(2, 2)[3] - s.pixel(2, 2)[3]).abs() < 1e-6, "{st:?}");
        let mut g = Surface::new(PixelFormat::new(ColorMode::Grayscale, st, true));
        g.fill_rect(r, &[0.4, 1.0]);
        let go = run(&g, &p, r);
        assert!(go.read_region(r).iter().all(|v| v.is_finite()), "gray {st:?}");
        assert!((go.pixel(5, 5)[1] - 1.0).abs() < 1e-6, "gray alpha {st:?}");
    }
}
