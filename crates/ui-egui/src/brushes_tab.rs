//! Window › Brushes: the brush presets in collapsible groups, as a list (tip, size, stroke
//! preview, name) or a grid of tips, with a size slider and a search field.
//!
//! Presets and groups are organised like Photoshop's panel: drag a preset within its group or
//! into another group or folder (onto a preset, or onto a header to append), drag a group's or
//! folder's header onto another header to put it inside that folder (or onto the header's upper
//! edge to place it before, which reorders siblings and moves a folder out to the top level), and
//! right-click for Rename and Delete. Groups hold nested folders (for example, folders imported
//! from an `.abr` file), drawn indented under their group and opened or
//! closed like groups. Every change is a `brush.presets.*` command, so it is journaled, drivable,
//! and persisted by the preset store.
//!
//! Both the Brushes tab and the right-click Brush Picker use the shared asset browser and its
//! standard card/list layout, with brush-tip thumbnails supplied by the adapter.

use crate::brush_panel::commit_gesture;
use crate::brush_preview;
use crate::pickers::adapters::brushes::preview_identity;
use crate::theme::Tokens;
use crate::{PhotocraftApp, widgets};
use egui::{RichText, vec2};
use photocraft_engine::paint::MAX_BRUSH_SIZE;

pub use crate::pickers::adapters::brushes::{
    Action, ListLayout, PANEL_LIST, apply, folder_view_key, group_key, preset_list,
};
fn responsive_list_layout(ui: &egui::Ui) -> ListLayout {
    let height = ui.available_height().clamp(180.0, 640.0);
    ListLayout { max_height: height, ..PANEL_LIST }
}

/// The Brushes tab.
pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    // Size of the current brush (Photoshop's Brushes panel slider).
    let before = app.session.tools.brush.clone();
    let mut b = before.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Size")).color(t.text_dim));
        let slider_w = (ui.available_width() - 90.0).clamp(100.0, 260.0);
        let mut lv = b.size.max(1.0).ln();
        ui.add_sized(vec2(slider_w, 18.0), |ui: &mut egui::Ui| {
            let r = widgets::slider(ui, &mut lv, 0.0..=MAX_BRUSH_SIZE.ln(), None);
            if r.changed() {
                b.size = lv.exp().round().clamp(1.0, MAX_BRUSH_SIZE);
            }
            r
        });
        let mut s = b.size;
        if widgets::value_field(ui, &mut s, 1.0..=MAX_BRUSH_SIZE, "px", 74.0).changed() {
            b.size = s.round().clamp(1.0, MAX_BRUSH_SIZE);
        }
    });
    commit_gesture(app, ui.ctx(), &before, &b);
    ui.add_space(4.0);
    let layout = responsive_list_layout(ui);
    // The brush adapter owns one complete shared shell: search row, browser viewport and footer.
    if !app.ui.presets_ui.display_modes.contains_key("brushes") {
        let list_layout = app.ui.presets_ui.display_mode("brushes").uses_list_layout();
        app.ui.presets_ui.sizes.insert("brushes".to_owned(), if list_layout {
            crate::pickers::browser::asset_browser::MIN_THUMB
        } else { 40.0 });
    }
    let acts = preset_list(
        ui,
        &app.session.tools.presets,
        app.session.tools.current_preset.as_deref(),
        &mut app.ui.presets_ui,
        layout,
    );
    apply(app, acts);
    ui.add_space(4.0);
    widgets::hairline(ui);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(crate::i18n::trn(crate::i18n::current(), app.session.tools.presets.len() as u64, "{n} preset", "{n} presets")).color(t.text_faint),
        );
    });
    // Forget previews of presets that no longer exist.
    let live_identities: std::collections::HashSet<String> = app.session.tools.presets.iter().map(preview_identity).collect();
    brush_preview::with_cache(ui.ctx(), |c| {
        c.retain(|slot| slot.split_once(':').is_none_or(|(_, identity)| live_identities.contains(identity)));
    });
}
