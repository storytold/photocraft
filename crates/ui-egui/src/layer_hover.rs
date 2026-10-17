//! Highlight a layer from the Layers panel (#3631): with Preferences › Tools › Show Highlight on
//! Rollover on (the default, as for the Move tool's canvas rollover in `move_hover`), hovering a
//! row in the Layers panel outlines that layer's content bounds (a group's, its visible layers'
//! together) on the canvas, whatever tool is current, so a row can be matched to its pixels
//! without selecting it. Nothing is drawn during a drag or a transform.

use egui::Stroke;
use photocraft_doc::{DocId, LayerId};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;

/// The Layers panel row under the pointer, recorded on the pass it was hovered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PanelHover {
    pub doc: DocId,
    pub layer: LayerId,
    pub pass: u64,
}

/// A Layers panel row is hovered this pass.
pub(crate) fn row_hovered(app: &mut PhotocraftApp, ctx: &egui::Context, layer: LayerId) {
    let Some(doc) = app.session.active().map(|st| st.doc.id) else { return };
    app.panel_hover = Some(PanelHover { doc, layer, pass: ctx.cumulative_pass_nr() });
}

/// The document bounds to outline for the hovered row: the row was hovered on pass `pass` or the
/// one before (the panel may be laid out after the canvas), the preference is on, and no drag or
/// transform runs. `None` for the Background, adjustment layers and empty layers.
pub(crate) fn target(app: &PhotocraftApp, pass: u64) -> Option<photocraft_geom::Rect> {
    if !app.session.prefs().tools.show_highlight_on_rollover || app.drag.is_some() || app.ui.transform.is_some() {
        return None;
    }
    let h = app.panel_hover.filter(|h| h.pass + 1 >= pass)?;
    let st = app.session.active().filter(|st| st.doc.id == h.doc)?;
    crate::move_hover::outline(&st.doc, h.layer)
}

/// Draws the hovered row's layer bounds on the active document's canvas, in the accent colour.
pub(crate) fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, idx: usize) {
    if app.session.active_index() != Some(idx) {
        return;
    }
    let Some(b) = target(app, painter.ctx().cumulative_pass_nr()) else { return };
    let accent = crate::theme::Tokens::get(painter.ctx()).accent;
    painter.rect_stroke(xf.doc_rect(b), 0.0, Stroke::new(1.5, accent), egui::StrokeKind::Outside);
}

#[cfg(test)]
#[path = "layer_hover_tests.rs"]
mod tests;
