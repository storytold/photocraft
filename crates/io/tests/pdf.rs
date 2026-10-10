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

/// Tiny valid PDF container; dimensions/compression belong to the image, not the page.
fn image_fixture(width: u32, height: u32, filter: &str, data: &[u8]) -> Vec<u8> {
    let stream = |dict: &str, data: &[u8]| {
        let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
        out.extend_from_slice(data);
        out.extend_from_slice(b"\nendstream");
        out
    };
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream("", b"q 1 0 0 1 0 0 cm /Im0 Do Q"),
        stream(&format!("/Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceRGB /BitsPerComponent 8 {filter}"), data),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(obj);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    out
}

#[test]
fn oversized_embedded_images_and_unbounded_codecs_fail_before_rendering() {
    // A one-point page may hide a huge image, and codec headers can contradict its dictionary.
    for (w, h, filter, data, expected) in [
        (100_000, 100_000, "", vec![0], "embedded PDF image exceeds"),
        (1, 1, "/Filter /JPXDecode", vec![0, 0, 0, 12, 106, 80, 32, 32], "no bounded decoder"),
        (1, 1, "/Filter /JBIG2Decode", vec![0x97, 0x4a, 0x42, 0x32], "no bounded decoder"),
        (1, 1, "/Filter /J#50XDecode", vec![0], "no bounded decoder"),
        (
            1,
            1,
            "/Filter /DCTDecode",
            vec![0xff, 0xd8, 0xff, 0xc0, 0, 17, 8, 0xff, 0xff, 0xff, 0xff, 3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0],
            "embedded PDF image exceeds",
        ),
    ] {
        let bytes = image_fixture(w, h, filter, &data);
        assert_eq!(photocraft_io::pdf::page_sizes(&bytes).unwrap(), [(1.0, 1.0)]);
        assert!(import("oversized.pdf", &bytes).unwrap_err().to_string().contains(expected));
        assert!(photocraft_io::pdf::page_preview(Arc::new(bytes), 0).unwrap_err().to_string().contains(expected));
    }
}

#[test]
fn pdf_inflation_budget_and_cancellation_are_checked_inside_streams() {
    use std::{
        io::Write,
        sync::atomic::{AtomicUsize, Ordering},
    };
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    for _ in 0..1025 {
        encoder.write_all(&[0; 65536]).unwrap();
    }
    let bytes = image_fixture(1, 1, "/Filter /FlateDecode", &encoder.finish().unwrap());
    assert!(import("bomb.pdf", &bytes).unwrap_err().to_string().contains("decoded stream exceeds"));
    let calls = AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= 20;
    let ctl = photocraft_raster::Interrupt::cancel_only(&cancel);
    assert!(matches!(photocraft_io::pdf::import_pdf("bomb.pdf", &bytes, &ctl), Err(photocraft_io::IoError::Cancelled)));
    assert_eq!(calls.load(Ordering::Relaxed), 21);
}

#[test]
fn pdf_resolution_validation_and_physical_size() {
    use photocraft_io::pdf::{ImportOptions, import_pdf_pages_with};
    let bytes = export(&document(), "pdf", &ExportOptions::default()).unwrap().bytes;
    let ctl = photocraft_raster::Interrupt::default();
    for dpi in [0.0, -1.0, f32::NAN, f32::INFINITY, 2401.0] {
        assert!(import_pdf_pages_with("test.pdf", &bytes, Some(&[0]), ImportOptions { resolution: dpi }, &ctl).is_err());
    }
    for dpi in [72.0, 300.0] {
        let imported = import_pdf_pages_with("test.pdf", &bytes, Some(&[0]), ImportOptions { resolution: dpi }, &ctl).unwrap().document;
        assert_eq!(imported.size, Size::new(dpi as u32, (dpi / 2.0) as u32));
        assert_eq!(imported.resolution_dpi, dpi);
        let output = export(&imported, "pdf", &ExportOptions::default()).unwrap();
        assert_eq!(photocraft_io::pdf::page_sizes(&output.bytes).unwrap(), [(72.0, 36.0)]);
    }
}

#[test]
fn pdf_import_renders_vector_shapes_and_text_without_raster_images() {
    let content = b"1 0 0 rg 0 0 18 36 re f 0 0 0 rg BT /F1 12 Tf 24 12 Td (PDF) Tj ET";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 36] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        [format!("<< /Length {} >>\nstream\n", content.len()).as_bytes(), content, b"\nendstream"].concat(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    let imported = import("text-and-vector.pdf", &bytes).unwrap();
    let page = photocraft_compose::render(&imported.document, imported.document.bounds());
    assert_eq!(imported.document.size, Size::new(144, 72));
    let red = page.px[10 * 144 + 10];
    assert!(red[0] > 0.95 && red[1] < 0.05 && red[2] < 0.05, "vector rectangle: {red:?}");
    assert!(
        page.px.as_chunks::<144>().0.iter().flat_map(|row| &row[48..]).any(|p| p[..3].iter().all(|&v| v < 0.2)),
        "standard PDF text must render as visible glyphs"
    );
}
