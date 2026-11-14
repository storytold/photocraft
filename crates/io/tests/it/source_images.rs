//! `FileEncoding::source_images`: a file holding more images than the one a document opens from
//! (an animation, a multi-size icon) is recorded so File › Save doesn't write over it (#5733).

use crate::common::raster;
use photocraft_codecs::web::{GifFrame, encode_gif_animated};
use photocraft_io::import;

/// A 3×2 animated GIF of `frames` identical frames.
fn gif(frames: usize) -> Vec<u8> {
    let frames: Vec<GifFrame> = (0..frames).map(|_| GifFrame { indices: vec![0; 6], palette: vec![[255, 0, 0]], transparent: None, delay_cs: 5 }).collect();
    encode_gif_animated(3, 2, &frames, true).unwrap()
}

/// An icon file whose `entries` directory entries all point at one PNG image.
fn ico(entries: u16) -> Vec<u8> {
    let mut d = photocraft_doc::Document::new("i", photocraft_geom::Size::new(4, 4), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let fmt = d.pixel_format();
    d.layers.push(raster("Layer 0", fmt, d.bounds(), 3, true));
    let one = photocraft_io::export(&d, "i.ico", &Default::default()).unwrap().bytes;
    // ICONDIR (6 bytes) + one 16-byte ICONDIRENTRY whose last 4 bytes are the image offset.
    let (entry, image) = (&one[6..22], &one[22..]);
    let offset = 6 + 16 * u32::from(entries);
    let mut out = vec![0, 0, 1, 0];
    out.extend(entries.to_le_bytes());
    for _ in 0..entries {
        out.extend(&entry[..12]);
        out.extend(offset.to_le_bytes());
    }
    out.extend(image);
    out
}

/// An animated GIF opens with all its frames as layers, so it leaves no image unopened to record.
#[test]
fn an_animated_gif_opens_whole_and_records_no_unopened_images() {
    // An animated GIF opens with every frame as a layer, so no image of the file is left unopened.
    let animated = import("a.gif", &gif(2)).unwrap().document;
    assert_eq!(animated.layers.len(), 2);
    assert_eq!(animated.file_encoding.source_images, None);
    assert_eq!(import("a.gif", &gif(1)).unwrap().document.file_encoding.source_images, None);
}

/// A multi-size icon records its directory count and opens one size; a single-size icon
/// records nothing.
#[test]
fn a_multi_size_icon_records_its_entry_count() {
    let two = import("a.ico", &ico(2)).unwrap().document;
    assert_eq!(two.layers.len(), 1);
    assert_eq!(two.file_encoding.source_images, Some(2));
    assert_eq!(import("a.ico", &ico(1)).unwrap().document.file_encoding.source_images, None);
}
