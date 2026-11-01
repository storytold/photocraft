//! PDF export: structure, pixel oracle, colour modes and refusal paths.

use super::*;
use photocraft_color::{PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Layer, LayerContent, Size};
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
    let out = export(&d, true).unwrap();
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
    let out = export(&d, true).unwrap();
    let img = stream_after(&out.bytes, "/Subtype /Image");
    // Pixels 1-2 of row 0: 0.5 red over white = (1.0, 0.5, 0.5); elsewhere plain white.
    assert_eq!(&img[3..6], &[255, 128, 128], "the blend lands: {:?}", &img[3..6]);
    assert_eq!(&img[0..3], &[255, 255, 255]);
}

#[test]
fn greyscale_and_cmyk_pages_keep_their_channels() {
    let g = export(&doc(ColorMode::Grayscale), true).unwrap();
    assert!(has(&g.bytes, "/ColorSpace /DeviceGray"), "gray is DeviceGray");
    assert_eq!(stream_after(&g.bytes, "/Subtype /Image"), vec![255; 4 * 2], "one sample per pixel");
    let c = export(&doc(ColorMode::Cmyk), true).unwrap();
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
    let out = export(&d, true).unwrap();
    assert!(has(&out.bytes, "/MediaBox [0 0 72.00 36.00]"), "300 px at 300 dpi is one inch");
}

#[test]
fn an_embedded_profile_travels_as_iccbased() {
    let mut d = doc(ColorMode::Rgb);
    d.icc_profile = Some(std::sync::Arc::new(vec![1, 2, 3, 4, 5, 6, 7, 8]));
    let out = export(&d, true).unwrap();
    assert!(has(&out.bytes, "[/ICCBased 7 0 R]"), "the image refers to the profile");
    assert_eq!(stream_after(&out.bytes, "/N 3 /Alternate"), vec![1, 2, 3, 4, 5, 6, 7, 8], "the profile bytes survive");
}

#[test]
fn other_modes_are_refused_with_an_actionable_error() {
    let d = doc(ColorMode::Lab);
    let e = export(&d, true).unwrap_err().to_string();
    assert!(e.contains("RGB") && e.contains("Mode"), "{e}");
    assert!(crate::export(&d, "x.pdf", &crate::ExportOptions::default()).is_err());
}

// ---- Spot plates (FILE-215-17) ----

/// A grayscale channel surface over the 4×2 test canvas, one value per pixel.
fn channel(vals: &[f32]) -> Surface {
    let mut s = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    s.write_region(Rect::new(0, 0, 4, 2), vals);
    s
}

fn spot(name: &str, color: photocraft_color::Color, vals: &[f32]) -> AlphaChannel {
    AlphaChannel { spot: Some((color, 0.0)), ..AlphaChannel::new(name, channel(vals)) }
}

/// The content stream (object 4), inflated, as text.
fn content(pdf: &[u8]) -> String {
    String::from_utf8(stream_after(pdf, "4 0 obj")).unwrap()
}

#[test]
fn spot_channels_follow_the_image_as_overprinting_separation_plates() {
    let mut d = doc(ColorMode::Rgb);
    let under: Vec<f32> = vec![1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0];
    let red: Vec<f32> = vec![0.0, 0.5, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    d.channels.push(spot("Underbase White", photocraft_color::Color::WHITE, &under));
    d.channels.push(spot("PANTONE 185 C", photocraft_color::Color::rgb(0.89, 0.09, 0.22), &red));
    let out = export(&d, true).unwrap();
    let pdf = &out.bytes;
    check_structure(pdf);
    // The composite is still the first image, in its own colour space.
    assert!(has(pdf, "/Width 4 /Height 2 /ColorSpace /DeviceRGB"), "the flattened composite stays");
    assert_eq!(stream_after(pdf, "/Subtype /Image"), vec![255u8; 4 * 2 * 3]);
    // One Separation XObject per channel, named after it, with a DeviceCMYK tint transform.
    assert!(
        has(
            pdf,
            "/ColorSpace [/Separation /Underbase#20White /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0.0000 0.0000 0.0000 0.0000] /N 1 >>]"
        ),
        "white ink previews as no ink"
    );
    assert!(has(pdf, "/Separation /PANTONE#20185#20C /DeviceCMYK"), "the second plate");
    // 1 = solid ink, as the channel stores it: the plate is the channel, 8-bit, not inverted.
    assert_eq!(stream_after(pdf, "/Separation /Underbase#20White"), vec![255, 255, 255, 255, 0, 0, 0, 0]);
    assert_eq!(stream_after(pdf, "/Separation /PANTONE#20185#20C"), vec![0, 128, 255, 0, 0, 0, 0, 0]);
    // Overprint is on for the plates, so each paints only its own colorant; the plates carry no
    // mask of any kind (plain viewers show the last plate opaque; a RIP separates them).
    assert!(has(pdf, "/Type /ExtGState /OP true /op true /OPM 1"), "overprint state");
    assert!(!has(pdf, "/Mask") && !has(pdf, "/ImageMask"), "no stencil or colour-key mask");
    assert!(!has(pdf, "/SMask"), "no transparency");
    assert!(!has(pdf, "/Width 4 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Mask"), "the composite is not masked");
    let c = content(pdf);
    let (im, p0, p1) = (
        c.find("/Im0 Do").unwrap(),
        c.find("/GS0 gs 4.000 0 0 2.000 0.000 0.000 cm /Sp0 Do").unwrap(),
        c.find("/GS0 gs 4.000 0 0 2.000 0.000 0.000 cm /Sp1 Do").unwrap(),
    );
    assert!(im < p0 && p0 < p1, "image, then the plates in channel order (underbase first): {c}");
    assert!(out.warnings.iter().any(|w| w.contains("2 spot plates") && w.contains("Underbase White, PANTONE 185 C")), "{:?}", out.warnings);
    // An alpha channel of an RGB document is a mask, not a plate.
    d.channels.push(AlphaChannel::new("Mask", channel(&[1.0; 8])));
    let out = export(&d, true).unwrap();
    assert!(!has(&out.bytes, "/Separation /Mask"), "alpha channels stay out of RGB plates");
    assert!(has(&out.bytes, "/Sp1 "), "the two spot plates remain");
    assert!(!has(&out.bytes, "/Sp2 "));
}

#[test]
fn a_multichannel_document_is_its_plates_alone() {
    let mut d = Document::new("m.pdf", Size { width: 4, height: 2 }, ColorMode::Multichannel, SampleType::F32);
    d.resolution_dpi = 72.0;
    d.channels.push(spot("Cyan", photocraft_color::Color::rgb(0.0, 1.0, 1.0), &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]));
    d.channels.push(spot("Black Ink", photocraft_color::Color::BLACK, &[0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]));
    // An alpha-type channel of a Multichannel document prints as black ink where it is
    // unselected (1 − value), as the composite shows it.
    d.channels.push(AlphaChannel::new("Alpha 1", channel(&[1.0, 1.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0])));
    let out = export(&d, true).unwrap();
    let pdf = &out.bytes;
    check_structure(pdf);
    assert!(!has(pdf, "/Im0"), "no composite image");
    assert!(has(pdf, "/MediaBox [0 0 4.00 2.00]"));
    assert_eq!(stream_after(pdf, "/Separation /Cyan"), vec![255, 0, 0, 0, 0, 0, 0, 0]);
    // The alternate is the ink separated through the CMYK profile in use: a rich black here,
    // whatever its CMY, with the black ink near solid.
    let marker = "/Separation /Black#20Ink /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [";
    let at = pdf.windows(marker.len()).position(|w| w == marker.as_bytes()).expect("the black plate's tint transform") + marker.len();
    let c1 = std::str::from_utf8(&pdf[at..at + 32]).unwrap().split(']').next().unwrap().to_string();
    let k: f32 = c1.split_whitespace().nth(3).unwrap().parse().unwrap();
    assert!(k > 0.85, "black ink's K alternate: {c1}");
    assert_eq!(stream_after(pdf, "/Separation /Alpha#201"), vec![0, 0, 255, 128, 0, 0, 0, 0], "alpha-type: ink = 1 − value");
    let c = content(pdf);
    assert!(c.find("/Sp0 Do").unwrap() < c.find("/Sp1 Do").unwrap() && c.find("/Sp1 Do").unwrap() < c.find("/Sp2 Do").unwrap());
    assert!(!out.warnings.iter().any(|w| w.contains("flattened")), "nothing was flattened: {:?}", out.warnings);
    // Without plates there is nothing to write.
    let e = export(&d, false).unwrap_err().to_string();
    assert!(e.contains("Multichannel") && e.contains("spot plates"), "{e}");
    d.channels.clear();
    assert!(export(&d, true).is_err(), "no channels, nothing to write");
}

#[test]
fn plate_names_are_escaped_as_pdf_names() {
    let mut d = doc(ColorMode::Grayscale);
    let ink = photocraft_color::Color::rgb(0.2, 0.3, 0.4);
    d.channels.push(spot("Black (Ink) #1/ü", ink, &[0.0; 8]));
    d.channels.push(spot("", ink, &[0.0; 8]));
    let out = export(&d, true).unwrap();
    check_structure(&out.bytes);
    assert!(has(&out.bytes, "/Separation /Black#20#28Ink#29#20#231#2F#C3#BC /DeviceCMYK"), "delimiters, #, space and UTF-8 bytes are #xx");
    assert!(has(&out.bytes, "/Separation /Spot /DeviceCMYK"), "an empty name is not the bare /");
}

#[test]
fn spot_plates_can_be_left_out_of_a_scripted_save() {
    let mut d = doc(ColorMode::Cmyk);
    d.channels.push(spot("Varnish", photocraft_color::Color::rgb(0.9, 0.9, 0.6), &[1.0; 8]));
    let with = crate::export(&d, "out.pdf", &crate::ExportOptions::default()).unwrap();
    assert!(has(&with.bytes, "/Separation /Varnish"), "plates by default");
    let without = crate::export(&d, "out.pdf", &crate::ExportOptions { pdf_spot_plates: false, ..Default::default() }).unwrap();
    assert!(!has(&without.bytes, "/Separation") && !has(&without.bytes, "/ExtGState"), "no plate, no overprint state, no mask");
    assert!(has(&without.bytes, "/ColorSpace /DeviceCMYK"), "the composite still saves");
    assert!(without.warnings.iter().any(|w| w.contains("not written")), "{:?}", without.warnings);
    check_structure(&without.bytes);
    // A plain document never gets the plate warning either way.
    let plain = export(&doc(ColorMode::Rgb), true).unwrap();
    assert!(!plain.warnings.iter().any(|w| w.contains("plate")), "{:?}", plain.warnings);
}

#[test]
fn a_16_bit_plate_is_written_at_8_bits() {
    let mut d = Document::new("m.pdf", Size { width: 4, height: 2 }, ColorMode::Multichannel, SampleType::U16);
    let mut s = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false));
    s.write_region(Rect::new(0, 0, 4, 2), &[0.0, 0.25, 0.5, 1.0, 0.0, 0.0, 0.0, 0.0]);
    d.channels.push(AlphaChannel { spot: Some((photocraft_color::Color::BLACK, 0.0)), ..AlphaChannel::new("K", s) });
    let out = export(&d, true).unwrap();
    assert!(has(&out.bytes, "/BitsPerComponent 8"));
    assert_eq!(stream_after(&out.bytes, "/Separation /K"), vec![0, 64, 128, 255, 0, 0, 0, 0]);
}

// ---- Spot-plate XMP description ----

/// The XMP packet (the catalog's /Metadata stream), as text.
fn xmp_of(pdf: &[u8]) -> String {
    let start = pdf.windows(9).position(|w| w == b"<?xpacket").expect("a packet");
    let tail = b"<?xpacket end=\"w\"?>";
    let end = pdf[start..].windows(tail.len()).position(|w| w == tail).expect("packet end") + start + tail.len();
    String::from_utf8(pdf[start..end].to_vec()).unwrap()
}

#[test]
fn the_plates_are_described_in_xmp_with_their_ink_colour_and_solidity() {
    let mut d = Document::new("m.pdf", Size { width: 4, height: 2 }, ColorMode::Multichannel, SampleType::F32);
    let ink = photocraft_color::Color::rgb(0.2, 0.3, 0.4);
    d.channels.push(AlphaChannel { spot: Some((ink, 0.35)), ..AlphaChannel::new("Black (Ink) #1/ü & <x>", channel(&[1.0; 8])) });
    d.channels.push(spot("Underbase White", photocraft_color::Color::WHITE, &[0.0; 8]));
    let out = export(&d, true).unwrap();
    let pdf = &out.bytes;
    check_structure(pdf);
    assert!(has(pdf, "/Type /Catalog /Pages 2 0 R /Metadata "), "the catalog points at the metadata stream");
    assert!(has(pdf, "/Type /Metadata /Subtype /XML /Length "), "an unfiltered XMP stream");
    let xmp = xmp_of(pdf);
    assert!(xmp.contains("xmlns:pcsp=\"http://ns.getartcraft.com/photocraft/spot-plates/1.0/\""), "{xmp}");
    let lab = photocraft_color::convert::srgb_to_lab([0.2, 0.3, 0.4]);
    let want = format!(
        "<rdf:li rdf:parseType=\"Resource\">\n      <pcsp:index>0</pcsp:index>\n      <pcsp:name>Black (Ink) #1/ü &amp; &lt;x&gt;</pcsp:name>\n      <pcsp:labL>{:.2}</pcsp:labL>\n      <pcsp:labA>{:.2}</pcsp:labA>\n      <pcsp:labB>{:.2}</pcsp:labB>\n      <pcsp:srgb>#334D66</pcsp:srgb>\n      <pcsp:solidity>35</pcsp:solidity>\n     </rdf:li>",
        lab[0], lab[1], lab[2]
    );
    assert!(xmp.contains(&want), "first plate:\n{xmp}");
    assert!(xmp.contains("<pcsp:index>1</pcsp:index>\n      <pcsp:name>Underbase White</pcsp:name>\n      <pcsp:labL>100.00</pcsp:labL>\n      <pcsp:labA>0.00</pcsp:labA>\n      <pcsp:labB>0.00</pcsp:labB>\n      <pcsp:srgb>#FFFFFF</pcsp:srgb>\n      <pcsp:solidity>0</pcsp:solidity>"), "second plate:\n{xmp}");
    // The order is the plates' paint order, which is the Separation names' order in the page.
    let pos = |needle: &str| pdf.windows(needle.len()).position(|w| w == needle.as_bytes()).expect(needle);
    assert!(pos("/Separation /Black#20#28Ink") < pos("/Separation /Underbase#20White"));
    // One packet, well-formed enough for an XML parser.
    assert_eq!(xmp.matches("<?xpacket begin").count(), 1);
    assert!(crate::xml::parse(&xmp[xmp.find("<x:xmpmeta").unwrap()..xmp.rfind("</x:xmpmeta>").unwrap() + 12], Default::default()).is_ok(), "{xmp}");
}

#[test]
fn the_plate_description_joins_the_documents_own_packet() {
    let mut d = doc(ColorMode::Rgb);
    d.channels.push(spot("Varnish", photocraft_color::Color::rgb(0.9, 0.9, 0.6), &[1.0; 8]));
    d.metadata.xmp = Some(std::sync::Arc::from(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n  <rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>Job 1</dc:title></rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>",
    ));
    let out = crate::export(&d, "out.pdf", &crate::ExportOptions::default()).unwrap();
    let xmp = xmp_of(&out.bytes);
    assert_eq!(xmp.matches("<?xpacket begin").count(), 1, "one packet, not two");
    assert!(xmp.contains("<dc:title>Job 1</dc:title>") && xmp.contains("<pcsp:name>Varnish</pcsp:name>"), "{xmp}");
    assert!(xmp.find("<dc:title>").unwrap() < xmp.find("<pcsp:plates>").unwrap() && xmp.find("<pcsp:plates>").unwrap() < xmp.rfind("</rdf:RDF>").unwrap());
    // A save that embeds no metadata drops the document's packet but still describes the plates.
    let none = crate::export(&d, "out.pdf", &crate::ExportOptions { xmp: crate::XmpEmbed::None, ..Default::default() }).unwrap();
    let xmp = xmp_of(&none.bytes);
    assert!(!xmp.contains("dc:title") && xmp.contains("<pcsp:name>Varnish</pcsp:name>"), "{xmp}");
    // Without plates the document's packet travels as it is, and a document without one has no stream.
    d.channels.clear();
    let plain = crate::export(&d, "out.pdf", &crate::ExportOptions::default()).unwrap();
    assert!(has(&plain.bytes, "<dc:title>Job 1</dc:title>") && !has(&plain.bytes, "pcsp:"), "the document packet alone");
    d.metadata.xmp = None;
    let bare = export(&d, true).unwrap();
    assert!(!has(&bare.bytes, "/Metadata") && !has(&bare.bytes, "<?xpacket"), "no metadata stream");
    check_structure(&bare.bytes);
}

#[test]
fn the_plate_description_joins_the_rdf_element_however_the_packet_spells_it() {
    const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
    let mut d = doc(ColorMode::Rgb);
    d.channels.push(spot("Varnish", photocraft_color::Color::rgb(0.9, 0.9, 0.6), &[1.0; 8]));
    // The packet must parse as namespaced XML with the plates a description inside its RDF element.
    let plates_in_rdf = |xmp: &str| {
        let tree = crate::xml::parse(xmp.trim_start_matches('\u{feff}'), roxmltree::ParsingOptions::default()).unwrap_or_else(|e| panic!("{e}: {xmp}"));
        let plates = tree
            .descendants()
            .find(|n| n.tag_name().namespace() == Some(crate::pdf::SPOT_PLATES_NS) && n.tag_name().name() == "plates")
            .unwrap_or_else(|| panic!("no pcsp:plates: {xmp}"));
        let description = plates.parent_element().unwrap();
        assert_eq!((description.tag_name().namespace(), description.tag_name().name()), (Some(RDF), "Description"), "{xmp}");
        let rdf = description.parent_element().unwrap();
        assert_eq!((rdf.tag_name().namespace(), rdf.tag_name().name()), (Some(RDF), "RDF"), "{xmp}");
        assert_eq!(xmp.matches("<?xpacket begin").count(), 1, "one packet, not two: {xmp}");
    };
    let head = "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n";
    let tail = "\n</x:xmpmeta>\n<?xpacket end=\"w\"?>";
    let title = "<r:Description r:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>Job 2</dc:title></r:Description>";
    for (packet, what) in [
        // Another prefix bound to RDF, whitespace before the `>` of the end tag, a leading byte order mark.
        (format!("\u{feff}{head} <r:RDF xmlns:r=\"{RDF}\">\n  {title}\n </r:RDF >{tail}"), "prefix, whitespace and BOM"),
        // An end tag spelled out inside a comment is not the element's end.
        (format!("{head} <rdf:RDF xmlns:rdf=\"{RDF}\">\n  <!-- </rdf:RDF> -->\n  {}\n </rdf:RDF>{tail}", title.replace("r:", "rdf:")), "comment"),
        // A self-closing RDF element is expanded around the description.
        (format!("{head} <r:RDF xmlns:r=\"{RDF}\"/>{tail}"), "self-closing"),
    ] {
        d.metadata.xmp = Some(std::sync::Arc::from(packet.as_str()));
        let out = crate::export(&d, "out.pdf", &crate::ExportOptions::default()).unwrap();
        let xmp = xmp_of(&out.bytes);
        plates_in_rdf(&xmp);
        assert_eq!(xmp.contains("<dc:title>Job 2</dc:title>"), what != "self-closing", "{what}: {xmp}");
        assert!(xmp.contains("<pcsp:name>Varnish</pcsp:name>"), "{what}: {xmp}");
    }
    // A packet with no RDF element, or that is not XML, is kept as it is: the plates go undescribed
    // rather than the document's metadata lost.
    for kept in [format!("{head}<x:note>kept</x:note>{tail}"), format!("{head}<rdf:RDF xmlns:rdf=\"{RDF}\"><unclosed>{tail}")] {
        d.metadata.xmp = Some(std::sync::Arc::from(kept.as_str()));
        let out = crate::export(&d, "out.pdf", &crate::ExportOptions::default()).unwrap();
        assert_eq!(xmp_of(&out.bytes), kept);
    }
}
