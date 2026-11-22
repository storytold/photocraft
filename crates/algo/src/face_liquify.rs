//! Filter › Liquify › Face-Aware Liquify: Photoshop's face sliders as local warps.
//!
//! A face is its five landmarks (from [`crate::faces`]): the eyes, the nose tip and the mouth
//! corners. They give the face's frame: `u` runs from the eye on the image's left to the other
//! one, `v` is perpendicular to it pointing down the face, and the eye distance `e` sets the
//! scale. Every slider (−100..100, 0 = unchanged) becomes one or two [`LocalWarp`]s: a scale,
//! rotation or move in that frame about a point the landmarks place (an eye, the nose, the mouth
//! or its corners, the forehead above the eyes, the chin and jaw below the mouth), fading out over
//! a radius proportional to `e`. The geometry is our own estimate from the five points, not
//! Photoshop's: they show where the features are, not their outline.

use serde::{Deserialize, Serialize};

use crate::liquify::{LiquifyField, LocalWarp};

/// A face's landmarks in document pixels: the eye on the image's left, the other eye, the nose
/// tip, the mouth corner on the image's left, the other mouth corner (the order of
/// [`crate::faces::Face::landmarks`]).
pub type Landmarks = [[f64; 2]; 5];

/// A slider for both eyes, or for each: `[left, right]` as they appear in the image.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EyeSlider {
    Both(f64),
    Each([f64; 2]),
}

impl Default for EyeSlider {
    fn default() -> Self {
        EyeSlider::Both(0.0)
    }
}

impl EyeSlider {
    fn get(self, i: usize) -> f64 {
        match self {
            EyeSlider::Both(v) => v,
            EyeSlider::Each(v) => v.get(i).copied().unwrap_or(0.0),
        }
    }
}

/// Photoshop's Face-Aware Liquify sliders, each −100..100.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct FaceSliders {
    pub eye_size: EyeSlider,
    pub eye_height: EyeSlider,
    pub eye_width: EyeSlider,
    pub eye_tilt: EyeSlider,
    pub eye_distance: f64,
    pub nose_height: f64,
    pub nose_width: f64,
    pub smile: f64,
    pub upper_lip: f64,
    pub lower_lip: f64,
    pub mouth_width: f64,
    pub mouth_height: f64,
    pub forehead: f64,
    pub chin_height: f64,
    pub jawline: f64,
    pub face_width: f64,
}

impl FaceSliders {
    fn values(&self) -> impl Iterator<Item = f64> {
        let eyes = [self.eye_size, self.eye_height, self.eye_width, self.eye_tilt].into_iter().flat_map(|e| [e.get(0), e.get(1)]);
        eyes.chain([
            self.eye_distance,
            self.nose_height,
            self.nose_width,
            self.smile,
            self.upper_lip,
            self.lower_lip,
            self.mouth_width,
            self.mouth_height,
            self.forehead,
            self.chin_height,
            self.jawline,
            self.face_width,
        ])
    }

    /// Every slider is a finite number in −100..100.
    pub fn validate(&self) -> Result<(), String> {
        if self.values().all(|v| v.is_finite() && (-100.0..=100.0).contains(&v)) { Ok(()) } else { Err("face sliders must be numbers in -100..100".into()) }
    }

    pub fn is_neutral(&self) -> bool {
        self.values().all(|v| v == 0.0)
    }
}

/// The face's frame: origin between the eyes, `u` toward the image-right eye, `v` down the face,
/// `e` the eye distance.
struct Frame {
    u: [f64; 2],
    v: [f64; 2],
    e: f64,
}

impl Frame {
    fn new(lm: &Landmarks) -> Option<Self> {
        let d = [lm[1][0] - lm[0][0], lm[1][1] - lm[0][1]];
        let e = d[0].hypot(d[1]);
        if !(e.is_finite() && e > 2.0) || lm.iter().flatten().any(|v| !v.is_finite()) {
            return None;
        }
        let u = [d[0] / e, d[1] / e];
        // Perpendicular in a y-down image, toward the mouth.
        let mut v = [-u[1], u[0]];
        let eyes = mid(lm[0], lm[1]);
        let mouth = mid(lm[3], lm[4]);
        if (mouth[0] - eyes[0]) * v[0] + (mouth[1] - eyes[1]) * v[1] < 0.0 {
            v = [-v[0], -v[1]];
        }
        Some(Frame { u, v, e })
    }

    /// The point `a·u + b·v` (in eye distances) from `p`.
    fn at(&self, p: [f64; 2], a: f64, b: f64) -> [f64; 2] {
        [p[0] + (a * self.u[0] + b * self.v[0]) * self.e, p[1] + (a * self.u[1] + b * self.v[1]) * self.e]
    }

    /// A warp about `c` within `r` eye distances: scale by `su` along `u` and `sv` along `v`, turn
    /// by `angle` (radians, clockwise on screen) and move by `mu` along `u` and `mv` along `v` (eye
    /// distances).
    fn warp(&self, c: [f64; 2], r: f64, [su, sv]: [f64; 2], angle: f64, [mu, mv]: [f64; 2]) -> LocalWarp {
        let (u, v) = (self.u, self.v);
        // M = R · B · diag(su, sv) · Bᵀ, with B = [u v] (columns) and R the rotation.
        let s = [
            [su * u[0] * u[0] + sv * v[0] * v[0], su * u[0] * u[1] + sv * v[0] * v[1]],
            [su * u[1] * u[0] + sv * v[1] * v[0], su * u[1] * u[1] + sv * v[1] * v[1]],
        ];
        let (sn, cs) = angle.sin_cos();
        let m = [[cs * s[0][0] - sn * s[1][0], cs * s[0][1] - sn * s[1][1]], [sn * s[0][0] + cs * s[1][0], sn * s[0][1] + cs * s[1][1]]];
        let offset = [(mu * u[0] + mv * v[0]) * self.e, (mu * u[1] + mv * v[1]) * self.e];
        LocalWarp { center: c, radius: r * self.e, m, offset }
    }
}

fn mid(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
}

/// The warps the sliders make on a face with landmarks `lm`, in the order they are applied
/// (none for neutral sliders or a degenerate face).
pub fn face_warps(lm: &Landmarks, s: &FaceSliders) -> Vec<LocalWarp> {
    let Some(f) = Frame::new(lm) else { return Vec::new() };
    let mut out = Vec::new();
    let mut push = |amount: f64, w: &dyn Fn(f64) -> LocalWarp| {
        if amount != 0.0 && amount.is_finite() {
            out.push(w(amount.clamp(-100.0, 100.0) / 100.0));
        }
    };
    let eye_mid = mid(lm[0], lm[1]);
    let mouth = mid(lm[3], lm[4]);
    let mouth_w = (lm[4][0] - lm[3][0]).hypot(lm[4][1] - lm[3][1]) / f.e;
    let nose = lm[2];
    // Eyes: the one on the image's left is index 0; outward is −u for it and +u for the other.
    for (i, side) in [(0usize, -1.0f64), (1, 1.0)] {
        let c = lm[i];
        push(s.eye_size.get(i), &|a| f.warp(c, 0.45, [1.0 + 0.3 * a, 1.0 + 0.3 * a], 0.0, [0.0, 0.0]));
        push(s.eye_width.get(i), &|a| f.warp(c, 0.45, [1.0 + 0.3 * a, 1.0], 0.0, [0.0, 0.0]));
        push(s.eye_height.get(i), &|a| f.warp(c, 0.45, [1.0, 1.0], 0.0, [0.0, -0.12 * a]));
        // Positive tilt raises the outer corner: the content moves toward the warp's map, and a
        // clockwise turn by `side · angle` would carry the outer corner (at `side · u`) down.
        push(s.eye_tilt.get(i), &|a| f.warp(c, 0.45, [1.0, 1.0], -side * 0.35 * a, [0.0, 0.0]));
        push(s.eye_distance, &|a| f.warp(c, 0.5, [1.0, 1.0], 0.0, [side * 0.1 * a, 0.0]));
    }
    push(s.nose_height, &|a| f.warp(nose, 0.55, [1.0, 1.0], 0.0, [0.0, -0.12 * a]));
    push(s.nose_width, &|a| f.warp(f.at(nose, 0.0, -0.05), 0.5, [1.0 + 0.45 * a, 1.0], 0.0, [0.0, 0.0]));
    // Smile: the corners up and a little out.
    for (i, side) in [(3usize, -1.0f64), (4, 1.0)] {
        push(s.smile, &|a| f.warp(lm[i], 0.35, [1.0, 1.0], 0.0, [side * 0.03 * a, -0.08 * a]));
    }
    push(s.upper_lip, &|a| f.warp(f.at(mouth, 0.0, -0.08 * mouth_w), 0.3 * mouth_w.max(0.5), [1.0, 1.0], 0.0, [0.0, -0.05 * a]));
    push(s.lower_lip, &|a| f.warp(f.at(mouth, 0.0, 0.12 * mouth_w), 0.3 * mouth_w.max(0.5), [1.0, 1.0], 0.0, [0.0, 0.05 * a]));
    push(s.mouth_width, &|a| f.warp(mouth, 0.75 * mouth_w.max(0.5), [1.0 + 0.25 * a, 1.0], 0.0, [0.0, 0.0]));
    push(s.mouth_height, &|a| f.warp(mouth, 0.6 * mouth_w.max(0.5), [1.0, 1.0 + 0.3 * a], 0.0, [0.0, 0.0]));
    push(s.forehead, &|a| f.warp(f.at(eye_mid, 0.0, -0.9), 0.9, [1.0, 1.0], 0.0, [0.0, -0.15 * a]));
    push(s.chin_height, &|a| f.warp(f.at(mouth, 0.0, 0.7), 0.65, [1.0, 1.0], 0.0, [0.0, 0.15 * a]));
    for side in [-1.0f64, 1.0] {
        push(s.jawline, &|a| f.warp(f.at(mouth, side * 0.85, 0.3), 0.6, [1.0, 1.0], 0.0, [side * 0.14 * a, 0.0]));
        push(s.face_width, &|a| f.warp(f.at(nose, side * 0.95, 0.1), 0.85, [1.0, 1.0], 0.0, [side * 0.15 * a, 0.0]));
    }
    out
}

/// Applies the sliders of each face (landmarks, sliders) to `field`, before any brush strokes.
pub fn apply_faces(field: &mut LiquifyField, faces: &[(Landmarks, FaceSliders)]) {
    for (lm, s) in faces {
        for w in face_warps(lm, s) {
            field.apply_warp(&w);
        }
    }
}

#[cfg(test)]
mod tests {
    use photocraft_geom::Rect;

    use super::*;

    /// An upright face: eyes at (40, 50) and (60, 50), nose at (50, 62), mouth corners at
    /// (43, 72) and (57, 72).
    const FACE: Landmarks = [[40.0, 50.0], [60.0, 50.0], [50.0, 62.0], [43.0, 72.0], [57.0, 72.0]];

    fn field(s: FaceSliders) -> LiquifyField {
        let mut f = LiquifyField::new(Rect::new(0, 0, 100, 120), 1.0);
        apply_faces(&mut f, &[(FACE, s)]);
        f
    }

    /// Where the content shown at `p` comes from.
    fn source(f: &LiquifyField, p: [f64; 2]) -> [f64; 2] {
        let d = f.sample(p[0], p[1]);
        [p[0] + d[0], p[1] + d[1]]
    }

    #[test]
    fn neutral_sliders_change_nothing() {
        assert!(FaceSliders::default().is_neutral());
        assert!(face_warps(&FACE, &FaceSliders::default()).is_empty());
        assert!(field(FaceSliders::default()).is_identity());
    }

    #[test]
    fn bigger_eyes_show_the_eye_centre_further_out() {
        let f = field(FaceSliders { eye_size: EyeSlider::Both(100.0), ..Default::default() });
        // A point right of the left eye's centre now shows content closer to the centre.
        let s = source(&f, [44.0, 50.0]);
        assert!(s[0] < 44.0 - 0.3 && (s[1] - 50.0).abs() < 0.2, "{s:?}");
        // The eye centre itself stays put; far away nothing moves.
        let c = source(&f, [40.0, 50.0]);
        assert!((c[0] - 40.0).abs() < 0.05 && (c[1] - 50.0).abs() < 0.05, "{c:?}");
        assert_eq!(source(&f, [5.0, 110.0]), [5.0, 110.0]);
        // Each eye on its own: only the right one changes.
        let f = field(FaceSliders { eye_size: EyeSlider::Each([0.0, 100.0]), ..Default::default() });
        assert_eq!(source(&f, [44.0, 50.0]), [44.0, 50.0]);
        assert!(source(&f, [64.0, 50.0])[0] < 64.0 - 0.3);
    }

    #[test]
    fn smile_lifts_the_mouth_corners_and_chin_height_lowers_the_chin() {
        let f = field(FaceSliders { smile: 100.0, ..Default::default() });
        // Content from below the corner now shows at it: the corner moved up.
        assert!(source(&f, [43.0, 71.0])[1] > 71.0 + 0.3);
        let f = field(FaceSliders { chin_height: 100.0, ..Default::default() });
        assert!(source(&f, [50.0, 88.0])[1] < 88.0 - 0.5, "the chin moved down");
        let f = field(FaceSliders { chin_height: -100.0, ..Default::default() });
        assert!(source(&f, [50.0, 84.0])[1] > 84.0 + 0.5, "the chin moved up");
    }

    #[test]
    fn positive_eye_tilt_raises_the_outer_corners() {
        // Content from below an outer corner now shows at it: the corner moved up. The inner
        // corners go the other way.
        let f = field(FaceSliders { eye_tilt: EyeSlider::Both(100.0), ..Default::default() });
        for (outer, inner) in [([36.0, 50.0], [44.0, 50.0]), ([64.0, 50.0], [56.0, 50.0])] {
            assert!(source(&f, outer)[1] > outer[1] + 0.3, "outer {outer:?}: {:?}", source(&f, outer));
            assert!(source(&f, inner)[1] < inner[1] - 0.3, "inner {inner:?}: {:?}", source(&f, inner));
        }
        // Negative tilt lowers them.
        let f = field(FaceSliders { eye_tilt: EyeSlider::Both(-100.0), ..Default::default() });
        for outer in [[36.0, 50.0], [64.0, 50.0]] {
            assert!(source(&f, outer)[1] < outer[1] - 0.3, "outer {outer:?}: {:?}", source(&f, outer));
        }
    }

    #[test]
    fn the_frame_follows_a_tilted_face() {
        // The same face turned 90° clockwise (eyes one above the other): eye height moves along x.
        let rot = |p: [f64; 2]| [100.0 - p[1], p[0]];
        let turned: Landmarks = FACE.map(rot);
        let mut f = LiquifyField::new(Rect::new(0, 0, 100, 120), 1.0);
        apply_faces(&mut f, &[(turned, FaceSliders { eye_height: EyeSlider::Both(100.0), ..Default::default() })]);
        let eye = turned[0];
        let d = f.sample(eye[0], eye[1]);
        assert!(d[0].abs() > 1.0 && d[1].abs() < 0.2, "{d:?}");
    }

    #[test]
    fn bad_sliders_and_degenerate_faces_are_refused_or_ignored() {
        assert!(FaceSliders { smile: 101.0, ..Default::default() }.validate().is_err());
        assert!(FaceSliders { nose_width: f64::NAN, ..Default::default() }.validate().is_err());
        assert!(FaceSliders { eye_size: EyeSlider::Each([0.0, -100.0]), ..Default::default() }.validate().is_ok());
        let same = [[50.0, 50.0]; 5];
        assert!(face_warps(&same, &FaceSliders { smile: 50.0, ..Default::default() }).is_empty());
        assert!(serde_json::from_str::<FaceSliders>(r#"{"smiel": 10}"#).is_err(), "unknown sliders are refused");
        let s: FaceSliders = serde_json::from_str(r#"{"eyeSize": [10, -20], "smile": 30}"#).unwrap();
        assert_eq!((s.eye_size, s.smile), (EyeSlider::Each([10.0, -20.0]), 30.0));
    }
}
