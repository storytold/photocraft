//! Original PhotoCraft artistic looks. These are parameterized tone/chroma
//! transforms, not Adobe camera calibrations, copied profiles or LUTs. Standard
//! preserves the existing rendering exactly; the other looks are opt-in.
use super::{band_weights, luma, rgb_to_hsv};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorProfile {
    #[default]
    Standard,
    Color,
    Vivid,
    Landscape,
    Portrait,
    Monochrome,
}

impl ColorProfile {
    pub const ALL: [Self; 6] = [Self::Standard, Self::Color, Self::Vivid, Self::Landscape, Self::Portrait, Self::Monochrome];
    pub fn id(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Color => "color",
            Self::Vivid => "vivid",
            Self::Landscape => "landscape",
            Self::Portrait => "portrait",
            Self::Monochrome => "monochrome",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }
    pub(super) fn apply(self, c: [f32; 3]) -> [f32; 3] {
        if self == Self::Standard {
            return c;
        }
        let l = luma(c);
        let hue = rgb_to_hsv(c.map(|v| v.max(0.0)))[0];
        let (sat, contrast) = match self {
            Self::Standard => (1.0, 0.0),
            Self::Color => (1.04, 0.04),
            Self::Vivid => (1.22, 0.18),
            Self::Landscape => {
                let bands = band_weights(hue);
                (1.12 + 0.10 * (bands[3] + bands[4] + bands[5]), 0.12)
            }
            Self::Portrait => {
                // A broad orange-hue weight, not face detection or a skin calibration.
                let orange = (1.0 - (hue - 25.0).abs() / 30.0).clamp(0.0, 1.0);
                (0.94 + 0.06 * orange, -0.06)
            }
            Self::Monochrome => (0.0, 0.10),
        };
        // Smooth bounded S-curve on display-encoded luminance. Preserve HDR
        // luminance above white and avoid a hard clip inside the look stage.
        let tone = if (0.0..=1.0).contains(&l) { l + contrast * l * (1.0 - l) * (2.0 * l - 1.0) } else { l };
        let gain = if l > 1e-6 { tone / l } else { 1.0 };
        c.map(|v| (l + (v - l) * sat) * gain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera_raw::{CameraRaw, develop};
    #[test]
    fn profiles_render_and_preserve_alpha_and_user_controls() {
        let source = [[0.25, 0.65, 0.85, 0.3], [0.7, 0.4, 0.2, 1.0], [2.0, 2.0, 2.0, 0.0]];
        let mut outputs = Vec::new();
        for profile in ColorProfile::ALL {
            let params = CameraRaw { profile, ..Default::default() };
            let mut pixels = source;
            develop(&mut pixels, 3, 1, &params, true);
            for (before, after) in source.iter().zip(pixels) {
                assert_eq!(before[3], after[3]);
                assert!(after.iter().all(|v| v.is_finite()));
            }
            assert_eq!(pixels[2], [2.0, 2.0, 2.0, 0.0]);
            assert_eq!(params.exposure, 0.0);
            assert_eq!(ColorProfile::from_id(profile.id()), Some(profile));
            if profile == ColorProfile::Standard {
                assert_eq!(pixels, source);
            }
            if profile == ColorProfile::Monochrome {
                for p in pixels {
                    assert_eq!(p[0], p[1]);
                    assert_eq!(p[1], p[2]);
                }
            }
            outputs.push(pixels[0]);
        }
        for (i, a) in outputs.iter().enumerate() {
            for b in &outputs[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
    #[test]
    fn old_settings_keep_standard_and_unknown_profiles_are_rejected() {
        let old: CameraRaw = serde_json::from_str("{\"exposure\":0.5}").unwrap();
        assert_eq!(old.profile, ColorProfile::Standard);
        assert!(serde_json::from_str::<CameraRaw>("{\"profile\":\"Adobe Color\"}").is_err());
        for profile in ColorProfile::ALL {
            let p = CameraRaw { profile, exposure: 0.5, tint: 12.0, ..Default::default() };
            assert_eq!(serde_json::from_value::<CameraRaw>(serde_json::to_value(&p).unwrap()).unwrap(), p);
        }
    }
}
