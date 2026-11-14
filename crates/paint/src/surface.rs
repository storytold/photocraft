//! Surface-aware brush: paint that takes on the photo's shading, and dabs that follow a
//! perspective plane.
//!
//! `paint` cannot see the imaging crates, so the two ingredients arrive as traits the engine
//! implements: a [`ShadeSource`] gives a per-pixel shading factor S (the pixel's luminance over a
//! blurred copy of it) and a [`SurfaceMap`] gives the local projection of a perspective plane.

use std::sync::Arc;

use photocraft_geom::{Point, Rect};
use serde::{Deserialize, Serialize};

/// Which pixels the shading is measured from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SampleMode {
    /// The visible composite of all layers.
    #[default]
    AllLayers,
    /// Only the layer being painted on.
    CurrentLayer,
}

/// Valid range of [`SurfaceOptions::detail`] (the blur radius in pixels).
pub const DETAIL_RANGE: std::ops::RangeInclusive<f32> = 1.0..=512.0;

/// The brush's Surface options: all off by default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SurfaceOptions {
    /// Surface Color: the paint takes on the photo's shading.
    pub color: bool,
    /// 0..1: how much of the shading is applied.
    pub strength: f32,
    /// Blur radius (σ, pixels) of the reference the shading is measured against: folds and
    /// shadows finer than this show through the paint. 1..=512.
    pub detail: f32,
    /// Where the shading is read from.
    pub sample: SampleMode,
    /// Follow Perspective: dabs are sized and foreshortened by the perspective plane under them.
    pub perspective: bool,
    /// Follow Folds: dabs are foreshortened and shifted by the slope and height of the photo's
    /// shading (a rough height map), so a stroke bends over folds. `detail` sets its scale too.
    pub folds: bool,
    /// 0..1: how strongly a slope foreshortens the dab.
    pub fold_depth: f32,
    /// 0..1: how far a fold's height shifts the dab.
    pub fold_bend: f32,
}

impl Default for SurfaceOptions {
    fn default() -> Self {
        Self { color: false, strength: 1.0, detail: 24.0, sample: SampleMode::AllLayers, perspective: false, folds: false, fold_depth: 0.5, fold_bend: 0.3 }
    }
}

impl SurfaceOptions {
    /// Reject non-finite or out-of-range values.
    pub fn validate(&self) -> Result<(), String> {
        if !self.strength.is_finite() || !(0.0..=1.0).contains(&self.strength) {
            return Err(format!("surface strength must be between 0 and 1, got {}", self.strength));
        }
        if !self.detail.is_finite() || !DETAIL_RANGE.contains(&self.detail) {
            return Err(format!("surface detail must be between {} and {}, got {}", DETAIL_RANGE.start(), DETAIL_RANGE.end(), self.detail));
        }
        for (name, v) in [("fold depth", self.fold_depth), ("fold bend", self.fold_bend)] {
            if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                return Err(format!("surface {name} must be between 0 and 1, got {v}"));
            }
        }
        Ok(())
    }
}

/// Per-pixel shading of the photo under a stroke.
pub trait ShadeSource: Send + Sync {
    /// Fill `out` (row-major, `rect.width() * rect.height()` values) with the shading factor S of
    /// each pixel of `rect`, already clamped to `0..=4`. 1 means "no change". Pixels the source
    /// does not cover are 1.
    fn shade(&self, rect: Rect, out: &mut [f32]);
}

/// The shading factor at `strength`: `1 + strength * (s - 1)`. A non-finite result is 1.
#[inline]
pub fn apply_strength(s: f32, strength: f32) -> f32 {
    let v = 1.0 + strength * (s - 1.0);
    if v.is_finite() { v } else { 1.0 }
}

/// The local projection of a perspective plane at one image point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Local {
    /// Image pixels per metric unit along the plane's major axis (σ1).
    pub scale: f32,
    /// Direction (radians, counter-clockwise, y up, like [`Dab::proj_angle`](crate::Dab::proj_angle))
    /// of the minor axis: the direction along which a dab is foreshortened.
    pub squash_angle: f32,
    /// σ2 / σ1 in `(0, 1]`: how much the minor axis is shortened.
    pub squash: f32,
    /// Image pixels (y down) the dab centre moves by (Follow Folds' bend); `[0, 0]` for a plane.
    pub offset: [f32; 2],
}

/// A perspective plane (or planes) the brush can follow.
pub trait SurfaceMap: Send + Sync {
    /// The projection at image point `p`, or `None` off every plane.
    fn local(&self, p: Point) -> Option<Local>;
}

impl Local {
    /// Is this a usable projection (finite, positive scale)?
    pub fn is_valid(&self) -> bool {
        self.scale.is_finite() && self.scale > 0.0 && self.offset.iter().all(|v| v.is_finite())
    }
}

/// A dab's `(radius, proj_angle, proj_scale)` on a plane.
///
/// `base_radius` is the radius the dab would have off the plane (size and dynamics applied);
/// `anchor_scale` is [`Local::scale`] at the stroke's first on-plane dab, `None` for that first dab
/// itself. The first dab keeps `base_radius`, so the brush size means pixels where the stroke
/// starts; later dabs grow and shrink with the plane's scale. The brush cursor uses this too, so
/// it shows the first dab exactly.
pub fn surface_dab_shape(base_radius: f32, anchor_scale: Option<f32>, local: &Local) -> (f32, f32, f32) {
    let ratio = match anchor_scale {
        Some(a) if a.is_finite() && a > 0.0 && local.is_valid() => (local.scale / a).clamp(1e-3, 1e3),
        _ => 1.0,
    };
    let radius = (base_radius * ratio).max(0.5);
    let angle = if local.squash_angle.is_finite() { local.squash_angle } else { 0.0 };
    let squash = if local.squash.is_finite() { local.squash.clamp(0.05, 1.0) } else { 1.0 };
    (radius, angle, squash)
}

/// A shade source and its strength, as a stroke holds them.
#[derive(Clone)]
pub(crate) struct Shade {
    pub(crate) source: Arc<dyn ShadeSource>,
    pub(crate) strength: f32,
}

impl std::fmt::Debug for Shade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shade").field("strength", &self.strength).finish_non_exhaustive()
    }
}

/// A surface map held by a dab generator.
#[derive(Clone)]
pub(crate) struct MapHandle(pub(crate) Arc<dyn SurfaceMap>);

impl std::fmt::Debug for MapHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MapHandle")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strength_blends_towards_one() {
        assert_eq!(apply_strength(2.0, 0.0), 1.0);
        assert_eq!(apply_strength(2.0, 1.0), 2.0);
        assert_eq!(apply_strength(0.5, 0.5), 0.75);
        assert_eq!(apply_strength(f32::NAN, 1.0), 1.0);
        assert_eq!(apply_strength(2.0, f32::NAN), 1.0);
        assert_eq!(apply_strength(f32::INFINITY, 1.0), 1.0);
    }

    #[test]
    fn options_validate() {
        assert!(SurfaceOptions::default().validate().is_ok());
        assert!(SurfaceOptions { strength: f32::NAN, ..Default::default() }.validate().is_err());
        assert!(SurfaceOptions { strength: 1.5, ..Default::default() }.validate().is_err());
        assert!(SurfaceOptions { detail: 0.5, ..Default::default() }.validate().is_err());
        assert!(SurfaceOptions { detail: 600.0, ..Default::default() }.validate().is_err());
        assert!(SurfaceOptions { detail: f32::INFINITY, ..Default::default() }.validate().is_err());
        for bad in [f32::NAN, -0.1, 1.5, f32::INFINITY] {
            assert!(SurfaceOptions { fold_depth: bad, ..Default::default() }.validate().is_err());
            assert!(SurfaceOptions { fold_bend: bad, ..Default::default() }.validate().is_err());
        }
    }

    #[test]
    fn options_default_from_empty_json() {
        let o: SurfaceOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(o, SurfaceOptions::default());
        let o: SurfaceOptions = serde_json::from_str(r#"{"color":true,"sample":"currentLayer"}"#).unwrap();
        assert!(o.color && o.sample == SampleMode::CurrentLayer);
    }

    #[test]
    fn dab_shape_anchors_at_first_dab() {
        let l = Local { scale: 2.0, squash_angle: 0.5, squash: 0.4, offset: [0.0; 2] };
        let (r, a, s) = surface_dab_shape(10.0, None, &l);
        assert_eq!((r, a, s), (10.0, 0.5, 0.4));
        // Same place, anchored: unchanged. Half the scale: half the radius.
        assert_eq!(surface_dab_shape(10.0, Some(2.0), &l).0, 10.0);
        let far = Local { scale: 1.0, ..l };
        assert_eq!(surface_dab_shape(10.0, Some(2.0), &far).0, 5.0);
        // Garbage never gives a non-finite dab.
        let bad = Local { scale: f32::NAN, squash_angle: f32::NAN, squash: f32::NAN, offset: [0.0; 2] };
        let (r, a, s) = surface_dab_shape(10.0, Some(2.0), &bad);
        assert_eq!((r, a, s), (10.0, 0.0, 1.0));
    }
}
