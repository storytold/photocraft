//! A Grayscale document's gray value ↔ CIE L* relation, sampled from its gray profile.

use std::sync::{Arc, OnceLock};

use craft_cms::{Builtin, ColorSpace, Profile, TransformOptions};
use photocraft_color::convert::{lab_to_srgb, srgb_to_lab};

/// Samples in the gray → L* table (an 8-bit gray falls within 1/16 of a level of one).
const SAMPLES: usize = 4096;

/// How a Grayscale document's stored gray relates to CIE L* (D50, relative colorimetric with
/// black point compensation), from the gray profile the document carries. Untagged documents,
/// and profiles that aren't gray or don't convert, relate them through sRGB: the reference
/// application's untagged Grayscale Gradient Map matches that within a level (photoshop corpus
/// gray8 and gray16 gradient-map.psd, saved without a profile), where reading through a Dot Gain
/// curve is off by about six levels there.
#[derive(Debug, Clone, PartialEq)]
pub struct GrayLightness {
    /// L* / 100 at gray `i / (SAMPLES - 1)`, never decreasing; `None`: sRGB.
    l: Option<Vec<f32>>,
}

impl GrayLightness {
    /// The relation for a document tagged with `icc` (`None`: untagged).
    pub fn for_profile(icc: Option<&[u8]>) -> Arc<Self> {
        icc.and_then(|b| Profile::parse(b).ok())
            .filter(|p| p.color_space == ColorSpace::Gray)
            .and_then(|p| Self::sample(&p))
            .map_or_else(|| Arc::clone(Self::untagged()), Arc::new)
    }

    /// The relation of an untagged Grayscale document (sRGB).
    pub fn untagged() -> &'static Arc<Self> {
        static UNTAGGED: OnceLock<Arc<GrayLightness>> = OnceLock::new();
        UNTAGGED.get_or_init(|| Arc::new(Self { l: None }))
    }

    fn sample(profile: &Profile) -> Option<Self> {
        let t = craft_cms::cached(profile, Builtin::LabD50.profile(), TransformOptions::default()).ok()?;
        let mut out = [0.0f32; 3];
        let mut l = Vec::with_capacity(SAMPLES);
        for i in 0..SAMPLES {
            t.eval(&[i as f32 / (SAMPLES - 1) as f32], &mut out).ok()?;
            l.push(out[0]);
        }
        // Both directions read this one table, so it must be finite and rise from black to white.
        let usable = l.iter().all(|v| v.is_finite()) && l.windows(2).all(|w| w[1] >= w[0]) && l[SAMPLES - 1] > l[0];
        usable.then_some(Self { l: Some(l) })
    }

    /// L* / 100 of stored gray `g` (0–1).
    pub fn lightness(&self, g: f32) -> f32 {
        let g = g.clamp(0.0, 1.0);
        let Some(l) = &self.l else {
            return srgb_to_lab([g; 3])[0] / 100.0;
        };
        let x = g * (SAMPLES - 1) as f32;
        let i = (x as usize).min(SAMPLES - 2);
        let f = x - i as f32;
        l[i] + (l[i + 1] - l[i]) * f
    }

    /// The stored gray (0–1) whose L* / 100 is `l`: the inverse of [`Self::lightness`].
    pub fn gray(&self, lightness: f32) -> f32 {
        let Some(l) = &self.l else {
            return lab_to_srgb([lightness * 100.0, 0.0, 0.0])[0].clamp(0.0, 1.0);
        };
        let (lo, hi) = (l[0], l[SAMPLES - 1]);
        if lightness <= lo {
            return 0.0;
        }
        if lightness >= hi {
            return 1.0;
        }
        // First sample at or above `lightness`; the one before it is below.
        let j = l.partition_point(|&v| v < lightness).clamp(1, SAMPLES - 1);
        let (a, b) = (l[j - 1], l[j]);
        let f = if b > a { (lightness - a) / (b - a) } else { 0.0 };
        ((j - 1) as f32 + f) / (SAMPLES - 1) as f32
    }

    /// Where a stored gray sits on a Gradient Map: the sRGB gray of the same L*, so a black to
    /// white gradient written back through [`Self::encode`] returns the gray it read. An
    /// untagged document's gray is its own position.
    pub(super) fn position(&self, g: f32) -> f32 {
        if self.l.is_none() {
            return g;
        }
        lab_to_srgb([self.lightness(g) * 100.0, 0.0, 0.0])[0].clamp(0.0, 1.0)
    }

    /// A Gradient Map colour as this document stores it: its L* through the gray profile, or
    /// the neutral sRGB colour of that L* when untagged.
    pub(super) fn encode(&self, c: [f32; 3]) -> [f32; 3] {
        let l = srgb_to_lab(c)[0];
        if self.l.is_none() {
            return [lab_to_srgb([l, 0.0, 0.0])[0]; 3];
        }
        [self.gray(l / 100.0); 3]
    }
}
