use super::*;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut writer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    writer.write_all(bytes).unwrap();
    writer.finish().unwrap()
}

fn read_seventeen(reader: &mut dyn Read) -> Result<[u8; 17]> {
    let mut output = [0; 17];
    reader.read_exact(&mut output).map_err(|error| invalid(error.to_string()))?;
    Ok(output)
}

#[test]
fn prefix_reader_counts_replayed_magic_and_refuses_reads_past_its_allowance() {
    let bytes = [MAGIC, &vec![0; CHUNK * 64]].concat();
    for wrapped in [bytes.clone(), gzip(&bytes), crabz2::compress(&bytes, crabz2::Level::default())] {
        let polls = AtomicUsize::new(0);
        let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 64;
        let ctl = Interrupt::cancel_only(&cancel);
        DECODER_STARTS.set(0);
        assert_eq!(with_xcf_prefix_limit("source.xcf", &wrapped, &ctl, 17, read_seventeen).unwrap().unwrap().as_slice(), &bytes[..17]);
        assert!(polls.load(Ordering::Relaxed) < 64);
        assert_eq!(DECODER_STARTS.get(), usize::from(wrapped != bytes));
        assert!(with_xcf_prefix_limit("source.xcf", &wrapped, &Interrupt::NONE, 16, read_seventeen).is_err());
    }
}

#[test]
fn prefix_reader_keeps_unclaimed_input_and_declared_alias_errors_distinct() {
    let wrapped = gzip(b"not an XCF file, even when compressed");
    assert!(
        with_xcf_prefix("unknown.bin", &wrapped, &Interrupt::NONE, |_| -> Result<()> { panic!("unclaimed input must not reach the XCF scanner") })
            .unwrap()
            .is_none()
    );
    for name in ["plain.xcf", "source.XCF.GZ", "source.xcfgz", "source.xcf.bz2", "source.xcfbz2", "source.xcf.xz", "source.xcfxz"] {
        assert!(matches!(with_xcf_prefix(name, &wrapped, &Interrupt::NONE, |_| Ok(())), Err(IoError::Xcf(_))), "{name}");
    }
    let polls = AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 3;
    let xcf = gzip(&[MAGIC, &vec![0; CHUNK]].concat());
    assert!(matches!(with_xcf_prefix("unknown.bin", &xcf, &Interrupt::cancel_only(&cancel), read_seventeen), Err(IoError::Cancelled)));
}

#[test]
fn svg_prefix_recognizes_a_large_drawing_without_validating_its_unread_body() {
    let bytes = [b"<svg xmlns='http://www.w3.org/2000/svg'><!--".as_slice(), &vec![b'x'; CHUNK * 64], b"--></svg>"].concat();
    let mut wrapped = gzip(&bytes);
    let crc = wrapped.len() - 8;
    wrapped[crc] ^= 1;
    let polls = AtomicUsize::new(0);
    let cancel = || polls.fetch_add(1, Ordering::Relaxed) >= 64;
    let prefix = svg_prefix("renamed.bin", &wrapped, &Interrupt::cancel_only(&cancel)).unwrap().unwrap();
    assert_eq!(prefix, bytes[..SVG_PROBE_BYTES]);
    assert!(polls.load(Ordering::Relaxed) < 64);
    assert!(
        matches!(decode_import("renamed.bin", &wrapped, &Interrupt::NONE, true), Err(IoError::Svg(_))),
        "actual import still verifies the first-member CRC"
    );
    assert!(svg_prefix("invalid.svgz", &gzip(b"not SVG"), &Interrupt::NONE).unwrap().is_none());
    assert!(svg_prefix("source.svgz", &gzip(&[MAGIC, &vec![0; SVG_PROBE_BYTES]].concat()), &Interrupt::NONE).unwrap().is_none());
}

#[test]
fn compressed_svg_under_a_plain_xcf_suffix_falls_back_to_svg_geometry() {
    let wrapped = gzip(b"<svg xmlns='http://www.w3.org/2000/svg' width='40' height='30'><rect x='10' y='12' width='5' height='6'/></svg>");
    assert!(crate::xcf::source_geometry("Drawing.XCF", &wrapped, &Interrupt::NONE).unwrap().is_none());
    let geometry = crate::linked::source_geometry("Drawing.XCF", &wrapped, &Interrupt::NONE).unwrap();
    assert_eq!(geometry.0, (40.0, 30.0));
    assert!(crate::xcf::source_geometry("Drawing.xcf.gz", &wrapped, &Interrupt::NONE).is_err());
}
