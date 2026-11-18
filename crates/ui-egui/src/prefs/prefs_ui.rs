//! Edit › Preferences, Keyboard Shortcuts and Menus, Color Settings and the rest of the Edit
//! menu's dialogs, plus the shell side of preferences: loading and saving them through
//! [`crate::Services`], applying the interface theme, autosave and crash recovery, and the
//! history log.
//!
//! The values live in the engine ([`photocraft_engine::prefs::Preferences`], so agents read and
//! change them with `prefs.get` / `prefs.set`); this module only edits a working copy in a dialog
//! and commits it with those commands on Apply or OK.

use std::collections::VecDeque;

use egui::Color32;
use photocraft_engine::jobs::{JobEvent, JobId, JobOutcome, Started};
use photocraft_engine::prefs::{self, AppearanceMode};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// Shell runtime state for preferences, autosave and snapping (not serialised).
#[derive(Default)]
pub struct Runtime {
    loaded: bool,
    recovery_started: bool,
    recovery_queue: VecDeque<crate::Recoverable>,
    recovery_pending: Option<(JobId, String)>,
    log_len: usize,
    /// The preferences revision last handed to the settings writer.
    saved_rev: u64,
    /// The open Keyboard Shortcuts and Menus window.
    pub(crate) shortcuts: Option<shortcuts::ShortcutsSession>,
    /// Style last sent to the GPU canvas.
    pub(crate) gpu_style: Option<crate::gpu_canvas::CanvasStyle>,
    /// `unitsAndRulers.showRulers` as last applied to View › Rulers.
    show_rulers: Option<bool>,
    /// Tool options as last remembered in the preferences (see `tool_memory`).
    pub(crate) tool_options: Option<crate::state::ToolOptions>,
    pub(crate) selected_tool: Option<crate::state::Tool>,
    pub(crate) tool_colors: Option<[[[f32; 4]; 2]; 2]>,
    pub(crate) tool_cmyk_picks: crate::tool_memory::CmykPicks,
    pub(crate) group_tools: Option<Vec<crate::state::Tool>>,
    /// The tool owning the session brush and the other tools' brushes, as last remembered.
    pub(crate) tool_brushes: Option<crate::tool_memory::ToolBrushes>,
}

// ------------------------------------------------------------------ shortcuts

pub use crate::shortcuts::{default_shortcut, effective_shortcut};

#[cfg(not(target_arch = "wasm32"))]
mod agents;
mod appearance;
mod custom_rows;
mod dialogs;
mod fields;
mod presets;
mod save;
pub(crate) mod shortcuts;
mod startup;

// PLAN: assumed top-w18's appearance file keeps `display_scale` and `selected_theme` and exports `appearance_button` and `theme_menu`.
pub(crate) use appearance::{appearance_button, cycle_appearance, interface_zoom, show_selected_theme, step_brightness, sync_appearance, theme_menu};
pub use appearance::{canvas_style, pasteboard_color};
#[cfg(test)]
pub(crate) use dialogs::set_draft;
pub use dialogs::{apply_window, invoke, open_preferences, window_open};
pub(crate) use dialogs::{feed, take as take_window};
pub(crate) use presets::{PRESETS_FAMILY, take as take_presets};
pub(crate) use save::save_preferences;
pub use save::{autosave_now, tick};
pub use shortcuts::{open as open_shortcuts, recording, shortcuts_open, show as show_shortcuts};
pub use startup::load;
pub(crate) use startup::{on_recovery_event, remember_rulers};

// Test modules stay after the production code: the prefs_usage test reads each file up to
// its first `#[cfg(test)] mod`.
#[cfg(test)]
mod agents_tests;
#[cfg(test)]
mod recovery_tests;

#[cfg(test)]
mod shortcut_capture_tests;
#[cfg(test)]
mod shortcut_set_tests;

#[cfg(test)]
mod tests;
