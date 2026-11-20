//! The coverage of a selection or mask surface, served to the framework tracer
//! (`craft::imaging::trace::outline`) and to `SelectionOverlay`: the one PhotoCraft mask adapter.

use craft::imaging::trace::{MaskRect, MaskSource};
use photocraft_raster::Surface;

/// A coverage mask over a PhotoCraft `Surface` (channel 0, 0 unselected to 1 selected).
pub(crate) struct SurfaceMask<'a> {
    surface: &'a Surface,
    revision: u64,
    size: [u32; 2],
}

impl<'a> SurfaceMask<'a> {
    /// The mask `surface` of a document `size` pixels large. `revision` is the mask's own (its surface generation), not
    /// the document's, so an edit elsewhere in the document keeps the overlay's trace.
    pub(crate) fn new(surface: &'a Surface, revision: u64, size: [u32; 2]) -> Self {
        SurfaceMask { surface, revision, size }
    }
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
        if r.is_empty() { MaskRect::default() } else { MaskRect::new(r.x0, r.y0, r.x1, r.y1) }
    }

    fn read_row(&self, y: i32, x0: i32, step: u32, out: &mut [f32]) {
        self.surface.sample_row_strided(y, x0, i32::try_from(step).unwrap_or(i32::MAX), 0, out);
    }

    fn at(&self, x: i32, y: i32) -> f32 {
        self.surface.sample_channel(x, y, 0)
    }
}
