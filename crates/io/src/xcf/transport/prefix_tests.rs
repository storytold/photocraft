use super::*;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

#[test]
fn xz_preset_nine_dictionary_includes_decoder_overhead_but_stays_bounded() {
    // Original minimal v26 XCF, compressed using Python's liblzma FORMAT_XZ
    // presets 9 and (9 | PRESET_EXTREME). Both produce these identical bytes.
    // Store the small result so running this test does not allocate a -9 encoder.
    let encoded = "fd377a585a000004e6d6b446020021011c00000010cf58cce0003d001b5d00339a49fbe397e4455415f47d24eb590444701acb0ca7bde17c6700000022f43d0d7a807d030001373eb408e0711fb6f37d010000000004595a";
    let bytes: Vec<u8> = encoded.as_bytes().as_chunks::<2>().0.iter().map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect();
    let mut expected = b"gimp xcf v026\0".to_vec();
    for value in [1_u32, 1, 0, 150] {
        expected.extend(value.to_be_bytes());
    }
    // Property END plus 64-bit layer, channel, and v18+ path-list terminators.
    expected.extend([0; 32]);
    assert_eq!(bytes[16], 28, "LZMA2 property 28 declares a 64 MiB dictionary");
    assert_eq!(lzma_rust2::lzma2_get_memory_usage(64 << 20), 65_640);
    assert_eq!(decode("preset-9.xcf.xz", &bytes, &Interrupt::NONE).unwrap().unwrap(), expected);
    let imported = crate::import("preset-9e.xcf.xz", &bytes).unwrap();
    assert_eq!(imported.document.size, photocraft_geom::Size::new(1, 1));
    assert!(imported.document.layers.is_empty());

    let mut larger_dictionary = bytes.clone();
    larger_dictionary[16] = 29; // 96 MiB: still outside the admitted dictionary size.
    let crc = crc32fast::hash(&larger_dictionary[12..20]);
    larger_dictionary[20..24].copy_from_slice(&crc.to_le_bytes());
    assert!(matches!(decode("too-large.xcf.xz", &larger_dictionary, &Interrupt::NONE), Err(IoError::Xcf(_))));
    let cancelled = Interrupt::cancel_only(&|| true);
    assert!(matches!(decode("cancelled.xcf.xz", &bytes, &cancelled), Err(IoError::Cancelled)));
    let mut trailing = bytes;
    trailing.extend(b"unchecked tail");
    assert!(decode("tail.xcf.xz", &trailing, &Interrupt::NONE).is_err());
}

#[test]
fn unknown_gzip_is_rejected_after_a_bounded_signature_not_the_svg_output_budget() {
    let body = vec![b'x'; CHUNK * 64];
    let compressed = gzip(&body);
    let calls = AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= 64;
    let ctl = Interrupt::cancel_only(&cancel);
    for name in ["unrelated.bin", "unrelated.psd"] {
        calls.store(0, Ordering::Relaxed);
        DECODER_STARTS.set(0);
        // A full decode would cross this injected budget long before EOF. Returning
        // None proves the unclaimed body was not inflated up to the SVG limit.
        assert!(decode_stream(name, &compressed, &ctl, SVG_PROBE_BYTES * 2, false, true).unwrap().is_none());
        assert!(calls.load(Ordering::Relaxed) < 64);
        assert_eq!(DECODER_STARTS.get(), 1);
    }
    let expected = crate::import_with_options_as("unrelated.psd", &compressed, Some("psd"), &crate::ImportOptions::default()).unwrap_err().to_string();
    let actual = crate::import("unrelated.psd", &compressed).unwrap_err().to_string();
    assert_eq!(actual, expected, "filename fallback must receive the original compressed bytes");

    let mut damaged = compressed.clone();
    let crc = damaged.len() - 8;
    damaged[crc] ^= 1;
    assert!(decode_import("unrelated.bin", &damaged, &Interrupt::NONE, true).unwrap().is_none(), "an unclaimed body's distant CRC is not an SVG error");
    // Explicit SVG names still request full validation, regardless of their prefix.
    assert!(matches!(decode_stream("declared.svgz", &compressed, &Interrupt::NONE, SVG_PROBE_BYTES * 2, false, true), Err(IoError::Svg(_))));
    let svg = [b"<svg xmlns='http://www.w3.org/2000/svg'><!--".as_slice(), &body, b"--></svg>"].concat();
    assert!(matches!(decode_stream("renamed.bin", &gzip(&svg), &Interrupt::NONE, SVG_PROBE_BYTES * 2, false, true), Err(IoError::Svg(_))));
}

#[test]
fn bounded_unknown_gzip_probe_still_observes_cancellation_inside_headers() {
    let mut bytes = gzip(&vec![b'x'; CHUNK * 64]);
    bytes[3] |= 8;
    bytes.splice(10..10, [vec![b'n'; CHUNK * 20], vec![0]].concat());
    let calls = AtomicUsize::new(0);
    let cancel = || calls.fetch_add(1, Ordering::Relaxed) >= 5;
    assert!(matches!(decode_import("unrelated.bin", &bytes, &Interrupt::cancel_only(&cancel), true), Err(IoError::Cancelled)));
    assert!(calls.load(Ordering::Relaxed) > 5);
}
