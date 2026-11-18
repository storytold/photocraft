//! Path Options (#9713): the gear's Thickness and Color are one display setting shared by every
//! path tool (`ViewOptions::path_options`), and its Rubber Band is the Pen's own tool option
//! (`ToolOptions::path_options.rubber_band`, #12900). The framework takes them as one
//! `PathOptionsSettings`; these join and split it.

use craft::draw::ui::PathOptionsSettings;

use crate::PhotocraftApp;

/// The Path Options the framework draws with: the shared Thickness and Color with the Pen's
/// Rubber Band. Every path overlay and the Pen read this, never the tool options' own colour.
pub(crate) fn path_options(app: &PhotocraftApp) -> PathOptionsSettings {
    let mut options = app.ui.tool_options.path_options;
    let shared = app.ui.view.path_options;
    options.color = shared.color;
    options.thickness = shared.thickness.filter(|px| thickness_in_range(*px));
    options
}

/// Stores `options` as the gear edited them: Thickness and Color in the shared setting, Rubber
/// Band in the Pen's tool options. The tool options never hold a colour or thickness of their own.
pub(crate) fn store_path_options(app: &mut PhotocraftApp, options: &PathOptionsSettings) {
    app.ui.view.path_options.color = options.color;
    app.ui.view.path_options.thickness = options.thickness.filter(|px| thickness_in_range(*px));
    let tool = &mut app.ui.tool_options.path_options;
    tool.rubber_band = options.rubber_band;
    tool.color = None;
    tool.thickness = None;
}

/// Whether `px` is a thickness the gear offers: a number in `PathOptionsSettings::THICKNESS_RANGE`.
pub(crate) fn thickness_in_range(px: f32) -> bool {
    PathOptionsSettings::THICKNESS_RANGE.contains(&px)
}
