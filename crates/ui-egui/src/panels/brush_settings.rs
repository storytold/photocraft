//! The Brush Settings tab on the framework's `BrushSettingsPanel`: the section list, the shown
//! section's controls (`brush_sections`) and the stroke strip.

use craft::draw::ui::brush_settings::{BrushSection, BrushSettingsPanel, BrushStrokeStrip, StripImage};
use craft::ui::contract::Bind;
use craft::ui::primitives::Revision;
use egui::vec2;
use photocraft_engine::BrushSettings;

use super::brush_panel::{CHECKBOX_ONLY_START, SECTIONS, section_flag, section_lock};
use super::brush_sections::SectionOut;
use super::brush_settings_events::{apply_action, apply_section_events};
use super::{brush_preview, brush_sections};
use crate::PhotocraftApp;
use crate::theme::Tokens;

/// The panel's height inside the floating window.
const HEIGHT: f32 = 586.0;

/// The section rows for `brush`: switches from [`CHECKBOX_ONLY_START`], an enable box and a lock
/// on every section but Brush Tip Shape.
fn sections(brush: &mut BrushSettings) -> Vec<BrushSection<usize>> {
    (0..SECTIONS.len())
        .map(|i| {
            let name = tl!(SECTIONS[i].0);
            let on = section_flag(brush, i).map(|flag| *flag);
            let row = match (i >= CHECKBOX_ONLY_START, on) {
                (true, on) => BrushSection::switch(i, name, on.unwrap_or(false)),
                (false, Some(on)) => BrushSection::pane(i, name).enabled(on),
                (false, None) => BrushSection::pane(i, name),
            };
            match section_lock(brush, i) {
                Some(lock) => row.locked(*lock),
                None => row,
            }
        })
        .collect()
}

pub(super) fn brush_settings(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let text = Tokens::get(&ctx).text;
    let before = (*app.session.tools.brush).clone();
    let mut brush = before.clone();
    // Older UI state and control clients could select any of the 13 entries. Switch rows have
    // no pane, so those selections recover to Brush Tip Shape.
    if app.ui.brush_section >= CHECKBOX_ONLY_START {
        app.ui.brush_section = 0;
    }
    let rows = sections(&mut brush);
    let can_update =
        app.session.tools.current_preset.as_deref().is_some_and(|name| app.session.tools.presets.iter().any(|p| p.name.eq_ignore_ascii_case(name)));
    let strip = BrushStrokeStrip::new(
        tl!("Stroke preview"),
        Revision::of(&(brush_preview::preview_sig(&before), text)),
        StripImage::new(|size| brush_preview::stroke_image(&before, size, text)),
    );
    let tool = app.ui.tool;
    // The section is bound through a copy so the pane closure can borrow the whole app.
    let mut section = app.ui.brush_section;
    let panel = BrushSettingsPanel::new(tl!("Brush Settings"), &rows, Bind::bound(&mut section)).link_row().strip(strip);
    let panel = if can_update { panel.can_update() } else { panel };
    // A frame can run the pane twice and egui gives a frame's input events to the first pass only: the reports of
    // every pass add up. The framework panel greys out the pane of a section that is off.
    let mut reported = SectionOut::default();
    let out = ui.allocate_ui(vec2(ui.available_width(), HEIGHT), |ui| {
        panel.show(ui, |ui, _, section| {
            reported.merge(brush_sections::section_body(app, ui, &mut brush, *section, tool));
        })
    });
    let out = out.inner;
    app.ui.brush_section = section;
    if let Some(name) = reported.picked.take() {
        super::brush_panel::run_or_status(app, "brush.presets.setCurrent", serde_json::json!({ "name": name }));
    }
    if apply_section_events(&mut brush, &mut app.ui.brush_section, out.sections) {
        let _ = crate::menus::invoke(app, &ctx, "window.panel.brushes", serde_json::json!({}));
    }
    if let Some(action) = out.action {
        apply_action(app, &mut brush, action);
    }
    super::brush_panel::commit_keyed(app, &before, &brush, reported.key);
}
