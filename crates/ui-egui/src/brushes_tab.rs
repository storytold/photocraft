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
//! Both the Brushes tab and the right-click Brush Picker use the shared asset browser, with
//! specialised brush previews and independent view state for their different layouts.

use egui::{RichText, vec2};
use photocraft_engine::paint::MAX_BRUSH_SIZE;
use serde_json::json;

use crate::brush_panel::{BrushesView, commit_gesture, run_or_status};
use crate::brush_preview;
use crate::pickers::brush_browser::preview_identity;
use crate::pickers::picker_shell::{CustomShellOptions, picker_shell_custom};
use crate::theme::Tokens;
use crate::{PhotocraftApp, icons, widgets};

#[cfg(test)]
pub(crate) use crate::pickers::brush_browser::browser_group_views;
pub use crate::pickers::brush_browser::{
    Action, GroupNode, ListLayout, MAX_VIEW_DEPTH, Node, PANEL_LIST, apply, drop_target, folder_drop, folder_tree, folder_view_key, group_key, preset_list,
};
#[cfg(test)]
pub(crate) use crate::pickers::brush_browser::{
    CARD_COMPACT_SCALE, CARD_GAP, CARD_MAX_H, CARD_MAX_W, CARD_MIN_H_SCALE, CARD_NAME_H, CARD_NUM_FONT, CARD_PAD, CARD_TIP_NAME_H, CARD_TIP_NAME_W,
    CARD_TIP_ONLY_H, CARD_TIP_ONLY_W, CARD_TIP_W, GRID_GAP, card_tip_rect, grid_columns, remap_collapsed_prefix, remove_collapsed_prefix,
};

fn responsive_list_layout(ui: &egui::Ui) -> ListLayout {
    let height = ui.available_height().clamp(180.0, 640.0);
    ListLayout { max_height: height, ..PANEL_LIST }
}

/// The Brushes tab.
pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let inner_w = ui.available_width().max(180.0);
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
    let filter = std::cell::RefCell::new(app.ui.brushes_panel.filter.clone());
    let view = std::cell::Cell::new(app.ui.brushes_panel.view);
    let mut search = |ui: &mut egui::Ui| {
        ui.horizontal(|ui| {
            icons::paint(ui, egui::Rect::from_min_size(ui.cursor().min + vec2(0.0, 3.0), vec2(16.0, 16.0)), "search", 14.0, t.text_faint);
            ui.add_space(20.0);
            let search_w = (inner_w - 112.0).clamp(90.0, inner_w - 40.0);
            widgets::text_field(ui, ui.id().with("brush-search"), &mut filter.borrow_mut(), tl!("Search Brushes"), search_w, false);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let grid_tip = "Grid view";
                let list_tip = "List view";
                if icons::button(ui, "grid-2x2", 24.0, view.get() == BrushesView::Grid, tl!(grid_tip)).clicked() {
                    view.set(BrushesView::Grid);
                }
                if icons::button(ui, "align-justify", 24.0, view.get() == BrushesView::List, tl!(list_tip)).clicked() {
                    view.set(BrushesView::List);
                }
            });
        });
    };
    let acts = picker_shell_custom(ui, layout.max_height, CustomShellOptions { search: Some(&mut search), ..Default::default() }, |ui, allocated_h| {
        app.ui.brushes_panel.filter = filter.borrow().clone();
        app.ui.brushes_panel.view = view.get();
        preset_list(
            ui,
            &app.session.tools.presets,
            app.session.tools.current_preset.as_deref(),
            &mut app.ui.brushes_panel,
            &mut app.ui.presets_ui,
            ListLayout { max_height: allocated_h, ..layout },
        )
    });
    apply(app, acts);
    ui.add_space(4.0);
    widgets::hairline(ui);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(crate::i18n::trn(crate::i18n::current(), app.session.tools.presets.len() as u64, "{n} preset", "{n} presets")).color(t.text_faint),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // The current preset by identity: editing the brush after picking must not deselect it.
            let current = app.session.tools.current_preset.clone().filter(|n| app.session.tools.presets.iter().any(|p| p.name.eq_ignore_ascii_case(n)));
            if ui.add_enabled_ui(current.is_some(), |ui| icons::button(ui, "trash", 24.0, false, "Delete brush")).inner.clicked()
                && let Some(name) = current
            {
                run_or_status(app, "brush.presets.delete", json!({ "name": name }));
            }
            let new = icons::button(ui, "square-plus", 24.0, false, "Create new brush from the current settings");
            if crate::pickers::brush_picker::named(new, "Create new brush from the current settings").clicked() {
                crate::brush_preset_dialog::open(app, None);
            }
            if icons::button(ui, "folder-open", 24.0, false, "Import Brushes… (.abr or image)").clicked() {
                let _ = crate::preset_files_ui::import_brushes(app);
            }
        });
    });
    // Forget previews of presets that no longer exist.
    let live_identities: std::collections::HashSet<String> = app.session.tools.presets.iter().map(preview_identity).collect();
    brush_preview::with_cache(ui.ctx(), |c| {
        c.retain(|slot| slot.split_once(':').is_none_or(|(_, identity)| live_identities.contains(identity)));
    });
}

#[cfg(test)]
#[path = "brushes_tab_tests.rs"]
mod tests;
