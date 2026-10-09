//! Photoshop's Brush Preset picker: the popup that the options-bar brush chip and a right-click on
//! the canvas open (`paint_mouse`). It is the quick picker (Size, Hardness for round tips, a search
//! field, the gear menu and the preset library in its groups); the full editor is the Brush
//! Settings panel (F5), which the picker and the options bar open with one click (#258).
//!
//! The preset list is the Brushes panel's ([`brushes_tab::preset_list`]): a click picks a preset
//! and keeps the picker open, a double-click picks it and closes the picker (#1031); drag to
//! reorder, right-click to rename or delete. Its view state (search, collapsed groups, list or
//! grid, a rename in progress) is `UiState::brush_picker_list`, so the control channel can read
//! and set it.
//!
//! The picker edits a copy of the brush; the caller sends the size and hardness edits through
//! `tools.setBrush` ([`crate::brush_panel::commit_gesture`]) and applies the returned [`Pick`]s
//! with [`apply`], so every change is a journaled command (Rule 1).

use egui::{RichText, vec2};
use photocraft_engine::BrushSettings;
use photocraft_engine::paint::{BrushPreset, MAX_BRUSH_SIZE, TipShape};
use serde_json::json;

use crate::brush_panel::{BrushesPanelState, BrushesView, Renaming, is_current, new_preset_name, run_or_status};
use crate::brushes_tab::{self, Action, ListLayout};
use crate::theme::Tokens;
use crate::{PhotocraftApp, icons, widgets};

/// Width of the picker's contents.
pub const WIDTH: f32 = 300.0;
/// The picker's preset list: denser than the Brushes tab's.
const LIST: ListLayout = ListLayout { id: "brush-picker-presets", max_height: 300.0, cell: 44.0, indent: 4.0 };

/// What the picker asks for beyond the size and hardness edits.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// A preset-list action: pick, reorder, rename or delete a preset, as in the Brushes panel.
    List(Action),
    /// Open the Brush Settings panel (Window › Brush Settings, F5).
    OpenSettings,
    /// Save the current brush as a new preset, then name it in the rename bar.
    NewPreset,
    /// Import brushes (.abr) with the file dialog.
    Import,
}

impl Pick {
    /// Does this pick dismiss the picker? A double-click on a preset does (Photoshop), and so does
    /// anything that opens another panel or dialog.
    pub fn closes(&self) -> bool {
        matches!(self, Pick::List(Action::Choose(_)) | Pick::OpenSettings | Pick::Import)
    }
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

/// Close the picker. A rename left open in it goes too, like Photoshop's name dialog.
pub fn close(ui: &mut crate::state::UiState) {
    ui.brush_picker = None;
    ui.brush_picker_list.renaming = None;
}

/// The picker's list view before it is changed: tip thumbnails, like Photoshop's picker.
pub fn list_state() -> BrushesPanelState {
    BrushesPanelState { view: BrushesView::Grid, ..Default::default() }
}

/// Run what the picker asked for.
pub fn apply(app: &mut PhotocraftApp, ctx: &egui::Context, picks: Vec<Pick>) {
    for pick in picks {
        match pick {
            Pick::List(a) => brushes_tab::apply(app, vec![a]),
            Pick::OpenSettings => open_settings(app, ctx),
            Pick::NewPreset => {
                let name = new_preset_name(&app.session.tools.presets);
                run_or_status(app, "brush.presets.save", json!({ "name": name }));
                // Photoshop asks for the new preset's name: the picker's rename bar does.
                if app.session.tools.presets.iter().any(|p| p.name == name) {
                    app.ui.brush_picker_list.renaming = Some(Renaming { group: false, name, text: String::new() });
                }
            }
            Pick::Import => {
                let _ = app.open_dialog_file();
            }
        }
    }
}

/// Show the Brush Settings panel on its settings tab (what F5 does when it is hidden).
pub fn open_settings(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.ui.panels.brush_settings {
        app.ui.brush_tab = 0;
        return;
    }
    if let Err(e) = crate::menus::invoke(app, ctx, "window.panel.brushSettings", json!({})) {
        app.ui.status = e;
    }
}

/// The options-bar button beside the brush chip that shows and hides the Brush Settings panel
/// (Photoshop's "Toggle the Brush Settings panel"). It runs Window › Brush Settings (F5).
pub fn settings_toggle(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let on = app.ui.panels.brush_settings;
    if named(icons::button(ui, "sliders-horizontal", 24.0, on, "Toggle the Brush Settings panel  (F5)"), "Toggle the Brush Settings panel").clicked()
        && let Err(e) = crate::menus::invoke(app, ui.ctx(), "window.panel.brushSettings", json!({}))
    {
        app.ui.status = e;
    }
}

/// An icon button's accessible name (icon buttons have only a tooltip).
pub(crate) fn named(resp: egui::Response, label: &str) -> egui::Response {
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    resp
}

/// The picker's contents: Size, Hardness (round tips only: a sampled tip has none), a search
/// field with the Brush Settings, New Preset and gear buttons, and the preset list.
pub fn body(ui: &mut egui::Ui, b: &mut BrushSettings, presets: &[BrushPreset], st: &mut BrushesPanelState) -> Vec<Pick> {
    let t = Tokens::get(ui.ctx());
    let mut picks = Vec::new();
    ui.set_width(WIDTH);
    // Size: value field plus a logarithmic slider (small sizes get most of the travel).
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Size")).color(t.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut size = b.size;
            if widgets::value_field(ui, &mut size, 1.0..=MAX_BRUSH_SIZE, "px", 72.0).changed() {
                b.size = size.round().clamp(1.0, MAX_BRUSH_SIZE);
            }
        });
    });
    let mut lv = b.size.max(1.0).ln();
    if widgets::slider(ui, &mut lv, 0.0..=MAX_BRUSH_SIZE.ln(), None).changed() {
        b.size = lv.exp().round().clamp(1.0, MAX_BRUSH_SIZE);
    }
    if b.tip == TipShape::Round {
        let mut hard = b.hardness * 100.0;
        if widgets::slider_row(ui, "Hardness", &mut hard, 0.0..=100.0, "%", None).changed() {
            b.hardness = (hard / 100.0).clamp(0.0, 1.0);
        }
    }
    ui.add_space(6.0);
    widgets::hairline(ui);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        icons::paint(ui, egui::Rect::from_min_size(ui.cursor().min + vec2(0.0, 3.0), vec2(16.0, 16.0)), "search", 14.0, t.text_faint);
        ui.add_space(20.0);
        ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text(tl!("Search Brushes")).desired_width(WIDTH - 118.0).id_salt("brush-picker-search"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            gear_menu(ui, b, presets, st, &mut picks);
            if named(icons::button(ui, "square-plus", 24.0, false, "Create new brush preset from the current settings"), "New Brush Preset").clicked() {
                picks.push(Pick::NewPreset);
            }
            if named(icons::button(ui, "sliders-horizontal", 24.0, false, "Brush Settings…  (F5): every option of this brush"), "Brush Settings…").clicked()
            {
                picks.push(Pick::OpenSettings);
            }
        });
    });
    ui.add_space(4.0);
    picks.extend(brushes_tab::preset_list(ui, presets, b, st, LIST).into_iter().map(Pick::List));
    picks
}

/// The gear: Photoshop's picker menu. A new preset, rename or delete the current one, the list's
/// view, and Import Brushes.
fn gear_menu(ui: &mut egui::Ui, b: &BrushSettings, presets: &[BrushPreset], st: &mut BrushesPanelState, picks: &mut Vec<Pick>) {
    let gear = named(icons::button(ui, "settings", 24.0, false, tl!("Brush Preset Options")), "Brush Preset Options");
    egui::Popup::menu(&gear).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        if ui.button(tl!("New Brush Preset…")).clicked() {
            picks.push(Pick::NewPreset);
            ui.close();
        }
        ui.separator();
        let current = presets.iter().find(|p| is_current(&p.brush, b)).map(|p| p.name.clone());
        ui.add_enabled_ui(current.is_some(), |ui| {
            if ui.button(tl!("Rename Brush…")).clicked()
                && let Some(name) = current.clone()
            {
                st.renaming = Some(Renaming { group: false, name, text: String::new() });
                ui.close();
            }
            if ui.button(tl!("Delete Brush")).clicked()
                && let Some(name) = current.clone()
            {
                picks.push(Pick::List(Action::Delete(name)));
                ui.close();
            }
        });
        ui.separator();
        for (view, label) in [(BrushesView::List, tl!("List view")), (BrushesView::Grid, tl!("Grid view"))] {
            if ui.selectable_label(st.view == view, label).clicked() {
                st.view = view;
                ui.close();
            }
        }
        ui.separator();
        if ui.button(tl!("Import Brushes…")).clicked() {
            picks.push(Pick::Import);
            ui.close();
        }
    });
}
