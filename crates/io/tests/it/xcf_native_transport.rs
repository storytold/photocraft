//! Transport cases use the existing published source pin, not private research fixtures.
#![cfg(feature = "corpus")]

use photocraft_testkit::xcf::xz_with_dictionary as xz;
use std::io::Write;

/// Read the pinned public layered XCF fixture, failing with the corpus-fetch instruction when it is unavailable.
fn source() -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/gimp/layers-8bit.xcf");
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}; run cargo xtask corpus --gimp", path.display()))
}

/// Wrap source bytes in a complete gzip member using the ordinary encoder for transport validation tests.
fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut writer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    writer.write_all(bytes).unwrap();
    writer.finish().unwrap()
}

/// Create a complete bzip2 member around independently selected source-stream bytes.
fn bzip2(bytes: &[u8]) -> Vec<u8> {
    crabz2::compress(bytes, crabz2::Level::default())
}

/// Verify plain or compressed XCF signatures select layered native import even when the filename suggests another known format.
#[test]
fn xcf_content_wins_over_misleading_known_suffixes() {
    let source = source();
    let expected = photocraft_io::import("source.xcf", &source).unwrap().document;
    for name in ["renamed.pcraft", "renamed.ora", "renamed.kra", "renamed.afphoto", "renamed.png", "renamed.psd", "unnamed"] {
        for bytes in [&source, &gzip(&source)] {
            let imported = photocraft_io::import(name, bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert!(imported.source_read_only && !imported.preview_only, "{name}");
            assert_eq!(imported.document.size, expected.size);
            assert_eq!(photocraft_compose::flatten(&imported.document).px, photocraft_compose::flatten(&expected).px);
        }
    }
}

/// Split source bytes across independently compressed gzip members and require one unchanged logical XCF image.
#[test]
fn gzip_members_form_one_verified_logical_xcf_stream() {
    let source = source();
    // The source signature and a tile can straddle independently compressed members.
    for split in [3, source.len() / 2, source.len() - 1] {
        let wrapped = [gzip(&source[..split]), gzip(&source[split..])].concat();
        let imported = photocraft_io::import("members.xcf.gz", &wrapped).unwrap();
        let expected = photocraft_io::import("plain.xcf", &source).unwrap();
        assert_eq!(photocraft_compose::flatten(&imported.document).px, photocraft_compose::flatten(&expected.document).px);
    }
}

/// Reject incomplete gzip trailers, bad checksums, unrelated tails and recursive transport rather than publishing partial XCF data.
#[test]
fn gzip_rejects_corruption_truncation_and_unrelated_compressed_trailing_bytes() {
    let wrapped = gzip(&source());
    for remove in [1, 4, 8] {
        assert!(photocraft_io::import("truncated.xcf.gz", &wrapped[..wrapped.len() - remove]).is_err());
    }
    let mut bad_crc = wrapped.clone();
    let checksum = bad_crc.len() - 8;
    bad_crc[checksum] ^= 1;
    assert!(photocraft_io::import("checksum.xcf.gz", &bad_crc).is_err());
    let mut trailing = wrapped;
    trailing.extend_from_slice(b"unrelated trailing bytes");
    assert!(photocraft_io::import("trailing.xcf.gz", &trailing).is_err());
    assert!(photocraft_io::import("recursive.xcf.gz", &gzip(&gzip(&source()))).is_err());
}

/// Keep ordinary compressed SVG routing intact while refusing a declared XCF wrapper whose content is not XCF.
#[test]
fn compressed_non_xcf_content_keeps_its_ordinary_route() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="7" height="5"><rect width="7" height="5" fill="red"/></svg>"#;
    let imported = photocraft_io::import("drawing.svgz", &gzip(svg)).unwrap();
    assert_eq!((imported.document.size.width, imported.document.size.height), (7, 5));
    assert!(photocraft_io::import("not-xcf.xcf.gz", &gzip(b"unrelated text")).is_err());
}

/// Compare native import across bzip2/XZ while rejecting every truncation, checksum error, unrelated tail and oversized XZ dictionary.
#[test]
fn bzip2_and_xz_use_the_same_native_import_and_validate_complete_streams() {
    let source = source();
    let expected = photocraft_io::import("source.xcf", &source).unwrap().document;
    for (name, wrapped) in [("source.xcf.bz2", bzip2(&source)), ("source.xcf.xz", xz(&source, 0))] {
        for suffix in [name, "renamed.pcraft", "no-suffix"] {
            let imported = photocraft_io::import(suffix, &wrapped).unwrap_or_else(|error| panic!("{suffix}: {error}"));
            assert!(imported.source_read_only);
            assert_eq!(photocraft_compose::flatten(&imported.document).px, photocraft_compose::flatten(&expected).px);
        }
        for end in 6..wrapped.len() {
            assert!(photocraft_io::import(name, &wrapped[..end]).is_err(), "{name}: truncation {end}");
        }
        assert!(photocraft_io::import(name, &[wrapped.as_slice(), b"unrelated trailing data"].concat()).is_err());
        let mut corrupt = wrapped.clone();
        let at = if name.ends_with("bz2") {
            // The penultimate byte is in the combined-stream CRC, before any final pad bits.
            corrupt.len() - 2
        } else {
            // Alter a literal sample in the uncompressed LZMA2 chunk, leaving XCF's signature.
            12 + 12 + 3 + source.len() / 2
        };
        corrupt[at] ^= 1;
        assert!(photocraft_io::import(name, &corrupt).is_err(), "{name}: checksum corruption");
    }
    // Property 28 requests the supported -9 dictionary; decoder overhead is admitted too.
    let largest = photocraft_io::import("preset-9.xcf.xz", &xz(&source, 28)).unwrap().document;
    assert_eq!(photocraft_compose::flatten(&largest).px, photocraft_compose::flatten(&expected).px);
    // The next property requests 96 MiB and still exceeds the bounded dictionary policy.
    let result = photocraft_io::import("excessive.xcf.xz", &xz(&source, 29));
    let Err(error) = result else { panic!("excessive XZ dictionary accepted") };
    assert!(error.to_string().contains("mem_limit_kb"), "{error}");
}

/// Verify concatenated transport members and permitted XZ padding reconstruct one unchanged native source image.
#[test]
fn concatenated_bzip2_xz_and_xz_padding_form_one_source_stream() {
    let source = source();
    let split = 3;
    let bz = [bzip2(&source[..split]), bzip2(&source[split..])].concat();
    let xz = [xz(&source[..split], 0), vec![0; 8], xz(&source[split..], 0), vec![0; 4]].concat();
    let expected = photocraft_compose::flatten(&photocraft_io::import("plain.xcf", &source).unwrap().document).into_px();
    for wrapped in [bz, xz] {
        assert_eq!(photocraft_compose::flatten(&photocraft_io::import("members.pcraft", &wrapped).unwrap().document).px, expected);
    }
}
