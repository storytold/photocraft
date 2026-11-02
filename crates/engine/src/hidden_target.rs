//! Photoshop refuses to paint on or move a hidden layer (#571): the painting and retouching
//! tools, the Paint Bucket, the Gradient and the Move tool stop with "the target layer is hidden"
//! instead of changing pixels nobody can see. Filters are refused too (Photoshop greys the Filter
//! menu out), and so are Cut, Copy, Clear and Fill while there is a selection (#10476); without
//! one, Fill fills the whole hidden layer, as Photoshop does. A layer inside a hidden group is
//! hidden too. An alpha channel or Quick Mask target has no layer, so it is never refused.

use photocraft_doc::{Document, LayerId};
use serde_json::Value;

use crate::Session;

/// Photoshop's alert text.
pub const HIDDEN: &str = "Could not complete your request because the target layer is hidden.";

/// Commands that paint on, or move, the target layer.
const GATED: &[&str] = &[
    "paint.stroke",
    "paint.pencil",
    "paint.mixerBrush",
    "paint.colorReplacement",
    "paint.bucket",
    "paint.gradient",
    "paint.magicEraser",
    "paint.backgroundEraser",
    "paint.cloneStamp",
    "paint.healingBrush",
    "paint.spotHealing",
    "paint.patch",
    "paint.dodge",
    "paint.burn",
    "paint.sponge",
    "paint.blur",
    "paint.sharpen",
    "paint.smudge",
    "paint.historyBrush",
    "paint.artHistoryBrush",
    "layer.translate",
];

/// Commands refused on a hidden layer only while there is a selection (#10476).
const GATED_WITH_SELECTION: &[&str] = &[
    "edit.cut",
    "edit.copy",
    "edit.clear",
    "edit.fill",
    "edit.fillForeground",
    "edit.fillBackground",
    "edit.fillForegroundPreserve",
    "edit.fillBackgroundPreserve",
    "edit.fillHistory",
];

/// Does command `id` change the target layer's pixels through a filter. Convert for Smart Filters
/// only wraps the layer, so it is not one.
fn is_filter(id: &str) -> bool {
    id.starts_with("filter.") && id != "filter.convertForSmartFilters"
}

/// Is command `id` refused on a hidden layer, given whether the document has a selection.
fn gated(id: &str, has_selection: bool) -> bool {
    GATED.contains(&id) || is_filter(id) || (has_selection && GATED_WITH_SELECTION.contains(&id))
}

/// The filters that edit the layer whatever is targeted, so a channel target in their params
/// says nothing about what they touch (#10476 review): the render filters paint through
/// `render_cmds::draw`, and Liquify, the lens filters, Camera Raw and Vanishing Point take their
/// layer from `"layer"` or the active one (`lens_cmds::run_filter`, `vp_cmds`). They are the same
/// commands `channel_cmds::follows_filter_mask` excludes, for the same reason; keep the two lists
/// in step.
const IGNORES_TARGET: &[&str] = &[
    "filter.render.flame",
    "filter.render.tree",
    "filter.render.pictureFrame",
    "filter.liquify",
    "filter.lensCorrection",
    "filter.adaptiveWideAngle",
    "filter.cameraRaw",
    "filter.vanishingPoint",
];

/// Does command `id` act on the channel target in its params? Only then does such a target mean
/// no layer is touched, and only then may it lift the refusal. A command that ignores the target
/// (a render filter, Liquify, or anything outside the routed set, like Cut) edits the layer
/// whatever the params say.
fn uses_channel_target(id: &str) -> bool {
    crate::channel_cmds::routed(id) && !IGNORES_TARGET.contains(&id)
}

/// Does command `id` act on a channel rather than the layer? Cut copies the channel it is given
/// but always clears the layer's own pixels (`edit_cmds`), so no target lets it through; the rest
/// follow the target in their params, the Channels panel's having been filled in already.
fn acts_on_channel(id: &str, params: &Value) -> bool {
    id != "edit.cut" && uses_channel_target(id) && crate::channel_cmds::is_channel_target(params)
}

/// The layer command `id` acts on, resolved the way the command resolves it. Cut and Copy read a
/// `"layer"` param only when they work on colour channels; their composite path takes the active
/// layer, so a visible layer named there must not clear the refusal (#10476 review).
fn layer_of(s: &Session, id: &str, params: &Value) -> Option<LayerId> {
    if matches!(id, "edit.cut" | "edit.copy") && crate::color_channels::clip_colors(s, params).ok().flatten().is_none() {
        return s.active()?.active_layer;
    }
    crate::commands::layer_param(s, params).ok()
}

/// Is the layer shown: it and every group around it visible.
fn shown(doc: &Document, id: LayerId) -> bool {
    doc.path_of(id).is_some_and(|path| (1..=path.len()).all(|n| path.get(..n).and_then(|p| doc.layer_at(p)).is_some_and(|l| l.visible)))
}

/// Why command `id` with `params` must not run: its target layer is hidden. `None` for other
/// commands, channel targets, and visible layers.
pub fn refusal(s: &Session, id: &str, params: &Value) -> Option<&'static str> {
    let st = s.active()?;
    if !gated(id, st.doc.selection.is_some()) {
        return None;
    }
    let params = crate::channel_cmds::inject_target(s, id, params.clone());
    if acts_on_channel(id, &params) {
        return None;
    }
    let layer = layer_of(s, id, &params)?;
    (st.doc.layer(layer).is_some() && !shown(&st.doc, layer)).then_some(HIDDEN)
}

/// [`refusal`] for menu enablement: Photoshop greys a filter out on a hidden layer, while the
/// other gated commands stay enabled and are refused when they run.
pub fn menu_refusal(s: &Session, id: &str, params: &Value) -> Option<&'static str> {
    if is_filter(id) { refusal(s, id, params) } else { None }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::HIDDEN;
    use crate::Session;

    fn session() -> (Session, u64) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
        let id = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        (s, id)
    }

    fn pixel(s: &mut Session) -> serde_json::Value {
        s.execute("document.pixel", json!({"x": 10, "y": 10})).unwrap()
    }

    #[test]
    fn painting_or_moving_a_hidden_layer_is_refused() {
        let (mut s, id) = session();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        let steps = s.active().unwrap().history.entries().len();
        for (cmd, p) in [
            ("paint.stroke", json!({"points": [[10, 10], [20, 20]], "color": "#ff0000"})),
            ("paint.bucket", json!({"x": 10, "y": 10})),
            ("paint.gradient", json!({"from": [0, 0], "to": [60, 60]})),
            ("layer.translate", json!({"dx": 5, "dy": 0})),
        ] {
            let e = s.execute(cmd, p).unwrap_err().to_string();
            assert!(e.contains(HIDDEN), "{cmd}: {e}");
        }
        assert_eq!(s.active().unwrap().history.entries().len(), steps, "nothing changed");
        // Shown again, it paints.
        s.execute("layer.setProps", json!({"layer": id, "visible": true})).unwrap();
        s.execute("paint.stroke", json!({"points": [[10, 10], [12, 12]], "color": "#ff0000", "size": 9})).unwrap();
        assert_ne!(pixel(&mut s), json!([1.0, 1.0, 1.0, 1.0]));
    }

    /// #10476: filters are refused and greyed out on a hidden layer; Cut, Copy, Clear and Fill are
    /// refused while there is a selection, and Fill without one fills the whole hidden layer.
    #[test]
    fn filters_and_selection_edits_on_a_hidden_layer_are_refused() {
        let (mut s, id) = session();
        s.execute("edit.fill", json!({"color": [0.0, 1.0, 0.0]})).unwrap();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        s.execute("select.all", json!({})).unwrap();
        let steps = s.active().unwrap().history.entries().len();
        for (cmd, p) in [
            ("filter.blur.gaussianBlur", json!({"radius": 2})),
            ("filter.other.minimum", json!({"radius": 1})),
            ("edit.cut", json!({})),
            ("edit.copy", json!({})),
            ("edit.clear", json!({})),
            ("edit.fill", json!({"color": [1.0, 0.0, 0.0]})),
            ("edit.fillForeground", json!({})),
        ] {
            let e = s.execute(cmd, p).unwrap_err().to_string();
            assert!(e.contains(HIDDEN), "{cmd}: {e}");
        }
        assert_eq!(s.active().unwrap().history.entries().len(), steps, "nothing changed");
        assert!(!s.is_enabled("filter.blur.gaussianBlur"), "the filter is greyed out");
        assert!(s.is_enabled("filter.convertForSmartFilters"), "converting only wraps the layer");
        assert!(s.is_enabled("edit.fill"), "Fill stays in the menu and is refused when it runs");
        // Without a selection Fill fills the whole hidden layer, as in Photoshop; filters stay refused.
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": [1.0, 0.0, 0.0]})).unwrap();
        s.execute("layer.setProps", json!({"layer": id, "visible": true})).unwrap();
        assert_eq!(pixel(&mut s), json!([1.0, 0.0, 0.0, 1.0]));
        assert!(s.is_enabled("filter.blur.gaussianBlur"), "shown again, it filters");
    }

    #[test]
    fn a_layer_in_a_hidden_group_is_hidden() {
        let (mut s, id) = session();
        s.execute("layer.groupLayers", json!({})).unwrap();
        let g = s.active().unwrap().active_layer.unwrap().0;
        assert_ne!(g, id, "the new group is active");
        s.execute("layer.setProps", json!({"layer": g, "visible": false})).unwrap();
        s.execute("layer.select", json!({"layer": id})).unwrap();
        assert!(s.execute("paint.stroke", json!({"points": [[10, 10]]})).unwrap_err().to_string().contains(HIDDEN));
    }

    /// Review on #10495: a channel target lifts the refusal only for the commands that act on it.
    /// A render filter paints the layer whatever the target says, and Cut never reads one — a
    /// target in its params must not wave the refusal through.
    #[test]
    fn a_channel_target_a_command_ignores_does_not_lift_the_refusal() {
        let (mut s, id) = session();
        s.execute("edit.fill", json!({"color": [0.0, 1.0, 0.0]})).unwrap();
        s.execute("channel.new", json!({})).unwrap();
        s.execute("layer.select", json!({"layer": id})).unwrap();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        s.execute("select.all", json!({})).unwrap();
        let steps = s.active().unwrap().history.entries().len();
        for (cmd, p) in [
            // The render filters take the layer, never the channel.
            ("filter.render.tree", json!({"target": {"channel": 0}, "baseTreeType": 7})),
            ("filter.render.flame", json!({"target": {"channel": 0}})),
            // Cut reads no target at all: it always clears the layer's pixels.
            ("edit.cut", json!({"target": "quickMask"})),
        ] {
            let e = s.execute(cmd, p).unwrap_err().to_string();
            assert!(e.contains(HIDDEN), "{cmd}: {e}");
        }
        assert_eq!(s.active().unwrap().history.entries().len(), steps, "the hidden layer is untouched");
        // The commands that do act on the channel still go through.
        s.execute("paint.stroke", json!({"points": [[10, 10], [20, 20]], "target": {"channel": 0}})).unwrap();
        s.execute("edit.fill", json!({"color": [1.0, 0.0, 0.0], "target": {"channel": 0}})).unwrap();
    }

    /// #10476 review: the exemption follows the target and the layer each command really uses.
    /// Liquify and Lens Correction take their layer from the params and never a channel; Cut cuts
    /// the layer's pixels whatever is targeted; an explicit mask target beats the Channels panel
    /// for Copy; and Clear does read a `"layer"` param, so a hidden layer named there is refused
    /// even from a visible active layer.
    #[test]
    fn the_exemption_follows_the_target_and_layer_each_command_really_uses() {
        let (mut s, hidden) = session();
        s.execute("channel.new", json!({})).unwrap();
        s.execute("layer.select", json!({"layer": hidden})).unwrap();
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        s.execute("layer.setProps", json!({"layer": hidden, "visible": false})).unwrap();
        s.execute("select.all", json!({})).unwrap();
        let steps = s.active().unwrap().history.entries().len();
        for (cmd, p) in [
            // Liquify and Lens Correction take their layer from the params, never the channel.
            ("filter.liquify", json!({"target": {"channel": 0}, "strokes": []})),
            ("filter.lensCorrection", json!({"target": {"channel": 0}, "scale": 1.0})),
            // Cut copies the channel it is given but clears the layer's own pixels.
            ("edit.cut", json!({"target": {"channel": 0}})),
            // A mask target wins over the targeted alpha channel: Copy copies the layer's mask.
            ("edit.copy", json!({"target": "mask"})),
        ] {
            let e = s.execute(cmd, p.clone()).unwrap_err().to_string();
            assert!(e.contains(HIDDEN), "{cmd} {p}: {e}");
        }
        assert_eq!(s.active().unwrap().history.entries().len(), steps, "the hidden layer is untouched");
        // The same for the filters that take a layer and never a channel, whose params are too
        // particular to build here: Adaptive Wide Angle, Camera Raw and Vanishing Point.
        for cmd in ["filter.adaptiveWideAngle", "filter.cameraRaw", "filter.vanishingPoint"] {
            assert_eq!(super::refusal(&s, cmd, &json!({"target": {"channel": 0}})), Some(HIDDEN), "{cmd}");
        }
        // Clear and Copy do act on the targeted channel, so they go through.
        s.execute("edit.clear", json!({})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        // Clear takes a `"layer"` param, so the hidden layer named there is refused even while a
        // visible layer is active and the composite is targeted.
        let shown = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("channel.target", json!({"channel": "composite"})).unwrap();
        s.execute("layer.select", json!({"layer": shown})).unwrap();
        let e = s.execute("edit.clear", json!({"layer": hidden})).unwrap_err().to_string();
        assert!(e.contains(HIDDEN), "a hidden layer named in the params: {e}");
        s.execute("edit.clear", json!({"layer": shown})).unwrap();
    }

    /// #10476 review: in Quick Mask mode Copy is let through, and what it copies has to be the
    /// Quick Mask — a hidden layer's pixels must not reach the clipboard through it.
    #[test]
    fn copy_in_quick_mask_mode_copies_the_mask_and_not_the_hidden_layer() {
        let (mut s, id) = session();
        s.execute("edit.fill", json!({"color": [0.0, 1.0, 0.0]})).unwrap();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 32})).unwrap();
        s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        let clip = s.clipboard.as_ref().expect("something was copied");
        assert_eq!(clip.surface.format().mode, photocraft_color::ColorMode::Grayscale, "the Quick Mask copies as grayscale, not the layer's pixels");
    }

    #[test]
    fn alpha_channels_of_a_hidden_layer_still_paint() {
        let (mut s, id) = session();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        s.execute("channel.new", json!({})).unwrap();
        s.execute("paint.stroke", json!({"points": [[10, 10], [20, 20]], "target": {"channel": 0}})).unwrap();
    }

    #[test]
    fn named_destinations_cannot_bypass_hidden_layer_protection() {
        let (mut s, hidden) = session();
        s.execute("layer.setProps", json!({"layer":hidden,"name":"hidden","visible":false})).unwrap();
        s.execute("layer.new.layer", json!({"name":"visible"})).unwrap();
        for command in ["paint.stroke", "paint.pencil", "paint.bucket", "paint.gradient"] {
            let params = json!({"layer":"hidden","target":{"colors":[0,2]},"points":[[8,8]],"from":[0,0],"to":[16,16],"x":8,"y":8});
            let before = s.active().unwrap().doc.clone();
            let steps = s.active().unwrap().history.past_len();
            assert!(s.execute(command, params).unwrap_err().to_string().contains(HIDDEN));
            assert_eq!(s.active().unwrap().doc.layer(photocraft_doc::LayerId(hidden)), before.layer(photocraft_doc::LayerId(hidden)));
            assert_eq!(s.active().unwrap().history.past_len(), steps);
        }
    }
}
