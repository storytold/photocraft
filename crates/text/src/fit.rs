//! Dynamic Text: chooses a box or area layer's lines and sizes each one to span its width
//! (Photoshop's Dynamic Text), and bakes the result into ordinary type.

pub const MIN_PT: f32 = 0.1;
pub const MAX_PT: f32 = 1296.0;

/// Share of its band's widest span a line in a shape is sized to: a hair inside, so layout always
/// keeps it on the band the solver placed it on.
pub(crate) const AREA_MARGIN: f32 = 0.999;
/// How far a line in a shape moves down per step when its band is too narrow, as a share of
/// its height at its floor scale.
pub(crate) const ROW_SKIP_STEP: f32 = 0.05;
/// Most steps a line in a shape moves down looking for a band that holds it.
pub(crate) const ROW_SKIP_STEPS: usize = 256;
