//! AVIF is a read/write format in the default build: File › Open and Batch pick it up, and Save As
//! writes it through the same flat-image pipeline as the other codecs.

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, decode, detect, encode};
use photocraft_doc::ColorMode;
use photocraft_io::{ExportOptions, export, import};

fn seed_png() -> Vec<u8> {
    let pixels: Vec<u8> = (0..64 * 32)
        .flat_map(|i| {
            let alpha = if i % 8 == 0 { 0 } else { 255 };
            [i as u8, (i * 3) as u8, (i * 7) as u8, alpha]
        })
        .collect();
    let image = photocraft_codecs::Image::from_u8(64, 32, ChannelLayout::Rgba, pixels).unwrap();
    encode(&image, Format::Png, &EncodeOptions::default()).unwrap()
}

fn avif_file() -> Vec<u8> {
    let doc = import("seed.png", &seed_png()).unwrap().document;
    export(&doc, "photo.avif", &ExportOptions::default()).unwrap().bytes
}

/// A flat AVIF written by Save As opens again with the same pixels and mode; the codec is lossy, so
/// compare geometry and alpha coverage rather than exact samples.
#[test]
fn avif_round_trips_through_import_and_export() {
    assert!(Format::Avif.caps().read && Format::Avif.caps().write, "this build reads and writes AVIF");
    let seed = decode(&seed_png()).unwrap();
    assert!(seed.layout().has_alpha(), "the seed PNG carries alpha");
    assert!(seed.to_rgba8().chunks_exact(4).any(|p| p[3] < 32), "the seed PNG has transparent pixels");

    let bytes = avif_file();
    assert_eq!(detect(&bytes), Some(Format::Avif), "the AVIF signature is recognized");

    let decoded = decode(&bytes).unwrap();
    assert_eq!(decoded.dimensions(), (64, 32));
    assert!(decoded.layout().has_alpha(), "the transparent source keeps its alpha");
    let rgba = decoded.to_rgba8();
    let alphas: Vec<u8> = rgba.chunks_exact(4).map(|pixel| pixel[3]).collect();
    assert!(alphas.iter().any(|alpha| *alpha < 32), "some pixels stay transparent");
    assert!(alphas.iter().any(|alpha| *alpha > 224), "some pixels stay opaque");
    assert!(rgba.iter().any(|sample| *sample > 200), "the decoded pixels are not blank");

    let doc = import("photo.avif", &bytes).unwrap().document;
    assert_eq!((doc.size.width, doc.size.height), (64, 32));
    // AVIF carries no Photoshop colour mode; transparency is a channel, not a mode.
    assert_eq!(doc.mode, ColorMode::Rgb);
}

/// A truncated AVIF is refused by the importer, not decoded into a partial document.
#[test]
fn truncated_avif_is_an_error() {
    let bytes = avif_file();
    for cut in [0, 12, 40, bytes.len() / 2] {
        assert!(import("cut.avif", &bytes[..cut]).is_err(), "cut at {cut}");
    }
}
