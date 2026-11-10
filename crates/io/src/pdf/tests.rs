//! PDF export: structure, pixel oracle, colour modes and refusal paths.

use super::*;
use lopdf::{Object, StringFormat};
use photocraft_color::{PixelFormat, SampleType};
use photocraft_doc::{Artboard, Group, Layer, LayerContent, Size};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};

fn solid(fmt: PixelFormat, x0: i32, y0: i32, x1: i32, y1: i32, px: &[f32]) -> Surface {
    let mut s = Surface::new(fmt);
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let data: Vec<f32> = px.iter().copied().cycle().take(w * h * px.len()).collect();
    s.write_region(Rect::from_xywh(x0, y0, w as u32, h as u32), &data);
    s
}

/// The opaque paper colour of `mode`, with alpha.
fn paper(mode: ColorMode) -> Vec<f32> {
    match mode {
        ColorMode::Grayscale => vec![1.0, 1.0],
        ColorMode::Cmyk => vec![0.0, 0.0, 0.0, 0.0, 1.0],
        _ => vec![1.0, 1.0, 1.0, 1.0],
    }
}

fn doc(mode: ColorMode) -> Document {
    let fmt = PixelFormat::new(mode, SampleType::F32, true);
    let bg = paper(mode);
    let mut d = Document::new("t.pdf", Size { width: 4, height: 2 }, mode, SampleType::F32);
    d.layers = vec![Layer::new("Paper", LayerContent::Raster(solid(fmt, 0, 0, 4, 2, &bg)))];
    d
}

/// The bytes of the stream following `marker` (e.g. "/Subtype /Image"), inflated.
fn stream_after(pdf: &[u8], marker: &str) -> Vec<u8> {
    let at = pdf.windows(marker.len()).position(|w| w == marker.as_bytes()).expect("marker");
    let body = &pdf[at..];
    let len_at = body.windows(8).position(|w| w == b"/Length ").expect("length") + 8;
    let len_end = body[len_at..].iter().position(|b| !b.is_ascii_digit()).expect("length end");
    let len: usize = std::str::from_utf8(&body[len_at..len_at + len_end]).unwrap().parse().unwrap();
    let start = body.windows(7).position(|w| w == b"stream\n").expect("stream") + 7;
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&body[start..start + len]), &mut out).unwrap();
    out
}

fn has(pdf: &[u8], needle: &str) -> bool {
    pdf.windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn check_structure(pdf: &[u8]) {
    assert!(pdf.starts_with(b"%PDF-1."), "PDF header");
    assert!(pdf.ends_with(b"%%EOF\n"), "EOF marker");
    let start = pdf.windows(9).rposition(|w| w == b"startxref").unwrap() + 9;
    let off: usize = std::str::from_utf8(&pdf[start..]).unwrap().split_whitespace().next().unwrap().parse().unwrap();
    assert_eq!(&pdf[off..off + 4], b"xref", "startxref points at the xref table");
    // Every xref offset really precedes its `n 0 obj` header. The xref/trailer region is pure
    // ASCII, so it is safe to decode just that slice.
    let table = std::str::from_utf8(&pdf[off..]).unwrap().lines().collect::<Vec<_>>();
    let count: usize = table[1].split_whitespace().nth(1).unwrap().parse().unwrap();
    for (i, line) in table.iter().skip(3).take(count - 1).enumerate() {
        let o: usize = line[..10].parse().unwrap();
        let want = format!("{} 0 obj", i + 1);
        assert_eq!(&pdf[o..o + want.len()], want.as_bytes(), "xref offset for object {}", i + 1);
    }
}

#[test]
fn a_flat_rgb_document_saves_as_one_page_pdf() {
    let d = doc(ColorMode::Rgb);
    let out = export(&d, &Interrupt::NONE).unwrap();
    let pdf = &out.bytes;
    check_structure(pdf);
    assert!(has(pdf, "/MediaBox [0 0 4.00 2.00]"), "page = image size at 72 pt/in");
    assert!(has(pdf, "/Width 4 /Height 2 /ColorSpace /DeviceRGB"), "RGB is DeviceRGB");
    assert_eq!(stream_after(pdf, "/Subtype /Image"), vec![255u8; 4 * 2 * 3], "the white paper, 8-bit");
    assert!(out.warnings.iter().any(|w| w.contains("flattened")), "layers are not preserved");
    // Through the general dispatch as well.
    let via = crate::export(&d, "out.pdf", &crate::ExportOptions::default()).unwrap();
    assert_eq!(via.bytes, *pdf);
}

#[test]
fn layers_composite_over_white_before_the_page() {
    let mut d = doc(ColorMode::Rgb);
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    d.layers.push(Layer::new("Half red", LayerContent::Raster(solid(fmt, 1, 0, 3, 2, &[1.0, 0.0, 0.0, 0.5]))));
    let out = export(&d, &Interrupt::NONE).unwrap();
    let img = stream_after(&out.bytes, "/Subtype /Image");
    // Pixels 1-2 of row 0: 0.5 red over white = (1.0, 0.5, 0.5); elsewhere plain white.
    assert_eq!(&img[3..6], &[255, 128, 128], "the blend lands: {:?}", &img[3..6]);
    assert_eq!(&img[0..3], &[255, 255, 255]);
}

#[test]
fn topmost_solid_shape_is_exported_as_a_vector_path() {
    use photocraft_doc::{Fill, Path, ShapeLayer, Subpath};
    let mut d = doc(ColorMode::Rgb);
    let shape = ShapeLayer {
        path: Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (2.0, 0.0), (2.0, 1.0)])]),
        fill: Some(Fill::Solid(photocraft_color::Color::rgb(1.0, 0.0, 0.0))),
        ..Default::default()
    };
    d.layers.push(Layer::new("Shape", LayerContent::Shape(shape)));
    let out = export(&d, &Interrupt::NONE).unwrap();
    check_structure(&out.bytes);
    let content = stream_after(&out.bytes, "/Contents 4 0 R");
    let content = String::from_utf8(content).unwrap();
    assert!(content.contains("1 0 0 rg\n"));
    assert!(content.contains("0 0 m\n"));
    assert!(content.contains("h\nf\n"));
    assert_eq!(stream_after(&out.bytes, "/Subtype /Image"), vec![255; 4 * 2 * 3]);
}

#[test]
fn topmost_text_layer_is_exported_with_selectable_text_and_vector_outlines() {
    use photocraft_doc::TextLayer;
    let mut d = doc(ColorMode::Rgb);
    let text = TextLayer {
        text: "Vector PDF".into(),
        size_pt: 24.0,
        color: photocraft_color::Color::rgb(0.0, 0.0, 1.0),
        transform: photocraft_geom::Affine::translate(1.0, 1.5),
        ..Default::default()
    };
    let layout = photocraft_text::TextEngine::new().layout(&text, 72.0);
    let cluster = layout.clusters.iter().filter(|cluster| !cluster.range.is_empty()).min_by_key(|cluster| cluster.range.start).expect("a shaped cluster");
    let baseline = layout.lines.get(cluster.line).expect("a line").baseline;
    let origin = text.transform.apply(photocraft_geom::Point::new(f64::from(cluster.x), f64::from(baseline)));
    d.layers.push(Layer::new("Text", LayerContent::Text(text)));
    let out = export(&d, &Interrupt::NONE).unwrap();
    check_structure(&out.bytes);
    let content = stream_after(&out.bytes, "/Contents 4 0 R");
    let content = String::from_utf8(content).unwrap();
    assert!(content.contains("/Span << /ActualText <FEFF"));
    assert!(content.contains("0 0 1 rg"));
    assert!(content.contains("BT"));
    assert!(content.contains("3 Tr"));
    assert!(content.contains("/FSelect"));
    assert!(has(&out.bytes, "/ToUnicode"));
    let tm = format!("{} {} Tm", origin.x, origin.y);
    assert!(content.lines().any(|line| line.ends_with(tm.as_str())), "the translated text starts at {tm}:\n{content}");
    let parsed = lopdf::Document::load_mem(&out.bytes).unwrap();
    assert_eq!(parsed.extract_text(&[1]).unwrap().trim(), "Vector PDF");
    assert!(content.contains("ET"));
    assert!(content.contains("EMC"));
    assert_eq!(stream_after(&out.bytes, "/Subtype /Image"), vec![255; 4 * 2 * 3]);
}

#[test]
fn topmost_shape_and_text_layers_are_both_exported_as_vectors() {
    use photocraft_doc::{Fill, Path, ShapeLayer, Subpath, TextLayer};
    let mut d = doc(ColorMode::Rgb);
    let shape = ShapeLayer {
        path: Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (2.0, 0.0), (2.0, 1.0)])]),
        fill: Some(Fill::Solid(photocraft_color::Color::rgb(1.0, 0.0, 0.0))),
        ..Default::default()
    };
    d.layers.push(Layer::new("Shape", LayerContent::Shape(shape)));
    let text = TextLayer { text: "Hi".into(), size_pt: 12.0, color: photocraft_color::Color::rgb(0.0, 1.0, 0.0), ..Default::default() };
    d.layers.push(Layer::new("Text", LayerContent::Text(text)));
    let out = export(&d, &Interrupt::NONE).unwrap();
    check_structure(&out.bytes);
    let content = stream_after(&out.bytes, "/Contents 4 0 R");
    let content = String::from_utf8(content).unwrap();
    let red = content.find("1 0 0 rg\n").expect("the shape fill");
    let green = content.find("0 1 0 rg\n").expect("the text fill");
    assert!(red < green, "the lower red shape is painted before the upper green text");
    assert!(content.contains("/Span << /ActualText <FEFF0048> >> BDC"));
    assert!(content.contains("> Tj"));
    assert_eq!(stream_after(&out.bytes, "/Subtype /Image"), vec![255; 4 * 2 * 3]);
}

#[test]
fn greyscale_and_cmyk_pages_keep_their_channels() {
    let g = export(&doc(ColorMode::Grayscale), &Interrupt::NONE).unwrap();
    assert!(has(&g.bytes, "/ColorSpace /DeviceGray"), "gray is DeviceGray");
    assert_eq!(stream_after(&g.bytes, "/Subtype /Image"), vec![255; 4 * 2], "one sample per pixel");
    let c = export(&doc(ColorMode::Cmyk), &Interrupt::NONE).unwrap();
    assert!(has(&c.bytes, "/ColorSpace /DeviceCMYK"), "cmyk is DeviceCMYK");
    assert_eq!(stream_after(&c.bytes, "/Subtype /Image"), vec![0; 4 * 2 * 4], "zero ink over the whole page");
}

#[test]
fn the_page_measures_at_the_document_resolution() {
    let mut d = doc(ColorMode::Rgb);
    d.size = Size { width: 300, height: 150 };
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    d.layers = vec![Layer::new("Paper", LayerContent::Raster(solid(fmt, 0, 0, 300, 150, &[1.0, 1.0, 1.0, 1.0])))];
    d.resolution_dpi = 300.0;
    let out = export(&d, &Interrupt::NONE).unwrap();
    assert!(has(&out.bytes, "/MediaBox [0 0 72.00 36.00]"), "300 px at 300 dpi is one inch");
}

#[test]
fn an_embedded_profile_travels_as_iccbased() {
    let mut d = doc(ColorMode::Rgb);
    d.icc_profile = Some(std::sync::Arc::new(vec![1, 2, 3, 4, 5, 6, 7, 8]));
    let out = export(&d, &Interrupt::NONE).unwrap();
    assert!(has(&out.bytes, "[/ICCBased 7 0 R]"), "the image refers to the profile");
    assert_eq!(stream_after(&out.bytes, "/N 3 /Alternate"), vec![1, 2, 3, 4, 5, 6, 7, 8], "the profile bytes survive");
}

#[test]
fn other_modes_are_refused_with_an_actionable_error() {
    let d = doc(ColorMode::Lab);
    let e = export(&d, &Interrupt::NONE).unwrap_err().to_string();
    assert!(e.contains("RGB") && e.contains("Mode"), "{e}");
    assert!(crate::export(&d, "x.pdf", &crate::ExportOptions::default()).is_err());
}

/// The built-in CMYK profile relabelled ICC v2.1, so PDF/X-1a takes it: the structure tests need
/// a v2 output profile and the repository ships none.
fn v2_cmyk() -> std::sync::Arc<Vec<u8>> {
    let mut b = photocraft_cms::Builtin::CoatedCmyk.profile().to_bytes().to_vec();
    b[8] = 2;
    b[9] = 0x10;
    std::sync::Arc::new(b)
}

fn pdfx(standard: PdfXStandard, source: Option<std::sync::Arc<Vec<u8>>>, convert: bool, bleed_pt: f64) -> PdfOptions {
    PdfOptions { x: Some(PdfX { standard, profile: v2_cmyk(), source, convert, intent: photocraft_cms::Intent::RelativeColorimetric, bpc: true, bleed_pt }) }
}

fn x1a(source: Option<std::sync::Arc<Vec<u8>>>, bleed_pt: f64) -> PdfOptions {
    pdfx(PdfXStandard::X1a2001, source, true, bleed_pt)
}

#[test]
fn x1a_writes_pdf_1_3_with_the_output_intent_boxes_and_identification() {
    let mut d = doc(ColorMode::Rgb);
    d.size = Size { width: 300, height: 150 };
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    d.layers = vec![Layer::new("Paper", LayerContent::Raster(solid(fmt, 0, 0, 300, 150, &[1.0, 1.0, 1.0, 1.0])))];
    d.resolution_dpi = 300.0;
    let out = crate::export(&d, "out.pdf", &crate::ExportOptions { pdf: x1a(None, 9.0), ..Default::default() }).unwrap();
    let pdf = &out.bytes;
    check_structure(pdf);
    assert!(pdf.starts_with(b"%PDF-1.3\n"), "PDF/X-1a:2001 is PDF 1.3");
    assert!(has(pdf, "/GTS_PDFXVersion (PDF/X-1:2001) /GTS_PDFXConformance (PDF/X-1a:2001)"));
    assert!(has(pdf, "/Trapped /False"));
    assert!(has(pdf, "/OutputIntents [<< /Type /OutputIntent /S /GTS_PDFX /OutputConditionIdentifier (Custom)"));
    assert!(has(pdf, "/DestOutputProfile 6 0 R"));
    assert!(has(pdf, "/MediaBox [0 0 72.00 36.00] /BleedBox [0 0 72.00 36.00] /TrimBox [9.00 9.00 63.00 27.00]"), "the bleed sits inside the page");
    assert!(has(pdf, "/ColorSpace /DeviceCMYK"), "RGB is converted to the output condition");
    assert!(!has(pdf, "/ICCBased") && !has(pdf, "/DeviceRGB"), "only device CMYK colour");
    assert!(!has(pdf, "/Font"), "no font, so none left unembedded");
    assert!(has(pdf, "/Info 7 0 R /ID [<"), "the trailer names the info dictionary and the file");
    assert_eq!(stream_after(pdf, "/N 4 /Filter"), *v2_cmyk(), "the output profile is embedded whole");
    let img = stream_after(pdf, "/Subtype /Image");
    assert!(img.iter().all(|&v| v == 0), "white paper is no ink");
    assert!(out.warnings.iter().any(|w| w.contains("converted")), "{:?}", out.warnings);
}

#[test]
fn x1a_keeps_a_cmyk_documents_numbers() {
    let mut d = doc(ColorMode::Cmyk);
    let fmt = PixelFormat::new(ColorMode::Cmyk, SampleType::F32, true);
    d.layers = vec![Layer::new("Ink", LayerContent::Raster(solid(fmt, 0, 0, 4, 2, &[1.0, 0.5, 0.0, 0.2, 1.0])))];
    let out = export_with_options(&d, &x1a(None, 0.0), &Interrupt::NONE).unwrap();
    let img = stream_after(&out.bytes, "/Subtype /Image");
    assert_eq!(&img[..4], &[255, 128, 0, 51], "no conversion");
    assert!(!out.warnings.iter().any(|w| w.contains("converted")));
}

#[test]
fn x1a_converts_a_cmyk_document_from_another_condition() {
    let d = doc(ColorMode::Cmyk);
    let other = photocraft_cms::Builtin::CoatedCmyk.profile().to_bytes();
    let out = export_with_options(&d, &x1a(Some(other), 0.0), &Interrupt::NONE).unwrap();
    assert!(out.warnings.iter().any(|w| w.contains("converted")), "{:?}", out.warnings);
}

#[test]
fn x1a_keeps_gray_as_device_gray() {
    let out = export_with_options(&doc(ColorMode::Grayscale), &x1a(None, 0.0), &Interrupt::NONE).unwrap();
    assert!(has(&out.bytes, "/ColorSpace /DeviceGray"));
    assert!(has(&out.bytes, "/GTS_PDFXConformance (PDF/X-1a:2001)"));
}

#[test]
fn x1a_refuses_what_it_cant_write() {
    let d = doc(ColorMode::Rgb);
    let v4 = PdfOptions { x: Some(PdfX { profile: photocraft_cms::Builtin::CoatedCmyk.profile().to_bytes(), ..x1a(None, 0.0).x.unwrap() }) };
    let e = export_with_options(&d, &v4, &Interrupt::NONE).unwrap_err().to_string();
    assert!(e.contains("version 2"), "{e}");
    let rgb = PdfOptions { x: Some(PdfX { profile: photocraft_cms::Builtin::Srgb.profile().to_bytes(), ..x1a(None, 0.0).x.unwrap() }) };
    assert!(export_with_options(&d, &rgb, &Interrupt::NONE).unwrap_err().to_string().contains("CMYK"));
    // The 4 × 2 px page at 72 dpi is 4 × 2 pt: a 1 pt bleed leaves nothing to trim.
    assert!(export_with_options(&d, &x1a(None, 1.0), &Interrupt::NONE).unwrap_err().to_string().contains("bleed"));
    assert!(export_with_options(&doc(ColorMode::Lab), &x1a(None, 0.0), &Interrupt::NONE).is_err());
}

#[test]
fn file_ids_take_the_info_values() {
    assert_eq!(file_id(b"abc", &["t"], 1).len(), 32);
    assert_ne!(file_id(b"abc", &["a"], 1), file_id(b"abc", &["b"], 1), "the title changes the id");
    assert_ne!(file_id(b"abc", &["ab", "c"], 1), file_id(b"abc", &["a", "bc"], 1), "values are kept apart");
}

/// `bytes` relabelled ICC v2.1 (the repository ships no v2 profile); only the version is read.
fn as_v2(p: &photocraft_cms::Profile) -> std::sync::Arc<Vec<u8>> {
    let mut b = p.to_bytes().to_vec();
    b[8] = 2;
    b[9] = 0x10;
    std::sync::Arc::new(b)
}

/// The text between `open` and the next `close` in `pdf`.
fn between(pdf: &[u8], open: &str, close: &str) -> String {
    let s = String::from_utf8_lossy(pdf);
    let at = s.find(open).expect(open) + open.len();
    s[at..].split(close).next().unwrap().to_string()
}

#[test]
fn x3_keeps_icc_tagged_rgb_for_the_printer_to_convert() {
    let mut d = doc(ColorMode::Rgb);
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    d.layers = vec![Layer::new("Red", LayerContent::Raster(solid(fmt, 0, 0, 4, 2, &[1.0, 0.0, 0.0, 1.0])))];
    let srgb = as_v2(photocraft_cms::Builtin::Srgb.profile());
    let out = export_with_options(&d, &pdfx(PdfXStandard::X3_2002, Some(srgb.clone()), false, 0.0), &Interrupt::NONE).unwrap();
    let pdf = &out.bytes;
    check_structure(pdf);
    assert!(
        pdf.starts_with(
            b"%PDF-1.3
"
        ),
        "PDF/X-3:2002 is PDF 1.3"
    );
    assert!(has(pdf, "/GTS_PDFXVersion (PDF/X-3:2002) >>"), "no conformance key: that is PDF/X-1a's");
    assert!(has(pdf, "/S /GTS_PDFX") && has(pdf, "/DestOutputProfile 6 0 R"));
    assert!(has(pdf, "/ColorSpace [/ICCBased 8 0 R]"), "the image keeps its RGB, tagged");
    assert!(has(pdf, "/N 3 /Alternate /DeviceRGB"));
    assert!(!has(pdf, "/ColorSpace /DeviceRGB"), "never untagged RGB");
    assert_eq!(stream_after(pdf, "/N 3 /Alternate"), *srgb, "the document's profile embedded whole");
    assert_eq!(&stream_after(pdf, "/Subtype /Image")[..3], &[255, 0, 0], "the numbers are the document's");
    assert!(!has(pdf, "/Metadata"), "XMP is PDF/X-4's");
    assert!(!out.warnings.iter().any(|w| w.contains("converted")));
}

#[test]
fn x3_converts_to_the_output_condition_when_asked() {
    let srgb = as_v2(photocraft_cms::Builtin::Srgb.profile());
    let out = export_with_options(&doc(ColorMode::Rgb), &pdfx(PdfXStandard::X3_2002, Some(srgb), true, 0.0), &Interrupt::NONE).unwrap();
    check_structure(&out.bytes);
    assert!(has(&out.bytes, "/ColorSpace /DeviceCMYK") && !has(&out.bytes, "/ICCBased"));
    assert!(out.warnings.iter().any(|w| w.contains("PDF/X-3:2002: colour converted")), "{:?}", out.warnings);
}

#[test]
fn x3_refuses_version_4_profiles_pdf_1_3_cant_hold() {
    let d = doc(ColorMode::Rgb);
    // The built-in sRGB is ICC v4: it can't be embedded, only converted from.
    let v4 = photocraft_cms::Builtin::Srgb.profile().to_bytes();
    let e = export_with_options(&d, &pdfx(PdfXStandard::X3_2002, Some(v4.clone()), false, 0.0), &Interrupt::NONE).unwrap_err().to_string();
    assert!(e.starts_with("unsupported") || e.contains("PDF/X-3:2002"), "{e}");
    assert!(e.contains("can't embed") && e.contains("save PDF/X-4"), "{e}");
    assert!(export_with_options(&d, &pdfx(PdfXStandard::X3_2002, Some(v4), true, 0.0), &Interrupt::NONE).is_ok(), "converting from it is fine");
    let mut opts = pdfx(PdfXStandard::X3_2002, None, true, 0.0);
    if let Some(x) = &mut opts.x {
        x.profile = photocraft_cms::Builtin::CoatedCmyk.profile().to_bytes();
    }
    let e = export_with_options(&d, &opts, &Interrupt::NONE).unwrap_err().to_string();
    assert!(e.contains("ICC version 2 and") && e.contains("save PDF/X-4"), "{e}");
}

#[test]
fn x4_writes_pdf_1_6_with_matching_xmp_and_takes_version_4_profiles() {
    let mut d = doc(ColorMode::Rgb);
    d.name = "Plakat Köln & Co.psd".into();
    let mut opts = pdfx(PdfXStandard::X4, Some(photocraft_cms::Builtin::Srgb.profile().to_bytes()), false, 0.0);
    if let Some(x) = &mut opts.x {
        x.profile = photocraft_cms::Builtin::CoatedCmyk.profile().to_bytes();
    }
    let x = opts.x.as_ref().unwrap();
    let out = export_x(&d, x, 1_760_000_000, &Interrupt::NONE).unwrap();
    let pdf = &out.bytes;
    check_structure(pdf);
    assert!(
        pdf.starts_with(
            b"%PDF-1.6
"
        ),
        "PDF/X-4 is PDF 1.6"
    );
    assert!(has(pdf, "/GTS_PDFXVersion (PDF/X-4) >>"));
    assert!(has(pdf, "/ColorSpace [/ICCBased 8 0 R]"), "v4 RGB stays tagged");
    assert!(has(pdf, "/Metadata 9 0 R"));
    assert_eq!(stream_after(pdf, "/N 4 /Filter"), *photocraft_cms::Builtin::CoatedCmyk.profile().to_bytes(), "a v4 output profile");
    // Info and XMP agree, as PDF/X-4 preflight checks.
    assert!(has(pdf, "/CreationDate (D:20251009085320+00'00') /ModDate (D:20251009085320+00'00')"));
    let xmp = between(pdf, "/Subtype /XML", "endstream");
    assert!(xmp.contains("<xmp:CreateDate>2025-10-09T08:53:20+00:00</xmp:CreateDate>"), "{xmp}");
    assert!(xmp.contains("<pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion>"));
    assert!(xmp.contains("<pdf:Trapped>False</pdf:Trapped>") && has(pdf, "/Trapped /False"));
    assert!(xmp.contains("<xmpMM:VersionID>1</xmpMM:VersionID>") && xmp.contains("<xmpMM:RenditionClass>default</xmpMM:RenditionClass>"));
    assert!(xmp.contains("<xmpMM:DocumentID>uuid:"));
    assert!(xmp.contains(">Plakat Köln &amp; Co</rdf:li>"), "{xmp}");
    let title: String = "Plakat Köln & Co".encode_utf16().map(|u| format!("{u:04X}")).collect();
    assert!(has(pdf, &format!("/Title <FEFF{title}>")), "a non-ASCII title as UTF-16, not question marks");
    assert!(has(pdf, "/Type /Metadata /Subtype /XML /Length"), "the packet is not compressed");
}

#[test]
fn every_standard_keeps_a_cmyk_documents_numbers_in_its_condition() {
    let mut d = doc(ColorMode::Cmyk);
    let fmt = PixelFormat::new(ColorMode::Cmyk, SampleType::F32, true);
    d.layers = vec![Layer::new("Ink", LayerContent::Raster(solid(fmt, 0, 0, 4, 2, &[0.2, 0.4, 0.6, 0.8, 1.0])))];
    for standard in PdfXStandard::ALL {
        for convert in [true, false] {
            let out = export_with_options(&d, &pdfx(standard, None, convert, 0.0), &Interrupt::NONE).unwrap();
            check_structure(&out.bytes);
            assert!(has(&out.bytes, "/ColorSpace /DeviceCMYK"), "{standard:?}");
            assert_eq!(&stream_after(&out.bytes, "/Subtype /Image")[..4], &[51, 102, 153, 204], "{standard:?}");
        }
    }
}

#[test]
fn standards_round_trip_their_ids() {
    for s in PdfXStandard::ALL {
        assert_eq!(PdfXStandard::from_id(s.id()), Some(s));
    }
    assert_eq!(PdfXStandard::from_id("X4"), Some(PdfXStandard::X4));
    assert_eq!(PdfXStandard::from_id("x3"), Some(PdfXStandard::X3_2002));
    assert_eq!(PdfXStandard::from_id("x5"), None);
    assert_eq!(pdf_text("abc"), "(abc)");
    assert_eq!(pdf_text("é"), "<FEFF00E9>");
}

/// `bytes` with its header patched at `at`; the data differs, the description doesn't.
fn patched(bytes: &[u8], at: usize, with: &[u8]) -> std::sync::Arc<Vec<u8>> {
    let mut b = bytes.to_vec();
    b[at..at + with.len()].copy_from_slice(with);
    std::sync::Arc::new(b)
}

#[test]
fn the_output_profile_must_be_a_printer_profile_of_version_2_or_4() {
    let d = doc(ColorMode::Rgb);
    let with = |profile: std::sync::Arc<Vec<u8>>, standard| {
        let mut o = pdfx(standard, None, true, 0.0);
        if let Some(x) = &mut o.x {
            x.profile = profile;
        }
        export_with_options(&d, &o, &Interrupt::NONE)
    };
    // A CMYK colour-space profile, not an output (printer) one.
    let e = with(patched(&v2_cmyk(), 12, b"spac"), PdfXStandard::X4).unwrap_err().to_string();
    assert!(e.contains("printer (output) profile"), "{e}");
    // There is no ICC version 3: PDF/X-4 takes 2 and 4 only.
    let e = with(patched(&v2_cmyk(), 8, &[3]), PdfXStandard::X4).unwrap_err().to_string();
    assert!(e.contains("version 3"), "{e}");
    assert!(with(patched(&v2_cmyk(), 8, &[4]), PdfXStandard::X4).is_ok());
    assert!(with(patched(&v2_cmyk(), 8, &[4]), PdfXStandard::X3_2002).is_err());
}

#[test]
fn no_conversion_keeps_a_gray_documents_own_profile() {
    let gray = as_v2(photocraft_cms::Builtin::SGray.profile());
    let out = export_with_options(&doc(ColorMode::Grayscale), &pdfx(PdfXStandard::X3_2002, Some(gray.clone()), false, 0.0), &Interrupt::NONE).unwrap();
    check_structure(&out.bytes);
    assert!(has(&out.bytes, "/ColorSpace [/ICCBased 8 0 R]") && has(&out.bytes, "/N 1 /Alternate /DeviceGray"));
    assert_eq!(stream_after(&out.bytes, "/N 1 /Alternate"), *gray);
}

#[test]
fn control_characters_in_the_title_agree_in_info_and_xmp() {
    let mut d = doc(ColorMode::Rgb);
    d.name = "Plakat	Tour
.psd"
        .into();
    let opts = pdfx(PdfXStandard::X4, None, true, 0.0);
    let out = export_x(&d, opts.x.as_ref().unwrap(), 0, &Interrupt::NONE).unwrap();
    assert!(has(&out.bytes, "/Title (Plakat Tour)"));
    assert!(between(&out.bytes, "/Subtype /XML", "endstream").contains(">Plakat Tour</rdf:li>"));
}

#[test]
fn the_output_condition_is_custom_with_its_profile_embedded() {
    // A profile's name can't prove the press condition it was made for, so none is claimed.
    let out = export_with_options(&doc(ColorMode::Rgb), &x1a(None, 0.0), &Interrupt::NONE).unwrap();
    let condition = photocraft_cms::Profile::parse(&v2_cmyk()).unwrap().description;
    assert!(has(
        &out.bytes,
        &format!("/OutputConditionIdentifier (Custom) /OutputCondition {} /Info {} /DestOutputProfile", pdf_text(&condition), pdf_text(&condition))
    ));
    assert!(!has(&out.bytes, "/RegistryName"), "Custom is no registry entry");
    assert!(has(&out.bytes, "/DestOutputProfile 6 0 R"), "Custom needs the embedded profile");
}

#[test]
fn a_device_link_document_profile_is_never_embedded() {
    let link = patched(&as_v2(photocraft_cms::Builtin::Srgb.profile()), 12, b"link");
    let e = export_with_options(&doc(ColorMode::Rgb), &pdfx(PdfXStandard::X3_2002, Some(link), false, 0.0), &Interrupt::NONE).unwrap_err().to_string();
    assert!(e.contains("DeviceLink"), "{e}");
    let abstract_ = patched(&as_v2(photocraft_cms::Builtin::Srgb.profile()), 12, b"abst");
    let e = export_with_options(&doc(ColorMode::Rgb), &pdfx(PdfXStandard::X3_2002, Some(abstract_), false, 0.0), &Interrupt::NONE).unwrap_err().to_string();
    assert!(e.contains("Abstract"), "{e}");
}

#[test]
fn a_non_ascii_output_condition_survives_as_text() {
    assert_eq!(pdf_text("Druckerei Köln"), format!("<FEFF{}>", "Druckerei Köln".encode_utf16().map(|u| format!("{u:04X}")).collect::<String>()));
    assert_eq!(pdf_text("ISO Coated v2 (ECI)"), r"(ISO Coated v2 \(ECI\))", "ASCII keeps its literal form");
}

#[test]
fn the_file_id_changes_with_the_bleed_and_the_profile() {
    let d = doc(ColorMode::Rgb);
    let id = |o: &PdfOptions| {
        let out = export_x(&d, o.x.as_ref().unwrap(), 7, &Interrupt::NONE).unwrap();
        between(&out.bytes, "/ID [<", ">")
    };
    let mut other = x1a(None, 0.0);
    if let Some(x) = &mut other.x {
        x.profile = patched(&x.profile, 64, &[1]);
    }
    assert_ne!(id(&x1a(None, 0.0)), id(&x1a(None, 0.5)), "bleed");
    assert_ne!(id(&x1a(None, 0.0)), id(&other), "profile bytes");
    assert_eq!(id(&x1a(None, 0.0)), id(&x1a(None, 0.0)), "the same save, the same id");
}

/// Absolute colorimetric keeps the source's paper white as the CMS converts it (that is the
/// intent's point); the other intents still map paper white to no ink.
#[test]
fn absolute_colorimetric_keeps_the_simulated_paper_white() {
    use photocraft_cms::{Builtin, Intent, Transform};
    let mut src = Builtin::Srgb.profile().clone();
    src.white_point = [0.6, 0.65, 0.5];
    let src = src.with_encoded_bytes();
    let dst = Builtin::CoatedCmyk.profile().clone();
    let mut expected = [0.0f32; 4];
    Transform::new(&src, &dst, Intent::AbsoluteColorimetric, true).unwrap().convert_f32(&[1.0, 1.0, 1.0], 3, &mut expected, 4, false);
    let expected: Vec<u8> = expected.iter().map(|v| to8(*v)).collect();
    assert!(expected.iter().any(|&v| v > 0), "the probe has a tinted paper white: {expected:?}");
    let export = |intent| {
        let mut o = pdfx(PdfXStandard::X4, Some(src.to_bytes()), true, 0.0);
        if let Some(x) = &mut o.x {
            x.profile = dst.to_bytes();
            x.intent = intent;
        }
        let out = export_with_options(&doc(ColorMode::Rgb), &o, &Interrupt::NONE).unwrap();
        stream_after(&out.bytes, "/Subtype /Image")[..4].to_vec()
    };
    assert_eq!(export(Intent::AbsoluteColorimetric), expected, "the simulated paper white");
    assert_eq!(export(Intent::RelativeColorimetric), [0, 0, 0, 0], "paper white is no ink");
}

/// A bleed that leaves a trim only before rounding to the written 0.01 pt is refused, on either
/// axis; an ordinary bleed writes a positive trim box.
#[test]
fn the_trim_box_stays_positive_as_written() {
    // The 4 × 2 px page at 72 dpi is 4 × 2 pt: 0.999 pt leaves 0.002 pt of height, 0.00 written.
    let e = export_with_options(&doc(ColorMode::Rgb), &x1a(None, 0.999), &Interrupt::NONE).unwrap_err().to_string();
    assert!(e.contains("leaves no trim"), "{e}");
    let mut tall = doc(ColorMode::Rgb);
    tall.size = Size { width: 2, height: 4 };
    let e = export_with_options(&tall, &x1a(None, 0.999), &Interrupt::NONE).unwrap_err().to_string();
    assert!(e.contains("leaves no trim"), "width: {e}");
    let out = export_with_options(&doc(ColorMode::Rgb), &x1a(None, 0.5), &Interrupt::NONE).unwrap();
    assert!(has(&out.bytes, "/TrimBox [0.50 0.50 3.50 1.50]"));
}

#[test]
fn unicode_text_over_a_photo_extracts_through_a_pdf_reader() {
    let mut d = doc(ColorMode::Rgb);
    d.size = Size { width: 128, height: 48 };
    let fmt = d.pixel_format();
    // Synthetic photo-like backdrop; no personal image or third-party asset.
    d.layers = vec![Layer::new("Photo", LayerContent::Raster(solid(fmt, 0, 0, 128, 48, &[0.2, 0.4, 0.6, 1.0])))];
    let text = "café Ω 😀";
    d.layers.push(Layer::new(
        "Text",
        LayerContent::Text(photocraft_doc::TextLayer {
            text: text.into(),
            size_pt: 12.0,
            transform: photocraft_geom::Affine::translate(2.0, 20.0),
            ..Default::default()
        }),
    ));
    let pdf = export(&d, &Interrupt::NONE).unwrap().bytes;
    let parsed = lopdf::Document::load_mem(&pdf).unwrap();
    assert_eq!(parsed.extract_text(&[1]).unwrap().trim(), text);
    assert!(has(&pdf, "/Subtype /Image"));
    assert!(has(&pdf, "/Subtype /Type3"));
    assert!(has(&pdf, "/ToUnicode"));
    assert_eq!(&stream_after(&pdf, "/Subtype /Image")[..3], &[51, 102, 153]);
    let content = String::from_utf8(stream_after(&pdf, "/Contents 4 0 R")).unwrap();
    assert!(!content.contains("/ActualText <FEFF00630061006600E9"), "selection spans are clusters, not the entire layer");
}

#[test]
fn text_with_compositing_or_a_profile_stays_in_the_raster_backdrop() {
    for case in 0..5 {
        let mut d = doc(ColorMode::Rgb);
        d.size = Size { width: 32, height: 32 };
        let mut layer = Layer::new(
            "Text",
            LayerContent::Text(photocraft_doc::TextLayer {
                text: "A".into(),
                size_pt: 16.0,
                transform: photocraft_geom::Affine::translate(2.0, 20.0),
                ..Default::default()
            }),
        );
        // The compositor consumes the text cache maintained by the engine/UI.
        if let LayerContent::Text(text) = &mut layer.content {
            photocraft_text::TextEngine::new().render_layer(text, d.resolution_dpi, d.pixel_format());
        }
        match case {
            0 => layer.opacity = 0.5,
            1 => layer.fill_opacity = 0.5,
            2 => layer.blend = photocraft_color::BlendMode::Multiply,
            3 => {
                layer.effects.enabled = true;
                layer.effects.items.push(photocraft_doc::effects::Effect::default_drop_shadow());
            }
            _ => d.icc_profile = Some(std::sync::Arc::new(vec![1, 2, 3, 4])),
        }
        d.layers.push(layer);
        let pdf = export(&d, &Interrupt::NONE).unwrap().bytes;
        let content = String::from_utf8(stream_after(&pdf, "/Contents 4 0 R")).unwrap();
        assert!(!content.contains("/ActualText"), "case {case} must retain the raster composite");
        assert!(stream_after(&pdf, "/Subtype /Image").iter().any(|v| *v != 255), "case {case} must retain visible text");
        if case == 4 {
            assert!(has(&pdf, "[/ICCBased 7 0 R]"));
        }
    }
}

/// 300 distinct characters: more than the selection font's 255 one-byte codes.
fn many_distinct_chars() -> String {
    let text: String = (0x100u32..0x22C).filter_map(char::from_u32).collect();
    assert_eq!(text.chars().count(), 300);
    text
}

#[test]
fn text_outside_the_vector_prefix_does_not_consume_the_selection_font() {
    use photocraft_doc::TextLayer;
    let many = many_distinct_chars();
    for case in 0..2 {
        let mut d = doc(ColorMode::Rgb);
        // Hidden in the first case; in the second a raster layer with reduced opacity sits above
        // it, so scanning stops before the oversized repertoire.
        let mut lower = Layer::new("Lower text", LayerContent::Text(TextLayer { text: many.clone(), size_pt: 12.0, ..Default::default() }));
        lower.visible = case == 1;
        d.layers.push(lower);
        if case == 1 {
            let fmt = d.pixel_format();
            let mut blocker = Layer::new("Blocker", LayerContent::Raster(solid(fmt, 0, 0, 4, 2, &[0.0, 0.0, 0.0, 0.5])));
            blocker.opacity = 0.5;
            d.layers.push(blocker);
        }
        d.layers.push(Layer::new(
            "Top text",
            LayerContent::Text(TextLayer { text: "Hi".into(), size_pt: 12.0, color: photocraft_color::Color::rgb(0.0, 1.0, 0.0), ..Default::default() }),
        ));
        let pdf = export(&d, &Interrupt::NONE).unwrap().bytes;
        let parsed = lopdf::Document::load_mem(&pdf).unwrap();
        assert_eq!(parsed.extract_text(&[1]).unwrap().trim(), "Hi", "case {case}: only the top text is selectable");
        let content = String::from_utf8(stream_after(&pdf, "/Contents 4 0 R")).unwrap();
        assert!(content.contains("/ActualText <FEFF0048"), "case {case}: the top text must stay vector:\n{content}");
        assert!(content.contains("/FSelect"), "case {case}");
    }
}

#[test]
fn a_text_layer_refused_at_emission_keeps_the_upper_vectors_and_stays_raster() {
    use photocraft_doc::{TextLayer, text::TextShape};
    let mut d = doc(ColorMode::Rgb);
    d.size = Size { width: 128, height: 48 };
    let fmt = d.pixel_format();
    d.layers = vec![Layer::new("Paper", LayerContent::Raster(solid(fmt, 0, 0, 128, 48, &[1.0, 1.0, 1.0, 1.0])))];
    // A box too short for its paragraph: the layout hides the overflow, so `prepare` accepts
    // the layer (its checks only see placed clusters) while emission refuses it, because the
    // hidden, unpainted source text would otherwise show up in the selection plane.
    let mut lower = Layer::new(
        "Lower text",
        LayerContent::Text(TextLayer {
            text: "The quick brown fox jumps over the lazy dog again and again".into(),
            font_family: "Inter".into(),
            size_pt: 12.0,
            color: photocraft_color::Color::rgb(1.0, 0.0, 0.0),
            shape: TextShape::Box { x: 0.0, y: 0.0, width: 100.0, height: 30.0 },
            transform: photocraft_geom::Affine::translate(2.0, 10.0),
            ..Default::default()
        }),
    );
    // The compositor consumes the text cache maintained by the engine/UI.
    if let LayerContent::Text(text) = &mut lower.content {
        photocraft_text::TextEngine::new().render_layer(text, d.resolution_dpi, fmt);
    }
    // Pin the fixture to the intended path: accepted by `prepare`, refused by `ops`.
    let LayerContent::Text(lower_text) = &lower.content else { panic!("the fixture is a text layer") };
    let prepared = super::text::prepare(lower_text, ColorMode::Rgb, d.resolution_dpi).expect("prepare accepts the truncated box");
    let lower_font = super::selection::SelectionFont::new(std::iter::once(lower_text.text.as_str()));
    assert!(super::text::ops(&prepared, ColorMode::Rgb, &lower_font).is_none(), "emission must refuse the unrepresented source text");
    d.layers.push(lower);
    d.layers.push(Layer::new(
        "Top text",
        LayerContent::Text(TextLayer {
            text: "Hi".into(),
            font_family: "Inter".into(),
            size_pt: 12.0,
            color: photocraft_color::Color::rgb(0.0, 1.0, 0.0),
            ..Default::default()
        }),
    ));
    let pdf = export(&d, &Interrupt::NONE).unwrap().bytes;
    check_structure(&pdf);
    let content = String::from_utf8(stream_after(&pdf, "/Contents 4 0 R")).unwrap();
    assert!(content.contains("/ActualText <FEFF0048"), "the upper text stays vector:\n{content}");
    assert!(content.contains("0 1 0 rg"), "the upper text keeps its fill:\n{content}");
    assert!(content.contains("/FSelect"), "the selection font survives the refused layer");
    assert!(!content.contains("1 0 0 rg"), "the refused layer must not be vector:\n{content}");
    assert!(!content.contains("/ActualText <FEFF0054"), "its text must stay out of the selection plane:\n{content}");
    let parsed = lopdf::Document::load_mem(&pdf).unwrap();
    assert_eq!(parsed.extract_text(&[1]).unwrap().trim(), "Hi", "only the upper text is selectable");
    let img = stream_after(&pdf, "/Subtype /Image");
    assert!(
        img.chunks(3).any(|p| i16::from(p[0]) - i16::from(p[1]) > 8 && i16::from(p[0]) - i16::from(p[2]) > 8),
        "the refused red text stays visible in the raster backdrop"
    );
}

#[test]
fn multiline_text_keeps_its_paragraph_breaks_in_the_selection() {
    use photocraft_doc::TextLayer;
    let mut d = doc(ColorMode::Rgb);
    d.layers.push(Layer::new("Text", LayerContent::Text(TextLayer { text: "A\nB".into(), size_pt: 12.0, ..Default::default() })));
    let pdf = export(&d, &Interrupt::NONE).unwrap().bytes;
    let parsed = lopdf::Document::load_mem(&pdf).unwrap();
    assert_eq!(parsed.extract_text(&[1]).unwrap().trim(), "A\nB", "the paragraph break survives selection");
    let content = String::from_utf8(stream_after(&pdf, "/Contents 4 0 R")).unwrap();
    assert!(content.contains("/ActualText <FEFF0041000A"), "the first span carries the break:\n{content}");
}

#[test]
fn leading_newline_stays_in_the_first_selection_span() {
    use photocraft_doc::TextLayer;
    let mut d = doc(ColorMode::Rgb);
    d.layers.push(Layer::new("Text", LayerContent::Text(TextLayer { text: "\nA".into(), size_pt: 12.0, ..Default::default() })));
    let pdf = export(&d, &Interrupt::NONE).unwrap().bytes;
    let content = String::from_utf8(stream_after(&pdf, "/Contents 4 0 R")).unwrap();
    assert!(content.contains("/ActualText <FEFF000A0041>"), "the first span retains the leading newline:\n{content}");
}

#[test]
fn pdf_x_keeps_text_in_the_image_without_a_font() {
    use photocraft_doc::TextLayer;
    // A plain PDF draws this top text layer as vectors with a selection font; PDF/X-1a can't hold
    // an unembedded font or RGB vectors, so there it stays in the converted image.
    let mut d = doc(ColorMode::Rgb);
    d.layers.push(Layer::new("Text", LayerContent::Text(TextLayer { text: "Print".into(), size_pt: 12.0, ..Default::default() })));
    assert!(has(&export(&d, &Interrupt::NONE).unwrap().bytes, "/FSelect"));
    let pdf = export_with_options(&d, &x1a(None, 0.5), &Interrupt::NONE).unwrap().bytes;
    assert!(!has(&pdf, "/Font") && !has(&pdf, "/FSelect") && !has(&pdf, "/Type3"));
    let content = String::from_utf8(stream_after(&pdf, "/Contents 4 0 R")).unwrap();
    assert!(!content.contains("BT") && !content.contains("ActualText"), "{content}");
}

#[test]
/// Each artboard becomes one labelled page, in document order.
fn artboards_export_as_one_page_each() {
    use photocraft_doc::{Artboard, Group};
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    let mut d = Document::new("pages.pdf", Size { width: 40, height: 30 }, ColorMode::Rgb, SampleType::F32);
    d.resolution_dpi = 72.0;
    for (rect, px) in [(Rect::new(0, 0, 20, 30), [1.0, 0.0, 0.0, 1.0]), (Rect::new(20, 0, 40, 30), [0.0, 0.0, 1.0, 1.0])] {
        let layer = Layer::new("Pixels", LayerContent::Raster(solid(fmt, rect.x0, rect.y0, rect.x1, rect.y1, &px)));
        let group = Group { children: vec![layer], expanded: true, artboard: Some(Artboard::new(rect)), frame_stroke: None };
        d.layers.push(Layer::new("Page", LayerContent::Group(group)));
    }
    let out = export(&d, &Interrupt::NONE).unwrap();
    check_structure(&out.bytes);
    let parsed = lopdf::Document::load_mem(&out.bytes).unwrap();
    let pages = parsed.get_pages();
    assert_eq!(pages.len(), 2);
    for id in pages.values() {
        let page = parsed.get_dictionary(*id).unwrap();
        let media = page.get(b"MediaBox").unwrap().as_array().unwrap();
        let w = media[2].as_float().or_else(|_| media[2].as_i64().map(|v| v as f32)).unwrap();
        let h = media[3].as_float().or_else(|_| media[3].as_i64().map(|v| v as f32)).unwrap();
        assert!((w - 20.0).abs() < 0.01 && (h - 30.0).abs() < 0.01, "page {w}x{h}");
    }
    assert_eq!(out.bytes.windows(15).filter(|w| *w == b"/Subtype /Image").count(), 2);
}

/// Side-by-side 2x2 artboards at 72 dpi, named by `names` (empty names are kept).
fn artboard_doc(names: &[&str]) -> Document {
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    let width = (names.len() as u32).saturating_mul(2).max(2);
    let mut d = Document::new("labels.pdf", Size { width, height: 2 }, ColorMode::Rgb, SampleType::F32);
    d.resolution_dpi = 72.0;
    for (index, name) in names.iter().enumerate() {
        let x0 = (index as i32) * 2;
        let rect = Rect::new(x0, 0, x0 + 2, 2);
        let layer = Layer::new("Pixels", LayerContent::Raster(solid(fmt, rect.x0, rect.y0, rect.x1, rect.y1, &[0.5, 0.5, 0.5, 1.0])));
        let group = Group { children: vec![layer], expanded: true, artboard: Some(Artboard::new(rect)), frame_stroke: None };
        d.layers.push(Layer::new(*name, LayerContent::Group(group)));
    }
    d
}

/// The decompressed samples of page `id`'s first image XObject (`/Im0`).
fn page_image(pdf: &lopdf::Document, id: lopdf::ObjectId) -> Vec<u8> {
    let page = pdf.get_dictionary(id).expect("page dictionary");
    let resources = page.get(b"Resources").expect("page resources").as_dict().expect("resources dictionary");
    let xobjects = resources.get(b"XObject").expect("xobjects").as_dict().expect("xobject dictionary");
    let image = xobjects.get(b"Im0").expect("Im0").as_reference().expect("image reference");
    pdf.get_object(image).expect("image object").as_stream().expect("image stream").decompressed_content().expect("image samples")
}

/// A named artboard gives every page a `/PageLabels` entry, and an empty name gets an empty
/// label dictionary rather than inheriting the previous name.
#[test]
fn page_labels_name_every_page_and_an_empty_name_does_not_inherit() {
    let d = artboard_doc(&["A", "", "C"]);
    let pdf = export(&d, &Interrupt::NONE).unwrap().bytes;
    check_structure(&pdf);
    let parsed = lopdf::Document::load_mem(&pdf).unwrap();
    let labels = parsed
        .catalog()
        .unwrap()
        .get(b"PageLabels")
        .expect("a named artboard gives the PDF page labels")
        .as_reference()
        .expect("the label tree is an indirect object");
    let nums = parsed.get_dictionary(labels).unwrap().get(b"Nums").unwrap().as_array().unwrap();
    assert_eq!(nums.len(), 6, "one key and one label dictionary per page");
    for page in 0..3 {
        assert_eq!(nums[page * 2].as_i64().unwrap(), page as i64, "the tree starts at page index 0 and covers every page");
    }
    let label = |page: usize| nums[page * 2 + 1].as_dict().unwrap();
    assert_eq!(label(0).get(b"P").unwrap(), &Object::String(vec![0xFE, 0xFF, 0x00, 0x41], StringFormat::Hexadecimal), "page 1 is A");
    assert!(label(1).get(b"P").is_err(), "the empty name gets an empty dictionary, not A");
    assert_eq!(label(2).get(b"P").unwrap(), &Object::String(vec![0xFE, 0xFF, 0x00, 0x43], StringFormat::Hexadecimal), "page 3 is C");
    assert_eq!(parsed.get_pages().len(), 3);
}

/// With no non-empty name there is no label tree at all: an empty `/Nums` would be invalid.
#[test]
fn an_all_empty_artboard_names_document_has_no_page_labels_tree() {
    let d = artboard_doc(&["", "", ""]);
    let pdf = export(&d, &Interrupt::NONE).unwrap().bytes;
    check_structure(&pdf);
    let parsed = lopdf::Document::load_mem(&pdf).unwrap();
    assert!(parsed.catalog().unwrap().get(b"PageLabels").is_err(), "no names means no /PageLabels");
    assert!(!has(&pdf, "/PageLabels"));
    assert_eq!(parsed.get_pages().len(), 3, "the pages themselves are still written");
}

/// Two artboards at the same rectangle: each page must hold only its own board's pixels, not
/// the other board composited over it.
#[test]
fn overlapping_artboards_export_only_their_own_pixels() {
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    let mut d = Document::new("overlap.pdf", Size { width: 2, height: 2 }, ColorMode::Rgb, SampleType::F32);
    d.resolution_dpi = 72.0;
    for (name, color) in [("Red", [1.0, 0.0, 0.0, 1.0]), ("Blue", [0.0, 0.0, 1.0, 1.0])] {
        let layer = Layer::new("Pixels", LayerContent::Raster(solid(fmt, 0, 0, 2, 2, &color)));
        let group = Group { children: vec![layer], expanded: true, artboard: Some(Artboard::new(Rect::new(0, 0, 2, 2))), frame_stroke: None };
        d.layers.push(Layer::new(name, LayerContent::Group(group)));
    }
    let pdf = export(&d, &Interrupt::NONE).unwrap().bytes;
    let parsed = lopdf::Document::load_mem(&pdf).unwrap();
    let pages = parsed.get_pages();
    let images = pages.values().map(|id| page_image(&parsed, *id)).collect::<Vec<_>>();
    assert_eq!(images.len(), 2);
    assert_eq!(images[0], [255u8, 0, 0].repeat(4), "the bottom board's page is red, not blue over red");
    assert_eq!(images[1], [0u8, 0, 255].repeat(4), "the top board's page is blue");
}

/// A large, mostly empty canvas must not be charged as if its whole area were rendered.
#[test]
fn sparse_canvas_artboards_export_without_a_false_memory_limit() {
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    let mut d = Document::new("sparse.pdf", Size { width: 9000, height: 9000 }, ColorMode::Rgb, SampleType::F32);
    d.resolution_dpi = 72.0;
    for index in 0..2 {
        let x0 = index * 10;
        let rect = Rect::new(x0, 0, x0 + 2, 2);
        let layer = Layer::new("Pixels", LayerContent::Raster(solid(fmt, rect.x0, 0, rect.x1, 2, &[1.0, 0.0, 0.0, 1.0])));
        let group = Group { children: vec![layer], expanded: true, artboard: Some(Artboard::new(rect)), frame_stroke: None };
        d.layers.push(Layer::new("Page", LayerContent::Group(group)));
    }
    let out = export(&d, &Interrupt::NONE).expect("two 2x2 boards on a 9000x9000 canvas must fit the export budget");
    let parsed = lopdf::Document::load_mem(&out.bytes).unwrap();
    assert_eq!(parsed.get_pages().len(), 2);
}

/// Seven US Letter boards at 300 dpi with the importer's gaps: the pages are retained as encoded
/// data, but no whole-canvas buffer exists, so the old whole-canvas charge rejected this.
#[test]
fn seven_us_letter_artboards_export_without_a_false_memory_limit() {
    const PAGE_W: u32 = 2550;
    const PAGE_H: u32 = 3300;
    const GAP: u32 = 80;
    const PAGES: u32 = 7;
    let mut d = Document::new("letters.pdf", Size { width: PAGES * PAGE_W + (PAGES - 1) * GAP, height: PAGE_H }, ColorMode::Rgb, SampleType::F32);
    d.resolution_dpi = 300.0;
    for index in 0..PAGES {
        let x0 = (index * (PAGE_W + GAP)) as i32;
        let rect = Rect::new(x0, 0, x0 + PAGE_W as i32, PAGE_H as i32);
        let group = Group { children: Vec::new(), expanded: true, artboard: Some(Artboard::new(rect)), frame_stroke: None };
        d.layers.push(Layer::new(format!("Letter {}", index + 1), LayerContent::Group(group)));
    }
    let out = export(&d, &Interrupt::NONE).expect("seven US Letter pages at 300 dpi fit the export budget");
    let parsed = lopdf::Document::load_mem(&out.bytes).unwrap();
    assert_eq!(parsed.get_pages().len(), 7);
}

/// PDF/X writes one page, and says so when the document has artboards.
#[test]
fn pdf_x_says_artboards_share_one_page() {
    use photocraft_doc::{Artboard, Group};
    let mut d = doc(ColorMode::Rgb);
    let rect = Rect::new(0, 0, d.size.width as i32, d.size.height as i32);
    let group = Group { children: Vec::new(), expanded: true, artboard: Some(Artboard::new(rect)), frame_stroke: None };
    d.layers.push(Layer::new("Board", LayerContent::Group(group)));
    let out = export_with_options(&d, &x1a(None, 0.0), &Interrupt::NONE).unwrap();
    assert_eq!(lopdf::Document::load_mem(&out.bytes).unwrap().get_pages().len(), 1);
    assert!(out.warnings.iter().any(|w| w.contains("artboards")), "{:?}", out.warnings);
}
