//! Building the "new area" mask and the result layer's coverage for Generative Expand.

use crate::Mask;

/// An axis-aligned pixel rectangle: `x, y` top-left, `w, h` size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// The rectangle of `keep` that stays untouched: `keep` clipped to the canvas, shrunk by `margin`
/// on every side that has new canvas beyond it. A side flush with the canvas edge got nothing
/// added, so it keeps its pixels to the edge.
fn inner(width: u32, height: u32, keep: Rect, margin: u32) -> (u32, u32, u32, u32) {
    let kx1 = keep.x.saturating_add(keep.w).min(width);
    let ky1 = keep.y.saturating_add(keep.h).min(height);
    let x0 = if keep.x > 0 { keep.x.saturating_add(margin) } else { 0 }.min(width);
    let y0 = if keep.y > 0 { keep.y.saturating_add(margin) } else { 0 }.min(height);
    let x1 = if kx1 < width { kx1.saturating_sub(margin) } else { kx1 };
    let y1 = if ky1 < height { ky1.saturating_sub(margin) } else { ky1 };
    (x0, y0, x1, y1)
}

/// Mask for a canvas of `width` x `height` where everything outside `keep` (the old image) is
/// marked for generation, plus a band of `margin` pixels just inside each extended edge. The band
/// gives the model a seam to blend into instead of a hard cut. Returns `None` for an empty canvas
/// or one too large to allocate.
pub fn expand_mask(width: u32, height: u32, keep: Rect, margin: u32) -> Option<Mask> {
    let n = (width as usize).checked_mul(height as usize)?;
    if n == 0 {
        return None;
    }
    let (x0, y0, x1, y1) = inner(width, height, keep, margin);
    let mut data = vec![255u8; n];
    if x1 > x0 && y1 > y0 {
        for y in y0..y1 {
            let row = (y as usize) * (width as usize);
            data[row + x0 as usize..row + x1 as usize].fill(0);
        }
    }
    Some(Mask { width, height, data })
}

/// Per-pixel opacity (0..=255) for the layer that carries the result: opaque over the new
/// canvas, fading from opaque at an extended edge of the old image to transparent `margin`
/// pixels inside it, and transparent over the rest of the old image. Same size as
/// [`expand_mask`]'s; `None` under the same conditions.
pub fn expand_coverage(width: u32, height: u32, keep: Rect, margin: u32) -> Option<Vec<u8>> {
    let n = (width as usize).checked_mul(height as usize)?;
    if n == 0 {
        return None;
    }
    let kx1 = keep.x.saturating_add(keep.w).min(width);
    let ky1 = keep.y.saturating_add(keep.h).min(height);
    let mut data = vec![255u8; n];
    if kx1 <= keep.x || ky1 <= keep.y {
        return Some(data);
    }
    // Distance from a pixel inside `keep` to its nearest extended edge; a side flush with the
    // canvas is infinitely far.
    let far = u32::MAX;
    for y in keep.y..ky1 {
        let dy0 = if keep.y > 0 { y - keep.y } else { far };
        let dy1 = if ky1 < height { ky1 - 1 - y } else { far };
        let row = (y as usize) * (width as usize);
        for x in keep.x..kx1 {
            let dx0 = if keep.x > 0 { x - keep.x } else { far };
            let dx1 = if kx1 < width { kx1 - 1 - x } else { far };
            let d = dx0.min(dx1).min(dy0).min(dy1);
            data[row + x as usize] = if d >= margin {
                0
            } else {
                // d = 0 at the edge -> 255; d = margin - 1 -> one step above 0.
                let t = (margin - d) as f32 / (margin as f32 + 1.0);
                (t * 255.0).round().clamp(0.0, 255.0) as u8
            };
        }
    }
    Some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(m: &Mask, x: u32, y: u32) -> u8 {
        m.data[(y * m.width + x) as usize]
    }

    #[test]
    fn right_extension_marks_only_new_strip_and_margin() {
        // Old image 6x4 at the left of an 8x4 canvas, margin 1.
        let m = expand_mask(8, 4, Rect { x: 0, y: 0, w: 6, h: 4 }, 1).unwrap_or_else(|| unreachable!());
        assert!(m.is_consistent());
        assert_eq!(at(&m, 7, 2), 255, "new strip");
        assert_eq!(at(&m, 6, 2), 255, "new strip");
        assert_eq!(at(&m, 5, 2), 255, "margin band inside the extended edge");
        assert_eq!(at(&m, 4, 2), 0, "old pixel stays");
        assert_eq!(at(&m, 0, 0), 0, "no band along the untouched left/top edges");
    }

    #[test]
    fn zero_margin_keeps_the_whole_old_image() {
        let m = expand_mask(4, 4, Rect { x: 1, y: 1, w: 2, h: 2 }, 0).unwrap_or_else(|| unreachable!());
        let kept = m.data.iter().filter(|&&v| v == 0).count();
        assert_eq!(kept, 4);
        assert_eq!(at(&m, 1, 1), 0);
        assert_eq!(at(&m, 0, 0), 255);
    }

    #[test]
    fn oversized_margin_or_rect_does_not_panic() {
        let m = expand_mask(4, 4, Rect { x: 1, y: 1, w: 2, h: 2 }, 10).unwrap_or_else(|| unreachable!());
        assert!(m.data.iter().all(|&v| v == 255));
        let m = expand_mask(4, 4, Rect { x: 100, y: 100, w: 5, h: 5 }, 0).unwrap_or_else(|| unreachable!());
        assert!(m.data.iter().all(|&v| v == 255));
        assert!(expand_mask(0, 4, Rect { x: 0, y: 0, w: 1, h: 1 }, 0).is_none());
        assert!(expand_coverage(0, 4, Rect { x: 0, y: 0, w: 1, h: 1 }, 0).is_none());
    }

    #[test]
    fn coverage_fades_only_towards_extended_edges() {
        // 6 px old image, 2 px added on the right, margin 2.
        let c = expand_coverage(8, 1, Rect { x: 0, y: 0, w: 6, h: 1 }, 2).unwrap_or_else(|| unreachable!());
        assert_eq!(&c[6..8], &[255, 255], "new strip is opaque");
        assert_eq!(c[5], 170, "edge pixel: 2/3 of 255");
        assert_eq!(c[4], 85, "one in: 1/3 of 255");
        assert_eq!(&c[0..4], &[0, 0, 0, 0], "the rest of the old image is untouched");
        // Margin 0: hard edge.
        let c = expand_coverage(8, 1, Rect { x: 0, y: 0, w: 6, h: 1 }, 0).unwrap_or_else(|| unreachable!());
        assert_eq!(c, vec![0, 0, 0, 0, 0, 0, 255, 255]);
    }
}
