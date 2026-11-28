use super::planes::color_count;
use super::{ColorMode, DocState, Document, Serialize, Session};

// ---------------------------------------------------------------------------------------------
// View state

/// What pixel commands edit (Channels panel click / ⌘2…⌘9).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, craft_ui_macros::PlainData)]
#[serde(rename_all = "camelCase", tag = "kind", content = "index")]
pub enum ChannelTarget {
    /// The layers (all colour channels). In Quick Mask mode this edits the Quick Mask.
    #[default]
    Composite,
    /// One colour channel of the active layer (index into the pixel format's colour channels).
    Color(usize),
    /// An alpha or spot channel (index into [`Document::channels`]).
    Alpha(usize),
    /// The active layer's layer mask.
    LayerMask,
    /// The active layer's vector mask.
    VectorMask,
}

/// Per-document Channels panel state: the target and the eye toggles. Not part of history.
#[derive(Clone, Debug, Default, PartialEq, Serialize, craft_ui_macros::PlainData)]
#[serde(rename_all = "camelCase")]
pub struct ChannelView {
    pub target: ChannelTarget,
    /// Selected native colour components; empty retains the legacy single target.
    pub selected_colors: Vec<usize>,
    /// Hidden colour channels (missing entries are visible).
    pub color_hidden: Vec<bool>,
    /// Visible alpha channels (missing entries are hidden).
    pub alpha_visible: Vec<bool>,
    pub quick_mask_hidden: bool,
    /// ⌥-click view of the active layer's mask (#196); see [`crate::mask_view_cmds`].
    pub layer_mask: Option<crate::mask_view_cmds::LayerMaskView>,
}

impl ChannelView {
    pub fn color_selection(&self, count: usize) -> Option<crate::color_channels::ColorChannels> {
        crate::color_channels::from_view(self, count)
    }
    pub fn color_visible(&self, k: usize) -> bool {
        !self.color_hidden.get(k).copied().unwrap_or(false)
    }
    pub fn alpha_shown(&self, i: usize) -> bool {
        self.alpha_visible.get(i).copied().unwrap_or(false)
    }
    pub fn set_color_visible(&mut self, k: usize, v: bool) {
        if self.color_hidden.len() <= k {
            self.color_hidden.resize(k + 1, false);
        }
        self.color_hidden[k] = !v;
    }
    pub fn set_alpha_visible(&mut self, i: usize, v: bool) {
        if self.alpha_visible.len() <= i {
            self.alpha_visible.resize(i + 1, false);
        }
        self.alpha_visible[i] = v;
    }
    /// Number of visible colour channels out of `colors`.
    pub fn visible_colors(&self, colors: usize) -> usize {
        (0..colors).filter(|k| self.color_visible(*k)).count()
    }
    /// True when the canvas shows just the normal composite (nothing extra to draw).
    pub fn is_plain(&self, doc: &Document) -> bool {
        let colors = color_count(doc);
        self.visible_colors(colors) == colors
            && !(0..doc.channels.len()).any(|i| self.alpha_shown(i))
            && (doc.quick_mask.is_none() || self.quick_mask_hidden)
            && self.shown_layer_mask(doc).is_none()
    }
    /// The layer mask shown on the canvas (#196) and how, if its layer still has a mask.
    pub fn shown_layer_mask<'a>(&self, doc: &'a Document) -> Option<(&'a photocraft_doc::LayerMask, crate::mask_view_cmds::MaskViewMode)> {
        let v = self.layer_mask?;
        Some((crate::mask_view_cmds::viewed_mask(doc.layer(v.layer)?, v.filter)?, v.mode))
    }
}

pub use crate::state::QuickMaskOptions;

/// Keep the view valid after the document changed (undo, deleted channels, mode changes).
pub(crate) fn fix_view(st: &mut DocState) {
    let n = st.doc.channels.len();
    let colors = color_count(&st.doc);
    let quick = st.doc.quick_mask.is_some();
    let active = st.active_layer.and_then(|id| st.doc.layer(id));
    let missing_layer_mask = active.is_none_or(|l| l.mask.is_none());
    let missing_vector_mask = active.is_none_or(|l| l.vector_mask.is_none());
    {
        let v = &mut st.channel_view;
        v.alpha_visible.truncate(n);
        v.color_hidden.truncate(colors);
        v.selected_colors.retain(|k| *k < colors);
        match v.target {
            ChannelTarget::Alpha(i) if i >= n => v.target = ChannelTarget::Composite,
            ChannelTarget::Color(k) if k >= colors || colors < 2 => v.target = ChannelTarget::Composite,
            ChannelTarget::LayerMask if missing_layer_mask => {
                v.target = ChannelTarget::Composite;
            }
            ChannelTarget::VectorMask if missing_vector_mask => {
                v.target = ChannelTarget::Composite;
            }
            _ => {}
        }
        if v.color_selection(colors).is_some_and(|selected| selected.len() == colors) {
            v.target = ChannelTarget::Composite;
        }
        if !matches!(v.target, ChannelTarget::Color(_)) {
            v.selected_colors.clear();
        }
    }
    crate::mask_view_cmds::fix(st);
    // An alpha/spot eye, Quick Mask, or the active layer's mask can be the only thing on the
    // canvas (so R, G and B can all be off). Otherwise the colour channels come back.
    let active = st.active_layer;
    let shown = {
        let doc = &st.doc;
        let v = &st.channel_view;
        (0..n).any(|i| v.alpha_shown(i)) || (quick && !v.quick_mask_hidden) || active.is_some_and(|id| doc.layer(id).is_some_and(|l| l.mask.is_some()))
    };
    if st.channel_view.visible_colors(colors) == 0 && !shown {
        st.channel_view.color_hidden.clear();
    }
}

/// Bump the revision for a view-only change without dirtying a clean document. The channels
/// shown change the picture, so the whole canvas is drawn again.
pub(super) fn touch(st: &mut DocState) {
    st.bump_view_revision(!st.is_dirty());
    st.last_damage = None;
    st.record_damage();
}

// ---------------------------------------------------------------------------------------------
// Channel names and formats

/// Colour channel names of a document mode (the rows under the composite).
pub fn color_names(mode: ColorMode) -> &'static [&'static str] {
    match mode {
        ColorMode::Cmyk => &["Cyan", "Magenta", "Yellow", "Black"],
        ColorMode::Lab => &["Lightness", "a", "b"],
        ColorMode::Grayscale => &["Gray"],
        _ => &["Red", "Green", "Blue"],
    }
}

/// Name of the composite row.
pub fn composite_name(mode: ColorMode) -> &'static str {
    match mode {
        ColorMode::Cmyk => "CMYK",
        ColorMode::Lab => "Lab",
        ColorMode::Grayscale => "Gray",
        _ => "RGB",
    }
}

// ---------------------------------------------------------------------------------------------
// Undo of a step that targets the channel it creates

/// Before New Channel or Calculations (Result: New Channel), which target the channel they make:
/// the history state the step starts from and the channel view then.
pub(super) fn view_before_step(s: &Session) -> Option<(u64, ChannelView)> {
    let st = s.active()?;
    Some((st.history.row_id(st.history.past_len())?, st.channel_view.clone()))
}

/// After that step: undoing it (leaving the state it made for the one it started from) brings
/// `before`'s view back (#14683). Kept for the step, so undo after redo brings it back again. The
/// views of steps history no longer holds are dropped.
pub(super) fn keep_view_for_undo(s: &mut Session, before: Option<(u64, ChannelView)>) {
    let (Some(st), Some((from, view))) = (s.active_mut(), before) else { return };
    let Some(to) = st.history.row_id(st.history.past_len()) else { return };
    let history = &st.history;
    st.undo_channel_views.retain(|(kept_from, kept_to, _)| *kept_to != to && history.row_of(*kept_from).is_some() && history.row_of(*kept_to).is_some());
    st.undo_channel_views.push((from, to, view));
}

/// Undo just left the state `left` for the current one: when that undid a step with a kept view,
/// target and show the channels it did.
pub(crate) fn restore_view_after_undo(st: &mut DocState, left: Option<u64>) {
    let Some(left) = left else { return };
    let Some(id) = st.history.row_id(st.history.past_len()) else { return };
    let Some((_, _, view)) = st.undo_channel_views.iter().find(|(from, to, _)| *from == id && *to == left) else { return };
    st.channel_view = view.clone();
    fix_view(st);
}
