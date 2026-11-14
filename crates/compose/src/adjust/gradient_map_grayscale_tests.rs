use photocraft_color::convert::{linear_to_srgb, srgb_to_lab};
use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent, Size};
use photocraft_raster::Surface;

use super::{Transfer, apply_depth, apply_in};
use crate::{Buffer, Rect, flatten_to_surface};

fn render_grayscale_strips(depth: SampleType, strips: &[u8], adj: Adjustment) -> Vec<f32> {
    let w = strips.len() as u32;
    let mut doc = Document::new("gray", Size::new(w, 1), ColorMode::Grayscale, depth);
    let fmt = doc.pixel_format();
    let mut base = Layer::raster("strips", fmt);
    let mut s = Surface::new(fmt);
    for (x, &v) in strips.iter().enumerate() {
        s.write_pixel(x as i32, 0, &[f32::from(v) / 255.0, 1.0]);
    }
    base.content = LayerContent::Raster(s);
    doc.layers.push(base);
    doc.layers.push(Layer::new("gm", LayerContent::Adjustment(adj)));

    let flat = flatten_to_surface(&doc, PixelFormat::new(ColorMode::Grayscale, depth, false), Some([1.0; 3]));
    (0..w).map(|x| flat.pixel(x as i32, 0)[0]).collect()
}

fn gray_lstar(g: f32) -> f32 {
    srgb_to_lab([g, g, g])[0]
}

fn linear_red_to_blue_stops() -> Vec<(f32, [f32; 3])> {
    (0u8..=255)
        .map(|i| {
            let t = f32::from(i) / 255.0;
            (t, [linear_to_srgb(1.0 - t), 0.0, linear_to_srgb(t)])
        })
        .collect()
}

#[test]
fn grayscale_8_and_16_bit_red_to_blue_uses_gradient_lightness() {
    let strips = [16u8, 48, 80, 112, 144, 176, 208, 240];
    let adj = Adjustment::GradientMap { stops: linear_red_to_blue_stops(), reverse: false, dither: false, source: None };

    let g8 = render_grayscale_strips(SampleType::U8, &strips, adj.clone());
    let g16 = render_grayscale_strips(SampleType::U16, &strips, adj);

    let l_first = gray_lstar(g8[0]);
    let l_last = gray_lstar(g8[strips.len() - 1]);
    assert!((l_first - 53.0).abs() <= 1.0, "strip 16: gray {} (L* {l_first:.2}), expected L* 53 ± 1", g8[0] * 255.0);
    assert!((l_last - 31.0).abs() <= 1.0, "strip 240: gray {} (L* {l_last:.2}), expected L* 31 ± 1", g8[strips.len() - 1] * 255.0);

    for (i, (&a, &b)) in g8.iter().zip(&g16).enumerate() {
        assert!((a - b).abs() <= 1.0 / 255.0 + 1e-4, "strip {} differs between 8-bit ({a}) and 16-bit ({b})", strips[i]);
    }
    for w in g16.windows(2) {
        assert!(w[0] > w[1], "lightness should fall from red to blue: {g16:?}");
    }
}

#[test]
fn grayscale_black_to_white_maps_each_gray_to_itself() {
    let strips = [0u8, 16, 48, 80, 112, 128, 144, 176, 208, 240, 255];
    let adj = Adjustment::GradientMap { stops: vec![(0.0, [0.0; 3]), (1.0, [1.0; 3])], reverse: false, dither: false, source: None };
    for depth in [SampleType::U8, SampleType::U16] {
        let out = render_grayscale_strips(depth, &strips, adj.clone());
        for (&want, &got) in strips.iter().zip(&out) {
            let got_255 = got * 255.0;
            assert!((got_255 - f32::from(want)).abs() <= 1.0, "{depth:?}: gray {want} mapped to {got_255:.2}");
        }
    }
}

#[test]
fn grayscale_reverse_and_dither_work_with_lightness() {
    let adj_rev = Adjustment::GradientMap { stops: linear_red_to_blue_stops(), reverse: true, dither: false, source: None };
    let out = render_grayscale_strips(SampleType::U8, &[16, 240], adj_rev);
    assert!((gray_lstar(out[0]) - 31.0).abs() <= 1.0, "reversed strip 16 L* {}", gray_lstar(out[0]));
    assert!((gray_lstar(out[1]) - 53.0).abs() <= 1.0, "reversed strip 240 L* {}", gray_lstar(out[1]));

    let stops = vec![(0.0, [1.0, 0.0, 0.0]), (1.0, [0.0, 0.0, 1.0])];
    let mut plain = Buffer::filled(Rect::new(0, 0, 16, 1), [0.5, 0.5, 0.5, 1.0]);
    let mut dithered = plain.clone();
    apply_in(
        &Adjustment::GradientMap { stops: stops.clone(), reverse: false, dither: false, source: None },
        &mut plain,
        Transfer::Gamma(1.732),
        Some(SampleType::U8),
        ColorMode::Grayscale,
    );
    apply_in(
        &Adjustment::GradientMap { stops, reverse: false, dither: true, source: None },
        &mut dithered,
        Transfer::Gamma(1.732),
        Some(SampleType::U8),
        ColorMode::Grayscale,
    );
    assert!(plain.px.iter().zip(&dithered.px).all(|(a, b)| (a[0] - b[0]).abs() <= 0.5 / 255.0 + 1e-6));
    assert!(plain.px.iter().zip(&dithered.px).any(|(a, b)| a[0] != b[0]));
}

#[test]
fn rgb_cmyk_lab_duotone_and_gray32_keep_todays_behavior() {
    let adj = Adjustment::GradientMap { stops: vec![(0.0, [1.0, 0.0, 0.0]), (1.0, [0.0, 0.0, 1.0])], reverse: false, dither: false, source: None };
    let px = [0.25, 0.5, 0.75, 1.0];

    for (mode, depth) in [
        (ColorMode::Rgb, Some(SampleType::U8)),
        (ColorMode::Cmyk, Some(SampleType::U8)),
        (ColorMode::Duotone, Some(SampleType::U8)),
        (ColorMode::Grayscale, Some(SampleType::F32)),
    ] {
        let mut got = Buffer::filled(Rect::new(0, 0, 1, 1), px);
        let mut want = got.clone();
        apply_in(&adj, &mut got, Transfer::Srgb, depth, mode);
        apply_depth(&adj, &mut want, Transfer::Srgb, depth);
        assert_eq!(got.px, want.px, "{mode:?} {depth:?} should keep today's screen-colour path");
    }

    // Lab keeps its L-driven RGB gradient output.
    let mut lab = Buffer::filled(Rect::new(0, 0, 1, 1), [Color::WHITE.c[0]; 4]);
    apply_in(&adj, &mut lab, Transfer::Srgb, Some(SampleType::U8), ColorMode::Lab);
    assert_eq!(lab.px[0][2], 1.0);
    assert!(lab.px[0][0] < 0.01);
}
