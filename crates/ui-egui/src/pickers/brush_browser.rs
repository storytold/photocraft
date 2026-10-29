//! Shared brush preset browser used by the Brushes tab and the right-click Brush Picker.

use egui::{Color32, RichText, pos2, vec2};
use photocraft_engine::paint::BrushPreset;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::brush_preview;
use crate::pickers::asset_browser::{self, BrowserAction, BrowserAdapter, BrowserCapabilities, BrowserOptions, GroupView, ItemPaintState, ItemView};
use crate::pickers::asset_browser_hierarchy::{HierarchyFolder, HierarchyNode, hierarchy_tree};
pub(crate) use crate::pickers::asset_browser_interaction::AssetDrag as BrushDrag;
use crate::pickers::asset_browser_interaction::{ContextMenuAction, HierarchyDrop, draw_group_context_menu, draw_item_context_menu, hierarchy_folder_drop};
use crate::pickers::picker_state::{FolderRenameState, PresetUi};
use crate::theme::Tokens;

/// Run a brush command, reporting failures in the status bar and returning whether it succeeded.
pub(crate) fn run_or_status(app: &mut PhotocraftApp, id: &str, p: Value) -> bool {
    match app.run(id, p) {
        Ok(_) => true,
        Err(e) => {
            app.ui.status = e;
            false
        }
    }
}

/// Clear the compact Texture picker selection when the active brush changes.
pub(crate) fn clear_texture_pattern_selection(app: &mut PhotocraftApp) {
    app.ui.presets_ui.selected.remove("brush-texture-patterns");
}

pub(crate) fn full_uv() -> egui::Rect {
    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0))
}

/// Group label for presets saved without a group.
pub const UNGROUPED: &str = "My Brushes";

/// Preset indices in panel order: groups in order of first appearance.
pub fn grouped_presets(presets: &[BrushPreset]) -> Vec<(String, Vec<usize>)> {
    let mut out: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, p) in presets.iter().enumerate() {
        let g = if p.group.is_empty() { UNGROUPED } else { p.group.as_str() };
        match out.iter_mut().find(|(n, _)| n == g) {
            Some((_, v)) => v.push(i),
            None => out.push((g.to_string(), vec![i])),
        }
    }
    out
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
pub struct Renaming {
    pub group: bool,
    /// The preset (or group) being renamed.
    pub name: String,
    /// With `group`: the nested folder of group `name` being renamed (empty = the group itself).
    pub folder: Vec<String>,
    /// The text typed so far.
    pub text: String,
}

/// Brushes panel view state (serde, so the control channel can read and drive it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BrushesPanelState {
    /// Collapsed groups and folders: a group's label, or for a nested folder the group's label and
    /// the folder path joined with `/` ([`crate::brushes_tab::folder_view_key`]).
    pub collapsed: Vec<String>,
    /// Case-insensitive name filter.
    pub filter: String,
    pub view: BrushesView,
    pub renaming: Option<Renaming>,
    /// The Brush Preset picker's cards: show the preset's name. At least one of the three parts
    /// is always on, and the picker's cards never go empty (see [`crate::pickers::brush_picker`]).
    pub show_name: bool,
    /// The Brush Preset picker's cards: show the stroke preview.
    pub show_stroke: bool,
    /// The Brush Preset picker's cards: show the tip thumbnail.
    pub show_tip: bool,
    /// The Brush Preset picker's cards: their size scale, from the picker's footer slider
    /// (`crate::pickers::brush_picker::body`). 1 is the standard size; the cards' width, height and
    /// insides multiply by it (`crate::brushes_tab`) — the height down to a 0.50 floor, the
    /// width all the way — the text holds its 1.0 size, and at or below the compact threshold
    /// the tips drop the size number under them and take their cell whole.
    pub scale: f32,
}

impl Default for BrushesPanelState {
    fn default() -> Self {
        Self {
            collapsed: Vec::new(),
            filter: String::new(),
            view: BrushesView::default(),
            renaming: None,
            show_name: true,
            show_stroke: true,
            show_tip: true,
            scale: 1.0,
        }
    }
}

/// What a preset list does after drawing (the presets are borrowed while it draws).
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// A click: make the preset current.
    Select(String),
    /// A double-click: make the preset current and dismiss the picker showing it (Photoshop).
    Choose(String),
    /// View only: open or close a group or folder (by its [`folder_view_key`]; [`preset_list`]
    /// applies it to its view state).
    ToggleGroup(String),
    /// Report an operation that cannot be represented safely by the name-based engine API.
    Status(String),
    Move {
        name: String,
        group: String,
        /// The nested folder in `group` (`None`: the command's default).
        folder: Option<Vec<String>>,
        index: Option<usize>,
    },
    /// Move a preset before a sibling in a specific nested folder.
    MoveBefore {
        name: String,
        group: String,
        folder: Option<Vec<String>>,
        before: String,
    },
    /// Move a group (`folder` empty) among the groups, or a nested folder among its siblings;
    /// `before` is a group, or a sibling folder's name.
    MoveGroup {
        group: String,
        folder: Vec<String>,
        before: Option<String>,
    },
    /// Move a group (`folder` empty) or nested folder into another folder: `into` is (group,
    /// folder path, `[]` = directly in the group), or `None` for the top level (it becomes a
    /// group). `before` is a folder (or top-level group) there to land before.
    MoveInto {
        group: String,
        folder: Vec<String>,
        into: Option<(String, Vec<String>)>,
        before: Option<String>,
    },
    /// View only: show the rename bar for a preset or group ([`preset_list`] applies it).
    BeginRename(Renaming),
    /// Commit a rename typed in the rename bar.
    Rename(Renaming),
    Delete(String),
    /// Delete a group (`folder` empty) or one of its nested folders, with their presets.
    DeleteGroup {
        group: String,
        folder: Vec<String>,
    },
}

/// How a preset list is laid out: the Brushes tab's, or the Brush Preset picker's (narrower,
/// denser).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListLayout {
    /// The scroll area's id and its height limit.
    pub id: &'static str,
    /// Maximum height available to the scrolling list.
    pub max_height: f32,
    /// Width of a grid cell (its height is 6 points more) and the grid's left indent.
    pub cell: f32,
    /// Horizontal inset applied to the grid and nested-folder contents.
    pub indent: f32,
    /// The Brush Preset picker's cards: a tip cell and a stroke cell on top and the name across
    /// the bottom (or, with only the tip and the name on, in the stroke's cell). Replaces the
    /// list/grid choice in the picker.
    pub cards: bool,
}

/// The Brushes tab's list.
pub const PANEL_LIST: ListLayout = ListLayout { id: "brush-presets", max_height: 400.0, cell: 52.0, indent: 20.0, cards: false };
/// Card scale shared with the right-click picker's footer slider.
pub(crate) const SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.15..=2.0;

/// The group key (`BrushPreset::group`) behind a panel label: ungrouped presets show as
/// [`UNGROUPED`].
pub fn group_key(presets: &[BrushPreset], label: &str) -> String {
    if label == UNGROUPED && !presets.iter().any(|p| p.group == UNGROUPED) { String::new() } else { label.to_string() }
}

/// A shared browser projection paired with its source presets, so specialised brush rendering can
/// retain the original item association instead of joining the projection back by name.
struct BrushGroupView<'a> {
    view: GroupView,
    presets: Vec<&'a BrushPreset>,
}

fn brush_group_views(presets: &[BrushPreset]) -> Vec<BrushGroupView<'_>> {
    grouped_presets(presets)
        .into_iter()
        .map(|(name, indices)| {
            let presets: Vec<&BrushPreset> = indices.into_iter().filter_map(|i| presets.get(i)).collect();
            let items =
                presets.iter().map(|preset| ItemView { key: preview_identity(preset), name: preset.name.clone(), folder: preset.folder.clone() }).collect();
            BrushGroupView { view: GroupView { name, items }, presets }
        })
        .collect()
}

/// Project the brush library into the shared asset-browser model while retaining stable group/item
/// ordering, identity keys, and nested folder paths.
#[cfg(test)]
pub(crate) fn browser_group_views(presets: &[BrushPreset]) -> Vec<GroupView> {
    brush_group_views(presets).into_iter().map(|group| group.view).collect()
}

/// Render the right-click picker's library through the shared browser. Generic search, expansion,
/// selection and rename state live in the namespaced `PresetUi`. The separate `BrushesPanelState`
/// passed here supplies only the picker's brush-specific card parts and scale; its filter, rename,
/// and collapse fields belong to the Brushes tab and are deliberately not bridged into picker state.
/// Occurrence keys remain separate from display names because imported libraries can contain
/// duplicate names even though engine commands still address presets by name.
pub(crate) fn picker_browser(
    ui: &mut egui::Ui,
    presets: &[BrushPreset],
    current: Option<&str>,
    brush_state: &mut BrushesPanelState,
    preset_ui: &mut PresetUi,
    allocated_height: f32,
) -> Vec<Action> {
    const PANEL: &str = "brush-picker";

    // Keep each projection paired with its source presets. Joining by display name picks the first
    // match when two groups contain a brush with the same name, painting the wrong preview.
    let group_views = brush_group_views(presets);
    let groups: Vec<GroupView> = group_views
        .iter()
        .map(|group| GroupView {
            name: group.view.name.clone(),
            items: group.view.items.iter().map(|item| ItemView { key: item.key.clone(), name: item.name.clone(), folder: item.folder.clone() }).collect(),
        })
        .collect();

    preset_ui.sizes.insert(PANEL.to_string(), 40.0);
    match current {
        Some(name) => {
            let matching: Vec<_> = presets.iter().filter(|preset| preset.name.eq_ignore_ascii_case(name)).collect();
            if matching.len() == 1 {
                preset_ui.selected.insert(PANEL.to_string(), preview_identity(matching[0]));
            } else {
                preset_ui.selected.remove(PANEL);
            }
        }
        None => {
            preset_ui.selected.remove(PANEL);
        }
    }
    // The footer slider is bounded by the browser's actual width. Otherwise a large scale makes
    // a single card wider than the scroll area's clip region, and the item can spill into the
    // picker frame. Keep the same cap in the slider so it stops at the container edge.
    let scale_max = max_picker_card_scale(ui.available_width(), show_parts(brush_state));
    let scale = brush_state.scale.clamp(*SCALE_RANGE.start(), scale_max);
    brush_state.scale = scale;
    let scale_h = scale.max(CARD_MIN_H_SCALE);
    let show = (brush_state.show_name, brush_state.show_stroke, brush_state.show_tip);
    let (card_w, card_h) = if tip_only(show) {
        (CARD_TIP_ONLY_W * scale, CARD_TIP_ONLY_H * scale_h)
    } else if tip_and_name(show) {
        (CARD_TIP_NAME_W * scale, CARD_TIP_NAME_H * scale_h)
    } else {
        (CARD_MAX_W * scale, (CARD_MAX_H - if brush_state.show_name { 0.0 } else { CARD_NAME_H }) * scale_h)
    };

    let mut paint_item = |ui: &egui::Ui, rect: egui::Rect, group_index: usize, item_index: usize, state: ItemPaintState| {
        let Some(preset) = group_views.get(group_index).and_then(|group| group.presets.get(item_index)).copied() else { return };
        let selected = matches!(state, ItemPaintState::Item { selected: true });
        paint_picker_card(ui, rect, preset, selected, false, show, scale);
    };
    let mut paint_list_item = |ui: &egui::Ui, rect: egui::Rect, group_index: usize, item_index: usize, selected: bool| {
        let Some(preset) = group_views.get(group_index).and_then(|group| group.presets.get(item_index)).copied() else { return };
        paint_brush_list_row(ui, rect, preset, selected);
    };
    let mut adapter = BrowserAdapter {
        panel: PANEL,
        groups: &groups,
        options: BrowserOptions {
            canvas: egui::Rect::NOTHING,
            flush: false,
            center_grid: tip_only(show),
            grid_indent: 8.0,
            all_groups_open: true,
            // The brush picker is an egui Area, not a parent Menu; use a normal context popup so
            // its menu actions can extend beyond the picker without being dismissed as a submenu.
            compact_context_menus: false,
            item_size: Some(vec2(card_w, card_h)),
            list_item_height: Some(44.0),
            domain_hierarchy_actions: true,
        },
        capabilities: BrowserCapabilities { item_drag: true, item_drop: true, group_drag: true, selection: true, activation: true },
        item_context_menu: Some(brush_item_context_menu),
        group_context_menu: Some(brush_group_context_menu),
        folder_context_menu: Some(brush_folder_context_menu),
        paint_group: None,
        paint_item: &mut paint_item,
        paint_list_item: Some(&mut paint_list_item),
    };
    let browser_actions = asset_browser::browser(ui, preset_ui, &mut adapter, allocated_height);
    paint_brush_hierarchy_drag_label(ui, PANEL, egui::Id::new(("brush-drag-label", PANEL)));

    browser_actions_to_actions(presets, browser_actions)
}

/// Translate the shared browser's asset-neutral events into brush preset commands.
fn browser_actions_to_actions(presets: &[BrushPreset], browser_actions: Vec<BrowserAction>) -> Vec<Action> {
    browser_actions
        .into_iter()
        .filter_map(|action| match action {
            BrowserAction::Select(key) => occurrence_name(presets, &key).map(Action::Select),
            BrowserAction::Activate(key) => occurrence_name(presets, &key).map(Action::Choose),
            BrowserAction::Rename(key, text) => {
                occurrence_name(presets, &key).map(|name| Action::Rename(Renaming { group: false, name, folder: Vec::new(), text }))
            }
            BrowserAction::Delete(key) => occurrence_name(presets, &key).map(Action::Delete),
            BrowserAction::Move(key, group) => {
                occurrence_name(presets, &key).map(|name| Action::Move { name, group: group_key(presets, &group), folder: Some(Vec::new()), index: None })
            }
            BrowserAction::MoveItem(key, group, index) => {
                occurrence_name(presets, &key).map(|name| Action::Move { name, group: group_key(presets, &group), folder: Some(Vec::new()), index })
            }
            BrowserAction::MoveItemToFolder { key, group, folder, before: Some(before) } => occurrence_name(presets, &key)
                .zip(occurrence_name(presets, &before))
                .map(|(name, before)| Action::MoveBefore { name, group: group_key(presets, &group), folder: Some(folder), before }),
            BrowserAction::MoveItemToFolder { key, group, folder, before: None } => {
                occurrence_name(presets, &key).map(|name| Action::Move { name, group: group_key(presets, &group), folder: Some(folder), index: None })
            }
            BrowserAction::RenameGroup(group, text) => {
                Some(Action::Rename(Renaming { group: true, name: group_key(presets, &group), folder: Vec::new(), text }))
            }
            BrowserAction::DeleteGroup(group) => Some(Action::DeleteGroup { group: group_key(presets, &group), folder: Vec::new() }),
            BrowserAction::RenameFolder { group, folder, text } => {
                Some(Action::Rename(Renaming { group: true, name: group_key(presets, &group), folder, text }))
            }
            BrowserAction::DeleteFolder { group, folder } => Some(Action::DeleteGroup { group: group_key(presets, &group), folder }),
            BrowserAction::MoveGroup { group, folder, before } => Some(Action::MoveGroup {
                group: group_key(presets, &group),
                before: if folder.is_empty() { before.map(|name| group_key(presets, &name)) } else { before },
                folder,
            }),
            BrowserAction::MoveInto { group, folder, into, before } => {
                let before_is_group = into.is_none();
                Some(Action::MoveInto {
                    group: group_key(presets, &group),
                    folder,
                    into: into.map(|(target, path)| (group_key(presets, &target), path)),
                    before: if before_is_group { before.map(|name| group_key(presets, &name)) } else { before },
                })
            }
            BrowserAction::Drop(_, _) | BrowserAction::NewGroup | BrowserAction::New => None,
        })
        .collect()
}

fn brush_item_context_menu(
    ui: &mut egui::Ui,
    st: &mut PresetUi,
    panel: &str,
    item: &ItemView,
    _group: &GroupView,
    _group_names: &[String],
    actions: &mut Vec<BrowserAction>,
) {
    match draw_item_context_menu(ui, tl!("Rename Brush…"), tl!("Delete Brush"), None) {
        Some(ContextMenuAction::Rename) => {
            if st.folder_renaming.as_ref().is_some_and(|rename| rename.panel == panel) {
                st.folder_renaming = None;
            }
            st.renaming = Some((panel.to_string(), item.key.clone(), item.name.clone()));
        }
        Some(ContextMenuAction::Delete) => actions.push(BrowserAction::Delete(item.key.clone())),
        Some(ContextMenuAction::MoveTo(_)) | None => {}
    }
}

fn brush_group_context_menu(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str, group: &GroupView, actions: &mut Vec<BrowserAction>) {
    match draw_group_context_menu(ui, tl!("Rename Group…"), tl!("Delete Group")) {
        Some(ContextMenuAction::Rename) => {
            if st.folder_renaming.as_ref().is_some_and(|rename| rename.panel == panel) {
                st.folder_renaming = None;
            }
            st.renaming = Some((panel.to_string(), format!("group:{}", group.name), group.name.clone()));
        }
        Some(ContextMenuAction::Delete) => actions.push(BrowserAction::DeleteGroup(group.name.clone())),
        Some(ContextMenuAction::MoveTo(_)) | None => {}
    }
}

fn brush_folder_context_menu(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str, group: &GroupView, folder: &[String], actions: &mut Vec<BrowserAction>) {
    match draw_group_context_menu(ui, tl!("Rename Group…"), tl!("Delete Group")) {
        Some(ContextMenuAction::Rename) => {
            st.renaming = None;
            st.folder_renaming = Some(FolderRenameState {
                panel: panel.to_string(),
                group: group.name.clone(),
                folder: folder.to_vec(),
                text: folder.last().cloned().unwrap_or_default(),
            });
        }
        Some(ContextMenuAction::Delete) => actions.push(BrowserAction::DeleteFolder { group: group.name.clone(), folder: folder.to_vec() }),
        Some(ContextMenuAction::MoveTo(_)) | None => {}
    }
}

/// Horizontal room reserved for the stroke preview gaps and the preset name in a list row.
const STROKE_NAME_ROOM: f32 = 140.0;

fn paint_brush_list_row(ui: &egui::Ui, rect: egui::Rect, preset: &BrushPreset, selected: bool) {
    let t = Tokens::get(ui.ctx());
    let bg = if selected { t.accent_soft } else { t.card };
    ui.painter().rect_filled(rect, 2.0, bg);
    let brush = &preset.brush;
    let identity = preview_identity(preset);
    let tip = brush_preview::tip_texture(ui.ctx(), &format!("brushes-tip:{identity}"), &brush.tip, (brush.hardness, brush.angle, brush.roundness), 36, t.text);
    let cell = egui::Rect::from_min_size(rect.left_top() + vec2(22.0, 2.0), vec2(36.0, 30.0));
    let tip_rect = egui::Rect::from_center_size(cell.center(), vec2(30.0, 30.0) * crate::brush_sections::thumb_scale(brush.size));
    ui.painter().image(tip.id(), tip_rect, full_uv(), Color32::WHITE);
    ui.painter().text(
        pos2(cell.center().x, rect.bottom() - 1.0),
        egui::Align2::CENTER_BOTTOM,
        format!("{}", brush.size.round() as i64),
        egui::FontId::proportional(9.0),
        t.text_faint,
    );
    let width = (rect.width() - (cell.right() - rect.left()) - STROKE_NAME_ROOM).clamp(60.0, 170.0).round();
    let stroke = brush_preview::stroke_texture(ui.ctx(), &format!("brushes-stroke-{width}:{identity}"), brush, width as u32, 36, t.text);
    let stroke_rect = egui::Rect::from_min_size(pos2(cell.right() + 8.0, rect.top() + 4.0), vec2(width, 36.0));
    ui.painter().image(stroke.id(), stroke_rect, full_uv(), Color32::WHITE);
    crate::layer_row_ui::label(
        ui.painter(),
        stroke_rect.right() + 12.0,
        rect.center().y,
        rect.right() - 4.0,
        &preset.name,
        egui::FontId::proportional(12.0),
        t.text_dim,
    );
}

/// Paint the brush-specific contents of a Brushes-tab grid cell. Selection and hit targets belong
/// to `asset_browser`; this callback only draws the tip and its size label.
fn paint_brush_grid_item(ui: &egui::Ui, rect: egui::Rect, preset: &BrushPreset, selected: bool) {
    let t = Tokens::get(ui.ctx());
    if selected {
        ui.painter().rect_filled(rect, 3.0, t.accent_soft);
    }
    if !ui.is_rect_visible(rect) {
        return;
    }
    let brush = &preset.brush;
    let identity = preview_identity(preset);
    let tip = brush_preview::tip_texture(ui.ctx(), &format!("brushes-tip:{identity}"), &brush.tip, (brush.hardness, brush.angle, brush.roundness), 36, t.text);
    let side = (rect.width() - 14.0).max(1.0);
    let tip_rect =
        egui::Rect::from_center_size(pos2(rect.center().x, rect.top() + side / 2.0 + 4.0), vec2(side, side) * crate::brush_sections::thumb_scale(brush.size));
    ui.painter().image(tip.id(), tip_rect, full_uv(), Color32::WHITE);
    ui.painter().text(
        pos2(rect.center().x, rect.bottom() - 7.0),
        egui::Align2::CENTER_CENTER,
        format!("{}", brush.size.round() as i64),
        egui::FontId::proportional(9.5),
        t.text_dim,
    );
}

fn paint_picker_card(ui: &egui::Ui, rect: egui::Rect, preset: &BrushPreset, selected: bool, hovered: bool, show: (bool, bool, bool), scale: f32) {
    let (show_name, show_stroke, show_tip) = show;
    let tip_only = tip_only(show);
    let name_in_top = tip_and_name(show);
    let compact = scale <= CARD_COMPACT_SCALE;
    let scale_h = scale.max(CARD_MIN_H_SCALE);
    let t = Tokens::get(ui.ctx());
    // The shared browser draws the selection outline around `rect`, so the card background must
    // use that same horizontal extent rather than leaving an unoutlined inset on either side.
    let card = rect;
    ui.painter().rect_filled(card, 3.0, darker(t.card, CARD_BG_SHADE));
    if selected {
        ui.painter().rect_filled(card, 3.0, t.accent_soft);
    } else if hovered {
        ui.painter().rect_filled(card, 3.0, t.hover);
    }
    let pad = CARD_PAD * scale;
    let pad_y = CARD_PAD * scale_h;
    let name_h = if show_name && !name_in_top { CARD_NAME_H * scale_h } else { 0.0 };
    let top_h = if tip_only {
        (CARD_TIP_ONLY_H - CARD_PAD * 2.0) * scale_h
    } else if name_in_top {
        (CARD_TIP_NAME_H - CARD_PAD * 2.0) * scale_h
    } else if show_tip || show_stroke {
        (CARD_MAX_H - CARD_PAD * 2.0 - CARD_NAME_H) * scale_h
    } else {
        0.0
    };
    let inner = card.shrink2(vec2(pad, pad_y));
    let top = egui::Rect::from_min_size(inner.left_top(), vec2(inner.width(), top_h));
    let mut x = top.left();
    let brush = &preset.brush;
    let identity = preview_identity(preset);
    if show_tip {
        let tip_w = if tip_only { top.width() } else { (CARD_TIP_W * scale).min(top.width() * 0.5) };
        let cell = egui::Rect::from_min_size(top.left_top(), vec2(tip_w, top_h));
        let tip =
            brush_preview::tip_texture(ui.ctx(), &format!("brushes-tip:{identity}"), &brush.tip, (brush.hardness, brush.angle, brush.roundness), 36, t.text);
        let number =
            (!compact).then(|| ui.painter().layout_no_wrap(format!("{}", brush.size.round() as i64), egui::FontId::proportional(CARD_NUM_FONT), t.text_faint));
        let tip_rect = card_tip_rect(cell, number.as_ref().map(|g| g.size().y), brush.size, scale);
        ui.painter().image(tip.id(), tip_rect, full_uv(), Color32::WHITE);
        if let Some(galley) = number {
            ui.painter().galley(pos2(cell.center().x - galley.size().x / 2.0, cell.bottom() - 1.0 - galley.size().y), galley, t.text_faint);
        }
        x = cell.right();
    }
    if show_stroke || name_in_top {
        let cell = egui::Rect::from_min_max(pos2(x, top.top()), top.right_bottom());
        if show_stroke {
            let width = ((cell.width() - 16.0 * scale).clamp(24.0 * scale, 260.0 * scale)).min(cell.width()).max(1.0).round();
            // Card scale changes the destination rectangle, not the brush's stroke. Keep one
            // canonical preview per preset so dragging the footer slider doesn't rasterize and
            // upload a new texture for every intermediate width/height.
            let stroke = picker_stroke_texture(ui.ctx(), &identity, brush, t.text);
            let stroke_rect = egui::Rect::from_min_size(pos2(cell.left() + 8.0 * scale, cell.center().y - 18.0 * scale_h), vec2(width, 36.0 * scale_h));
            ui.painter().image(stroke.id(), stroke_rect, full_uv(), Color32::WHITE);
        } else {
            crate::layer_row_ui::label(
                ui.painter(),
                cell.left() + 8.0 * scale,
                cell.center().y,
                cell.right() - 6.0 * scale,
                &preset.name,
                egui::FontId::proportional(CARD_NAME_FONT),
                t.text_dim,
            );
        }
    }
    if name_h > 0.0 {
        let name_rect = egui::Rect::from_min_size(pos2(inner.left(), top.bottom()), vec2(inner.width(), name_h));
        crate::layer_row_ui::label(
            ui.painter(),
            name_rect.left() + 6.0 * scale,
            name_rect.center().y,
            name_rect.right() - 6.0 * scale,
            &preset.name,
            egui::FontId::proportional(CARD_NAME_FONT),
            t.text_dim,
        );
    }
}

/// Deepest nested folder the panel draws; deeper folders show at this depth.
pub const MAX_VIEW_DEPTH: usize = photocraft_engine::brush_preset_cmds::MAX_FOLDER_DEPTH;

/// A preview slot must identify the preset occurrence, not only its display name: different groups
/// can contain brushes with the same name and different tips/settings. The address is only a
/// per-frame cache discriminator; the preview cache is rebuilt from the current borrowed library.
pub(crate) fn preview_identity(preset: &BrushPreset) -> String {
    format!("{:x}@{}", std::ptr::from_ref(preset) as usize, preset.name)
}

/// Resolve a browser's opaque occurrence key back to the exact borrowed preset.
pub(crate) fn preset_by_identity<'a>(presets: &'a [BrushPreset], key: &str) -> Option<&'a BrushPreset> {
    presets.iter().find(|preset| preview_identity(preset) == key)
}

/// Whether a name-based engine command cannot identify one preset occurrence unambiguously.
pub(crate) fn is_ambiguous_preset_name(presets: &[BrushPreset], name: &str) -> bool {
    presets.iter().filter(|preset| preset.name.eq_ignore_ascii_case(name)).take(2).count() > 1
}

fn occurrence_name(presets: &[BrushPreset], key: &str) -> Option<String> {
    preset_by_identity(presets, key).map(|preset| preset.name.clone())
}

/// The collapse key of a nested folder of the group shown as `label` (`label` itself for the
/// group).
pub fn folder_view_key(label: &str, folder: &[String]) -> String {
    let label = crate::pickers::picker_state::escape_hierarchy_key_part(label);
    if folder.is_empty() {
        label
    } else {
        format!("{label}/{}", folder.iter().map(|part| crate::pickers::picker_state::escape_hierarchy_key_part(part)).collect::<Vec<_>>().join("/"))
    }
}

/// Keep collapse state attached to a group or folder when its display path is renamed.
pub(crate) fn remap_collapsed_prefix(collapsed: &mut [String], from: &str, to: &str) {
    for key in collapsed {
        if key == from {
            *key = to.to_string();
        } else if key.strip_prefix(from).is_some_and(|tail| tail.starts_with('/')) {
            key.replace_range(..from.len(), to);
        }
    }
}

pub(crate) fn remove_collapsed_prefix(collapsed: &mut Vec<String>, prefix: &str) {
    collapsed.retain(|key| key != prefix && !key.strip_prefix(prefix).is_some_and(|tail| tail.starts_with('/')));
}

fn group_label_for_key(presets: &[BrushPreset], key: &str) -> Option<String> {
    grouped_presets(presets).into_iter().find(|(label, _)| group_key(presets, label) == key).map(|(label, _)| label)
}

/// A group's content in panel order: its presets and nested folders.
pub type Node<'a> = HierarchyNode<'a, BrushPreset>;
pub type FolderNode<'a> = HierarchyFolder<'a, BrushPreset>;

/// The tree of a group's presets (`items`, in library order): each folder appears where its first
/// preset is, presets keep their order. Paths deeper than [`MAX_VIEW_DEPTH`] are cut there.
pub fn folder_tree<'a>(items: &[&'a BrushPreset]) -> Vec<Node<'a>> {
    hierarchy_tree(items.iter().map(|p| (p.folder.as_slice(), *p)).collect(), MAX_VIEW_DEPTH)
}

/// Where a preset dropped onto `target` lands: (group, folder, index in that group after the
/// dragged preset is taken out). `after` = dropped on the target's lower (list) or right (grid)
/// half.
pub fn drop_target(presets: &[BrushPreset], dragged: &str, target: &str, after: bool) -> Option<(String, Vec<String>, usize)> {
    let resolve = |key: &str| {
        preset_by_identity(presets, key).or_else(|| {
            let mut matches = presets.iter().filter(|preset| preset.name == key);
            let first = matches.next()?;
            matches.next().is_none().then_some(first)
        })
    };
    let dragged_preset = resolve(dragged)?;
    let t = resolve(target)?;
    let members: Vec<&BrushPreset> = presets.iter().filter(|p| p.group == t.group).collect();
    let tp = members.iter().position(|preset| std::ptr::eq(*preset, t))?;
    let mut idx = tp + usize::from(after);
    if let Some(dp) = members.iter().position(|preset| std::ptr::eq(*preset, dragged_preset))
        && dp < idx
    {
        idx -= 1;
    }
    Some((t.group.clone(), t.folder.clone(), idx))
}

fn set_current_preset(app: &mut PhotocraftApp, name: &str) {
    if is_ambiguous_preset_name(&app.session.tools.presets, name) {
        app.ui.status = format!("Cannot select brush `{name}` because multiple presets share that name.");
        return;
    }
    match app.run("tools.setBrush", json!({ "preset": name })) {
        Ok(_) => clear_texture_pattern_selection(app),
        Err(e) => app.ui.status = e,
    }
}

/// Turn a preset list's actions into commands.
pub fn apply(app: &mut PhotocraftApp, acts: Vec<Action>) {
    for a in acts {
        match a {
            Action::Select(name) => set_current_preset(app, &name),
            Action::Choose(name) => {
                // The double-click's first click already picked it.
                if is_ambiguous_preset_name(&app.session.tools.presets, &name) || app.session.tools.current_preset.as_deref() != Some(name.as_str()) {
                    set_current_preset(app, &name);
                }
            }
            Action::ToggleGroup(_) | Action::BeginRename(_) => {}
            Action::Status(message) => app.ui.status = message,
            Action::Move { name, group, folder, index } => {
                if is_ambiguous_preset_name(&app.session.tools.presets, &name) {
                    app.ui.status = format!("Cannot move brush `{name}` because multiple presets share that name.");
                    continue;
                }
                let mut p = json!({ "name": name, "group": group });
                if let Some(f) = folder {
                    p["folder"] = json!(f);
                }
                if let Some(i) = index {
                    p["index"] = json!(i);
                }
                run_or_status(app, "brush.presets.move", p);
            }
            Action::MoveBefore { name, group, folder, before } => {
                if is_ambiguous_preset_name(&app.session.tools.presets, &name) || is_ambiguous_preset_name(&app.session.tools.presets, &before) {
                    app.ui.status = format!("Cannot reorder brush `{name}` because the dragged brush or destination `{before}` has a duplicate name.");
                    continue;
                }
                let mut p = json!({ "name": name, "group": group, "before": before });
                if let Some(f) = folder {
                    p["folder"] = json!(f);
                }
                run_or_status(app, "brush.presets.move", p);
            }
            Action::MoveGroup { group, folder, before } => {
                let mut p = json!({ "group": group, "before": before });
                if !folder.is_empty() {
                    p["folder"] = json!(folder);
                }
                run_or_status(app, "brush.presets.moveGroup", p);
            }
            Action::MoveInto { group, folder, into, before } => {
                let into = match into {
                    Some((g, f)) => json!({ "group": g, "folder": f }),
                    None => json!({}),
                };
                let mut p = json!({ "group": group, "into": into });
                if !folder.is_empty() {
                    p["folder"] = json!(folder);
                }
                if let Some(b) = before {
                    p["before"] = json!(b);
                }
                run_or_status(app, "brush.presets.moveGroup", p);
            }
            Action::Rename(r) => {
                let text = r.text.trim().to_string();
                let old = r.folder.last().unwrap_or(&r.name);
                if !text.is_empty() && text != *old {
                    if r.group {
                        let label = group_label_for_key(&app.session.tools.presets, &r.name);
                        let mut p = json!({ "group": r.name, "newName": text });
                        if !r.folder.is_empty() {
                            p["folder"] = json!(r.folder);
                        }
                        if run_or_status(app, "brush.presets.renameGroup", p)
                            && let Some(label) = label
                        {
                            let from = folder_view_key(&label, &r.folder);
                            let to = if r.folder.is_empty() {
                                folder_view_key(&text, &[])
                            } else {
                                let mut folder = r.folder.clone();
                                if let Some(last) = folder.last_mut() {
                                    *last = text.clone();
                                }
                                folder_view_key(&label, &folder)
                            };
                            remap_collapsed_prefix(&mut app.ui.brushes_panel.collapsed, &from, &to);
                        }
                    } else {
                        if is_ambiguous_preset_name(&app.session.tools.presets, &r.name) {
                            app.ui.status = format!("Cannot rename brush `{}` because multiple presets share that name.", r.name);
                            continue;
                        }
                        run_or_status(app, "brush.presets.rename", json!({ "name": r.name, "newName": text }));
                    }
                }
            }
            Action::Delete(name) => {
                if is_ambiguous_preset_name(&app.session.tools.presets, &name) {
                    app.ui.status = format!("Cannot delete brush `{name}` because multiple presets share that name.");
                    continue;
                }
                run_or_status(app, "brush.presets.delete", json!({ "name": name }));
            }
            Action::DeleteGroup { group, folder } => {
                let label = group_label_for_key(&app.session.tools.presets, &group);
                let mut p = json!({ "group": group });
                if !folder.is_empty() {
                    p["folder"] = json!(folder);
                }
                if run_or_status(app, "brush.presets.deleteGroup", p)
                    && let Some(label) = label
                {
                    remove_collapsed_prefix(&mut app.ui.brushes_panel.collapsed, &folder_view_key(&label, &folder));
                }
            }
        }
    }
}

/// The gap between grid cells, across and down.
#[cfg(test)]
pub(crate) const GRID_GAP: f32 = 3.0;
/// The picker's cards pack tighter than the grid: their horizontal gap (the vertical one stays
/// [`GRID_GAP`]).
#[cfg(test)]
pub(crate) const CARD_GAP: f32 = 1.0;

/// The grid's columns in a list `width` points wide: how many `cell`-wide cells separated by
/// `gap` fit past the `indent`, and the left margin that centres them in the room past it. At
/// least one column.
#[cfg(test)]
pub(crate) fn grid_columns(width: f32, cell: f32, indent: f32, gap: f32) -> (f32, usize) {
    let room = if width.is_finite() { (width - indent).max(0.0) } else { 0.0 };
    let cols = if cell.is_finite() && cell > 0.0 { ((room + gap) / (cell + gap)).floor() } else { 1.0 };
    // `as` saturates (and NaN becomes 0), so this is a whole number from 1 to a sane bound.
    let cols = (cols as usize).clamp(1, 256);
    let used = cols as f32 * cell + (cols - 1) as f32 * gap;
    // Whole points keep the tiles' edges crisp.
    let left = indent + ((room - used) / 2.0).max(0.0).floor();
    (if left.is_finite() { left } else { 0.0 }, cols)
}

/// Padding, tip cell width and name row height, at scale 1; the footer slider multiplies them
/// (heights down to [`CARD_MIN_H_SCALE`]).
pub(crate) const CARD_PAD: f32 = 4.0;
pub(crate) const CARD_TIP_W: f32 = 46.0;
/// The tip thumbnail's base size at scale 1, before the footer slider and the per-preset
/// [`crate::brush_sections::thumb_scale`]: the tips read small at 28 px, so the base carries a
/// 60 % bump.
const CARD_TIP_BASE: f32 = 28.0 * 1.6;
pub(crate) const CARD_NAME_H: f32 = 20.0;
/// A full card's height at scale 1: the tip/stroke row takes what the name row leaves. The
/// footer slider multiplies it down to [`CARD_MIN_H_SCALE`].
pub(crate) const CARD_MAX_H: f32 = 70.0;
/// The widest a lone card grows at scale 1; past it the room is shared between columns.
pub(crate) const CARD_MAX_W: f32 = 300.0;
/// Tip alone: a grid of small [`CARD_TIP_ONLY_W`] × [`CARD_TIP_ONLY_H`] cells (scale 1).
pub(crate) const CARD_TIP_ONLY_W: f32 = 60.0;
pub(crate) const CARD_TIP_ONLY_H: f32 = 70.0;
/// Tip + name, no stroke: a grid of [`CARD_TIP_NAME_W`] × [`CARD_TIP_NAME_H`] cells (scale 1).
pub(crate) const CARD_TIP_NAME_W: f32 = 150.0;
pub(crate) const CARD_TIP_NAME_H: f32 = 70.0;
/// The footer scale's floor for heights and padding (the widths keep shrinking).
pub(crate) const CARD_MIN_H_SCALE: f32 = 0.60;
/// At or below this scale the tips drop their size numbers and fill their cell whole.
pub(crate) const CARD_COMPACT_SCALE: f32 = 0.70;
/// stay readable from the smallest card to the largest.
const CARD_NAME_FONT: f32 = 12.0;
pub(crate) const CARD_NUM_FONT: f32 = 11.0;
/// The cards' own fill: [`Tokens::card`] (what the picker's popup is filled with) this much
/// darker, so the cards read as tiles against it.
const CARD_BG_SHADE: f32 = 0.90;

/// Maximum footer scale that keeps a picker card within the available browser width.
pub(crate) fn max_picker_card_scale(width: f32, show: (bool, bool, bool)) -> f32 {
    let base_width = if tip_only(show) {
        CARD_TIP_ONLY_W
    } else if tip_and_name(show) {
        CARD_TIP_NAME_W
    } else {
        CARD_MAX_W
    };
    // The grid's base inset is 8 points. Keep the minimum slider scale available for narrow
    // pickers; for normal widths the upper bound is exactly where the card meets the container.
    ((width - 8.0).max(0.0) / base_width).clamp(*SCALE_RANGE.start(), *SCALE_RANGE.end())
}

/// Return the picker's canonical stroke texture. Card scale changes only the destination rectangle;
/// keeping the source dimensions fixed prevents a resize drag from rasterizing a new preview each
/// frame.
fn picker_stroke_texture(
    ctx: &egui::Context,
    identity: &str,
    brush: &photocraft_engine::BrushSettings,
    color: Color32,
) -> egui::TextureHandle {
    brush_preview::stroke_texture(ctx, &format!("brushes-stroke:{identity}"), brush, 260, 36, color)
}

fn show_parts(state: &BrushesPanelState) -> (bool, bool, bool) {
    (state.show_name, state.show_stroke, state.show_tip)
}

/// The picker's cards in a list `width` points wide, `indent` past its left edge: how wide each
/// card is and how many sit in a row. A lone card fills the room up to [`CARD_MAX_W`] × `scale`
/// (wider lists leave the rest empty); from two columns on, the cards share the room, filling it
/// fully. `scale` is the picker's footer slider ([`crate::pickers::brush_picker::body`]), already floored
/// so the widths never reach zero.
/// Is the card showing the tip alone (no name, no stroke)? Then it's a small tip cell in a grid.
fn tip_only(show: (bool, bool, bool)) -> bool {
    show == (false, false, true)
}

/// Is the card showing the tip and the name with the stroke off? Then it's a tip-and-name cell.
fn tip_and_name(show: (bool, bool, bool)) -> bool {
    show == (true, false, true)
}

/// `c` darker by `f` in gamma space, kept opaque — [`Color32::gamma_multiply`] fades the alpha
/// along with the channels, so it can't darken a fill over its own color.
fn darker(c: Color32, f: f32) -> Color32 {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied((r as f32 * f).round() as u8, (g as f32 * f).round() as u8, (b as f32 * f).round() as u8, a)
}

/// Where a card's tip thumbnail goes in its `cell`. `number_h` is the height of the size number
/// under it, `None` on compact cards (no number: the tip takes the cell whole). Otherwise the
/// number keeps a band at the bottom of the cell and the tip is centred in what's left above it,
/// at [`CARD_TIP_BASE`] × `scale` or that room's side, whichever is smaller, times the preset's
/// [`crate::brush_sections::thumb_scale`]: big brushes still read bigger than small ones, and
/// none of them reaches down over the number.
pub(crate) fn card_tip_rect(cell: egui::Rect, number_h: Option<f32>, size: f32, scale: f32) -> egui::Rect {
    let Some(number_h) = number_h else {
        return egui::Rect::from_center_size(cell.center(), egui::Vec2::splat((cell.width().min(cell.height()) - 2.0).max(1.0)));
    };
    // The number sits 1 pt off the cell's bottom, with a 1 pt gap above it.
    let room = egui::Rect::from_min_max(cell.min, pos2(cell.right(), (cell.bottom() - number_h - 2.0).max(cell.top())));
    let side = (CARD_TIP_BASE * scale).min(room.width().min(room.height())).max(0.0) * crate::brush_sections::thumb_scale(size);
    egui::Rect::from_center_size(room.center(), egui::Vec2::splat(if side.is_finite() { side.max(1.0) } else { 1.0 }))
}

/// What dropping the group or folder (`src_group`, `src_folder`; a group when `src_folder` is
/// empty) on the header of (`group`, `folder`) does, as in Photoshop's Brushes panel: on the
/// header's upper edge (`edge`) it lands before that folder, next to it (reordering when they are
/// siblings); anywhere else it goes into that folder. `None` when the drop does nothing: onto
/// itself, into one of its own folders, or into the folder it is already in.
pub fn folder_drop(src_group: &str, src_folder: &[String], group: &str, folder: &[String], edge: bool) -> Option<Action> {
    match hierarchy_folder_drop(src_group, src_folder, group, folder, edge)? {
        HierarchyDrop::MoveGroup { group, folder, before } => Some(Action::MoveGroup { group, folder, before }),
        HierarchyDrop::MoveInto { group, folder, into, before } => Some(Action::MoveInto { group, folder, into, before }),
    }
}

/// Render the Brushes tab through the shared browser. Only the brush-specific projection, preview
/// painters, and context-menu policy remain here; common hierarchy, selection, filtering, rename,
/// and drag/drop behavior are owned by `asset_browser`.
pub fn preset_list(
    ui: &mut egui::Ui,
    presets: &[BrushPreset],
    current: Option<&str>,
    st: &mut BrushesPanelState,
    preset_ui: &mut PresetUi,
    layout: ListLayout,
) -> Vec<Action> {
    const PANEL: &str = "brushes";
    if presets.is_empty() {
        ui.label(RichText::new(tl!("No brush presets")).color(Tokens::get(ui.ctx()).text_faint));
        return Vec::new();
    }

    sync_brush_state_to_browser(st, preset_ui, presets, current, PANEL);
    let group_views = brush_group_views(presets);
    let groups: Vec<GroupView> = group_views
        .iter()
        .map(|group| GroupView {
            name: group.view.name.clone(),
            items: group.view.items.iter().map(|item| ItemView { key: item.key.clone(), name: item.name.clone(), folder: item.folder.clone() }).collect(),
        })
        .collect();
    let list = st.view == BrushesView::List;
    let mut paint_item = |ui: &egui::Ui, rect: egui::Rect, group_index: usize, item_index: usize, state: ItemPaintState| {
        let Some(preset) = group_views.get(group_index).and_then(|group| group.presets.get(item_index)).copied() else { return };
        let selected = matches!(state, ItemPaintState::Item { selected: true });
        if list {
            paint_brush_list_row(ui, rect, preset, false);
        } else {
            paint_brush_grid_item(ui, rect, preset, selected);
        }
    };
    let mut paint_list_item = |ui: &egui::Ui, rect: egui::Rect, group_index: usize, item_index: usize, selected: bool| {
        if let Some(preset) = group_views.get(group_index).and_then(|group| group.presets.get(item_index)).copied() {
            paint_brush_list_row(ui, rect, preset, selected);
        }
    };
    let mut adapter = BrowserAdapter {
        panel: PANEL,
        groups: &groups,
        options: BrowserOptions {
            canvas: egui::Rect::NOTHING,
            flush: false,
            center_grid: true,
            grid_indent: layout.indent,
            all_groups_open: true,
            compact_context_menus: false,
            item_size: Some(vec2(layout.cell, layout.cell + 6.0)),
            list_item_height: Some(44.0),
            domain_hierarchy_actions: true,
        },
        capabilities: BrowserCapabilities { item_drag: true, item_drop: true, group_drag: true, selection: true, activation: true },
        item_context_menu: Some(brush_item_context_menu),
        group_context_menu: Some(brush_group_context_menu),
        folder_context_menu: Some(brush_folder_context_menu),
        paint_group: None,
        paint_item: &mut paint_item,
        paint_list_item: list.then_some(&mut paint_list_item as &mut crate::pickers::asset_browser::PaintListItem<'_>),
    };
    let browser_actions = asset_browser::browser(ui, preset_ui, &mut adapter, layout.max_height);
    sync_browser_state_to_brush_state(st, preset_ui, presets, PANEL);
    paint_brush_hierarchy_drag_label(ui, PANEL, egui::Id::new(("brush-drag-label", layout.id)));
    browser_actions_to_actions(presets, browser_actions)
}

/// Bridge the Brushes panel's persisted public state to the shared transient browser state. The
/// legacy JSON shape stays stable while the browser owns the live interaction model.
fn sync_brush_state_to_browser(st: &BrushesPanelState, browser: &mut PresetUi, presets: &[BrushPreset], current: Option<&str>, panel: &str) {
    browser.search.insert(panel.to_string(), st.filter.clone());
    browser.sizes.insert(panel.to_string(), if st.view == BrushesView::List { asset_browser::MIN_THUMB } else { 40.0 });
    browser.group_order.remove(panel);
    browser.group_parent.remove(panel);
    browser.toggled.retain(|key| !key.starts_with(&format!("{panel}/")));
    let group_labels: Vec<String> = grouped_presets(presets).into_iter().map(|(label, _)| label).collect();
    browser.toggled.extend(st.collapsed.iter().map(|key| {
        let key = if group_labels.iter().any(|label| label == key) { folder_view_key(key, &[]) } else { key.clone() };
        format!("{panel}/{key}")
    }));

    match current {
        Some(name) => {
            let mut matching = presets.iter().filter(|preset| preset.name.eq_ignore_ascii_case(name));
            if let Some(preset) = matching.next().filter(|_| matching.next().is_none()) {
                browser.selected.insert(panel.to_string(), preview_identity(preset));
            } else {
                browser.selected.remove(panel);
            }
        }
        None => {
            browser.selected.remove(panel);
        }
    }

    let has_browser_rename =
        browser.renaming.as_ref().is_some_and(|rename| rename.0 == panel) || browser.folder_renaming.as_ref().is_some_and(|rename| rename.panel == panel);
    if !has_browser_rename && let Some(rename) = &st.renaming {
        let group_label = group_label_for_key(presets, &rename.name).unwrap_or_else(|| rename.name.clone());
        if rename.group && !rename.folder.is_empty() {
            browser.renaming = None;
            browser.folder_renaming =
                Some(FolderRenameState { panel: panel.to_string(), group: group_label, folder: rename.folder.clone(), text: rename.text.clone() });
        } else {
            let key = if rename.group {
                format!("group:{group_label}")
            } else {
                presets.iter().find(|preset| preset.name.eq_ignore_ascii_case(&rename.name)).map_or_else(|| rename.name.clone(), preview_identity)
            };
            browser.renaming = Some((panel.to_string(), key, rename.text.clone()));
            browser.folder_renaming = None;
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
            Some(group_labels.iter().find(|label| folder_view_key(label, &[]) == key).cloned().unwrap_or_else(|| key.to_string()))
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

    st.renaming = if let Some(rename) = browser.folder_renaming.as_ref().filter(|rename| rename.panel == panel) {
        Some(Renaming { group: true, name: group_key(presets, &rename.group), folder: rename.folder.clone(), text: rename.text.clone() })
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

/// Keep brush group/folder labels visible while dragging hierarchy nodes. Item artwork is painted
/// by the shared browser so its ghost stays identical to the source card.
fn paint_brush_hierarchy_drag_label(ui: &egui::Ui, scope: &str, id: egui::Id) {
    let Some(drag) = egui::DragAndDrop::payload::<BrushDrag>(ui.ctx()) else { return };
    let (drag_scope, name) = match &*drag {
        BrushDrag::Group { scope, name, .. } => (scope.as_str(), if name.is_empty() { UNGROUPED.to_string() } else { name.clone() }),
        BrushDrag::Folder { scope, folder, .. } => (scope.as_str(), folder.last().cloned().unwrap_or_default()),
        BrushDrag::Preset { .. } => return,
    };
    if drag_scope != scope {
        return;
    }
    let Some(pointer) = ui.ctx().pointer_interact_pos() else { return };
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    egui::Area::new(id).order(egui::Order::Tooltip).fixed_pos(pointer + vec2(12.0, 8.0)).interactable(false).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(name));
    });
}

#[cfg(test)]
mod preview_regression_tests {
    use super::{CARD_MAX_W, CARD_TIP_NAME_W, CARD_TIP_ONLY_W, max_picker_card_scale, paint_picker_card};
    use photocraft_engine::paint::BrushPreset;

    #[test]
    fn resizing_picker_preview_reuses_the_cached_stroke_texture() {
        let ctx = egui::Context::default();
        let preset = BrushPreset {
            name: "Regression brush".into(),
            brush: Default::default(),
            builtin: false,
            group: "Test".into(),
            folder: Vec::new(),
            captures: Default::default(),
        };

        // Exercise the actual card painter over successive UI frames, rather than testing a helper
        // that could remain correct while the production call site regresses to scale-keyed slots.
        let mut renders_after_first_frame = None;
        for scale in [0.6_f32, 0.75, 1.0, 1.25, 1.5, 2.0] {
            let mut output = ctx.run_ui(Default::default(), |ui| {
                paint_picker_card(ui, ui.max_rect(), &preset, false, false, (true, true, true), scale);
            });
            output.textures_delta.clear();
            let renders = crate::brush_preview::render_count(&ctx);
            if let Some(expected) = renders_after_first_frame {
                assert_eq!(renders, expected, "resizing to scale {scale} re-rendered a preview");
            } else {
                renders_after_first_frame = Some(renders);
            }
        }
    }

    #[test]
    fn picker_card_width_never_exceeds_its_container() {
        let modes = [
            ((false, false, true), CARD_TIP_ONLY_W),
            ((true, false, true), CARD_TIP_NAME_W),
            ((true, true, true), CARD_MAX_W),
            ((false, true, true), CARD_MAX_W),
            ((true, true, false), CARD_MAX_W),
        ];

        for width in [64.0_f32, 96.0, 160.0, 240.0, 360.0, 640.0] {
            for (show, base_card_width) in modes {
                let scale = max_picker_card_scale(width, show);
                let card_width = base_card_width * scale;
                assert!(
                    card_width <= width,
                    "card width {card_width} exceeds container width {width} for mode {show:?} at scale {scale}"
                );
            }
        }
    }
}
