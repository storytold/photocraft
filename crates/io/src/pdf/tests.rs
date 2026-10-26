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

// ---------- Multi-page (FILE-215-10) ----------

fn page_of(mode: ColorMode, w: u32, h: u32, dpi: f32) -> PrintPage {
    let mut d = doc(mode);
    d.size = Size { width: w, height: h };
    let fmt = PixelFormat::new(mode, SampleType::F32, true);
    d.layers = vec![Layer::new("Paper", LayerContent::Raster(solid(fmt, 0, 0, w as i32, h as i32, &paper(mode))))];
    d.resolution_dpi = dpi;
    flat_page(&d, OtherModes::Refuse).unwrap()
}

#[test]
fn pages_keep_their_order_size_and_colour_space() {
    let mut w = PdfWriter::new();
    w.add_page(&page_of(ColorMode::Rgb, 72, 144, 72.0)).unwrap();
    w.add_page(&page_of(ColorMode::Cmyk, 300, 150, 300.0)).unwrap();
    w.add_page(&page_of(ColorMode::Grayscale, 10, 10, 72.0)).unwrap();
    assert_eq!(w.pages(), 3);
    let pdf = w.finish();
    check_structure(&pdf);
    assert!(has(&pdf, "/Kids [4 0 R 7 0 R 10 0 R] /Count 3"), "page order = add order");
    let text = String::from_utf8_lossy(&pdf);
    let boxes: Vec<&str> = text.match_indices("/MediaBox [").map(|(i, _)| &text[i..i + text[i..].find(']').unwrap() + 1]).collect();
    assert_eq!(boxes, ["/MediaBox [0 0 72.00 144.00]", "/MediaBox [0 0 72.00 36.00]", "/MediaBox [0 0 10.00 10.00]"], "each page at its own resolution");
    assert!(has(&pdf, "/ColorSpace /DeviceCMYK"), "CMYK page stays CMYK");
    assert!(has(&pdf, "/ColorSpace /DeviceGray"));
    assert_eq!(text.matches("/BaseFont /Helvetica").count(), 1, "one shared font");
}

#[test]
fn each_page_carries_its_own_profile() {
    let mut a = doc(ColorMode::Rgb);
    a.icc_profile = Some(std::sync::Arc::new(vec![1; 8]));
    let b = doc(ColorMode::Rgb);
    let mut c = doc(ColorMode::Rgb);
    c.icc_profile = Some(std::sync::Arc::new(vec![2; 8]));
    let mut w = PdfWriter::new();
    for d in [&a, &b, &c] {
        w.add_page(&flat_page(d, OtherModes::Refuse).unwrap()).unwrap();
    }
    let pdf = w.finish();
    check_structure(&pdf);
    // a: 4-7 (with ICC 7), b: 8-10, c: 11-14 (with ICC 14).
    assert!(has(&pdf, "[/ICCBased 7 0 R]") && has(&pdf, "[/ICCBased 14 0 R]"));
    assert!(has(&pdf, "/Kids [4 0 R 8 0 R 11 0 R]"));
}

#[test]
fn other_modes_become_rgb_pages_when_asked() {
    let p = flat_page(&doc(ColorMode::Lab), OtherModes::AsRgb).unwrap();
    assert_eq!(p.image.channels, 3);
    assert!(p.image.icc.is_none(), "a Lab profile can't describe RGB samples");
    assert!(flat_page(&doc(ColorMode::Lab), OtherModes::Refuse).is_err());
}

#[test]
fn jpeg_pages_embed_the_stream_as_is() {
    let jpeg = vec![0xff, 0xd8, 0xff, 0xd9];
    let image = PrintImage { width: 2, height: 3, channels: 3, data: jpeg.clone(), icc: None, encoding: ImageEncoding::Jpeg };
    let page = PrintPage { paper: (2.0, 3.0), image, rect: (0.0, 0.0, 2.0, 3.0), marks: Marks::default(), description: None, label: None };
    let pdf = print_pdf(&page).unwrap();
    check_structure(&pdf);
    let want = b"/Filter /DCTDecode /Length 4 >>\nstream\n\xff\xd8\xff\xd9";
    assert!(pdf.windows(want.len()).any(|w| w == want), "embedded unchanged");
}

#[test]
fn bad_pages_are_refused_not_written() {
    let mut w = PdfWriter::new();
    let mut p = page_of(ColorMode::Rgb, 4, 2, 72.0);
    p.image.data.pop();
    assert!(w.add_page(&p).is_err(), "short sample buffer");
    p.image.channels = 2;
    assert!(w.add_page(&p).is_err(), "no 2-channel device space");
    p.image.width = 0;
    assert!(w.add_page(&p).is_err());
    assert_eq!(w.pages(), 0);
    let pdf = w.finish();
    check_structure(&pdf);
    assert!(has(&pdf, "/Kids [] /Count 0"));
}

#[test]
fn transparency_shows_the_chosen_background() {
    let empty = |mode| {
        let mut d = doc(mode);
        d.layers.clear();
        d
    };
    let black = flat_page_over(&empty(ColorMode::Rgb), OtherModes::Refuse, 0.0).unwrap();
    assert_eq!(black.image.data, vec![0; 4 * 2 * 3]);
    let gray = flat_page_over(&empty(ColorMode::Grayscale), OtherModes::Refuse, 0.5).unwrap();
    assert_eq!(gray.image.data, vec![128; 4 * 2]);
    let cmyk = flat_page_over(&empty(ColorMode::Cmyk), OtherModes::Refuse, 0.0).unwrap();
    assert_eq!(&cmyk.image.data[..4], &[0, 0, 0, 255], "black paper is K ink only");
    // Opaque pixels are untouched by the background; NaN falls back to white.
    assert_eq!(flat_page_over(&doc(ColorMode::Rgb), OtherModes::Refuse, 0.0).unwrap().image.data, vec![255; 4 * 2 * 3]);
    assert_eq!(flat_page_over(&empty(ColorMode::Rgb), OtherModes::Refuse, f32::NAN).unwrap().image.data, vec![255; 4 * 2 * 3]);
}

// ---------- Save As Presentation (#6637) ----------

fn deck(p: Presentation, pages: usize) -> String {
    let mut w = PdfWriter::presentation(p).unwrap();
    for _ in 0..pages {
        w.add_page(&page_of(ColorMode::Rgb, 8, 8, 72.0)).unwrap();
    }
    let pdf = w.finish();
    check_structure(&pdf);
    String::from_utf8_lossy(&pdf).into_owned()
}

#[test]
fn a_presentation_opens_full_screen_with_timing_and_transitions() {
    let text = deck(Presentation { advance: Some(2.5), loop_after_last: false, transition: Transition::WipeDown }, 3);
    assert!(text.contains("/Type /Catalog /Pages 2 0 R /PageMode /FullScreen >>"), "full screen, no script");
    assert_eq!(text.matches("/Dur 2.5").count(), 3, "every page advances");
    assert_eq!(text.matches("/Trans << /Type /Trans /S /Wipe /Di 270 /D 1 >>").count(), 3);
    assert!(!text.contains("JavaScript"));
    // Without timing or a transition, the pages carry neither.
    let plain = deck(Presentation::default(), 2);
    assert!(plain.contains("/PageMode /FullScreen"));
    assert!(!plain.contains("/Dur") && !plain.contains("/Trans"));
    // A plain multi-page document is not a presentation.
    let mut w = PdfWriter::new();
    w.add_page(&page_of(ColorMode::Rgb, 8, 8, 72.0)).unwrap();
    let doc = String::from_utf8_lossy(&w.finish()).into_owned();
    assert!(doc.contains("<< /Type /Catalog /Pages 2 0 R >>") && !doc.contains("/PageMode"));
}

#[test]
fn loop_after_last_page_is_a_document_script() {
    let text = deck(Presentation { advance: None, loop_after_last: true, transition: Transition::None }, 2);
    assert!(text.contains("/S /JavaScript /JS (app.fs.loop = true;)"));
    assert!(text.contains("/Names << /JavaScript "), "registered in the document's name tree");
}

#[test]
fn every_transition_writes_a_valid_style() {
    for t in Transition::ALL {
        assert_eq!(Transition::from_id(t.id()), Some(t));
        let text = deck(Presentation { advance: None, loop_after_last: false, transition: t }, 4);
        let styles: Vec<&str> = text.match_indices("/Type /Trans /S /").map(|(i, m)| text[i + m.len()..].split_whitespace().next().unwrap()).collect();
        if t == Transition::None {
            assert!(styles.is_empty());
            continue;
        }
        assert_eq!(styles.len(), 4, "{t:?}");
        for s in &styles {
            assert!(["Split", "Blinds", "Box", "Wipe", "Dissolve", "Glitter"].contains(s), "{t:?}: {s}");
        }
        if t == Transition::Random {
            assert!(styles.iter().any(|s| *s != styles[0]), "Random varies from page to page: {styles:?}");
        }
    }
    assert_eq!(Transition::from_id("fade"), None);
}

#[test]
fn advance_must_be_a_sensible_number_of_seconds() {
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, MAX_ADVANCE + 1.0] {
        assert!(PdfWriter::presentation(Presentation { advance: Some(bad), ..Default::default() }).is_err(), "{bad}");
    }
    assert!(PdfWriter::presentation(Presentation { advance: Some(MAX_ADVANCE), ..Default::default() }).is_ok());
}
