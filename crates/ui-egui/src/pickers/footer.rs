//! Standard preset-picker footer controls, independent of the shell's viewport sizing.

use egui::vec2;

use crate::pickers::state::PresetUi;

/// Optional standard-picker footer controls, configured by the shell's consumer.
pub(crate) struct FooterOptions<'a> {
    pub(crate) new_tip: &'a str,
    pub(crate) size_slider: bool,
    pub(crate) new_group: bool,
    pub(crate) new_item: bool,
    pub(crate) delete: bool,
}

impl<'a> FooterOptions<'a> {
    /// Enable the standard new-item, new-group, and delete controls for a picker footer.
    pub(crate) fn standard(new_tip: &'a str) -> Self {
        Self { new_tip, size_slider: false, new_group: true, new_item: true, delete: true }
    }
}

/// User intent produced by the shared footer, before the shell translates it into browser events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FooterAction {
    DeleteItem { group: String, group_path: Vec<String>, key: String },
    DeleteSelection,
    New,
    NewGroup,
}

fn delete_action(st: &PresetUi, panel: &str) -> Option<FooterAction> {
    if panel == "gradients" {
        return st
            .browser_selection
            .get(panel)
            .filter(|selection| !selection.is_empty())
            .map(|_| FooterAction::DeleteSelection);
    }
    if let Some(selection) = st.browser_selection.get(panel) {
        return (!selection.is_empty()).then_some(FooterAction::DeleteSelection);
    }
    let key = st.selected.get(panel)?.clone();
    let group = st.selected_group.get(panel)?.clone();
    let group_path = st.selected_group_path.get(panel)?.clone();
    Some(FooterAction::DeleteItem { group, group_path, key })
}

/// Draw the standard footer controls and return requested actions without dispatching them.
pub(crate) fn draw(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str, footer: FooterOptions<'_>) -> Vec<FooterAction> {
    let mut actions = Vec::new();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if footer.size_slider {
            let size = *st.sizes.get(panel).unwrap_or(&40.0);
            let mut next = size;
            ui.add_sized(vec2(90.0, 18.0), egui::Slider::new(&mut next, crate::pickers::browser::asset_browser::MIN_THUMB..=96.0).show_value(false))
                .on_hover_text(tl!("Thumbnail size (smallest: list)"));
            if (next - size).abs() > 0.01 {
                st.sizes.insert(panel.to_string(), next);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if footer.delete {
                let delete = delete_action(st, panel);
                let response = ui.add_enabled_ui(delete.is_some(), |ui| crate::icons::button(ui, "trash", 24.0, false, tl!("Delete"))).inner;
                if response.clicked() {
                    if let Some(action) = delete {
                        actions.push(action);
                    }
                }
            }
            if footer.new_item && crate::icons::button(ui, "plus", 24.0, false, footer.new_tip).clicked() {
                actions.push(FooterAction::New);
            }
            if footer.new_group && crate::icons::button(ui, "folder-plus", 24.0, false, tl!("Create new group")).clicked() {
                actions.push(FooterAction::NewGroup);
            }
        });
    });
    actions
}

#[cfg(test)]
mod tests {
    use super::{FooterAction, delete_action};
    use crate::pickers::browser::path_identity_model::PickerId;
    use crate::pickers::state::PresetUi;

    #[test]
    fn footer_delete_prefers_the_complete_browser_selection() {
        let mut state = PresetUi::default();
        state.set_selected_item("panel", "A", &[], "active");
        state.select_browser_node("panel", PickerId::group("A"), false);
        state.select_browser_node("panel", PickerId::item("B", "other"), true);

        assert!(matches!(delete_action(&state, "panel"), Some(FooterAction::DeleteSelection)));
    }

    #[test]
    fn footer_delete_falls_back_to_the_active_item_without_a_browser_selection() {
        let mut state = PresetUi::default();
        state.set_selected_item("panel", "A", &["Folder".into()], "active");

        assert!(matches!(
            delete_action(&state, "panel"),
            Some(FooterAction::DeleteItem { group, group_path, key })
                if group == "A" && group_path == ["Folder"] && key == "active"
        ));
    }

    #[test]
    fn footer_delete_is_disabled_after_the_browser_selection_is_cleared() {
        let mut state = PresetUi::default();
        state.set_selected_item("gradients", "A", &[], "active");
        state.browser_selection.insert("gradients".into(), Default::default());

        assert_eq!(delete_action(&state, "gradients"), None);
    }

    #[test]
    fn gradient_footer_delete_requires_a_browser_selection_even_if_an_active_preset_exists() {
        let mut state = PresetUi::default();
        state.set_selected_item("gradients", "A", &[], "active");

        assert_eq!(delete_action(&state, "gradients"), None);
    }
}
