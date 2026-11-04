use super::*;
use photocraft_raster::{Interrupt, Surface};

#[test]
fn cmyk_convolution_offset_is_a_brightness_offset() {
    let rect = Rect::new(0, 0, 1, 1);
    let source = Surface::with_default(PixelFormat::new(ColorMode::Cmyk, SampleType::F32, false), &[0.3; 4]);
    for (offset, ink) in [(0., 1.), (0.25, 0.75)] {
        let params =
            convolution::Params { kernel: vec![0.; 25], scale: 1., offset, channels: vec![true; 4], alpha_weighted: false, edge: convolution::Edge::Repeat };
        assert_eq!(convolution::apply_with(&source, &params, rect, rect, None, &Interrupt::NONE).unwrap().pixel(0, 0), vec![ink; 4]);
    }
}

#[test]
fn flat_auto_levels_are_identity_at_every_native_depth() {
    for depth in SampleType::ALL {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for value in [0., 128. / 255., 1.] {
                let format = PixelFormat::new(mode, depth, true);
                let source = Surface::with_default(format, &[vec![value; format.channels() - 1], vec![1.]].concat());
                for kind in [tone::auto_levels::Kind::Tone, tone::auto_levels::Kind::Contrast, tone::auto_levels::Kind::Color] {
                    assert_eq!(tone::auto_levels::analyze(&source, Rect::new(0, 0, 2, 2), kind, &Interrupt::NONE).unwrap(), [[0., 1., 1.]; 3]);
                }
            }
        }
    }
}

#[test]
fn native_matte_unassociation_keeps_separations_and_hdr() {
    let rect = Rect::new(0, 0, 1, 1);
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Multichannel] {
        for depth in SampleType::ALL {
            let format = PixelFormat::new(mode, depth, true);
            for alpha in [0., 0.5, 1.] {
                let mut values = (0..format.channels() - 1).map(|c| 0.1 * (c + 1) as f32).collect::<Vec<_>>();
                if depth == SampleType::F32 {
                    values[0] = -0.75;
                    if values.len() > 1 {
                        values[1] = 2.;
                    }
                }
                values.push(alpha);
                let mut source = Surface::new(format);
                source.write_pixel(0, 0, &values);
                let before = source.pixel(0, 0);
                let mut expected = before.clone();
                let a = *before.last().unwrap();
                if a > 0. && a < 1. {
                    for v in expected.iter_mut().take(format.channels() - 1) {
                        *v /= a;
                    }
                }
                let mut oracle = Surface::new(format);
                oracle.write_pixel(0, 0, &expected);
                assert_eq!(
                    matting::remove_black_matte_with(&source, rect, &Interrupt::NONE).unwrap().pixel(0, 0),
                    oracle.pixel(0, 0),
                    "{mode:?}/{depth:?}/{alpha}"
                );
                assert_eq!(source.pixel(0, 0), before);
            }
        }
    }
}

#[test]
fn selected_alpha_filters_preserve_every_unselected_channel_bit() {
    let rect = Rect::new(0, 0, 1, 1);
    let mask = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.3]);
    let mut source = Surface::new(PixelFormat::RGBA32F);
    source.write_pixel(0, 0, &[0.3, -0., 1e30, 0.25]);
    let before = source.pixel(0, 0);
    let convolution = convolution::Params {
        kernel: vec![0.; 25],
        scale: 1.,
        offset: 0.7,
        channels: vec![false, false, false, true],
        alpha_weighted: false,
        edge: convolution::Edge::Repeat,
    };
    let got = convolution::apply_with(&source, &convolution, rect, rect, Some(&mask), &Interrupt::NONE).unwrap();
    for (v, b) in got.pixel(0, 0).iter().zip(&before).take(3) {
        assert_eq!(v.to_bits(), b.to_bits());
    }
    for seed in 0..32 {
        let noise = channel_noise::Params { amounts: vec![0., 0., 0., 100.], distribution: Distribution::Uniform, monochromatic: false, relative: false, seed };
        let got = channel_noise::apply_with(&source, &noise, rect, rect, Some(&mask), &Interrupt::NONE).unwrap();
        for (v, b) in got.pixel(0, 0).iter().zip(&before).take(3) {
            assert_eq!(v.to_bits(), b.to_bits());
        }
    }
}

#[test]
fn average_selection_does_not_mix_hidden_colors_into_new_coverage() {
    let rect = Rect::new(0, 0, 2, 1);
    let mask = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.5]);
    for depth in SampleType::ALL {
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
        source.write_pixel(0, 0, &[1., 0., 0., 0.]);
        source.write_pixel(1, 0, &[0., 0., 1., 1.]);
        let out = average::apply_with(&source, rect, rect, Some(&mask), &Interrupt::NONE).unwrap();
        for (v, expected) in out.pixel(0, 0).into_iter().zip([0., 0., 1., 0.25]) {
            assert!((v - expected).abs() < 0.005);
        }
    }
}

#[test]
fn checked_hdr_extreme_finite_inputs_and_controls_stay_finite() {
    for (shadow, highlight) in [(0., 0.), (50., 50.), (50., -50.), (-50., 50.)] {
        let mut pixels = [[0.001, 0.001, 0.001, 1.], [1e30, 1e30, 1e30, 1.]];
        let params = tone::HdrToning { radius: 2., detail: 300., exposure: 5., gamma: 0.1, shadow, highlight, ..Default::default() };
        tone::checked_hdr::process_with(&mut pixels, 2, 1, &params, &Interrupt::NONE).unwrap();
        assert!(pixels.iter().flatten().all(|v| v.is_finite() && (0. ..=1.).contains(v)));
    }
}
