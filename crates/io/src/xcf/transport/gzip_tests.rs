use super::*;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn plain() -> Vec<u8> {
    use crate::xcf::tests::{TestImage, TestLayer};
    let mut image = TestImage::rgb8(1, 1);
    image.layers.push(TestLayer::rgba("Padded gzip", 1, 1, vec![17, 31, 57, 255]));
    image.write()
}

#[test]
fn terminal_zero_padding_preserves_gzip_import_and_header_probe() {
    let plain = plain();
    for padding in [1, 511, CHUNK * 3 + 1] {
        let mut bytes = gzip(&plain);
        bytes.resize(bytes.len() + padding, 0);
        // The previous single-member decoder accepted these exact padded bytes.
        let mut previous = flate2::bufread::GzDecoder::new(bytes.as_slice());
        let mut expected = Vec::new();
        previous.read_to_end(&mut expected).unwrap();
        assert_eq!(expected, plain);
        for name in ["padded.xcf.gz", "padded.bin"] {
            assert_eq!(decode(name, &bytes, &Interrupt::NONE).unwrap().unwrap(), plain);
            assert_eq!(header_prefix(name, &bytes, &Interrupt::NONE).unwrap().unwrap(), plain[..30]);
            let imported = crate::import(name, &bytes).unwrap();
            assert_eq!(
                imported.document.layers[0].surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 1, 1)),
                vec![17. / 255., 31. / 255., 57. / 255., 1.]
            );
            assert!(imported.source_read_only && !imported.preview_only);
        }
    }
}

#[test]
fn padded_concatenated_members_keep_every_crc_and_truncation_check() {
    let plain = plain();
    let first = gzip(&plain[..20]);
    let second = gzip(&plain[20..]);
    let mut valid = [first.clone(), gzip(&[]), second.clone()].concat();
    valid.extend([0; 512]);
    assert_eq!(decode("members.xcf.gz", &valid, &Interrupt::NONE).unwrap().unwrap(), plain);
    assert_eq!(header_prefix("members.xcf.gz", &valid, &Interrupt::NONE).unwrap().unwrap(), plain[..30]);
    let mut corrupt_first = first.clone();
    let crc = corrupt_first.len() - 8;
    corrupt_first[crc] ^= 1;
    assert!(decode("first.xcf.gz", &[corrupt_first, second.clone(), vec![0; 32]].concat(), &Interrupt::NONE).is_err());
    let mut corrupt_second = second.clone();
    let crc = corrupt_second.len() - 8;
    corrupt_second[crc] ^= 1;
    assert!(decode("second.xcf.gz", &[first, corrupt_second, vec![0; 32]].concat(), &Interrupt::NONE).is_err());
    let complete = gzip(&plain);
    // Even an empty next member must have a valid CRC and complete trailer.
    let mut bad_empty = gzip(&[]);
    let crc = bad_empty.len() - 8;
    bad_empty[crc] ^= 1;
    assert!(decode("empty.xcf.gz", &[complete.clone(), bad_empty, vec![0; 32]].concat(), &Interrupt::NONE).is_err());
    for end in 1..second.len() {
        let truncated = [complete.clone(), second[..end].to_vec()].concat();
        assert!(decode("truncated.xcf.gz", &truncated, &Interrupt::NONE).is_err(), "next member truncated at {end}");
    }
    for tail in [vec![0x1f], vec![0x1f, 0x8b], b"junk".to_vec(), vec![0, 0, 1], [vec![0; 32], gzip(&[])].concat()] {
        assert!(decode("tail.xcf.gz", &[complete.clone(), tail].concat(), &Interrupt::NONE).is_err());
    }
    // A bounded probe deliberately stops before later trailer/tail validation.
    assert_eq!(header_prefix("tail.xcf.gz", &[complete, vec![0x1f]].concat(), &Interrupt::NONE).unwrap().unwrap(), plain[..30]);
}

#[test]
fn gzip_padding_is_cancellable_does_not_count_as_output_and_cannot_claim_other_formats() {
    let plain = plain();
    let mut bytes = gzip(&plain);
    bytes.resize(bytes.len() + CHUNK * 32, 0);
    assert_eq!(decode_with_limit("limit.xcf.gz", &bytes, &Interrupt::NONE, plain.len()).unwrap().unwrap(), plain);
    assert!(decode_with_limit("limit.xcf.gz", &bytes, &Interrupt::NONE, plain.len() - 1).is_err());
    let mut many_members = gzip(&plain);
    many_members.extend(gzip(&vec![1; 4096]));
    many_members.extend([0; 32]);
    assert!(decode_with_limit("limit.xcf.gz", &many_members, &Interrupt::NONE, plain.len() + 128).is_err());
    // Exercise the actual terminal-padding reader directly so cancellation is
    // proved to happen after the first member and before all padding is consumed.
    let checks = AtomicUsize::new(0);
    let armed = AtomicBool::new(false);
    let cancelled = || armed.load(Ordering::Relaxed) && checks.fetch_add(1, Ordering::Relaxed) >= 5;
    let ctl = Interrupt::cancel_only(&cancelled);
    let mut input = Input { remaining: &bytes, ctl };
    let policy = GzipPolicy { strict: std::cell::Cell::new(true), verified_output: std::cell::Cell::new(0), rollback: std::cell::Cell::new(None) };
    let mut reader = Gzip::new(&mut input, &policy);
    let mut output = vec![0; plain.len()];
    reader.read_exact(&mut output).unwrap();
    assert_eq!(output, plain);
    checks.store(0, Ordering::Relaxed);
    armed.store(true, Ordering::Relaxed);
    assert!(reader.read(&mut [0; 1]).is_err());
    drop(reader);
    assert!(input.remaining.len() >= CHUNK * 20, "cancel before scanning the whole padding tail");
    assert!(input.remaining.iter().all(|byte| *byte == 0));
    checks.store(0, Ordering::Relaxed);
    assert!(matches!(decode("cancel.xcf.gz", &bytes, &ctl), Err(IoError::Cancelled)));
    let mut unrelated = gzip(b"not an XCF");
    unrelated.extend([0; 32]);
    assert!(decode("unknown.bin", &unrelated, &Interrupt::NONE).unwrap().is_none());
    assert!(header_prefix("unknown.bin", &unrelated, &Interrupt::NONE).unwrap().is_none());
    assert!(decode("claimed.xcf.gz", &unrelated, &Interrupt::NONE).is_err());
    let mut recursive = gzip(&gzip(&plain));
    recursive.extend([0; 32]);
    assert!(decode("recursive.xcf.gz", &recursive, &Interrupt::NONE).is_err());
    assert!(decode("unknown.bin", &recursive, &Interrupt::NONE).unwrap().is_none());
}
