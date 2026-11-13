//! The develop view's icon rail at the far right of the dialog (LightCraft's right tool strip):
//! one button per tool that works on a raw the shared engine develops. The tool chooses the panel
//! beside the rail and what a drag on the preview does.

use egui::{Rect, Sense, Ui, pos2, vec2};

use super::lc::Lc;
use super::lc::icons::Icon;
use super::lc::widgets::icon_button;

/// The develop view's tools.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Tool {
    #[default]
    Edit,
    Heal,
    Masking,
    RedEye,
}

impl Tool {
    /// The rail's tools, top to bottom (LightCraft's order).
    pub(crate) const RAIL: [Tool; 4] = [Tool::Edit, Tool::Heal, Tool::Masking, Tool::RedEye];

    pub(crate) fn id(self) -> &'static str {
        match self {
            Tool::Edit => "edit",
            Tool::Heal => "heal",
            Tool::Masking => "masking",
            Tool::RedEye => "redEye",
        }
    }

    pub(crate) fn from_id(id: &str) -> Option<Tool> {
        Tool::RAIL.into_iter().find(|t| t.id() == id)
    }

    fn icon(self) -> Icon {
        match self {
            Tool::Edit => Icon::Sliders,
            Tool::Heal => Icon::Eraser,
            Tool::Masking => Icon::Mask,
            Tool::RedEye => Icon::RedEye,
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Tool::Edit => "Edit (E)",
            Tool::Heal => "Heal (Q)",
            Tool::Masking => "Masking (M)",
            Tool::RedEye => "Red Eye",
        }
    }

    /// The key that picks the tool, as in LightCraft.
    fn key(self) -> Option<egui::Key> {
        match self {
            Tool::Edit => Some(egui::Key::E),
            Tool::Heal => Some(egui::Key::Q),
            Tool::Masking => Some(egui::Key::M),
            Tool::RedEye => None,
        }
    }
}

/// The rail in `rect`; a click (or the tool's key, outside a text field) picks a tool. Returns the
/// tool picked this frame.
pub(crate) fn rail(ui: &mut Ui, rect: Rect, tool: &mut Tool) -> Option<Tool> {
    let t = Lc::get(ui.ctx());
    ui.painter().rect_filled(rect, 0.0, t.chrome);
    ui.painter().line_segment([rect.left_top(), rect.left_bottom()], egui::Stroke::new(1.0, t.divider));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(4.0, 10.0))).layout(egui::Layout::top_down(egui::Align::Center)));
    child.spacing_mut().item_spacing.y = 8.0;
    let mut picked = None;
    for candidate in Tool::RAIL {
        let tip = super::develop_panel::tr(candidate.tip());
        if icon_button(&mut child, candidate.icon(), vec2(40.0, 40.0), *tool == candidate, true, tip).clicked() {
            picked = Some(candidate);
        }
    }
    let _ = child.allocate_rect(Rect::from_min_size(pos2(rect.left(), rect.bottom()), vec2(0.0, 0.0)), Sense::hover());
    if picked.is_none() && !ui.ctx().text_edit_focused() {
        picked = Tool::RAIL.into_iter().find(|c| c.key().is_some_and(|k| ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::NONE, k))));
    }
    if let Some(p) = picked {
        // the current tool's button (or key) again goes back to Edit, as LightCraft's rail does
        *tool = if *tool == p && p != Tool::Edit { Tool::Edit } else { p };
    }
    picked
}
