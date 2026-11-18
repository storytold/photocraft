//! Photoshop's Alt+2 to Alt+6 in a Levels or Curves layer's Properties panel: each picks the Channel select's entry
//! at its position (Alt+2 the composite, Alt+3 Red, Alt+4 Green, Alt+5 Blue).
//!
//! Those keys are the application's `tools.smoothing.N` commands (`opacity_keys`), which set a painting tool's
//! Smoothing. While the Properties panel shows a Levels or Curves layer's editor, docked or in its own window, they pick
//! its channel instead, wherever key focus is; otherwise they keep setting Smoothing. One command per key keeps one
//! binding: rebinding it in Keyboard Shortcuts moves both, and the Channel select shows the key it has. The Image ›
//! Adjustments dialogs run the framework's own `ToneChannelCommands`, which win inside the modal. While a text field
//! has the keys, Alt chords stay with its text.

use std::sync::LazyLock;

use craft::ui::runtime::commands::CommandRef;
use photocraft_doc::LayerContent;
use photocraft_engine::adjust_params;

use super::is_gray;
use super::values;
use crate::PhotocraftApp;

/// The digits of the keys, in entry order: Alt+2 picks the first entry.
const DIGITS: [u8; 5] = [2, 3, 4, 5, 6];

/// The commands of the keys, in entry order, for the panel's Channel select to show their keys.
pub(crate) fn commands() -> &'static [CommandRef] {
    static REFS: LazyLock<Vec<CommandRef>> = LazyLock::new(|| DIGITS.iter().map(|d| CommandRef::new(format!("tools.smoothing.{d}"))).collect());
    &REFS
}

/// How many entries the Channel select of the layer the Properties panel shows has, when the panel is in front,
/// docked or floating, and shows a Levels or Curves layer of the active document; `None` otherwise. An idle panel
/// draws no pass of its own, so its record's pass is no test here.
fn shown_entries(app: &PhotocraftApp) -> Option<usize> {
    if !crate::panel_docking::is_front(app, "properties") {
        return None;
    }
    let record = app.adjust_session.as_ref()?;
    let st = app.session.active()?;
    if st.active_layer != Some(record.layer) {
        return None;
    }
    let Some(LayerContent::Adjustment(adj)) = st.doc.layer(record.layer).map(|l| &l.content) else { return None };
    if !matches!(photocraft_engine::commands::adjustment_kind(adj), "levels" | "curves") {
        return None;
    }
    Some(values::tone_channels(adjust_params::default_space(st.doc.mode), is_gray(st.doc.mode), None).len())
}

/// Takes the Alt+`digit` key when the Properties panel shows a Levels or Curves editor: shows the entry it picks, and
/// takes one past the list without changing anything. False when the panel shows no such editor, or for a digit that
/// picks no entry, so the key sets Smoothing.
pub(crate) fn pick(app: &mut PhotocraftApp, digit: u8) -> bool {
    let Some(at) = DIGITS.iter().position(|d| *d == digit) else { return false };
    let Some(len) = shown_entries(app) else { return false };
    let shown_in = match app.adjust_session.as_mut() {
        Some(record) if at < len => {
            record.channel = at;
            Some(record.shown_in)
        }
        _ => None,
    };
    // The panel's content version takes the channel (`region_versions`), so its next pass runs the body again: in
    // its own window when it floats, and in the main window's pass, which draws a floating panel's window.
    if let Some(viewport) = shown_in {
        app.ctx().request_repaint_of(egui::ViewportId::ROOT);
        app.ctx().request_repaint_of(viewport);
    }
    true
}
