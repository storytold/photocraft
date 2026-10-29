//! A PNG imported in steps gives the document `import_with_options_and_interrupt` gives.

use std::sync::Arc;

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType, encode};
use photocraft_doc::Document;
use photocraft_io::{ImportOptions, ImportResult, PngImport, import_with_options_and_interrupt};
use photocraft_raster::Interrupt;

/// A pseudo-random `w`×`h` image.
fn noise(w: u32, h: u32, layout: ChannelLayout, sample: SampleType) -> Image {
    let len = w as usize * h as usize * layout.channels() * sample.bytes();
    Image::from_raw(w, h, layout, sample, (0..len).map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8).collect()).unwrap()
}

fn whole(name: &str, bytes: &[u8]) -> ImportResult {
    import_with_options_and_interrupt(name, bytes, &ImportOptions::default(), &Interrupt::NONE).unwrap()
}

/// Imports `bytes` in steps, `per_call` rows at a time (0: unlimited); returns the result and
/// how many calls it took.
fn stepped(name: &str, bytes: &[u8], per_call: usize) -> (ImportResult, usize) {
    let bytes = Arc::new(bytes.to_vec());
    let mut import = PngImport::new(name, &bytes).unwrap().expect("a plain PNG");
    let mut calls = 0;
    loop {
        calls += 1;
        let mut left = per_call;
        let mut more = || {
            left = left.wrapping_sub(1);
            per_call == 0 || left > 0
        };
        if let Some(r) = import.step(&mut more).unwrap() {
            return (r, calls);
        }
        assert!(calls < 100_000);
    }
}

/// The documents are the same but for the ids minted for them.
fn assert_same(a: &Document, b: &Document, what: &str) {
    let (mut a, mut b) = (a.clone(), b.clone());
    b.id = a.id;
    assert_eq!(a.layers.len(), b.layers.len(), "{what}");
    for (x, y) in a.layers.iter_mut().zip(&mut b.layers) {
        y.id = x.id;
    }
    assert!(a == b, "{what}: the documents differ");
}

fn assert_stepped_equals_import(name: &str, bytes: &[u8], what: &str) {
    let expected = whole(name, bytes);
    for per_call in [1, 7, 0] {
        let (got, _) = stepped(name, bytes, per_call);
        assert_same(&got.document, &expected.document, &format!("{what}, {per_call} rows per call"));
        assert_eq!(got.warnings, expected.warnings, "{what}");
        assert_eq!((got.source_read_only, got.preview_only), (expected.source_read_only, expected.preview_only), "{what}");
    }
}

#[test]
fn stepped_png_import_equals_import() {
    for layout in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba] {
        for sample in [SampleType::U8, SampleType::U16] {
            let mut img = noise(45, 38, layout, sample);
            img.meta.dpi = Some((144.0, 144.0));
            img.meta.text.push(("Comment".into(), "stepped".into()));
            for interlaced in [false, true] {
                let bytes = encode(&img, Format::Png, &EncodeOptions { png_interlaced: interlaced, ..Default::default() }).unwrap();
                assert_stepped_equals_import("a.png", &bytes, &format!("{layout:?} {sample:?} interlaced={interlaced}"));
            }
        }
    }
}

#[test]
fn stepped_png_import_keeps_a_palette_as_indexed_colour() {
    let pal = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [9, 9, 9]];
    let idx: Vec<u8> = (0..40 * 30).map(|i| (i % 4) as u8).collect();
    let bytes = photocraft_codecs::encode_png_indexed(40, 30, &idx, &pal, Some(3), None, None).unwrap();
    let (got, _) = stepped("p.png", &bytes, 0);
    assert!(got.document.color_table.is_some(), "an Indexed Color document");
    assert_stepped_equals_import("p.png", &bytes, "palette");
}

#[test]
fn stepped_png_import_takes_a_step_per_row_and_one_to_build_the_document() {
    let bytes = encode(&noise(8, 20, ChannelLayout::Rgba, SampleType::U8), Format::Png, &EncodeOptions::default()).unwrap();
    let (_, calls) = stepped("a.png", &bytes, 1);
    // 19 calls that each decode a row, one that decodes the last and finishes the file, one that
    // builds the document.
    assert_eq!(calls, 21);
    let arc = Arc::new(bytes);
    let mut import = PngImport::new("a.png", &arc).unwrap().unwrap();
    assert_eq!(import.progress(), 0.0);
    while import.step(&mut || false).unwrap().is_none() {
        assert!(import.progress() <= 1.0);
    }
    assert!(import.step(&mut || true).is_err(), "a finished import is spent");
}

#[test]
fn only_a_plain_png_is_stepped() {
    let png = Arc::new(encode(&noise(4, 4, ChannelLayout::Rgb, SampleType::U8), Format::Png, &EncodeOptions::default()).unwrap());
    assert!(PngImport::new("a.png", &png).unwrap().is_some());
    // The same bytes under a name that sends them to another importer, or another format.
    assert!(PngImport::new("a.pcraft", &png).unwrap().is_none());
    let jpeg = Arc::new(encode(&noise(4, 4, ChannelLayout::Rgb, SampleType::U8), Format::Jpeg, &EncodeOptions::default()).unwrap());
    assert!(PngImport::new("a.jpg", &jpeg).unwrap().is_none());
    assert!(PngImport::new("a.svg", &Arc::new(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec())).unwrap().is_none());
}

#[test]
fn a_damaged_png_fails_in_steps_as_it_does_whole() {
    let bytes = encode(&noise(64, 64, ChannelLayout::Rgb, SampleType::U8), Format::Png, &EncodeOptions::default()).unwrap();
    for cut in [30, bytes.len() / 2] {
        let cut = Arc::new(bytes.get(..cut).unwrap().to_vec());
        let whole = import_with_options_and_interrupt("a.png", &cut, &ImportOptions::default(), &Interrupt::NONE);
        let stepped = PngImport::new("a.png", &cut).and_then(|import| {
            let mut import = import.expect("a PNG by its signature");
            loop {
                if let Some(r) = import.step(&mut || true)? {
                    return Ok(r);
                }
            }
        });
        assert!(whole.is_err() && stepped.is_err(), "cut at {}", cut.len());
    }
}
