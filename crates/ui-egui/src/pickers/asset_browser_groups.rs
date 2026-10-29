//! Shared group and folder header presentation for asset browsers.
//!
//! The browser owns expansion state and hierarchy traversal; brush and preset consumers share
//! this header's hit target and visual treatment while retaining their own group actions.

use egui::{Align2, Rect, Sense, pos2, vec2};

use crate::pickers::picker_state::PresetUi;
use crate::theme::Tokens;

const DARK_SURFACE_CONTRAST: f32 = 1.13;
const LIGHT_SURFACE_CONTRAST: f32 = 1.12;
const DARK_TEXT_CONTRAST: f32 = 7.11;
const LIGHT_TEXT_CONTRAST: f32 = 10.5;

/// Initial expansion policy for a browser group, with explicit user toggles overriding the default.
pub(crate) fn group_open(st: &PresetUi, panel: &str, index: usize, name: &str, all_groups_open: bool) -> bool {
    (index == 0 || all_groups_open) != st.toggled.contains(&format!("{panel}/{}", crate::pickers::picker_state::escape_hierarchy_key_part(name)))
}

/// Shared collapsible group/folder row. The caller owns toggle state and context actions.
pub(crate) fn header(ui: &mut egui::Ui, name: &str, open: bool, depth: usize, count: Option<usize>, height: f32) -> (Rect, egui::Response) {
    let (rect, allocated) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let response = ui.interact(inset_row_rect(rect, depth), allocated.id, Sense::click_and_drag());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    paint(ui, rect, open, name, depth, false, count);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, name));
    (rect, response)
}

/// Paint a group/folder header; nesting is conveyed by indentation rather than an accent bar.
pub(crate) fn paint(ui: &egui::Ui, rect: Rect, open: bool, name: &str, depth: usize, _hovered: bool, count: Option<usize>) {
    paint_with_opacity(ui, rect, open, name, depth, count, 1.0);
}

/// Paint the same header at a reduced opacity for its drag preview.
pub(crate) fn paint_with_opacity(ui: &egui::Ui, rect: Rect, open: bool, name: &str, depth: usize, count: Option<usize>, opacity: f32) {
    let tokens = Tokens::get(ui.ctx());
    let indent = row_indent(depth);
    let (surface_contrast, text_contrast) =
        if tokens.dark() { (DARK_SURFACE_CONTRAST, DARK_TEXT_CONTRAST) } else { (LIGHT_SURFACE_CONTRAST, LIGHT_TEXT_CONTRAST) };
    let colors = crate::color_utils::contrasting_surface_colors(tokens.card, tokens.dark(), surface_contrast, text_contrast);
    let with_opacity = |color| crate::color_utils::with_opacity(color, opacity);
    ui.painter().rect_filled(inset_row_rect(rect, depth), egui::CornerRadius::ZERO, with_opacity(colors.background));
    let content_left = rect.left() + indent;
    crate::icons::paint(
        ui,
        // The chevron path is inset within its SVG viewBox; position its visible stroke, not the viewport.
        Rect::from_center_size(pos2(content_left + 9.0, rect.center().y), vec2(12.0, 12.0)),
        if open { "chevron-down" } else { "chevron-right" },
        12.0,
        with_opacity(colors.foreground),
    );
    crate::icons::paint(
        ui,
        Rect::from_center_size(pos2(content_left + 23.0, rect.center().y), vec2(12.0, 12.0)),
        if open { "folder-open" } else { "folder" },
        12.0,
        with_opacity(colors.foreground),
    );
    ui.painter().text(pos2(content_left + 38.0, rect.center().y), Align2::LEFT_CENTER, name, egui::FontId::proportional(11.5), with_opacity(colors.foreground));
    if let Some(count) = count {
        ui.painter().text(
            pos2(rect.right() - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            count.to_string(),
            egui::FontId::proportional(11.0),
            with_opacity(colors.foreground),
        );
    }
}

pub(crate) fn row_indent(depth: usize) -> f32 {
    (14.0 * depth.min(4) as f32 - 7.0).max(0.0)
}

/// Inset both the painted row and its interactive target; preserve the right edge and height.
pub(crate) fn inset_row_rect(rect: Rect, depth: usize) -> Rect {
    Rect::from_min_max(pos2((rect.left() + row_indent(depth)).min(rect.right()), rect.top()), rect.max)
}

#[cfg(test)]
mod tests {
    use super::{inset_row_rect, row_indent};
    use egui::{Rect, pos2};

    #[test]
    fn row_hit_target_uses_the_visual_indent() {
        let rect = Rect::from_min_max(pos2(10.0, 20.0), pos2(210.0, 50.0));

        assert_eq!(row_indent(0), 0.0);
        assert_eq!(row_indent(1), 7.0);
        assert_eq!(inset_row_rect(rect, 1), Rect::from_min_max(pos2(17.0, 20.0), pos2(210.0, 50.0)));
    }
}
