//! Fixed local directions for editing a perspective guide, not a pixel warp.
//! Capture once on pointer down; project onto the nearest unoriented axis and
//! keep that choice until Shift is released. Edge lengths must not bias selection.

#[derive(Clone, Copy, Debug)]
pub(crate) struct LocalAxes {
    directions: [[f64; 2]; 2],
}

impl LocalAxes {
    pub(crate) fn at_corner(corners: &[[f64; 2]; 4], corner: usize) -> Option<Self> {
        let anchor = corners.get(corner)?;
        let next = corners.get((corner + 1) % 4)?;
        let previous = corners.get((corner + 3) % 4)?;
        let unit = |p: &[f64; 2]| {
            let v = [p[0] - anchor[0], p[1] - anchor[1]];
            let length = v[0].hypot(v[1]);
            (length.is_finite() && length > 1e-9).then(|| [v[0] / length, v[1] / length])
        };
        Some(Self { directions: [unit(next)?, unit(previous)?] })
    }

    pub(crate) fn direction(&self, axis: usize) -> Option<[f64; 2]> {
        self.directions.get(axis).copied()
    }

    /// A signed projection allows dragging both towards and away from a neighbour.
    /// Keeping the locked index prevents diagonal mouse noise from changing axes.
    pub(crate) fn restrict(&self, delta: [f64; 2], locked: Option<usize>, threshold: f64) -> ([f64; 2], Option<usize>) {
        if delta.iter().any(|n| !n.is_finite()) || !threshold.is_finite() || threshold < 0.0 {
            return ([0.0, 0.0], None);
        }
        let dot = |v: [f64; 2]| delta[0] * v[0] + delta[1] * v[1];
        let index = match locked {
            Some(i) => i,
            None => {
                if delta[0].hypot(delta[1]) < threshold.max(f64::EPSILON) {
                    return ([0.0, 0.0], None);
                }
                usize::from(dot(self.directions[1]).abs() > dot(self.directions[0]).abs())
            }
        };
        let Some(axis) = self.direction(index) else { return ([0.0, 0.0], None) };
        let distance = dot(axis);
        let projected = [axis[0] * distance, axis[1] * distance];
        if projected.iter().any(|n| !n.is_finite()) {
            return ([0.0, 0.0], None);
        }
        (projected, Some(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn quad() -> [[f64; 2]; 4] {
        [[25.0, 20.0], [100.0, 16.0], [109.0, 105.0], [21.0, 110.0]]
    }
    fn parallel(delta: [f64; 2], axis: [f64; 2]) {
        assert!((delta[0] * axis[1] - delta[1] * axis[0]).abs() < 1e-9);
    }
    #[test]
    fn local_axes_are_adjacent_edges_not_screen_axes() {
        let axes = LocalAxes::at_corner(&quad(), 0).unwrap();
        let (d, lock) = axes.restrict([30.0, -6.0], None, 2.0);
        assert_eq!(lock, Some(0));
        parallel(d, [75.0, -4.0]);
        assert!(d[1] < -1.0);
        let (d, lock) = axes.restrict([-7.0, 25.0], None, 2.0);
        assert_eq!(lock, Some(1));
        parallel(d, [-4.0, 90.0]);
        assert!(d[0] < -0.5);
    }
    #[test]
    fn axis_is_locked_despite_later_diagonal_motion() {
        let axes = LocalAxes::at_corner(&quad(), 0).unwrap();
        let (_, locked) = axes.restrict([15.0, -1.0], None, 2.0);
        let (d, locked2) = axes.restrict([5.0, 80.0], locked, 2.0);
        assert_eq!(locked, locked2);
        parallel(d, [75.0, -4.0]);
    }
    #[test]
    fn unoriented_axes_allow_backwards_off_canvas_movement() {
        let axes = LocalAxes::at_corner(&quad(), 0).unwrap();
        let (d, chosen) = axes.restrict([-150.0, 10.0], None, 2.0);
        assert_eq!(chosen, Some(0));
        parallel(d, [75.0, -4.0]);
        assert!(d[0] < -140.0 && d[1] > 0.0);
    }
    #[test]
    fn a_longer_neighbour_does_not_bias_axis_choice() {
        let q = [[0.0, 0.0], [10000.0, 0.0], [10000.0, 10.0], [0.0, 10.0]];
        let axes = LocalAxes::at_corner(&q, 0).unwrap();
        let (d, chosen) = axes.restrict([3.0, 6.0], None, 2.0);
        assert_eq!(chosen, Some(1));
        assert_eq!(d, [0.0, 6.0]);
    }
    #[test]
    fn jitter_and_zero_delta_do_not_choose_an_axis() {
        let axes = LocalAxes::at_corner(&quad(), 0).unwrap();
        for delta in [[0.0, 0.0], [0.2, -0.1], [1.0, 1.0]] {
            assert_eq!(axes.restrict(delta, None, 2.0), ([0.0, 0.0], None));
        }
        assert_eq!(axes.restrict([0.0, 0.0], None, 0.0), ([0.0, 0.0], None));
    }
    #[test]
    fn frozen_axes_do_not_follow_modified_guide() {
        let mut q = quad();
        let axes = LocalAxes::at_corner(&q, 0).unwrap();
        q[0] = [39.0, 55.0];
        let (d, _) = axes.restrict([30.0, 2.0], Some(0), 2.0);
        parallel(d, [75.0, -4.0]);
        let changed = LocalAxes::at_corner(&q, 0).unwrap().direction(0).unwrap();
        assert!((d[0] * changed[1] - d[1] * changed[0]).abs() > 0.1);
    }
    #[test]
    fn invalid_inputs_do_not_panic_or_propagate_nan() {
        let axes = LocalAxes::at_corner(&quad(), 0).unwrap();
        assert!(LocalAxes::at_corner(&quad(), usize::MAX).is_none());
        let mut q = quad();
        q[1] = q[0];
        assert!(LocalAxes::at_corner(&q, 0).is_none());
        q[1][0] = f64::NAN;
        assert!(LocalAxes::at_corner(&q, 0).is_none());
        for d in [[f64::INFINITY, 1.0], [0.0, f64::NAN]] {
            assert_eq!(axes.restrict(d, None, 2.0), ([0.0, 0.0], None));
        }
        assert_eq!(axes.restrict([3.0, 4.0], Some(usize::MAX), 2.0), ([0.0, 0.0], None));
    }
    #[test]
    fn all_corners_and_rotated_guides_remain_collinear() {
        for angle in 0..180 {
            let (s, c) = (f64::from(angle).to_radians()).sin_cos();
            let q = quad().map(|p| [c * p[0] - s * p[1], s * p[0] + c * p[1]]);
            for corner in 0..4 {
                let axes = LocalAxes::at_corner(&q, corner).unwrap();
                for axis in 0..2 {
                    let unit = axes.direction(axis).unwrap();
                    for sign in [-1.0, 1.0] {
                        let delta = [sign * unit[0] * 17.0, sign * unit[1] * 17.0];
                        let (d, chosen) = axes.restrict(delta, None, 2.0);
                        assert_eq!(chosen, Some(axis));
                        parallel(d, unit);
                        assert!((d[0] - delta[0]).abs() + (d[1] - delta[1]).abs() < 1e-9);
                    }
                }
            }
        }
    }
}
