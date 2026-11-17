//! #14267: Convert to Profile from sRGB to the built-in Coated FOGRA39 gives the reference
//! application's inks for each intent, and Perceptual and Saturation separate differently.

use super::*;

/// Inks (CMYK %) the reference application gives for an sRGB patch, per intent: relative +
/// BPC, relative, perceptual, saturation, absolute (Convert to Profile, dither on).
const REFERENCE: [(&str, [[f32; 4]; 5]); 3] = [
    ("#c81e1e", [[14.5, 97.7, 95.3, 4.3], [13.3, 100.0, 100.0, 5.5], [18.0, 96.9, 92.9, 8.6], [14.9, 100.0, 100.0, 5.9], [8.6, 100.0, 100.0, 2.4]]),
    ("#1e3cc8", [[91.8, 75.3, 0.0, 0.0], [94.5, 76.5, 0.0, 0.0], [94.5, 77.7, 0.0, 0.0], [93.7, 76.9, 0.0, 0.0], [92.6, 74.9, 0.0, 0.0]]),
    ("#808080", [[49.0, 38.4, 38.4, 20.0], [49.8, 39.2, 38.8, 21.2], [49.0, 38.4, 38.4, 20.0], [49.0, 38.4, 38.4, 20.0], [46.7, 36.9, 38.4, 18.4]]),
];

const INTENTS: [(&str, bool); 5] = [("relative", true), ("relative", false), ("perceptual", true), ("saturation", true), ("absolute", false)];

/// The inks of an 8-bit sRGB document filled with `background`, converted to Coated FOGRA39.
fn separate(background: &str, intent: &str, bpc: bool) -> [f32; 4] {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": background})).unwrap();
    s.execute("edit.assignProfile", json!({"profile": "srgb"})).unwrap();
    s.execute("edit.convertToProfile", json!({"profile": "coated-fogra39", "intent": intent, "bpc": bpc, "dither": false})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.mode, ColorMode::Cmyk);
    let px = doc.layers[0].surface().unwrap().pixel(4, 4);
    [px[0], px[1], px[2], px[3]].map(|v| v * 100.0)
}

/// Within 2.5 % per ink: most land within 2 %; red at the magenta–yellow edge falls between
/// clipped and unclipped nodes of the profile's tables, which costs up to 2.2 %.
#[test]
fn srgb_to_fogra39_matches_the_reference_inks() {
    let mut worst = (0.0f32, String::new());
    for (background, want) in REFERENCE {
        for ((intent, bpc), want) in INTENTS.into_iter().zip(want) {
            let got = separate(background, intent, bpc);
            let diff = (0..4).map(|i| (got[i] - want[i]).abs()).fold(0.0f32, f32::max);
            if diff > worst.0 {
                worst = (diff, format!("{background} {intent} bpc {bpc}: {got:.1?}, reference {want:?}"));
            }
        }
    }
    assert!(worst.0 <= 2.5, "{} (max {:.1})", worst.1, worst.0);
}

#[test]
fn perceptual_and_saturation_separate_differently() {
    let (perceptual, saturation) = (separate("#c81e1e", "perceptual", true), separate("#c81e1e", "saturation", true));
    assert!(saturation[1] > perceptual[1] + 1.0 && saturation[2] > perceptual[2] + 1.0, "perceptual {perceptual:?}, saturation {saturation:?}");
}
