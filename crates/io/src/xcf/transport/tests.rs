use super::*;
use crate::{ImportOptions, IoError};
use std::io::Write;

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut writer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    writer.write_all(bytes).unwrap();
    writer.finish().unwrap()
}

fn plain() -> Vec<u8> {
    use crate::xcf::tests::{TestImage, TestLayer};
    let mut image = TestImage::rgb8(1, 1);
    image.layers.push(TestLayer::rgba("Transport", 1, 1, vec![17, 31, 57, 255]));
    image.write()
}

fn hex(encoded: &str) -> Vec<u8> {
    encoded.as_bytes().as_chunks::<2>().0.iter().map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect()
}

#[test]
fn borrowed_plain_and_continuing_gzip_import_preserve_native_pixels() {
    let plain = plain();
    assert!(
        matches!(decode("plain.xcf", &plain, &Interrupt::NONE, false, true).unwrap(), Decoded::Xcf(Cow::Borrowed(bytes)) if std::ptr::eq(bytes.as_ptr(), plain.as_ptr()))
    );
    let wrapped = gzip(&plain);
    for name in ["source.xcf", "source.XCF.GZ", "source.xcfgz", "renamed.bin", "renamed.svg", "source.af"] {
        let result = crate::import(name, &wrapped).unwrap();
        assert!(result.source_read_only && !result.preview_only, "{name}");
        let surface = result.document.layers[0].surface().unwrap();
        assert_eq!(surface.read_region(photocraft_geom::Rect::new(0, 0, 1, 1)), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    }
}

#[test]
fn modern_shared_wrapper_aliases_accept_original_bzip_and_xz_sources() {
    // Original empty 1x1 RGB8 v26 XCF: public header framing, three null lists.
    // Compressed with Python standard-library bz2/lzma, not the decoder under test.
    let sources = [
        (
            hex("425a6839314159265359d369cc7000000df9a0680000044000510009a241400100200022bd134190cd42869a600348a514011351c3eecd1363f177245385090d369cc700"),
            ["source.XCF.BZ2", "source.xcfbz2"],
        ),
        (
            hex(
                "fd377a585a000004e6d6b4460200210116000000742fe5a3e0003d001b5d00339a49fbe397e4455415f47d24eb590444701acb0ca7bde17c6700000022f43d0d7a807d030001373eb408e0711fb6f37d010000000004595a",
            ),
            ["source.XCF.XZ", "source.xcfxz"],
        ),
    ];
    for (bytes, names) in sources {
        for name in names.into_iter().chain(["unknown.bin"]) {
            let result = crate::import(name, &bytes).unwrap();
            assert_eq!(result.document.size, photocraft_geom::Size::new(1, 1));
            assert!(result.document.layers.is_empty());
            assert!(result.source_read_only && !result.preview_only);
        }
        for ext in [".XCF", "xcf.gz", "xcfbz2", "xcfxz"] {
            let result = crate::import_with_options_as("source.bin", &bytes, Some(ext), &ImportOptions::default()).unwrap();
            assert_eq!(result.document.size, photocraft_geom::Size::new(1, 1));
        }
    }
}

#[test]
fn automatic_signatures_and_forced_open_as_keep_distinct_precedence() {
    let plain = plain();
    let options = ImportOptions::default();
    for ext in ["png", ".PDF", "af"] {
        let result = crate::import_with_options_as("source.xcf", &plain, Some(ext), &options);
        assert!(result.is_err());
        assert!(!matches!(result, Err(IoError::Xcf(_))), "{ext}");
    }
    let image = photocraft_codecs::Image::from_raw(2, 1, photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8, vec![90; 8]).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    assert_eq!(crate::import("renamed.xcf", &png).unwrap().document.size, photocraft_geom::Size::new(2, 1));
    assert!(matches!(crate::import_with_options_as("source.png", &png, Some("xcf"), &options), Err(IoError::Xcf(_))));
    assert!(matches!(crate::import_preflight("source.psd", &gzip(&plain)), crate::ImportPreflight::Unbounded));
}

#[test]
fn renamed_svgz_uses_its_existing_full_decoder_and_keeps_affinity_precedence() {
    let xml = b"<svg xmlns='http://www.w3.org/2000/svg' width='4' height='3'><rect width='4' height='3' fill='red'/></svg>";
    let mut wrapped = gzip(xml);
    wrapped.extend(b"nonmember SVG tail");
    for name in ["renamed.bin", "renamed.psd", "renamed.pdf", "source.svgz", "source.xcf"] {
        assert_eq!(crate::import(name, &wrapped).unwrap().document.size, photocraft_geom::Size::new(4, 3), "{name}");
    }
    assert!(matches!(crate::import("source.af", &wrapped), Err(IoError::Unsupported(_))));
    assert!(matches!(crate::import_with_options_as("source.svgz", &wrapped, Some("xcf"), &ImportOptions::default()), Err(IoError::Xcf(_))));
    let mut xcf = gzip(&plain());
    xcf.extend(b"nonmember XCF tail");
    assert!(matches!(crate::import("renamed.bin", &xcf), Err(IoError::Xcf(_))));
}

#[test]
fn wrapper_damage_and_cancellation_are_not_lost_to_filename_fallback() {
    let mut wrapped = gzip(&plain());
    let crc = wrapped.len() - 8;
    wrapped[crc] ^= 1;
    assert!(matches!(crate::import("renamed.bin", &wrapped), Err(IoError::Xcf(_))));
    let polls = std::sync::atomic::AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 5;
    let mut header = gzip(&plain());
    header[3] |= 8;
    header.splice(10..10, [vec![b'x'; 32 * 1024 * 16], vec![0]].concat());
    assert!(matches!(crate::import_with("renamed.bin", &header, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
}
