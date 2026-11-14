//! The develop view's icon rail at the far right of the dialog (LightCraft's right tool strip):
//! one button per tool that works on a raw the shared engine develops. The tool chooses the panel
//! beside the rail and what a drag on the preview does.

use egui::{Rect, Sense, Ui, pos2, vec2};

use super::keymap::{Action, Keymap};
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

    /// The tool's name on the rail (its tooltip and accessible label add its shortcut).
    pub(crate) fn name(self) -> &'static str {
        match self {
            Tool::Edit => "Edit",
            Tool::Heal => "Heal",
            Tool::Masking => "Masking",
            Tool::RedEye => "Red Eye",
        }
    }

    /// The tool after `picked` was chosen while this one is: the current tool's button (or key)
    /// again goes back to Edit, as LightCraft's rail does.
    pub(crate) fn after_picking(self, picked: Tool) -> Tool {
        if self == picked && picked != Tool::Edit { Tool::Edit } else { picked }
    }
}

/// The rail in `rect`; a click picks a tool (its key is in `keymap`, handled with the dialog's other
/// keys). Returns the tool picked this frame.
pub(crate) fn rail(ui: &mut Ui, rect: Rect, tool: &mut Tool, keymap: &Keymap) -> Option<Tool> {
    let t = Lc::get(ui.ctx());
    ui.painter().rect_filled(rect, 0.0, t.chrome);
    ui.painter().line_segment([rect.left_top(), rect.left_bottom()], egui::Stroke::new(1.0, t.divider));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(4.0, 10.0))).layout(egui::Layout::top_down(egui::Align::Center)));
    child.spacing_mut().item_spacing.y = 8.0;
    let mut picked = None;
    for candidate in Tool::RAIL {
        let tip = keymap.tip(super::develop_panel::tr(candidate.name()), Action::Tool(candidate));
        if icon_button(&mut child, candidate.icon(), vec2(40.0, 40.0), *tool == candidate, true, &tip).clicked() {
            picked = Some(candidate);
        }
    }
    let _ = child.allocate_rect(Rect::from_min_size(pos2(rect.left(), rect.bottom()), vec2(0.0, 0.0)), Sense::hover());
    if let Some(p) = picked {
        *tool = tool.after_picking(p);
    }
    picked
}
