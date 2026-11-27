//! Held keys the app itself reads (#249, #2474, #3439).
//!
//! The momentary Hand and Zoom tools and the ⌘ Move stand-in belong to the canvas host
//! (`canvas::tools`); this module keeps the rules that stay with the app:
//!
//! - ⌘-arrows nudge as the Move tool's arrows do ([`cmd_nudges`]).
//! - `~` (the backtick key) with the Brush or Pencil: a stroke started while it is held erases,
//!   with that tool's own brush ([`erase_key`], [`erases_while_held`], #3439).
//! - Space while a marquee, lasso or shape is being dragged repositions it; releasing Space goes
//!   back to sizing it ([`repositions`], [`reposition_held`]).

use craft::ui::canvas::MomentaryCommands;
use craft::ui::runtime::commands::FirePhase;

use crate::PhotocraftApp;
use crate::state::Tool;

/// Tools on which ⌘-arrows nudge as the Move tool's arrows do: the tools whose ⌘ press the Move
/// stand-in takes. Photoshop excepts the Hand, Rotate View, Zoom, Slice, Path and Direct
/// Selection, shape, Pen and Type tools (⌘ means something else to each), the Crop tools and the
/// marquees, lassos and Magic Wand (those are added back by [`cmd_nudges`] as selection tools).
fn nudges_with_cmd(t: Tool) -> bool {
    !t.is_type()
        && !matches!(
            t,
            Tool::Move
                | Tool::Hand
                | Tool::RotateView
                | Tool::Zoom
                | Tool::Crop
                | Tool::PerspectiveCrop
                | Tool::Slice
                | Tool::SliceSelect
                | Tool::PathSelection
                | Tool::DirectSelection
                | Tool::Pen
                | Tool::FreeformPen
                | Tool::CurvaturePen
                | Tool::AddAnchorPoint
                | Tool::DeleteAnchorPoint
                | Tool::ConvertPoint
                | Tool::RectMarquee
                | Tool::EllipseMarquee
                | Tool::Lasso
                | Tool::PolygonLasso
                | Tool::MagicWand
        )
        && !crate::vector_ui::is_shape_tool(t)
}

/// Do ⌘-arrows (Ctrl off the Mac) nudge as the Move tool's arrows do (#2474)? With the tools
/// whose ⌘ is the Move tool, with the Move tool itself, and with the selection tools: in
/// Photoshop their ⌘ is the Move tool too, so ⌘-arrows move the selected pixels (or the layer)
/// while the plain arrows move the outline. Not while a Lasso, Polygonal Lasso or Magnetic Lasso
/// border is open, during Free Transform or while editing text.
pub fn cmd_nudges(app: &PhotocraftApp) -> bool {
    let t = app.ui.tool;
    let selecting = crate::tool_feedback::is_selection_tool(t);
    app.ui.transform.is_none()
        && app.ui.text_edit.is_none()
        && (nudges_with_cmd(t) || t == Tool::Move || selecting)
        && !(selecting && app.canvas_tools.selection.outline_open())
}

/// Tools tied to the erase key: their strokes erase with their own brush while it is held.
pub fn erases_while_held(t: Tool) -> bool {
    matches!(t, Tool::Brush | Tool::Pencil)
}

/// Records the erase key (`tools.temporary.erase`, `~` by default) going down or up. The drain
/// calls it for every fired command, so the runtime's typing rule already kept text fields out.
/// It reads no frame input: a second pass in the same frame drains no fired command and leaves the
/// flag as the first pass set it. A tool switch while the key is held is cleared by the next
/// release; [`erases_while_held`] gates the read too.
pub fn erase_key(app: &mut PhotocraftApp, phase: FirePhase) {
    app.held_erase = phase != FirePhase::Release && erases_while_held(app.ui.tool);
}

/// Is the reposition key (the momentary Hand's key, any modifiers) down? Not while editing type,
/// where Space inserts a space.
pub fn reposition_held(app: &PhotocraftApp) -> bool {
    app.ui.text_edit.is_none() && app.canvas_tools.momentary.held() == Some(MomentaryCommands::Hand)
}

/// Tools whose in-progress drag the reposition key moves.
pub fn repositions(t: Tool) -> bool {
    matches!(t, Tool::RectMarquee | Tool::EllipseMarquee | Tool::Lasso | Tool::ObjectSelection) || crate::vector_ui::is_shape_tool(t)
}

#[cfg(test)]
mod tests;
