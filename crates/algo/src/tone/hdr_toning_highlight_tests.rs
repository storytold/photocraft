//! #14724: HDR Toning's Highlight works as in the reference application (-100 darkens the
//! highlights, +100 brightens them), and Shadow leaves ordinary images unchanged while images with
//! values above 1.0 keep today's Shadow.

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

/// The test document toned with `p`, as 8-bit levels per pixel (RGB).
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

#[test]
fn shadow_leaves_the_gray_patches_and_ramp_within_3_levels() {
    let default = toned(&HdrToning::default());
    for shadow in [-100.0, 100.0] {
        let changed = toned(&with(|p| p.shadow = shadow));
        // The ramp (rows 0..64) and the gray patches (rows 64..128).
        for i in (0..128 * W).step_by(7) {
            let d = (changed[i][0] - default[i][0]).abs();
            assert!(d <= 3, "Shadow {shadow}: pixel {i} moved {d} levels ({} -> {})", default[i][0], changed[i][0]);
        }
    }
}

#[test]
fn shadow_still_acts_on_values_above_one() {
    // A high-dynamic-range ramp, 0 to 4: Shadow there keeps today's effect.
    let hdr = || (0..64 * 16).map(|i| [(i % 64) as f32 / 16.0; 3]).map(|c| [c[0], c[1], c[2], 1.0]).collect::<Vec<_>>();
    let run = |shadow: f32| {
        let mut px = hdr();
        hdr_toning(&mut px, 64, 16, &with(|p| p.shadow = shadow));
        px
    };
    assert_ne!(run(0.0), run(100.0), "Shadow +100 changes an image with values above 1.0");
    assert_ne!(run(0.0), run(-100.0), "Shadow -100 changes an image with values above 1.0");
}

#[test]
fn results_stay_in_range_for_every_slider_value() {
    for shadow in [-100.0, -50.0, 0.0, 50.0, 100.0] {
        for highlight in [-100.0, -50.0, 0.0, 50.0, 100.0] {
            let mut px = test_document();
            hdr_toning(&mut px, W, H, &with(|p| (p.shadow, p.highlight) = (shadow, highlight)));
            assert!(px.iter().flat_map(|q| &q[..3]).all(|v| v.is_finite() && (0.0..=1.0).contains(v)), "Shadow {shadow}, Highlight {highlight}");
        }
    }
}

#[test]
fn the_checked_path_gives_the_same_result() {
    // An ordinary image and one with values above 1.0, so both Shadow cases are compared.
    let ordinary: Vec<[f32; 4]> = (0..72).map(|i| [i as f32 / 71.0, 0.3, 0.6, 1.0]).collect();
    let hdr: Vec<[f32; 4]> = (0..72).map(|i| [i as f32 / 71.0 * 3.0, 0.3, 0.6, 1.0]).collect();
    for input in [ordinary, hdr] {
        for (shadow, highlight) in [(-100.0, 0.0), (100.0, 0.0), (0.0, -100.0), (0.0, 100.0), (60.0, -40.0)] {
            let p = with(|p| (p.radius, p.shadow, p.highlight) = (7.0, shadow, highlight));
            let mut native = input.clone();
            hdr_toning(&mut native, 9, 8, &p);
            let mut checked = input.clone();
            process_with(&mut checked, 9, 8, &p, &Interrupt::NONE).unwrap();
            for (a, b) in checked.iter().zip(&native) {
                assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6), "Shadow {shadow}, Highlight {highlight}: {a:?} vs {b:?}");
            }
        }
    }
}

#[test]
fn the_wide_range_fallback_follows_the_same_sign() {
    // Values large enough that the f32 polynomial overflows take the checked path's f64 fallback;
    // with the same exposure, Highlight -100 must not come out brighter there either.
    let run = |highlight: f32| {
        let mut px: Vec<[f32; 4]> = (0..16).map(|i| [1e37 * (i + 1) as f32, 1e37, 1e37, 1.0]).collect();
        process_with(&mut px, 4, 4, &with(|p| (p.radius, p.exposure, p.highlight) = (2.0, 5.0, highlight)), &Interrupt::NONE).unwrap();
        px
    };
    let (minus, plus) = (run(-100.0), run(100.0));
    for (a, b) in minus.iter().zip(&plus) {
        assert!(a.iter().chain(b).all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        assert!(a[0] <= b[0], "Highlight -100 {} vs +100 {}", a[0], b[0]);
    }
}
