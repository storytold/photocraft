use photocraft_color::{PixelFormat, SampleType};
use photocraft_gpu::{DeviceHealth, brush::Rasterizer};
use photocraft_paint::{BrushSettings, DualBrush, Dynamic, GrayTile, ShapeDynamics, StrokePoint, StrokeRenderer, TipShape};
use photocraft_raster::Surface;
use std::sync::Arc;
use std::time::Instant;

fn backend() -> Option<(Arc<Rasterizer>, DeviceHealth)> {
    let adapter = pollster::block_on(wgpu::Instance::default().request_adapter(&Default::default())).ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    let health = DeviceHealth::watch(&device);
    let mut raster = Rasterizer::new(device, queue, health.clone());
    raster.min_work = 0;
    Some((Arc::new(raster), health))
}

#[test]
fn compute_brush_matches_cpu_and_preserves_native_depth() {
    let Some((gpu, health)) = backend() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let base = BrushSettings {
        size: 320.0,
        hardness: 0.0,
        pressure_size: false,
        spacing: 0.1,
        flow: 0.23,
        opacity: 0.65,
        color: [0.3, 0.7, 0.9, 1.0],
        ..Default::default()
    };
    for brush in [
        base.clone(),
        BrushSettings {
            hardness: 1.0,
            roundness: 0.3,
            angle: 42.0,
            wet_edges: true,
            shape_dynamics: ShapeDynamics { enabled: true, angle: Dynamic::jitter(0.7), ..Default::default() },
            ..base.clone()
        },
        BrushSettings { aliased: true, ..base.clone() },
        BrushSettings { dual_brush: DualBrush { enabled: true, size: 300.0, hardness: 0.7, ..Default::default() }, ..base.clone() },
        BrushSettings { shape_dynamics: ShapeDynamics { enabled: true, brush_projection: true, ..Default::default() }, ..base.clone() },
    ] {
        for fmt in [
            PixelFormat::RGBA8,
            PixelFormat::RGBA8.with_sample(SampleType::U16),
            PixelFormat::RGBA32F,
            PixelFormat::CMYKA8,
            PixelFormat::GRAYA8.with_sample(SampleType::U16),
        ] {
            let mut cpu = StrokeRenderer::new(&brush, Some(fmt), 1.0);
            let mut accelerated = StrokeRenderer::new(&brush, Some(fmt), 1.0).with_accelerator(Some(gpu.clone()));
            for point in [StrokePoint::new(-17.25, -32.75, 1.0), StrokePoint::new(70.125, 50.25, 1.0), StrokePoint::new(230.5, 80.5, 1.0)] {
                let point = StrokePoint { tilt_x: 45.0, tilt_y: 20.0, ..point };
                cpu.push(&[point]);
                accelerated.push(&[point]);
            }
            cpu.finish();
            accelerated.finish();
            let (bounds, a) = cpu.dense_coverage();
            let (gpu_bounds, b) = accelerated.dense_coverage();
            assert_eq!(bounds, gpu_bounds);
            let error = a.iter().zip(&b).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            eprintln!("{fmt:?} aliased={} hard={} max coverage error={error}", brush.aliased, brush.hardness);
            assert!(error <= 3e-5);
            let mut color = vec![0.0; fmt.channels()];
            photocraft_raster::from_rgba_into(&fmt, [0.1, 0.2, 0.3, 0.7], &mut color);
            let pre = Surface::with_default(fmt, &color);
            let mut expected = pre.clone();
            let mut actual = pre.clone();
            cpu.composite(&pre, &mut expected, None, false, true);
            accelerated.composite(&pre, &mut actual, None, false, true);
            let a = expected.read_region(bounds);
            let b = actual.read_region(bounds);
            let error = a.iter().zip(&b).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            let tolerance = match fmt.sample {
                SampleType::U8 => 1.0 / 255.0 + f32::EPSILON,
                SampleType::U16 => 2.0 / 65535.0,
                _ => 3e-5,
            };
            assert!(error <= tolerance, "{fmt:?} native error {error}");
        }
    }
    assert!(gpu.batches() > 0, "must exercise compute, not CPU fallback");
    let before = gpu.batches();
    for brush in [
        BrushSettings { noise: true, ..base.clone() },
        BrushSettings { wet_edges: true, ..base.clone() },
        BrushSettings { tip: TipShape::Sampled(GrayTile::from_fn(8, 8, |_, _| 1.0)), ..base },
    ] {
        let mut r = StrokeRenderer::new(&brush, None, 1.0).with_accelerator(Some(gpu.clone()));
        r.push(&[StrokePoint::new(0.0, 0.0, 1.0)]);
        r.finish();
    }
    assert_eq!(before, gpu.batches(), "unsupported tips retain CPU rendering");
    health.mark(photocraft_gpu::Fault::Lost("simulated".into()));
    let brush = BrushSettings { size: 1000.0, ..Default::default() };
    let mut r = StrokeRenderer::new(&brush, None, 1.0).with_accelerator(Some(gpu.clone()));
    r.push(&[StrokePoint::new(0.0, 0.0, 1.0)]);
    r.finish();
    assert!(r.coverage_at(0, 0) > 0.0);
    assert_eq!(before, gpu.batches(), "lost devices must use the CPU");
}

#[test]
#[ignore = "release hardware benchmark"]
fn completed_compute_brush_benchmark() {
    let Some((gpu, _)) = backend() else {
        eprintln!("skipping: no GPU adapter");
        return;
    };
    let brush = BrushSettings { size: 1000.0, pressure_size: false, spacing: 0.05, flow: 0.3, ..Default::default() };
    for count in [1, 4, 24] {
        let points: Vec<_> = (0..count).map(|i| StrokePoint::new(500.25 + f64::from(i) * 50.0, 500.75, 1.0)).collect();
        // Prime pipeline and pooled buffers before comparison.
        let mut warm = StrokeRenderer::new(&brush, None, 1.0).with_accelerator(Some(gpu.clone()));
        warm.push(&points);
        warm.finish();
        for enabled in [false, true] {
            let mut elapsed = Vec::new();
            for _ in 0..10 {
                let mut r = StrokeRenderer::new(&brush, None, 1.0).with_accelerator(if enabled { Some(gpu.clone()) } else { None });
                let start = Instant::now();
                r.push(&points);
                r.finish();
                elapsed.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            elapsed.sort_by(f64::total_cmp);
            eprintln!("{count} points GPU={enabled}: median={:.3} p90={:.3} ms (transfers and completed work included)", elapsed[5], elapsed[9]);
        }
    }
}
