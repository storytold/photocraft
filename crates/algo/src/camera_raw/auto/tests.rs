use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::Surface;

use super::{CameraRaw, Image, Panel, measure_shadow_noise, suggest, suggest_full};
use crate::camera_raw::DevelopInputs;

fn develop(px: &mut [[f32; 4]], w: usize, h: usize, p: &CameraRaw, float: bool) {
    crate::camera_raw::develop(px, w, h, p, &DevelopInputs::default(), float);
}

const SIDE: usize = 192;

fn frame(sample: impl Fn(usize, usize) -> [f32; 4]) -> Vec<[f32; 4]> {
    (0..SIDE * SIDE).map(|i| sample(i % SIDE, i / SIDE)).collect()
}

fn gray(v: f32) -> [f32; 4] {
    [v, v, v, 1.0]
}

fn auto(px: &[[f32; 4]], panels: &[Panel], base: &CameraRaw) -> CameraRaw {
    let image = Image::new(px, SIDE, SIDE, None).unwrap();
    suggest(&image, panels, base)
}

#[test]
fn full_auto_fits_a_monotone_curve_and_respects_a_manual_curve() {
    let px = frame(|x, _| gray(0.18 + 0.28 * x as f32 / (SIDE - 1) as f32));
    let image = Image::new(&px, SIDE, SIDE, None).unwrap();
    let base = CameraRaw::default();
    let render = |px: &mut [[f32; 4]], w, h, p: &CameraRaw| develop(px, w, h, p, false);
    let fitted = suggest_full(&image, &[], &base, false, render);
    assert_eq!(fitted.point_curve.first(), Some(&[0.0, 0.0]));
    assert_eq!(fitted.point_curve.last(), Some(&[255.0, 255.0]));
    assert!(fitted.point_curve.windows(2).all(|p| p[0][0] < p[1][0] && p[0][1] < p[1][1]));
    let middle = SIDE / 2;
    let mut output = px.clone();
    render(&mut output, SIDE, SIDE, &fitted);
    assert!(output[middle][0] > px[middle][0]);

    let mut manual = base.clone();
    manual.point_curve = vec![[0.0, 0.0], [128.0, 96.0], [255.0, 255.0]];
    assert_eq!(suggest_full(&image, &Panel::ALL, &manual, false, render).point_curve, manual.point_curve);
    assert_eq!(suggest_full(&image, &[], &base, true, |_, _, _, _| panic!("HDR used SDR feedback")), base);
}

#[test]
fn full_auto_can_reject_the_seed_when_combined_controls_clip() {
    let px = frame(|x, _| {
        let v = 0.38 + 0.25 * x as f32 / SIDE as f32;
        [v, v * 0.84, v * 0.76, 1.0]
    });
    let image = Image::new(&px, SIDE, SIDE, None).unwrap();
    let base = CameraRaw::default();
    let seed = suggest(&image, &Panel::ALL, &base);
    assert!(seed.vibrance > 0.0);
    // Model a downstream stage with limited red-channel headroom to isolate how the
    // optimizer reacts to clipping it could not predict from the source histogram.
    let render = |px: &mut [[f32; 4]], w, h, p: &CameraRaw| {
        develop(px, w, h, p, false);
        if p.vibrance >= seed.vibrance * 0.9 {
            for q in px.iter_mut().take(w) {
                q[0] = 1.0;
            }
        }
    };
    let fitted = suggest_full(&image, &Panel::ALL, &base, false, render);
    assert!(fitted.vibrance < seed.vibrance * 0.9);
}

#[test]
fn full_auto_keeps_highlight_recovery_when_reducing_light_on_a_clipped_source() {
    let px = frame(|x, y| {
        if y < SIDE / 20 {
            gray(1.0)
        } else if y < SIDE / 6 {
            gray(0.97)
        } else {
            gray(0.54 + 0.15 * x as f32 / SIDE as f32)
        }
    });
    let image = Image::new(&px, SIDE, SIDE, None).unwrap();
    let base = CameraRaw::default();
    let seed = suggest(&image, &[Panel::Light], &base);
    assert!(seed.highlights < -10.0 && seed.contrast > 0.0);
    let render = |px: &mut [[f32; 4]], _, _, p: &CameraRaw| {
        if p.contrast >= seed.contrast * 0.9 {
            for q in px.iter_mut().filter(|q| q[0] < 0.7).take(SIDE) {
                q[0] = 1.0;
            }
        }
    };
    let fitted = suggest_full(&image, &[Panel::Light], &base, false, render);
    assert!(fitted.contrast < seed.contrast * 0.9);
    assert_eq!(fitted.highlights, seed.highlights);
}

#[test]
fn white_balance_reduces_the_cast_on_neutral_samples_and_defers_in_a_colored_scene() {
    let px = frame(|x, y| {
        let v = 0.28 + 0.40 * (x + y) as f32 / (2 * SIDE) as f32;
        [v * 1.10, v, v * 0.92, 1.0]
    });
    let mut base = CameraRaw::default();
    base.vibrance = 9.0;
    let auto_wb = auto(&px, &[Panel::WhiteBalance], &base);
    assert!(auto_wb.temperature < 0.0);
    assert_eq!(auto_wb.vibrance, base.vibrance);
    let mst = |p: &[[f32; 4]]| p.iter().map(|v| (v[0] - v[2]).abs()).sum::<f32>() / p.len() as f32;
    let mut corrected = px.clone();
    develop(&mut corrected, SIDE, SIDE, &auto_wb, false);
    assert!(mst(&corrected) < mst(&px) * 0.57);
    let sunset = frame(|x, _| [0.75 + x as f32 / (4 * SIDE) as f32, 0.42, 0.21, 1.0]);
    let mut original = CameraRaw::default();
    (original.temperature, original.tint) = (15.0, -3.0);
    assert_eq!(auto(&sunset, &[Panel::WhiteBalance], &original), original);
}

#[test]
fn color_boosts_muted_colors_and_leaves_a_neutral_image_alone() {
    let muted = frame(|x, _| {
        let v = 0.35 + x as f32 / (3 * SIDE) as f32;
        [v, v * 0.84, v * 0.76, 1.0]
    });
    let base = CameraRaw::default();
    let settings = auto(&muted, &[Panel::Color], &base);
    assert!(settings.vibrance > 0.0 && settings.vibrance <= 20.0);
    assert_eq!(settings.temperature, 0.0);
    let neutral = frame(|x, _| gray(0.3 + x as f32 / (3 * SIDE) as f32));
    assert_eq!(auto(&neutral, &[Panel::Color], &base), base);
}

#[test]
fn color_can_reduce_saturation_in_a_high_chroma_scene() {
    let pixels = frame(|x, y| {
        let v = 0.40 + 0.28 * x as f32 / SIDE as f32;
        if y < SIDE / 2 { [v, 0.06 * v, 0.09 * v, 1.0] } else { [0.07 * v, v, 0.08 * v, 1.0] }
    });
    let settings = auto(&pixels, &[Panel::Color], &CameraRaw::default());
    assert!(settings.saturation < 0.0);
    assert_eq!(settings.vibrance, 0.0);
    let mut output = pixels.clone();
    develop(&mut output, SIDE, SIDE, &settings, false);
    let chroma = |px: &[[f32; 4]]| px.iter().map(|q| q[0].max(q[1]).max(q[2]) - q[0].min(q[1]).min(q[2])).sum::<f32>();
    assert!(chroma(&output) < chroma(&pixels));
}

#[test]
fn dark_scene_with_a_bright_area_keeps_highlight_headroom() {
    let px = frame(|x, y| gray(if x < 32 && y < 32 { 0.99 } else { 0.20 }));
    let base = CameraRaw::default();
    let p = auto(&px, &[Panel::Light], &base);
    assert_eq!(p.exposure, 0.0);
    assert!(p.shadows > 0.0);
    let mut rendered = px.clone();
    develop(&mut rendered, SIDE, SIDE, &p, false);
    assert!(rendered.iter().all(|q| q[0] < 1.0));
}

#[test]
fn effects_distinguish_texture_from_a_gradient_or_an_isolated_edge() {
    let base = CameraRaw::default();
    let ramp = frame(|x, _| gray(0.25 + 0.5 * x as f32 / (SIDE - 1) as f32));
    let step = frame(|x, _| gray(if x < SIDE / 2 { 0.25 } else { 0.75 }));
    for px in [ramp, step] {
        assert_eq!(auto(&px, &[Panel::Effects], &base), base);
    }
    let texture = frame(|x, y| gray(0.5 + 0.074 * (x as f32 * std::f32::consts::TAU / 12.0).sin() + 0.04 * (y as f32 * std::f32::consts::TAU / 15.0).cos()));
    let settings = auto(&texture, &[Panel::Effects], &base);
    assert!(settings.clarity > 0.0 && settings.texture > 0.0, "{settings:?}");
    let mut rendered = texture.clone();
    develop(&mut rendered, SIDE, SIDE, &settings, false);
    let contrast = |pixels: &[[f32; 4]]| pixels.iter().map(|p| (p[0] - 0.5).abs()).sum::<f32>() / pixels.len() as f32;
    assert!(contrast(&rendered) > contrast(&texture) + 0.0005);
}

#[test]
fn native_shadow_noise_dampens_light_and_detail_when_the_averaged_preview_is_identical() {
    let area = Rect::new(0, 0, SIDE as i32, SIDE as i32);
    let mut clean = Surface::new(PixelFormat::RGBA32F);
    let mut noisy = Surface::new(PixelFormat::RGBA32F);
    for y in 0..area.y1 {
        for x in 0..area.x1 {
            let v = 0.12 + (x + y) as f32 * 0.0002;
            let region = Rect::new(x, y, x + 1, y + 1);
            clean.fill_rect(region, &gray(v));
            let phase = (x.wrapping_mul(73) ^ y.wrapping_mul(113) ^ x.wrapping_mul(y)) % 31;
            let perturbation = (phase - 15) as f32 * 0.002;
            noisy.fill_rect(region, &[v + perturbation, v, v - perturbation, 1.0]);
        }
    }
    let clean_noise = measure_shadow_noise(&clean, area, 1.8, |_, _| true);
    let noisy_amplitude = measure_shadow_noise(&noisy, area, 1.8, |_, _| true);
    assert!(clean_noise < 0.001 && noisy_amplitude > 0.012, "{clean_noise}, {noisy_amplitude}");
    assert_eq!(measure_shadow_noise(&noisy, area, 1.8, |_, _| false), 0.0);

    // The smoothed proxy cannot tell these two source surfaces apart.
    let pixels = frame(|x, y| gray(0.24 + 0.053 * (x as f32 * std::f32::consts::TAU / 12.0).sin() + 0.029 * (y as f32 * std::f32::consts::TAU / 15.0).cos()));
    let base = CameraRaw::default();
    let settings = |noise| {
        let image = Image::new(&pixels, SIDE, SIDE, None).unwrap().with_encoding_gamma(1.8).with_shadow_noise(noise);
        suggest(&image, &[Panel::Light, Panel::Effects], &base)
    };
    let plain = settings(clean_noise);
    let protected = settings(noisy_amplitude);
    assert!(protected.exposure < plain.exposure && protected.shadows < plain.shadows, "{plain:?} vs {protected:?}");
    assert!(protected.clarity < plain.clarity && protected.texture < plain.texture, "{plain:?} vs {protected:?}");
}

#[test]
fn vignette_lifts_symmetric_falloff_but_not_a_side_lit_ramp() {
    let falloff = frame(|x, y| {
        let xy = |v: usize| ((v as f32 + 0.5) / SIDE as f32) * 2.0 - 1.0;
        gray(0.56 - 0.14 * (xy(x).powi(2) + xy(y).powi(2)))
    });
    let setup = auto(&falloff, &[Panel::Effects], &CameraRaw::default());
    assert!(setup.vignette_amount > 0.0);
    let mut out = falloff.clone();
    develop(&mut out, SIDE, SIDE, &setup, false);
    let difference = |pixels: &[[f32; 4]]| pixels[SIDE * SIDE / 2 + SIDE / 2][0] - pixels[0][0];
    assert!(difference(&out) > 0.0 && difference(&out) < difference(&falloff));
    let side_lit = frame(|x, _| gray(0.25 + 0.45 * x as f32 / SIDE as f32));
    assert_eq!(auto(&side_lit, &[Panel::Effects], &CameraRaw::default()).vignette_amount, 0.0);
}

#[test]
fn vignette_stays_neutral_when_a_dark_quadrant_interrupts_the_falloff() {
    let pixels = frame(|x, y| {
        let xy = |v: usize| ((v as f32 + 0.5) / SIDE as f32) * 2.0 - 1.0;
        let (x, y) = (xy(x), xy(y));
        let radius = x.hypot(y);
        gray(if radius < 0.35 {
            0.56
        } else if radius < 0.82 && x > 0.0 && y < 0.0 {
            0.26
        } else if radius < 0.82 {
            0.47
        } else {
            0.22
        })
    });
    let p = auto(&pixels, &[Panel::Effects], &CameraRaw::default());
    assert_eq!(p.vignette_amount, 0.0);
}

#[test]
fn grain_is_suggested_for_repeated_tonal_bands() {
    let banded = frame(|x, _| gray(0.25 + (x / 7) as f32 * 0.008));
    let continuous = frame(|x, _| gray(0.25 + x as f32 / 7.0 * 0.008));
    let base = CameraRaw::default();
    let grain = auto(&banded, &[Panel::Effects], &base);
    assert!((1.0..=15.0).contains(&grain.grain_amount), "{grain:?}");
    assert_eq!(auto(&continuous, &[Panel::Effects], &base).grain_amount, 0.0);
}

#[test]
fn dehaze_opens_a_veiled_scene_without_dehazing_the_clear_reference() {
    let clean = frame(|x, y| gray(if (x / 29 + y / 38) % 3 == 0 { 0.06 } else { 0.60 }));
    let mut veiled: Vec<_> = clean.iter().map(|p| gray(p[0] * 0.62 + 0.27)).collect();
    let base = CameraRaw::default();
    let p = auto(&veiled, &[Panel::Effects], &base);
    assert!((1.0..=24.0).contains(&p.dehaze), "{p:?}");
    assert_eq!(auto(&clean, &[Panel::Effects], &base).dehaze, 0.0);
    let error = |pixels: &[[f32; 4]]| pixels.iter().zip(&clean).map(|(a, b)| (a[0] - b[0]).abs()).sum::<f32>();
    let before = error(&veiled);
    let mut dehaze = base;
    dehaze.dehaze = p.dehaze;
    develop(&mut veiled, SIDE, SIDE, &dehaze, false);
    assert!(error(&veiled) < before);
}

#[test]
fn auto_dehaze_preserves_existing_blacks_at_the_frame_boundary() {
    let veiled = |x, y| if (x / 29 + y / 38) % 3 == 0 { 0.31 } else { 0.64 };
    let unframed = frame(|x, y| gray(veiled(x, y)));
    let framed = frame(|x, y| if x < SIDE / 32 { [0.025, 0.035, 0.06, 1.0] } else { gray(veiled(x, y)) });
    let base = CameraRaw::default();
    assert!(auto(&unframed, &[Panel::Effects], &base).dehaze > 0.0);
    assert_eq!(auto(&framed, &[Panel::Effects], &base).dehaze, 0.0);
}

#[test]
fn mask_and_invalid_pixels_do_not_control_any_auto_panel() {
    let px = frame(|_, _| [0.19, 0.45, f32::NAN, 1.0]);
    let coverage = vec![0.0; px.len()];
    let image = Image::new(&px, SIDE, SIDE, Some(&coverage)).unwrap();
    assert_eq!(suggest(&image, &Panel::ALL, &CameraRaw::default()), CameraRaw::default());
    assert!(Image::new(&px, usize::MAX, 2, None).is_none());
    assert!(Image::new(&px, SIDE, SIDE, Some(&coverage[..1])).is_none());
}
