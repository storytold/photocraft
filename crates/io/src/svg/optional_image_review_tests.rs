use super::*;
use std::io::Write;

fn assert_incomplete_image_is_disclosed(text: &str) {
    for bytes in [text.as_bytes().to_vec(), gzip(text.as_bytes())] {
        let imported = crate::import("damaged.svg", &bytes).unwrap();
        assert_omission_warning(&imported, 1);
        assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [1., 0., 0., 1.]);
        let bounded = import_svg_bounded_images("damaged.svg", &bytes, MAX_RASTER_PIXELS).unwrap();
        assert_omission_warning(&bounded, 1);
        assert!(import_svg_preview_with_work("damaged.svg", &bytes, MAX_RASTER_PIXELS).is_err());
        assert!(import_svg_bounded_images_strict("damaged.svg", &bytes, MAX_RASTER_PIXELS).is_err());
        assert!(rasterize(&bytes, 1.).is_err());
        assert!(rasterize_bounded(&bytes, 1., MAX_RASTER_PIXELS).is_err());
        let (pixels, warnings) = rasterize_bounded_with_warnings(&bytes, 1., MAX_RASTER_PIXELS).unwrap();
        assert_eq!(pixels.get(1, 1), [1., 0., 0., 1.]);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].starts_with("SVG: 1 embedded "));
        assert!(warnings[0].len() < 256);
        assert!(!warnings[0].contains("data:"));
        // A warning-aware parse cannot make a later strict render accept an omission.
        assert!(rasterize_bounded(&bytes, 1., MAX_RASTER_PIXELS).is_err());
        assert!(rasterize_rgba8(&bytes).is_err());
    }
}

#[test]
fn valid_twelve_bit_jpeg_rejected_by_the_svg_renderer_is_disclosed() {
    // ISO/IEC 10918-1 marker syntax: one 8x8, 12-bit gray DCT block.
    // Unit quantization, zero DC difference and EOB yield a constant mid-gray.
    // This is generated data, not a camera or third-party binary fixture.
    fn segment(out: &mut Vec<u8>, marker: u8, payload: &[u8]) {
        out.extend_from_slice(&[0xff, marker]);
        out.extend_from_slice(&u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
        out.extend_from_slice(payload);
    }
    let mut jpeg = vec![0xff, 0xd8];
    let mut quantization = vec![0];
    quantization.extend_from_slice(&[1; 64]);
    segment(&mut jpeg, 0xdb, &quantization);
    segment(&mut jpeg, 0xc1, &[12, 0, 8, 0, 8, 1, 1, 0x11, 0]);
    let mut tables = Vec::new();
    for class in [0x00, 0x10] {
        tables.extend_from_slice(&[class, 1]);
        tables.extend_from_slice(&[0; 15]);
        tables.push(0); // One one-bit code: DC category zero / AC end-of-block.
    }
    segment(&mut jpeg, 0xc4, &tables);
    segment(&mut jpeg, 0xda, &[1, 1, 0, 0, 63, 0]);
    jpeg.extend_from_slice(&[0x3f, 0xff, 0xd9]); // DC=0, EOB=0, then six pad bits.
    let decoded = photocraft_codecs::decode(&jpeg).expect("valid extended sequential JPEG");
    assert_eq!(decoded.dimensions(), (8, 8));
    // Match resvg's actual JPEG backend/output policy, rather than the broader
    // shared codec which also supports valid 12-bit and arithmetic JPEG files.
    let kind = usvg::ImageKind::JPEG(Arc::new(jpeg.clone()));
    let mut pixels_left = MAX_RASTER_PIXELS;
    assert_eq!(
        draw::validate_raster(&kind, MAX_RASTER_PIXELS, &mut pixels_left),
        Err(draw::RasterValidationFailure::Invalid),
        "fixture must exercise the shared renderer capability gap"
    );
    for mime in ["image/jpeg", "text/plain"] {
        let direct = drawing(&url(mime, &jpeg));
        assert_incomplete_image_is_disclosed(&direct);
        for child in [direct.as_bytes().to_vec(), gzip(direct.as_bytes())] {
            assert_incomplete_image_is_disclosed(&drawing(&url("image/svg+xml", &child)));
        }
    }
}

#[test]
fn corrupt_png_pixels_with_valid_dimensions_are_disclosed_before_rasterization() {
    let image = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, [0, 255, 0].repeat(4)).unwrap();
    let mut png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let mut at = 8;
    loop {
        let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        if &png[at + 4..at + 8] == b"IDAT" {
            assert!(length >= 2);
            png[at + 8..at + 10].fill(0); // Invalid zlib header, with an honest chunk CRC.
            let checksum = crc32fast::hash(&png[at + 4..at + 8 + length]);
            png[at + 8 + length..at + 12 + length].copy_from_slice(&checksum.to_be_bytes());
            break;
        }
        at += length + 12;
    }
    assert_eq!(photocraft_codecs::probe_allocation_size(&png).unwrap(), (2, 2));
    assert!(tiny_skia::Pixmap::decode_png(&png).is_err());
    let direct = drawing(&url("image/png", &png));
    assert_incomplete_image_is_disclosed(&direct);
    for child in [direct.as_bytes().to_vec(), gzip(direct.as_bytes())] {
        assert_incomplete_image_is_disclosed(&drawing(&url("image/svg+xml", &child)));
    }
}

#[test]
fn malformed_data_image_hrefs_are_disclosed_before_the_parser_discards_them() {
    let direct = drawing("data:image/png;base64,A");
    assert_incomplete_image_is_disclosed(&direct);
    assert_incomplete_image_is_disclosed(&direct.replace("href=", "xmlns:xlink=\"http://www.w3.org/1999/xlink\" xlink:href="));
    assert_incomplete_image_is_disclosed(&direct.replace("base64,A", "base64,&#65;"));
    for child in [direct.as_bytes().to_vec(), gzip(direct.as_bytes())] {
        assert_incomplete_image_is_disclosed(&drawing(&url("image/svg+xml", &child)));
    }
}

fn assert_no_omission_or_unexpected_import_warning(imported: &ImportResult) {
    assert!(omission_warnings(imported).is_empty(), "{:?}", imported.warnings);
    assert!(
        imported.warnings.iter().all(|warning| warning == "SVG: embedded images are pixel layers"),
        "only the existing pixel-layer information is expected: {:?}",
        imported.warnings
    );
}

fn assert_ignored_data_image_stays_quiet(text: &str) {
    for bytes in [text.as_bytes().to_vec(), gzip(text.as_bytes())] {
        let imported = crate::import("ignored.svg", &bytes).unwrap();
        assert_no_omission_or_unexpected_import_warning(&imported);
        assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [1., 0., 0., 1.]);
        let strict = import_svg_bounded_images_strict("ignored.svg", &bytes, MAX_RASTER_PIXELS).unwrap();
        assert_no_omission_or_unexpected_import_warning(&strict);
        import_svg_preview_with_work("ignored.svg", &bytes, MAX_RASTER_PIXELS).unwrap();
        let (pixels, warnings) = rasterize_bounded_with_warnings(&bytes, 1., MAX_RASTER_PIXELS).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(pixels.get(1, 1), [1., 0., 0., 1.]);
        assert_eq!(rasterize_bounded(&bytes, 1., MAX_RASTER_PIXELS).unwrap().get(1, 1), [1., 0., 0., 1.]);
        assert_eq!(rasterize(&bytes, 1.).unwrap().get(1, 1), [1., 0., 0., 1.]);
    }
}

#[test]
fn malformed_non_image_data_hrefs_stay_ignored_in_open_and_strict_rendering() {
    for href in [
        "data:application/octet-stream;base64,A",
        "data:application/octet-stream;base64",
        "data:APPLICATION/OCTET-STREAM;BASE64,A",
        "data:text/css;base64,A",
        "data:application/octet-stream;base64#fragment,not-the-delimiter",
    ] {
        let text = drawing(href);
        assert_ignored_data_image_stays_quiet(&text);
        assert_ignored_data_image_stays_quiet(&text.replace("href=", "xmlns:x='http://www.w3.org/1999/xlink' x:href="));
        for child in [text.as_bytes().to_vec(), gzip(text.as_bytes())] {
            assert_ignored_data_image_stays_quiet(&drawing(&url("image/svg+xml", &child)));
        }
    }
}

#[test]
fn default_and_explicit_plain_data_failures_keep_image_omission_policy() {
    for href in ["data:text/plain;base64,A", "data:;base64,A", "data:;base64", "data:image/png;base64"] {
        assert_incomplete_image_is_disclosed(&drawing(href));
    }
}

#[test]
fn ignored_non_image_repair_preserves_hidden_foreign_and_valid_image_controls() {
    for content in [
        "<image display='none' width='2' height='2' href='data:application/octet-stream;base64'/>",
        "<foreign:image xmlns:foreign='urn:unrelated' width='2' height='2' href='data:application/octet-stream;base64,A'/>",
        "<image width='2' height='2' xmlns:x='http://www.w3.org/1999/xlink' x:href='data:image/png;base64,A' href='data:application/octet-stream;base64,A'/>",
    ] {
        let text = format!("<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'><rect width='2' height='2' fill='red'/>{content}</svg>");
        assert_ignored_data_image_stays_quiet(&text);
    }
    let image = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, [0, 255, 0].repeat(4)).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let text = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'><image width='2' height='2' href='{}'/><image width='2' height='2' href='data:application/octet-stream;base64,A'/></svg>",
        url("image/png", &png)
    );
    let imported = crate::import("mixed.svg", text.as_bytes()).unwrap();
    assert_eq!(imported.warnings, vec!["SVG: embedded images are pixel layers"]);
    assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [0., 1., 0., 1.]);
    let strict = import_svg_bounded_images_strict("mixed.svg", text.as_bytes(), MAX_RASTER_PIXELS).unwrap();
    assert_eq!(strict.warnings, vec!["SVG: embedded images are pixel layers"]);
    assert_eq!(rasterize_bounded(text.as_bytes(), 1., MAX_RASTER_PIXELS).unwrap().get(1, 1), [0., 1., 0., 1.]);
}

#[test]
fn malformed_nested_declarations_are_not_removed_into_valid_images() {
    let child = b"<?xml bogus?><svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'><rect width='2' height='2' fill='green'/></svg>";
    for bytes in [child.to_vec(), gzip(child)] {
        assert_incomplete_image_is_disclosed(&drawing(&url("image/svg+xml", &bytes)));
    }
}

#[test]
fn used_data_images_with_mismatched_image_mime_are_disclosed() {
    let image = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, [0, 255, 0].repeat(4)).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    assert_eq!(photocraft_codecs::decode(&png).unwrap().dimensions(), (2, 2));
    for mime in ["image/jpeg", "image/jpg"] {
        let direct = drawing(&url(mime, &png));
        assert_incomplete_image_is_disclosed(&direct);
        for child in [direct.as_bytes().to_vec(), gzip(direct.as_bytes())] {
            assert_incomplete_image_is_disclosed(&drawing(&url("image/svg+xml", &child)));
        }
    }
}

#[test]
fn complete_renderer_raster_formats_and_plain_text_sniffing_stay_usable() {
    let image = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, [0, 255, 0].repeat(4)).unwrap();
    for (format, mime) in [
        (photocraft_codecs::Format::Png, "image/png"),
        (photocraft_codecs::Format::Jpeg, "image/jpeg"),
        (photocraft_codecs::Format::Gif, "image/gif"),
        (photocraft_codecs::Format::WebP, "image/webp"),
    ] {
        let encoded = photocraft_codecs::encode(&image, format, &Default::default()).unwrap();
        for mime in [mime, "text/plain"] {
            let text = drawing(&url(mime, &encoded));
            let imported = crate::import("complete.svg", text.as_bytes()).unwrap();
            assert!(omission_warnings(&imported).is_empty(), "{format:?}: {:?}", imported.warnings);
            let pixel = rasterize_bounded(text.as_bytes(), 1., MAX_RASTER_PIXELS).unwrap().get(1, 1);
            assert!(pixel[1] > 0.8 && pixel[0] < 0.2 && pixel[2] < 0.2 && pixel[3] == 1., "{format:?}: {pixel:?}");
        }
    }
}

#[test]
fn unused_hidden_and_unselected_href_failures_do_not_acquire_warnings() {
    for content in [
        "<defs><image id='unused' width='2' height='2' href='data:image/png;base64,A'/></defs>",
        "<image display='none' width='2' height='2' href='data:image/png;base64,A'/>",
        "<image width='2' height='2' xmlns:x='http://www.w3.org/1999/xlink' x:href='data:image/png;base64,A' href='application-ignored'/>",
        "<foreign:image xmlns:foreign='urn:unrelated' width='2' height='2' href='data:image/png;base64,A'/>",
    ] {
        let text = format!("<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'><rect width='2' height='2' fill='red'/>{content}</svg>");
        for source_text in [text.clone(), drawing(&url("image/svg+xml", text.as_bytes())), drawing(&url("image/svg+xml", &gzip(text.as_bytes())))] {
            for bytes in [source_text.as_bytes().to_vec(), gzip(source_text.as_bytes())] {
                let imported = crate::import("unused.svg", &bytes).unwrap();
                assert!(omission_warnings(&imported).is_empty(), "{content}: {:?}", imported.warnings);
                assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [1., 0., 0., 1.]);
                let imported = import_svg_bounded_images("unused.svg", &bytes, MAX_RASTER_PIXELS).unwrap();
                assert!(omission_warnings(&imported).is_empty());
                let (pixels, warnings) = rasterize_bounded_with_warnings(&bytes, 1., MAX_RASTER_PIXELS).unwrap();
                assert!(warnings.is_empty(), "{content}: {warnings:?}");
                assert_eq!(pixels.get(1, 1), [1., 0., 0., 1.]);
                assert_eq!(rasterize_bounded(&bytes, 1., MAX_RASTER_PIXELS).unwrap().get(1, 1), [1., 0., 0., 1.]);
                assert_eq!(rasterize(&bytes, 1.).unwrap().get(1, 1), [1., 0., 0., 1.]);
                rasterize_rgba8(&bytes).unwrap();
                import_svg_preview_with_work("unused.svg", &bytes, MAX_RASTER_PIXELS).unwrap();
                import_svg_bounded_images_strict("unused.svg", &bytes, MAX_RASTER_PIXELS).unwrap();
            }
        }
    }
}

fn url(mime: &str, bytes: &[u8]) -> String {
    let mut value = format!("data:{mime},");
    for byte in bytes {
        value.push_str(&format!("%{byte:02X}"));
    }
    value
}

#[test]
fn svgz_content_detection_and_parser_agree_on_later_gzip_members() {
    let xml = b"<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'><rect width='2' height='2'/></svg>";
    // The first member alone looks like SVG. Detection must validate the
    // following member as well, even when there is no filename extension.
    let split = xml.len() - b"</svg>".len();
    let mut compressed = gzip(&xml[..split]);
    compressed.extend(gzip(&xml[split..]));
    assert!(crate::is_svg_input("untitled", &compressed));
    assert!(rasterize_bounded(&compressed, 1., 16).is_ok());

    let crc = compressed.len() - 8;
    compressed[crc] ^= 1;
    assert!(!crate::is_svg_input("untitled", &compressed));
    assert!(rasterize_bounded(&compressed, 1., 16).is_err());
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut writer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    writer.write_all(bytes).unwrap();
    writer.finish().unwrap()
}

fn drawing(url: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\"><rect width=\"2\" height=\"2\" fill=\"#ff0000\"/><image width=\"2\" height=\"2\" href=\"{url}\"/></svg>"
    )
}

fn omission_warnings(imported: &ImportResult) -> Vec<&str> {
    imported.warnings.iter().filter(|warning| warning.contains("embedded") && warning.contains("omitted")).map(String::as_str).collect()
}

fn assert_omission_warning(imported: &ImportResult, count: usize) {
    let warnings = omission_warnings(imported);
    assert_eq!(warnings.len(), 1, "all omitted references share one bounded warning");
    assert!(warnings[0].starts_with(&format!("SVG: {count} embedded ")), "{}", warnings[0]);
    assert!(warnings[0].len() < 256);
    assert!(!warnings[0].contains("data:"));
}

fn oversized_png(seed: u8) -> Vec<u8> {
    let image = photocraft_codecs::Image::from_u8(1, 1, photocraft_codecs::ChannelLayout::Rgb, vec![0, seed, 0]).unwrap();
    let mut png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    png[16..20].copy_from_slice(&40_000u32.to_be_bytes());
    png[20..24].copy_from_slice(&40_000u32.to_be_bytes());
    let checksum = crc32fast::hash(&png[12..29]);
    png[29..33].copy_from_slice(&checksum.to_be_bytes());
    png
}

#[test]
fn ordinary_open_preserves_outer_pixels_when_optional_images_exceed_limits_but_previews_refuse() {
    let deep = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\">{}<rect width=\"2\" height=\"2\" fill=\"#00ff00\"/>{}</svg>",
        "<g>".repeat(MAX_XML_DEPTH + 1),
        "</g>".repeat(MAX_XML_DEPTH + 1)
    );
    let image = photocraft_codecs::Image::from_u8(1, 1, photocraft_codecs::ChannelLayout::Rgb, vec![0, 255, 0]).unwrap();
    let mut png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    png[16..20].copy_from_slice(&40_000u32.to_be_bytes());
    png[20..24].copy_from_slice(&40_000u32.to_be_bytes());
    let checksum = crc32fast::hash(&png[12..29]);
    png[29..33].copy_from_slice(&checksum.to_be_bytes());
    for source in [url("image/png", &png), url("image/svg+xml", deep.as_bytes()), url("image/svg+xml", &gzip(deep.as_bytes()))] {
        let text = drawing(&source);
        let imported = crate::import("outer.svg", text.as_bytes()).expect("only the over-budget optional image is omitted");
        let pixel = photocraft_compose::flatten(&imported.document).get(1, 1);
        assert_eq!(pixel, [1., 0., 0., 1.], "the valid outer shape survives, and the rejected child is not decoded/drawn");
        assert_omission_warning(&imported, 1);
        assert!(import_svg_preview_with_work("strict.svg", text.as_bytes(), MAX_RASTER_PIXELS).is_err(), "stored previews must still draw completely");
    }
}

#[test]
fn valid_embedded_images_do_not_acquire_omission_warnings() {
    let green = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\"><rect width=\"2\" height=\"2\" fill=\"#00ff00\"/></svg>";
    let image = photocraft_codecs::Image::from_u8(1, 1, photocraft_codecs::ChannelLayout::Rgb, vec![0, 255, 0]).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    for source in [url("image/png", &png), url("image/svg+xml", green.as_bytes()), url("image/svg+xml", &gzip(green.as_bytes()))] {
        let text = drawing(&source);
        let imported = import_svg_with_group_depth("valid.svg", text.as_bytes(), MAX_GROUP_DEPTH).unwrap();
        assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [0., 1., 0., 1.]);
        assert!(omission_warnings(&imported).is_empty(), "a valid child must not acquire an omission warning");
        import_svg_preview_with_work("valid-preview.svg", text.as_bytes(), MAX_RASTER_PIXELS).unwrap();
    }
}

#[test]
fn ordinary_open_warns_once_for_embedded_svg_byte_and_chain_limits() {
    let green = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\"><rect width=\"2\" height=\"2\" fill=\"#00ff00\"/></svg>";
    let mut oversized = green.as_bytes().to_vec();
    oversized.resize(MAX_BOUNDED_SVG_BYTES as usize + 1, b' ');
    let bytes = drawing(&url("image/svg+xml", &gzip(&oversized)));
    drop(oversized);
    // Compress each level before embedding it; the chain remains tiny rather
    // than multiplying a base64 payload, and it reaches the actual chain cap.
    let mut chain = green.to_string();
    for _ in 0..=MAX_EMBEDDED_SVG_DEPTH {
        chain = drawing(&url("image/svg+xml", &gzip(chain.as_bytes())));
    }
    for text in [bytes, chain] {
        let imported = crate::import("omitted.svg", text.as_bytes()).unwrap();
        assert_omission_warning(&imported, 1); // The resource counter is a subset, not a second omission.
        assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [1., 0., 0., 1.]);
        assert!(import_svg_preview_with_work("strict.svg", text.as_bytes(), MAX_RASTER_PIXELS).is_err());
        let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(photocraft_compose::flatten(&reopened).get(1, 1), [1., 0., 0., 1.]);
    }
}

#[test]
fn omitted_image_flood_has_one_bounded_warning_even_in_whole_drawing_fallback() {
    let mut text = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\">".to_string();
    for _ in 0..=MAX_NODES {
        text.push_str("<rect width=\"2\" height=\"2\" fill=\"#ff0000\"/>");
    }
    for seed in 0..64 {
        text.push_str(&format!("<image width=\"2\" height=\"2\" href=\"{}\"/>", url("image/png", &oversized_png(seed))));
    }
    text.push_str("</svg>");
    let imported = crate::import("flood.svg", text.as_bytes()).unwrap();
    assert_omission_warning(&imported, 64);
    assert_eq!(imported.document.layers.len(), 1);
    assert!(matches!(imported.document.layers[0].content, LayerContent::Raster(_)));
    assert!(imported.warnings.iter().any(|warning| warning.contains("one pixel layer")), "exercise the whole-drawing fallback");
    assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [1., 0., 0., 1.]);
}

#[test]
fn bounded_non_strict_xml_omission_warns_while_strict_preview_still_refuses() {
    let child = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\">{}<rect width=\"2\" height=\"2\" fill=\"#00ff00\"/>{}</svg>",
        "<g>".repeat(MAX_XML_DEPTH - 2),
        "</g>".repeat(MAX_XML_DEPTH - 2)
    );
    let text = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\"><rect width=\"2\" height=\"2\" fill=\"#ff0000\"/>{}<image width=\"2\" height=\"2\" href=\"{}\"/>{}</svg>",
        "<g>".repeat(MAX_XML_DEPTH - 2),
        url("image/svg+xml", child.as_bytes()),
        "</g>".repeat(MAX_XML_DEPTH - 2)
    );
    let imported = import_svg_bounded_images("soft.svg", text.as_bytes(), MAX_RASTER_PIXELS).unwrap();
    assert_omission_warning(&imported, 1);
    assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [1., 0., 0., 1.]);
    assert!(import_svg_preview_with_work("strict.svg", text.as_bytes(), MAX_RASTER_PIXELS).is_err());
}

#[test]
fn raster_only_callers_reject_small_budget_omissions_while_import_keeps_its_warning() {
    fn no_fonts() -> Arc<usvg::fontdb::Database> {
        Arc::new(usvg::fontdb::Database::new())
    }
    let image = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, [0, 255, 0].repeat(4)).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let text = drawing(&url("image/png", &png));
    let parsed = || parse_with_image_limit(text.as_bytes(), no_fonts, 3).unwrap();
    assert_eq!(parsed().image_usage.raster_limit_images, 1, "a valid four-pixel PNG exceeds the three-pixel allowance before decode");
    assert!(matches!(complete_raster_tree(parsed()), Err(IoError::Svg(message)) if message.contains("incomplete drawing")));
    let imported = import_parsed("limited.svg", parsed(), MAX_GROUP_DEPTH, None, false).unwrap();
    assert_omission_warning(&imported, 1);
    assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [1., 0., 0., 1.]);
    let complete = parse_with_image_limit(text.as_bytes(), no_fonts, 4).unwrap();
    assert_eq!(omitted_image_count(complete.image_usage), 0);
    let pixels = rasterize_tree(&complete_raster_tree(complete).unwrap(), 1.).unwrap();
    assert_eq!(pixels.get(1, 1), [0., 1., 0., 1.]);
}

#[test]
fn raster_only_public_apis_reject_omitted_images_instead_of_returning_partial_pixels() {
    let deep = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\">{}<rect width=\"2\" height=\"2\" fill=\"#00ff00\"/>{}</svg>",
        "<g>".repeat(MAX_XML_DEPTH + 1),
        "</g>".repeat(MAX_XML_DEPTH + 1)
    );
    for source in [url("image/png", &oversized_png(0)), url("image/svg+xml", deep.as_bytes()), url("image/png", b"not a PNG")] {
        let text = drawing(&source);
        assert!(matches!(rasterize(text.as_bytes(), 1.), Err(IoError::Svg(message)) if message.contains("incomplete drawing")));
        assert!(matches!(rasterize_rgba8(text.as_bytes()), Err(IoError::Svg(message)) if message.contains("incomplete drawing")));
        assert!(rasterize_bounded(text.as_bytes(), 1., MAX_RASTER_PIXELS).is_err(), "bounded rasterization must also refuse omitted images");
        let imported = crate::import("outer.svg", text.as_bytes()).unwrap();
        assert_omission_warning(&imported, 1);
        assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [1., 0., 0., 1.]);
    }
}

#[test]
fn raster_only_public_apis_preserve_complete_embedded_image_pixels() {
    let image = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, [0, 255, 0].repeat(4)).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let green = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\"><rect width=\"2\" height=\"2\" fill=\"#00ff00\"/></svg>";
    for source in [url("image/png", &png), url("text/plain", &png), url("image/svg+xml", green.as_bytes())] {
        let text = drawing(&source);
        assert_eq!(rasterize(text.as_bytes(), 1.).unwrap().get(1, 1), [0., 1., 0., 1.]);
        assert_eq!(rasterize_bounded(text.as_bytes(), 1., MAX_RASTER_PIXELS).unwrap().get(1, 1), [0., 1., 0., 1.]);
        let (width, height, pixels) = rasterize_rgba8(text.as_bytes()).unwrap();
        assert_eq!((width, height), (2, 2));
        assert_eq!(pixels, [0, 255, 0, 255].repeat(4));
    }
}

#[test]
fn malformed_data_urls_and_broken_pixel_payloads_are_disclosed_before_rendering() {
    let image = photocraft_codecs::Image::from_u8(1, 1, photocraft_codecs::ChannelLayout::Rgb, vec![0, 255, 0]).unwrap();
    let mut png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    // Keep a valid signature and IHDR, but remove IDAT/IEND: dimensions alone are not a decode.
    png.truncate(33);
    for source in ["data:image/png;base64,A".to_string(), "data:image/png;base64".to_string(), url("image/png", &png)] {
        let text = drawing(&source);
        let imported = crate::import("omitted.svg", text.as_bytes()).unwrap();
        assert_omission_warning(&imported, 1);
        assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [1., 0., 0., 1.]);
        let (pixels, warnings) = rasterize_bounded_with_warnings(text.as_bytes(), 1., MAX_RASTER_PIXELS).unwrap();
        assert_eq!(pixels.get(1, 1), [1., 0., 0., 1.]);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].starts_with("SVG: 1 embedded "));
        assert!(rasterize(text.as_bytes(), 1.).is_err());
        assert!(rasterize_rgba8(text.as_bytes()).is_err());
        assert!(rasterize_bounded(text.as_bytes(), 1., MAX_RASTER_PIXELS).is_err());
        assert!(import_svg_preview_with_work("strict.svg", text.as_bytes(), MAX_RASTER_PIXELS).is_err());
    }
}

#[test]
fn excessive_mime_parameters_preserve_valid_images_and_disclose_only_real_failures() {
    fn assert_complete_green_image(text: &str) {
        for bytes in [text.as_bytes().to_vec(), gzip(text.as_bytes())] {
            let imported = crate::import("valid.svg", &bytes).unwrap();
            assert_no_omission_or_unexpected_import_warning(&imported);
            assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [0., 1., 0., 1.]);
            let strict = import_svg_bounded_images_strict("valid.svg", &bytes, MAX_RASTER_PIXELS).unwrap();
            assert_no_omission_or_unexpected_import_warning(&strict);
            assert_eq!(photocraft_compose::flatten(&strict.document).get(1, 1), [0., 1., 0., 1.]);
            import_svg_preview_with_work("valid.svg", &bytes, MAX_RASTER_PIXELS).unwrap();
            let (pixels, warnings) = rasterize_bounded_with_warnings(&bytes, 1., MAX_RASTER_PIXELS).unwrap();
            assert!(warnings.is_empty());
            assert_eq!(pixels.get(1, 1), [0., 1., 0., 1.]);
            assert_eq!(rasterize_bounded(&bytes, 1., MAX_RASTER_PIXELS).unwrap().get(1, 1), [0., 1., 0., 1.]);
            assert_eq!(rasterize(&bytes, 1.).unwrap().get(1, 1), [0., 1., 0., 1.]);
            let (width, height, pixels) = rasterize_rgba8(&bytes).unwrap();
            assert_eq!((width, height), (2, 2));
            assert_eq!(pixels, [0, 255, 0, 255].repeat(4));
        }
    }
    let parameters = (0..65).map(|index| format!(";p{index}=x")).collect::<String>();
    let image = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, [0, 255, 0].repeat(4)).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    for essence in ["image/png", "text/plain", ""] {
        let href = url(&format!("{essence}{parameters}"), &png);
        let text = drawing(&href);
        assert_complete_green_image(&text);
        for child in [text.as_bytes().to_vec(), gzip(text.as_bytes())] {
            assert_complete_green_image(&drawing(&url("image/svg+xml", &child)));
        }
        // Header compaction cannot turn a malformed base64 body or a missing
        // delimiter into a valid image, including default text/plain MIME.
        for href in [format!("data:{essence}{parameters};base64,A"), format!("data:{essence}{parameters};base64")] {
            assert_incomplete_image_is_disclosed(&drawing(&href));
        }
    }
    assert_incomplete_image_is_disclosed(&drawing(&url(&format!("image/unsupported{parameters}"), &png)));
    let href = format!("data:application/octet-stream{parameters};base64,AA==");
    assert_ignored_data_image_stays_quiet(&drawing(&href));
    let image = url(&format!("image/png{parameters}"), &png);
    for content in [
        format!("<defs><image id='unused' href='{image}'/></defs>"),
        format!("<image display='none' width='2' height='2' href='{image}'/>"),
        format!("<foreign:image xmlns:foreign='urn:unrelated' width='2' height='2' href='{image}'/>"),
        format!("<image width='2' height='2' xmlns:x='http://www.w3.org/1999/xlink' x:href='{image}' href='{href}'/>"),
    ] {
        let text = format!("<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'><rect width='2' height='2' fill='red'/>{content}</svg>");
        assert_ignored_data_image_stays_quiet(&text);
    }
}

#[test]
fn valid_image_at_the_mime_parameter_boundary_keeps_its_pixels() {
    let image = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, [0, 255, 0].repeat(4)).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    // This helper percent-encodes the payload; 64 parameters hit the separator boundary.
    let parameters = (0..64).map(|index| format!(";p{index}=x")).collect::<String>();
    let text = drawing(&url(&format!("image/png{parameters}"), &png));
    for bytes in [text.as_bytes().to_vec(), gzip(text.as_bytes())] {
        let imported = crate::import("valid.svg", &bytes).unwrap();
        assert_eq!(imported.warnings, vec!["SVG: embedded images are pixel layers"]);
        assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [0., 1., 0., 1.]);
        assert_eq!(rasterize(&bytes, 1.).unwrap().get(1, 1), [0., 1., 0., 1.]);
    }
}
