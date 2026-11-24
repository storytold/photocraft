//! Move-tool modifier keys, as in Photoshop (#90):
//!
//! - ⇧ while dragging locks the move to the nearest of the eight directions (the axes and the 45°
//!   diagonals), as the reference application does when a layer is dragged (#11382): +100,+30
//!   moves +100,0 and +80,+80 moves +80,+80. The Move tool's `MoveGesture` does the locking.
//! - ⌥ held when the drag starts duplicates the selected layer(s) (groups included) once the
//!   pointer actually moves, and the drag then moves the copies. Duplicate and move land as one
//!   history step ("Move", as in Photoshop; an ⌥-arrow nudge keeps "Duplicate + Move"). ⇧⌥
//!   combines both.
//! - Arrow keys nudge the selected layers one screen pixel (⇧: 10 px); ⌥ duplicates first. A run
//!   of nudges is one History state (`nudge_bundle`, #2259).
//! - With a selection the drag and the arrow keys move the selected pixels (`move_ui`): ⇧ locks
//!   the drag to multiples of 45° and ⌥ copies the pixels instead of duplicating the layer.
//!
//! The Move tool calls [`begin`] at a layer or pixel move press, [`moved`] on each drag and
//! [`finish`] after the drag, which folds the history. The tool host's dispatcher calls
//! [`arrow_key`] for a nudge no tool family took.

use craft::keymap::Mods;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// ⌥-drag duplicates only once the pointer has moved this far (screen points), so an ⌥-click
/// without a drag doesn't make a copy (Photoshop).
const DUPLICATE_THRESHOLD: f64 = 2.0;
/// History label of an ⌥-nudge. An ⌥-drag of a layer is a plain "Move", as in Photoshop (#11383).
pub const DUPLICATE_MOVE_LABEL: &str = "Duplicate + Move";

/// Per-drag state (one Move-tool gesture at a time).
#[derive(Clone, Debug, Default)]
pub struct MoveDrag {
    /// A Move drag of layers or pixels is under way.
    active: bool,
    /// ⌥ was held when the drag started and the layers are not copied yet.
    alt: bool,
    /// The drag moves selected pixels (`move_ui`), which ⌥ copies itself.
    pixels: bool,
    /// History length before the ⌥-drag duplicated (set once it has).
    dup_from: Option<usize>,
}

fn past_len(app: &PhotocraftApp) -> Option<usize> {
    app.session.active().map(|st| st.history.past_len())
}

/// A Move drag of layers or selected pixels starts; `alt` is ⌥ at the press.
pub fn begin(app: &mut PhotocraftApp, alt: bool) {
    let pixels = crate::move_ui::moves_selected_pixels_with(app, Tool::Move);
    app.move_mods = MoveDrag { active: true, alt, pixels, dup_from: None };
}

/// The drag has moved `screen_len` screen points from the press: the first time past
/// [`DUPLICATE_THRESHOLD`] of an ⌥-drag of layers duplicates them. Once per drag, even if
/// duplicating failed.
pub fn moved(app: &mut PhotocraftApp, screen_len: f64) {
    let m = &app.move_mods;
    if m.active && m.alt && !m.pixels && m.dup_from.is_none() && screen_len >= DUPLICATE_THRESHOLD {
        app.move_mods.dup_from = duplicate(app);
        app.move_mods.alt = false;
    }
}

/// The pointer reflects the pending copy, then the latched duplication for the whole drag.
pub fn copying(app: &PhotocraftApp, alt: bool) -> bool {
    if app.move_mods.active { app.move_mods.alt || app.move_mods.dup_from.is_some() } else { alt }
}

/// Duplicates the selected layers (the copies become the selection), remembering the history
/// length before so [`fold_history`] can merge the copy and the move into one step.
fn duplicate(app: &mut PhotocraftApp) -> Option<usize> {
    let before = past_len(app)?;
    // In place: the copy follows the pointer from the original, artboards too (#1531).
    match app.run("layer.duplicate", json!({"inPlace": true})) {
        // The state just before the copy's: in non-linear history the copy's step also moved
        // the later states in front of it (`History::record`), and those must stay.
        Ok(_) => Some(past_len(app).and_then(|n| n.checked_sub(1)).unwrap_or(before).max(before)),
        Err(e) => {
            app.ui.status = e;
            None
        }
    }
}

/// Merges every history step after `from` into one step called `label`.
fn fold_history(app: &mut PhotocraftApp, from: usize, label: &str) {
    let Some(st) = app.session.active_mut() else { return };
    while st.history.past_len() > from + 1 {
        if !st.history.purge_last() {
            break;
        }
    }
    if st.history.past_len() == from + 1 {
        st.history.set_current_label(label);
    }
}

/// The Move drag in progress is an ⌥-drag that has duplicated the layers (it now moves the copies).
pub(crate) fn duplicated(app: &PhotocraftApp) -> bool {
    app.move_mods.active && app.move_mods.dup_from.is_some()
}

/// Call after the Move tool finished its drag: an ⌥-drag's copy and move become one step.
pub fn finish(app: &mut PhotocraftApp) {
    let st = std::mem::take(&mut app.move_mods);
    if let Some(from) = st.dup_from {
        fold_history(app, from, tl!("Move"));
    }
}

/// Call when a Move drag ends without moving (its layers were dragged to another document): an
/// ⌥-drag's copy is taken back.
pub fn abandon(app: &mut PhotocraftApp) {
    if std::mem::take(&mut app.move_mods).dup_from.is_some() {
        // Exactly, also in non-linear history (where an undo would land on a later state).
        app.session.rollback();
    }
}

/// The document pixels one plain arrow press moves layers by with the Move tool: one screen
/// pixel, as in Photoshop (#9511). Zoomed out that is several document pixels (2 at 50 %, 4 at
/// 25 %; 1 at 66.7 %: whole pixels only), zoomed in still one. No cap: the zoom bottoms out
/// with the document's shorter side 2 px on screen (`ZoomLadder::min`), so a step is at most
/// half that side. ⇧'s 10 px don't scale.
fn screen_pixel_step(app: &PhotocraftApp) -> f64 {
    let zoom = f64::from(app.current_zoom());
    if !zoom.is_finite() || zoom <= 0.0 {
        return 1.0;
    }
    // A ladder rung is a hair off in f32 (0.1 % is 1 / 999.99995): snap a reciprocal within f32
    // precision of a whole number to it, and floor any other zoom (0.3333334 is 2, not 3).
    let (r, whole) = (1.0 / zoom, (1.0 / zoom).round());
    let steps = if (r - whole).abs() <= r * f64::from(f32::EPSILON) { whole } else { r.floor() };
    steps.max(1.0)
}

/// An arrow key no tool family took, with no gesture running. `direction` is the unit screen
/// vector of the arrow. With the Move tool: nudge one screen pixel ([`screen_pixel_step`]), ⇧ 10
/// px; ⌥ duplicates the layers first. ⌘-arrows (Ctrl off the Mac) do the same from most other
/// tools, where ⌘ is the temporary Move tool (`hold_keys::cmd_nudges`, #2474). `outline_open` is
/// the selection tools' open outline, which the app's canvas tools hold out of reach during the
/// pass. Never while text is edited. Returns true when the key was used.
pub(crate) fn arrow_key(app: &mut PhotocraftApp, direction: egui::Vec2, mods: Mods, time: f64, outline_open: bool) -> bool {
    if app.ui.text_edit.is_some() {
        return false;
    }
    // An open outline keeps ⌘-arrows as it keeps the plain ones: `cmd_nudges` cannot see it.
    let cmd = mods.cmd && !outline_open && crate::hold_keys::cmd_nudges(app);
    if app.ui.tool != Tool::Move && !cmd {
        return selection_arrow_keys(app, direction, mods, time, outline_open) || brush_arrow_keys(app, direction, mods);
    }
    let k = if mods.shift { 10.0 } else { screen_pixel_step(app) };
    app.nudge_bundle.now = time;
    nudge(app, f64::from(direction.x) * k, f64::from(direction.y) * k, mods.alt);
    true
}

/// ← / → with a painting tool: turn the brush tip 1° (⇧ 15°), as Photoshop does; → turns it
/// clockwise. Only plain or ⇧ arrows (⌥ kerns type, ⌘ moves). Returns true when a key was used.
fn brush_arrow_keys(app: &mut PhotocraftApp, direction: egui::Vec2, mods: Mods) -> bool {
    if !app.ui.tool.is_brushlike() || app.ui.text_edit.is_some() || mods.alt || mods.cmd || mods.ctrl {
        return false;
    }
    let sign = if direction.x < 0.0 {
        1.0
    } else if direction.x > 0.0 {
        -1.0
    } else {
        return false;
    };
    let step = if mods.shift { 15.0 } else { 1.0 };
    if let Err(e) = app.run("tools.rotateBrush", json!({"degrees": sign * step})) {
        app.ui.status = e;
    }
    true
}

/// Moves the selected layers (or the selected pixels) by `(dx, dy)` pixels, as the Move tool
/// (also when ⌘ makes it the temporary Move tool). Nudges that follow each other are one History
/// state (`nudge_bundle`); the selected pixels float until dropped, which is one state already.
pub fn nudge(app: &mut PhotocraftApp, dx: f64, dy: f64, duplicate_first: bool) {
    if crate::move_ui::moves_selected_pixels_with(app, Tool::Move) {
        crate::move_ui::float_selected(app, duplicate_first, dx, dy);
        return;
    }
    let from = if duplicate_first { duplicate(app) } else { None };
    let revision = app.session.active().map(|st| st.revision);
    let moved = if duplicate_first {
        // A copy is a state of its own, folded with its move below.
        app.nudge_bundle.reset();
        app.run("layer.translate", crate::move_ui::pattern_wrap(app, json!({"dx": dx, "dy": dy})))
    } else {
        crate::nudge_bundle::run(app, "layers", "layer.translate", crate::move_ui::pattern_wrap(app, json!({"dx": dx, "dy": dy})))
    };
    match moved {
        // The reference names an arrow-key move "Nudge"; a bundled run is one step with that name.
        // Only a move that changed the document: a zero offset records nothing, and the state
        // on top is then another edit's.
        Ok(_) if from.is_none() => {
            if let Some(st) = app.session.active_mut().filter(|st| Some(st.revision) != revision) {
                st.history.set_current_label(tl!("Nudge"));
            }
        }
        Ok(_) => {}
        Err(e) => app.ui.status = e,
    }
    if let Some(from) = from {
        fold_history(app, from, DUPLICATE_MOVE_LABEL);
    }
}

/// Arrow keys with a selection tool (#1428), as in Photoshop: nudge the selection outline 1 px
/// (⇧ 10 px) through `select.transformSelection`, a run of presses being one history step
/// (`nudge_bundle`); the pixels stay put. A floating piece (⌘-dragged pixels) moves instead, like
/// a plain drag on it. Not while a polygon or a Magnetic Lasso border is open (`outline_open`).
/// Returns true when a key was used.
fn selection_arrow_keys(app: &mut PhotocraftApp, direction: egui::Vec2, mods: Mods, time: f64, outline_open: bool) -> bool {
    if !crate::canvas::tool_feedback::is_selection_tool(app.ui.tool) || outline_open || mods.cmd || mods.ctrl {
        return false;
    }
    let floating = app.session.active().is_some_and(|st| photocraft_engine::float_cmds::floating(st).is_some());
    if !floating && !app.session.is_enabled("select.transformSelection") {
        return false;
    }
    let k = if mods.shift { 10.0 } else { 1.0 };
    let (dx, dy) = (f64::from(direction.x) * k, f64::from(direction.y) * k);
    app.nudge_bundle.now = time;
    // A floating piece is no state until dropped; the outline's nudges bundle.
    let result = if floating {
        app.run("select.float", json!({"dx": dx, "dy": dy}))
    } else {
        crate::nudge_bundle::run(app, "selection", "select.transformSelection", json!({"dx": dx, "dy": dy}))
    };
    if let Err(e) = result {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    true
}

#[cfg(test)]
mod tests;
