//! JPEG XL files (#2789) open as documents: through the same `import` File › Open uses, upright,
//! at their depth, with their EXIF and XMP, and an export never turns them a second time.
//! Feature `jxl`; the fixtures are `photocraft-codecs`' (`scripts/jxl_fixtures.py`).

#![cfg(feature = "jxl")]

use photocraft_codecs::{Format, decode, detect, exif_orientation};
use photocraft_doc::{ColorMode, SampleType};
use photocraft_io::{ExportOptions, export, import};

const RGB8: &[u8] = include_bytes!("../../codecs/tests/fixtures/jxl/rgb8-lossless.jxl");
const RGBA8: &[u8] = include_bytes!("../../codecs/tests/fixtures/jxl/rgba8-lossless.jxl");
const GRAY8: &[u8] = include_bytes!("../../codecs/tests/fixtures/jxl/gray8-lossless.jxl");
const RGB16: &[u8] = include_bytes!("../../codecs/tests/fixtures/jxl/rgb16-lossless.jxl");
const RGB_F32: &[u8] = include_bytes!("../../codecs/tests/fixtures/jxl/rgb-f32-lossless.jxl");
const EXIF_XMP_ORIENT6: &[u8] = include_bytes!("../../codecs/tests/fixtures/jxl/exif-xmp-orient6.jxl");
const ANIMATED: &[u8] = include_bytes!("../../codecs/tests/fixtures/jxl/animated.jxl");

#[test]
fn jxl_opens_as_an_8_bit_rgb_document() {
    let r = import("IMG_0001.JXL", RGB8).unwrap();
    let d = &r.document;
    assert_eq!((d.size.width, d.size.height), (12, 9));
    assert_eq!((d.mode, d.depth), (ColorMode::Rgb, SampleType::U8));
    assert_eq!(d.layers.len(), 1);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert!(d.layers[0].locks.transparency, "an opaque image opens as a Background layer");
    assert!(!import("x.jxl", RGBA8).unwrap().document.layers[0].locks.transparency, "an image with alpha opens as an unlocked layer");
}

#[test]
fn depth_and_colour_mode_follow_the_file() {
    let d = import("g.jxl", GRAY8).unwrap().document;
    assert_eq!((d.mode, d.depth), (ColorMode::Grayscale, SampleType::U8));
    let d = import("16.jxl", RGB16).unwrap().document;
    assert_eq!((d.mode, d.depth), (ColorMode::Rgb, SampleType::U16));
    let d = import("f.jxl", RGB_F32).unwrap().document;
    assert_eq!((d.mode, d.depth), (ColorMode::Rgb, SampleType::F32));
}

#[test]
fn metadata_reaches_the_document_and_exports_upright() {
    let r = import("photo.jxl", EXIF_XMP_ORIENT6).unwrap();
    let d = &r.document;
    assert_eq!((d.size.width, d.size.height), (9, 12), "orientation 6 applied once");
    assert!(d.metadata.exif.is_some() && d.metadata.xmp.as_deref().is_some_and(|x| x.contains("x:xmpmeta")));
    // JPEG XL is read-only: save the picture as a JPEG, which keeps the EXIF with Orientation 1.
    let out = export(d, "photo.jpg", &ExportOptions::default()).unwrap();
    assert_eq!(detect(&out.bytes), Some(Format::Jpeg));
    let back = decode(&out.bytes).unwrap();
    assert_eq!(back.dimensions(), (9, 12));
    assert_eq!(exif_orientation(back.meta.exif.as_deref().unwrap()), 1);
}

#[test]
fn an_animation_opens_with_a_warning() {
    let r = import("a.jxl", ANIMATED).unwrap();
    assert_eq!((r.document.size.width, r.document.size.height), (12, 9));
    assert!(r.warnings.iter().any(|w| w.to_string().contains("first of 3 frames")), "{:?}", r.warnings);
}

#[test]
fn jxl_cannot_be_written_and_says_so() {
    let d = import("x.jxl", RGB8).unwrap().document;
    assert!(export(&d, "x.jxl", &ExportOptions::default()).is_err());
}

#[test]
fn broken_jxl_is_an_error_not_a_crash() {
    for bytes in [&b""[..], &RGB8[..RGB8.len() / 2], &[0xFF, 0x0A, 0xFF, 0xFF, 0xFF, 0xFF][..]] {
        assert!(import("x.jxl", bytes).is_err());
    }
}
