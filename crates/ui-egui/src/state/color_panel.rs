use super::{Deserialize, Serialize};

/// Color panel state: the framework panel's settings. The Color Picker's Only Web Colors is the
/// framework picker memory's `web_colors`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ColorPanelState {
    /// What the panel shows, its ramp and which colour it edits (#6000).
    #[serde(flatten)]
    pub panel: craft::color_ui::ui::ColorPanelState,
}
