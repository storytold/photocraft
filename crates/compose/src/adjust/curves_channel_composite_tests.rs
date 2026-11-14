//! #14446: in an 8-bit document a Curves channel curve lands on a whole level before the
//! composite (master) curve is applied to it, as Levels' steps do (see [`super::levels_q`]).
//! The values below are the reference application the issue measures, Red `0,0 128,64 255,255`
//! with composite `0,0 128,200 255,255`. 16-bit, float, CMYK and Lab curves are marked *not
//! measured* there and keep today's continuous chain.

use super::*;
use photocraft_geom::Rect;

/// One Photoshop level as a sample.
fn level(v: u32) -> f32 {
    v as f32 / 255.0
}

/// Curve points given in Photoshop's 0..255 units.
fn pts(p: &[(u32, u32)]) -> Vec<CurvePoint> {
    p.iter().map(|&(i, o)| CurvePoint { input: level(i), output: level(o) }).collect()
}

/// The measured pair: a red channel curve that halves the midtones and a composite curve that
/// lifts them.
const RED: &[(u32, u32)] = &[(0, 0), (128, 64), (255, 255)];
const COMPOSITE: &[(u32, u32)] = &[(0, 0), (128, 200), (255, 255)];
const STRAIGHT: &[(u32, u32)] = &[(0, 0), (255, 255)];

fn measured() -> Adjustment {
    Adjustment::Curves {
        master: pts(COMPOSITE),
        per_channel: [pts(RED), pts(STRAIGHT), pts(STRAIGHT)],
        space: ToneSpace::Rgb,
        black: Vec::new(),
        pencil: false,
    }
}

/// One gray through the compositor's real adjustment path in an 8-bit document, as whole levels.
fn gray8(adj: &Adjustment, g: u32) -> [u32; 3] {
    let mut b = Buffer::filled(Rect::new(0, 0, 1, 1), [level(g), level(g), level(g), 1.0]);
    apply_depth(adj, &mut b, Transfer::Srgb, Some(SampleType::U8));
    [0, 1, 2].map(|i| (b.px[0][i] * 255.0 + 0.5).floor() as u32)
}

/// Round half up to a whole level (both for the reference and for the rendered result).
fn whole(v: f32) -> f32 {
    (v * 255.0 + 0.5).floor()
}

/// The measured grays: 128 -> 64 -> 114, so the channel curve's output is a whole level on its
/// way to the composite curve.
#[test]
fn measured_grays_round_the_channel_before_the_composite() {
    let adj = measured();
    for (g, want) in [(64u32, [37, 114, 114]), (128, [114, 200, 200]), (192, [217, 241, 241])] {
        assert_eq!(gray8(&adj, g), want, "gray {g}");
    }
}

/// Every level of an 8-bit ramp: the red output equals the composite curve applied to the channel
/// curve rounded to a whole level, each curve the natural spline through its points.
#[test]
fn the_eight_bit_ramp_is_the_composite_of_the_rounded_channel_curve() {
    let adj = measured();
    let red = curve_samples::<256>(&pts(RED));
    let master = curve_samples::<256>(&pts(COMPOSITE));
    let mut b = Buffer { rect: Rect::new(0, 0, 256, 1), px: (0..256).map(|x| [level(x), level(x), level(x), 1.0]).collect() };
    apply_depth(&adj, &mut b, Transfer::Srgb, Some(SampleType::U8));
    let bad: Vec<String> = (0..256)
        .filter_map(|x| {
            let (got, want) = (whole(b.px[x][0]), whole(master[whole(red[x]) as usize]));
            (got != want).then(|| format!("{x}: {got} vs {want}"))
        })
        .collect();
    assert!(bad.is_empty(), "{} of 256 red levels differ from composite(round(red(x))): {bad:?}", bad.len());
}

/// Not measured, so unchanged: at 16 bits, in float documents and without a depth the channel
/// LUT still goes into the composite LUT unrounded (an 8-bit level is 1/255, far above the
/// tolerance below).
#[test]
fn other_depths_keep_the_continuous_chain() {
    let adj = measured();
    let master_lut = curve_lut_mode(&pts(COMPOSITE), false);
    let continuous = |points: &[CurvePoint]| -> Vec<f32> { curve_lut_mode(points, false).iter().map(|&v| lut(&master_lut, v)).collect() };
    let rows = [continuous(&pts(RED)), continuous(&pts(STRAIGHT)), continuous(&pts(STRAIGHT)), continuous(&[])];
    for depth in [None, Some(SampleType::U16), Some(SampleType::F32)] {
        let built = tone_luts_depth(&adj, depth);
        for ch in 0..4 {
            let drift = built[ch].iter().zip(&rows[ch]).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            assert!(drift < 1e-6, "{depth:?} channel {ch}: {drift} off the continuous chain, a whole level is {}", 1.0 / 255.0);
        }
    }
    let eight = tone_luts_depth(&adj, Some(SampleType::U8));
    let stepped = eight[0].iter().zip(&rows[0]).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(stepped > 1e-4, "the 8-bit row must be the rounded chain, and is {stepped} off the unrounded one");
}

/// Also not measured: CMYK and Lab curves keep today's chain at every depth.
#[test]
fn cmyk_and_lab_curves_keep_the_continuous_chain() {
    let straight = pts(STRAIGHT);
    let black = pts(RED);
    for space in [ToneSpace::Cmyk, ToneSpace::Lab] {
        let mut adj = measured();
        if let Adjustment::Curves { space: s, black: b, .. } = &mut adj {
            *s = space;
            *b = black.clone();
        }
        let master_lut = if space == ToneSpace::Lab { curve_lut(&[]) } else { curve_lut_mode(&pts(COMPOSITE), false) };
        let continuous = |points: &[CurvePoint]| -> Vec<f32> { curve_lut_mode(points, false).iter().map(|&v| lut(&master_lut, v)).collect() };
        // The fourth row is the black ink only in a CMYK curve; otherwise it is the unused identity.
        let ink: &[CurvePoint] = if space == ToneSpace::Cmyk { &black } else { &[] };
        let want = [continuous(&pts(RED)), continuous(&straight), continuous(&straight), continuous(ink)];
        for depth in [Some(SampleType::U8), Some(SampleType::U16), None] {
            let built = tone_luts_depth(&adj, depth);
            for ch in 0..4 {
                let drift = built[ch].iter().zip(&want[ch]).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
                assert!(drift < 1e-6, "{space:?} {depth:?} channel {ch}: {drift} off the continuous chain");
            }
        }
    }
}

/// Listing a curve's points in another order describes the same curve, and the rounding follows
/// the curve, not the order its points were typed in.
#[test]
fn the_point_order_of_each_curve_does_not_change_the_result() {
    let mut adj = measured();
    if let Adjustment::Curves { master, per_channel, .. } = &mut adj {
        master.reverse();
        for c in per_channel.iter_mut() {
            c.reverse();
        }
    }
    for g in [64u32, 128, 192] {
        assert_eq!(gray8(&adj, g), gray8(&measured(), g), "gray {g} with the points listed in reverse");
    }
}
