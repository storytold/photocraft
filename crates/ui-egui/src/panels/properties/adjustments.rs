//! The Adjustments panel: a grid of one-click adjustment layers.

use craft::imaging::ui::adjust::AdjustmentsPanel;
use serde_json::json;

use crate::PhotocraftApp;

/// The Adjustments panel body: every adjustment kind, in the framework's order. A pick adds that
/// adjustment layer; the command brings Properties forward to edit it. The pick is the pass's own
/// click, so a rerun pass of the frame adds no second layer.
pub(crate) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(kind) = AdjustmentsPanel::new(tl!("Adjustments")).show(ui).picked else { return };
    let _ = app.run(&format!("layer.newAdjustmentLayer.{}", kind.key()), json!({}));
}
