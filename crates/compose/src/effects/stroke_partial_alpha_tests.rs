use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{Document, Effect, FxCommon, FxPaint, Layer, StrokeFx, StrokePosition};
use photocraft_geom::{Rect, Size};

/// The three squares and blue stroke measured in issue #15002.
fn fixture(depth: SampleType, position: StrokePosition) -> Document {
    let mut doc = Document::with_background("Stroke partial alpha", Size::new(240, 120), ColorMode::Rgb, depth, Color::WHITE);
    let mut layer = Layer::raster("Orange squares", doc.pixel_format());
    let surface = layer.surface_mut().unwrap();
    for (x, alpha) in [(20, 1.0), (100, 0.5), (180, 0.25)] {
        surface.fill_rect(Rect::from_xywh(x, 40, 40, 40), &[1.0, 128.0 / 255.0, 0.0, alpha]);
    }
    layer.effects.items.push(Effect::Stroke(StrokeFx {
        common: FxCommon::new(BlendMode::Normal, 1.0),
        size: if position == StrokePosition::Center { 6.0 } else { 5.0 },
        position,
        paint: FxPaint::Color(Color::rgb(0.0, 0.0, 1.0)),
        overprint: false,
    }));
    doc.layers.push(layer);
    doc
}

#[test]
fn stroke_partial_alpha_matches_the_measured_square_columns() {
    const BLUE: [i32; 3] = [0, 0, 255];
    const ORANGE: [i32; 3] = [255, 128, 0];
    const WHITE: [i32; 3] = [255; 3];
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for position in [StrokePosition::Inside, StrokePosition::Center, StrokePosition::Outside] {
            let doc = fixture(depth, position);
            let buf = crate::render(&doc, doc.bounds());
            let close = |x, y, expected: [i32; 3]| {
                let pixel = buf.get(x, y);
                let actual = [pixel[0], pixel[1], pixel[2]].map(|c| (c * 255.0).round() as i32);
                assert!(
                    actual.iter().zip(expected).all(|(a, b)| (a - b).abs() <= 1),
                    "{depth:?} {position:?} at ({x}, {y}): {actual:?}, expected {expected:?}"
                );
                assert!((pixel[3] - 1.0).abs() < 1e-6);
            };
            for (x, inside, outside) in [(120, [127, 127, 255], [128, 64, 127]), (200, [191, 191, 255], [64, 32, 191])] {
                for y in 40..80 {
                    close(
                        x,
                        y,
                        match position {
                            StrokePosition::Inside => inside,
                            StrokePosition::Center => BLUE,
                            StrokePosition::Outside => outside,
                        },
                    );
                }
                match position {
                    StrokePosition::Inside => close(x, 39, WHITE),
                    StrokePosition::Center => {
                        close(x, 36, WHITE);
                        close(x, 37, inside);
                        close(x, 38, BLUE);
                        close(x, 39, BLUE);
                    }
                    StrokePosition::Outside => {
                        close(x, 34, WHITE);
                        close(x, 35, inside);
                        close(x, 36, BLUE);
                        close(x, 39, BLUE);
                    }
                }
            }
            let (first, last) = match position {
                StrokePosition::Inside => (40, 44),
                StrokePosition::Center => (37, 42),
                StrokePosition::Outside => (35, 39),
            };
            close(40, first - 1, WHITE);
            for y in first..=last {
                close(40, y, BLUE);
            }
            close(40, last + 1, ORANGE);
            close(40, 60, ORANGE);
        }
    }
}

#[test]
fn the_raster_distance_preserves_the_opaque_interior_and_subpixel_outline() {
    use super::{FieldKind, distance_field};

    let row = [0.0, 0.25, 0.75, 1.0, 1.0, 1.0, 1.0, 1.0, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.0];
    let source = row.repeat(9);
    let inside = distance_field(FieldKind::StrokeInside, source.clone(), row.len(), 9, 6.0);
    let raster = distance_field(FieldKind::StrokeInsideRaster, source, row.len(), 9, 6.0);
    let mid = row.len() * 4;
    // Opaque pixels retain their distances even beside a translucent area.
    assert_eq!(raster[mid..mid + 8], inside[mid..mid + 8]);
    assert_eq!(raster[mid + 8], inside[mid + 8], "the existing subpixel seed is retained");
    assert!(inside[mid + 11] > 0.5, "the old outline did not reach this interior");
    assert_eq!(raster[mid + 11], 0.0, "the translucent interior is covered");
    assert_eq!(raster[mid + 15], inside[mid + 15], "the fully transparent pixel stays outside");
}

#[test]
fn stroke_partial_alpha_keeps_intrinsic_alpha_for_inside_and_fills_it_for_center() {
    for position in [StrokePosition::Inside, StrokePosition::Center] {
        let mut doc = fixture(SampleType::U8, position);
        let background = doc.layers[0].id;
        doc.remove(background).unwrap();
        let pixels = crate::render(&doc, doc.bounds());
        for (x, source_alpha) in [(120, 128.0 / 255.0), (200, 64.0 / 255.0)] {
            let actual = pixels.get(x, 60);
            let expected_alpha = if position == StrokePosition::Inside { source_alpha } else { 1.0 };
            assert_eq!(&actual[..3], &[0.0, 0.0, 1.0], "{position:?} at x={x}");
            assert!((actual[3] - expected_alpha).abs() < 1e-6, "{position:?} at x={x}: {actual:?}");
        }
        assert_eq!(pixels.get(80, 60), [0.0; 4]);
    }
}
