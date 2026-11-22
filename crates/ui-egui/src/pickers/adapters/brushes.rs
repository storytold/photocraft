//! Shared brush preset browser used by the Brushes tab and the right-click Brush Picker.

use std::collections::BTreeMap;

use egui::Color32;
use photocraft_engine::paint::BrushPreset;
use serde::{Deserialize, Serialize};

pub(crate) use super::brush_engine_adapter::browser_groups;
use super::brush_engine_adapter::brush_group_views;
pub use super::brush_engine_adapter::{
    Action, Renaming, UNGROUPED, apply, browser_actions_to_actions, drop_target, folder_view_key, group_key, group_view_key, grouped_presets,
    is_ambiguous_preset_name, preset_by_identity, preview_identity, run_or_status,
};
use super::brush_engine_adapter::{
    brush_drop_intent_to_action, brush_projection_keys, brush_root_sequence, group_label_for_key, occurrence_name, rekey_brush_hierarchy,
};
use crate::PhotocraftApp;
use crate::brush_preview;
use crate::pickers::browser::asset_browser::{
    self, BrowserAction, BrowserAdapter, BrowserCapabilities, BrowserOptions, GroupView, ItemPaintState, ItemView, begin_item_rename,
};
use crate::pickers::browser::dragging_model::DropMoveIntent;
use crate::pickers::browser::hierarchy::{HierarchyGroup, HierarchyNode, hierarchy_tree};
use crate::pickers::browser::hierarchy_utils::{HierarchyDrop, hierarchy_group_drop};
use crate::pickers::browser::identity::PickerNodeIdentity;
use crate::pickers::browser::path_identity_model::{PickerBrowserHierarchy, PickerId, PickerIdRegistry};
use crate::pickers::state::{NestedGroupRenameState, PresetUi};
use crate::theme::Tokens;

/// Clear the compact Texture picker selection when the active brush changes.
pub(crate) fn clear_texture_pattern_selection(app: &mut PhotocraftApp) {
    app.ui.presets_ui.selected.remove("brush-texture-patterns");
}

pub(crate) fn full_uv() -> egui::Rect {
    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0))
}

/// How the Brushes tab lists presets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BrushesView {
    /// Rows: tip, size, stroke preview, name.
    #[default]
    List,
    /// Tip thumbnails with their sizes.
    Grid,
}

/// A rename in progress in the Brushes tab (a preset, or a group when `group` is set).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BrushesPanelState {
    /// Collapsed groups: a group's label, or for a nested group the root label and its path joined
    /// with `/` ([`crate::brushes_tab::group_view_key`]).
    pub collapsed: Vec<String>,
    /// Case-insensitive name filter.
    pub filter: String,
    /// Legacy two-state projection for serialized/control compatibility; shared picker display
    /// preferences are authoritative once present.
    pub view: BrushesView,
    pub renaming: Option<Renaming>,
}

/// Scroll-area settings for a brush preset browser.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListLayout {
    /// The scroll area's id and its height limit.
    pub id: &'static str,
    /// Maximum height available to the scrolling list.
    pub max_height: f32,
}

/// The Brushes tab's list.
pub const PANEL_LIST: ListLayout = ListLayout { id: "brush-presets", max_height: 400.0 };

/// The group key (`BrushPreset::group`) behind a panel label: ungrouped presets show as
/// [`UNGROUPED`].
/// Brush owns the identity policy: stable preset names are qualified by their current group/path,
/// while the reverse metadata retains the preview occurrence key for exact command targeting.

/// Render the right-click picker's library through the shared browser. Generic search, expansion,
/// selection, rename and brush-specific presentation state live in the namespaced `PresetUi`.
/// Occurrence keys remain separate from display names because imported libraries can contain
/// duplicate names even though engine commands still address presets by name.
/// Keep brush-specific policy identical across the docked panel and floating picker.
macro_rules! brush_browser_adapter {
    ($panel:expr, $groups:expr, $projection_keys:expr, $options:expr, $canonical_move:expr, $rekey:expr, $paint_item:expr, $paint_list_item:expr) => {
        BrowserAdapter {
            panel: $panel,
            groups: $groups,
            projection_keys: Some($projection_keys),
            options: $options,
            capabilities: BrowserCapabilities { item_drag: true, item_drop: true, group_drag: true, selection: true, activation: true },
            item_context_menu: Some(asset_browser::standard_item_context_menu),
            group_context_menu: Some(asset_browser::standard_group_context_menu),
            nested_group_context_menu: Some(asset_browser::standard_nested_group_context_menu),
            canonical_hierarchy_move: Some($canonical_move),
            rekey_hierarchy_after_move: Some($rekey),
            hierarchy_item_move: None,
            paint_group: None,
            paint_item: $paint_item,
            paint_list_item: $paint_list_item,
        }
    };
}

/// Brush-specific footer controls shared by the docked Brushes tab and floating right-click picker.
/// The shell owns their placement; this adapter only translates the three controls into browser actions.
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
        if delete.clicked() {
            actions.extend(delete_targets);
        }
        if crate::icons::button(ui, "plus", 24.0, false, tl!("Create new brush preset")).clicked() {
            actions.push(BrowserAction::New);
        }
        if crate::icons::button(ui, "folder-plus", 24.0, false, tl!("Create new group")).clicked() {
            actions.push(BrowserAction::NewGroup);
        }
        if preview_size {
            ui.add_space(8.0);
            let response = crate::widgets::slider(ui, &mut state.brush_picker.scale, 0.15..=2.0, None);
            if response.is_pointer_button_down_on() {
                egui::Popup::close_all(&response.ctx);
            }
        }
    });
    actions
}

pub(crate) fn picker_browser_with_shell(
    ui: &mut egui::Ui,
    presets: &[BrushPreset],
    hierarchy: &photocraft_engine::BrushPresetHierarchy,
    current: Option<&str>,
    preset_ui: &mut PresetUi,
    shell: crate::pickers::shell::ShellOptions<'_>,
) -> Vec<Action> {
    const PANEL: &str = "brush-picker";

    // Keep each projection paired with its source presets. Joining by display name picks the first
    // match when two groups contain a brush with the same name, painting the wrong preview.
    let group_views = brush_group_views(presets, hierarchy);
    let root_sequence = brush_root_sequence(presets, hierarchy);
    let view = preset_ui.brush_picker.clone();
    preset_ui.custom_item_sizes.insert(PANEL.into(), brush_picker_card_size(&view));
    let groups: Vec<GroupView> = group_views
        .iter()
        .map(|group| GroupView {
            name: group.view.name.clone(),
            root_only: group.view.root_only,
            items: group
                .view
                .items
                .iter()
                .map(|item| ItemView {
                    key: item.key.clone(),
                    // Keep the real identity label in the browser projection even when the
                    // adapter hides its painted text; an empty label breaks card presentation.
                    name: item.name.clone(),
                    group_path: item.group_path.clone(),
                })
                .collect(),
        })
        .collect();
    let projection_keys = brush_projection_keys(&groups, hierarchy);
    let mut hierarchy_actions = Vec::new();

    match current {
        Some(name) => {
            let matching: Vec<_> = presets.iter().filter(|preset| preset.name.eq_ignore_ascii_case(name)).collect();
            if matching.len() == 1 {
                preset_ui.set_selected(PANEL, &preview_identity(matching[0]));
            } else {
                preset_ui.clear_selected(PANEL);
            }
        }
        None => {
            preset_ui.clear_selected(PANEL);
        }
    }
    let mut paint_item = |ui: &egui::Ui, rect: egui::Rect, group_index: usize, item_index: usize, state: ItemPaintState| {
        let Some(preset) = group_views.get(group_index).and_then(|group| group.presets.get(item_index)).copied() else { return };
        paint_brush_item(ui, rect, preset, &view, state);
    };
    let options = BrowserOptions {
        canvas: egui::Rect::NOTHING,
        flush: false,
        center_grid: false,
        all_groups_open: false,
        // The brush picker is an egui Area, not a parent Menu; use a normal context popup so
        // its menu actions can extend beyond the picker without being dismissed as a submenu.
        compact_context_menus: false,
        domain_hierarchy_actions: true,
        root_item_group: None,
    };
    let browser_actions = {
        let mut rekey = |hierarchy: &PickerBrowserHierarchy, projection: &BTreeMap<PickerId, PickerNodeIdentity>, registry: &mut PickerIdRegistry| {
            rekey_brush_hierarchy(hierarchy, projection, presets, registry);
        };
        let mut canonical_move = |intent: &DropMoveIntent, projection: &BTreeMap<PickerId, PickerNodeIdentity>| {
            hierarchy_actions.push(brush_drop_intent_to_action(presets, hierarchy, projection, &root_sequence, intent)?);
            Ok(())
        };
        let mut adapter = brush_browser_adapter!(PANEL, &groups, &projection_keys, options, &mut canonical_move, &mut rekey, &mut paint_item, None);
        crate::pickers::shell::picker_shell(
            ui,
            preset_ui,
            &mut adapter,
            crate::pickers::shell::ShellOptions { root_sequence: Some(root_sequence.clone()), ..shell },
        )
    };
    apply_inline_rename_actions(preset_ui, PANEL, &browser_actions);
    let mut actions = browser_actions_to_actions(presets, browser_actions, preset_ui.browser_selection_snapshot.get(PANEL));
    actions.extend(hierarchy_actions);
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

#[cfg(test)]
mod brush_picker_presentation_tests {
    use super::{brush_item_uses_tip_only_preview, brush_picker_card_size};
    use crate::pickers::browser::asset_browser::ItemPaintState;
    use crate::pickers::state::BrushPickerView;

    #[test]
    fn brush_tip_keeps_its_base_size_when_other_previews_are_enabled() {
        let base = BrushPickerView { show_name: false, show_stroke: false, show_tip: true, scale: 1.0 };
        let with_name = BrushPickerView { show_name: true, ..base.clone() };
        let with_stroke = BrushPickerView { show_stroke: true, ..base.clone() };
        let with_both = BrushPickerView { show_name: true, show_stroke: true, ..base };

        assert_eq!(brush_picker_card_size(&with_name), egui::vec2(176.0, 40.0));
        assert_eq!(brush_picker_card_size(&with_stroke), egui::vec2(176.0, 40.0));
        assert_eq!(brush_picker_card_size(&with_both), egui::vec2(176.0, 64.0));
    }

    #[test]
    fn stroke_only_cards_do_not_reserve_space_for_a_hidden_tip() {
        let view = BrushPickerView { show_name: true, show_stroke: true, show_tip: false, scale: 1.0 };
        assert_eq!(brush_picker_card_size(&view), egui::vec2(128.0, 56.0));
    }

    #[test]
    fn name_only_cards_match_text_only_card_height() {
        let view = BrushPickerView { show_name: true, show_stroke: false, show_tip: false, scale: 1.0 };
        assert_eq!(brush_picker_card_size(&view), egui::vec2(180.0, 20.0));
    }

    #[test]
    fn recent_brush_cards_always_use_the_tip_only_preview() {
        assert!(brush_item_uses_tip_only_preview(ItemPaintState::Recent));
        assert!(brush_item_uses_tip_only_preview(ItemPaintState::RecentDragGhost));
        assert!(!brush_item_uses_tip_only_preview(ItemPaintState::Item { selected: true }));
        assert!(!brush_item_uses_tip_only_preview(ItemPaintState::DragGhost));
    }
}

/// Deepest nested group the panel draws; deeper groups show at this depth.
pub const MAX_VIEW_DEPTH: usize = photocraft_engine::brush_preset_cmds::MAX_FOLDER_DEPTH;

/// A preview slot must identify the preset occurrence, not only its display name: different groups
/// can contain brushes with the same name and different tips/settings. The address is only a
/// per-frame cache discriminator; the preview cache is rebuilt from the current borrowed library.

/// A group's content in panel order: its presets and nested groups.
pub type Node<'a> = HierarchyNode<'a, BrushPreset>;
pub type GroupTreeNode<'a> = HierarchyNode<'a, BrushPreset>;
pub type GroupNode<'a> = HierarchyGroup<'a, BrushPreset>;

/// The tree of a group's presets (`items`, in library order): each nested group appears where its
/// first preset is, and presets keep their order. Deeper paths are cut at [`MAX_VIEW_DEPTH`].
pub fn group_tree<'a>(items: &[&'a BrushPreset]) -> Vec<GroupTreeNode<'a>> {
    hierarchy_tree(items.iter().map(|p| (p.folder.as_slice(), *p)).collect(), MAX_VIEW_DEPTH)
}

/// Legacy Brushes-tab API name retained while its callers migrate to the group terminology.
pub fn folder_tree<'a>(items: &[&'a BrushPreset]) -> Vec<Node<'a>> {
    group_tree(items)
}

/// Where a preset dropped onto `target` lands: (root group, nested group path, index after the
/// dragged preset is taken out). `after` = dropped on the target's lower (list) or right (grid)
/// half.

/// What dropping one group on another does in the Brushes panel: on the header's upper edge it
/// lands before that group (reordering siblings); anywhere else it nests inside that group.
/// `None` means the drop is a no-op, such as dropping onto itself or into a descendant.
pub fn group_drop(src_group: &str, src_path: &[String], group: &str, target_path: &[String], edge: bool) -> Option<Action> {
    match hierarchy_group_drop(src_group, src_path, group, target_path, edge)? {
        HierarchyDrop::MoveGroup { group, group_path, before } => Some(Action::MoveGroup { group, folder: group_path, before, root_index: None }),
        HierarchyDrop::MoveInto { group, group_path, into, before } => {
            Some(Action::MoveInto { group, folder: group_path, into, before, before_item: None, root_index: None })
        }
    }
}

/// Legacy Brushes-tab API name; both arguments are group paths.
pub fn folder_drop(src_group: &str, src_path: &[String], group: &str, target_path: &[String], edge: bool) -> Option<Action> {
    group_drop(src_group, src_path, group, target_path, edge)
}

/// Render the Brushes tab through the shared browser. Only the brush-specific projection, preview
/// painters, and context-menu policy remain here; common hierarchy, selection, filtering, rename,
/// and drag/drop behavior are owned by `asset_browser`.
pub fn preset_list(
    ui: &mut egui::Ui,
    presets: &[BrushPreset],
    hierarchy: &photocraft_engine::BrushPresetHierarchy,
    current: Option<&str>,
    st: &mut BrushesPanelState,
    preset_ui: &mut PresetUi,
    layout: ListLayout,
) -> Vec<Action> {
    const PANEL: &str = "brushes";
    sync_brush_state_to_browser(st, preset_ui, presets, current, PANEL);
    let group_views = brush_group_views(presets, hierarchy);
    let root_sequence = brush_root_sequence(presets, hierarchy);
    let view = preset_ui.brush_picker.clone();
    preset_ui.custom_item_sizes.insert(PANEL.into(), brush_picker_card_size(&view));
    let groups: Vec<GroupView> = group_views
        .iter()
        .map(|group| GroupView {
            name: group.view.name.clone(),
            root_only: group.view.root_only,
            items: group
                .view
                .items
                .iter()
                .map(|item| ItemView { key: item.key.clone(), name: item.name.clone(), group_path: item.group_path.clone() })
                .collect(),
        })
        .collect();
    let projection_keys = brush_projection_keys(&groups, hierarchy);
    let mut hierarchy_actions = Vec::new();
    let mut paint_item = |ui: &egui::Ui, rect: egui::Rect, group_index: usize, item_index: usize, state: ItemPaintState| {
        let Some(preset) = group_views.get(group_index).and_then(|group| group.presets.get(item_index)).copied() else { return };
        paint_brush_item(ui, rect, preset, &view, state);
    };
    let mut footer = |ui: &mut egui::Ui, state: &mut PresetUi| draw_brush_footer_controls(ui, state, presets, PANEL, true);
    let browser_actions = {
        let mut rekey = |hierarchy: &PickerBrowserHierarchy, projection: &BTreeMap<PickerId, PickerNodeIdentity>, registry: &mut PickerIdRegistry| {
            rekey_brush_hierarchy(hierarchy, projection, presets, registry);
        };
        let mut canonical_move = |intent: &DropMoveIntent, projection: &BTreeMap<PickerId, PickerNodeIdentity>| {
            hierarchy_actions.push(brush_drop_intent_to_action(presets, hierarchy, projection, &root_sequence, intent)?);
            Ok(())
        };
        let mut adapter = brush_browser_adapter!(
            PANEL,
            &groups,
            &projection_keys,
            BrowserOptions {
                canvas: egui::Rect::NOTHING,
                flush: false,
                center_grid: false,
                all_groups_open: true,
                compact_context_menus: false,
                domain_hierarchy_actions: true,
                root_item_group: None,
            },
            &mut canonical_move,
            &mut rekey,
            &mut paint_item,
            None
        );
        crate::pickers::shell::picker_shell(
            ui,
            preset_ui,
            &mut adapter,
            crate::pickers::shell::ShellOptions {
                custom_footer: Some((&mut footer, 28.0)),
                root_sequence: Some(root_sequence.clone()),
                max_total_height: Some(layout.max_height),
                ..Default::default()
            },
        )
    };
    apply_inline_rename_actions(preset_ui, PANEL, &browser_actions);
    sync_browser_state_to_brush_state(st, preset_ui, presets, PANEL);
    let mut actions = browser_actions_to_actions(presets, browser_actions, preset_ui.browser_selection_snapshot.get(PANEL));
    actions.extend(hierarchy_actions);
    actions
}

fn apply_inline_rename_actions(state: &mut PresetUi, panel: &str, actions: &[BrowserAction]) {
    for action in actions {
        match action {
            BrowserAction::BeginInlineRenameItem(group, item) => {
                begin_item_rename(state, panel, &[(group.clone(), item.clone())]);
            }
            BrowserAction::CancelInlineRenameItem => {
                state.item_renaming = None;
                state.item_rename_queue.clear();
                state.renaming = None;
            }
            BrowserAction::RenameItem { .. } | BrowserAction::Rename(_, _) => {
                state.renaming = state.item_renaming.as_ref().map(|rename| (panel.to_string(), rename.key.clone(), rename.text.clone()));
            }
            _ => {}
        }
    }
}

/// Bridge the Brushes panel's persisted public state to the shared transient browser state. The
/// legacy JSON shape stays stable while the browser owns the live interaction model.
fn sync_brush_state_to_browser(st: &BrushesPanelState, browser: &mut PresetUi, presets: &[BrushPreset], current: Option<&str>, panel: &str) {
    browser.search.insert(panel.to_string(), st.filter.clone());
    if !browser.display_modes.contains_key(panel) {
        browser.sizes.insert(panel.to_string(), if st.view == BrushesView::List { asset_browser::MIN_THUMB } else { 40.0 });
    }
    browser.flatten_group_nesting(panel);
    browser.toggled.retain(|key| !key.starts_with(&format!("{panel}/")));
    let group_labels: Vec<String> = grouped_presets(presets).into_iter().map(|(label, _)| label).collect();
    browser.toggled.extend(st.collapsed.iter().map(|key| {
        let key = if group_labels.iter().any(|label| label == key) { group_view_key(key, &[]) } else { key.clone() };
        format!("{panel}/{key}")
    }));

    match current {
        Some(name) => {
            let mut matching = presets.iter().filter(|preset| preset.name.eq_ignore_ascii_case(name));
            if let Some(preset) = matching.next().filter(|_| matching.next().is_none()) {
                browser.set_selected(panel, &preview_identity(preset));
            } else {
                browser.clear_selected(panel);
            }
        }
        None => {
            browser.clear_selected(panel);
        }
    }

    let has_browser_rename =
        browser.renaming.as_ref().is_some_and(|rename| rename.0 == panel) || browser.group_renaming.as_ref().is_some_and(|rename| rename.panel == panel);
    if !has_browser_rename && let Some(rename) = &st.renaming {
        let group_label = group_label_for_key(presets, &rename.name).unwrap_or_else(|| rename.name.clone());
        if rename.group && !rename.folder.is_empty() {
            browser.renaming = None;
            browser.group_renaming =
                Some(NestedGroupRenameState { panel: panel.to_string(), group: group_label, group_path: rename.folder.clone(), text: rename.text.clone() });
        } else {
            let key = if rename.group {
                format!("group:{group_label}")
            } else {
                presets.iter().find(|preset| preset.name.eq_ignore_ascii_case(&rename.name)).map_or_else(|| rename.name.clone(), preview_identity)
            };
            browser.renaming = Some((panel.to_string(), key, rename.text.clone()));
            browser.group_renaming = None;
        }
    }
}

/// Copy shared expansion and rename edits back into the Brushes panel's serialized state.
fn sync_browser_state_to_brush_state(st: &mut BrushesPanelState, browser: &PresetUi, presets: &[BrushPreset], panel: &str) {
    let group_labels: Vec<String> = grouped_presets(presets).into_iter().map(|(label, _)| label).collect();
    let expanded_keys: Vec<String> = browser
        .toggled
        .iter()
        .filter_map(|key| {
            let key = key.strip_prefix(&format!("{panel}/"))?;
            Some(group_labels.iter().find(|label| group_view_key(label, &[]) == key).cloned().unwrap_or_else(|| key.to_string()))
        })
        .collect();
    let mut collapsed: Vec<String> = st.collapsed.iter().filter(|key| expanded_keys.contains(key)).cloned().collect();
    for key in expanded_keys {
        if !collapsed.contains(&key) {
            collapsed.push(key);
        }
    }
    st.collapsed = collapsed;
    st.filter = browser.search.get(panel).cloned().unwrap_or_default();

    st.renaming = if let Some(rename) = browser.group_renaming.as_ref().filter(|rename| rename.panel == panel) {
        Some(Renaming { group: true, name: group_key(presets, &rename.group), folder: rename.group_path.clone(), text: rename.text.clone() })
    } else if let Some((_, key, text)) = browser.renaming.as_ref().filter(|rename| rename.0 == panel) {
        if let Some(group) = key.strip_prefix("group:") {
            Some(Renaming { group: true, name: group_key(presets, group), folder: Vec::new(), text: text.clone() })
        } else {
            Some(Renaming { group: false, name: occurrence_name(presets, key).unwrap_or_else(|| key.clone()), folder: Vec::new(), text: text.clone() })
        }
    } else {
        None
    };
}

#[cfg(test)]
mod brush_hierarchy_identity_tests {
    use super::super::brush_engine_adapter::brush_item_identity_key;
    use super::{draw_brush_footer_controls, preview_identity, rekey_brush_hierarchy};
    use crate::PhotocraftApp;
    use crate::pickers::browser::asset_browser::BrowserAction;
    use crate::pickers::browser::identity::{PickerIdentityKey, PickerNodeIdentity};
    use crate::pickers::browser::path_identity_model::{HierarchyNode, PickerBrowserHierarchy, PickerIdRegistry};
    use crate::pickers::browser::selection::{BrowserSelection, SelectionTarget};
    use crate::pickers::state::PresetUi;
    use egui_kittest::kittest::NodeT;
    use photocraft_engine::paint::BrushPreset;
    use std::collections::BTreeMap;

    #[test]
    fn moving_a_root_group_into_another_group_rekeys_it_as_a_folder_and_keeps_its_cards() {
        let source_group = "Source";
        let target_group = "Target";
        let preset = BrushPreset {
            name: "Brush A".into(),
            brush: Default::default(),
            builtin: false,
            group: target_group.into(),
            folder: vec![source_group.into()],
            captures: Default::default(),
        };
        let item_key = preview_identity(&preset);
        let mut registry = PickerIdRegistry::with_scope(7);
        let target_id = registry.id_for(PickerIdentityKey::from_parts("brush-group", [target_group]));
        let group_id = registry.id_for(PickerIdentityKey::from_parts("brush-group", [source_group]));
        let item_id = registry.id_for(brush_item_identity_key("Brush A", source_group, &[]));
        // This is the post-drop browser tree: Source is now a child folder of Target.
        let hierarchy = PickerBrowserHierarchy::new(vec![HierarchyNode::group(
            target_id.clone(),
            vec![HierarchyNode::group(group_id.clone(), vec![HierarchyNode::item(item_id.clone())])],
        )])
        .unwrap();
        let identities = BTreeMap::from([
            (target_id, PickerNodeIdentity::Group(target_group.into())),
            (group_id.clone(), PickerNodeIdentity::Group(source_group.into())),
            (item_id.clone(), PickerNodeIdentity::Item { group: source_group.into(), key: item_key }),
        ]);

        rekey_brush_hierarchy(&hierarchy, &identities, std::slice::from_ref(&preset), &mut registry);

        let folder_id = registry.id_for(PickerIdentityKey::from_parts("brush-folder", [target_group, source_group]));
        let moved_item_id = registry.id_for(brush_item_identity_key("Brush A", target_group, &[source_group.into()]));
        assert_eq!(folder_id, group_id, "the moved group's runtime identity must survive its root-to-folder transition");
        assert_eq!(moved_item_id, item_id, "the moved card's runtime identity must survive its owner/path transition");

        assert_eq!(preset.group, target_group);
    }

    #[test]
    fn brush_footer_delete_includes_selected_groups() {
        use egui::vec2;
        use egui_kittest::{Harness, kittest::Queryable};

        let mut preset_ui = PresetUi::default();
        preset_ui
            .browser_selection_snapshot
            .insert("brushes".into(), BrowserSelection { targets: vec![SelectionTarget::Group("Basics".into())], contains_group: true, ..Default::default() });
        let mut harness = Harness::builder().with_size(vec2(420.0, 100.0)).build_ui_state(
            |ui, state: &mut (PresetUi, Vec<BrowserAction>)| {
                let actions = draw_brush_footer_controls(ui, &mut state.0, &[], "brushes", false);
                if !actions.is_empty() {
                    state.1 = actions;
                }
            },
            (preset_ui, Vec::new()),
        );
        crate::PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(2);
        harness.get_by_label("Delete").click();
        harness.run_steps(2);

        assert!(matches!(harness.state().1.as_slice(), [BrowserAction::DeleteGroup(group)] if group == "Basics"));
    }

    #[test]
    fn footer_delete_confirms_and_deletes_a_selected_empty_brush_group() {
        use crate::brushes_tab;
        use egui::vec2;
        use egui_kittest::{Harness, kittest::Queryable};

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.tools.presets.clear();
        app.session.tools.preset_hierarchy = photocraft_engine::BrushPresetHierarchy {
            groups: vec!["Basics".into()],
            folders: Vec::new(),
            root_order: vec![photocraft_engine::BrushPresetRootEntry::Group("Basics".into())],
        };
        let mut harness = Harness::builder().with_size(vec2(520.0, 700.0)).build_ui_state(|ui, app: &mut PhotocraftApp| brushes_tab::show(app, ui), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(3);

        harness.get_by_label("Basics").click();
        harness.run_steps(2);
        let delete = harness.get_by_label("Delete");
        assert!(!delete.accesskit_node().is_disabled(), "selecting a group enables the footer delete action");
        delete.click();
        harness.run_steps(2);
        assert!(harness.query_by_label("OK").is_some(), "footer deletion uses the shared confirmation dialog");
        harness.get_by_label("OK").click();
        harness.run_steps(3);

        assert!(!harness.state().session.tools.preset_hierarchy.groups.iter().any(|group| group == "Basics"));
        assert!(harness.query_by_label("Basics").is_none(), "the deleted empty group is no longer rendered");
    }

    #[test]
    fn empty_brush_library_keeps_the_shared_browser_shell_visible() {
        use crate::brushes_tab;
        use egui::vec2;
        use egui_kittest::{Harness, kittest::Queryable};

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.tools.presets.clear();
        app.session.tools.preset_hierarchy = Default::default();
        let mut harness = Harness::builder().with_size(vec2(520.0, 700.0)).build_ui_state(|ui, app: &mut PhotocraftApp| brushes_tab::show(app, ui), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(3);

        let browser = crate::pickers::browser::asset_browser::last_browser_rect(&harness.ctx, "brushes").expect("empty browser viewport is recorded");
        assert!(browser.is_positive(), "the empty browser viewport remains allocated");
        assert!(harness.query_by_label("No brush presets").is_none(), "the old standalone empty label is not rendered in place of the shell");
        assert!(harness.query_by_label("Create new group").is_some(), "the footer remains available in an empty library");
    }

    #[test]
    fn first_root_group_is_rendered_before_root_cards() {
        use crate::brushes_tab;
        use egui::vec2;
        use egui_kittest::{Harness, kittest::Queryable};

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let template = app.session.tools.presets.first().cloned().expect("default brush preset template");
        app.session.tools.presets = [("Group A card", "Group A"), ("Root card", ""), ("Group B card", "Group B")]
            .into_iter()
            .map(|(name, group)| BrushPreset { name: name.into(), group: group.into(), folder: Vec::new(), builtin: false, ..template.clone() })
            .collect();
        app.session.tools.preset_hierarchy = Default::default();
        let mut harness = Harness::builder().with_size(vec2(520.0, 700.0)).build_ui_state(|ui, app: &mut PhotocraftApp| brushes_tab::show(app, ui), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(3);

        let headers = crate::pickers::browser::asset_browser::last_group_headers(&harness.ctx, "brushes");
        assert_eq!(headers.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["Group A", "Group B"]);
        assert!(headers[0].1.top() < harness.get_by_label("Root card").rect().top(), "the first group precedes the root card in the rendered browser");
    }

    #[test]
    fn dragging_a_group_between_root_cards_preserves_its_root_position() {
        use crate::brushes_tab;
        use egui::{Modifiers, vec2};
        use egui_kittest::{Harness, kittest::Queryable};

        fn drag(h: &mut Harness<'_, PhotocraftApp>, from: egui::Pos2, to: egui::Pos2) {
            h.event(egui::Event::PointerMoved(from));
            h.run_steps(1);
            h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            h.run_steps(1);
            for step in 1..=8 {
                h.event(egui::Event::PointerMoved(from + (to - from) * (step as f32 / 8.0)));
                h.run_steps(1);
            }
            h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            h.run_steps(3);
        }

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let template = app.session.tools.presets.first().cloned().expect("default brush preset template");
        app.session.tools.presets = [("Root A", ""), ("Root B", ""), ("Source A", "Source"), ("Source B", "Source")]
            .into_iter()
            .map(|(name, group)| BrushPreset { name: name.into(), group: group.into(), folder: Vec::new(), builtin: false, ..template.clone() })
            .collect();
        app.session.tools.preset_hierarchy = Default::default();
        let mut harness = Harness::builder().with_size(vec2(520.0, 900.0)).build_ui_state(|ui, app: &mut PhotocraftApp| brushes_tab::show(app, ui), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(3);

        let source = crate::pickers::browser::asset_browser::last_group_headers(&harness.ctx, "brushes")
            .into_iter()
            .find(|(name, _)| name == "Source")
            .map(|(_, rect)| rect.center())
            .expect("Source group header");
        let target_card = harness.get_by_label("Root A").rect();
        drag(&mut harness, source, egui::pos2(target_card.right() - 2.0, target_card.center().y));

        let order = harness.state().session.tools.presets.iter().map(|preset| preset.name.as_str()).collect::<Vec<_>>();
        assert_eq!(order, ["Root A", "Source A", "Source B", "Root B"]);
        let headers = crate::pickers::browser::asset_browser::last_group_headers(&harness.ctx, "brushes");
        assert!(headers.iter().any(|(name, _)| name == "Source"), "the moved group remains visible between the root cards");
    }

    #[test]
    fn dragging_an_empty_group_between_root_cards_persists_its_mixed_sibling_position() {
        use crate::brushes_tab;
        use egui::{Modifiers, vec2};
        use egui_kittest::{Harness, kittest::Queryable};

        fn drag(harness: &mut Harness<'_, PhotocraftApp>, source: egui::Pos2, target: egui::Pos2) {
            harness.event(egui::Event::PointerMoved(source));
            harness.run_steps(1);
            harness.event(egui::Event::PointerButton { pos: source, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            harness.run_steps(1);
            for step in 1..=8 {
                harness.event(egui::Event::PointerMoved(source + (target - source) * (step as f32 / 8.0)));
                harness.run_steps(1);
            }
            harness.event(egui::Event::PointerButton { pos: target, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            harness.run_steps(3);
        }

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let template = app.session.tools.presets.first().cloned().expect("default brush preset template");
        app.session.tools.presets = ["Root A", "Root B"]
            .into_iter()
            .map(|name| BrushPreset { name: name.into(), group: String::new(), folder: Vec::new(), builtin: false, ..template.clone() })
            .collect();
        app.session.tools.preset_hierarchy = photocraft_engine::BrushPresetHierarchy {
            groups: vec!["Empty".into()],
            folders: Vec::new(),
            root_order: vec![
                photocraft_engine::BrushPresetRootEntry::Preset("Root A".into()),
                photocraft_engine::BrushPresetRootEntry::Preset("Root B".into()),
                photocraft_engine::BrushPresetRootEntry::Group("Empty".into()),
            ],
        };
        let mut harness = Harness::builder().with_size(vec2(520.0, 900.0)).build_ui_state(|ui, app: &mut PhotocraftApp| brushes_tab::show(app, ui), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(3);

        let source = crate::pickers::browser::asset_browser::last_group_headers(&harness.ctx, "brushes")
            .into_iter()
            .find(|(name, _)| name == "Empty")
            .map(|(_, rect)| rect.center())
            .expect("empty group header");
        let first = harness.get_by_label("Root A").rect();
        let second = harness.get_by_label("Root B").rect();
        let target = egui::pos2((first.right() + second.left()) * 0.5, first.center().y);
        drag(&mut harness, source, target);

        assert_eq!(
            harness.state().session.tools.preset_hierarchy.root_order,
            [
                photocraft_engine::BrushPresetRootEntry::Preset("Root A".into()),
                photocraft_engine::BrushPresetRootEntry::Group("Empty".into()),
                photocraft_engine::BrushPresetRootEntry::Preset("Root B".into()),
            ]
        );
        assert!(crate::pickers::browser::asset_browser::last_group_headers(&harness.ctx, "brushes").iter().any(|(name, _)| name == "Empty"));

        let empty_header = crate::pickers::browser::asset_browser::last_group_headers(&harness.ctx, "brushes")
            .into_iter()
            .find(|(name, _)| name == "Empty")
            .map(|(_, rect)| rect.center())
            .expect("empty group header after sibling insertion");
        let first_card = harness.get_by_label("Root A").rect();
        drag(&mut harness, empty_header, egui::pos2(first_card.left() + 2.0, first_card.center().y));
        assert_eq!(
            harness.state().session.tools.preset_hierarchy.root_order.first(),
            Some(&photocraft_engine::BrushPresetRootEntry::Group("Empty".into())),
            "the empty group can be placed at the absolute start"
        );

        let empty_header = crate::pickers::browser::asset_browser::last_group_headers(&harness.ctx, "brushes")
            .into_iter()
            .find(|(name, _)| name == "Empty")
            .map(|(_, rect)| rect.center())
            .expect("empty group header after moving to start");
        let last_card = harness.get_by_label("Root B").rect();
        drag(&mut harness, empty_header, egui::pos2(last_card.right() - 2.0, last_card.center().y));
        assert_eq!(
            harness.state().session.tools.preset_hierarchy.root_order.last(),
            Some(&photocraft_engine::BrushPresetRootEntry::Group("Empty".into())),
            "the empty group can also be placed at the absolute end"
        );
    }

    #[test]
    fn root_brush_cards_can_be_dragged_to_the_absolute_start_and_end() {
        use crate::brushes_tab;
        use egui::{Modifiers, vec2};
        use egui_kittest::{Harness, kittest::Queryable};

        for (source_name, target_name, after, expected) in
            [("Root C", "Root A", false, ["Root C", "Root A", "Root B"]), ("Root A", "Root C", true, ["Root B", "Root C", "Root A"])]
        {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            let template = app.session.tools.presets.first().cloned().expect("default brush preset template");
            app.session.tools.presets = ["Root A", "Root B", "Root C"]
                .into_iter()
                .map(|name| BrushPreset { name: name.into(), group: String::new(), folder: Vec::new(), builtin: false, ..template.clone() })
                .collect();
            app.session.tools.preset_hierarchy = Default::default();
            let mut harness = Harness::builder().with_size(vec2(520.0, 900.0)).build_ui_state(|ui, app: &mut PhotocraftApp| brushes_tab::show(app, ui), app);
            PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
            harness.run_steps(3);

            let source = harness.get_by_label(source_name).rect().center();
            let target_rect = harness.get_by_label(target_name).rect();
            let target = egui::pos2(if after { target_rect.right() - 2.0 } else { target_rect.left() + 2.0 }, target_rect.center().y);
            harness.event(egui::Event::PointerMoved(source));
            harness.run_steps(1);
            harness.event(egui::Event::PointerButton { pos: source, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            harness.run_steps(1);
            for step in 1..=8 {
                harness.event(egui::Event::PointerMoved(source + (target - source) * (step as f32 / 8.0)));
                harness.run_steps(1);
            }
            harness.event(egui::Event::PointerButton { pos: target, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            harness.run_steps(3);

            let actual = harness
                .state()
                .session
                .tools
                .preset_hierarchy
                .root_order
                .iter()
                .filter_map(|entry| match entry {
                    photocraft_engine::BrushPresetRootEntry::Preset(name) => Some(name.as_str()),
                    photocraft_engine::BrushPresetRootEntry::Group(_) => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "source={source_name}, target={target_name}, after={after}");
        }
    }

    #[test]
    fn dragging_a_group_between_cards_keeps_the_group_and_its_cards_visible() {
        use crate::brushes_tab;
        use egui::{Modifiers, vec2};
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;

        fn drag(h: &mut Harness<'_, PhotocraftApp>, from: egui::Pos2, to: egui::Pos2) {
            h.event(egui::Event::PointerMoved(from));
            h.run_steps(1);
            h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            h.run_steps(1);
            for step in 1..=8 {
                h.event(egui::Event::PointerMoved(from + (to - from) * (step as f32 / 8.0)));
                h.run_steps(1);
            }
            h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            h.run_steps(3);
        }

        for (after, expected_order) in [(false, ["Target 1", "Source 1", "Source 2", "Target 2"]), (true, ["Target 1", "Target 2", "Source 1", "Source 2"])] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            let Some(template) = app.session.tools.presets.first().cloned() else { panic!("default brush library is populated") };
            app.session.tools.presets = [("Target 1", "Target"), ("Target 2", "Target"), ("Source 1", "Source"), ("Source 2", "Source")]
                .into_iter()
                .map(|(name, group)| photocraft_engine::paint::BrushPreset {
                    name: name.into(),
                    group: group.into(),
                    folder: Vec::new(),
                    builtin: false,
                    ..template.clone()
                })
                .collect();

            let mut h = Harness::builder().with_size(vec2(520.0, 900.0)).build_ui_state(
                |ui, app: &mut PhotocraftApp| {
                    if ui.ctx().fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("semibold".into()))) {
                        brushes_tab::show(app, ui);
                    }
                },
                app,
            );
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
            h.run_steps(3);

            let source_header = crate::pickers::browser::asset_browser::last_group_headers(&h.ctx, "brushes")
                .into_iter()
                .find(|(name, _)| name == "Source")
                .map(|(_, rect)| rect)
                .expect("source group header is rendered");
            let target_card = h.get_by_label("Target 2").rect();
            let x = if after { target_card.right() - 2.0 } else { target_card.left() + 2.0 };
            let target = egui::pos2(x, target_card.center().y);
            drag(&mut h, source_header.center(), target);

            let presets = &h.state().session.tools.presets;
            for name in ["Source 1", "Source 2"] {
                let preset = presets.iter().find(|preset| preset.name == name).expect("the moved brush remains in the library");
                assert_eq!(preset.group, "Target", "after={after}");
                assert_eq!(preset.folder, ["Source"], "after={after}");
            }
            let actual_order: Vec<_> = presets.iter().map(|preset| preset.name.as_str()).collect();
            assert_eq!(actual_order, expected_order, "group insertion order, after={after}");
            // Nested folders are not included in the root-only group-header geometry helper.
            let _source_folder = h.get_by_label("Source").rect();
            let _source_one = h.get_by_label("Source 1").rect();
            let _source_two = h.get_by_label("Source 2").rect();
        }
    }
}
