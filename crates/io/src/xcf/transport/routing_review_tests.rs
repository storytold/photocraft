use super::*;
use crate::xcf::tests::{TestImage, TestLayer};
use crate::{ImportCompression, ImportOptions, ImportPreflight, Route};
use std::io::Write;

fn reset_counts() {
    DECODER_STARTS.set(0);
    crate::COMPRESSED_DETECTOR_CALLS.set(0);
}

fn assert_no_compressed_central_detection() {
    assert_eq!(crate::COMPRESSED_DETECTOR_CALLS.get(), 0, "the central detector must never receive compressed input (its SVGZ probe inflates internally)");
}

fn plain() -> Vec<u8> {
    let mut image = TestImage::rgb8(1, 1);
    image.version = 26;
    image.layers.push(TestLayer::rgba("Routed pixels", 1, 1, vec![17, 31, 57, 255]));
    image.write()
}

fn wrappers(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(bytes).unwrap();
    vec![gzip.finish().unwrap(), crabz2::compress(bytes, crabz2::Level::default()), xz(bytes)]
}

/// Original uncompressed LZMA2 chunks in an XZ CRC32 stream; no encoder feature is needed.
fn xz(bytes: &[u8]) -> Vec<u8> {
    fn vli(mut value: usize, out: &mut Vec<u8>) {
        loop {
            let byte = (value & 127) as u8;
            value >>= 7;
            out.push(if value == 0 { byte } else { byte | 128 });
            if value == 0 {
                break;
            }
        }
    }
    let mut out = b"\xfd7zXZ\0\0\x01".to_vec();
    out.extend(crc32fast::hash(&[0, 1]).to_le_bytes());
    let mut header = vec![2, 0, 0x21, 1, 0, 0, 0, 0];
    header.extend(crc32fast::hash(&header).to_le_bytes());
    out.extend(&header);
    let mut data = Vec::new();
    for chunk in bytes.chunks(65536) {
        data.push(1);
        data.extend(((chunk.len() - 1) as u16).to_be_bytes());
        data.extend(chunk);
    }
    data.push(0);
    out.extend(&data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    out.extend(crc32fast::hash(bytes).to_le_bytes());
    let mut index = vec![0, 1];
    vli(header.len() + data.len() + 4, &mut index);
    vli(bytes.len(), &mut index);
    while !index.len().is_multiple_of(4) {
        index.push(0);
    }
    index.extend(crc32fast::hash(&index).to_le_bytes());
    out.extend(&index);
    let mut footer = ((index.len() / 4 - 1) as u32).to_le_bytes().to_vec();
    footer.extend([0, 1]);
    out.extend(crc32fast::hash(&footer).to_le_bytes());
    out.extend(footer);
    out.extend(b"YZ");
    out
}

#[test]
fn compressed_signatures_are_bounded_and_shared_with_preflight_routing() {
    for (signature, kind) in [(&b"\x1f\x8b"[..], ImportCompression::Gzip), (&b"BZh"[..], ImportCompression::Bzip2), (&b"\xfd7zXZ\0"[..], ImportCompression::Xz)]
    {
        for end in 0..signature.len() {
            assert_eq!(crate::import_compression(&signature[..end]), None);
        }
        assert_eq!(crate::import_compression(signature), Some(kind));
        for name in ["wrong.psd", "wrong.tif", "wrong.pdf", "wrong.svgz", "unknown.bin"] {
            assert_eq!(crate::route(name, signature), Route::Compressed);
            assert_eq!(crate::import_preflight(name, signature), ImportPreflight::Unbounded);
        }
    }
    for bytes in [&b"gimp xcf "[..], &b"<svg/>"[..], &b"8BPS"[..], &b"\x89PNG\r\n\x1a\n"[..]] {
        assert_eq!(crate::import_compression(bytes), None);
    }
}

#[test]
fn plain_and_wrapped_xcf_magic_wins_suffixes_without_repeating_the_decoder() {
    let plain = plain();
    let inputs: Vec<_> = std::iter::once(plain.clone()).chain(wrappers(&plain)).collect();
    for (index, bytes) in inputs.iter().enumerate() {
        for name in ["wrong.psd", "wrong.tif", "wrong.pdf", "wrong.svg", "wrong.svgz", "wrong.pcraft", "unknown.bin"] {
            assert_eq!(crate::import_preflight(name, bytes), ImportPreflight::Unbounded);
            reset_counts();
            let imported = crate::import(name, bytes).unwrap_or_else(|error| panic!("input {index}, {name}: {error}"));
            assert_eq!(DECODER_STARTS.get(), usize::from(index != 0), "exactly one compressed decoder is constructed per wrapped import");
            assert_no_compressed_central_detection();
            assert!(imported.source_read_only);
            assert_eq!(imported.document.layers[0].surface().unwrap().read_region(imported.document.bounds()), [17. / 255., 31. / 255., 57. / 255., 1.]);
        }
    }
}

#[test]
fn forced_open_as_keeps_precedence_over_xcf_magic_and_automatic_wrapper_probes() {
    let plain = plain();
    for bytes in std::iter::once(plain.clone()).chain(wrappers(&plain)) {
        for ext in ["png", "psd", "svg", "pdf"] {
            reset_counts();
            assert!(crate::import_with_options_as("source.svgz", &bytes, Some(ext), &ImportOptions::default()).is_err(), "forced {ext} must not become XCF");
            assert_eq!(DECODER_STARTS.get(), 0, "forced non-XCF decoders must bypass automatic transport probing");
            assert_no_compressed_central_detection();
        }
        reset_counts();
        let imported = crate::import_with_options_as("source.bin", &bytes, Some("xcf"), &ImportOptions::default()).unwrap();
        assert_eq!(imported.document.layers.len(), 1);
        assert_no_compressed_central_detection();
    }
}

#[test]
fn ordinary_svgz_reuses_the_probe_decoder_and_keeps_non_xcf_fallbacks() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="#123456"/></svg>"##;
    let gzip = wrappers(svg).remove(0);
    for name in ["drawing.svgz", "drawing.svg", "wrong.psd", "wrong.pdf", "unknown.bin"] {
        reset_counts();
        let imported = crate::import(name, &gzip).unwrap();
        assert_eq!(DECODER_STARTS.get(), 1);
        assert_no_compressed_central_detection();
        assert_eq!(imported.document.size, photocraft_geom::Size::new(1, 1));
        assert!(!imported.source_read_only);
    }
    // A wrapper does not turn arbitrary compressed content into SVG without the SVG route.
    assert!(decode_import("unrelated.bin", &gzip, &Interrupt::NONE, false).unwrap().is_none());
    let bytes = wrappers(b"<svg/>").remove(0);
    let decoded = decode_import("tiny.svgz", &bytes, &Interrupt::NONE, true).unwrap().unwrap();
    assert!(!decoded.is_xcf);
    assert_eq!(decoded.bytes, b"<svg/>");
    let too_large = wrappers(&[svg.as_slice(), &[b' '; 1024]].concat()).remove(0);
    assert!(
        matches!(decode_stream("limit.svgz", &too_large, &Interrupt::NONE, 512, false, true), Err(IoError::Svg(message)) if message.contains("compressed SVG inflates past"))
    );
    let nested = wrappers(&gzip).remove(0);
    assert!(matches!(crate::import("nested.svgz", &nested), Err(IoError::Svg(message)) if message.contains("recursive wrapping")));
    let mut corrupt = gzip;
    let crc = corrupt.len() - 8;
    corrupt[crc] ^= 1;
    assert!(matches!(crate::import("corrupt.svgz", &corrupt), Err(IoError::Svg(_))));
    assert_no_compressed_central_detection();
}

#[test]
fn compressed_candidates_use_filename_policy_until_the_single_cancellable_probe() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>"#;
    let gzip = wrappers(svg).remove(0);
    reset_counts();
    // A behavioral oracle: the shared detector would inflate this and return SVGZ.
    // The candidate classifier must use only filename policy before the worker probe.
    assert_eq!(crate::detected_route("misleading.psd", &gzip), Route::NamedPsd);
    assert_eq!(DECODER_STARTS.get(), 0);
    assert_no_compressed_central_detection();
    let imported = crate::import("misleading.psd", &gzip).unwrap();
    assert_eq!(imported.document.size, photocraft_geom::Size::new(1, 1));
    assert_eq!(DECODER_STARTS.get(), 1);
    assert_no_compressed_central_detection();
    // Keep the shared 4096-byte signature window for renamed files; a declared SVGZ
    // may have more whitespace because its actual parser remains authoritative.
    for (spaces, name) in [(3000, "misleading.psd"), (70 * 1024, "whitespace.svgz")] {
        let bytes = wrappers(&[vec![b' '; spaces], svg.to_vec()].concat()).remove(0);
        reset_counts();
        assert_eq!(crate::import(name, &bytes).unwrap().document.size, photocraft_geom::Size::new(1, 1));
        assert_eq!(DECODER_STARTS.get(), 1);
        assert_no_compressed_central_detection();
    }
    let unrelated = wrappers(b"not SVG, PNG, or XCF").remove(0);
    assert!(decode_import("unrelated.bin", &unrelated, &Interrupt::NONE, true).unwrap().is_none());
    reset_counts();
    assert!(crate::import("unrelated.bin", &unrelated).is_err());
    assert_no_compressed_central_detection();
}

#[test]
fn renamed_utf16_svg_keeps_signature_at_bom_and_surrogate_boundaries() {
    let root = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"><!--";
    let mut texts = vec![format!("{}<svg/>", " ".repeat(4092))];
    // Cover a surrogate pair crossing both the old 8192-byte cut and the corrected
    // BOM-inclusive cut. Only the sniff prefix is shortened; the payload stays intact.
    for high_at in [4094, 4095] {
        texts.push(format!("{root}{}😀--></svg>", " ".repeat(high_at - root.encode_utf16().count())));
    }
    for little in [false, true] {
        for text in &texts {
            let mut bytes = if little { vec![0xff, 0xfe] } else { vec![0xfe, 0xff] };
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&if little { unit.to_le_bytes() } else { unit.to_be_bytes() });
            }
            let complete = craft_svg::svgz::decode(&bytes).unwrap();
            assert!(crate::svg::is_svg(complete.as_bytes()), "independent full-text signature oracle");
            let gzip = wrappers(&bytes).remove(0);
            for name in ["renamed.bin", "renamed.psd"] {
                reset_counts();
                let decoded = decode_import(name, &gzip, &Interrupt::NONE, true).unwrap().expect("a valid UTF-16 signature must survive the sniff boundary");
                assert!(!decoded.is_xcf);
                assert_eq!(decoded.bytes, bytes, "sniffing must not truncate or repair actual parser input");
                assert_eq!(DECODER_STARTS.get(), 1);
                assert_no_compressed_central_detection();
            }
        }
        let mut invalid = if little { vec![0xff, 0xfe] } else { vec![0xfe, 0xff] };
        for unit in "<svg/>".encode_utf16().chain(std::iter::once(0xd800)) {
            invalid.extend_from_slice(&if little { unit.to_le_bytes() } else { unit.to_be_bytes() });
        }
        assert!(craft_svg::svgz::decode(&invalid).is_err(), "the complete decoder must still reject a real unpaired surrogate");
        assert!(crate::import("invalid.svgz", &wrappers(&invalid).remove(0)).is_err());
    }
}

#[test]
fn svgz_legacy_tail_and_later_member_fallback_preserves_verified_output() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>"#;
    let first = wrappers(svg).remove(0);
    let mut corrupt_second = wrappers(&vec![b'x'; CHUNK * 3]).remove(0);
    let crc = corrupt_second.len() - 8;
    corrupt_second[crc] ^= 1;
    let tails = [b"arbitrary tail".to_vec(), [vec![0; 17], b"tail".to_vec()].concat(), vec![0x1f], vec![0x1f, 0x8b], corrupt_second.clone()];
    let xcf = wrappers(&plain()).remove(0);
    for tail in tails {
        let bytes = [first.as_slice(), tail.as_slice()].concat();
        let legacy = craft_svg::svgz::inflate(&bytes, crate::svg::MAX_INFLATED_BYTES as usize).unwrap();
        assert_eq!(legacy.as_ref(), svg, "independent preexisting SVGZ policy oracle");
        for name in ["legacy.svgz", "renamed.psd"] {
            reset_counts();
            let decoded = decode_import(name, &bytes, &Interrupt::NONE, true).unwrap().unwrap();
            assert!(!decoded.is_xcf);
            assert_eq!(decoded.bytes, legacy.as_ref());
            assert_eq!(DECODER_STARTS.get(), 1);
            assert_no_compressed_central_detection();
            reset_counts();
            assert_eq!(crate::import(name, &bytes).unwrap().document.size, photocraft_geom::Size::new(1, 1));
            assert_eq!(DECODER_STARTS.get(), 1);
            assert_no_compressed_central_detection();
        }
        let strict = [xcf.as_slice(), tail.as_slice()].concat();
        assert!(decode("strict.xcf.gz", &strict, &Interrupt::NONE).is_err());
        assert!(crate::import("strict.bin", &strict).is_err(), "claiming the XCF prefix must enable strict policy in the automatic probe too");
    }
    // Roll back only the failed member, preserving all earlier verified members.
    let second = wrappers(b" ").remove(0);
    let bytes = [first, second, corrupt_second].concat();
    let legacy = craft_svg::svgz::inflate(&bytes, crate::svg::MAX_INFLATED_BYTES as usize).unwrap();
    assert_eq!(legacy.as_ref(), [svg.as_slice(), b" "].concat());
    assert_eq!(decode_import("three.svgz", &bytes, &Interrupt::NONE, true).unwrap().unwrap().bytes, legacy.as_ref());
}

#[test]
fn svgz_first_member_damage_remains_an_error_and_cancellation_is_not_a_fallback() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>"#;
    let mut bad = wrappers(svg).remove(0);
    let crc = bad.len() - 8;
    bad[crc] ^= 1;
    assert!(craft_svg::svgz::inflate(&bad, crate::svg::MAX_INFLATED_BYTES as usize).is_err());
    assert!(matches!(crate::import("first.svgz", &bad), Err(IoError::Svg(_))));
    let cancelled = Interrupt::cancel_only(&|| true);
    assert!(matches!(crate::import_with("cancelled.svgz", &[wrappers(svg).remove(0), bad].concat(), &cancelled), Err(IoError::Cancelled)));
}

#[test]
fn wrapper_import_preserves_immediate_and_mid_stream_cancellation() {
    let mut bytes = plain();
    bytes.resize(bytes.len() + CHUNK * 64, 0);
    for wrapped in wrappers(&bytes) {
        let cancelled = Interrupt::cancel_only(&|| true);
        reset_counts();
        assert!(matches!(crate::import_with("wrong.pdf", &wrapped, &cancelled), Err(IoError::Cancelled)));
        assert_eq!(DECODER_STARTS.get(), 0);
        assert_no_compressed_central_detection();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 8;
        assert!(matches!(crate::import_with("wrong.svgz", &wrapped, &Interrupt::cancel_only(&cancel)), Err(IoError::Cancelled)));
        assert_eq!(DECODER_STARTS.get(), 1);
        assert_no_compressed_central_detection();
    }
}

#[test]
fn explicit_af_alias_keeps_upstream_precedence_except_for_real_xcf_magic() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>"#;
    let gzip = wrappers(svg).remove(0);
    for name in ["drawing.af", "drawing.AF", ".af", "folder/drawing.af"] {
        // Independent oracle: this branch returns before the shared detector's SVGZ probe.
        assert_eq!(photocraft_formats::detect(name, &gzip), Some(photocraft_formats::Format::Affinity));
        let expected = crate::affinity::import(name, &gzip, &Interrupt::NONE).unwrap_err().to_string();
        reset_counts();
        let error = crate::import(name, &gzip).expect_err("the explicit native alias must not open as a generic SVG preview");
        assert_eq!(error.to_string(), expected);
        assert_eq!(DECODER_STARTS.get(), 1, "the XCF signature probe still uses one decoder");
        assert_no_compressed_central_detection();
    }
    // Longer Affinity extensions are suffix fallbacks, not the special .af policy.
    for name in ["drawing.afphoto", "drawing.afdesign", "drawing.afpub"] {
        reset_counts();
        assert_eq!(crate::import(name, &gzip).unwrap().document.size, photocraft_geom::Size::new(1, 1));
        assert_eq!(DECODER_STARTS.get(), 1);
        assert_no_compressed_central_detection();
    }
    for bytes in std::iter::once(plain()).chain(wrappers(&plain())) {
        reset_counts();
        let imported = crate::import("drawing.af", &bytes).unwrap();
        assert!(imported.source_read_only);
        assert_eq!(imported.document.layers[0].surface().unwrap().read_region(imported.document.bounds()), [17. / 255., 31. / 255., 57. / 255., 1.]);
        assert_no_compressed_central_detection();
    }
    assert_eq!(
        crate::import_with_options_as("drawing.af", &gzip, Some("svg"), &ImportOptions::default()).unwrap().document.size,
        photocraft_geom::Size::new(1, 1)
    );
}
