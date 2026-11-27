//! Shared brush preset browser used by the Brushes tab and the right-click Brush Picker.

use egui::Color32;
use std::cell::RefCell;
use photocraft_engine::paint::BrushPreset;

pub(crate) use super::brush_engine_adapter::browser_groups;
pub use super::brush_engine_adapter::{
    Action, Renaming, UNGROUPED, apply, browser_actions_to_actions, browser_actions_to_actions_for_panel,
    group_view_key, preset_by_identity, preview_identity, run_or_status,
};
use super::brush_engine_adapter::{
    active_brush_selection, brush_group_move_actions, brush_item_move_action, container_key_for_preset, project_brush_browser,
    ROOT_ITEMS_GROUP,
};
use crate::PhotocraftApp;
use crate::brush_preview;
use crate::pickers::browser::asset_browser::{self, BrowserAction, BrowserAdapter, BrowserCapabilities, BrowserOptions, ItemPaintState, begin_item_rename};
use crate::pickers::browser::identity::PickerNodeIdentity;
use crate::pickers::browser::path_identity_model::{PickerBrowserHierarchy, PickerId};
use crate::pickers::state::PresetUi;
use crate::theme::Tokens;

/// Clear the compact Texture picker selection when the active brush changes.
pub(crate) fn clear_texture_pattern_selection(app: &mut PhotocraftApp) {
    app.ui.presets_ui.selected.remove("brush-texture-patterns");
}

pub(crate) fn full_uv() -> egui::Rect {
    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0))
}

/// Scroll-area settings for a brush preset browser.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListLayout {
    pub id: &'static str,
    pub max_height: f32,
}

pub const PANEL_LIST: ListLayout = ListLayout { id: "brush-presets", max_height: 400.0 };

pub(crate) fn draw_brush_footer_controls(
    ui: &mut egui::Ui,
    state: &mut PresetUi,
    presets: &[BrushPreset],
    panel: &str,
    preview_size: bool,
) -> Vec<BrowserAction> {
    let selection = state.browser_selection_snapshot.get(panel);
    let mut delete_targets = selection.map(crate::pickers::browser::selection_operations::delete_selection).unwrap_or_default();
    if delete_targets.is_empty()
        && let Some(key) = state.selected.get(panel)
        && let Some(preset) = preset_by_identity(presets, key)
    {
        delete_targets.push(BrowserAction::DeleteItem { group: preset.group.clone(), group_path: preset.folder.clone(), key: key.clone() });
    }
    let mut actions = Vec::new();
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let delete = ui.add_enabled_ui(!delete_targets.is_empty(), |ui| crate::icons::button(ui, "trash", 24.0, false, tl!("Delete"))).inner;
        if delete.clicked() { actions.extend(delete_targets); }
        if crate::icons::button(ui, "plus", 24.0, false, tl!("Create new brush preset")).clicked() { actions.push(BrowserAction::New); }
        if crate::icons::button(ui, "folder-plus", 24.0, false, tl!("Create new group")).clicked() { actions.push(BrowserAction::NewGroup); }
        if preview_size {
            ui.add_space(8.0);
            let response = crate::widgets::slider(ui, &mut state.brush_picker.scale, 0.15..=2.0, None);
            if response.is_pointer_button_down_on() { egui::Popup::close_all(&response.ctx); }
        }
    });
    actions
}

pub(crate) fn picker_browser_with_shell(
    ui: &mut egui::Ui,
    presets: &[BrushPreset],
    current: Option<&str>,
    preset_ui: &mut PresetUi,
    shell: crate::pickers::shell::ShellOptions<'_>,
) -> Vec<Action> {
    const PANEL: &str = "brushes";
    let projection = project_brush_browser(presets, preset_ui, PANEL);
    let groups = projection.groups.clone();
    let projection_keys = projection.keys.clone();
    let view = preset_ui.brush_picker.clone();
    preset_ui.custom_item_sizes.insert(PANEL.into(), brush_picker_card_size(&view));
    let selected_container = preset_ui.selected_group.contains_key(PANEL)
        && !preset_ui.selected.contains_key(PANEL)
        && preset_ui.browser_selection.get(PANEL).is_none_or(|selection| selection.is_empty());
    if let Some(name) = current {
        if !selected_container {
            if let Some(preset) = presets.iter().find(|preset| preset.name.eq_ignore_ascii_case(name)) {
                let container = container_key_for_preset(&projection, preset);
                preset_ui.set_selected_item(PANEL, &container, &[], &preset.name);
            } else { preset_ui.clear_selected(PANEL); }
        }
    } else if !selected_container { preset_ui.clear_selected(PANEL); }

    let hierarchy_actions = RefCell::new(Vec::new());
    let mut paint_item = |ui: &egui::Ui, rect: egui::Rect, group_index: usize, item_index: usize, state: ItemPaintState| {
        let Some(preset) = projection.group_views.get(group_index).and_then(|group| group.presets.get(item_index)).copied() else { return };
        paint_brush_item(ui, rect, preset, &view, state);
    };
    let mut item_move = |source_group: &str, name: &str, destination_group: &str, index: usize| -> Result<(), String> {
        hierarchy_actions.borrow_mut().push(brush_item_move_action(PANEL, &projection, &groups, name, destination_group, index)?);
        let _ = source_group;
        Ok(())
    };
    let mut group_move = |intent: &crate::pickers::browser::dragging_model::DropMoveIntent,
                         hierarchy: &PickerBrowserHierarchy,
                         identities: &std::collections::BTreeMap<PickerId, PickerNodeIdentity>| {
        hierarchy_actions.borrow_mut().extend(brush_group_move_actions(PANEL, &projection, hierarchy, identities, intent)?);
        Ok(())
    };
    let mut adapter = BrowserAdapter {
        panel: PANEL,
        groups: &groups,
        projection_keys: Some(&projection_keys),
        options: BrowserOptions {
            canvas: egui::Rect::NOTHING, flush: false, center_grid: false, compact_context_menus: false,
            domain_hierarchy_actions: false, root_item_group: Some(ROOT_ITEMS_GROUP.into()),
        },
        capabilities: BrowserCapabilities { item_drag: true, item_drop: true, group_drag: true, selection: true, activation: true },
        item_context_menu: Some(asset_browser::standard_item_context_menu),
        group_context_menu: Some(asset_browser::standard_group_context_menu),
        nested_group_context_menu: Some(asset_browser::standard_nested_group_context_menu),
        canonical_hierarchy_move: None,
        rekey_hierarchy_after_move: None,
        hierarchy_item_move: Some(&mut item_move),
        hierarchy_group_move: Some(&mut group_move),
        paint_group: None,
        paint_item: &mut paint_item,
        paint_list_item: None,
    };
    let browser_actions = crate::pickers::shell::picker_shell(ui, preset_ui, &mut adapter, shell);
    drop(adapter);
    apply_inline_rename_actions(preset_ui, PANEL, &browser_actions);
    if browser_actions.iter().any(|action| matches!(action, BrowserAction::NewGroup)) {
        crate::pickers::browser::popup_dialogs::request_new_group(ui.ctx(), PANEL);
    }
    let selection = preset_ui.browser_selection_snapshot.get(PANEL).filter(|selection| !selection.targets.is_empty()).cloned()
        .or_else(|| active_brush_selection(preset_ui, PANEL));
    let mut actions = browser_actions_to_actions_for_panel(presets, PANEL, browser_actions, selection.as_ref());
    let staged_actions = hierarchy_actions.borrow_mut().drain(..).collect::<Vec<_>>();
    actions.extend(staged_actions);
    actions
}

fn brush_picker_card_size(view: &crate::pickers::state::BrushPickerView) -> egui::Vec2 {
    let scale = view.scale.clamp(0.15, 2.0);
    // Keep the tip card close to the largest thumbnail actually painted inside it
    // (80% of the cell). Text visibility must not change preview hit-target geometry.
    let tip_side = 40.0 * scale;
    let stroke_length = 128.0 * scale;
    let stroke_height = 32.0 * scale;
    let preview_gap = 8.0 * scale;
    const NAME_HEIGHT: f32 = 20.0;
    const NAME_GAP: f32 = 4.0;
    let preview_width = match (view.show_tip, view.show_stroke) {
        (true, true) => tip_side + preview_gap + stroke_length,
        (true, false) => tip_side,
        (false, true) => stroke_length,
        (false, false) => 0.0,
    };
    let preview_height = if view.show_tip {
        tip_side
    } else if view.show_stroke {
        stroke_height
    } else {
        0.0
    };
    let name_in_stroke_slot = view.show_name && view.show_tip && !view.show_stroke;
    let width = if name_in_stroke_slot {
        tip_side + preview_gap + stroke_length
    } else if preview_width == 0.0 && view.show_name {
        180.0
    } else {
        preview_width
    };
    let height = if view.show_name && preview_height > 0.0 && view.show_stroke {
        preview_height + NAME_GAP + NAME_HEIGHT
    } else if preview_height == 0.0 && view.show_name {
        NAME_HEIGHT
    } else {
        preview_height
    };
    egui::vec2(width.max(1.0), height.max(1.0))
}

/// Translate the shared browser's asset-neutral events into brush preset commands.

/// Paint only the brush tip inside the shared asset-browser cell. The browser owns card
/// geometry, labels, selection, clipping, and list/grid presentation.
fn paint_brush_thumbnail(ui: &egui::Ui, rect: egui::Rect, preset: &BrushPreset) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let brush = &preset.brush;
    let identity = preview_identity(preset);
    let color = Tokens::get(ui.ctx()).text;
    let texture =
        brush_preview::tip_texture(ui.ctx(), &format!("brushes-tip:{identity}"), &brush.tip, (brush.hardness, brush.angle, brush.roundness), 36, color);
    let side = rect.width().min(rect.height()).max(1.0) * 0.8 * crate::brush_sections::thumb_scale(brush.size);
    let thumb = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(side));
    ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).image(texture.id(), thumb, full_uv(), Color32::WHITE);
}

fn paint_brush_presentation(ui: &egui::Ui, rect: egui::Rect, preset: &BrushPreset, view: &crate::pickers::state::BrushPickerView) {
    let tokens = Tokens::get(ui.ctx());
    let scale = view.scale.clamp(0.15, 2.0);
    let tip_side = 40.0 * scale;
    let stroke_length = 128.0 * scale;
    let stroke_height = 32.0 * scale;
    const NAME_HEIGHT: f32 = 20.0;
    let tip_rect = egui::Rect::from_min_size(rect.min, egui::vec2(tip_side, tip_side));
    if view.show_tip {
        paint_brush_thumbnail(ui, tip_rect, preset);
    }
    if view.show_stroke {
        let stroke_left = if view.show_tip { tip_rect.right() + 8.0 * scale } else { rect.left() };
        let stroke_top = if view.show_tip { rect.top() + (tip_side - stroke_height) * 0.5 } else { rect.top() };
        let stroke_rect = egui::Rect::from_min_size(egui::pos2(stroke_left, stroke_top), egui::vec2(stroke_length, stroke_height));
        if ui.is_rect_visible(stroke_rect) {
            let texture = brush_preview::stroke_texture(ui.ctx(), &format!("brushes-stroke:{}", preview_identity(preset)), &preset.brush, 128, 32, tokens.text);
            ui.painter().with_clip_rect(stroke_rect.intersect(ui.clip_rect())).image(texture.id(), stroke_rect, full_uv(), Color32::WHITE);
        }
    }
    if view.show_name {
        let name_rect = if !view.show_tip && !view.show_stroke {
            rect
        } else if view.show_tip && !view.show_stroke {
            let left = tip_rect.right() + 8.0 * scale;
            egui::Rect::from_min_size(egui::pos2(left, rect.center().y - NAME_HEIGHT * 0.5), egui::vec2((rect.right() - left).max(0.0), NAME_HEIGHT))
        } else {
            // When a stroke is visible, put the name on its own row beneath both previews.
            egui::Rect::from_min_size(egui::pos2(rect.left(), rect.top() + rect.height() - NAME_HEIGHT), egui::vec2(rect.width(), NAME_HEIGHT))
        };
        ui.painter().with_clip_rect(name_rect.intersect(ui.clip_rect())).text(
            egui::pos2(name_rect.left() + 4.0, name_rect.center().y),
            egui::Align2::LEFT_CENTER,
            &preset.name,
            egui::FontId::proportional(11.5),
            tokens.text,
        );
    }
}

/// Recent cards are deliberately independent of the configurable browser-card presentation.
/// Both the Brushes panel and the right-click brush picker use this adapter, so keep their
/// compact history previews on the same brush-tip-only path.
fn paint_brush_item(ui: &egui::Ui, rect: egui::Rect, preset: &BrushPreset, view: &crate::pickers::state::BrushPickerView, state: ItemPaintState) {
    if brush_item_uses_tip_only_preview(state) {
        paint_brush_thumbnail(ui, rect, preset);
    } else {
        paint_brush_presentation(ui, rect, preset, view);
    }
}

fn brush_item_uses_tip_only_preview(state: ItemPaintState) -> bool {
    matches!(state, ItemPaintState::Recent | ItemPaintState::RecentDragGhost)
}

/// Render the Brushes tab through the shared browser. `PresetUi` owns the complete picker state.
pub fn preset_list(ui: &mut egui::Ui, presets: &[BrushPreset], current: Option<&str>, preset_ui: &mut PresetUi, layout: ListLayout) -> Vec<Action> {
    const PANEL: &str = "brushes";
    let projection = project_brush_browser(presets, preset_ui, PANEL);
    let groups = projection.groups.clone();
    let projection_keys = projection.keys.clone();
    let view = preset_ui.brush_picker.clone();
    preset_ui.custom_item_sizes.insert(PANEL.into(), brush_picker_card_size(&view));
    let selected_container = preset_ui.selected_group.contains_key(PANEL)
        && !preset_ui.selected.contains_key(PANEL)
        && preset_ui.browser_selection.get(PANEL).is_none_or(|selection| selection.is_empty());
    if let Some(name) = current {
        if !selected_container {
            if let Some(preset) = presets.iter().find(|preset| preset.name.eq_ignore_ascii_case(name)) {
                let container = container_key_for_preset(&projection, preset);
                preset_ui.set_selected_item(PANEL, &container, &[], &preset.name);
            } else { preset_ui.clear_selected(PANEL); }
        }
    } else if !selected_container { preset_ui.clear_selected(PANEL); }

    let hierarchy_actions = RefCell::new(Vec::new());
    let mut paint_item = |ui: &egui::Ui, rect: egui::Rect, group_index: usize, item_index: usize, state: ItemPaintState| {
        let Some(preset) = projection.group_views.get(group_index).and_then(|group| group.presets.get(item_index)).copied() else { return };
        paint_brush_item(ui, rect, preset, &view, state);
    };
    let mut item_move = |source_group: &str, name: &str, destination_group: &str, index: usize| -> Result<(), String> {
        hierarchy_actions.borrow_mut().push(brush_item_move_action(PANEL, &projection, &groups, name, destination_group, index)?);
        let _ = source_group;
        Ok(())
    };
    let mut group_move = |intent: &crate::pickers::browser::dragging_model::DropMoveIntent,
                         hierarchy: &PickerBrowserHierarchy,
                         identities: &std::collections::BTreeMap<PickerId, PickerNodeIdentity>| {
        hierarchy_actions.borrow_mut().extend(brush_group_move_actions(PANEL, &projection, hierarchy, identities, intent)?);
        Ok(())
    };
    let mut footer = |ui: &mut egui::Ui, state: &mut PresetUi| draw_brush_footer_controls(ui, state, presets, PANEL, true);
    let mut adapter = BrowserAdapter {
        panel: PANEL,
        groups: &groups,
        projection_keys: Some(&projection_keys),
        options: BrowserOptions {
            canvas: egui::Rect::NOTHING, flush: false, center_grid: false, compact_context_menus: false,
            domain_hierarchy_actions: false, root_item_group: Some(ROOT_ITEMS_GROUP.into()),
        },
        capabilities: BrowserCapabilities { item_drag: true, item_drop: true, group_drag: true, selection: true, activation: true },
        item_context_menu: Some(asset_browser::standard_item_context_menu),
        group_context_menu: Some(asset_browser::standard_group_context_menu),
        nested_group_context_menu: Some(asset_browser::standard_nested_group_context_menu),
        canonical_hierarchy_move: None,
        rekey_hierarchy_after_move: None,
        hierarchy_item_move: Some(&mut item_move),
        hierarchy_group_move: Some(&mut group_move),
        paint_group: None,
        paint_item: &mut paint_item,
        paint_list_item: None,
    };
    let browser_actions = crate::pickers::shell::picker_shell(
        ui, preset_ui, &mut adapter,
        crate::pickers::shell::ShellOptions { custom_footer: Some((&mut footer, 28.0)), max_total_height: Some(layout.max_height), ..Default::default() },
    );
    drop(adapter);
    if browser_actions.iter().any(|action| matches!(action, BrowserAction::NewGroup)) {
        crate::pickers::browser::popup_dialogs::request_new_group(ui.ctx(), PANEL);
    }
    apply_inline_rename_actions(preset_ui, PANEL, &browser_actions);
    let selection = preset_ui.browser_selection_snapshot.get(PANEL).filter(|selection| !selection.targets.is_empty()).cloned()
        .or_else(|| active_brush_selection(preset_ui, PANEL));
    let mut actions = browser_actions_to_actions_for_panel(presets, PANEL, browser_actions, selection.as_ref());
    let staged_actions = hierarchy_actions.borrow_mut().drain(..).collect::<Vec<_>>();
    actions.extend(staged_actions);
    actions
}

fn apply_inline_rename_actions(state: &mut PresetUi, panel: &str, actions: &[BrowserAction]) {
    for action in actions {
        match action {
            BrowserAction::BeginInlineRenameItem(group, item) => begin_item_rename(state, panel, &[(group.clone(), item.clone())]),
            BrowserAction::CancelInlineRenameItem => {
                state.item_renaming = None;
                state.item_rename_queue.clear();
                state.renaming = None;
            }
            BrowserAction::RenameItem { .. } | BrowserAction::Rename(_, _) => {
                state.renaming = state.item_renaming.as_ref().map(|rename| (panel.to_owned(), rename.key.clone(), rename.text.clone()));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "brushes_tests.rs"]
mod tests;
