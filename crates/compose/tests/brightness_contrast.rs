//! Black-box Photoshop 26.11.6 measurements, RGB/8, generated sRGB gray ramp.
//! Each pixel is (x, x, x), x=0..255. Measurements taken through PS Generator Bridge.
//! The issue's (64,64,64), brightness 75, contrast -17 must become (105,105,105).

use photocraft_compose::{Buffer, adjust};
use photocraft_doc::Adjustment;
use photocraft_geom::Rect;

const PHOTOSHOP: [(f32, f32, [u8; 256]); 3] = [
    (
        75.0,
        -17.0,
        [
            0, 2, 4, 5, 7, 9, 11, 13, 14, 16, 18, 20, 21, 23, 25, 27, 28, 30, 32, 33, 35, 37, 39, 40, 42, 44, 45, 47, 49, 50, 52, 54, 55, 57, 59, 60, 62, 63,
            65, 67, 68, 70, 71, 73, 75, 76, 78, 79, 81, 82, 84, 86, 87, 89, 90, 92, 93, 95, 96, 98, 99, 101, 102, 104, 105, 107, 108, 110, 111, 113, 114, 115,
            117, 118, 120, 121, 123, 124, 125, 127, 128, 130, 131, 132, 134, 135, 136, 138, 139, 141, 142, 143, 144, 146, 147, 148, 150, 151, 152, 153, 155,
            156, 157, 158, 160, 161, 162, 163, 165, 166, 167, 168, 169, 170, 172, 173, 174, 175, 176, 177, 178, 179, 180, 182, 183, 184, 185, 186, 187, 188,
            189, 190, 191, 192, 193, 194, 195, 196, 197, 198, 199, 199, 200, 201, 202, 203, 204, 205, 206, 207, 207, 208, 209, 210, 211, 212, 212, 213, 214,
            215, 215, 216, 217, 218, 218, 219, 220, 221, 221, 222, 223, 223, 224, 225, 225, 226, 227, 227, 228, 228, 229, 230, 230, 231, 231, 232, 233, 233,
            234, 234, 235, 235, 236, 236, 237, 237, 238, 238, 239, 239, 240, 240, 240, 241, 241, 242, 242, 243, 243, 243, 244, 244, 244, 245, 245, 246, 246,
            246, 247, 247, 247, 248, 248, 248, 248, 249, 249, 249, 250, 250, 250, 250, 251, 251, 251, 251, 252, 252, 252, 252, 252, 253, 253, 253, 253, 253,
            254, 254, 254, 254, 254, 254, 255, 255, 255, 255,
        ],
    ),
    (
        75.0,
        0.0,
        [
            0, 2, 3, 5, 6, 8, 10, 11, 13, 14, 16, 18, 19, 21, 22, 24, 26, 27, 29, 30, 32, 34, 35, 37, 38, 40, 42, 43, 45, 47, 48, 50, 51, 53, 55, 56, 58, 59,
            61, 63, 64, 66, 67, 69, 71, 72, 74, 75, 77, 79, 80, 82, 83, 85, 87, 88, 90, 91, 93, 95, 96, 98, 99, 101, 103, 104, 106, 107, 109, 111, 112, 114,
            115, 117, 119, 120, 122, 124, 125, 127, 128, 130, 131, 133, 135, 136, 138, 139, 141, 142, 144, 145, 147, 148, 149, 151, 152, 154, 155, 156, 158,
            159, 160, 162, 163, 164, 166, 167, 168, 169, 171, 172, 173, 174, 175, 177, 178, 179, 180, 181, 182, 183, 185, 186, 187, 188, 189, 190, 191, 192,
            193, 194, 195, 196, 197, 198, 199, 200, 201, 202, 203, 203, 204, 205, 206, 207, 208, 209, 209, 210, 211, 212, 213, 214, 214, 215, 216, 217, 217,
            218, 219, 220, 220, 221, 222, 222, 223, 224, 224, 225, 226, 226, 227, 227, 228, 229, 229, 230, 230, 231, 232, 232, 233, 233, 234, 234, 235, 235,
            236, 236, 237, 237, 238, 238, 239, 239, 239, 240, 240, 241, 241, 242, 242, 242, 243, 243, 243, 244, 244, 245, 245, 245, 246, 246, 246, 247, 247,
            247, 247, 248, 248, 248, 249, 249, 249, 249, 250, 250, 250, 250, 251, 251, 251, 251, 252, 252, 252, 252, 252, 253, 253, 253, 253, 253, 253, 254,
            254, 254, 254, 254, 254, 254, 255, 255, 255, 255,
        ],
    ),
    (
        0.0,
        -17.0,
        [
            0, 1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41,
            42, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79,
            80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 92, 93, 94, 95, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 106, 107, 108, 109, 110,
            111, 112, 113, 114, 115, 116, 116, 117, 118, 119, 120, 121, 122, 123, 124, 124, 125, 126, 127, 128, 129, 130, 131, 131, 132, 133, 134, 135, 136,
            137, 138, 139, 139, 140, 141, 142, 143, 144, 145, 146, 147, 148, 149, 149, 150, 151, 152, 153, 154, 155, 156, 157, 158, 159, 160, 161, 162, 163,
            163, 164, 165, 166, 167, 168, 169, 170, 171, 172, 173, 174, 175, 176, 177, 178, 179, 180, 181, 182, 183, 184, 185, 186, 187, 188, 189, 190, 191,
            192, 193, 194, 195, 196, 197, 198, 199, 200, 201, 202, 203, 204, 205, 206, 207, 208, 209, 210, 211, 213, 214, 215, 216, 217, 218, 219, 220, 221,
            222, 223, 224, 225, 226, 227, 228, 230, 231, 232, 233, 234, 235, 236, 237, 238, 239, 240, 242, 243, 244, 245, 246, 247, 248, 249, 250, 252, 253,
            254, 255,
        ],
    ),
];

/// Matches all 256 RGB/8 gray-ramp samples to the measured Photoshop settings,
/// including the issue's brightness 75 / contrast -17 rounding regression.
#[test]
fn modern_brightness_contrast_matches_photoshop_ramps_exactly() {
    for (brightness, contrast, expected) in PHOTOSHOP {
        let mut buffer = Buffer { rect: Rect::new(0, 0, 256, 1), px: (0..256).map(|x| [x as f32 / 255.0, x as f32 / 255.0, x as f32 / 255.0, 1.0]).collect() };
        adjust::apply(&Adjustment::BrightnessContrast { brightness, contrast, legacy: false }, &mut buffer);
        for (x, (pixel, expected)) in buffer.to_rgba8().pixels.chunks_exact(4).zip(expected).enumerate() {
            assert_eq!(pixel, &[expected, expected, expected, 255], "b={brightness} c={contrast} input={x}");
        }
    }
}

/// A neutral modern adjustment must preserve negative and above-white F32 samples
/// and alpha exactly, rather than clipping through the lookup table's 0..1 domain.
#[test]
fn neutral_modern_brightness_contrast_preserves_float_range() {
    use photocraft_color::SampleType;
    let mut buffer = Buffer { rect: Rect::new(0, 0, 1, 1), px: vec![[2.0, -0.25, 1.5, 0.4]] };
    let before = buffer.clone();
    adjust::apply_depth(
        &Adjustment::BrightnessContrast { brightness: 0.0, contrast: 0.0, legacy: false },
        &mut buffer,
        adjust::Transfer::Gamma(1.0),
        Some(SampleType::F32),
    );
    assert_eq!(buffer, before);
}

/// Checks the measured RGB/8 result for a partially transparent pixel while
/// preserving its alpha and leaving a fully transparent pixel's hidden RGB intact.
#[test]
fn modern_brightness_contrast_preserves_alpha_and_transparent_rgb() {
    let source = [[64.0 / 255.0, 33.0 / 255.0, 0.0, 0.4], [0.2, 0.3, 0.4, 0.0]];
    let mut buffer = Buffer { rect: Rect::new(0, 0, 2, 1), px: source.to_vec() };
    adjust::apply(&Adjustment::BrightnessContrast { brightness: 75.0, contrast: -17.0, legacy: false }, &mut buffer);
    assert_eq!(&buffer.to_rgba8().pixels[..4], &[105, 57, 0, 102]);
    assert_eq!(buffer.px[0][3], source[0][3]);
    assert_eq!(buffer.px[1], source[1]);
}

// RGB/16 PNG samples (stored as 0..65535); Photoshop internally uses 0..32768.
const INPUT16: [u16; 17] = [0, 4096, 8192, 12288, 16384, 20480, 24576, 28672, 32768, 36863, 40959, 45055, 49151, 53247, 57343, 61439, 65535];
const PHOTOSHOP16: [(f32, f32, [u16; 17]); 11] = [
    (25.0, 0.0, [0, 4794, 9590, 14384, 19180, 23974, 28770, 33563, 38307, 42923, 47337, 51477, 55269, 58645, 61529, 63849, 65535]),
    (50.0, 0.0, [0, 5612, 11226, 16838, 22452, 28064, 33671, 38983, 43835, 48223, 52141, 55587, 58553, 61037, 63031, 64531, 65535]),
    (75.0, 0.0, [0, 6570, 13142, 19712, 26282, 32853, 38993, 44335, 48933, 52841, 56113, 58809, 60981, 62683, 63973, 64905, 65535]),
    (100.0, 0.0, [0, 7692, 15384, 23076, 30766, 38119, 44333, 49473, 53649, 56973, 59551, 61495, 62915, 63919, 64617, 65119, 65535]),
    (-50.0, 0.0, [0, 2990, 5978, 8968, 11956, 14946, 17934, 20924, 23912, 26996, 30294, 33863, 37791, 42213, 47399, 53999, 65535]),
    (0.0, -50.0, [0, 5458, 10526, 15206, 19496, 23398, 26910, 30034, 32768, 35501, 38625, 42137, 46039, 50329, 55009, 60077, 65535]),
    (0.0, -17.0, [0, 4560, 8986, 13280, 17442, 21472, 25370, 29136, 32768, 36399, 40165, 44063, 48093, 52255, 56549, 60975, 65535]),
    (0.0, 25.0, [0, 3416, 7024, 10828, 14828, 19020, 23408, 27992, 32768, 37543, 42127, 46515, 50707, 54707, 58511, 62119, 65535]),
    (0.0, 50.0, [0, 2734, 5858, 9370, 13272, 17562, 22242, 27310, 32768, 38225, 43293, 47973, 52263, 56165, 59677, 62801, 65535]),
    (0.0, 100.0, [0, 1372, 3522, 6452, 10158, 14644, 19906, 25948, 32768, 39587, 45629, 50891, 55377, 59083, 62013, 64163, 65535]),
    (75.0, -17.0, [0, 7250, 14158, 20726, 26954, 32841, 38341, 43369, 47873, 51835, 55247, 58119, 60473, 62347, 63781, 64825, 65535]),
];

/// Compares RGB/16 ramps to Photoshop within two internal 1/32768 steps,
/// guarding against byte quantization without claiming exact RGB/16 parity.
#[test]
fn modern_brightness_contrast_retains_16_bit_precision() {
    use photocraft_color::SampleType;
    for (brightness, contrast, expected) in PHOTOSHOP16 {
        let mut buffer =
            Buffer { rect: Rect::new(0, 0, 17, 1), px: INPUT16.iter().map(|&x| [x as f32 / 65535.0, x as f32 / 65535.0, x as f32 / 65535.0, 1.0]).collect() };
        adjust::apply_depth(
            &Adjustment::BrightnessContrast { brightness, contrast, legacy: false },
            &mut buffer,
            adjust::Transfer::Srgb,
            Some(SampleType::U16),
        );
        for (&pixel, expected) in buffer.px.iter().zip(expected) {
            assert!((pixel[0] - expected as f32 / 65535.0).abs() <= 2.0 / 32768.0, "b={brightness} c={contrast} pixel={pixel:?} PS={expected}");
            assert_eq!(pixel[3], 1.0);
        }
    }
}

/// Checks LUT monotonicity and exact black/white endpoints across slider extremes.
/// Non-finite slider inputs must leave the directly evaluated samples unchanged.
#[test]
fn modern_brightness_contrast_curves_are_monotone_and_pin_endpoints() {
    for brightness in [-150.0, -125.0, -75.0, 0.0, 75.0, 125.0, 150.0] {
        for contrast in [-50.0, -17.0, 0.0, 100.0] {
            let table = adjust::modern_lut(brightness, contrast);
            assert_eq!(table.first(), Some(&0.0));
            assert_eq!(table.last(), Some(&1.0));
            assert!(table.windows(2).all(|p| p[0].is_finite() && p[0] <= p[1]));
        }
    }
    for x in [0.0, 0.25, 0.5, 0.75, 1.0] {
        assert_eq!(adjust::modern_brightness(x, f32::NAN), x);
        assert_eq!(adjust::modern_contrast(x, f32::INFINITY), x);
    }
}

/// Matches a clipped adjustment's rendered RGB/8 ramp to the Photoshop reference,
/// exercising document compositing in addition to direct buffer adjustment.
#[test]
fn clipped_brightness_contrast_render_matches_photoshop() {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{Document, Layer, LayerContent, Size};
    use photocraft_raster::Surface;
    let mut doc = Document::new("clipped ramp", Size::new(256, 1), ColorMode::Rgb, SampleType::U8);
    let mut surface = Surface::new(PixelFormat::RGBA8);
    let bytes: Vec<u8> = (0..=255).flat_map(|x| [x, x, x, 255]).collect();
    surface.write_interleaved(doc.bounds(), &bytes);
    doc.layers.push(Layer::new("base", LayerContent::Raster(surface)));
    let mut adj = Layer::new("brightness", LayerContent::Adjustment(Adjustment::BrightnessContrast { brightness: 75.0, contrast: -17.0, legacy: false }));
    adj.clipped = true;
    doc.layers.push(adj);
    let actual = photocraft_compose::render(&doc, doc.bounds()).to_rgba8();
    for (pixel, expected) in actual.pixels.chunks_exact(4).zip(PHOTOSHOP[0].2) {
        assert_eq!(pixel, &[expected, expected, expected, 255]);
    }
}
