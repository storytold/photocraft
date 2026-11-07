//! The Magic Wand's Tolerance in the document's own colour channels (#11785).
//!
//! The fixtures are the reference's tests. The grey ramp is ten 40 px blocks (grey 100, 105, …
//! 145) from x 100, clicked on the 100 block, with Contiguous on; the CMYK row is the re-measured
//! dither-free ramp, filled block by block with the reference's own ink levels so no colour
//! profile sits between the fixture and the assertion (#11785 comment 6705983844).
//!
//! The reference reads the reached range off the selection bounds in x and counts the block it
//! stops on, so a range is written here as `lo..hi` with `hi` one past the last selected column:
//! one 40 px block at x 100 is `100..140`, which is the table's "x 100-140". Anti-alias is off in
//! these tests: with it on the wand adds its one-pixel soft fringe to the bounds, which is the
//! same in every mode and not what the table measures.

use super::*;
use photocraft_doc::{ColorMode, SampleType};

const W: i32 = 500;
const H: i32 = 8;
const X0: i32 = 100;
const BLOCK: i32 = 40;

/// The reference's dither-free CMYK ramp: the ink levels (0-255) of the ten grey blocks, in the
/// order Cyan, Magenta, Yellow, Black.
const INKS: [[u8; 4]; 10] = [
    [111, 138, 139, 169],
    [114, 141, 143, 176],
    [117, 144, 145, 182],
    [121, 148, 149, 188],
    [124, 150, 152, 194],
    [127, 154, 155, 200],
    [131, 158, 158, 206],
    [135, 161, 161, 210],
    [139, 164, 164, 215],
    [143, 167, 167, 220],
];

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
    paint_ramp(&mut s, &|k| {
        let v = grey(k) as f32 / 255.0;
        [v, v, v, 1.0, 1.0]
    });
    if let Some(cmd) = mode {
        s.execute(cmd, json!({})).unwrap();
    }
    s
}

/// An 8-bit CMYK document with each block filled directly with its ink from [`INKS`]: one fill
/// per block, so the blocks are uniform and no conversion or dither is involved (#11785).
fn ink_ramp() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "mode": "cmyk", "depth": 8})).unwrap();
    paint_ramp(&mut s, &|k| {
        let mut v = [1.0f32; 5];
        v[..4].copy_from_slice(&INKS[usize::from(k)].map(|x| f32::from(x) / 255.0));
        v
    });
    s
}

/// Fill the ten blocks across the canvas with one colour each, `pixel` of block `k` as the
/// surface's own channels (a CMYK surface has five, an RGB one four; the extra is ignored).
fn paint_ramp(s: &mut Session, pixel: &dyn Fn(u8) -> [f32; 5]) {
    s.edit("paint", |doc, _| {
        let bg = doc.layers[0].surface_mut().unwrap();
        let n = bg.channels().min(5);
        for k in 0..10u8 {
            let v = pixel(k);
            bg.fill_rect(Rect::new(X0 + BLOCK * i32::from(k), 0, X0 + BLOCK * (i32::from(k) + 1), H), &v[..n]);
        }
        Ok(())
    })
    .unwrap();
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

/// #11785 as re-measured without dither: in an 8-bit CMYK document the wand measures Tolerance on
/// the document's inks, and the largest of the four differences is what counts.
#[test]
fn cmyk_document_measures_tolerance_on_the_largest_ink_difference() {
    let mut s = ink_ramp();
    assert_eq!(s.active().unwrap().doc.mode, ColorMode::Cmyk);
    assert_eq!(s.active().unwrap().doc.depth, SampleType::U8);
    // The blocks hold the reference's levels exactly: filled one colour per block, not converted.
    assert_eq!(stored(&s, 0), INKS.iter().map(|ink| ink[0]).collect::<Vec<_>>(), "cyan");
    assert_eq!(stored(&s, 1), INKS.iter().map(|ink| ink[1]).collect::<Vec<_>>(), "magenta");
    assert_eq!(stored(&s, 2), INKS.iter().map(|ink| ink[2]).collect::<Vec<_>>(), "yellow");
    assert_eq!(stored(&s, 3), INKS.iter().map(|ink| ink[3]).collect::<Vec<_>>(), "black");

    // The largest ink difference from the 100 block is 7, 13, 19, 25, 31, 37, 41, 46, 51 levels
    // (Black moves most), so Tolerance 5 stops after the first block and Tolerance 32 reaches the
    // 125 block: the two cells #11785's "Done when" asks for.
    wand(&mut s, 5.0);
    assert_eq!(reach(&s), Some((100, 140)), "Tolerance 5 stops before the 7-level ink step");
    wand(&mut s, 32.0);
    assert_eq!(reach(&s), Some((100, 340)), "Tolerance 32 reaches the 31-level block, not the 37-level one");

    // Tolerance 0 keeps the same inks; the RGB picture of these blocks would step only 5 levels
    // to the next block, so these two ranges are the inks' and not the picture's.
    wand(&mut s, 0.0);
    assert_eq!(reach(&s), Some((100, 140)), "Tolerance 0 keeps the same inks");
    s.execute("select.magicWand", json!({"x": X0 + 5, "y": 1, "tolerance": 32.0, "contiguous": false, "antiAlias": false})).unwrap();
    assert_eq!(reach(&s), Some((100, 340)), "the same ink measure without Contiguous");
}

/// The same ink measure on an 8-bit CMYK document converted from grey with the working profile.
/// Its inks are not the reference's, but the rule is: Black moves most, by 9, 17, 25, 32, 38 …
/// levels, so Tolerance 32 stops one block earlier than the RGB picture would.
#[test]
fn a_converted_cmyk_document_measures_the_same_ink_difference() {
    let mut s = ramp("rgb", 8, Some("image.mode.cmyk"));
    assert_eq!(stored(&s, 0), [95, 96, 97, 97, 97, 96, 95, 94, 93, 91], "cyan");
    assert_eq!(stored(&s, 3), [81, 72, 64, 56, 49, 43, 36, 31, 25, 20], "black");
    wand(&mut s, 5.0);
    assert_eq!(reach(&s), Some((100, 140)), "converted CMYK, Tolerance 5");
    wand(&mut s, 32.0);
    assert_eq!(reach(&s), Some((100, 260)), "converted CMYK, Tolerance 32 stops at the 32-level ink step");
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

/// Lab and CMYK above 8 bits are marked *not measured* in #11785, so they keep today's sampling.
/// If those rows get measured, this is the test to change with them.
#[test]
fn documents_above_8_bits_keep_todays_sampling() {
    for (cmd, name, tol5, tol32) in [("image.mode.lab", "lab", (100, 180), (100, 380)), ("image.mode.cmyk", "cmyk", (100, 140), (100, 380))] {
        for depth in [16u32, 32] {
            let mut s = ramp("rgb", depth, Some(cmd));
            assert_eq!(s.active().unwrap().doc.depth, if depth == 16 { SampleType::U16 } else { SampleType::F32 });
            // The Lab Lightness is 108.06, 113.26, …, so a Lab comparison would stop as in 8-bit;
            // today's RGB sampling reaches the 105 block at Tolerance 5 instead, and stays.
            wand(&mut s, 5.0);
            assert_eq!(reach(&s), Some(tol5), "{name} {depth} keeps today's Tolerance 5 reach");
            wand(&mut s, 32.0);
            assert_eq!(reach(&s), Some(tol32), "{name} {depth} keeps today's Tolerance 32 reach");
        }
    }
}

/// The shared sampler the Paint Bucket, the Magic Eraser and the Background Eraser go through is
/// untouched: both document modes still hand it the RGB picture, and Sample All Layers keeps the
/// RGB composite for the wand too.
#[test]
fn the_shared_rgba8_sample_path_still_reads_rgb() {
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

    let s = ink_ramp();
    let (area, img) = sample_rgba8(&s, false).unwrap();
    let seed = img[(X0 + 5 - area.x0) as usize];
    assert_ne!(seed[..4].to_vec(), INKS[0].to_vec(), "the other tools sample the CMYK picture as RGB, not as inks");
    assert_eq!(seed[3], 255);
    assert!(wand_document_channels(&s, false).is_some(), "the wand's own ink route is in use here");
    assert!(wand_document_channels(&s, true).is_none(), "and not with Sample All Layers");
}
