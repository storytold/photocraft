//! Standard preset-picker footer controls, independent of the shell's viewport sizing.

use egui::vec2;

use crate::pickers::picker_state::PresetUi;

/// Optional standard-picker footer controls, configured by the shell's consumer.
pub(crate) struct FooterOptions<'a> {
    pub(crate) new_tip: &'a str,
    pub(crate) size_slider: bool,
    pub(crate) new_group: bool,
    pub(crate) new_item: bool,
    pub(crate) delete: bool,
}

impl<'a> FooterOptions<'a> {
    /// Enable the standard new-item, new-group, delete, and size controls for a picker footer.
    pub(crate) fn standard(new_tip: &'a str) -> Self {
        Self { new_tip, size_slider: true, new_group: true, new_item: true, delete: true }
    }
}

/// User intent produced by the shared footer, before the shell translates it into browser events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FooterAction {
    DeleteItem { group: String, group_path: Vec<String>, key: String },
    New,
    NewGroup,
}

/// Draw the standard footer controls and return requested actions without dispatching them.
pub(crate) fn draw(ui: &mut egui::Ui, st: &mut PresetUi, panel: &str, footer: FooterOptions<'_>) -> Vec<FooterAction> {
    let mut actions = Vec::new();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if footer.size_slider {
            let size = *st.sizes.get(panel).unwrap_or(&40.0);
            let mut next = size;
            ui.add_sized(vec2(90.0, 18.0), egui::Slider::new(&mut next, crate::pickers::asset_browser::MIN_THUMB..=96.0).show_value(false))
                .on_hover_text(tl!("Thumbnail size (smallest: list)"));
            if (next - size).abs() > 0.01 {
                st.sizes.insert(panel.to_string(), next);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if footer.delete
                && crate::icons::button(ui, "trash", 24.0, false, tl!("Delete")).clicked()
                && let Some(key) = st.selected.get(panel).cloned()
                && let (Some(group), Some(group_path)) = (st.selected_group.get(panel).cloned(), st.selected_group_path.get(panel).cloned())
            {
                actions.push(FooterAction::DeleteItem { group, group_path, key });
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
