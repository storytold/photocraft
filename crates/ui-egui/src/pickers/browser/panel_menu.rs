//! Asset-browser contributions to the shared docking panel menu.

use crate::PhotocraftApp;

/// Render the shared asset picker's menu points and its domain-specific extension.
///
/// The generic display/search controls live in `picker_panel_menu`; this module connects those
/// controls to the asset-browser panels and their domain-owned import/export/reset actions.
/// Taking picker state temporarily keeps its borrow independent from domain actions on the app.
pub(crate) fn shared_picker_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui, panel: &str) -> bool {
    if !crate::pickers::panels::shared_docking(&format!("window.panel.{panel}")) {
        return false;
    }
    let mut state = std::mem::take(&mut app.ui.presets_ui);
    let mut extension = |ui: &mut egui::Ui, state: &mut crate::pickers::state::PresetUi, _panel: &str| match panel {
        "gradients" => crate::pickers::adapters::gradients::panel_menu(app, ui),
        "patterns" => crate::pickers::adapters::patterns::panel_menu(app, ui),
        "swatches" => crate::pickers::adapters::swatches::panel_menu(app, ui, state),
        _ => {}
    };
    let extension = matches!(panel, "gradients" | "patterns" | "swatches").then_some(&mut extension as crate::pickers::shell::MenuContentsCallback<'_>);
    crate::pickers::shell::picker_menu_contents(ui, &mut state, panel, crate::pickers::shell::PickerMenuOptions::for_panel(panel), extension);
    app.ui.presets_ui = state;
    true
}
