//! Window › Brush Settings (F5) and Window › Brushes: Photoshop's brush panels, drawn by the dock.
//!
//! Brush Settings: the section list with enable boxes on the left (clicking a controls section
//! shows and enables it; the last five rows only toggle), the selected section's controls on the right (see [`crate::brush_sections`]),
//! and a live stroke preview strip below, rendered with the real brush engine and cached on the
//! settings (it re-renders only when they change). Brushes ([`brushes`]): the presets in
//! collapsible groups with tip thumbnails and stroke previews, a size field and a search field.
//!
//! The panels never write the session brush themselves: each frame's edits become one
//! `tools.setBrush` call carrying only the changed fields (Rule 1), so the journal, the control
//! channel and the MCP server see exactly what the panel did and can do the same. A drag's calls
//! share a `coalesce` key, so a whole gesture is one journal entry ([`commit_keyed`]).
//!
//! The lock beside a section name keeps that section's settings when another preset is picked
//! (`locks` in the brush, applied by the engine).

use craft::ui::contract::CommandSet;
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::{CommandRef, CommandState};
use photocraft_engine::BrushSettings;
use photocraft_engine::paint;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

pub(crate) mod brushes;

pub use crate::brush_preview::{preview_pixels, preview_sig};
pub use photocraft_engine::brush_cmds::brush_patch;

/// Sections in Photoshop's order. The bool says whether the section has an enable box.
pub const SECTIONS: [(&str, bool); 13] = [
    ("Brush Tip Shape", false),
    ("Shape Dynamics", true),
    ("Scattering", true),
    ("Texture", true),
    ("Dual Brush", true),
    ("Color Dynamics", true),
    ("Transfer", true),
    ("Brush Pose", true),
    ("Noise", true),
    ("Wet Edges", true),
    ("Build-up", true),
    ("Smoothing", true),
    ("Protect Texture", true),
];

/// The final five entries are switches, not sections with a properties pane.
pub(super) const CHECKBOX_ONLY_START: usize = 8;

/// The enable flag behind section `i` (None for Brush Tip Shape).
pub fn section_flag(b: &mut BrushSettings, i: usize) -> Option<&mut bool> {
    Some(match i {
        1 => &mut b.shape_dynamics.enabled,
        2 => &mut b.scattering.enabled,
        3 => &mut b.texture.enabled,
        4 => &mut b.dual_brush.enabled,
        5 => &mut b.color_dynamics.enabled,
        6 => &mut b.transfer.enabled,
        7 => &mut b.pose.enabled,
        8 => &mut b.noise,
        9 => &mut b.wet_edges,
        10 => &mut b.build_up,
        11 => &mut b.smoothing.enabled,
        12 => &mut b.protect_texture,
        _ => return None,
    })
}

/// The lock behind section `i` (every section but Brush Tip Shape has one, as in Photoshop).
pub fn section_lock(b: &mut BrushSettings, i: usize) -> Option<&mut bool> {
    let l = &mut b.locks;
    Some(match i {
        1 => &mut l.shape_dynamics,
        2 => &mut l.scattering,
        3 => &mut l.texture,
        4 => &mut l.dual_brush,
        5 => &mut l.color_dynamics,
        6 => &mut l.transfer,
        7 => &mut l.pose,
        8 => &mut l.noise,
        9 => &mut l.wet_edges,
        10 => &mut l.build_up,
        11 => &mut l.smoothing,
        12 => &mut l.protect_texture,
        _ => return None,
    })
}

/// Send the panel's edits (`before` → `after`) through `tools.setBrush`.
pub fn commit(app: &mut PhotocraftApp, before: &BrushSettings, after: &BrushSettings) {
    send(app, before, after, None);
}

/// [`commit`] as part of an edit gesture: every call of one gesture carries the same `coalesce`
/// `key` (from `crate::history_keys::coalesce` of the edit event; `None` for a step of its own), so
/// the engine journals the gesture as a single `tools.setBrush` (Rule 1: one command per gesture).
pub(crate) fn commit_keyed(app: &mut PhotocraftApp, before: &BrushSettings, after: &BrushSettings, key: Option<String>) {
    send(app, before, after, key);
}

fn send(app: &mut PhotocraftApp, before: &BrushSettings, after: &BrushSettings, key: Option<String>) -> bool {
    if before == after {
        return false;
    }
    let patch = brush_patch(before, after);
    if patch.as_object().is_some_and(Map::is_empty) {
        return false;
    }
    let mut p = json!({ "brush": patch });
    if let Some(k) = key {
        p["coalesce"] = json!(k);
    }
    if let Err(e) = app.run("tools.setBrush", p) {
        app.ui.status = e;
    }
    true
}

/// Group label for presets saved without a group.
pub const UNGROUPED: &str = photocraft_engine::brush_key_cmds::UNGROUPED;

/// A fresh "Brush N" name.
pub(crate) fn new_preset_name(presets: &[paint::BrushPreset]) -> String {
    (1..).map(|n| format!("Brush {n}")).find(|n| !presets.iter().any(|p| &p.name == n)).unwrap_or_else(|| "Brush".into())
}

pub(crate) fn run_or_status(app: &mut PhotocraftApp, id: &str, p: Value) {
    if let Err(e) = app.run(id, p) {
        app.ui.status = e;
    }
}

/// Clear brush controls: turns off every section except Smoothing and resets their options
/// to the defaults (Photoshop), keeping locked sections intact.
pub fn clear_brush_controls(current: &BrushSettings) -> BrushSettings {
    let mut b = current.clone();
    let d = BrushSettings::default();
    let l = &current.locks;
    if !l.shape_dynamics {
        b.shape_dynamics = d.shape_dynamics;
    }
    if !l.scattering {
        b.scattering = d.scattering;
    }
    if !l.texture {
        b.texture = d.texture;
    }
    if !l.dual_brush {
        b.dual_brush = d.dual_brush;
    }
    if !l.color_dynamics {
        b.color_dynamics = d.color_dynamics;
    }
    if !l.transfer {
        b.transfer = d.transfer;
    }
    if !l.pose {
        b.pose = d.pose;
    }
    if !l.noise {
        b.noise = d.noise;
    }
    if !l.wet_edges {
        b.wet_edges = d.wet_edges;
    }
    if !l.build_up {
        b.build_up = d.build_up;
        b.build_up_rate = d.build_up_rate;
    }
    if !l.protect_texture {
        b.protect_texture = d.protect_texture;
    }
    b
}

/// Copy the current brush's Texture settings to other painting tools that have Texture enabled.
pub fn copy_texture_to_other_tools(app: &mut PhotocraftApp) {
    let texture = app.session.tools.brush.texture.clone();
    for (_, brush) in &mut app.ui.tool_brushes {
        if brush.texture.enabled {
            brush.texture = texture.clone();
        }
    }
}

/// The rows of the Brush Settings panel menu that are the panel's own; the dock adds Close and
/// the group rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "brushSettingsPanel", scope = "app")]
pub(crate) enum BrushSettingsCommand {
    /// Open the New Brush Preset dialog.
    #[command(label = "New Brush Preset…")]
    NewPreset,
    /// Turn off every unlocked section except Smoothing and reset its options.
    #[command(label = "Clear Brush Controls")]
    ClearControls,
    /// Unlock every section.
    #[command(label = "Reset All Locked Settings")]
    ResetLocks,
    /// Copy the Texture settings to the other tools that have Texture on.
    #[command(label = "Copy Texture to Other Tools")]
    CopyTexture,
}

/// The Brush Settings panel menu's rows (pn.PC12), in production's order.
pub(crate) fn panel_rows(_app: &PhotocraftApp) -> Vec<PanelRow> {
    let row = |command: BrushSettingsCommand| PanelRow::Command(CommandRef::from(command));
    vec![
        row(BrushSettingsCommand::NewPreset),
        PanelRow::Separator,
        row(BrushSettingsCommand::ClearControls),
        row(BrushSettingsCommand::ResetLocks),
        row(BrushSettingsCommand::CopyTexture),
    ]
}

/// What the menu's commands may do now: Reset needs a lock, Copy Texture needs Texture on.
pub(crate) fn command_states(app: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    let brush = &app.session.tools.brush;
    vec![
        (CommandRef::from(BrushSettingsCommand::ResetLocks), CommandState::new().enabled(brush.locks != photocraft_paint::brush::SectionLocks::default())),
        (CommandRef::from(BrushSettingsCommand::CopyTexture), CommandState::new().enabled(brush.texture.enabled)),
    ]
}

/// Runs a fired menu command; false for a command this panel does not own.
pub(crate) fn run_fired(app: &mut PhotocraftApp, _ctx: &egui::Context, command: &CommandRef) -> bool {
    let Some(command) = BrushSettingsCommand::from_ref(command) else { return false };
    match command {
        // Every way of making a preset opens the dialog, which names it and says what it captures.
        BrushSettingsCommand::NewPreset => {
            let _ = crate::brush_preset_dialog::open(app, None);
        }
        BrushSettingsCommand::ClearControls => {
            let before = app.session.tools.brush.clone();
            let cleared = clear_brush_controls(&before);
            commit(app, &before, &cleared);
        }
        BrushSettingsCommand::ResetLocks => {
            let before = app.session.tools.brush.clone();
            let mut after = before.clone();
            after.locks = photocraft_paint::brush::SectionLocks::default();
            commit(app, &before, &after);
        }
        BrushSettingsCommand::CopyTexture => copy_texture_to_other_tools(app),
    }
    true
}

#[cfg(test)]
mod tests;
