//! Photoshop's alerts when a painting tool can't paint (#11898): a press on a hidden target
//! layer, on a layer with Lock All, or with a group active (its pixels, not its mask) opens a
//! modal alert with OK instead of a status-bar message, and paints nothing. Only the tools whose
//! wording was measured have a name here ([`tool_name`]); the others keep today's status bar. The
//! Classic gradient has the same three alerts (#12017); the live Gradient keeps today's. The Remove
//! tool has one alert, without the tool's name: Lock All on a pixel layer (#12552).

use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The alerts; `{tool}` is [`tool_name`].
pub const HIDDEN: &str = "Could not use the {tool} because the target layer is hidden.";
pub const LOCKED: &str = "Could not use the {tool} because the layer is locked.";
pub const GROUP: &str = "Could not use the {tool} because the pixels in a group cannot be edited without merging the group.";
/// The Remove tool's one measured alert, Lock All (#12552): it doesn't name the tool.
pub const REMOVE_LOCKED: &str = "Can't use the tool because the layer is locked.";

/// The colour-mode alerts (#12686); `{tool}` is [`mode_tool_name`]. The engine refuses the
/// command with the same words.
pub const INDEXED: &str = "Could not use the {tool} because it does not work with Indexed Color images (convert image to RGB Color mode to edit).";
pub const BITMAP: &str = "Could not use the {tool} because it does not work with Bitmap images (convert image to Grayscale mode to edit).";

/// The tool's name in the colour-mode alerts, for the tools whose wording was measured: the Dodge,
/// Burn, Sponge, Blur, Sharpen and Smudge tools. Kept apart from [`tool_name`]: the layer alerts
/// aren't wired for all of them yet (#12685).
fn mode_tool_name(tool: Tool) -> Option<&'static str> {
    match tool {
        Tool::Dodge => Some(tl!("dodge tool")),
        Tool::Burn => Some(tl!("burn tool")),
        Tool::Sponge => Some(tl!("sponge tool")),
        Tool::Blur => Some(tl!("blur tool")),
        Tool::Sharpen => Some(tl!("sharpen tool")),
        Tool::Smudge => Some(tl!("smudge tool")),
        _ => None,
    }
}

/// The tool's name in the alerts, for the tools whose wording was measured: the Brush, the
/// Sharpen tool (#12952), the Sponge (#13030), the Smudge tool (#12965), the Burn tool (#12672), the Healing Brush (#12111), the Art History Brush
/// (#12638) and the Classic gradient (the live Gradient makes a fill layer and isn't measured, #12017).
pub fn tool_name(app: &PhotocraftApp, tool: Tool) -> Option<&'static str> {
    match tool {
        Tool::Brush => Some(tl!("brush tool")),
        // The Magic Eraser's click (#12020).
        Tool::MagicEraser => Some(tl!("magic eraser")),
        Tool::Sharpen => Some(tl!("sharpen tool")),
        Tool::Sponge => Some(tl!("sponge tool")),
        Tool::Smudge => Some(tl!("smudge tool")),
        // The Spot Healing Brush (#12581).
        Tool::SpotHealing => Some(tl!("spot healing brush")),
        Tool::Burn => Some(tl!("burn tool")),
        Tool::Healing => Some(tl!("healing brush")),
        Tool::ArtHistoryBrush => Some(tl!("art history brush")),
        Tool::Gradient if app.ui.tool_options.gradient_classic => Some(tl!("gradient tool")),
        _ => None,
    }
}

/// The alert a press with `tool` gets, if any. `hidden` is the press-time hidden-target refusal
/// (`canvas::tool_input`), which this only words; the lock and the group are checked here. Lock
/// transparent pixels is no refusal, and a group's targeted mask paints. A tool with a
/// [`mode_tool_name`] gets its colour-mode alert first, in an Indexed Color or Bitmap document; the
/// Pattern Stamp only the engine's, for any pattern in an Indexed Color document.
pub fn alert(app: &PhotocraftApp, tool: Tool, hidden: bool) -> Option<String> {
    // The Pattern Stamp's colour-mode alert depends on its pattern too (#12887).
    if tool == Tool::PatternStamp {
        use photocraft_engine::retouch_cmds::{PATTERN_STAMP_INDEXED_REFUSAL, pattern_stamp_press_refusal};
        let p = crate::retouch_ui::pattern_stamp_params(app, &[]);
        return pattern_stamp_press_refusal(&app.session, &p).is_some().then(|| tl!(PATTERN_STAMP_INDEXED_REFUSAL).to_owned());
    }
    if let Some(name) = mode_tool_name(tool)
        && let Some(template) = match app.session.active()?.doc.mode {
            photocraft_doc::ColorMode::Indexed => Some(INDEXED),
            photocraft_doc::ColorMode::Bitmap => Some(BITMAP),
            _ => None,
        }
    {
        return Some(crate::i18n::fmt(tl!(template), &[("tool", name)]));
    }
    if tool == Tool::Remove {
        // Measured on a pixel layer only: a hidden target, a group, a type, shape, Smart Object or
        // fill layer keep today's behaviour.
        let st = app.session.active()?;
        let pixel_layer = st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| matches!(l.content, photocraft_doc::LayerContent::Raster(_)));
        return (!hidden && pixel_layer && layer_refusal(app, tool) == Some(LOCKED)).then(|| tl!(REMOVE_LOCKED).to_string());
    }
    // A Spot Healing press on a hidden empty pixel layer, Sample All Layers off, is refused quietly (#12581).
    if hidden && spot_healing_quiet(app, tool) {
        return None;
    }
    let name = tool_name(app, tool)?;
    // The Art History Brush's hidden alert was measured on a layer's pixels only (#12638): a
    // hidden mask or channel target keeps the status bar.
    if hidden && tool == Tool::ArtHistoryBrush && !pixels_target(&crate::canvas::paint_target(app)) {
        return None;
    }
    // A group is the Healing Brush's no-source alert (#12109), hidden or not: it keeps today's
    // messages (the status bar for a hidden one, `refuse_group`'s alert for a shown one).
    if tool == Tool::Healing && app.session.active().and_then(|st| st.doc.layer(st.active_layer?)).is_some_and(photocraft_doc::Layer::is_group) {
        return None;
    }
    let template = if hidden { HIDDEN } else { layer_refusal(app, tool)? };
    Some(crate::i18n::fmt(tl!(template), &[("tool", name)]))
}

/// Whether `target`, a `canvas::paint_target` value, is a layer's pixels as the engine reads it
/// (`channel_cmds::target_of`): not a layer or filter mask, the Quick Mask or an alpha channel. A
/// set of colour channels edits the layer's pixels.
pub(crate) fn pixels_target(target: &serde_json::Value) -> bool {
    use serde_json::Value;
    match target {
        Value::String(s) => !matches!(s.as_str(), "mask" | "filterMask" | "quickMask"),
        Value::Object(o) => o.get("channel").and_then(Value::as_u64).is_none(),
        _ => true,
    }
}

/// [`GROUP`] or [`LOCKED`] when `tool` can't paint the active layer's pixels.
fn layer_refusal(app: &PhotocraftApp, tool: Tool) -> Option<&'static str> {
    let st = app.session.active()?;
    let layer = st.active_layer.and_then(|id| st.doc.layer(id))?;
    if crate::canvas::paint_target(app) != json!("pixels") {
        None
    } else if layer.is_group() {
        Some(GROUP)
    } else if locked(st.doc.effective_locks(layer.id), tool) {
        Some(LOCKED)
    } else {
        None
    }
}

/// Whether `locks` refuse `tool`: Lock All, and for the Sharpen, Sponge, Smudge and Burn tools and the
/// Art History Brush Lock Image Pixels as well (measured for them; the Brush keeps today's
/// behaviour there).
fn locked(locks: photocraft_doc::Locks, tool: Tool) -> bool {
    locks.all || (matches!(tool, Tool::Sharpen | Tool::Sponge | Tool::Smudge | Tool::Burn | Tool::ArtHistoryBrush | Tool::MagicEraser) && locks.pixels)
}

/// A Spot Healing press on an empty pixel layer with Sample All Layers off (#12581): when that layer is hidden, the
/// press does nothing and says nothing, as in Photoshop; there is nothing to heal from.
pub(crate) fn spot_healing_quiet(app: &PhotocraftApp, tool: Tool) -> bool {
    if tool != Tool::SpotHealing || app.ui.tool_options.sample_all.spot_healing || crate::canvas::paint_target(app) != json!("pixels") {
        return false;
    }
    let Some(st) = app.session.active() else { return false };
    st.active_layer
        .and_then(|id| st.doc.layer(id))
        .is_some_and(|l| matches!(l.content, photocraft_doc::LayerContent::Raster(_)) && l.surface().is_some_and(|s| s.content_bounds().is_empty()))
}

#[cfg(test)]
#[path = "../magic_eraser_refusal_alerts_tests.rs"]
mod magic_eraser_refusal_alerts_tests;

#[cfg(test)]
#[path = "../brush_refusal_alert_tests.rs"]
pub(super) mod brush_refusal_alert_tests;

#[cfg(test)]
#[path = "../spot_healing_refusal_alert_tests.rs"]
mod spot_healing_refusal_alert_tests;

#[cfg(test)]
#[path = "../retouch_mode_refusal_alert_tests.rs"]
mod retouch_mode_refusal_alert_tests;

#[cfg(test)]
#[path = "../smudge_refusal_alert_tests.rs"]
mod smudge_refusal_alert_tests;

#[cfg(test)]
#[path = "remove_hidden_layer_tests.rs"]
mod remove_hidden_layer_tests;

#[cfg(test)]
#[path = "art_history_refusal_alert_tests.rs"]
mod art_history_refusal_alert_tests;

#[cfg(test)]
#[path = "../burn_refusal_alert_tests.rs"]
mod burn_refusal_alert_tests;

#[cfg(test)]
#[path = "../healing_refusal_alert_tests.rs"]
mod healing_refusal_alert_tests;
