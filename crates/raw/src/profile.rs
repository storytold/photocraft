//! Model-bound DNG camera calibration and rendering tables. This crate contains no profile assets.
use crate::{Calibration, RawError, Sensor, color};

#[path = "profile_parse.rs"]
mod parse;
#[path = "profile_table.rs"]
mod table;
pub use table::HueSatMap;

#[derive(Debug, Clone, PartialEq)]
pub struct CameraProfile {
    pub name: String,
    pub camera_model: String,
    pub calibrations: Vec<Calibration>,
    pub exposure_offset: f64,
    /// Linear ProPhoto tone curve with strictly increasing inputs. Clipped endpoint domains
    /// are allowed (some installed SDR profiles reach output white before input 1).
    pub tone_curve: Vec<[f64; 2]>,
    pub hue_sat_maps: Vec<HueSatMap>,
    pub look_table: Option<HueSatMap>,
    pub default_black_render: u32,
    /// DNG ProfileEmbedPolicy: 0/1 permit DNG input only; 2 is external-use/no embedding;
    /// 3 declares no profile-specific usage restrictions. Separate licence terms still apply.
    pub embed_policy: u32,
    pub copyright: String,
}

impl CameraProfile {
    pub fn from_dcp(bytes: &[u8]) -> Result<Self, RawError> {
        parse::parse(bytes)
    }
    pub fn validate(&self, sensor: &Sensor) -> Result<(), RawError> {
        if self.embed_policy <= 1 && sensor.format != crate::RawFormat::Dng {
            return Err(RawError::unsupported("this camera profile's usage policy permits DNG input only; non-DNG processing is disabled"));
        }
        if !sensor.model.as_deref().is_some_and(|m| camera_model_matches(&self.camera_model, sensor.make.as_deref().unwrap_or(""), m)) {
            return Err(RawError::unsupported("camera profile does not match the raw camera model"));
        }
        self.validate_data()
    }

    fn validate_data(&self) -> Result<(), RawError> {
        if self.embed_policy > 3 || self.copyright.len() > 4096 {
            return Err(RawError::unsupported("unknown camera profile usage policy or oversized copyright notice"));
        }
        if self.name.trim().is_empty() || self.name.len() > 128 || self.camera_model.trim().is_empty() || self.camera_model.len() > 128 {
            return Err(RawError::malformed("camera profile needs a bounded name and camera model"));
        }
        if !(1..=2).contains(&self.calibrations.len()) || !self.exposure_offset.is_finite() || self.exposure_offset.abs() > 5.0 {
            return Err(RawError::malformed("camera profile needs 1–2 illuminants and a finite exposure offset within ±5 EV"));
        }
        let mut previous = 0.0;
        for c in &self.calibrations {
            if !c.temperature.is_finite() || !(1500.0..=25000.0).contains(&c.temperature) || c.temperature <= previous {
                return Err(RawError::malformed("camera profile illuminants must have increasing finite temperatures"));
            }
            previous = c.temperature;
            for matrix in [&c.color_matrix, &c.camera_calibration].into_iter().chain(c.forward_matrix.as_ref()) {
                if matrix.iter().flatten().any(|v| !v.is_finite() || v.abs() > 32.0) || color::invert(matrix).is_none() {
                    return Err(RawError::malformed("camera profile contains an invalid or singular matrix"));
                }
            }
        }
        if self.calibrations.iter().any(|c| c.forward_matrix.is_some()) && self.calibrations.iter().any(|c| c.forward_matrix.is_none()) {
            return Err(RawError::malformed("camera profile must supply ForwardMatrix for all illuminants or none"));
        }
        if !self.tone_curve.is_empty() {
            if !(2..=4096).contains(&self.tone_curve.len())
                || self.tone_curve.first().is_none_or(|p| p[1] != 0.0)
                || self.tone_curve.last().is_none_or(|p| p[1] != 1.0)
            {
                return Err(RawError::malformed("camera profile tone curve needs 2–4096 points spanning black to white"));
            }
            let mut previous: Option<[f64; 2]> = None;
            for &point in &self.tone_curve {
                if point.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) || previous.is_some_and(|p| point[0] <= p[0]) {
                    return Err(RawError::malformed("camera profile tone curve must have finite, strictly increasing inputs"));
                }
                previous = Some(point);
            }
        }
        if self.hue_sat_maps.len() > 2 || (self.hue_sat_maps.len() == 2 && self.calibrations.len() != 2) || self.default_black_render > 1 {
            return Err(RawError::unsupported("unsupported camera profile rendering controls"));
        }
        for table in self.hue_sat_maps.iter().chain(self.look_table.iter()) {
            table.validate()?;
        }
        Ok(())
    }

    pub(crate) fn prepare(&self, color: &crate::ColorInfo, neutral: [f64; 3], gain: f32) -> table::Prepared<'_> {
        table::Prepared::new(self, color.weight(color::cct(color.neutral_to_xy(neutral))) as f32, gain)
    }
}

/// Camera model spelling is often prefixed with the make in DCP but not in EXIF (e.g. Sony).
/// Match the complete model, never a substring or a neighbouring/newer camera generation.
pub fn camera_model_matches(profile: &str, make: &str, model: &str) -> bool {
    let normalize = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let (profile, make, model) = (normalize(profile), normalize(make), normalize(model));
    !model.is_empty()
        && !profile.is_empty()
        && (profile == model
            || profile.strip_suffix(&model).is_some_and(|prefix| !prefix.is_empty() && make.starts_with(prefix))
            || model.strip_suffix(&profile).is_some_and(|prefix| !prefix.is_empty() && make.starts_with(prefix)))
}
