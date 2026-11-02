//! The only mapping from PhotoCraft chrome metrics/tokens to shared docking defaults.
//! Changes to shared UI are reconciled here and exercised by compatibility tests.
use super::{COMPACT_COLUMN_WIDTH, DOCK_LABELS};

pub(super) const FULL_HEADER_HEIGHT: f32 = 26.0;
pub(super) const HEADER_CONTROL_SIZE: f32 = 20.0;
pub(super) const HEADER_PADDING: f32 = (FULL_HEADER_HEIGHT - HEADER_CONTROL_SIZE) * 0.5;

pub(super) fn style(ui: &egui::Ui) -> craft_ui::docking::DockStyle {
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut style = craft_ui::docking::DockStyle::from_ui(ui);
    style.tab_height = FULL_HEADER_HEIGHT;
    style.title_height = 0.0;
    style.column_bar_height = 0.0;
    style.close_button = false;
    style.iconic_width = COMPACT_COLUMN_WIDTH;
    style.icon_size = 25.0;
    style.iconic_max_width = 240.0;
    style.iconic_label_width = 78.0;
    style.floating_background = t.card;
    style.floating_stroke = egui::Stroke::new(ui.ctx().pixels_per_point().recip(), t.card_border);
    style.min_pane = 0.0;
    // Side-dock chrome owns its seam; floating stack dividers still need a real split.
    style.gap = ui.ctx().pixels_per_point().recip();
    style.dock_gap = Some(0.0);
    style.background = t.card;
    // Studio themes leave the strip transparent over the dock's card; a floating group has no
    // card behind it, so the canvas showed through its titles (#4509).
    style.tab_background = tab_background(&t);
    style.active_background = t.card;
    style.text = t.text;
    style.inactive_text = t.text_dim;
    style.border = egui::Stroke::NONE;
    style.accent = t.accent;
    style.right_to_left = crate::direction::is_rtl();
    style.new_dock_zones = true;
    style.edge_band = 12.0;
    style.column_edge_band = 18.0;
    style.new_dock_band = 18.0;
    style.localized(|key| DOCK_LABELS.iter().find(|(k, _)| *k == key).map(|(_, english)| tl!(english).to_string()))
}

/// The same active-tab and strip treatment used by the shared expanded renderer.
fn tab_background(t: &crate::theme::Tokens) -> egui::Color32 {
    if t.tab_strip.a() == 0 { t.card } else { t.tab_strip }
}

pub(super) fn single_header(ui: &egui::Ui, rect: egui::Rect, label: &str) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(12.0), t.text);
    let tab = egui::Rect::from_min_size(rect.min, egui::vec2((galley.size().x + 24.0).min((rect.width() - 44.0).max(0.0)), rect.height()));
    ui.painter().rect_filled(rect, 0.0, tab_background(&t));
    ui.painter().rect_filled(tab, 0.0, t.card);
    ui.painter().with_clip_rect(tab).galley(egui::pos2(tab.left() + 12.0, tab.center().y - galley.size().y * 0.5), galley, t.text);
}
