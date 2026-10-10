//! JPEG 2000 document import/export, including extension-selected raw codestreams.

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType, decode, encode};
use photocraft_io::{ExportOptions, export, import};

fn image() -> Image {
    Image::from_u16(
        3,
        2,
        ChannelLayout::Rgba,
        &[0, 1, 65535, 0, 4096, 32768, 12345, 32768, 65535, 45678, 8, 65535, 1, 2, 3, 65535, 32768, 32768, 32768, 16384, 42, 999, 60000, 65535],
    )
    .unwrap()
}

#[test]
fn jpeg2000_suffix_selects_container_even_when_options_disagree() {
    let img = image();
    let source = encode(&img, Format::Jpeg2000, &EncodeOptions::default()).unwrap();
    // Magic takes precedence over a misleading file name.
    let doc = import("renamed.png", &source).unwrap().document;
    assert_eq!(doc.depth, photocraft_color::SampleType::U16);
    assert!(!doc.layers[0].locks.transparency);
    for (ext, raw) in [("JP2", false), ("j2k", true), ("J2C", true)] {
        let opts = ExportOptions { encode: EncodeOptions { jpeg2000_codestream: !raw, ..Default::default() }, ..Default::default() };
        let out = export(&doc, &format!("copy.{ext}"), &opts).unwrap();
        assert_eq!(out.bytes.starts_with(&[0xFF, 0x4F, 0xFF, 0x51]), raw, "{ext}");
        assert_eq!(out.bytes.starts_with(b"\0\0\0\x0cjP  \r\n\x87\n"), !raw, "{ext}");
        let back = decode(&out.bytes).unwrap();
        assert_eq!(back.sample_type(), SampleType::U16);
        assert_eq!(back.layout(), ChannelLayout::Rgba);
        assert_eq!(back.data(), img.data(), "{ext} preserves native pixels and transparency");
    }
}

#[test]
fn jpeg2000_jp2_keeps_profile_and_resolution_but_raw_converts_to_srgb() {
    let img = Image::from_u16(2, 1, ChannelLayout::Rgb, &[32768, 16384, 8192, 0, 65535, 32768]).unwrap();
    let bytes = encode(&img, Format::Jpeg2000, &EncodeOptions::default()).unwrap();
    let mut doc = import("linear.jp2", &bytes).unwrap().document;
    doc.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
    doc.resolution_dpi = 300.0;
    let jp2 = export(&doc, "copy.jp2", &ExportOptions::default()).unwrap();
    let back = import("copy.jp2", &jp2.bytes).unwrap().document;
    assert_eq!(back.icc_profile, doc.icc_profile);
    assert!((back.resolution_dpi - 300.0).abs() < 0.01);
    assert_eq!(decode(&jp2.bytes).unwrap().data(), img.data());
    let raw = export(&doc, "copy.j2k", &ExportOptions::default()).unwrap();
    assert!(raw.warnings.iter().any(|w| w.contains("converted to sRGB")), "{:?}", raw.warnings);
    let back = decode(&raw.bytes).unwrap();
    assert!(back.icc.is_none());
    assert!(back.meta.dpi.is_none());
    for (linear, srgb) in img.to_normalized().iter().zip(back.to_normalized()) {
        let expected = photocraft_color::convert::linear_to_srgb(*linear);
        assert!((srgb - expected).abs() < 1e-4, "{linear} -> {srgb}, expected {expected}");
    }
}
