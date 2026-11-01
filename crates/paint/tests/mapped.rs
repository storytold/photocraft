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
