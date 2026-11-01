use photocraft_codecs::{ChannelLayout, CodecError, EncodeOptions, Format, Image, SampleType, decode, decode_as, encode};

const PAM_GRAY: &[u8] = b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 1\nMAXVAL 255\nENDHDR";
const ENDINGS: &[&[u8]] = &[b"\n", b" \n", b"\t \n", b"\r\n", b" \t\r\n"];

#[test]
fn pam_consumes_the_complete_endhdr_line() {
    for ending in ENDINGS {
        let mut bytes = PAM_GRAY.to_vec();
        bytes.extend_from_slice(ending);
        bytes.push(0x80);
        let img = decode(&bytes).unwrap();
        assert_eq!(img.dimensions(), (1, 1));
        assert_eq!(img.layout(), ChannelLayout::Gray);
        assert_eq!(img.data(), &[0x80], "ENDHDR ending {ending:?}");
    }
}

#[test]
fn pam_preserves_every_possible_first_raster_byte() {
    // In particular, LF, CR, space, tab and '#' must not be skipped as header syntax.
    for ending in ENDINGS {
        for first in 0..=255u8 {
            let mut bytes = PAM_GRAY.to_vec();
            bytes.extend_from_slice(ending);
            bytes.push(first);
            assert_eq!(decode(&bytes).unwrap().data(), &[first], "ENDHDR ending {ending:?}, raster byte {first}");
        }
    }
}

#[test]
fn pam_crlf_header_preserves_u16_samples_and_alpha() {
    for ending in ENDINGS {
        let mut bytes = b"P7\r\nWIDTH 1\r\nHEIGHT 1\r\nDEPTH 2\r\nMAXVAL 65535\r\nTUPLTYPE GRAYSCALE_ALPHA\r\nENDHDR".to_vec();
        bytes.extend_from_slice(ending);
        bytes.extend_from_slice(b" \n#\r");
        let img = decode(&bytes).unwrap();
        assert_eq!(img.layout(), ChannelLayout::GrayA);
        assert_eq!(img.sample_type(), SampleType::U16);
        assert_eq!(img.to_u16_samples().unwrap(), vec![0x200a, 0x230d], "ENDHDR ending {ending:?}");
    }
}

#[test]
fn pam_rejects_unterminated_or_invalid_endhdr_lines() {
    for suffix in [b"".as_slice(), b" ", b"\r", b" \t\r", b" \x80", b" \x80\n\x7f", b"#comment\n\x7f"] {
        let mut bytes = PAM_GRAY.to_vec();
        bytes.extend_from_slice(suffix);
        assert!(matches!(decode_as(Format::Pnm, &bytes), Err(CodecError::Malformed { .. })), "ENDHDR suffix {suffix:?}");
    }
}

#[test]
fn pam_truncation_never_uses_header_whitespace_as_pixels() {
    for ending in ENDINGS {
        let mut bytes = PAM_GRAY.to_vec();
        bytes.extend_from_slice(ending);
        bytes.push(0x80);
        for cut in 0..bytes.len() {
            assert!(decode_as(Format::Pnm, &bytes[..cut]).is_err(), "ENDHDR ending {ending:?}, truncated at {cut}");
        }
    }
}

#[test]
fn binary_pnm_keeps_its_single_whitespace_separator() {
    for separator in *b" \t\r\n" {
        for first in *b" \t\r\n#" {
            for (header, channels) in [(b"P5\n1 1\n255".as_slice(), 1), (b"P6\n1 1\n255", 3)] {
                let mut bytes = header.to_vec();
                bytes.push(separator);
                bytes.extend(std::iter::repeat_n(first, channels));
                assert_eq!(decode(&bytes).unwrap().data(), vec![first; channels]);
            }
            let mut bytes = b"P4\n8 1".to_vec();
            bytes.extend_from_slice(&[separator, first]);
            let expected: Vec<u8> = (0..8).map(|bit| if first & (0x80 >> bit) == 0 { 255 } else { 0 }).collect();
            assert_eq!(decode(&bytes).unwrap().data(), expected);
        }
    }
}

/// Wide integer exports retain network byte order in PGM, PPM and PAM.
#[test]
fn binary_pnm_export_writes_u16_in_network_byte_order() {
    for (layout, tag, tuple) in [
        (ChannelLayout::Gray, "P5", ""),
        (ChannelLayout::Rgb, "P6", ""),
        (ChannelLayout::GrayA, "P7", "GRAYSCALE_ALPHA"),
        (ChannelLayout::Rgba, "P7", "RGB_ALPHA"),
        (ChannelLayout::Cmyk, "P7", "CMYK"),
        (ChannelLayout::CmykA, "P7", "CMYK_ALPHA"),
    ] {
        let samples: Vec<u16> = (0..6 * layout.channels()).map(|i| (i as u16).wrapping_mul(1021)).collect();
        let image = Image::from_u16(3, 2, layout, &samples).unwrap();
        let mut expected = if tag == "P7" {
            format!("P7\nWIDTH 3\nHEIGHT 2\nDEPTH {}\nMAXVAL 65535\nTUPLTYPE {tuple}\nENDHDR\n", layout.channels()).into_bytes()
        } else {
            format!("{tag}\n3 2\n65535\n").into_bytes()
        };
        expected.extend(samples.iter().flat_map(|v| v.to_be_bytes()));
        assert_eq!(encode(&image, Format::Pnm, &EncodeOptions::default()).unwrap(), expected, "{layout:?}");
    }
}

/// PFM reverses rows while preserving native floating-point bits, including NaNs.
#[test]
fn pfm_export_writes_native_float_bits_bottom_up() {
    let bits = [0u32, 0x80000000, 1, 0x3f400000, 0x7f800000, 0xff800000, 0x7fc12345, 0x7f812345];
    for (layout, tag) in [(ChannelLayout::Gray, "Pf"), (ChannelLayout::Rgb, "PF")] {
        let row = 3 * layout.channels();
        let samples: Vec<u32> = (0..2 * row).map(|i| bits[i % bits.len()]).collect();
        let raw = samples.iter().flat_map(|v| v.to_ne_bytes()).collect();
        let image = Image::from_raw(3, 2, layout, SampleType::F32, raw).unwrap();
        let mut expected = format!("{tag}\n3 2\n-1.0\n").into_bytes();
        for row in samples.chunks_exact(row).rev() {
            expected.extend(row.iter().flat_map(|v| v.to_le_bytes()));
        }
        assert_eq!(encode(&image, Format::Pnm, &EncodeOptions::default()).unwrap(), expected, "{layout:?}");
    }
}
