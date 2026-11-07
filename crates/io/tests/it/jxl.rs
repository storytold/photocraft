//! JPEG XL files (#2789) open as documents: through the same `import` File › Open uses, upright,
//! at their depth, with their EXIF and XMP, and an export never turns them a second time.
//! Features `corpus` and `jxl` (`cargo xtask test-corpus -p io`); the files come from `corpus/jxl`
//! (`cargo xtask corpus --jxl`), the same JPEG XL oracles `photocraft-codecs` tests against.

#![cfg(all(feature = "corpus", feature = "jxl"))]

use photocraft_codecs::{Format, decode, detect, exif_orientation};
use photocraft_doc::{ColorMode, SampleType};
use photocraft_io::{ExportOptions, export, import};

use std::sync::LazyLock;

/// A file of `corpus/jxl/`.
fn corpus(rel: &str) -> Vec<u8> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/jxl").join(rel);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}: run `cargo xtask corpus --jxl`", p.display()))
}

static FILES: LazyLock<[Vec<u8>; 7]> = LazyLock::new(|| {
    ["rgb8-lossless.jxl", "rgba8-lossless.jxl", "gray8-lossless.jxl", "rgb16-lossless.jxl", "rgb-f32-lossless.jxl", "exif-xmp-orient6.jxl", "animated.jxl"]
        .map(corpus)
});

fn rgb8() -> &'static [u8] {
    &FILES[0]
}
fn rgba8() -> &'static [u8] {
    &FILES[1]
}
fn gray8() -> &'static [u8] {
    &FILES[2]
}
fn rgb16() -> &'static [u8] {
    &FILES[3]
}
fn rgb_f32() -> &'static [u8] {
    &FILES[4]
}
fn exif_xmp_orient6() -> &'static [u8] {
    &FILES[5]
}
fn animated() -> &'static [u8] {
    &FILES[6]
}

#[test]
fn jxl_opens_as_an_8_bit_rgb_document() {
    let r = import("IMG_0001.JXL", rgb8()).unwrap();
    let d = &r.document;
    assert_eq!((d.size.width, d.size.height), (12, 9));
    assert_eq!((d.mode, d.depth), (ColorMode::Rgb, SampleType::U8));
    assert_eq!(d.layers.len(), 1);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert!(d.layers[0].locks.transparency, "an opaque image opens as a Background layer");
    assert!(!import("x.jxl", rgba8()).unwrap().document.layers[0].locks.transparency, "an image with alpha opens as an unlocked layer");
}

#[test]
fn depth_and_colour_mode_follow_the_file() {
    let d = import("g.jxl", gray8()).unwrap().document;
    assert_eq!((d.mode, d.depth), (ColorMode::Grayscale, SampleType::U8));
    let d = import("16.jxl", rgb16()).unwrap().document;
    assert_eq!((d.mode, d.depth), (ColorMode::Rgb, SampleType::U16));
    let d = import("f.jxl", rgb_f32()).unwrap().document;
    assert_eq!((d.mode, d.depth), (ColorMode::Rgb, SampleType::F32));
}

#[test]
fn metadata_reaches_the_document_and_exports_upright() {
    let r = import("photo.jxl", exif_xmp_orient6()).unwrap();
    let d = &r.document;
    assert_eq!((d.size.width, d.size.height), (9, 12), "orientation 6 applied once");
    assert!(d.metadata.exif.is_some() && d.metadata.xmp.as_deref().is_some_and(|x| x.contains("x:xmpmeta")));
    // Save the picture as a JPEG, which keeps the EXIF with Orientation 1 (a JPEG XL save writes no
    // metadata boxes).
    let out = export(d, "photo.jpg", &ExportOptions::default()).unwrap();
    assert_eq!(detect(&out.bytes), Some(Format::Jpeg));
    let back = decode(&out.bytes).unwrap();
    assert_eq!(back.dimensions(), (9, 12));
    assert_eq!(exif_orientation(back.meta.exif.as_deref().unwrap()), 1);
}

#[test]
fn an_animation_opens_with_a_warning() {
    let r = import("a.jxl", animated()).unwrap();
    assert_eq!((r.document.size.width, r.document.size.height), (12, 9));
    assert!(r.warnings.iter().any(|w| w.to_string().contains("first of 3 frames")), "{:?}", r.warnings);
}

#[test]
fn jxl_saves_losslessly_and_reopens() {
    let d = import("x.jxl", rgb8()).unwrap().document;
    let out = export(&d, "x.jxl", &ExportOptions::default()).unwrap();
    assert_eq!(detect(&out.bytes), Some(Format::Jxl));
    let back = import("x.jxl", &out.bytes).unwrap().document;
    assert_eq!((back.size, back.mode, back.depth), (d.size, d.mode, d.depth));
    assert_eq!(decode(&out.bytes).unwrap().data(), decode(rgb8()).unwrap().data(), "lossless round trip");
}

#[test]
fn broken_jxl_is_an_error_not_a_crash() {
    for bytes in [&b""[..], &rgb8()[..rgb8().len() / 2], &[0xFF, 0x0A, 0xFF, 0xFF, 0xFF, 0xFF][..]] {
        assert!(import("x.jxl", bytes).is_err());
    }
}
