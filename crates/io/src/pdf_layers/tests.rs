use super::*;
use crate::{ImportOptions, pdf_import::PdfOpenOptions};
use photocraft_color::{ColorMode as DocColorMode, SampleType};
use photocraft_doc::{Document, Layer, PdfImportMode};
use photocraft_geom::{Rect, Size};
use photocraft_psd::tiff::ByteOrder;

/// Serializes fixture objects with a cross-reference table and object 1 as the document catalog.
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

/// Wraps raw fixture bytes in a length-delimited stream with the supplied dictionary attributes.
fn stream(data: &[u8], attributes: &str) -> Vec<u8> {
    let mut bytes = format!("<< /Length {} {attributes} >>\nstream\n", data.len()).into_bytes();
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(b"\nendstream");
    bytes
}

/// Builds a Photoshop-preserved PDF with supplied layer blocks, composite depth, and optional ICC profile.
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

/// Creates a one-point green page without Photoshop preservation metadata.
fn ordinary() -> Vec<u8> {
    ordinary_with_page_metadata("")
}

/// Creates a green page with caller-supplied page metadata for preservation discovery tests.
fn ordinary_with_page_metadata(metadata: &str) -> Vec<u8> {
    pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] /Resources << >> /Contents 4 0 R {metadata} >>").into_bytes(),
        stream(b"0 1 0 rg 0 0 1 1 re f", ""),
    ])
}

/// Selects a PDF layer policy at one pixel per point, leaving other import settings at their defaults.
fn options(mode: PdfImportMode) -> ImportOptions {
    ImportOptions { pdf: Some(PdfOpenOptions { layer_mode: mode, resolution: 72.0, ..Default::default() }), ..Default::default() }
}

/// Creates a three-by-two RGB document with one named, partially transparent layer at the requested depth.
fn source(depth: SampleType) -> Document {
    let mut document = Document::new("source", Size::new(3, 2), DocColorMode::Rgb, depth);
    document.layers.push(Layer::raster("Authored layer", document.pixel_format()));
    document.layers[0].surface_mut().unwrap().fill_rect(Rect::new(0, 0, 3, 2), &[0.125, 0.5, 0.875, 0.5]);
    document
}

/// Encodes a document as big-endian Photoshop image-source blocks without the outer signature.
fn blocks(document: &Document) -> Vec<u8> {
    let psd = crate::document_to_psd(document);
    let (source, _) = ImageSourceData::from_psd(&psd).to_bytes(ByteOrder::Big).unwrap();
    source[SIGNATURE.len()..].to_vec()
}

/// Restores authored layer names, pixels, and sample depth without flattening in both preservation modes.
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

/// Restores the composite image's ICC profile when no image-resources profile copy exists.
#[test]
fn composite_icc_profile_survives_without_an_image_resources_copy() {
    let original = source(SampleType::U8);
    let profile = photocraft_cms::Builtin::Srgb.profile().to_bytes();
    let bytes = preserved(&blocks(&original), 8, Some(&profile), "", "");
    let imported = crate::import("profile.pdf", &bytes).unwrap();
    assert_eq!(imported.document.icc_profile.as_deref().map(Vec::as_slice), Some(profile.as_slice()));
}

/// Decodes Flate-compressed preservation blocks through the bounded path without changing layer pixels.
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

/// Renders ordinary PDFs in auto mode but refuses them when editable layers are explicitly required.
#[test]
fn absent_layers_fall_back_only_in_auto_mode() {
    let bytes = ordinary();
    let imported = crate::import_with_options("ordinary.pdf", &bytes, &options(PdfImportMode::Auto)).unwrap();
    assert_eq!(imported.document.size, Size::new(1, 1));
    assert_eq!(imported.document.layers[0].surface().unwrap().pixel(0, 0), [0.0, 1.0, 0.0, 1.0]);
    assert!(imported.source_read_only);
    assert!(matches!(crate::import_with_options("ordinary.pdf", &bytes, &options(PdfImportMode::Layers)), Err(IoError::Pdf(_))));
}

/// Treats an omitted optional private entry as absent layers, allowing fallback only in auto mode.
#[test]
fn absent_optional_private_entry_falls_back_only_in_auto_mode() {
    for adobe in ["<< >>", "<< /LastModified (D:20261105000000Z) >>"] {
        let bytes = ordinary_with_page_metadata(&format!("/PieceInfo << /AdobePhotoshop {adobe} >>"));
        assert!(matches!(layered_result("metadata.pdf", &bytes, 1, None, &Interrupt::NONE), Err(LayeredPdfError::Absent)));
        let imported = crate::import_with_options("metadata.pdf", &bytes, &options(PdfImportMode::Auto)).unwrap();
        assert_eq!(imported.document.size, Size::new(1, 1));
        assert_eq!(imported.document.layers[0].surface().unwrap().pixel(0, 0), [0.0, 1.0, 0.0, 1.0]);
        assert!(imported.source_read_only);
        assert!(matches!(crate::import_with_options("metadata.pdf", &bytes, &options(PdfImportMode::Layers)), Err(IoError::Pdf(message))
                if message.contains("no Photoshop editable layer data")));
    }
}

/// Treats a present malformed private entry as corruption unless rendering is explicitly requested.
#[test]
fn present_malformed_private_entry_is_terminal_in_auto_and_layers_modes() {
    for private in ["null", "42", "[]", "<< >>"] {
        let bytes = ordinary_with_page_metadata(&format!("/PieceInfo << /AdobePhotoshop << /Private {private} >> >>"));
        assert!(matches!(layered_result("malformed.pdf", &bytes, 1, None, &Interrupt::NONE), Err(LayeredPdfError::Corrupt(_))));
        for mode in [PdfImportMode::Auto, PdfImportMode::Layers] {
            assert!(matches!(crate::import_with_options("malformed.pdf", &bytes, &options(mode)), Err(IoError::Pdf(message))
                    if message.contains("corrupt or unsupported")));
        }
        let rendered = crate::import_with_options("malformed.pdf", &bytes, &options(PdfImportMode::Render)).unwrap();
        assert_eq!(rendered.document.layers[0].surface().unwrap().pixel(0, 0), [0.0, 1.0, 0.0, 1.0]);
    }
}

/// Refuses damaged preserved channels without auto-flattening, while explicit rendering bypasses layer data.
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

/// Materializes distinct layer and render projections for embedded or stored linked PDFs without filesystem reads.
#[test]
fn smart_pdf_export_materializes_each_projection_without_reading_links() {
    use photocraft_doc::{LayerContent, SmartContentsId, SmartObject, SmartSource};
    use photocraft_geom::Affine;
    use std::sync::Arc;

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

/// Classifies invalid layer blocks or image resources as corruption instead of silently rendering in auto mode.
#[test]
fn present_invalid_preservation_metadata_never_silently_flattens() {
    let valid = blocks(&source(SampleType::U8));
    for bytes in [preserved(b"8BIMLr16\0\0\0\x10", 16, None, "", ""), preserved(&valid, 8, None, "/ImageResources 42", "")] {
        assert!(matches!(layered_result("invalid.pdf", &bytes, 1, None, &Interrupt::NONE), Err(LayeredPdfError::Corrupt(_))));
        assert!(matches!(crate::import_with_options("invalid.pdf", &bytes, &options(PdfImportMode::Auto)), Err(IoError::Pdf(_))));
    }
}

/// Keeps invalid input, page selection, source size, and cancellation distinct from preservation corruption.
#[test]
fn input_page_and_size_errors_are_not_preservation_corruption() {
    assert!(matches!(layered_result("bad.pdf", b"%PDF-not a document", 1, None, &Interrupt::NONE), Err(LayeredPdfError::Pdf(_))));
    let bytes = ordinary();
    for page in [0, 2] {
        assert!(matches!(layered_result("page.pdf", &bytes, page, None, &Interrupt::NONE), Err(LayeredPdfError::Pdf(_))));
    }
    assert!(matches!(validate_pdf_size_len(MAX_PDF_BYTES + 1, PdfImportMode::Layers), Err(LayeredPdfError::Pdf(_))));
    assert!(matches!(layered_result("cancelled.pdf", &bytes, 1, None, &Interrupt::new(&|| true, &|_| {})), Err(LayeredPdfError::Cancelled)));
}

/// Enforces the source byte cap at the boundary for preservation modes without restricting explicit rendering.
#[test]
fn source_pdf_size_limit_applies_only_to_preservation_modes() {
    for mode in [PdfImportMode::Auto, PdfImportMode::Layers] {
        assert!(validate_pdf_size_len(MAX_PDF_BYTES, mode).is_ok());
        assert!(matches!(validate_pdf_size_len(MAX_PDF_BYTES + 1, mode), Err(LayeredPdfError::Pdf(_))));
    }
    for len in [MAX_PDF_BYTES, MAX_PDF_BYTES + 1, usize::MAX] {
        assert!(validate_pdf_size_len(len, PdfImportMode::Render).is_ok());
    }
}

/// Names preserved documents from the source file stem, matching ordinary page import.
#[test]
fn preserved_layers_are_named_like_a_rendered_page() {
    let bytes = preserved(&blocks(&source(SampleType::U8)), 8, None, "", "");
    let imported = crate::import_with_options("/home/me/scans/layers.pdf", &bytes, &options(PdfImportMode::Layers)).unwrap();
    assert_eq!(imported.document.name, "layers", "the file stem, as page_name gives an ordinary page");
}

/// Refuses preserved layers above the caller's pixel allowance and releases decode admission.
#[test]
fn preserved_layers_honor_the_callers_pixel_allowance() {
    let bytes = preserved(&blocks(&source(SampleType::U8)), 8, None, "", "");
    let snapshot = crate::memory::MemorySnapshot { available_ram: Some(12 << 30), swap_free: None, commit_headroom: None };
    crate::memory::with_mock_memory(snapshot, || {
        let mut opts = options(PdfImportMode::Layers);
        opts.psd_max_pixel_bytes = 1;
        assert!(matches!(crate::import_with_options("limited.pdf", &bytes, &opts), Err(IoError::PsdImportBudget { limit: 1, .. })));
        assert!(!crate::psd_gate::is_holding_admission());
    });
}

/// Requires typed memory approval and enforces its estimate and pixel bounds before channel decoding.
#[test]
#[cfg(all(not(target_arch = "wasm32"), target_pointer_width = "64"))]
fn preserved_layers_keep_typed_approval_and_apply_approved_bounds() {
    let mut psd = crate::document_to_psd(&source(SampleType::U8));
    // Only the declaration is large: the original channel bytes remain tiny and invalid for it.
    // Approval must be requested before those channels are decoded.
    psd.layers_mut()[0].rect = photocraft_psd::Rect::from_xywh(0, 0, 30_000, 30_000);
    let (tagged, _) = ImageSourceData::from_psd(&psd).to_bytes(ByteOrder::Big).unwrap();
    let bytes = preserved(&tagged[SIGNATURE.len()..], 8, None, "", "");
    let (layers, _) = ImageSourceData::from_bytes(&tagged).unwrap();
    let file = layers.into_psd(Header::new(photocraft_psd::Version::Psd, 3, 2, 3, 8, ColorMode::Rgb), Vec::new(), ImageData::new(Compression::Raw, Vec::new()));
    let required = crate::psd_budget::estimate(&file).unwrap().total_bytes;
    assert!(required > 1 << 30);
    let snapshot = crate::memory::MemorySnapshot { available_ram: Some(required * 2 - 1), swap_free: None, commit_headroom: None };
    crate::memory::with_mock_memory(snapshot, || {
        let mut opts = options(PdfImportMode::Layers);
        assert!(
            matches!(crate::import_with_options("approval.pdf", &bytes, &opts), Err(IoError::PsdApprovalRequired { required: actual, .. }) if actual == required)
        );
        assert!(!crate::psd_gate::is_holding_admission());
        opts.psd_high_memory_approved = true;
        opts.psd_approved_estimate = Some(required - 1);
        assert!(matches!(crate::import_with_options("approval.pdf", &bytes, &opts), Err(IoError::PsdApprovalRequired { .. })));
        opts.psd_approved_estimate = Some(required);
        opts.psd_max_pixel_bytes = required - 1;
        assert!(matches!(crate::import_with_options("approval.pdf", &bytes, &opts), Err(IoError::PsdImportBudget { .. })));
        opts.psd_max_pixel_bytes = 0;
        // Authorization reaches channel decoding, which rejects the deliberately truncated data.
        assert!(matches!(crate::import_with_options("approval.pdf", &bytes, &opts), Err(IoError::Pdf(_))));
        assert!(!crate::psd_gate::is_holding_admission());
    });
}

/// Rechecks available memory after decode admission and releases the gate when capacity has fallen.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn preserved_layers_recheck_memory_after_acquiring_admission() {
    use std::{cell::Cell, rc::Rc};
    let bytes = preserved(&blocks(&source(SampleType::U8)), 8, None, "", "");
    let queries = Rc::new(Cell::new(0));
    let provider = {
        let queries = queries.clone();
        Rc::new(move || {
            let count = queries.get();
            queries.set(count + 1);
            crate::memory::MemorySnapshot { available_ram: Some(if count == 0 { 12 << 30 } else { 0 }), swap_free: None, commit_headroom: None }
        })
    };
    crate::memory::with_mock_memory_provider(provider, || {
        assert!(matches!(crate::import_with_options("drop.pdf", &bytes, &options(PdfImportMode::Layers)), Err(IoError::PsdImportBudget { .. })));
        assert!(!crate::psd_gate::is_holding_admission());
    });
    assert!(queries.get() >= 2, "capacity must be checked before and after acquiring the gate");
}
