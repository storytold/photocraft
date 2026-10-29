use super::*;
use crate::{ImportOptions, pdf_import::PdfOpenOptions};
use photocraft_color::{ColorMode as DocColorMode, SampleType};
use photocraft_doc::{Document, Layer, PdfImportMode};
use photocraft_geom::{Rect, Size};
use photocraft_psd::tiff::ByteOrder;

fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    bytes
}

fn stream(data: &[u8], attributes: &str) -> Vec<u8> {
    let mut bytes = format!("<< /Length {} {attributes} >>\nstream\n", data.len()).into_bytes();
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(b"\nendstream");
    bytes
}

fn preserved(blocks: &[u8], depth: u16, profile: Option<&[u8]>, resources: &str, filter: &str) -> Vec<u8> {
    let color = if profile.is_some() { "[/ICCBased 7 0 R]" } else { "/DeviceRGB" };
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 3 2] /Resources << >> /PieceInfo << /AdobePhotoshop << /Private 4 0 R >> >> >>".to_vec(),
        format!("<< /StandardImageFileData 5 0 R /CompositeImage 6 0 R {resources} >>").into_bytes(),
        stream(blocks, filter),
        stream(&[], &format!("/Type /XObject /Subtype /Image /Width 3 /Height 2 /BitsPerComponent {depth} /ColorSpace {color}")),
    ];
    if let Some(profile) = profile {
        objects.push(stream(profile, "/N 3"));
    }
    pdf(&objects)
}

fn ordinary() -> Vec<u8> {
    pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] /Resources << >> /Contents 4 0 R >>".to_vec(),
        stream(b"0 1 0 rg 0 0 1 1 re f", ""),
    ])
}

fn options(mode: PdfImportMode) -> ImportOptions {
    ImportOptions { pdf: Some(PdfOpenOptions { layer_mode: mode, resolution: 72.0, ..Default::default() }), ..Default::default() }
}

fn source(depth: SampleType) -> Document {
    let mut document = Document::new("source", Size::new(3, 2), DocColorMode::Rgb, depth);
    document.layers.push(Layer::raster("Authored layer", document.pixel_format()));
    document.layers[0].surface_mut().unwrap().fill_rect(Rect::new(0, 0, 3, 2), &[0.125, 0.5, 0.875, 0.5]);
    document
}

fn blocks(document: &Document) -> Vec<u8> {
    let psd = crate::document_to_psd(document);
    let (source, _) = ImageSourceData::from_psd(&psd).to_bytes(ByteOrder::Big).unwrap();
    source[SIGNATURE.len()..].to_vec()
}

#[test]
fn preserved_layers_keep_pixels_and_depth_in_auto_and_layers_modes() {
    for (sample, bits) in [(SampleType::U8, 8), (SampleType::U16, 16), (SampleType::F32, 32)] {
        let original = source(sample);
        let expected = original.layers[0].surface().unwrap().to_interleaved(Rect::new(0, 0, 3, 2));
        let bytes = preserved(&blocks(&original), bits, None, "", "");
        for mode in [PdfImportMode::Auto, PdfImportMode::Layers] {
            let imported = crate::import_with_options("layers.pdf", &bytes, &options(mode)).unwrap();
            assert!(imported.source_read_only);
            assert!(!imported.preview_only);
            assert_eq!(imported.document.depth, sample);
            assert_eq!(imported.document.size, Size::new(3, 2));
            assert_eq!(imported.document.layers[0].name, "Authored layer");
            assert_eq!(imported.document.layers[0].surface().unwrap().to_interleaved(Rect::new(0, 0, 3, 2)), expected);
        }
    }
}

#[test]
fn composite_icc_profile_survives_without_an_image_resources_copy() {
    let original = source(SampleType::U8);
    let profile = photocraft_cms::Builtin::Srgb.profile().to_bytes();
    let bytes = preserved(&blocks(&original), 8, Some(&profile), "", "");
    let imported = crate::import("profile.pdf", &bytes).unwrap();
    assert_eq!(imported.document.icc_profile.as_deref().map(Vec::as_slice), Some(profile.as_slice()));
}

#[test]
fn flate_preserved_layers_use_bounded_stream_decoder() {
    use std::io::Write;
    let original = source(SampleType::U16);
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&blocks(&original)).unwrap();
    let bytes = preserved(&encoder.finish().unwrap(), 16, None, "", "/Filter /FlateDecode");
    let imported = crate::import("compressed.pdf", &bytes).unwrap();
    assert_eq!(
        imported.document.layers[0].surface().unwrap().to_interleaved(Rect::new(0, 0, 3, 2)),
        original.layers[0].surface().unwrap().to_interleaved(Rect::new(0, 0, 3, 2))
    );
}

#[test]
fn absent_layers_fall_back_only_in_auto_mode() {
    let bytes = ordinary();
    let imported = crate::import_with_options("ordinary.pdf", &bytes, &options(PdfImportMode::Auto)).unwrap();
    assert_eq!(imported.document.size, Size::new(1, 1));
    assert_eq!(imported.document.layers[0].surface().unwrap().pixel(0, 0), [0.0, 1.0, 0.0, 1.0]);
    assert!(imported.source_read_only);
    assert!(matches!(crate::import_with_options("ordinary.pdf", &bytes, &options(PdfImportMode::Layers)), Err(IoError::Pdf(_))));
}

#[test]
fn damaged_channels_are_terminal_but_explicit_render_still_opens_the_page() {
    let mut psd = crate::document_to_psd(&source(SampleType::U8));
    psd.layers_mut()[0].channels[0].data.clear();
    let (tagged, _) = ImageSourceData::from_psd(&psd).to_bytes(ByteOrder::Big).unwrap();
    let bytes = preserved(&tagged[SIGNATURE.len()..], 8, None, "", "");
    assert!(matches!(layered_result("damaged.pdf", &bytes, 1, None, &Interrupt::NONE), Err(LayeredPdfError::Corrupt(_))));
    for mode in [PdfImportMode::Auto, PdfImportMode::Layers] {
        assert!(matches!(crate::import_with_options("damaged.pdf", &bytes, &options(mode)), Err(IoError::Pdf(_))));
    }
    let rendered = crate::import_with_options("damaged.pdf", &bytes, &options(PdfImportMode::Render)).unwrap();
    assert_eq!(rendered.document.size, Size::new(3, 2));
    assert_eq!(rendered.document.layers[0].surface().unwrap().pixel(0, 0), [0.0, 0.0, 0.0, 0.0]);
}

#[test]
fn smart_pdf_export_materializes_each_projection_without_reading_links() {
    use std::sync::Arc;
    use photocraft_doc::{LayerContent, SmartContentsId, SmartObject, SmartSource};
    use photocraft_geom::Affine;

    let original = source(SampleType::U8);
    let bytes = Arc::new(preserved(&blocks(&original), 8, None, "", ""));
    for embedded in [true, false] {
        let mut document = Document::new("placed", original.size, original.mode, original.depth);
        let source = if embedded {
            SmartSource::Embedded { file_name: "layers.pdf".into(), bytes: bytes.clone() }
        } else {
            let file = crate::linked::LinkedFile { uuid: "preserved-pdf".into(), file_name: "layers.pdf".into(), bytes: bytes.as_ref().clone() };
            document.metadata.psd_global_blocks.push((*b"8BIM", *b"lnk2", Arc::new(crate::linked::encode_linked_file(&file))));
            SmartSource::Linked { path: file.uuid }
        };
        let contents_id = SmartContentsId::fresh();
        for mode in [PdfImportMode::Layers, PdfImportMode::Render] {
            let mut smart = SmartObject::new(source.clone(), Affine::IDENTITY, original.layers[0].surface().cloned());
            smart.contents_id = contents_id;
            smart.pdf_import = Some(options(mode).pdf.unwrap().projection());
            document.layers.push(Layer::new(format!("{mode:?}"), LayerContent::Smart(smart)));
        }
        let (file, warnings) = crate::document_to_psd_with(&document, &crate::PsdExportOptions { read_linked_files: false, ..Default::default() });
        assert!(!warnings.iter().any(|warning| warning.contains("smart object written as pixels")), "{warnings:?}");
        let payloads: Vec<_> =
            file.global_blocks.iter().filter(|block| &block.key == b"lnk2").flat_map(|block| crate::linked::parse_linked_files(&block.data)).collect();
        assert_eq!(payloads.len(), 2, "different projections must not share a converted source");
        let mut nested_names = Vec::new();
        for payload in payloads {
            assert!(payload.file_name.ends_with(".psb"), "the PDF projection must be materialized");
            let nested = crate::import(&payload.file_name, &payload.bytes).unwrap().document;
            nested_names.push(nested.layers[0].name.clone());
        }
        nested_names.sort();
        assert_eq!(nested_names, ["Authored layer", "Layer 1"]);
    }
}

#[test]
fn present_invalid_preservation_metadata_never_silently_flattens() {
    let valid = blocks(&source(SampleType::U8));
    for bytes in [preserved(b"8BIMLr16\0\0\0\x10", 16, None, "", ""), preserved(&valid, 8, None, "/ImageResources 42", "")] {
        assert!(matches!(layered_result("invalid.pdf", &bytes, 1, None, &Interrupt::NONE), Err(LayeredPdfError::Corrupt(_))));
        assert!(matches!(crate::import_with_options("invalid.pdf", &bytes, &options(PdfImportMode::Auto)), Err(IoError::Pdf(_))));
    }
}

#[test]
fn input_page_and_size_errors_are_not_preservation_corruption() {
    assert!(matches!(layered_result("bad.pdf", b"%PDF-not a document", 1, None, &Interrupt::NONE), Err(LayeredPdfError::Pdf(_))));
    let bytes = ordinary();
    for page in [0, 2] {
        assert!(matches!(layered_result("page.pdf", &bytes, page, None, &Interrupt::NONE), Err(LayeredPdfError::Pdf(_))));
    }
    assert!(matches!(validate_pdf_size_len(MAX_PDF_BYTES + 1), Err(LayeredPdfError::Pdf(_))));
    assert!(matches!(layered_result("cancelled.pdf", &bytes, 1, None, &Interrupt::new(&|| true, &|_| {})), Err(LayeredPdfError::Cancelled)));
}

#[test]
fn preserved_layers_are_named_like_a_rendered_page() {
    let bytes = preserved(&blocks(&source(SampleType::U8)), 8, None, "", "");
    let imported = crate::import_with_options("/home/me/scans/layers.pdf", &bytes, &options(PdfImportMode::Layers)).unwrap();
    assert_eq!(imported.document.name, "layers", "the file stem, as page_name gives an ordinary page");
}
