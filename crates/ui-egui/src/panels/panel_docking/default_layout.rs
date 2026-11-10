//! PhotoCraft's default panel layout, converted from the native dock state.
//!
//! The un-customized workspace draws this per frame: the dock column beside the canvas, and
//! beside it — between the dock and the canvas, the way Photoshop places it — the icon column of
//! [`ICON_PANEL_GROUPS`]. Everything the user or a named workspace customizes is a layout of its
//! own and never re-derives.
//!
//! The panels of [`ICON_PANEL_GROUPS`] live in the icon column even while the native state lists
//! them in the one column: the icon column is the default dock's second column in every theme.

use crate::dock::Group;
use craft_ui::docking::{Column, Floating, Node, Side};

use super::{ICONIC_WIDTH, PROPERTIES_WINDOW, default_width, normalize, with_canvas};
use crate::PhotocraftApp;

/// The panel groups of the default layout's icon column, top to bottom, in every theme.
pub(crate) const ICON_PANEL_GROUPS: &[Group] = &[Group::Navigator, Group::History, Group::Brushes];

/// The native dock's arrangement as a craft-ui layout: the canvas beside a dock column of the
/// shown groups, top to bottom, with their tab order, selected tab, user-set heights and
/// collapsed (minimized) groups, plus the icon column of their panels' groups at the canvas
/// side ([`ICON_PANEL_GROUPS`]). The Studio themes float Properties over the canvas's top right
/// corner. `all` includes the groups that are not shown, with every tab.
pub(crate) fn layout(app: &PhotocraftApp, all: bool) -> craft_ui::docking::Layout<String> {
    let pro = app.ui.theme.is_pro();
    let mut groups = Vec::new();
    let mut icon_groups = Vec::new();
    let mut floating = Vec::new();
    for group in app.ui.dock.order().into_iter().filter(|g| all || g.shown(&app.ui.panels)) {
        let labels = group.tabs(pro);
        let visible = if all { group.tabs(pro).iter().copied().enumerate().collect() } else { app.ui.dock.visible_tabs(group, pro) };
        let panels: Vec<String> = visible.iter().filter_map(|(_, label)| normalize(label).map(str::to_owned)).collect();
        if panels.is_empty() {
            continue;
        }
        let selected = *group.tab_mut(&mut app.ui.dock_tabs.clone());
        let active = labels.get(selected).and_then(|label| normalize(label)).and_then(|id| panels.iter().position(|p| p == id)).unwrap_or(0);
        if group == Group::Properties && !pro {
            floating.push(Floating::single(panels, active, studio_properties_rect(app)).with_id(PROPERTIES_WINDOW));
            continue;
        }
        let height = app.ui.dock.heights.contains_key(&group).then(|| app.ui.dock.height(group));
        // The native collapse flag is the group's in-place tab-strip minimize; the workspace
        // carries it as `minimized`, so the conversion needs no per-frame remap.
        let minimized = app.ui.dock.is_collapsed(group);
        let converted = craft_ui::docking::Group { minimized, height, ..craft_ui::docking::Group::new(panels, active) };
        if ICON_PANEL_GROUPS.contains(&group) {
            icon_groups.push(converted);
        } else {
            groups.push(converted);
        }
    }
    let width = default_width(pro);
    if groups.is_empty() && icon_groups.is_empty() {
        return craft_ui::docking::Layout { root: Some(with_canvas(Node::Dock { side: Side::End, columns: Vec::new() }, 0.0)), floating };
    }
    let (columns, total) = if groups.is_empty() {
        (vec![icon_column(icon_groups, 0.0)], ICONIC_WIDTH)
    } else if icon_groups.is_empty() {
        (vec![Column::new(groups, width)], width)
    } else {
        (vec![Column::new(groups, width), icon_column(icon_groups, width)], width + ICONIC_WIDTH)
    };
    craft_ui::docking::Layout { root: Some(with_canvas(Node::Dock { side: Side::End, columns }, total)), floating }
}

/// The default icon column: all of a theme's [`ICON_PANEL_GROUPS`] that are shown, collapsed to
/// their icons. No explicit `iconic_width`: the docking surface sizes the icons, and a user
/// resize on the column sets its own width. The column's expanded `width` stays the dock's
/// default, so «/» expands to full panel bodies instead of the 36-point icon strip.
fn icon_column(groups: Vec<craft_ui::docking::Group<String>>, width: f32) -> Column<String> {
    Column { iconic: true, iconic_width: None, ..Column::new(groups, width) }
}

/// Where the Studio themes' Properties card floats: inside the canvas's top right corner.
pub(crate) fn studio_properties_rect(app: &PhotocraftApp) -> [f32; 4] {
    let (width, height) = (320.0, 440.0);
    let canvas = app.last_canvas_rect;
    if canvas.is_positive() && canvas.width() > width + 24.0 {
        [canvas.right() - width - 12.0, canvas.top() + 44.0, width, height]
    } else {
        [120.0, 160.0, width, height]
    }
}
