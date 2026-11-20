//! The options bar of the retouch tools: one family function per tool inside the host's `Bar` body,
//! and the commit and cancel buttons of the two tools that wait for an answer.

use craft::ui::runtime::env::Label;
use craft::ui::text::RangeAlert;

use super::{remove_pending_cancel, remove_pending_commit};
use crate::PhotocraftApp;
use crate::state::Tool;
use crate::tools::option_cells::range_alert;

mod heal;
pub(crate) mod history;
mod select;
mod tone;

#[cfg(test)]
mod tests;

/// The bar's commit and cancel button labels while the tool waits for an answer.
pub(crate) struct Pending {
    /// The commit button's label.
    pub commit: Label,
    /// The cancel button's label.
    pub cancel: Label,
}

/// Shows the options of the retouch and smart-selection tools. False for other tools.
pub(crate) fn options_bar(ui: &mut egui::Ui, app: &mut PhotocraftApp, tool: Tool) -> bool {
    if !tool.is_brushlike() && !matches!(tool, Tool::QuickSelection | Tool::ObjectSelection | Tool::Patch | Tool::ContentAwareMove | Tool::RedEye)
        || matches!(tool, Tool::Brush | Tool::Pencil | Tool::MixerBrush | Tool::Eraser)
    {
        return false;
    }
    let alert: Option<RangeAlert> = match tool {
        Tool::Remove => heal::remove(ui, app),
        Tool::Patch => heal::patch(ui, app),
        Tool::ContentAwareMove => heal::content_aware_move(ui, app),
        Tool::SpotHealing => heal::spot_healing(ui, app),
        Tool::Healing | Tool::CloneStamp => heal::clone(ui, app),
        Tool::Dodge | Tool::Burn => tone::dodge_burn(ui, app),
        Tool::Sponge => tone::sponge(ui, app),
        Tool::Blur | Tool::Sharpen | Tool::Smudge => tone::blur_sharpen_smudge(ui, app, tool),
        Tool::ColorReplacement => tone::color_replacement(ui, app),
        Tool::QuickSelection => select::quick_selection(ui, app),
        Tool::ObjectSelection => select::object_selection(ui, app),
        Tool::HistoryBrush => history::history_brush(ui, app),
        Tool::ArtHistoryBrush => history::art_history(ui, app),
        Tool::PatternStamp => history::pattern_stamp(ui, app),
        Tool::RedEye => history::red_eye(ui, app),
        _ => None,
    };
    if let Some(alert) = alert {
        range_alert(app, ui, &alert);
    }
    true
}

/// The commit and cancel buttons `tool`'s bar shows now: Remove with strokes waiting, Content-Aware
/// Move with its transform box up.
pub(crate) fn pending(app: &PhotocraftApp, tool: Tool) -> Option<Pending> {
    match tool {
        Tool::Remove if !app.remove_pending.is_empty() => Some(Pending { commit: tl!("Apply the removal").into(), cancel: tl!("Cancel").into() }),
        Tool::ContentAwareMove if crate::cam_transform::active(app) => {
            Some(Pending { commit: tl!("Commit transform").into(), cancel: tl!("Cancel transform").into() })
        }
        _ => None,
    }
}

/// Runs the answer of [`pending`]'s buttons.
pub(crate) fn answer(app: &mut PhotocraftApp, tool: Tool, committed: bool, cancelled: bool) {
    match (tool, committed, cancelled) {
        (Tool::Remove, true, _) => {
            remove_pending_commit(app);
        }
        (Tool::Remove, false, true) => {
            remove_pending_cancel(app);
        }
        (Tool::ContentAwareMove, true, _) => crate::cam_transform::commit(app),
        (Tool::ContentAwareMove, false, true) => crate::cam_transform::cancel(app),
        _ => {}
    }
}
