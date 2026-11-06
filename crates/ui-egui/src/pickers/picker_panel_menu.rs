//! Shared menu contents for picker panels.

use crate::pickers::state::{PickerDisplayMode, PresetUi};

pub(crate) type MenuContentsCallback<'a> = &'a mut dyn FnMut(&mut egui::Ui, &mut PresetUi, &str);

/// Shared menu points offered by an asset-picker shell. Consumers choose capabilities, never
/// ordering or separators; domain actions are appended through the final extension point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PickerMenuOptions {
    pub(crate) display: bool,
    pub(crate) search_visibility: bool,
}

impl PickerMenuOptions {
    pub(crate) fn for_panel(panel: &str) -> Self {
        Self { display: true, search_visibility: matches!(panel, "gradients" | "patterns" | "swatches" | "brush-picker") }
    }
}

#[cfg(test)]
pub(crate) fn picker_menu_popup(ui: &mut egui::Ui, label: &str, anchor_offset: egui::Vec2, contents: impl FnOnce(&mut egui::Ui)) {
    let button = crate::widgets::panel_menu_button(ui, label);
    let anchor = match egui::PopupAnchor::from(&button) {
        egui::PopupAnchor::ParentRect(rect) if anchor_offset != egui::Vec2::ZERO => egui::PopupAnchor::ParentRect(rect.translate(anchor_offset)),
        other => other,
    };
    egui::Popup::menu(&button).anchor(anchor).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        crate::widgets::style_spectrum_popup_menu(ui);
        contents(ui);
    });
}

/// Render the shared display controls without owning section boundaries; the composer owns them.
fn display_mode_items(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str) {
    let selected = st.display_mode(panel);
    for mode in PickerDisplayMode::ALL {
        let label = match mode {
            PickerDisplayMode::TextOnly => tl!("Text Only"),
            PickerDisplayMode::SmallThumbnail => tl!("Small Thumbnail"),
            PickerDisplayMode::LargeThumbnail => tl!("Large Thumbnail"),
            PickerDisplayMode::SmallList => tl!("Small List"),
            PickerDisplayMode::LargeList => tl!("Large List"),
        };
        if menu_check_item(ui, selected == mode, label, true).clicked() {
            st.display_modes.insert(panel.to_owned(), mode);
            ui.close();
        }
    }
}

fn wrap_list_item(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str) {
    let selected = st.display_mode(panel);
    let list_mode = selected.is_list();
    let wrap = st.wrap_list.get(panel).copied().unwrap_or(true);
    if menu_check_item(ui, wrap, tl!("Wrap List"), list_mode).clicked() && list_mode {
        st.wrap_list.insert(panel.to_owned(), !wrap);
        ui.close();
    }
}

/// Render a checked menu row with the same fixed checkmark gutter as the application menu bar.
fn menu_check_item(ui: &mut egui::Ui, checked: bool, label: &str, enabled: bool) -> egui::Response {
    let text = crate::menu_keys::menu_row_text(ui, checked, label, None);
    let enabled = enabled && ui.is_enabled();
    let response = ui.add_enabled(enabled, egui::Button::new(text));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    response
}

/// Common visibility control for the standard pickers' search row.
fn search_toggle(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str) {
    let visible = st.search_visible.entry(panel.to_string()).or_insert(true);
    if ui.checkbox(visible, tl!("Show Search Bar")).changed() {
        if !*visible {
            st.search.remove(panel);
        }
        ui.close();
    }
}

/// Compose a preset picker's menu. The shell owns shared points, their order, and section boundary;
/// consumers contribute only their optional domain-specific entries.
pub(crate) fn picker_menu_contents(ui: &mut egui::Ui, state: &mut PresetUi, panel: &str, options: PickerMenuOptions, extra: Option<MenuContentsCallback<'_>>) {
    let mut has_shared_points = false;
    if options.display {
        display_mode_items(ui, state, panel);
        ui.separator();
        wrap_list_item(ui, state, panel);
        has_shared_points = true;
    }
    if options.search_visibility {
        if has_shared_points {
            ui.separator();
        }
        search_toggle(ui, state, panel);
        has_shared_points = true;
    }
    // One shell-owned boundary separates shared controls from domain extensions. The presence of
    // an extension is the source of truth; callers never coordinate a second separator flag.
    if let Some(extra) = extra {
        if has_shared_points {
            ui.separator();
        }
        extra(ui, state, panel);
    }
}
