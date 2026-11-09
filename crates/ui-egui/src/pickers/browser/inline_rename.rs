//! Shared transient state and rendering for inline item and group renaming.

use egui::{Rect, pos2, vec2};

use crate::pickers::browser::asset_browser::{BrowserAction, ItemView};

/// Transient item rename target. Item keys are only unique inside their owning domain group.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ItemRenameState {
    pub(crate) panel: String,
    pub(crate) group: String,
    pub(crate) group_path: Vec<String>,
    pub(crate) key: String,
    pub(crate) text: String,
    pub(crate) focused: bool,
}

/// Transient inline group rename target, identified by its owning group and folder path.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct InlineGroupRenameState {
    pub(crate) panel: String,
    pub(crate) group: String,
    pub(crate) group_path: Vec<String>,
    pub(crate) text: String,
    pub(crate) focused: bool,
}

/// Paint the inline editor for an item label and return the action when editing completes.
pub(crate) fn paint_item_name(ui: &mut egui::Ui, rename: &mut ItemRenameState, panel: &str, group: &str, item: &ItemView) -> Option<BrowserAction> {
    let edit_id = ui.make_persistent_id(("browser-inline-item-rename", panel, group, &item.group_path, &item.key));
    let max_width = (ui.available_width() - 2.0).max(0.0);
    let field_width = field_width(ui, &rename.text, max_width, 8.0);
    let response = ui.add(egui::TextEdit::singleline(&mut rename.text).id(edit_id).desired_width(field_width).margin(egui::vec2(4.0, 2.0)).cursor_at_end(true));
    if !rename.focused {
        response.request_focus();
        rename.focused = true;
    }
    let escape = response.ctx.input(|input| input.key_pressed(egui::Key::Escape));
    let enter = response.ctx.input(|input| input.key_pressed(egui::Key::Enter));
    if escape {
        return Some(BrowserAction::CancelInlineRenameItem);
    }
    if (enter || response.lost_focus()) && !rename.text.trim().is_empty() {
        return Some(BrowserAction::RenameItem {
            group: rename.group.clone(),
            group_path: rename.group_path.clone(),
            key: rename.key.clone(),
            text: rename.text.trim().to_owned(),
        });
    }
    None
}

/// Replace a group label with an editor while preserving its header's icons, count and hitbox.
pub(crate) fn paint_group_name(
    ui: &mut egui::Ui,
    rect: Rect,
    depth: usize,
    panel: &str,
    group: &str,
    group_path: &[String],
    rename: &mut Option<InlineGroupRenameState>,
) -> Option<BrowserAction> {
    let matches_target = rename.as_ref().is_some_and(|rename| rename.panel == panel && rename.group == group && rename.group_path == group_path);
    if !matches_target {
        return None;
    }

    let rename = rename.as_mut()?;
    let left = rect.left() + 1.0 + crate::pickers::browser::groups::row_indent(depth) + 38.0;
    let right = (rect.right() - 38.0 - 2.0).max(left);
    let max_width = (right - left).max(0.0);
    let width = field_width(ui, &rename.text, max_width, 6.0);
    let edit_rect = Rect::from_min_size(pos2(left, rect.top() + 3.0), vec2(width, (rect.height() - 6.0).max(0.0)));
    let id = ui.make_persistent_id(("browser-inline-group-rename", panel, group, group_path));
    let response = ui
        .put(edit_rect, egui::TextEdit::singleline(&mut rename.text).id(id).desired_width(edit_rect.width()).margin(egui::vec2(3.0, 1.0)).cursor_at_end(true));
    if !rename.focused {
        response.request_focus();
        rename.focused = true;
    }
    let escape = response.ctx.input(|input| input.key_pressed(egui::Key::Escape));
    let enter = response.ctx.input(|input| input.key_pressed(egui::Key::Enter));
    if escape {
        Some(BrowserAction::CancelInlineRenameGroup)
    } else if (enter || response.lost_focus()) && !rename.text.trim().is_empty() {
        Some(if group_path.is_empty() {
            BrowserAction::RenameGroup(group.to_owned(), rename.text.trim().to_owned())
        } else {
            BrowserAction::RenameNestedGroup { group: group.to_owned(), group_path: group_path.to_vec(), text: rename.text.trim().to_owned() }
        })
    } else {
        None
    }
}

/// Size an editor to its current text, bounded by its containing row/header.
fn field_width(ui: &egui::Ui, text: &str, max_width: f32, horizontal_margin: f32) -> f32 {
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let text_width = ui.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), font_id, ui.visuals().text_color()).size().x);
    (text_width + horizontal_margin).max(72.0).min(max_width.max(0.0))
}
