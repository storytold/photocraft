//! #14723: HDR Toning's Local Adaptation stretches an ordinary image to the full range (white
//! stays white, dark tones are lifted) instead of compressing it towards its mean, and keeps the
//! order of the tones for every Radius and Strength.

use super::*;
use crate::hdr::{MergeOptions, ToneMethod, merge, tone_map};
use crate::photo_util::{linear_to_srgb, srgb_to_linear};

/// The reference application's factory Radius and Strength, passed explicitly so the samples
/// don't depend on the defaults (#14720).
fn reference(radius: f32, strength: f32) -> HdrToning {
    HdrToning { radius, strength, ..Default::default() }
}

const RADIUS: f32 = 7.68;
const STRENGTH: f32 = 0.5296;

/// A grey 8-bit level as a pixel.
fn grey(v: u32) -> [f32; 4] {
    let v = v as f32 / 255.0;
    [v, v, v, 1.0]
}

/// The issue's 512 x 256 document: a ramp (rows 0-63, column x is level x/2), eight flat grey
/// patches (rows 64-127), a 40 | 200 step edge at x 256 (rows 128-191) and eight colour patches
/// (rows 192-255).
fn document() -> (Vec<[f32; 4]>, usize, usize) {
    const COLOURS: [[u32; 3]; 8] = [[60, 20, 20], [20, 60, 20], [40, 40, 90], [120, 60, 30], [200, 60, 60], [60, 200, 60], [220, 200, 120], [230, 180, 200]];
    let (w, h) = (512, 256);
    let px = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            match y {
                0..64 => grey(x as u32 / 2),
                64..128 => grey(PATCHES[x / 64]),
                128..192 => grey(if x < 256 { 40 } else { 200 }),
                _ => {
                    let c = COLOURS[x / 64].map(|v| v as f32 / 255.0);
                    [c[0], c[1], c[2], 1.0]
                }
            }
        })
        .collect();
    (px, w, h)
}

const PATCHES: [u32; 8] = [0, 16, 48, 80, 128, 176, 224, 255];

/// The issue's dark 256 x 64 document: a ramp 0..63 at 4 px per level (rows 0-31) and a 30 | 90
/// step edge at x 128 (rows 32-63).
fn dark_document() -> (Vec<[f32; 4]>, usize, usize) {
    let (w, h) = (256, 64);
    let px = (0..w * h).map(|i| if i / w < 32 { grey((i % w) as u32 / 4) } else { grey(if i % w < 128 { 30 } else { 90 }) }).collect();
    (px, w, h)
}

/// The grey level of pixel `(x, y)`.
fn level(px: &[[f32; 4]], w: usize, x: usize, y: usize) -> f32 {
    let q = px[y * w + x];
    luma([q[0], q[1], q[2]]) * 255.0
}

fn toned((mut px, w, h): (Vec<[f32; 4]>, usize, usize), p: &HdrToning) -> (Vec<[f32; 4]>, usize) {
    hdr_toning(&mut px, w, h, p);
    (px, w)
}

#[test]
fn an_ordinary_image_is_stretched_to_white_with_its_dark_tones_lifted() {
    let (px, w) = toned(document(), &reference(RADIUS, STRENGTH));
    let patches = [0.0, 93.0, 131.0, 158.0, 191.0, 217.0, 240.0, 253.0];
    for (i, want) in patches.into_iter().enumerate() {
        let got = level(&px, w, i * 64 + 32, 96);
        assert!((got - want).abs() <= 12.0, "grey patch {}: {got:.1}, the reference gives {want}", PATCHES[i]);
    }
    for (v, want) in [(15, 91.0), (120, 186.0), (255, 254.0)] {
        let got = level(&px, w, 2 * v + 1, 32);
        assert!((got - want).abs() <= 12.0, "ramp {v}: {got:.1}, the reference gives {want}");
    }
    assert!(level(&px, w, 511, 32) > 240.0 && level(&px, w, 255 * 2, 96) > 240.0, "white stays white");
}

#[test]
fn a_dark_image_is_stretched_to_fill_the_range() {
    let (px, w) = toned(dark_document(), &reference(RADIUS, STRENGTH));
    for (v, want) in [(8, 105.0), (56, 203.0)] {
        let got = level(&px, w, 4 * v + 2, 16);
        assert!((got - want).abs() <= 15.0, "ramp {v}: {got:.1}, the reference gives {want}");
    }
    for (x, v, want) in [(64, 30, 157.0), (192, 90, 253.0)] {
        let got = level(&px, w, x, 48);
        assert!((got - want).abs() <= 15.0, "edge {v}: {got:.1}, the reference gives {want}");
    }
}

#[test]
fn a_darker_ramp_level_never_comes_out_brighter() {
    // The ramp alone, sampled every 15 levels: the blur at a large radius sees nothing else.
    // The issue's document at its three ramp samples: there the patches below the ramp fall
    // inside a large blur.
    let (w, h) = (512, 64);
    let ramp = (0..w * h).map(|i| grey((i % w) as u32 / 2)).collect::<Vec<_>>();
    for radius in [1.0, RADIUS, 100.0] {
        for strength in [0.1, 0.53, 4.0] {
            let (px, w) = toned((ramp.clone(), w, h), &reference(radius, strength));
            let samples: Vec<f32> = (1..=17).map(|k| level(&px, w, 2 * 15 * k + 1, 32)).collect();
            assert!(samples.windows(2).all(|p| p[0] <= p[1]), "ramp, radius {radius}, strength {strength}: levels 15, 30, .. 255 give {samples:?}");
            let (px, w) = toned(document(), &reference(radius, strength));
            let samples = [15, 120, 255].map(|v| level(&px, w, 2 * v + 1, 32));
            assert!(samples[0] <= samples[1] && samples[1] <= samples[2], "document, radius {radius}, strength {strength}: ramp 15/120/255 give {samples:?}");
            // The reference keeps the flat greys in order even at Radius 100 and Strength 4.
            let patches: Vec<f32> = (0..8).map(|i| level(&px, w, i * 64 + 32, 96)).collect();
            assert!(patches.windows(2).all(|p| p[0] <= p[1]), "document, radius {radius}, strength {strength}: grey patches give {patches:?}");
        }
    }
}

#[test]
fn black_stays_black_and_results_stay_in_range_at_every_depth() {
    // 8- and 16-bit documents arrive as 0..1; a 32-bit one can hold values far above 1.
    let (doc, w, h) = document();
    let bright: Vec<[f32; 4]> = doc.iter().map(|q| [q[0] * 40.0, q[1] * 40.0, q[2] * 40.0, 1.0]).collect();
    for px in [doc, bright] {
        let (px, w) = toned((px, w, h), &reference(RADIUS, STRENGTH));
        assert!(px.iter().all(|q| q[..3].iter().all(|v| (0.0..=1.0).contains(v))), "every channel in 0..1");
        assert_eq!(level(&px, w, 32, 96), 0.0, "black stays 0");
    }
}

/// A 4 x 1 strip of flat 64 x 64 areas with the given linear values, as Merge to HDR Pro hands
/// its radiance to the tone map.
fn strip(values: [f32; 4]) -> Vec<[f32; 4]> {
    (0..256 * 64)
        .map(|i| {
            let v = values[i % 256 / 64];
            [v, v, v, 1.0]
        })
        .collect()
}

#[test]
fn this_tone_map_reaches_the_reference_merge_from_the_reference_radiance() {
    // The reference application's own 32-bit merge of #14023's scene (#14021): grey 128, ramp 26,
    // ramp 246 and the white patch. Its 16-bit Local Adaptation result of that merge (#14023):
    // 35725, 14858 and 63103. Merge to HDR Pro keys the scene to middle grey, tones it and encodes
    // it (`hdr::tone_map`); here with this tone map, which #14023 will move the merge to.
    let mut px = strip([0.0531, 0.00074, 1.130, 1.341]);
    let log_mean = (px.iter().map(|q| f64::from(luma([q[0], q[1], q[2]]).max(1e-6)).ln()).sum::<f64>() / px.len() as f64).exp() as f32;
    for q in &mut px {
        q[..3].iter_mut().for_each(|v| *v *= 0.18 / log_mean);
    }
    hdr_toning(&mut px, 256, 64, &reference(6.0, 0.53));
    for q in &mut px {
        q[..3].iter_mut().for_each(|v| *v = linear_to_srgb(v.clamp(0.0, 1.0)));
    }
    for (i, name, want) in [(0, "grey 128", 35725.0), (1, "ramp 26", 14858.0), (2, "ramp 246", 63103.0)] {
        let got = px[32 * 256 + i * 64 + 32][1] * 65535.0;
        assert!((got - want).abs() <= 0.08 * 65535.0, "{name}: {got:.0}, the reference gives {want}");
    }
}

#[test]
fn a_strong_local_contrast_at_a_low_gamma_stays_finite() {
    // A bright pixel between dark ones with Detail 300 and Gamma 0.1 tones far past f32 before the
    // shadow and highlight curve; the result is still a colour, and the checked replay agrees.
    let p = HdrToning {
        radius: 100.0,
        strength: 0.1,
        gamma: 0.1,
        exposure: 0.0,
        detail: 300.0,
        shadow: 0.0,
        highlight: 0.0,
        vibrance: 0.0,
        saturation: 0.0,
        curve: Vec::new(),
    };
    let input = vec![[0.01, 0.01, 0.01, 1.0], [0.9, 0.9, 0.9, 1.0], [0.01, 0.01, 0.01, 1.0]];
    let mut px = input.clone();
    hdr_toning(&mut px, 3, 1, &p);
    assert!(px.iter().all(|q| q.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v))), "{px:?}");
    let mut checked = input;
    checked_hdr::process_with(&mut checked, 3, 1, &p, &photocraft_raster::Interrupt::NONE).unwrap();
    assert!(px.iter().flatten().zip(checked.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-6), "{px:?} vs {checked:?}");
}

/// #14023's scene: 300 x 200, a grey ramp in 10 px steps (rows 0-99, level round(x0 / 290 * 255)),
/// a grey 128 band (rows 100-119) and red, green, blue, white and black 60 px patches (rows
/// 120-179), shot at `ev` stops in 8 bits.
fn hdr_scene_shot(ev: f32) -> Vec<[f32; 4]> {
    let (w, h) = (300, 200);
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let level = |v: f32| v / 255.0;
            let c = match y {
                0..100 => [level((((x / 10 * 10) as f32) / 290.0 * 255.0).round()); 3],
                100..120 => [level(128.0); 3],
                120..180 => [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]][x / 60],
                _ => [0.0; 3],
            };
            let shot = c.map(|v| (linear_to_srgb((srgb_to_linear(v) * 2f32.powf(ev)).clamp(0.0, 1.0)) * 255.0).round() / 255.0);
            [shot[0], shot[1], shot[2], 1.0]
        })
        .collect()
}

#[test]
fn merge_to_hdr_local_adaptation_gets_no_worse_on_todays_merge() {
    // Merge to HDR Pro keeps the mean-anchored tone map until #14023 moves it to the reference's
    // radiance (#14723): on today's merge of #14023's scene its worst sample stays within main's
    // 11530 levels of the reference at 16 bits (11435 here); the white-anchored map's is 17001.
    let (w, h) = (300, 200);
    let shots: Vec<Vec<[f32; 4]>> = [-2.0, 0.0, 2.0].map(hdr_scene_shot).into();
    let refs: Vec<&[[f32; 4]]> = shots.iter().map(Vec::as_slice).collect();
    let options = MergeOptions { exposures: vec![0.25, 1.0, 4.0], remove_ghosts: false, ghost_base: None, response: None };
    let mut px = merge(w, h, &refs, &options).px;
    let p = HdrToning { radius: 6.0, strength: 0.53, ..Default::default() };
    tone_map(&mut px, w, h, &ToneMethod::LocalAdaptation(p));
    // (x, y, the reference's 16-bit green) for ramp 0, 26, 62, 123, 185, 246, grey 128 and black.
    let samples = [
        (5, 50, 1538.0),
        (35, 50, 14858.0),
        (75, 50, 21840.0),
        (145, 50, 33663.0),
        (215, 50, 48003.0),
        (285, 50, 63103.0),
        (150, 110, 35725.0),
        (270, 150, 1538.0),
    ];
    let error = |&(x, y, want): &(usize, usize, f32)| {
        let error = (px[y * w + x][1] * 65535.0 - want).abs();
        assert!(error.is_finite(), "({x}, {y}) is not a colour: {:?}", px[y * w + x]);
        error
    };
    let worst = samples.iter().map(error).fold(0.0, f32::max);
    assert!(worst <= 11530.0, "the worst sample is {worst:.0} levels off the reference");
}
