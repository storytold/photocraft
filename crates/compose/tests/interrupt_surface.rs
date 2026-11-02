use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_compose::{composite_to_surface, composite_to_surface_with_interrupt};
use photocraft_doc::{Document, Layer};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Interrupt;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn controlled_composite_keeps_native_pixels_and_cancels_between_bands() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc = Document::new("bands", Size::new(16, 600), mode, depth);
            let fmt = PixelFormat::new(mode, depth, true);
            let mut layer = Layer::raster("pixels", fmt);
            for y in 0..600 {
                layer.surface_mut().unwrap().fill_rect(Rect::new(0, y, 16, 1), &vec![(y + 1) as f32 / 601.0; fmt.channels()]);
            }
            doc.layers.push(layer);
            let expected = composite_to_surface(&doc, doc.bounds(), fmt, None);
            let actual = composite_to_surface_with_interrupt(&doc, doc.bounds(), fmt, None, &Interrupt::NONE).unwrap();
            assert_eq!(actual.read_region(doc.bounds()), expected.read_region(doc.bounds()), "{mode:?} {depth:?}");
            let calls = AtomicUsize::new(0);
            let cancelled = || calls.fetch_add(1, Ordering::Relaxed) >= 3;
            let progress = |_| {};
            assert!(composite_to_surface_with_interrupt(&doc, doc.bounds(), fmt, None, &Interrupt::new(&cancelled, &progress)).is_err());
        }
    }
}
