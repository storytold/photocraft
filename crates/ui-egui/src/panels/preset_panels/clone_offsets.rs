//! Read-only clone geometry shared by the panel, overlay and source marker.

use photocraft_engine::presets::PresetsExt;
use photocraft_engine::presets::clone_source::{Mapping, resolve};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;

/// Remember a document-space preview position when the pointer is over the active canvas.
/// Retain it while the pointer enters the panel so editing X/Y uses the readout's reference.
pub(crate) fn remember_clone_cursor(app: &mut PhotocraftApp, at: [f64; 2]) -> bool {
    if !matches!(app.ui.tool, Tool::CloneStamp | Tool::Healing) || !at.into_iter().all(|v| v.is_finite() && v.abs() < 1_000_000.0) {
        return false;
    }
    if let Some(document) = app.session.active().map(|st| st.doc.id) {
        let next = Some((document, at));
        if app.ui.presets_ui.clone_cursor != next {
            app.ui.presets_ui.clone_cursor = next;
            return true;
        }
    }
    false
}

/// Resolve the exact sampling geometry without storing a painting anchor or changing a source.
pub fn clone_mapping_at(app: &PhotocraftApp, at: Option<[f64; 2]>) -> Option<Mapping> {
    let slot = app.session.presets().clone.active();
    let source = slot.source?;
    let start = crate::canvas::paint_tool::stroking(app).filter(|stroke| matches!(stroke.tool, Tool::CloneStamp | Tool::Healing)).map(|stroke| stroke.start);
    let first = start.or(at).or(slot.last_anchor).or(slot.anchor).unwrap_or(source);
    let params = json!({"aligned": app.ui.tool_options.clone_aligned});
    let (map, _) = resolve(&app.session, &params, (first[0], first[1]), "clone.preview").ok()?;
    [map.source.0, map.source.1, map.anchor.0, map.anchor.1].into_iter().chain(map.m).all(|v| v.is_finite() && v.abs() < 1_000_000.0).then_some(map)
}

/// Pairing currently previewed in this document, or the most recently committed pairing.
fn panel_mapping(app: &PhotocraftApp) -> Option<Mapping> {
    let cursor = app
        .ui
        .presets_ui
        .clone_cursor
        .filter(|(id, _)| app.session.active().is_some_and(|st| st.doc.id == *id))
        .filter(|_| matches!(app.ui.tool, Tool::CloneStamp | Tool::Healing))
        .map(|(_, at)| at);
    clone_mapping_at(app, cursor)
}

/// Source-minus-destination displacement in document pixels, independent of canvas zoom.
pub(crate) fn clone_offset_readout(app: &PhotocraftApp) -> Option<[f64; 2]> {
    let map = panel_mapping(app)?;
    Some([map.source.0 - map.anchor.0, map.source.1 - map.anchor.1])
}

/// Edit exactly the reference shown in X/Y, including before the first paint or with Aligned off.
pub(crate) fn clone_offset_edit(app: &PhotocraftApp, offset: [f64; 2]) -> Option<Value> {
    let map = panel_mapping(app)?;
    Some(json!({"offset": offset, "anchor": [map.anchor.0, map.anchor.1]}))
}

/// ⇧⌥ with Clone Stamp or the Healing Brush moves the source overlay: an arrow nudges it one document pixel
/// (`direction` is the arrow's unit screen vector; → moves the overlay right, which samples one pixel further
/// left), and Enter (`direction` `None`) lines the source up with the canvas, the source pixel under every
/// destination pixel being the same position (X and Y 0). Each goes through `cloneSource.set`. Returns true when
/// the key was used.
pub(crate) fn clone_overlay_key(app: &mut PhotocraftApp, direction: Option<[f64; 2]>, mods: craft::keymap::Mods) -> bool {
    if !matches!(app.ui.tool, Tool::CloneStamp | Tool::Healing) || !(mods.shift && mods.alt) || mods.cmd || mods.ctrl {
        return false;
    }
    let Some(now) = clone_offset_readout(app) else { return false };
    let offset = match direction {
        Some([dx, dy]) => [now[0] - step(dx), now[1] - step(dy)],
        None => [0.0, 0.0],
    };
    if offset == now {
        return true;
    }
    let Some(params) = clone_offset_edit(app, offset) else { return false };
    if let Err(e) = app.run("cloneSource.set", params) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    true
}

/// ⇧⌥-arrows and ⇧⌥-Enter reach [`clone_overlay_key`] wherever keyboard focus is (a frame changed on the timeline
/// leaves it there, where the arrows would step frames and Enter press a control), unless text is being typed, a
/// dialog is open or a stroke is under way. A key it uses, and its release, leave the frame's input. Runs from
/// `raw_input_hook`, before any widget sees the keys.
pub(crate) fn clone_overlay_raw_keys(app: &mut PhotocraftApp, ctx: &egui::Context, raw: &mut egui::RawInput) {
    if !matches!(app.ui.tool, Tool::CloneStamp | Tool::Healing)
        || ctx.text_edit_focused()
        || app.ui.text_edit.is_some()
        || app.dialog_open()
        || crate::canvas::tools::in_gesture(app)
    {
        return;
    }
    raw.events.retain(|event| {
        let egui::Event::Key { key, pressed, modifiers: m, .. } = event else { return true };
        let direction = match key {
            egui::Key::ArrowLeft => Some([-1.0, 0.0]),
            egui::Key::ArrowRight => Some([1.0, 0.0]),
            egui::Key::ArrowUp => Some([0.0, -1.0]),
            egui::Key::ArrowDown => Some([0.0, 1.0]),
            egui::Key::Enter => None,
            _ => return true,
        };
        let mods = craft::keymap::Mods { cmd: m.mac_cmd, ctrl: m.ctrl, alt: m.alt, shift: m.shift };
        if !*pressed {
            // The release of a key used here: no control acts on it either.
            return !(mods.shift && mods.alt && !mods.ctrl && !mods.cmd);
        }
        !clone_overlay_key(app, direction, mods)
    });
}

/// One pixel along an arrow's axis, none across it (`f64::signum` gives 1 for 0).
fn step(v: f64) -> f64 {
    if v.abs() < 0.5 { 0.0 } else { v.signum() }
}

/// Where the source marker samples at a canvas point, using the overlay's mapping.
pub fn clone_sample_point(app: &PhotocraftApp, at: Option<[f64; 2]>) -> Option<[f64; 2]> {
    let map = clone_mapping_at(app, at)?;
    let (x, y) = at.map_or(map.source, |at| map.map(at[0], at[1]));
    Some([x, y])
}

#[cfg(test)]
mod tests;
