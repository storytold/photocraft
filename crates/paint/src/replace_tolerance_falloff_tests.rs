//! #11916: colours near the Tolerance limit are only partly replaced, fading over the whole
//! range, as the reference application measures.

use super::*;
use crate::{BrushSettings, StrokePoint};
use photocraft_color::PixelFormat;

/// Eight 100 px blocks at y 300..600, block i = (200, 40 + 20 i, 40); a 40 px hard round tip
/// swept from x 50 to 750 at y 450 in Color mode with blue, Sampling Once, Discontiguous.
fn blocks_after(tolerance: f32) -> Vec<[f32; 3]> {
    let fmt = PixelFormat::RGBA8;
    let mut s = Surface::new(fmt);
    for i in 0..8 {
        let c = [200.0 / 255.0, (40.0 + 20.0 * i as f32) / 255.0, 40.0 / 255.0, 1.0];
        s.fill_rect(Rect::new(100 * i, 300, 100 * i + 100, 600), &photocraft_raster::from_rgba(&fmt, c));
    }
    let stroke = Stroke {
        brush: BrushSettings { size: 40.0, hardness: 1.0, spacing: 0.25, pressure_size: false, ..Default::default() },
        points: vec![StrokePoint::new(50.0, 450.0, 1.0), StrokePoint::new(750.0, 450.0, 1.0)],
    };
    let rs = ReplaceSettings {
        mode: ReplaceMode::Color,
        sampling: Sampling::Once,
        limits: Limits::Discontiguous,
        tolerance: tolerance / 100.0,
        anti_alias: true,
        color: [0.0, 0.0, 1.0, 1.0],
        ..Default::default()
    };
    apply_color_replacement(&mut s, &stroke, &rs, None, false);
    (0..8).map(|i| s.rgba(100 * i + 50, 450)).map(|c| [c[0] * 255.0, c[1] * 255.0, c[2] * 255.0]).collect()
}

fn untouched(i: usize) -> [f32; 3] {
    [200.0, 40.0 + 20.0 * i as f32, 40.0]
}

#[test]
fn the_tolerance_fades_over_its_whole_range() {
    // The reference's table; None = untouched. Tolerance 100's block 7 sits under the stroke's
    // last dab, which the reference's stroke reached fewer times than ours, so it isn't compared.
    let table: [(f32, [Option<[f32; 3]>; 7]); 4] = [
        (10.0, [Some([71.0, 71.0, 255.0]), Some([122.0, 74.0, 183.0]), None, None, None, None, None]),
        (30.0, [Some([71.0, 71.0, 255.0]), Some([85.0, 85.0, 254.0]), Some([102.0, 95.0, 242.0]), Some([141.0, 106.0, 181.0]), None, None, None]),
        (
            50.0,
            [
                Some([71.0, 71.0, 255.0]),
                Some([85.0, 85.0, 255.0]),
                Some([97.0, 96.0, 254.0]),
                Some([114.0, 110.0, 247.0]),
                Some([132.0, 120.0, 226.0]),
                Some([157.0, 137.0, 180.0]),
                Some([188.0, 156.0, 92.0]),
            ],
        ),
        (
            100.0,
            [
                Some([71.0, 71.0, 255.0]),
                Some([85.0, 85.0, 255.0]),
                Some([99.0, 99.0, 255.0]),
                Some([111.0, 111.0, 255.0]),
                Some([123.0, 123.0, 254.0]),
                Some([138.0, 137.0, 251.0]),
                Some([151.0, 149.0, 246.0]),
            ],
        ),
    ];
    for (tol, row) in table {
        let got = blocks_after(tol);
        for (i, want) in row.iter().enumerate() {
            let want = want.unwrap_or_else(|| untouched(i));
            let within = if row[i].is_some() { 6.0 } else { 0.5 };
            for k in 0..3 {
                assert!((got[i][k] - want[k]).abs() <= within, "tolerance {tol}, block {i}: {:?}, expected {want:?}", got[i]);
            }
        }
    }
    // Block 7 lies outside Tolerance 50 and below, and is still only partly replaced at 100.
    assert_eq!(blocks_after(50.0)[7].map(f32::round), untouched(7));
    let b7 = blocks_after(100.0)[7];
    assert!(b7[0] > 160.0 && b7[2] < 230.0, "tolerance 100, block 7 partly replaced: {b7:?}");
}

/// A click on the border between red above and block 1 below samples red: the red is replaced
/// fully, block 1 only partly (the reference: 67,67,255 and 157,68,119).
#[test]
fn a_border_click_partly_replaces_the_neighbouring_colour() {
    let fmt = PixelFormat::RGBA8;
    let mut s = Surface::new(fmt);
    s.fill_rect(Rect::new(0, 0, 200, 450), &photocraft_raster::from_rgba(&fmt, [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0]));
    s.fill_rect(Rect::new(0, 450, 200, 600), &photocraft_raster::from_rgba(&fmt, [200.0 / 255.0, 60.0 / 255.0, 40.0 / 255.0, 1.0]));
    let stroke = Stroke {
        brush: BrushSettings { size: 40.0, hardness: 1.0, spacing: 0.25, pressure_size: false, ..Default::default() },
        points: vec![StrokePoint::new(100.0, 449.5, 1.0)],
    };
    let rs = ReplaceSettings {
        mode: ReplaceMode::Color,
        sampling: Sampling::Continuous,
        limits: Limits::Contiguous,
        tolerance: 0.3,
        color: [0.0, 0.0, 1.0, 1.0],
        ..Default::default()
    };
    apply_color_replacement(&mut s, &stroke, &rs, None, false);
    let (red, below) = (s.rgba(100, 440).map(|v| v * 255.0), s.rgba(100, 455).map(|v| v * 255.0));
    assert!(red[0] < 75.0 && red[2] > 250.0, "red replaced fully: {red:?}");
    assert!(below[0] > 120.0 && below[0] < 199.0 && below[2] > 41.0, "block 1 partly replaced: {below:?}");
}
