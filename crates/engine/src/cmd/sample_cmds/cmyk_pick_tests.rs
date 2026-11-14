use super::*;
use photocraft_doc::{Layer, Size};

const INKS: [[f32; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

fn chart(depth: SampleType) -> Document {
    let mut doc = Document::new("CMYK chart", Size::new(32, 8), ColorMode::Cmyk, depth);
    let mut layer = Layer::raster("Chart", doc.pixel_format());
    for (i, ink) in INKS.iter().enumerate() {
        layer.surface_mut().unwrap().fill_rect(Rect::new(i as i32 * 8, 0, (i as i32 + 1) * 8, 8), &[ink[0], ink[1], ink[2], ink[3], 1.0]);
    }
    doc.layers.push(layer);
    doc
}

#[test]
fn cmyk_picks_preserve_all_four_pure_inks_at_every_depth() {
    for depth in SampleType::ALL {
        let doc = chart(depth);
        for (i, ink) in INKS.iter().enumerate() {
            for zoom in [None, Some(0.5), Some(1.0), Some(2.0)] {
                for size in [1, 3] {
                    // A flat 8x8 patch also covers the 8x8 box at 50% zoom.
                    assert_eq!(
                        sample_cmyk_at(&doc, None, (i * 8 + 3) as f64, 3.0, size, zoom, SampleLayers::All),
                        Some(*ink),
                        "{depth:?}, patch {i}, size {size}, zoom {zoom:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn cmyk_picks_follow_layer_selection_and_no_adjustments() {
    let mut doc = chart(SampleType::U8);
    let mut top = Layer::raster("Magenta", doc.pixel_format());
    top.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 32, 8), &[0.0, 1.0, 0.0, 0.0, 1.0]);
    let active = doc.layers[0].id;
    doc.layers.push(top);
    let pick = |doc: &Document, which| sample_cmyk_at(doc, Some(active), 2.0, 2.0, 1, None, which);
    assert_eq!(pick(&doc, SampleLayers::All), Some(INKS[1]));
    assert_eq!(pick(&doc, SampleLayers::Current), Some(INKS[0]));
    assert_eq!(pick(&doc, SampleLayers::CurrentAndBelow), Some(INKS[0]));
    doc.layers.push(Layer::new("Invert", LayerContent::Adjustment(photocraft_doc::Adjustment::Invert)));
    assert_eq!(pick(&doc, SampleLayers::AllNoAdjustments), Some(INKS[1]));
    assert_eq!(pick(&doc, SampleLayers::All), Some([1.0, 0.0, 1.0, 1.0]));
}

#[test]
fn cmyk_picks_average_covered_inks_and_use_the_zoom_box() {
    let mut doc = chart(SampleType::F32);
    let surface = doc.layers[0].surface_mut().unwrap();
    surface.fill_rect(Rect::new(0, 0, 32, 8), &[0.0; 5]);
    surface.write_pixel(2, 2, &[1.0, 0.0, 0.0, 0.0, 1.0]);
    surface.write_pixel(3, 2, &[0.0, 1.0, 0.0, 0.0, 0.5]);
    let got = sample_cmyk_at(&doc, None, 2.0, 2.0, 3, None, SampleLayers::All).unwrap();
    assert!((got[0] - 2.0 / 3.0).abs() < 1e-6 && (got[1] - 1.0 / 3.0).abs() < 1e-6, "{got:?}");
    assert_eq!(got[2..], [0.0, 0.0]);

    let surface = doc.layers[0].surface_mut().unwrap();
    surface.fill_rect(Rect::new(0, 0, 32, 8), &[0.0, 0.0, 0.0, 0.0, 1.0]);
    surface.write_pixel(4, 4, &[1.0, 0.0, 0.0, 0.0, 1.0]);
    // At 100%, 3 by 3 covers 4x4, including (4,4); above 100% it is 3x3.
    assert_eq!(sample_cmyk_at(&doc, None, 2.0, 2.0, 3, Some(1.0), SampleLayers::All), Some([1.0 / 16.0, 0.0, 0.0, 0.0]));
    assert_eq!(sample_cmyk_at(&doc, None, 2.0, 2.0, 3, Some(2.0), SampleLayers::All), Some([0.0; 4]));
}

#[test]
fn cmyk_picks_decline_non_cmyk_unsupported_empty_and_invalid_samples() {
    let mut doc = chart(SampleType::U8);
    let pick = |doc: &Document, x, size| sample_cmyk_at(doc, None, x, 2.0, size, None, SampleLayers::All);
    assert_eq!(pick(&doc, -1.0, 1), None);
    assert_eq!(pick(&doc, f64::NAN, 1), None);
    assert_eq!(pick(&doc, 2.0, 7), None);
    assert_eq!(sample_cmyk_at(&doc, None, 2.0, 2.0, 1, None, SampleLayers::Current), None);
    doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(0, 0, 32, 8), &[0.0; 5]);
    assert_eq!(pick(&doc, 2.0, 1), None);
    doc.layers[0].blend = photocraft_color::BlendMode::Hue;
    assert_eq!(pick(&doc, 2.0, 1), None);
    doc.mode = ColorMode::Rgb;
    assert_eq!(pick(&doc, 2.0, 1), None);
}
