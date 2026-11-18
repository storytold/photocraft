//! What the Brush Settings panel's events do: section events edit the brush copy the panel works
//! on, buttons run the commands PhotoCraft's own palette runs. The brush edits leave as one
//! `tools.setBrush` per gesture (`brush_panel::commit_gesture`).

use craft::draw::ui::brush_settings::{BrushPanelAction, BrushSectionEvent};
use photocraft_engine::BrushSettings;
use serde_json::{Value, json};

use super::brush_panel::{section_flag, section_lock};
use crate::PhotocraftApp;

/// Applies the section list's events: edits to `brush`, a clicked pane row to `shown`. True when the "Brushes" link was clicked.
pub(crate) fn apply_section_events(brush: &mut BrushSettings, shown: &mut usize, events: Vec<BrushSectionEvent<usize>>) -> bool {
    let mut open_brushes = false;
    for event in events {
        match event {
            BrushSectionEvent::Enable { key, on, .. } | BrushSectionEvent::Switch { key, on, .. } => {
                if let Some(flag) = section_flag(brush, key) {
                    *flag = on;
                }
            }
            BrushSectionEvent::Lock { key, on, .. } => {
                if let Some(lock) = section_lock(brush, key) {
                    *lock = on;
                }
            }
            BrushSectionEvent::Select(key) => *shown = key,
            BrushSectionEvent::OpenLink => open_brushes = true,
            _ => {}
        }
    }
    open_brushes
}

/// The brush after Reset: the defaults, keeping the colours, smoothing and locks.
pub(crate) fn reset(brush: &BrushSettings) -> BrushSettings {
    BrushSettings { color: brush.color, background: brush.background, smoothing: brush.smoothing.clone(), locks: brush.locks.clone(), ..Default::default() }
}

/// Runs a button beside the stroke strip on `brush` (the panel's working copy).
pub(crate) fn apply_action(app: &mut PhotocraftApp, brush: &mut BrushSettings, action: BrushPanelAction) {
    match action {
        BrushPanelAction::NewPreset => {
            crate::brush_preset_dialog::open(app, Some(brush));
        }
        BrushPanelAction::Reset => *brush = reset(brush),
        BrushPanelAction::UpdatePreset => {
            super::brush_panel::run_or_status(app, "brush.presets.update", json!({ "brush": serde_json::to_value(&*brush).unwrap_or(Value::Null) }));
        }
        _ => {}
    }
}
