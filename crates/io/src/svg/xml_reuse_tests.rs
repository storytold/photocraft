//! App-visible XML validation, rendering and omission contracts. Detailed repair
//! source projection and parse-count tests live with the shared SVG implementation.
use super as svg;
use super::tests::base64;
use super::tiny_skia;
use crate::IoError;
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
    let opening = "<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'>";
    let image = "<image width='1' height='1' href='data:image/png;base64,A'/>";
    let mut attributes = String::new();
    for index in 0..=svg::tree_limits(svg::MAX_XML_DEPTH, 16).attributes {
        use std::fmt::Write;
        write!(attributes, " a{index}='x'").unwrap();
    }
    let rejected = format!("{opening}<g{attributes}>{image}</g></svg>");
    let Err(error) = svg::parse_with(rejected.as_bytes(), empty_fonts) else {
        panic!("original attribute limits must apply before URL repair");
    };
    assert!(error.to_string().contains("attribute"), "{error}");

    let accepted = format!(
        "<!DOCTYPE svg [<!ENTITY a 'data:image/png;base64,A'>]>{opening}<image width='1' height='1' href='&a;'/><rect width='4' height='4' fill='red'/></svg>"
    );
    let (pixels, warnings) = svg::rasterize_bounded_with_warnings(accepted.as_bytes(), 1., 16).unwrap();
    assert_eq!(pixels.get(2, 2), [1., 0., 0., 1.]);
    assert_eq!(warnings.len(), 1);
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

#[test]
fn repeated_use_reports_encoded_byte_exhaustion_at_small_pixel_counts() {
    let mut png = tiny_skia::Pixmap::new(1, 1).unwrap();
    png.fill(tiny_skia::Color::from_rgba8(0, 255, 0, 255));
    let png = png.encode_png().unwrap();
    let references = "<use href='#i'/>".repeat(64);
    let body = base64(&png);
    let source = format!(
        "<svg xmlns='{SVG_NS}' width='4' height='4'><defs><image id='i' width='1' height='1' href='data:image/png;base64,{body}'/></defs>{references}</svg>"
    );
    // Keep the test small while using the real resolver and PhotoCraft error
    // adapter. The repeated callbacks exhaust encoded bytes before raster pixels.
    let mut limits = svg::tree_limits(svg::MAX_XML_DEPTH, 1024);
    limits.bytes = source.len();
    limits.embedded_bytes = 0;
    assert!(64 * png.len() > limits.bytes);
    let resolver = Arc::new(craft_svg::tree::ValidatedImageResolver::new(limits));
    let budget = Arc::new(std::sync::Mutex::new(svg::ImageBudget {
        pixels: 1024,
        xml_bytes: svg::MAX_BOUNDED_SVG_BYTES,
        svg_images: 0,
        svg_chain: 0,
        draw_nodes: 0,
        repeatable_raster: false,
        repeatable_svg: false,
        fontless_text: false,
        strict: false,
        error: None,
        resolver: resolver.clone(),
    }));
    let error = svg::parse_image_bounded_with(source.as_bytes(), budget, empty_fonts).unwrap_err();
    assert!(error.to_string().contains("encoded byte or decoded pixel budget"), "{error}");
    let usage = resolver.usage();
    assert!(usage.raster_limit_images > 0);
    assert!(usage.raster_pixels > 0 && usage.raster_pixels < 64, "{usage:?}");
}
