//! The Zoom tool's pointer as a signed distance field (negative inside), in logical pixels from the
//! hotspot with y pointing down: a magnifier with + in its lens, or − while it zooms out, as in
//! Photoshop. The hotspot is the centre of the lens, where the click zooms. Only Windows needs it:
//! winit maps `ZoomIn` and `ZoomOut` to the arrow there, while macOS and the Linux themes draw
//! their own magnifiers.

use super::capsule;

/// Half the side of the square the magnifier fits in, outline excluded: the handle's far end.
pub(super) const HALF: f32 = 13.0;

/// The rim's centre line, and half its thickness.
const LENS: f32 = 7.0;
const RIM: f32 = 1.0;
/// Half the length and half the thickness of the + and − bars.
const SIGN: f32 = 2.3;
const BAR: f32 = 0.8;

/// The magnifier's silhouette: negative inside it. The lens itself stays clear so the image shows
/// through, as with the Brush's outline.
pub(super) fn shape(out: bool, x: f32, y: f32) -> f32 {
    let rim = (x.hypot(y) - LENS).abs() - RIM;
    // From just outside the rim, down to the right.
    let handle = capsule(x, y, [5.8, 5.8], [11.0, 11.0], 1.9);
    let minus = capsule(x, y, [-SIGN, 0.0], [SIGN, 0.0], BAR);
    let d = rim.min(handle).min(minus);
    if out { d } else { d.min(capsule(x, y, [0.0, -SIGN], [0.0, SIGN], BAR)) }
}
