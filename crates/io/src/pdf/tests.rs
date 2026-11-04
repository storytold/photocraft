//! PDF export: structure, pixel oracle, colour modes and refusal paths.

use super::*;
use photocraft_color::{PixelFormat, SampleType};
use photocraft_doc::{Layer, LayerContent, Size};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

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
    let out = export(&d).unwrap();
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
    let out = export(&d).unwrap();
    let img = stream_after(&out.bytes, "/Subtype /Image");
    // Pixels 1-2 of row 0: 0.5 red over white = (1.0, 0.5, 0.5); elsewhere plain white.
    assert_eq!(&img[3..6], &[255, 128, 128], "the blend lands: {:?}", &img[3..6]);
    assert_eq!(&img[0..3], &[255, 255, 255]);
}

#[test]
fn greyscale_and_cmyk_pages_keep_their_channels() {
    let g = export(&doc(ColorMode::Grayscale)).unwrap();
    assert!(has(&g.bytes, "/ColorSpace /DeviceGray"), "gray is DeviceGray");
    assert_eq!(stream_after(&g.bytes, "/Subtype /Image"), vec![255; 4 * 2], "one sample per pixel");
    let c = export(&doc(ColorMode::Cmyk)).unwrap();
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
    let out = export(&d).unwrap();
    assert!(has(&out.bytes, "/MediaBox [0 0 72.00 36.00]"), "300 px at 300 dpi is one inch");
}

#[test]
fn an_embedded_profile_travels_as_iccbased() {
    let mut d = doc(ColorMode::Rgb);
    d.icc_profile = Some(std::sync::Arc::new(vec![1, 2, 3, 4, 5, 6, 7, 8]));
    let out = export(&d).unwrap();
    assert!(has(&out.bytes, "[/ICCBased 7 0 R]"), "the image refers to the profile");
    assert_eq!(stream_after(&out.bytes, "/N 3 /Alternate"), vec![1, 2, 3, 4, 5, 6, 7, 8], "the profile bytes survive");
}

#[test]
fn other_modes_are_refused_with_an_actionable_error() {
    let d = doc(ColorMode::Lab);
    let e = export(&d).unwrap_err().to_string();
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

fn x1a(source: Option<std::sync::Arc<Vec<u8>>>, bleed_pt: f64) -> PdfOptions {
    PdfOptions { x1a: Some(PdfX1a { profile: v2_cmyk(), source, intent: photocraft_cms::Intent::RelativeColorimetric, bpc: true, bleed_pt }) }
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
    let out = export_with_options(&d, &x1a(None, 0.0)).unwrap();
    let img = stream_after(&out.bytes, "/Subtype /Image");
    assert_eq!(&img[..4], &[255, 128, 0, 51], "no conversion");
    assert!(!out.warnings.iter().any(|w| w.contains("converted")));
}

#[test]
fn x1a_converts_a_cmyk_document_from_another_condition() {
    let d = doc(ColorMode::Cmyk);
    let other = photocraft_cms::Builtin::CoatedCmyk.profile().to_bytes();
    let out = export_with_options(&d, &x1a(Some(other), 0.0)).unwrap();
    assert!(out.warnings.iter().any(|w| w.contains("converted")), "{:?}", out.warnings);
}

#[test]
fn x1a_keeps_gray_as_device_gray() {
    let out = export_with_options(&doc(ColorMode::Grayscale), &x1a(None, 0.0)).unwrap();
    assert!(has(&out.bytes, "/ColorSpace /DeviceGray"));
    assert!(has(&out.bytes, "/GTS_PDFXConformance (PDF/X-1a:2001)"));
}

#[test]
fn x1a_refuses_what_it_cant_write() {
    let d = doc(ColorMode::Rgb);
    let v4 = PdfOptions { x1a: Some(PdfX1a { profile: photocraft_cms::Builtin::CoatedCmyk.profile().to_bytes(), ..x1a(None, 0.0).x1a.unwrap() }) };
    let e = export_with_options(&d, &v4).unwrap_err().to_string();
    assert!(e.contains("version 2"), "{e}");
    let rgb = PdfOptions { x1a: Some(PdfX1a { profile: photocraft_cms::Builtin::Srgb.profile().to_bytes(), ..x1a(None, 0.0).x1a.unwrap() }) };
    assert!(export_with_options(&d, &rgb).unwrap_err().to_string().contains("CMYK"));
    // The 4 × 2 px page at 72 dpi is 4 × 2 pt: a 1 pt bleed leaves nothing to trim.
    assert!(export_with_options(&d, &x1a(None, 1.0)).unwrap_err().to_string().contains("bleed"));
    assert!(export_with_options(&doc(ColorMode::Lab), &x1a(None, 0.0)).is_err());
}

#[test]
fn output_conditions_and_dates() {
    assert_eq!(output_condition_id("Coated FOGRA39 (ISO 12647-2:2004)"), "FOGRA39");
    assert_eq!(output_condition_id("ISO Coated v2 (ECI)"), "FOGRA39");
    assert_eq!(output_condition_id("PSO Coated v3"), "FOGRA51");
    assert_eq!(output_condition_id("U.S. Web Coated (SWOP) v2"), "CGATS TR 001");
    assert_eq!(output_condition_id("PhotoCraft Coated CMYK"), "Custom");
    assert_eq!(pdf_date(0), "D:19700101000000+00'00'");
    assert_eq!(pdf_date(1_760_000_000), "D:20251009085320+00'00'");
    assert_eq!(file_id(b"abc", 1).len(), 32);
}
