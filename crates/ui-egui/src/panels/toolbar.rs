use super::{PhotocraftApp, Tool, json, vec2};
use craft::behavior::overlay::Side;
use craft::color_ui::ui::{PairWell, SwatchPair, Well};
use craft::ui::contract::{Bind, Gap, Props as _, Show as _, ShowContainer as _, clicked};
use craft::ui::layout::RowAlign;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::rails::{
    DrawerGroup, FlyoutButton, LastUsed, RailFooter, RailTool, ToolRail, ToolSlot, ToolbarDrawer, ToolbarEdit, apply_toolbar_edit, visible_slots,
};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Label;
use craft::ui::shell::ScreenModeButton;
use photocraft_engine::prefs::ToolbarCustomization;

use super::tool_commands;

#[cfg(test)]
#[path = "toolbar_tab_reveal_tests.rs"]
mod toolbar_tab_reveal_tests;

#[cfg(test)]
#[path = "toolbar_measure_tests.rs"]
mod toolbar_measure_tests;

// ----------------------------------------------------------------------------- toolbar

/// Tool groups separated by hairlines, Photoshop order.
/// Photoshop toolbar: divider-separated sections of slots; each slot is a flyout group whose button
/// shows the group's last-used tool.
pub(crate) const TOOL_SECTIONS: &[&[&[Tool]]] = &[
    &[&[Tool::Move, Tool::Artboard]],
    &[
        &[Tool::RectMarquee, Tool::EllipseMarquee, Tool::SingleRowMarquee, Tool::SingleColumnMarquee],
        &[Tool::Lasso, Tool::PolygonLasso, Tool::MagneticLasso],
        &[Tool::ObjectSelection, Tool::QuickSelection, Tool::MagicWand],
        &[Tool::Crop, Tool::PerspectiveCrop, Tool::Slice, Tool::SliceSelect],
        &[Tool::Frame],
        &[Tool::Eyedropper, Tool::ColorSampler, Tool::Ruler, Tool::Note, Tool::Count],
    ],
    &[
        &[Tool::SpotHealing, Tool::Remove, Tool::Healing, Tool::Patch, Tool::ContentAwareMove, Tool::RedEye],
        &[Tool::Brush, Tool::Pencil, Tool::ColorReplacement, Tool::MixerBrush],
        &[Tool::CloneStamp, Tool::PatternStamp],
        &[Tool::HistoryBrush, Tool::ArtHistoryBrush],
        &[Tool::Eraser, Tool::BackgroundEraser, Tool::MagicEraser],
        &[Tool::Gradient, Tool::PaintBucket],
        &[Tool::Blur, Tool::Sharpen, Tool::Smudge],
        &[Tool::Dodge, Tool::Burn, Tool::Sponge],
    ],
    &[
        &[Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen, Tool::AddAnchorPoint, Tool::DeleteAnchorPoint, Tool::ConvertPoint],
        &[Tool::Type, Tool::VerticalType, Tool::VerticalTypeMask, Tool::HorizontalTypeMask],
        &[Tool::PathSelection, Tool::DirectSelection],
        &[Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Star, Tool::Line, Tool::CustomShape],
    ],
    &[&[Tool::Hand, Tool::RotateView], &[Tool::Zoom]],
];

/// Position of `tool` in [`TOOL_SECTIONS`] read top to bottom, slot by slot. Tools outside the
/// toolbar sort last. A key's first tool and the Edit Toolbar drawer's list follow this order.
pub(crate) fn toolbar_position(tool: Tool) -> usize {
    TOOL_SECTIONS.iter().flat_map(|section| section.iter()).flat_map(|slot| slot.iter()).position(|&t| t == tool).unwrap_or(usize::MAX)
}

/// Opens the Edit Toolbar drawer beside the rail. The rail's "…" button and the `edit.toolbar` command both come here.
pub(crate) fn open_drawer(app: &mut PhotocraftApp) {
    app.ui.toolbar_drawer = true;
}

/// The toolbar's group layout from Edit › Toolbar (`edit.toolbar`), as tools: the slots' groups
/// in order, then the Extra Tools groups. Names were resolved by the engine
/// (`ToolbarCustomization::layout`), which writes `Tool` debug names.
pub(crate) fn tool_layout(custom: &ToolbarCustomization) -> (Vec<Vec<Tool>>, Vec<Vec<Tool>>) {
    let layout = custom.layout();
    let tools = |groups: Vec<Vec<String>>| -> Vec<Vec<Tool>> {
        groups.iter().map(|g| g.iter().filter_map(|n| Tool::from_name(n)).collect::<Vec<Tool>>()).filter(|g| !g.is_empty()).collect()
    };
    (tools(layout.toolbar), tools(layout.extras))
}

/// Every tool in layout order: the slots' groups, then the extras. Shift+letter steps through a
/// key's tools in this order, across slots.
pub(crate) fn layout_order(custom: &ToolbarCustomization) -> Vec<Tool> {
    let (toolbar, extras) = tool_layout(custom);
    toolbar.into_iter().chain(extras).flatten().collect()
}

/// The default slot (section, index in [`TOOL_SECTIONS`] read slot by slot) that holds `tool`.
fn home_slot(tool: Tool) -> Option<(usize, usize)> {
    let mut index = 0usize;
    for (si, section) in TOOL_SECTIONS.iter().enumerate() {
        for slot in section.iter() {
            if slot.contains(&tool) {
                return Some((si, index));
            }
            index += 1;
        }
    }
    None
}

/// The slots the toolbar draws: one per toolbar group of the stored layout, in order, each
/// listing its group's tools in order. Consecutive groups whose first tools come from the same
/// default section share a section, so the default layout keeps its dividers. Each slot carries
/// the index of its first tool's default slot in `TOOL_SECTIONS` (or one past the defaults when
/// that index is taken), so its flyout stays put when other slots are hidden or moved.
pub(crate) fn layout_sections(custom: &ToolbarCustomization) -> Vec<Vec<(usize, Vec<Tool>)>> {
    let (groups, _) = tool_layout(custom);
    let defaults: usize = TOOL_SECTIONS.iter().map(|section| section.len()).sum();
    let mut used: Vec<usize> = Vec::new();
    let mut sections: Vec<Vec<(usize, Vec<Tool>)>> = Vec::new();
    let mut last_section = None;
    for (i, group) in groups.into_iter().enumerate() {
        let home = group.first().copied().and_then(home_slot);
        let index = match home {
            Some((_, slot)) if !used.contains(&slot) => slot,
            _ => defaults + i,
        };
        used.push(index);
        let section = home.map(|(si, _)| si);
        if sections.is_empty() || section != last_section {
            sections.push(Vec::new());
        }
        last_section = section;
        if let Some(current) = sections.last_mut() {
            current.push((index, group));
        }
    }
    sections
}

/// The tool each slot shows when the current tool is not in it: the most recent tool of `group_tools` (most recent first)
/// that the slot holds.
fn last_used(slots: &[ToolSlot<Tool>], group_tools: &[Tool]) -> LastUsed<Tool> {
    let mut last = LastUsed::default();
    for slot in slots {
        if let Some(tool) = group_tools.iter().find(|tool| slot.tools.iter().any(|held| held.id == **tool)) {
            last.set(slot.id, *tool);
        }
    }
    last
}

/// The cells of the toolbar's grid: its visible slots and the Edit Toolbar button. The dock column's width in steps goes
/// up to this many content columns.
pub(crate) fn cells(app: &PhotocraftApp) -> usize {
    visible_slots(&app.toolbar_slots.get(&app.session.prefs), &[]).len() + 1
}

/// The Edit in Quick Mask Mode command, which the rail's footer shows as a rail button.
pub(crate) const QUICK_MASK: &str = "select.editInQuickMaskMode";

/// Quick Mask as a rail button, the size of the tool buttons: a click runs Edit in Quick Mask Mode, and the button shows
/// on while the active document is in Quick Mask.
fn quick_mask_button(app: &PhotocraftApp, ui: &mut egui::Ui) {
    let on = app.session.active().is_some_and(|s| s.doc.quick_mask.is_some());
    let tools = [RailTool::new(0_u8, CommandRef::new(QUICK_MASK))];
    let _ = FlyoutButton::new(&tools, &0).id_salt("quick-mask").selected(on).show(ui);
}

/// The rail over `slots`, its flyouts and drawer opening on `opens`, the side of the canvas.
fn rail<'a>(slots: &'a [ToolSlot<Tool>], current: &'a Tool, remembered: &'a LastUsed<Tool>, opens: Side, editing: bool) -> ToolRail<'a, Tool> {
    let rail = ToolRail::new(Label::Empty, slots, current, remembered).opens(opens).edit_toolbar(tl!("Edit Toolbar…").to_string());
    // The drawer's open flag is the one source of "editing".
    if editing { rail.editing() } else { rail }
}

/// The rail's footer: the colour chips, then the Quick Mask and Screen Mode buttons.
fn footer(app: &mut PhotocraftApp, ui: &mut egui::Ui, rail: RailFooter) {
    let column = ui.available_width();
    color_chips(app, ui, column);
    let modes = |ui: &mut egui::Ui, app: &mut PhotocraftApp| {
        quick_mask_button(app, ui);
        ScreenModeButton::new(app.ui.view.screen_mode).show(ui);
    };
    // The two buttons sit centred in the rail's columns, side by side in two columns.
    Row::new().fill_width().align(RowAlign::Center).gap(Gap::Tight).show(ui, |ui, _| {
        if rail.columns > 1 {
            modes(ui, app);
        } else {
            Stack::new().gap(Gap::Tight).show(ui, |ui, _| modes(ui, app));
        }
    });
}

/// What the rail's measured height depends on: the width and room it is laid out in, the slots (the same `Arc` while
/// the preferences are the same), where its flyouts open, the Edit Toolbar drawer, and the scale, direction and resolved
/// tokens (theme, density, text size) it is drawn with. The current and remembered tools change which icon a button
/// shows, not its size, and the footer's buttons keep their sizes whatever they show.
#[derive(Clone)]
struct MeasureKey {
    width: u32,
    room: [u32; 2],
    slots: std::sync::Arc<[ToolSlot<Tool>]>,
    opens: Side,
    editing: bool,
    pixels_per_point: u32,
    rtl: bool,
    tokens: std::sync::Arc<craft::ui::style::UiTokens>,
}

impl PartialEq for MeasureKey {
    fn eq(&self, other: &Self) -> bool {
        (self.width, self.room, self.opens, self.editing, self.pixels_per_point, self.rtl)
            == (other.width, other.room, other.opens, other.editing, other.pixels_per_point, other.rtl)
            && std::sync::Arc::ptr_eq(&self.slots, &other.slots)
            && std::sync::Arc::ptr_eq(&self.tokens, &other.tokens)
    }
}

/// The height of the toolbar's rail laid out `width` wide: as many tool columns as `width` holds, with the footer.
///
/// The dock asks for it on every frame before it shows the rail, and a measuring run of the rail (every tool button
/// and the footer, each resolving its command) was the largest part of the UI pass (#15493). The last height is kept
/// with what it depends on ([`MeasureKey`]) and measured again only when one of them changes.
pub(crate) fn measure(app: &mut PhotocraftApp, ui: &egui::Ui, width: f32) -> f32 {
    thread_local! {
        static LAST: std::cell::RefCell<Option<(MeasureKey, f32)>> = const { std::cell::RefCell::new(None) };
    }
    let slots = app.toolbar_slots.get(&app.session.prefs);
    let (opens, editing) = (crate::panel_docking::toolbar_opens(app), app.ui.toolbar_drawer);
    let env = craft::ui::runtime::env::Env::of(ui);
    let room = ui.available_size();
    let key = MeasureKey {
        width: width.to_bits(),
        room: [room.x.to_bits(), room.y.to_bits()],
        slots: std::sync::Arc::clone(&slots),
        opens,
        editing,
        pixels_per_point: ui.ctx().pixels_per_point().to_bits(),
        rtl: env.direction.is_rtl(),
        tokens: std::sync::Arc::clone(&env.tokens),
    };
    if let Some(height) = LAST.with(|last| last.borrow().as_ref().filter(|(kept, _)| *kept == key).map(|(_, height)| *height)) {
        return height;
    }
    let height = measure_now(app, ui, width);
    LAST.with(|last| *last.borrow_mut() = Some((key, height)));
    height
}

/// [`measure`] without the kept height: a measuring run of the rail.
fn measure_now(app: &mut PhotocraftApp, ui: &egui::Ui, width: f32) -> f32 {
    let slots = app.toolbar_slots.get(&app.session.prefs);
    let remembered = last_used(&slots, &app.ui.group_tools);
    let current = app.ui.tool;
    let (opens, editing) = (crate::panel_docking::toolbar_opens(app), app.ui.toolbar_drawer);
    rail(&slots, &current, &remembered, opens, editing).measure_rail(ui, width, |ui, at| footer(app, ui, at)).y
}

/// The tool palette: a `ToolRail` over the slots, with the colour chips and then the Quick Mask and Screen Mode buttons
/// in its footer, and, while the Edit Toolbar drawer is open, the drawer beside it. The Toolbar panel's group view
/// shows it in the group's full rect; the rail takes as many tool columns as the rect's width holds.
///
/// A frame can run this twice. Every outcome comes from the pass's own widgets (a click, a drop), and the slots and
/// remembered tools are read again at the start of each pass, so a second pass applies only its own input.
pub fn toolbar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let slots = app.toolbar_slots.get(&app.session.prefs);
    let remembered = last_used(&slots, &app.ui.group_tools);
    let current = app.ui.tool;
    let (opens, editing) = (crate::panel_docking::toolbar_opens(app), app.ui.toolbar_drawer);
    let out = rail(&slots, &current, &remembered, opens, editing).show_rail(ui, |ui, at| footer(app, ui, at));

    if let Some(tool) = out.select {
        crate::tool_memory::select(app, tool);
    }
    // A double click on Hand fits the image on screen, on Zoom shows it at 100% (Photoshop); with no document open there
    // is nothing to do. The framework reports no double click after a modified click.
    match out.double_clicked {
        Some(Tool::Hand) => {
            let _ = crate::menus::invoke(app, &ctx, "view.fitOnScreen", json!({}));
        }
        Some(Tool::Zoom) => {
            let _ = crate::menus::invoke(app, &ctx, "view.actualPixels", json!({}));
        }
        _ => {}
    }
    if let Some(next) = &out.last_used {
        for slot in slots.iter() {
            if let Some(tool) = next.get(slot.id)
                && remembered.get(slot.id) != Some(tool)
            {
                app.ui.remember_group_tool(*tool);
            }
        }
    }
    if out.edit_toolbar.as_ref().is_some_and(clicked) {
        open_drawer(app);
    }
    if app.ui.toolbar_drawer {
        show_drawer(app, ui, &ctx, &slots, opens, &out);
    }
}

/// The Edit Toolbar drawer beside the rail: its tool list, and the arrangement edits saved through `edit.toolbar`.
fn show_drawer(
    app: &mut PhotocraftApp,
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    slots: &[ToolSlot<Tool>],
    opens: Side,
    rail: &craft::ui::rails::ToolRailOutput<Tool>,
) {
    // Every tool is listed in toolbar order; the drawer dims the ones the toolbar holds.
    let mut all: Vec<Tool> = Tool::ALL.to_vec();
    all.sort_by_key(|tool| toolbar_position(*tool));
    let groups = [DrawerGroup::new(tl!("All Tools").to_string(), all.into_iter().map(tool_commands::rail_tool).collect())];
    let mut drawer = ToolbarDrawer::new(rail.response.rect, &groups, slots, rail.dropped).opens(opens);
    if let Some(opener) = &rail.edit_toolbar {
        drawer = drawer.opened_by(opener);
    }
    let out = drawer.show(ui);
    if let Some(tool) = out.select {
        crate::tool_memory::select(app, tool);
    }
    if !out.edits.is_empty() {
        // `DropAt::NewSlot { before }` counts the slots as the rail was given them, the same list the edits apply to.
        let defaults = tool_commands::slots(&ToolbarCustomization::default());
        let mut arranged = slots.to_vec();
        for edit in &out.edits {
            arranged = apply_toolbar_edit(&arranged, edit, &defaults);
        }
        let params = if matches!(out.edits.last(), Some(ToolbarEdit::Reset)) {
            json!({"reset": true})
        } else {
            json!({"toolbar": tool_commands::saved_groups(&arranged)})
        };
        if let Err(e) = crate::menus::invoke(app, ctx, "edit.toolbar", params) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    if out.closed {
        app.ui.toolbar_drawer = false;
    }
}

/// Photoshop's foreground / background colour chips: `SwatchPair` in host-picker mode, the
/// foreground in front, Default Colors (D) and Switch Colors (X) over them, centred in the tool
/// column.
fn chip_pair<'a>(app: &PhotocraftApp, active: &'a mut Well) -> SwatchPair<'a> {
    let tools = &app.session.tools;
    let wells = [
        PairWell::new(tl!("Set foreground color"), crate::panels::color::srgba(tools.foreground)),
        PairWell::new(tl!("Set background color"), crate::panels::color::srgba(tools.background)),
    ];
    SwatchPair::new(tl!("Colors"), wells, Bind::bound(active))
        .popover(false)
        .click_activates(false)
        .default_button(tl!("Default Foreground and Background Colors (D)"))
        .swap_button(tl!("Switch Foreground and Background Colors (X)"))
}

/// The colour chips: a click on a chip opens the Color Picker for that colour (Photoshop).
pub(super) fn color_chips(app: &mut PhotocraftApp, ui: &mut egui::Ui, column: f32) {
    let mut active = Well::Primary;
    let layout = egui::Layout::top_down(egui::Align::Center);
    let out = ui.allocate_ui_with_layout(vec2(column, 0.0), layout, |ui| chip_pair(app, &mut active).show(ui)).inner;
    match out.clicked {
        Some(Well::Secondary) => {
            crate::color_picker_ui::open(app, "background");
        }
        Some(_) => {
            crate::color_picker_ui::open(app, "foreground");
        }
        None => {}
    }
    if out.swap {
        chip_command(app, "tools.swapColors");
    }
    if out.defaults {
        chip_command(app, "tools.defaultColors");
    }
}

/// Run Switch Colors or Default Colors from the chips. As in Photoshop, characters highlighted
/// in the type layer being edited take the new foreground colour, like picking one (#5690).
fn chip_command(app: &mut PhotocraftApp, id: &str) {
    if app.run(id, json!({})).is_ok() {
        crate::type_tool::foreground_changed(app);
    }
}
