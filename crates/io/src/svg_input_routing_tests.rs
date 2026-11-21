use super::*;
use std::io::Write;

const SVG: &[u8] = br#"<svg xmlns='http://www.w3.org/2000/svg' width='2' height='3'><rect width='2' height='3' fill='red'/></svg>"#;

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

#[test]
fn renamed_gzip_svg_recognition_agrees_with_import_without_shared_inflation() {
    let bytes = gzip(SVG);
    for name in ["logo.gz", "", "download.bin", "drawing.svgz", "wrong.psd", "drawing.afphoto"] {
        COMPRESSED_DETECTOR_CALLS.set(0);
        assert!(is_svg_input(name, &bytes), "{name}");
        assert!(is_svg_input_with(name, &bytes, &photocraft_raster::Interrupt::NONE).unwrap(), "{name}");
        assert_eq!(import(name, &bytes).unwrap().document.size, photocraft_geom::Size::new(2, 3));
        assert_eq!(COMPRESSED_DETECTOR_CALLS.get(), 0, "only the bounded transport may inspect compressed bytes");
    }
    for order in [true, false] {
        let mut utf16 = if order { vec![0xff, 0xfe] } else { vec![0xfe, 0xff] };
        utf16.extend(std::str::from_utf8(SVG).unwrap().encode_utf16().flat_map(|c| if order { c.to_le_bytes() } else { c.to_be_bytes() }));
        assert!(is_svg_input("", &gzip(&utf16)));
    }
}

#[test]
fn svg_recognition_rejects_archives_and_obeys_central_affinity_precedence() {
    for name in ["archive.gz", "", "drawing.af", "DRAWING.AF", ".af"] {
        let bytes = if name.to_ascii_lowercase().ends_with(".af") { gzip(SVG) } else { gzip(b"not an SVG or XCF") };
        assert!(!is_svg_input(name, &bytes), "{name}");
    }
    let wrapped_xcf = gzip(b"gimp xcf v026\0");
    assert!(!is_svg_input("wrong.svgz", &wrapped_xcf));
    assert!(!is_svg_input("drawing.svg", b"%PDF-1.7\n"));
    assert!(!is_svg_input("drawing.svg", b"RIFF\x04\0\0\0WEBP"));
}

#[test]
fn svg_recognition_reads_only_a_prefix_and_preserves_cancellation() {
    let mut text = SVG.to_vec();
    text.resize(1024 * 1024, b' ');
    let mut bytes = gzip(&text);
    let end = bytes.len();
    bytes[end - 8] ^= 1; // Corrupt the uninspected trailer, beyond a one-megabyte SVG body.
    assert!(is_svg_input_with("logo.gz", &bytes, &photocraft_raster::Interrupt::NONE).unwrap());
    assert!(import("logo.gz", &bytes).is_err(), "the real import still validates the complete envelope");
    let cancelled = photocraft_raster::Interrupt::cancel_only(&|| true);
    assert!(matches!(is_svg_input_with("logo.gz", &bytes, &cancelled), Err(IoError::Cancelled)));
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 8;
    let members = [gzip(&[]).repeat(1000), gzip(SVG)].concat();
    assert!(matches!(is_svg_input_with("", &members, &photocraft_raster::Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
}

#[test]
fn xcf_name_policy_covers_all_aliases_without_claiming_generic_archives() {
    for name in
        ["image.xcf", "image.xcf.gz", "image.xcfgz", "image.xcf.bz2", "image.xcfbz2", "image.xcf.xz", "image.xcfxz", "dir/IMAGE.XCFBZ2", "dir\\IMAGE.XCFXZ"]
    {
        assert!(is_xcf_input_name(name), "{name}");
    }
    for name in ["image.gz", "image.bz2", "image.xz", "archive.tar.gz", "image.xcfxz.bak", "xcf", "dir.xcf/no-extension"] {
        assert!(!is_xcf_input_name(name), "{name}");
    }
}
