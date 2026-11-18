//! Live preview for the Color Picker targets that are UI state (#9886): a colour row of the hosted
//! parent dialog ([`open_for_request`](super::open_for_request)) and the Crop shield's custom colour.
//!
//! Photoshop shows the colour under the picker's cursor in its target while the picker is open. The
//! parent shows it through `DialogHost::preview_color`, which writes nothing into the parent's
//! values; the shield is set and put back to what it was when the picker opened. The document is
//! never touched.

use craft::color_ui::ui::ColorPickerDialog;
use craft::theme::Srgba;
use craft::ui::canvas::ShieldColor;
use craft::ui::dialog::DialogId;

use super::{PickTarget, rgb_hex};
use crate::PhotocraftApp;

/// A live target as it was when its picker opened.
enum Before {
    /// Colour row `key` of dialog `parent`.
    Row { parent: DialogId, key: String },
    /// The Crop shield's colour choice and custom colour as they were just before the preview took it.
    CropShield { color: ShieldColor, custom: [u8; 3] },
}

/// One open picker's live target.
pub(crate) struct Live {
    picker: DialogId,
    before: Before,
    /// The colour shown in the target; `None` while it shows what it was.
    shown: Option<Srgba>,
}

/// Starts previewing picker `picker` when its `target` has a live target.
pub(super) fn begin(app: &mut PhotocraftApp, picker: DialogId, target: &PickTarget) {
    let before = match target {
        PickTarget::Request { parent, key } => Before::Row { parent: *parent, key: key.clone() },
        PickTarget::CropShield => {
            let s = &app.ui.tool_options.crop.shield;
            Before::CropShield { color: s.color, custom: s.custom_color }
        }
        _ => return,
    };
    app.color_picker_live.push(Live { picker, before, shown: None });
}

#[cfg(test)]
/// The dialogs an open picker is previewing in: they record no ⌘Z step meanwhile, so a Cancel leaves
/// no trace and an OK is one step.
pub(crate) fn previewed_dialogs(app: &PhotocraftApp) -> Vec<DialogId> {
    app.color_picker_live
        .iter()
        .filter_map(|l| match &l.before {
            Before::Row { parent, .. } => Some(*parent),
            Before::CropShield { .. } => None,
        })
        .collect()
}

/// The colour picker `picker` shows: `Some(None)` while it is the one it opened on, `None` once the
/// picker is closed.
fn picked(app: &PhotocraftApp, picker: DialogId) -> Option<Option<Srgba>> {
    let d = app.framework.dialogs.get_by_id::<ColorPickerDialog>(picker)?;
    let color = d.current()?.color;
    let opening = d.opening().map(|o| o.color);
    Some(opening.is_none_or(|o| !o.same_rgb8(color)).then_some(color))
}

fn same(a: Option<Srgba>, b: Option<Srgba>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.same_rgb8(b),
        _ => false,
    }
}

/// Shows `color` in the target, or what it was (`None`). `shown` is what the target showed until now. A shield that
/// no longer holds the shown colour was set elsewhere while the picker was open, and keeps that choice.
fn show(app: &mut PhotocraftApp, before: &mut Before, shown: Option<Srgba>, color: Option<Srgba>) -> Result<(), String> {
    match before {
        Before::Row { parent, key } => {
            app.framework.dialogs.preview_color(*parent, key, color);
            Ok(())
        }
        Before::CropShield { color: was, custom } => match color {
            Some(c) => {
                if shown.is_none() {
                    let s = &app.ui.tool_options.crop.shield;
                    (*was, *custom) = (s.color, s.custom_color);
                }
                crate::crop_ui::set_shield_color(app, &rgb_hex(c))
            }
            None => {
                let s = &mut app.ui.tool_options.crop.shield;
                let rgb8 = |c: Srgba| [c.r, c.g, c.b].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
                if shown.is_some_and(|c| s.color == ShieldColor::Custom && s.custom_color == rgb8(c)) {
                    (s.color, s.custom_color) = (*was, *custom);
                }
                Ok(())
            }
        },
    }
}

/// Shows each open picker's colour in its target, and puts back the target of a picker that closed
/// without [`end`]. True when a target changed (the caller repaints, so the parent redraws).
pub(crate) fn sync(app: &mut PhotocraftApp) -> bool {
    if app.color_picker_live.is_empty() {
        return false;
    }
    let mut live = std::mem::take(&mut app.color_picker_live);
    let mut changed = false;
    live.retain_mut(|l| {
        let Some(color) = picked(app, l.picker) else {
            if l.shown.is_some() {
                changed |= show(app, &mut l.before, l.shown, None).is_ok();
            }
            return false;
        };
        if same(color, l.shown) {
            return true;
        }
        // A target that refuses (the shield with no Crop options) stops the preview.
        if show(app, &mut l.before, l.shown, color).is_err() {
            return false;
        }
        l.shown = color;
        changed = true;
        true
    });
    live.append(&mut app.color_picker_live);
    app.color_picker_live = live;
    changed
}

/// Picker `picker` is closing (OK or Cancel): put its target back as it was when it opened and stop
/// previewing. OK then sets the target.
pub(crate) fn end(app: &mut PhotocraftApp, picker: DialogId) {
    let Some(i) = app.color_picker_live.iter().position(|l| l.picker == picker) else { return };
    let mut l = app.color_picker_live.remove(i);
    let _ = show(app, &mut l.before, l.shown, None);
}

#[cfg(test)]
mod tests;
