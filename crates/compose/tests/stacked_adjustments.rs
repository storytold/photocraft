//! RGB/8 pixels measured from Photoshop 26.11.6 exports of the issue's clipped
//! brightness/contrast and hue/saturation stacks. The source pixel comes from
//! the same PSD's opaque image layer at (0, 0).

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_compose::render;
use photocraft_doc::adjust::HueRange;
use photocraft_doc::{Adjustment, Document, Layer, LayerContent, Size};
use photocraft_raster::Surface;

fn clipped_stack_on_pixel(source: [u8; 4], adjustments: impl IntoIterator<Item = Adjustment>) -> [u8; 4] {
    let mut doc = Document::new("stacked adjustments", Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
    let mut surface = Surface::new(PixelFormat::RGBA8);
    surface.write_interleaved(doc.bounds(), &source);
    doc.layers.push(Layer::new("image", LayerContent::Raster(surface)));
    for adjustment in adjustments {
        let mut layer = Layer::new("adjustment", LayerContent::Adjustment(adjustment));
        layer.clipped = true;
        doc.layers.push(layer);
    }
    let pixels = render(&doc, doc.bounds()).to_rgba8().pixels;
    [pixels[0], pixels[1], pixels[2], pixels[3]]
}

fn clipped_stack(adjustments: impl IntoIterator<Item = Adjustment>) -> [u8; 4] {
    clipped_stack_on_pixel([1, 34, 65, 255], adjustments)
}

#[test]
fn clipped_brightness_then_hue_saturation_matches_photoshop_rgb8() {
    let first = Adjustment::BrightnessContrast { brightness: -52.0, contrast: 43.0, legacy: false };
    let second = Adjustment::HueSaturation { hue: -62.0, saturation: 15.0, lightness: -16.0, colorize: false, ranges: HueRange::defaults() };
    assert_eq!(clipped_stack([first, second]), [0, 31, 15, 255]);
}

#[test]
fn clipped_hue_saturation_then_brightness_matches_photoshop_rgb8() {
    let first = Adjustment::HueSaturation { hue: -138.0, saturation: -42.0, lightness: 41.0, colorize: false, ranges: HueRange::defaults() };
    let second = Adjustment::BrightnessContrast { brightness: -54.0, contrast: 39.0, legacy: false };
    assert_eq!(clipped_stack([first, second]), [85, 88, 71, 255]);
}

#[test]
fn single_hue_saturation_from_second_stack_matches_photoshop_rgb8() {
    let adjustment = Adjustment::HueSaturation { hue: -138.0, saturation: -42.0, lightness: 41.0, colorize: false, ranges: HueRange::defaults() };
    assert_eq!(clipped_stack([adjustment]), [130, 134, 112, 255]);
}

#[test]
fn single_brightness_contrast_from_second_stack_matches_photoshop_rgb8() {
    let adjustment = Adjustment::BrightnessContrast { brightness: -54.0, contrast: 39.0, legacy: false };
    assert_eq!(clipped_stack([adjustment]), [1, 18, 38, 255]);
}

#[test]
fn single_hue_saturation_from_first_stack_matches_photoshop_rgb8() {
    let adjustment = Adjustment::HueSaturation { hue: -62.0, saturation: 15.0, lightness: -16.0, colorize: false, ranges: HueRange::defaults() };
    assert_eq!(clipped_stack([adjustment]), [0, 56, 25, 255]);
}

#[test]
fn hue_saturation_on_brightness_result_keeps_the_sector_edge_below_maximum() {
    let adjustment = Adjustment::HueSaturation { hue: -62.0, saturation: 15.0, lightness: -16.0, colorize: false, ranges: HueRange::defaults() };
    // The first stack's BC layer produces this exact byte pixel. Photoshop's single-layer
    // probe returns 132 in red even though blue is the maximum channel at 133.
    assert_eq!(clipped_stack_on_pixel([154, 107, 105, 255], [adjustment]), [132, 85, 133, 255]);
}

/// Crossed Lightness signs and near-tie colours from a separate Photoshop RGB/8
/// probe keep the byte-path choice independent of either issue parameter tuple.
#[test]
fn hue_saturation_rgb8_crossed_lightness_and_sector_edges_match_photoshop() {
    let inputs = [[2, 38, 74], [1, 34, 65], [1, 19, 19], [27, 8, 27], [131, 130, 112], [43, 103, 104]];
    let oracles = [
        (-62.0, 15.0, 16.0, [[37, 107, 70], [36, 99, 64], [40, 57, 39], [45, 46, 64], [151, 132, 133], [72, 132, 71]]),
        (-138.0, -42.0, -41.0, [[30, 35, 10], [26, 30, 8], [9, 7, 3], [7, 13, 11], [72, 68, 75], [54, 48, 32]]),
        (-62.0, 15.0, -16.0, [[0, 64, 30], [0, 56, 25], [1, 17, 0], [5, 6, 24], [111, 92, 93], [32, 92, 31]]),
        (-138.0, -42.0, 41.0, [[134, 139, 114], [130, 134, 112], [113, 111, 107], [111, 117, 115], [176, 172, 179], [158, 152, 136]]),
    ];
    let mut mismatches = Vec::new();
    for (hue, saturation, lightness, expected) in oracles {
        let adjustment = Adjustment::HueSaturation { hue, saturation, lightness, colorize: false, ranges: HueRange::defaults() };
        for (input, want) in inputs.into_iter().zip(expected) {
            let got = clipped_stack_on_pixel([input[0], input[1], input[2], 255], [adjustment.clone()]);
            if got != [want[0], want[1], want[2], 255] {
                mismatches.push((hue, saturation, lightness, input, got, want));
            }
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:?}");
}
