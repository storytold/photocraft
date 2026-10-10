use super::*;
use crate::camera_raw::develop;

fn ramp(lo: f32, hi: f32, color: [f32; 3]) -> Vec<[f32; 4]> {
    (0..256)
        .map(|i| {
            let v = lo + (hi - lo) * i as f32 / 255.0;
            [v * color[0], v * color[1], v * color[2], 1.0]
        })
        .collect()
}

#[test]
fn improves_dark_and_bright_exposures_without_changing_white_balance_or_other_edits() {
    for (lo, hi, sign) in [(0.06, 0.28, 1.0), (0.55, 0.95, -1.0)] {
        let source = ramp(lo, hi, [1.0, 0.95, 0.9]);
        let original = CameraRaw { temperature: 18.0, tint: -8.0, texture: 12.0, point_curve: vec![[0.0, 0.0], [255.0, 255.0]], ..Default::default() };
        let settings = estimate(&source, &original).unwrap();
        assert!(settings.exposure * sign > 0.0);
        let mut params = original.clone();
        settings.apply(&mut params);
        assert_eq!(params.temperature, original.temperature);
        assert_eq!(params.tint, original.tint);
        assert_eq!(params.texture, original.texture);
        assert_eq!(params.point_curve, original.point_curve);
        assert_eq!(estimate(&source, &params).unwrap(), settings, "Auto is repeatable");
        let mut output = source.clone();
        develop(&mut output, 256, 1, &params, false);
        let mean = |pixels: &[[f32; 4]]| pixels.iter().map(|p| luma([p[0], p[1], p[2]])).sum::<f32>() / pixels.len() as f32;
        assert!((mean(&output) - 0.46).abs() < (mean(&source) - 0.46).abs());
        assert!(output.iter().flatten().all(|v| v.is_finite()));
    }
}

#[test]
fn protects_bright_tails_and_does_not_colorize_neutral_images() {
    let mut source = ramp(0.03, 0.12, [1.0; 3]);
    source.extend(vec![[0.98, 0.98, 0.98, 1.0]; 32]);
    let p = estimate(&source, &CameraRaw::default()).unwrap();
    assert!(p.exposure <= 0.01, "do not blow out highlights to normalize a dark scene");
    assert_eq!(p.vibrance, 0.0);
    assert_eq!(p.saturation, 0.0);
}

#[test]
fn boosts_muted_colors_and_reduces_excessive_saturation() {
    let muted = estimate(&ramp(0.1, 0.7, [1.0, 0.9, 0.85]), &CameraRaw::default()).unwrap();
    let saturated = estimate(&ramp(0.1, 0.7, [1.0, 0.2, 0.05]), &CameraRaw::default()).unwrap();
    assert!(muted.vibrance > 0.0 && muted.saturation > 0.0);
    assert!(saturated.vibrance < 0.0 && saturated.saturation < 0.0);
}

#[test]
fn transparent_invalid_and_flat_inputs_are_safe() {
    let mut pixels = ramp(0.1, 0.3, [1.0; 3]);
    let expected = estimate(&pixels, &CameraRaw::default()).unwrap();
    pixels.extend([[1.0, 0.0, 0.0, 0.0], [f32::NAN, 0.0, 0.0, 1.0], [0.0, f32::INFINITY, 0.0, 1.0]]);
    assert_eq!(estimate(&pixels, &CameraRaw::default()).unwrap(), expected);
    assert!(estimate(&[], &CameraRaw::default()).is_err());
    assert!(estimate(&[[0.0; 4]], &CameraRaw::default()).is_err());
    assert!(estimate(&vec![[0.5; 4]; MAX_SAMPLES + 1], &CameraRaw::default()).is_err());
    for v in [0.0, 0.5, 1.0, 8.0] {
        let result = estimate(&[[v, v, v, 1.0]; 64], &CameraRaw::default()).unwrap();
        assert_eq!(result.exposure, 0.0);
    }
}

#[test]
fn coverage_weights_prevent_masked_highlights_from_biasing_exposure() {
    let source = ramp(0.06, 0.25, [1.0; 3]);
    let expected = estimate(&source, &CameraRaw::default()).unwrap();
    let mut masked = source.clone();
    masked.extend(vec![[1.0, 1.0, 1.0, 0.0]; 1024]);
    assert_eq!(estimate(&masked, &CameraRaw::default()).unwrap(), expected);
    for pixel in masked.iter_mut().skip(source.len()) {
        pixel[3] = 1.0;
    }
    assert!(estimate(&masked, &CameraRaw::default()).unwrap().exposure < expected.exposure);
}
