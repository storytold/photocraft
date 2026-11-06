//! The options bar's Add New Artboard placing mode (#11386): the button switches the mode on, and
//! while it is on a click on empty canvas puts a board of the bar's size with its top-left corner
//! at the click. The engine's `layer.new.artboard` does the work; this only turns the click into
//! its `rect`.

use serde_json::{Value, json};

use super::active_board;
use crate::PhotocraftApp;
use crate::state::Tool;

/// True while the Artboard tool is current and its placing mode is on.
pub(super) fn active(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Artboard && app.ui.artboard_tool.adding
}

/// The Add New Artboard button: turns the mode on, or off when it is already on.
pub(super) fn toggle(app: &mut PhotocraftApp) {
    app.ui.artboard_tool.adding = !app.ui.artboard_tool.adding;
}

/// Another tool is current: the mode ends.
pub(super) fn leave(app: &mut PhotocraftApp) {
    app.ui.artboard_tool.adding = false;
}

/// The size the options bar shows: the selected board's, else the canvas size (the engine's
/// fallback for a board with no size).
fn bar_size(app: &PhotocraftApp) -> Option<[i64; 2]> {
    if let Some((_, r)) = active_board(app) {
        return Some([r[2].saturating_sub(r[0]), r[3].saturating_sub(r[1])]);
    }
    let size = app.session.active()?.doc.size;
    Some([i64::from(size.width), i64::from(size.height)])
}

/// A click at document point `p` on empty canvas: one `layer.new.artboard` with the bar's size and
/// its top-left corner at the click, rounded to whole pixels. None when the mode is off.
pub(super) fn click(app: &mut PhotocraftApp, p: [f64; 2]) -> Option<Result<Value, String>> {
    if !active(app) {
        return None;
    }
    let [w, h] = bar_size(app)?;
    let rect = [p[0].round() as i64, p[1].round() as i64, w, h];
    Some(app.run("layer.new.artboard", json!({"rect": rect})))
}
