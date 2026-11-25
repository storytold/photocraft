//! Shared root and nested group header layout and presentation for asset browsers.
//!
//! The browser owns expansion state and hierarchy traversal; brush and preset consumers share
//! this header's hit target and visual treatment while retaining their own group actions.

use egui::{Align2, Rect, Sense, pos2, vec2};

use crate::pickers::browser::hierarchy::HierarchyGroup;
use crate::pickers::browser::projection::ItemView;
use crate::pickers::state::PresetUi;
use crate::theme::Tokens;

const DARK_SURFACE_CONTRAST: f32 = 1.13;
const LIGHT_SURFACE_CONTRAST: f32 = 1.12;
const DARK_TEXT_CONTRAST: f32 = 7.11;
const LIGHT_TEXT_CONTRAST: f32 = 10.5;
pub(crate) const ROW_INDENT: f32 = 20.0;
pub(crate) const GROUP_HEADER_HEIGHT: f32 = 29.0;
const TREE_LEFT_INSET: f32 = 1.0;

/// Group expansion is explicit persisted state. No picker or group position supplies an implicit
/// default; a group is open only when its stable key is present in `expanded_groups`.
pub(crate) fn group_open(st: &PresetUi, panel: &str, name: &str) -> bool {
    st.expanded_groups.contains(&format!("{panel}/{}", crate::pickers::state::escape_hierarchy_key_part(name)))
}

/// Shared collapsible group row. The caller owns toggle state and context actions.
pub(crate) fn header(ui: &mut egui::Ui, name: &str, open: bool, depth: usize, height: f32, selected: bool) -> (Rect, egui::Response) {
    let (allocated_rect, allocated) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let rect = Rect::from_min_max(pos2(ui.clip_rect().left(), allocated_rect.top()), pos2(allocated_rect.right(), allocated_rect.bottom()));
    let response = ui.interact(inset_row_rect(rect, depth), allocated.id, Sense::click_and_drag());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    paint_with_selection(ui, rect, open, name, depth, false, selected);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, name));
    (rect, response)
}

/// Allocate and paint a nested-folder header, returning its hit and disclosure responses.
pub(crate) fn nested_header(
    ui: &mut egui::Ui,
    group: &HierarchyGroup<'_, ItemView>,
    open: bool,
    group_depth: usize,
    parent_depth: usize,
    selected: bool,
) -> (Rect, egui::Response, egui::Response, usize) {
    let (allocated_rect, allocated) = ui.allocate_exact_size(vec2(ui.available_width(), GROUP_HEADER_HEIGHT), Sense::hover());
    let rect = Rect::from_min_max(pos2(ui.clip_rect().left(), allocated_rect.top()), pos2(allocated_rect.right(), allocated_rect.bottom()));
    let row_depth = group_depth + 1 + parent_depth;
    let response = ui.interact(inset_row_rect(rect, row_depth), allocated.id, Sense::click_and_drag());
    let nested_header_rect = inset_row_rect(rect, row_depth);
    let disclosure_rect =
        Rect::from_min_max(nested_header_rect.min, pos2((nested_header_rect.left() + 18.0).min(nested_header_rect.right()), nested_header_rect.bottom()));
    let disclosure = ui.interact(disclosure_rect, response.id.with("disclosure"), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, group.path.last().map(String::as_str).unwrap_or("")));
    paint_with_selection(ui, rect, open, group.path.last().map(String::as_str).unwrap_or(""), row_depth, response.hovered(), selected);
    (rect, response, disclosure, row_depth)
}

fn paint_with_selection(ui: &egui::Ui, rect: Rect, open: bool, name: &str, depth: usize, _hovered: bool, selected: bool) {
    let tokens = Tokens::get(ui.ctx());
    let (surface_contrast, text_contrast) =
        if tokens.dark() { (DARK_SURFACE_CONTRAST, DARK_TEXT_CONTRAST) } else { (LIGHT_SURFACE_CONTRAST, LIGHT_TEXT_CONTRAST) };
    let colors = crate::color_utils::contrasting_surface_colors(tokens.card, tokens.dark(), surface_contrast, text_contrast);
    paint_header_contents(
        ui,
        rect,
        open,
        name,
        depth,
        if selected { tokens.picker_selection() } else { colors.background },
        colors.foreground,
        tokens.card_border,
    );
}

/// Paint the same header at a reduced opacity for its drag preview.
pub(crate) fn paint_with_opacity(ui: &egui::Ui, rect: Rect, open: bool, name: &str, depth: usize, opacity: f32) {
    let tokens = Tokens::get(ui.ctx());
    let (surface_contrast, text_contrast) =
        if tokens.dark() { (DARK_SURFACE_CONTRAST, DARK_TEXT_CONTRAST) } else { (LIGHT_SURFACE_CONTRAST, LIGHT_TEXT_CONTRAST) };
    let colors = crate::color_utils::contrasting_surface_colors(tokens.card, tokens.dark(), surface_contrast, text_contrast);
    let with_opacity = |color| crate::color_utils::with_opacity(color, opacity);
    paint_header_contents(ui, rect, open, name, depth, with_opacity(colors.background), with_opacity(colors.foreground), with_opacity(tokens.card_border));
}

fn paint_header_contents(
    ui: &egui::Ui,
    rect: Rect,
    open: bool,
    name: &str,
    depth: usize,
    background: egui::Color32,
    foreground: egui::Color32,
    border: egui::Color32,
) {
    let indent = row_indent(depth);
    let row_rect = inset_row_rect(rect, depth);
    ui.painter().rect_filled(row_rect, egui::CornerRadius::ZERO, background);
    ui.painter().rect_stroke(
        row_rect,
        egui::CornerRadius::ZERO,
        egui::Stroke::new(1.0, border),
        // The border is contained by the allocated header dimensions.
        egui::StrokeKind::Inside,
    );
    let content_left = rect.left() + TREE_LEFT_INSET + indent;
    crate::icons::paint(
        ui,
        // The chevron path is inset within its SVG viewBox; position its visible stroke, not the viewport.
        Rect::from_center_size(pos2(content_left + 9.0, rect.center().y), vec2(12.0, 12.0)),
        if open { "chevron-down" } else { "chevron-right" },
        12.0,
        foreground,
    );
    crate::icons::paint(
        ui,
        Rect::from_center_size(pos2(content_left + 23.0, rect.center().y), vec2(12.0, 12.0)),
        if open { "folder-open" } else { "folder" },
        12.0,
        foreground,
    );
    ui.painter().text(pos2(content_left + 38.0, rect.center().y), Align2::LEFT_CENTER, name, egui::FontId::proportional(11.5), foreground);
}

pub(crate) fn row_indent(depth: usize) -> f32 {
    ROW_INDENT * depth.min(4) as f32
}

/// Inset both the painted row and its interactive target; preserve the right edge and height.
pub(crate) fn inset_row_rect(rect: Rect, depth: usize) -> Rect {
    Rect::from_min_max(pos2((rect.left() + TREE_LEFT_INSET + row_indent(depth)).min(rect.right()), rect.top()), rect.max)
}

#[cfg(test)]
mod tests {
    use super::{group_open, inset_row_rect, row_indent};
    use crate::pickers::state::PresetUi;
    use egui::{Rect, pos2};

    #[test]
    fn row_hit_target_uses_the_visual_indent() {
        let rect = Rect::from_min_max(pos2(10.0, 20.0), pos2(210.0, 50.0));

        assert_eq!(row_indent(0), 0.0);
        assert_eq!(row_indent(1), 20.0);
        assert_eq!(row_indent(2), 40.0);
        assert_eq!(inset_row_rect(rect, 0), Rect::from_min_max(pos2(11.0, 20.0), pos2(210.0, 50.0)));
        assert_eq!(inset_row_rect(rect, 1), Rect::from_min_max(pos2(31.0, 20.0), pos2(210.0, 50.0)));
    }

    #[test]
    fn every_root_group_has_an_independent_explicit_expansion_state() {
        let names = ["Gradients", "Favorites", "Recent", "Imported", "Custom"];
        for expanded in 0..(1usize << names.len()) {
            let mut state = PresetUi::default();
            for (index, name) in names.iter().enumerate() {
                if expanded & (1 << index) != 0 {
                    state.expanded_groups.insert(format!("gradients/{name}"));
                }
            }
            for (index, name) in names.iter().enumerate() {
                assert_eq!(group_open(&state, "gradients", name), expanded & (1 << index) != 0, "expansion mask={expanded:05b}, group={name}");
            }
        }
    }
}
