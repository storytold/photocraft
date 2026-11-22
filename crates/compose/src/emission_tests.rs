use crate::flatten;
use photocraft_color::{BlendMode, ColorMode, EMISSIVE_ALPHA, PixelFormat, SampleType, is_emissive_alpha};
use photocraft_doc::{Document, Layer, LayerMask};
use photocraft_geom::Size;
use photocraft_raster::Surface;

fn document() -> Document {
    Document::new("Emission", Size::new(1, 1), ColorMode::Rgb, SampleType::F32)
}

fn layer(pixel: [f32; 4]) -> Layer {
    let mut layer = Layer::raster("pixel", PixelFormat::RGBA32F);
    layer.surface_mut().unwrap().fill_rect(document().bounds(), &pixel);
    layer
}

fn close(actual: [f32; 4], expected: [f32; 4]) {
    for (a, e) in actual.into_iter().zip(expected) {
        assert!((a - e).abs() <= 1e-6, "{actual:?} != {expected:?}");
    }
}

#[test]
fn clipped_emission_uses_the_same_additive_rule_as_unclipped_emission() {
    for mode in [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen] {
        let mut doc = document();
        let mut emitter = layer([0.5, 0.25, -0.125, EMISSIVE_ALPHA]);
        emitter.blend = mode;
        emitter.clipped = true;
        doc.layers = vec![layer([0.2, 0.4, 0.6, 0.5]), emitter].into();
        close(flatten(&doc).get(0, 0), [0.7, 0.65, 0.475, 0.5]);
    }
}

#[test]
fn group_opacity_and_masks_preserve_zero_coverage_light() {
    for (opacity, mask) in [(0.5, 1.0), (1.0, 0.5), (0.5, 0.5)] {
        for clipped in [false, true] {
            let mut doc = document();
            let mut group = Layer::group("group", vec![layer([0.5, 0.25, -0.125, EMISSIVE_ALPHA])]);
            group.opacity = opacity;
            group.mask = Some(LayerMask {
                surface: Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[mask]),
                ..LayerMask::reveal_all()
            });
            doc.layers.push(group);
            if clipped {
                let mut clip = layer([0.0, 0.0, 1.0, 1.0]);
                clip.clipped = true;
                doc.layers.push(clip);
            }
            let buf = flatten(&doc);
            assert!(is_emissive_alpha(buf.get(0, 0)[3]), "opacity={opacity}, mask={mask}, clipped={clipped}: {:?}", buf.px);
            let k = opacity * mask;
            close(buf.over_background([1.0; 3]).get(0, 0), [1.0 + 0.5 * k, 1.0 + 0.25 * k, 1.0 - 0.125 * k, 1.0]);
        }
    }
}

#[test]
fn ordinary_negative_zero_alpha_remains_transparent() {
    let mut doc = document();
    doc.layers.push(layer([0.5, 0.25, 0.125, -0.0]));
    close(flatten(&doc).over_background([1.0; 3]).get(0, 0), [1.0; 4]);
}

#[test]
fn emission_composite_overflow_keeps_representable_associated_light() {
    let mut doc = document();
    let light = f32::MAX / 2.0;
    let coverage = 1e-10;
    doc.layers = vec![layer([light, 0.0, 0.0, EMISSIVE_ALPHA]), layer([0.0, 0.0, 0.0, coverage])].into();
    let pixel = flatten(&doc).get(0, 0);
    assert!(pixel[0].is_finite());
    let associated = f64::from(pixel[0]) * f64::from(pixel[3]);
    let expected = f64::from(light) * f64::from(1.0 - coverage);
    assert!((associated / expected - 1.0).abs() <= 1e-6, "{pixel:?}");
}

#[test]
fn reduced_emission_preserves_light_and_zero_coverage() {
    use crate::reduced::{ReducedStats, render_reduced_with};
    use photocraft_geom::Rect;
    for light in [0.5, f32::MAX / 2.0, f32::MIN_POSITIVE] {
        for sparse in [false, true] {
            let mut doc = Document::new("Reduced emission", Size::new(128, 64), ColorMode::Rgb, SampleType::F32);
            let mut emitter = Layer::raster("emission", PixelFormat::RGBA32F);
            let pixel = [light, 0.0, 0.0, EMISSIVE_ALPHA];
            if sparse {
                emitter.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 64, 64), &pixel);
            } else {
                *emitter.surface_mut().unwrap() = Surface::with_default(PixelFormat::RGBA32F, &pixel);
            }
            doc.layers.push(emitter);
            let expected = if sparse { light * 0.5 } else { light };
            let reductions = [
                crate::render_reduced(&doc, 1, 1),
                crate::render_reduced_by(&doc, doc.bounds(), 128),
                crate::render_reduced_damage(&doc, 1, 1, Rect::new(0, 0, 1, 1)),
                render_reduced_with(&doc, doc.bounds(), 1, 1, None, 3, false, &mut ReducedStats::default()),
            ];
            for reduced in reductions {
                let actual = reduced.get(0, 0);
                assert_eq!(actual[0], expected, "sparse={sparse}, light={light}: {actual:?}");
                assert!(is_emissive_alpha(actual[3]), "{actual:?}");
            }
        }
    }
}
