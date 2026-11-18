//! Photoshop's "Could not use the move tool because the layer is locked." A Move-tool press (the
//! Move tool itself, or the Move tool standing in for ⌘ with another tool) on a layer it can't
//! move opens that message as an alert instead of silently doing nothing: the Background and any
//! position-locked layer when there is no selection, and a layer whose selected pixels can't
//! float (`photocraft_engine::float_cmds::locked_for_float`) when there is one. As in Photoshop
//! the message comes when the pointer starts to drag; a plain click says nothing.

use crate::PhotocraftApp;
use crate::state::Tool;

pub const MESSAGE: &str = "Could not use the move tool because the layer is locked.";

/// The selected pixels can't float: only a fresh cut can be refused, so a floating piece is never
/// locked. The selection family's ⌘-drag of pixels asks this.
pub fn pixels_locked(app: &PhotocraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    photocraft_engine::float_cmds::floating(st).is_none()
        && st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| photocraft_engine::float_cmds::locked_for_float(&st.doc, l))
}

/// The layers: any of them locked in place refuses the whole move.
fn layers_locked(app: &PhotocraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    photocraft_engine::layer_multi_cmds::move_targets(&st.doc, &st.selected_layers()).iter().any(|id| {
        let locks = st.doc.effective_locks(*id);
        locks.position || locks.all
    })
}

/// Would a press with the Move tool in effect be a move that fails because of a lock?
/// `standing_in` is true when the Move tool stands in for another tool's ⌘ press.
pub fn blocked(app: &PhotocraftApp, standing_in: bool) -> bool {
    let Some(st) = app.session.active() else { return false };
    if standing_in {
        return layers_locked(app);
    }
    // `moves_selected_pixels_with` is false for a layer whose pixels can't float: then the Move tool
    // moves the layer, which its position lock refuses too.
    if crate::move_ui::moves_selected_pixels_with(app, Tool::Move) {
        return pixels_locked(app);
    }
    let floating = photocraft_engine::float_cmds::floating(st).is_some();
    !floating && st.doc.selection.is_some() && pixels_locked(app) || layers_locked(app)
}

/// Opens the message as an alert.
pub fn prompt(app: &mut PhotocraftApp) {
    crate::dialogs::alerts::alert(app, tl!(MESSAGE));
}
