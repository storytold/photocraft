//! #14724: HDR Toning's Highlight works as in the reference application (-100 darkens the
//! highlights, +100 brightens them) without reversing the tone order, and Shadow changes nothing.

use super::checked_hdr::process_with;
use super::{HdrToning, hdr_toning};
use photocraft_raster::Interrupt;

const W: usize = 512;
const H: usize = 256;
const GRAYS: [u8; 8] = [0, 16, 48, 80, 128, 176, 224, 255];
const COLOURS: [[u8; 3]; 8] = [[60, 20, 20], [20, 60, 20], [40, 40, 90], [120, 60, 30], [200, 60, 60], [60, 200, 60], [220, 200, 120], [230, 180, 200]];

/// The issue's 8-bit test document: a gray ramp (column x is floor(x / 2)) on top, eight flat gray
/// patches, a 40 | 200 step edge and eight colour patches below.
fn test_document() -> Vec<[f32; 4]> {
    let level = |v: u8| f32::from(v) / 255.0;
    let mut px = vec![[0.0; 4]; W * H];
    for (i, q) in px.iter_mut().enumerate() {
        let (x, y) = (i % W, i / W);
        let rgb = match y {
            0..64 => [level((x / 2) as u8); 3],
            64..128 => [level(GRAYS[x / 64]); 3],
            128..192 => [level(if x < W / 2 { 40 } else { 200 }); 3],
            _ => COLOURS[x / 64].map(level),
        };
        *q = [rgb[0], rgb[1], rgb[2], 1.0];
    }
    px
}

fn toned(p: &HdrToning) -> Vec<[i32; 3]> {
    let mut px = test_document();
    hdr_toning(&mut px, W, H, p);
    px.iter().map(|q| [q[0], q[1], q[2]].map(|v| (v * 255.0).round() as i32)).collect()
}

/// The level of the gray patch `value` (the middle of its 64 x 64 square).
fn patch(levels: &[[i32; 3]], value: u8) -> i32 {
    let i = GRAYS.iter().position(|g| *g == value).unwrap();
    levels[96 * W + i * 64 + 32][0]
}

fn with(f: impl FnOnce(&mut HdrToning)) -> HdrToning {
    let mut p = HdrToning::default();
    f(&mut p);
    p
}

#[test]
fn highlight_minus_100_darkens_white_and_plus_100_brightens_it() {
    let default = patch(&toned(&HdrToning::default()), 255);
    let minus = patch(&toned(&with(|p| p.highlight = -100.0)), 255);
    let plus = patch(&toned(&with(|p| p.highlight = 100.0)), 255);
    assert!(minus < default, "Highlight -100 gives white {minus}, the default {default}");
    assert!(plus > default, "Highlight +100 gives white {plus}, the default {default}");
}

fn ramp(p: &HdrToning) -> Vec<i32> {
    let levels = toned(p);
    (0..256).map(|v| levels[32 * W + v * 2][0]).collect()
}

#[test]
fn highlight_keeps_the_tone_order_and_never_turns_white_black() {
    // l + c·l³ with c < 0 turns back down past its peak: with Exposure +1 and Highlight -100 the
    // ramp's white came out black. Its shoulder holds the peak instead.
    for exposure in [0.0, 1.0, 1.5, 3.0] {
        for highlight in [-100.0, -50.0, 0.0, 50.0, 100.0] {
            let r = ramp(&with(|p| (p.exposure, p.highlight) = (exposure, highlight)));
            assert!(r.windows(2).all(|w| w[1] >= w[0]), "Exposure {exposure}, Highlight {highlight}: {r:?}");
        }
    }
}

#[test]
fn shadow_changes_nothing() {
    // The reference's Shadow changed neither measured document; HDR data wasn't measured.
    let default = toned(&HdrToning::default());
    for shadow in [-100.0, 100.0] {
        assert!(toned(&with(|p| p.shadow = shadow)) == default, "Shadow {shadow}, 8-bit document");
    }
    let hdr = || (0..64 * 16).map(|i| [(i % 64) as f32 / 16.0; 3]).map(|c| [c[0], c[1], c[2], 1.0]).collect::<Vec<_>>();
    let run = |shadow: f32| {
        let mut px = hdr();
        hdr_toning(&mut px, 64, 16, &with(|p| (p.exposure, p.shadow) = (1.0, shadow)));
        px
    };
    assert_eq!(run(100.0), run(0.0), "Shadow +100 on values above 1.0");
    assert_eq!(run(-100.0), run(0.0), "Shadow -100 on values above 1.0");
}

#[test]
fn the_highlight_curve() {
    let curve = |h: f32, l: f64| super::HdrHighlight::new(h).apply(l);
    for l in [0.0, 0.3, 0.7, 1.0, 2.0] {
        assert_eq!(curve(0.0, l), l, "Highlight 0 leaves {l}");
        assert!((curve(100.0, l) - (l + 0.5 * l.powi(3))).abs() < 1e-12, "+100 at {l}");
    }
    // -100: l - l³/3 up to white (2/3), with a slope that never goes below 0, then white's level.
    assert!((curve(-100.0, 1.0) - 2.0 / 3.0).abs() < 1e-12);
    let samples: Vec<f64> = (0..=1000).map(|i| curve(-100.0, f64::from(i) / 1000.0)).collect();
    assert!(samples.windows(2).all(|w| w[1] >= w[0]), "-100 keeps the order up to white");
    assert!(samples.windows(2).take(999).all(|w| w[1] > w[0]), "-100 keeps distinct highlights apart below white");
    for l in [1.5, 10.0, 1e30, f64::INFINITY] {
        assert_eq!(curve(-100.0, l), curve(-100.0, 1.0), "{l}");
    }
    // A milder darkening keeps values above 1.0 apart up to its own turning point, √2 at -50.
    let (a, b, knee) = (curve(-50.0, 1.2), curve(-50.0, 1.4), curve(-50.0, 2f64.sqrt()));
    assert!(a < b && b < knee && knee < 1.0, "{a} {b} {knee}");
    assert_eq!(curve(-50.0, 3.0), knee);
    assert!(curve(-100.0, f64::NAN).is_nan(), "a NaN stays NaN");
}

#[test]
fn results_stay_in_range_for_every_slider_value() {
    for exposure in [0.0, 2.0] {
        for highlight in [-100.0, -50.0, 0.0, 50.0, 100.0] {
            let mut px = test_document();
            hdr_toning(&mut px, W, H, &with(|p| (p.highlight, p.exposure) = (highlight, exposure)));
            assert!(px.iter().flat_map(|q| &q[..3]).all(|v| v.is_finite() && (0.0..=1.0).contains(v)), "Exposure {exposure}, Highlight {highlight}");
        }
    }
}

#[test]
fn the_checked_path_gives_the_same_result() {
    let ordinary: Vec<[f32; 4]> = (0..72).map(|i| [i as f32 / 71.0, 0.3, 0.6, 1.0]).collect();
    let hdr: Vec<[f32; 4]> = (0..72).map(|i| [i as f32 / 71.0 * 3.0, 0.3, 0.6, 1.0]).collect();
    for input in [ordinary, hdr] {
        for (highlight, exposure, shadow) in [(-100.0, 1.0, 0.0), (100.0, 0.0, 0.0), (-40.0, 1.5, 60.0), (60.0, 0.5, -60.0)] {
            let p = with(|p| (p.radius, p.highlight, p.exposure, p.shadow) = (7.0, highlight, exposure, shadow));
            let mut native = input.clone();
            hdr_toning(&mut native, 9, 8, &p);
            let mut checked = input.clone();
            process_with(&mut checked, 9, 8, &p, &Interrupt::NONE).unwrap();
            for (a, b) in checked.iter().zip(&native) {
                assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6), "Highlight {highlight}, Exposure {exposure}: {a:?} vs {b:?}");
            }
        }
    }
}

#[test]
fn values_past_f32_saturate_and_never_turn_black() {
    // Gamma 0.1 takes these gray values past f32 before Highlight. +100 and 0 make them white;
    // -100 gives white's level, 2/3. Both paths agree.
    let input: Vec<[f32; 4]> = (0..16).map(|i| [1e37 * (i + 1) as f32; 3]).map(|c| [c[0], c[1], c[2], 1.0]).collect();
    for (highlight, want) in [(-100.0, 2.0 / 3.0), (0.0, 1.0), (100.0, 1.0)] {
        let p = with(|p| (p.radius, p.exposure, p.gamma, p.saturation, p.highlight) = (2.0, 5.0, 0.1, 0.0, highlight));
        let mut checked = input.clone();
        process_with(&mut checked, 4, 4, &p, &Interrupt::NONE).unwrap();
        let mut native = input.clone();
        hdr_toning(&mut native, 4, 4, &p);
        for (a, b) in checked.iter().zip(&native) {
            assert!((a[1] - want).abs() < 1e-3, "Highlight {highlight}: {a:?}");
            assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6), "Highlight {highlight}: {a:?} vs {b:?}");
        }
    }
}
