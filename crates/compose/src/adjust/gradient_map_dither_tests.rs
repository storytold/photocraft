use photocraft_color::SampleType;
use photocraft_color::convert::linear_to_srgb;
use photocraft_doc::Adjustment;

use super::{Transfer, apply, apply_depth, bayer4, gradient, lut3d_sample};
use crate::{Buffer, Rect};

const SMOOTH_EASE: [f32; 17] = [0.0, 0.0438, 0.099, 0.1644, 0.2303, 0.2988, 0.3667, 0.433, 0.4974, 0.5642, 0.6302, 0.6985, 0.7656, 0.8333, 0.8945, 0.9517, 1.0];

fn smooth_ease(u: f32) -> f32 {
    let x = u.clamp(0.0, 1.0) * 16.0;
    let i = (x.floor() as usize).min(15);
    let (a, b) = (SMOOTH_EASE[i], SMOOTH_EASE[i + 1]);
    a + (b - a) * (x - i as f32)
}

fn bw_linear_stops() -> Vec<(f32, [f32; 3])> {
    const N: usize = 96;
    (0..=N)
        .map(|k| {
            let t = k as f32 / N as f32;
            let v = linear_to_srgb(t);
            (t, [v, v, v])
        })
        .collect()
}

fn bw_smooth_stops() -> Vec<(f32, [f32; 3])> {
    const N: usize = 96;
    (0..=N)
        .map(|k| {
            let t = k as f32 / N as f32;
            let e = smooth_ease(t);
            let v = linear_to_srgb(e.powi(3));
            (t, [v, v, v])
        })
        .collect()
}

fn ramp_256x6() -> Buffer {
    let mut buf = Buffer::transparent(Rect::new(0, 0, 256, 6));
    for y in 0..6 {
        for x in 0..256 {
            let xf = x as f32 / 255.0;
            let rgb = match y {
                0 => [xf, xf, xf],
                1 => [xf, 0.0, 0.0],
                2 => [0.0, xf, 0.0],
                3 => [0.0, 0.0, xf],
                4 => [1.0, xf, 0.0],
                _ => [xf, (255 - x) as f32 / 255.0, 128.0 / 255.0],
            };
            buf.px[y * 256 + x] = [rgb[0], rgb[1], rgb[2], 1.0];
        }
    }
    buf
}

fn diff_histogram(a: &Buffer, b: &Buffer) -> [usize; 16] {
    let mut counts = [0usize; 16];
    for (pa, pb) in a.px.iter().zip(&b.px) {
        let qa = (pa[0] * 255.0).round() as i32;
        let qb = (pb[0] * 255.0).round() as i32;
        let d = (qb - qa).unsigned_abs() as usize;
        counts[d.min(15)] += 1;
    }
    counts
}

/// A bin is within 10% of its reference count, rounded down, and never by more than 3 of the
/// 1536 pixels. The second bound is what the tail bins can meet: 10% of a bin holding 4 pixels is
/// under half a pixel, so a percentage band there would demand a count no dither reproduces. The
/// two bins that hold the bulk of the pixels are held to the 10% alone.
fn within_reference(got: usize, want: usize) -> bool {
    got.abs_diff(want) <= ((want as f32 * 0.10) as usize).max(3)
}

#[test]
fn ramp_256x6_linear_black_to_white_dither_histogram() {
    let stops = bw_linear_stops();
    let mut no_dither = ramp_256x6();
    let mut with_dither = ramp_256x6();
    apply_depth(
        &Adjustment::GradientMap { stops: stops.clone(), reverse: false, dither: false, source: None },
        &mut no_dither,
        Transfer::Srgb,
        Some(SampleType::U8),
    );
    apply_depth(&Adjustment::GradientMap { stops, reverse: false, dither: true, source: None }, &mut with_dither, Transfer::Srgb, Some(SampleType::U8));

    let hist = diff_histogram(&no_dither, &with_dither);
    // Reference: 0: 700, ±1: 672, ±2: 121, ±3: 23, ±4: 16, ±5: 4, >5: 0.
    assert_eq!(hist[6..].iter().sum::<usize>(), 0, "unexpected >5 diffs: {hist:?}");
    assert!(hist[5] > 0, "expected changes up to ±5, got {hist:?}");
    assert!(within_reference(hist[0], 700), "unchanged count {} vs ~700: {hist:?}", hist[0]);
    assert!(within_reference(hist[1], 672), "±1 count {} vs ~672: {hist:?}", hist[1]);
    assert!(within_reference(hist[2], 121), "±2 count {} vs ~121: {hist:?}", hist[2]);
    assert!(within_reference(hist[3], 23), "±3 count {} vs ~23: {hist:?}", hist[3]);
    assert!(within_reference(hist[4], 16), "±4 count {} vs ~16: {hist:?}", hist[4]);
    assert!(within_reference(hist[5], 4), "±5 count {} vs ~4: {hist:?}", hist[5]);
}

#[test]
fn ramp_256x6_smooth_black_to_white_dither_histogram() {
    let stops = bw_smooth_stops();
    let mut no_dither = ramp_256x6();
    let mut with_dither = ramp_256x6();
    apply_depth(
        &Adjustment::GradientMap { stops: stops.clone(), reverse: false, dither: false, source: None },
        &mut no_dither,
        Transfer::Srgb,
        Some(SampleType::U8),
    );
    apply_depth(&Adjustment::GradientMap { stops, reverse: false, dither: true, source: None }, &mut with_dither, Transfer::Srgb, Some(SampleType::U8));

    let hist = diff_histogram(&no_dither, &with_dither);
    // Reference: 0: 910, ±1: 605, ±2: 21, >2: 0.
    assert_eq!(hist[3..].iter().sum::<usize>(), 0, "Smooth changes must stay within ±2: {hist:?}");
    assert!(within_reference(hist[0], 910), "unchanged count {} vs ~910: {hist:?}", hist[0]);
    assert!(within_reference(hist[1], 605), "±1 count {} vs ~605: {hist:?}", hist[1]);
    assert!(within_reference(hist[2], 21), "±2 count {} vs ~21: {hist:?}", hist[2]);
}

#[test]
fn gray_input_stays_gray_apart_from_rare_one_level_split() {
    let stops = bw_linear_stops();
    let mut buf = Buffer::filled(Rect::new(0, 0, 128, 32), [176.0 / 255.0, 176.0 / 255.0, 176.0 / 255.0, 1.0]);
    apply_depth(&Adjustment::GradientMap { stops, reverse: false, dither: true, source: None }, &mut buf, Transfer::Srgb, Some(SampleType::U8));
    let mut splits = 0usize;
    for p in &buf.px {
        let r = (p[0] * 255.0).round() as i32;
        let g = (p[1] * 255.0).round() as i32;
        let b = (p[2] * 255.0).round() as i32;
        let spread = r.max(g).max(b) - r.min(g).min(b);
        assert!(spread <= 1, "gray split more than 1 level: ({r}, {g}, {b})");
        if spread == 1 {
            splits += 1;
        }
    }
    assert!(splits <= buf.px.len() / 20, "too many gray splits: {splits}/{}", buf.px.len());
}

#[test]
fn gradient_map_dither_is_deterministic_across_tiles() {
    let stops = bw_linear_stops();
    let adj = Adjustment::GradientMap { stops, reverse: false, dither: true, source: None };
    let mut full = ramp_256x6();
    apply_depth(&adj, &mut full, Transfer::Srgb, Some(SampleType::U8));

    let mut tile = Buffer::transparent(Rect::new(37, 2, 37 + 64, 2 + 3));
    for y in 0..3 {
        for x in 0..64 {
            tile.px[y * 64 + x] = ramp_256x6().px[(y + 2) * 256 + (x + 37)];
        }
    }
    apply_depth(&adj, &mut tile, Transfer::Srgb, Some(SampleType::U8));

    for y in 0..3 {
        for x in 0..64 {
            assert_eq!(tile.px[y * 64 + x], full.px[(y + 2) * 256 + (x + 37)]);
        }
    }
}

/// The slope-scaled noise is an 8-bit fix (#14700): 16- and 32-bit documents keep the ordered
/// dither they had, which is what the GPU is lowered to for them.
#[test]
fn u16_and_f32_documents_keep_bayer_dither() {
    let stops = bw_linear_stops();
    for depth in [SampleType::U16, SampleType::F32] {
        let input = ramp_256x6();
        let mut got = ramp_256x6();
        apply_depth(&Adjustment::GradientMap { stops: stops.clone(), reverse: false, dither: true, source: None }, &mut got, Transfer::Srgb, Some(depth));
        for (i, (o, p)) in got.px.iter().zip(&input.px).enumerate() {
            let t = photocraft_color::blend::lum([p[0], p[1], p[2]]);
            let want: [f32; 3] = std::array::from_fn(|ch| (gradient(&stops, t)[ch] + bayer4((i % 256) as i32, (i / 256) as i32) / 255.0).clamp(0.0, 1.0));
            for ch in 0..3 {
                assert!((o[ch] - want[ch]).abs() < 1e-6, "{depth:?} pixel {i} channel {ch}: {} vs {}", o[ch], want[ch]);
            }
        }
    }
}

#[test]
fn color_lookup_dither_keeps_bayer4_behaviour() {
    let lut = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0];
    let adj = Adjustment::ColorLookup { name: String::new(), lut: Some(lut.clone().into()), size: 2, tetrahedral: false, dither: true, source: None };
    let mut buf = Buffer::filled(Rect::new(0, 0, 4, 4), [0.5, 0.5, 0.5, 1.0]);
    apply(&adj, &mut buf);
    let base = lut3d_sample(&lut, 2, [0.5, 0.5, 0.5], false);
    for y in 0..4 {
        for x in 0..4 {
            let d = bayer4(x, y) / 255.0;
            let got = buf.px[(y * 4 + x) as usize];
            for ch in 0..3 {
                assert!((got[ch] - (base[ch] + d).clamp(0.0, 1.0)).abs() < 1e-6);
            }
        }
    }
}
