use super::{CameraRaw, Image, Panel, suggest};
use crate::camera_raw::develop;

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
fn white_balance_reduces_the_cast_on_neutral_samples_and_defers_in_a_colored_scene() {
    let px = frame(|x, y| {
        let v = 0.28 + 0.40 * (x + y) as f32 / (2 * SIDE) as f32;
        [v * 1.10, v, v * 0.92, 1.0]
    });
    let base = CameraRaw { vibrance: 9.0, ..Default::default() };
    let auto_wb = auto(&px, &[Panel::WhiteBalance], &base);
    assert!(auto_wb.temperature < 0.0);
    assert_eq!(auto_wb.vibrance, base.vibrance);
    let mst = |p: &[[f32; 4]]| p.iter().map(|v| (v[0] - v[2]).abs()).sum::<f32>() / p.len() as f32;
    let mut corrected = px.clone();
    develop(&mut corrected, SIDE, SIDE, &auto_wb, false);
    assert!(mst(&corrected) < mst(&px) * 0.57);
    let sunset = frame(|x, _| [0.75 + x as f32 / (4 * SIDE) as f32, 0.42, 0.21, 1.0]);
    let original = CameraRaw { temperature: 15.0, tint: -3.0, ..Default::default() };
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
    develop(&mut veiled, SIDE, SIDE, &CameraRaw { dehaze: p.dehaze, ..base }, false);
    assert!(error(&veiled) < before);
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
