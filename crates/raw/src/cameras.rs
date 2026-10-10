//! Camera profiles measured by PhotoCraft, for raw formats that carry no colour description of
//! their own (DNG does; Nikon NEF does not).
//!
//! Clean-room: every number here comes from our own measurements of the camera's output, never
//! from another converter's tables. A profile gives the image area inside the sensor data (the
//! camera's own JPEG crop), the saturation level, a ForwardMatrix (white-balanced camera → XYZ D50,
//! DNG semantics) and a default tone curve (like DNG ProfileToneCurve).
//!
//! **Nikon D4** (148 uncompressed NEFs from one body, 2023–2026, ISO 100–10000): the sensor data is
//! 4992 × 3292; the camera's full-size JPEG (4928 × 3280) sits at (8, 6), found by aligning the two
//! (2 px of optical black on the left, 50 + 8 on the right show as a purple stripe otherwise). The
//! data is already black-subtracted (the darkest image pixels reach 0). Green saturates at
//! 15546–15672 while red and blue reach 15687–16383, so one level of 15520 makes clipped
//! highlights white without shifting the white balance. Matrix and curve were fitted to the
//! camera's embedded JPEGs (Picture Control Standard, Active D-Lighting off; 52 frames, smooth
//! areas, 4 × 4 means): on 15 held-out frames the development is 7.2/255 per channel off the
//! camera's JPEG on average, against 34 without a profile.
//!
//! Bodies whose full profile is not measured yet can still get their measured image area — the
//! same alignment method, see [`IMAGE_AREAS`] — so their frames crop like the camera's JPEG.

use crate::color::{Calibration, ColorInfo, IDENTITY, Mat3};
use crate::sensor::{Rect, Sensor};

const D4_FORWARD: Mat3 = [[0.644598, 0.0703598, 0.249262], [0.342249, 0.518929, 0.138822], [0.0568265, -0.269568, 1.03795]];
const D4_COLOR: Mat3 = [[0.895936, -0.218092, -0.185988], [-1.09736, 2.06898, -0.0131889], [-0.26977, 0.393884, 0.687829]];
const D4_TONE: [[f32; 2]; 31] = [
    [0.005524272, 0.00285028],
    [0.006569503, 0.00357083],
    [0.0078125, 0.00447912],
    [0.009290681, 0.00605813],
    [0.01104854, 0.00841445],
    [0.01313901, 0.011536],
    [0.015625, 0.0165399],
    [0.01858136, 0.0230234],
    [0.02209709, 0.0319205],
    [0.02627801, 0.0434858],
    [0.03125, 0.0582138],
    [0.03716272, 0.0771963],
    [0.04419417, 0.0993894],
    [0.05255603, 0.125058],
    [0.0625, 0.157207],
    [0.07432544, 0.194929],
    [0.08838835, 0.236109],
    [0.1051121, 0.279155],
    [0.125, 0.329365],
    [0.1486509, 0.388405],
    [0.1767767, 0.457944],
    [0.2102241, 0.52456],
    [0.25, 0.598051],
    [0.2973018, 0.668794],
    [0.3535534, 0.749954],
    [0.4204482, 0.815443],
    [0.5, 0.870167],
    [0.5946036, 0.907869],
    [std::f32::consts::FRAC_1_SQRT_2, 0.939351],
    [0.8408964, 0.970567],
    [1.0, 1.0],
];

struct Profile {
    make: &'static str,
    model: &'static str,
    /// Sensor data size the profile was measured on.
    size: (usize, usize),
    /// The image area: x, y, width, height.
    crop: (usize, usize, usize, usize),
    white: f32,
    /// The black level is known to be 0 (the data is black-subtracted in camera).
    black_zero: bool,
    forward: Mat3,
    color: Mat3,
    tone: &'static [[f32; 2]],
}

const PROFILES: &[Profile] = &[Profile {
    make: "NIKON CORPORATION",
    model: "NIKON D4",
    size: (4992, 3292),
    crop: (8, 6, 4928, 3280),
    white: 15520.0,
    black_zero: true,
    forward: D4_FORWARD,
    color: D4_COLOR,
    tone: &D4_TONE,
}];

/// A measured image area of a body whose remaining profile has not been
/// measured yet. Measured the same way as the profiles: by aligning the decoded
/// plane with the camera's own JPEG.
///
/// Both origins are even, so the Bayer phase of the crop is unchanged.
/// * **D800E** (2 files): the camera's JPEG sits 8 x 6 pixels in from the left
///   and top of the 7424 x 4924 sensor data (56 and 4 pixels of margin on the
///   right and bottom), the same corner as the D4.
/// * **ILCE-7** (2 files): 12 x 12 pixels in from the left and top of the
///   6048 x 4024 cRAW data (36 and 12 pixels of margin on the right and bottom).
struct ImageArea {
    make: &'static str,
    model: &'static str,
    /// Sensor data size the measurement was taken on.
    size: (usize, usize),
    /// The image area: x, y, width, height.
    crop: (usize, usize, usize, usize),
}

const IMAGE_AREAS: &[ImageArea] = &[
    ImageArea { make: "NIKON CORPORATION", model: "NIKON D800E", size: (7424, 4924), crop: (8, 6, 7360, 4912) },
    ImageArea { make: "SONY", model: "ILCE-7", size: (6048, 4024), crop: (12, 12, 6000, 4000) },
];

/// Applies the measured profile of the camera that made `s`, when there is one,
/// and its measured image area when the profile is not measured yet.
pub(crate) fn apply(s: &mut Sensor) {
    let (Some(make), Some(model)) = (s.make.as_deref(), s.model.as_deref()) else { return };
    let (make, model) = (make.trim(), model.trim());
    if let Some(a) = IMAGE_AREAS.iter().find(|a| a.make == make && a.model == model && a.size == (s.width, s.height)) {
        let (x, y, w, h) = a.crop;
        s.crop = Rect::new(x, y, w, h);
    }
    let Some(p) = PROFILES.iter().find(|p| make == p.make && model == p.model && (s.width, s.height) == p.size) else { return };
    let (x, y, w, h) = p.crop;
    s.crop = Rect::new(x, y, w, h);
    s.white = [p.white; 3];
    if p.black_zero {
        s.warnings.retain(|w| !w.contains("black level not recorded"));
    }
    s.color = ColorInfo {
        // D50 daylight: the matrices are consistent at the camera's daylight neutral.
        calibrations: vec![Calibration { temperature: 5003.0, color_matrix: p.color, forward_matrix: Some(p.forward), camera_calibration: IDENTITY }],
        ..ColorInfo::default()
    };
    s.tone_curve = p.tone.to_vec();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RawFormat;
    use crate::color::apply as mat_apply;
    use crate::sensor::BlackLevels;

    fn sensor(make: &str, model: &str, width: usize, height: usize) -> Sensor {
        Sensor {
            format: RawFormat::Nef,
            make: Some(make.into()),
            model: Some(model.into()),
            width,
            height,
            samples: 1,
            data: Vec::new(),
            cfa: None,
            linearization: None,
            black: BlackLevels::uniform(0.0),
            white: [16383.0; 3],
            active: Rect::new(0, 0, width, height),
            crop: Rect::new(0, 0, width, height),
            color: ColorInfo::default(),
            camera_wb: Some([2.0, 1.0, 1.4]),
            orientation: 1,
            baseline_exposure: 0.0,
            gain_maps: Vec::new(),
            tone_curve: Vec::new(),
            warnings: vec!["NEF: black level not recorded in a documented tag; assumed 0".into()],
        }
    }

    fn nef(model: &str, width: usize, height: usize) -> Sensor {
        sensor("NIKON CORPORATION", model, width, height)
    }

    #[test]
    fn a_d4_nef_gets_its_crop_levels_matrix_and_curve() {
        let mut s = nef("NIKON D4", 4992, 3292);
        apply(&mut s);
        assert_eq!(s.crop, Rect::new(8, 6, 4928, 3280));
        assert_eq!(s.white, [15520.0; 3]);
        assert_eq!(s.color.calibrations.len(), 1);
        assert_eq!(s.color.calibrations[0].forward_matrix, Some(D4_FORWARD));
        assert_eq!(s.tone_curve.len(), D4_TONE.len());
        assert!(s.warnings.is_empty(), "the D4's black level is known: {:?}", s.warnings);
        // The profile gives a colour conversion for the as-shot white balance.
        assert!(s.color.balanced_to_xyz_d50([0.5, 1.0, 0.7]).is_some());
    }

    #[test]
    fn measured_image_areas_crop_the_borders_without_touching_the_rest() {
        let mut s = nef("NIKON D800E", 7424, 4924);
        apply(&mut s);
        assert_eq!(s.crop, Rect::new(8, 6, 7360, 4912));
        // Only the image area is measured: levels, colour and tone stay untouched.
        assert!(s.color.calibrations.is_empty() && s.tone_curve.is_empty());
        assert_eq!(s.white, [16383.0; 3]);

        let mut s = sensor("SONY", "ILCE-7", 6048, 4024);
        apply(&mut s);
        assert_eq!(s.crop, Rect::new(12, 12, 6000, 4000));

        // Another size or model of the same maker is left alone.
        let mut s = nef("NIKON D800E", 2496, 1646);
        apply(&mut s);
        assert_eq!(s.crop, Rect::new(0, 0, 2496, 1646));
        let mut s = nef("NIKON D800", 7424, 4924);
        apply(&mut s);
        assert_eq!(s.crop, Rect::new(0, 0, 7424, 4924));
    }

    #[test]
    fn other_cameras_and_sizes_are_left_alone() {
        for (model, w, h) in [("NIKON D4S", 4992, 3292), ("NIKON D4", 2496, 1646), ("NIKON D800", 7424, 4924)] {
            let mut s = nef(model, w, h);
            apply(&mut s);
            assert_eq!(s.crop, Rect::new(0, 0, w, h), "{model} {w}x{h}");
            assert!(s.tone_curve.is_empty() && s.color.calibrations.is_empty() && s.white == [16383.0; 3]);
            assert_eq!(s.warnings.len(), 1);
        }
    }

    #[test]
    fn d4_forward_matrix_maps_neutral_to_d50_and_curve_reaches_white() {
        let n = mat_apply(&D4_FORWARD, [1.0; 3]);
        assert!((n[0] - 0.9642).abs() < 1e-3 && (n[1] - 1.0).abs() < 1e-3 && (n[2] - 0.8252).abs() < 1e-3, "{n:?}");
        assert!(D4_TONE.windows(2).all(|w| w[1][0] > w[0][0] && w[1][1] >= w[0][1]), "increasing");
        assert_eq!(D4_TONE.last().map(|p| (p[0], p[1])), Some((1.0, 1.0)));
    }
}
