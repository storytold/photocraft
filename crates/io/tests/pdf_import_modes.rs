//! A tiny, synthetic vector PDF: red landscape, green portrait, blue landscape pages.
use photocraft_color::SampleType;
use photocraft_io::{
    ImportOptions, import_with_options,
    pdf_import::{CropTo, PdfOpenOptions},
};

const PDF: &[u8] = include_bytes!("fixtures/pdf-three-pages.pdf");

#[test]
fn selected_pdf_pages_are_distinct_layers_in_one_document() {
    for depth in [SampleType::U8, SampleType::U16] {
        for pages in [vec![1, 2, 3], vec![1, 3]] {
            let opts = ImportOptions {
                pdf: Some(PdfOpenOptions { pages_as_layers: Some(pages.clone()), resolution: 72.0, crop: CropTo::MediaBox, depth, ..Default::default() }),
                ..Default::default()
            };
            let result = import_with_options("colors.pdf", PDF, &opts).unwrap();
            let doc = result.document;
            assert_eq!(doc.layers.len(), pages.len());
            assert!(result.source_read_only);
            assert_eq!(doc.size.width, 96);
            for (layer, page) in doc.layers.iter().zip(pages) {
                assert_eq!(layer.name, format!("Page {page}"));
                let pixel = layer.surface().unwrap().pixel(48, 30);
                let expected = match page {
                    1 => [1.0, 0.0, 0.0, 1.0],
                    2 => [0.0, 1.0, 0.0, 1.0],
                    _ => [0.0, 0.0, 1.0, 1.0],
                };
                assert_eq!(pixel.as_slice(), expected, "page {page} must keep its own pixels");
            }
        }
    }
}

#[test]
fn individual_pdf_pages_keep_their_dimensions_and_content() {
    for (page, width, height) in [(1, 72, 48), (2, 48, 72), (3, 96, 60)] {
        let opts = ImportOptions { pdf: Some(PdfOpenOptions { page, resolution: 72.0, crop: CropTo::MediaBox, ..Default::default() }), ..Default::default() };
        let doc = import_with_options("colors.pdf", PDF, &opts).unwrap().document;
        assert_eq!(doc.layers.len(), 1);
        assert_eq!((doc.size.width, doc.size.height), (width, height));
        assert_eq!(doc.name, format!("colors-{page}.pdf"));
        assert_eq!(doc.layers[0].surface().unwrap().pixel(12, 12)[page - 1], 1.0);
    }
}

/// Minimal one-page PDF containing only a 1x1 red RGB image.
/// Construct offsets so the test does not depend on an external PDF generator.
fn one_image_pdf() -> Vec<u8> {
    let drawing = b"q 72 0 0 72 0 0 cm /Im0 Do Q\n";
    let objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        [format!("<< /Length {} >>\nstream\n", drawing.len()).into_bytes(), drawing.to_vec(), b"endstream".to_vec()].concat(),
        [
            b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 3 >>\nstream\n".to_vec(),
            vec![255, 0, 0],
            b"\nendstream".to_vec(),
        ]
        .concat(),
    ];
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0usize];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in offsets.iter().skip(1) {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len()).as_bytes());
    bytes
}

#[test]
fn single_image_pdf_never_allows_direct_save_over_original() {
    let bytes = one_image_pdf();
    let result = import_with_options("scanned.pdf", &bytes, &ImportOptions::default()).unwrap();
    assert!(result.source_read_only, "even a PDF containing only an image must require Save As");
    assert_eq!(result.document.layers.len(), 1);
    assert_eq!((result.document.size.width, result.document.size.height), (1, 1));
}

#[test]
fn invalid_pdf_page_selection_returns_error_instead_of_panicking() {
    for page in [0, 4, usize::MAX] {
        let opts = ImportOptions { pdf: Some(PdfOpenOptions { page, ..Default::default() }), ..Default::default() };
        assert!(import_with_options("colors.pdf", PDF, &opts).is_err(), "invalid page {page}");
    }
    for selected in [vec![], vec![0], vec![4], vec![usize::MAX]] {
        let opts = ImportOptions { pdf: Some(PdfOpenOptions { pages_as_layers: Some(selected.clone()), ..Default::default() }), ..Default::default() };
        assert!(import_with_options("colors.pdf", PDF, &opts).is_err(), "invalid selection {selected:?}");
    }
}

#[test]
fn pdf_pages_as_artboards_keep_page_identity_and_individual_sizes() {
    let opts = ImportOptions {
        pdf: Some(PdfOpenOptions { pages_as_artboards: Some(vec![1, 2, 3]), resolution: 72.0, crop: CropTo::MediaBox, ..Default::default() }),
        ..Default::default()
    };
    let result = import_with_options("colors.pdf", PDF, &opts).unwrap();
    let doc = result.document;
    assert_eq!(doc.artboards().len(), 3);
    assert_eq!(doc.layers.len(), 3);
    let expected = [(1, 72, 48), (2, 48, 72), (3, 96, 60)];
    for (board, (page, width, height)) in doc.layers.iter().zip(expected) {
        let artboard = board.artboard().expect("PDF pages are artboards");
        assert_eq!(artboard.preset, format!("photocraft.pdf.page.v1:{page}"));
        assert_eq!((artboard.rect.width(), artboard.rect.height()), (width, height));
        let children = board.children().unwrap();
        assert_eq!(children.len(), 1);
        assert!(children[0].locks.position, "PDF source must not be accidentally moved");
        assert!(children[0].locks.all, "PDF source must remain protected from accidental painting");
        let x = artboard.rect.x0 + 12;
        let y = artboard.rect.y0 + 12;
        assert_eq!(children[0].surface().unwrap().pixel(x, y)[page - 1], 1.0);
    }
    // The canvas must leave enough room between neighboring sheets to draw their
    // labels; the old fixed 24px gap vanished when a full 300dpi PDF was fitted.
    let rects: Vec<_> = doc.layers.iter().map(|l| l.artboard().unwrap().rect).collect();
    for pair in rects.windows(2) {
        assert!(pair[1].x0 - pair[0].x1 >= 24, "page labels must not overlap another sheet");
    }
    assert!(result.source_read_only);
}

#[test]
fn pdf_artboards_reject_empty_and_invalid_page_selection() {
    for selected in [vec![], vec![0], vec![4], vec![1, usize::MAX], vec![2, 2]] {
        let opts = ImportOptions { pdf: Some(PdfOpenOptions { pages_as_artboards: Some(selected), ..Default::default() }), ..Default::default() };
        assert!(import_with_options("colors.pdf", PDF, &opts).is_err());
    }
}

#[test]
fn pdf_page_identity_survives_editable_psd_save_and_reopen() {
    let opts = ImportOptions {
        pdf: Some(PdfOpenOptions { pages_as_artboards: Some(vec![3, 1, 2]), resolution: 72.0, crop: CropTo::MediaBox, ..Default::default() }),
        ..Default::default()
    };
    let source = import_with_options("colors.pdf", PDF, &opts).unwrap().document;
    let saved = photocraft_io::export(&source, "editable.psd", &photocraft_io::ExportOptions::default())
        .expect("save the imported multipage document as a layered PSD")
        .bytes;
    let reopened = photocraft_io::import("editable.psd", &saved).expect("reopen editable PSD").document;
    assert_eq!(reopened.artboards().len(), 3);
    let actual: Vec<_> = reopened.artboards().iter().map(|(_, _, board)| (board.preset.clone(), board.rect.width(), board.rect.height())).collect();
    let expected: Vec<_> = source.artboards().iter().map(|(_, _, board)| (board.preset.clone(), board.rect.width(), board.rect.height())).collect();
    assert_eq!(actual, expected, "PDF-page identity and sizes must survive saving");
    for layer in &reopened.layers {
        let children = layer.children().expect("PDF page should remain an editable group");
        assert_eq!(children.len(), 1);
        assert!(children[0].surface().is_some(), "PDF original must remain a raster layer");
    }
}

#[test]
fn multipage_pdf_import_cannot_silently_save_as_one_big_page() {
    let opts = ImportOptions {
        pdf: Some(PdfOpenOptions { pages_as_artboards: Some(vec![1, 2, 3]), resolution: 72.0, crop: CropTo::MediaBox, ..Default::default() }),
        ..Default::default()
    };
    let doc = import_with_options("multi.pdf", PDF, &opts).unwrap().document;
    let result = photocraft_io::export(&doc, "wrong.pdf", &photocraft_io::ExportOptions::default());
    assert!(result.is_err(), "legacy single-page PDF writer must refuse tagged multipage documents");
}
