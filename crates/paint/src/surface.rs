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
    /// Follow Depth: dabs are foreshortened by the slope of the document's depth channel, and
    /// (with `size_by_depth`) sized by it.
    pub depth: bool,
    /// 0..1: how strongly a depth slope foreshortens the dab.
    pub relief: f32,
    /// Far surfaces get smaller dabs (the first dab keeps the brush size).
    pub size_by_depth: bool,
    /// Name of the alpha channel holding the depth (white = near).
    pub depth_channel: String,
}

impl Default for SurfaceOptions {
    fn default() -> Self {
        Self {
            color: false,
            strength: 1.0,
            detail: 24.0,
            sample: SampleMode::AllLayers,
            perspective: false,
            depth: false,
            relief: 0.5,
            size_by_depth: true,
            depth_channel: "Depth".to_string(),
        }
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
        if !self.relief.is_finite() || !(0.0..=1.0).contains(&self.relief) {
            return Err(format!("surface relief must be between 0 and 1, got {}", self.relief));
        }
        if self.depth_channel.is_empty() || self.depth_channel.chars().count() > 255 {
            return Err("surface depth channel must be a channel name of 1 to 255 characters".to_string());
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

    /// The scale (in [`Local::scale`] units) at which the brush size is meant: the same for every
    /// stroke and for the brush cursor, so a stroke is as large at a place wherever it started.
    /// `None` (the default) anchors each stroke at its own first on-surface dab instead.
    fn reference_scale(&self) -> Option<f32> {
        None
    }

    /// The dab's outline bent over the surface at the (undisplaced) centre `p`, for a dab of
    /// `radius` image pixels whose projection is `local`; `None` (the default) keeps the ellipse
    /// of `local`. Only a round computed tip follows it; it costs far more than
    /// [`local`](Self::local), so the spacing never asks.
    fn footprint(&self, _p: Point, _radius: f32, _local: &Local) -> Option<Footprint> {
        None
    }
}

/// Number of directions of a [`Footprint`].
pub const FOOTPRINT_RAYS: usize = 32;

/// A dab's outline in image space: the distance from its centre to the edge along
/// [`FOOTPRINT_RAYS`] evenly spaced directions (ray `k` at angle `k · 2π / 32`, counter-clockwise,
/// y up, like the projection angles), size scaling included. Every ray is in
/// `(0, radius]` of the dab it belongs to. Held inline (128 bytes) so a [`Dab`](crate::Dab) stays `Copy`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Footprint {
    pub rays: [f32; FOOTPRINT_RAYS],
}

impl Footprint {
    /// Is every ray finite and positive?
    pub fn is_valid(&self) -> bool {
        self.rays.iter().all(|r| r.is_finite() && *r > 0.0)
    }

    /// The outline distance at `theta` (radians, y up): the linear interpolation of the two
    /// nearest rays.
    pub fn radius_at(&self, theta: f32) -> f32 {
        let t = theta.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU * FOOTPRINT_RAYS as f32;
        let k = (t.floor() as usize).min(FOOTPRINT_RAYS - 1);
        let f = (t - k as f32).clamp(0.0, 1.0);
        let at = |i: usize| self.rays.get(i % FOOTPRINT_RAYS).copied().unwrap_or(1.0);
        at(k) * (1.0 - f) + at(k + 1) * f
    }

    /// The tip-space point of the pixel offset `(ux, uy)` (y up) from the centre of a dab of
    /// `radius`: the offset stretched so the outline lands on the circle of `radius`. A tip
    /// painted at the result has the outline of this footprint.
    pub fn to_tip(&self, ux: f32, uy: f32, radius: f32) -> (f32, f32) {
        let d = ux.hypot(uy);
        if d <= 1e-6 {
            return (ux, uy);
        }
        let edge = self.radius_at(uy.atan2(ux)).clamp(radius * 0.01, radius.max(0.01));
        let k = radius / edge;
        (ux * k, uy * k)
    }
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
/// `anchor_scale` is the scale the brush size is meant at: [`SurfaceMap::reference_scale`] when
/// the map has one, else [`Local::scale`] at the stroke's first on-surface dab (`None` for that
/// first dab itself, which then keeps `base_radius`). A dab grows and shrinks with the surface's
/// scale relative to it. The brush cursor uses this too, so it shows the dab at that point.
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
            assert!(SurfaceOptions { relief: bad, ..Default::default() }.validate().is_err());
        }
        assert!(SurfaceOptions { depth_channel: String::new(), ..Default::default() }.validate().is_err());
        assert!(SurfaceOptions { depth_channel: "x".repeat(256), ..Default::default() }.validate().is_err());
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
