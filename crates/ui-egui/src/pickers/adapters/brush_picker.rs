//! Photoshop's Brush Preset picker: the popup that the options-bar brush chip and a right-click on
//! the canvas open (`paint_mouse`). It is the quick picker (Size, Hardness for round tips, a search
//! field and the preset library in its groups); the full editor is the Brush
//! Settings panel (F5), which the picker and the options bar open with one click (#258).
//!
//! The preset browser uses the shared asset browser's normal display modes and shell menu. A click
//! picks a preset and keeps the picker open, a double-click picks it and closes the picker (#1031);
//! drag to reorder, right-click to rename or delete. Only the brush-tip thumbnail is domain-specific.
//!
//! The picker edits a copy of the brush; the caller sends the size and hardness edits through
//! `tools.setBrush` ([`crate::brush_panel::commit_gesture`]) and applies the returned [`Pick`]s
//! with [`apply`], so every change is a journaled command (Rule 1).

use egui::{CursorIcon, RichText, Stroke, vec2};
use photocraft_engine::BrushSettings;
use photocraft_engine::paint::{BrushPreset, MAX_BRUSH_SIZE, TipShape};
use serde_json::json;

use crate::brushes_tab::{self, Action};
use crate::theme::Tokens;
use crate::{PhotocraftApp, icons, widgets};

/// Width of the picker's contents.
pub const WIDTH: f32 = 300.0;
/// The picker's content size before its grip is dragged, and the smallest it can be dragged to.
pub const DEFAULT_SIZE: [f32; 2] = [WIDTH, 420.0];
pub const MIN_SIZE: [f32; 2] = [230.0, 170.0];
/// The block the Size and Hardness controls share, flush with the picker's left edge and at most
/// this wide: a picker dragged wider keeps the controls their size, the space past them empty.
pub(crate) const HEAD_MAX_W: f32 = 350.0;
/// What the picker asks for beyond the size and hardness edits.
#[derive(Debug)]
pub enum Pick {
    /// A preset-list action: pick, reorder, rename or delete a preset, as in the Brushes panel.
    List(Action),
    /// Open the picker options menu anchored to its header button.
    OpenMenu(egui::Response),
    /// Save the current brush as a new preset, then name it in the rename bar.
    NewPreset,
}

impl Pick {
    /// Does this pick dismiss the picker? A double-click on a preset does (Photoshop), and so does
    /// anything that opens another panel or dialog.
    pub fn closes(&self) -> bool {
        matches!(self, Pick::List(Action::Choose(_) | Action::NewPreset { .. } | Action::NewAtSelection { .. } | Action::NewNamed { .. }))
    }
}

/// Stable egui identity shared by the picker surface and the canvas-popup dismissal logic.
pub(crate) fn picker_id() -> egui::Id {
    egui::Id::new("canvas-brush-picker")
}

/// All opening paths protect the same frame, including the control channel and options bar.
pub(crate) fn open(ui: &mut crate::state::UiState, ctx: &egui::Context, at: [f32; 2]) {
    ui.brush_picker = Some(at);
    crate::canvas_popup::mark_open(ctx, picker_id());
}

/// The options-bar brush chip that shows and hides the picker. A press on it isn't an outside
/// press that closes the picker: the chip's click toggles it.
pub fn chip_id() -> egui::Id {
    egui::Id::new("brush-preset-chip")
}

/// Is screen point `p` on the options-bar brush chip?
pub fn on_chip(ctx: &egui::Context, p: egui::Pos2) -> bool {
    ctx.read_response(chip_id()).is_some_and(|r| r.rect.contains(p))
}

/// Close the picker. Only transient rename state owned by this picker is dismissed; other preset
/// panels share `PresetUi` and must retain their own rename state.
pub fn close(ui: &mut crate::state::UiState) {
    ui.brush_picker = None;
    if ui.presets_ui.item_renaming.as_ref().is_some_and(|rename| rename.panel == "brushes") {
        ui.presets_ui.item_renaming = None;
    }
    ui.presets_ui.item_rename_queue.retain(|rename| rename.panel != "brushes");
    if ui.presets_ui.renaming.as_ref().is_some_and(|(panel, _, _)| panel == "brushes") {
        ui.presets_ui.renaming = None;
    }
    if ui.presets_ui.group_renaming.as_ref().is_some_and(|rename| rename.panel == "brushes") {
        ui.presets_ui.group_renaming = None;
    }
}

/// Run what the picker asked for.
pub fn apply(app: &mut PhotocraftApp, picks: Vec<Pick>) {
    for pick in picks {
        match pick {
            Pick::List(a) => brushes_tab::apply(app, vec![a]),
            Pick::OpenMenu(response) => {
                crate::pickers::picker_panel_menu::panel_menu_popup(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                    crate::menu_keys::with_menu_item_indent(ui, |ui| {
                        widgets::style_spectrum_popup_menu(ui);
                        ui.set_min_width(180.0);
                        crate::brush_panel::picker_menu(app, ui);
                    });
                });
            }
            // Photoshop asks for the new preset's name and what it captures first, in a dialog that
            // takes the picker's place.
            Pick::NewPreset => {
                close(&mut app.ui);
                crate::brush_preset_dialog::open(app, None);
            }
        }
    }
}

/// The options-bar button beside the brush chip that shows and hides the Brush Settings panel
/// (Photoshop's "Toggle the Brush Settings panel"). It runs Window › Brush Settings (F5).
pub fn settings_toggle(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let on = app.ui.panels.brush_settings;
    let tooltip = "Toggle the Brush Settings panel  (F5)";
    if named(icons::button(ui, "sliders-horizontal", 24.0, on, tl!(tooltip)), "Toggle the Brush Settings panel").clicked()
        && let Err(e) = crate::menus::invoke(app, ui.ctx(), "window.panel.brushSettings", json!({}))
    {
        app.ui.status = e;
    }
}

/// An icon button's accessible name (icon buttons have only a tooltip).
pub(crate) fn named(resp: egui::Response, label: &str) -> egui::Response {
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!(label)));
    resp
}

/// The picker's contents: Size, Hardness (round tips only: a sampled tip has none), the shared
/// picker shell, brush-specific header actions, and the preset library. Size and Hardness sit in a block flush with
/// the left edge, at most [`HEAD_MAX_W`] wide, so a picker dragged wider doesn't stretch them.
/// `current` is the selected preset's name (by identity, so edits don't deselect). `size` is the
/// content size the user dragged the picker to ([`DEFAULT_SIZE`] before that): the list fills
/// it, so closing every group doesn't shrink the picker and reopening one has the room to show
/// it.
pub fn body(
    ui: &mut egui::Ui,
    b: &mut BrushSettings,
    presets: &[BrushPreset],
    current: Option<&str>,
    preset_ui: &mut crate::pickers::state::PresetUi,
    size: egui::Vec2,
) -> Vec<Pick> {
    let t = Tokens::get(ui.ctx());
    let picks = std::cell::RefCell::new(Vec::new());
    // The picker is its dragged (or remembered) size: at least that tall even when every group
    // is closed, so reopening one has the room to show it.
    ui.set_width(size.x);
    ui.set_min_height(size.y);
    ui.set_max_height(size.y);
    let top = ui.cursor().min.y;
    // These are brush-tool controls, not part of the shared picker shell. Keep them stacked above
    // the shell so its header/search/browser layout stays consistent with other pickers. The
    // action buttons own a real column: sliders never paint or interact underneath them.
    let row_top = ui.cursor().min;
    let controls_width = (size.x - 44.0).min(HEAD_MAX_W).max(60.0);
    let controls_rect = egui::Rect::from_min_size(row_top, vec2(controls_width, size.y));
    let mut controls_ui = ui.new_child(egui::UiBuilder::new().max_rect(controls_rect));
    controls_ui.set_width(controls_width);
    controls_ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!("Size")).color(t.text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut value = b.size;
                if widgets::value_field(ui, &mut value, 1.0..=MAX_BRUSH_SIZE, "px", 72.0).changed() {
                    b.size = value.round().clamp(1.0, MAX_BRUSH_SIZE);
                }
            });
        });
        let mut lv = b.size.max(1.0).ln();
        let size_slider = widgets::slider(ui, &mut lv, 0.0..=MAX_BRUSH_SIZE.ln(), None);
        if size_slider.changed() {
            b.size = lv.exp().round().clamp(1.0, MAX_BRUSH_SIZE);
        }
        dismiss_popup(&size_slider);
        if b.tip == TipShape::Round {
            let mut hard = b.hardness * 100.0;
            let hardness = widgets::slider_row(ui, "Hardness", &mut hard, 0.0..=100.0, "%", None);
            if hardness.changed() {
                b.hardness = (hard / 100.0).clamp(0.0, 1.0);
            }
            dismiss_popup(&hardness);
        }
    });
    let row_height = (controls_ui.cursor().min.y - row_top.y).max(56.0);
    let (row_rect, _) = ui.allocate_exact_size(vec2(size.x, row_height), egui::Sense::hover());
    let gear_rect = egui::Rect::from_min_size(egui::pos2(row_rect.right() - 32.0, row_rect.top()), vec2(24.0, 24.0));
    let mut gear_ui = ui.new_child(egui::UiBuilder::new().max_rect(gear_rect));
    let gear = icons::button(&mut gear_ui, "settings", 24.0, false, tl!("Brush Preset Options"));
    picks.borrow_mut().push(Pick::OpenMenu(gear));
    let new_preset_rect = gear_rect.translate(vec2(0.0, 28.0));
    let mut new_preset_ui = ui.new_child(egui::UiBuilder::new().max_rect(new_preset_rect));
    if named(icons::button(&mut new_preset_ui, "square-plus", 24.0, false, tl!("Create new brush preset from the current settings")), "New Brush Preset")
        .clicked()
    {
        picks.borrow_mut().push(Pick::NewPreset);
    }
    // The Size/Hardness block sits above the shared shell, so reserve its measured height from
    // the shell's total budget. The shell subtracts its own header/search/footer; without this
    // outer reservation the picker grows by the height of these brush-specific controls.
    let controls_h = (ui.cursor().min.y - top).max(0.0);
    let mut footer = |ui: &mut egui::Ui, state: &mut crate::pickers::state::PresetUi| -> Vec<crate::pickers::browser::asset_browser::BrowserAction> {
        crate::pickers::adapters::brushes::draw_brush_footer_controls(ui, state, presets, "brushes", true)
    };
    let actions = crate::pickers::adapters::brushes::picker_browser_with_shell(
        ui,
        presets,
        current,
        preset_ui,
        crate::pickers::shell::ShellOptions {
            custom_footer: Some((&mut footer, 28.0)),
            max_total_height: Some((size.y - controls_h).max(0.0)),
            ..Default::default()
        },
    );
    picks.borrow_mut().extend(actions.into_iter().map(Pick::List));
    picks.into_inner()
}

/// Closes any open popup as soon as `resp` is grabbed: a slider or grip drag never counts as a
/// click, so [`egui::PopupCloseBehavior::CloseOnClickOutside`] alone can leave an overlapping
/// context menu open while the picker's own interaction pulls its layer in front.
fn dismiss_popup(resp: &egui::Response) {
    if resp.is_pointer_button_down_on() {
        egui::Popup::close_all(&resp.ctx);
    }
}

/// A remembered picker size, clamped for `screen` (a corrupt or wildly stale value falls back to
/// [`DEFAULT_SIZE`]).
pub fn clamp_size(size: [f32; 2], screen: egui::Vec2) -> egui::Vec2 {
    if !size[0].is_finite() || !size[1].is_finite() {
        return egui::vec2(DEFAULT_SIZE[0], DEFAULT_SIZE[1]);
    }
    let min = egui::vec2(MIN_SIZE[0], MIN_SIZE[1]);
    let max = (screen - vec2(40.0, 40.0)).max(min);
    egui::vec2(size[0], size[1]).clamp(min, max)
}

/// The picker's resize grip, in its frame's bottom-right corner: three ticks, dragging gives the
/// new content size (clamped to [`MIN_SIZE`] and the screen). Returns the size while dragged. The
/// corner tracks the pointer's travel since the drag began, not each frame's movement: held past
/// a limit (the minimum, the screen), the overshoot isn't banked, so coming back the picker grows
/// only once the pointer reaches the corner again — it waits for the pointer.
pub fn resize_grip(ui: &mut egui::Ui, frame: egui::Rect, content: egui::Vec2) -> Option<egui::Vec2> {
    let t = Tokens::get(ui.ctx());
    let grip = egui::Rect::from_min_size(frame.right_bottom() - vec2(16.0, 16.0), vec2(16.0, 16.0));
    // A context menu can extend over the bottom-right corner. Give the grip its own top interaction
    // layer so it remains draggable there and can dismiss that popup, rather than letting it
    // intercept the press.
    let layer = egui::LayerId::new(egui::Order::Tooltip, ui.id().with("brush-picker-resize-layer"));
    let grip_ui = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(grip));
    let resp = grip_ui.interact(grip, ui.id().with("brush-picker-resize"), egui::Sense::drag());
    dismiss_popup(&resp);
    let corner = grip.right_bottom() - vec2(3.0, 3.0);
    for o in [0.0, 4.0, 8.0] {
        grip_ui.painter().line_segment([corner - vec2(o, 0.0), corner - vec2(0.0, o)], Stroke::new(1.0, t.text_faint));
    }
    if resp.hovered() || resp.dragged() {
        grip_ui.ctx().set_cursor_icon(CursorIcon::ResizeNwSe);
    }
    // The size this drag started from, kept while the button is held.
    let start_id = resp.id.with("start");
    if resp.drag_started() {
        ui.ctx().data_mut(|d| d.insert_temp(start_id, content));
    }
    resp.dragged().then(|| {
        let min = egui::vec2(MIN_SIZE[0], MIN_SIZE[1]);
        let max = (grip_ui.ctx().content_rect().size() - vec2(40.0, 40.0)).max(min);
        let start: egui::Vec2 = grip_ui.ctx().data_mut(|d| d.get_temp(start_id)).unwrap_or(content);
        (start + resp.total_drag_delta().unwrap_or_default()).clamp(min, max)
    })
}

#[cfg(test)]
mod menu_tests {
    use egui::vec2;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };

    use super::{apply, body};
    use crate::PhotocraftApp;

    #[test]
    fn gear_menu_opens_and_renders_the_brush_picker_menu_contents() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut harness = Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let mut brush = app.session.tools.brush.clone();
                let presets = app.session.tools.presets.clone();
                let current = app.session.tools.current_preset.clone();
                let mut preset_ui = std::mem::take(&mut app.ui.presets_ui);
                let picks = body(ui, &mut brush, &presets, current.as_deref(), &mut preset_ui, vec2(420.0, 620.0));
                app.ui.presets_ui = preset_ui;
                apply(app, picks);
            },
            app,
        );
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ProMedium);
        harness.run_steps(3);

        harness.get_by_label("Brush Preset Options").click();
        harness.run_steps(3);

        let gear = harness.get_by_label("Brush Preset Options").rect();
        assert!((gear.right() - 420.0).abs() < 1.0, "gear must sit 8px from the picker right edge: {gear:?}");
        let first_item = harness.get_by_label("New Brush Preset…").rect();
        assert!(first_item.left() >= gear.right() - 1.0, "popup item left {} must meet gear right {}", first_item.left(), gear.right());
        assert!((first_item.top() - gear.top()).abs() < 20.0, "popup item top {} must align with gear top {}", first_item.top(), gear.top());

        let labels = [
            "New Brush Preset…",
            "New Brush Group…",
            "Rename Brush…",
            "Delete Brush…",
            "Brush Name",
            "Brush Stroke",
            "Brush Tip",
            "Show Search Bar",
            "Show Recent Brushes",
            "Append Default Brushes…",
            "Import Brushes…",
            "Export Selected Brushes…",
        ];
        let mut previous_bottom = None;
        for label in labels {
            let rect = harness.get_by_label(label).rect();
            if let Some(bottom) = previous_bottom {
                assert!(rect.top() >= bottom, "{label} must follow the previous menu entry");
            }
            previous_bottom = Some(rect.bottom());
        }
        assert!(harness.get_by_label("Rename Brush…").accesskit_node().is_disabled());
        assert!(harness.get_by_label("Delete Brush…").accesskit_node().is_disabled());
        assert!(harness.get_by_label("Export Selected Brushes…").accesskit_node().is_disabled());
    }
}
