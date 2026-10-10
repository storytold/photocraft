//! Independent integer raster moves retain one immutable source plus offsets, not shifted pixels.
use crate::Session;
use photocraft_doc::{Document, LayerContent, LayerId};
use std::sync::{Arc, Weak};

#[derive(Debug)]
struct Moved {
    source: Arc<Document>,
    layers: Vec<LayerId>,
    dx: i32,
    dy: i32,
}
impl photocraft_ops::ArchivedDocument for Moved {
    fn load(&self) -> Result<Arc<Document>, String> {
        if self.dx == 0 && self.dy == 0 {
            return Ok(self.source.clone());
        }
        let mut doc = (*self.source).clone();
        let moves: Vec<_> = self.layers.iter().map(|id| (*id, self.dx, self.dy)).collect();
        crate::layer_multi_cmds::move_layers(&mut doc, &moves).map_err(|e| e.to_string())?;
        Ok(Arc::new(doc))
    }
    fn metadata_bytes(&self) -> usize {
        std::mem::size_of::<Self>().saturating_add(self.layers.capacity().saturating_mul(std::mem::size_of::<LayerId>()))
    }
    fn resident(&self) -> Option<&Arc<Document>> {
        Some(&self.source)
    }
}

pub(crate) struct Run {
    source: Weak<Document>,
    current: Weak<Document>,
    layers: Vec<LayerId>,
    dx: i32,
    dy: i32,
}

impl Session {
    pub(crate) fn compact_translation(&mut self, before: &Arc<Document>, layers: &[LayerId], dx: i32, dy: i32) {
        // Other content has rendering, canvas fitting or mask semantics: keep its exact snapshots.
        if !before.slices.list.is_empty()
            || !layers.iter().all(|id| {
                before.layer(*id).is_some_and(|l| {
                    matches!(l.content, LayerContent::Raster(_)) && l.mask.is_none() && l.vector_mask.is_none() && l.effects.reference.is_none()
                })
            })
        {
            self.compact_move = None;
            return;
        }
        let previous = self.compact_move.as_ref().filter(|run| run.layers == layers && run.current.upgrade().is_some_and(|doc| Arc::ptr_eq(&doc, before)));
        let (source, x, y) = match previous.and_then(|run| run.source.upgrade().map(|source| (source, run.dx, run.dy))) {
            Some(value) => value,
            None => (before.clone(), 0, 0),
        };
        let (Some(next_x), Some(next_y)) = (x.checked_add(dx), y.checked_add(dy)) else {
            self.compact_move = None;
            return;
        };
        // Near the coordinate limits, intermediate saturation can discard pixels. Replay
        // is permitted only where both the old and new translations are ordinary addition.
        let safe = layers.iter().all(|id| {
            source.layer(*id).and_then(|layer| layer.surface()).is_some_and(|surface| {
                let bounds = surface.tile_bounds();
                [
                    (bounds.x0, x),
                    (bounds.x1, x),
                    (bounds.y0, y),
                    (bounds.y1, y),
                    (bounds.x0, next_x),
                    (bounds.x1, next_x),
                    (bounds.y0, next_y),
                    (bounds.y1, next_y),
                ]
                .into_iter()
                .all(|(value, offset)| value.checked_add(offset).is_some_and(|n| (-1_000_000_000..=1_000_000_000).contains(&n)))
            })
        });
        if !safe {
            self.compact_move = None;
            return;
        }
        let archive = Arc::new(Moved { source: source.clone(), layers: layers.to_vec(), dx: x, dy: y });
        let Some(st) = self.active_mut() else { return };
        st.history.replace_resident(before, archive);
        st.history.set_current_archive(&st.doc, Arc::new(Moved { source: source.clone(), layers: layers.to_vec(), dx: next_x, dy: next_y }));
        self.compact_move = Some(Run { source: Arc::downgrade(&source), current: Arc::downgrade(&st.doc), layers: layers.to_vec(), dx: next_x, dy: next_y });
    }
}

#[cfg(test)]
mod tests;
