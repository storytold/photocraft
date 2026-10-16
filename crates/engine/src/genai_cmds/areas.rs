//! The separate areas of a selection: Generative Fill can fill each one on its own,
//! so several objects selected together are generated side by side, each in its own layer.
//!
//! Areas are the 8-connected parts of the selection, found on a grid of at most [`MAX_CELLS`]
//! cells (one cell per pixel for selections up to about a megapixel, blocks of pixels beyond):
//! parts closer than a block are one area.

use photocraft_geom::Rect;
use photocraft_raster::Surface;

/// Coverage from which a pixel belongs to an area.
const ON: f32 = 0.02;
/// An area must be selected at least this much somewhere: fainter wisps (a feather's tail) are
/// left out.
const SOLID: f32 = 0.5;
/// Most grid cells the areas are looked for on.
const MAX_CELLS: u64 = 1 << 20;
/// Most areas told apart; a selection with more is one area.
pub const MAX_AREAS: usize = 64;

/// A selection split into its areas.
pub(crate) struct Areas {
    bounds: Rect,
    step: i32,
    gw: usize,
    /// Per cell: 0, or the area (1-based) it belongs to.
    labels: Vec<u32>,
    /// Each area's bounds, in pixels, top to bottom.
    rects: Vec<Rect>,
}

impl Areas {
    /// The areas of `sel` inside `canvas`.
    pub(crate) fn find(sel: &Surface, canvas: Rect) -> Areas {
        let bounds = sel.content_bounds().intersect(&canvas);
        let empty = Areas { bounds, step: 1, gw: 0, labels: Vec::new(), rects: Vec::new() };
        if bounds.is_empty() {
            return empty;
        }
        let (w, h) = (u64::from(bounds.width()), u64::from(bounds.height()));
        let mut step = 1u64;
        while w.div_ceil(step).saturating_mul(h.div_ceil(step)) > MAX_CELLS {
            step += 1;
        }
        let (gw, gh) = (w.div_ceil(step) as usize, h.div_ceil(step) as usize);
        let step = step as i32;
        // Each cell's highest coverage.
        let mut peak = vec![0.0f32; gw * gh];
        let n = sel.channels().max(1);
        for gy in 0..gh {
            let y0 = bounds.y0 + gy as i32 * step;
            let band = Rect::new(bounds.x0, y0, bounds.x1, (y0 + step).min(bounds.y1));
            let px = sel.read_region(band);
            let bw = band.width() as usize;
            for (i, v) in px.iter().step_by(n).enumerate() {
                let cell = gy * gw + (i % bw.max(1)) / step as usize;
                if let Some(p) = peak.get_mut(cell)
                    && *v > *p
                {
                    *p = *v;
                }
            }
        }
        // 8-connected parts of the cells at or above ON, in scan order.
        let mut labels = vec![0u32; gw * gh];
        let mut rects = Vec::new();
        let mut solid = Vec::new();
        let mut stack = Vec::new();
        for start in 0..labels.len() {
            if labels[start] != 0 || peak[start] < ON {
                continue;
            }
            if rects.len() >= MAX_AREAS {
                // Too many to tell apart: the selection is one area.
                return Areas { bounds, step, gw, labels: vec![1; gw * gh], rects: vec![bounds] };
            }
            let label = rects.len() as u32 + 1;
            let (mut cx0, mut cy0, mut cx1, mut cy1) = (usize::MAX, usize::MAX, 0, 0);
            let mut top = 0.0f32;
            labels[start] = label;
            stack.push(start);
            while let Some(c) = stack.pop() {
                let (cx, cy) = (c % gw, c / gw);
                (cx0, cy0, cx1, cy1) = (cx0.min(cx), cy0.min(cy), cx1.max(cx), cy1.max(cy));
                top = top.max(peak[c]);
                for (dx, dy) in [(-1i64, -1i64), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                    let (nx, ny) = (cx as i64 + dx, cy as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= gw as i64 || ny >= gh as i64 {
                        continue;
                    }
                    let nc = ny as usize * gw + nx as usize;
                    if labels[nc] == 0 && peak[nc] >= ON {
                        labels[nc] = label;
                        stack.push(nc);
                    }
                }
            }
            let r = Rect::new(
                bounds.x0 + cx0 as i32 * step,
                bounds.y0 + cy0 as i32 * step,
                (bounds.x0 + (cx1 as i32 + 1) * step).min(bounds.x1),
                (bounds.y0 + (cy1 as i32 + 1) * step).min(bounds.y1),
            );
            rects.push(r);
            solid.push(top >= SOLID);
        }
        // Drop the wisps and number the rest from 1 again.
        let mut renumber = vec![0u32; rects.len() + 1];
        let mut kept = Vec::new();
        for (i, r) in rects.into_iter().enumerate() {
            if solid.get(i) == Some(&true) {
                kept.push(r);
                if let Some(slot) = renumber.get_mut(i + 1) {
                    *slot = kept.len() as u32;
                }
            }
        }
        for l in &mut labels {
            *l = renumber.get(*l as usize).copied().unwrap_or(0);
        }
        Areas { bounds, step, gw, labels, rects: kept }
    }

    pub(crate) fn len(&self) -> usize {
        self.rects.len()
    }

    pub(crate) fn rects(&self) -> &[Rect] {
        &self.rects
    }

    /// The selection limited to area `k` (0-based): its coverage there, nothing elsewhere.
    pub(crate) fn mask(&self, sel: &Surface, k: usize) -> Option<Surface> {
        let r = *self.rects.get(k)?;
        let label = k as u32 + 1;
        let n = sel.channels().max(1);
        let mut px = sel.read_region(r);
        let step = self.step.max(1);
        let w = r.width() as usize;
        for (i, p) in px.chunks_mut(n).enumerate() {
            let (x, y) = (r.x0 + (i % w.max(1)) as i32, r.y0 + (i / w.max(1)) as i32);
            let cell = ((y - self.bounds.y0) / step) as usize * self.gw + ((x - self.bounds.x0) / step) as usize;
            if self.labels.get(cell) != Some(&label) {
                p.fill(0.0);
            }
        }
        let mut out = Surface::new(sel.format());
        out.write_region(r, &px);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;

    /// An empty selection, in the format selections have.
    fn mask() -> Surface {
        Surface::new(PixelFormat::GRAY8)
    }

    fn rect(s: &mut Surface, r: Rect, v: f32) {
        s.fill_rect(r, &[v]);
    }

    #[test]
    fn separate_parts_are_areas_in_reading_order() {
        let mut s = mask();
        rect(&mut s, Rect::new(120, 10, 150, 40), 1.0);
        rect(&mut s, Rect::new(10, 50, 40, 90), 1.0);
        // Touching at a corner: one area with the first.
        rect(&mut s, Rect::new(150, 40, 160, 50), 1.0);
        let a = Areas::find(&s, Rect::new(0, 0, 200, 100));
        assert_eq!(a.len(), 2);
        assert_eq!(a.rects(), &[Rect::new(120, 10, 160, 50), Rect::new(10, 50, 40, 90)]);
        let first = a.mask(&s, 0).unwrap();
        assert_eq!(first.content_bounds(), Rect::new(120, 10, 160, 50));
        assert_eq!(first.pixel(130, 20), vec![1.0]);
        let second = a.mask(&s, 1).unwrap();
        assert_eq!(second.pixel(130, 20), vec![0.0]);
        assert_eq!(second.pixel(20, 60), vec![1.0]);
        assert!(a.mask(&s, 2).is_none());
    }

    #[test]
    fn faint_wisps_are_not_areas_and_nothing_selected_has_none() {
        let mut s = mask();
        rect(&mut s, Rect::new(10, 10, 30, 30), 1.0);
        rect(&mut s, Rect::new(60, 60, 70, 70), 0.1);
        let a = Areas::find(&s, Rect::new(0, 0, 100, 100));
        assert_eq!(a.len(), 1);
        assert_eq!(Areas::find(&mask(), Rect::new(0, 0, 10, 10)).len(), 0);
    }

    #[test]
    fn large_selections_are_split_on_blocks() {
        // 3000 × 1000: more cells than MAX_CELLS at one per pixel.
        let mut s = mask();
        rect(&mut s, Rect::new(0, 0, 100, 100), 1.0);
        rect(&mut s, Rect::new(2900, 900, 3000, 1000), 1.0);
        let a = Areas::find(&s, Rect::new(0, 0, 3000, 1000));
        assert!(a.step > 1);
        assert_eq!(a.len(), 2);
        let m = a.mask(&s, 1).unwrap();
        assert_eq!(m.content_bounds(), Rect::new(2900, 900, 3000, 1000));
    }
}
