//! Free Warp, Distort and Perspective on live type ("Better Than PS"): Photoshop rasterizes type
//! (or refuses) for these, PhotoCraft keeps them on the type layer as
//! [`TypeDistortion`]s, so the text stays editable and re-renders under them.
//!
//! Each distortion is stored in the document space of the moment it was made (its `frame` is the
//! layer's `transform` then). `M = transform · frame⁻¹` takes that space to today's document
//! space, so a later move, scale or rotation carries the distortion along.

use photocraft_algo::transform::Homography;
use photocraft_doc::TextLayer;
use photocraft_doc::text::{MAX_TYPE_DISTORTIONS, TypeDistortion, TypeDistortionKind};
use photocraft_geom::Affine;
use photocraft_geom::warp::{Warp, WarpStyle};

use crate::{EngineError, Result};

fn hom(a: &Affine) -> Homography {
    let [a0, b0, c0, d0, e0, f0] = a.m;
    Homography([a0, c0, e0, b0, d0, f0, 0.0, 0.0, 1.0])
}

/// `transform · frame⁻¹`: the distortion's space → today's document space.
fn to_doc(t: &TextLayer, d: &TypeDistortion) -> Option<Affine> {
    Some(t.transform.mul(&d.frame.inverse()?))
}

fn too_many() -> EngineError {
    EngineError::Other(format!("a type layer keeps at most {MAX_TYPE_DISTORTIONS} free transforms; rasterize it to go on"))
}

/// Distort / Perspective on type: the document-space projective map `h` goes onto the type,
/// merged into a trailing Distort (exact: both are projective maps).
pub fn push_projective(t: &mut TextLayer, h: &Homography) -> Result<()> {
    if !h.0.iter().all(|v| v.is_finite()) || h.inverse().is_none() {
        return Err(EngineError::Other("the transform collapses the type".into()));
    }
    if let Some(last) = t.distortions.last()
        && let TypeDistortionKind::Projective(p) = &last.kind
        && let Some(m) = to_doc(t, last)
        && let Some(mi) = hom(&m).inverse()
    {
        // In the old frame: M⁻¹ · h · M after the existing map.
        let merged = mi.mul(h).mul(&hom(&m)).mul(&Homography(*p));
        let d = TypeDistortion { frame: last.frame, kind: TypeDistortionKind::Projective(merged.0) };
        if d.is_valid()
            && let Some(slot) = t.distortions.last_mut()
        {
            *slot = d;
            return Ok(());
        }
    }
    if t.distortions.len() >= MAX_TYPE_DISTORTIONS {
        return Err(too_many());
    }
    let d = TypeDistortion { frame: t.transform, kind: TypeDistortionKind::Projective(h.0) };
    if !d.is_valid() {
        return Err(EngineError::Other("the type's transform can't be inverted".into()));
    }
    t.distortions.push(d);
    Ok(())
}

/// What Edit › Transform › Warp edits on a type layer.
#[derive(Clone, Debug, PartialEq)]
pub enum WarpTarget {
    /// No free transforms yet: the Warp Text style (a custom mesh replaces it).
    Style,
    /// The trailing free warp, shown in today's document space.
    Last(Warp),
    /// The type ends in a Distort: a new warp goes on top.
    Append,
}

/// What a warp of `t` starts from and replaces (see [`set_warp`]).
pub fn warp_target(t: &TextLayer) -> WarpTarget {
    match t.distortions.last() {
        None => WarpTarget::Style,
        Some(d) => match d.in_document(&t.transform) {
            Some(TypeDistortionKind::Warp(w)) if matches!(d.kind, TypeDistortionKind::Warp(_)) => WarpTarget::Last(w),
            _ => WarpTarget::Append,
        },
    }
}

/// The Warp Text a preset (or none) stands for.
fn text_warp(w: &Warp) -> Option<photocraft_doc::text::TextWarp> {
    let clamp = |v: f64| if v.is_finite() { v.clamp(-100.0, 100.0) as f32 } else { 0.0 };
    w.style.is_preset().then(|| photocraft_doc::text::TextWarp {
        style: w.style.psd_name().into(),
        value: clamp(w.bend),
        horizontal_distortion: clamp(w.h_distort),
        vertical_distortion: clamp(w.v_distort),
        horizontal: !w.vertical,
    })
}

/// Edit › Transform › Warp on type, the document-space warp `w` replacing what [`warp_target`]
/// says: with no free transforms yet a style (or none) is the Warp Text and a custom mesh
/// replaces it; else `w` replaces the trailing free warp (none removes it), or goes on top of a
/// trailing Distort. Returns whether anything changed.
pub fn set_warp(t: &mut TextLayer, w: &Warp) -> Result<bool> {
    if w.mesh.as_ref().is_some_and(|m| !m.is_valid() || m.points.len() > 193 * 193) || !w.bounds.iter().all(|v| v.is_finite()) {
        return Err(EngineError::Other("malformed warp mesh".into()));
    }
    let free = |t: &TextLayer, w: &Warp| TypeDistortion { frame: t.transform, kind: TypeDistortionKind::Warp(w.clone()) };
    match warp_target(t) {
        WarpTarget::Style => {
            if w.style == WarpStyle::Custom && !w.is_identity() {
                let d = free(t, w);
                if !d.is_valid() {
                    return Err(EngineError::Other("the warp box is empty".into()));
                }
                t.warp = None;
                t.distortions.push(d);
                return Ok(true);
            }
            let tw = text_warp(w);
            if t.warp == tw {
                return Ok(false);
            }
            t.warp = tw;
            Ok(true)
        }
        WarpTarget::Last(_) => {
            t.distortions.pop();
            if !w.is_identity() {
                let d = free(t, w);
                if !d.is_valid() {
                    return Err(EngineError::Other("the warp box is empty".into()));
                }
                t.distortions.push(d);
            }
            Ok(true)
        }
        WarpTarget::Append => {
            if w.is_identity() {
                return Ok(false);
            }
            if t.distortions.len() >= MAX_TYPE_DISTORTIONS {
                return Err(too_many());
            }
            let d = free(t, w);
            if !d.is_valid() {
                return Err(EngineError::Other("the warp box is empty".into()));
            }
            t.distortions.push(d);
            Ok(true)
        }
    }
}

/// `t` without what a warp edits (its Warp Text, or its trailing free warp): the type the Warp
/// preview bends.
pub fn without_warp_target(t: &TextLayer) -> TextLayer {
    let mut plain = t.clone();
    match warp_target(t) {
        WarpTarget::Style => plain.warp = None,
        WarpTarget::Last(_) => {
            plain.distortions.pop();
        }
        WarpTarget::Append => {}
    }
    plain
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_geom::warp::BezierMesh;

    fn bent() -> Warp {
        let b = [0.0, 0.0, 100.0, 40.0];
        Warp::custom(BezierMesh::identity(b, 1, 1).map_points(|p| [p[0], p[1] + p[0] * 0.2]), b)
    }

    #[test]
    fn a_custom_warp_replaces_the_style_then_itself() {
        let mut t =
            TextLayer { warp: Some(photocraft_doc::text::TextWarp { style: "warpArc".into(), value: 20.0, ..Default::default() }), ..Default::default() };
        assert!(set_warp(&mut t, &bent()).unwrap());
        assert!(t.warp.is_none());
        assert_eq!(t.distortions.len(), 1);
        // Moved since: the warp shows where the type is now.
        t.transform = Affine::translate(10.0, 5.0);
        let WarpTarget::Last(w) = warp_target(&t) else { panic!("edits the free warp") };
        assert_eq!(w.bounds, [10.0, 5.0, 110.0, 45.0]);
        assert!(set_warp(&mut t, &Warp::none(w.bounds)).unwrap());
        assert!(t.distortions.is_empty());
    }

    #[test]
    fn distorts_merge_and_bad_maps_are_refused() {
        let mut t = TextLayer::default();
        let h = Homography::rect_to_quad([0.0, 0.0, 10.0, 10.0], [[0.0, 0.0], [12.0, 1.0], [10.0, 10.0], [0.0, 9.0]]).unwrap();
        push_projective(&mut t, &h).unwrap();
        push_projective(&mut t, &h).unwrap();
        assert_eq!(t.distortions.len(), 1, "two Distorts are one map");
        assert!(push_projective(&mut t, &Homography([0.0; 9])).is_err());
        assert!(push_projective(&mut t, &Homography([f64::NAN, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])).is_err());
        let mut bad = bent();
        if let Some(m) = bad.mesh.as_mut() {
            m.points.pop();
        }
        assert!(set_warp(&mut t, &bad).is_err());
    }
}
