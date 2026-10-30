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
fn topmost_solid_shape_is_exported_as_a_vector_path() {
    use photocraft_doc::{Fill, Path, ShapeLayer, Subpath};
    let mut d = doc(ColorMode::Rgb);
    let shape = ShapeLayer {
        path: Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (2.0, 0.0), (2.0, 1.0)])]),
        fill: Some(Fill::Solid(photocraft_color::Color::rgb(1.0, 0.0, 0.0))),
        ..Default::default()
    };
    d.layers.push(Layer::new("Shape", LayerContent::Shape(shape)));
    let out = export(&d).unwrap();
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
    d.layers.push(Layer::new("Text", LayerContent::Text(text)));
    let out = export(&d).unwrap();
    check_structure(&out.bytes);
    let content = stream_after(&out.bytes, "/Contents 4 0 R");
    let content = String::from_utf8(content).unwrap();
    assert!(content.contains("/Span << /ActualText <FEFF"));
    assert!(content.contains("0 0 1 rg"));
    assert!(content.contains("BT"));
    assert!(content.contains("3 Tr"));
    assert!(content.contains("/F1"));
    assert!(content.contains("(Vector PDF) Tj"));
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
    let out = export(&d).unwrap();
    check_structure(&out.bytes);
    let content = stream_after(&out.bytes, "/Contents 4 0 R");
    let content = String::from_utf8(content).unwrap();
    assert!(content.contains("1 0 0 rg\n"));
    assert!(content.contains("0 1 0 rg\n"));
    assert!(content.contains("/Span << /ActualText <FEFF00480069> >> BDC"));
    assert!(content.contains("(Hi) Tj"));
    assert_eq!(stream_after(&out.bytes, "/Subtype /Image"), vec![255; 4 * 2 * 3]);
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
