//! Photoshop's Brush Preset picker: the popup that the options-bar brush chip and a right-click on
//! the canvas open (`paint_mouse`). It is the quick picker (Size, Hardness for round tips, a search
//! field, the gear menu and the preset library in its groups); the full editor is the Brush
//! Settings panel (F5), which the picker and the options bar open with one click (#258).
//!
//! The preset browser uses the brush-specific adapter over the shared asset browser, always as
//! cards: a click picks a preset and keeps the picker open, a double-click picks it and closes the picker
//! (#1031); drag to reorder, right-click to rename or delete. Each card is a 2 × 2 grid — tip at
//! the top left, stroke preview at the top right and the name in the merged bottom row (or in the
//! stroke's cell when only the tip and the name show) — and the gear's three boxes ([`body`]'s
//! Brush Name/Brush Stroke/Brush Tip) hide its parts, at least one always on; the footer slider
//! scales the cards. Presentation preferences live alongside the shared browser state in
//! `UiState::presets_ui`; search, expansion, selection and presentation share one persistence path.
//!
//! The picker edits a copy of the brush; the caller sends the size and hardness edits through
//! `tools.setBrush` ([`crate::brush_panel::commit_gesture`]) and applies the returned [`Pick`]s
//! with [`apply`], so every change is a journaled command (Rule 1).

use egui::{CursorIcon, RichText, Stroke, vec2};
use photocraft_engine::BrushSettings;
use photocraft_engine::paint::{BrushPreset, MAX_BRUSH_SIZE, TipShape};
use serde_json::json;

use crate::brushes_tab::{self, Action};
pub(crate) use crate::pickers::brush_browser::SCALE_RANGE;
use crate::pickers::picker_state::BrushPickerView;
use crate::theme::Tokens;
use crate::{PhotocraftApp, icons, widgets};

/// Width of the picker's contents.
pub const WIDTH: f32 = 300.0;
/// The picker's content size before its grip is dragged, and the smallest it can be dragged to.
pub const DEFAULT_SIZE: [f32; 2] = [WIDTH, 420.0];
pub const MIN_SIZE: [f32; 2] = [230.0, 170.0];
/// The scale slider's height (the Size and Hardness sliders').
const SLIDER_H: f32 = 18.0;
/// The least gap between the preset list and the slider under it.
const FOOTER_GAP: f32 = 6.0;
/// The room the footer takes from the picker's bottom.
const FOOTER_H: f32 = SLIDER_H + FOOTER_GAP;
/// Room the footer leaves on its right for the resize grip ([`resize_grip`] is 16 wide): the
/// scale slider runs up to the grip's left edge, the two corner controls side by side.
const GRIP_ROOM: f32 = 20.0;
/// The block the Size and Hardness controls share, flush with the picker's left edge and at most
/// this wide: a picker dragged wider keeps the controls their size, the space past them empty.
pub(crate) const HEAD_MAX_W: f32 = 350.0;
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
    if ui.presets_ui.renaming.as_ref().is_some_and(|(panel, _, _)| panel == "brush-picker") {
        ui.presets_ui.renaming = None;
    }
    if ui.presets_ui.group_renaming.as_ref().is_some_and(|rename| rename.panel == "brush-picker") {
        ui.presets_ui.group_renaming = None;
    }
}

/// Start the shared browser's in-place rename for this exact preset occurrence. Brush commands
/// still address names, so the gear menu only offers this action when that name is unambiguous.
pub(crate) fn begin_rename(preset_ui: &mut crate::pickers::picker_state::PresetUi, preset: &BrushPreset) {
    preset_ui.renaming = Some(("brush-picker".to_string(), crate::pickers::brush_browser::preview_identity(preset), preset.name.clone()));
}

/// Run what the picker asked for.
pub fn apply(app: &mut PhotocraftApp, ctx: &egui::Context, picks: Vec<Pick>) {
    for pick in picks {
        match pick {
            Pick::List(a) => brushes_tab::apply(app, vec![a]),
            Pick::OpenSettings => open_settings(app, ctx),
            // Photoshop asks for the new preset's name and what it captures first, in a dialog that
            // takes the picker's place.
            Pick::NewPreset => {
                close(&mut app.ui);
                crate::brush_preset_dialog::open(app, None);
            }
            Pick::Import => {
                let _ = crate::preset_files_ui::import_brushes(app);
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

/// The picker's contents: Size, Hardness (round tips only: a sampled tip has none), a search
/// field with the Brush Settings, New Preset and gear buttons, the preset list, and the footer
/// slider that scales the cards ([`scale_slider`]). Size and Hardness sit in a block flush with
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
    preset_ui: &mut crate::pickers::picker_state::PresetUi,
    size: egui::Vec2,
) -> Vec<Pick> {
    let t = Tokens::get(ui.ctx());
    let preset_ui = std::cell::RefCell::new(preset_ui);
    let picks = std::cell::RefCell::new(Vec::new());
    // The picker is its dragged (or remembered) size: at least that tall even when every group
    // is closed, so reopening one has the room to show it.
    ui.set_width(size.x);
    ui.set_min_height(size.y);
    let top = ui.cursor().min.y;
    // These are brush-tool controls, not part of the shared picker shell. Keep them stacked above
    // the shell so its menu/search/browser layout stays consistent with other pickers.
    ui.vertical(|ui| {
        ui.set_max_width(ui.available_width().min(HEAD_MAX_W));
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
    let controls_h = (ui.cursor().min.y - top).max(0.0);
    let mut menu = |ui: &mut egui::Ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            gear_menu(ui, current, presets, &mut preset_ui.borrow_mut(), &mut picks.borrow_mut());
            let new_tip = "Create new brush preset from the current settings";
            let settings_tip = "Brush Settings…  (F5): every option of this brush";
            if named(icons::button(ui, "square-plus", 24.0, false, tl!(new_tip)), "New Brush Preset").clicked() {
                picks.borrow_mut().push(Pick::NewPreset);
            }
            if named(icons::button(ui, "sliders-horizontal", 24.0, false, tl!(settings_tip)), "Brush Settings…").clicked() {
                picks.borrow_mut().push(Pick::OpenSettings);
            }
        });
    };
    let mut search = |ui: &mut egui::Ui| {
        ui.horizontal(|ui| {
            icons::paint(ui, egui::Rect::from_min_size(ui.cursor().min + vec2(0.0, 3.0), vec2(16.0, 16.0)), "search", 14.0, t.text_faint);
            ui.add_space(20.0);
            widgets::text_field(
                ui,
                ui.id().with("brush-picker-search"),
                preset_ui.borrow_mut().search.entry("brush-picker".to_string()).or_default(),
                tl!("Search Brushes"),
                (size.x - 40.0).max(60.0),
                false,
            );
        });
    };
    let mut footer = |ui: &mut egui::Ui| {
        ui.add_space((size.y - (ui.cursor().min.y - top) - SLIDER_H).max(FOOTER_GAP));
        // The browser is inset inside the picker shell; subtract that inset so the slider's cap
        // matches the width available to its cards rather than the outer frame.
        scale_slider(ui, &mut preset_ui.borrow_mut().brush_picker, (size.x - 16.0).max(0.0));
    };
    let show_search = { preset_ui.borrow().search_visible.get("brush-picker").copied().unwrap_or(true) };

    let list_picks = crate::pickers::picker_shell::picker_shell_custom(
        ui,
        (size.y - controls_h).max(0.0),
        crate::pickers::picker_shell::CustomShellOptions {
            panel_menu: Some(&mut menu),
            search: if show_search { Some(&mut search) } else { None },
            footer: Some((&mut footer, FOOTER_H)),
            max_total_height: Some((size.y - controls_h).max(0.0)),
            ..Default::default()
        },
        |ui, allocated_h| {
            let actions = { crate::pickers::brush_browser::picker_browser(ui, presets, current, &mut preset_ui.borrow_mut(), (allocated_h - 4.0).max(0.0)) };
            actions.into_iter().map(Pick::List).collect::<Vec<_>>()
        },
    );
    picks.borrow_mut().extend(list_picks);
    picks.into_inner()
}

/// The footer's card-scale slider: how big the preset cards are drawn (1 the standard, 2 the
/// largest), bare like the Size and Hardness sliders — a readout would be noise for a control
/// that's set by feel. The value snaps to hundredths and runs over [`SCALE_RANGE`], from 0.15,
/// under which the cards' padding and rows stop making sense; 0.40 and below is where
/// [`brushes_tab`]'s cards go compact, the tips dropping their size numbers (the text itself
/// holds its size at every scale).
fn scale_slider(ui: &mut egui::Ui, st: &mut BrushPickerView, container_width: f32) {
    let w = (ui.available_width() - GRIP_ROOM).max(60.0);
    ui.allocate_ui_with_layout(vec2(w, SLIDER_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        let show = (st.show_name, st.show_stroke, st.show_tip);
        let max_scale = crate::pickers::brush_browser::max_picker_card_scale(container_width, show);
        st.scale = st.scale.clamp(*SCALE_RANGE.start(), max_scale);
        let resp = widgets::slider(ui, &mut st.scale, *SCALE_RANGE.start()..=max_scale, None);
        if resp.changed() {
            st.scale = (st.scale * 100.0).round() / 100.0;
        }
        dismiss_popup(&resp);
    });
}

/// Closes any open popup (the gear menu) as soon as `resp` is grabbed: a slider or grip drag never
/// counts as a click, so the menu's [`egui::PopupCloseBehavior::CloseOnClickOutside`] alone leaves
/// it up — and the picker's own interaction pulls its layer in front, sinking the menu behind it.
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
    // The gear menu can extend over the bottom-right corner. Give the grip its own top interaction
    // layer so it remains draggable there and can dismiss that menu, rather than letting the popup
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

/// The gear: Photoshop's picker menu. A new preset, rename or delete the current one, the list's
/// view, and Import Brushes.
fn gear_menu(ui: &mut egui::Ui, current: Option<&str>, presets: &[BrushPreset], preset_ui: &mut crate::pickers::picker_state::PresetUi, picks: &mut Vec<Pick>) {
    let gear = named(crate::pickers::picker_shell::picker_menu_button(ui, "Brush Preset Options"), "Brush Preset Options");
    // The menu hangs 30 pt right of the gear, off the button's own edge; the align fallbacks still
    // keep it on screen near the window's edge.
    let anchor = match egui::PopupAnchor::from(&gear) {
        egui::PopupAnchor::ParentRect(rect) => egui::PopupAnchor::ParentRect(rect.translate(vec2(30.0, 0.0))),
        other => other,
    };
    egui::Popup::menu(&gear).anchor(anchor).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        if ui.button(tl!("New Brush Preset…")).clicked() {
            picks.push(Pick::NewPreset);
            ui.close();
        }
        ui.separator();
        let ambiguous = current.is_some_and(|name| crate::pickers::brush_browser::is_ambiguous_preset_name(presets, name));
        let current = current.filter(|_| !ambiguous).and_then(|name| presets.iter().find(|preset| preset.name.eq_ignore_ascii_case(name)));
        let rename = ui.add_enabled(current.is_some(), egui::Button::new(tl!("Rename Brush…")));
        if ambiguous {
            rename.clone().on_hover_text("Rename unavailable because multiple brushes share this name");
        }
        if rename.clicked()
            && let Some(preset) = current
        {
            begin_rename(preset_ui, preset);
            ui.close();
        }
        let delete = ui.add_enabled(current.is_some(), egui::Button::new(tl!("Delete Brush")));
        if ambiguous {
            delete.clone().on_hover_text("Delete unavailable because multiple brushes share this name");
        }
        if delete.clicked()
            && let Some(preset) = current
        {
            picks.push(Pick::List(Action::Delete(preset.name.clone())));
            ui.close();
        }
        ui.separator();
        // What a preset's card shows: a checkbox per part. At least one stays on — clicking the
        // last checked box does nothing, since an empty card would show no brush at all.
        let on = [preset_ui.brush_picker.show_name, preset_ui.brush_picker.show_stroke, preset_ui.brush_picker.show_tip].into_iter().filter(|b| *b).count();
        for (flag, label) in [
            (&mut preset_ui.brush_picker.show_name, tl!("Brush Name")),
            (&mut preset_ui.brush_picker.show_stroke, tl!("Brush Stroke")),
            (&mut preset_ui.brush_picker.show_tip, tl!("Brush Tip")),
        ] {
            let was = *flag;
            if ui.checkbox(flag, label).changed() && was && on == 1 {
                *flag = true;
            }
        }
        ui.separator();
        crate::pickers::picker_shell::search_toggle(ui, preset_ui, "brush-picker");
        ui.separator();
        if ui.button(tl!("Import Brushes…")).clicked() {
            picks.push(Pick::Import);
            ui.close();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::begin_rename;

    #[test]
    fn picker_rename_uses_shared_panel_and_occurrence_identity() {
        let session = photocraft_engine::Session::new();
        let preset = session.tools.presets.first().expect("session has built-in brush presets");
        let expected_key = crate::pickers::brush_browser::preview_identity(preset);
        let expected_name = preset.name.clone();
        let mut state = crate::pickers::picker_state::PresetUi::default();

        begin_rename(&mut state, preset);

        assert_eq!(state.renaming, Some(("brush-picker".into(), expected_key, expected_name.clone())));
        assert_eq!(state.renaming.as_ref().map(|(_, _, text)| text.as_str()), Some(expected_name.as_str()));
    }
}
