//! Window › Brushes: the brush presets in collapsible groups, as a list (tip, size, stroke
//! preview, name) or a grid of tips, with a size slider and a search field.
//!
//! Presets and groups are organised like Photoshop's panel: drag a preset within its group or
//! into another group or folder (onto a preset, or onto a header to append), drag a group's
//! header to reorder groups (a nested folder's header to reorder it among its sibling folders),
//! and right-click for Rename and Delete. Groups hold nested folders ([`BrushPreset::folder`],
//! e.g. an imported `.abr` file's own folders), drawn indented under their group and opened or
//! closed like groups. Every change is a `brush.presets.*` command, so it is journaled, drivable,
//! and persisted by the preset store.
//!
//! The list itself ([`preset_list`]) is shared with the Brush Preset picker (`brush_picker`), so
//! both show, pick and organise presets the same way; each keeps its own view state.

use egui::{Color32, RichText, Sense, Stroke, pos2, vec2};
use photocraft_engine::paint::{BrushPreset, MAX_BRUSH_SIZE};
use serde_json::json;

use crate::brush_panel::{
    BrushesPanelState, BrushesView, Renaming, UNGROUPED, WIDTH, commit_gesture, full_uv, grouped_presets, new_preset_name, run_or_status,
};
use crate::brush_preview;
use crate::theme::{self, Tokens};
use crate::{PhotocraftApp, icons, widgets};

/// What a drag in the Brushes tab carries.
#[derive(Clone, Debug, PartialEq)]
pub enum BrushDrag {
    Preset(String),
    Group(String),
    /// A nested folder of a group.
    Folder {
        group: String,
        folder: Vec<String>,
    },
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
    Move {
        name: String,
        group: String,
        /// The nested folder in `group` (`None`: the command's default).
        folder: Option<Vec<String>>,
        index: Option<usize>,
    },
    /// Move a group (`folder` empty) among the groups, or a nested folder among its siblings;
    /// `before` is a group, or a sibling folder's name.
    MoveGroup {
        group: String,
        folder: Vec<String>,
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
    pub max_height: f32,
    /// Width of a grid cell (its height is 6 points more) and the grid's left indent.
    pub cell: f32,
    pub indent: f32,
}

/// The Brushes tab's list.
pub const PANEL_LIST: ListLayout = ListLayout { id: "brush-presets", max_height: 400.0, cell: 52.0, indent: 20.0 };

/// The group key (`BrushPreset::group`) behind a panel label: ungrouped presets show as
/// [`UNGROUPED`].
pub fn group_key(presets: &[BrushPreset], label: &str) -> String {
    if label == UNGROUPED && !presets.iter().any(|p| p.group == UNGROUPED) { String::new() } else { label.to_string() }
}

/// Deepest nested folder the panel draws; deeper folders show at this depth.
pub const MAX_VIEW_DEPTH: usize = photocraft_engine::brush_preset_cmds::MAX_FOLDER_DEPTH;

/// The collapse key of a nested folder of the group shown as `label` (`label` itself for the
/// group).
pub fn folder_view_key(label: &str, folder: &[String]) -> String {
    if folder.is_empty() { label.to_string() } else { format!("{label}/{}", folder.join("/")) }
}

/// A group's content in panel order: its presets and nested folders.
#[derive(Debug)]
pub enum Node<'a> {
    Preset(&'a BrushPreset),
    Folder(FolderNode<'a>),
}

/// A nested folder and what it holds.
#[derive(Debug)]
pub struct FolderNode<'a> {
    /// The folder's path in its group.
    pub path: Vec<String>,
    pub children: Vec<Node<'a>>,
    /// Presets in it and below it.
    pub count: usize,
}

/// The tree of a group's presets (`items`, in library order): each folder appears where its first
/// preset is, presets keep their order. Paths deeper than [`MAX_VIEW_DEPTH`] are cut there.
pub fn folder_tree<'a>(items: &[&'a BrushPreset]) -> Vec<Node<'a>> {
    level(items.iter().map(|p| (p.folder.get(..p.folder.len().min(MAX_VIEW_DEPTH)).unwrap_or_default(), *p)).collect(), &[])
}

/// A preset with its folder path below the level being built.
type Entry<'a> = (&'a [String], &'a BrushPreset);

/// One level of [`folder_tree`]: `items` pairs each preset with its path below `prefix`. The
/// recursion is as deep as the longest path (at most [`MAX_VIEW_DEPTH`]).
fn level<'a>(items: Vec<Entry<'a>>, prefix: &[String]) -> Vec<Node<'a>> {
    enum Slot {
        Preset(usize),
        Folder(usize),
    }
    let mut slots = Vec::new();
    let mut folders: Vec<(&String, Vec<Entry<'a>>)> = Vec::new();
    for (i, (path, p)) in items.iter().enumerate() {
        match path.split_first() {
            None => slots.push(Slot::Preset(i)),
            Some((name, rest)) => match folders.iter_mut().position(|(n, _)| *n == name) {
                Some(k) => {
                    if let Some((_, v)) = folders.get_mut(k) {
                        v.push((rest, *p));
                    }
                }
                None => {
                    slots.push(Slot::Folder(folders.len()));
                    folders.push((name, vec![(rest, *p)]));
                }
            },
        }
    }
    let mut folders: Vec<Option<(&String, Vec<Entry<'a>>)>> = folders.into_iter().map(Some).collect();
    slots
        .into_iter()
        .filter_map(|s| match s {
            Slot::Preset(i) => items.get(i).map(|(_, p)| Node::Preset(p)),
            Slot::Folder(k) => {
                let (name, sub) = folders.get_mut(k)?.take()?;
                let path: Vec<String> = prefix.iter().cloned().chain(std::iter::once(name.clone())).collect();
                let count = sub.len();
                let children = level(sub, &path);
                Some(Node::Folder(FolderNode { path, children, count }))
            }
        })
        .collect()
}

/// Where a preset dropped onto `target` lands: (group, folder, index in that group after the
/// dragged preset is taken out). `after` = dropped on the target's lower (list) or right (grid)
/// half.
pub fn drop_target(presets: &[BrushPreset], dragged: &str, target: &str, after: bool) -> Option<(String, Vec<String>, usize)> {
    let t = presets.iter().find(|p| p.name == target)?;
    let members: Vec<&str> = presets.iter().filter(|p| p.group == t.group).map(|p| p.name.as_str()).collect();
    let tp = members.iter().position(|n| *n == target)?;
    let mut idx = tp + usize::from(after);
    if let Some(dp) = members.iter().position(|n| *n == dragged)
        && dp < idx
    {
        idx -= 1;
    }
    Some((t.group.clone(), t.folder.clone(), idx))
}

/// Apply the view-only actions to `st`; returns the rest, the commands for [`apply`].
fn view_actions(st: &mut BrushesPanelState, acts: Vec<Action>) -> Vec<Action> {
    acts.into_iter()
        .filter_map(|a| match a {
            Action::ToggleGroup(g) => {
                let c = &mut st.collapsed;
                match c.iter().position(|x| *x == g) {
                    Some(i) => drop(c.remove(i)),
                    None => c.push(g),
                }
                None
            }
            Action::BeginRename(r) => {
                st.renaming = Some(r);
                None
            }
            a => Some(a),
        })
        .collect()
}

/// Turn a preset list's actions into commands.
pub fn apply(app: &mut PhotocraftApp, acts: Vec<Action>) {
    for a in acts {
        match a {
            Action::Select(name) => run_or_status(app, "tools.setBrush", json!({ "preset": name })),
            Action::Choose(name) => {
                // The double-click's first click already picked it.
                if app.session.tools.current_preset.as_deref() != Some(name.as_str()) {
                    run_or_status(app, "tools.setBrush", json!({ "preset": name }));
                }
            }
            Action::ToggleGroup(_) | Action::BeginRename(_) => {}
            Action::Move { name, group, folder, index } => {
                let mut p = json!({ "name": name, "group": group });
                if let Some(f) = folder {
                    p["folder"] = json!(f);
                }
                if let Some(i) = index {
                    p["index"] = json!(i);
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
            Action::Rename(r) => {
                let text = r.text.trim().to_string();
                let old = r.folder.last().unwrap_or(&r.name);
                if !text.is_empty() && text != *old {
                    if r.group {
                        let mut p = json!({ "group": r.name, "newName": text });
                        if !r.folder.is_empty() {
                            p["folder"] = json!(r.folder);
                        }
                        run_or_status(app, "brush.presets.renameGroup", p);
                    } else {
                        run_or_status(app, "brush.presets.rename", json!({ "name": r.name, "newName": text }));
                    }
                }
            }
            Action::Delete(name) => run_or_status(app, "brush.presets.delete", json!({ "name": name })),
            Action::DeleteGroup { group, folder } => {
                let mut p = json!({ "group": group });
                if !folder.is_empty() {
                    p["folder"] = json!(folder);
                }
                run_or_status(app, "brush.presets.deleteGroup", p);
            }
        }
    }
}

/// Is the pointer on the second half of `r` (lower in a list, right in a grid)?
fn second_half(ui: &egui::Ui, r: egui::Rect, grid: bool) -> bool {
    ui.ctx().pointer_interact_pos().is_some_and(|p| if grid { p.x > r.center().x } else { p.y > r.center().y })
}

/// Drag, drop and context menu of one preset cell or row.
fn preset_interactions(ui: &egui::Ui, resp: &egui::Response, r: egui::Rect, p: &BrushPreset, presets: &[BrushPreset], grid: bool, acts: &mut Vec<Action>) {
    let t = Tokens::get(ui.ctx());
    resp.dnd_set_drag_payload(BrushDrag::Preset(p.name.clone()));
    if let Some(d) = resp.dnd_hover_payload::<BrushDrag>()
        && matches!(&*d, BrushDrag::Preset(n) if *n != p.name)
    {
        widgets::drop_line(ui, r, second_half(ui, r, grid), grid, &t);
    }
    if let Some(d) = resp.dnd_release_payload::<BrushDrag>()
        && let BrushDrag::Preset(n) = &*d
        && *n != p.name
        && let Some((group, folder, index)) = drop_target(presets, n, &p.name, second_half(ui, r, grid))
    {
        acts.push(Action::Move { name: n.clone(), group, folder: Some(folder), index: Some(index) });
    }
    if resp.double_clicked() {
        acts.push(Action::Choose(p.name.clone()));
    } else if resp.clicked() {
        acts.push(Action::Select(p.name.clone()));
    }
    resp.context_menu(|ui| {
        if ui.button(tl!("Rename Brush…")).clicked() {
            acts.push(Action::BeginRename(Renaming { group: false, name: p.name.clone(), folder: Vec::new(), text: String::new() }));
            ui.close();
        }
        if ui.button(tl!("Delete Brush")).clicked() {
            acts.push(Action::Delete(p.name.clone()));
            ui.close();
        }
    });
}

/// Width a list row keeps for the gaps around its stroke preview and the preset name.
const STROKE_NAME_ROOM: f32 = 140.0;

fn list_row(ui: &mut egui::Ui, p: &BrushPreset, current: bool, presets: &[BrushPreset], indent: f32, acts: &mut Vec<Action>) {
    let t = Tokens::get(ui.ctx());
    let (full, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click_and_drag());
    // Rows inside nested folders are indented; the whole width stays the drop target.
    let r = egui::Rect::from_min_max(full.min + vec2(indent.min(full.width() / 2.0), 0.0), full.max);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, current, &p.name));
    if !ui.is_rect_visible(r) {
        return;
    }
    if current {
        ui.painter().rect_filled(r, t.radius_sm, t.accent_soft);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, t.radius_sm, t.hover);
    }
    let pb = &p.brush;
    let tip = brush_preview::tip_texture(ui.ctx(), &format!("brushes-tip:{}", p.name), &pb.tip, (pb.hardness, pb.angle, pb.roundness), 36, t.text);
    let cell = egui::Rect::from_min_size(r.left_top() + vec2(22.0, 2.0), vec2(36.0, 30.0));
    let tr = egui::Rect::from_center_size(cell.center(), vec2(30.0, 30.0) * crate::brush_sections::thumb_scale(pb.size));
    ui.painter().image(tip.id(), tr, full_uv(), Color32::WHITE);
    ui.painter().text(
        pos2(cell.center().x, r.bottom() - 1.0),
        egui::Align2::CENTER_BOTTOM,
        format!("{}", pb.size.round() as i64),
        egui::FontId::proportional(9.0),
        t.text_faint,
    );
    // The stroke preview gives up width so a narrow list (the picker) still shows the name; its
    // texture slot carries the width, so lists of different widths don't re-render each other's.
    let w = (r.width() - (cell.right() - r.left()) - STROKE_NAME_ROOM).clamp(60.0, 170.0).round();
    let stroke = brush_preview::stroke_texture(ui.ctx(), &format!("brushes-stroke-{w}:{}", p.name), pb, w as u32, 36, t.text);
    let sr = egui::Rect::from_min_size(pos2(cell.right() + 8.0, r.top() + 4.0), vec2(w, 36.0));
    ui.painter().image(stroke.id(), sr, full_uv(), Color32::WHITE);
    crate::layer_row_ui::label(ui.painter(), sr.right() + 12.0, r.center().y, r.right() - 4.0, &p.name, egui::FontId::proportional(12.0), t.text_dim);
    preset_interactions(ui, &resp, r, p, presets, false, acts);
}

/// The gap between grid cells, across and down.
const GRID_GAP: f32 = 3.0;

/// The grid's columns in a list `width` points wide: how many `cell`-wide cells fit past the
/// `indent`, and the left margin that centres them in the room past it. At least one column.
pub(crate) fn grid_columns(width: f32, cell: f32, indent: f32) -> (f32, usize) {
    let room = if width.is_finite() { (width - indent).max(0.0) } else { 0.0 };
    let cols = if cell.is_finite() && cell > 0.0 { ((room + GRID_GAP) / (cell + GRID_GAP)).floor() } else { 1.0 };
    // `as` saturates (and NaN becomes 0), so this is a whole number from 1 to a sane bound.
    let cols = (cols as usize).clamp(1, 256);
    let used = cols as f32 * cell + (cols - 1) as f32 * GRID_GAP;
    // Whole points keep the tiles' edges crisp.
    let left = indent + ((room - used) / 2.0).max(0.0).floor();
    (if left.is_finite() { left } else { 0.0 }, cols)
}

fn grid_cell(ui: &mut egui::Ui, p: &BrushPreset, current: bool, presets: &[BrushPreset], cell: f32, acts: &mut Vec<Action>) {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(cell, cell + 6.0), Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, current, &p.name));
    if !ui.is_rect_visible(r) {
        return;
    }
    if current {
        ui.painter().rect_filled(r, 3.0, t.accent_soft);
        ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 3.0, t.hover);
    }
    let pb = &p.brush;
    let tip = brush_preview::tip_texture(ui.ctx(), &format!("brushes-tip:{}", p.name), &pb.tip, (pb.hardness, pb.angle, pb.roundness), 36, t.text);
    let side = cell - 14.0;
    let ir = egui::Rect::from_center_size(pos2(r.center().x, r.top() + side / 2.0 + 4.0), vec2(side, side) * crate::brush_sections::thumb_scale(pb.size));
    ui.painter().image(tip.id(), ir, full_uv(), Color32::WHITE);
    ui.painter().text(
        pos2(r.center().x, r.bottom() - 7.0),
        egui::Align2::CENTER_CENTER,
        format!("{}", pb.size.round() as i64),
        egui::FontId::proportional(9.5),
        t.text_dim,
    );
    let resp = resp.on_hover_text(&p.name);
    preset_interactions(ui, &resp, r, p, presets, true, acts);
}

/// Indent of one nested folder level.
const FOLDER_INDENT: f32 = 16.0;

/// A group's or nested folder's header: chevron, folder, name, count. Click toggles; drag
/// reorders groups (a folder among its sibling folders); a preset dropped here moves to the end of
/// the group or folder. `key` is the group key, `folder` the nested folder's path (empty for the
/// group), `label` the group's panel label.
fn group_header(ui: &mut egui::Ui, label: &str, key: &str, folder: &[String], open: bool, count: usize, acts: &mut Vec<Action>) {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click_and_drag());
    if resp.hovered() {
        ui.painter().rect_filled(r, t.radius_sm, t.hover);
    }
    let shown = folder.last().map_or(label, String::as_str);
    let x = r.left() + 4.0 + FOLDER_INDENT * folder.len().min(MAX_VIEW_DEPTH) as f32;
    icons::paint(
        ui,
        egui::Rect::from_min_size(pos2(x, r.center().y - 7.0), vec2(14.0, 14.0)),
        if open { "chevron-down" } else { "chevron-right" },
        12.0,
        t.text_dim,
    );
    icons::paint(
        ui,
        egui::Rect::from_min_size(pos2(x + 18.0, r.center().y - 8.0), vec2(16.0, 16.0)),
        if open { "folder-open" } else { "folder" },
        14.0,
        t.icon,
    );
    let font = if folder.is_empty() { theme::semibold(12.0) } else { theme::medium(12.0) };
    ui.painter().text(pos2(x + 40.0, r.center().y), egui::Align2::LEFT_CENTER, shown, font, t.text);
    ui.painter().text(pos2(r.right() - 8.0, r.center().y), egui::Align2::RIGHT_CENTER, count.to_string(), theme::medium(11.0), t.text_faint);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, shown));
    let me = match folder {
        [] => BrushDrag::Group(key.to_string()),
        f => BrushDrag::Folder { group: key.to_string(), folder: f.to_vec() },
    };
    // A folder moves among its siblings: same group, same parent, not itself.
    let sibling = |g: &str, f: &[String]| {
        g == key && !folder.is_empty() && f.len() == folder.len() && f != folder && f.split_last().map(|x| x.1) == folder.split_last().map(|x| x.1)
    };
    resp.dnd_set_drag_payload(me);
    if let Some(d) = resp.dnd_hover_payload::<BrushDrag>() {
        match &*d {
            BrushDrag::Group(g) if folder.is_empty() && g != key => widgets::drop_line(ui, r, false, false, &t),
            BrushDrag::Folder { group, folder: f } if sibling(group, f) => widgets::drop_line(ui, r, false, false, &t),
            BrushDrag::Preset(_) => {
                ui.painter().rect_stroke(r, t.radius_sm, Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
            }
            _ => {}
        }
    }
    if let Some(d) = resp.dnd_release_payload::<BrushDrag>() {
        match &*d {
            BrushDrag::Group(g) if folder.is_empty() && g != key => {
                acts.push(Action::MoveGroup { group: g.clone(), folder: Vec::new(), before: Some(key.to_string()) })
            }
            BrushDrag::Folder { group, folder: f } if sibling(group, f) => {
                acts.push(Action::MoveGroup { group: group.clone(), folder: f.clone(), before: Some(shown.to_string()) })
            }
            BrushDrag::Preset(n) => acts.push(Action::Move { name: n.clone(), group: key.to_string(), folder: Some(folder.to_vec()), index: None }),
            _ => {}
        }
    }
    if resp.clicked() {
        acts.push(Action::ToggleGroup(folder_view_key(label, folder)));
    }
    resp.context_menu(|ui| {
        if ui.button(tl!("Rename Group…")).clicked() {
            acts.push(Action::BeginRename(Renaming { group: true, name: key.to_string(), folder: folder.to_vec(), text: String::new() }));
            ui.close();
        }
        if ui.button(tl!("Delete Group")).clicked() {
            acts.push(Action::DeleteGroup { group: key.to_string(), folder: folder.to_vec() });
            ui.close();
        }
    });
}

/// How a group's content is drawn.
struct Draw<'a> {
    label: &'a str,
    key: &'a str,
    presets: &'a [BrushPreset],
    /// The selected preset's name (by identity, so look-alike duplicates stay distinct).
    current: Option<&'a str>,
    collapsed: &'a [String],
    filtering: bool,
    grid: bool,
    layout: ListLayout,
}

/// Draw `nodes` (a group's or folder's content) at nesting `depth`: runs of presets as rows or a
/// grid, nested folders as headers with their content below when open. `depth` is bounded by
/// [`folder_tree`] (paths are cut at [`MAX_VIEW_DEPTH`]).
fn draw_nodes(ui: &mut egui::Ui, d: &Draw, nodes: &[Node], depth: usize, acts: &mut Vec<Action>) {
    let indent = FOLDER_INDENT * depth as f32;
    let mut run: Vec<&BrushPreset> = Vec::new();
    let flush = |ui: &mut egui::Ui, run: &mut Vec<&BrushPreset>, acts: &mut Vec<Action>| {
        if run.is_empty() {
            return;
        }
        if d.grid {
            // Rows of whole columns, each with the same margin: a wrapped row indented only
            // its first line and left the spare width on the right.
            let (left, cols) = grid_columns(ui.available_width(), d.layout.cell, d.layout.indent + indent);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = vec2(GRID_GAP, GRID_GAP);
                for row in run.chunks(cols) {
                    ui.horizontal(|ui| {
                        ui.add_space(left);
                        for p in row {
                            grid_cell(ui, p, d.current.is_some_and(|n| p.name.eq_ignore_ascii_case(n)), d.presets, d.layout.cell, acts);
                        }
                    });
                }
            });
        } else {
            for p in run.iter() {
                list_row(ui, p, d.current.is_some_and(|n| p.name.eq_ignore_ascii_case(n)), d.presets, indent, acts);
            }
        }
        run.clear();
    };
    for n in nodes {
        match n {
            Node::Preset(p) => run.push(p),
            Node::Folder(f) => {
                flush(ui, &mut run, acts);
                let open = d.filtering || !d.collapsed.contains(&folder_view_key(d.label, &f.path));
                group_header(ui, d.label, d.key, &f.path, open, f.count, acts);
                if open && depth < MAX_VIEW_DEPTH {
                    draw_nodes(ui, d, &f.children, depth + 1, acts);
                }
            }
        }
    }
    flush(ui, &mut run, acts);
}

/// The rename bar shown while a preset or group is being renamed. Enter or OK renames, Escape or
/// Cancel stops.
fn rename_bar(ui: &mut egui::Ui, st: &mut BrushesPanelState, acts: &mut Vec<Action>) {
    let Some(r) = st.renaming.as_mut() else { return };
    let t = Tokens::get(ui.ctx());
    if r.text.is_empty() {
        r.text = r.folder.last().unwrap_or(&r.name).clone();
    }
    let mut done = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(if r.group { tl!("Group name") } else { tl!("Brush name") }).color(t.text_dim));
        let resp = ui.add(egui::TextEdit::singleline(&mut r.text).desired_width((ui.available_width() - 140.0).max(60.0)).id_salt("brush-rename"));
        if !resp.has_focus() && !resp.lost_focus() {
            resp.request_focus();
        }
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let clicked = widgets::dialog_buttons(
            ui,
            &[
                widgets::DialogButton::new(widgets::ButtonRole::Default, tl!("OK"), 52.0),
                widgets::DialogButton::new(widgets::ButtonRole::Cancel, tl!("Cancel"), 60.0),
            ],
        );
        if clicked == Some(widgets::ButtonRole::Default) || enter {
            acts.push(Action::Rename(r.clone()));
            done = true;
        }
        if clicked == Some(widgets::ButtonRole::Cancel) || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            done = true;
        }
    });
    if done {
        st.renaming = None;
    }
    ui.add_space(4.0);
}

/// The presets in their groups, filtered by `st.filter`, as rows or tip cells (`st.view`), with
/// the rename bar above them while a rename is open. `current` is the selected preset's name:
/// selection is by identity, so it survives edits and tells look-alike duplicates apart. Group
/// toggles and renames update `st`; the returned actions are commands for [`apply`].
pub fn preset_list(ui: &mut egui::Ui, presets: &[BrushPreset], current: Option<&str>, st: &mut BrushesPanelState, layout: ListLayout) -> Vec<Action> {
    let mut acts = Vec::new();
    rename_bar(ui, st, &mut acts);
    let filter = st.filter.trim().to_lowercase();
    let grid = st.view == BrushesView::Grid;
    let collapsed = &st.collapsed;
    egui::ScrollArea::vertical().id_salt(layout.id).max_height(layout.max_height).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        for (label, items) in grouped_presets(presets) {
            let items: Vec<&BrushPreset> =
                items.into_iter().filter_map(|i| presets.get(i)).filter(|p| filter.is_empty() || p.name.to_lowercase().contains(&filter)).collect();
            if items.is_empty() {
                continue;
            }
            let key = group_key(presets, &label);
            let open = !filter.is_empty() || !collapsed.contains(&label);
            group_header(ui, &label, &key, &[], open, items.len(), &mut acts);
            if !open {
                continue;
            }
            let d = Draw { label: &label, key: &key, presets, current, collapsed, filtering: !filter.is_empty(), grid, layout };
            draw_nodes(ui, &d, &folder_tree(&items), 0, &mut acts);
            ui.add_space(2.0);
        }
        if presets.is_empty() {
            ui.label(RichText::new(tl!("No brush presets")).color(Tokens::get(ui.ctx()).text_faint));
        }
    });
    // The dragged preset or group follows the pointer.
    if let Some(d) = egui::DragAndDrop::payload::<BrushDrag>(ui.ctx())
        && let Some(p) = ui.ctx().pointer_interact_pos()
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let name = match &*d {
            BrushDrag::Preset(n) => n.clone(),
            BrushDrag::Group(g) => {
                if g.is_empty() {
                    UNGROUPED.to_string()
                } else {
                    g.clone()
                }
            }
            BrushDrag::Folder { folder, .. } => folder.last().cloned().unwrap_or_default(),
        };
        egui::Area::new(egui::Id::new(("brush-drag-label", layout.id))).order(egui::Order::Tooltip).fixed_pos(p + vec2(12.0, 8.0)).interactable(false).show(
            ui.ctx(),
            |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(name));
            },
        );
    }
    view_actions(st, acts)
}

/// The Brushes tab.
pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    // Size of the current brush (Photoshop's Brushes panel slider).
    let before = app.session.tools.brush.clone();
    let mut b = before.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Size")).color(t.text_dim));
        let mut lv = b.size.max(1.0).ln();
        ui.add_sized(vec2(WIDTH - 140.0, 18.0), |ui: &mut egui::Ui| {
            let r = widgets::slider(ui, &mut lv, 0.0..=MAX_BRUSH_SIZE.ln(), None);
            if r.changed() {
                b.size = lv.exp().round().clamp(1.0, MAX_BRUSH_SIZE);
            }
            r
        });
        let mut s = b.size;
        if widgets::value_field(ui, &mut s, 1.0..=MAX_BRUSH_SIZE, "px", 74.0).changed() {
            b.size = s.round().clamp(1.0, MAX_BRUSH_SIZE);
        }
    });
    commit_gesture(app, ui.ctx(), &before, &b);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        icons::paint(ui, egui::Rect::from_min_size(ui.cursor().min + vec2(0.0, 3.0), vec2(16.0, 16.0)), "search", 14.0, t.text_faint);
        ui.add_space(20.0);
        ui.add(egui::TextEdit::singleline(&mut app.ui.brushes_panel.filter).hint_text(tl!("Search Brushes")).desired_width(WIDTH - 120.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let view = &mut app.ui.brushes_panel.view;
            if icons::button(ui, "grid-2x2", 24.0, *view == BrushesView::Grid, "Grid view").clicked() {
                *view = BrushesView::Grid;
            }
            if icons::button(ui, "align-justify", 24.0, *view == BrushesView::List, "List view").clicked() {
                *view = BrushesView::List;
            }
        });
    });
    ui.add_space(6.0);
    let acts = preset_list(ui, &app.session.tools.presets, app.session.tools.current_preset.as_deref(), &mut app.ui.brushes_panel, PANEL_LIST);
    apply(app, acts);
    ui.add_space(4.0);
    widgets::hairline(ui);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} presets", app.session.tools.presets.len())).color(t.text_faint));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // The current preset by identity: editing the brush after picking must not deselect it.
            let current = app.session.tools.current_preset.clone().filter(|n| app.session.tools.presets.iter().any(|p| p.name.eq_ignore_ascii_case(n)));
            if ui.add_enabled_ui(current.is_some(), |ui| icons::button(ui, "trash", 24.0, false, "Delete brush")).inner.clicked()
                && let Some(name) = current
            {
                run_or_status(app, "brush.presets.delete", json!({ "name": name }));
            }
            if icons::button(ui, "square-plus", 24.0, false, "Create new brush from the current settings").clicked() {
                let name = new_preset_name(&app.session.tools.presets);
                run_or_status(app, "brush.presets.save", json!({ "name": name }));
            }
            if icons::button(ui, "folder-open", 24.0, false, "Import Brushes… (.abr)").clicked() {
                let _ = app.open_dialog_file();
            }
        });
    });
    // Forget previews of presets that no longer exist.
    let names: std::collections::HashSet<&str> = app.session.tools.presets.iter().map(|p| p.name.as_str()).collect();
    brush_preview::with_cache(ui.ctx(), |c| {
        c.retain(|slot| slot.split_once(':').is_none_or(|(_, n)| names.contains(n)));
    });
}

#[cfg(test)]
#[path = "brushes_tab_tests.rs"]
mod tests;
