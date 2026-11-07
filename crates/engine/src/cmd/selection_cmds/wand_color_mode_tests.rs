//! The Magic Wand's Tolerance in the document's own colour channels (#11785).
//!
//! The fixture is the reference's test: a grey ramp of ten 40 px blocks (grey 100, 105, … 145)
//! from x 100, clicked on the 100 block, with Contiguous on. The reference reads the reached
//! range off the selection bounds in x, and counts the block it stops on, so a range is written
//! here as `lo..hi` with `hi` one past the last selected column: one 40 px block at x 100 is
//! `100..140`, which is the table's "x 100-140". Anti-alias is off in these tests: with it on,
//! the wand adds its one-pixel soft fringe to the bounds, which is the same in every mode and
//! not what the table measures.

use super::*;
use photocraft_doc::{ColorMode, SampleType};

const W: i32 = 500;
const H: i32 = 8;
const X0: i32 = 100;
const BLOCK: i32 = 40;

/// The grey at block `k` of the ramp.
fn grey(k: u8) -> u8 {
    100 + 5 * k
}

/// The ramp document: painted as grey in an RGB document, then converted to `mode` by its Image
/// › Mode command (the reference's "Lab document" and "CMYK document" rows), or left as the
/// `create` mode (its RGB and Grayscale rows).
fn ramp(create: &str, depth: u32, mode: Option<&str>) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "mode": create, "depth": depth})).unwrap();
    s.edit("paint", |doc, _| {
        let bg = doc.layers[0].surface_mut().unwrap();
        for k in 0..10u8 {
            let v = grey(k) as f32 / 255.0;
            bg.fill_rect(Rect::new(X0 + BLOCK * i32::from(k), 0, X0 + BLOCK * (i32::from(k) + 1), H), &[v, v, v, 1.0]);
        }
        Ok(())
    })
    .unwrap();
    if let Some(cmd) = mode {
        s.execute(cmd, json!({})).unwrap();
    }
    s
}

/// Click block 0 with `tolerance`.
fn wand(s: &mut Session, tolerance: f32) {
    s.execute("select.magicWand", json!({"x": X0 + 5, "y": 1, "tolerance": tolerance, "contiguous": true, "antiAlias": false})).unwrap();
}

/// The x-range the selection covers, as `lo..hi` (`hi` one past the last selected column), or
/// `None` when the click selected nothing.
fn reach(s: &Session) -> Option<(i32, i32)> {
    let mask = s.active()?.doc.selection.as_ref()?;
    let (mut lo, mut hi) = (i32::MAX, i32::MIN);
    for x in 0..W {
        if mask.sample_channel(x, 1, 0) > 0.0 {
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    (lo <= hi).then(|| (lo, hi + 1))
}

/// The levels the background layer stores for the ramp, one per block, in `channel` of the
/// document's own colour model.
fn stored(s: &Session, channel: usize) -> Vec<u8> {
    let surf = s.active().unwrap().doc.layers[0].surface().unwrap();
    (0..10u8).map(|k| ((surf.pixel(X0 + BLOCK * i32::from(k) + 5, 1)[channel]) * 255.0).round() as u8).collect()
}

/// #11785: in an 8-bit Lab document the wand measures Tolerance on the document's Lightness, a
/// and b, not on the RGB it shows.
#[test]
fn lab_document_measures_tolerance_in_its_own_channels() {
    let mut s = ramp("rgb", 8, Some("image.mode.lab"));
    assert_eq!(s.active().unwrap().doc.mode, ColorMode::Lab);
    assert_eq!(s.active().unwrap().doc.depth, SampleType::U8);
    // The ramp as this document stores it: Lightness 108, 113, 118, 124, … 153 (L* / 100 × 255),
    // a and b neutral. Steps from the seed: 0, 5, 10, 16, 21, 26, 31, 36, 41, 45 levels.
    assert_eq!(stored(&s, 0), [108, 113, 118, 124, 129, 134, 139, 144, 149, 153], "the Lightness the ramp is stored at");
    assert_eq!(stored(&s, 1), [128; 10], "neutral a");
    assert_eq!(stored(&s, 2), [128; 10], "neutral b");

    // The two measured rows of #11785's table.
    wand(&mut s, 5.0);
    assert_eq!(reach(&s), Some((100, 140)), "Tolerance 5 reaches only the 100 block, not the 105 one");
    wand(&mut s, 32.0);
    assert_eq!(reach(&s), Some((100, 380)), "Tolerance 32 reaches the 130 block, not the 135 one");

    // Derived from the same table, and what says the comparison happens in Lab rather than only
    // at a different bound: on the RGB picture (100, 105, … 145, steps of 5) Tolerance 16 reaches
    // the 115 block (x 100-260); on the Lightness steps above, where that step is 16 levels and
    // does not match, it stops after the 120 block.
    wand(&mut s, 16.0);
    assert_eq!(reach(&s), Some((100, 220)), "Tolerance 16 stops after a 16-level Lightness step");

    // Tolerance 0 and 1 both mean the same colour: the clicked block, and nothing beside it.
    for tolerance in [0.0, 1.0] {
        wand(&mut s, tolerance);
        assert_eq!(reach(&s), Some((100, 140)), "Tolerance {tolerance} keeps the same colour");
    }

    // Without Contiguous the same Lab measure applies wherever it is on the canvas.
    s.execute("select.magicWand", json!({"x": X0 + 5, "y": 1, "tolerance": 32.0, "contiguous": false, "antiAlias": false})).unwrap();
    assert_eq!(reach(&s), Some((100, 380)));
}

/// #11785's unchanged rows: RGB documents keep the selection they make today, at every depth.
#[test]
fn rgb_documents_keep_measuring_tolerance_in_rgb() {
    for depth in [8u32, 16, 32] {
        let mut s = ramp("rgb", depth, None);
        wand(&mut s, 5.0);
        assert_eq!(reach(&s), Some((100, 180)), "rgb {depth}: Tolerance 5 reaches 100 and 105");
        wand(&mut s, 32.0);
        assert_eq!(reach(&s), Some((100, 380)), "rgb {depth}: Tolerance 32 reaches the 130 block");
    }
}

/// #11785's unchanged rows: Grayscale documents, at every depth.
#[test]
fn grayscale_documents_keep_their_selection_at_every_depth() {
    for depth in [8u32, 16, 32] {
        let mut s = ramp("gray", depth, None);
        assert_eq!(s.active().unwrap().doc.mode, ColorMode::Grayscale);
        wand(&mut s, 5.0);
        assert_eq!(reach(&s), Some((100, 180)), "gray {depth}");
        wand(&mut s, 32.0);
        assert_eq!(reach(&s), Some((100, 380)), "gray {depth}");
    }
}

/// Lab at 16 and 32 bits is marked *not measured* in #11785, so it keeps today's sampling. If the
/// rows get measured, this is the test to change with them.
#[test]
fn lab_documents_above_8_bits_keep_todays_sampling() {
    for depth in [16u32, 32] {
        let mut s = ramp("rgb", depth, Some("image.mode.lab"));
        assert_eq!(s.active().unwrap().doc.depth, if depth == 16 { SampleType::U16 } else { SampleType::F32 });
        // Its Lightness is 108.06, 113.26, …, so a Lab comparison would stop as in 8-bit; today's
        // RGB sampling reaches the 105 block at Tolerance 5 instead, and stays.
        assert_eq!(stored(&s, 0), [108, 113, 118, 124, 129, 134, 139, 144, 149, 153], "lab {depth} Lightness");
        wand(&mut s, 5.0);
        assert_eq!(reach(&s), Some((100, 180)), "lab {depth} keeps today's Tolerance 5 reach");
        wand(&mut s, 32.0);
        assert_eq!(reach(&s), Some((100, 380)), "lab {depth} keeps today's Tolerance 32 reach");
    }
}

/// CMYK 8-bit is left on today's sampling for now: the reference's two CMYK numbers cannot come
/// from a per-ink comparison on this ramp, so the ink channels are not compared yet (the question
/// is on #11785). This pins what the wand does there today, and records the ink values an ink
/// comparison would measure, so the difference is on the table when that row is settled.
#[test]
fn cmyk_document_keeps_todays_sampling_until_its_numbers_are_settled() {
    let mut s = ramp("rgb", 8, Some("image.mode.cmyk"));
    assert_eq!(s.active().unwrap().doc.mode, ColorMode::Cmyk);
    // Grey 100 … 145 as coated-CMYK inks: the cyan hovers near 96 while the black ink falls from
    // 81 to 20, so the seed's steps in the inks are 0, 9, 17, 25, 32, 38, 45, 50, 56, 61.
    assert_eq!(stored(&s, 0), [95, 96, 97, 97, 97, 96, 95, 94, 93, 91], "cyan");
    assert_eq!(stored(&s, 3), [81, 72, 64, 56, 49, 43, 36, 31, 25, 20], "black");
    // On these inks Tolerance 5 would reach 100-140 and Tolerance 32 100-260, and on today's RGB
    // picture (100, 105/106, 110/111, …) it reaches 100-140 and 100-380. The table's numbers,
    // 100-146 and 100-341, are inside neither: they are not on the 40 px block grid.
    wand(&mut s, 5.0);
    assert_eq!(reach(&s), Some((100, 140)), "cmyk keeps today's Tolerance 5 reach");
    wand(&mut s, 32.0);
    assert_eq!(reach(&s), Some((100, 380)), "cmyk keeps today's Tolerance 32 reach");
}

/// The shared sampler the Paint Bucket, the Magic Eraser and the Background Eraser go through is
/// untouched: an 8-bit Lab document still hands it the RGB picture.
#[test]
fn the_shared_rgba8_sample_path_still_reads_rgb_in_a_lab_document() {
    let s = ramp("rgb", 8, Some("image.mode.lab"));
    let (area, img) = sample_rgba8(&s, false).unwrap();
    let at = |k: u8| img[(X0 + BLOCK * i32::from(k) + 5 - area.x0) as usize];
    for k in 0..10u8 {
        assert_eq!(at(k)[0], grey(k), "block {k} still samples as RGB for the other tools");
        assert_eq!(at(k)[1], grey(k));
        assert_eq!(at(k)[2], grey(k));
        assert_eq!(at(k)[3], 255);
    }
    assert!(wand_document_channels(&s, false).is_some(), "the wand's own Lab route is in use here");
    assert!(wand_document_channels(&s, true).is_none(), "Sample All Layers is not measured, so it keeps the RGB composite");
}
