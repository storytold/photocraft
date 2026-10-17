//! Window › Timeline in frame-animation mode (Photoshop's frame strip): a list of frames, each with
//! a delay and its own layer visibility, played in order. Unlike the video [`Timeline`](crate::Timeline)
//! it needs no video layers: a frame is a state of the ordinary layer stack.
//!
//! `Layer::visible` always shows the selected frame. A frame stores only the layers hidden in it, so a
//! layer added later is visible in every frame (Photoshop's "New Layers Visible in All Frames").

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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimFrame {
    /// How long the frame shows, in milliseconds (0 plays as fast as the player allows).
    pub delay_ms: u32,
    /// The layers hidden in this frame, sorted by id; every other layer is shown.
    pub hidden: Vec<LayerId>,
}

/// The document's frame animation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameAnimation {
    /// At least one frame.
    pub frames: Vec<AnimFrame>,
    /// The selected frame (0-based): the one `Layer::visible` shows.
    pub current: usize,
    #[serde(default)]
    pub loops: LoopCount,
}

impl FrameAnimation {
    /// One frame showing the layers as they are now.
    pub fn new(layers: &[Layer]) -> Self {
        FrameAnimation { frames: vec![AnimFrame { delay_ms: 0, hidden: hidden_ids(layers) }], current: 0, loops: LoopCount::Forever }
    }

    /// Keeps the selected frame and every delay valid, whatever was stored.
    pub fn clamp(&mut self) {
        if self.frames.is_empty() {
            self.frames.push(AnimFrame::default());
        }
        self.frames.truncate(MAX_FRAMES);
        for f in &mut self.frames {
            f.delay_ms = f.delay_ms.min(MAX_DELAY_MS);
        }
        self.current = self.current.min(self.frames.len() - 1);
    }

    /// Records the visibility `layers` show as the selected frame's (an edit made while a frame is
    /// selected changes that frame only).
    pub fn capture(&mut self, layers: &[Layer]) {
        self.clamp();
        let hidden = hidden_ids(layers);
        if let Some(f) = self.frames.get_mut(self.current) {
            f.hidden = hidden;
        }
    }

    /// Shows frame `index` on `layers`; false when there is no such frame.
    pub fn apply(&self, index: usize, layers: &mut [Layer]) -> bool {
        let Some(f) = self.frames.get(index) else { return false };
        set_visibility(layers, &f.hidden);
        true
    }

    /// Total running time of one play, in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        self.frames.iter().map(|f| u64::from(f.delay_ms)).sum()
    }
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

fn set_visibility(layers: &mut [Layer], hidden: &[LayerId]) {
    for l in layers {
        l.visible = hidden.binary_search(&l.id).is_err();
        if let Some(ch) = l.children_mut() {
            set_visibility(ch, hidden);
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
        a.frames.push(AnimFrame { delay_ms: 100, hidden: vec![d.layers[0].id, d.layers[2].id].into_iter().collect() });
        a.frames[1].hidden.sort();
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
        a.frames[0].hidden = vec![d.layers[0].id];
        a.frames.push(AnimFrame::default());
        d.frame_animation = Some(a);
        d.layers.push(layer("new", true));
        for f in 0..2 {
            assert!(d.select_animation_frame(f));
            assert!(d.layers[1].visible, "frame {f}");
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
        a.frames[0].hidden = vec![inner_id];
        d.frame_animation = Some(a);
        d.select_animation_frame(0);
        assert!(!d.layers[0].children().unwrap()[0].visible);
    }

    #[test]
    fn clamp_repairs_whatever_was_stored() {
        let mut a = FrameAnimation { frames: Vec::new(), current: 99, loops: LoopCount::Times(5) };
        a.clamp();
        assert_eq!((a.frames.len(), a.current), (1, 0));
        a.frames[0].delay_ms = u32::MAX;
        a.clamp();
        assert_eq!(a.frames[0].delay_ms, MAX_DELAY_MS);
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
        a.frames.push(AnimFrame { delay_ms: 250, hidden: vec![LayerId(3), LayerId(9)] });
        a.loops = LoopCount::Times(3);
        let j = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<FrameAnimation>(&j).unwrap(), a);
    }
}
