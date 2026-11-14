use super::*;
use std::io::Write;

fn url(mime: &str, bytes: &[u8]) -> String {
    let mut value = format!("data:{mime},");
    for byte in bytes {
        value.push_str(&format!("%{byte:02X}"));
    }
    value
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

#[test]
fn ordinary_open_preserves_outer_pixels_when_optional_images_exceed_limits_but_previews_refuse() {
    let green = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\"><rect width=\"2\" height=\"2\" fill=\"#00ff00\"/></svg>";
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
        let imported = import_svg_with_group_depth("outer.svg", text.as_bytes(), MAX_GROUP_DEPTH).expect("only the over-budget optional image is omitted");
        let pixel = photocraft_compose::flatten(&imported.document).get(1, 1);
        assert_eq!(pixel, [1., 0., 0., 1.], "the valid outer shape survives, and the rejected child is not decoded/drawn");
        assert!(import_svg_preview_with_work("strict.svg", text.as_bytes(), MAX_RASTER_PIXELS).is_err(), "stored previews must still draw completely");
    }
    for source in [url("image/svg+xml", green.as_bytes()), url("image/svg+xml", &gzip(green.as_bytes()))] {
        let text = drawing(&source);
        let imported = import_svg_with_group_depth("valid.svg", text.as_bytes(), MAX_GROUP_DEPTH).unwrap();
        assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [0., 1., 0., 1.]);
        import_svg_preview_with_work("valid-preview.svg", text.as_bytes(), MAX_RASTER_PIXELS).unwrap();
    }
}
