//! Magic-number detection, extensions, and the capability table
//! (including the symmetric read/write guarantee).

mod common;
use common::*;
use photocraft_codecs::*;

// ---------------------------------------------------------------------------
// Symmetric guarantee
// ---------------------------------------------------------------------------

#[test]
fn every_enabled_format_is_symmetric_or_whitelisted() {
    for f in Format::ALL {
        let c = caps(f);
        if !(c.read || c.write) {
            continue;
        }
        if c.read && c.write {
            continue;
        }
        let exc = ASYMMETRIC_EXCEPTIONS.iter().find(|(g, _)| *g == f);
        let (_, reason) = exc.unwrap_or_else(|| panic!("{f:?} is enabled but not symmetric (read={}, write={})", c.read, c.write));
        assert!(reason.len() > 20, "{f:?} exception needs a real reason");
    }
}

#[test]
fn exceptions_are_documented_and_unique() {
    for (i, (f, reason)) in ASYMMETRIC_EXCEPTIONS.iter().enumerate() {
        assert!(!reason.is_empty());
        assert!(ASYMMETRIC_EXCEPTIONS[i + 1..].iter().all(|(g, _)| g != f));
    }
}

#[test]
fn never_write_only_without_exception() {
    // Stronger form: no format may be writable-but-unreadable unless listed.
    for f in writable_formats() {
        if !caps(f).read {
            assert!(ASYMMETRIC_EXCEPTIONS.iter().any(|(g, _)| *g == f), "{f:?}");
        }
    }
}

#[test]
fn default_build_formats_all_symmetric() {
    let expected = [
        Format::Png,
        Format::Jpeg,
        Format::Tiff,
        Format::WebP,
        Format::Gif,
        Format::Bmp,
        Format::Tga,
        Format::Ico,
        Format::Pnm,
        Format::Qoi,
        Format::OpenExr,
        Format::Hdr,
        Format::Fits,
        Format::Dpx,
        Format::Jp2,
        Format::J2k,
        Format::Jpx,
        Format::Jpf,
    ];
    for f in expected {
        assert!(caps(f).read && caps(f).write, "{f:?}");
    }
}

#[test]
fn avif_never_readable() {
    assert!(!caps(Format::Avif).read);
    assert_eq!(caps(Format::Avif).write, cfg!(feature = "avif"));
    assert!(matches!(decode_as(Format::Avif, b"\0\0\0\x1cftypavif"), Err(CodecError::Unsupported { .. })));
}

#[test]
fn heif_is_detected_always_and_readable_only_with_the_feature() {
    let header = b"\0\0\0\x18ftypheic\0\0\0\0mif1heic";
    assert_eq!(detect(header), Some(Format::Heif));
    assert_eq!(from_extension("IMG_0001.HEIC"), Some(Format::Heif));
    assert_eq!(caps(Format::Heif).read, cfg!(feature = "heif"));
    assert!(!caps(Format::Heif).write);
    assert!(ASYMMETRIC_EXCEPTIONS.iter().any(|(f, _)| *f == Format::Heif));
    // Either way a bare header is an error, never a panic.
    assert!(decode_as(Format::Heif, header).is_err());
}

#[test]
fn detect_heif_brands() {
    assert_eq!(detect(b"\0\0\0\x18ftypheic\0\0\0\0mif1heic"), Some(Format::Heif));
    assert_eq!(detect(b"\0\0\0\x18ftypmif1\0\0\0\0mif1heic"), Some(Format::Heif), "generic major brand");
    assert_eq!(detect(b"\0\0\0\x18ftypheix\0\0\0\0mif1heix"), Some(Format::Heif), "10-bit");
    assert_eq!(detect(b"\0\0\0\x18ftypmsf1\0\0\0\0msf1hevc"), Some(Format::Heif), "image sequence");
    assert_eq!(detect(b"\0\0\0\x18ftypmif1\0\0\0\0mif1avif"), Some(Format::Avif), "AVIF wins over the generic brand");
    assert_eq!(detect(b"\0\0\0\x18ftypisom\0\0\0\0isommp41"), None, "mp4 is not HEIF");
    assert_eq!(detect(b"\0\0\0\x14ftypqt  \0\0\0\0qt  "), None, "QuickTime is not HEIF");
}

#[test]
fn caps_are_internally_consistent() {
    for f in Format::ALL {
        let c = caps(f);
        assert!(!c.depths.is_empty() && !c.layouts.is_empty(), "{f:?}");
        assert_eq!(c.alpha, c.layouts.iter().any(|l| l.has_alpha()), "{f:?} alpha flag vs layouts");
        assert!(!f.extensions().is_empty() && !f.name().is_empty(), "{f:?}");
        // Every still-image format declares an `image/` type. The one container that is not a
        // still image declares the registered `video/` type instead.
        assert!(f.mime_type().starts_with("image/") || (f == Format::Mj2 && f.mime_type() == "video/mjp2"), "{f:?}: {}", f.mime_type());
    }
}

#[test]
fn caps_function_matches_method() {
    for f in Format::ALL {
        assert_eq!(caps(f), f.caps());
    }
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

#[test]
fn detect_encoded_output_of_every_format() {
    for f in writable_formats() {
        let img = test_image(ChannelLayout::Rgba, SampleType::U8);
        let bytes = encode(&img, f, &EncodeOptions::default()).unwrap();
        // The Part 2 and Part 6 wrappers are written as the Part 1 container, so their bytes
        // read back as JP2; a raw codestream keeps its own marker.
        let expected = if matches!(f, Format::Jpx | Format::Jpf | Format::Jpm) { Format::Jp2 } else { f };
        assert_eq!(detect(&bytes), Some(expected), "{f:?}");
    }
}

/// JPEG 2000 identifies itself from its own bytes, and its extensions each resolve back to the
/// format that handles them — including the two containers we only recognise.
#[test]
fn jpeg2000_extensions_and_magic_agree() {
    for (ext, expected, family) in [
        ("jp2", Format::Jp2, true),
        ("j2k", Format::J2k, true),
        ("jpx", Format::Jpx, true),
        ("jpf", Format::Jpf, true),
        ("jpm", Format::Jpm, false),
        ("mj2", Format::Mj2, false),
    ] {
        assert_eq!(from_extension(ext), Some(expected), ".{ext}");
        assert_eq!(from_extension(&format!("photo.{ext}")), Some(expected), "photo.{ext}");
        assert_eq!(from_extension(&ext.to_uppercase()), Some(expected), ".{ext} uppercase");
        assert_eq!(is_jpeg2000(expected), family, ".{ext} is not in the right family");
    }
    assert_eq!(from_extension("j2c"), Some(Format::J2k), ".j2c is the codestream alias");
    // The Part 1 signature box opens the file, then `ftyp` names the container.
    let signature = [0x00, 0x00, 0x00, 0x0C, b'j', b'P', b' ', b' ', 0x0D, 0x0A, 0x87, 0x0A];
    let file = |brand: &[u8; 4]| {
        let mut ftyp = 0x14u32.to_be_bytes().to_vec();
        ftyp.extend_from_slice(b"ftyp");
        ftyp.extend_from_slice(brand);
        ftyp.extend_from_slice(&0u32.to_be_bytes());
        ftyp.extend_from_slice(brand);
        signature.iter().chain(&ftyp).copied().collect::<Vec<u8>>()
    };
    assert_eq!(detect(&signature), Some(Format::Jp2), "a signature box with no ftyp is a Part 1 file");
    for (brand, want) in [(b"jp2 ", Format::Jp2), (b"jpx ", Format::Jp2), (b"jpf ", Format::Jp2), (b"jpm ", Format::Jpm), (b"mjp2", Format::Mj2)] {
        assert_eq!(detect(&file(brand)), Some(want), "{brand:?}");
    }
    assert_eq!(detect(&[0xFF, 0x4F, 0xFF, 0x51]), Some(Format::J2k));
}

#[test]
fn detect_magic_png() {
    assert_eq!(detect(b"\x89PNG\r\n\x1a\n\0\0"), Some(Format::Png));
}
#[test]
fn detect_magic_jpeg() {
    assert_eq!(detect(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(Format::Jpeg));
    assert_eq!(detect(&[0xFF, 0xD8]), None);
}
#[test]
fn detect_magic_tiff_both_endians_and_bigtiff() {
    assert_eq!(detect(b"II*\0\x08\0\0\0"), Some(Format::Tiff));
    assert_eq!(detect(b"MM\0*\0\0\0\x08"), Some(Format::Tiff));
    assert_eq!(detect(b"II+\0\x08\0\0\0"), Some(Format::Tiff));
    assert_eq!(detect(b"MM\0+\0\x08\0\0"), Some(Format::Tiff));
}
#[test]
fn detect_magic_webp() {
    assert_eq!(detect(b"RIFF\x10\0\0\0WEBPVP8L"), Some(Format::WebP));
    assert_eq!(detect(b"RIFF\x10\0\0\0WAVEfmt "), None);
}
#[test]
fn detect_magic_gif() {
    assert_eq!(detect(b"GIF87a\x01\0"), Some(Format::Gif));
    assert_eq!(detect(b"GIF89a\x01\0"), Some(Format::Gif));
    assert_eq!(detect(b"GIF90a\x01\0"), None);
}
#[test]
fn detect_magic_bmp() {
    let mut b = b"BM".to_vec();
    b.resize(54, 0);
    assert_eq!(detect(&b), Some(Format::Bmp));
    assert_eq!(detect(b"BM"), None);
}
#[test]
fn detect_magic_ico() {
    assert_eq!(detect(&[0, 0, 1, 0, 1, 0, 16, 16]), Some(Format::Ico));
    assert_eq!(detect(&[0, 0, 1, 0, 0, 0]), None, "zero images");
}
#[test]
fn detect_magic_pnm_family() {
    for m in [&b"P1\n"[..], b"P2 ", b"P3\t", b"P4\n", b"P5\r", b"P6\n", b"P7\n", b"PF\n", b"Pf\n"] {
        assert_eq!(detect(m), Some(Format::Pnm), "{:?}", std::str::from_utf8(m));
    }
    assert_eq!(detect(b"P8\n"), None);
    assert_eq!(detect(b"P6x"), None);
}
#[test]
fn detect_magic_qoi() {
    assert_eq!(detect(b"qoif\0\0\0\x01\0\0\0\x01\x03\0"), Some(Format::Qoi));
}
#[test]
fn detect_magic_exr() {
    assert_eq!(detect(&[0x76, 0x2F, 0x31, 0x01, 2, 0, 0, 0]), Some(Format::OpenExr));
}
#[test]
fn detect_magic_hdr() {
    assert_eq!(detect(b"#?RADIANCE\n"), Some(Format::Hdr));
    assert_eq!(detect(b"#?RGBE\n"), Some(Format::Hdr));
}
#[test]
fn detect_magic_cineon() {
    let mut b = vec![0u8; 208];
    b[0..4].copy_from_slice(&[0x80, 0x2A, 0x5F, 0xD7]);
    assert_eq!(detect(&b), Some(Format::Cineon));
    assert_eq!(from_extension("scan.CIN"), Some(Format::Cineon));
    assert!(caps(Format::Cineon).read && !caps(Format::Cineon).write, "Cineon is read-only");
    assert!(ASYMMETRIC_EXCEPTIONS.iter().any(|(f, _)| *f == Format::Cineon));
    // A bare magic is an error, never a panic.
    assert!(decode_as(Format::Cineon, &b).is_err());
}
#[test]
fn detect_magic_avif() {
    assert_eq!(detect(b"\0\0\0\x1cftypavif\0\0\0\0avifmif1miaf"), Some(Format::Avif));
    assert_eq!(detect(b"\0\0\0\x1cftypavis\0\0\0\0avismsf1miaf"), Some(Format::Avif));
    assert_eq!(detect(b"\0\0\0\x18ftypmif1\0\0\0\0mif1avif"), Some(Format::Avif), "compatible brand");
    assert_eq!(detect(b"\0\0\0\x18ftypisom\0\0\0\0isommp41"), None, "mp4 is not avif");
}
#[test]
fn detect_tga_heuristic() {
    // Uncompressed true-colour 2x2, 24 bpp.
    let mut b = vec![0u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 24, 0];
    b.extend_from_slice(&[0; 12]);
    assert_eq!(detect(&b), Some(Format::Tga));
    let mut bad = b.clone();
    bad[2] = 7; // invalid image type
    assert_eq!(detect(&bad), None);
    let mut bad = b.clone();
    bad[16] = 13; // invalid depth
    assert_eq!(detect(&bad), None);
}
#[test]
fn detect_negatives() {
    assert_eq!(detect(b""), None);
    assert_eq!(detect(b"x"), None);
    assert_eq!(detect(b"hello world, this is not an image"), None);
    assert_eq!(detect(b"%PDF-1.7\n"), None);
    assert_eq!(detect(b"PK\x03\x04"), None);
    assert_eq!(detect(&[0u8; 64]), None);
}
#[test]
fn detect_random_bytes_rarely_matches_tga() {
    let mut rng = Rng::new(5);
    let hits = (0..2000).filter(|_| detect(&rng.bytes(64)) == Some(Format::Tga)).count();
    assert!(hits < 10, "{hits} false TGA positives");
}
#[test]
fn decode_unknown_is_error() {
    assert!(matches!(decode(b"garbage garbage"), Err(CodecError::UnknownFormat)));
}

// ---------------------------------------------------------------------------
// Extensions
// ---------------------------------------------------------------------------

#[test]
fn extensions_map_back() {
    for f in Format::ALL {
        for e in f.extensions() {
            assert_eq!(from_extension(e), Some(f), "{e}");
            assert_eq!(from_extension(&e.to_uppercase()), Some(f));
            assert_eq!(from_extension(&format!(".{e}")), Some(f));
            assert_eq!(from_extension(&format!("dir/file.name.{e}")), Some(f));
        }
    }
}
#[test]
fn extension_specific_cases() {
    assert_eq!(from_extension("JPG"), Some(Format::Jpeg));
    assert_eq!(from_extension("tif"), Some(Format::Tiff));
    assert_eq!(from_extension("pfm"), Some(Format::Pnm));
    assert_eq!(from_extension("pam"), Some(Format::Pnm));
    assert_eq!(from_extension("FIT"), Some(Format::Fits));
    assert_eq!(from_extension("m31.fits"), Some(Format::Fits));
    assert_eq!(from_extension("exr"), Some(Format::OpenExr));
    assert_eq!(from_extension("psd"), None);
    assert_eq!(from_extension(""), None);
    assert_eq!(from_extension("png.bak"), None);
}
#[test]
fn extensions_unique_across_formats() {
    let mut all: Vec<&str> = Format::ALL.iter().flat_map(|f| f.extensions().iter().copied()).collect();
    let n = all.len();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), n);
}
