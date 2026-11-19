//! The colour an encoder leaves under a clear pixel, through a save and a reopen.

use photocraft_io::*;

/// A PNG whose clear pixels hold white, as an encoder writing unassociated alpha leaves them:
/// the artwork is a 4x4 black patch on a 8x6 white-but-transparent canvas.
fn white_under_clear_png() -> Vec<u8> {
    use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image};
    let (w, h) = (8u32, 6u32);
    let mut px = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let painted = (2..6).contains(&x) && (1..5).contains(&y);
            px.extend_from_slice(if painted { &[0u8, 0, 0, 255] } else { &[255, 255, 255, 0] });
        }
    }
    let img = Image::from_u8(w, h, ChannelLayout::Rgba, px).expect("image");
    photocraft_codecs::encode(&img, Format::Png, &EncodeOptions::default()).expect("encode")
}

fn opened() -> photocraft_doc::Document {
    import("clear.png", &white_under_clear_png()).expect("import png").document
}

#[test]
fn a_psd_round_trip_keeps_the_colour_under_the_clear_pixels() {
    // Photoshop's layer record stores the pixels the layer holds, clear or not: it saves the
    // canvas-sized rect it opened from the PNG, with the white still under the transparent
    // pixels. Trimming that rect to the visible artwork would drop stored data on save, so the
    // exporter writes the stored bounds (#9824).
    let doc = opened();
    assert_eq!(doc.layers[0].surface().expect("raster").content_bounds(), photocraft_geom::Rect::new(2, 1, 6, 5));

    let psd = export(&doc, "clear.psd", &ExportOptions::default()).expect("export psd");
    let back = import("clear.psd", &psd.bytes).expect("reimport psd").document;
    let s = back.layers[0].surface().expect("raster layer");

    assert_eq!(s.content_bounds(), photocraft_geom::Rect::new(2, 1, 6, 5), "the artwork's box survives the save");
    assert_eq!(s.stored_bounds(), photocraft_geom::Rect::new(0, 0, 8, 6), "the layer record kept every stored pixel");
    assert_eq!(s.pixel(0, 0), vec![1.0, 1.0, 1.0, 0.0], "the white under the clear pixels is kept");
    assert_eq!(s.pixel(3, 2), vec![0.0, 0.0, 0.0, 1.0], "the artwork is unchanged");
}

#[test]
fn a_flattened_png_keeps_the_visible_pixels_and_the_artworks_box() {
    // A flat PNG has no layer records: the composite is what is saved, so what the encoder left
    // under the clear pixels is flattened away rather than preserved per layer. The visible
    // pixels and the box a reopened file measures are what a save must not change.
    let doc = opened();
    let png = export(&doc, "clear-out.png", &ExportOptions::default()).expect("export png");
    let back = import("clear-out.png", &png.bytes).expect("reimport png").document;
    let s = back.layers[0].surface().expect("raster layer");

    assert_eq!(s.content_bounds(), photocraft_geom::Rect::new(2, 1, 6, 5), "the artwork's box");
    assert_eq!(s.pixel(3, 2), vec![0.0, 0.0, 0.0, 1.0], "the artwork is unchanged");
    assert_eq!(s.pixel(0, 0)[3], 0.0, "the outside stays clear");
    eprintln!("PNG round trip: pixel(0,0) = {:?}, pixel(7,5) = {:?}", s.pixel(0, 0), s.pixel(7, 5));
}
