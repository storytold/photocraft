//! Shared item layout, filtering, row presentation and accessibility for asset browsers.
//!
//! Asset adapters paint their own previews; this module owns card/list geometry and query
//! semantics while keeping row feedback and accessibility consistent across consumers.

use egui::{Rect, vec2};

use crate::pickers::picker_browser_projection::{GroupView, ItemView};
use crate::theme::Tokens;

/// Return the visual grid coordinate for an item in row-major thumbnails or column-major wrapped lists.
pub(crate) fn item_grid_position(index: usize, rows: usize, cols: usize, column_major: bool) -> (f32, f32) {
    if column_major {
        let rows = rows.max(1);
        ((index / rows) as f32, (index % rows) as f32)
    } else {
        ((index % cols.max(1)) as f32, (index / cols.max(1)) as f32)
    }
}

/// Calculate list-card dimensions and wrapping from the available browser viewport.
pub(crate) fn list_layout_geometry(
    available_width: f32,
    indent: f32,
    height: f32,
    wrap: bool,
    fixed_width: Option<f32>,
    item_count: usize,
) -> (egui::Vec2, usize, usize) {
    let content_width = (available_width - indent).max(0.0);
    // `available_width` is already constrained by the ScrollArea viewport, so its
    // scrollbar gutter is preserved. The row origin is inset separately; subtract
    // that same inset here so the row still ends at the header's right edge.
    let width = if wrap { fixed_width.map(|width| width.min(content_width)).unwrap_or(content_width) } else { content_width };
    let columns = if wrap { (content_width / width.max(1.0)).floor().max(1.0) as usize } else { 1 };
    let rows = item_count.div_ceil(columns);
    (vec2(width, height), columns, rows)
}

pub(crate) fn group_matches_query(group: &GroupView, query: &str) -> bool {
    !query.is_empty() && group.name.to_lowercase().contains(query)
}

pub(crate) fn item_matches_query(item: &ItemView, query: &str) -> bool {
    !query.is_empty() && (item.name.to_lowercase().contains(query) || item.group_path.iter().any(|part| part.to_lowercase().contains(query)))
}

pub(crate) fn group_visible_for_query(query: &str, group_matches: bool, has_matching_items: bool) -> bool {
    query.is_empty() || group_matches || has_matching_items
}

pub(crate) fn item_visible_for_query(item: &ItemView, query: &str, group_matches: bool) -> bool {
    query.is_empty() || group_matches || item_matches_query(item, query)
}

/// Attach selection semantics for screen readers to a custom item renderer.
pub(crate) fn item_info(response: &egui::Response, label: &str, selected: bool) {
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, label));
}

/// Paint shared selected/hovered feedback for a list row.
pub(crate) fn list_row(ui: &egui::Ui, response: &egui::Response, rect: Rect, label: &str, selected: bool) {
    item_info(response, label, selected);
    let tokens = Tokens::get(ui.ctx());
    if selected {
        ui.painter().rect_filled(rect, 0.0, tokens.row_selected);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, tokens.hover.gamma_multiply(0.5));
    }
}
