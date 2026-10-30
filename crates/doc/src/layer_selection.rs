//! Layer targeting transported by file import and save snapshots, separate from live editing state.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{Document, LayerId};

/// The primary layer and selected set a file should reopen with. An empty value explicitly
/// deselects every layer; the document's optional field distinguishes that from absent metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedLayerSelection {
    pub active: Option<LayerId>,
    pub selected: Vec<LayerId>,
}

impl SavedLayerSelection {
    fn top(doc: &Document) -> Self {
        let active = doc.top_layer();
        Self { active, selected: active.into_iter().collect() }
    }

    /// Resolve file-provided targeting against the loaded tree. A nonempty selected set is
    /// authoritative: discard missing IDs and duplicates, keeping its order and its primary
    /// when still a member; otherwise the last surviving member becomes primary. Active-only
    /// metadata selects that layer. Entirely stale targeting falls back to the top layer.
    /// An originally empty value remains deselected, including in a nonempty document.
    ///
    /// The walk is iterative and linear in the layer and selection counts, so an untrusted
    /// tree or selection does not add recursive traversal or repeated layer lookups.
    pub fn resolve(mut self, doc: &Document) -> Self {
        if self.active.is_none() && self.selected.is_empty() {
            return self;
        }
        let mut live = HashSet::new();
        let mut stack = vec![doc.layers.as_slice()];
        while let Some(layers) = stack.pop() {
            for layer in layers {
                live.insert(layer.id);
                if let Some(children) = layer.children() {
                    stack.push(children);
                }
            }
        }
        if self.selected.is_empty() {
            return match self.active.filter(|id| live.contains(id)) {
                Some(active) => Self { active: Some(active), selected: vec![active] },
                None => Self::top(doc),
            };
        }
        let mut seen = HashSet::new();
        self.selected.retain(|id| live.contains(id) && seen.insert(*id));
        if self.selected.is_empty() {
            return Self::top(doc);
        }
        self.active = self.active.filter(|id| seen.contains(id)).or_else(|| self.selected.last().copied());
        self
    }
}

impl Document {
    /// Consume a file's layer targeting when it enters an editing session. Missing metadata
    /// keeps the ordinary top-layer default; the returned primary always belongs to its set.
    pub fn take_saved_layer_selection(&mut self) -> SavedLayerSelection {
        match self.saved_layer_selection.take() {
            Some(selection) => selection.resolve(self),
            None => SavedLayerSelection::top(self),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorMode, Layer, SampleType, Size};

    fn document() -> Document {
        let mut doc = Document::new("selection", Size::new(2, 1), ColorMode::Rgb, SampleType::U8);
        let child = Layer::raster("child", doc.pixel_format());
        doc.layers.push(Layer::group("group", vec![child]));
        doc.layers.push(Layer::raster("top", doc.pixel_format()));
        doc
    }

    #[test]
    fn resolves_nested_ids_and_deduplicates_without_changing_order() {
        let doc = document();
        let group = doc.layers.first().unwrap();
        let child = group.children().unwrap().first().unwrap().id;
        let top = doc.top_layer().unwrap();
        let selection = SavedLayerSelection { active: Some(child), selected: vec![top, child, top, LayerId(u64::MAX)] }.resolve(&doc);
        assert_eq!(selection, SavedLayerSelection { active: Some(child), selected: vec![top, child] });
    }

    #[test]
    fn selected_set_wins_over_a_conflicting_or_stale_primary() {
        let doc = document();
        let group = doc.layers.first().unwrap().id;
        let top = doc.top_layer().unwrap();
        for active in [None, Some(top), Some(LayerId(u64::MAX))] {
            assert_eq!(SavedLayerSelection { active, selected: vec![group] }.resolve(&doc), SavedLayerSelection { active: Some(group), selected: vec![group] });
        }
        assert_eq!(
            SavedLayerSelection { active: None, selected: vec![top, group] }.resolve(&doc).active,
            Some(group),
            "fallback is the last surviving input member"
        );
    }

    #[test]
    fn absent_stale_and_explicitly_empty_targeting_are_distinct() {
        let mut doc = document();
        let top = doc.top_layer().unwrap();
        let default = SavedLayerSelection { active: Some(top), selected: vec![top] };
        assert_eq!(doc.take_saved_layer_selection(), default);
        for stale in [
            SavedLayerSelection { active: Some(LayerId(u64::MAX)), selected: Vec::new() },
            SavedLayerSelection { active: Some(top), selected: vec![LayerId(u64::MAX)] },
        ] {
            doc.saved_layer_selection = Some(stale);
            assert_eq!(doc.take_saved_layer_selection(), default);
            assert!(doc.saved_layer_selection.is_none());
        }
        doc.saved_layer_selection = Some(SavedLayerSelection::default());
        assert_eq!(doc.take_saved_layer_selection(), SavedLayerSelection::default());
        assert!(doc.saved_layer_selection.is_none());
        assert_eq!(SavedLayerSelection { active: Some(top), selected: Vec::new() }.resolve(&doc), default);
        doc.layers.clear();
        assert_eq!(SavedLayerSelection { active: Some(top), selected: vec![top] }.resolve(&doc), SavedLayerSelection::default());
    }
}
