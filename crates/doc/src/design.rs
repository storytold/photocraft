//! Design timeline (Fusion 360-style parametric history): the edits made to a document since its
//! timeline was turned on, each re-runnable from the document as it was then.
//!
//! Each step keeps the [`crate::IdScope`] step its ids were minted under, so replaying it
//! recreates the same layer ids, and the targets it acted on, so a step whose layer was removed
//! fails visibly instead of acting on another layer.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Document, LayerId};

#[derive(Clone, Debug, PartialEq)]
pub struct DesignTimeline {
    /// The document when the timeline started, without a timeline of its own.
    pub base: Arc<Document>,
    pub steps: Vec<DesignStep>,
    /// The rollback marker: how many steps are in effect (None = all). New steps go in here.
    pub marker: Option<usize>,
}

impl DesignTimeline {
    /// An empty timeline starting from `base`.
    pub fn new(base: Arc<Document>) -> Self {
        DesignTimeline { base, steps: Vec::new(), marker: None }
    }

    /// Steps before the marker.
    pub fn in_effect(&self) -> usize {
        self.marker.map_or(self.steps.len(), |m| m.min(self.steps.len()))
    }

    /// The position of the step with id `step`.
    pub fn index_of(&self, step: u64) -> Option<usize> {
        self.steps.iter().position(|s| s.step == step)
    }
}

/// A command run before a step that changed no document but feeds later ones (a foreground
/// colour, brush settings). Replayed ahead of the step it precedes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prelude {
    pub command: String,
    pub params: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DesignStep {
    /// The id-scope step the command ran under; also identifies the step in the timeline.
    pub step: u64,
    pub command: String,
    pub params: Value,
    /// History label of the edit.
    pub label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prelude: Vec<Prelude>,
    /// Active and selected layers when the step ran: replay re-selects them first.
    pub active: Option<LayerId>,
    pub selected: Vec<LayerId>,
    /// Layers the step added, and layers it changed or removed.
    pub created: Vec<LayerId>,
    pub changed: Vec<LayerId>,
    #[serde(default)]
    pub suppressed: bool,
    /// Shares the previous step's undo state (a coalesced edit, e.g. one typing session).
    #[serde(default)]
    pub joins_previous: bool,
    /// The result of a command that can't replay identically, applied instead of running it.
    #[serde(default, skip)]
    pub baked: Option<Arc<Baked>>,
    /// The params were edited: the next replay re-records what the step creates and changes
    /// instead of checking them.
    #[serde(default, skip)]
    pub rerecord: bool,
    /// Outcome of the last replay (Ok when recorded).
    #[serde(default, skip)]
    pub state: StepState,
    /// The document right after this step, to resume replay from. Not saved.
    #[serde(default, skip)]
    pub checkpoint: Checkpoint,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum StepState {
    #[default]
    Ok,
    /// The command failed.
    Error { message: String },
    /// A layer the step acted on no longer exists, so the step was skipped.
    MissingReference { layers: Vec<LayerId> },
    /// The layer the step placed its new layers beside is gone; they went elsewhere.
    PlacedElsewhere,
    /// The step changed a layer it didn't change when recorded, so its result was refused.
    Retargeted { layers: Vec<LayerId> },
    /// A baked step after a changed earlier step: its stored pixels may be out of date.
    Stale,
}

impl StepState {
    /// The step couldn't replay as recorded (an error, a missing layer, another target).
    pub fn is_failure(&self) -> bool {
        matches!(self, StepState::Error { .. } | StepState::MissingReference { .. } | StepState::Retargeted { .. })
    }
}

/// A document's mode, depth and ICC profile.
pub type BakedColor = (crate::ColorMode, crate::SampleType, Option<Arc<Vec<u8>>>);

/// A step's result as it came out: the layers it created and changed (a changed layer missing
/// here was removed), for each created layer the layers directly below and above it, and the
/// selection and colour state when the step changed them.
#[derive(Clone, Debug, PartialEq)]
pub struct Baked {
    pub layers: Vec<crate::Layer>,
    pub placed: Vec<(LayerId, Option<LayerId>, Option<LayerId>)>,
    pub selection: Option<Option<crate::Surface>>,
    /// The document's mode, depth and profile when the step changed them (Assign Profile, a mode
    /// change): document state no layer holds.
    pub color: Option<BakedColor>,
}

impl Baked {
    /// What `after` holds of the `created` and `changed` layers, and its selection if it differs
    /// from `before`'s.
    pub fn capture(before: &Document, after: &Document, created: &[LayerId], changed: &[LayerId]) -> Self {
        let layers = created.iter().chain(changed).filter_map(|id| after.layer(*id).cloned()).collect();
        let placed = created
            .iter()
            .filter_map(|id| {
                let path = after.path_of(*id)?;
                let (last, parent) = path.split_last()?;
                let siblings = if parent.is_empty() { &after.layers } else { after.layer_at(parent)?.children()? };
                let below = last.checked_sub(1).and_then(|i| siblings.get(i)).map(|l| l.id);
                let above = siblings.get(last + 1).map(|l| l.id);
                Some((*id, below, above))
            })
            .collect();
        let selection = (before.selection != after.selection).then(|| after.selection.clone());
        let color = ((before.mode, before.depth, &before.icc_profile) != (after.mode, after.depth, &after.icc_profile))
            .then(|| (after.mode, after.depth, after.icc_profile.clone()));
        Baked { layers, placed, selection, color }
    }

    /// Put the stored layers (and selection and colour state) into `doc` in place of the step's.
    pub fn apply(&self, doc: &mut Document, created: &[LayerId], changed: &[LayerId]) {
        if let Some(sel) = &self.selection {
            doc.selection = sel.clone();
        }
        if let Some((mode, depth, profile)) = &self.color {
            (doc.mode, doc.depth, doc.icc_profile) = (*mode, *depth, profile.clone());
        }
        for id in changed {
            match self.layers.iter().find(|l| l.id == *id) {
                Some(l) => {
                    if let Some(slot) = doc.layer_mut(*id) {
                        *slot = l.clone();
                    }
                }
                None => {
                    doc.remove(*id);
                }
            }
        }
        for (id, below, above) in &self.placed {
            let Some(l) = self.layers.iter().find(|l| l.id == *id).filter(|_| created.contains(id) && doc.layer(*id).is_none()) else { continue };
            match (below.filter(|b| doc.layer(*b).is_some()), above.filter(|a| doc.layer(*a).is_some())) {
                (Some(b), _) => doc.insert_above(Some(b), l.clone()),
                (None, Some(a)) => doc.insert_below(Some(a), l.clone()),
                (None, None) => doc.insert_above(None, l.clone()),
            };
        }
    }
}

/// A cached document state. Never compared: two timelines are equal whatever they cache.
#[derive(Clone, Debug, Default)]
pub struct Checkpoint(pub Option<Arc<Document>>);

impl PartialEq for Checkpoint {
    /// Always equal: a cache never makes two timelines differ.
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// Layers `after` added and layers it changed or removed, compared with `before`.
pub fn layer_changes(before: &Document, after: &Document) -> (Vec<LayerId>, Vec<LayerId>) {
    let old: std::collections::HashMap<LayerId, &crate::Layer> = before.walk().into_iter().map(|(_, _, l)| (l.id, l)).collect();
    let new: std::collections::HashMap<LayerId, &crate::Layer> = after.walk().into_iter().map(|(_, _, l)| (l.id, l)).collect();
    let mut created: Vec<LayerId> = after.walk().into_iter().map(|(_, _, l)| l.id).filter(|id| !old.contains_key(id)).collect();
    let mut changed: Vec<LayerId> =
        before.walk().into_iter().filter(|(_, _, l)| new.get(&l.id).is_none_or(|n| !same_layer(l, n))).map(|(_, _, l)| l.id).collect();
    created.sort_unstable();
    changed.sort_unstable();
    (created, changed)
}

/// A layer's own properties and pixels, ignoring its children (a group isn't changed because a
/// layer inside it is).
fn same_layer(a: &crate::Layer, b: &crate::Layer) -> bool {
    match (a.children(), b.children()) {
        (Some(ca), Some(cb)) => {
            let ids = |c: &[crate::Layer]| c.iter().map(|l| l.id).collect::<Vec<_>>();
            ids(ca) == ids(cb) && strip_children(a) == strip_children(b)
        }
        _ => a == b,
    }
}

/// A copy of `l` without its children.
fn strip_children(l: &crate::Layer) -> crate::Layer {
    let mut l = l.clone();
    if let Some(c) = l.children_mut() {
        c.clear();
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorMode, Layer, PixelFormat, SampleType, Size};

    #[test]
    fn layer_changes_reports_own_changes_not_a_parents() {
        let mut before = Document::new("d", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
        let child = Layer::raster("c", PixelFormat::RGBA8);
        let gone = Layer::raster("gone", PixelFormat::RGBA8);
        let (cid, gid) = (child.id, gone.id);
        before.layers = vec![Layer::group("g", vec![child]), gone];
        let mut after = before.clone();
        after.layers.pop();
        after.layer_mut(cid).unwrap().opacity = 0.5;
        let added = Layer::raster("new", PixelFormat::RGBA8);
        let aid = added.id;
        after.layers.push(added);
        let (created, changed) = layer_changes(&before, &after);
        assert_eq!(created, vec![aid]);
        let mut want = vec![cid, gid];
        want.sort_unstable();
        assert_eq!(changed, want);
    }

    #[test]
    fn baked_layers_go_back_where_they_were() {
        let mut before = Document::new("d", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
        let (low, high) = (Layer::raster("low", PixelFormat::RGBA8), Layer::raster("high", PixelFormat::RGBA8));
        let (lid, hid) = (low.id, high.id);
        before.layers = vec![low, high];
        let mut after = before.clone();
        let mid = Layer::raster("mid", PixelFormat::RGBA8);
        let mid_id = mid.id;
        after.insert_above(Some(lid), mid);
        after.layer_mut(hid).unwrap().opacity = 0.25;
        let baked = Baked::capture(&before, &after, &[mid_id], &[hid]);
        let mut doc = before.clone();
        baked.apply(&mut doc, &[mid_id], &[hid]);
        assert_eq!(doc, after);
        assert_eq!(baked.selection, None, "the selection didn't change");
    }
}
