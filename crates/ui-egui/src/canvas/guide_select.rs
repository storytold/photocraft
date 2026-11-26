//! Selecting a guide on the canvas, as in Photoshop: a click on a guide with the Move tool selects
//! it, a press anywhere else deselects it, and right-click opens the guide menu
//! (`canvas_tool_menu`), whose Edit Selected Guides reopens New Guide on it. The framework
//! `Guides` reports the clicks and draws the selected look (`rulers::show` maps them here).
//!
//! One guide at a time. The selection is view state, not a history step: it names the guide by
//! document, orientation and position, as guide colours do, so it lapses by itself when the guide
//! moves away, is deleted or undone, and the index is looked up when it is used.

use craft::ui::canvas::GuideKey;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::PhotocraftApp;

/// The bit of a [`GuideKey`] that marks a vertical guide; the low bits hold the guide's index
/// among the guides of its orientation, as the engine numbers them (`view.moveGuide {orientation, index}`).
const VERTICAL_BIT: u64 = 1 << 63;

/// The key the framework `Guides` names guide `index` of an orientation by. Keys are rebuilt each
/// pass from the engine's lists, so an undo or a deletion renumbers them.
pub fn key(vertical: bool, index: usize) -> GuideKey {
    GuideKey::new(index as u64 | if vertical { VERTICAL_BIT } else { 0 })
}

/// The orientation (vertical) and index a [`key`] packs. Only keys built by [`key`] decode to a
/// guide of the document: `Guides` names a guide it creates with a key of its own.
pub fn unkey(key: GuideKey) -> (bool, usize) {
    let bits = key.get();
    (bits & VERTICAL_BIT != 0, usize::try_from(bits & !VERTICAL_BIT).unwrap_or(usize::MAX))
}

/// The selected guide's key in the active document's guides, while guides can be interacted with.
pub fn selected_key(app: &PhotocraftApp) -> Option<GuideKey> {
    selected(app).map(|(vertical, index)| key(vertical, index))
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectedGuide {
    /// The document's index among the open documents.
    pub doc: usize,
    pub vertical: bool,
    pub position: f64,
}

/// The selected guide's orientation and index in the active document, while guides can be
/// interacted with (shown, unlocked).
pub fn selected(app: &PhotocraftApp) -> Option<(bool, usize)> {
    let sel = app.ui.selected_guide?;
    if !interactive(app) || app.session.active_index() != Some(sel.doc) {
        return None;
    }
    let guides = &app.session.active()?.doc.guides;
    let list = if sel.vertical { &guides.vertical } else { &guides.horizontal };
    list.iter().position(|p| *p == sel.position).map(|i| (sel.vertical, i))
}

/// Guides take clicks only while they're shown and unlocked.
pub(crate) fn interactive(app: &PhotocraftApp) -> bool {
    app.ui.extras.guides && app.ui.view.shows(app.ui.view.show.canvas_guides) && !app.ui.extras.lock_guides
}

/// Select guide `index` of the active document.
pub fn select(app: &mut PhotocraftApp, vertical: bool, index: usize) {
    let Some(doc) = app.session.active_index() else { return };
    let Some(guides) = app.session.active().map(|st| &st.doc.guides) else { return };
    let list = if vertical { &guides.vertical } else { &guides.horizontal };
    app.ui.selected_guide = list.get(index).map(|position| SelectedGuide { doc, vertical, position: *position });
}

pub fn clear(app: &mut PhotocraftApp) {
    app.ui.selected_guide = None;
}

/// Delete the selected guide (Delete, Clear Selected Guides). False when no guide is selected.
pub fn delete_selected(app: &mut PhotocraftApp) -> Result<bool, String> {
    let Some((vertical, index)) = selected(app) else { return Ok(false) };
    app.run("view.deleteGuide", delete_params(vertical, index))?;
    clear(app);
    Ok(true)
}

pub fn delete_params(vertical: bool, index: usize) -> serde_json::Value {
    json!({"orientation": if vertical { "vertical" } else { "horizontal" }, "index": index})
}

#[cfg(test)]
mod tests;
