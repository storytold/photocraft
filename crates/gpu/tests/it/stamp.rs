//! Dab stamping into a GPU-owned layer mask (the preview-only prototype of #17133): the dabs a
//! stroke makes are drawn into a texture the compositor owns, and that texture is the layer's mask.
//! The CPU's coverage (`StrokeRenderer`) is the reference, so every comparison here feeds both the
//! same dabs.

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Color, Document, Fill, Layer, LayerContent, LayerId, LayerMask, Size};
use photocraft_geom::Rect;
use photocraft_gpu::{Compositor, GpuDab, MaskPrecision};
use photocraft_paint::{BrushContext, BrushSettings, Dab, StrokePoint, StrokeRenderer, TipGeometry};
use photocraft_raster::Surface;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    comp: Compositor,
}

/// The default adapter with R32F blending where it has it, and a compositor paging at the tile
/// size, so a stroke crosses several cells.
fn gpu() -> Option<Gpu> {
    let adapter = pollster::block_on(crate::instance().request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    let features = adapter.features() & wgpu::Features::FLOAT32_BLENDABLE;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { required_features: features, ..Default::default() })).ok()?;
    let mut comp = Compositor::try_new_with_format(&device, Compositor::preferred_acc_format(&adapter)).ok()?;
    comp.set_texture_limit(256);
    Some(Gpu { device, queue, comp })
}

fn gpu_dab(g: &TipGeometry) -> GpuDab {
    GpuDab {
        rect: g.rect,
        cx: g.cx,
        cy: g.cy,
        ro: g.ro,
        core: g.core,
        shoulder: g.shoulder,
        reach2: g.reach2,
        small_reach2: g.small_reach2,
        cs: g.cs,
        sn: g.sn,
        fx: g.fx,
        fy: g.fy,
        proj: g.proj,
        small: g.small,
        flow: g.flow,
        ceil: g.ceil,
    }
}

const W: i32 = 640;
const H: i32 = 480;

/// A wavy path across the canvas, with `n` points a little under a brush diameter apart.
fn path(n: usize) -> Vec<StrokePoint> {
    (0..n).map(|i| StrokePoint::new(40.0 + i as f64 * 560.0 / n as f64, 240.0 + (i as f64 * 0.31).sin() * 150.0, 1.0)).collect()
}

/// The stroke's dabs and its CPU coverage over the canvas.
fn cpu_stroke(brush: &BrushSettings, pts: &[StrokePoint]) -> (Vec<Dab>, Vec<f32>) {
    let mut renderer = StrokeRenderer::new(brush, None, 1.0).record_dabs();
    renderer.push(pts);
    renderer.finish();
    (renderer.dabs().to_vec(), renderer.coverage_in(Rect::new(0, 0, W, H)))
}

/// A document with a stroke colour layer above the background, its mask empty.
fn document(opacity: f32) -> (Document, LayerId) {
    let mut doc = Document::with_background("stamp", Size::new(W as u32, H as u32), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut layer = Layer::new("Stroke", LayerContent::Fill(Fill::Solid(Color::rgba(0.9, 0.2, 0.1, 1.0))));
    layer.mask = Some(LayerMask { surface: Surface::with_default(PixelFormat::GRAY8, &[0.0]), enabled: true, linked: true, density: 1.0, feather: 0.0 });
    layer.opacity = opacity;
    let id = layer.id;
    doc.insert_above(None, layer);
    (doc, id)
}

/// Stamp `dabs` a few at a time, each call submitted on its own as a live stroke's frames are.
fn stamp(g: &mut Gpu, layer: LayerId, dabs: &[GpuDab]) -> usize {
    let mut created = 0;
    for chunk in dabs.chunks(7) {
        let mut encoder = g.device.create_command_encoder(&Default::default());
        let stats = g.comp.stamp_dabs(&g.device, &g.queue, &mut encoder, layer, chunk).expect("stamping");
        g.queue.submit([encoder.finish()]);
        created += stats.cells_created;
    }
    created
}

/// A mask texture's values (R32F or R16F), row-major over `rect`.
fn read(g: &Gpu, texture: &wgpu::Texture, rect: Rect) -> Vec<f32> {
    let (w, h) = (rect.width(), rect.height());
    let half = texture.format() == wgpu::TextureFormat::R16Float;
    let row = (w * if half { 2 } else { 4 }).next_multiple_of(256);
    let buffer = g.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("stamp readback"),
        size: u64::from(row * h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = g.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    g.queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = send.send(r);
    });
    g.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::from_secs(30)) }).unwrap();
    receive.recv_timeout(std::time::Duration::from_secs(1)).unwrap().unwrap();
    let data = buffer.slice(..).get_mapped_range().unwrap();
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h as usize {
        for x in 0..w as usize {
            out.push(if half {
                let at = y * row as usize + x * 2;
                photocraft_gpu::f16_to_f32(u16::from_le_bytes([data[at], data[at + 1]]))
            } else {
                let at = y * row as usize + x * 4;
                f32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
            });
        }
    }
    out
}

/// The external mask over the canvas (0 where no cell exists).
fn mask_of(g: &Gpu, layer: LayerId) -> Vec<f32> {
    let mut out = vec![0.0f32; (W * H) as usize];
    for (rect, texture) in g.comp.external_mask_cells(layer) {
        let values = read(g, texture, rect);
        for (i, v) in values.into_iter().enumerate() {
            let (x, y) = (rect.x0 + (i as u32 % rect.width()) as i32, rect.y0 + (i as u32 / rect.width()) as i32);
            // A cell reaches to the tile grid, past the canvas.
            if x < W && y < H {
                out[(y * W + x) as usize] = v;
            }
        }
    }
    out
}

/// The worst difference between the stamped mask and the CPU's coverage of the same dabs, and the
/// precision it was stored in.
fn worst_difference(g: &mut Gpu, brush: &BrushSettings, pts: &[StrokePoint], precision: Option<MaskPrecision>) -> (f32, MaskPrecision) {
    let (doc, layer) = document(1.0);
    let used = g.comp.begin_external_mask(&g.device, doc.id, layer, doc.bounds(), precision);
    let (dabs, cpu) = cpu_stroke(brush, pts);
    let ctx = BrushContext::new(brush);
    let gpu_dabs: Vec<GpuDab> = dabs.iter().filter_map(|d| ctx.tip_geometry(d)).map(|t| gpu_dab(&t)).collect();
    assert_eq!(gpu_dabs.len(), dabs.len(), "every dab of a plain brush is stampable");
    stamp(g, layer, &gpu_dabs);
    let mask = mask_of(g, layer);
    g.comp.clear_external_mask(layer);
    (mask.iter().zip(&cpu).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max), used)
}

fn precisions(g: &Gpu) -> Vec<MaskPrecision> {
    let mut all = vec![MaskPrecision::Half];
    if Compositor::full_precision_masks(&g.device) {
        all.push(MaskPrecision::Full);
    }
    all
}

/// Plain round tips, 3 to 300 px, soft to hard, round and squashed and turned, low flow and high,
/// many overlapping dabs: the stamped mask is the CPU's coverage within the precision's rounding.
#[test]
fn stamped_coverage_matches_the_cpu() {
    let _lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(mut g) = gpu() else { return };
    // size, hardness, roundness, angle (degrees), flow, spacing, points
    let cases = [
        (3.0, 0.0, 1.0, 0.0, 1.0, 0.25, 160),
        (3.0, 1.0, 1.0, 0.0, 0.5, 0.25, 160),
        (8.0, 0.5, 0.5, 30.0, 1.0, 0.1, 160),
        (20.0, 0.0, 1.0, 0.0, 0.2, 0.1, 120),
        (40.0, 0.5, 0.6, 40.0, 1.0, 0.1, 80),
        (40.0, 1.0, 1.0, 0.0, 0.3, 0.05, 80),
        (100.0, 0.0, 1.0, 0.0, 1.0, 0.1, 40),
        (100.0, 0.5, 0.4, 75.0, 0.6, 0.1, 40),
        (300.0, 0.0, 1.0, 0.0, 1.0, 0.1, 24),
        (300.0, 1.0, 0.7, 20.0, 0.1, 0.05, 24),
    ];
    for (size, hardness, roundness, angle, flow, spacing, n) in cases {
        let brush = BrushSettings { size, hardness, roundness, angle, flow, spacing, ..BrushSettings::default() };
        for precision in precisions(&g) {
            let (worst, used) = worst_difference(&mut g, &brush, &path(n), Some(precision));
            assert_eq!(used, precision);
            // R32F: only the shader's float arithmetic against the rasteriser's (a few 1e-6 on an
            // RTX 4060, 6e-8 on lavapipe, tip profile and a hundred overlapping blends included).
            // R16F: each blend rounds to a half (an 11-bit significand, a step of 2^-11 near 1),
            // so the error is the sum of those roundings over the dabs that overlap a pixel,
            // about 1 / spacing of them. A GPU rounding to nearest stays under 2.2e-3 (4 dabs per
            // pixel and more); lavapipe measured 6.6e-3 at 10 (a step and a third per dab, as if
            // it rounded down). The limit allows two steps per overlapping dab.
            let limit = match precision {
                MaskPrecision::Full => 2e-4,
                MaskPrecision::Half => 2.0 * (1.0 / 2048.0) / spacing,
            };
            eprintln!("stamp size {size} hardness {hardness} roundness {roundness} flow {flow} {precision:?}: worst difference {worst:.2e}");
            assert!(
                worst <= limit,
                "size {size} hardness {hardness} roundness {roundness} angle {angle} flow {flow} {precision:?}: worst difference {worst:.2e}"
            );
        }
    }
}

/// The live document with the external mask, rendered on the GPU, is the document whose mask holds
/// the same stroke's strength (`min(coverage × opacity, 1)`, what the CPU's live overlay writes).
#[test]
fn the_live_document_with_an_external_mask_matches_the_cpu_overlay() {
    let _lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(mut g) = gpu() else { return };
    let opacity = 0.6;
    let brush = BrushSettings { size: 60.0, hardness: 0.3, flow: 0.5, spacing: 0.1, opacity, ..BrushSettings::default() };
    let (dabs, cpu) = cpu_stroke(&brush, &path(60));
    let (live, layer) = document(opacity);
    g.comp.begin_external_mask(&g.device, live.id, layer, live.bounds(), None);
    let ctx = BrushContext::new(&brush);
    let gpu_dabs: Vec<GpuDab> = dabs.iter().filter_map(|d| ctx.tip_geometry(d)).map(|t| gpu_dab(&t)).collect();
    stamp(&mut g, layer, &gpu_dabs);
    let shown = photocraft_gpu::render_to_vec(&mut g.comp, &g.device, &g.queue, &live, live.bounds()).expect("the live document renders");

    // The CPU overlay: the layer at full opacity through a mask holding the stroke's strength.
    let (mut overlay, id) = document(1.0);
    let strength: Vec<f32> = cpu.iter().map(|c| (c * opacity).min(1.0)).collect();
    let mask = overlay.layer_mut(id).and_then(|l| l.mask.as_mut()).expect("the colour layer has a mask");
    mask.surface.write_region(Rect::new(0, 0, W, H), &strength);
    let expected = photocraft_compose::flatten(&overlay);

    let worst = shown
        .iter()
        .zip(&expected.px)
        .map(|(a, b)| (0..4).map(|k| if k == 3 { (a[3] - b[3]).abs() } else { (a[k] * a[3] - b[k] * b[3]).abs() }).fold(0.0f32, f32::max))
        .fold(0.0f32, f32::max);
    assert!(worst <= 3.0 / 255.0, "the live document differs from the overlay by {:.2}/255", worst * 255.0);
    // The stroke is on screen, not the empty document mask.
    assert!(shown.iter().any(|p| p[1] < 0.7), "the stroke did not show");
    g.comp.clear_external_mask(layer);
}

/// A render never writes an external mask: its pages are not uploaded to, cleared or evicted, the
/// document's mask surface (empty, or holding something else) says nothing about them, and a
/// layer whose document mask is empty still shows the stamped pixels.
#[test]
fn a_render_leaves_the_external_mask_alone() {
    let _lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(mut g) = gpu() else { return };
    let brush = BrushSettings { size: 50.0, hardness: 0.0, ..BrushSettings::default() };
    let (empty, layer) = document(1.0);
    g.comp.begin_external_mask(&g.device, empty.id, layer, empty.bounds(), None);
    let (dabs, _) = cpu_stroke(&brush, &path(30));
    let ctx = BrushContext::new(&brush);
    let gpu_dabs: Vec<GpuDab> = dabs.iter().filter_map(|d| ctx.tip_geometry(d)).map(|t| gpu_dab(&t)).collect();
    stamp(&mut g, layer, &gpu_dabs);
    let before = mask_of(&g, layer);
    assert!(before.iter().any(|v| *v > 0.5), "the stroke stamped something");

    // An empty mask surface: the stroke shows through the external texture.
    let first = photocraft_gpu::render_to_vec(&mut g.comp, &g.device, &g.queue, &empty, empty.bounds()).expect("renders");
    let (_, again) = photocraft_gpu::render_to_vec_stats(&mut g.comp, &g.device, &g.queue, &empty, empty.bounds()).expect("renders again");
    assert_eq!(again.tiles_uploaded, 0, "a second render of an unchanged document uploads nothing");
    assert_eq!(mask_of(&g, layer), before);
    let at = |out: &[[f32; 4]], x: i32, y: i32| out[(y * W + x) as usize];
    let inked = before.iter().position(|v| *v > 0.99).expect("a fully covered pixel");
    let (ix, iy) = (inked as i32 % W, inked as i32 / W);
    assert!(at(&first, ix, iy)[1] < 0.5, "the covered pixel shows the stroke colour");
    assert!(at(&first, W - 1, H - 1)[1] > 0.99, "an uncovered pixel stays background");

    // A mask surface that holds something else, away from the stroke, changes nothing.
    let mut changed = empty.clone();
    changed.layer_mut(layer).and_then(|l| l.mask.as_mut()).expect("a mask").surface.fill_rect(Rect::new(W - 100, H - 100, W, H), &[1.0]);
    let second = photocraft_gpu::render_to_vec(&mut g.comp, &g.device, &g.queue, &changed, changed.bounds()).expect("renders");
    assert_eq!(mask_of(&g, layer), before);
    assert!(at(&second, W - 1, H - 1)[1] > 0.99, "the document mask's own pixels are not the external mask's");
    assert_eq!(at(&second, ix, iy), at(&first, ix, iy));
}

/// Cells exist only where dabs reach: a stroke inside one cell makes one texture, one across
/// several makes each it touches; clearing the mask frees them, and the precision follows the
/// device.
#[test]
fn cells_are_made_as_dabs_reach_them() {
    let _lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(mut g) = gpu() else { return };
    let (doc, layer) = document(1.0);
    let used = g.comp.begin_external_mask(&g.device, doc.id, layer, doc.bounds(), None);
    assert_eq!(used == MaskPrecision::Full, Compositor::full_precision_masks(&g.device));
    assert_eq!(g.comp.begin_external_mask(&g.device, doc.id, layer, doc.bounds(), Some(MaskPrecision::Half)), MaskPrecision::Half);
    assert!(g.comp.has_external_mask(layer));
    let brush = BrushSettings { size: 20.0, ..BrushSettings::default() };
    let ctx = BrushContext::new(&brush);
    let dab_at = |x: f64, y: f64| {
        let d = Dab::round(photocraft_geom::Point { x, y }, 10.0, 1.0);
        gpu_dab(&ctx.tip_geometry(&d).expect("plain"))
    };
    // 256-px cells: the first dab is inside cell (0, 0).
    assert_eq!(stamp(&mut g, layer, &[dab_at(100.0, 100.0)]), 1);
    assert_eq!(g.comp.external_mask_cells(layer).len(), 1);
    // One on the corner of four cells reaches all of those on the canvas.
    assert_eq!(stamp(&mut g, layer, &[dab_at(256.0, 256.0)]), 3);
    assert_eq!(g.comp.external_mask_cells(layer).len(), 4);
    // Outside the canvas: nothing.
    assert_eq!(stamp(&mut g, layer, &[dab_at(-500.0, -500.0), dab_at(5000.0, 40.0)]), 0);
    assert_eq!(g.comp.external_mask_cells(layer).len(), 4);
    g.comp.clear_external_mask(layer);
    assert!(g.comp.external_mask_cells(layer).is_empty());
    assert!(!g.comp.has_external_mask(layer));
}

/// Requested coverage tiles come back with the values stamped, from tiles in several page cells,
/// in both precisions (R16F converted to `f32`); a tile no texture holds reads as zeros; and the
/// queue holds a bounded number of buffers.
#[test]
fn requested_tiles_come_back_with_the_stamped_values() {
    let _lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(mut g) = gpu() else { return };
    let brush = BrushSettings { size: 70.0, hardness: 0.0, spacing: 0.1, ..BrushSettings::default() };
    let (dabs, _) = cpu_stroke(&brush, &path(50));
    let ctx = BrushContext::new(&brush);
    let gpu_dabs: Vec<GpuDab> = dabs.iter().filter_map(|d| ctx.tip_geometry(d)).map(|t| gpu_dab(&t)).collect();
    for precision in precisions(&g) {
        let (doc, layer) = document(1.0);
        g.comp.begin_external_mask(&g.device, doc.id, layer, doc.bounds(), Some(precision));
        stamp(&mut g, layer, &gpu_dabs);
        let mask = mask_of(&g, layer);
        let mut keys: Vec<(i32, i32)> = (0..(W + 63) / 64).flat_map(|x| (0..(H + 63) / 64).map(move |y| (x, y))).collect();
        keys.push((500, 500));
        let readback = photocraft_gpu::MaskReadback::new();
        let mut pending = keys.clone();
        let mut tiles = Vec::new();
        while tiles.len() < keys.len() {
            let taken = readback.request(&g.comp, &g.device, &g.queue, layer, &pending, 7);
            pending.drain(..taken);
            assert!(readback.in_flight() <= photocraft_gpu::MAX_IN_FLIGHT + 1);
            tiles.extend(readback.poll(&g.device, true));
        }
        assert_eq!(tiles.len(), keys.len());
        assert!(readback.bytes_requested() > 0);
        let mut seen = 0;
        for ((tx, ty), values, tag) in tiles {
            assert_eq!(tag, 7, "the tag of the request comes back");
            assert_eq!(values.len(), 64 * 64);
            if tx == 500 {
                assert!(values.iter().all(|v| *v == 0.0), "no texture, no coverage");
                continue;
            }
            for (i, v) in values.iter().enumerate() {
                let (x, y) = (tx * 64 + (i % 64) as i32, ty * 64 + (i / 64) as i32);
                if x < W && y < H {
                    assert_eq!(*v, mask[(y * W + x) as usize], "{precision:?} tile ({tx}, {ty}) pixel ({x}, {y})");
                    seen += 1;
                }
            }
        }
        assert_eq!(seen, (W * H) as usize, "every pixel of the canvas came back");
        g.comp.clear_external_mask(layer);
    }
}

/// `request_all` asks for every key whatever is in flight: more staging buffers than the capped
/// request allows are out at once, and every tile still comes back with the request's tag.
#[test]
fn request_all_goes_past_the_in_flight_cap() {
    let _lock = crate::GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(mut g) = gpu() else { return };
    let brush = BrushSettings { size: 70.0, hardness: 0.0, spacing: 0.1, ..BrushSettings::default() };
    let (dabs, _) = cpu_stroke(&brush, &path(20));
    let ctx = BrushContext::new(&brush);
    let gpu_dabs: Vec<GpuDab> = dabs.iter().filter_map(|d| ctx.tip_geometry(d)).map(|t| gpu_dab(&t)).collect();
    let (doc, layer) = document(1.0);
    g.comp.begin_external_mask(&g.device, doc.id, layer, doc.bounds(), None);
    stamp(&mut g, layer, &gpu_dabs);
    let tiles: Vec<(i32, i32)> = (0..(W + 63) / 64).flat_map(|x| (0..(H + 63) / 64).map(move |y| (x, y))).collect();
    // More keys than the capped request would take at once: each of MAX_IN_FLIGHT + 2 buffers full.
    let keys: Vec<(i32, i32)> = tiles.iter().copied().cycle().take(photocraft_gpu::MAX_TILES_PER_BUFFER * (photocraft_gpu::MAX_IN_FLIGHT + 2)).collect();
    let readback = photocraft_gpu::MaskReadback::new();
    readback.request_all(&g.comp, &g.device, &g.queue, layer, &keys, 9);
    assert!(readback.in_flight() > photocraft_gpu::MAX_IN_FLIGHT, "every buffer is out at once: {}", readback.in_flight());
    let back = readback.poll(&g.device, true);
    assert_eq!(back.len(), keys.len(), "every tile came back");
    assert!(back.iter().all(|(_, values, tag)| *tag == 9 && values.len() == 64 * 64));
    assert_eq!(readback.in_flight(), 0);
    g.comp.clear_external_mask(layer);
}
