//! Persistent layer ids (`Layer::psd_id`, the PSD `lyid`): every layer gets one when it is
//! created (new, duplicate, paste, place, open), as in Photoshop, and keeps it through undo,
//! redo, snapshots and saves in PSD, PSB, layered TIFF and `.pcraft`. A layer's `LayerId` belongs
//! to the session (a PSD's layers get new ones every time it opens); a tool that records a layer
//! names this id instead (`layer.persistentId`, `document.inspect`).
//!
//! Ids are unique within the document and never given out twice in a session: each document's
//! [`LayerIds`] remembers, outside history, every id it has given or adopted and the layer that
//! has it. An undone layer's id is not given to the next new layer, and a copy of a layer gets a
//! new id. A reopened file continues after its highest id, as Photoshop does.

use std::collections::{HashMap, HashSet};

use photocraft_doc::{Document, Layer, LayerId};

/// How a document got its layers, for [`LayerIds::assign`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Arrival {
    /// An edit: a layer keeps an id only when it already has it in this document (undo, redo and
    /// snapshots bring layers back with theirs). Any other layer, a copy among them, gets a new
    /// one, so an id never moves to another layer.
    Edit,
    /// The document's own file (open, revert): its ids are adopted, and a layer without one, or
    /// with one another layer of the file has, gets a new one.
    File,
}

/// The persistent layer ids a document has given out this session.
#[derive(Debug, Clone)]
pub(crate) struct LayerIds {
    /// Every id given or adopted, and the layer that has it.
    owners: HashMap<u32, LayerId>,
    /// The next id to give: above every id in `owners` (a `u64`, so running past `u32::MAX`
    /// is a state, not an overflow).
    next: u64,
}

impl Default for LayerIds {
    fn default() -> Self {
        Self { owners: HashMap::new(), next: 1 }
    }
}

impl LayerIds {
    /// Give every layer of `doc` without a valid id of its own one (see [`Arrival`]).
    pub(crate) fn assign(&mut self, doc: &mut Document, arrival: Arrival) {
        if arrival == Arrival::File {
            // New ids go above the file's, so a layer numbered first never takes one a later
            // layer of the file has.
            let top = doc.walk().iter().filter_map(|(_, _, l)| l.psd_id).max().unwrap_or(0);
            self.next = self.next.max(u64::from(top) + 1);
        }
        let mut seen = HashSet::new();
        // Depth-first in document order, without recursion: documents can nest deeply.
        let mut stack: Vec<&mut Layer> = doc.layers.iter_mut().rev().collect();
        while let Some(layer) = stack.pop() {
            let keep = layer.psd_id.filter(|id| {
                *id != 0
                    && !seen.contains(id)
                    && match arrival {
                        Arrival::Edit => self.owners.get(id) == Some(&layer.id),
                        Arrival::File => true,
                    }
            });
            let id = keep.or_else(|| self.fresh());
            if let Some(id) = id {
                seen.insert(id);
                // The common case, a layer keeping its id, changes nothing here.
                if self.owners.get(&id) != Some(&layer.id) {
                    self.owners.insert(id, layer.id);
                    self.next = self.next.max(u64::from(id) + 1);
                }
            }
            layer.psd_id = id;
            if let Some(children) = layer.children_mut() {
                stack.extend(children.iter_mut().rev());
            }
        }
    }

    /// An id never given in this session: the next one, or past `u32::MAX` the lowest free one.
    /// `None` only once every id is taken (the PSD writer then numbers the layer).
    fn fresh(&mut self) -> Option<u32> {
        if let Ok(id) = u32::try_from(self.next) {
            self.next += 1;
            return Some(id);
        }
        (1..=u32::MAX).find(|id| !self.owners.contains_key(id))
    }
}

#[cfg(test)]
mod tests {
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::{LayerContent, Size};
    use photocraft_raster::Surface;

    use super::*;

    fn layer(name: &str, psd_id: Option<u32>) -> Layer {
        let mut l = Layer::new(name, LayerContent::Raster(Surface::new(photocraft_color::PixelFormat::RGBA8)));
        l.psd_id = psd_id;
        l
    }

    fn ids(doc: &Document) -> Vec<(String, Option<u32>)> {
        doc.walk().iter().map(|(_, _, l)| (l.name.clone(), l.psd_id)).collect()
    }

    #[test]
    fn a_file_keeps_its_ids_and_numbers_the_rest_above_them() {
        let mut doc = Document::new("d", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
        let group = Layer::group("group", vec![layer("inner", Some(9)), layer("twin", Some(9))]);
        doc.layers = vec![layer("none", None), layer("zero", Some(0)), group, layer("five", Some(5))];
        let mut ids_ = LayerIds::default();
        ids_.assign(&mut doc, Arrival::File);
        assert_eq!(
            ids(&doc),
            [("none", Some(10)), ("zero", Some(11)), ("group", Some(12)), ("inner", Some(9)), ("twin", Some(13)), ("five", Some(5))]
                .map(|(n, i)| (n.to_string(), i))
        );
    }

    #[test]
    fn an_edit_keeps_owned_ids_and_never_reuses_one() {
        let mut doc = Document::new("d", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
        doc.layers = vec![layer("a", None), layer("b", None)];
        let mut ids_ = LayerIds::default();
        ids_.assign(&mut doc, Arrival::File);
        let (a, b) = (doc.layers[0].clone(), doc.layers[1].clone());
        assert_eq!((a.psd_id, b.psd_id), (Some(1), Some(2)));
        // A copy carrying `a`'s id, and a layer arriving with an id no layer here has: both new.
        let mut copy = a.clone();
        copy.id = LayerId::fresh();
        let mut stranger = layer("stranger", Some(77));
        stranger.id = LayerId::fresh();
        doc.layers.push(copy);
        doc.layers.push(stranger);
        ids_.assign(&mut doc, Arrival::Edit);
        assert_eq!(ids(&doc).iter().map(|(_, i)| *i).collect::<Vec<_>>(), [Some(1), Some(2), Some(3), Some(4)]);
        // `b` deleted (an undo of its creation, say): its id is not given to the next layer, and
        // `b` brought back (redo, a snapshot) keeps it.
        doc.layers.retain(|l| l.id != b.id);
        doc.layers.push(layer("new", None));
        ids_.assign(&mut doc, Arrival::Edit);
        assert_eq!(doc.layers.last().unwrap().psd_id, Some(5));
        doc.layers.push(b.clone());
        ids_.assign(&mut doc, Arrival::Edit);
        assert_eq!(doc.layer(b.id).unwrap().psd_id, Some(2));
    }

    #[test]
    fn past_the_last_id_the_lowest_free_one_is_given() {
        let mut doc = Document::new("d", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
        doc.layers = vec![layer("max", Some(u32::MAX)), layer("one", Some(1)), layer("new", None)];
        let mut ids_ = LayerIds::default();
        ids_.assign(&mut doc, Arrival::File);
        assert_eq!(doc.layers[2].psd_id, Some(2));
    }
}
