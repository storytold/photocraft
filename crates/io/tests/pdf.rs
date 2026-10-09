use hayro::{RenderCache, RenderSettings, hayro_interpret::InterpreterSettings, hayro_syntax::Pdf};
use photocraft_doc::{Artboard, ColorMode, Document, Group, Layer, LayerContent, PixelFormat, Rect, SampleType, Size, Surface};
use photocraft_io::{ExportOptions, XmpEmbed, export, import};
use std::sync::Arc;

fn document() -> Document {
    let mut doc = Document::new("pages", Size::new(144, 176), ColorMode::Rgb, SampleType::U8);
    doc.resolution_dpi = 144.0;
    for (i, color) in [[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]].iter().enumerate() {
        let y = i as i32 * 104;
        let rect = Rect::new(0, y, 144, y + 72);
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
        surface.write_region(rect, &color.repeat(144 * 72));
        let image = Layer::new("image", LayerContent::Raster(surface));
        doc.layers.push(Layer::new(
            format!("Page {}", i + 1),
            LayerContent::Group(Group { children: vec![image], expanded: true, artboard: Some(Artboard::new(rect)) }),
        ));
    }
    doc.layers.reverse();
    doc
}

#[test]
fn selected_pdf_pages_skip_unselected_pages_and_preserve_page_number() {
    let bytes = export(&document(), "pdf", &ExportOptions::default()).unwrap().bytes;
    let ctl = photocraft_raster::Interrupt::default();
    let result = photocraft_io::pdf::import_pdf_pages("binder.pdf", &bytes, Some(&[1]), &ctl).unwrap();
    assert_eq!(result.document.artboards().len(), 1);
    assert_eq!(result.document.layers[0].name, "Page 2");
    assert_eq!(result.document.size, Size::new(144, 72));
    let pixel = photocraft_compose::render(&result.document, Rect::new(20, 20, 21, 21)).px[0];
    assert!(pixel[2] > 0.97 && pixel[0] < 0.03);
    assert!(photocraft_io::pdf::import_pdf_pages("binder.pdf", &bytes, Some(&[]), &ctl).is_err());
    assert!(photocraft_io::pdf::import_pdf_pages("binder.pdf", &bytes, Some(&[2]), &ctl).is_err());
    let sizes = photocraft_io::pdf::page_sizes(&bytes).unwrap();
    assert_eq!(sizes, [(72.0, 36.0), (72.0, 36.0)]);
    let (w, h, pixels) = photocraft_io::pdf::page_preview(Arc::new(bytes), 1).unwrap();
    assert_eq!(pixels.len(), (w * h * 4) as usize);
}

#[test]
fn pdf_all_pages_order_color_size_and_magic_round_trip() {
    let output = export(&document(), "pdf", &ExportOptions::default()).unwrap();
    let parsed = Pdf::new(Arc::new(output.bytes.clone())).unwrap();
    assert_eq!(parsed.pages().len(), 2);
    for p in parsed.pages().iter() {
        assert_eq!(p.render_dimensions(), (72.0, 36.0));
    }
    let result = import("magic-detection", &output.bytes).unwrap();
    assert!(result.source_read_only);
    assert!(!result.preview_only);
    let doc = result.document;
    assert_eq!(doc.resolution_dpi, 144.0);
    assert_eq!(doc.layers[1].name, "Page 1");
    assert_eq!(doc.layers[0].name, "Page 2");
    assert_eq!(doc.size, Size::new(144, 176));
    let second = export(&doc, "test.PDF", &ExportOptions::default()).unwrap();
    let again = import("again.pdf", &second.bytes).unwrap().document;
    for (i, expected) in [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]].iter().enumerate() {
        let y = i as i32 * 104 + 20;
        let pixel = photocraft_compose::render(&again, Rect::new(20, y, 21, y + 1)).px[0];
        for c in 0..3 {
            assert!((pixel[c] - expected[c]).abs() < 0.03, "page {i}: {pixel:?}");
        }
    }
}

#[test]
fn pdf_malformed_and_oversized_fail_without_panicking() {
    for input in [b"%PDF-".as_slice(), b"not a pdf".as_slice(), b"".as_slice()] {
        assert!(import("bad.pdf", input).is_err());
        assert!(photocraft_io::pdf::page_sizes(input).is_err());
        assert!(photocraft_io::pdf::page_preview(Arc::new(input.to_vec()), 0).is_err());
    }
    let mut doc = document();
    doc.layers.clear();
    doc.size = Size::new(1_000_000, 1_000_000);
    assert!(export(&doc, "pdf", &ExportOptions::default()).is_err());
}

#[test]
fn pdf_cancelled_import_stops() {
    let bytes = export(&document(), "pdf", &ExportOptions::default()).unwrap().bytes;
    let ctl = photocraft_raster::Interrupt::cancel_only(&|| true);
    assert!(matches!(photocraft_io::pdf::import_pdf("test.pdf", &bytes, &ctl), Err(photocraft_io::IoError::Cancelled)));
}

#[test]
fn pdf_rejects_invalid_dimensions_and_resolution() {
    let mut doc = Document::new("test", Size::new(0, 0), ColorMode::Rgb, SampleType::U8);
    assert!(export(&doc, "pdf", &ExportOptions::default()).is_err());
    doc.size = Size::new(10, 10);
    for dpi in [0.0, f32::NAN, f32::INFINITY, -72.0] {
        doc.resolution_dpi = dpi;
        assert!(export(&doc, "pdf", &ExportOptions::default()).is_err());
    }
}

#[test]
fn pdf_all_depths_transparency_and_metadata_control() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut doc = Document::new("transparent", Size::new(20, 10), ColorMode::Rgb, depth);
        doc.resolution_dpi = 72.0;
        doc.metadata.xmp = Some("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">PDF-test-marker</x:xmpmeta>".into());
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
        surface.write_region(doc.bounds(), &[0.0, 1.0, 0.0, 0.5].repeat(200));
        doc.layers.push(Layer::new("half green", LayerContent::Raster(surface)));
        let output = export(&doc, "pdf", &ExportOptions::default()).unwrap();
        assert!(String::from_utf8_lossy(&output.bytes).contains("PDF-test-marker"));
        let output = export(&doc, "pdf", &ExportOptions { xmp: XmpEmbed::None, ..Default::default() }).unwrap();
        assert!(!String::from_utf8_lossy(&output.bytes).contains("PDF-test-marker"));
        let pdf = Pdf::new(Arc::new(output.bytes)).unwrap();
        let pixels = hayro::render(&pdf.pages()[0], &RenderCache::new(), &InterpreterSettings::default(), &RenderSettings::default());
        let pixel = &pixels.data_as_u8_slice()[0..4];
        assert!(pixel[0] < 3 && (125..=131).contains(&pixel[1]) && pixel[2] < 3 && (125..=131).contains(&pixel[3]), "{depth:?}: {pixel:?}");
    }
}
