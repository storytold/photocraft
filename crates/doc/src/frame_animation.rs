//! Window › Timeline in frame-animation mode (Photoshop's frame strip): a list of frames, each with
//! a delay and its own layer visibility, played in order. Unlike the video [`Timeline`](crate::Timeline)
//! it needs no video layers: a frame is a state of the ordinary layer stack.
//!
//! `Layer::visible` and `Layer::opacity` always show the selected frame. A frame stores the layers
//! hidden in it, so a layer added later is visible in every frame (Photoshop's "New Layers Visible
//! in All Frames"), and each layer's opacity in it, so a fade is a change of opacity between frames.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{Document, Layer, LayerId};

/// Most frames an animation holds: every frame is a thumbnail, and an exported GIF stages each one.
pub const MAX_FRAMES: usize = 10_000;
/// Longest frame delay, in milliseconds: a GIF stores centiseconds in 16 bits.
pub const MAX_DELAY_MS: u32 = 65_535 * 10;

/// How many times playback (and an exported GIF) runs through the frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoopCount {
    Once,
    #[default]
    Forever,
    /// Plays this many times in all (two or more; one is [`LoopCount::Once`]).
    Times(u32),
}

impl LoopCount {
    /// `n` plays: 1 is [`Once`](Self::Once), 0 is [`Forever`](Self::Forever).
    pub fn from_plays(n: u32) -> Self {
        match n {
            0 => LoopCount::Forever,
            1 => LoopCount::Once,
            n => LoopCount::Times(n),
        }
    }

    /// The same count with a `Times` of fewer than two made the `Once` or `Forever` it means.
    pub fn normalized(self) -> Self {
        match self {
            LoopCount::Times(n) => LoopCount::from_plays(n),
            other => other,
        }
    }

    /// The number of plays; 0 for forever.
    pub fn plays(self) -> u32 {
        match self {
            LoopCount::Once => 1,
            LoopCount::Forever => 0,
            LoopCount::Times(n) => n.max(2),
        }
    }
}

/// One frame of the animation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AnimFrame {
    /// How long the frame shows, in milliseconds (0 plays as fast as the player allows).
    pub delay_ms: u32,
    /// The layers hidden in this frame, sorted by id; every other layer is shown.
    pub hidden: Vec<LayerId>,
    /// The layers this frame shows at an opacity other than the animation's default for them
    /// ([`FrameAnimation::default_opacity`]), sorted by id. Any other layer has its default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub opacities: Vec<(LayerId, f32)>,
}

impl AnimFrame {
    /// The opacity this frame sets for layer `id` apart from the default, if it does.
    pub fn opacity_of(&self, id: LayerId) -> Option<f32> {
        self.opacities.binary_search_by_key(&id, |(l, _)| *l).ok().and_then(|i| self.opacities.get(i)).map(|(_, o)| *o)
    }

    /// Gives layer `id` opacity `v` (0 to 1) in this frame, where `default` is the animation's for it:
    /// the default itself is not stored.
    pub fn set_opacity(&mut self, id: LayerId, v: f32, default: Option<f32>) {
        let found = self.opacities.binary_search_by_key(&id, |(l, _)| *l);
        match (found, default == Some(v)) {
            (Ok(i), true) => {
                self.opacities.remove(i);
            }
            (Ok(i), false) => self.opacities[i].1 = v,
            (Err(i), false) => self.opacities.insert(i, (id, v)),
            (Err(_), true) => {}
        }
    }
}

/// The document's frame animation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameAnimation {
    /// At least one frame. Frames are shared by copies of the animation (a document clone, a history
    /// snapshot) until one is edited, and an edit copies that frame alone: see [`Self::frame_mut`].
    pub frames: Vec<Arc<AnimFrame>>,
    /// The selected frame (0-based): the one `Layer::visible` shows.
    pub current: usize,
    #[serde(default)]
    pub loops: LoopCount,
    /// The opacity each layer has in a frame that sets none of its own, sorted by id: what the layer
    /// had when the animation first knew of it. Kept here once, so that a frame stores only how it
    /// differs and a thousand frames of a thousand layers don't hold a million opacities.
    #[serde(default, skip_serializing_if = "defaults_are_empty")]
    pub defaults: Arc<Vec<(LayerId, f32)>>,
}

fn defaults_are_empty(d: &Arc<Vec<(LayerId, f32)>>) -> bool {
    d.is_empty()
}

impl FrameAnimation {
    /// One frame showing the layers as they are now.
    pub fn new(layers: &[Layer]) -> Self {
        FrameAnimation {
            frames: vec![Arc::new(AnimFrame { delay_ms: 0, hidden: hidden_ids(layers), opacities: Vec::new() })],
            current: 0,
            loops: LoopCount::Forever,
            defaults: Arc::new(opacities(layers)),
        }
    }

    /// Frame `index` for editing. A frame shared with a copy of the animation is copied first, so the
    /// copy keeps what it had.
    pub fn frame_mut(&mut self, index: usize) -> Option<&mut AnimFrame> {
        self.frames.get_mut(index).map(Arc::make_mut)
    }

    /// The opacity layer `id` has in a frame that sets none for it, if the animation knows the layer.
    pub fn default_opacity(&self, id: LayerId) -> Option<f32> {
        self.defaults.binary_search_by_key(&id, |(l, _)| *l).ok().and_then(|i| self.defaults.get(i)).map(|(_, o)| *o)
    }

    /// The opacity layer `id` has in `frame`: its own, else the default.
    pub fn opacity_in(&self, frame: &AnimFrame, id: LayerId) -> Option<f32> {
        frame.opacity_of(id).or_else(|| self.default_opacity(id))
    }

    /// Keeps the selected frame and every delay valid, whatever was stored.
    pub fn clamp(&mut self) {
        if self.frames.is_empty() {
            self.frames.push(Arc::default());
        }
        self.frames.truncate(MAX_FRAMES);
        let in_range = |o: f32| if o.is_finite() { o.clamp(0.0, 1.0) } else { 1.0 };
        if !sorted_by_id(&self.defaults) || self.defaults.iter().any(|(_, o)| in_range(*o) != *o) {
            let d = Arc::make_mut(&mut self.defaults);
            d.sort_by_key(|(l, _)| *l);
            d.dedup_by_key(|(l, _)| *l);
            for (_, o) in d.iter_mut() {
                *o = in_range(*o);
            }
        }
        let defaults = Arc::clone(&self.defaults);
        let default_of = |id: LayerId| defaults.binary_search_by_key(&id, |(l, _)| *l).ok().and_then(|i| defaults.get(i)).map(|(_, o)| *o);
        for f in &mut self.frames {
            // Looked up by binary search, so they must be sorted: a stored list may not be. A frame that
            // is already valid is not touched, which would copy it if it were shared.
            let valid = f.delay_ms <= MAX_DELAY_MS
                && f.hidden.windows(2).all(|w| w[0] < w[1])
                && sorted_by_id(&f.opacities)
                && f.opacities.iter().all(|(l, o)| in_range(*o) == *o && default_of(*l) != Some(*o));
            if !valid {
                let f = Arc::make_mut(f);
                f.delay_ms = f.delay_ms.min(MAX_DELAY_MS);
                f.hidden.sort();
                f.hidden.dedup();
                f.opacities.sort_by_key(|(l, _)| *l);
                f.opacities.dedup_by_key(|(l, _)| *l);
                for (_, o) in &mut f.opacities {
                    *o = in_range(*o);
                }
                // Only how a frame differs from the defaults is kept.
                f.opacities.retain(|(l, o)| default_of(*l) != Some(*o));
            }
        }
        self.current = self.current.min(self.frames.len() - 1);
        // `Times` holds two or more plays; one is `Once` and none is `Forever`.
        self.loops = self.loops.normalized();
    }

    /// Rewrites the layer ids the frames name through `map`, dropping those it has no layer for.
    pub fn remap_layers(&mut self, map: impl Fn(LayerId) -> Option<LayerId>) {
        for f in &mut self.frames {
            let f = Arc::make_mut(f);
            f.hidden = f.hidden.iter().filter_map(|h| map(*h)).collect();
            f.opacities = f.opacities.iter().filter_map(|(l, o)| map(*l).map(|l| (l, *o))).collect();
        }
        self.defaults = Arc::new(self.defaults.iter().filter_map(|(l, o)| map(*l).map(|l| (l, *o))).collect());
        self.clamp();
    }

    /// Records the visibility and opacity `layers` show as the selected frame's (an edit made while a
    /// frame is selected changes that frame only). A layer the animation doesn't know yet, such as one
    /// the edit just added, takes the opacity it has now as its default, so every other frame has it too.
    pub fn capture(&mut self, layers: &[Layer]) {
        // This runs after every edit: only the selected frame is rewritten, and only if it changed, so
        // the other frames are neither visited nor copied.
        if self.frames.is_empty() {
            self.frames.push(Arc::default());
        }
        self.current = self.current.min(self.frames.len() - 1);
        let now = opacities(layers);
        let hidden = hidden_ids(layers);
        self.learn_defaults(&now);
        let overrides: Vec<(LayerId, f32)> = now.into_iter().filter(|(id, o)| self.default_opacity(*id) != Some(*o)).collect();
        if let Some(f) = self.frames.get_mut(self.current).filter(|f| f.hidden != hidden || f.opacities != overrides) {
            // A new frame, not a copy of the shared one: its lists are replaced anyway.
            *f = Arc::new(AnimFrame { delay_ms: f.delay_ms, hidden, opacities: overrides });
        }
    }

    /// Makes the opacity every layer in `layers` has now its default, where the animation has none.
    pub fn fill_opacities(&mut self, layers: &[Layer]) {
        self.learn_defaults(&opacities(layers));
    }

    fn learn_defaults(&mut self, now: &[(LayerId, f32)]) {
        let new: Vec<(LayerId, f32)> = now.iter().filter(|(id, _)| self.default_opacity(*id).is_none()).copied().collect();
        if !new.is_empty() {
            let d = Arc::make_mut(&mut self.defaults);
            d.extend(new);
            d.sort_by_key(|(l, _)| *l);
        }
    }

    /// Shows frame `index` on `layers`; false when there is no such frame.
    pub fn apply(&self, index: usize, layers: &mut [Layer]) -> bool {
        let Some(f) = self.frames.get(index) else { return false };
        set_state(layers, self, f);
        true
    }

    /// Total running time of one play, in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        self.frames.iter().map(|f| u64::from(f.delay_ms)).sum()
    }
}

fn sorted_by_id(v: &[(LayerId, f32)]) -> bool {
    v.windows(2).all(|w| w[0].0 < w[1].0)
}

/// The ids of every layer in the tree.
pub fn layer_ids(layers: &[Layer]) -> std::collections::HashSet<LayerId> {
    fn rec(layers: &[Layer], out: &mut std::collections::HashSet<LayerId>) {
        for l in layers {
            out.insert(l.id);
            if let Some(ch) = l.children() {
                rec(ch, out);
            }
        }
    }
    let mut out = std::collections::HashSet::new();
    rec(layers, &mut out);
    out
}

/// Every layer in the tree with its opacity, by id.
fn opacities(layers: &[Layer]) -> Vec<(LayerId, f32)> {
    fn rec(layers: &[Layer], out: &mut Vec<(LayerId, f32)>) {
        for l in layers {
            out.push((l.id, l.opacity));
            if let Some(ch) = l.children() {
                rec(ch, out);
            }
        }
    }
    let mut out = Vec::new();
    rec(layers, &mut out);
    out.sort_by_key(|(l, _)| *l);
    out
}

/// Every layer in the tree with its eye off, by id.
fn hidden_ids(layers: &[Layer]) -> Vec<LayerId> {
    fn rec(layers: &[Layer], out: &mut Vec<LayerId>) {
        for l in layers {
            if !l.visible {
                out.push(l.id);
            }
            if let Some(ch) = l.children() {
                rec(ch, out);
            }
        }
    }
    let mut out = Vec::new();
    rec(layers, &mut out);
    out.sort();
    out
}

fn set_state(layers: &mut [Layer], animation: &FrameAnimation, frame: &AnimFrame) {
    for l in layers {
        l.visible = frame.hidden.binary_search(&l.id).is_err();
        if let Some(o) = animation.opacity_in(frame, l.id) {
            l.opacity = o;
        }
        if let Some(ch) = l.children_mut() {
            set_state(ch, animation, frame);
        }
    }
}

impl Document {
    /// Writes the layers' visibility into the selected animation frame, if there is an animation.
    pub fn capture_animation_frame(&mut self) {
        if let Some(a) = &mut self.frame_animation {
            a.capture(&self.layers);
        }
    }

    /// Selects frame `index` of the animation and shows it on the layers. False (and nothing
    /// changed) with no animation or no such frame.
    pub fn select_animation_frame(&mut self, index: usize) -> bool {
        let Some(a) = &mut self.frame_animation else { return false };
        if !a.apply(index, &mut self.layers) {
            return false;
        }
        a.current = index;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Group, LayerContent};
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_geom::Size;

    fn layer(name: &str, visible: bool) -> Layer {
        let mut l = Layer::new(name, LayerContent::Group(Group { children: Vec::new(), expanded: true, artboard: None }));
        l.visible = visible;
        l
    }

    fn doc(n: usize) -> Document {
        let mut d = Document::new("t", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
        for i in 0..n {
            d.layers.push(layer(&format!("L{i}"), true));
        }
        d
    }

    fn shown(d: &Document) -> Vec<bool> {
        d.layers.iter().map(|l| l.visible).collect()
    }

    #[test]
    fn a_frame_stores_what_is_hidden_and_selecting_applies_it() {
        let mut d = doc(3);
        d.layers[1].visible = false;
        let mut a = FrameAnimation::new(&d.layers);
        a.frames.push(Arc::new(AnimFrame { delay_ms: 100, hidden: vec![d.layers[0].id, d.layers[2].id].into_iter().collect(), ..Default::default() }));
        a.frame_mut(1).unwrap().hidden.sort();
        d.frame_animation = Some(a);
        assert!(d.select_animation_frame(1));
        assert_eq!(shown(&d), [false, true, false]);
        assert!(d.select_animation_frame(0));
        assert_eq!(shown(&d), [true, false, true]);
        assert!(!d.select_animation_frame(2));
        assert_eq!(d.frame_animation.as_ref().map(|a| a.current), Some(0));
    }

    #[test]
    fn capture_changes_only_the_selected_frame() {
        let mut d = doc(2);
        let mut a = FrameAnimation::new(&d.layers);
        a.frames.push(a.frames[0].clone());
        d.frame_animation = Some(a);
        d.layers[0].visible = false;
        d.capture_animation_frame();
        let a = d.frame_animation.as_ref().unwrap();
        assert_eq!(a.frames[0].hidden, [d.layers[0].id]);
        assert!(a.frames[1].hidden.is_empty());
    }

    #[test]
    fn a_layer_added_later_is_visible_in_every_frame() {
        let mut d = doc(1);
        let mut a = FrameAnimation::new(&d.layers);
        a.frame_mut(0).unwrap().hidden = vec![d.layers[0].id];
        a.frames.push(Arc::default());
        d.frame_animation = Some(a);
        d.layers.push(layer("new", true));
        for f in 0..2 {
            assert!(d.select_animation_frame(f));
            assert!(d.layers[1].visible, "frame {f}");
        }
    }

    #[test]
    fn opacity_belongs_to_the_selected_frame() {
        let mut d = doc(2);
        let mut a = FrameAnimation::new(&d.layers);
        a.frames.push(a.frames[0].clone());
        d.frame_animation = Some(a);
        d.layers[0].opacity = 0.25;
        d.capture_animation_frame();
        let a = d.frame_animation.as_ref().unwrap();
        assert_eq!(a.frames[0].opacity_of(d.layers[0].id), Some(0.25), "the selected frame sets its own");
        assert_eq!(a.frames[1].opacity_of(d.layers[0].id), None, "the other frame stores nothing");
        assert_eq!(a.opacity_in(&a.frames[1], d.layers[0].id), Some(1.0), "and keeps the default");
        assert!(d.select_animation_frame(1));
        assert_eq!(d.layers[0].opacity, 1.0);
        assert!(d.select_animation_frame(0));
        assert_eq!(d.layers[0].opacity, 0.25);
    }

    #[test]
    fn a_layer_added_later_has_its_opacity_in_every_frame() {
        let mut d = doc(1);
        let mut a = FrameAnimation::new(&d.layers);
        a.frames.push(a.frames[0].clone());
        d.frame_animation = Some(a);
        let mut l = layer("new", true);
        l.opacity = 0.5;
        let id = l.id;
        d.layers.push(l);
        d.capture_animation_frame();
        let a = d.frame_animation.as_ref().unwrap();
        for f in &a.frames {
            assert_eq!(a.opacity_in(f, id), Some(0.5));
            assert_eq!(f.opacity_of(id), None, "as the default, not stored per frame");
        }
    }

    #[test]
    fn nested_layers_follow_their_frame() {
        let mut d = doc(1);
        let inner = layer("inner", true);
        let inner_id = inner.id;
        if let Some(ch) = d.layers[0].children_mut() {
            ch.push(inner);
        }
        let mut a = FrameAnimation::new(&d.layers);
        a.frame_mut(0).unwrap().hidden = vec![inner_id];
        d.frame_animation = Some(a);
        d.select_animation_frame(0);
        assert!(!d.layers[0].children().unwrap()[0].visible);
    }

    #[test]
    fn clamp_repairs_whatever_was_stored() {
        let mut a = FrameAnimation { frames: Vec::new(), current: 99, loops: LoopCount::Times(5), defaults: Default::default() };
        a.clamp();
        assert_eq!((a.frames.len(), a.current), (1, 0));
        a.frame_mut(0).unwrap().delay_ms = u32::MAX;
        a.frame_mut(0).unwrap().hidden = vec![LayerId(9), LayerId(3), LayerId(9)];
        a.frame_mut(0).unwrap().opacities = vec![(LayerId(2), 7.0), (LayerId(1), f32::NAN), (LayerId(2), 0.5)];
        a.clamp();
        assert_eq!(a.frames[0].delay_ms, MAX_DELAY_MS);
        assert_eq!(a.frames[0].hidden, [LayerId(3), LayerId(9)], "sorted and without repeats");
        for (stored, repaired) in [(0, LoopCount::Forever), (1, LoopCount::Once), (2, LoopCount::Times(2))] {
            a.loops = LoopCount::Times(stored);
            a.clamp();
            assert_eq!(a.loops, repaired);
        }
        assert_eq!(a.frames[0].opacities, [(LayerId(1), 1.0), (LayerId(2), 1.0)], "sorted, unique and in range");
    }

    #[test]
    fn loop_counts_round_trip_through_plays() {
        for n in [0, 1, 2, 7] {
            assert_eq!(LoopCount::from_plays(n).plays(), n);
        }
    }

    #[test]
    fn serde_round_trips() {
        let mut a = FrameAnimation::new(&[]);
        a.frames.push(Arc::new(AnimFrame { delay_ms: 250, hidden: vec![LayerId(3), LayerId(9)], opacities: vec![(LayerId(3), 0.25)] }));
        a.defaults = Arc::new(vec![(LayerId(3), 1.0)]);
        a.loops = LoopCount::Times(3);
        let j = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<FrameAnimation>(&j).unwrap(), a);
    }

    #[test]
    fn frames_store_only_how_they_differ_from_the_defaults() {
        let mut d = doc(3);
        d.layers[1].opacity = 0.5;
        let mut a = FrameAnimation::new(&d.layers);
        assert!(a.frames[0].opacities.is_empty() && a.default_opacity(d.layers[1].id) == Some(0.5));
        a.frames.push(a.frames[0].clone());
        d.frame_animation = Some(a);
        d.layers[0].opacity = 0.2;
        d.capture_animation_frame();
        let a = d.frame_animation.as_ref().unwrap();
        assert_eq!(a.frames[0].opacities, [(d.layers[0].id, 0.2)], "only the layer that differs");
        // Back to the default, the override goes.
        d.layers[0].opacity = 1.0;
        d.capture_animation_frame();
        assert!(d.frame_animation.as_ref().unwrap().frames[0].opacities.is_empty());
    }

    #[test]
    fn copies_share_their_frames_until_one_is_edited() {
        let mut d = doc(3);
        let mut a = FrameAnimation::new(&d.layers);
        a.frames = (0..4).map(|i| Arc::new(AnimFrame { delay_ms: i * 10, hidden: vec![d.layers[(i % 3) as usize].id], ..Default::default() })).collect();
        a.clamp();
        let copy = a.clone();
        assert!(a.frames.iter().zip(&copy.frames).all(|(x, y)| Arc::ptr_eq(x, y)), "a clone copies no frame");
        assert!(Arc::ptr_eq(&a.defaults, &copy.defaults));
        // Editing a frame, capturing the selected one, repairing and an unchanged capture leave the rest shared.
        a.frame_mut(2).unwrap().delay_ms = 5;
        d.layers[0].visible = false;
        a.current = 1;
        a.capture(&d.layers);
        a.clamp();
        let shared: Vec<bool> = a.frames.iter().zip(&copy.frames).map(|(x, y)| Arc::ptr_eq(x, y)).collect();
        assert_eq!(shared, [true, false, false, true], "only the edited frames were copied");
        assert_eq!(copy.frames[2].delay_ms, 20, "the copy kept its own value");
        let again = a.clone();
        a.capture(&d.layers);
        assert!(Arc::ptr_eq(&a.frames[1], &again.frames[1]), "capturing what the frame already holds copies nothing");
    }

    #[test]
    fn a_dense_list_of_opacities_is_compacted_on_clamp() {
        let d = doc(3);
        let mut a = FrameAnimation::new(&d.layers);
        let all: Vec<(LayerId, f32)> = d.layers.iter().map(|l| (l.id, 1.0)).collect();
        a.frame_mut(0).unwrap().opacities = all;
        a.clamp();
        assert!(a.frames[0].opacities.is_empty(), "values equal to the defaults are not kept");
    }
}
