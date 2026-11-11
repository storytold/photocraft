//! Edit › Color Settings › Spot drives the compositor's application-wide spot dot gain
//! (`photocraft_compose::multichannel::spot_dot_gain`), which the ink preview of Multichannel
//! documents adds to every channel.
//!
//! This test has its own integration-test binary (its own process) for the same reason as
//! `text_gamma.rs`: changing the gain changes how every Multichannel document composites, so a
//! concurrent test could flatten one document under two settings. Keep any other test that
//! changes the spot dot gain in this file.

use photocraft_cms::Builtin;
use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_compose::multichannel::{InkLight, InkSpace, SPOT_DOT_GAIN, inks_in, spot_dot_gain};
use photocraft_doc::{AlphaChannel, Document, Size};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::json;

/// A Multichannel document with one transparent black ink at 50 %.
fn half_tint(s: &mut Session) {
    s.execute("file.new", json!({"width": 2, "height": 2, "mode": "grayscale", "background": "white"})).unwrap();
    s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
    s.execute("image.mode.multichannel", json!({})).unwrap();
}

#[test]
fn spot_dot_gain_setting() {
    let mut s = Session::new();
    assert_eq!(s.color.settings.spot_dot_gain, SPOT_DOT_GAIN, "Photoshop's default Spot working space: Dot Gain 20 %");
    for bad in [json!(-1), json!(50), json!("20%"), json!(true)] {
        assert!(s.execute("edit.colorSettings", json!({"spotDotGain": bad})).is_err(), "{bad}");
    }
    assert_eq!(spot_dot_gain(), SPOT_DOT_GAIN, "a refused request changes nothing");

    // The ink preview follows the setting: a 50 % dot darkens as the gain rises.
    half_tint(&mut s);
    let paper = |s: &Session| photocraft_compose::flatten(&s.active().unwrap().doc).px[0][0];
    let r = s.execute("edit.colorSettings", json!({"spotDotGain": 0})).unwrap();
    assert_eq!(r["settings"]["spotDotGain"], 0.0);
    assert_eq!(spot_dot_gain(), 0.0);
    let linear = paper(&s);
    s.execute("edit.colorSettings", json!({"spotDotGain": 38})).unwrap();
    assert_eq!(spot_dot_gain(), 38.0);
    let gained = paper(&s);
    assert!(gained < linear - 0.2, "38 % gain prints the 50 % dot much darker: {linear} → {gained}");

    // `prefs.set` and `prefs.reset` on `colorSettings` move the compositor's gain too. Same
    // function, not a second test: tests in one binary run concurrently.
    let in_sync = |s: &Session| spot_dot_gain() == s.color.settings.spot_dot_gain;
    s.execute("prefs.set", json!({"path": "colorSettings.spotDotGain", "value": 25})).unwrap();
    assert_eq!(spot_dot_gain(), 25.0);
    assert!(in_sync(&s));
    s.execute("prefs.reset", json!({"path": "colorSettings.spotDotGain"})).unwrap();
    assert_eq!(spot_dot_gain(), SPOT_DOT_GAIN);
    assert!(in_sync(&s));
    for reset in [json!({"path": "colorSettings"}), json!({})] {
        s.execute("edit.colorSettings", json!({"spotDotGain": 30})).unwrap();
        s.execute("prefs.reset", reset).unwrap();
        assert_eq!(spot_dot_gain(), SPOT_DOT_GAIN);
        assert!(in_sync(&s));
    }
    // Only colour changes move the process-wide gain: another session's unrelated
    // preferences leave this session's value alone.
    s.execute("edit.colorSettings", json!({"spotDotGain": 30})).unwrap();
    let mut other = Session::new();
    other.execute("prefs.set", json!({"path": "unitsAndRulers.pointSize", "value": "traditional"})).unwrap();
    other.execute("prefs.reset", json!({"path": "unitsAndRulers"})).unwrap();
    assert_eq!(spot_dot_gain(), 30.0);
    s.execute("prefs.reset", json!({"path": "colorSettings.spotDotGain"})).unwrap();

    // Saved with the preferences and applied when they load.
    s.execute("edit.colorSettings", json!({"spotDotGain": 38})).unwrap();
    let saved = s.prefs_to_json();
    photocraft_compose::multichannel::set_spot_dot_gain(SPOT_DOT_GAIN);
    let mut again = Session::new();
    again.load_prefs_json(&saved).unwrap();
    assert_eq!(again.color.settings.spot_dot_gain, 38.0);
    assert_eq!(spot_dot_gain(), 38.0);

    // A saved gain out of range is reported and reset, so later Color Settings edits still pass.
    let mut bad: serde_json::Value = serde_json::from_str(&saved).unwrap();
    bad["colorSettings"]["spotDotGain"] = json!(80);
    let mut third = Session::new();
    let invalid = third.load_prefs_json(&bad.to_string()).unwrap();
    assert!(invalid.iter().any(|p| p.path() == "colorSettings.spotDotGain"), "{:?}", invalid.iter().map(|p| p.path()).collect::<Vec<_>>());
    assert_eq!(third.color.settings.spot_dot_gain, SPOT_DOT_GAIN);
    assert_eq!(spot_dot_gain(), SPOT_DOT_GAIN);
    third.execute("edit.colorSettings", json!({"intent": "relative"})).unwrap();

    // Same function, not a second test: the merges read the process-wide gain.
    merges_print_the_inks_as_the_reference_does();
    photocraft_compose::multichannel::set_spot_dot_gain(SPOT_DOT_GAIN);
}

/// Spot channels of a chart, one pixel per patch (columns: coverage 0, 10 … 100 %): rows 0–4 one
/// ink #3366cc at solidity 0, 30, 50, 80 and 100 %; rows 5–6 "Red" #dd2211 and then "Yellow"
/// #ffcc00, both at 50 %, overlapping (row 5: Red at the column's coverage under solid Yellow;
/// row 6: solid Red under Yellow at the column's coverage).
fn chart_channels(depth: SampleType) -> Vec<AlphaChannel> {
    let (w, h) = (11usize, 7usize);
    let hex = |v: u32| Color::rgb(((v >> 16) & 255) as f32 / 255.0, ((v >> 8) & 255) as f32 / 255.0, (v & 255) as f32 / 255.0);
    let col = |c: usize| c as f32 / 10.0;
    let mut out = Vec::new();
    let mut channel = |name: &str, color: u32, solidity: f32, cover: &dyn Fn(usize, usize) -> f32| {
        let vals: Vec<f32> = (0..w * h).map(|i| cover(i / w, i % w)).collect();
        let mut s = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
        s.write_region(Rect::new(0, 0, w as i32, h as i32), &vals);
        out.push(AlphaChannel { spot: Some((hex(color), solidity)), ..AlphaChannel::new(name, s) });
    };
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
    out
}

/// D-G7: Image › Mode › RGB out of Multichannel prints the inks as the reference editor's Merge
/// Spot Channels (Adobe RGB, Spot dot gain 38 %), and Merge Spot Channel prints what the canvas
/// preview shows.
fn merges_print_the_inks_as_the_reference_does() {
    photocraft_compose::multichannel::set_spot_dot_gain(38.0);
    let mut s = Session::new();
    let mut doc = Document::new("chart", Size::new(11, 7), ColorMode::Multichannel, SampleType::U8);
    doc.channels = chart_channels(SampleType::U8);
    s.add_document(doc, None);
    s.execute("image.mode.rgb", json!({"profile": "adobe-rgb-compat"})).unwrap();
    let got = photocraft_compose::flatten(&s.active().unwrap().doc);
    // Patch centres of the reference's merge: (row, column, RGB). Over white the solidity makes
    // no difference (rows 0 and 4); the second ink prints over the first (rows 5 and 6).
    for (r, c, want) in [
        (0, 5, [106, 132, 211]),
        (4, 5, [106, 132, 211]),
        (2, 1, [221, 225, 242]),
        (3, 8, [56, 104, 204]),
        (4, 10, [51, 102, 204]),
        (5, 3, [244, 171, 1]),
        (6, 3, [233, 125, 10]),
        (6, 10, [239, 150, 1]),
    ] {
        let p = got.px[r * 11 + c];
        for k in 0..3 {
            assert!((p[k] * 255.0 - want[k] as f32).abs() <= 1.5, "row {r} column {c}: {:?} vs {want:?}", p.map(|v| v * 255.0));
        }
    }

    // The inks mix in the light of the profile asked for, else of the working RGB (sRGB by
    // default).
    let mut chart = Document::new("chart", Size::new(11, 7), ColorMode::Multichannel, SampleType::U8);
    chart.channels = chart_channels(SampleType::U8);
    let printed = |light: InkLight| inks_in(&chart, chart.bounds(), &InkSpace::of(&chart).in_light(light));
    let same = |a: &[[f32; 4]], b: &[[f32; 4]]| a.iter().zip(b).all(|(x, y)| x.iter().zip(y).all(|(u, v)| (u - v).abs() <= 0.6 / 255.0));
    assert!(same(&got.px, &printed(InkLight::of_profile(Builtin::AdobeRgbCompat.profile()).unwrap()).px));
    let mut s = Session::new();
    s.add_document(chart.clone(), None);
    s.execute("image.mode.rgb", json!({})).unwrap();
    let srgb = photocraft_compose::flatten(&s.active().unwrap().doc);
    assert!(same(&srgb.px, &printed(InkLight::srgb()).px));
    assert!(!same(&srgb.px, &got.px), "a different light gives different patches");

    // Merge Spot Channel, channel by channel in panel order, ends where the preview of all the
    // inks over the image is (16 bits, so the in-between merges add no rounding).
    let mut doc = Document::with_background("merge", Size::new(11, 7), ColorMode::Rgb, SampleType::U16, Color::rgb(1.0, 1.0, 1.0));
    doc.icc_profile = Some(Builtin::AdobeRgbCompat.profile().to_bytes());
    doc.channels = chart_channels(SampleType::U16);
    let mut preview = photocraft_compose::flatten(&doc);
    photocraft_compose::multichannel::overprint(&mut preview, &doc, |_| true);
    let mut s = Session::new();
    s.add_document(doc, None);
    for _ in 0..7 {
        s.execute("channel.mergeSpot", json!({"channel": 0})).unwrap();
    }
    let merged = photocraft_compose::flatten(&s.active().unwrap().doc);
    assert!(s.active().unwrap().doc.channels.is_empty());
    for (k, (m, p)) in merged.px.iter().zip(&preview.px).enumerate() {
        assert!(m.iter().zip(p).all(|(a, b)| (a - b).abs() <= 1e-3), "pixel {k}: merged {m:?}, preview {p:?}");
    }
}
