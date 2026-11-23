//! Shell-side View, Window and Type menu items: screen modes, Extras and the Show / Snap To
//! toggles, view flipping, document arrangement (tiles, n-up, floating windows,
//! Match Zoom/Location), type UI preferences, and the picker/dialog front ends of the engine's
//! File commands.
//!
//! Everything here is view or window state ([`ViewOptions`], serde, part of `UiState`, so the
//! control channel reads and drives it). Anything that changes a document is an engine command;
//! this module only opens its dialog or file picker first.

use craft::ui::shell::ScreenMode;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

mod arrange;
pub(crate) mod dialogs;
mod dispatch;
mod options;

pub(crate) use arrange::document_is_floating;
pub(crate) use dialogs::{nest_artboard_export_options, strip_artboard_name_options, write_copy};
#[cfg(test)]
use dispatch::is_enabled_with_engine;
pub use dispatch::{checked, handles, invoke, is_enabled, leave_full_screen_without_document};
pub use options::{persist, pixel_aspect_ratio, restore};

/// View › Show flags (Photoshop defaults).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, craft::ui::contract::MemoProps)]
#[serde(default)]
pub struct Show {
    pub layer_edges: bool,
    pub selection_edges: bool,
    pub target_path: bool,
    pub pixel_grid: bool,
    pub smart_guides: bool,
    pub brush_preview: bool,
    pub slices: bool,
    pub notes: bool,
    pub mesh: bool,
    pub edit_pins: bool,
    pub count: bool,
    pub canvas_guides: bool,
    pub artboard_guides: bool,
}

impl Default for Show {
    fn default() -> Self {
        Self {
            layer_edges: false,
            selection_edges: true,
            target_path: true,
            pixel_grid: true,
            smart_guides: true,
            brush_preview: true,
            slices: true,
            notes: true,
            mesh: false,
            edit_pins: true,
            count: true,
            canvas_guides: true,
            artboard_guides: true,
        }
    }
}

/// View › Snap To targets (snapping itself is the `view.snap` master switch).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, craft::ui::contract::MemoProps)]
#[serde(default)]
pub struct SnapTo {
    pub guides: bool,
    pub grid: bool,
    pub layers: bool,
    pub slices: bool,
    pub document_bounds: bool,
}

impl Default for SnapTo {
    fn default() -> Self {
        Self { guides: true, grid: true, layers: true, slices: true, document_bounds: true }
    }
}

/// View-, window- and type-preference state of the shell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, craft::ui::contract::MemoProps)]
#[serde(default)]
pub struct ViewOptions {
    /// View › Screen Mode (F cycles). Saved as "standard" | "fullScreenWithMenuBar" | "fullScreen".
    #[serde(with = "screen_mode_names")]
    #[memo_props(hash = "craft-shell's screen mode: a fieldless enum")]
    pub screen_mode: ScreenMode,
    /// View › Extras (⌘H): master switch for guides, grid, edges and other overlays.
    pub extras: bool,
    pub show: Show,
    pub snap_to: SnapTo,
    /// View › Flip Horizontal: mirror the view only (the document is untouched).
    pub flip_horizontal: bool,
    /// View › Pixel Aspect Ratio preset id ("square", "d1DvNtsc", …) and its correction toggle.
    pub pixel_aspect: String,
    pub pixel_aspect_correction: bool,
    pub pattern_preview: bool,
    pub pixel_art_preview: bool,
    /// Type › Language Options: "defaultFeatures" | "eastAsianFeatures" | "middleEasternFeatures",
    /// and the Middle Eastern & South Asian composer.
    pub language_features: String,
    pub middle_eastern_composer: bool,
    /// Parameters from the last successfully applied New Guide Layout dialog.
    pub guide_layout: Value,
    /// Developer overlay of the active layer's storage tiles (tile_overlay.rs), set with the
    /// control channel's `ui.set {"tileGrid": true}`.
    pub tile_grid: bool,
}

/// Factory New Guide Layout values (Alt+Reset and a fresh ViewOptions).
pub(crate) fn default_guide_layout() -> Value {
    json!({"columns": 8, "gutter": 20, "rows": 0, "rowGutter": 0, "margin": 0, "centerColumns": false, "clearExisting": false})
}

impl Default for ViewOptions {
    fn default() -> Self {
        Self {
            screen_mode: ScreenMode::Standard,
            extras: true,
            show: Show::default(),
            snap_to: SnapTo::default(),
            flip_horizontal: false,
            pixel_aspect: "square".into(),
            // Photoshop shows non-square pixels corrected until the user turns it off.
            pixel_aspect_correction: true,
            pattern_preview: false,
            pixel_art_preview: false,

            language_features: "defaultFeatures".into(),
            middle_eastern_composer: false,
            guide_layout: default_guide_layout(),
            tile_grid: false,
        }
    }
}

impl ViewOptions {
    /// Is an overlay with this Show flag visible (Extras on and the flag on)?
    pub fn shows(&self, flag: bool) -> bool {
        self.extras && flag
    }
    /// Whether the screen mode shows the body only.
    pub fn hides_chrome(&self) -> bool {
        self.screen_mode.body_only()
    }
    /// Whether the screen mode hides the document tabs.
    pub fn hides_tabs(&self) -> bool {
        !self.screen_mode.tabs_visible()
    }
    /// Horizontal display scale of a document pixel: the pixel aspect ratio while View › Pixel
    /// Aspect Ratio Correction is on, else 1 (square). The document's pixels never change.
    pub fn display_aspect(&self) -> f32 {
        let r = pixel_aspect_ratio(&self.pixel_aspect);
        // `ui.set` can store any string; only the dialog's 0.1–10 range is drawn.
        if self.pixel_aspect_correction && r.is_finite() { r.clamp(0.1, 10.0) } else { 1.0 }
    }
    /// Photoshop's full screen modes show no scroll bars.
    pub fn shows_scrollbars(&self) -> bool {
        self.screen_mode.tabs_visible()
    }
}

/// The saved and command names of the screen modes, in [`ScreenMode`]'s cycle order.
const SCREEN_MODES: [&str; 3] = ["standard", "fullScreenWithMenuBar", "fullScreen"];

/// The one mapping between [`ScreenMode`] and its names (the saved form and the `view.screenMode.<name>` ids).
mod screen_mode_names {
    use craft::ui::shell::ScreenMode;
    use serde::{Deserialize, Deserializer, Serializer};

    use super::SCREEN_MODES;

    /// The name of `mode`.
    pub(in super::super) fn name(mode: ScreenMode) -> &'static str {
        match mode {
            ScreenMode::Standard => SCREEN_MODES[0],
            ScreenMode::FullScreenWithMenuBar => SCREEN_MODES[1],
            ScreenMode::FullScreen => SCREEN_MODES[2],
            // `ScreenMode` is non-exhaustive; a mode added later has no saved name yet and saves as Standard.
            _ => SCREEN_MODES[0],
        }
    }

    /// The mode named `name`, if it is one.
    pub(in super::super) fn parse(name: &str) -> Option<ScreenMode> {
        [ScreenMode::Standard, ScreenMode::FullScreenWithMenuBar, ScreenMode::FullScreen].into_iter().find(|mode| self::name(*mode) == name)
    }

    pub(super) fn serialize<S: Serializer>(mode: &ScreenMode, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(name(*mode))
    }

    /// An unknown name is Standard.
    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<ScreenMode, D::Error> {
        let saved = String::deserialize(deserializer)?;
        Ok(parse(&saved).unwrap_or_default())
    }
}
const PIXEL_ASPECTS: [&str; 8] = ["square", "d1DvNtsc", "d1DvPal", "d1DvNtscWidescreen", "hdv1080", "d1DvPalWidescreen", "dvcproHd1080", "anamorphic2To1"];

const LANGUAGE: [&str; 3] = ["defaultFeatures", "eastAsianFeatures", "middleEasternFeatures"];
#[cfg(test)]
#[path = "view_cmds/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "view_cmds/fit_image_sticky_tests.rs"]
mod fit_image_sticky_tests;

#[cfg(test)]
#[path = "view_cmds/save_a_copy_name_tests.rs"]
mod save_a_copy_name_tests;
