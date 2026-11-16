use super::super as svg;
use super::*;
use std::sync::Arc;

fn empty_fonts() -> Arc<svg::usvg::fontdb::Database> {
    Arc::new(svg::usvg::fontdb::Database::new())
}

fn parse_count<T>(run: impl FnOnce() -> T) -> (T, usize) {
    XML_PARSES.with(|count| count.set(0));
    let result = run();
    (result, XML_PARSES.with(|count| count.get()))
}

#[test]
fn unchanged_xml_is_borrowed_and_converted_with_one_checked_parse() {
    let text = "<?xml version='1.0'?><svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><a href='#shape'/><rect id='shape' width='4' height='4' fill='red'/></svg>";
    let limits = svg::tree_limits(svg::MAX_XML_DEPTH, svg::MAX_RASTER_PIXELS);
    let (result, count) = parse_count(|| {
        with_repaired_xml(text, &limits, |xml| {
            assert_eq!(xml.input_text().as_ptr(), text.as_ptr(), "unchanged XML retains the caller's source");
            assert_eq!(xml.descendants().filter(|n| n.has_tag_name((SVG_NS, "rect"))).count(), 1);
            Ok(())
        })
    });
    result.unwrap();
    assert_eq!(count, 1);
    let (parsed, count) = parse_count(|| svg::parse_with(text.as_bytes(), empty_fonts));
    let parsed = parsed.unwrap();
    assert_eq!(count, 1, "the ordinary parser must not parse the checked XML again");
    let pixels = svg::render(&parsed.tree, 1.).unwrap();
    assert_eq!(pixels.pixel(1, 1).unwrap().red(), 255);
    let (parsed, count) = parse_count(|| svg::parse_bounded_inner_with(text.as_bytes(), 16, empty_fonts));
    parsed.unwrap();
    assert_eq!(count, 1, "bounded previews use the same checked-DOM path");
}

#[test]
fn repaired_xml_is_checked_again_and_keeps_warning_and_strict_contracts() {
    let text = "<?xml version='1.0'?><svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><rect width='4' height='4' fill='red'/><image width='1' height='1' href='data:image/png;base64,A'/></svg>";
    let (parsed, count) = parse_count(|| svg::parse_with(text.as_bytes(), empty_fonts));
    let parsed = parsed.unwrap();
    assert_eq!(count, 2, "repair reparses only the replacement, after checking the original");
    assert_eq!(parsed.image_usage.invalid_raster_images, 1);
    assert_eq!(svg::render(&parsed.tree, 1.).unwrap().pixel(2, 2).unwrap().red(), 255);
    let (pixels, warnings) = svg::rasterize_bounded_with_warnings(text.as_bytes(), 1., 16).unwrap();
    assert_eq!(pixels.get(2, 2), [1., 0., 0., 1.]);
    assert_eq!(warnings.len(), 1);
    assert!(svg::rasterize_bounded(text.as_bytes(), 1., 16).is_err());
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
        let mut converted = false;
        let result = with_repaired_xml(text, &limits, |_| {
            converted = true;
            Ok(())
        });
        assert!(result.is_err(), "guard must reject original XML: {text}");
        assert!(!converted, "an unchecked DOM must never reach conversion");
    }
    let allowed = Limits { entity_bytes: 1024, ..Default::default() };
    with_repaired_xml(entity, &allowed, |xml| {
        assert_eq!(xml.descendants().find(|n| n.has_tag_name("image")).unwrap().attribute("href"), Some(INVALID_IMAGE));
        Ok(())
    })
    .unwrap();
}

// Retain the previous public-library sequence solely as a timing control. It
// uses the same resolver/validation implementations, not a copied decoder.
fn prior_parse(text: &str) -> Result<svg::usvg::Tree, IoError> {
    let limits = svg::tree_limits(svg::MAX_XML_DEPTH, svg::MAX_RASTER_PIXELS);
    let resolver = Arc::new(craft_svg::tree::ImageResolver::new(limits));
    resolver.set_document_depth(svg::nesting_depth(text.as_bytes(), svg::MAX_XML_DEPTH)?);
    let state = Arc::new(std::sync::Mutex::new(svg::image_integrity::Integrity::new(svg::MAX_RASTER_PIXELS)));
    let options = svg::options_with_isolation(empty_fonts(), svg::MAX_XML_DEPTH, false, Arc::clone(&resolver), state);
    let repaired = repair(text, &limits)?;
    resolver.check_document(&repaired).map_err(svg::svg_parse_error)?;
    craft_svg::tree::parse_text(&repaired, &limits, &options).map_err(svg::svg_parse_error)
}

#[test]
#[ignore = "release-only XML and 24 MP embedded-source timing probe"]
fn xml_reuse_and_24mp_source_timing() {
    use std::time::Instant;
    let image = photocraft_codecs::Image::from_u8(6000, 4000, photocraft_codecs::ChannelLayout::Rgb, [17, 83, 149].repeat(24_000_000)).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    drop(image);
    let mut href = String::from("data:image/png,");
    for byte in png {
        use std::fmt::Write;
        write!(&mut href, "%{byte:02X}").unwrap();
    }
    let embedded = format!("<?xml version='1.0'?><svg xmlns='{SVG_NS}' width='16' height='16'><image width='16' height='16' href='{href}'/></svg>");
    let mut xml = format!("<?xml version='1.0'?><svg xmlns='{SVG_NS}' width='16' height='16'>");
    for _ in 0..20_000 {
        xml.push_str("<rect width='1' height='1' fill='red'/>");
    }
    xml.push_str("</svg>");
    for (name, text) in [("xml-20000-nodes", xml), ("png-24mp", embedded)] {
        let mut expected = None;
        for old in [false, true, false] {
            let started = Instant::now();
            let tree = if old { prior_parse(&text).unwrap() } else { svg::parse_with(text.as_bytes(), empty_fonts).unwrap().tree };
            let elapsed = started.elapsed();
            let rendered = svg::render(&tree, 1.).unwrap();
            let checksum: u64 = rendered.data().iter().map(|b| u64::from(*b)).sum();
            if let Some(previous) = expected {
                assert_eq!(checksum, previous);
            } else {
                expected = Some(checksum);
            }
            assert_eq!((rendered.width(), rendered.height()), (16, 16));
            eprintln!("{name} {} parse_ms={:.3} output_checksum={checksum}", if old { "prior" } else { "reuse" }, elapsed.as_secs_f64() * 1000.);
        }
    }
}

#[test]
fn checked_xml_conversion_panic_is_an_isolation_failure() {
    let limits = svg::tree_limits(svg::MAX_XML_DEPTH, svg::MAX_RASTER_PIXELS);
    let result: Result<Result<(), IoError>, IoError> = svg::guarded_parse(|| with_repaired_xml("<svg/>", &limits, |_| panic!("injected XML conversion panic")));
    assert!(matches!(result, Err(IoError::Svg(message)) if message.contains("bounded SVG worker failed")));
}
