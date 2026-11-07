//! Protect Foreground Color (#11903): the protection fades with the distance from the foreground
//! colour, so the sampled colour is still erased while colours at and beyond the foreground are kept.

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba};

use crate::bg_erase::{BgEraseSettings, apply_background_eraser};
use crate::replace::{Limits, Sampling};
use crate::{BrushSettings, Stroke, StrokePoint};

/// Block `i` of the reference test layer: (200, 40 + 20 i, 40), so block i differs from block 0
/// by 20 i in green only.
fn block(i: usize) -> [f32; 4] {
    [200.0 / 255.0, (40.0 + 20.0 * i as f32) / 255.0, 40.0 / 255.0, 1.0]
}

/// Eight 100 px blocks side by side, 900 x 300 (the reference's layout, shifted up).
fn blocks(fmt: PixelFormat) -> Surface {
    let mut s = Surface::new(fmt);
    for i in 0..8 {
        let x = 100 * i as i32;
        s.fill_rect(Rect::new(x, 0, x + 100, 300), &from_rgba(&fmt, block(i)));
    }
    s
}

fn stroke(points: &[(f64, f64)]) -> Stroke {
    Stroke {
        brush: BrushSettings { size: 40.0, hardness: 1.0, spacing: 0.25, pressure_size: false, ..Default::default() },
        points: points.iter().map(|&(x, y)| StrokePoint::new(x, y, 1.0)).collect(),
    }
}

fn settings(protect: Option<usize>) -> BgEraseSettings {
    BgEraseSettings {
        sampling: Sampling::Once,
        limits: Limits::Discontiguous,
        tolerance: 0.5,
        protect_foreground: protect.is_some(),
        foreground: block(protect.unwrap_or(0)),
        ..Default::default()
    }
}

/// The reference's drag from block 0's centre through all eight; alpha (0-255) at each centre.
fn drag(fmt: PixelFormat, protect: Option<usize>) -> [i32; 8] {
    let mut s = blocks(fmt);
    apply_background_eraser(&mut s, &stroke(&[(50.0, 150.0), (750.0, 150.0)]), &settings(protect), None);
    std::array::from_fn(|i| (s.rgba(100 * i as i32 + 50, 150)[3] * 255.0).round() as i32)
}

/// The measured rows: Sampling Once, Discontiguous, Tolerance 50, 40 px hard tip. Without
/// protection the reference gives `0 30 75 114 156 198 238 255` (the graded erase of #11901).
const REFERENCE: [(usize, [i32; 8]); 3] =
    [(1, [0, 255, 255, 255, 255, 255, 255, 255]), (3, [0, 76, 166, 255, 255, 255, 255, 255]), (5, [0, 40, 94, 148, 201, 255, 255, 255])];

#[test]
fn the_sampled_colour_is_erased_and_colours_from_the_foreground_on_are_kept() {
    for (fg, want) in REFERENCE {
        let got = drag(PixelFormat::RGBA8, Some(fg));
        let off = drag(PixelFormat::RGBA8, None);
        for i in 0..8 {
            match want[i] {
                // Fully kept in the reference: kept here too.
                255 => assert_eq!(got[i], 255, "fg block {fg}: block {i} kept, got {got:?}"),
                // The sample itself: erased.
                0 => assert_eq!(got[i], 0, "fg block {fg}: block {i} (the sample) erased, got {got:?}"),
                // Partly erased in the reference: erased here too, never more than without protection.
                _ => assert!(got[i] < 255 && got[i] >= off[i], "fg block {fg}: block {i} partly erased, got {got:?}, off {off:?}"),
            }
        }
    }
}

#[test]
fn protection_narrows_the_tolerance_to_the_foreground_in_one_dab() {
    // One click (Background Swatch = block 0, so the click's own pixel does not matter) on each
    // block: with the foreground at block `fg`, the protected click erases what an unprotected
    // click at Tolerance = the sample-to-foreground distance erases, and keeps block fg and beyond.
    let fmt = PixelFormat::RGBA8;
    let click = |protect: Option<usize>, tolerance: f32| -> Vec<f32> {
        (0..8)
            .map(|i| {
                let mut s = blocks(fmt);
                let bs = BgEraseSettings { sampling: Sampling::BackgroundSwatch, background: block(0), tolerance, ..settings(protect) };
                let x = 100.0 * i as f64 + 50.0;
                apply_background_eraser(&mut s, &stroke(&[(x, 150.0)]), &bs, None);
                s.rgba(x as i32, 150)[3]
            })
            .collect()
    };
    for fg in [1, 3, 5] {
        let protected = click(Some(fg), 0.5);
        let narrowed = click(None, 20.0 * fg as f32 / 255.0);
        for i in 0..8 {
            if i < fg {
                assert!((protected[i] - narrowed[i]).abs() <= 1.0 / 255.0, "fg block {fg}: block {i} {protected:?} vs {narrowed:?}");
            } else {
                assert_eq!(protected[i], 1.0, "fg block {fg}: block {i} kept, {protected:?}");
            }
        }
    }
}

#[test]
fn a_foreground_equal_to_the_sample_erases_nothing() {
    for limits in [Limits::Contiguous, Limits::Discontiguous, Limits::FindEdges] {
        for tolerance in [0.0, 0.1, 0.5, 1.0] {
            let s = blocks(PixelFormat::RGBA8);
            let mut t = s.clone();
            let bs = BgEraseSettings { limits, tolerance, ..settings(Some(0)) };
            apply_background_eraser(&mut t, &stroke(&[(50.0, 150.0), (750.0, 150.0)]), &bs, None);
            assert_eq!(t, s, "{limits:?} tolerance {tolerance}");
        }
    }
}

#[test]
fn find_edges_keeps_the_foreground_colour_itself() {
    // Find Edges erases fully inside the Tolerance; a pixel exactly at the foreground stays.
    let fmt = PixelFormat::RGBA8;
    let mut s = blocks(fmt);
    let bs = BgEraseSettings { limits: Limits::FindEdges, ..settings(Some(1)) };
    apply_background_eraser(&mut s, &stroke(&[(80.0, 150.0), (120.0, 150.0)]), &bs, None);
    assert_eq!(s.rgba(75, 150)[3], 0.0, "the sample erased");
    assert_eq!(s.rgba(110, 150)[3], 1.0, "the foreground colour kept");
}

#[test]
fn protect_off_is_unchanged() {
    // The drag without protection, as main gives it before the graded erase of #11901: today's
    // pre-stroke matching over overlapping dabs. Pinned so the protection change cannot leak in.
    let off = drag(PixelFormat::RGBA8, None);
    assert_eq!(off[0], 0);
    assert_eq!(off[7], 255);
    let mut s = blocks(PixelFormat::RGBA8);
    let mut t = s.clone();
    let on_far = BgEraseSettings { foreground: [0.0, 0.0, 1.0, 1.0], protect_foreground: true, ..settings(None) };
    apply_background_eraser(&mut s, &stroke(&[(50.0, 150.0), (750.0, 150.0)]), &settings(None), None);
    apply_background_eraser(&mut t, &stroke(&[(50.0, 150.0), (750.0, 150.0)]), &on_far, None);
    assert_eq!(s, t, "a foreground beyond the Tolerance protects nothing");
}

#[test]
fn protection_works_in_the_documents_own_channels() {
    // 16-bit RGB: the same rows as 8-bit.
    let rgb16 = PixelFormat::new(ColorMode::Rgb, SampleType::U16, true);
    for (fg, want) in REFERENCE {
        let got = drag(rgb16, Some(fg));
        for i in 0..8 {
            if want[i] == 255 || want[i] == 0 {
                assert_eq!(got[i], want[i], "16-bit fg block {fg}: block {i}, got {got:?}");
            }
        }
    }
    // Gray: blocks stepping by 20 in gray; foreground at block 3 keeps blocks 3 and beyond and
    // still erases block 0.
    let gray = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, true);
    let mut s = Surface::new(gray);
    let level = |i: usize| (40.0 + 20.0 * i as f32) / 255.0;
    for i in 0..8 {
        let x = 100 * i as i32;
        s.fill_rect(Rect::new(x, 0, x + 100, 300), &[level(i), 1.0]);
    }
    let fg = [level(3), level(3), level(3), 1.0];
    let bs = BgEraseSettings { foreground: fg, ..settings(Some(3)) };
    apply_background_eraser(&mut s, &stroke(&[(50.0, 150.0), (750.0, 150.0)]), &bs, None);
    let a: Vec<f32> = (0..8).map(|i| s.rgba(100 * i + 50, 150)[3]).collect();
    assert_eq!(a[0], 0.0, "gray: the sample erased, {a:?}");
    for (i, v) in a.iter().enumerate().skip(3) {
        assert_eq!(*v, 1.0, "gray: block {i} kept, {a:?}");
    }
}
