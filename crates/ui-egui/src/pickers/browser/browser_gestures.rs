//! Non-drag browser gesture handling.

use crate::pickers::browser::asset_browser::{BrowserAction, BrowserCtx, BrowserSink, PaintedCell};
use crate::pickers::browser::browser_interaction::asset_browser_item_gesture;
use crate::pickers::browser::path_identity_model::PickerId;

/// Apply shared click/double-click selection semantics and emit the corresponding browser action.
pub(super) fn apply_browser_item_gesture(
    ui: &egui::Ui,
    painted: &PaintedCell<'_>,
    ctx: &BrowserCtx<'_>,
    sink: &mut BrowserSink<'_>,
    visible_selection_order: &[PickerId],
) -> egui::Response {
    let group = painted.item.group;
    let item = painted.item.item;
    let response = painted.response.clone();
    let BrowserCtx { panel, capabilities, .. } = *ctx;
    let hover = if item.group_path.is_empty() { item.name.clone() } else { format!("{} / {}", item.group_path.join(" / "), item.name) };
    let response = response.on_hover_text(hover);
    let (selected, activated) = asset_browser_item_gesture(&response, capabilities.selection, capabilities.activation);
    if selected && let Some(id) = painted.node_id.as_ref() {
        let modifiers = ui.input(|input| input.modifiers);
        if modifiers.shift {
            sink.select_range(panel, visible_selection_order, id);
        } else {
            sink.select_node(panel, id.clone(), modifiers.ctrl);
        }
    }
    if activated {
        sink.emit(BrowserAction::ActivateItem { group: group.name.clone(), group_path: item.group_path.clone(), key: item.key.clone() });
    } else if selected {
        sink.emit(BrowserAction::SelectItem { group: group.name.clone(), group_path: item.group_path.clone(), key: item.key.clone() });
    }
    response
}
