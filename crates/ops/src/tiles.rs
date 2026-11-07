//! Tile references owned by history snapshots. The snapshots keep these addresses alive;
//! COW edits cannot change a retained tile in place. Count occurrences (including aliases
//! within one document), so removing a state releases a tile only after its last reference.
use photocraft_doc::{Document, LayerContent, SmartSource, Surface};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub(crate) struct Tiles {
    refs: HashMap<usize, (u128, usize)>,
    bytes: u128,
    /// Lazy surfaces the states hold (see `Surface::lazy`), by identity: (references, one of
    /// them). Their tiles aren't counted per state, since they decode later (or never); the
    /// decoded ones are charged once, when the budget is checked.
    lazy: HashMap<usize, (u128, Surface)>,
}

pub(crate) fn surfaces(doc: &Document, visit: impl FnMut(&Surface)) {
    // Undo retains the whole clip, not only the displayed frame. Video aliases are visited
    // too: Tiles tracks every owner and counts their shared allocations only once.
    doc.for_each_surface(visit);
}

/// Every shared allocation `doc` holds that history can retain, as `visit(address, length)`:
/// the tiles of [`surfaces`] and the embedded contents of smart objects (an `Arc` that undo
/// states and duplicated layers share, replaced by Replace Contents). A lazy surface's tiles
/// (see `Surface::lazy`) are never decoded here: with `loaded` the ones already decoded are
/// visited; without, none (History counts them by identity, see [`Tiles`]).
pub(crate) fn blocks(doc: &Document, loaded: bool, mut visit: impl FnMut(usize, usize)) {
    surfaces(doc, |surface| {
        let tiles: Box<dyn Iterator<Item = _>> = if loaded { Box::new(surface.loaded_tiles()) } else { Box::new(surface.owned_tiles()) };
        for (_, tile) in tiles {
            visit(Arc::as_ptr(tile) as usize, tile.len());
        }
    });
    for (_, _, layer) in doc.walk() {
        if let LayerContent::Smart(smart) = &layer.content
            && let SmartSource::Embedded { bytes, .. } = &smart.source
        {
            visit(Arc::as_ptr(bytes) as usize, bytes.len());
        }
    }
}

impl Tiles {
    // A lazy surface's tiles (see `Surface::lazy`) are not counted per state: they are not in
    // memory until decoded, and decoded ones are counted once, with the current document.
    pub(crate) fn add(&mut self, doc: &Document) {
        surfaces(doc, |surface| {
            if let Some(id) = surface.lazy_identity() {
                self.lazy.entry(id).or_insert_with(|| (0, surface.clone())).0 += 1;
            }
        });
        blocks(doc, false, |ptr, len| {
            let count = self.refs.entry(ptr).or_insert_with(|| {
                self.bytes += len as u128;
                (0, len)
            });
            count.0 += 1;
        });
    }

    pub(crate) fn remove(&mut self, doc: &Document) {
        surfaces(doc, |surface| {
            if let Some(id) = surface.lazy_identity()
                && let Some(entry) = self.lazy.get_mut(&id)
            {
                entry.0 -= 1;
                if entry.0 == 0 {
                    self.lazy.remove(&id);
                }
            }
        });
        blocks(doc, false, |ptr, _| {
            if let Some(count) = self.refs.get_mut(&ptr) {
                count.0 -= 1;
                if count.0 == 0 {
                    self.bytes -= count.1 as u128;
                    self.refs.remove(&ptr);
                }
            }
        });
    }

    /// Retained bytes are incremental; only current tiles need examining below the limit.
    /// Current can change without recording (coalescing), and tile transfers can carry a
    /// different byte length, so a tile-count estimate must not underestimate its memory.
    pub(crate) fn fits(&self, current: &Document, budget: usize) -> bool {
        self.bytes <= budget as u128 && self.bytes_with(current) <= budget as u128
    }

    pub(crate) fn bytes_with(&self, current: &Document) -> u128 {
        let mut total = self.bytes;
        let mut seen = HashSet::new();
        blocks(current, false, |ptr, len| {
            if !self.refs.contains_key(&ptr) && seen.insert(ptr) {
                total += len as u128;
            }
        });
        // Decoded lazy tiles, once each: the current document's, then those only History still
        // holds (decoded since they were recorded).
        let mut lazy = HashSet::new();
        let mut charge = |surface: &Surface| {
            if let Some((id, tiles)) = surface.lazy_decoded()
                && lazy.insert(id)
            {
                for (_, tile) in tiles {
                    if seen.insert(Arc::as_ptr(tile) as usize) {
                        total += tile.len() as u128;
                    }
                }
            }
        };
        surfaces(current, &mut charge);
        for (_, surface) in self.lazy.values() {
            charge(surface);
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, Document, Layer, LayerContent, PixelFormat, Rect, SampleType, Size};

    /// A lazy layer decoded after its state was recorded is charged once, also when only History
    /// still holds it.
    #[test]
    fn decoded_lazy_tiles_held_by_history_are_counted() {
        let fmt = PixelFormat::RGBA8;
        let r = Rect::new(0, 0, 256, 256);
        let lazy = Surface::lazy(
            fmt,
            r,
            None,
            Box::new(move || {
                let mut s = Surface::new(fmt);
                s.fill_rect(r, &[1.0, 0.0, 0.0, 1.0]);
                s.write_pixel(0, 0, &[0.0, 0.0, 0.0, 1.0]);
                Ok(s)
            }),
        );
        let mut held = Document::new("held", Size::new(256, 256), ColorMode::Rgb, SampleType::U8);
        let mut layer = Layer::raster("hidden", fmt);
        layer.content = LayerContent::Raster(lazy.clone());
        held.layers.push(layer);
        let mut tiles = Tiles::default();
        tiles.add(&held);
        let empty = Document::new("current", Size::new(256, 256), ColorMode::Rgb, SampleType::U8);
        assert_eq!(tiles.bytes_with(&empty), 0, "pending: nothing in memory");
        lazy.tile_count();
        let tile = 256 * 256 * 4;
        assert_eq!(tiles.bytes_with(&empty), tile, "decoded, held by History only");
        assert_eq!(tiles.bytes_with(&held), tile, "and counted once when current holds it too");
        tiles.remove(&held);
        assert_eq!(tiles.bytes_with(&empty), 0);
    }
}
