//! App-visible XML validation, rendering and omission contracts. Detailed repair
//! source projection and parse-count tests live with the shared SVG implementation.
use super as svg;
use super::tiny_skia;
use crate::IoError;
use craft_svg::tree::Limits;
use std::sync::Arc;

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";

fn empty_fonts() -> Arc<svg::usvg::fontdb::Database> {
    Arc::new(svg::usvg::fontdb::Database::new())
}

#[test]
fn unchanged_xml_keeps_ordinary_and_bounded_rendering() {
    let text = "<?xml version='1.0'?><svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><a href='#shape'/><rect id='shape' width='4' height='4' fill='red'/></svg>";
    let parsed = svg::parse_with(text.as_bytes(), empty_fonts).unwrap();
    assert_eq!(svg::omitted_image_count(parsed.image_usage), 0);
    let pixels = svg::render(&parsed.tree, 1.).unwrap();
    assert_eq!(pixels.pixel(1, 1).unwrap().red(), 255);
    svg::parse_bounded_inner_with(text.as_bytes(), 16, empty_fonts).unwrap();
}

#[test]
fn repaired_xml_keeps_warning_and_strict_contracts() {
    for text in [
        "<?xml version='1.0'?><svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><rect width='4' height='4' fill='red'/><image width='1' height='1' href='data:image/png;base64,A'/></svg>",
        "<!DOCTYPE svg [<!ENTITY a 'data:image/png;base64,A'>]><svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><rect width='4' height='4' fill='red'/><image width='1' height='1' href='&a;'/></svg>",
    ] {
        let parsed = svg::parse_with(text.as_bytes(), empty_fonts).unwrap();
        assert_eq!(parsed.image_usage.invalid_raster_images, 1);
        assert_eq!(svg::render(&parsed.tree, 1.).unwrap().pixel(2, 2).unwrap().red(), 255);
        let (pixels, warnings) = svg::rasterize_bounded_with_warnings(text.as_bytes(), 1., 16).unwrap();
        assert_eq!(pixels.get(2, 2), [1., 0., 0., 1.]);
        assert_eq!(warnings.len(), 1);
        assert!(svg::rasterize_bounded(text.as_bytes(), 1., 16).is_err());
    }
}

#[test]
fn original_xml_guards_run_before_repair_or_conversion() {
    let normal = "<svg xmlns='http://www.w3.org/2000/svg'><image href='data:image/png;base64,A'/></svg>";
    let entity = "<!DOCTYPE svg [<!ENTITY a 'data:image/png;base64,A'>]><svg><image href='&a;'/></svg>";
    let markup = "<!DOCTYPE svg [<!ENTITY a '<g/>'>]><svg>&a;</svg>";
    let referenced = "<svg><defs><g id='a'><g id='b'><rect/></g></g></defs><use href='#a'/></svg>";
    for (text, limits) in [
        (normal, Limits { bytes: normal.len() - 1, ..Default::default() }),
        (normal, Limits { attributes: 0, ..Default::default() }),
        (normal, Limits { xml_nodes: 1, ..Default::default() }),
        (entity, Limits { entity_bytes: 1, ..Default::default() }),
        (markup, Limits { refuse_entity_markup: true, ..Default::default() }),
        (referenced, Limits { tree_depth: 2, ..Default::default() }),
    ] {
        assert!(craft_svg::tree::preflight(text, &limits).is_err(), "guard must reject original XML before conversion: {text}");
    }
    let allowed = Limits { entity_bytes: 1024, ..Default::default() };
    assert!(craft_svg::tree::preflight(entity, &allowed).is_ok(), "bounded entity expansion remains admitted");
}

#[test]
fn checked_xml_conversion_panic_is_an_isolation_failure() {
    let result: Result<(), IoError> = svg::guarded_parse(|| panic!("injected XML conversion panic"));
    assert!(matches!(result, Err(IoError::Svg(message)) if message.contains("bounded SVG worker failed")));
}

#[test]
fn long_attribute_spacing_preserves_artwork_and_discloses_only_used_images() {
    for spacing in [" ".repeat(300), " \t\n\r".repeat(300)] {
        for attribute in ["href", "x:href"] {
            let text = format!(
                "<svg xmlns='{SVG_NS}' xmlns:x='{XLINK_NS}' width='4' height='4'><rect width='4' height='4' fill='red'/><image width='1' height='1' {attribute}{spacing}={spacing}'data:image/png;base64,A'/></svg>"
            );
            let (pixels, warnings) = svg::rasterize_bounded_with_warnings(text.as_bytes(), 1., 16).unwrap();
            assert_eq!(pixels.get(2, 2), [1., 0., 0., 1.]);
            assert_eq!(warnings.len(), 1);
            assert!(svg::rasterize_bounded(text.as_bytes(), 1., 16).is_err());
            let hidden = text.replace("<image ", "<image display='none' ");
            let (_, warnings) = svg::rasterize_bounded_with_warnings(hidden.as_bytes(), 1., 16).unwrap();
            assert!(warnings.is_empty());
        }
    }
}

#[test]
fn many_mime_parameters_preserve_valid_pixels_and_invalid_image_omissions() {
    let mut image = tiny_skia::Pixmap::new(1, 1).unwrap();
    image.fill(tiny_skia::Color::from_rgba8(0, 255, 0, 255));
    let body = base64(&image.encode_png().unwrap());
    let parameters = ";p=x".repeat(100);
    let drawing = |body: &str| {
        format!(
            "<svg xmlns='{SVG_NS}' width='4' height='4'><rect width='4' height='4' fill='red'/><image width='1' height='1' href='data:image/png{parameters};base64,{body}'/></svg>"
        )
    };
    let text = drawing(&body);
    let (pixels, warnings) = svg::rasterize_bounded_with_warnings(text.as_bytes(), 1., 16).unwrap();
    assert!(warnings.is_empty());
    assert_eq!(pixels.get(0, 0), [0., 1., 0., 1.]);
    assert_eq!(pixels.get(2, 2), [1., 0., 0., 1.]);
    assert!(svg::rasterize_bounded(text.as_bytes(), 1., 16).is_ok());
    let text = drawing("A");
    let (pixels, warnings) = svg::rasterize_bounded_with_warnings(text.as_bytes(), 1., 16).unwrap();
    assert_eq!(pixels.get(2, 2), [1., 0., 0., 1.]);
    assert_eq!(warnings.len(), 1);
    assert!(svg::rasterize_bounded(text.as_bytes(), 1., 16).is_err());
}

/// Test-only RFC4648 encoding, keeping the real base64 image path covered without
/// adding a dependency for fixture construction.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        result.push(char::from(ALPHABET[usize::from(a >> 2)]));
        result.push(char::from(ALPHABET[usize::from((a & 3) << 4 | b >> 4)]));
        result.push(if chunk.len() > 1 { char::from(ALPHABET[usize::from((b & 15) << 2 | c >> 6)]) } else { '=' });
        result.push(if chunk.len() > 2 { char::from(ALPHABET[usize::from(c & 63)]) } else { '=' });
    }
    result
}
