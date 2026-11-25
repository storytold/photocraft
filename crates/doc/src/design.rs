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

#[derive(Clone, Debug, PartialEq, craft_ui_macros::PlainData)]
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

    /// This timeline keeping only the checkpoint of the last step in effect (the document it
    /// belongs to, so it holds no pixels of its own), or `None` when it keeps no other. For an undo
    /// state: the other checkpoints are caches a later timeline's budget may already have released,
    /// and no history budget counts the pixels they hold.
    pub fn without_spare_checkpoints(&self) -> Option<Self> {
        let keep = self.in_effect().checked_sub(1);
        let spare = |(i, st): (usize, &DesignStep)| Some(i) != keep && st.checkpoint.0.is_some();
        self.steps.iter().enumerate().any(spare).then(|| {
            let mut t = self.clone();
            for (i, st) in t.steps.iter_mut().enumerate() {
                if Some(i) != keep {
                    st.checkpoint = Checkpoint(None);
                }
            }
            t
        })
    }

    /// The position of the step with id `step`.
    pub fn index_of(&self, step: u64) -> Option<usize> {
        self.steps.iter().position(|s| s.step == step)
    }

    /// Every pixel surface a save writes for the timeline: the base and the baked results.
    pub fn for_each_surface(&self, mut visit: impl FnMut(&crate::Surface)) {
        self.base.for_each_surface(&mut visit);
        for p in self.steps.iter().flat_map(|st| &st.patterns) {
            visit(&p.surface);
        }
        for b in self.steps.iter().filter_map(|st| st.baked.as_deref()) {
            for l in &b.layers {
                crate::layer_surfaces(l, &mut visit);
            }
            if let Some(Some(sel)) = &b.selection {
                visit(sel);
            }
            if let Some((doc, _)) = &b.state {
                doc.for_each_surface(&mut visit);
            }
        }
    }
}

/// A command run before a step that changed no document but feeds later ones (a foreground
/// colour, brush settings). Replayed ahead of the step it precedes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, craft_ui_macros::PlainData)]
pub struct Prelude {
    pub command: String,
    #[plain_data(assume = "JSON value: plain data")]
    pub params: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, craft_ui_macros::PlainData)]
pub struct DesignStep {
    /// The id-scope step the command ran under; also identifies the step in the timeline.
    pub step: u64,
    pub command: String,
    #[plain_data(assume = "JSON value: plain data")]
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
    /// Outcome of the last replay (Ok when recorded). Only a stale warning is saved: the other
    /// outcomes are the next replay's to find again, while a stale result stays stale.
    #[serde(default, skip_serializing_if = "StepState::is_transient", deserialize_with = "stale_only")]
    pub state: StepState,
    /// The document right after this step, to resume replay from. Not saved.
    #[serde(default, skip)]
    pub checkpoint: Checkpoint,
    /// The library patterns the step used in a version no step before it holds, as they were: a
    /// replay installs them first, whatever the library holds by then.
    #[serde(default, skip)]
    pub patterns: Vec<crate::pattern::Pattern>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, craft_ui_macros::PlainData)]
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

    /// An outcome the next replay finds again, so a save leaves it out: all but [`StepState::Stale`].
    pub fn is_transient(&self) -> bool {
        *self != StepState::Stale
    }
}

/// A saved step state: a stale warning is kept, anything else reads as Ok (the next replay finds it).
fn stale_only<'de, D: serde::Deserializer<'de>>(d: D) -> Result<StepState, D::Error> {
    let state = StepState::deserialize(d)?;
    Ok(if state.is_transient() { StepState::Ok } else { state })
}

/// A document's mode, depth and ICC profile.
pub type BakedColor = (crate::ColorMode, crate::SampleType, Option<Arc<Vec<u8>>>);

/// A step's result as it came out: the layers it created and changed (a changed layer missing
/// here was removed), for each created layer the layers directly below and above it, and the
/// selection and colour state when the step changed them.
#[derive(Clone, Debug, PartialEq, craft_ui_macros::PlainData)]
pub struct Baked {
    pub layers: Vec<crate::Layer>,
    pub placed: Vec<(LayerId, Option<LayerId>, Option<LayerId>)>,
    pub selection: Option<Option<crate::Surface>>,
    /// The document's mode, depth and profile when the step changed them (Assign Profile, a mode
    /// change): document state no layer holds.
    pub color: Option<BakedColor>,
    /// Any other document state the step changed (imported data sets, converted text styles, a
    /// palette): the document after it, without layers, and the names of the fields it changed.
    pub state: Option<(Arc<Document>, Vec<String>)>,
    /// The root stack bottom to top when the step reordered it (no layer records a root move).
    pub root: Option<Vec<LayerId>>,
    /// Layers the step moved to another parent (into or out of a group), with the layers directly
    /// below and above where it put them: no layer records the move of an unchanged one.
    pub moved: Vec<(LayerId, Option<LayerId>, Option<LayerId>)>,
}

/// The document fields a baked step's `state` restores: all but those it keeps apart (`layers`,
/// `selection`, the colour state) and identity. Destructured, so a new field fails to compile
/// here until it is listed.
macro_rules! doc_state {
    ($($f:ident),* $(,)?) => {
        /// The names of the [`doc_state`] fields `after` changed from `before`.
        fn changed_state(before: &Document, after: &Document) -> Vec<String> {
            let Document { id: _, name: _, layers: _, selection: _, mode: _, depth: _, icc_profile: _, design_timeline: _, $($f: _),* } = after;
            let mut names = Vec::new();
            $(if before.$f != after.$f { names.push(stringify!($f).to_string()); })*
            names
        }

        /// Copy the [`doc_state`] field `name` from `src` into `dst` (an unknown name, from a
        /// damaged file, copies nothing).
        fn copy_state(dst: &mut Document, src: &Document, name: &str) {
            match name {
                $(stringify!($f) => dst.$f = src.$f.clone(),)*
                _ => {}
            }
        }
    };
}

doc_state!(
    size,
    resolution_dpi,
    channels,
    guides,
    ruler_origin,
    metadata,
    global_light,
    paths,
    work_path,
    clipping_path,
    quick_mask,
    quick_mask_unselected,
    patterns,
    color_table,
    duotone,
    layer_comps,
    last_applied_comp,
    last_document_state,
    measurement,
    notes,
    text_styles,
    slices,
    variables,
    timeline,
    color_indices,
    file_encoding,
);

impl Baked {
    /// What `after` holds of the `created` and `changed` layers, and its selection if it differs
    /// from `before`'s.
    pub fn capture(before: &Document, after: &Document, created: &[LayerId], changed: &[LayerId]) -> Self {
        let layers = created.iter().chain(changed).filter_map(|id| after.layer(*id).cloned()).collect();
        // A layer's place in `d`: its parent group (None at the root) and its neighbours below and above.
        let place = |d: &Document, id: LayerId| {
            let path = d.path_of(id)?;
            let (last, parent) = path.split_last()?;
            let (group, siblings) = if parent.is_empty() { (None, &d.layers[..]) } else { (Some(d.layer_at(parent)?), d.layer_at(parent)?.children()?) };
            let below = last.checked_sub(1).and_then(|i| siblings.get(i)).map(|l| l.id);
            let above = siblings.get(last + 1).map(|l| l.id);
            Some((group.map(|g| g.id), below, above))
        };
        let placed = created.iter().filter_map(|id| place(after, *id).map(|(_, below, above)| (*id, below, above))).collect();
        let moved = after
            .walk()
            .into_iter()
            .map(|(_, _, l)| l.id)
            .filter(|id| !created.contains(id))
            .filter_map(|id| {
                let (from, _, _) = place(before, id)?;
                let (to, below, above) = place(after, id)?;
                (from != to).then_some((id, below, above))
            })
            .collect();
        let selection = (before.selection != after.selection).then(|| after.selection.clone());
        let color = ((before.mode, before.depth, &before.icc_profile) != (after.mode, after.depth, &after.icc_profile))
            .then(|| (after.mode, after.depth, after.icc_profile.clone()));
        let fields = changed_state(before, after);
        let state = (!fields.is_empty()).then(|| (Arc::new(Document { layers: Default::default(), design_timeline: None, ..after.clone() }), fields));
        // Only the layers in the root both before and after: a created or removed one moves the
        // others without reordering them.
        let ids = |d: &Document, other: &Document| d.layers.iter().map(|l| l.id).filter(|id| other.layers.iter().any(|o| o.id == *id)).collect::<Vec<_>>();
        let root = (ids(before, after) != ids(after, before)).then(|| after.layers.iter().map(|l| l.id).collect());
        Baked { layers, placed, selection, color, state, root, moved }
    }

    /// Put the stored layers (and selection and colour state) into `doc` in place of the step's.
    pub fn apply(&self, doc: &mut Document, created: &[LayerId], changed: &[LayerId]) {
        if let Some(sel) = &self.selection {
            doc.selection = sel.clone();
        }
        if let Some((mode, depth, profile)) = &self.color {
            (doc.mode, doc.depth, doc.icc_profile) = (*mode, *depth, profile.clone());
        }
        if let Some((src, fields)) = &self.state {
            for f in fields {
                copy_state(doc, src, f);
            }
        }
        // The document before any of this step's layers go in: a changed group keeps the children it
        // has here (a replay may have changed them) unless the step itself changed them.
        let current = doc.clone();
        // A moved layer leaves its old place first; it goes in where the step put it below.
        for (id, _, _) in &self.moved {
            doc.remove(*id);
        }
        for id in changed {
            match self.layers.iter().find(|l| l.id == *id) {
                Some(l) => {
                    let merged = self.merged(l, &current, created, changed);
                    if let Some(slot) = doc.layer_mut(*id) {
                        *slot = merged;
                    }
                }
                None => {
                    doc.remove(*id);
                }
            }
        }
        // Each created layer goes beside a recorded neighbour; one whose neighbours are themselves
        // still to come waits for them, so the stack comes out as recorded whatever the id order.
        // A moved layer a changed group already took in needs no place of its own.
        let mut pending: Vec<&(LayerId, Option<LayerId>, Option<LayerId>)> =
            self.placed.iter().filter(|(id, _, _)| created.contains(id)).chain(&self.moved).filter(|(id, _, _)| doc.layer(*id).is_none()).collect();
        let mut fallback = false;
        while !pending.is_empty() {
            let before = pending.len();
            pending.retain(|(id, below, above)| {
                // As the step left it if it made or changed it, else as the replay has it.
                let l = match self.layers.iter().find(|l| l.id == *id) {
                    Some(l) => self.merged(l, &current, created, changed),
                    None => match current.layer(*id) {
                        Some(l) => l.clone(),
                        None => return false,
                    },
                };
                match (below.filter(|b| doc.layer(*b).is_some()), above.filter(|a| doc.layer(*a).is_some())) {
                    (Some(b), _) => doc.insert_above(Some(b), l),
                    (None, Some(a)) => doc.insert_below(Some(a), l),
                    // No neighbour left at all: the top of the stack.
                    (None, None) if fallback => doc.insert_above(None, l),
                    (None, None) => return true,
                };
                false
            });
            fallback = pending.len() == before;
        }
        if let Some(order) = &self.root {
            reorder_root(doc, order);
        }
    }

    /// The stored layer `l` with its children as the replayed document has them: a child the step
    /// created or changed comes from the stored result, any other from `current`. A stored child
    /// the replay lacks comes back only if this step created it (an earlier step that made it may
    /// be suppressed), and a child the replay has there stays unless this step removed it or moved
    /// it into another stored layer.
    fn merged(&self, l: &crate::Layer, current: &Document, created: &[LayerId], changed: &[LayerId]) -> crate::Layer {
        let mut l = l.clone();
        let id = l.id;
        let Some(children) = l.children_mut() else { return l };
        let mut kept: Vec<crate::Layer> = Vec::with_capacity(children.len());
        for c in children.iter() {
            if created.contains(&c.id) || changed.contains(&c.id) {
                kept.push(self.merged(c, current, created, changed));
            } else if let Some(now) = current.layer(c.id) {
                kept.push(now.clone());
            }
        }
        // The replayed group's own children this step left alone, each after the one it follows there.
        if let Some(replayed) = current.layer(id).and_then(crate::Layer::children) {
            let mut after: Option<LayerId> = None;
            for c in replayed {
                let placed = kept.iter().any(|k| k.id == c.id);
                let moved = self.moved.iter().any(|(m, _, _)| *m == c.id);
                if !placed && !moved && !changed.contains(&c.id) && !self.holds(c.id) {
                    let at = after.and_then(|a| kept.iter().position(|k| k.id == a)).map_or(0, |i| i + 1);
                    kept.insert(at, c.clone());
                }
                after = Some(c.id);
            }
        }
        *children = kept;
        l
    }

    /// Whether a stored layer is `id` or holds it.
    fn holds(&self, id: LayerId) -> bool {
        fn within(l: &crate::Layer, id: LayerId) -> bool {
            l.id == id || l.children().is_some_and(|c| c.iter().any(|c| within(c, id)))
        }
        self.layers.iter().any(|l| within(l, id))
    }
}

/// Put the root layers named in `order` back in that order, in the places they hold; any other
/// root layer stays where it is.
fn reorder_root(doc: &mut Document, order: &[LayerId]) {
    let slots: Vec<usize> = doc.layers.iter().enumerate().filter(|(_, l)| order.contains(&l.id)).map(|(i, _)| i).collect();
    let mut moved: Vec<crate::Layer> = slots.iter().map(|i| doc.layers[*i].clone()).collect();
    moved.sort_by_key(|l| order.iter().position(|id| *id == l.id));
    let layers = &mut *doc.layers;
    for (slot, l) in slots.into_iter().zip(moved) {
        layers[slot] = l;
    }
}

/// A cached document state. Never compared: two timelines are equal whatever they cache.
#[derive(Clone, Debug, Default, craft_ui_macros::PlainData)]
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
        before.layers = vec![Layer::group("g", vec![child]), gone].into();
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
        before.layers = vec![low, high].into();
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

    /// A step that changed only a group's own properties leaves its children as the replay has them.
    #[test]
    fn a_baked_group_change_keeps_the_replayed_children() {
        let mut before = Document::new("d", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
        let child = Layer::raster("c", PixelFormat::RGBA8);
        let cid = child.id;
        let group = Layer::group("g", vec![child]);
        let gid = group.id;
        before.layers = vec![group].into();
        let mut after = before.clone();
        after.layer_mut(gid).unwrap().opacity = 0.5;
        let (created, changed) = layer_changes(&before, &after);
        assert_eq!(changed, vec![gid]);
        let baked = Baked::capture(&before, &after, &created, &changed);
        // The replay reaches the step with the child changed by an earlier step.
        let mut doc = before.clone();
        doc.layer_mut(cid).unwrap().opacity = 0.25;
        baked.apply(&mut doc, &created, &changed);
        assert_eq!(doc.layer(gid).unwrap().opacity, 0.5, "the group's own change");
        assert_eq!(doc.layer(cid).unwrap().opacity, 0.25, "the child as the replay has it");
    }

    /// A child an earlier step created, suppressed since, doesn't come back through a later baked
    /// change to its group's own properties.
    #[test]
    fn a_baked_group_change_brings_back_no_child_of_a_suppressed_step() {
        let mut before = Document::new("d", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
        let (a, b) = (Layer::raster("A", PixelFormat::RGBA8), Layer::raster("B", PixelFormat::RGBA8));
        let aid = a.id;
        let group = Layer::group("g", vec![a.clone(), b]);
        let gid = group.id;
        // As recorded: B (made by an earlier step) is in the group when this step changes it.
        before.layers = vec![group].into();
        let mut after = before.clone();
        after.layer_mut(gid).unwrap().opacity = 0.5;
        let (created, changed) = layer_changes(&before, &after);
        let baked = Baked::capture(&before, &after, &created, &changed);
        // The replay, with the step that made B suppressed.
        let mut doc = Document::new("d", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
        doc.layers = vec![Layer { id: gid, ..Layer::group("g", vec![a]) }].into();
        baked.apply(&mut doc, &created, &changed);
        let ids: Vec<LayerId> = doc.layer(gid).unwrap().children().unwrap().iter().map(|l| l.id).collect();
        assert_eq!(ids, [aid]);
        assert_eq!(doc.layer(gid).unwrap().opacity, 0.5);
    }

    /// A step that moves a layer into a group, or out of one to the root, replays the move: the
    /// layer leaves where it was and is where the step put it, once.
    #[test]
    fn a_baked_move_between_parents_replays() {
        let c = Layer::raster("C", PixelFormat::RGBA8);
        let cid = c.id;
        let group = Layer::group("g", Vec::new());
        let gid = group.id;
        let root_c = {
            let mut d = Document::new("d", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
            d.layers = vec![c.clone(), group.clone()].into();
            d
        };
        let c_in_g = {
            let mut d = Document::new("d", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
            d.layers = vec![Layer { id: gid, ..Layer::group("g", vec![c.clone()]) }].into();
            d
        };
        let ids = |d: &Document| d.walk().into_iter().map(|(_, _, l)| l.id).collect::<Vec<_>>();
        for (before, after) in [(&root_c, &c_in_g), (&c_in_g, &root_c)] {
            let (created, changed) = layer_changes(before, after);
            let baked = Baked::capture(before, after, &created, &changed);
            let mut doc = before.clone();
            baked.apply(&mut doc, &created, &changed);
            assert_eq!(ids(&doc), ids(after), "moved once, to where the step put it");
            assert_eq!(doc.path_of(cid).map(|p| p.len()), after.path_of(cid).map(|p| p.len()));
        }
    }

    /// Created layers go back below a surviving layer as recorded, whatever order their ids are in.
    #[test]
    fn baked_layers_created_below_a_surviving_one_stay_below_it() {
        let mut before = Document::new("d", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
        let (a, b, x) = (Layer::raster("A", PixelFormat::RGBA8), Layer::raster("B", PixelFormat::RGBA8), Layer::raster("X", PixelFormat::RGBA8));
        let (aid, bid, xid) = (a.id, b.id, x.id);
        before.layers = vec![x.clone()].into();
        let mut after = before.clone();
        after.layers = vec![a, b, x].into();
        let (created, changed) = layer_changes(&before, &after);
        assert!(aid < bid, "A has the lower id");
        let baked = Baked::capture(&before, &after, &created, &changed);
        let mut doc = before.clone();
        baked.apply(&mut doc, &created, &changed);
        assert_eq!(doc.layers.iter().map(|l| l.id).collect::<Vec<_>>(), [aid, bid, xid]);
    }

    /// A step that only reorders root layers replays the order.
    #[test]
    fn a_baked_root_reorder_replays() {
        let mut before = Document::new("d", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
        let (a, b) = (Layer::raster("A", PixelFormat::RGBA8), Layer::raster("B", PixelFormat::RGBA8));
        let (aid, bid) = (a.id, b.id);
        before.layers = vec![a.clone(), b.clone()].into();
        let mut after = before.clone();
        after.layers = vec![b, a].into();
        let (created, changed) = layer_changes(&before, &after);
        assert!(created.is_empty() && changed.is_empty(), "no layer records a root move");
        let baked = Baked::capture(&before, &after, &created, &changed);
        let mut doc = before.clone();
        baked.apply(&mut doc, &created, &changed);
        assert_eq!(doc.layers.iter().map(|l| l.id).collect::<Vec<_>>(), [bid, aid]);
        assert!(Baked::capture(&before, &before, &[], &[]).root.is_none());
    }

    #[test]
    fn baked_state_restores_only_the_fields_the_step_changed() {
        let before = Document::new("d", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
        let mut after = before.clone();
        after.resolution_dpi = 300.0;
        let baked = Baked::capture(&before, &after, &[], &[]);
        let mut doc = before.clone();
        doc.ruler_origin = [5.0, 5.0];
        baked.apply(&mut doc, &[], &[]);
        assert_eq!(doc.resolution_dpi, 300.0);
        assert_eq!(doc.ruler_origin, [5.0, 5.0], "a field the step didn't change keeps its value");
        assert!(Baked::capture(&before, &before, &[], &[]).state.is_none());
    }

    /// A tree nested far past what recursion could walk is still enumerated.
    #[test]
    fn a_deep_layer_tree_is_walked_without_recursion() {
        let mut layer = Layer::raster("leaf", PixelFormat::RGBA8);
        for _ in 0..50_000 {
            layer = Layer::group("g", vec![layer]);
        }
        let mut n = 0;
        crate::layer_surfaces(&layer, &mut |_| n += 1);
        assert_eq!(n, 1);
        // Dropping it recurses; leak it rather than test the destructor here.
        std::mem::forget(layer);
    }
}
