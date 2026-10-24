//! Transport cases use the existing published source pin, not private research fixtures.
#![cfg(feature = "corpus")]

use std::io::Write;

fn source() -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/gimp/layers-8bit.xcf");
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}; run cargo xtask corpus --gimp", path.display()))
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut writer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    writer.write_all(bytes).unwrap();
    writer.finish().unwrap()
}

fn bzip2(bytes: &[u8]) -> Vec<u8> {
    crabz2::compress(bytes, crabz2::Level::default())
}

// Independent wire writer from the XZ/LZMA2 specifications: an uncompressed LZMA2 chunk,
// CRC32 check, one block and one index record. No XZ encoder feature is enabled.
fn xz(bytes: &[u8], dictionary: u8) -> Vec<u8> {
    fn vli(mut value: usize, out: &mut Vec<u8>) {
        loop {
            let mut byte = (value & 127) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 128;
            }
            out.push(byte);
            if value == 0 {
                break;
            }
        }
    }
    assert!(!bytes.is_empty() && bytes.len() <= 65536);
    let mut out = b"\xfd7zXZ\0\0\x01".to_vec();
    out.extend_from_slice(&crc32fast::hash(&[0, 1]).to_le_bytes());
    let mut header = vec![2, 0, 0x21, 1, dictionary, 0, 0, 0];
    header.extend_from_slice(&crc32fast::hash(&header).to_le_bytes());
    out.extend_from_slice(&header);
    let mut data = vec![1]; // Uncompressed LZMA2 chunk, dictionary reset.
    data.extend_from_slice(&((bytes.len() - 1) as u16).to_be_bytes());
    data.extend_from_slice(bytes);
    data.push(0); // LZMA2 end marker.
    out.extend_from_slice(&data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    out.extend_from_slice(&crc32fast::hash(bytes).to_le_bytes());
    let mut index = vec![0, 1];
    vli(header.len() + data.len() + 4, &mut index);
    vli(bytes.len(), &mut index);
    while !index.len().is_multiple_of(4) {
        index.push(0);
    }
    index.extend_from_slice(&crc32fast::hash(&index).to_le_bytes());
    out.extend_from_slice(&index);
    let mut footer = ((index.len() / 4 - 1) as u32).to_le_bytes().to_vec();
    footer.extend_from_slice(&[0, 1]);
    out.extend_from_slice(&crc32fast::hash(&footer).to_le_bytes());
    out.extend_from_slice(&footer);
    out.extend_from_slice(b"YZ");
    out
}

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

#[test]
fn compressed_non_xcf_content_keeps_its_ordinary_route() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="7" height="5"><rect width="7" height="5" fill="red"/></svg>"#;
    let imported = photocraft_io::import("drawing.svgz", &gzip(svg)).unwrap();
    assert_eq!((imported.document.size.width, imported.document.size.height), (7, 5));
    assert!(photocraft_io::import("not-xcf.xcf.gz", &gzip(b"unrelated text")).is_err());
}

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
    // LZMA2 property 28 requests 64 MiB plus decoder overhead, exceeding the 64 MiB budget.
    let result = photocraft_io::import("excessive.xcf.xz", &xz(&source, 28));
    let Err(error) = result else { panic!("excessive XZ dictionary accepted") };
    assert!(error.to_string().contains("mem_limit_kb"), "{error}");
}

#[test]
fn concatenated_bzip2_xz_and_xz_padding_form_one_source_stream() {
    let source = source();
    let split = 3;
    let bz = [bzip2(&source[..split]), bzip2(&source[split..])].concat();
    let xz = [xz(&source[..split], 0), vec![0; 8], xz(&source[split..], 0), vec![0; 4]].concat();
    let expected = photocraft_compose::flatten(&photocraft_io::import("plain.xcf", &source).unwrap().document).px;
    for wrapped in [bz, xz] {
        assert_eq!(photocraft_compose::flatten(&photocraft_io::import("members.pcraft", &wrapped).unwrap().document).px, expected);
    }
}
