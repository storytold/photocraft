//! Shared menu contents for picker panels.

use crate::pickers::state::{PickerDisplayMode, PresetUi};

pub(crate) type MenuContentsCallback<'a> = &'a mut dyn FnMut(&mut egui::Ui, &mut PresetUi, &str);

const PANEL_MENU_ALIGN_ALTERNATIVES: &[egui::RectAlign] = &[egui::RectAlign::RIGHT_END];

/// Align the popup's left edge to the trigger's right edge, flipping vertically if needed.
pub(crate) fn panel_menu_popup(response: &egui::Response) -> egui::Popup<'_> {
    let popup = egui::Popup::menu(response);
    if popup.get_expected_size().is_some_and(|size| response.rect.top() + size.y > response.ctx.content_rect().bottom()) {
        popup.align(egui::RectAlign::RIGHT_END)
    } else {
        popup.align(egui::RectAlign::RIGHT_START).align_alternatives(PANEL_MENU_ALIGN_ALTERNATIVES)
    }
}

/// Shared menu points offered by an asset-picker shell. Consumers choose capabilities, never
/// ordering or separators; domain actions are appended through the final extension point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PickerMenuOptions {
    pub(crate) display: bool,
    pub(crate) wrap_list: bool,
    pub(crate) search_visibility: bool,
    pub(crate) recents_visibility: bool,
}

impl PickerMenuOptions {
    pub(crate) fn for_panel(panel: &str) -> Self {
        Self {
            display: true,
            wrap_list: true,
            search_visibility: matches!(panel, "gradients" | "patterns" | "swatches"),
            recents_visibility: matches!(panel, "gradients" | "patterns" | "swatches"),
        }
    }
}

#[cfg(test)]
pub(crate) fn picker_menu_popup(ui: &mut egui::Ui, label: &str, anchor_offset: egui::Vec2, contents: impl FnOnce(&mut egui::Ui)) -> egui::Response {
    let button = crate::widgets::panel_menu_button(ui, label);
    let anchor = match egui::PopupAnchor::from(&button) {
        egui::PopupAnchor::ParentRect(rect) if anchor_offset != egui::Vec2::ZERO => egui::PopupAnchor::ParentRect(rect.translate(anchor_offset)),
        other => other,
    };
    panel_menu_popup(&button).anchor(anchor).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        crate::menu_keys::with_menu_item_indent(ui, |ui| {
            crate::widgets::style_spectrum_popup_menu(ui);
            contents(ui);
        })
    });
    button
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

/// Wrapping is meaningful only in list modes, but remains visible so the menu layout is stable.
fn wrap_list_item(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str) {
    let enabled = st.display_mode(panel).is_list();
    let checked = st.wrap_list.get(panel).copied().unwrap_or(true);
    if menu_check_item(ui, checked, tl!("Wrap List"), enabled).clicked() {
        st.wrap_list.insert(panel.to_owned(), !checked);
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
    let visible = st.search_visible.get(panel).copied().unwrap_or(true);
    if menu_check_item(ui, visible, tl!("Show Search Bar"), true).clicked() {
        let visible = !visible;
        st.search_visible.insert(panel.to_owned(), visible);
        if !visible {
            st.search.remove(panel);
        }
        ui.close();
    }
}

/// Common visibility control for the standard pickers' recent-items strip.
fn recents_toggle(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str) {
    let visible = st.recents_visible(panel);
    if menu_check_item(ui, visible, tl!("Show Recents"), true).clicked() {
        let visible = !visible;
        st.recents_visible.insert(panel.to_owned(), visible);
        ui.close();
    }
}

/// Compose a preset picker's menu. The shell owns shared points, their order, and section boundary;
/// consumers contribute only their optional domain-specific entries.
pub(crate) fn picker_menu_contents(ui: &mut egui::Ui, state: &mut PresetUi, panel: &str, options: PickerMenuOptions, extra: Option<MenuContentsCallback<'_>>) {
    picker_menu_contents_ordered(ui, state, panel, options, None, extra);
}

/// Compose a menu with browser actions before display controls and adapter extensions after
/// visibility controls. This is the canonical ordering for shared asset browsers.
pub(crate) fn picker_menu_contents_ordered(
    ui: &mut egui::Ui,
    state: &mut PresetUi,
    panel: &str,
    options: PickerMenuOptions,
    leading: Option<MenuContentsCallback<'_>>,
    trailing: Option<MenuContentsCallback<'_>>,
) {
    if let Some(leading) = leading {
        leading(ui, state, panel);
    }
    let mut has_shared_points = false;
    if options.display {
        display_mode_items(ui, state, panel);
        has_shared_points = true;
    }
    if options.wrap_list {
        if has_shared_points {
            ui.separator();
        }
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
    if options.recents_visibility {
        recents_toggle(ui, state, panel);
        has_shared_points = true;
    }
    // A single boundary separates visibility controls from domain-specific append/import actions.
    if let Some(trailing) = trailing {
        if has_shared_points {
            ui.separator();
        }
        trailing(ui, state, panel);
    }
}
