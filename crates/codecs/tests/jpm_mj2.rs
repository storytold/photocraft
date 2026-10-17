//! Decoding the first page (JPM, Part 6) and first frame (MJ2, Part 3) of multi-image JPEG 2000
//! containers, plus the write refusal and graceful failure on hostile input.

use photocraft_codecs::ChannelLayout;
use photocraft_codecs::{DecodeOptions, DecodeWarning, EncodeOptions, Format, Image, Limits, decode_as, decode_as_with, encode};

const SIGNATURE: &[u8] = &[0x00, 0x00, 0x00, 0x0C, b'j', b'P', b' ', b' ', 0x0D, 0x0A, 0x87, 0x0A];

/// A small RGB8 image whose pixels are easy to recognise.
fn rgb8_image(seed: u8) -> Image {
    let (w, h) = (4, 4);
    let mut data = Vec::with_capacity(w as usize * h as usize * 3);
    for y in 0..h {
        for x in 0..w {
            data.push(seed.wrapping_add(x as u8).wrapping_mul(31));
            data.push(seed.wrapping_add(y as u8).wrapping_mul(17));
            data.push(128);
        }
    }
    Image::from_u8(w, h, ChannelLayout::Rgb, data).unwrap()
}

/// Lossless JPEG 2000 codestream (Part 1, raw).
fn codestream(seed: u8) -> Vec<u8> {
    encode(&rgb8_image(seed), Format::J2k, &EncodeOptions::default()).unwrap()
}

/// A minimal ISO base-media box: `size` + `type` + `payload`, size counting the header.
fn box_bytes(kind: [u8; 4], payload: &[u8]) -> Vec<u8> {
    let size = (8u32 + payload.len() as u32).to_be_bytes();
    let mut b = size.to_vec();
    b.extend_from_slice(&kind);
    b.extend_from_slice(payload);
    b
}

/// A full box (version + flags before the payload).
fn full_box(kind: [u8; 4], payload: &[u8]) -> Vec<u8> {
    let size = (8u32 + 4 + payload.len() as u32).to_be_bytes();
    let mut b = size.to_vec();
    b.extend_from_slice(&kind);
    b.extend_from_slice(&[0, 0, 0, 0]); // version + flags
    b.extend_from_slice(payload);
    b
}

fn ftyp(brand: [u8; 4]) -> Vec<u8> {
    let mut b = 0x14u32.to_be_bytes().to_vec();
    b.extend_from_slice(b"ftyp");
    b.extend_from_slice(&brand);
    b.extend_from_slice(&0u32.to_be_bytes());
    b.extend_from_slice(&brand);
    b
}

/// A JPM compound file: signature, `ftyp` brand `jpm `, an `mhdr`, one `page` box per page,
/// and the corresponding codestreams in top-level `jp2c` boxes.
fn build_jpm(codestreams: &[Vec<u8>]) -> Vec<u8> {
    let mut out = SIGNATURE.to_vec();
    out.extend_from_slice(&ftyp(*b"jpm "));
    out.extend_from_slice(&box_bytes(*b"mhdr", &[]));
    for cs in codestreams {
        out.extend_from_slice(&box_bytes(*b"page", &[]));
        out.extend_from_slice(&box_bytes(*b"jp2c", cs));
    }
    out
}

/// An MJ2 motion container: signature, `ftyp` brand `mjp2`, a `moov` whose track sample table
/// describes every codestream, then an `mdat` holding every codestream concatenated.
fn build_mj2(codestreams: &[Vec<u8>]) -> Vec<u8> {
    let samples: Vec<Vec<u8>> = codestreams.iter().map(|cs| box_bytes(*b"jp2c", cs)).collect();
    let sizes: Vec<u32> = samples.iter().map(|sample| sample.len() as u32).collect();
    let n = codestreams.len() as u32;

    // stsd: one sample description entry.
    let stsd_payload = {
        let mut p = 1u32.to_be_bytes().to_vec(); // entry_count
        let mut entry = 12u32.to_be_bytes().to_vec(); // entry size
        entry.extend_from_slice(b"jp2m");
        entry.extend_from_slice(&[0, 0, 0, 0]);
        p.extend_from_slice(&entry);
        p
    };
    let stsd = full_box(*b"stsd", &stsd_payload);
    // stco: one chunk at offset 0.
    let stco_payload = [
        1u32.to_be_bytes().to_vec(), // entry_count
        0u32.to_be_bytes().to_vec(), // chunk offset 0
    ]
    .concat();
    let stco = full_box(*b"stco", &stco_payload);
    // stsz: sample_size 0 (per-sample), entry_count n, then every sample size.
    let mut stsz_payload = 0u32.to_be_bytes().to_vec(); // sample_size 0 => per-sample
    stsz_payload.extend_from_slice(&n.to_be_bytes()); // entry_count
    for s in &sizes {
        stsz_payload.extend_from_slice(&s.to_be_bytes());
    }
    let stsz = full_box(*b"stsz", &stsz_payload);
    // stsc: one chunk holding n samples.
    let stsc_payload = [
        1u32.to_be_bytes().to_vec(), // entry_count
        1u32.to_be_bytes().to_vec(), // first_chunk
        n.to_be_bytes().to_vec(),    // samples_per_chunk
        1u32.to_be_bytes().to_vec(), // sample_description_index
    ]
    .concat();
    let stsc = full_box(*b"stsc", &stsc_payload);
    // stts: n samples with delta 1.
    let stts_payload = [
        1u32.to_be_bytes().to_vec(), // entry_count
        n.to_be_bytes().to_vec(),    // sample_count
        1u32.to_be_bytes().to_vec(), // sample_delta
    ]
    .concat();
    let stts = full_box(*b"stts", &stts_payload);

    let stbl = box_bytes(*b"stbl", &[stsd, stco, stsz, stsc, stts].concat());
    let minf = box_bytes(*b"minf", &stbl);
    let mdia = box_bytes(*b"mdia", &[box_bytes(*b"mdhd", &[]), box_bytes(*b"hdlr", &[]), minf].concat());
    let trak = box_bytes(*b"trak", &[box_bytes(*b"tkhd", &[]), mdia].concat());
    let moov = box_bytes(*b"moov", &[box_bytes(*b"mvhd", &[]), trak].concat());

    let mut mdat = Vec::new();
    for sample in &samples {
        mdat.extend_from_slice(sample);
    }
    let mdat_box = box_bytes(*b"mdat", &mdat);

    let mut out = SIGNATURE.to_vec();
    out.extend_from_slice(&ftyp(*b"mjp2"));
    out.extend_from_slice(&moov);
    out.extend_from_slice(&mdat_box);
    out
}

fn decode_first(bytes: &[u8], format: Format) -> Image {
    decode_as(format, bytes).unwrap()
}

#[test]
fn jpm_decodes_the_first_page_and_reports_the_rest() {
    let file = build_jpm(&[codestream(10), codestream(20)]);
    assert_eq!(photocraft_codecs::detect(&file), Some(Format::Jpm));

    let img = decode_first(&file, Format::Jpm);
    assert_eq!((img.width(), img.height(), img.layout()), (4, 4, ChannelLayout::Rgb));
    // The first page is the first codestream.
    assert_eq!(img.data(), rgb8_image(10).data());
    assert_eq!(img.warnings, vec![DecodeWarning::MorePages { total: Some(2) }]);
}

#[test]
fn jpm_decodes_every_page_when_asked_by_index_is_out_of_scope_first_only() {
    // A single-page JPM yields no MorePages warning.
    let file = build_jpm(&[codestream(10)]);
    let img = decode_first(&file, Format::Jpm);
    assert_eq!(img.data(), rgb8_image(10).data());
    assert!(img.warnings.is_empty());
}

#[test]
fn jpm_without_any_page_or_codestream_is_an_error_not_a_panic() {
    // Signature + ftyp only: no page at all.
    let mut out = SIGNATURE.to_vec();
    out.extend_from_slice(&ftyp(*b"jpm "));
    let err = decode_as(Format::Jpm, &out).unwrap_err();
    assert!(err.to_string().contains("no page"));

    // A page with no jp2c inside.
    let mut out = SIGNATURE.to_vec();
    out.extend_from_slice(&ftyp(*b"jpm "));
    out.extend_from_slice(&box_bytes(*b"page", &[]));
    let err = decode_as(Format::Jpm, &out).unwrap_err();
    assert!(err.to_string().contains("no jp2c"));
}

#[test]
fn jpm_truncated_prefixes_do_not_panic() {
    let file = build_jpm(&[codestream(10)]);
    for cut in [0, 1, 8, 12, file.len() / 2, file.len().saturating_sub(1)] {
        let prefix = &file[..cut.min(file.len())];
        let _ = decode_as_with(Format::Jpm, prefix, &DecodeOptions { limits: Limits::none(), ..Default::default() });
    }
}

#[test]
fn mj2_decodes_the_first_frame_and_reports_the_rest() {
    let file = build_mj2(&[codestream(10), codestream(20)]);
    assert_eq!(photocraft_codecs::detect(&file), Some(Format::Mj2));

    let img = decode_first(&file, Format::Mj2);
    assert_eq!((img.width(), img.height(), img.layout()), (4, 4, ChannelLayout::Rgb));
    assert_eq!(img.data(), rgb8_image(10).data());
    assert_eq!(img.warnings, vec![DecodeWarning::MoreFrames { total: Some(2) }]);
}

#[test]
fn mj2_single_frame_has_no_more_frames_warning() {
    let file = build_mj2(&[codestream(10)]);
    let img = decode_first(&file, Format::Mj2);
    assert_eq!(img.data(), rgb8_image(10).data());
    assert!(img.warnings.is_empty());
}

#[test]
fn mj2_without_mdat_or_sample_table_is_an_error_not_a_panic() {
    let mut out = SIGNATURE.to_vec();
    out.extend_from_slice(&ftyp(*b"mjp2"));
    out.extend_from_slice(&box_bytes(*b"mdat", &[]));
    let err = decode_as(Format::Mj2, &out).unwrap_err();
    assert!(err.to_string().contains("no moov"), "got: {err}");

    // mdat with data but no moov at all.
    let mut out = SIGNATURE.to_vec();
    out.extend_from_slice(&ftyp(*b"mjp2"));
    out.extend_from_slice(&box_bytes(*b"mdat", &[0xFF, 0x4F, 0xFF, 0x51, 0, 0]));
    let err = decode_as(Format::Mj2, &out).unwrap_err();
    assert!(err.to_string().contains("no moov"));
}

#[test]
fn mj2_truncated_prefixes_do_not_panic() {
    let file = build_mj2(&[codestream(10)]);
    for cut in [0, 1, 8, 12, file.len() / 2, file.len().saturating_sub(1)] {
        let prefix = &file[..cut.min(file.len())];
        let _ = decode_as_with(Format::Mj2, prefix, &DecodeOptions { limits: Limits::none(), ..Default::default() });
    }
}

#[test]
fn jpm_and_mj2_cannot_be_written() {
    let img = rgb8_image(10);
    for format in [Format::Jpm, Format::Mj2] {
        let err = encode(&img, format, &EncodeOptions::default()).unwrap_err();
        assert!(matches!(err, photocraft_codecs::CodecError::Unsupported { .. }), "{format:?}: {err}");
        // The export layer guides the user to JP2/J2K via the fidelity warning.
        let warning = photocraft_codecs::fidelity_warnings_with(&img, format, &EncodeOptions::default());
        assert!(warning.iter().any(|w| w.to_string().contains("JP2 or J2K")), "{format:?}: {warning:?}");
        assert!(!photocraft_codecs::caps(format).write, "{format:?} must not claim to write");
        assert!(photocraft_codecs::caps(format).read, "{format:?} must claim to read");
    }
}
