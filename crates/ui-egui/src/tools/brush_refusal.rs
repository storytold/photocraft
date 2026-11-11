//! Photoshop's alerts when a painting tool can't paint (#11898): a press on a hidden target
//! layer, on a layer with Lock All, or with a group active (its pixels, not its mask) opens a
//! modal alert with OK instead of a status-bar message, and paints nothing. Only the tools whose
//! wording was measured have a name here ([`tool_name`]); the others keep today's status bar. The
//! Classic gradient has the same three alerts (#12017); the live Gradient keeps today's.

use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{DialogKind, Tool};

/// The alerts; `{tool}` is [`tool_name`].
pub const HIDDEN: &str = "Could not use the {tool} because the target layer is hidden.";
pub const LOCKED: &str = "Could not use the {tool} because the layer is locked.";
pub const GROUP: &str = "Could not use the {tool} because the pixels in a group cannot be edited without merging the group.";

/// The colour-mode alerts (#12686); `{tool}` is [`mode_tool_name`]. The engine refuses the
/// command with the same words.
pub const INDEXED: &str = "Could not use the {tool} because it does not work with Indexed Color images (convert image to RGB Color mode to edit).";
pub const BITMAP: &str = "Could not use the {tool} because it does not work with Bitmap images (convert image to Grayscale mode to edit).";

/// The tool's name in the colour-mode alerts, for the tools whose wording was measured: the Dodge
/// and Sharpen tools. Kept apart from [`tool_name`]: the layer alerts aren't wired for the Dodge
/// tool yet (#12685).
fn mode_tool_name(tool: Tool) -> Option<&'static str> {
    match tool {
        Tool::Dodge => Some(tl!("dodge tool")),
        Tool::Sharpen => Some(tl!("sharpen tool")),
        _ => None,
    }
}

/// The tool's name in the alerts, for the tools whose wording was measured: the Brush, the
/// Sharpen tool (#12952), the Sponge (#13030) and the Classic gradient (the live Gradient makes a
/// fill layer and isn't measured, #12017).
pub fn tool_name(app: &PhotocraftApp, tool: Tool) -> Option<&'static str> {
    match tool {
        Tool::Brush => Some(tl!("brush tool")),
        Tool::Sharpen => Some(tl!("sharpen tool")),
        Tool::Sponge => Some(tl!("sponge tool")),
        Tool::Gradient if app.ui.tool_options.gradient_classic => Some(tl!("gradient tool")),
        _ => None,
    }
}

/// The alert a press with `tool` gets, if any. `hidden` is the press-time hidden-target refusal
/// (`canvas::tool_input`), which this only words; the lock and the group are checked here. Lock
/// transparent pixels is no refusal, and a group's targeted mask paints. A tool with a
/// [`mode_tool_name`] gets its colour-mode alert first, in an Indexed Color or Bitmap document.
pub fn alert(app: &PhotocraftApp, tool: Tool, hidden: bool) -> Option<String> {
    if let Some(name) = mode_tool_name(tool)
        && let Some(template) = match app.session.active()?.doc.mode {
            photocraft_doc::ColorMode::Indexed => Some(INDEXED),
            photocraft_doc::ColorMode::Bitmap => Some(BITMAP),
            _ => None,
        }
    {
        return Some(crate::i18n::fmt(tl!(template), &[("tool", name)]));
    }
    let name = tool_name(app, tool)?;
    let template = if hidden {
        HIDDEN
    } else {
        let st = app.session.active()?;
        let layer = st.active_layer.and_then(|id| st.doc.layer(id))?;
        if crate::canvas::paint_target(app) != json!("pixels") {
            return None;
        }
        if layer.is_group() {
            GROUP
        } else if locked(st.doc.effective_locks(layer.id), tool) {
            LOCKED
        } else {
            return None;
        }
    };
    Some(crate::i18n::fmt(tl!(template), &[("tool", name)]))
}

/// Whether `locks` refuse `tool`: Lock All, and for the Sharpen and Sponge tools Lock Image Pixels
/// as well (measured for them; the Brush keeps today's behaviour there).
fn locked(locks: photocraft_doc::Locks, tool: Tool) -> bool {
    locks.all || (matches!(tool, Tool::Sharpen | Tool::Sponge) && locks.pixels)
}

/// Opens the alert: ordinary dialog state, listed by `ui.inspect` and closed by `ui.dialog.confirm`.
pub fn prompt(app: &mut PhotocraftApp, message: &str) {
    app.ui.open_dialog(DialogKind::Error, json!({ "message": message }).as_object().cloned().unwrap_or_default());
}

#[cfg(test)]
#[path = "../brush_refusal_alert_tests.rs"]
pub(super) mod brush_refusal_alert_tests;

#[cfg(test)]
#[path = "../sharpen_refusal_alert_tests.rs"]
mod sharpen_refusal_alert_tests;

#[cfg(test)]
#[path = "../sponge_refusal_alert_tests.rs"]
mod sponge_refusal_alert_tests;

#[cfg(test)]
#[path = "../bucket_bitmap_alert_tests.rs"]
mod bucket_bitmap_alert_tests;
