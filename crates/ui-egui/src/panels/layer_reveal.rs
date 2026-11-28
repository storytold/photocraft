//! Reveal the active layer in the Layers panel (#152), like Photoshop: whenever the active
//! layer changes, by whatever means (a click on the canvas with Auto-Select, Select › layer
//! commands, ⌥[ / ⌥], undo, automation), its closed parent groups open and the panel scrolls
//! just enough to show its row, unless the row is already in view. The panel does both
//! (`LayersPanel::reveal`); this module decides when, and whether the groups open.
//!
//! The command dispatch takes a [`mark`] before a command and calls [`file`] after it; when the
//! active layer moved, the reveal waits in `PhotocraftApp::pending_reveal` until the panel next
//! draws and applies it, so a hidden panel reveals the layer when it is shown again.
//!
//! Only a change of the active layer reveals: closing the group that holds it, or scrolling
//! away, is left alone until the next change.

use craft::layers::ui::panel::Groups;
use photocraft_doc::LayerId;

use crate::PhotocraftApp;

/// The active document and layer at one moment: what [`file`] compares across a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Mark {
    document: Option<usize>,
    layer: Option<LayerId>,
}

/// The active document index and active layer now.
pub(crate) fn mark(app: &PhotocraftApp) -> Mark {
    Mark { document: app.session.active_index(), layer: app.session.active().and_then(|st| st.active_layer) }
}

/// Called after a command ran, with the [`mark`] taken before it. When the active layer changed
/// inside the same document, files the reveal for the Layers panel's next draw; a switch of
/// document reveals nothing.
pub(crate) fn file(app: &mut PhotocraftApp, before: Mark) {
    let after = mark(app);
    // The marker says the latest active-layer change was a Move-way pick of that layer: it holds only
    // while no later change happened (the engine's change count is unchanged), whether or not the
    // final layer is the same id.
    let changes = app.session.active_layer_changes();
    let pick = app.move_pick.take().filter(|&(id, at)| Some(id) == after.layer && at == changes).map(|(id, _)| id);
    if before.document.is_none() || before.document != after.document || before.layer == after.layer {
        return;
    }
    let Some(id) = after.layer else { return };
    // Expand layer group on click (Move gear menu, #11381) governs a canvas click that picked this
    // layer the Move tool way (Auto-Select, or a ⌘-click from another tool); every other change
    // opens the groups. The row (or its outermost closed group's) still scrolls into view.
    let move_pick = pick == Some(id);
    let groups = if !move_pick || app.ui.tool_options.move_expand_group_on_click { Groups::Open } else { Groups::Keep };
    app.pending_reveal = Some((id, groups));
}

/// The pending reveal: the layer and what its closed parent groups do. The Layers panel takes it
/// once its real run applied it (`LayersOutput::requests_taken`), so a measuring run or a later
/// pass of the same frame reveals nothing again.
pub(crate) fn take(app: &mut PhotocraftApp) -> Option<(LayerId, Groups)> {
    app.pending_reveal.take()
}
