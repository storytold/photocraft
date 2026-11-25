//! Modern Brightness/Contrast at the ends of the Brightness slider (#14348), against the
//! reference application's 8-bit gray ramps.

use photocraft_compose::Buffer;
use photocraft_compose::adjust::apply;
use photocraft_doc::Adjustment;
use photocraft_geom::Rect;

/// Brightness -150, Contrast 0, Use Legacy off: output for gray inputs 0..=255.
const LOWERED: [u8; 256] = [
    0, 0, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4, 5, 5, 5, 6, 6, 7, 7, 7, 8, 8, 9, 9, 9, 10, 10, 10, 11, 11, 12, 12, 12, 13, 13, 14, 14, 14, 15, 15, 16, 16, 16, 17, 17,
    17, 18, 18, 19, 19, 19, 20, 20, 21, 21, 21, 22, 22, 23, 23, 23, 24, 24, 24, 25, 25, 26, 26, 26, 27, 27, 28, 28, 28, 29, 29, 30, 30, 30, 31, 31, 31, 32, 32,
    33, 33, 33, 34, 34, 35, 35, 35, 36, 36, 37, 37, 37, 38, 38, 38, 39, 39, 40, 40, 40, 41, 41, 42, 42, 42, 43, 43, 44, 44, 44, 45, 45, 45, 46, 46, 47, 47, 47,
    48, 48, 49, 49, 49, 50, 50, 51, 51, 51, 52, 52, 53, 53, 53, 54, 54, 55, 55, 55, 56, 56, 57, 57, 57, 58, 58, 59, 59, 60, 60, 60, 61, 61, 62, 62, 63, 63, 63,
    64, 64, 65, 65, 66, 66, 67, 67, 68, 68, 69, 69, 70, 70, 71, 71, 72, 72, 73, 73, 74, 74, 75, 75, 76, 76, 77, 78, 78, 79, 79, 80, 81, 81, 82, 83, 83, 84, 85,
    85, 86, 87, 87, 88, 89, 90, 90, 91, 92, 93, 94, 95, 96, 96, 97, 98, 99, 100, 101, 102, 103, 105, 106, 107, 108, 109, 111, 112, 113, 115, 116, 118, 120,
    121, 123, 125, 127, 129, 131, 133, 136, 139, 141, 145, 148, 152, 156, 161, 167, 174, 183, 195, 214, 255,
];

/// Brightness +150, Contrast 0, Use Legacy off: output for gray inputs 0..=255.
const RAISED: [u8; 256] = [
    0, 3, 5, 8, 10, 13, 15, 18, 21, 23, 26, 28, 31, 33, 36, 39, 41, 44, 46, 49, 51, 54, 57, 59, 62, 64, 67, 69, 72, 75, 77, 80, 82, 85, 87, 90, 93, 95, 98,
    100, 103, 106, 108, 111, 113, 116, 118, 121, 124, 126, 129, 131, 134, 136, 139, 141, 144, 146, 148, 151, 153, 155, 158, 160, 162, 164, 167, 169, 171, 173,
    175, 177, 179, 181, 183, 184, 186, 188, 190, 191, 193, 195, 196, 198, 199, 201, 202, 203, 205, 206, 207, 209, 210, 211, 212, 213, 214, 216, 217, 218, 219,
    220, 221, 222, 222, 223, 224, 225, 226, 227, 227, 228, 229, 230, 230, 231, 232, 232, 233, 234, 234, 235, 235, 236, 237, 237, 238, 238, 239, 239, 240, 240,
    240, 241, 241, 242, 242, 242, 243, 243, 244, 244, 244, 244, 245, 245, 245, 246, 246, 246, 247, 247, 247, 247, 247, 248, 248, 248, 248, 249, 249, 249, 249,
    249, 250, 250, 250, 250, 250, 250, 250, 251, 251, 251, 251, 251, 251, 251, 251, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 253, 253, 253, 253, 253,
    253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254,
    254, 254, 254, 254, 254, 254, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
];

/// The 8-bit gray ramp 0..=255 through modern Brightness/Contrast, read back as levels.
fn ramp_through(brightness: f32, contrast: f32) -> Vec<i32> {
    let px = (0..=255u8).map(|x| f32::from(x) / 255.0).map(|v| [v, v, v, 1.0]).collect();
    let mut b = Buffer { rect: Rect::new(0, 0, 256, 1), px };
    apply(&Adjustment::BrightnessContrast { brightness, contrast, legacy: false }, &mut b);
    b.px.iter().map(|p| (p[0] * 255.0).round() as i32).collect()
}

/// At -150 and +150 every input lands within a level of the reference, and black and white stay
/// put; near white the lowered curve climbs steeply (254 -> 214) where it used to stop at 191.
#[test]
fn brightness_150_either_way_matches_the_reference_ramp() {
    for (brightness, want) in [(-150.0, &LOWERED), (150.0, &RAISED)] {
        let got = ramp_through(brightness, 0.0);
        let off: Vec<_> = got.iter().zip(want.iter()).enumerate().filter(|(_, (g, w))| (**g - i32::from(**w)).abs() > 1).collect();
        assert!(off.is_empty(), "brightness {brightness}: (input, (got, reference)) off by more than a level: {off:?}");
        assert_eq!((got[0], got[255]), (0, 255), "brightness {brightness} keeps black and white");
    }
}

/// The 8-bit gray ramp 0..=255 through modern Brightness at Contrast 0, as unrounded outputs.
fn ramp_levels(brightness: f32) -> Vec<f32> {
    let px = (0..=255u8).map(|x| f32::from(x) / 255.0).map(|v| [v, v, v, 1.0]).collect();
    let mut b = Buffer { rect: Rect::new(0, 0, 256, 1), px };
    apply(&Adjustment::BrightnessContrast { brightness, contrast: 0.0, legacy: false }, &mut b);
    b.px.iter().map(|p| p[0] * 255.0).collect()
}

/// Over the top of the slider, where the curve eases into the +150 fit, no input gets darker as
/// Brightness goes up (input 200 used to fall from 253.55 at +134 to 253.32 at +150), and none
/// gets lighter as it goes down.
#[test]
fn each_input_follows_the_slider_up_to_150_either_way() {
    for sign in [1.0f32, -1.0] {
        let mut prev = ramp_levels(sign * 100.0);
        for b in 101..=150u8 {
            let brightness = sign * f32::from(b);
            let got = ramp_levels(brightness);
            let wrong: Vec<_> = (0..=255usize)
                .filter_map(|x| Some((x, *prev.get(x)?, *got.get(x)?)))
                .filter(|&(_, before, now)| if sign > 0.0 { now < before - 1e-4 } else { now > before + 1e-4 })
                .collect();
            assert!(wrong.is_empty(), "brightness {brightness}: (input, one step before, now) moved against the slider: {wrong:?}");
            prev = got;
        }
    }
}
