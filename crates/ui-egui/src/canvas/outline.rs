//! The coverage of a selection or mask surface, served to the framework tracer
//! (`craft::imaging::trace::outline`) and to `SelectionOverlay`: the one PhotoCraft mask adapter.

use craft::imaging::trace::{MaskRect, MaskSource};
use photocraft_raster::Surface;

/// A coverage mask over a PhotoCraft `Surface` (channel 0, 0 unselected to 1 selected).
pub(crate) struct SurfaceMask<'a> {
    surface: &'a Surface,
    revision: u64,
    size: [u32; 2],
    wrapped: Option<(i32, i32)>,
    implicit: bool,
}

impl<'a> SurfaceMask<'a> {
    /// The mask `surface` of a document `size` pixels large. `revision` is the mask's own (its surface generation), not
    /// the document's, so an edit elsewhere in the document keeps the overlay's trace.
    pub(crate) fn new(surface: &'a Surface, revision: u64, size: [u32; 2]) -> Self {
        SurfaceMask { surface, revision, size, wrapped: None, implicit: surface.default_pixel().first().is_some_and(|v| *v > 0.0) }
    }
    /// A Pattern Preview drag projects bounded source coverage around the canvas, without
    /// allocating or tracing a full-size surface on every pointer event.
    pub(crate) fn wrapped(mut self, offset: Option<(i32, i32)>) -> Self {
        self.wrapped = offset;
        self
    }

    fn canvas(&self) -> MaskRect {
        MaskRect::new(0, 0, i32::try_from(self.size[0]).unwrap_or(i32::MAX), i32::try_from(self.size[1]).unwrap_or(i32::MAX))
    }

    fn implicit(&self) -> bool {
        self.implicit
    }
}

impl MaskSource for SurfaceMask<'_> {
    fn size(&self) -> [u32; 2] {
        self.size
    }

    fn revision(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (self.revision, self.size, self.wrapped).hash(&mut hash);
        hash.finish()
    }

    /// The allocated tiles: every pixel that differs from the default lies in one, and the union of their rects is
    /// cheap, where the exact bounds scan the pixels; the overlay asks every pass.
    fn content_bounds(&self) -> MaskRect {
        if self.wrapped.is_some() || self.implicit() {
            return self.canvas();
        }
        let r = self.surface.tile_bounds();
        if r.is_empty() { MaskRect::default() } else { MaskRect::new(r.x0, r.y0, r.x1, r.y1) }
    }

    fn read_row(&self, y: i32, x0: i32, step: u32, out: &mut [f32]) {
        if self.wrapped.is_none() && !self.implicit() {
            self.surface.sample_row_strided(y, x0, i32::try_from(step).unwrap_or(i32::MAX), 0, out);
        } else {
            for (i, v) in out.iter_mut().enumerate() {
                let x = i64::from(x0).saturating_add((i as i64).saturating_mul(i64::from(step)));
                *v = i32::try_from(x).map_or(0.0, |x| self.at(x, y));
            }
        }
    }

    fn at(&self, x: i32, y: i32) -> f32 {
        let canvas = self.canvas();
        if let Some((dx, dy)) = self.wrapped {
            if x < 0 || y < 0 || x >= canvas.x1 || y >= canvas.y1 || canvas.x1 == 0 || canvas.y1 == 0 {
                return 0.0;
            }
            let x = (i64::from(x) - i64::from(dx)).rem_euclid(i64::from(canvas.x1)) as i32;
            let y = (i64::from(y) - i64::from(dy)).rem_euclid(i64::from(canvas.y1)) as i32;
            return self.surface.sample_channel(x, y, 0);
        }
        if self.implicit() && (x < 0 || y < 0 || x >= canvas.x1 || y >= canvas.y1) {
            return 0.0;
        }
        self.surface.sample_channel(x, y, 0)
    }
}
