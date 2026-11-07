//! Indexed Color and Duotone documents in flat exports.

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{ColorTable, Document, Duotone, DuotoneInk, Size};

fn doc(mode: ColorMode) -> Document {
    let mut d = Document::with_background("m", Size::new(4, 2), ColorMode::Rgb, SampleType::U8, Color::rgba(1.0, 0.0, 0.0, 1.0));
    d.mode = mode;
    d
}

#[test]
fn indexed_png_is_palette_png() {
    let mut d = doc(ColorMode::Indexed);
    d.color_table = Some(ColorTable { colors: vec![[0, 0, 0], [255, 0, 0]], transparent: None });
    let r = photocraft_io::export(&d, "x.png", &Default::default()).unwrap();
    // IHDR colour type 3 = indexed; a PLTE chunk follows.
    assert_eq!(r.bytes[25], 3);
    assert!(r.bytes.windows(4).any(|w| w == b"PLTE"));
    let back = photocraft_io::import("x.png", &r.bytes).unwrap().document;
    let px = back.layers[0].surface().unwrap().rgba(1, 1);
    assert_eq!(&px[..3], &[1.0, 0.0, 0.0]);
    // Other formats keep the expanded pixels.
    let j = photocraft_io::export(&d, "x.tif", &Default::default()).unwrap();
    assert!(!j.bytes.is_empty());
}

#[test]
fn indexed_png_keeps_duplicate_palette_colors_opaque() {
    // PLTE entries may have identical RGB but different tRNS alpha. RGB-only lookup used to
    // choose the first red entry for both pixels, making the opaque one disappear on save.
    let palette = [[255, 0, 0], [0, 255, 0], [255, 0, 0], [0, 0, 255]];
    for transparent in [0, 2] {
        let png = photocraft_codecs::encode_png_indexed(4, 1, &[0, 1, 2, 3], &palette, Some(transparent), None, None).unwrap();
        let d = photocraft_io::import("in.png", &png).unwrap().document;
        assert_eq!(d.mode, ColorMode::Indexed);
        let before = d.color_table.clone();
        let r = photocraft_io::export(&d, "out.png", &Default::default()).unwrap();
        let table = photocraft_codecs::png_palette(&r.bytes).unwrap().unwrap();
        assert_eq!(table.colors, palette, "palette order must not change");
        assert_eq!(table.transparent, Some(transparent));
        assert_eq!(photocraft_codecs::decode(&r.bytes).unwrap().data(), photocraft_codecs::decode(&png).unwrap().data());
        let back = photocraft_io::import("out.png", &r.bytes).unwrap().document;
        assert_eq!(back.color_table, before);
        assert_eq!(d.color_table, before, "export must not edit the document's table");
        for x in 0..4 {
            assert_eq!(back.layers[0].surface().unwrap().rgba(x, 0)[3], if x == i32::from(transparent) { 0.0 } else { 1.0 });
        }
    }
}

#[test]
fn indexed_png_uses_the_nearest_opaque_color() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for transparent in [0, 1, 2] {
            let mut palette = vec![[0, 0, 255], [255, 128, 0]];
            palette.insert(usize::from(transparent), [255, 0, 0]);
            let mut d = Document::with_background("m", Size::new(1, 1), ColorMode::Rgb, depth, Color::rgba(1.0, 0.0, 0.0, 1.0));
            d.mode = ColorMode::Indexed;
            d.color_table = Some(ColorTable { colors: palette.clone(), transparent: Some(transparent) });
            let r = photocraft_io::export(&d, "out.png", &Default::default()).unwrap();
            let image = photocraft_codecs::decode(&r.bytes).unwrap();
            assert_eq!(image.data(), &[255, 128, 0, 255], "{depth:?}, transparent = {transparent}");
            let table = photocraft_codecs::png_palette(&r.bytes).unwrap().unwrap();
            assert_eq!(table.colors, palette);
            assert_eq!(table.transparent, Some(transparent));
        }
    }
}

#[test]
fn indexed_png_breaks_opaque_color_ties_in_palette_order() {
    // Both opaque entries are equally far from magenta; the transparent entry is an exact
    // match. Filtering transparency must still choose the earlier original opaque entry.
    for opaque in [[[255, 0, 0], [0, 0, 255]], [[0, 0, 255], [255, 0, 0]]] {
        let mut d = Document::with_background("m", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::rgba(1.0, 0.0, 1.0, 1.0));
        d.mode = ColorMode::Indexed;
        d.color_table = Some(ColorTable { colors: vec![[255, 0, 255], opaque[0], opaque[1]], transparent: Some(0) });
        let r = photocraft_io::export(&d, "out.png", &Default::default()).unwrap();
        let image = photocraft_codecs::decode(&r.bytes).unwrap();
        assert_eq!(image.data(), &[opaque[0][0], opaque[0][1], opaque[0][2], 255]);
    }
}

#[test]
fn indexed_png_keeps_the_alpha_threshold() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut d = Document::with_background("m", Size::new(3, 1), ColorMode::Rgb, depth, Color::rgba(1.0, 0.0, 0.0, 1.0));
        d.mode = ColorMode::Indexed;
        d.color_table = Some(ColorTable { colors: vec![[255, 0, 0], [255, 0, 0]], transparent: Some(0) });
        let surface = d.layers[0].surface_mut().unwrap();
        surface.write_pixel(0, 0, &[1.0, 0.0, 0.0, 0.49]);
        surface.write_pixel(1, 0, &[1.0, 0.0, 0.0, 0.5]);
        let r = photocraft_io::export(&d, "out.png", &Default::default()).unwrap();
        assert_eq!(photocraft_codecs::decode(&r.bytes).unwrap().data(), &[255, 0, 0, 0, 255, 0, 0, 255, 255, 0, 0, 255], "{depth:?}");
    }
}

#[test]
fn indexed_png_keeps_the_last_palette_index() {
    let mut palette = vec![[0, 0, 255]; 256];
    palette[0] = [255, 0, 0];
    palette[255] = [255, 0, 0];
    for transparent in [0, 255] {
        let png = photocraft_codecs::encode_png_indexed(2, 1, &[0, 255], &palette, Some(transparent), None, None).unwrap();
        let d = photocraft_io::import("in.png", &png).unwrap().document;
        let r = photocraft_io::export(&d, "out.png", &Default::default()).unwrap();
        assert_eq!(photocraft_codecs::png_palette(&r.bytes).unwrap(), photocraft_codecs::png_palette(&png).unwrap());
        assert_eq!(photocraft_codecs::decode(&r.bytes).unwrap().data(), photocraft_codecs::decode(&png).unwrap().data());
    }
}

#[test]
fn indexed_png_keeps_a_single_transparent_entry() {
    let png = photocraft_codecs::encode_png_indexed(2, 1, &[0, 0], &[[255, 0, 0]], Some(0), None, None).unwrap();
    let d = photocraft_io::import("in.png", &png).unwrap().document;
    let r = photocraft_io::export(&d, "out.png", &Default::default()).unwrap();
    assert_eq!(photocraft_codecs::png_palette(&r.bytes).unwrap(), photocraft_codecs::png_palette(&png).unwrap());
    assert_eq!(photocraft_codecs::decode(&r.bytes).unwrap().data(), photocraft_codecs::decode(&png).unwrap().data());
}

#[test]
fn indexed_png_rejects_opaque_pixels_without_an_opaque_entry() {
    let mut d = doc(ColorMode::Indexed);
    d.color_table = Some(ColorTable { colors: vec![[255, 0, 0]], transparent: Some(0) });
    let err = photocraft_io::export(&d, "out.png", &Default::default()).unwrap_err();
    assert!(matches!(err, photocraft_io::IoError::Unsupported(_)), "{err}");
    assert!(err.to_string().contains("no opaque entry"), "{err}");
}

#[test]
fn duotone_exports_the_inks_as_rgb() {
    // Mid gray background.
    let mut d = Document::with_background("m", Size::new(4, 2), ColorMode::Grayscale, SampleType::U8, Color::rgba(0.5, 0.5, 0.5, 1.0));
    d.mode = ColorMode::Duotone;
    d.duotone = Some(Duotone { inks: vec![DuotoneInk::new("Black", [0.0; 3]), DuotoneInk::new("Orange", [1.0, 0.5, 0.0])], psd_raw: None });
    let r = photocraft_io::export(&d, "x.png", &Default::default()).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("Duotone")), "{:?}", r.warnings);
    let back = photocraft_io::import("x.png", &r.bytes).unwrap().document;
    let px = back.layers[0].surface().unwrap().rgba(0, 0);
    assert!(px[0] > px[2] + 0.05, "warm: {px:?}");
}

#[test]
fn indexed_png_keeps_the_document_resolution() {
    // #3047: the palette PNG lost its pHYs chunk and reopened at 72 ppi.
    let mut d = doc(ColorMode::Indexed);
    d.resolution_dpi = 300.0;
    d.color_table = Some(ColorTable { colors: vec![[0, 0, 0], [255, 0, 0]], transparent: None });
    let r = photocraft_io::export(&d, "x.png", &Default::default()).unwrap();
    assert_eq!(r.bytes[25], 3, "still a palette PNG");
    let back = photocraft_io::import("x.png", &r.bytes).unwrap().document;
    assert!((back.resolution_dpi - 300.0).abs() < 0.01, "{}", back.resolution_dpi);
}

#[test]
fn indexed_bmp_is_palette_bmp() {
    // The BMP export wrote an Indexed Color document as 24-bit RGB, so reopening it lost the mode (#3869 follow-up).
    let mut d = doc(ColorMode::Indexed);
    d.color_table = Some(ColorTable { colors: vec![[0, 0, 0], [255, 0, 0], [0, 255, 0]], transparent: None });
    let r = photocraft_io::export(&d, "x.bmp", &Default::default()).unwrap();
    assert_eq!(u16::from_le_bytes([r.bytes[28], r.bytes[29]]), 8, "8 bits per pixel");
    assert!(r.warnings.iter().any(|w| w.contains("palette BMP")), "{:?}", r.warnings);
    let back = photocraft_io::import("x.bmp", &r.bytes).unwrap().document;
    assert_eq!(back.mode, ColorMode::Indexed);
    assert_eq!(back.color_table.as_ref().map(|t| t.colors.clone()), Some(vec![[0, 0, 0], [255, 0, 0], [0, 255, 0]]));
    let px = back.layers[0].surface().unwrap().rgba(1, 1);
    assert_eq!(&px[..3], &[1.0, 0.0, 0.0]);
}

#[test]
fn indexed_bmp_warns_that_it_has_no_transparency() {
    let mut d = doc(ColorMode::Indexed);
    d.color_table = Some(ColorTable { colors: vec![[0, 0, 0], [255, 0, 0]], transparent: Some(0) });
    let r = photocraft_io::export(&d, "x.bmp", &Default::default()).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("no transparency")), "{:?}", r.warnings);
}

#[test]
fn indexed_bmp_exports_when_its_only_palette_entry_is_marked_transparent() {
    let mut d = doc(ColorMode::Indexed);
    d.color_table = Some(ColorTable { colors: vec![[255, 0, 0]], transparent: Some(0) });

    let r = photocraft_io::export(&d, "x.bmp", &Default::default()).unwrap();

    assert_eq!(u16::from_le_bytes([r.bytes[28], r.bytes[29]]), 8, "8 bits per pixel");
    let back = photocraft_io::import("x.bmp", &r.bytes).unwrap().document;
    assert_eq!(&back.layers[0].surface().unwrap().rgba(1, 1)[..3], &[1.0, 0.0, 0.0]);
}

#[test]
fn indexed_bmp_matches_opaque_pixels_against_the_transparent_palette_entry() {
    let mut d = doc(ColorMode::Indexed);
    d.color_table = Some(ColorTable { colors: vec![[255, 0, 0], [0, 0, 255]], transparent: Some(0) });

    let r = photocraft_io::export(&d, "x.bmp", &Default::default()).unwrap();

    let back = photocraft_io::import("x.bmp", &r.bytes).unwrap().document;
    assert_eq!(&back.layers[0].surface().unwrap().rgba(1, 1)[..3], &[1.0, 0.0, 0.0]);
}

#[test]
fn indexed_bmp_without_a_usable_table_falls_back_to_rgb() {
    // No table (or one past 256 entries) keeps the old 24-bit path rather than failing the save.
    for table in [None, Some(ColorTable { colors: vec![[0; 3]; 257], transparent: None })] {
        let mut d = doc(ColorMode::Indexed);
        d.color_table = table;
        let r = photocraft_io::export(&d, "x.bmp", &Default::default()).unwrap();
        assert_eq!(u16::from_le_bytes([r.bytes[28], r.bytes[29]]), 24);
    }
}

/// Entry `i` of `palette` as the normalized RGB an Indexed Color document holds, plus its alpha.
fn entry_rgb(palette: &[[u8; 3]], i: usize) -> [f32; 4] {
    let c = palette.get(i).copied().unwrap_or([0, 0, 0]);
    [f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, 1.0]
}

/// An Indexed Color document, one pixel per palette entry, each holding that entry's colour.
fn indexed_doc(palette: &[[u8; 3]], transparent: Option<u8>, depth: SampleType) -> Document {
    let mut d = Document::with_background("m", Size::new(palette.len() as u32, 1), ColorMode::Rgb, depth, Color::rgba(0.0, 0.0, 0.0, 1.0));
    d.mode = ColorMode::Indexed;
    d.color_table = Some(ColorTable { colors: palette.to_vec(), transparent });
    {
        let s = d.layers[0].surface_mut().unwrap();
        for x in 0..palette.len() {
            s.write_pixel(x as i32, 0, &entry_rgb(palette, x));
        }
    }
    d
}

#[test]
fn indexed_gif_keeps_unused_palette_entries() {
    // #11824: a GIF save rebuilt the colour table from the colours present in the image, so the
    // entries no pixel used vanished and the rest changed index. Palette PNG and BMP keep the
    // document's table; GIF now does the same.
    let mut palette = vec![[10, 20, 30], [200, 100, 50]];
    // Reserved entries no pixel uses: pixel art keeps them for later edits and palette swaps.
    palette.extend((2..8u8).map(|i| [i, i * 16, 255 - i]));
    let mut d = doc(ColorMode::Indexed);
    d.color_table = Some(ColorTable { colors: palette.clone(), transparent: None });
    let r = photocraft_io::export(&d, "x.gif", &Default::default()).unwrap();
    assert_eq!(photocraft_codecs::gif_palette(&r.bytes).unwrap().0, palette, "every entry, in the original order");
    let back = photocraft_io::import("x.gif", &r.bytes).unwrap().document;
    assert_eq!(back.mode, ColorMode::Indexed);
    assert_eq!(back.color_table.as_ref().map(|t| t.colors.clone()), Some(palette), "reopened with the same table");
    let px = back.layers[0].surface().unwrap().rgba(1, 1);
    assert_eq!(&px[..3], &[200.0 / 255.0, 100.0 / 255.0, 50.0 / 255.0], "{px:?}");
    assert!(r.warnings.iter().any(|w| w.contains("colour table")), "{:?}", r.warnings);
    assert!(!r.warnings.iter().any(|w| w.contains("quantiz")), "the palette was not rebuilt: {:?}", r.warnings);
}

#[test]
fn indexed_gif_keeps_the_palette_index_of_every_pixel() {
    // Each pixel was drawn as one palette entry, so after the save its index must still mean that
    // entry rather than the nearest colour of a freshly built table.
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let palette: Vec<[u8; 3]> = (0..9u8).map(|i| [i * 28, (9 - i) * 24, i * 7 + 3]).collect();
        let d = indexed_doc(&palette, None, depth);
        let r = photocraft_io::export(&d, "out.gif", &Default::default()).unwrap();
        let table = photocraft_codecs::gif_palette(&r.bytes).unwrap();
        assert_eq!(table.0.get(..palette.len()), Some(&palette[..]), "{depth:?}");
        let back = photocraft_io::import("out.gif", &r.bytes).unwrap().document;
        let s = back.layers[0].surface().unwrap();
        for (x, c) in palette.iter().enumerate() {
            let px = s.rgba(x as i32, 0);
            for k in 0..3 {
                assert!((px[k] - f32::from(c[k]) / 255.0).abs() < 1.0 / 255.0, "{depth:?}, index {x}: {px:?} vs {c:?}");
            }
        }
    }
}

#[test]
fn indexed_gif_pads_a_200_entry_palette_to_256_entries() {
    // A GIF colour table holds 2^n entries. The document's 200 entries keep their slots and the
    // rest is padded with black (the palette BMP writer's convention too), so no used index moves.
    let palette: Vec<[u8; 3]> = (0..200u16).map(|i| [(i * 7 % 256) as u8, (i * 13 % 256) as u8, (255 - i % 256) as u8]).collect();
    let mut d = Document::with_background("m", Size::new(2, 1), ColorMode::Rgb, SampleType::U8, Color::rgba(0.0, 0.0, 0.0, 1.0));
    d.mode = ColorMode::Indexed;
    d.color_table = Some(ColorTable { colors: palette.clone(), transparent: None });
    {
        let s = d.layers[0].surface_mut().unwrap();
        s.write_pixel(0, 0, &entry_rgb(&palette, 0));
        s.write_pixel(1, 0, &entry_rgb(&palette, 199));
    }
    let r = photocraft_io::export(&d, "x.gif", &Default::default()).unwrap();
    let (colors, _) = photocraft_codecs::gif_palette(&r.bytes).unwrap();
    assert_eq!(colors.len(), 256, "padded to the next power of two");
    assert_eq!(&colors[..200], &palette[..], "the document's entries, in their original order");
    assert!(colors[200..].iter().all(|c| *c == [0, 0, 0]), "the padding holds no palette colours");
    let px = photocraft_io::import("x.gif", &r.bytes).unwrap().document.layers[0].surface().unwrap().rgba(1, 0);
    let want = entry_rgb(&palette, 199);
    for k in 0..3 {
        assert!((px[k] - want[k]).abs() < 1.0 / 255.0, "index 199 still means entry 199: {px:?} vs {:?}", palette[199]);
    }
}

#[test]
fn indexed_gif_keeps_the_transparent_index() {
    let mut d = Document::with_background("m", Size::new(2, 1), ColorMode::Rgb, SampleType::U8, Color::rgba(1.0, 0.0, 0.0, 1.0));
    d.mode = ColorMode::Indexed;
    d.color_table = Some(ColorTable { colors: vec![[255, 0, 0], [0, 255, 0], [0, 0, 255]], transparent: Some(2) });
    d.layers[0].surface_mut().unwrap().write_pixel(1, 0, &[0.0, 0.0, 1.0, 0.0]);
    let r = photocraft_io::export(&d, "x.gif", &Default::default()).unwrap();
    assert_eq!(photocraft_codecs::gif_palette(&r.bytes).unwrap().1, Some(2), "the designated transparent index");
    let back = photocraft_io::import("x.gif", &r.bytes).unwrap().document;
    assert_eq!(back.color_table.as_ref().unwrap().transparent, Some(2));
    let s = back.layers[0].surface().unwrap();
    assert_eq!(s.rgba(1, 0)[3], 0.0, "the transparent entry stays transparent");
    assert_eq!(s.rgba(0, 0)[3], 1.0, "the opaque pixel keeps entry 0");
}

#[test]
fn indexed_gif_round_trips_a_saved_gif() {
    // Regression: a GIF opened and saved again keeps its table and its decoded pixels.
    let palette = [[10, 20, 30], [200, 100, 50], [7, 8, 9], [0, 255, 0]];
    let bytes = photocraft_codecs::web::encode_gif_indexed(4, 1, &[0, 1, 2, 3], &palette, Some(2), false).unwrap();
    let d = photocraft_io::import("in.gif", &bytes).unwrap().document;
    let before = d.color_table.clone();
    let r = photocraft_io::export(&d, "out.gif", &Default::default()).unwrap();
    assert_eq!(photocraft_codecs::gif_palette(&r.bytes), photocraft_codecs::gif_palette(&bytes));
    assert_eq!(photocraft_codecs::decode(&r.bytes).unwrap().data(), photocraft_codecs::decode(&bytes).unwrap().data());
    let back = photocraft_io::import("out.gif", &r.bytes).unwrap().document;
    assert_eq!(back.color_table, before, "reopening the save changes nothing");
    assert_eq!(d.color_table, before, "export must not edit the document's table");
}

#[test]
fn indexed_gif_warns_about_the_metadata_it_cannot_hold() {
    let mut d = doc(ColorMode::Indexed);
    d.color_table = Some(ColorTable { colors: vec![[0, 0, 0], [255, 0, 0]], transparent: None });
    d.metadata.text = vec![("Comment".into(), "kept".into())];
    let r = photocraft_io::export(&d, "x.gif", &Default::default()).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("text metadata")), "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.contains("resolution (DPI)")), "{:?}", r.warnings);
}

#[test]
fn rgb_gif_still_quantizes_its_colors() {
    // Only an Indexed Color document keeps its table: an RGB GIF is still quantized as before.
    let d = doc(ColorMode::Rgb);
    let r = photocraft_io::export(&d, "x.gif", &Default::default()).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("quantized")), "{:?}", r.warnings);
    assert!(photocraft_codecs::gif_palette(&r.bytes).unwrap().0.contains(&[255, 0, 0]), "a table of the colours present");
    assert_eq!(photocraft_io::import("x.gif", &r.bytes).unwrap().document.mode, ColorMode::Indexed);
}

#[test]
fn indexed_gif_without_a_usable_table_falls_back_to_rgb() {
    // As with the palette BMP: no table (or one past 256 entries) keeps the quantizing path
    // rather than failing the save.
    for table in [None, Some(ColorTable { colors: vec![[0; 3]; 257], transparent: None })] {
        let mut d = doc(ColorMode::Indexed);
        d.color_table = table;
        let r = photocraft_io::export(&d, "x.gif", &Default::default()).unwrap();
        assert!(!r.bytes.is_empty());
        assert!(r.warnings.iter().any(|w| w.contains("quantized")), "{:?}", r.warnings);
    }
}

#[test]
fn indexed_gif_rejects_opaque_pixels_without_an_opaque_entry() {
    // Same rule as the palette PNG: with only the transparent entry left, an opaque pixel has
    // nothing to point at, and the save says so instead of writing a GIF that shows nothing.
    let mut d = doc(ColorMode::Indexed);
    d.color_table = Some(ColorTable { colors: vec![[255, 0, 0]], transparent: Some(0) });
    let err = photocraft_io::export(&d, "out.gif", &Default::default()).unwrap_err();
    assert!(matches!(err, photocraft_io::IoError::Unsupported(_)), "{err}");
    assert!(err.to_string().contains("no opaque entry"), "{err}");
}
