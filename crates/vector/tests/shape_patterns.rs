use photocraft_doc::{ColorMode, Fill, Pattern, PixelFormat, SampleType, ShapeLayer, ShapeStroke};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use photocraft_vector::{Paint, pattern::Tile, render_shape_with_patterns};

fn pattern(sample: SampleType, mode: ColorMode) -> Pattern {
    let format = PixelFormat::new(mode, sample, true);
    let mut s = Surface::new(format);
    for (x, rgba) in [(0, [0.9, 0.2, 0.1, 1.0]), (1, [0.1, 0.3, 0.9, 0.5])] {
        let mut values = [0.0; 5];
        let n = photocraft_raster::from_rgba_into(&format, rgba, &mut values);
        s.fill_rect(Rect::new(x, 0, x + 1, 2), &values[..n]);
    }
    Pattern::new("$$$/Patterns/Test=Tile", s, 2, 2)
}

fn fill(p: &Pattern) -> Fill {
    Fill::Pattern { id: p.id.clone(), name: p.name.clone(), scale: 1.0, angle: 0.0, link: true, phase: (0.0, 0.0) }
}

fn close(a: [f32; 4], b: [f32; 4], tolerance: f32) {
    for c in 0..4 {
        assert!((a[c] - b[c]).abs() < tolerance, "{a:?} != {b:?}");
    }
}

#[test]
fn pattern_shape_fill_and_stroke_render_at_every_depth_and_model() {
    let canvas = Rect::new(0, 0, 32, 32);
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            let p = pattern(sample, mode);
            let fmt = PixelFormat::new(mode, sample, true);
            let check = |actual: [f32; 4], expected: [f32; 4], where_: &str| {
                for c in 0..4 {
                    assert!((actual[c] - expected[c]).abs() < 1e-4, "{sample:?}/{mode:?} {where_}: {actual:?} != {expected:?}");
                }
            };
            // The cache stores the document's format. CMYK's decode/encode/decode
            // roundtrip need not preserve RGB, even when the source tile is also CMYK.
            let encoded = |x| {
                let values = photocraft_raster::from_rgba(&fmt, p.surface.rgba(x, 0));
                Surface::with_default(fmt, &values).rgba(0, 0)
            };
            let sh = ShapeLayer { path: photocraft_vector::shapes::rounded_rect(8.0, 8.0, 16.0, 16.0, [0.0; 4]), fill: Some(fill(&p)), ..Default::default() };
            let sampler =
                Paint::from_fill_with_patterns(sh.fill.as_ref().unwrap(), (8.0, 8.0, 24.0, 24.0), (0.0, 0.0, 32.0, 32.0), std::slice::from_ref(&p), mode);
            check(sampler.at(10.5, 10.5), p.surface.rgba(0, 0), "opaque tile sampler");
            check(sampler.at(11.5, 10.5), p.surface.rgba(1, 0), "half-alpha tile sampler");
            let surface = render_shape_with_patterns(&sh, fmt, canvas, std::slice::from_ref(&p));
            check(surface.rgba(10, 10), encoded(0), "opaque fill cache");
            check(surface.rgba(11, 10), encoded(1), "half-alpha fill cache");
            assert_eq!(surface.rgba(2, 2)[3], 0.0);
            let stroke = ShapeLayer { fill: None, stroke: Some(ShapeStroke { width: 4.0, paint: fill(&p), ..Default::default() }), ..sh };
            let surface = render_shape_with_patterns(&stroke, fmt, canvas, std::slice::from_ref(&p));
            check(surface.rgba(8, 12), encoded(0), "stroke cache");
            assert_eq!(surface.rgba(16, 16)[3], 0.0);
        }
    }
}

#[test]
fn pattern_lookup_placement_and_premultiplied_sampling() {
    let p = pattern(SampleType::F32, ColorMode::Rgb);
    let mut f = fill(&p);
    if let Fill::Pattern { id, name, link, phase, scale, angle } = &mut f {
        *id = "missing-id".into();
        *name = "tile".into();
        *link = false;
        *phase = (4.0, 6.0);
        *scale = 2.0;
        *angle = 90.0;
    }
    let paint = Paint::from_fill_with_patterns(&f, (8.0, 8.0, 24.0, 24.0), (0.0, 0.0, 32.0, 32.0), std::slice::from_ref(&p), ColorMode::Rgb);
    // Undo 90° and 2× scale: (5,5) relative to phase maps to tile centre (.5,.5).
    close(paint.at(5.0, 5.0), p.surface.rgba(0, 0), 0.001);
    assert!(matches!(Paint::from_fill(&f, (0.0, 0.0, 1.0, 1.0), (0.0, 0.0, 32.0, 32.0)), Paint::None));
    let tile = Tile::new(&p).unwrap();
    let a = p.surface.rgba(0, 0);
    let b = p.surface.rgba(1, 0);
    let alpha = (a[3] + b[3]) / 2.0;
    close(
        tile.sample(1.0, 0.5),
        [(a[0] * a[3] + b[0] * b[3]) / (2.0 * alpha), (a[1] * a[3] + b[1] * b[3]) / (2.0 * alpha), (a[2] * a[3] + b[2] * b[3]) / (2.0 * alpha), alpha],
        0.001,
    );
    close(tile.sample(-1.5, 0.5), a, 0.001);
    assert_eq!(tile.sample(f64::NAN, 0.0), [0.0; 4]);
    assert!(tile.sample(f64::MAX, -f64::MAX).iter().all(|v| v.is_finite()));
}

#[test]
fn tiny_positive_pattern_scale_is_preserved_in_sampling_and_shape_rendering() {
    let p = pattern(SampleType::F32, ColorMode::Rgb);
    let scale = 0.0005;
    let placement = photocraft_vector::pattern::Placement::anchored((8.0, 8.0), true, (0.0, 0.0), scale, 0.0);
    let mapped = placement.map(10.5, 10.5);
    assert!((mapped.0 - 5000.0).abs() < 0.001 && (mapped.1 - 5000.0).abs() < 0.001, "{mapped:?}");
    let expected = Tile::new(&p).unwrap().sample(mapped.0, mapped.1);
    let paint = Fill::Pattern { id: p.id.clone(), name: p.name.clone(), scale, angle: 0.0, link: true, phase: (0.0, 0.0) };
    let shape = ShapeLayer { path: photocraft_vector::shapes::rounded_rect(8.0, 8.0, 16.0, 16.0, [0.0; 4]), fill: Some(paint), ..Default::default() };
    let rendered = render_shape_with_patterns(&shape, PixelFormat::RGBA32F, Rect::new(0, 0, 32, 32), std::slice::from_ref(&p));
    close(rendered.rgba(10, 10), expected, 0.001);
}
#[test]
fn oversized_sparse_pattern_samples_without_a_second_full_surface() {
    let p = Pattern {
        id: "large".into(),
        name: "large".into(),
        width: 4096,
        height: 4096,
        surface: Surface::with_default(PixelFormat::RGBA8, &[0.2, 0.6, 0.9, 1.0]),
    };
    let tile = Tile::new(&p).unwrap();
    close(tile.sample(4000.5, 2000.5), p.surface.rgba(4000, 2000), 0.001);
    assert!(Tile::new(&Pattern { width: u32::MAX, ..p.clone() }).is_none());
    assert!(Tile::new(&Pattern { height: 0, ..p }).is_none());
}
