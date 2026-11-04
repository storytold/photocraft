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
    let v4 = PdfOptions { x: Some(PdfX { profile: photocraft_cms::Builtin::CoatedCmyk.profile().to_bytes(), ..x1a(None, 0.0).x.unwrap() }) };
    let e = export_with_options(&d, &v4).unwrap_err().to_string();
    assert!(e.contains("version 2"), "{e}");
    let rgb = PdfOptions { x: Some(PdfX { profile: photocraft_cms::Builtin::Srgb.profile().to_bytes(), ..x1a(None, 0.0).x.unwrap() }) };
    assert!(export_with_options(&d, &rgb).unwrap_err().to_string().contains("CMYK"));
    // The 4 × 2 px page at 72 dpi is 4 × 2 pt: a 1 pt bleed leaves nothing to trim.
    assert!(export_with_options(&d, &x1a(None, 1.0)).unwrap_err().to_string().contains("bleed"));
    assert!(export_with_options(&doc(ColorMode::Lab), &x1a(None, 0.0)).is_err());
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
    let out = export_with_options(&d, &pdfx(PdfXStandard::X3_2002, Some(srgb.clone()), false, 0.0)).unwrap();
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
    let out = export_with_options(&doc(ColorMode::Rgb), &pdfx(PdfXStandard::X3_2002, Some(srgb), true, 0.0)).unwrap();
    check_structure(&out.bytes);
    assert!(has(&out.bytes, "/ColorSpace /DeviceCMYK") && !has(&out.bytes, "/ICCBased"));
    assert!(out.warnings.iter().any(|w| w.contains("PDF/X-3:2002: colour converted")), "{:?}", out.warnings);
}

#[test]
fn x3_refuses_version_4_profiles_pdf_1_3_cant_hold() {
    let d = doc(ColorMode::Rgb);
    // The built-in sRGB is ICC v4: it can't be embedded, only converted from.
    let v4 = photocraft_cms::Builtin::Srgb.profile().to_bytes();
    let e = export_with_options(&d, &pdfx(PdfXStandard::X3_2002, Some(v4.clone()), false, 0.0)).unwrap_err().to_string();
    assert!(e.starts_with("unsupported") || e.contains("PDF/X-3:2002"), "{e}");
    assert!(e.contains("can't embed") && e.contains("save PDF/X-4"), "{e}");
    assert!(export_with_options(&d, &pdfx(PdfXStandard::X3_2002, Some(v4), true, 0.0)).is_ok(), "converting from it is fine");
    let mut opts = pdfx(PdfXStandard::X3_2002, None, true, 0.0);
    if let Some(x) = &mut opts.x {
        x.profile = photocraft_cms::Builtin::CoatedCmyk.profile().to_bytes();
    }
    let e = export_with_options(&d, &opts).unwrap_err().to_string();
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
    let out = export_x(&d, x, 1_760_000_000).unwrap();
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
            let out = export_with_options(&d, &pdfx(standard, None, convert, 0.0)).unwrap();
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
        export_with_options(&d, &o)
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
    let out = export_with_options(&doc(ColorMode::Grayscale), &pdfx(PdfXStandard::X3_2002, Some(gray.clone()), false, 0.0)).unwrap();
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
    let out = export_x(&d, opts.x.as_ref().unwrap(), 0).unwrap();
    assert!(has(&out.bytes, "/Title (Plakat Tour)"));
    assert!(between(&out.bytes, "/Subtype /XML", "endstream").contains(">Plakat Tour</rdf:li>"));
}

#[test]
fn the_output_condition_is_custom_with_its_profile_embedded() {
    // A profile's name can't prove the press condition it was made for, so none is claimed.
    let out = export_with_options(&doc(ColorMode::Rgb), &x1a(None, 0.0)).unwrap();
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
    let e = export_with_options(&doc(ColorMode::Rgb), &pdfx(PdfXStandard::X3_2002, Some(link), false, 0.0)).unwrap_err().to_string();
    assert!(e.contains("DeviceLink"), "{e}");
    let abstract_ = patched(&as_v2(photocraft_cms::Builtin::Srgb.profile()), 12, b"abst");
    let e = export_with_options(&doc(ColorMode::Rgb), &pdfx(PdfXStandard::X3_2002, Some(abstract_), false, 0.0)).unwrap_err().to_string();
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
        let out = export_x(&d, o.x.as_ref().unwrap(), 7).unwrap();
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
