//! Pattern paint: Pattern Overlay, pattern strokes and pattern fill layers.
//!
//! A pattern tiles the plane from an origin (the layer's frame when linked, else the canvas
//! origin, plus the phase), scaled and rotated about that origin. Samples are bilinear with
//! wrap-around on premultiplied colour, so integer placements at 100 % reproduce the tile
//! exactly and scaled ones stay seamless.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use photocraft_doc::{ColorMode, Pattern, Rect};

pub(crate) const PREPARED_PATTERN_BYTES: usize = 64 << 20;
const MAX_PREPARED_PATTERNS: usize = 64;
type PreparedSlot = Arc<OnceLock<Option<Arc<Tile>>>>;

struct PreparedState {
    slots: HashMap<usize, PreparedSlot>,
    reserved_bytes: usize,
}

/// Demand-prepared pixels owned by one rendering call, never by the document.
pub(crate) struct PreparedPatterns<'a> {
    patterns: &'a [Pattern],
    budget: usize,
    /// The colour mode the patterns are painted into (see [`pattern_rgba`]).
    mode: ColorMode,
    state: Mutex<PreparedState>,
}

impl<'a> PreparedPatterns<'a> {
    pub(crate) fn new(patterns: &'a [Pattern], budget: usize) -> Self {
        Self { patterns, budget, mode: ColorMode::Rgb, state: Mutex::new(PreparedState { slots: HashMap::new(), reserved_bytes: 0 }) }
    }

    /// Patterns painted into a document of colour mode `mode`.
    pub(crate) fn in_mode(mut self, mode: ColorMode) -> Self {
        self.mode = mode;
        self
    }

    pub(crate) fn source(&self) -> &'a [Pattern] {
        self.patterns
    }

    pub(crate) fn get(&self, id: &str, name: &str) -> Option<Arc<Tile>> {
        let p = photocraft_doc::pattern::find(self.patterns, id, name)?;
        if !p.is_paintable() {
            return None;
        }
        let bytes = usize::try_from(p.width)
            .ok()
            .and_then(|w| usize::try_from(p.height).ok().and_then(|h| w.checked_mul(h)))
            .and_then(|pixels| pixels.checked_mul(std::mem::size_of::<[f32; 4]>()));
        // The borrowed immutable slice pins these addresses for the complete call.
        let key = std::ptr::from_ref(p) as usize;
        let slot = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(slot) = state.slots.get(&key) {
                Some(Arc::clone(slot))
            } else if let Some(bytes) = bytes
                && bytes <= self.budget.saturating_sub(state.reserved_bytes)
                && state.slots.len() < MAX_PREPARED_PATTERNS
            {
                let slot: PreparedSlot = Arc::new(OnceLock::new());
                // Reserve before initialization: concurrent pending buffers consume the budget too.
                state.reserved_bytes += bytes;
                state.slots.insert(key, Arc::clone(&slot));
                Some(slot)
            } else {
                None
            }
        };
        match slot {
            Some(slot) => {
                // Conversion must stay serial: Rayon work stealing can re-enter this slot
                // while its initializer is running inside an effect-map OnceLock.
                // Run on the requesting worker to preserve its active CMYK profile.
                slot.get_or_init(|| Tile::new_in(p, self.mode).map(Arc::new)).clone()
            }
            None => Tile::new_in(p, self.mode).map(Arc::new),
        }
    }
}

pub use photocraft_vector::pattern::{Placement, Tile, pattern_rgba};

/// Renders `tile` over `rect` (row-major straight RGBA).
pub fn render(tile: &Tile, place: &Placement, rect: Rect) -> Vec<[f32; 4]> {
    let mut out = Vec::with_capacity(rect.width() as usize * rect.height() as usize);
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let (u, v) = place.map(f64::from(x) + 0.5, f64::from(y) + 0.5);
            out.push(tile.sample(u, v));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{PixelFormat, Surface};

    fn checker() -> Pattern {
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(0, 0, 4, 4), &[1.0, 0.0, 0.0, 1.0]);
        s.fill_rect(Rect::new(0, 0, 2, 2), &[0.0, 0.0, 1.0, 1.0]);
        s.fill_rect(Rect::new(2, 2, 4, 4), &[0.0, 0.0, 1.0, 1.0]);
        Pattern::new("checker", s, 4, 4)
    }

    #[test]
    fn identity_placement_tiles_exactly() {
        let patterns = [checker()];
        let prepared = PreparedPatterns::new(&patterns, PREPARED_PATTERN_BYTES);
        let t = prepared.get(&patterns[0].id, "").unwrap();
        let again = prepared.get(&patterns[0].id, "").unwrap();
        assert!(Arc::ptr_eq(&t, &again));
        let uncached = PreparedPatterns::new(&patterns, 0);
        let fallback = uncached.get(&patterns[0].id, "").unwrap();
        let p = Placement::new(Rect::new(0, 0, 10, 10), false, (0.0, 0.0), 1.0, 0.0);
        let px = render(&t, &p, Rect::new(-4, 0, 8, 1));
        assert_eq!(px, render(&fallback, &p, Rect::new(-4, 0, 8, 1)));
        assert_eq!(px[0], [0.0, 0.0, 1.0, 1.0]); // x = -4 wraps to 0
        assert_eq!(px[6], [1.0, 0.0, 0.0, 1.0]); // x = 2
        assert_eq!(px[8], [0.0, 0.0, 1.0, 1.0]); // x = 4
    }

    #[test]
    fn patterns_too_large_to_paint_are_skipped_without_allocating() {
        // Only the size is declared (no tiles): a hostile file can claim any size.
        let p = Pattern { id: "big".into(), name: "Big".into(), width: 9000, height: 8000, surface: Surface::new(PixelFormat::RGBA8) };
        assert!(!p.is_paintable());
        assert!(Tile::new(&p).is_none());
        assert!(pattern_rgba(&p, ColorMode::Rgb).is_empty());
        let list = [p];
        assert!(PreparedPatterns::new(&list, 0).get("big", "").is_none());
    }

    #[test]
    fn link_phase_scale_and_angle_move_the_origin() {
        let t = Tile::new(&checker()).unwrap();
        let linked = Placement::new(Rect::new(2, 0, 10, 10), true, (0.0, 0.0), 1.0, 0.0);
        assert_eq!(render(&t, &linked, Rect::new(2, 0, 3, 1))[0], [0.0, 0.0, 1.0, 1.0]);
        let phased = Placement::new(Rect::EMPTY, false, (2.0, 0.0), 1.0, 0.0);
        assert_eq!(render(&t, &phased, Rect::new(2, 0, 3, 1))[0], [0.0, 0.0, 1.0, 1.0]);
        let scaled = Placement::new(Rect::EMPTY, false, (0.0, 0.0), 2.0, 0.0);
        let row = render(&t, &scaled, Rect::new(0, 1, 8, 2));
        assert_eq!(row[2], [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(row[6], [1.0, 0.0, 0.0, 1.0]);
        // 90°: x and y swap roles.
        let rot = Placement::new(Rect::EMPTY, false, (0.0, 0.0), 1.0, 90.0);
        let (u, v) = rot.map(1.0, 3.0);
        assert!((u - -3.0).abs() < 1e-9 && (v - 1.0).abs() < 1e-9, "{u},{v}");
    }

    #[test]
    fn sampling_at_hostile_phases_does_not_overflow() {
        let patterns = [checker()];
        let prepared = PreparedPatterns::new(&patterns, PREPARED_PATTERN_BYTES);
        let t = prepared.get(&patterns[0].id, "").unwrap();
        for u in [1.0e30, -1.0e30, f64::INFINITY, f64::NAN] {
            let _ = t.sample(u, u);
        }
    }
}
