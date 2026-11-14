//! Tolerance decides where a stroke may start, with the reference application's threshold (#12633).

use super::*;

const W: usize = 80;
const H: usize = 40;
const CH: usize = 4;

fn level(v: u8) -> f32 {
    f32::from(v) / 255.0
}

/// Source: grey on the left half, green on the right half (the measured Setup state).
fn setup_source() -> Vec<f32> {
    image(|x| if x < W / 2 { [128, 128, 128] } else { [40, 160, 60] })
}

/// Current layer: red on the left half (far from grey), 20 levels off the green on the right.
fn current_layer() -> Vec<f32> {
    image(|x| if x < W / 2 { [255, 0, 0] } else { [60, 180, 80] })
}

fn image(rgb: impl Fn(usize) -> [u8; 3]) -> Vec<f32> {
    let mut v = Vec::with_capacity(W * H * CH);
    for _ in 0..H {
        for x in 0..W {
            let [r, g, b] = rgb(x);
            v.extend_from_slice(&[level(r), level(g), level(b), 1.0]);
        }
    }
    v
}

fn dab(x: f32, radius: f32) -> Dab {
    Dab { x, y: H as f32 / 2.0, theta: 0.0, radius, pressure: 1.0 }
}

/// Stamps `dabs` at `tolerance` (a fraction, Tolerance 50 % = 0.5) and returns the new layer.
fn paint(dest: &[f32], dabs: &[Dab], tolerance: f32) -> Vec<f32> {
    let src = setup_source();
    let mut out = dest.to_vec();
    let mut p = StampParams {
        dest: &mut out,
        dest_w: W,
        dest_h: H,
        dest_origin: (0, 0),
        src: &src,
        src_w: W,
        src_h: H,
        src_origin: (0, 0),
        ch: CH,
        dabs,
        hardness: 1.0,
        opacity: 1.0,
        flow: 1.0,
        tolerance,
        tile: 0,
        selection: None,
        lock_transparent: false,
        alpha: Some(3),
    };
    stamp(&mut p).unwrap();
    out
}

/// Pixels changed in columns `x0..x1`.
fn changed(before: &[f32], after: &[f32], x0: usize, x1: usize) -> usize {
    let mut n = 0;
    for y in 0..H {
        for x in x0..x1 {
            let i = (y * W + x) * CH;
            if (0..CH).any(|c| (before[i + c] - after[i + c]).abs() > 1.0e-4) {
                n += 1;
            }
        }
    }
    n
}

const LEFT: f32 = 12.0;
const RIGHT: f32 = 68.0;

#[test]
fn a_small_difference_starts_strokes_at_10_but_not_at_20() {
    let before = current_layer();
    let dabs = [dab(RIGHT, 6.0)];
    assert!(changed(&before, &paint(&before, &dabs, 0.10), 40, 80) > 0, "20 levels off paints at Tolerance 10");
    assert_eq!(changed(&before, &paint(&before, &dabs, 0.20), 0, 80), 0, "20 levels off paints nothing at Tolerance 20");
}

#[test]
fn red_over_grey_starts_strokes_up_to_80_and_nothing_from_90() {
    let before = current_layer();
    let dabs = [dab(LEFT, 6.0)];
    for tolerance in [0.60, 0.80] {
        assert!(changed(&before, &paint(&before, &dabs, tolerance), 0, 40) > 0, "red over grey paints at {tolerance}");
    }
    let both = [dab(LEFT, 6.0), dab(RIGHT, 6.0)];
    for tolerance in [0.90, 0.95, 0.99, 1.0] {
        assert_eq!(changed(&before, &paint(&before, &both, tolerance), 0, 80), 0, "nothing paints at {tolerance}");
    }
}

#[test]
fn tolerance_zero_paints_everywhere() {
    let before = current_layer();
    let after = paint(&before, &[dab(LEFT, 6.0), dab(RIGHT, 6.0)], 0.0);
    assert!(changed(&before, &after, 0, 40) > 0);
    assert!(changed(&before, &after, 40, 80) > 0);
    // Even where the layer equals the source, the dab is stamped (nothing visible changes).
    let src = setup_source();
    let mut out = src.clone();
    let d = [dab(LEFT, 6.0)];
    let mut p = StampParams {
        dest: &mut out,
        dest_w: W,
        dest_h: H,
        dest_origin: (0, 0),
        src: &src,
        src_w: W,
        src_h: H,
        src_origin: (0, 0),
        ch: CH,
        dabs: &d,
        hardness: 1.0,
        opacity: 1.0,
        flow: 1.0,
        tolerance: 0.0,
        tile: 0,
        selection: None,
        lock_transparent: false,
        alpha: Some(3),
    };
    assert!(stamp(&mut p).unwrap() > 0);
}

/// Several dabs across both halves at Tolerance 20: only those starting on the red half paint, and
/// the right half changes only where a red-half dab reaches into it.
#[test]
fn at_20_strokes_start_only_on_the_large_difference() {
    let before = current_layer();
    let dabs: Vec<Dab> = [8.0, 20.0, 36.0, 52.0, 64.0, 76.0].iter().map(|&x| dab(x, 6.0)).collect();
    let after = paint(&before, &dabs, 0.20);
    assert!(changed(&before, &after, 0, 40) > 0);
    assert_eq!(changed(&before, &after, 44, 80), 0, "no dab starts on the 20-level half");
    assert!(changed(&before, &after, 40, 44) > 0, "the dab at x 36 runs into the right half");
}

/// The rule is where a stroke starts, not the mean under the whole dab: a big dab that starts just
/// inside the red half paints over the right half too, and one that starts just inside the right
/// half paints nothing although a large part of it covers red.
#[test]
fn the_start_decides_not_the_whole_dab() {
    let before = current_layer();
    let started_on_red = paint(&before, &[dab(37.0, 14.0)], 0.80);
    assert!(changed(&before, &started_on_red, 40, 80) > 0, "a stroke started on red runs into the green half");
    let started_on_green = paint(&before, &[dab(43.0, 14.0)], 0.20);
    assert_eq!(changed(&before, &started_on_green, 0, 80), 0, "a stroke started on the 20-level half stays unpainted");
}
