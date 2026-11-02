//! Tests for bounded, checked band readback across chunks, pages, and device failure modes.

use std::sync::Arc;

use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Effect, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_gpu::context::{NativeContext, NativeContextOptions};
use photocraft_gpu::readback::{ReadbackError, render_bands, render_to_vec};
use photocraft_gpu::{Compositor, DeviceHealth};
use photocraft_raster::Surface;

use crate::GPU_LOCK;

fn test_gpu() -> Option<(NativeContext, Compositor, std::sync::MutexGuard<'static, ()>)> {
    let lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let ctx = match NativeContext::request(&NativeContextOptions::default()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("skipping bounded readback test: {e}");
            return None;
        }
    };
    let comp = match ctx.create_compositor() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("skipping bounded readback test: {e}");
            return None;
        }
    };
    Some((ctx, comp, lock))
}

fn sample_doc(w: u32, h: u32) -> Document {
    let mut doc = Document::with_background("sample", Size::new(w, h), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut s = Surface::new(doc.pixel_format());
    s.fill_rect(Rect::new(10, 10, w as i32 - 10, h as i32 - 10), &[0.2, 0.6, 0.8, 0.7]);
    let mut top = Layer::new("top", LayerContent::Raster(s));
    top.opacity = 0.85;
    top.blend = BlendMode::Multiply;
    doc.layers.push(top);
    doc
}

#[test]
fn bounded_readback_small_staging_matches_full_readback() {
    let Some((ctx, mut comp, _lock)) = test_gpu() else { return };
    let doc = sample_doc(200, 300);
    let rect = doc.bounds();

    let full = render_to_vec(&mut comp, ctx.device(), ctx.queue(), &doc, rect).expect("full readback");

    // Force small bands with small staging budget (e.g. 16 KiB = 1024 RGBA32F pixels = ~5 rows of 200 px)
    let mut banded = Vec::new();
    let mut total_rows = 0;
    render_bands(
        &mut comp,
        ctx.device(),
        ctx.queue(),
        &doc,
        rect,
        8,         // 8 rows per band
        16 * 1024, // 16 KiB max staging bytes
        |band_rect, px| {
            total_rows += band_rect.height();
            banded.extend_from_slice(&px);
            Ok::<(), std::convert::Infallible>(())
        },
    )
    .expect("banded readback");

    assert_eq!(total_rows, rect.height());
    assert_eq!(full.len(), banded.len());
    for (i, (f, b)) in full.iter().zip(&banded).enumerate() {
        for c in 0..4 {
            assert!((f[c] - b[c]).abs() < 1e-4, "pixel {i} channel {c} differs: full={}, banded={}", f[c], b[c]);
        }
    }
}

#[test]
fn bounded_readback_preserves_effect_halos_across_seams() {
    let Some((ctx, mut comp, _lock)) = test_gpu() else { return };
    let (w, h) = (256, 256);
    let mut doc = Document::with_background("fx_doc", Size::new(w, h), ColorMode::Rgb, SampleType::U8, Color::WHITE);

    // Layer positioned right across row 128 (the middle of the canvas)
    let mut s = Surface::new(doc.pixel_format());
    s.fill_rect(Rect::new(50, 110, 200, 145), &[0.9, 0.1, 0.1, 1.0]);
    let mut layer = Layer::new("shadow_layer", LayerContent::Raster(s));

    // Drop shadow with blur radius 16 extending downward and upward
    let mut shadow = photocraft_doc::DropShadow::default();
    shadow.distance = 8.0;
    shadow.blur = 16.0;
    shadow.opacity = 0.8;
    layer.effects.items.push(Effect::DropShadow(shadow));
    doc.layers.push(layer);

    let rect = doc.bounds();
    let full = render_to_vec(&mut comp, ctx.device(), ctx.queue(), &doc, rect).expect("full readback");

    // Render with bands of 32 rows so the shadow seamlessly crosses multiple band boundaries
    let mut banded = Vec::new();
    render_bands(&mut comp, ctx.device(), ctx.queue(), &doc, rect, 32, 32 * 1024, |_band_rect, px| {
        banded.extend_from_slice(&px);
        Ok::<(), std::convert::Infallible>(())
    })
    .expect("banded readback with effects");

    assert_eq!(full.len(), banded.len());
    for (i, (f, b)) in full.iter().zip(&banded).enumerate() {
        for c in 0..4 {
            assert!((f[c] - b[c]).abs() < 2.0 / 255.0, "halo pixel {i} channel {c} differs: full={}, banded={}", f[c], b[c]);
        }
    }
}

#[test]
fn bounded_readback_handles_empty_rect() {
    let Some((ctx, mut comp, _lock)) = test_gpu() else { return };
    let doc = sample_doc(100, 100);
    let mut called = false;
    let res = render_bands(&mut comp, ctx.device(), ctx.queue(), &doc, Rect::new(0, 0, 0, 0), 16, 64 * 1024, |_rect, _px| {
        called = true;
        Ok::<(), std::convert::Infallible>(())
    });
    assert!(res.is_ok());
    assert!(!called, "sink should not be called for empty rect");
}

#[test]
fn bounded_readback_notices_device_fault() {
    let Some((ctx, mut comp, _lock)) = test_gpu() else { return };
    let doc = sample_doc(100, 100);

    // Simulate an explicit device fault
    let health = DeviceHealth::new();
    health.mark(photocraft_gpu::Fault::Lost("simulated driver crash".into()));
    comp.set_health(health);

    let res = render_bands(&mut comp, ctx.device(), ctx.queue(), &doc, doc.bounds(), 16, 64 * 1024, |_rect, _px| Ok::<(), std::convert::Infallible>(()));

    assert!(matches!(res, Err(ReadbackError::DeviceLost(_))));
}
