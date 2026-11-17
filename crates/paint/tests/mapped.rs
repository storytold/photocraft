use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_paint::{BrushContext, BrushSettings, Stroke, StrokePoint, StrokeRenderer, dabs};
use photocraft_raster::Surface;

#[test]
fn mapped_native_coverage_matches_unwarped_strokes_at_every_depth() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for wet in [false, true] {
            let format = PixelFormat::new(ColorMode::Rgb, depth, true);
            let brush = BrushSettings { size: 17.0, hardness: 0.6, flow: 0.4, opacity: 0.6, color: [0.2, 0.7, 0.4, 1.0], wet_edges: wet, ..Default::default() };
            let points = vec![StrokePoint::new(20.0, 20.0, 0.7), StrokePoint::new(28.0, 26.0, 1.0)];
            let pre = Surface::new(format);
            let mut expected = pre.clone();
            let mut actual = pre.clone();
            let mut native = StrokeRenderer::new(&brush, Some(format), 1.0);
            native.push(&points);
            native.finish();
            native.composite(&pre, &mut expected, None, false, true);
            let mut mapped = StrokeRenderer::new(&brush, Some(format), 1.0);
            let context = BrushContext::new(&brush);
            for dab in dabs(&Stroke { brush: brush.clone(), points: points.clone() }) {
                let rect = context.dab_rect(&dab, false);
                let (mut coverage, mut depth) = (Vec::new(), Vec::new());
                context.rasterize_into(&dab, false, rect, &mut coverage, Some(&mut depth));
                assert!(mapped.accumulate_mapped(rect, &coverage, &vec![0.0; coverage.len()], &depth, &dab));
            }
            mapped.composite(&pre, &mut actual, None, false, true);
            assert_eq!(expected.read_region(Rect::new(0, 0, 64, 64)), actual.read_region(Rect::new(0, 0, 64, 64)), "{depth:?} wet={wet}");
        }
    }
}

#[test]
fn mapped_batch_rejects_invalid_samples_before_changing_coverage() {
    let format = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let brush = BrushSettings { size: 5.0, ..Default::default() };
    let dab = dabs(&Stroke { brush: brush.clone(), points: vec![StrokePoint::new(0.0, 0.0, 1.0)] }).remove(0);
    let rect = Rect::from_xywh(0, 0, 2, 2);
    let coverage = [1.0; 4];
    let zeros = [0.0; 4];
    let invalid = [f32::NAN, 0.0, 0.0, 0.0];
    let batch = [
        photocraft_paint::MappedDab { rect, coverage: &coverage, dual_coverage: &zeros, depth: &zeros, dab: &dab },
        photocraft_paint::MappedDab { rect, coverage: &invalid, dual_coverage: &zeros, depth: &zeros, dab: &dab },
    ];
    let mut renderer = StrokeRenderer::new(&brush, Some(format), 1.0);
    assert!(!renderer.accumulate_mapped_batch(&batch));
    assert!(renderer.bounds().is_empty());
    assert!(renderer.accumulate_mapped_batch(&[batch[0]]));
    assert!(!renderer.bounds().is_empty());
}
