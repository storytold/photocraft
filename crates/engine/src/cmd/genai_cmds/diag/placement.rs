//! The placement transform Save Fill Diagnostics reports for each placed variation (#10939):
//! which part of the returned picture was used, the size it was matched at, and where and at what
//! scale it went into the document. It restates what [`super::super::pixels::place`] did from the
//! [`Framing`] it recorded; it never changes how a picture is placed.
//!
//! The steps are the ones `place` takes: the used part is scaled to the request size, colour
//! matched there, then scaled into the area sent and written at its corner. So a document pixel
//! `x` came from returned pixel `used.x + (x - area.x) / scale.x`.

use photocraft_geom::Rect;
use serde::Serialize;

use super::super::pixels::{Framing, centred};

/// Where one returned picture went.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Placement {
    /// The returned picture's size.
    pub(super) returned: [u32; 2],
    /// The part of it that was used: x, y, width, height in its pixels.
    pub(super) used: [u32; 4],
    /// The size it was scaled to for colour matching (the size sent).
    pub(super) request: [u32; 2],
    /// The document area it was scaled into: x, y, width, height in document pixels.
    pub(super) area: [i64; 4],
    /// Document pixels per used returned pixel, across and down.
    pub(super) scale: [f64; 2],
    /// `scale` across divided by `scale` down: 1 when the shape was kept, other values when the
    /// picture was stretched (a same-shape result is allowed 2 % either way).
    pub(super) stretch: f64,
}

/// How a picture of size `returned`, lined up as `framing`, was placed for a request of
/// `request` (width, height) into document `area`.
pub(super) fn placement(returned: [u32; 2], framing: &Framing, request: [u32; 2], area: Rect) -> Placement {
    let full = Rect::from_xywh(0, 0, returned[0], returned[1]);
    let used = match framing {
        // `centred` needs a picture to cut from; an empty one is reported as it is.
        Framing::Centre { .. } if !full.is_empty() => centred(full, request[0], request[1]),
        _ => full,
    };
    let ratio = |to: u32, from: u32| if from == 0 { 0.0 } else { f64::from(to) / f64::from(from) };
    let scale = [ratio(area.width(), used.width()), ratio(area.height(), used.height())];
    let stretch = if scale[1] == 0.0 { 0.0 } else { scale[0] / scale[1] };
    let corner = |v: i32| u32::try_from(v).unwrap_or(0);
    Placement {
        returned,
        used: [corner(used.x0), corner(used.y0), used.width(), used.height()],
        request,
        area: [i64::from(area.x0), i64::from(area.y0), i64::from(area.width()), i64::from(area.height())],
        scale,
        stretch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_same_shape_result_uses_the_whole_picture() {
        let p = placement([1024, 1024], &Framing::Same, [512, 512], Rect::from_xywh(40, 60, 256, 256));
        assert_eq!(p.used, [0, 0, 1024, 1024]);
        assert_eq!(p.area, [40, 60, 256, 256]);
        assert_eq!(p.scale, [0.25, 0.25]);
        assert_eq!(p.stretch, 1.0);
    }

    #[test]
    fn a_centred_result_reports_the_cut_it_used() {
        let framing = Framing::Centre { returned: [1536, 1024], sent: [1024, 1024] };
        let p = placement([1536, 1024], &framing, [1024, 1024], Rect::from_xywh(0, 0, 512, 512));
        assert_eq!(p.used, [256, 0, 1024, 1024], "the middle square of a wide picture");
        assert_eq!(p.scale, [0.5, 0.5]);
        assert_eq!(p.stretch, 1.0);
    }

    #[test]
    fn a_scaled_result_shows_its_stretch() {
        let framing = Framing::Scaled { returned: [1536, 1024], sent: [1024, 1024] };
        let p = placement([1536, 1024], &framing, [1024, 1024], Rect::from_xywh(10, 20, 1024, 1024));
        assert_eq!(p.used, [0, 0, 1536, 1024], "the whole picture, squeezed");
        assert!((p.stretch - 2.0 / 3.0).abs() < 1e-9, "{}", p.stretch);
    }

    #[test]
    fn a_slightly_different_same_shape_result_is_not_hidden() {
        // 1000×1000 sent, 1008×992 back: within the 2 % allowance, stretched a little.
        let p = placement([1008, 992], &Framing::Same, [1000, 1000], Rect::from_xywh(0, 0, 1000, 1000));
        assert!(p.stretch < 1.0 && p.stretch > 0.98, "{}", p.stretch);
    }

    #[test]
    fn an_empty_picture_gives_zero_scales_not_a_crash() {
        let p = placement([0, 0], &Framing::Same, [0, 0], Rect::from_xywh(0, 0, 10, 10));
        assert_eq!(p.scale, [0.0, 0.0]);
        assert_eq!(p.stretch, 0.0);
        let centre = Framing::Centre { returned: [0, 0], sent: [10, 10] };
        assert_eq!(placement([0, 0], &centre, [10, 10], Rect::from_xywh(0, 0, 10, 10)).used, [0, 0, 0, 0]);
    }
}
