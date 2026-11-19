//! The rules the tools' cursors and the Alt stand-in share: which tools show a brush-tip centre,
//! which let ⌥ sample or load paint, the Pencil's tip shape, and the pointers of the tools whose
//! cursor is no brush tip.

use craft::behavior::{Cursor, PaintedCursor, PointerGlyph};
use craft::keymap::Mods;
use photocraft_engine::prefs::PaintingCursor as PaintingPref;

use super::PhotocraftApp;
use crate::state::Tool;

/// Whether an aliased (Pencil) dab of `size` pixels is round rather than its whole square. The dab
/// fills the pixels whose centres lie within its radius of the grid centre
/// (`paint::grid_center`), so up to 3 px the corner pixels are in and the dab is a square; from 4 px
/// on they fall outside and it is a disc (#2662).
pub(crate) fn pencil_tip_is_round(size: f32) -> bool {
    let n = if size.is_finite() { size.round().clamp(1.0, 100_000.0) } else { 1.0 };
    let h = n / 2.0;
    (h - 0.5) * std::f32::consts::SQRT_2 > h
}

/// Whether a brush-tip circle draws its centre mark.
/// Clone Stamp and Healing Brush stay an empty circle until Option is held, the source-point
/// cursor (the Healing Brush's ⌥ with Normal Brush Tip shows the sample target instead,
/// [`source_target`]). Other brushes follow Preferences › Cursors › Show Crosshair in Brush Tip, as
/// in Photoshop; the Background Eraser and the Color Replacement tool (#11934) always show their
/// sampling hotspot, and the Remove tool its centre + (#12550).
pub(crate) fn brush_tip_centre(tool: Tool, option: bool, show_crosshair: bool) -> bool {
    if tool == Tool::QuickSelection {
        return false;
    }
    if matches!(tool, Tool::CloneStamp | Tool::Healing) {
        return option || show_crosshair;
    }
    show_crosshair || matches!(tool, Tool::BackgroundEraser | Tool::ColorReplacement | Tool::Remove)
}

/// Whether the brush cursor is the reference's fixed sample target (a small circle with a cross
/// through the hotspot) at any tip size: the Healing Brush's ⌥, the source point, with Painting
/// Cursors = Normal Brush Tip (#12113). The other Painting Cursors settings were not measured and
/// keep the tip with its centre mark ([`brush_tip_centre`]); Caps Lock keeps the precise crosshair.
pub(crate) fn source_target(tool: Tool, option: bool, painting: PaintingPref, caps_lock: bool) -> bool {
    tool == Tool::Healing && option && painting == PaintingPref::NormalTip && !caps_lock
}

/// The sample target [`source_target`] and the Mixer Brush's ⌥ load show.
pub(crate) fn sample_target() -> Cursor {
    Cursor::Painted(PaintedCursor::circle(5.0).marked(true, None))
}

/// The pointer of a painting tool that acts on a click and has no brush tip: the Paint Bucket's
/// arrow with the bucket below right of it, the hotspot on the arrow's tip, so a click fills the
/// pixel under the tip (#12539), and the Magic Eraser's crosshair. Precise Other Cursors (or Caps
/// Lock, `precise`) gives the Paint Bucket the crosshair too (#3968). `None` for a tool that
/// strokes with the brush.
pub(crate) fn click_tool_cursor(tool: Tool, precise: bool) -> Option<Cursor> {
    match tool {
        Tool::PaintBucket if !precise => Some(Cursor::Painted(PaintedCursor::pointer_tool(PointerGlyph::Arrow, crate::icons::tool_icon(tool)))),
        Tool::PaintBucket | Tool::MagicEraser => Some(Cursor::Painted(PaintedCursor::Crosshair)),
        _ => None,
    }
}

/// Path Selection's own pointer (#13295): the light arrow, and the solid one with ⌥ (without ⌘)
/// where a press would land on a component (`path_components::over_component`). ⌘ keeps the light
/// one; Precise Other Cursors (or Caps Lock, `precise`) the crosshair. `mods` and `at` are the
/// press's while a drag runs, so the arrow does not flicker under an ⌥-drag: the components move
/// away from the pointer only on release.
pub(crate) fn path_selection_cursor(app: &PhotocraftApp, mods: Mods, at: Option<[f64; 2]>, precise: bool) -> Cursor {
    if precise {
        return Cursor::Painted(PaintedCursor::Crosshair);
    }
    let over = mods.alt && !mods.cmd && at.is_some_and(|at| crate::path_components::over_component(app, at));
    Cursor::Painted(PaintedCursor::arrow(!over))
}

/// Tools on which holding ⌥ (Alt) switches to the Eyedropper: a click or drag sets the
/// foreground colour, as with the Eyedropper itself (#417, the Background Eraser #11906, the Color
/// Replacement tool #11914).
pub(crate) fn alt_samples(tool: Tool, mods: craft::keymap::Mods) -> bool {
    // Control+Alt is the brush-resize drag (`brush_resize`), not sampling.
    mods.alt && !mods.ctrl && matches!(tool, Tool::Brush | Tool::Pencil | Tool::Gradient | Tool::PaintBucket | Tool::BackgroundEraser | Tool::ColorReplacement)
}

/// Whether the Eyedropper is `tool`'s ⌥ stand-in: the tools ⌥ makes sample ([`alt_samples`]), and
/// the Triangle, whose ⌥ press samples where [`shape_alt_samples`] holds (#13364).
pub(crate) fn alt_eyedropper(tool: Tool) -> bool {
    alt_samples(tool, Mods::ALT) || tool == Tool::Triangle
}

/// Whether the Triangle tool's press at (x, y) with `mods` samples the colour there as the
/// foreground and the tool's Fill instead of drawing (#13364): ⌥ (⇧⌥ too) held at the press, in
/// Shape mode, with no shape layer active (with one, ⌥ subtracts from it). Path or Pixels mode and
/// a press with nothing to sample (off the image, over transparency) were not measured and keep
/// drawing from the centre; Control+⌥ stays the brush-resize drag, as in [`alt_samples`].
pub(crate) fn shape_alt_samples(app: &mut PhotocraftApp, mods: Mods, x: f64, y: f64) -> bool {
    mods.alt
        && !mods.ctrl
        && app.ui.tool == Tool::Triangle
        && app.ui.tool_options.shape_mode == craft::draw::ui::VectorMode::Shape
        && crate::vector_ui::active_shape_path(app).is_none()
        && super::sampling::eyedropper_color(app, [x, y]).is_some()
}

/// Whether ⌥ loads the Mixer Brush with paint from the canvas instead of painting.
pub(crate) fn alt_loads_mixer(tool: Tool, mods: craft::keymap::Mods) -> bool {
    mods.alt && !mods.ctrl && tool == Tool::MixerBrush
}

/// Whether the Mixer Brush's target layer has Lock All on, so its cursor shows the not-allowed mark.
pub(crate) fn mixer_target_locked(app: &PhotocraftApp) -> bool {
    app.session.active().and_then(|st| st.active_layer.and_then(|id| st.doc.layer(id))).is_some_and(|layer| layer.locks.all)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every row is (tool, option held, preference on) → centre mark.
    #[test]
    fn brush_tip_centre_rule() {
        let rows = [
            (Tool::QuickSelection, true, true, false),
            (Tool::CloneStamp, false, false, false),
            (Tool::CloneStamp, true, false, true),
            (Tool::CloneStamp, false, true, true),
            (Tool::Healing, false, false, false),
            (Tool::Healing, true, false, true),
            (Tool::PatternStamp, true, false, false),
            (Tool::PatternStamp, false, true, true),
            (Tool::BackgroundEraser, false, false, true),
            (Tool::Remove, false, false, true),
            (Tool::Brush, false, false, false),
            (Tool::Brush, false, true, true),
            (Tool::Eraser, true, false, false),
        ];
        for (tool, option, preference, centre) in rows {
            assert_eq!(brush_tip_centre(tool, option, preference), centre, "{tool:?} option={option} preference={preference}");
        }
    }
}

#[cfg(test)]
#[path = "../color_replacement_cursor_tests.rs"]
mod color_replacement_cursor_tests;

#[cfg(test)]
#[path = "../healing_alt_cursor_tests.rs"]
mod healing_alt_cursor_tests;

#[cfg(test)]
#[path = "paint_bucket_cursor_tests.rs"]
mod paint_bucket_cursor_tests;

#[cfg(test)]
#[path = "triangle_alt_eyedropper_tests.rs"]
mod triangle_alt_eyedropper_tests;
