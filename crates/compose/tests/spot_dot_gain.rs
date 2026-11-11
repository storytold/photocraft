//! Edit › Color Settings › Spot drives the ink preview of Multichannel documents and the saved
//! thumbnail. The setting is process-wide (`multichannel::set_spot_dot_gain`), so this test has its
//! own binary, and it is one test: changing it here can't race other tests that render inks.

use photocraft_cms::Builtin;
use photocraft_color::convert::linear_to_srgb;
use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_compose::multichannel::{InkLight, InkSpace, SPOT_DOT_GAIN, dot_gain, inks_in, set_spot_dot_gain, spot_dot_gain};
use photocraft_doc::{AlphaChannel, Document, Size};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

fn half_tint_doc() -> Document {
    let mut doc = Document::new("m", Size::new(2, 1), ColorMode::Multichannel, SampleType::F32);
    let mut ink = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    ink.write_region(Rect::new(0, 0, 2, 1), &[0.5, 1.0]);
    // A black, transparent ink: the printed value is 1 − coverage on every channel.
    doc.channels.push(AlphaChannel { spot: Some((Color::rgb(0.0, 0.0, 0.0), 0.0)), ..AlphaChannel::new("Black", ink) });
    doc
}

#[test]
fn spot_dot_gain_darkens_midtones_of_the_ink_preview() {
    assert_eq!(spot_dot_gain(), SPOT_DOT_GAIN, "Photoshop's default Spot working space");
    let doc = half_tint_doc();
    // The same ink as a spot channel of an RGB document, printed into its saved thumbnail.
    let mut rgb = Document::new("r", Size::new(2, 1), ColorMode::Rgb, SampleType::F32);
    rgb.channels = doc.channels.clone();
    for gain in [0.0, 20.0, 38.0] {
        set_spot_dot_gain(gain);
        let r = photocraft_compose::flatten(&doc);
        // The dot prints at 50 % + the gain and takes that share of the light away (mixed in
        // linear light; these documents are untagged, so sRGB's).
        let expect = linear_to_srgb(1.0 - (0.5 + gain / 100.0));
        assert!((r.px[0][0] - expect).abs() < 1e-4, "{gain} %: a 50 % dot prints at {} %, got {:?}", 50.0 + gain, r.px[0]);
        assert!(r.px[1][0].abs() < 1e-6, "a solid stays solid at {gain} %");
        assert!((r.px[0][0] - linear_to_srgb(1.0 - dot_gain(0.5, gain))).abs() < 1e-5);
        let t = photocraft_compose::multichannel::thumbnail_with_inks(&rgb, 160);
        assert!((t.px[0][0] - expect).abs() < 1e-4, "{gain} %: the thumbnail's 50 % dot prints at {} %, got {:?}", 50.0 + gain, t.px[0]);
        assert!(t.px[1][0].abs() < 1e-6, "the thumbnail's solid stays solid at {gain} %");
    }
    set_spot_dot_gain(f32::NAN);
    assert_eq!(spot_dot_gain(), SPOT_DOT_GAIN, "a non-finite value falls back to the default");
    set_spot_dot_gain(80.0);
    assert_eq!(spot_dot_gain(), photocraft_compose::multichannel::MAX_SPOT_DOT_GAIN);
    // Same function, not a second test: tests in one binary run concurrently.
    reference_spot_chart();
    set_spot_dot_gain(SPOT_DOT_GAIN);
}

/// The reference editor's Merge Spot Channels of a spot chart (Adobe RGB document, Color Settings ›
/// Spot at 38 % dot gain), sampled at each patch's centre: rows 0–4 are one ink #3366cc at
/// solidity 0, 30, 50, 80 and 100 % over eleven coverages 0, 10 … 100 %; in rows 5 and 6 two inks
/// at 50 % solidity overlap, "Red" #dd2211 first in the Channels panel and "Yellow" #ffcc00 after
/// it: row 5 has Red at the column's coverage under solid Yellow, row 6 solid Red under Yellow at
/// the column's coverage.
const REFERENCE_CHART: [[[u8; 3]; 11]; 7] = {
    const BLUE: [[u8; 3]; 11] = [
        [255, 255, 255],
        [221, 225, 242],
        [189, 198, 231],
        [160, 173, 223],
        [132, 151, 216],
        [106, 132, 211],
        [85, 119, 208],
        [68, 109, 206],
        [56, 104, 204],
        [52, 102, 204],
        [51, 102, 204],
    ];
    [
        BLUE,
        BLUE,
        BLUE,
        BLUE,
        BLUE,
        [
            [255, 204, 1],
            [251, 191, 1],
            [247, 180, 1],
            [244, 171, 1],
            [242, 163, 1],
            [241, 158, 1],
            [240, 154, 1],
            [239, 152, 1],
            [239, 150, 1],
            [239, 150, 1],
            [239, 150, 1],
        ],
        [
            [221, 34, 17],
            [226, 88, 15],
            [230, 111, 12],
            [233, 125, 10],
            [235, 135, 8],
            [237, 142, 6],
            [238, 146, 4],
            [238, 148, 3],
            [239, 149, 2],
            [239, 150, 1],
            [239, 150, 1],
        ],
    ]
};

/// The chart as a Multichannel document, one pixel per patch: seven 8-bit spot channels in panel
/// order (Blue at five solidities, Red, Yellow).
fn chart() -> Document {
    let (w, h) = (11usize, 7usize);
    let mut doc = Document::new("chart", Size::new(w as u32, h as u32), ColorMode::Multichannel, SampleType::U8);
    let hex = |v: u32| Color::rgb(((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0);
    let mut channel = |name: &str, color: u32, solidity: f32, cover: &dyn Fn(usize, usize) -> f32| {
        let vals: Vec<f32> = (0..w * h).map(|i| cover(i / w, i % w)).collect();
        let mut s = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
        s.write_region(Rect::new(0, 0, w as i32, h as i32), &vals);
        doc.channels.push(AlphaChannel { spot: Some((hex(color), solidity)), ..AlphaChannel::new(name, s) });
    };
    let col = |c: usize| c as f32 / 10.0;
    for (row, solidity) in [0.0, 0.3, 0.5, 0.8, 1.0].into_iter().enumerate() {
        channel("Blue", 0x3366cc, solidity, &|r, c| if r == row { col(c) } else { 0.0 });
    }
    channel("Red", 0xdd2211, 0.5, &|r, c| match r {
        5 => col(c),
        6 => 1.0,
        _ => 0.0,
    });
    channel("Yellow", 0xffcc00, 0.5, &|r, c| match r {
        5 => 1.0,
        6 => col(c),
        _ => 0.0,
    });
    doc
}

/// The inks mixed in Adobe RGB's light with a 38 % spot dot gain reproduce the reference's
/// merge: every channel of every patch within one level. (The engine's curve, `1 − (1 − v)^γ`,
/// is within about 1 % of the reference's Textile 38 % dot gain table.)
fn reference_spot_chart() {
    set_spot_dot_gain(38.0);
    let doc = chart();
    let adobe = InkLight::of_profile(Builtin::AdobeRgbCompat.profile()).unwrap();
    let got = inks_in(&doc, doc.bounds(), &InkSpace::of(&doc).in_light(adobe));
    let (mut sum, mut worst) = (0.0f32, 0.0f32);
    for (r, row) in REFERENCE_CHART.iter().enumerate() {
        for (c, want) in row.iter().enumerate() {
            let p = got.px[r * 11 + c];
            for k in 0..3 {
                let d = (p[k] * 255.0 - f32::from(want[k])).abs();
                assert!(d <= 1.5, "row {r} column {c} channel {k}: {} vs {}", p[k] * 255.0, want[k]);
                sum += d;
                worst = worst.max(d);
            }
        }
    }
    let mean = sum / (7.0 * 11.0 * 3.0);
    assert!(mean <= 0.5, "mean {mean} levels, worst {worst}");
}
