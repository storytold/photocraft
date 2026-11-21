//! The coverage of a selection or mask surface, served to the framework tracer
//! (`craft::imaging::trace::outline`) and to `SelectionOverlay`: the one PhotoCraft mask adapter.

use craft::imaging::trace::{MaskRect, MaskSource};
use photocraft_raster::Surface;

/// A coverage mask over a PhotoCraft `Surface` (channel 0, 0 unselected to 1 selected).
pub(crate) struct SurfaceMask<'a> {
    surface: &'a Surface,
    revision: u64,
    size: [u32; 2],
    /// The part of the mask that is read, in its own pixels; everything else reads as unselected. `None` reads it all.
    window: Option<MaskRect>,
}

impl<'a> SurfaceMask<'a> {
    /// The mask `surface` of a document `size` pixels large. `revision` is the mask's own (its surface generation), not
    /// the document's, so an edit elsewhere in the document keeps the overlay's trace.
    pub(crate) fn new(surface: &'a Surface, revision: u64, size: [u32; 2]) -> Self {
        SurfaceMask { surface, revision, size, window: None }
    }

    /// Reads only the part of the mask that lands on the canvas when it is shown `offset` pixels from where it lies, so
    /// its outline runs along the canvas border there and never over the pasteboard: nothing outside the canvas can
    /// be selected, and ants there would hide where the canvas ends. A mask wholly on the canvas there is read as it
    /// is and keeps its revision, so its trace is reused as the offset changes.
    pub(crate) fn on_canvas_at(mut self, offset: [i32; 2]) -> Self {
        let [w, h] = self.size.map(|side| i32::try_from(side).unwrap_or(i32::MAX));
        let [dx, dy] = offset;
        let canvas = MaskRect::new(0i32.saturating_sub(dx), 0i32.saturating_sub(dy), w.saturating_sub(dx), h.saturating_sub(dy));
        if !selected_outside(self.surface, canvas) {
            return self;
        }
        // The window moves with the offset, so the trace must too.
        let shifted = (u64::from(dx as u32) << 32) | u64::from(dy as u32);
        self.revision = self.revision.rotate_left(17) ^ shifted.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.window = Some(canvas);
        self
    }

    /// Is pixel `(x, y)` read?
    fn reads(&self, x: i32, y: i32) -> bool {
        self.window.is_none_or(|r| x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1)
    }
}

/// Is any pixel of `surface` outside `window` selected? Only the parts of tiles past the window are read: the strip of
/// a tile the window's edge crosses, or a whole tile past it.
fn selected_outside(surface: &Surface, window: MaskRect) -> bool {
    if surface.format() != photocraft_color::PixelFormat::GRAY8 || surface.default_bytes().iter().any(|&v| v != 0) {
        // Selected everywhere it has no tile (an inverted selection), or a format read here byte by byte: cut it.
        return true;
    }
    let side = photocraft_geom::TILE_SIZE;
    surface.tiles().any(|(coord, tile)| {
        let r = coord.rect();
        if r.x0 >= window.x0 && r.y0 >= window.y0 && r.x1 <= window.x1 && r.y1 <= window.y1 {
            return false;
        }
        let bytes = tile.bytes();
        // Columns `a..b` of the tile, in document pixels, hold a selected pixel on row `y`.
        let any = |y: i32, a: i32, b: i32| {
            let row = usize::try_from((y - r.y0) * side).unwrap_or(0);
            let (a, b) = (usize::try_from(a - r.x0).unwrap_or(0), usize::try_from(b - r.x0).unwrap_or(0));
            a < b && bytes.get(row + a..row + b).is_some_and(|run| run.iter().any(|&v| v != 0))
        };
        (r.y0..r.y1).any(|y| {
            if y < window.y0 || y >= window.y1 {
                return any(y, r.x0, r.x1);
            }
            any(y, r.x0, window.x0.clamp(r.x0, r.x1)) || any(y, window.x1.clamp(r.x0, r.x1), r.x1)
        })
    })
}

impl MaskSource for SurfaceMask<'_> {
    fn size(&self) -> [u32; 2] {
        self.size
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    /// The allocated tiles: every pixel that differs from the default lies in one, and the union of their rects is
    /// cheap, where the exact bounds scan the pixels; the overlay asks every pass.
    fn content_bounds(&self) -> MaskRect {
        let r = self.surface.tile_bounds();
        let (x0, y0, x1, y1) = match self.window {
            Some(w) => (r.x0.max(w.x0), r.y0.max(w.y0), r.x1.min(w.x1), r.y1.min(w.y1)),
            None => (r.x0, r.y0, r.x1, r.y1),
        };
        if x0 >= x1 || y0 >= y1 { MaskRect::default() } else { MaskRect::new(x0, y0, x1, y1) }
    }

    fn read_row(&self, y: i32, x0: i32, step: u32, out: &mut [f32]) {
        let step = i32::try_from(step).unwrap_or(i32::MAX);
        self.surface.sample_row_strided(y, x0, step, 0, out);
        if self.window.is_some() {
            let mut x = x0;
            for v in out.iter_mut() {
                if !self.reads(x, y) {
                    *v = 0.0;
                }
                x = x.saturating_add(step);
            }
        }
    }

    fn at(&self, x: i32, y: i32) -> f32 {
        if self.reads(x, y) { self.surface.sample_channel(x, y, 0) } else { 0.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_geom::Rect;

    /// A 100 × 80 canvas with `selected` selected.
    fn mask_of(selected: Rect) -> Surface {
        let mut s = Surface::new(PixelFormat::GRAY8);
        s.fill_rect(selected, &[1.0]);
        s
    }

    fn row(mask: &SurfaceMask<'_>, y: i32, x0: i32, n: usize) -> Vec<f32> {
        let mut out = vec![9.0; n];
        mask.read_row(y, x0, 1, &mut out);
        out
    }

    #[test]
    fn a_selection_on_the_canvas_reads_as_it_is() {
        let s = mask_of(Rect::new(10, 10, 50, 40));
        let plain = SurfaceMask::new(&s, 7, [100, 80]);
        let clipped = SurfaceMask::new(&s, 7, [100, 80]).on_canvas_at([0, 0]);
        assert_eq!(clipped.revision(), 7, "the trace is reused");
        assert_eq!(clipped.content_bounds(), plain.content_bounds());
        // Moved within the canvas, still nothing to cut.
        assert_eq!(SurfaceMask::new(&s, 7, [100, 80]).on_canvas_at([20, 10]).revision(), 7);
    }

    #[test]
    fn the_part_past_the_canvas_is_not_read() {
        let s = mask_of(Rect::new(-60, 10, 40, 40));
        let mask = SurfaceMask::new(&s, 7, [100, 80]).on_canvas_at([0, 0]);
        assert_ne!(mask.revision(), 7);
        let b = mask.content_bounds();
        assert!(b.x0 >= 0 && b.y0 >= 0 && b.x1 <= 100 && b.y1 <= 80, "{b:?}");
        assert_eq!(mask.at(-10, 20), 0.0, "off the canvas");
        assert_eq!(mask.at(10, 20), 1.0, "on the canvas");
        assert_eq!(row(&mask, 20, -2, 4), [0.0, 0.0, 1.0, 1.0], "the row stops at the canvas edge");
    }

    /// Shown moved onto the canvas, the part that was past it is read again; the part moved off the other edge is not.
    #[test]
    fn the_window_follows_the_offset() {
        let s = mask_of(Rect::new(-60, 10, 40, 40));
        let mask = SurfaceMask::new(&s, 7, [100, 80]).on_canvas_at([60, 0]);
        assert_eq!(mask.revision(), 7, "wholly on the canvas there");
        assert_eq!(mask.at(-50, 20), 1.0);
        let s = mask_of(Rect::new(10, 10, 90, 40));
        let mask = SurfaceMask::new(&s, 7, [100, 80]).on_canvas_at([30, 0]);
        assert_eq!(mask.at(60, 20), 1.0, "shown at 90");
        assert_eq!(mask.at(75, 20), 0.0, "shown at 105, past the right edge");
        assert_ne!(SurfaceMask::new(&s, 7, [100, 80]).on_canvas_at([31, 0]).revision(), mask.revision(), "each offset traces anew");
    }
}
